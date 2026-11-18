use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::data::datamap::DataMap;

pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// A unit of work for the Firebus thread pools (mirrors FirebusRunnable). The
// closure sits behind a Mutex (take-once) so that Task stays Sync across all
// the thread-pool queues it is shared through.
pub struct Task {
    run: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub function_name: Option<String>,
    pub execution_id: i64,
    pub created: i64,
    pub pushed: i64,
    pub warning_time: i64,
    pub expiry: i64,
    pub log_data: Option<DataMap>
}

impl Task {
    pub fn new(run: Box<dyn FnOnce() + Send>, function_name: Option<String>, execution_id: i64, timeout: i64, warning_time: i64, log_data: Option<DataMap>) -> Self {
        let created = now_millis();
        Self {
            run: Mutex::new(Some(run)),
            function_name,
            execution_id,
            created,
            pushed: created,
            warning_time,
            expiry: created + timeout,
            log_data
        }
    }

    pub fn run(self: &std::sync::Arc<Self>) {
        if let Some(run) = self.run.lock().unwrap().take() {
            run();
        }
    }
}

// Blocking FIFO used by the pools (mirrors io.firebus.utils.Queue, grow-only variant).
pub struct TaskQueue {
    items: Mutex<std::collections::VecDeque<Arc<Task>>>,
    cond: Condvar,
    consumers: AtomicI32,
    count: AtomicI64,
    max_depth: AtomicI32,
    min_consumers: AtomicI32,
    depth: AtomicI32,
    quit: AtomicBool
}

impl TaskQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(std::collections::VecDeque::new()),
            cond: Condvar::new(),
            consumers: AtomicI32::new(0),
            count: AtomicI64::new(0),
            max_depth: AtomicI32::new(0),
            min_consumers: AtomicI32::new(0),
            depth: AtomicI32::new(0),
            quit: AtomicBool::new(false)
        })
    }

    pub fn push(&self, task: Arc<Task>) {
        let mut guard = self.items.lock().unwrap();
        guard.push_back(task);
        let d = guard.len() as i32;
        self.depth.store(d, Ordering::SeqCst);
        self.count.fetch_add(1, Ordering::SeqCst);
        let mut max = self.max_depth.load(Ordering::SeqCst);
        while d > max {
            match self.max_depth.compare_exchange_weak(max, d, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(m) => max = m,
            }
        }
        drop(guard);
        self.cond.notify_one();
    }

    pub fn get_depth(&self) -> i32 {
        self.depth.load(Ordering::SeqCst)
    }

    pub fn get_consumers(&self) -> i32 {
        self.consumers.load(Ordering::SeqCst)
    }

    // Blocking pop; returns None once the queue (or the requesting worker) is closed.
    // Mirrors Queue.popWait().
    pub fn pop_wait(&self, worker_quit: &AtomicBool) -> Option<Arc<Task>> {
        self.consumers.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.items.lock().unwrap();
        let result = loop {
            if self.quit.load(Ordering::SeqCst) || worker_quit.load(Ordering::SeqCst) {
                break None;
            }
            if let Some(t) = guard.pop_front() {
                self.depth.fetch_sub(1, Ordering::SeqCst);
                break Some(t);
            }
            match self.cond.wait_timeout(guard, Duration::from_millis(100)) {
                Ok((g, _)) => guard = g,
                Err(e) => { let _ = e.into_inner(); break None }
            }
        };
        let c = self.consumers.fetch_add(-1, Ordering::SeqCst);
        let mut min = self.min_consumers.load(Ordering::SeqCst);
        while c < min {
            match self.min_consumers.compare_exchange_weak(min, c, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(m) => min = m,
            }
        }
        result
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
        self.cond.notify_all();
    }

    pub fn get_status(&self) -> DataMap {
        let d = self.get_depth();
        let mut status = DataMap::new();
        status.put_int("size", d as i64);
        status.put_int("depth", d as i64);
        status.put_int("count", self.count.load(Ordering::SeqCst));
        status.put_int("consumers", self.get_consumers() as i64);
        status.put_int("maxdepth", self.max_depth.load(Ordering::SeqCst) as i64);
        status.put_int("minconsumers", self.min_consumers.load(Ordering::SeqCst) as i64);
        status
    }
}

struct ThreadContext {
    started: i64,
    expiry: i64,
    function_name: Option<String>,
    execution_id: i64
}

struct ThreadStats {
    total_execution_count: i64,
    cumul_execution_time: i64,
    max_execution_time: i64
}

// One pool worker (mirrors FirebusThread). Workers only need the queue, so no
// self-reference to the manager is required.
pub struct FirebusThread {
    thread_name: String,
    queue: Arc<TaskQueue>,
    quit: AtomicBool,
    ctx: Mutex<Option<ThreadContext>>,
    stats: Mutex<ThreadStats>
}

impl FirebusThread {
    pub fn new(queue: Arc<TaskQueue>, id: i32, name_part: &str) -> Arc<Self> {
        let name = format!("fb{}Thread{:02}", name_part, id);
        let t = Arc::new(Self {
            thread_name: name.clone(),
            queue,
            quit: AtomicBool::new(false),
            ctx: Mutex::new(None),
            stats: Mutex::new(ThreadStats { total_execution_count: 0, cumul_execution_time: 0, max_execution_time: 0 })
        });
        let me = t.clone();
        thread::Builder::new().name(name).spawn(move || me.run()).unwrap();
        t
    }

    pub fn get_name(&self) -> &str {
        &self.thread_name
    }

    fn run(&self) {
        while !self.quit.load(Ordering::SeqCst) {
            let task = match self.queue.pop_wait(&self.quit) {
                Some(t) => t,
                None => return
            };
            let now = now_millis();
            self.ctx.lock().unwrap().replace(ThreadContext {
                started: now,
                expiry: task.expiry,
                function_name: task.function_name.clone(),
                execution_id: task.execution_id
            });
            if now - task.created > 100 {
                eprintln!("fb.thread.run.longwait ms={} consumers={} depth={}", now - task.created, self.queue.get_consumers(), self.queue.get_depth());
            }
            let warn_time = task.warning_time;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || task.run()));
            if result.is_err() {
                eprintln!("fb.thread.run: task panicked");
            }
            let completion = now_millis();
            let duration = completion - now;
            {
                let mut stats = self.stats.lock().unwrap();
                stats.total_execution_count += 1;
                stats.cumul_execution_time += duration;
                if duration > stats.max_execution_time {
                    stats.max_execution_time = duration;
                }
            }
            if warn_time > -1 && duration > warn_time {
                eprintln!("fb.thread.run.longexec ms={}", duration);
            }
            self.ctx.lock().unwrap().take();
        }
    }

    // Cooperative timeout check (Java interrupts the thread; a Rust task cannot be
    // killed, so expiry is only flagged).
    pub fn check_expiry(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            if now_millis() > ctx.expiry {
                eprintln!("fb.thread.expired fn={:?}", ctx.function_name);
            }
        }
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
        self.queue.pop_wait_and_wake();
    }

    pub fn get_status(&self) -> DataMap {
        let now = now_millis();
        let executing = self.ctx.lock().unwrap().is_some();
        let mut status = DataMap::new();
        status.put_bool("executing", executing);
        if executing {
            if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
                status.put_string("executingFunctionName", ctx.function_name.clone().as_deref().unwrap_or(""));
                status.put_int("executingFunctionId", ctx.execution_id);
                status.put_int("executingSince", now - ctx.started);
            }
        }
        let stats = self.stats.lock().unwrap();
        status.put_int("totalExecutionCount", stats.total_execution_count);
        status.put_int("cumulExecutionTime", stats.cumul_execution_time);
        let mut max = DataMap::new();
        max.put_int("time", stats.max_execution_time);
        status.put_map("max", max);
        status
    }
}

impl TaskQueue {
    // Wake a single blocked worker, if any (used when a worker is being removed).
    fn pop_wait_and_wake(&self) {
        self.cond.notify_all();
    }
}

// A dynamic min/max pool of FirebusThreads plus a 1s maintenance tick,
// mirroring io.firebus.threads.ThreadManager.
pub struct ThreadManager {
    queue: Arc<TaskQueue>,
    threads: Mutex<Vec<Arc<FirebusThread>>>,
    min_threads: AtomicI32,
    max_threads: AtomicI32,
    default_timeout: i64,
    default_warning: i64,
    name: String,
    quit: AtomicBool,
    tick: Mutex<Option<JoinHandle<()>>>
}

impl ThreadManager {
    pub fn new(name: &str, min_threads: i32, max_threads: i32, default_timeout: i64, default_warning: i64) -> Arc<Self> {
        let tm = Arc::new(Self {
            queue: TaskQueue::new(),
            threads: Mutex::new(Vec::new()),
            min_threads: AtomicI32::new(min_threads),
            max_threads: AtomicI32::new(max_threads),
            default_timeout,
            default_warning,
            name: String::from(name),
            quit: AtomicBool::new(false),
            tick: Mutex::new(None)
        });
        tm.set_thread_count(min_threads, max_threads);
        let tick = tm.clone();
        let handle = thread::Builder::new().name(format!("fb{}ThreadMgr", tick.name)).spawn(move || tick.run_tick()).unwrap();
        tm.tick.lock().unwrap().replace(handle);
        tm
    }

    fn run_tick(&self) {
        // for all threads to be connected before managing
        loop {
            if self.quit.load(Ordering::SeqCst) {
                return;
            }
            if self.queue.get_consumers() >= self.threads.lock().unwrap().len() as i32 {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        while !self.quit.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1000));
            if self.quit.load(Ordering::SeqCst) {
                return;
            }
            let threads: Vec<Arc<FirebusThread>> = self.threads.lock().unwrap().clone();
            for t in threads {
                t.check_expiry();
            }
            if self.queue.get_consumers() == 0 && (self.threads.lock().unwrap().len() as i32) < self.max_threads.load(Ordering::SeqCst) {
                self.spawn_thread();
            }
        }
    }

    fn spawn_thread(&self) {
        let threads = self.threads.lock().unwrap();
        let id = threads.len() as i32;
        drop(threads);
        let t = FirebusThread::new(self.queue.clone(), id, &self.name);
        self.threads.lock().unwrap().push(t);
    }

    pub fn set_thread_count(&self, min_threads: i32, max_threads: i32) {
        self.min_threads.store(min_threads, Ordering::SeqCst);
        self.max_threads.store(max_threads, Ordering::SeqCst);
        let min_threads = self.min_threads.load(Ordering::SeqCst);
        let max_threads = self.max_threads.load(Ordering::SeqCst);
        loop {
            let count = self.threads.lock().unwrap().len() as i32;
            if count < min_threads {
                self.spawn_thread();
            } else if count > max_threads {
                if let Some(t) = self.threads.lock().unwrap().pop() {
                    t.close();
                }
            } else {
                break;
            }
        }
    }

    pub fn enqueue(&self, run: Box<dyn FnOnce() + Send>, function_name: Option<&str>, execution_id: i64) {
        let task = Task::new(run, function_name.map(String::from), execution_id, self.default_timeout, self.default_warning, None);
        self.queue.push(Arc::new(task));
    }

    pub fn enqueue_with(&self, run: Box<dyn FnOnce() + Send>, function_name: Option<&str>, execution_id: i64, timeout: i64, warning: i64, log_data: Option<DataMap>) {
        let task = Task::new(run, function_name.map(String::from), execution_id, timeout, warning, log_data);
        self.queue.push(Arc::new(task));
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_thread_count(&self) -> i32 {
        self.threads.lock().unwrap().len() as i32
    }

    pub fn get_queue_depth(&self) -> i32 {
        self.queue.get_depth()
    }

    pub fn queue(&self) -> &Arc<TaskQueue> {
        &self.queue
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
        self.set_thread_count(0, 0);
        self.queue.close();
        if let Some(h) = self.tick.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        let mut tmap = DataMap::new();
        for t in self.threads.lock().unwrap().iter() {
            tmap.put_map(t.get_name(), t.get_status());
        }
        status.put_map("threads", tmap);
        status.put_int("threadCount", self.get_thread_count() as i64);
        status.put_map("queue", self.queue.get_status());
        status
    }
}
