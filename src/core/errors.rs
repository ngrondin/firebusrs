use std::fmt;

// Mirrors io.firebus.exceptions. FunctionError carries the optional errorcode that is
// transported over the wire in payload metadata ("errorcode") and restored by the
// requesting side.

#[derive(Debug, Clone)]
pub struct FunctionError {
    pub message: String,
    pub error_code: i32
}

impl FunctionError {
    pub fn new(message: &str) -> Self {
        Self { message: String::from(message), error_code: 0 }
    }

    pub fn with_code(message: &str, error_code: i32) -> Self {
        Self { message: String::from(message), error_code }
    }
}

impl fmt::Display for FunctionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FunctionError {}

#[derive(Debug, Clone)]
pub struct FunctionTimeoutError {
    pub message: String
}

impl FunctionTimeoutError {
    pub fn new(message: &str) -> Self {
        Self { message: String::from(message) }
    }
}

impl fmt::Display for FunctionTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FunctionTimeoutError {}

#[derive(Debug, Clone)]
pub struct FunctionUnavailableError {
    pub message: String
}

impl FunctionUnavailableError {
    pub fn new(message: &str) -> Self {
        Self { message: String::from(message) }
    }
}

impl fmt::Display for FunctionUnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FunctionUnavailableError {}

#[derive(Debug, Clone)]
pub struct ConnectionError {
    pub message: String
}

impl ConnectionError {
    pub fn new(message: &str) -> Self {
        Self { message: String::from(message) }
    }
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConnectionError {}

// Outcome of a client-side service/stream request: either the service failed
// (FunctionError, optionally with an errorcode transported in metadata) or the
// overall deadline passed (FunctionTimeoutError).
#[derive(Debug, Clone)]
pub enum RequestError {
    Service(FunctionError),
    Timeout(FunctionTimeoutError)
}

impl RequestError {
    pub fn message(&self) -> &str {
        match self {
            RequestError::Service(e) => &e.message,
            RequestError::Timeout(e) => &e.message
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Service(e) => write!(f, "{}", e),
            RequestError::Timeout(e) => write!(f, "{}", e)
        }
    }
}

impl std::error::Error for RequestError {}

impl From<FunctionError> for RequestError {
    fn from(e: FunctionError) -> Self {
        RequestError::Service(e)
    }
}

impl From<FunctionTimeoutError> for RequestError {
    fn from(e: FunctionTimeoutError) -> Self {
        RequestError::Timeout(e)
    }
}

#[derive(Debug, Clone)]
pub struct MessageError {
    pub message: String
}

impl MessageError {
    pub fn new(message: &str) -> Self {
        Self { message: String::from(message) }
    }
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MessageError {}
