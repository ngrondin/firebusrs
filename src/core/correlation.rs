use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::data::datamap::DataMap;

use super::interfaces::CorrelationListener;
use super::message::{Message, MessageType};
use super::nodecore::NodeSelfRef;
use super::threadman::{now_millis, ThreadManager};

// Handles the correlation manager needs from the half-built node (mirrors the
// NodeCore parameters Java's CorrelationManager constructor receives, minus
// `this` which Rust models as NodeSelfRef).
#[derive(Clone)]
pub struct CorrHandles {
    pub node_id: u32,
    pub self_ref: NodeSelfRef,
    pub msg_pool: Arc<ThreadManager>,
    pub svc_pool: Arc<ThreadManager>,
    pub stream_pool: Arc<ThreadManager>
}

struct EntryState {
    sequence: i32,
    first_outbound: Option<Arc<Message>>,
    last_received: Option<Arc<Message>>,
    last_popped: Option<Arc<Message>>,
    inbound: HashMap<i32, Arc<Message>>,
    listener: Option<Arc<dyn CorrelationListener>>,
    listener_fn: Option<String>,
    pool: Option<Arc<ThreadManager>>,
    start: i64,
    timeout: i64,
    expiry: i64,
    expired: bool,
    node_id: u32
}

struct ListenerJob {
    pool: Arc<ThreadManager>,
    listener: Arc<dyn CorrelationListener>,
    fn_name: Option<String>,
    out: Option<Arc<Message>>,
    in_msg: Option<Arc<Message>>,
    timeout: bool
}

pub struct CorrelationEntry {
    pub id: i32,
    state: Mutex<EntryState>,
    cond: Condvar
}

impl CorrelationEntry {
    pub fn new(id: i32, timeout: i64, node_id: u32) -> Arc<Self> {
        let start = now_millis();
        Arc::new(Self {
            id,
            state: Mutex::new(EntryState {
                sequence: 0,
                first_outbound: None,
                last_received: None,
                last_popped: None,
                inbound: HashMap::new(),
                listener: None,
                listener_fn: None,
                pool: None,
                start,
                timeout,
                expiry: start + timeout,
                expired: false,
                node_id
            }),
            cond: Condvar::new()
        })
    }

    pub fn set_first_outbound(&self, msg: &Message) {
        let mut st = self.state.lock().unwrap();
        st.first_outbound = Some(Arc::new(msg.clone()));
    }

    // Mirrors CorrelationEntry.setListener: resets the clock and drains anything
    // already queued.
    pub fn set_listener(&self, listener: Arc<dyn CorrelationListener>, fn_name: Option<&str>, pool: &Arc<ThreadManager>, timeout: i64) {
        let mut st = self.state.lock().unwrap();
        let now = now_millis();
        st.timeout = timeout;
        st.start = now;
        st.expiry = now + timeout;
        st.listener = Some(listener);
        st.listener_fn = fn_name.map(String::from);
        st.pool = Some(pool.clone());
        let jobs = drain_jobs(&mut st);
        drop(st);
        run_jobs(jobs);
    }

    // Mirrors CorrelationEntry.waitForNext: returns the next in-sequence inbound
    // message, or None when the entry has been expired by the sweep.
    pub fn wait_for_response(&self, timeout: i64) -> Option<Arc<Message>> {
        let mut st = self.state.lock().unwrap();
        if st.expired {
            return None;
        }
        st.timeout = timeout;
        st.expiry = now_millis() + timeout;
        let deadline = Instant::now() + Duration::from_millis(timeout.max(0) as u64);
        loop {
            if st.expired {
                return None;
            }
            let seq = st.sequence;
            if let Some(m) = st.inbound.remove(&seq) {
                st.last_popped = Some(m.clone());
                st.sequence += 1;
                return Some(m);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.cond.wait_timeout(st, remaining) {
                Ok(result) => st = result.0,
                Err(result) => st = result.into_inner().0
            }
        }
    }

    // Mirrors CorrelationEntry.push.
    pub fn push(&self, msg: &Arc<Message>) {
        let mut st = self.state.lock().unwrap();
        st.inbound.insert(msg.correlation_seq as i32, msg.clone());
        st.last_received = Some(msg.clone());
        st.expiry = now_millis() + st.timeout;
        let jobs = drain_jobs(&mut st);
        drop(st);
        run_jobs(jobs);
        self.cond.notify_all();
    }

    // Mirrors CorrelationEntry.checkExipred (sic).
    pub fn check_expired(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        let now = now_millis();
        if !st.expired && now > st.expiry {
            eprintln!(
                "fb.correntry.expired id={} nodeid={} dur={} exp={} start={} timeout={} listenerfunc={:?}",
                self.id, st.node_id, now - st.start, st.expiry, st.start, st.timeout, st.listener_fn
            );
            st.expired = true;
            let jobs = expiry_jobs(&mut st);
            let expired = st.expired;
            drop(st);
            run_jobs(jobs);
            self.cond.notify_all();
            return expired;
        }
        st.expired
    }

    pub fn force_expire(&self) {
        let mut st = self.state.lock().unwrap();
        st.expired = true;
        let jobs = expiry_jobs(&mut st);
        drop(st);
        run_jobs(jobs);
        self.cond.notify_all();
    }

    pub fn get_status(&self) -> DataMap {
        let st = self.state.lock().unwrap();
        let now = now_millis();
        let mut status = DataMap::new();
        status.put_int("startSince", now - st.start);
        status.put_int("expiryUntil", st.expiry - now);
        if let Some(fn_name) = st.listener_fn.as_ref() {
            status.put_string("listenerFunctionName", fn_name);
        }
        status
    }
}

// Pop messages in sequence order and queue their listener callbacks, exactly
// like drainInboundQueue().
fn drain_jobs(st: &mut EntryState) -> Vec<ListenerJob> {
    let mut jobs = Vec::new();
    if let (Some(listener), Some(pool)) = (st.listener.as_ref(), st.pool.as_ref()) {
        while let Some(msg) = st.inbound.remove(&st.sequence) {
            st.last_popped = Some(msg.clone());
            st.sequence += 1;
            jobs.push(ListenerJob {
                pool: pool.clone(),
                listener: listener.clone(),
                fn_name: st.listener_fn.clone(),
                out: st.first_outbound.clone(),
                in_msg: Some(msg),
                timeout: false
            });
        }
    }
    jobs
}

// Queue the correlation_timedout callback on the listener's pool.
fn expiry_jobs(st: &mut EntryState) -> Vec<ListenerJob> {
    let mut jobs = Vec::new();
    if let (Some(listener), Some(pool)) = (st.listener.as_ref(), st.pool.as_ref()) {
        jobs.push(ListenerJob {
            pool: pool.clone(),
            listener: listener.clone(),
            fn_name: st.listener_fn.clone(),
            out: st.first_outbound.clone(),
            in_msg: None,
            timeout: true
        });
    }
    jobs
}

fn run_jobs(jobs: Vec<ListenerJob>) {
    for job in jobs {
        if job.timeout {
            let listener = job.listener.clone();
            let out = job.out.clone();
            let fn_name = job.fn_name.as_deref();
            job.pool.enqueue(
                Box::new(move || {
                    listener.correlation_timedout(out);
                }),
                fn_name,
                -1
            );
        } else if let Some(in_msg) = job.in_msg {
            let listener = job.listener.clone();
            let out = job.out.clone();
            let fn_name = job.fn_name.as_deref();
            job.pool.enqueue(
                Box::new(move || {
                    listener.correlated_response_received(out, in_msg);
                }),
                fn_name,
                -1
            );
        }
    }
}

// Mirrors io.firebus.CorrelationManager.
pub struct CorrelationManager {
    handles: CorrHandles,
    entries: Mutex<HashMap<i32, Arc<CorrelationEntry>>>,
    next_correlation: AtomicI32,
    last_expiry_check: AtomicI64,
    expired_count: AtomicI64,
    quit: AtomicBool,
    tick: Mutex<Option<JoinHandle<()>>>
}

impl CorrelationManager {
    pub fn new(handles: CorrHandles) -> Arc<Self> {
        let cm = Arc::new(Self {
            handles,
            entries: Mutex::new(HashMap::new()),
            next_correlation: AtomicI32::new(1),
            last_expiry_check: AtomicI64::new(now_millis()),
            expired_count: AtomicI64::new(0),
            quit: AtomicBool::new(false),
            tick: Mutex::new(None)
        });
        let t = cm.clone();
        let handle = thread::Builder::new().name("fbCorrMgr".to_string()).spawn(move || t.run_tick()).unwrap();
        cm.tick.lock().unwrap().replace(handle);
        cm
    }

    // Mirrors createEntry.
    pub fn create_entry(&self, timeout: i64) -> Arc<CorrelationEntry> {
        let id = self.next_correlation.fetch_add(1, Ordering::SeqCst);
        let entry = CorrelationEntry::new(id, timeout, self.handles.node_id);
        self.entries.lock().unwrap().insert(id, entry.clone());
        entry
    }

    pub fn get_entry(&self, correlation_id: i32) -> Option<Arc<CorrelationEntry>> {
        self.entries.lock().unwrap().get(&correlation_id).cloned()
    }

    pub fn remove_entry(&self, correlation_id: i32) {
        self.entries.lock().unwrap().remove(&correlation_id);
    }

    // Mirrors setListenerOnEntry.
    pub fn set_listener_on_entry(&self, correlation_id: i32, listener: Arc<dyn CorrelationListener>, fn_name: Option<&str>, pool: &Arc<ThreadManager>, timeout: i64) {
        match self.get_entry(correlation_id) {
            Some(entry) => entry.set_listener(listener, fn_name, pool, timeout),
            None => eprintln!("fb.correlation.notfoundtowaitfor corr={}", correlation_id)
        }
    }

    // Mirrors waitForResponse.
    pub fn wait_for_response(&self, correlation_id: i32, timeout: i32) -> Option<Arc<Message>> {
        match self.get_entry(correlation_id) {
            Some(entry) => entry.wait_for_response(timeout as i64),
            None => {
                eprintln!("fb.correlation.notfoundtowaitfor corr={}", correlation_id);
                None
            }
        }
    }

    // Mirrors send (with the listener pool chosen by message type).
    pub fn send(&self, mut out_msg: Message, listener: Option<Arc<dyn CorrelationListener>>, timeout: i64) -> i32 {
        let entry = self.create_entry(timeout);
        entry.set_first_outbound(&out_msg);
        if let Some(cl) = listener {
            let pool = if out_msg.message_type == MessageType::RequestStream {
                self.handles.stream_pool.clone()
            } else {
                self.handles.svc_pool.clone()
            };
            entry.set_listener(cl, None, &pool, timeout);
        }
        let id = entry.id;
        out_msg.correlation = id as u32;
        out_msg.correlation_seq = 0;
        self.handles.self_ref.get().enqueue(Arc::new(out_msg));
        id
    }

    // Mirrors sendAndWait.
    pub fn send_and_wait(&self, out_msg: Message, timeout: i32) -> Option<Arc<Message>> {
        let c = self.send(out_msg, None, timeout as i64);
        let m = self.wait_for_response(c, timeout);
        self.remove_entry(c);
        m
    }

    // Mirrors receiveResponse.
    pub fn receive_response(&self, in_msg: &Arc<Message>) {
        let correlation_id = in_msg.correlation as i32;
        if correlation_id != 0 {
            match self.get_entry(correlation_id) {
                Some(entry) => entry.push(in_msg),
                None => {
                    if in_msg.message_type != MessageType::FunctionInfo {
                        eprintln!(
                            "fb.correlation.notfound corr={} service={} originator={} destination={}",
                            correlation_id, in_msg.subject, in_msg.originator, in_msg.destination
                        );
                    }
                }
            }
        }
    }

    fn run_tick(&self) {
        while !self.quit.load(Ordering::SeqCst) {
            let now = now_millis();
            let entries: Vec<Arc<CorrelationEntry>> = self.entries.lock().unwrap().values().cloned().collect();
            for entry in entries {
                if entry.check_expired() {
                    self.entries.lock().unwrap().remove(&entry.id);
                    self.expired_count.fetch_add(1, Ordering::SeqCst);
                }
            }
            self.last_expiry_check.store(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
        let entries: Vec<Arc<CorrelationEntry>> = self.entries.lock().unwrap().values().cloned().collect();
        for entry in entries {
            entry.force_expire();
        }
        if let Some(h) = self.tick.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    pub fn get_status(&self) -> DataMap {
        let entries = self.entries.lock().unwrap();
        let now = now_millis();
        let mut status = DataMap::new();
        let mut entry_map = DataMap::new();
        for (id, entry) in entries.iter() {
            entry_map.put_map(&id.to_string(), entry.get_status());
        }
        status.put_map("entries", entry_map);
        status.put_int("entryCount", entries.len() as i64);
        status.put_int("expiredCount", self.expired_count.load(Ordering::SeqCst));
        status.put_int("lastExpiryCheck", now - self.last_expiry_check.load(Ordering::SeqCst));
        status
    }
}

impl Drop for CorrelationManager {
    fn drop(&mut self) {
        if !self.quit.load(Ordering::SeqCst) {
            self.quit.store(true, Ordering::SeqCst);
        }
    }
}
