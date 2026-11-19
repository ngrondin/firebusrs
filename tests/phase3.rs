use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use firebus::core::payload::Payload;
use firebus::core::{
    Firebus, FunctionError, FunctionInfoSpec, RequestError, StreamEndpoint, StreamHandler, StreamProvider
};

fn wait_for(mut f: impl FnMut() -> bool, ms: u64) -> bool {
    let start = Instant::now();
    while !f() {
        if start.elapsed() > Duration::from_millis(ms) {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
    true
}

fn connected_pair(net: &str) -> (Firebus, Firebus) {
    let a = Firebus::with_network(net, "firebuspassword0");
    let b = Firebus::with_network(net, "firebuspassword0");
    let cm_a = a.get_node().get_connection_manager().unwrap();
    let cm_b = b.get_node().get_connection_manager().unwrap();
    let pb = cm_b.get_port();
    let bid = b.get_node_id();
    let aid = a.get_node_id();
    a.add_known_node_address("127.0.0.1", pb);
    assert!(
        wait_for(
            || cm_a.has_connection_for_node(bid) && cm_b.has_connection_for_node(aid),
            5000
        ),
        "connections did not come up"
    );
    (a, b)
}

struct Chunks {
    got: Arc<Mutex<Vec<String>>>,
    closed: Arc<Mutex<bool>>
}

impl StreamHandler for Chunks {
    fn receive_stream_data(&self, payload: &Payload) {
        self.got.lock().unwrap().push(payload.get_string());
    }
    fn stream_closed(&self) {
        *self.closed.lock().unwrap() = true;
    }
    fn stream_error(&self, _error: FunctionError) {}
}

// Pushes three ticks as soon as the stream is accepted, then closes.
struct Ticker {
    idle: i64
}

impl StreamProvider for Ticker {
    fn accept_stream(&self, _payload: &Payload, endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError> {
        for i in 1..=3 {
            endpoint.send(Arc::new(Payload::from_string(&format!("tick {}", i))));
        }
        endpoint.close();
        Ok(None)
    }
    fn get_stream_idle_timeout(&self) -> i64 {
        self.idle
    }
    fn get_stream_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

// Keeps the stream open and records what the client sends.
struct Holder {
    chunks: Chunks
}

impl StreamProvider for Holder {
    fn accept_stream(&self, _payload: &Payload, endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError> {
        endpoint.set_handler(Arc::new(Chunks {
            got: Arc::clone(&self.chunks.got),
            closed: Arc::clone(&self.chunks.closed)
        }));
        Ok(None)
    }
    fn get_stream_idle_timeout(&self) -> i64 {
        60000
    }
    fn get_stream_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

struct Failing;

impl StreamProvider for Failing {
    fn accept_stream(&self, _payload: &Payload, _endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError> {
        Err(FunctionError::with_code("provider exploded", 42))
    }
    fn get_stream_idle_timeout(&self) -> i64 {
        30000
    }
    fn get_stream_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

// Provider pushes into the endpoint during accept(); the client must receive
// the chunks (queued pre-activation, buffered pre-handler) and see the close.
#[test]
fn stream_data_flows_provider_to_client() {
    let (a, b) = connected_pair(&format!("fbphase3-p2c-{}", std::process::id()));
    a.register_stream_provider("ticker", Arc::new(Ticker { idle: 30000 }), 10);

    let ep = b
        .request_stream_with_timeout("ticker", &Payload::from_string("go"), None, 8000)
        .expect("request_stream failed");
    assert!(ep.is_active());
    assert!(ep.get_request_payload().map(|p| p.get_string() == "go").unwrap_or(false));

    let chunks = Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) };
    // The close is already in flight by the time the handler is installed; the
    // endpoint must buffer pre-handler messages and replay them.
    ep.set_handler(Arc::new(Chunks {
        got: Arc::clone(&chunks.got),
        closed: Arc::clone(&chunks.closed)
    }));

    assert!(
        wait_for(|| *chunks.closed.lock().unwrap(), 8000),
        "stream never closed: {:?} (ep_closed={})",
        chunks.got.lock().unwrap(),
        ep.is_closed()
    );
    // The stream pool is parallel (like Java's), so chunks may be delivered
    // out of order; both sides must still see every chunk exactly once.
    let mut got: Vec<String> = chunks.got.lock().unwrap().clone();
    got.sort();
    assert_eq!(got, vec!["tick 1".to_string(), "tick 2".to_string(), "tick 3".to_string()]);
    assert!(ep.is_closed());

    a.close();
    b.close();
}

// The client drives data into the provider; the provider's handler on the same
// endpoint receives it, and both sides observe the close.
#[test]
fn stream_data_flows_client_to_provider() {
    let (a, b) = connected_pair(&format!("fbphase3-c2p-{}", std::process::id()));
    let chunks = Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) };
    a.register_stream_provider(
        "pipe",
        Arc::new(Holder {
            chunks: Chunks {
                got: Arc::clone(&chunks.got),
                closed: Arc::clone(&chunks.closed)
            }
        }),
        10
    );

    let ep = b
        .request_stream_with_timeout("pipe", &Payload::from_string("open"), None, 8000)
        .expect("request_stream failed");
    let client_chunks = Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) };
    ep.set_handler(Arc::new(Chunks {
        got: Arc::clone(&client_chunks.got),
        closed: Arc::clone(&client_chunks.closed)
    }));

    ep.send(Arc::new(Payload::from_string("line one")));
    ep.send(Arc::new(Payload::from_string("line two")));
    ep.close();

    assert!(
        wait_for(|| chunks.got.lock().unwrap().len() == 2 && *chunks.closed.lock().unwrap(), 8000),
        "provider never saw the data/close: got={:?} closed={:?}",
        chunks.got.lock().unwrap(),
        *chunks.closed.lock().unwrap()
    );
    let mut got: Vec<String> = chunks.got.lock().unwrap().clone();
    got.sort();
    assert_eq!(got, vec!["line one".to_string(), "line two".to_string()]);
    assert!(
        wait_for(|| *client_chunks.closed.lock().unwrap(), 8000),
        "client endpoint did not report closed"
    );

    a.close();
    b.close();
}

// An error thrown by acceptStream must surface to the requester as a
// FunctionError carrying the transported errorcode.
#[test]
fn stream_accept_error_surfaces_to_requester() {
    let (a, b) = connected_pair(&format!("fbphase3-err-{}", std::process::id()));
    a.register_stream_provider("boom", Arc::new(Failing), 10);

    let err = match b.request_stream_with_timeout("boom", &Payload::from_bytes(vec![1, 2, 3]), None, 8000) {
        Err(e) => e,
        Ok(ep) => panic!("request_stream must fail, got endpoint {}", ep)
    };
    match err {
        RequestError::Service(e) => {
            assert_eq!(e.message, "provider exploded");
            assert_eq!(e.error_code, 42);
        }
        RequestError::Timeout(t) => panic!("expected service error, got timeout: {}", t.message)
    }

    a.close();
    b.close();
}

// A stream's execution slot is held only for the duration of acceptStream
// (mirroring StreamManager.connectStream), so max_concurrent=1 must reject a
// second request while the first accept is still running.
struct SlowHolder {
    chunks: Chunks,
    block_ms: u64
}

impl StreamProvider for SlowHolder {
    fn accept_stream(&self, _payload: &Payload, endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError> {
        thread::sleep(Duration::from_millis(self.block_ms));
        endpoint.set_handler(Arc::new(Chunks {
            got: Arc::clone(&self.chunks.got),
            closed: Arc::clone(&self.chunks.closed)
        }));
        Ok(None)
    }
    fn get_stream_idle_timeout(&self) -> i64 {
        60000
    }
    fn get_stream_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

#[test]
fn stream_max_concurrent_rejects_while_accept_running() {
    let (a, b) = connected_pair(&format!("fbphase3-mc-{}", std::process::id()));
    let chunks = Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) };
    a.register_stream_provider(
        "solo",
        Arc::new(SlowHolder {
            chunks: Chunks {
                got: Arc::clone(&chunks.got),
                closed: Arc::clone(&chunks.closed)
            },
            block_ms: 3000
        }),
        1
    );

    // Request 1 (background thread): blocks in initiate() on the delayed
    // StreamAccept; meanwhile its accept holds the single execution slot.
    let node1 = b.get_node();
    let t1 = thread::spawn(move || {
        firebus::core::StreamRequest::new(node1, "solo", Arc::new(Payload::from_string("one")), None, 10000).initiate()
    });

    // Let request 1 enter its (delayed) accept so the slot is held.
    thread::sleep(Duration::from_millis(800));

    // Request 2 (main thread): every attempt hits FunctionUnavailable until
    // the budget runs out.
    let node2 = b.get_node();
    let err = match firebus::core::StreamRequest::new(node2, "solo", Arc::new(Payload::from_string("two")), None, 2000).initiate() {
        Err(e) => e,
        Ok(ep) => panic!("second stream request must fail while the accept runs, got endpoint {}", ep)
    };
    match err {
        RequestError::Timeout(t) => assert!(t.message.contains("could not be found"), "unexpected message: {}", t.message),
        RequestError::Service(e) => panic!("expected timeout, got service error: {}", e.message)
    }

    // Once the first accept finishes, request 1 succeeds.
    let ep1 = t1.join().unwrap().expect("first stream request should succeed once the slot is free");
    ep1.set_handler(Arc::new(Chunks {
        got: Arc::new(Mutex::new(Vec::new())),
        closed: Arc::new(Mutex::new(false))
    }));
    ep1.close();

    a.close();
    b.close();
}

// Keeps the stream open (no data on either side) with a short idle timeout so
// only the provider-side idle expiry path runs.
struct Quiet {
    provider_chunks: std::sync::Arc<Chunks>
}

impl StreamProvider for Quiet {
    fn accept_stream(&self, _payload: &Payload, endpoint: &StreamEndpoint) -> Result<Option<Payload>, FunctionError> {
        endpoint.set_handler(Arc::new(Chunks {
            got: Arc::clone(&self.provider_chunks.got),
            closed: Arc::clone(&self.provider_chunks.closed)
        }));
        Ok(None)
    }
    fn get_stream_idle_timeout(&self) -> i64 {
        1500
    }
    fn get_stream_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

// A quiet stream must be reaped by the provider-side idle timeout, producing a
// StreamEnd that the client observes as stream_closed.
#[test]
fn stream_idle_timeout_closes_quiet_stream() {
    let (a, b) = connected_pair(&format!("fbphase3-idle-{}", std::process::id()));
    let provider_chunks = Arc::new(Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) });
    let client_chunks = Chunks { got: Arc::new(Mutex::new(Vec::new())), closed: Arc::new(Mutex::new(false)) };
    a.register_stream_provider(
        "quiet",
        Arc::new(Quiet { provider_chunks: Arc::clone(&provider_chunks) }),
        10
    );

    let ep = b
        .request_stream_with_timeout("quiet", &Payload::from_string("hi"), None, 10000)
        .expect("request_stream failed");
    ep.set_handler(Arc::new(Chunks {
        got: Arc::clone(&client_chunks.got),
        closed: Arc::clone(&client_chunks.closed)
    }));

    // Nothing is sent on either side: after the provider's idle window the
    // correlation expires and the stream ends by itself.
    assert!(
        wait_for(|| *provider_chunks.closed.lock().unwrap() && *client_chunks.closed.lock().unwrap(), 10000),
        "idle timeout never closed the stream (provider={:?} client={:?})",
        *provider_chunks.closed.lock().unwrap(),
        *client_chunks.closed.lock().unwrap()
    );
    assert!(provider_chunks.got.lock().unwrap().is_empty());

    a.close();
    b.close();
}
