use std::{ sync::{Arc, Mutex}, thread };

use super::queue::Queue;


pub trait ThreadTaskRunner {
    type Data;
    fn run(&self, data: Arc<Self::Data>);
}

pub struct Thread<D: Send + Sync> {
    id: u32,
    queue: Arc<Queue<D>>,
    task_runner: Arc<dyn ThreadTaskRunner<Data = D> + Send + Sync>    
}

impl<D: Send + Sync> Thread<D> {
    fn new(id: u32, queue: Arc<Queue<D>>, task_runner: Arc<dyn ThreadTaskRunner<Data = D> + Send + Sync>) -> Self {
        Self { id, queue, task_runner }
    }

    fn run(&self) {
        //println!("Thread {} started", self.id);
        loop {
            let data = self.queue.get_next();
            self.task_runner.run(data);
        }
    }
}

pub struct ThreadManager<D: Send + Sync + 'static> {
    threads: Arc<Mutex<Vec<Arc<Thread<D>>>>>,
    queue: Arc<Queue<D>>,
    task_runner: Arc<dyn ThreadTaskRunner<Data = D> + Send + Sync>
}

impl<D: Send + Sync> ThreadManager<D> {
    pub fn new(queue: Arc<Queue<D>>, task_runner: Arc<dyn ThreadTaskRunner<Data = D> + Send + Sync>) -> Self {
        let me = Self { 
            threads: Arc::new( Mutex::new( Vec::new() ) ) ,
            queue,
            task_runner
        };
        for i in 0..2 {
            me.spawn(i);
        } 
        me 
    }

    fn spawn(&self, id: u32) {
        let threads = self.threads.clone();
        let q = self.queue.clone();
        let tr = self.task_runner.clone();
        thread::spawn(move || {
            let thread = Arc::new(Thread::new(id, q, tr));
            threads.lock().unwrap().push(thread.clone());
            thread.run();
        });
    }


}
