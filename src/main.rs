#![allow(dead_code)]

mod data;
mod core;

use core::{message::{Message, MessageType}, payload::Payload, rcu::{RCUMap, RCUValue}, serviceman::Service, Firebus};
use std::{sync::Arc, thread, time::{self}};

use chrono::Utc;
use data::parsers;

// use chrono::Utc;
// use data::parsers;

struct Service1 { }

impl Service for Service1 {
    fn call(&self) {
        println!("Executing service");
        thread::sleep(time::Duration::from_millis(50));
    }
}

fn main() {
    /*let data_str = "{ \"dt\":\"2024-08-25T12:15:28.999+10:00\", num: 8.288 }";
    match parsers::parse_map(data_str) {
        Ok(mut map) => {
            map.put_date_utc("date", Utc::now());
            println!("Map is: {}", map);
            let mut payload = Payload::from_datamap(map);
            payload.set_meta("token", "bebop");
            let msg = Message::new(1, 2, MessageType::QueryNode, "test", Arc::new(payload));
            let bytes = msg.serialize();
            println!("Bytes are: {:#04X?}", bytes);

            let new_msg = Message::deserialize(&bytes).unwrap();
            println!("Payload data is {}", new_msg.payload.get_datamap().unwrap())

        },
        Err(e) => print!("Error: {}", e.message),
    } */
    
    let firebus = Firebus::new();
    thread::sleep(time::Duration::from_millis(20));
    firebus.register_service("service01", Arc::new(Service1{}));
    //firebus.dirman.add_function("service01", firebus.node_id);
    for _ in 0..10 {
        let payload = Payload::from_string("Request String");
        firebus.request("service01", Arc::new(payload));
        thread::sleep(time::Duration::from_millis(20));
    }
    

    /*let val: Arc<RCUValue<String>> = Arc::new(RCUValue::new("allo".to_owned()));
    for _ in 0..4 {
        let valref = val.clone();
        thread::spawn(move || {
            loop {
                println!("Value is {}", valref.read());
                thread::sleep(time::Duration::from_millis(100));
            }
        });    
    }

    let valref = val.clone();
    thread::spawn(move || {
        for i in 0..5 {
            valref.set(|val| {
                *val = format!("value{}", i);
            });
            thread::sleep(time::Duration::from_millis(1000));
        }
    });*/

    thread::sleep(time::Duration::from_millis(5000));
    println!("Main thread completing");
}
