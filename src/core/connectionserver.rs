use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;

// Mirrors io.firebus.ConnectionServer (the socket side): binds the TCP port,
// probing up from 1990 when 0 is requested. The accept loop itself runs in the
// ConnectionManager so the two can share the Arc without a cycle.
pub struct ConnectionServer {
    listener: Mutex<Option<TcpListener>>,
    port: u32
}

impl ConnectionServer {
    pub fn bind(port: u32) -> Arc<Self> {
        let (port, listener) = if port == 0 {
            let mut p = 1990u32;
            loop {
                match TcpListener::bind(("0.0.0.0", p as u16)) {
                    Ok(l) => break (p, l),
                    Err(_) => p += 1
                }
            }
        } else {
            (port, TcpListener::bind(("0.0.0.0", port as u16)).expect("fb.connections.starting: cannot bind"))
        };
        eprintln!("fb.connections.starting port={}", port);
        Arc::new(Self {
            listener: Mutex::new(Some(listener)),
            port
        })
    }

    pub fn get_port(&self) -> u32 {
        self.port
    }

    pub fn accept(&self) -> Option<TcpStream> {
        self.listener.lock().unwrap().as_ref()?.accept().ok().map(|(s, _)| s)
    }

    pub fn close(&self) {
        eprintln!("fb.connections.closing port={}", self.port);
        // The accept loop holds the lock across the blocking accept(), so a
        // plain take() would deadlock. Wake it up first: a connect to our own
        // port makes accept() return, releasing the lock; the wake connection
        // then simply fails its handshake (peer closed) and is closed by the
        // manager along with the rest.
        let wake = std::net::TcpStream::connect(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(127, 0, 0, 1), self.port as u16)));
        let _ = wake;
        self.listener.lock().unwrap().take();
    }
}

impl Drop for ConnectionServer {
    fn drop(&mut self) {
        // Same wake-then-take as close(): never block on the lock that the
        // accept loop holds across a blocking accept().
        if self.listener.lock().unwrap().is_some() {
            let _ = std::net::TcpStream::connect(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(127, 0, 0, 1), self.port as u16)));
            self.listener.lock().unwrap().take();
        }
    }
}
