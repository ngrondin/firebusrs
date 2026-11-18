// Golden-vector tests: byte-exact fixtures produced by the ORIGINAL Java implementation
// (see tools/golden/VectorGen.java + gen.sh) are deserialized here with the Rust code and
// asserted to be logically identical. This proves Rust can READ Java's wire format.
//
// The reverse direction (Java reading Rust's byte-exact output) is covered by
// tools/golden/VectorVerify.java, asserted against tests/golden/rust_*.hex fixtures.

use firebus::core::message::{Message, MessageType};
use firebus::core::payload::Payload;
use firebus::data::DataValue;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/golden/{}", env!("CARGO_MANIFEST_DIR"), name);
    let hexstr = std::fs::read_to_string(path).unwrap();
    hex::decode(hexstr.trim()).unwrap()
}

fn build_map_payload() -> Payload {
    use firebus::data::datamap::DataMap;
    let mut map = DataMap::new();
    map.put_string("key", "value");
    let mut p = Payload::from_datamap(map);
    p.set_meta("token", "bebop");
    p.set_meta("other", "value2");
    p
}

#[test]
fn payload_bytes_from_java() {
    let p = Payload::deserialize(&fixture("java_payload_bytes.hex")).unwrap();
    assert_eq!(p.get_type(), 1);
    assert_eq!(p.get_bytes(), "raw-bytes-0123456789".as_bytes());
    assert_eq!(p.get_meta("token"), "");
}

#[test]
fn payload_string_with_metadata_from_java() {
    let p = Payload::deserialize(&fixture("java_payload_string_meta1.hex")).unwrap();
    assert_eq!(p.get_type(), 2);
    assert_eq!(p.get_string(), "some string payload");
    assert_eq!(p.get_meta("token"), "bebop");
}

#[test]
fn payload_datamap_with_metadata_from_java() {
    let p = Payload::deserialize(&fixture("java_payload_datamap.hex")).unwrap();
    assert_eq!(p.get_type(), 3);
    let map = p.get_datamap().unwrap();
    assert!(matches!(map.get("key"), DataValue::String(s) if s == "value"));
    assert_eq!(p.get_meta("token"), "bebop");
    assert_eq!(p.get_meta("other"), "value2");
}

#[test]
fn payload_datalist_from_java() {
    let p = Payload::deserialize(&fixture("java_payload_datalist.hex")).unwrap();
    assert_eq!(p.get_type(), 4);
    let list = p.get_datalist().unwrap();
    assert_eq!(list.len(), 4);
}

#[test]
fn payload_empty_from_java() {
    let p = Payload::deserialize(&fixture("java_payload_empty.hex")).unwrap();
    assert_eq!(p.get_type(), 0);
    assert!(p.get_bytes().is_empty());
}

#[test]
fn message_request_from_java() {
    let m = Message::try_deserialize(&fixture("java_msg_request.hex")).unwrap();
    assert_eq!(m.version, 5);
    assert_eq!(m.destination, 222);
    assert_eq!(m.originator, 111);
    assert_eq!(m.message_type, MessageType::RequestService);
    assert_eq!(m.subject, "service01");
    assert_eq!(m.correlation, 42);
    assert_eq!(m.correlation_seq, 7);
    assert_eq!(m.repeats_left, 3);
    let map = m.payload.get_datamap().unwrap();
    assert!(matches!(map.get("key"), DataValue::String(s) if s == "value"));
}

// Round-trip: a Rust-serialized payload/message must be byte-stable through
// deserialize -> serialize, proving the Rust (de)serializer is idempotent.
#[test]
fn rust_payload_roundtrip_byte_stable() {
    for name in [
        "rust_payload_bytes",
        "rust_payload_string_meta1",
        "rust_payload_datamap",
        "rust_payload_datalist",
        "rust_payload_empty",
    ] {
        let bytes = fixture(&format!("{}.hex", name));
        let p = Payload::deserialize(&bytes).unwrap();
        assert_eq!(p.serialize(), bytes, "round-trip unstable for {}", name);
    }
}

#[test]
fn rust_message_roundtrip_byte_stable() {
    for name in ["rust_msg_request", "rust_msg_nodeinfo"] {
        let bytes = fixture(&format!("{}.hex", name));
        let m = Message::try_deserialize(&bytes).unwrap();
        assert_eq!(m.serialize(), bytes, "round-trip unstable for {}", name);
    }
}

// These tests pin Rust's serialized output to the exact byte sequences in tests/golden/rust_*.hex.
// VectorVerify.java parses those fixtures with the Java Message/Payload classes; a drift here
// would silently invalidate the Rust->Java direction until re-verified.

#[test]
fn rust_payload_serialisation_stable() {
    assert_eq!(Payload::from_bytes("raw-bytes-0123456789".as_bytes().to_vec()).serialize(), fixture("rust_payload_bytes.hex"));
    let mut p = Payload::from_string("some string payload");
    p.set_meta("token", "bebop");
    assert_eq!(p.serialize(), fixture("rust_payload_string_meta1.hex"));
    assert_eq!(build_map_payload().serialize(), fixture("rust_payload_datamap.hex"));
    use firebus::data::datalist::DataList;
    use firebus::data::DataValue;
    use firebus::data::Number;
    let mut list = DataList::new();
    list.push(DataValue::Number(Number::Int(1)));
    list.push_string("two");
    list.push(DataValue::Number(Number::Float(3.5)));
    list.push(DataValue::Bool(true));
    assert_eq!(Payload::from_datalist(list).serialize(), fixture("rust_payload_datalist.hex"));
    assert_eq!(Payload::empty().serialize(), fixture("rust_payload_empty.hex"));
}

#[test]
fn rust_message_serialisation_stable() {
    let req = Message {
        version: 5,
        message_id: 777,
        originator: 111,
        destination: 222,
        flags: 0,
        repeat_count: 0,
        repeats_left: 3,
        message_type: MessageType::RequestService,
        correlation: 42,
        correlation_seq: 7,
        subject: String::from("service01"),
        payload: std::sync::Arc::new(build_map_payload())
    };
    assert_eq!(req.serialize(), fixture("rust_msg_request.hex"));
}

#[test]
fn message_nodeinfo_null_subject_from_java() {
    let m = Message::try_deserialize(&fixture("java_msg_nodeinfo.hex")).unwrap();
    assert_eq!(m.version, 5);
    assert_eq!(m.destination, 111);
    assert_eq!(m.originator, 222);
    assert_eq!(m.message_type, MessageType::NodeInformation);
    assert_eq!(m.subject, "");
    assert_eq!(m.correlation, 42);
    assert_eq!(m.correlation_seq, 0);
    // payload carries the directory snapshot text as raw bytes
    let text = String::from_utf8(m.payload.get_bytes()).unwrap();
    assert!(text.contains("777,f,s,serviceOne"));
    assert!(text.contains("777,a,10.0.0.1,1990"));
}
