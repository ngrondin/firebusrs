use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use super::dirman::FunctionInfo;
use super::errors::{FunctionError, FunctionTimeoutError, RequestError};
use super::functionfinder::FunctionFinder;
use super::message::MessageType;
use super::nodecore::NodeCore;
use super::payload::Payload;
use super::streamep::StreamEndpoint;
use super::threadman::now_millis;

// Mirrors io.firebus.StreamRequest: polls the function finder for a candidate,
// sends a RequestStream, waits for the StreamAccept (or a refusal), and hands
// back the live StreamEndpoint.
pub struct StreamRequest {
    node_core: Arc<NodeCore>,
    stream_name: String,
    request_payload: Arc<Payload>,
    requestor_function_name: Option<String>,
    sub_timeout: i32
}

impl StreamRequest {
    pub fn new(
        node_core: Arc<NodeCore>,
        stream_name: &str,
        request_payload: Arc<Payload>,
        requestor_function_name: Option<String>,
        sub_timeout: i32
    ) -> Self {
        Self {
            node_core,
            stream_name: String::from(stream_name),
            request_payload,
            requestor_function_name,
            sub_timeout
        }
    }

    pub fn initiate(&self) -> Result<Arc<StreamEndpoint>, RequestError> {
        let mut endpoint: Option<Arc<StreamEndpoint>> = None;
        let mut last_requested: Option<Arc<FunctionInfo>> = None;
        let mut finder = FunctionFinder::new(self.node_core.clone(), &self.stream_name);
        let expiry = now_millis() + self.sub_timeout as i64;

        while endpoint.is_none() && now_millis() < expiry {
            let fi = match finder.find_next() {
                Some(fi) => fi,
                None => continue
            };
            if last_requested.as_ref().map(|prev| Arc::ptr_eq(prev, &fi)).unwrap_or(false) {
                sleep(Duration::from_millis(1000));
            }
            last_requested = Some(fi.clone());
            eprintln!("fb.stream.request.send node={}", fi.get_node_id());
            let req_msg = self.node_core.message_generator.new_message(
                fi.get_node_id(),
                MessageType::RequestStream,
                &self.stream_name,
                self.request_payload.clone()
            );
            let corr = self.node_core.corrman.send(req_msg, None, self.sub_timeout as i64);
            let resp_msg = self.node_core.corrman.wait_for_response(corr, self.sub_timeout);
            if let Some(resp) = resp_msg {
                while now_millis() < expiry {
                    match resp.message_type {
                        MessageType::StreamError => {
                            let error_message = resp.payload.get_string();
                            let error_code: i32 = resp.payload.get_meta("errorcode").parse().unwrap_or(0);
                            fi.returned_error();
                            self.node_core.corrman.remove_entry(corr);
                            return Err(RequestError::Service(FunctionError::with_code(&error_message, error_code)));
                        }
                        MessageType::FunctionUnavailable => {
                            eprintln!(
                                "fb.stream.request.unavailable stream={} node={}",
                                self.stream_name,
                                fi.get_node_id()
                            );
                            fi.was_unavailable();
                            self.node_core.corrman.remove_entry(corr);
                            break;
                        }
                        MessageType::StreamAccept => {
                            let accept_payload = resp.payload.clone();
                            let remote_correlation: i32 = accept_payload.get_meta("correlationid").parse().unwrap_or(0);
                            // Let the stream server expire first.
                            let idle_timeout: i64 = accept_payload.get_meta("timeout").parse().unwrap_or(0) + 2000;
                            let ep = StreamEndpoint::new(
                                self.node_core.ctx.clone(),
                                &self.stream_name,
                                corr,
                                remote_correlation,
                                0,
                                fi.get_node_id()
                            );
                            ep.set_accept_payload(accept_payload);
                            ep.set_request_payload(self.request_payload.clone());
                            self.node_core.corrman.set_listener_on_entry(
                                corr,
                                ep.clone(),
                                self.requestor_function_name.as_deref(),
                                &self.node_core.stream_threads,
                                idle_timeout
                            );
                            ep.activate();
                            fi.was_succesful();
                            endpoint = Some(ep);
                            break;
                        }
                        _ => {}
                    }
                    if now_millis() > expiry {
                        eprintln!(
                            "fb.stream.request.timeout stream={} node={} corr={}",
                            self.stream_name,
                            fi.get_node_id(),
                            corr
                        );
                        fi.timed_out_while_executing();
                        self.node_core.corrman.remove_entry(corr);
                        return Err(RequestError::Timeout(FunctionTimeoutError::new(&format!(
                            "Stream request {} has timed out while executing (corr: {})",
                            self.stream_name, corr
                        ))));
                    }
                }
            } else {
                eprintln!(
                    "fb.stream.request.noresp stream={} node={} corr={}",
                    self.stream_name,
                    fi.get_node_id(),
                    corr
                );
                fi.did_not_respond();
                self.node_core.corrman.remove_entry(corr);
            }
        }

        match endpoint {
            Some(ep) => Ok(ep),
            None => Err(RequestError::Timeout(FunctionTimeoutError::new(&format!(
                "Stream {} could not be found",
                self.stream_name
            ))))
        }
    }
}
