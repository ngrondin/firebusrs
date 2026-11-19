use std::sync::{Arc, Mutex};

use super::errors::{FunctionError, FunctionTimeoutError, RequestError};
use super::functionfinder::FunctionFinder;
use super::interfaces::{CorrelationListener, ServiceRequestor};
use super::message::{Message, MessageType};
use super::nodecore::NodeCore;
use super::payload::Payload;
use super::threadman::now_millis;

// Mirrors io.firebus.ServiceRequest (synchronous execute with the
// per-candidate sub-timeout loop).
pub struct ServiceRequest {
    node_core: Arc<NodeCore>,
    service_name: String,
    request_payload: Arc<Payload>,
    sub_timeout: i32,
    request_timeout: i32
}

impl ServiceRequest {
    pub fn new(node_core: Arc<NodeCore>, service_name: &str, request_payload: Arc<Payload>, request_timeout: i32) -> Self {
        Self {
            node_core,
            service_name: String::from(service_name),
            request_payload,
            sub_timeout: 500,
            request_timeout
        }
    }

    pub fn execute(&self) -> Result<Arc<Payload>, RequestError> {
        let mut response_received = false;
        let mut response_payload: Option<Arc<Payload>> = None;
        let mut finder = FunctionFinder::new(self.node_core.clone(), &self.service_name);
        let expiry = now_millis() + if self.request_timeout > -1 { self.request_timeout as i64 } else { self.sub_timeout as i64 };
        let msg_type = if self.request_timeout >= 0 {
            MessageType::RequestService
        } else {
            MessageType::RequestServiceAndForget
        };

        while !response_received && now_millis() < expiry {
            let fi = match finder.find_next() {
                Some(fi) => fi,
                None => continue
            };
            eprintln!("fb.service.request.send node={}", fi.get_node_id());
            let req_msg = self.node_core.message_generator.new_message(
                fi.get_node_id(),
                msg_type,
                &self.service_name,
                self.request_payload.clone()
            );
            let corr = self.node_core.corrman.send(req_msg, None, self.sub_timeout as i64);
            let mut resp_msg = self.node_core.corrman.wait_for_response(corr, self.sub_timeout);
            if resp_msg.is_some() {
                while let Some(resp) = resp_msg.clone() {
                    if now_millis() >= expiry {
                        break;
                    }
                    match resp.message_type {
                        MessageType::ServiceError => {
                            let error_message = resp.payload.get_string();
                            let error_code: i32 = resp.payload.get_meta("errorcode").parse().unwrap_or(0);
                            fi.returned_error();
                            self.node_core.corrman.remove_entry(corr);
                            return Err(RequestError::Service(FunctionError::with_code(&error_message, error_code)));
                        }
                        MessageType::FunctionUnavailable => {
                            eprintln!(
                                "fb.service.request.unavailable service={} node={}",
                                self.service_name,
                                fi.get_node_id()
                            );
                            fi.was_unavailable();
                            break;
                        }
                        MessageType::ServiceResponse => {
                            response_received = true;
                            response_payload = Some(resp.payload.clone());
                            fi.was_succesful();
                            break;
                        }
                        MessageType::Progress => {
                            if msg_type == MessageType::RequestServiceAndForget {
                                response_received = true;
                                response_payload = Some(resp.payload.clone());
                                break;
                            }
                            fi.returned_progress();
                            resp_msg = self.node_core.corrman.wait_for_response(corr, self.request_timeout);
                        }
                        _ => {}
                    }
                    if now_millis() > expiry {
                        eprintln!(
                            "fb.service.request.timeout service={} node={} corr={}",
                            self.service_name,
                            fi.get_node_id(),
                            corr
                        );
                        fi.timed_out_while_executing();
                        return Err(RequestError::Timeout(FunctionTimeoutError::new(&format!(
                            "Service request {} has timed out while executing (corr: {})",
                            self.service_name, corr
                        ))));
                    }
                }
            } else {
                eprintln!(
                    "fb.service.request.noresp service={} node={} corr={}",
                    self.service_name,
                    fi.get_node_id(),
                    corr
                );
                fi.did_not_respond();
            }
            self.node_core.corrman.remove_entry(corr);
        }

        match response_payload {
            Some(payload) if response_received => Ok(payload),
            _ => Err(RequestError::Timeout(FunctionTimeoutError::new(&format!(
                "Service {} could not be called succesfully",
                self.service_name
            ))))
        }
    }
}

// Mirrors io.firebus.ServiceRequestAsync: blocks only until the first
// PROGRESS, then hands the correlation to the requestor.
pub struct ServiceRequestAsync {
    node_core: Arc<NodeCore>,
    service_name: String,
    request_payload: Arc<Payload>,
    requestor: Arc<dyn ServiceRequestor>,
    requestor_function_name: String,
    request_timeout: i32,
    current_fi: Mutex<Option<std::sync::Arc<super::dirman::FunctionInfo>>>
}

impl ServiceRequestAsync {
    pub fn new(
        node_core: Arc<NodeCore>,
        service_name: &str,
        request_payload: Arc<Payload>,
        requestor: Arc<dyn ServiceRequestor>,
        requestor_function_name: &str,
        request_timeout: i32
    ) -> Arc<Self> {
        Arc::new(Self {
            node_core,
            service_name: String::from(service_name),
            request_payload,
            requestor,
            requestor_function_name: String::from(requestor_function_name),
            request_timeout,
            current_fi: Mutex::new(None)
        })
    }

    pub fn execute(self: &Arc<Self>) -> Result<(), RequestError> {
        let mut request_in_progress = false;
        let expiry = now_millis() + if self.request_timeout > -1 { self.request_timeout as i64 } else { 500 };
        let mut last_requested: Option<std::sync::Arc<super::dirman::FunctionInfo>> = None;
        let mut finder = FunctionFinder::new(self.node_core.clone(), &self.service_name);

        while !request_in_progress && now_millis() < expiry {
            let fi = match finder.find_next() {
                Some(fi) => fi,
                None => continue
            };
            if last_requested.as_ref().map(|prev| Arc::ptr_eq(prev, &fi)).unwrap_or(false) {
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
            last_requested = Some(fi.clone());
            eprintln!("fb.service.requestasync.send node={}", fi.get_node_id());
            let msg_type = if self.request_timeout >= 0 {
                MessageType::RequestService
            } else {
                MessageType::RequestServiceAndForget
            };
            let req_msg = self.node_core.message_generator.new_message(
                fi.get_node_id(),
                msg_type,
                &self.service_name,
                self.request_payload.clone()
            );
            let corr = self.node_core.corrman.send(req_msg, None, 500);
            let resp_msg = self.node_core.corrman.wait_for_response(corr, 500);
            if let Some(resp) = resp_msg {
                if resp.message_type == MessageType::Progress {
                    request_in_progress = true;
                    fi.returned_progress();
                    *self.current_fi.lock().unwrap() = Some(fi.clone());
                    self.node_core.corrman.set_listener_on_entry(
                        corr,
                        self.clone(),
                        Some(&self.requestor_function_name),
                        &self.node_core.service_threads,
                        self.request_timeout as i64
                    );
                } else {
                    // Will always only be FunctionUnavailable.
                    eprintln!(
                        "fb.service.requestasync.unavailable service={} node={}",
                        self.service_name,
                        fi.get_node_id()
                    );
                    fi.was_unavailable();
                    self.node_core.corrman.remove_entry(corr);
                }
            } else {
                eprintln!(
                    "fb.service.requestasync.noresp service={} node={} corr={}",
                    self.service_name,
                    fi.get_node_id(),
                    corr
                );
                fi.did_not_respond();
                self.node_core.corrman.remove_entry(corr);
            }
        }

        if !request_in_progress {
            return Err(RequestError::Timeout(FunctionTimeoutError::new(&format!(
                "Service {} could not be found",
                self.service_name
            ))));
        }
        Ok(())
    }
}

impl CorrelationListener for ServiceRequestAsync {
    fn correlated_response_received(&self, _out: Option<Arc<Message>>, in_msg: Arc<Message>) {
        match in_msg.message_type {
            MessageType::ServiceError => {
                let error_message = in_msg.payload.get_string();
                let error_code: i32 = in_msg.payload.get_meta("errorcode").parse().unwrap_or(0);
                if let Some(fi) = self.current_fi.lock().unwrap().as_ref() {
                    fi.returned_error();
                }
                self.requestor.error(FunctionError::with_code(&error_message, error_code));
            }
            MessageType::ServiceResponse => {
                let response_payload = in_msg.payload.clone();
                if let Some(fi) = self.current_fi.lock().unwrap().as_ref() {
                    fi.was_succesful();
                }
                self.requestor.response(response_payload);
            }
            _ => {}
        }
        self.node_core.corrman.remove_entry(in_msg.correlation as i32);
    }

    fn correlation_timedout(&self, _out: Option<Arc<Message>>) {
        if let Some(fi) = self.current_fi.lock().unwrap().as_ref() {
            fi.timed_out_while_executing();
        }
        self.requestor.timeout();
    }
}
