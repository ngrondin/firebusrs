use std::sync::Arc;

use crate::core::message::MessageType;

use super::{corrman::CorrManager, dirman::DirManager, message::Message, queue::Queue, serviceman::ServiceManager, threadman::ThreadTaskRunner};



pub struct Router {
    node_id: u32,
    dirman: Arc<DirManager>,
    corrman: Arc<CorrManager>,
    serviceman: Arc<ServiceManager>,
    msgqueue: Arc<Queue<Message>>,
}

impl Router {
    pub fn new(node_id: u32, dirman: Arc<DirManager>, corrman: Arc<CorrManager>, serviceman: Arc<ServiceManager>, msgqueue: Arc<Queue<Message>>) -> Self {
        Self {node_id, dirman, corrman, serviceman, msgqueue }        
    }
}

impl ThreadTaskRunner for Router {
    type Data = Message;

    fn run(&self, message: Arc<Message>) {
        println!("Routing message {}", message.subject);
        if message.destination == 0 || message.destination == self.node_id {
            match message.message_type {
                MessageType::RequestService => self.serviceman.request_service(message),
                MessageType::GetFunctionInfo => self.serviceman.process_info_request(message),
                _ => println!("Message type not implemented")
            }    
        }
        
    }
}