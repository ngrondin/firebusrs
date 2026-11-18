use firebus::core::errors::FunctionError;
use firebus::core::payload::Payload;
use firebus::core::{Firebus, ServiceProvider};

use std::sync::Arc;
use std::{thread, time};

struct EchoService {}

impl ServiceProvider for EchoService {
    fn service(&self, payload: &Payload) -> Result<Payload, FunctionError> {
        Ok(Payload::from_string(&format!("echo:{}", payload.get_string())))
    }

    fn get_service_information(&self) -> Option<firebus::core::FunctionInfoSpec> {
        None
    }
}

fn main() {
    let firebus = Firebus::new();
    let node_id = firebus.get_node_id();
    println!("Node id: {}", node_id);

    firebus.register_service_provider("service01", Arc::new(EchoService {}), 5);
    thread::sleep(time::Duration::from_millis(50));

    for i in 0..3 {
        let payload = Payload::from_string(&format!("Request String {}", i));
        match firebus.request_service("service01", &payload) {
            Ok(response) => println!("Response {}: {}", i, response.get_string()),
            Err(e) => eprintln!("Error: {}", e)
        }
        thread::sleep(time::Duration::from_millis(100));
    }

    println!("{}", firebus);
    firebus.close();
    println!("Main thread completing");
}
