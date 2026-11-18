
use std::{error::Error, io::{Read, Write}, sync::{atomic::{AtomicU32, Ordering}, Arc}};

use bytebuffer::{ByteBuffer, ByteReader};

use crate::data::errors::DataError;

use super::payload::Payload;

static NEXT_MSG_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    QueryNode,
    NodeInformation,
    GetFunctionInfo,
    FunctionInfo,
    RequestService,
    RequestServiceAndForget,
    ServiceResponse,
    FunctionUnavailable,
    ServiceError,
    Progress,
    Publish,
    Republish,
    RequestStream,
    StreamAccept,
    StreamError,
    StreamData,
    StreamEnd
}

impl MessageType {
    pub fn get_u8(&self) -> u8 {
        match self {
            MessageType::QueryNode => 1,
            MessageType::NodeInformation => 2,
            MessageType::GetFunctionInfo => 4,
            MessageType::FunctionInfo => 5,
            MessageType::RequestService => 6,
            MessageType::RequestServiceAndForget => 19,
            MessageType::ServiceResponse => 7,
            MessageType::FunctionUnavailable => 8,
            MessageType::ServiceError => 9,
            MessageType::Progress => 10,
            MessageType::Publish => 11,
            MessageType::Republish => 12,
            MessageType::RequestStream => 14,
            MessageType::StreamAccept => 15,
            MessageType::StreamError => 16,
            MessageType::StreamData => 17,
            MessageType::StreamEnd => 18
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => MessageType::QueryNode,
            2 => MessageType::NodeInformation,
            4 => MessageType::GetFunctionInfo,
            5 => MessageType::FunctionInfo,
            6 => MessageType::RequestService,
            19 => MessageType::RequestServiceAndForget,
            7 => MessageType::ServiceResponse,
            8 => MessageType::FunctionUnavailable,
            9 => MessageType::ServiceError,
            10 => MessageType::Progress,
            11 => MessageType::Publish,
            12 => MessageType::Republish,
            14 => MessageType::RequestStream,
            15 => MessageType::StreamAccept,
            16 => MessageType::StreamError,
            17 => MessageType::StreamData,
            18 => MessageType::StreamEnd,
            _ => MessageType::QueryNode
        }
    }


}

pub struct MessageGenerator {
    pub node_id: u32,
    pub next_id: AtomicU32
}

impl MessageGenerator {

    pub fn new(node_id: u32) -> Self {
        Self { node_id, next_id: AtomicU32::new(0) }
    }

    fn next(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn next_message_id(&self) -> u32 {
        self.next()
    }

    pub fn new_message(&self, dest: u32, mt: MessageType, sub: &str, pl: Arc<Payload>) -> Message {
        Message {
            version: 5,
            message_id: self.next(),
            originator: self.node_id,
            destination: dest,
            flags: 0,
            repeat_count: 0,
            repeats_left: 10,
            message_type: mt,
            correlation: 0,
            correlation_seq: 0,
            subject: String::from(sub),
            payload: pl            
        }
    }

    pub fn reply_to(&self, msg: Arc<Message>, mt: MessageType, pl: Arc<Payload>) -> Message {
        Message {
            version: 5,
            message_id: self.next(),
            originator: self.node_id,
            destination: msg.originator,
            flags: 0,
            repeat_count: 0,
            repeats_left: 10,
            message_type: mt,
            correlation: 0,
            correlation_seq: 0,
            subject: msg.subject.clone(),
            payload: pl            
        }
    }

    pub fn repeat(&self, msg: Arc<Message>) -> Message {
        Message {
            version: 5,
            message_id: msg.message_id,
            originator: msg.originator,
            destination: msg.destination,
            flags: msg.flags,
            repeat_count: msg.repeat_count + 1,
            repeats_left: msg.repeats_left - 1,
            message_type: msg.message_type,
            correlation: msg.correlation,
            correlation_seq: msg.correlation_seq,
            subject: msg.subject.clone(),
            payload: msg.payload.clone()            
        }
    }


}

#[derive(Clone)]
pub struct Message {
    pub version: u16,
    pub message_id: u32,
    pub originator: u32,
    pub destination: u32,
    pub flags: u32,
    pub repeat_count: u8,
    pub repeats_left: u8,
    pub message_type: MessageType,
    pub correlation: u32,
    pub correlation_seq: u32,
    pub subject: String,
    pub payload: Arc<Payload>
}

impl Message {

    pub fn serialize(&self) -> Vec<u8> {
        let payloadbytes = self.payload.serialize();
        let subjectbytes = self.subject.as_bytes().to_vec();
        let subjectlen: u32 = subjectbytes.len().try_into().unwrap();
        let mut bb = ByteBuffer::new();
        bb.write_u16(self.version);
        bb.write_u32(self.message_id);
        bb.write_u32(self.destination);
        bb.write_u32(self.originator);
        bb.write_u32(self.flags);
        bb.write_u8(self.repeat_count);
        bb.write_u8(self.repeats_left);
        bb.write_u8(self.message_type.get_u8());
        bb.write_u32(self.correlation);
        bb.write_u32(self.correlation_seq);
        bb.write_u32(subjectlen);
        let _ = bb.write_all(&subjectbytes);
        let _ = bb.write_all(&payloadbytes);
        bb.into_vec()
    }

    pub fn try_deserialize(data: &[u8]) -> Result<Self, Box<dyn Error>> {
        let mut bb = ByteReader::from_bytes(data);
        let version = bb.read_u16()?;
        if version != 5 { return Err( Box::new( DataError { message: String::from("Wrong message version") } ) ) }
        let message_id = bb.read_u32()?;
        let destination = bb.read_u32()?;
        let originator = bb.read_u32()?;
        let flags = bb.read_u32()?;
        let repeat_count = bb.read_u8()?;
        let repeats_left = bb.read_u8()?;
        let message_type = MessageType::from_u8(bb.read_u8()?);
        let correlation = bb.read_u32()?;
        let correlation_seq = bb.read_u32()?;
        let subject_len = bb.read_u32()?.try_into()?;
        let subjectbytes = bb.read_bytes(subject_len)?;
        let subject: String = String::from_utf8(subjectbytes)?;
        let mut payloadbytes: Vec<u8> = Vec::new();
        let _ = bb.read_to_end(&mut payloadbytes);  
        let payload = Payload::deserialize(&payloadbytes)?;

        Ok( Message {version, message_id, destination, originator, flags, repeat_count, repeats_left, message_type, correlation, correlation_seq, subject, payload: Arc::new(payload) } )
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, DataError> {
        match Message::try_deserialize(data) {
            Ok(o) => Ok(o),
            Err(err) => Err( DataError { message: format!("Error deserializing message: {}", err ) })
        }
    }
}