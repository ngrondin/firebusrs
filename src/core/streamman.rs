use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::data::datamap::DataMap;

use super::dirman::FunctionInfo;
use super::execman;
use super::functionentry::{FunctionEntry, FunctionKind};
use super::interfaces::StreamProvider;
use super::message::{Message, MessageType};
use super::nodecore::NodeContext;
use super::payload::Payload;
use super::streamep::StreamEndpoint;

// Mirrors io.firebus.StreamManager.
pub struct StreamManager {
    ctx: Arc<NodeContext>,
    streams: Mutex<HashMap<String, Arc<FunctionEntry>>>
}

impl StreamManager {
    pub fn new(ctx: Arc<NodeContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            streams: Mutex::new(HashMap::new())
        })
    }

    // Mirrors addStream; also registers the function in this node's directory
    // entry (see ServiceManager::add_service for the rationale).
    pub fn add_stream(&self, name: &str, provider: Arc<dyn StreamProvider>, max_concurrent: i32) {
        eprintln!("fb.stream.manager.add stream={}", name);
        let mut streams = self.streams.lock().unwrap();
        if !streams.contains_key(name) {
            let entry = FunctionEntry::new(name, FunctionKind::Stream(provider.clone()), max_concurrent);
            streams.insert(String::from(name), entry);
        }
        drop(streams);
        let spec = (*provider).get_stream_information().unwrap_or_default();
        let dir = self.ctx.self_ref.get().get_directory().get_or_create_node_information(self.ctx.node_id);
        dir.add_function_information(
            name,
            FunctionInfo::new(self.ctx.node_id, Some(Arc::clone(&dir.rating)), 't', name, spec)
        );
    }

    pub fn remove_stream(&self, name: &str) {
        eprintln!("fb.stream.manager.remove stream={}", name);
        self.streams.lock().unwrap().remove(name);
    }

    pub fn has_stream(&self, name: &str) -> bool {
        self.streams.lock().unwrap().contains_key(name)
    }

    fn get_function_entry(&self, name: &str) -> Option<Arc<FunctionEntry>> {
        self.streams.lock().unwrap().get(name).cloned()
    }

    fn entries(&self) -> Vec<Arc<FunctionEntry>> {
        self.streams.lock().unwrap().values().cloned().collect()
    }

    // Mirrors connectStream: runs the provider's accept call on the service
    // pool, wires the new StreamEndpoint into the correlation manager, and
    // answers with a StreamAccept.
    pub fn connect_stream(&self, msg: Arc<Message>) {
        let name = msg.subject.clone();
        let in_payload = msg.payload.clone();
        match self.get_function_entry(&name) {
            Some(fe) => {
                let execution_id = fe.get_execution_id();
                if execution_id != -1 {
                    let provider = fe.get_stream_provider().cloned();
                    let originator = msg.originator;
                    let correlation = msg.correlation;
                    let ctx = self.ctx.clone();
                    let entry_clone = fe.clone();
                    let name_clone = name.clone();
                    let exec_id = execution_id;
                    self.ctx.send_service_task(
                        Box::new(move || {
                            if let Some(provider) = provider {
                                let idle_timeout = provider.get_stream_idle_timeout();
                                let corr_entry = ctx.corrman.create_entry(idle_timeout);
                                let local_correlation_id = corr_entry.id;
                                let endpoint = StreamEndpoint::new(
                                    ctx.clone(),
                                    &name_clone,
                                    local_correlation_id,
                                    correlation as i32,
                                    1,
                                    originator
                                );
                                endpoint.set_request_payload(in_payload.clone());
                                match provider.accept_stream(in_payload.as_ref(), endpoint.as_ref()) {
                                    Ok(accept_payload_opt) => {
                                        let mut accept_payload =
                                            accept_payload_opt.unwrap_or_else(Payload::empty);
                                        accept_payload.set_meta("correlationid", &local_correlation_id.to_string());
                                        accept_payload.set_meta("timeout", &idle_timeout.to_string());
                                        let accept_payload = Arc::new(accept_payload);
                                        endpoint.set_accept_payload(accept_payload.clone());
                                        ctx.corrman.set_listener_on_entry(
                                            local_correlation_id,
                                            endpoint.clone(),
                                            Some(&name_clone),
                                            &ctx.stream_pool,
                                            idle_timeout
                                        );
                                        execman::send_message(&ctx, originator, correlation, 0, MessageType::StreamAccept, &name_clone, accept_payload);
                                        endpoint.activate();
                                    }
                                    Err(e) => {
                                        execman::send_error(&ctx, originator, correlation, 0, MessageType::StreamError, &name_clone, &e);
                                    }
                                }
                            }
                            entry_clone.release_execution_id(exec_id);
                        }),
                        &name,
                        execution_id
                    );
                } else {
                    eprintln!(
                        "fb.stream.manager.maxreached stream={} corr={} count={}",
                        name,
                        msg.correlation,
                        fe.get_concurrent_count()
                    );
                    execman::send_string(&self.ctx, msg.originator, msg.correlation, 0, MessageType::FunctionUnavailable, &name, "Maximum concurrent functions running");
                }
            }
            None => {
                eprintln!("fb.stream.manager.nofunction stream={}", name);
                execman::send_string(&self.ctx, msg.originator, msg.correlation, 0, MessageType::FunctionUnavailable, &name, "No such function registered in this node");
            }
        }
    }

    pub fn process_service_information_request(&self, msg: &Arc<Message>) {
        let entries = self.entries();
        execman::process_information_request(&self.ctx, &entries, msg);
    }

    pub fn get_function_state_string(&self, node_id: u32) -> String {
        let entries = self.entries();
        execman::function_state_string(&entries, node_id)
    }

    pub fn get_statistics(&self) -> Vec<(String, i32, i32, i32)> {
        execman::collect_statistics(&self.entries())
    }

    pub fn get_status(&self) -> DataMap {
        execman::collect_status(&self.entries())
    }

}

impl std::fmt::Display for StreamManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb = String::new();
        for fe in self.entries() {
            sb.push_str(&format!("{}\r\n", fe.get_name()));
        }
        write!(f, "{}", sb)
    }
}
