// Regenerates the Rust golden fixtures (rust_*.hex) consumed by VectorVerify.java.
// Output is byte-deterministic (fixed ids, BTreeMap metadata order), so the committed
// fixtures in tests/golden/ only change if the serialization itself changes, and
// cargo test (tests/golden.rs) verifies that.

use firebus::core::message::Message;
use firebus::core::payload::Payload;
use firebus::data::datamap::DataMap;
use firebus::data::datalist::DataList;
use firebus::data::DataValue;
use firebus::data::Number;
use std::io::Write;
use std::sync::Arc;

const SNAPSHOT: &str = "777,a,10.0.0.1,1990\r\n\
                        777,f,s,serviceOne\r\n\
                        777,f,t,streamOne\r\n\
                        777,f,c,consumerOne\r\n\
                        777,d,12345,\r\n\
                        777,d,12345,a,10.1.2.3,1990\r\n\
                        777,d,12345,a,10.1.2.4,1991\r\n\
                        777,d,54321,\r\n\
                        777,d,54321,a,192.168.0.9,2000\r\n";

fn write_hex(dir: &str, name: &str, data: &[u8]) {
    let path = format!("{}/tests/golden/{}", dir, name);
    let mut f = std::fs::File::create(&path).unwrap();
    for b in data {
        write!(f, "{:02X}", b).unwrap();
    }
    println!("wrote {}", path);
}

fn main() {
    let dir = std::env::current_dir().unwrap().to_string_lossy().to_string();

    let p_bytes = Payload::from_bytes("raw-bytes-0123456789".as_bytes().to_vec());
    write_hex(&dir, "rust_payload_bytes.hex", &p_bytes.serialize());

    let mut p_str = Payload::from_string("some string payload");
    p_str.set_meta("token", "bebop");
    write_hex(&dir, "rust_payload_string_meta1.hex", &p_str.serialize());

    let mut map = DataMap::new();
    map.put_string("key", "value");
    let mut p_map = Payload::from_datamap(map);
    p_map.set_meta("token", "bebop");
    p_map.set_meta("other", "value2");
    write_hex(&dir, "rust_payload_datamap.hex", &p_map.serialize());

    let mut list = DataList::new();
    list.push(DataValue::Number(Number::Int(1)));
    list.push_string("two");
    list.push(DataValue::Number(Number::Float(3.5)));
    list.push(DataValue::Bool(true));
    write_hex(&dir, "rust_payload_datalist.hex", &Payload::from_datalist(list).serialize());

    write_hex(&dir, "rust_payload_empty.hex", &Payload::empty().serialize());

    let payload = Arc::new(build_map_payload());
    let req = Message {
        version: 5,
        message_id: 777,
        originator: 111,
        destination: 222,
        flags: 0,
        repeat_count: 0,
        repeats_left: 3,
        message_type: firebus::core::message::MessageType::RequestService,
        correlation: 42,
        correlation_seq: 7,
        subject: String::from("service01"),
        payload
    };
    write_hex(&dir, "rust_msg_request.hex", &req.serialize());

    let info_payload = std::sync::Arc::new(Payload::from_bytes(SNAPSHOT.as_bytes().to_vec()));
    let info = Message {
        version: 5,
        message_id: 778,
        originator: 222,
        destination: 111,
        flags: 0,
        repeat_count: 0,
        repeats_left: 10,
        message_type: firebus::core::message::MessageType::NodeInformation,
        correlation: 42,
        correlation_seq: 0,
        subject: String::new(),
        payload: info_payload
    };
    write_hex(&dir, "rust_msg_nodeinfo.hex", &info.serialize());
}

fn build_map_payload() -> Payload {
    let mut map = DataMap::new();
    map.put_string("key", "value");
    let mut p = Payload::from_datamap(map);
    p.set_meta("token", "bebop");
    p.set_meta("other", "value2");
    p
}
