use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use std::io::Write;

use bytebuffer::{ByteReader, ByteBuffer};
use rand::Rng;

use crate::data::datalist::DataList;
use crate::data::datamap::DataMap;

use super::address::Address;
use super::threadman::now_millis;

// MIME/contract spec returned by providers (mirrors the user-fillable part of
// the Java *Information classes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FunctionInfoSpec {
    pub request_mime: Option<String>,
    pub request_contract: Option<String>,
    pub response_mime: Option<String>,
    pub response_contract: Option<String>
}

// Directory-side function record (mirrors io.firebus.information.FunctionInformation
// and its Service/Stream/Consumer subclasses). Kinds: 's' service, 't' stream,
// 'c' consumer.
pub struct FunctionInfo {
    pub name: String,
    kind: char,
    spec: Mutex<FunctionInfoSpec>,
    full_information: AtomicBool,
    pub rating: AtomicI32,
    node_id: u32,
    node_rating: Option<Arc<AtomicI32>>
}

impl FunctionInfo {
    pub fn new(node_id: u32, node_rating: Option<Arc<AtomicI32>>, kind: char, name: &str, spec: FunctionInfoSpec) -> Arc<Self> {
        Arc::new(Self {
            name: String::from(name),
            kind,
            spec: Mutex::new(spec),
            full_information: AtomicBool::new(false),
            rating: AtomicI32::new(100),
            node_id,
            node_rating
        })
    }

    // A standalone record (no node back-reference), used when answering
    // GetFunctionInfo with the local provider's spec.
    pub fn standalone(kind: char, name: &str, spec: Option<FunctionInfoSpec>) -> Arc<Self> {
        Arc::new(Self {
            name: String::from(name),
            kind,
            spec: Mutex::new(spec.unwrap_or_default()),
            full_information: AtomicBool::new(true),
            rating: AtomicI32::new(100),
            node_id: 0,
            node_rating: None
        })
    }

    pub fn get_kind(&self) -> char {
        self.kind
    }

    pub fn get_spec(&self) -> FunctionInfoSpec {
        self.spec.lock().unwrap().clone()
    }

    pub fn has_full_information(&self) -> bool {
        self.full_information.load(Ordering::SeqCst)
    }

    // Byte format mirrors ServiceInformation/StreamInformation.serialise():
    // kind char, u8 mime len + mime, i32-BE contract len + contract (x2).
    pub fn serialise(&self) -> Vec<u8> {
        let spec = self.spec.lock().unwrap();
        let mut bb = ByteBuffer::new();
        bb.write_u8(self.kind as u8);
        write_mime(&mut bb, spec.request_mime.as_deref());
        write_contract(&mut bb, spec.request_contract.as_deref());
        write_mime(&mut bb, spec.response_mime.as_deref());
        write_contract(&mut bb, spec.response_contract.as_deref());
        bb.into_vec()
    }

    // Returns false when the buffer is malformed (Java would throw and the
    // message would be dropped by the pool).
    pub fn deserialise(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let mut bb = ByteReader::from_bytes(bytes);
        if bb.read_u8().is_err() {
            return false;
        }
        let mime = read_mime(&mut bb).unwrap_or_default();
        let contract = read_contract(&mut bb).unwrap_or_default();
        let response_mime = read_mime(&mut bb).unwrap_or_default();
        let response_contract = read_contract(&mut bb).unwrap_or_default();
        *self.spec.lock().unwrap() = FunctionInfoSpec {
            request_mime: mime,
            request_contract: contract,
            response_mime,
            response_contract
        };
        self.full_information.store(true, Ordering::SeqCst);
        true
    }

    pub fn did_not_respond(&self) {
        if let Some(r) = self.node_rating.as_ref() {
            decrease_clamped(r);
        }
    }

    pub fn was_unavailable(&self) {
        decrease_clamped(&self.rating);
        if let Some(r) = self.node_rating.as_ref() {
            increase_clamped(r);
        }
    }

    pub fn timed_out_while_executing(&self) {
        decrease_clamped(&self.rating);
    }

    pub fn returned_error(&self) {
    }

    pub fn returned_progress(&self) {
        if let Some(r) = self.node_rating.as_ref() {
            increase_clamped(r);
        }
    }

    pub fn was_succesful(&self) {
        increase_clamped(&self.rating);
        if let Some(r) = self.node_rating.as_ref() {
            increase_clamped(r);
        }
    }

    pub fn should_remove(&self) -> bool {
        self.rating.load(Ordering::SeqCst) == 0
    }

    pub fn get_rating(&self) -> i32 {
        self.rating.load(Ordering::SeqCst)
    }

    pub fn get_combined_rating(&self) -> i32 {
        let node_rating = self.node_rating.as_ref().map(|r| r.load(Ordering::SeqCst)).unwrap_or(1);
        node_rating * self.get_rating()
    }

    pub fn get_node_id(&self) -> u32 {
        self.node_id
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        status.put_string("type", kind_type_str(self.kind));
        status.put_int("rating", self.get_rating() as i64);
        if self.kind != 'c' {
            status.put_int("combinedRating", self.get_combined_rating() as i64);
        }
        status
    }
}

fn kind_type_str(kind: char) -> &'static str {
    match kind {
        's' => "service",
        't' => "stream",
        _ => "consumer"
    }
}

fn write_mime(bb: &mut ByteBuffer, mime: Option<&str>) {
    match mime {
        Some(m) => {
            bb.write_u8(m.len() as u8);
            let _ = bb.write_all(m.as_bytes());
        }
        None => bb.write_u8(0)
    }
}

fn write_contract(bb: &mut ByteBuffer, contract: Option<&str>) {
    match contract {
        Some(c) => {
            let len = c.len() as u32;
            bb.write_u32(len);
            let _ = bb.write_all(c.as_bytes());
        }
        None => bb.write_u32(0)
    }
}

fn read_mime(bb: &mut ByteReader) -> Option<Option<String>> {
    let len = bb.read_u8().ok()?;
    if len == 0 {
        return Some(None);
    }
    let bytes = bb.read_bytes(len as usize).ok()?;
    String::from_utf8(bytes).ok().map(Some)
}

fn read_contract(bb: &mut ByteReader) -> Option<Option<String>> {
    let len = bb.read_u32().ok()? as usize;
    if len == 0 {
        return Some(None);
    }
    let bytes = bb.read_bytes(len).ok()?;
    String::from_utf8(bytes).ok().map(Some)
}

fn decrease_clamped(counter: &AtomicI32) {
    let mut v = counter.load(Ordering::SeqCst);
    loop {
        if v == 0 {
            return;
        }
        match counter.compare_exchange_weak(v, v - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(cur) => v = cur
        }
    }
}

fn increase_clamped(counter: &AtomicI32) {
    let mut v = counter.load(Ordering::SeqCst);
    loop {
        if v >= 100 {
            return;
        }
        match counter.compare_exchange_weak(v, v + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(cur) => v = cur
        }
    }
}

// Mirrors io.firebus.information.NodeInformation.
pub struct NodeInfo {
    pub id: u32,
    addresses: Mutex<Vec<Address>>,
    repeaters: Mutex<Vec<u32>>,
    functions: Mutex<HashMap<String, Arc<FunctionInfo>>>,
    // Shared with every FunctionInfo of this node (the Java node rating that
    // functions' combined rating multiplies in).
    pub rating: Arc<AtomicI32>,
    last_updated: AtomicI64,
    last_discovery_sent: AtomicI64
}

impl NodeInfo {
    pub fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            addresses: Mutex::new(Vec::new()),
            repeaters: Mutex::new(Vec::new()),
            functions: Mutex::new(HashMap::new()),
            rating: Arc::new(AtomicI32::new(100)),
            last_updated: AtomicI64::new(0),
            last_discovery_sent: AtomicI64::new(0)
        })
    }

    pub fn set_last_discovery_sent_time(&self, t: i64) {
        self.last_discovery_sent.store(t, Ordering::SeqCst);
    }

    pub fn get_last_discovery_sent_time(&self) -> i64 {
        self.last_discovery_sent.load(Ordering::SeqCst)
    }

    pub fn set_last_updated_time(&self, t: i64) {
        self.last_updated.store(t, Ordering::SeqCst);
    }

    pub fn get_last_advertised_time(&self) -> i64 {
        self.last_updated.load(Ordering::SeqCst)
    }

    pub fn add_address(&self, a: &Address) {
        let mut addresses = self.addresses.lock().unwrap();
        if !addresses.iter().any(|x| x.equals(a)) {
            addresses.push(a.clone());
        }
    }

    pub fn remove_address(&self, a: &Address) {
        let mut addresses = self.addresses.lock().unwrap();
        addresses.retain(|x| !x.equals(a));
    }

    pub fn contains_address(&self, a: &Address) -> bool {
        self.addresses.lock().unwrap().iter().any(|x| x.equals(a))
    }

    pub fn get_address_count(&self) -> usize {
        self.addresses.lock().unwrap().len()
    }

    pub fn get_address(&self, i: usize) -> Option<Address> {
        self.addresses.lock().unwrap().get(i).cloned()
    }

    pub fn add_repeater(&self, id: u32) {
        let mut repeaters = self.repeaters.lock().unwrap();
        if !repeaters.contains(&id) {
            repeaters.push(id);
        }
    }

    pub fn get_repeater_count(&self) -> usize {
        self.repeaters.lock().unwrap().len()
    }

    pub fn get_repeater(&self, i: usize) -> u32 {
        self.repeaters.lock().unwrap().get(i).copied().unwrap_or(0)
    }

    pub fn get_random_repeater(&self) -> u32 {
        let repeaters = self.repeaters.lock().unwrap();
        if repeaters.is_empty() {
            return 0;
        }
        let mut r = rand::thread_rng();
        repeaters[r.gen_range(0..repeaters.len())]
    }

    pub fn add_function_information(&self, name: &str, fi: Arc<FunctionInfo>) {
        let mut functions = self.functions.lock().unwrap();
        if !functions.contains_key(name) {
            functions.insert(String::from(name), fi);
        }
    }

    pub fn get_function_information(&self, name: &str) -> Option<Arc<FunctionInfo>> {
        self.functions.lock().unwrap().get(name).cloned()
    }

    pub fn remove_function_information(&self, name: &str) {
        eprintln!("fb.nodeinfo.removing function={}", name);
        self.functions.lock().unwrap().remove(name);
    }

    pub fn did_not_respond(&self) {
        decrease_clamped(&self.rating);
    }

    pub fn responded(&self) {
        increase_clamped(&self.rating);
    }

    pub fn should_remove(&self) -> bool {
        self.rating.load(Ordering::SeqCst) == 0
    }

    pub fn get_rating(&self) -> i32 {
        self.rating.load(Ordering::SeqCst)
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        let mut add_list = DataList::new();
        for a in self.addresses.lock().unwrap().iter() {
            add_list.push_string(&format!("{}", a));
        }
        status.put_list("addresses", add_list);
        let mut rpt_list = DataList::new();
        for i in self.repeaters.lock().unwrap().iter() {
            rpt_list.push(crate::data::DataValue::Number(crate::data::Number::Int(*i as i64)));
        }
        status.put_list("repeaters", rpt_list);
        let mut func_map = DataMap::new();
        for (name, fi) in self.functions.lock().unwrap().iter() {
            func_map.put_map(name, fi.get_status());
        }
        status.put_map("functions", func_map);
        status.put_int("rating", self.get_rating() as i64);
        status
    }
}

// Mirrors io.firebus.Directory.
pub struct Directory {
    nodes: Mutex<Vec<Arc<NodeInfo>>>
}

impl Directory {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { nodes: Mutex::new(Vec::new()) })
    }

    pub fn get_node_count(&self) -> usize {
        self.nodes.lock().unwrap().len()
    }

    pub fn get_node(&self, index: usize) -> Option<Arc<NodeInfo>> {
        self.nodes.lock().unwrap().get(index).cloned()
    }

    pub fn get_node_by_id(&self, id: u32) -> Option<Arc<NodeInfo>> {
        if id == 0 {
            return None;
        }
        self.nodes.lock().unwrap().iter().find(|n| n.id == id).cloned()
    }

    pub fn get_node_by_address(&self, a: &Address) -> Option<Arc<NodeInfo>> {
        self.nodes.lock().unwrap().iter().find(|n| n.contains_address(a)).cloned()
    }

    pub fn delete_node(&self, n: &Arc<NodeInfo>) {
        eprintln!("fb.directory.delete node={}", n.id);
        self.nodes.lock().unwrap().retain(|x| !Arc::ptr_eq(x, n));
    }

    pub fn get_or_create_node_information(&self, node_id: u32) -> Arc<NodeInfo> {
        if let Some(ni) = self.get_node_by_id(node_id) {
            return ni;
        }
        let ni = NodeInfo::new(node_id);
        self.nodes.lock().unwrap().push(ni.clone());
        ni
    }

    // Mirrors processDiscoveredNode.
    pub fn process_discovered_node(&self, node_id: u32, address: Option<Address>) {
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.iter().any(|n| n.id == node_id) {
            drop(nodes);
            let ni = self.get_node_by_id(node_id).unwrap();
            if let Some(a) = address {
                if !ni.contains_address(&a) {
                    eprintln!("fb.directory.addressdiscovered node={} address={}", node_id, a);
                    ni.add_address(&a);
                }
            }
            return;
        }
        if let Some(a) = address.as_ref() {
            let victim = nodes.iter().find(|n| n.contains_address(a)).cloned();
            if let Some(node_by_address) = victim {
                eprintln!("fb.directory.nodediscovered node={} address={}", node_id, a);
                nodes.retain(|n| !Arc::ptr_eq(n, &node_by_address));
            }
        } else {
            eprintln!("fb.directory.nodediscovered node={} address=-", node_id);
        }
        let ni = NodeInfo::new(node_id);
        if let Some(a) = address {
            ni.add_address(&a);
        }
        nodes.push(ni);
    }

    // Mirrors processNodeInformation: parses the CRLF text snapshot. Only 'a'
    // and 'f' lines are honoured; 'd' lines are informational.
    pub fn process_node_information(&self, ad: &str) {
        for raw_line in ad.split('\n') {
            let line = raw_line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 2 {
                continue;
            }
            let id: u32 = match parts[0].trim().parse() {
                Ok(v) => v,
                Err(_) => continue
            };
            let ni = self.get_or_create_node_information(id);
            ni.set_last_updated_time(now_millis());
            match parts[1].trim() {
                "a" => {
                    if parts.len() >= 4 {
                        if let Ok(port) = parts[3].trim().parse() {
                            let a = Address::new(parts[2].trim(), port);
                            if let Some(other) = self.get_node_by_address(&a) {
                                if !Arc::ptr_eq(&other, &ni) {
                                    self.delete_node(&other);
                                }
                            }
                            ni.add_address(&a);
                        }
                    }
                }
                "f" if parts.len() >= 4 => {
                    let kind = parts[2].trim().chars().next();
                    let name = parts[3].trim();
                    if matches!(kind, Some('s') | Some('t') | Some('c'))
                        && ni.get_function_information(name).is_none()
                    {
                        let fi = FunctionInfo::new(
                            id,
                            Some(Arc::clone(&ni.rating)),
                            kind.unwrap(),
                            name,
                            FunctionInfoSpec::default()
                        );
                        ni.add_function_information(name, fi);
                    }
                }
                _ => {}
            }
        }
    }

    // Mirrors processFunctionInformation.
    pub fn process_function_information(&self, node_id: u32, function_name: &str, payload: &[u8]) {
        let ni = self.get_or_create_node_information(node_id);
        let fi = match ni.get_function_information(function_name) {
            Some(fi) => fi,
            None => {
                let kind = payload.first().map(|b| *b as char).unwrap_or('s');
                let fi = FunctionInfo::new(node_id, Some(Arc::clone(&ni.rating)), kind, function_name, FunctionInfoSpec::default());
                ni.add_function_information(function_name, fi.clone());
                fi
            }
        };
        let _ = fi.deserialise(payload);
    }

    // Mirrors getDirectoryStateString.
    pub fn get_directory_state_string(&self, node_id: u32) -> String {
        let mut sb = String::new();
        let nodes = self.nodes.lock().unwrap();
        for ni in nodes.iter() {
            sb.push_str(&format!("{},d,{},\r\n", node_id, ni.id));
            let addresses = ni.addresses.lock().unwrap();
            for a in addresses.iter() {
                sb.push_str(&format!("{},d,{},a,{},{}\r\n", node_id, ni.id, a.get_ip_address(), a.get_port()));
            }
        }
        sb
    }

    pub fn get_status(&self) -> DataMap {
        let mut status = DataMap::new();
        for ni in self.nodes.lock().unwrap().iter() {
            status.put_map(&ni.id.to_string(), ni.get_status());
        }
        status
    }
}

impl std::fmt::Display for Directory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nodes = self.nodes.lock().unwrap();
        if nodes.is_empty() {
            return write!(f, "Directory Empty");
        }
        let mut sb = String::new();
        for (i, ni) in nodes.iter().enumerate() {
            if i > 0 {
                sb.push_str("\r\n");
            }
            sb.push_str("---------Directory Entry--\r\n");
            let addresses = ni.addresses.lock().unwrap();
            let functions = ni.functions.lock().unwrap();
            sb.push_str(&format!("Id         : {}   [{}]\r\n", ni.id, ni.get_rating()));
            for a in addresses.iter() {
                sb.push_str(&format!("Address    : {}\r\n", a));
            }
            for (name, fi) in functions.iter() {
                sb.push_str(&format!("Function   : {:?} {}\r\n", name, fi.get_rating()));
            }
        }
        write!(f, "{}", sb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_info_serialise_default() {
        let fi = FunctionInfo::standalone('s', "svc", None);
        // 's' + u8 0 + i32 0 + u8 0 + i32 0
        let bytes = fi.serialise();
        assert_eq!(bytes, vec![0x73, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn function_info_serialise_matches_java_golden() {
        // tests/golden/java_funcinfo_service.hex, generated by the Java
        // ServiceInformation.serialise():
        //  's', 0x10 "application/json", 0x00000012 "request-contract-s",
        //  0x09 "text/html", 0x00000013 "response-contract-s"
        let fi = FunctionInfo::standalone('s', "svc", Some(FunctionInfoSpec {
            request_mime: Some(String::from("application/json")),
            request_contract: Some(String::from("request-contract-s")),
            response_mime: Some(String::from("text/html")),
            response_contract: Some(String::from("response-contract-s"))
        }));
        let expected: Vec<u8> = hex::decode(
            "73106170706C69636174696F6E2F6A736F6E00000012726571756573742D636F6E74726163742D7309746578742F68746D6C00000013726573706F6E73652D636F6E74726163742D73"
        ).unwrap();
        assert_eq!(fi.serialise(), expected);

        let fi2 = FunctionInfo::standalone('s', "svc", None);
        assert!(fi2.deserialise(&expected));
        assert!(fi2.has_full_information());
        assert_eq!(fi2.get_spec().request_mime.as_deref(), Some("application/json"));
        assert_eq!(fi2.get_spec().request_contract.as_deref(), Some("request-contract-s"));
        assert_eq!(fi2.get_spec().response_mime.as_deref(), Some("text/html"));
        assert_eq!(fi2.get_spec().response_contract.as_deref(), Some("response-contract-s"));
    }

    #[test]
    fn process_node_information_parses_java_snapshot() {
        let dir = Directory::new();
        // A realistic Java-generated snapshot from sender 777: every line is
        // prefixed with the sender's id (see NodeCore.processNodeInformationRequest
        // and Directory.getDirectoryStateString). The 'd' lines reference other
        // nodes (12345, 54321) in the third column but must NOT create them.
        let snapshot = "777,a,10.0.0.1,1990\r\n\
                        777,f,s,serviceOne\r\n\
                        777,f,t,streamOne\r\n\
                        777,f,c,consumerOne\r\n\
                        777,d,777,\r\n\
                        777,d,777,a,10.0.0.1,1990\r\n\
                        777,d,12345,\r\n\
                        777,d,12345,a,10.1.2.3,1990\r\n\
                        777,d,54321,\r\n\
                        777,d,54321,a,192.168.0.9,2000\r\n";
        dir.process_node_information(snapshot);
        // Only '777' (parts[0] of the 'a'/'f' lines) becomes a node; the ids in
        // the third column of the 'd' lines are not materialised.
        assert_eq!(dir.get_node_count(), 1);
        let n777 = dir.get_node_by_id(777).unwrap();
        assert_eq!(n777.get_address_count(), 1);
        assert_eq!(n777.get_address(0).unwrap().get_ip_address(), "10.0.0.1");
        assert_eq!(n777.get_address(0).unwrap().get_port(), 1990);
        assert!(n777.get_function_information("serviceOne").is_some());
        assert_eq!(n777.get_function_information("serviceOne").unwrap().get_kind(), 's');
        assert_eq!(n777.get_function_information("streamOne").unwrap().get_kind(), 't');
        assert_eq!(n777.get_function_information("consumerOne").unwrap().get_kind(), 'c');
        assert!(!n777.get_function_information("serviceOne").unwrap().has_full_information());
        assert!(dir.get_node_by_id(12345).is_none());
    }

    #[test]
    fn process_node_information_multiple_nodes() {
        let dir = Directory::new();
        // Snapshots from two different responders, as a mesh would deliver.
        dir.process_node_information("11,a,10.0.0.1,1990\r\n11,f,s,svcA\r\n11,d,22,\r\n11,d,22,a,10.0.0.2,1990\r\n");
        dir.process_node_information("22,a,10.0.0.2,1990\r\n22,f,t,strB\r\n22,d,11,\r\n22,d,11,a,10.0.0.1,1990\r\n");
        assert_eq!(dir.get_node_count(), 2);
        assert_eq!(dir.get_node_by_id(11).unwrap().get_address(0).unwrap().get_ip_address(), "10.0.0.1");
        assert_eq!(dir.get_node_by_id(22).unwrap().get_address(0).unwrap().get_ip_address(), "10.0.0.2");
        assert!(dir.get_node_by_id(11).unwrap().get_function_information("svcA").is_some());
        assert!(dir.get_node_by_id(22).unwrap().get_function_information("strB").is_some());
    }

    #[test]
    fn directory_state_string_format() {
        let dir = Directory::new();
        let n777 = dir.get_or_create_node_information(777);
        n777.add_address(&Address::new("10.0.0.1", 1990));
        n777.add_function_information(
            "serviceOne",
            FunctionInfo::new(777, Some(Arc::clone(&n777.rating)), 's', "serviceOne", FunctionInfoSpec::default())
        );
        dir.get_or_create_node_information(12345);
        let text = dir.get_directory_state_string(777);
        assert_eq!(text, "777,d,777,\r\n777,d,777,a,10.0.0.1,1990\r\n777,d,12345,\r\n");
    }

    #[test]
    fn address_ownership_moves_node() {
        let dir = Directory::new();
        let a = Address::new("10.9.9.9", 1990);
        let n1 = dir.get_or_create_node_information(1);
        n1.add_address(&a);
        // A different node claiming the same address replaces it.
        dir.process_node_information("2,a,10.9.9.9,1990\r\n2,f,s,s1\r\n");
        assert!(dir.get_node_by_id(1).is_none());
        assert!(dir.get_node_by_id(2).is_some());
    }
}
