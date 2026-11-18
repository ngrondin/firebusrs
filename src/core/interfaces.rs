use std::sync::Arc;

use super::dirman::FunctionInfoSpec;
use super::errors::FunctionError;
use super::message::Message;
use super::payload::Payload;
use super::streamep::StreamEndpoint;

// SPI contracts mirroring io.firebus.interfaces.

pub trait ServiceProvider: Send + Sync {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError>;
    fn get_service_information(&self) -> Option<FunctionInfoSpec>;
}

pub trait StreamProvider: Send + Sync {
    fn accept_stream(&self, payload: &Payload, endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError>;
    fn get_stream_idle_timeout(&self) -> i64;
    fn get_stream_information(&self) -> Option<FunctionInfoSpec>;
}

pub trait Consumer: Send + Sync {
    fn consume(&self, payload: &Payload);
}

pub trait StreamHandler: Send + Sync {
    fn receive_stream_data(&self, payload: &Payload);
    fn stream_closed(&self);
    fn stream_error(&self, error: FunctionError);
}

pub trait ServiceRequestor: Send + Sync {
    fn response(&self, payload: Arc<Payload>);
    fn error(&self, err: FunctionError);
    fn timeout(&self);
}

pub trait CorrelationListener: Send + Sync {
    fn correlated_response_received(&self, out: Arc<Message>, in_msg: Arc<Message>);
    fn correlation_timedout(&self, out: Arc<Message>);
}
