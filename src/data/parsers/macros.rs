
macro_rules! parse_err {
    () => {
        return Err(DataError { message: String::from("Unexpected character")})
    }
}

pub(crate) use parse_err; 

macro_rules! to_int {
    ($c: ident) => {
        ((($c as u8) - 48u8) as i64)
    }
}

pub(crate) use to_int; 

macro_rules! to_float {
    ($c: expr) => {
        ((($c as u8) - 48u8) as f64)
    }
}

pub(crate) use to_float; 

macro_rules! assert_not_end {
    ($c: expr) => {
        if $c == 0u8 as char { parse_err!() }
    }
}

pub(crate) use assert_not_end; 

macro_rules! assert_numeric {
    ($c: expr) => {
        if !$c.is_digit(10) { parse_err!() }
    }
}

pub(crate) use assert_numeric; 

macro_rules! assert_same {
    ($c: expr, $o: expr) => {
        if $c != $o { parse_err!() }
    }
}

pub(crate) use assert_same; 