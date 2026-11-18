use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};

use crate::data::datamap::DataMap;

use super::connectionman::ConnectionManager;
use super::consumerman::ConsumerManager;
use super::correlation::{CorrHandles, CorrelationManager};
use super::dirman::Directory;
use super::historyqueue::HistoryQueue;
use super::message::{Message, MessageGenerator, MessageType};
use super::payload::Payload;
use super::streamman::StreamManager;
use super::threadman::ThreadManager;
use rand::Rng;

// Deferred pointer to the fully constructed NodeCore. Java passes `this`
// (NodeCore) into all managers at construction; Rust cannot hold an Arc to a
// struct before it exists, so managers hold this reference which is filled in
// as the last step of NodeCore construction.
#[derive(Clone)]
pub struct NodeSelfRef {
    inner: Arc<RwLock<Option<Arc<NodeCore>>>>
}

impl NodeSelfRef {
    pub fn empty() -> Self {
        Self { inner: Arc::new(RwLock::new(None)) }
    }

    pub fn init(&self, nc: Arc<NodeCore>) {
        *self.inner.write().unwrap() = Some(nc);
    }

    pub fn get(&self) -> Arc<NodeCore> {
        self.inner.read().unwrap().clone().expect("NodeCore not initialised")
    }
}

// The set of node handles available to all managers (mirrors the NodeCore
// parameters Java hands to every manager constructor).
#[derive(Clone)]
pub struct NodeContext {
    pub node_id: u32,
    pub corrman: Arc<CorrelationManager>,
    pub msg_pool: Arc<ThreadManager>,
    pub svc_pool: Arc<ThreadManager>,
    pub stream_pool: Arc<ThreadManager>,
    pub adhoc_pool: Arc<ThreadManager>,
    pub self_ref: NodeSelfRef,
    pub message_generator: Arc<MessageGenerator>
}

impl NodeContext {
    // Mirrors NodeCore.enqueue: hand the message to the messaging pool for routing.
    pub fn enqueue(&self, msg: Arc<Message>) {
        let ctx = self.clone();
        self.msg_pool.enqueue(Box::new(move || {
            let nc = ctx.self_ref.get();
            nc.route(msg);
        }), Some("msg"), -1);
    }

    pub fn send_service_task(&self, run: Box<dyn std::ops::FnOnce() + Send>, function_name: &str, execution_id: i64) {
        self.svc_pool.enqueue(run, Some(function_name), execution_id);
    }

    pub fn send_stream_task(&self, run: Box<dyn std::ops::FnOnce() + Send>, function_name: Option<&str>, execution_id: i64) {
        self.stream_pool.enqueue(run, function_name, execution_id);
    }

    pub fn send_adhoc_task(&self, run: Box<dyn std::ops::FnOnce() + Send>, execution_id: i64) {
        self.adhoc_pool.enqueue(run, None, execution_id);
    }
}

pub struct NodeCore {
    pub node_id: u32,
    pub network_name: String,
    pub directory: Arc<Directory>,
    pub corrman: Arc<CorrelationManager>,
    pub serviceman: Arc<super::serviceman::ServiceManager>,
    pub streamman: Arc<StreamManager>,
    pub consumerman: Arc<ConsumerManager>,
    pub connman: Mutex<Option<Arc<ConnectionManager>>>,
    pub message_threads: Arc<ThreadManager>,
    pub service_threads: Arc<ThreadManager>,
    pub stream_threads: Arc<ThreadManager>,
    pub adhoc_threads: Arc<ThreadManager>,
    pub history: Arc<HistoryQueue>,
    pub ctx: Arc<NodeContext>,
    pub message_generator: Arc<MessageGenerator>,
    pub quit: std::sync::atomic::AtomicBool
}

impl NodeCore {
    pub fn new(port: u32, network: &str, password: &str) -> Arc<Self> {
        let _ = (port, password);
        Self::initialise(port, network, password)
    }

    fn initialise(port: u32, network: &str, _password: &str) -> Arc<Self> {
        let node_id: u32 = {
            let mut r = rand::thread_rng();
            r.gen::<u32>() & 0x7fffffff
        };

        // Java: ThreadManager(nodeCore, min, max, priority, name, defaultTimeout, warningTime)
        let message_threads = ThreadManager::new("Mesg", 10, 20, 10000, 1000);
        let service_threads = ThreadManager::new("Service", 10, 20, 70000, 10000);
        let stream_threads = ThreadManager::new("Stream", 10, 20, 10000, 1000);
        let adhoc_threads = ThreadManager::new("adhoc", 2, 2, 70000, 10000);

        let self_ref = NodeSelfRef::empty();
        let handles = CorrHandles {
            node_id,
            self_ref: self_ref.clone(),
            msg_pool: message_threads.clone(),
            svc_pool: service_threads.clone(),
            stream_pool: stream_threads.clone()
        };
        let corrman = CorrelationManager::new(handles);

        let directory = Directory::new();
        directory.get_or_create_node_information(node_id);

        let ctx = Arc::new(NodeContext {
            node_id,
            corrman: corrman.clone(),
            msg_pool: message_threads.clone(),
            svc_pool: service_threads.clone(),
            stream_pool: stream_threads.clone(),
            adhoc_pool: adhoc_threads.clone(),
            self_ref: self_ref.clone(),
            message_generator: Arc::new(MessageGenerator::new(node_id))
        });

        let serviceman = super::serviceman::ServiceManager::new(ctx.clone());
        let streamman = StreamManager::new(ctx.clone());
        let consumerman = ConsumerManager::new(ctx.clone());

        // Phase 1: no sockets yet; the stub keeps the address book + status
        // surface so the facade and diagnostics work. Phase 2 replaces it with
        // the real TCP/ciphered ConnectionManager.
        let connman = Arc::new(ConnectionManager::new(node_id, network, port));
        let message_generator = ctx.message_generator.clone();

        let nc = Arc::new(Self {
            node_id,
            network_name: String::from(network),
            directory: directory.clone(),
            corrman,
            serviceman,
            streamman,
            consumerman,
            connman: Mutex::new(Some(connman)),
            message_threads,
            service_threads,
            stream_threads,
            adhoc_threads,
            history: Arc::new(HistoryQueue::new(256)),
            ctx,
            message_generator,
            quit: std::sync::atomic::AtomicBool::new(false)
        });
        self_ref.init(nc.clone());
        nc
    }

    pub fn get_node_id(&self) -> u32 {
        self.node_id
    }

    pub fn get_network_name(&self) -> &str {
        &self.network_name
    }

    pub fn get_directory(&self) -> Arc<Directory> {
        self.directory.clone()
    }

    pub fn get_correlation_manager(&self) -> Arc<CorrelationManager> {
        self.corrman.clone()
    }

    pub fn get_connection_manager(&self) -> Option<Arc<ConnectionManager>> {
        self.connman.lock().unwrap().clone()
    }

    // Mirrors NodeCore.enqueue.
    pub fn enqueue(&self, msg: Arc<Message>) {
        self.ctx.enqueue(msg);
    }

    // Mirrors NodeCore.route: dedup, then process locally and/or forward.
    pub fn route(&self, msg: Arc<Message>) {
        let uid = ((msg.originator as u64) << 32) | (msg.message_id as u64);
        if !self.history.check(uid) {
            return;
        }
        let dest = msg.destination;
        if dest == self.node_id || dest == 0 {
            self.process(msg.clone());
        }
        if dest != self.node_id || dest == 0 {
            if let Some(cm) = self.get_connection_manager() {
                cm.send_message(msg);
            }
        }
    }

    // Mirrors NodeCore.process: dispatch by message type.
    pub fn process(&self, msg: Arc<Message>) {
        if msg.destination != 0 && msg.destination != self.node_id {
            return;
        }
        match msg.message_type {
            MessageType::QueryNode => self.process_node_information_request(msg),
            MessageType::NodeInformation => {
                self.directory.process_node_information(&msg.payload.get_string());
                self.corrman.receive_response(&msg);
            }
            MessageType::GetFunctionInfo => {
                let name = &msg.subject;
                if self.serviceman.has_service(name) {
                    self.serviceman.process_service_information_request(&msg);
                } else if self.streamman.has_stream(name) {
                    self.streamman.process_service_information_request(&msg);
                } else if self.consumerman.has_consumer(name) {
                    self.consumerman.process_service_information_request(&msg);
                }
            }
            MessageType::FunctionInfo => {
                self.directory.process_function_information(msg.originator, &msg.subject, &msg.payload.get_bytes());
                self.corrman.receive_response(&msg);
            }
            MessageType::RequestService | MessageType::RequestServiceAndForget => self.serviceman.execute_service(msg),
            MessageType::Progress
            | MessageType::ServiceResponse
            | MessageType::ServiceError
            | MessageType::FunctionUnavailable
            | MessageType::StreamAccept
            | MessageType::StreamError
            | MessageType::StreamData
            | MessageType::StreamEnd => self.corrman.receive_response(&msg),
            MessageType::Publish => {
                if self.consumerman.has_consumer(&msg.subject) {
                    self.consumerman.consume(&msg);
                }
            }
            MessageType::RequestStream => self.streamman.connect_stream(msg),
            _ => {}
        }
    }

    // Mirrors NodeCore.processNodeInformationRequest.
    pub fn process_node_information_request(&self, req: Arc<Message>) {
        let mut sb = String::new();
        if let Some(cm) = self.get_connection_manager() {
            sb.push_str(&cm.get_address_state_string(self.node_id));
        }
        sb.push_str(&self.serviceman.get_function_state_string(self.node_id));
        sb.push_str(&self.streamman.get_function_state_string(self.node_id));
        sb.push_str(&self.consumerman.get_function_state_string(self.node_id));
        sb.push_str(&self.directory.get_directory_state_string(self.node_id));
        let payload = Arc::new(Payload::from_bytes(sb.into_bytes()));
        let mut reply = self.message_generator.new_message(req.originator, MessageType::NodeInformation, "", payload);
        reply.correlation = req.correlation;
        reply.correlation_seq = 0;
        self.route(Arc::new(reply));
    }

    pub fn set_message_thread_count(&self, min: i32, max: i32) {
        self.message_threads.set_thread_count(min, max);
    }

    pub fn set_service_thread_count(&self, min: i32, max: i32) {
        self.service_threads.set_thread_count(min, max);
    }

    pub fn set_stream_thread_count(&self, min: i32, max: i32) {
        self.stream_threads.set_thread_count(min, max);
    }

    pub fn add_known_node_address(&self, address: &str, port: u32) {
        if let Some(cm) = self.get_connection_manager() {
            cm.add_known_node_address(address, port);
        }
    }

    // Mirrors Firebus.runAdhoc.
    pub fn run_adhoc(&self, run: Box<dyn std::ops::FnOnce() + Send>) {
        self.ctx.send_adhoc_task(run, 10000);
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
        if let Some(cm) = self.get_connection_manager() {
            cm.close();
        }
        self.corrman.close();
        self.message_threads.close();
        self.service_threads.close();
        self.stream_threads.close();
        self.adhoc_threads.close();
    }

    pub fn get_statistics(&self) -> Vec<(String, i32, i32, i32)> {
        let mut list = self.serviceman.get_statistics();
        list.extend(self.streamman.get_statistics());
        list
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        status.put_int("nodeid", self.node_id as i64);
        status.put_string("network", &self.network_name);
        status.put_map("directory", self.directory.get_status());
        status.put_map("correlation", self.corrman.get_status());
        let mut funcs = DataMap::new();
        funcs.put_map("services", self.serviceman.get_status());
        funcs.put_map("streams", self.streamman.get_status());
        funcs.put_map("consumers", self.consumerman.get_status());
        status.put_map("functions", funcs);
        let mut threads = DataMap::new();
        threads.put_map("messaging", self.message_threads.get_status());
        threads.put_map("services", self.service_threads.get_status());
        threads.put_map("streams", self.stream_threads.get_status());
        threads.put_map("adhoc", self.adhoc_threads.get_status());
        status.put_map("threads", threads);
        if let Some(cm) = self.get_connection_manager() {
            status.put_map("connmgr", cm.get_status());
        }
        status
    }
}

impl std::fmt::Display for NodeCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = format!("Node Id : {}\r\n", self.node_id)
            + "-------Functions----------\r\n"
            + &self.serviceman.to_string()
            + &self.streamman.to_string()
            + &self.consumerman.to_string()
            + "-------Connections--------\r\n"
            + &self.connman.lock().unwrap().as_ref().map(|cm| cm.to_string()).unwrap_or_default()
            + "-------Directory----------\r\n"
            + &self.directory.to_string()
            + "\r\n";
        write!(f, "{}", s)
    }
}
