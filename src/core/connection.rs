use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use std::collections::VecDeque;

use crate::data::datamap::DataMap;
use rand::RngCore;

use super::address::Address;
use super::crypto::Cfb8;
use super::framing::{build_frame, FrameDecoder};
use super::interfaces::ConnectionListener;
use super::message::Message;
use super::threadman::now_millis;

// Mirrors the io.firebus.Connection state constants.
pub const STATE_NEW: i32 = 0;
pub const STATE_INITIALIZING: i32 = 1;
pub const STATE_INITIALIZED: i32 = 2;
pub const STATE_ACTIVE: i32 = 3;
pub const STATE_FAILED: i32 = 4;
pub const STATE_CLOSING: i32 = 5;
pub const STATE_DEAD: i32 = 6;

static NEXT_CONN_ID: AtomicI64 = AtomicI64::new(0);

struct Inner {
    // Kept so close() can tear down the socket from another thread.
    socket: Option<TcpStream>,
    // Install the ciphers at the end of the handshake; the send task takes
    // `enc`, the listen loop takes `dec`.
    enc: Option<Cfb8>,
    dec: Option<Cfb8>
}

// Mirrors io.firebus.Connection: one connection per socket. A listen task
// performs the handshake and then reads frames forever; once initialised a
// second task drains the send queue.
pub struct Connection {
    pub id: i64,
    state: AtomicI32,
    inner: Mutex<Inner>,
    queue: Mutex<VecDeque<Arc<Message>>>,
    queue_cv: Condvar,
    listener: Arc<dyn ConnectionListener>,
    pub local_node_id: u32,
    pub local_port: u32,
    remote_node_id: AtomicU32,
    remote_address: Mutex<Option<Address>>,
    init_time: AtomicI64,
    time_mark: AtomicI64,
    sent_count: AtomicI64,
    sent_bytes: AtomicI64,
    sent_last: AtomicI64,
    recv_count: AtomicI64,
    recv_bytes: AtomicI64,
    recv_last: AtomicI64
}

impl Connection {
    fn constructor_setup(listener: Arc<dyn ConnectionListener>, local_node_id: u32, local_port: u32, remote_address: Option<Address>) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_CONN_ID.fetch_add(1, Ordering::SeqCst),
            state: AtomicI32::new(STATE_NEW),
            inner: Mutex::new(Inner { socket: None, enc: None, dec: None }),
            queue: Mutex::new(VecDeque::new()),
            queue_cv: Condvar::new(),
            listener,
            local_node_id,
            local_port,
            remote_node_id: AtomicU32::new(0),
            remote_address: Mutex::new(remote_address),
            init_time: AtomicI64::new(now_millis()),
            time_mark: AtomicI64::new(now_millis()),
            sent_count: AtomicI64::new(0),
            sent_bytes: AtomicI64::new(0),
            sent_last: AtomicI64::new(0),
            recv_count: AtomicI64::new(0),
            recv_bytes: AtomicI64::new(0),
            recv_last: AtomicI64::new(0)
        })
    }

    // Inbound: the server already accepted this socket.
    pub fn inbound(socket: TcpStream, network: &str, key: [u8; 32], local_node_id: u32, local_port: u32, listener: Arc<dyn ConnectionListener>) -> Arc<Self> {
        let c = Self::constructor_setup(listener, local_node_id, local_port, None);
        c.spawn_listen(Some(socket), network, key);
        c
    }

    // Outbound: the listen task opens the TCP connection.
    pub fn outbound(address: &Address, network: &str, key: [u8; 32], local_node_id: u32, local_port: u32, listener: Arc<dyn ConnectionListener>) -> Arc<Self> {
        let c = Self::constructor_setup(listener, local_node_id, local_port, Some(address.clone()));
        c.spawn_listen(None, network, key);
        c
    }

    fn spawn_listen(self: &Arc<Self>, socket: Option<TcpStream>, network: &str, key: [u8; 32]) {
        let c = Arc::clone(self);
        let net = String::from(network);
        let name = format!("fbConn{}Recv", c.id);
        let _ = std::thread::Builder::new().name(name).spawn(move || c.listen_run(socket, &net, key));
    }

    pub fn get_remote_address(&self) -> Option<Address> {
        self.remote_address.lock().unwrap().clone()
    }

    pub fn remote_address_equals(&self, a: &Address) -> bool {
        self.remote_address.lock().unwrap().as_ref().map(|x| x.equals(a)).unwrap_or(false)
    }

    pub fn get_remote_node_id(&self) -> u32 {
        self.remote_node_id.load(Ordering::SeqCst)
    }

    fn get_state(&self) -> i32 {
        self.state.load(Ordering::SeqCst)
    }

    fn set_state(&self, s: i32) {
        self.state.store(s, Ordering::SeqCst);
    }

    pub fn get_state_as_string(&self) -> &'static str {
        match self.get_state() {
            STATE_NEW => "new",
            STATE_INITIALIZING => "initializing",
            STATE_INITIALIZED => "initialized",
            STATE_ACTIVE => "active",
            STATE_FAILED => "failed",
            STATE_CLOSING => "closing",
            STATE_DEAD => "dead",
            _ => "unknown"
        }
    }

    pub fn is_active(&self) -> bool {
        self.get_state() == STATE_ACTIVE
    }

    // Mirrors Connection.isHealthy.
    pub fn is_healthy(&self) -> bool {
        let state = self.get_state();
        if state == STATE_FAILED || state == STATE_CLOSING || state == STATE_DEAD {
            return false;
        }
        let mark = now_millis() - 10000;
        if state == STATE_ACTIVE && self.queue.lock().unwrap().len() > 2 && self.sent_last.load(Ordering::SeqCst) < mark {
            return false;
        }
        if (state == STATE_INITIALIZING || state == STATE_NEW) && self.init_time.load(Ordering::SeqCst) < mark {
            return false;
        }
        true
    }

    pub fn is_closing(&self) -> bool {
        self.get_state() == STATE_CLOSING
    }

    pub fn is_dead(&self) -> bool {
        self.get_state() == STATE_DEAD
    }

    // Mirrors Connection.getLoad: average sends/sec since the last call,
    // resetting the interval counter.
    pub fn get_load(&self) -> i32 {
        let now = now_millis();
        let delta = now - self.time_mark.load(Ordering::SeqCst);
        let count = self.sent_count.swap(0, Ordering::SeqCst);
        self.time_mark.store(now, Ordering::SeqCst);
        if delta > 0 {
            (1000 * count / delta) as i32
        } else {
            0
        }
    }

    // Mirrors Connection.sendMessage.
    pub fn send_message(&self, msg: Arc<Message>) {
        {
            let mut q = self.queue.lock().unwrap();
            q.push_back(msg);
        }
        self.queue_cv.notify_one();
    }

    // Mirrors Connection.close.
    pub fn close(&self) {
        if self.get_state() != STATE_DEAD {
            eprintln!("fb.connection.closing conn={} state={}", self.id, self.get_state_as_string());
            self.set_state(STATE_CLOSING);
            if let Some(s) = self.inner.lock().unwrap().socket.as_ref() {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            self.queue_cv.notify_all();
        }
    }

    // Mirrors Connection.getStatus.
    pub fn get_status(&self) -> DataMap {
        let now = now_millis();
        let mut status = DataMap::new();
        status.put_int("remoteNodeId", self.get_remote_node_id() as i64);
        if let Some(a) = self.get_remote_address() {
            status.put_string("remoteAddress", &a.to_string());
        }
        status.put_int("startedSince", now - self.init_time.load(Ordering::SeqCst));
        let mut sent = DataMap::new();
        sent.put_int("count", self.sent_count.load(Ordering::SeqCst));
        sent.put_int("bytes", self.sent_bytes.load(Ordering::SeqCst));
        sent.put_int("last", now - self.sent_last.load(Ordering::SeqCst));
        status.put_map("sent", sent);
        let mut recv = DataMap::new();
        recv.put_int("count", self.recv_count.load(Ordering::SeqCst));
        recv.put_int("bytes", self.recv_bytes.load(Ordering::SeqCst));
        recv.put_int("last", now - self.recv_last.load(Ordering::SeqCst));
        status.put_map("recv", recv);
        status.put_string("state", self.get_state_as_string());
        status.put_int("queueDepth", self.queue.lock().unwrap().len() as i64);
        status
    }
}

impl std::fmt::Display for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Connection {} to node {} at {:?}",
            self.id,
            self.get_remote_node_id(),
            self.get_remote_address()
        )
    }
}

// ---------------- listen task (handshake + receive) ----------------

fn outbound_connect(a: &Address) -> std::io::Result<TcpStream> {
    let ip: Ipv4Addr = a.get_ip_address()
        .parse()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not an ipv4 address"))?;
    TcpStream::connect(SocketAddrV4::new(ip, a.get_port() as u16))
}

fn read_exact(reader: &mut impl Read, buf: &mut [u8]) -> Result<(), std::io::Error> {
    let mut off = 0;
    while off < buf.len() {
        let n = reader.read(&mut buf[off..])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof"));
        }
        off += n;
    }
    Ok(())
}

impl Connection {
    // Java's run(): handshake, then (if initialised) ACTIVE + created + send
    // thread + listen; always followed by connectionClosed and DEAD.
    fn listen_run(self: &Arc<Self>, socket: Option<TcpStream>, network: &str, key: [u8; 32]) {
        eprintln!("fb.connection.init conn={} dir={}", self.id, if socket.is_some() { "in" } else { "out" });
        self.set_state(STATE_INITIALIZING);

        // Obtain the socket (outbound connections here) and register it so
        // close() can interrupt the handshake from another thread.
        let socket = match socket {
            Some(s) => Some(s),
            None => {
                let remote = self.remote_address.lock().unwrap().clone();
                match remote {
                    Some(a) => match outbound_connect(&a) {
                        Ok(s) => Some(s),
                        Err(_) => {
                            eprintln!("fb.connection.errorconnecting conn={} remote={}", self.id, a);
                            None
                        }
                    },
                    None => {
                        eprintln!("fb.connection.socketnotconnected conn={}", self.id);
                        None
                    }
                }
            }
        };

        if let Some(mut s) = socket {
            if let Ok(keeper) = s.try_clone() {
                self.inner.lock().unwrap().socket = Some(keeper);
            }
            let _ = s.set_nodelay(true);
            let initialized = self.do_handshake(&mut s, network, key);
            if initialized && self.get_state() == STATE_INITIALIZED {
                self.set_state(STATE_ACTIVE);
                self.listener.connection_created(self);
                match (s.try_clone(), s.try_clone()) {
                    (Ok(rd), Ok(wr)) => {
                        self.inner.lock().unwrap().socket = Some(s);
                        if let Some(enc) = self.inner.lock().unwrap().enc.take() {
                            self.spawn_send(wr, enc);
                        }
                        self.listen_loop(rd);
                    }
                    _ => eprintln!("fb.connection.clone conn={}", self.id)
                }
            } else {
                self.listener.connection_failed(self);
            }
        } else {
            self.listener.connection_failed(self);
        }
        self.listener.connection_closed(self);
        self.set_state(STATE_DEAD);
    }

    fn spawn_send(self: &Arc<Self>, wr: TcpStream, enc: Cfb8) {
        let c = Arc::clone(self);
        let name = format!("fbConn{}Send", c.id);
        let _ = std::thread::Builder::new().name(name).spawn(move || c.send_loop(wr, enc));
    }

    // Java's initialise(): the four-phase handshake. Returns true and leaves
    // the state at STATE_INITIALIZED when the far end is a firebus of the same
    // network and not this very node.
    fn do_handshake(&self, s: &mut TcpStream, network: &str, key: [u8; 32]) -> bool {
        // 1. network names (plaintext)
        let plen = network.len() as u8;
        let mut head = vec![plen];
        head.extend_from_slice(network.as_bytes());
        if s.write_all(&head).is_err() || s.flush().is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        let mut rlen = [0u8; 1];
        if read_exact(s, &mut rlen).is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        let mut rnet = vec![0u8; rlen[0] as usize];
        if read_exact(s, &mut rnet).is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        if String::from_utf8_lossy(&rnet) != network {
            eprintln!("fb.connection.badnetwork conn={}", self.id);
            self.set_state(STATE_FAILED);
            return false;
        }

        // 2. exchange AES IVs (plaintext)
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);
        let mut enc = Cfb8::new(&key, &iv);
        if s.write_all(&iv).is_err() || s.flush().is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        let mut riv = [0u8; 16];
        if read_exact(s, &mut riv).is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        let mut dec = Cfb8::new(&key, &riv);

        // 3. 0xcafebabe through the ciphers
        let check: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBE];
        let mut sent = check.to_vec();
        enc.encrypt(&mut sent);
        if s.write_all(&sent).is_err() || s.flush().is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        let mut rcheck = [0u8; 4];
        if read_exact(s, &mut rcheck).is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        // Decrypt the received ciphertext (as Java's CipherInputStream does)
        // and compare against the constant.
        let mut rcheck_dec = rcheck.to_vec();
        dec.decrypt(&mut rcheck_dec);
        if rcheck_dec.as_slice() != &check[..] {
            eprintln!("fb.connection.badcipher conn={}", self.id);
            self.set_state(STATE_FAILED);
            return false;
        }

        // 4. node identity (encrypted both ways)
        let mut info: Vec<u8> = Vec::new();
        info.extend_from_slice(&self.local_node_id.to_be_bytes());
        match s.local_addr().ok().and_then(|la| match la.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            _ => None
        }) {
            Some(v4) => info.extend_from_slice(&v4.octets()),
            None => {
                eprintln!("fb.connection.errorconnecting conn={} (no ipv4 local addr)", self.id);
                self.set_state(STATE_FAILED);
                return false;
            }
        }
        info.extend_from_slice(&self.local_port.to_be_bytes());
        let mut enc_info = info;
        enc.encrypt(&mut enc_info);
        if s.write_all(&enc_info).is_err() || s.flush().is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }

        let mut rinfo = [0u8; 12];
        if read_exact(s, &mut rinfo).is_err() {
            return self.fail_handshake("fb.connection.errorconnecting");
        }
        dec.decrypt(&mut rinfo);
        let remote_node_id = u32::from_be_bytes([rinfo[0], rinfo[1], rinfo[2], rinfo[3]]);
        self.remote_node_id.store(remote_node_id, Ordering::SeqCst);
        let rip = std::net::Ipv4Addr::new(rinfo[4], rinfo[5], rinfo[6], rinfo[7]);
        let rport = u32::from_be_bytes([rinfo[8], rinfo[9], rinfo[10], rinfo[11]]);
        let advertised = Address::new(&rip.to_string(), rport);

        if self.get_remote_address().is_none() {
            if let Ok(peer) = s.peer_addr() {
                if peer.ip() == std::net::IpAddr::V4(rip) {
                    *self.remote_address.lock().unwrap() = Some(advertised);
                }
            }
        }

        if remote_node_id == self.local_node_id {
            eprintln!("fb.connection.withself conn={} address={:?}", self.id, self.get_remote_address());
            self.set_state(STATE_FAILED);
            return false;
        }

        eprintln!(
            "fb.connection.established conn={} node={} address={:?}",
            self.id,
            remote_node_id,
            self.get_remote_address()
        );
        {
            let mut inner = self.inner.lock().unwrap();
            inner.enc = Some(enc);
            inner.dec = Some(dec);
        }
        self.set_state(STATE_INITIALIZED);
        true
    }

    fn fail_handshake(&self, tag: &str) -> bool {
        eprintln!("{} conn={}", tag, self.id);
        self.set_state(STATE_FAILED);
        false
    }

    fn listen_loop(self: &Arc<Self>, mut rd: TcpStream) {
        let mut dec = match self.inner.lock().unwrap().dec.take() {
            Some(d) => d,
            None => return
        };
        let mut decoder = FrameDecoder::new();
        let mut in_buf = vec![0u8; 1024];
        while self.get_state() == STATE_ACTIVE {
            match rd.read(&mut in_buf) {
                Ok(0) => {
                    eprintln!("fb.connection.receiveddisconnect conn={}", self.id);
                    self.set_state(STATE_FAILED);
                    break;
                }
                Ok(n) => {
                    self.recv_bytes.fetch_add(n as i64, Ordering::SeqCst);
                    dec.decrypt(&mut in_buf[..n]);
                    decoder.feed(&in_buf[..n]);
                    if decoder.bad_crc {
                        eprintln!("fb.connection.badcrc conn={} node={}", self.id, self.get_remote_node_id());
                        self.set_state(STATE_FAILED);
                        break;
                    }
                    while let Some(payload) = decoder.take() {
                        if let Ok(m) = Message::try_deserialize(&payload) {
                            self.receiver_count();
                            self.listener.message_received(Arc::new(m), self);
                        }
                    }
                }
                Err(_) => {
                    if self.get_state() == STATE_ACTIVE {
                        eprintln!("fb.connection.exception conn={}", self.id);
                        self.set_state(STATE_FAILED);
                    }
                    break;
                }
            }
        }
    }

    fn receiver_count(&self) {
        self.recv_count.fetch_add(1, Ordering::SeqCst);
        self.recv_last.store(now_millis(), Ordering::SeqCst);
    }

    // ---------------- send task (Java's send()) ----------------

    fn send_loop(&self, mut wr: TcpStream, mut enc: Cfb8) {
        while self.get_state() == STATE_ACTIVE {
            let msg = {
                let mut q = self.queue.lock().unwrap();
                while q.is_empty() {
                    let (g, _) = match self.queue_cv.wait_timeout(q, Duration::from_millis(500)) {
                        Ok(payload) => payload,
                        Err(poison) => poison.into_inner()
                    };
                    q = g;
                    if self.get_state() != STATE_ACTIVE {
                        return;
                    }
                }
                q.pop_front()
            };
            let Some(msg) = msg else {
                return;
            };
            if self.get_state() != STATE_ACTIVE {
                return;
            }
            let data = msg.serialize();
            let mut frame = build_frame(&data);
            enc.encrypt(&mut frame);
            match wr.write_all(&frame).and(wr.flush()) {
                Ok(()) => {
                    self.sent_count.fetch_add(1, Ordering::SeqCst);
                    self.sent_bytes.fetch_add(frame.len() as i64, Ordering::SeqCst);
                    self.sent_last.store(now_millis(), Ordering::SeqCst);
                }
                Err(_) => {
                    eprintln!("fb.connection.sending conn={}", self.id);
                    self.set_state(STATE_FAILED);
                    return;
                }
            }
        }
    }
}
