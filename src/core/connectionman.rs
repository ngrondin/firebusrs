use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::data::datalist::DataList;
use crate::data::datamap::DataMap;

use super::address::Address;
use super::message::Message;

// Phase 1 stand-in for io.firebus.ConnectionManager: no sockets yet. It keeps
// the address book, the "address state string" contribution to directory
// snapshots and the status surface, so the Phase 1 runtime is self-contained.
// Phase 2 replaces the internals with the real TCP + CFB8-ciphered
// ConnectionManager while preserving this interface.
pub struct ConnectionManager {
    node_id: u32,
    network: String,
    port: u32,
    known_addresses: Mutex<Vec<Address>>,
    quit: AtomicBool
}

impl ConnectionManager {
    pub fn new(node_id: u32, network: &str, port: u32) -> Self {
        Self {
            node_id,
            network: String::from(network),
            port,
            known_addresses: Mutex::new(Vec::new()),
            quit: AtomicBool::new(false)
        }
    }

    pub fn get_port(&self) -> u32 {
        self.port
    }

    pub fn add_known_node_address(&self, address: &str, port: u32) {
        let a = Address::new(address, port);
        let mut known = self.known_addresses.lock().unwrap();
        if !known.iter().any(|x| x.equals(&a)) {
            known.push(a);
        }
    }

    // Phase 1: no transport; outbound messages are simply dropped.
    pub fn send_message(&self, _msg: Arc<Message>) {
        eprintln!("fb.connections.no.transport (phase 1)");
    }

    pub fn get_connected_node_count(&self) -> usize {
        0
    }

    pub fn get_address_state_string(&self, node_id: u32) -> String {
        // Placeholder address so single-node directory snapshots are shaped
        // like the Java ones; Phase 2 uses the real bound endpoint.
        format!("{},a,127.0.0.1,{}\r\n", node_id, self.port)
    }

    pub fn close(&self) {
        self.quit.store(true, Ordering::SeqCst);
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        let conn_map = DataMap::new();
        status.put_map("connections", conn_map);
        let mut ka_list = DataList::new();
        for a in self.known_addresses.lock().unwrap().iter() {
            ka_list.push_string(&format!("{}", a));
        }
        status.put_list("knownAddress", ka_list);
        status
    }
}

impl std::fmt::Display for ConnectionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb = String::from("(phase 1: no transport)\r\n");
        for a in self.known_addresses.lock().unwrap().iter() {
            sb.push_str(&format!("{}\r\n", a));
        }
        write!(f, "{}", sb)
    }
}
