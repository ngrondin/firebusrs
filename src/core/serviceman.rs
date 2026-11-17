use std::sync::Arc;

use super::message::{Message, MessageGenerator, MessageType};
use super::queue::Queue;
use super::rcu::RCUValue;
use super::threadman::ThreadTaskRunner;

pub trait Service {
    fn call(&self);
}

pub struct StartService {
    request_message: Arc<Message>,
    service_name: String
}

struct FunctionEntry {
    name: String,
    service: Arc<dyn Service + Send + Sync>
}

impl Clone for FunctionEntry {
    fn clone(&self) -> Self {
        Self { name: self.name.clone(), service: self.service.clone() }
    }
}

pub struct ServiceManager {
    services: Arc<RCUValue<Vec<FunctionEntry>>>,
    message_queue: Arc<Queue<Message>>,
    service_queue: Arc<Queue<StartService>>,
    message_generator: Arc<MessageGenerator>,
}

impl ServiceManager {
    pub fn new(message_queue: Arc<Queue<Message>>, service_queue: Arc<Queue<StartService>>, message_generator: Arc<MessageGenerator>) -> Self {
        Self { 
            services: Arc::new(RCUValue::new(Vec::new())),
            message_queue,
            service_queue,
            message_generator
        }
    }

    fn get_function_entry(&self, name: &str) -> Option<&FunctionEntry> {
        for function_entry in self.services.read() {
            if function_entry.name == name {
                return Some(function_entry);
            }
        }
        None
    }

    pub fn register_service(&self, name: &str, service: Arc<dyn Service + Send + Sync>) {
        self.services.set(|val| {
            val.push(FunctionEntry { name: String::from(name), service: service.clone()});
        });
    }

    pub fn request_service(&self, msg: Arc<Message>) {
        //Check that the service can be run, and if so, enqueue the start
        let progmsg = self.message_generator.new_message(2, MessageType::Progress, &msg.subject, msg.payload.clone());
        self.message_queue.push(Arc::new(progmsg));
        
        self.service_queue.push( Arc::new( StartService {request_message: msg.clone(), service_name: msg.subject.clone() } ));       
    }

    pub fn process_info_request(&self, msg: Arc<Message>) {

    }

}

impl ThreadTaskRunner for ServiceManager {
    type Data = StartService;
    fn run(&self, start_service: Arc<StartService>) {
        println!("Starting service {}", start_service.service_name);
        match self.get_function_entry(start_service.service_name.as_str()) {
            Some(entry) => entry.service.call(),
            None => ()
        }
    }
}