// StreamEndpoint: a bidirectional data channel carried over correlation entries
// (mirrors io.firebus.StreamEndpoint).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::errors::FunctionError;
use super::interfaces::{CorrelationListener, StreamHandler};
use super::message::{Message, MessageType};
use super::nodecore::NodeContext;
use super::payload::Payload;

struct PendingSend {
    msg_type: MessageType,
    payload: Arc<Payload>
}

pub struct StreamEndpoint {
    ctx: Arc<NodeContext>,
    handler: Mutex<Option<Arc<dyn StreamHandler>>>,
    node_id: u32,
    stream_name: String,
    local_correlation_id: i32,
    remote_correlation_id: i32,
    remote_correlation_sequence: AtomicI32Like,
    remote_node_id: u32,
    pre_handler_in: Mutex<Vec<Arc<Message>>>,
    pre_active_out: Mutex<VecDeque<PendingSend>>,
    request_payload: Mutex<Option<Arc<Payload>>>,
    accept_payload: Mutex<Option<Arc<Payload>>>,
    active: AtomicBool,
    closed: AtomicBool
}

type AtomicI32Like = std::sync::atomic::AtomicI32;

impl StreamEndpoint {
    pub fn new(ctx: Arc<NodeContext>, stream_name: &str, local_corr: i32, remote_corr: i32, remote_corr_seq: i32, remote_node_id: u32) -> Arc<Self> {
        let node_id = ctx.node_id;
        Arc::new(Self {
            ctx,
            handler: Mutex::new(None),
            node_id,
            stream_name: String::from(stream_name),
            local_correlation_id: local_corr,
            remote_correlation_id: remote_corr,
            remote_correlation_sequence: AtomicI32Like::new(remote_corr_seq),
            remote_node_id,
            pre_handler_in: Mutex::new(Vec::new()),
            pre_active_out: Mutex::new(VecDeque::new()),
            request_payload: Mutex::new(None),
            accept_payload: Mutex::new(None),
            active: AtomicBool::new(false),
            closed: AtomicBool::new(false)
        })
    }

    pub fn set_request_payload(&self, p: Arc<Payload>) {
        *self.request_payload.lock().unwrap() = Some(p);
    }

    pub fn get_request_payload(&self) -> Option<Arc<Payload>> {
        self.request_payload.lock().unwrap().clone()
    }

    pub fn set_accept_payload(&self, p: Arc<Payload>) {
        *self.accept_payload.lock().unwrap() = Some(p);
    }

    pub fn get_accept_payload(&self) -> Option<Arc<Payload>> {
        self.accept_payload.lock().unwrap().clone()
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    // Mirrors activate: flushes outbound messages queued before activation.
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
        let pending: Vec<PendingSend> = self.pre_active_out.lock().unwrap().drain(..).collect();
        for p in pending {
            let mut msg = self.build_outbound(p.msg_type, p.payload);
            self.apply_remote_correlation(&mut msg);
            let out = Arc::new(msg);
            self.ctx.enqueue(out);
            if p.msg_type == MessageType::StreamEnd || p.msg_type == MessageType::StreamError {
                // The closing message may be queued before activation; perform
                // the disconnect once it is on its way.
                self._disconnect();
            }
        }
    }

    pub fn set_handler(&self, handler: Arc<dyn StreamHandler>) {
        // Java's synchronized setHandler + non-reentrant flush: replay buffered
        // messages without holding the handler lock (a replayed StreamEnd
        // re-locks it via _disconnect).
        let pending: Vec<Arc<Message>>;
        {
            *self.handler.lock().unwrap() = Some(handler.clone());
            pending = self.pre_handler_in.lock().unwrap().drain(..).collect();
        }
        for msg in pending {
            self._process_inbound(&handler, msg);
        }
    }

    // Mirrors _send (Java can mutate the Message in place; Rust rebuilds it with
    // a fresh, unique id from the node's generator).
    fn _send(&self, msg_type: MessageType, payload: Arc<Payload>) {
        if self.active.load(Ordering::SeqCst) {
            let mut msg = self.build_outbound(msg_type, payload);
            self.apply_remote_correlation(&mut msg);
            let out = Arc::new(msg);
            self.ctx.enqueue(out);
            if msg_type == MessageType::StreamEnd || msg_type == MessageType::StreamError {
                self._disconnect();
            }
        } else {
            self.pre_active_out.lock().unwrap().push_back(PendingSend { msg_type, payload });
        }
    }

    pub fn send(&self, payload: Arc<Payload>) {
        if !self.is_closed() {
            self._send(MessageType::StreamData, payload);
        }
    }

    pub fn close(&self) {
        if !self.is_closed() {
            self._send(MessageType::StreamEnd, Arc::new(Payload::from_bytes(Vec::new())));
        }
    }

    pub fn error(&self, error: &FunctionError) {
        if !self.is_closed() {
            let mut payload = Payload::from_string(&error.message);
            if error.error_code != 0 {
                payload.set_meta("errorcode", &error.error_code.to_string());
            }
            self._send(MessageType::StreamError, Arc::new(payload));
        }
    }

    fn build_outbound(&self, msg_type: MessageType, payload: Arc<Payload>) -> Message {
        Message {
            version: 5,
            message_id: self.ctx.message_generator.next_message_id(),
            originator: self.node_id,
            destination: self.remote_node_id,
            flags: 0,
            repeat_count: 0,
            repeats_left: 10,
            message_type: msg_type,
            correlation: 0,
            correlation_seq: 0,
            subject: String::from(&self.stream_name),
            payload
        }
    }

    fn apply_remote_correlation(&self, msg: &mut Message) {
        let seq = self.remote_correlation_sequence.fetch_add(1, Ordering::SeqCst);
        msg.correlation = self.remote_correlation_id as u32;
        msg.correlation_seq = seq as u32;
    }

    fn _process_inbound(&self, handler: &Arc<dyn StreamHandler>, in_msg: Arc<Message>) {
        match in_msg.message_type {
            MessageType::StreamData => handler.receive_stream_data(in_msg.payload.as_ref()),
            MessageType::StreamEnd => self._disconnect(),
            MessageType::StreamError => {
                let error_message = in_msg.payload.get_string();
                let error_code: i32 = in_msg.payload.get_meta("errorcode").parse().unwrap_or(0);
                handler.stream_error(FunctionError::with_code(&error_message, error_code));
                self._disconnect();
            }
            _ => {}
        }
    }

    fn _disconnect(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.ctx.corrman.remove_entry(self.local_correlation_id);
        if let Some(handler) = self.handler.lock().unwrap().as_ref() {
            handler.stream_closed();
        }
    }
}

impl CorrelationListener for StreamEndpoint {
    fn correlated_response_received(&self, _out: Option<Arc<Message>>, in_msg: Arc<Message>) {
        // Clone the handler out of the lock: processing may re-enter _disconnect,
        // which takes the handler lock again (Java's synchronized methods are
        // reentrant; std::sync::Mutex is not).
        let handler = self.handler.lock().unwrap().clone();
        match handler {
            Some(handler) => self._process_inbound(&handler, in_msg),
            None => self.pre_handler_in.lock().unwrap().push(in_msg)
        }
    }

    fn correlation_timedout(&self, _out: Option<Arc<Message>>) {
        self.close();
    }
}

impl std::fmt::Display for StreamEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{} -> {}.{}",
            self.remote_node_id,
            self.remote_correlation_id,
            self.node_id,
            self.local_correlation_id
        )
    }
}
