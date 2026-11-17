use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::Arc;

use super::message::Message;
use super::queue::Queue;


pub struct CorrManager {
    msgqueue: Arc<Queue<Message>>,
    entries: Arc<Mutex<HashMap<String, String>>>,
}

impl CorrManager {
    pub fn new(msgqueue: Arc<Queue<Message>>) -> Self {
        Self { 
            msgqueue,
            entries: Arc::new(Mutex::new(HashMap::new()))
        }
    }

    pub fn send(&self, msg: Arc<Message>) {
        self.msgqueue.push(msg);
    }

}