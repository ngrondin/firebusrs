use std::sync::Arc;
use rand::Rng;

use corrman::CorrManager;
use dirman::DirManager;
use message::{Message, MessageGenerator};
use payload::Payload;
use queue::Queue;
use request::Request;
use router::Router;
use serviceman::{Service, ServiceManager, StartService};
use threadman::ThreadManager;

mod router;
pub mod serviceman;
mod threadman;
mod dirman;
mod corrman;
pub mod message;
pub mod payload;
mod request;
mod queue;
pub mod rcu;


pub struct Firebus {
    pub node_id: u32,
    message_generator: Arc<MessageGenerator>,
    message_queue: Arc<Queue<Message>>,
    service_queue: Arc<Queue<StartService>>,
    pub dirman: Arc<DirManager>,
    corrman: Arc<CorrManager>,
    serviceman: Arc<ServiceManager>,
    router: Arc<Router>,
    message_threads: Arc<ThreadManager<Message>>,
    service_threads: Arc<ThreadManager<StartService>>
}

impl Firebus {
    pub fn new() -> Self {
        let mut rand = rand::thread_rng();
        let node_id: u32 = rand.gen();
        let message_generator = Arc::new( MessageGenerator::new(node_id) );
        let message_queue: Arc<Queue<Message>> = Arc::new( Queue::new() );
        let service_queue: Arc<Queue<StartService>> = Arc::new( Queue::new() );
        let dirman = Arc::new( DirManager::new() );
        let corrman = Arc::new( CorrManager::new( message_queue.clone() ) );
        let serviceman = Arc::new( ServiceManager::new( message_queue.clone(), service_queue.clone(), message_generator.clone() ) );
        let router = Arc::new( Router::new(node_id, dirman.clone(), corrman.clone(), serviceman.clone(), message_queue.clone() ) );
        let message_threads: Arc<ThreadManager<Message>> = Arc::new( ThreadManager::new(message_queue.clone(), router.clone()) );
        let service_threads: Arc<ThreadManager<StartService>> = Arc::new( ThreadManager::new(service_queue.clone(), serviceman.clone() ));
        Self{ node_id, message_generator, message_queue, service_queue, dirman, corrman, serviceman, router, message_threads, service_threads }
    }

    pub fn register_service(&self, name: &str, func: Arc<dyn Service + Send + Sync>) {
        self.serviceman.register_service(name, func)
    }

    pub fn request(&self, service: &str, payload: Arc<Payload>) {
        let request = Request::new(self.dirman.clone(), self.corrman.clone(), self.message_generator.clone(), service, payload);
        let _ = request.execute();
        println!("Service {} requested", service);
    }
}


