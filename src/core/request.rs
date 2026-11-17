use std::sync::Arc;

use crate::data::errors::DataError;

use super::{corrman::CorrManager, dirman::DirManager, message::{MessageGenerator, MessageType}, payload::Payload};



pub struct Request {
    dirman: Arc<DirManager>,
    corrman: Arc<CorrManager>,
    message_generator: Arc<MessageGenerator>,
    service_name: String,
    request_payload: Arc<Payload>,
}

impl Request {
    pub fn new(dirman: Arc<DirManager>, corrman: Arc<CorrManager>, mg: Arc<MessageGenerator>, sn: &str, request_payload: Arc<Payload>) -> Self {
        Self { dirman, corrman, message_generator: mg, service_name: String::from(sn), request_payload }
    }

    pub fn execute(&self) -> Result<Payload, DataError> {
        match self.find_function() {
            Some(dest) => {
                let reqmsg = self.message_generator.new_message(dest, MessageType::RequestService, &self.service_name, self.request_payload.clone());
                self.corrman.send(Arc::new(reqmsg));
                Ok(Payload::from_string("Result"))       
            },
            None => {
                Err( DataError { message: "No response".to_owned()})
            }
        }
    }

    fn find_function(&self) -> Option<u32> {
        match self.dirman.find_best_function(&self.service_name) {
            Some(dest) => {
                return Some(dest);     
            },
            None => {
                let reqmsg = self.message_generator.new_message(0, MessageType::GetFunctionInfo, &self.service_name, Arc::new(Payload::empty()));
                self.corrman.send(Arc::new(reqmsg));
                return None;
            }
        }
    }

}