use std::collections::BTreeMap;
use std::error::Error;
use std::io::{Read, Write};

use bytebuffer::{ByteBuffer, ByteReader};

use crate::data::{datalist::DataList, datamap::DataMap, errors::DataError, parsers};

#[derive(Clone)]
enum PayloadData {
    Bytes(Vec<u8>),
    Map(DataMap),
    List(DataList),
    String(String),
    Empty
}

#[derive(Clone)]
pub struct Payload {
    data: PayloadData,
    pub metadata: BTreeMap<String, String>
}

impl Payload {
    pub fn empty() -> Self {
        Self {
            data: PayloadData::Empty,
            metadata: BTreeMap::new()
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            data: PayloadData::Bytes(bytes),
            metadata: BTreeMap::new()
        }
    }

    pub fn from_datamap(map: DataMap) -> Self {
        Self {
            data: PayloadData::Map(map),
            metadata: BTreeMap::new()
        }
    }

    pub fn from_datalist(list: DataList) -> Self {
        Self {
            data: PayloadData::List(list),
            metadata: BTreeMap::new()
        }
    }

    pub fn from_string(str: &str) -> Self {
        Self {
            data: PayloadData::String(String::from(str)),
            metadata: BTreeMap::new()
        }
    }

    pub fn set_meta(&mut self, key: &str, val: &str) {
        self.metadata.insert(String::from(key), String::from(val));
    }

    pub fn get_meta(&self, key: &str) -> &str {
        match self.metadata.get(key) {
            Some(v) => v,
            None => ""
        }
    }

    pub fn get_type(&self) -> u32 {
        match &self.data {
            PayloadData::Empty => 0,
            PayloadData::Bytes(_b) => 1,
            PayloadData::String(_s) => 2,
            PayloadData::Map(_m) => 3,
            PayloadData::List(_l) => 4
        }
    }

    pub fn get_bytes(&self) -> Vec<u8> {
        match &self.data {
            PayloadData::Bytes(bytes) => bytes.clone(),
            PayloadData::Map(map) => map.to_string().as_bytes().to_vec(),
            PayloadData::List(list) => list.to_string().as_bytes().to_vec(),
            PayloadData::String(str) => str.as_bytes().to_vec(),
            PayloadData::Empty => Vec::new()
        }
    }

    pub fn get_string(&self) -> String {
        match &self.data {
            PayloadData::Bytes(bytes) => String::from_utf8_lossy(bytes).to_string(),
            PayloadData::Map(map) => map.to_string(),
            PayloadData::List(list) => list.to_string(),
            PayloadData::String(str) => str.clone(),
            PayloadData::Empty => String::new()
        }
    }

    pub fn get_datamap(&self) -> Result<DataMap, DataError> {
        match &self.data {
            PayloadData::Map(map) => Ok(map.clone()),
            PayloadData::List(_l) => Err(DataError { message: String::from("Not a datamap type") }),
            PayloadData::Bytes(_) | PayloadData::String(_) => {
                let s = match &self.data {
                    PayloadData::Bytes(b) => String::from_utf8(b.clone())
                        .map_err(|err| DataError { message: format!("Not a UTF8 stream: {}", err) })?,
                    PayloadData::String(s) => s.clone(),
                    _ => unreachable!()
                };
                parsers::parse_map(&s)
            },
            PayloadData::Empty => Err(DataError { message: String::from("Not a datamap") })
        }
    }

    pub fn get_datalist(&self) -> Result<DataList, DataError> {
        match &self.data {
            PayloadData::List(list) => Ok(list.clone()),
            PayloadData::Map(_m) => Err(DataError { message: String::from("Not a datalist type") }),
            PayloadData::Bytes(_) | PayloadData::String(_) => {
                let s = match &self.data {
                    PayloadData::Bytes(b) => String::from_utf8(b.clone())
                        .map_err(|err| DataError { message: format!("Not a UTF8 stream: {}", err) })?,
                    PayloadData::String(s) => s.clone(),
                    _ => unreachable!()
                };
                parsers::parse_list(&s)
            },
            PayloadData::Empty => Err(DataError { message: String::from("Not a datalist") })
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut meta = String::from("{");
        let mut first = true;
        for (key, value) in self.metadata.iter() {
            if !first {
                meta.push_str(", ");
            }
            meta.push_str(key);
            meta.push('=');
            meta.push_str(value);
            first = false;
        }
        meta.push('}');
        let metadatabytes = meta.as_bytes().to_vec();
        let metadatalen: u32 = metadatabytes.len() as u32;
        let data = self.get_bytes();
        let mut bb = ByteBuffer::new();
        bb.write_u32(metadatalen);
        let _ = bb.write_all(&metadatabytes);
        bb.write_u32(self.get_type());
        if !data.is_empty() {
            let _ = bb.write_all(&data);
        }
        bb.into_vec()
    }

    fn try_deserialize(data: &[u8]) -> Result<Self, Box<dyn Error>> {
        if data.is_empty() {
            return Ok(Payload::empty());
        }
        let mut bb = ByteReader::from_bytes(data);
        let metadatalen = bb.read_u32()? as usize;
        let metadatabytes = bb.read_bytes(metadatalen)?;
        let metadatastr = String::from_utf8(metadatabytes.to_vec())?;
        let metadata = parse_metadata_str(&metadatastr);
        let datatype = bb.read_u32()?;
        let mut bytes: Vec<u8> = Vec::new();
        let _ = bb.read_to_end(&mut bytes);
        let data = match datatype {
            1 => PayloadData::Bytes(bytes),
            2 => PayloadData::String(String::from_utf8(bytes)?),
            3 => {
                match String::from_utf8(bytes) {
                    Ok(s) => match parsers::parse_map(&s) {
                        Ok(map) => PayloadData::Map(map),
                        Err(_e) => PayloadData::Bytes(s.into_bytes())
                    },
                    Err(e) => return Err(Box::new(e))
                }
            },
            4 => {
                match String::from_utf8(bytes) {
                    Ok(s) => match parsers::parse_list(&s) {
                        Ok(list) => PayloadData::List(list),
                        Err(_e) => PayloadData::Bytes(s.into_bytes())
                    },
                    Err(e) => return Err(Box::new(e))
                }
            },
            _ => if bytes.is_empty() { PayloadData::Empty } else { PayloadData::Bytes(bytes) }
        };
        Ok( Self {
            metadata,
            data
        } )
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, DataError> {
        match Payload::try_deserialize(data) {
            Ok(o) => Ok(o),
            Err(err) => Err( DataError { message: format!("Error deserializing payload: {}", err ) })
        }
    }
}

// Java side serialises metadata as HashMap.toString(): `{k=v, k2=v2}` (empty = `{}`) and
// restores it with `new StringReader(str.substring(1, len-1).replace(", ", "\n"))` fed to
// Properties.load. This mirrors that behaviour, including Properties escape handling.
fn parse_metadata_str(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if s.is_empty() || s.len() < 2 {
        return out;
    }
    let inner = &s[1..s.len() - 1];
    let normalized = inner.replace(", ", "\n");
    parse_properties(&normalized, &mut out);
    out
}

fn parse_properties(input: &str, out: &mut BTreeMap<String, String>) {
    for line in input.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() && matches!(chars[i], ' ' | '\t' | '\r' | '\u{c}') {
            i += 1;
        }
        if i >= chars.len() {
            continue;
        }
        if chars[i] == '#' || chars[i] == '!' {
            continue;
        }
        let mut key: String = String::new();
        let mut in_value = false;
        // key scan
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() {
                let (ch, adv) = unescape(&chars, i + 1);
                key.push(ch);
                i += adv;
                continue;
            }
            if matches!(c, ' ' | '\t' | '\r' | '\u{c}' | '=' | ':') {
                break;
            }
            key.push(c);
            i += 1;
        }
        // skip whitespace between key and separator (java only treats '=' or ':' as real separator)
        while i < chars.len() && matches!(chars[i], ' ' | '\t' | '\r' | '\u{c}') {
            i += 1;
        }
        let mut value: String = String::new();
        if i < chars.len() && (chars[i] == '=' || chars[i] == ':') {
            in_value = true;
            i += 1;
            while i < chars.len() && matches!(chars[i], ' ' | '\t' | '\r' | '\u{c}') {
                i += 1;
            }
            while i < chars.len() {
                let c = chars[i];
                if c == '\\' && i + 1 < chars.len() {
                    let (ch, adv) = unescape(&chars, i + 1);
                    value.push(ch);
                    i += adv;
                    continue;
                }
                value.push(c);
                i += 1;
            }
        }
        if !in_value {
            key = key.trim().to_string();
        }
        if !key.is_empty() || !value.is_empty() {
            out.insert(key, value);
        }
    }
}

// java.util.Properties.load escape handling: \uXXXX, \n \t \r \f \\, any other \c -> c
fn unescape(chars: &[char], start: usize) -> (char, usize) {
    let c = chars[start];
    match c {
        'n' => ('\n', 1),
        't' => ('\t', 1),
        'r' => ('\r', 1),
        'f' => ('\u{c}', 1),
        '\\' => ('\\', 1),
        'u' if start + 4 < chars.len() => {
            let hex: String = chars[start + 1..start + 5].iter().collect();
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                if let Some(ch) = char::from_u32(code) {
                    return (ch, 5);
                }
            }
            ('u', 1)
        },
        _ => (c, 1)
    }
}
