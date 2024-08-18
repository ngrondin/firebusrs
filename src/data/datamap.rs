
use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};

use super::{datalist::DataList, DataValue, Date, Number, SerializableData};

pub struct DataMap {
    map: std::collections::HashMap<String, DataValue>
}

#[allow(dead_code)]
impl DataMap {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    pub fn new_with(arr: &[&str]) -> Self {
        let mut s: Self = Self::new();
        let mut i = 0;
        while i + 1 < arr.len() {
            let key = arr[i];
            let val = arr[i + 1];
            s.put_string(key, val);
            i += 1;
        }
        s
    }

    pub fn get(&self, k: &str) -> &DataValue {
        let opt = self.map.get(k);
        match opt {
            Some(val) => return val,
            None => return &(DataValue::None),
        }
    }

    pub fn get_string(&self, k: &str) -> String {
        let val = self.get(k);
        return val.get_string();
    }

    pub fn put(&mut self, k: &str, v: DataValue) {
        self.map.insert(k.to_string(), v);
    }

    pub fn put_string(&mut self, k: &str, v: &str) {
        self.put(k, DataValue::String(v.to_string()));
    }

    pub fn put_int(&mut self, k: &str, v: i64) {
        self.put(k, DataValue::Number(Number::Int(v)));
    }

    pub fn put_float(&mut self, k: &str, v: f64) {
        self.put(k, DataValue::Number(Number::Float(v)));
    }

    pub fn put_bool(&mut self, k: &str, v: bool) {
        self.put(k, DataValue::Bool(v));
    }

    pub fn put_date_utc(&mut self, k: &str, v: DateTime<Utc>) {
        self.put(k, DataValue::Date(Date::DateTimeUtc(v)));
    }

    pub fn put_map(&mut self, k: &str, v: DataMap) {
        self.put(k, DataValue::DataMap(v));
    }

    pub fn put_list(&mut self, k: &str, v: DataList) {
        self.put(k, DataValue::DataList(v));
    }
}

impl fmt::Display for DataMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_serialized_string())
    }
}

impl SerializableData for DataMap {
    fn serialize_to_string(&self, buffer: &mut String, indent: u8) {
        let mut indent_str = String::new();
        for _ in 0..indent {
            indent_str.push_str("  ");
        }
        buffer.push_str("{\r\n");
        let mut i = self.map.len();
        for (key, val) in self.map.iter() {
            buffer.push_str(&indent_str);
            buffer.push_str("  \"");
            buffer.push_str(key);
            buffer.push_str("\":");
            val.serialize_to_string(buffer, indent + 1);
            if i > 1 {
                buffer.push_str(",");
            }
            buffer.push_str("\r\n");
            i -= 1;
        }
        buffer.push_str(&indent_str);
        buffer.push_str("}");
    }
}