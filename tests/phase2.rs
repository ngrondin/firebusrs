use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use firebus::core::payload::Payload;
use firebus::core::{Consumer, FunctionInfoSpec, FunctionError, ServiceProvider};

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

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/golden/{}", env!("CARGO_MANIFEST_DIR"), name);
    hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap()
}

// AES-256/CFB8 must match JCE exactly: these vectors were produced by the real
// Java stack (VectorGen.java).
#[test]
fn cfb8_vectors_match_java() {
    use firebus::core::crypto::{Cfb8, derive_key};
    let plaintext = fixture("java_cfb8_plaintext.hex");
    let cases = [
        ("firebuspassword0", "java_cfb8_iv1.hex", "java_cfb8_cipher1.hex"),
        ("firebuspassword0", "java_cfb8_iv2.hex", "java_cfb8_cipher2.hex"),
        ("anotherpassword123", "java_cfb8_iv1.hex", "java_cfb8_cipher3.hex"),
    ];
    for (pw, iv_name, ct_name) in cases {
        let key = derive_key(pw);
        let ivv = fixture(iv_name);
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&ivv);
        let mut ct = plaintext.clone();
        Cfb8::new(&key, &iv).encrypt(&mut ct);
        assert_eq!(ct, fixture(ct_name), "cfb8 encrypt mismatch for {}", ct_name);
        let mut pt = ct.clone();
        Cfb8::new(&key, &iv).decrypt(&mut pt);
        assert_eq!(pt, plaintext, "cfb8 decrypt mismatch for {}", ct_name);
    }
}

struct Echo;

impl ServiceProvider for Echo {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        Ok(Payload::from_string(&format!("echo:{}", payload.get_string())))
    }
    fn get_service_information(&self) -> Option<FunctionInfoSpec> {
        None
    }
}

struct Events {
    got: Arc<Mutex<Vec<String>>>
}

impl Consumer for Events {
    fn consume(&self, payload: &Payload) {
        self.got.lock().unwrap().push(payload.get_string());
    }
}

#[test]
fn two_nodes_exchange_over_tcp() {
    let net = "fbphase2";
    let a = firebus::core::Firebus::with_network(net, "firebuspassword0");
    let b = firebus::core::Firebus::with_network(net, "firebuspassword0");

    let cm_a = a.get_node().get_connection_manager().unwrap();
    let cm_b = b.get_node().get_connection_manager().unwrap();
    let pa = cm_a.get_port();
    let pb = cm_b.get_port();
    assert_ne!(pa, pb, "nodes must bind different ports");

    b.register_service_provider("echo", Arc::new(Echo), 10);
    let got = Arc::new(Mutex::new(Vec::new()));
    b.register_consumer("events", Arc::new(Events { got: Arc::clone(&got) }), 10);

    let bid = b.get_node_id();
    let aid = a.get_node_id();
    a.add_known_node_address("127.0.0.1", pb);

    // The 500ms maintenance loop dials the known address; the handshake brings
    // bidirectional connections up.
    assert!(
        wait_for(
            || cm_a.has_connection_for_node(bid) && cm_b.has_connection_for_node(aid),
            5000
        ),
        "connections did not come up (a-ports {} {})",
        pa,
        pb
    );

    // QueryNode round trip: A learns B's functions from B's directory snapshot.
    let info = a.get_node_information(bid).expect("node information request failed");
    assert!(
        info.get_function_information("echo").is_some(),
        "B's snapshot must list the echo service"
    );

    // Service request A -> B -> A (the request and the response travel as
    // framed, CFB8-encrypted messages over TCP).
    let resp = a
        .request_service("echo", &Payload::from_string("hello-over-tcp"))
        .expect("service request over tcp failed");
    assert_eq!(resp.get_string(), "echo:hello-over-tcp");

    // Publish A -> B (destination 0: fan-out broadcast to connected peers).
    a.publish("events", &Payload::from_string("ping"));
    assert!(
        wait_for(|| !got.lock().unwrap().is_empty(), 5000),
        "published message never reached B's consumer"
    );
    assert_eq!(got.lock().unwrap()[0], "ping");

    // B's directory must have seen A (processDiscoveredNode at connect time).
    assert!(
        b.get_node()
            .get_directory()
            .get_node_by_id(aid)
            .map(|ni| ni.get_address_count() > 0)
            .unwrap_or(false),
        "B's directory should hold A's address"
    );

    a.close();
    b.close();
}

#[test]
fn different_networks_do_not_connect() {
    let n1 = format!("fbnet-{}", std::process::id());
    let n2 = format!("fbnet-{}", std::process::id() + 1);
    let a = firebus::core::Firebus::with_network(&n1, "firebuspassword0");
    let b = firebus::core::Firebus::with_network(&n2, "firebuspassword0");

    let cm_a = a.get_node().get_connection_manager().unwrap();
    let pb = b.get_node().get_connection_manager().unwrap().get_port();
    a.add_known_node_address("127.0.0.1", pb);

    // Wait past a full maintenance cycle + handshake window: the handshake must
    // fail with badnetwork, so no usable connection ever becomes active.
    thread::sleep(Duration::from_millis(1500));
    let conns = cm_a.get_connections();
    assert!(
        conns.iter().all(|c| !c.is_active()),
        "expected no active connection across networks, got {:?}",
        conns.iter().map(|c| c.get_state_as_string()).collect::<Vec<_>>()
    );

    a.close();
    b.close();
}

// The history queue must still suppress rebroadcast loops when a full mesh is
// up: publishing on A must deliver exactly once to B's consumer even though the
// message bounces around the loop via the originator-exclusion rules.
#[test]
fn publish_delivers_once_to_connected_peer() {
    let net = format!("fbphase2-{}", std::process::id());
    let a = firebus::core::Firebus::with_network(&net, "firebuspassword0");
    let b = firebus::core::Firebus::with_network(&net, "firebuspassword0");

    let pb = b.get_node().get_connection_manager().unwrap().get_port();
    let bid = b.get_node_id();
    a.add_known_node_address("127.0.0.1", pb);

    let got = Arc::new(Mutex::new(Vec::new()));
    b.register_consumer("once", Arc::new(Events { got: Arc::clone(&got) }), 10);

    assert!(
        wait_for(
            || a.get_node().get_connection_manager().unwrap().has_connection_for_node(bid),
            5000
        ),
        "connection did not come up"
    );

    // B must learn the consumer from A's point of view? No - consumers live on
    // B itself, so B's own directory knows about 'once' locally and the
    // publish's destination-0 broadcast reaches B's consumer exactly once.
    a.publish("once", &Payload::from_string("x1"));
    assert!(wait_for(|| got.lock().unwrap().len() == 1, 5000));
    thread::sleep(Duration::from_millis(500)); // let any echo deliverables settle
    assert_eq!(got.lock().unwrap().len(), 1, "duplicate delivery: {:?}", got.lock().unwrap());

    a.close();
    b.close();
}
