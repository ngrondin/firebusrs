use std::sync::{Arc, Mutex};

use crate::data::datamap::DataMap;

use super::dirman::FunctionInfo;
use super::execman;
use super::functionentry::{FunctionEntry, FunctionKind};
use super::interfaces::Consumer;
use super::message::Message;
use super::nodecore::NodeContext;

// Mirrors io.firebus.ConsumerManager.
pub struct ConsumerManager {
    ctx: Arc<NodeContext>,
    consumers: Mutex<Vec<Arc<FunctionEntry>>>
}

impl ConsumerManager {
    pub fn new(ctx: Arc<NodeContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            consumers: Mutex::new(Vec::new())
        })
    }

    // Mirrors addConsumer; also registers the function in this node's directory
    // entry (see ServiceManager::add_service for the rationale).
    pub fn add_consumer(&self, name: &str, consumer: Arc<dyn Consumer>, max_concurrent: i32) {
        eprintln!("fb.consumer.manager.add consumer={}", name);
        let entry = FunctionEntry::new(name, FunctionKind::Consumer(consumer), max_concurrent);
        self.consumers.lock().unwrap().push(entry.clone());
        let dir = self.ctx.self_ref.get().get_directory().get_or_create_node_information(self.ctx.node_id);
        dir.add_function_information(
            name,
            FunctionInfo::new(self.ctx.node_id, Some(Arc::clone(&dir.rating)), 'c', name, Default::default())
        );
    }

    // Mirrors removeConsumer (removes every entry with the name).
    pub fn remove_consumer(&self, name: &str) {
        eprintln!("fb.consumer.manager.remove consumer={}", name);
        self.consumers.lock().unwrap().retain(|fe| fe.get_name() != name);
    }

    pub fn has_consumer(&self, name: &str) -> bool {
        self.consumers.lock().unwrap().iter().any(|fe| fe.get_name() == name)
    }

    fn entries(&self) -> Vec<Arc<FunctionEntry>> {
        self.consumers.lock().unwrap().clone()
    }

    // Mirrors consume: every registered consumer with this name runs on the
    // stream pool.
    pub fn consume(&self, msg: &Arc<Message>) {
        for fe in self.entries() {
            if fe.get_name() == msg.subject {
                let in_payload = msg.payload.clone();
                let consumer = fe.get_consumer().cloned();
                let ctx = self.ctx.clone();
                let name = msg.subject.clone();
                if let Some(consumer) = consumer {
                    ctx.send_stream_task(
                        Box::new(move || {
                            consumer.consume(in_payload.as_ref());
                        }),
                        Some(&name),
                        -1
                    );
                }
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

impl std::fmt::Display for ConsumerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb = String::new();
        for fe in self.entries() {
            sb.push_str(&format!("{}\r\n", fe.get_name()));
        }
        write!(f, "{}", sb)
    }
}
