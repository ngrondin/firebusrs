// Node-side modules of the Firebus runtime (port of the io.firebus core
// subproject). See NodeCore for the composition root.

mod address;
mod consumerman;
mod connection;
mod connectionman;
mod connectionserver;
mod correlation;
pub mod crypto;
mod dirman;
mod discovery;
pub mod errors;
mod execman;
pub mod framing;
mod functionentry;
mod functionfinder;
mod historyqueue;
mod interfaces;
pub mod message;
mod nodecore;
pub mod payload;
mod request;
mod serviceman;
mod streamep;
mod streamman;
mod threadman;

pub use address::Address;
pub use consumerman::ConsumerManager;
pub use connection::Connection;
pub use connectionman::ConnectionManager;
pub use correlation::CorrelationManager;
pub use dirman::{Directory, FunctionInfo, FunctionInfoSpec, NodeInfo};
pub use errors::{
    ConnectionError, FunctionError, FunctionTimeoutError, FunctionUnavailableError, MessageError, RequestError
};
pub use functionentry::FunctionEntry;
pub use functionfinder::FunctionFinder;
pub use historyqueue::HistoryQueue;
pub use interfaces::{
    Consumer, ConnectionListener, CorrelationListener, ServiceProvider, ServiceRequestor, StreamHandler,
    StreamProvider
};
pub use nodecore::{NodeContext, NodeCore, NodeSelfRef};
pub use request::{ServiceRequest, ServiceRequestAsync};
pub use serviceman::ServiceManager;
pub use streamep::StreamEndpoint;
pub use streamman::StreamManager;
pub use threadman::ThreadManager;

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::data::datamap::DataMap;

use self::payload::Payload;

// The user-facing facade (mirrors io.firebus.Firebus).
pub struct Firebus {
    node: Arc<NodeCore>,
    default_timeout: AtomicI32
}

impl Firebus {
    pub fn new() -> Self {
        Self::with(0, "firebus", "firebuspassword0")
    }

    pub fn with_network(network: &str, password: &str) -> Self {
        Self::with(0, network, password)
    }

    pub fn with(port: u32, network: &str, password: &str) -> Self {
        Self {
            node: NodeCore::new(port, network, password),
            default_timeout: AtomicI32::new(10000)
        }
    }

    pub fn get_node(&self) -> Arc<NodeCore> {
        self.node.clone()
    }

    pub fn get_node_id(&self) -> u32 {
        self.node.get_node_id()
    }

    pub fn set_default_timeout(&self, ms: i32) {
        self.default_timeout.store(ms, Ordering::SeqCst);
    }

    pub fn default_timeout(&self) -> i32 {
        self.default_timeout.load(Ordering::SeqCst)
    }

    pub fn set_messaging_thread_count(&self, min: i32, max: i32) {
        self.node.set_message_thread_count(min, max);
    }

    pub fn set_service_thread_count(&self, min: i32, max: i32) {
        self.node.set_service_thread_count(min, max);
    }

    pub fn set_stream_thread_count(&self, min: i32, max: i32) {
        self.node.set_stream_thread_count(min, max);
    }

    pub fn add_known_node_address(&self, address: &str, port: u32) {
        self.node.add_known_node_address(address, port);
    }

    pub fn register_service_provider(&self, service_name: &str, provider: Arc<dyn ServiceProvider>, max_concurrent: i32) {
        self.node.serviceman.add_service(service_name, provider, max_concurrent);
    }

    pub fn deregister_service_provider(&self, service_name: &str) {
        self.node.serviceman.remove_service(service_name);
    }

    pub fn register_stream_provider(&self, stream_name: &str, provider: Arc<dyn StreamProvider>, max_concurrent: i32) {
        self.node.streamman.add_stream(stream_name, provider, max_concurrent);
    }

    pub fn deregister_stream_provider(&self, stream_name: &str) {
        self.node.streamman.remove_stream(stream_name);
    }

    pub fn register_consumer(&self, consumer_name: &str, consumer: Arc<dyn Consumer>, max_concurrent: i32) {
        self.node.consumerman.add_consumer(consumer_name, consumer, max_concurrent);
    }

    pub fn deregister_consumer(&self, consumer_name: &str) {
        self.node.consumerman.remove_consumer(consumer_name);
    }

    pub fn has_registered_function(&self, name: &str) -> bool {
        self.node.serviceman.has_service(name)
            || self.node.streamman.has_stream(name)
            || self.node.consumerman.has_consumer(name)
    }

    // Mirrors Firebus.getNodeInformation: QueryNode round-trip (needs transport
    // for remote nodes; answers itself in Phase 1).
    pub fn get_node_information(&self, node_id: u32) -> Option<Arc<NodeInfo>> {
        let query_msg = self.node.message_generator.new_message(
            node_id,
            crate::core::message::MessageType::QueryNode,
            "",
            Arc::new(crate::core::payload::Payload::empty())
        );
        let resp = self.node.corrman.send_and_wait(query_msg, 2000);
        if resp.is_some() {
            self.node.get_directory().get_node_by_id(node_id)
        } else {
            None
        }
    }

    pub fn request_service(&self, service_name: &str, payload: &Payload) -> Result<Arc<Payload>, RequestError> {
        self.request_service_with_timeout(service_name, payload, self.default_timeout())
    }

    pub fn request_service_with_timeout(&self, service_name: &str, payload: &Payload, timeout_ms: i32) -> Result<Arc<Payload>, RequestError> {
        let request = ServiceRequest::new(self.node.clone(), service_name, Arc::new(payload.clone()), timeout_ms);
        request.execute()
    }

    pub fn request_service_and_forget(&self, service_name: &str, payload: &Payload) -> Result<(), RequestError> {
        let request = ServiceRequest::new(self.node.clone(), service_name, Arc::new(payload.clone()), -1);
        request.execute().map(|_| ())
    }

    pub fn request_service_async(
        &self,
        service_name: &str,
        payload: &Payload,
        requestor: Arc<dyn ServiceRequestor>,
        requestor_function_name: &str,
        timeout_ms: i32
    ) -> Result<(), RequestError> {
        let request = ServiceRequestAsync::new(
            self.node.clone(),
            service_name,
            Arc::new(payload.clone()),
            requestor,
            requestor_function_name,
            timeout_ms
        );
        request.execute()
    }

    // Phase 1: streams carry no endpoint plumbing yet beyond the endpoint type;
    // request_stream lands in Phase 3.
    pub fn publish(&self, data_name: &str, payload: &Payload) {
        let msg = self.node.message_generator.new_message(
            0,
            crate::core::message::MessageType::Publish,
            data_name,
            Arc::new(payload.clone())
        );
        self.node.enqueue(Arc::new(msg));
    }

    pub fn run_adhoc(&self, run: Box<dyn std::ops::FnOnce() + Send>) {
        self.node.run_adhoc(run);
    }

    pub fn get_statistics(&self) -> Vec<(String, i32, i32, i32)> {
        self.node.get_statistics()
    }

    pub fn get_status(&self) -> DataMap {
        self.node.get_status()
    }

    pub fn close(&self) {
        self.node.close();
    }
}

impl Default for Firebus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Firebus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.node)
    }
}
