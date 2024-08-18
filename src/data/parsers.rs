mod macros;
mod serialreader;
mod stringreader;

use super::datalist;
use super::datamap;
use super::DataError;
use super::DataValue;
use super::Date;
use super::Number;
use chrono::FixedOffset;
use chrono::TimeZone;
use chrono::Timelike;
use chrono::Utc;
use datalist::DataList;
use datamap::DataMap;
use serialreader::SerialReader;
use stringreader::StringReader;
use macros::*;

#[allow(dead_code)]
pub fn parse(s: &str) -> Result<DataValue, DataError> {
    let mut reader = SerialReader::new(s);
    let value: DataValue = parse_from_reader(&mut reader)?;
    return Ok(value);
}

pub fn parse_map(s: &str) -> Result<DataMap, DataError> {
    let mut reader = SerialReader::new(s);
    let map: DataMap = parse_map_from_reader(&mut reader)?;
    return Ok(map);
}

fn parse_from_reader(reader: &mut SerialReader) -> Result<DataValue, DataError> {
    while reader.has_more() {
        let c = reader.next();
        let spaceequiv = c == ' ' || c == '\r' || c == '\n' || c == '\t';
        if !spaceequiv {
            reader.back();
            match c {
                '{' => return Ok(DataValue::DataMap(parse_map_from_reader(reader)?)),
                '[' => return Ok(DataValue::DataList(parse_list_from_reader(reader)?)),
                _ => return Ok(parse_literal_from_reader(reader)?),
            }
        }
    }
    return Err(DataError { message : String::from("Unexpected end of string")})
}

enum MapDeserialState {  BeforeBrace, BeforeKey, InKey, AfterKey, BeforeValue, AfterValue }

fn parse_map_from_reader(reader: &mut SerialReader) -> Result<DataMap, DataError> {
    let mut state = MapDeserialState::BeforeBrace;
    let mut key: String = String::new();
    let mut inquote: bool = false;
    let mut map = DataMap::new();
    while reader.has_more() {
        let c = reader.next();
        let spaceequiv = c == ' ' || c == '\r' || c == '\n' || c == '\t';
        match state {
            MapDeserialState::BeforeBrace => {
                if !spaceequiv {
                    if c == '{' {
                        state = MapDeserialState::BeforeKey;
                    } else {
                        reader.back();
                        return Err(DataError { message : format!("Expecting '{{' at line {}, col {}", reader.row, reader.col)})
                    }
                }
            },
            MapDeserialState::BeforeKey => {
                if !spaceequiv {
                    if c == '"' {
                        inquote = true;
                    } else {
                        key.push(c);
                    }
                    state = MapDeserialState::InKey;
                }
            },
            MapDeserialState::InKey => {
                if inquote {
                    if c == '"' {
                        inquote = false;
                        state = MapDeserialState::AfterKey;
                    } else {
                        key.push(c);
                    }
                } else {
                    if spaceequiv {
                        state = MapDeserialState::AfterKey;
                    } else if c == ':' {
                        state = MapDeserialState::BeforeValue;
                    } else if c == '"' {
                        return Err(DataError { message : format!("Unexpected '\"' at line {}, col {}", reader.row, reader.col)})
                    } else {
                        key.push(c);
                    }
                }
            },
            MapDeserialState::AfterKey => {
                if !spaceequiv {
                    if c == ':' {
                        state = MapDeserialState::BeforeValue;
                    } else {
                        return Err(DataError { message : format!("Unexpected character at line {}, col {}", reader.row, reader.col)})
                    }
                }
            },
            MapDeserialState::BeforeValue => {
                if !spaceequiv {
                    reader.back();
                    let value = parse_from_reader(reader)?;
                    map.put(&key, value);
                    key.clear();
                    state = MapDeserialState::AfterValue;
                }
            },
            MapDeserialState::AfterValue => {
                if !spaceequiv {
                    if c == ',' {
                        state = MapDeserialState::BeforeKey;
                    } else if c == '}' {
                        return Ok(map);
                    } else {
                        return Err(DataError { message : format!("Unexpected character at line {}, col {}", reader.row, reader.col)})
                    }
                }
            }                                  
        }
    }
    return Err(DataError { message : String::from("String unexpectedly ended")})
}

enum ListDeserialState {  BeforeBracket, BeforeValue, AfterValue }

fn parse_list_from_reader(reader: &mut SerialReader) -> Result<DataList, DataError> {
    let mut state: ListDeserialState = ListDeserialState::BeforeBracket;
    let mut list = DataList::new();
    while reader.has_more() {
        let c = reader.next();
        let spaceequiv = c == ' ' || c == '\r' || c == '\n' || c == '\t';
        match state {
            ListDeserialState::BeforeBracket => {
                if !spaceequiv {
                    if c == '[' {
                        state = ListDeserialState::BeforeValue;
                    } else {
                        reader.back();
                        return Err(DataError { message : format!("Expected [ at line {}, col {}", reader.row, reader.col)})
                    }
                }
            },
            ListDeserialState::BeforeValue => {
                if !spaceequiv {
                    reader.back();
                    let value = parse_from_reader(reader)?;
                    list.push(value);
                    state = ListDeserialState::AfterValue;
                }
            },
            ListDeserialState::AfterValue => {
                if !spaceequiv {
                    if c == ',' {
                        state = ListDeserialState::BeforeValue;
                    } else if c == ']' {
                        return Ok(list);
                    } else {
                        return Err(DataError { message : format!("Expected ] at line {}, col {}", reader.row, reader.col)})
                    }
                }
            }                                  
        }
    }
    return Err(DataError { message : String::from("String unexpectedly ended")})
}

enum LiteralDeserialState {  BeforeValue, InValue }

fn parse_literal_from_reader(reader: &mut SerialReader) -> Result<DataValue, DataError> {
    let mut state = LiteralDeserialState::BeforeValue;
    let mut value: String = String::new();
    let mut inquote: bool = false;
    let mut escaping: bool = false;
    let mut hasquotes: bool = false;
    let mut done: bool = false;
    while reader.has_more() && !done {
        let c = reader.next();
        let spaceequiv = c == ' ' || c == '\r' || c == '\n' || c == '\t';
        match state {
            LiteralDeserialState::BeforeValue => {
                if !spaceequiv {
                    if c == '"' {
                        inquote = true;
                        hasquotes = true;
                    } else {
                        value.push(c);  
                    }
                    state = LiteralDeserialState::InValue;
                }
            },
            LiteralDeserialState::InValue => {
                if inquote {
                    if escaping {
                        match c {
                            '\\' => value.push('\\'),
                            'n' => value.push('\n'),
                            'r' => value.push('\r'),
                            't' => value.push('\t'),
                            '/' => value.push('/'),
                            '"' => value.push('\"'),
                            _ => ()
                        }
                        escaping = false;
                    } else if c == '\\' {
                        escaping = true
                    } else if c == '"' {
                        inquote = false;
                        done = true;
                    } else {
                        value.push(c);
                    }
                } else {
                    if spaceequiv || c == '}' || c == ']' || c == ','  {
                        reader.back();
                        done = true;
                    } else {
                        value.push(c);
                    }
                }
            }                                  
        }
    }
    if hasquotes {
        let dt_res = parse_date(&value);
        match dt_res {
            Result::Ok(dt) => return Ok(DataValue::Date(dt)),
            _ => return Ok(DataValue::String(value))
        }
    } else {
        if value.eq("true") {
            return Ok(DataValue::Bool(true));
        } else if value.eq("false") {
            return Ok(DataValue::Bool(false));
        } else if value.eq("mull") {
            return Ok(DataValue::Null);
        } else {
            let num_res = parse_number(&value);
            match num_res {
                Result::Ok(n) => return Ok(DataValue::Number(n)),
                _ => return Ok(DataValue::String(value))
            }
        }
    }
}

pub fn parse_number(s: &str) -> Result<Number, DataError> {
    if s.eq("Infinity") { return Ok(Number::PositiveInfinity) }
    if s.eq("-Infinity") { return Ok(Number::NegativeInfinity) }
    if s.eq("NaN") { return Ok(Number::NaN) }
    let mut neg = false;
    let mut int: i64 = 0;
    let mut dec: f64 = 0.0;
    let mut reader = StringReader { chars: s.chars() };
    let endchar = 0u8 as char;
    let mut c = reader.next();
    if c == '-' {
        neg = true;
        c = reader.next();
    }
    while c != endchar && c != '.' {
        assert_numeric!(c);
        int = (10 * int) + to_int!(c);
        c = reader.next();
    }
    if c == endchar {
        if neg {int *= -1;}
        return Ok(Number::Int(int));
    } else {
        let mut div: f64 = 10.0;
        c = reader.next();
        while c != endchar {
            assert_numeric!(c);
            dec += to_float!(c) / div;
            div *= 10.0;
            c = reader.next();
        }
        return Ok(Number::Float((int as f64) + dec))
    }
}

pub fn parse_date(s: &str) -> Result<Date, DataError> {
    let mut year = 0;
    let mut month = 0;
    let mut day = 0;
    let mut hour = 0;
    let mut min = 0;
    let mut sec = 0;
    let mut nano = 0;
    let mut offset = 0;
    let mut reader = StringReader { chars: s.chars() };
    let mut c = reader.next();
    assert_numeric!(c);
    year += 1000 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    year += 100 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    year += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    year += 1 * to_int!(c);
    c = reader.next();
    assert_same!(c, '-');
    c = reader.next();
    assert_numeric!(c);
    month += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    month += 1 * to_int!(c);
    c = reader.next();
    assert_same!(c, '-');
    c = reader.next();
    assert_numeric!(c);
    day += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    day += 1 * to_int!(c);
    c = reader.next();
    assert_same!(c, 'T');
    c = reader.next();
    assert_numeric!(c);
    hour += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    hour += 1 * to_int!(c);
    c = reader.next();
    assert_same!(c, ':');
    c = reader.next();
    assert_numeric!(c);
    min += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    min += 1 * to_int!(c);
    c = reader.next();
    assert_same!(c, ':');
    c = reader.next();
    assert_numeric!(c);
    sec += 10 * to_int!(c);
    c = reader.next();
    assert_numeric!(c);
    sec += 1 * to_int!(c);

    c = reader.next();
    assert_not_end!(c);
    if c == '.' {
        let mut nanoorder = 1;
        loop {
            c = reader.next();
            assert_not_end!(c);
            if c.is_digit(10) {
                nano = (10 * nano) + to_int!(c);
                nanoorder *= 10;
            } else if c == 'Z' || c == '+' || c == '-' {
                nano *= 1000000000 / nanoorder;
                break;
            } else {
                parse_err!()
            }
        }
    }

    if c == 'Z' {
        offset = 0;
        c = reader.next();
        if c != 0u8 as char { parse_err!() }
    } else {
        let neg = if c == '-' { true } else { false };
        c = reader.next();
        assert_numeric!(c);
        offset += to_int!(c) * 600;
        c = reader.next();
        assert_numeric!(c);
        offset += to_int!(c) * 60;
        c = reader.next();
        if c != 0u8 as char {
            assert_same!(c, ':');
            c = reader.next();
            assert_numeric!(c);
            offset += to_int!(c) * 10;
            c = reader.next();
            assert_numeric!(c);
            offset += to_int!(c) * 1;
            c = reader.next();
            if c != 0u8 as char { parse_err!() }
        }
        offset *= 60 * (if neg { -1 } else { 1 })
    }

    if offset == 0 {
        let dt = Utc.with_ymd_and_hms(year as i32, month as u32, day as u32, hour as u32, min as u32, sec as u32).unwrap().with_nanosecond(nano as u32).unwrap();
        return Ok(Date::DateTimeUtc(dt))
    } else {
        let dt = FixedOffset::east_opt(offset as i32).unwrap().with_ymd_and_hms(year as i32, month as u32, day as u32, hour as u32, min as u32, sec as u32).unwrap().with_nanosecond(nano as u32).unwrap();
        return Ok(Date::DateTimeOffset(dt))
    }
} 