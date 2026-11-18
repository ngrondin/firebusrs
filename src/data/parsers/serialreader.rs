pub struct SerialReader {
    chars: Vec<char>,
    i: usize,
    len: usize,
    pub row: usize,
    pub col: usize
}

impl SerialReader {
    pub fn new(in_str: & str) -> Self {
        let vec: Vec<char> = in_str.chars().collect();
        let l = vec.len();
        Self { chars: vec, i: 0, len: l, row: 0, col: 0 }
    }

    pub fn next(&mut self) -> char {
        if self.i < self.len {
            let c = self.chars[self.i];
            self.i += 1;
            self.col += 1;
            if c == '\n' {
                self.col = 0;
                self.row += 1;
            }
            return c;
        } else {
            return ' ';
        }
    }

    pub fn back(&mut self) {
        if self.i > 0 {
            self.i -= 1;
            self.col -= 1;
        }
    }

    pub fn has_more(&self) -> bool {
        self.i < self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_at_start_does_not_panic() {
        let mut reader = SerialReader::new("abc");
        reader.back();
        assert_eq!(reader.i, 0);
        reader.next();
        reader.back();
        reader.back();
        assert_eq!(reader.i, 0);
    }
}