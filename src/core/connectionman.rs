use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::data::datalist::DataList;
use crate::data::datamap::DataMap;

use super::address::Address;
use super::connection::Connection;
use super::connectionserver::ConnectionServer;
use super::interfaces::ConnectionListener;
use super::message::Message;
use super::nodecore::NodeSelfRef;
use super::threadman::now_millis;

// Mirrors io.firebus.information.KnownAddressInformation (retry backoff).
pub struct KnownAddressInfo {
    address: Address,
    failed_count: i32,
    next_try: i64,
    once_connected: bool,
    self_flag: bool
}

impl KnownAddressInfo {
    pub fn new(address: &Address) -> Self {
        Self {
            address: address.clone(),
            failed_count: 0,
            next_try: 0,
            once_connected: false,
            self_flag: false
        }
    }

    pub fn get_address(&self) -> &Address {
        &self.address
    }

    pub fn is_due_to_try(&self) -> bool {
        !self.self_flag && now_millis() > self.next_try
    }

    pub fn is_self(&self) -> bool {
        self.self_flag
    }

    pub fn should_remove(&self) -> bool {
        (self.once_connected && self.failed_count > 30) || (!self.once_connected && self.failed_count > 3)
    }

    pub fn connection_failed(&mut self) {
        self.failed_count += 1;
        let fc = self.failed_count;
        let now = now_millis();
        self.next_try = if fc <= 3 {
            now + 2000
        } else if fc <= 15 {
            now + 2000 * (fc as i64 - 3)
        } else {
            now + 30000
        };
    }

    pub fn connection_succeeded(&mut self) {
        self.failed_count = 0;
        self.once_connected = true;
    }

    pub fn set_as_self(&mut self) {
        self.self_flag = true;
    }
}

impl std::fmt::Display for KnownAddressInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.address)
    }
}

// Best-effort local IPv4 (Java uses InetAddress.getLocalHost()).
pub fn local_ip() -> Ipv4Addr {
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                if let std::net::IpAddr::V4(v4) = a.ip() {
                    return v4;
                }
            }
        }
    }
    Ipv4Addr::LOCALHOST
}

// Mirrors io.firebus.ConnectionManager.
pub struct ConnectionManager {
    pub node_id: u32,
    network_name: String,
    key: [u8; 32],
    server: Arc<ConnectionServer>,
    connections: Mutex<Vec<Arc<Connection>>>,
    known_addresses: Mutex<Vec<KnownAddressInfo>>,
    quit: AtomicBool,
    self_ref: NodeSelfRef
}

impl ConnectionManager {
    pub fn new(node_id: u32, network: &str, key: [u8; 32], port: u32, self_ref: NodeSelfRef) -> Arc<Self> {
        let server = ConnectionServer::bind(port);
        Arc::new(Self {
            node_id,
            network_name: String::from(network),
            key,
            server,
            connections: Mutex::new(Vec::new()),
            known_addresses: Mutex::new(Vec::new()),
            quit: AtomicBool::new(false),
            self_ref
        })
    }

    // Spawns the accept loop (Java's ConnectionServer.run) and the 500ms
    // maintenance loop. Must be called only after the wrapping NodeCore is fully
    // constructed, since the loops dereference NodeSelfRef.
    pub fn start(self: &Arc<Self>) {
        {
            let c = Arc::clone(self);
            let _ = thread::Builder::new().name("fbConnServer".to_string()).spawn(move || {
                c.accept_loop();
            });
        }
        {
            let c = Arc::clone(self);
            let _ = thread::Builder::new().name("fbConnMgr".to_string()).spawn(move || {
                c.maintenance_loop();
            });
        }
    }

    fn accept_loop(self: &Arc<Self>) {
        while !self.quit.load(Ordering::SeqCst) {
            match self.server.accept() {
                Some(stream) => self.socket_received(stream, self.server.get_port()),
                None => {
                    if !self.quit.load(Ordering::SeqCst) {
                        eprintln!("fb.connections.receiving");
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn maintenance_loop(self: &Arc<Self>) {
        while !self.quit.load(Ordering::SeqCst) {
            // 1. known-address book: drop gone addresses, dial due ones.
            let due = {
                let mut kas = self.known_addresses.lock().unwrap();
                for a in kas.iter().filter(|k| k.should_remove()).map(|k| k.get_address().clone()) {
                    eprintln!("fb.connections.maintaining.removeknownaddress address={}", a);
                }
                kas.retain(|k| !k.should_remove());
                kas.iter().filter(|k| k.is_due_to_try()).map(|k| k.get_address().clone()).collect::<Vec<Address>>()
            };
            for addr in due {
                if !self.has_connection_for_address(&addr) {
                    self.create_connection(&addr);
                }
            }

            // 2. keep at least the minimum connected node count.
            let mut connected_node_count = self.get_connected_node_count();
            if connected_node_count < 3 {
                let nc = self.self_ref.get();
                let dir = nc.get_directory();
                let node_count = dir.get_node_count();
                let mut i = 0;
                while i < node_count && connected_node_count < 3 {
                    if let Some(ni) = dir.get_node(i) {
                        let nid = ni.id;
                        if nid != self.node_id && ni.get_address_count() > 0 && !self.has_connection_for_node(nid) {
                            if let Some(a) = ni.get_address(0) {
                                self.create_connection(&a);
                                connected_node_count += 1;
                            }
                        }
                    }
                    i += 1;
                }
            }

            // 3. close unhealthy connections.
            for c in self.connections.lock().unwrap().iter() {
                if !c.is_healthy() && !c.is_closing() {
                    c.close();
                }
            }

            thread::sleep(Duration::from_millis(500));
        }
    }

    // ---------------- event handlers (ConnectionListener impl below) ----------------

    pub fn socket_received(self: &Arc<Self>, stream: std::net::TcpStream, port: u32) {
        let c = Connection::inbound(stream, &self.network_name, self.key, self.node_id, port, Arc::clone(self) as Arc<dyn ConnectionListener>);
        self.connections.lock().unwrap().push(c);
    }

    // ---------------- services ----------------

    // Mirrors addKnownNodeAddress.
    pub fn add_known_node_address(&self, ip: &str, port: u32) {
        let address = Address::new(ip, port);
        let mut kas = self.known_addresses.lock().unwrap();
        if !kas.iter().any(|k| k.get_address().equals(&address)) {
            kas.push(KnownAddressInfo::new(&address));
        }
    }

    // Mirrors sendMessage: destination 0 broadcasts; otherwise route to (or
    // create a connection for) the destination node.
    pub fn send_message(self: &Arc<Self>, msg: &Arc<Message>) {
        let c = if msg.destination != 0 {
            self.get_or_create_connection_for_node(msg.destination)
        } else {
            None
        };
        match c {
            Some(c) => c.send_message(Arc::clone(msg)),
            None => self.broadcast_to_all_connections(msg)
        }
    }

    fn get_or_create_connection_for_node(self: &Arc<Self>, node_id: u32) -> Option<Arc<Connection>> {
        if let Some(c) = self.get_active_connection_for_node_id(node_id) {
            return Some(c);
        }
        {
            let conns = self.connections.lock().unwrap();
            for c in conns.iter() {
                if c.get_remote_node_id() == node_id && c.is_healthy() {
                    return Some(c.clone());
                }
            }
        }

        let nc = self.self_ref.get();
        let dir = nc.get_directory();
        let ni = dir.get_node_by_id(node_id)?;
        // Java quirk kept faithful: the loop creates exactly one connection
        // (to address 0) and returns it immediately.
        if ni.get_address_count() > 0 {
            if let Some(a) = ni.get_address(0) {
                eprintln!("fb.connections.creating.directmessage node={}", node_id);
                return Some(self.create_connection(&a));
            }
        }
        for i in 0..ni.get_repeater_count() {
            if let Some(c) = self.get_active_connection_for_node_id(ni.get_repeater(i)) {
                return Some(c);
            }
        }
        for i in 0..ni.get_repeater_count() {
            if let Some(repeater) = dir.get_node_by_id(ni.get_repeater(i)) {
                for j in 0..repeater.get_address_count() {
                    if let Some(a) = repeater.get_address(j) {
                        eprintln!("fb.connections.creating.repeatermessage node={}", node_id);
                        return Some(self.create_connection(&a));
                    }
                }
            }
        }
        None
    }

    pub fn get_active_connection_for_node_id(&self, node_id: u32) -> Option<Arc<Connection>> {
        for c in self.connections.lock().unwrap().iter() {
            if c.get_remote_node_id() == node_id && c.is_active() {
                return Some(c.clone());
            }
        }
        None
    }

    fn create_connection(self: &Arc<Self>, a: &Address) -> Arc<Connection> {
        let c = Connection::outbound(a, &self.network_name, self.key, self.node_id, self.server.get_port(), Arc::clone(self) as Arc<dyn ConnectionListener>);
        self.connections.lock().unwrap().push(Arc::clone(&c));
        c
    }

    fn broadcast_to_all_connections(&self, msg: &Arc<Message>) {
        let mut sent_to: Vec<u32> = Vec::new();
        for c in self.connections.lock().unwrap().iter() {
            if c.is_healthy() {
                let remote = c.get_remote_node_id();
                if remote != msg.originator && !sent_to.contains(&remote) {
                    c.send_message(Arc::clone(msg));
                    sent_to.push(remote);
                }
            }
        }
    }

    // ---------------- getters ----------------

    pub fn get_port(&self) -> u32 {
        self.server.get_port()
    }

    pub fn get_address(&self) -> Option<Address> {
        Some(Address::new(&local_ip().to_string(), self.server.get_port()))
    }

    pub fn has_connection_for_address(&self, a: &Address) -> bool {
        for c in self.connections.lock().unwrap().iter() {
            if c.remote_address_equals(a) {
                return true;
            }
        }
        false
    }

    pub fn has_connection_for_node(&self, node_id: u32) -> bool {
        for c in self.connections.lock().unwrap().iter() {
            if c.get_remote_node_id() == node_id {
                return true;
            }
        }
        false
    }

    pub fn get_connected_node_count(&self) -> usize {
        let mut ids: Vec<u32> = Vec::new();
        for c in self.connections.lock().unwrap().iter() {
            let id = c.get_remote_node_id();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.len()
    }

    pub fn get_connections(&self) -> Vec<Arc<Connection>> {
        self.connections.lock().unwrap().clone()
    }

    // Mirrors close.
    pub fn close(&self) {
        eprintln!("fb.connections.closing manager node={}", self.node_id);
        self.quit.store(true, Ordering::SeqCst);
        self.server.close();
        for c in self.connections.lock().unwrap().iter().rev() {
            c.close();
        }
    }

    // Mirrors getAddressStateString.
    pub fn get_address_state_string(&self, node_id: u32) -> String {
        match self.get_address() {
            Some(a) => format!("{},a,{},{}\r\n", node_id, a.get_ip_address(), a.get_port()),
            None => String::new()
        }
    }

    // Mirrors getStatus.
    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        let mut conn_map = DataMap::new();
        for c in self.connections.lock().unwrap().iter() {
            conn_map.put_map(&c.id.to_string(), c.get_status());
        }
        status.put_map("connections", conn_map);
        let mut ka_list = DataList::new();
        for a in self.known_addresses.lock().unwrap().iter() {
            ka_list.push_string(&a.to_string());
        }
        status.put_list("knownAddress", ka_list);
        status
    }
}

impl ConnectionListener for ConnectionManager {
    fn connection_created(&self, c: &Arc<Connection>) {
        let nc = self.self_ref.get();
        nc.get_directory().process_discovered_node(c.get_remote_node_id(), c.get_remote_address());
        if let Some(a) = c.get_remote_address() {
            for kai in self.known_addresses.lock().unwrap().iter_mut() {
                if kai.get_address().equals(&a) {
                    kai.connection_succeeded();
                }
            }
        }
    }

    fn connection_failed(&self, c: &Arc<Connection>) {
        if let Some(a) = c.get_remote_address() {
            let nc = self.self_ref.get();
            if let Some(ni) = nc.get_directory().get_node_by_address(&a) {
                eprintln!("fb.connections.removingaddr address={}", a);
                ni.remove_address(&a);
            }
            let with_self = c.get_remote_node_id() == c.local_node_id;
            for kai in self.known_addresses.lock().unwrap().iter_mut() {
                if kai.get_address().equals(&a) {
                    kai.connection_failed();
                    if with_self {
                        kai.set_as_self();
                    }
                }
            }
        }
    }

    fn message_received(&self, m: Arc<Message>, c: &Arc<Connection>) {
        let originator = m.originator;
        let connected = c.get_remote_node_id();
        if originator != self.node_id {
            if connected != originator {
                let nc = self.self_ref.get();
                let ni = nc.get_directory().get_or_create_node_information(originator);
                ni.add_repeater(connected);
            }
            self.self_ref.get().enqueue(m);
        } else {
            eprintln!("fb.connections.blockedself");
        }
    }

    fn connection_closed(&self, c: &Arc<Connection>) {
        self.connections.lock().unwrap().retain(|x| !Arc::ptr_eq(x, c));
    }
}

impl std::fmt::Display for ConnectionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb = String::new();
        for c in self.connections.lock().unwrap().iter() {
            sb.push_str(&format!("{}\r\n", c));
        }
        write!(f, "{}", sb)
    }
}
