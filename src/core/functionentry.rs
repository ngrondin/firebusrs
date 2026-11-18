use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::data::datamap::DataMap;

use super::interfaces::{Consumer, ServiceProvider, StreamProvider};
use super::threadman::now_millis;

pub enum FunctionKind {
    Service(Arc<dyn ServiceProvider>),
    Stream(Arc<dyn StreamProvider>),
    Consumer(Arc<dyn Consumer>)
}

impl Clone for FunctionKind {
    fn clone(&self) -> Self {
        match self {
            FunctionKind::Service(s) => FunctionKind::Service(Arc::clone(s)),
            FunctionKind::Stream(s) => FunctionKind::Stream(Arc::clone(s)),
            FunctionKind::Consumer(c) => FunctionKind::Consumer(Arc::clone(c))
        }
    }
}

impl FunctionKind {
    pub fn name_char(&self) -> char {
        match self {
            FunctionKind::Service(_) => 's',
            FunctionKind::Stream(_) => 't',
            FunctionKind::Consumer(_) => 'c'
        }
    }
}

// Per-function concurrency slots + statistics (mirrors io.firebus.FunctionEntry).
pub struct FunctionEntry {
    name: String,
    kind: FunctionKind,
    limit: i32,
    reserved: Mutex<Vec<bool>>,
    start: Mutex<Vec<i64>>,
    concurrent: AtomicI32,
    max_since_reset: AtomicI32,
    max_all_time: AtomicI32,
    entry_start: i64,
    cumul_time: AtomicI64,
    exec_count: AtomicI64
}

impl FunctionEntry {
    pub fn new(name: &str, kind: FunctionKind, limit: i32) -> Arc<Self> {
        Arc::new(Self {
            name: String::from(name),
            kind,
            limit,
            reserved: Mutex::new(vec![false; limit.max(1) as usize]),
            start: Mutex::new(vec![0; limit.max(1) as usize]),
            concurrent: AtomicI32::new(0),
            max_since_reset: AtomicI32::new(0),
            max_all_time: AtomicI32::new(0),
            entry_start: now_millis(),
            cumul_time: AtomicI64::new(0),
            exec_count: AtomicI64::new(0)
        })
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_kind(&self) -> &FunctionKind {
        &self.kind
    }

    pub fn get_service_provider(&self) -> Option<&Arc<dyn ServiceProvider>> {
        match &self.kind {
            FunctionKind::Service(s) => Some(s),
            _ => None
        }
    }

    pub fn get_stream_provider(&self) -> Option<&Arc<dyn StreamProvider>> {
        match &self.kind {
            FunctionKind::Stream(s) => Some(s),
            _ => None
        }
    }

    pub fn get_consumer(&self) -> Option<&Arc<dyn Consumer>> {
        match &self.kind {
            FunctionKind::Consumer(c) => Some(c),
            _ => None
        }
    }

    // Reserve one of the `limit` concurrent slots; returns -1 when saturated.
    // Lock order is always reserved -> start.
    pub fn get_execution_id(&self) -> i64 {
        if self.concurrent.load(Ordering::SeqCst) >= self.limit {
            return -1;
        }
        let mut reserved = self.reserved.lock().unwrap();
        let mut start = self.start.lock().unwrap();
        for i in 0..reserved.len() {
            if !reserved[i] {
                reserved[i] = true;
                start[i] = now_millis();
                drop(start);
                drop(reserved);
                let c = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                if c > self.max_since_reset.load(Ordering::SeqCst) {
                    let mut msr = self.max_since_reset.load(Ordering::SeqCst);
                    while c > msr && self.max_since_reset.compare_exchange_weak(msr, c, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                        msr = self.max_since_reset.load(Ordering::SeqCst);
                    }
                    let mut mat = self.max_all_time.load(Ordering::SeqCst);
                    while c > mat && self.max_all_time.compare_exchange_weak(mat, c, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                        mat = self.max_all_time.load(Ordering::SeqCst);
                    }
                }
                self.exec_count.fetch_add(1, Ordering::SeqCst);
                return i as i64;
            }
        }
        -1
    }

    pub fn release_execution_id(&self, id: i64) {
        if id < 0 {
            return;
        }
        let i = id as usize;
        let mut reserved = self.reserved.lock().unwrap();
        let mut start = self.start.lock().unwrap();
        if i < reserved.len() && reserved[i] {
            let s = start[i];
            reserved[i] = false;
            start[i] = 0;
            self.cumul_time.fetch_add(now_millis() - s, Ordering::SeqCst);
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn get_concurrent_count(&self) -> i32 {
        self.concurrent.load(Ordering::SeqCst)
    }

    pub fn get_statistics(&self) -> (String, i32, i32, i32) {
        let name = self.name.clone();
        let lpc = self.max_since_reset.swap(0, Ordering::SeqCst);
        let mat = self.max_all_time.load(Ordering::SeqCst);
        let limit = self.limit;
        (name, lpc, mat, limit)
    }

    pub fn get_status(&self) -> DataMap {
        let now = now_millis();
        let mut status = DataMap::new();
        status.put_int("concurrentLimit", self.limit as i64);
        status.put_int("currentExecutions", self.get_concurrent_count() as i64);
        status.put_int("maxConcurrentSinceLast", self.max_since_reset.load(Ordering::SeqCst) as i64);
        status.put_int("maxConcurrentAllTime", self.max_all_time.load(Ordering::SeqCst) as i64);
        status.put_int("totalExecutionCount", self.exec_count.load(Ordering::SeqCst));
        status.put_int("cumulExecutionTime", self.cumul_time.load(Ordering::SeqCst));
        let span = (now - self.entry_start).max(1);
        status.put_int("utilisation", 100 * self.cumul_time.load(Ordering::SeqCst) / span);
        status
    }
}
