use std::{collections::VecDeque, sync::{mpsc::{channel, Sender}, Arc, Mutex}};

pub struct Queue<T> {
    data: Arc<Mutex<VecDeque<Arc<T>>>>,
    waiting: Arc<Mutex<Vec<Sender<bool>>>>
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Queue {
            data: Arc::new( Mutex::new( VecDeque::new() ) ),
            waiting: Arc::new( Mutex::new( Vec::new() ) )
        }
    }

    pub fn push(&self, value: Arc<T>) {
        self.data.lock().unwrap().push_back(value);
        let sender_result = self.waiting.lock().unwrap().pop();
        if let Some(sender) = sender_result {
            let _ = sender.send(true);
        }
    }

    pub fn get_next(&self) -> Arc<T> {
        loop {
            let value = self.data.lock().unwrap().pop_front();
            match value {
                Some(val) => return val,
                None => {
                    let (sender, receiver) = channel::<bool>();                    
                    self.waiting.lock().unwrap().push(sender);
                    receiver.recv().unwrap();       
                }
            }
        }
    }
}
