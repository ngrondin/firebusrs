use std::collections::VecDeque;

pub const MAGIC: u8 = 0x7E;

// Mirrors Connection.send: 0x7E | u32-LE payload length | payload | xor-crc |
// zero padding so the frame total is a multiple of 16.
pub fn build_frame(msg: &[u8]) -> Vec<u8> {
    let mut crc: u8 = 0;
    for b in msg {
        crc ^= b;
    }
    let mut size = msg.len() + 6;
    size += 16 - (size % 16);
    let mut bytes = vec![0u8; size];
    bytes[0] = MAGIC;
    let len = msg.len() as u32;
    bytes[1] = (len & 0xFF) as u8;
    bytes[2] = ((len >> 8) & 0xFF) as u8;
    bytes[3] = ((len >> 16) & 0xFF) as u8;
    bytes[4] = ((len >> 24) & 0xFF) as u8;
    bytes[5..5 + msg.len()].copy_from_slice(msg);
    bytes[5 + msg.len()] = crc;
    bytes
}

// Mirrors Connection.listen: byte-by-byte state machine over the (decrypted)
// stream. Emits each complete payload; on a CRC mismatch the `bad_crc` flag is
// set (Java fails the connection in that case).
#[derive(Default)]
pub struct FrameDecoder {
    state: u8,
    msg_len: u32,
    msg_pos: usize,
    crc: u8,
    buf: Vec<u8>,
    out: VecDeque<Vec<u8>>,
    pub bad_crc: bool
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &raw in bytes.iter() {
            match self.state {
                0 => {
                    if raw == MAGIC {
                        self.msg_len = 0;
                        self.msg_pos = 0;
                        self.state = 1;
                    }
                }
                1 => {
                    self.msg_len |= (raw as u32) << (8 * (self.msg_pos as u32));
                    self.msg_pos += 1;
                    if self.msg_pos == 4 {
                        self.buf = vec![0u8; self.msg_len as usize];
                        self.msg_pos = 0;
                        self.crc = 0;
                        self.state = 2;
                    }
                }
                2 => {
                    self.buf[self.msg_pos] = raw;
                    self.crc ^= raw;
                    self.msg_pos += 1;
                    if self.msg_pos >= self.msg_len as usize {
                        self.state = 3;
                    }
                }
                3 => {
                    if raw == self.crc {
                        self.out.push_back(self.buf.clone());
                    } else {
                        self.bad_crc = true;
                    }
                    self.state = 0;
                }
                _ => {
                    self.bad_crc = true;
                    self.state = 0;
                }
            }
        }
    }

    pub fn take(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_fixture(name: &str, msg_name: &str) {
        let msg: Vec<u8> = hex::decode(std::fs::read_to_string(format!("tests/golden/{}", msg_name)).unwrap().trim()).unwrap();
        let frame = build_frame(&msg);
        let expected = std::fs::read_to_string(format!("tests/golden/{}", name)).unwrap().trim().to_lowercase();
        assert_eq!(hex::encode(&frame), expected);
    }

    #[test]
    fn build_frame_matches_java_fixture() {
        assert_fixture("java_framed_request.hex", "java_msg_request.hex");
    }

    #[test]
    fn build_frame_matches_java_nodeinfo_fixture() {
        assert_fixture("java_framed_nodeinfo.hex", "java_msg_nodeinfo.hex");
    }

    #[test]
    fn round_trip_recovers_payload_with_marker_bytes() {
        // Payload deliberately starts with the start marker so 0x7E bytes appear
        // inside the framed payload; framing stays unambiguous because 0x7E only
        // resynchronises while in state 0.
        let payload: Vec<u8> = vec![0x7E, 0x7E, 1, 2, 3, 0x7E];
        let frame = build_frame(&payload);
        let mut dec = FrameDecoder::new();
        dec.feed(&frame);
        assert!(!dec.bad_crc);
        assert_eq!(dec.take().unwrap(), payload);
        assert!(dec.take().is_none());
    }

    #[test]
    fn consecutive_frames_both_decode() {
        let p1: Vec<u8> = vec![1, 2, 3];
        let p2: Vec<u8> = vec![4, 5];
        let mut wire = build_frame(&p1);
        wire.extend(build_frame(&p2));
        let mut dec = FrameDecoder::new();
        dec.feed(&wire);
        assert!(!dec.bad_crc);
        assert_eq!(dec.take().unwrap(), p1);
        assert_eq!(dec.take().unwrap(), p2);
        assert!(dec.take().is_none());
    }

    #[test]
    fn decode_java_framed_crc7e_fixture() {
        // Java vector whose XOR-CRC byte is 0x7E (framing edge case).
        let frame: Vec<u8> = hex::decode(std::fs::read_to_string("tests/golden/java_framed_crc7e.hex").unwrap().trim()).unwrap();
        let mut dec = FrameDecoder::new();
        dec.feed(&frame);
        assert!(!dec.bad_crc);
        let out = dec.take().unwrap();
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn crc_mismatch_flags_bad_crc() {
        let msg: Vec<u8> = vec![1, 2, 3];
        let mut frame = build_frame(&msg);
        frame[5] ^= 0xFF; // corrupt first payload byte, keep crc
        let mut dec = FrameDecoder::new();
        dec.feed(&frame);
        assert!(dec.bad_crc);
        assert!(dec.take().is_none());
    }
}
