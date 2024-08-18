mod data;

use chrono::Utc;
use data::parsers;

fn main() {
    let data_str = "{ \"dt\":\"2024-08-25T12:15:28.999+10:00\", num: 8.288 }";
    match parsers::parse_map(data_str) {
        Ok(mut map) => {
            map.put_date_utc("date", Utc::now());
            println!("Deserialized is: {}", map);
        },
        Err(e) => print!("Error: {}", e.message),
    }    
}
