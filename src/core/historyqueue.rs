use std::collections::VecDeque;
use std::sync::Mutex;

// Mirrors io.firebus.HistoryQueue: a fixed-size ring of universal message ids
// ((originator << 32) | messageId) used to drop looped/duplicate messages.
pub struct HistoryQueue {
    ids: Mutex<VecDeque<u64>>,
    capacity: usize
}

impl HistoryQueue {
    pub fn new(capacity: usize) -> Self {
        Self { ids: Mutex::new(VecDeque::with_capacity(capacity)), capacity }
    }

    pub fn check(&self, id: u64) -> bool {
        let mut ids = self.ids.lock().unwrap();
        if ids.contains(&id) {
            return false;
        }
        ids.push_back(id);
        while ids.len() > self.capacity {
            ids.pop_front();
        }
        true
    }
}
