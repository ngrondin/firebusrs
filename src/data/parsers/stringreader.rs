use std::str::Chars;

pub struct StringReader<'a> {
    pub chars: Chars<'a>
}

impl<'a> StringReader<'a> {
    pub fn next(&mut self) -> char {
        let res = self.chars.next();
        match res {
            Option::Some(c) => return c,
            Option::None => return 0u8 as char
        }
    }
}