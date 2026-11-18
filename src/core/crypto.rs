use aes::cipher::generic_array::{typenum::U16, GenericArray};
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes256;

use sha2::Sha256;

// Mirrors the NodeCore.java:40 salt.
pub const SALT: [u8; 16] = [
    0x62, 0x11, 0xD5, 0x21, 0xC6, 0xEA, 0x26, 0x57, 0xFB, 0xC2, 0x43, 0x47, 0x09, 0x36, 0xC9, 0x0C
];

// Mirrors NodeCore.initialise: PBKDF2WithHmacSHA256, 65536 rounds, 256-bit key.
pub fn derive_key(password: &str) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &SALT, 65536, &mut key);
    key
}

// AES-256 CFB8 (feedback size 8 bits). The JCE "AES/CFB8/NoPadding" keystream is
// byte-oriented, so encrypt and decrypt use the same feedback step:
//   S_i = MSB(E_k(X_i)); Y_i = P_i XOR S_i; X_{i+1} = (X_i <<< 1 byte) || Y_i
// where for ciphertext X starts as the IV and for plaintexts the *cipher* byte
// fed back is the (already transformed) output byte - identical loop.
pub struct Cfb8 {
    cipher: Aes256,
    feedback: GenericArray<u8, U16>
}

impl Cfb8 {
    pub fn new(key: &[u8; 32], iv: &[u8; 16]) -> Self {
        Self {
            cipher: Aes256::new_from_slice(key).expect("32-byte aes key"),
            feedback: GenericArray::clone_from_slice(iv)
        }
    }

    fn keystream(&self, x: &GenericArray<u8, U16>) -> u8 {
        let mut block = *x;
        self.cipher.encrypt_block(&mut block);
        block[0]
    }

    // In-place P -> C. Next feedback block: shift the ORIGINAL block and
    // append the ciphertext byte produced.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let x = self.feedback;
            *byte ^= self.keystream(&x);
            self.feedback[..15].copy_from_slice(&x[1..16]);
            self.feedback[15] = *byte;
        }
    }

    // In-place C -> P. Same keystream, but the feedback shift must append the
    // *ciphertext* byte, i.e. the input byte before the XOR.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let x = self.feedback;
            let c = *byte;
            *byte ^= self.keystream(&x);
            self.feedback[..15].copy_from_slice(&x[1..16]);
            self.feedback[15] = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_fixture(name: &str, pw: &str) {
        let key = derive_key(pw);
        let expected = std::fs::read_to_string(format!("tests/golden/{}", name)).unwrap().trim().to_lowercase();
        assert_eq!(hex::encode(key), expected);
    }

    #[test]
    fn derive_key_default_password() {
        assert_fixture("java_pbkdf2_key_default.hex", "firebuspassword0");
    }

    #[test]
    fn derive_key_alt_password() {
        assert_fixture("java_pbkdf2_key_alt.hex", "anotherpassword123");
    }
}
