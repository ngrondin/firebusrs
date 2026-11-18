use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::data::datamap::DataMap;

use super::execman;
use super::functionentry::{FunctionEntry, FunctionKind};
use super::interfaces::ServiceProvider;
use super::message::{Message, MessageType};
use super::nodecore::NodeContext;
use super::payload::Payload;

// Mirrors io.firebus.ServiceManager.
pub struct ServiceManager {
    ctx: Arc<NodeContext>,
    services: Mutex<HashMap<String, Arc<FunctionEntry>>>
}

impl ServiceManager {
    pub fn new(ctx: Arc<NodeContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            services: Mutex::new(HashMap::new())
        })
    }

    // Mirrors addService; also registers the function in this node's directory
    // entry, matching the Java steady state where the local node learns its
    // own functions via the discovery loop (which Phase 1 does not run yet).
    pub fn add_service(&self, name: &str, provider: Arc<dyn ServiceProvider>, max_concurrent: i32) {
        eprintln!("fb.service.manager.add service={}", name);
        let mut services = self.services.lock().unwrap();
        if !services.contains_key(name) {
            let entry = FunctionEntry::new(name, FunctionKind::Service(provider.clone()), max_concurrent);
            services.insert(String::from(name), entry);
        }
        drop(services);
        let spec = (*provider).get_service_information().unwrap_or_default();
        let dir = self.get_local_node();
        dir.add_function_information(
            name,
            super::dirman::FunctionInfo::new(self.ctx.node_id, Some(Arc::clone(&dir.rating)), 's', name, spec)
        );
    }

    pub fn remove_service(&self, name: &str) {
        eprintln!("fb.service.manager.remove service={}", name);
        self.services.lock().unwrap().remove(name);
    }

    pub fn has_service(&self, name: &str) -> bool {
        self.services.lock().unwrap().contains_key(name)
    }

    fn get_local_node(&self) -> Arc<super::dirman::NodeInfo> {
        // The directory is owned by the node; reach it through the node itself
        // to avoid a second Directory instance.
        self.ctx.self_ref.get().get_directory().get_or_create_node_information(self.ctx.node_id)
    }

    fn get_function_entry(&self, name: &str) -> Option<Arc<FunctionEntry>> {
        self.services.lock().unwrap().get(name).cloned()
    }

    fn entries(&self) -> Vec<Arc<FunctionEntry>> {
        self.services.lock().unwrap().values().cloned().collect()
    }

    // Mirrors executeService.
    pub fn execute_service(&self, msg: Arc<Message>) {
        let name = msg.subject.clone();
        let in_payload = msg.payload.clone();
        match self.get_function_entry(&name) {
            Some(fe) => {
                let execution_id = fe.get_execution_id();
                if execution_id != -1 {
                    execman::send_message(&self.ctx, msg.originator, msg.correlation, 0, MessageType::Progress, &name, Arc::new(Payload::empty()));
                    let provider = fe.get_service_provider().cloned();
                    let is_request_service = msg.message_type == MessageType::RequestService;
                    let originator = msg.originator;
                    let correlation = msg.correlation;
                    let ctx = self.ctx.clone();
                    let entry_clone = fe.clone();
                    let name_clone = name.clone();
                    let exec_id = execution_id;
                    self.ctx.send_service_task(
                        Box::new(move || {
                            if let Some(provider) = provider {
                                match provider.service(in_payload.as_ref()) {
                                    Ok(return_payload) => {
                                        if is_request_service {
                                            execman::send_message(&ctx, originator, correlation, 1, MessageType::ServiceResponse, &name_clone, Arc::new(return_payload));
                                        }
                                    }
                                    Err(e) => {
                                        if is_request_service {
                                            execman::send_error(&ctx, originator, correlation, 1, MessageType::ServiceError, &name_clone, &e);
                                        }
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
                        "fb.service.manager.maxreached service={} corr={} count={}",
                        name,
                        msg.correlation,
                        fe.get_concurrent_count()
                    );
                    execman::send_string(&self.ctx, msg.originator, msg.correlation, 0, MessageType::FunctionUnavailable, &name, "Maximum concurrent functions running");
                }
            }
            None => {
                eprintln!("fb.service.manager.nofunction service={}", name);
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

impl std::fmt::Display for ServiceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb = String::new();
        for fe in self.entries() {
            sb.push_str(&format!("{}\r\n", fe.get_name()));
        }
        write!(f, "{}", sb)
    }
}
