use std::{collections::HashMap, error::Error, io::{Read, Write}};

use bytebuffer::{ByteBuffer, ByteReader};

use crate::data::{datalist::DataList, datamap::DataMap, errors::DataError, parsers};


enum PayloadData {
    Bytes(Vec<u8>),
    Map(DataMap),
    List(DataList),
    String(String),
    Empty
}


pub struct Payload {
    data: PayloadData,
    metadata: HashMap<String, String>
}

impl Payload {
    pub fn empty() -> Self {
        Self {
            data: PayloadData::Empty,
            metadata: HashMap::new()
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            data: PayloadData::Bytes(bytes),
            metadata: HashMap::new()
        }
    }

    pub fn from_datamap(map: DataMap) -> Self {
        Self {
            data: PayloadData::Map(map),
            metadata: HashMap::new()
        }
    }

    pub fn from_datalist(list: DataList) -> Self {
        Self {
            data: PayloadData::List(list),
            metadata: HashMap::new()
        }
    }

    pub fn from_string(str: &str) -> Self {
        Self {
            data: PayloadData::String(String::from(str)),
            metadata: HashMap::new()
        }
    }

    pub fn set_meta(&mut self, key: &str, val: &str) {
        self.metadata.insert(String::from(key), String::from(val));
    }

    pub fn get_bytes(&self) -> Vec<u8> {
        match &(self.data) {
            PayloadData::Bytes(bytes) => bytes.clone(),
            PayloadData::Map(map) => map.to_string().as_bytes().into(),
            PayloadData::List(list) => list.to_string().as_bytes().into(),
            PayloadData::String(str) => str.as_bytes().into(),
            PayloadData::Empty => Vec::new()
        }
    }

    pub fn get_datamap(&self) -> Result<DataMap, DataError> {
        match &(self.data) {
            PayloadData::Map(map) => Ok(map.clone()),
            PayloadData::Bytes(bytes) => {
                match String::from_utf8(bytes.clone()) {
                    Ok(str) => {
                        match parsers::parse_map(&str) {
                            Ok(map) => Ok(map),
                            Err(err) => Err(err)
                        }
                    },
                    Err(err) => Err(DataError { message: format!("Not a UTF8 stream: {}", err) })
                }
            },
            PayloadData::String(str) => {
                match parsers::parse_map(&str) {
                    Ok(map) => Ok(map),
                    Err(err) => Err(err)
                }  
            },
            _ => Err(DataError { message: format!("Not a datamap") }),
        }
    }
    
    fn get_type(&self) -> u32 {
        match &(self.data) {
            PayloadData::Empty => 0,
            PayloadData::Bytes(_b) => 1,
            PayloadData::String(_s) => 2,
            PayloadData::Map(_m) => 3,
            PayloadData::List(_l) => 4
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut parts: Vec<String> = Vec::new();
        for (key, value) in self.metadata.iter() {
            parts.push(format!("{}:{}", key, value));
        }
        let metadatabytes = parts.join(",").as_bytes().to_vec();
        let metadatalen: u32 = metadatabytes.len().try_into().unwrap();
        let data = self.get_bytes();
        let mut bb = ByteBuffer::new();
        bb.write_u32(metadatalen);
        let _ = bb.write(&metadatabytes);
        bb.write_u32(self.get_type());
        let _ = bb.write(&data);
        bb.into_vec()
    }

    fn try_deserialize(data: &Vec<u8>) -> Result<Self, Box<dyn Error>> {
        let mut bb = ByteReader::from_bytes(&data);
        let metadatalen = bb.read_u32()?.try_into()?;
        let metadatabytes = bb.read_bytes(metadatalen)?;
        let metadatastr = String::from_utf8(metadatabytes)?;
        let metadataparts = metadatastr.split(":");
        let mut metadata: HashMap<String, String> = HashMap::new();
        for part in metadataparts {
            let mut split = part.split(":");
            if let Some(key) = split.next() {
                if let Some(val) = split.next() {
                    metadata.insert(String::from(key), String::from(val));
                }
            }
        }
        let datatype = bb.read_u32()?;
        let mut bytes: Vec<u8> = Vec::new();
        let _ = bb.read_to_end(&mut bytes);
        let data = match datatype {
            1 => PayloadData::Bytes(bytes),
            2 => PayloadData::String(String::from_utf8(bytes)?),
            3 => PayloadData::Map( parsers::parse_map(&String::from_utf8(bytes)?)? ),
            4 => PayloadData::List( parsers::parse_list(&String::from_utf8(bytes)?)? ),
            _ => PayloadData::Empty
        };
        Ok( Self {
            metadata: metadata,
            data: data
        } )
    }

    pub fn deserialize(data: &Vec<u8>) -> Result<Self, DataError> {
        match Payload::try_deserialize(data) {
            Ok(o) => Ok(o),
            Err(err) => Err( DataError { message: format!("Error deserializing payload: {}", err ) })
        }
    }

}

