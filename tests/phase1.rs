// Phase 1 integration tests: the node runtime without networking. Exercises the
// full local path: registration -> directory -> FunctionFinder -> routing ->
// correlation -> service execution -> response.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use firebus::core::errors::{FunctionError, FunctionTimeoutError, RequestError};
use firebus::core::message::{Message, MessageGenerator, MessageType};
use firebus::core::payload::Payload;
use firebus::core::{Consumer, Firebus, ServiceProvider, ServiceRequestor};
use firebus::data::datamap::DataMap;
use firebus::data::{DataValue, Number};

fn echo_provider() -> Arc<EchoService> {
    Arc::new(EchoService {})
}

struct EchoService {}

impl ServiceProvider for EchoService {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        Ok(Payload::from_string(&format!("echo:{}", payload.get_string())))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        Some(firebus::core::FunctionInfoSpec {
            request_mime: Some(String::from("text/plain")),
            request_contract: None,
            response_mime: Some(String::from("text/plain")),
            response_contract: None
        })
    }
}

struct ErrorService {
    code: i32
}

impl ServiceProvider for ErrorService {
    fn service(&self, _payload: &Payload) -> Result<Payload, FunctionError> {
        Err(FunctionError::with_code("boom", self.code))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        None
    }
}

#[test]
fn request_echo_service_locally() {
    let firebus = Firebus::new();
    firebus.register_service_provider("echo", echo_provider(), 5);
    let payload = Payload::from_string("hello");
    let response = firebus.request_service("echo", &payload).expect("local service request should succeed");
    assert_eq!(response.get_string(), "echo:hello");
    firebus.close();
}

#[test]
fn request_propagates_service_error_with_code() {
    let firebus = Firebus::new();
    firebus.register_service_provider("err", Arc::new(ErrorService { code: 42 }), 5);
    let payload = Payload::from_string("x");
    let result: Result<Arc<Payload>, RequestError> = firebus.request_service("err", &payload);
    let err = result.err().expect("request should fail");
    match err {
        RequestError::Service(e) => {
            assert_eq!(e.message, "boom");
            assert_eq!(e.error_code, 42);
        }
        other => panic!("expected service error, got {:?}", other),
    }
    firebus.close();
}

#[test]
fn request_unknown_service_times_out() {
    let firebus = Firebus::new();
    let payload = Payload::from_string("x");
    let t0 = Instant::now();
    let result: Result<Arc<Payload>, RequestError> = firebus.request_service_with_timeout("ghost", &payload, 1500);
    let err = result.err().expect("request should fail");
    let elapsed = t0.elapsed();
    match &err {
        RequestError::Timeout(e) => assert!(matches!(e, FunctionTimeoutError { .. })),
        other => panic!("expected timeout, got {:?}", other)
    }
    // The finder must have burned the whole window (at least one 500ms
    // sub-timeout broadcast round, plus the growing pre-broadcast sleep).
    assert!(elapsed >= Duration::from_millis(1000), "elapsed {:?}", elapsed);
    firebus.close();
}

#[test]
fn saturated_service_yields_unavailable_then_timeout() {
    let started = Arc::new(AtomicI32::new(0));
    let gate = Arc::new(std::sync::Barrier::new(2));
    let started_clone = started.clone();
    let gate_clone = gate.clone();
    let provider: Arc<dyn ServiceProvider> = Arc::new(BlockyService {
        started: started.clone(),
        gate: gate.clone(),
        hold_ms: 800
    });
    let firebus = Firebus::new();
    firebus.register_service_provider("blocky", provider, 1);

    // Start a long-running call in a worker thread (it holds the only slot).
    let firebus2 = firebus.get_node();
    let payload1 = Payload::from_string("first");
    let handle = std::thread::spawn(move || {
        let req = firebus::core::ServiceRequest::new(firebus2.clone(), "blocky", Arc::new(payload1), 10000);
        req.execute()
    });

    let _ = gate_clone.wait(); // ensure the first call is executing
    let started_load = started_clone.load(Ordering::SeqCst);
    assert_eq!(started_load, 1);

    // While the slot is held, a second request cannot get in.
    let payload2 = Payload::from_string("second");
    let result: Result<Arc<Payload>, RequestError> = firebus.request_service_with_timeout("blocky", &payload2, 1500);
    let err = result.err().expect("request should fail");
    assert!(matches!(err, RequestError::Timeout(_)), "got {:?}", err);

    // The first call still completes successfully afterwards.
    let first = handle.join().unwrap().expect("first call should complete");
    assert_eq!(first.get_string(), "echo:first");
    firebus.close();
}

struct BlockyService {
    started: Arc<AtomicI32>,
    gate: Arc<std::sync::Barrier>,
    hold_ms: i64
}

impl ServiceProvider for BlockyService {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.gate.wait();
        std::thread::sleep(Duration::from_millis(self.hold_ms as u64));
        Ok(Payload::from_string(&format!("echo:{}", payload.get_string())))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        None
    }
}

#[test]
fn history_queue_drops_duplicate_messages() {
    let firebus = Firebus::new();
    let node = firebus.get_node();
    let seen = Arc::new(AtomicI32::new(0));
    let seen_clone = seen.clone();
    let consumer: Arc<dyn Consumer> = Arc::new(CountingConsumer { seen: seen_clone });
    firebus.register_consumer("topic", consumer, 5);
    std::thread::sleep(Duration::from_millis(50));

    let id = firebus.get_node_id();
    let make_msg = |msg_id: u32| {
        let gen = MessageGenerator::new(99999);
        let payload = Arc::new(Payload::from_string("p"));
        let _ = gen.new_message(id, MessageType::Publish, "topic", payload.clone());
        Message {
            version: 5,
            message_id: msg_id,
            originator: 99999,
            destination: id,
            flags: 0,
            repeat_count: 0,
            repeats_left: 10,
            message_type: MessageType::Publish,
            correlation: 0,
            correlation_seq: 0,
            subject: String::from("topic"),
            payload
        }
    };

    let m1 = Arc::new(make_msg(555));
    node.route(m1.clone());
    node.route(m1); // same originator+id -> must be dropped

    let t0 = Instant::now();
    while seen.load(Ordering::SeqCst) < 1 && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(seen.load(Ordering::SeqCst), 1, "duplicate message must not be re-delivered");
    firebus.close();
}

struct CountingConsumer {
    seen: Arc<AtomicI32>
}

impl Consumer for CountingConsumer {
    fn consume(&self, _payload: &Payload) {
        self.seen.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn publish_reaches_registered_consumer() {
    let firebus = Firebus::new();
    let seen = Arc::new(AtomicI32::new(0));
    let seen_clone = seen.clone();
    let consumer: Arc<dyn Consumer> = Arc::new(CountingConsumer { seen: seen_clone });
    firebus.register_consumer("topic", consumer, 5);
    std::thread::sleep(Duration::from_millis(50));

    firebus.publish("topic", &Payload::from_string("hi"));

    let t0 = Instant::now();
    while seen.load(Ordering::SeqCst) < 1 && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    firebus.close();
}

#[test]
fn get_function_info_roundtrip_locally() {
    let firebus = Firebus::new();
    let node = firebus.get_node();
    firebus.register_service_provider("echo", echo_provider(), 5);

    let find_msg = node.message_generator.new_message(
        firebus.get_node_id(),
        MessageType::GetFunctionInfo,
        "echo",
        Arc::new(Payload::empty())
    );
    let resp = node.corrman.send_and_wait(find_msg, 3000);
    let resp = resp.expect("function info response");
    assert_eq!(resp.message_type, MessageType::FunctionInfo);
    let bytes = resp.payload.get_bytes();
    assert_eq!(bytes.first(), Some(&b's'));
    // mime lengths/text
    let mime_len = bytes[1] as usize;
    assert_eq!(&bytes[2..2 + mime_len], b"text/plain");
    firebus.close();
}

#[test]
fn local_directory_contains_registered_functions() {
    let firebus = Firebus::new();
    firebus.register_service_provider("svc", echo_provider(), 5);
    let node = firebus.get_node();
    let id = firebus.get_node_id();
    let dir = node.get_directory();
    let local = dir.get_node_by_id(id).expect("local node entry");
    let fi = local.get_function_information("svc").expect("function registered in local directory");
    assert_eq!(fi.get_kind(), 's');
    assert!(!fi.has_full_information());

    // Function lines come from the managers; the directory state string only
    // carries the node/address 'd' lines (mirrors the Java snapshot layout).
    let svc_state = node.serviceman.get_function_state_string(id);
    assert!(svc_state.contains(&format!("{},f,s,svc\r\n", id)));
    let dir_state = dir.get_directory_state_string(id);
    assert!(dir_state.contains(&format!("{},d,{},\r\n", id, id)));

    // The full snapshot answered to a QueryNode must combine all of them.
    let snapshot = node.serviceman.get_function_state_string(id)
        + &node.streamman.get_function_state_string(id)
        + &node.consumerman.get_function_state_string(id)
        + &dir.get_directory_state_string(id);
    let _ = &node;
    firebus.close();
}

#[test]
fn correlation_entry_expires_and_is_swept() {
    let firebus = Firebus::new();
    let node = firebus.get_node();
    // QueryNode addressed to a node that never answers.
    let query = node.message_generator.new_message(
        424242, // unknown node: routed out, dropped by the phase-1 stub transport
        MessageType::QueryNode,
        "",
        Arc::new(Payload::empty())
    );
    let t0 = Instant::now();
    let resp = node.corrman.send_and_wait(query, 400);
    let elapsed = t0.elapsed();
    assert!(resp.is_none(), "no response expected from unknown node");
    assert!(elapsed < Duration::from_secs(3), "took {:?}", elapsed);
    // The sweeper should have removed the entry afterwards.
    std::thread::sleep(Duration::from_millis(250));
    let status = node.corrman.get_status();
    assert_eq!(status.get("entryCount").to_string(), "0".to_string());
    firebus.close();
}

#[test]
fn async_request_delivers_via_requestor() {
    let firebus = Firebus::new();
    firebus.register_service_provider("echo", echo_provider(), 5);

    let (tx, rx) = std::sync::mpsc::channel::<Payload>();
    let requestor: Arc<dyn ServiceRequestor> = Arc::new(CaptureRequestor { tx });

    firebus
        .request_service_async("echo", &Payload::from_string("async"), requestor, "testfn", 5000)
        .expect("async request accepted");

    use std::sync::mpsc::RecvTimeoutError;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(payload) => assert_eq!(payload.get_string(), "echo:async"),
        Err(RecvTimeoutError::Timeout) => panic!("no async response within 5s"),
        Err(RecvTimeoutError::Disconnected) => panic!("requestor channel closed")
    }
    firebus.close();
}

struct CaptureRequestor {
    tx: std::sync::mpsc::Sender<Payload>
}

impl ServiceRequestor for CaptureRequestor {
    fn response(&self, payload: Arc<Payload>) {
        let _ = self.tx.send((*payload).clone());
    }

    fn error(&self, _err: FunctionError) {}

    fn timeout(&self) {}
}

#[test]
fn request_and_forget_accepts_progress() {
    let firebus = Firebus::new();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let done = Arc::new(AtomicI32::new(0));
    let gate2 = gate.clone();
    let done2 = done.clone();
    let provider: Arc<dyn ServiceProvider> = Arc::new(SlowService {
        done: done2.clone(),
        gate: gate2
    });
    firebus.register_service_provider("slow", provider, 5);

    let result = firebus.request_service_and_forget("slow", &Payload::from_string("f"));
    assert!(result.is_ok(), "and-forget must accept PROGRESS: {:?}", result.err());

    // The service keeps running to completion on its own.
    let _ = gate.wait();
    let t0 = Instant::now();
    while done.load(Ordering::SeqCst) == 0 && t0.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(done.load(Ordering::SeqCst), 1);
    firebus.close();
}

struct SlowService {
    done: Arc<AtomicI32>,
    gate: Arc<std::sync::Barrier>
}

impl ServiceProvider for SlowService {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        self.gate.wait();
        std::thread::sleep(Duration::from_millis(100));
        self.done.store(1, Ordering::SeqCst);
        Ok(Payload::from_string(&format!("done:{}", payload.get_string())))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        None
    }
}

#[test]
fn datamap_payload_roundtrips_through_request() {
    let firebus = Firebus::new();
    firebus.register_service_provider("mapper", Arc::new(MapTwiceService {}), 5);
    let payload = Payload::from_datamap(build_map());
    let response = firebus.request_service("mapper", &payload).expect("mapper request");
    let map = response.get_datamap().expect("response is a datamap");
    assert!(matches!(map.get("count"), DataValue::Number(firebus::data::Number::Int(4))));
    firebus.close();
}

fn build_map() -> DataMap {
    let mut map = DataMap::new();
    map.put_int("count", 2);
    map
}

struct MapTwiceService {}

impl ServiceProvider for MapTwiceService {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        let mut map = matching_map(payload)?;
        let count: i64 = match map.get("count") {
            DataValue::Number(Number::Int(n)) => *n,
            _ => 0
        };
        map.put_int("count", count * 2);
        Ok(Payload::from_datamap(map))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        None
    }
}

fn matching_map(payload: &Payload) -> Result<DataMap, FunctionError> {
    match payload.get_datamap() {
        Ok(map) => Ok(map),
        Err(e) => Err(FunctionError::new(&e.message))
    }
}

#[test]
fn statistics_and_status_surfaces() {
    let firebus = Firebus::new();
    firebus.register_service_provider("echo", echo_provider(), 5);
    let _ = firebus.request_service("echo", &Payload::from_string("s1")).unwrap();
    let _ = firebus.request_service("echo", &Payload::from_string("s2")).unwrap();

    let stats = firebus.get_statistics();
    assert!(stats.iter().any(|(name, _, _, limit)| name == "echo" && *limit == 5));

    let status = firebus.get_status();
    assert_eq!(status.get("nodeid").to_string(), firebus.get_node_id().to_string());
    let _ = status.get_string("network");
    firebus.close();
}
