use std::fmt;

// Mirrors io.firebus.Address: an ip (host string) plus port, rendered as "ip:port".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    pub address: String,
    pub port: u32
}

impl Address {
    pub fn new(address: &str, port: u32) -> Self {
        Self { address: String::from(address), port }
    }

    pub fn get_ip_address(&self) -> &str {
        &self.address
    }

    pub fn get_port(&self) -> u32 {
        self.port
    }

    pub fn equals(&self, a: &Address) -> bool {
        self.address == a.address && self.port == a.port
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.address, self.port)
    }
}
