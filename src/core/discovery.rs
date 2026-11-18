use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::core::address::Address;

use super::connectionman::local_ip;
use super::nodecore::NodeSelfRef;
use super::threadman::now_millis;

// Mirrors io.firebus.discovery.DefaultDiscoveryAgent. Multicast group and port
// match the Java agent (note the original "anouncement" spelling is part of
// the wire protocol).
pub const DISCOVERY_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
pub const DISCOVERY_PORT: u16 = 1900;

pub fn parse_request(s: &str) -> Option<(u32, String)> {
    let p: Vec<&str> = s.split(' ').collect();
    if p.len() >= 6 && p[0] == "Firebus" && p[1] == "discovery" && p[2] == "request" && p[3] == "from" {
        Some((p[4].parse().ok()?, String::from(p[5])))
    } else {
        None
    }
}

pub fn parse_announcement(s: &str) -> Option<(u32, String, String, u32)> {
    let p: Vec<&str> = s.split(' ').collect();
    if p.len() >= 7 && p[0] == "Firebus" && p[1] == "anouncement" && p[2] == "from" {
        Some((p[3].parse().ok()?, String::from(p[4]), String::from(p[5]), p[6].parse().ok()?))
    } else {
        None
    }
}

pub struct DiscoveryAgent {
    self_ref: NodeSelfRef,
    node_id: u32,
    network: String,
    port: u32,
    last_request_sent: AtomicI64,
    quit: AtomicBool,
    socket: Mutex<Option<UdpSocket>>
}

impl DiscoveryAgent {
    pub fn new(self_ref: NodeSelfRef, node_id: u32, network: &str, port: u32) -> Arc<Self> {
        let mut socket = None;
        let bind_addr: std::net::SocketAddr = std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT));
        match UdpSocket::bind(bind_addr) {
            Ok(s) => {
                // Periodic timeout so the receive loop can observe quit/close.
                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                match s.join_multicast_v4(&DISCOVERY_GROUP, &Ipv4Addr::UNSPECIFIED) {
                    Ok(_) => socket = Some(s),
                    Err(_) => eprintln!("fb.discovery.default.cannotjoin")
                }
            }
            Err(_) => eprintln!("fb.discovery.default bind failed")
        }

        Arc::new(Self {
            self_ref,
            node_id,
            network: String::from(network),
            port,
            last_request_sent: AtomicI64::new(0),
            quit: AtomicBool::new(false),
            socket: Mutex::new(socket)
        })
    }

    // Spawns the agent thread; must run after the wrapping NodeCore exists.
    pub fn start(self: &Arc<Self>) {
        let a = Arc::clone(self);
        let _ = thread::Builder::new().name("fbDefaultDiscoveryAgent".to_string()).spawn(move || {
            a.run();
        });
    }

    fn send_discovery_request(&self) {
        let now = now_millis();
        if now > self.last_request_sent.load(Ordering::SeqCst) + 10000 {
            let msg = format!("Firebus discovery request from {} {}", self.node_id, self.network);
            if let Some(s) = self.socket.lock().unwrap().as_ref().and_then(|s| s.try_clone().ok()) {
                let _ = s.send_to(msg.as_bytes(), DISCOVERY_TARGET);
            }
            self.last_request_sent.store(now, Ordering::SeqCst);
        }
    }

    fn send_announcement(&self) {
        let msg = format!(
            "Firebus anouncement from {} {} {} {}",
            self.node_id,
            self.network,
            local_ip(),
            self.port
        );
        if let Some(s) = self.socket.lock().unwrap().as_ref().and_then(|s| s.try_clone().ok()) {
            let _ = s.send_to(msg.as_bytes(), DISCOVERY_TARGET);
            eprintln!("fb.discovery.default.sent node={} address={}", self.node_id, local_ip());
        }
    }

    fn run(&self) {
        self.send_discovery_request();
        let buf_len = 256;
        while !self.quit.load(Ordering::SeqCst) {
            let s = match self.socket.lock().unwrap().as_ref().and_then(|s| s.try_clone().ok()) {
                Some(s) => s,
                None => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };
            let mut buf = vec![0u8; buf_len];
            let n = s.recv_from(&mut buf).map(|(n, _)| n).unwrap_or(0);
            if self.quit.load(Ordering::SeqCst) {
                break;
            }
            let received = String::from_utf8_lossy(&buf[..n]).trim().to_string();
            if let Some((id, net)) = parse_request(&received) {
                if id != self.node_id && net == self.network {
                    eprintln!("fb.discovery.default.request id={} network={}", id, net);
                    self.send_announcement();
                }
            } else if let Some((id, net, ad, port)) = parse_announcement(&received) {
                if id != self.node_id && net == self.network {
                    eprintln!(
                        "fb.discovery.default.announcement id={} network={} address={}:{}",
                        id, net, ad, port
                    );
                    self.self_ref.get().get_directory().process_discovered_node(id, Some(Address::new(&ad, port)));
                }
            }
        }
    }

    pub fn close(&self) {
        eprintln!("fb.discovery.default.close node={}", self.node_id);
        self.quit.store(true, Ordering::SeqCst);
        self.socket.lock().unwrap().take();
    }
}

impl Drop for DiscoveryAgent {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        self.socket.lock().unwrap().take();
    }
}

pub const DISCOVERY_TARGET: std::net::SocketAddr = std::net::SocketAddr::V4(SocketAddrV4::new(DISCOVERY_GROUP, DISCOVERY_PORT));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_lines() {
        assert_eq!(parse_request("Firebus discovery request from 42 firebus").unwrap(), (42, String::from("firebus")));
        assert!(parse_request("Firebus discovery request from 42").is_none());
        assert!(parse_request("Firebus anouncement from 42 firebus 10.0.0.1 1990").is_none());
        assert!(parse_request("Firebus discovery request from notanum firebus").is_none());
    }

    #[test]
    fn parse_announcement_lines() {
        let (id, net, ad, port) = parse_announcement("Firebus anouncement from 42 firebus 10.0.0.1 1990").unwrap();
        assert_eq!((id, net, ad, port), (42, String::from("firebus"), String::from("10.0.0.1"), 1990));
        assert!(parse_announcement("Firebus anouncement from 42 firebus 10.0.0.1").is_none());
        assert!(parse_announcement("Firebus discovery request from 42 firebus").is_none());
    }
}
