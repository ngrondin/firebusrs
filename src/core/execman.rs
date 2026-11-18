use std::sync::Arc;

use crate::data::datamap::DataMap;

use super::dirman::FunctionInfo;
use super::errors::FunctionError;
use super::functionentry::FunctionEntry;
use super::message::MessageType;
use super::nodecore::NodeContext;
use super::payload::Payload;

// Shared helpers mirroring io.firebus.ExecutionManager (the abstract base of
// ServiceManager/StreamManager/ConsumerManager).

// Mirrors ExecutionManager.sendMessage: build the reply and hand it to the
// router via the messaging pool.
pub fn send_message(ctx: &NodeContext, dest: u32, corr: u32, corr_seq: u32, msg_type: MessageType, subject: &str, payload: Arc<Payload>) {
    let mut out = ctx.message_generator.new_message(dest, msg_type, subject, payload);
    out.correlation = corr;
    out.correlation_seq = corr_seq;
    ctx.enqueue(Arc::new(out));
}

pub fn send_string(ctx: &NodeContext, dest: u32, corr: u32, corr_seq: u32, msg_type: MessageType, subject: &str, body: &str) {
    send_message(ctx, dest, corr, corr_seq, msg_type, subject, Arc::new(Payload::from_string(body)));
}

// Mirrors ExecutionManager.sendError: plain-text payload plus the optional
// "errorcode" metadata entry (skipped when the code is 0).
pub fn send_error(ctx: &NodeContext, dest: u32, corr: u32, corr_seq: u32, msg_type: MessageType, subject: &str, err: &FunctionError) {
    let mut payload = Payload::from_string(&err.message);
    if err.error_code != 0 {
        payload.set_meta("errorcode", &err.error_code.to_string());
    }
    send_message(ctx, dest, corr, corr_seq, msg_type, subject, Arc::new(payload));
}

// Mirrors ExecutionManager.getFunctionStateString: "id,f,<s|t|c>,name\r\n" per entry.
pub fn function_state_string(entries: &[Arc<FunctionEntry>], node_id: u32) -> String {
    let mut sb = String::new();
    for fe in entries.iter() {
        sb.push_str(&format!("{},f,{},{}\r\n", node_id, fe.get_kind().name_char(), fe.get_name()));
    }
    sb
}

// Mirrors ExecutionManager.processServiceInformationRequest.
pub fn process_information_request(ctx: &NodeContext, entries: &[Arc<FunctionEntry>], msg: &Arc<super::message::Message>) {
    let name = msg.subject.as_str();
    if let Some(fe) = entries.iter().find(|e| e.get_name() == name) {
        let payload = match fe.get_kind() {
            super::functionentry::FunctionKind::Service(s) => {
                let spec = (*s).get_service_information();
                Arc::new(Payload::from_bytes(FunctionInfo::standalone('s', name, spec).serialise()))
            }
            super::functionentry::FunctionKind::Stream(s) => {
                let spec = (*s).get_stream_information();
                Arc::new(Payload::from_bytes(FunctionInfo::standalone('t', name, spec).serialise()))
            }
            // Java replies with a null payload for consumers.
            super::functionentry::FunctionKind::Consumer(_) => Arc::new(Payload::empty())
        };
        send_message(ctx, msg.originator, msg.correlation, 0, MessageType::FunctionInfo, name, payload);
    }
}

pub fn collect_statistics(entries: &[Arc<FunctionEntry>]) -> Vec<(String, i32, i32, i32)> {
    entries.iter().map(|fe| fe.get_statistics()).collect()
}

pub fn collect_status(entries: &[Arc<FunctionEntry>]) -> DataMap {
    let mut status = DataMap::new();
    for fe in entries.iter() {
        status.put_map(fe.get_name(), fe.get_status());
    }
    status
}
