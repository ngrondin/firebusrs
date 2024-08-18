use std::fmt;

use super::{DataValue, SerializableData};

pub struct DataList {
    vec: std::vec::Vec<DataValue>
}

#[allow(dead_code)]
impl DataList {
    pub fn new() -> Self {
        Self { vec: Vec::new() }
    } 

    pub fn push(&mut self, val: DataValue) {
        self.vec.push(val);
    }

    pub fn push_string(&mut self, val: &str) {
        self.vec.push(DataValue::String(val.to_string()));
    } 
}

impl fmt::Display for DataList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_serialized_string())
    }
}

impl SerializableData for DataList {
    fn serialize_to_string(&self, buffer: &mut String, indent: u8) {
        let mut indent_str = String::new();
        for _ in 0..indent {
            indent_str.push_str("  ");
        }
        buffer.push_str("[\r\n");
        let mut i = self.vec.len();
        for val in self.vec.iter() {
            buffer.push_str(&indent_str);
            buffer.push_str("  ");
            val.serialize_to_string(buffer, indent + 1);
            if i > 1 {
                buffer.push_str(",");
            }
            buffer.push_str("\r\n");
            i -= 1;
        }
        buffer.push_str(&indent_str);
        buffer.push_str("]");
    }
}