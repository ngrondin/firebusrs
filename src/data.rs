use std::fmt::{self, Display, Formatter};

use chrono::prelude::*;
use datalist::DataList;
use datamap::DataMap;

pub mod parsers;
pub mod datamap;
pub mod datalist;
pub mod errors;


trait SerializableData {
    fn get_serialized_string(&self) -> String {
        let mut s = String::new();
        self.serialize_to_string(&mut s, 0);
        return s;
    }

    fn serialize_to_string(&self, buffer: &mut String, indent: u8);
}

pub enum Number {
    Int(i64),
    Float(f64),
    PositiveInfinity,
    NegativeInfinity,
    NaN
}

impl Display for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Number::Int(i) => write!(f, "{}", i),
            Number::Float(fl) => write!(f, "{}", fl),
            Number::PositiveInfinity => write!(f, "Infinity"),
            Number::NegativeInfinity => write!(f, "-Infinity"),
            Number::NaN => write!(f, "NaN")
        }
    }
}

impl Clone for Number {
    fn clone(&self) -> Self {
        match self {
            Self::Int(arg0) => Self::Int(arg0.clone()),
            Self::Float(arg0) => Self::Float(arg0.clone()),
            Self::PositiveInfinity => Self::PositiveInfinity,
            Self::NegativeInfinity => Self::NegativeInfinity,
            Self::NaN => Self::NaN,
        }
    }
}

pub enum Date {
    DateTimeUtc(DateTime<Utc>),
    DateTimeOffset(DateTime<FixedOffset>)
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Date::DateTimeUtc(dt) => write!(f, "{}", dt.to_rfc3339_opts(SecondsFormat::Millis, true)),
            Date::DateTimeOffset(dt) => write!(f, "{}", dt.to_rfc3339_opts(SecondsFormat::Millis, true)),
        }
    }
}

impl Clone for Date {
    fn clone(&self) -> Self {
        match self {
            Self::DateTimeUtc(dt) => Self::DateTimeUtc(dt.clone()),
            Self::DateTimeOffset(dt) => Self::DateTimeOffset(dt.clone()),
        }
    }
}

pub enum DataValue {
    String(String),
    Number(Number),
    Bool(bool),
    Date(Date),    
    DataMap(DataMap),
    DataList(DataList),
    Null,
    None
}

impl DataValue {
    pub fn get_string(&self) -> String {
        match self {
            DataValue::String(s) => return s.clone(),
            DataValue::Number(n) => return n.to_string(),
            DataValue::Bool(b) => return if *b { String::from("true") } else { String::from("false") },
            DataValue::DataMap(map) => return map.to_string(),
            DataValue::DataList(list) => return list.to_string(),
            _ => return String::from("null")
        }
    }
}

impl fmt::Display for DataValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_string())
    }
}

impl SerializableData for DataValue {
    fn serialize_to_string(&self, buffer: &mut String, indent: u8) {
        match self {
            DataValue::String(s) => buffer.push_str(&format!("\"{}\"", s)),
            DataValue::Date(d) => buffer.push_str(&format!("\"{}\"", d)),
            DataValue::DataMap(map) => map.serialize_to_string(buffer, indent),
            DataValue::DataList(list) => list.serialize_to_string(buffer, indent),
            _ => return buffer.push_str(&self.get_string()),            
        }    
     }
}

impl Clone for DataValue {
    fn clone(&self) -> Self {
        match self {
            Self::String(str) => Self::String(str.clone()),
            Self::Number(num) => Self::Number(num.clone()),
            Self::Bool(bl) => Self::Bool(bl.clone()),
            Self::Date(dt) => Self::Date(dt.clone()),
            Self::DataMap(map) => Self::DataMap(map.clone()),
            Self::DataList(list) => Self::DataList(list.clone()),
            Self::Null => Self::Null,
            Self::None => Self::None,
        }
    }
}

