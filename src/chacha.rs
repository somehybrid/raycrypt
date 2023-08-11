// A Rust implementation of XChaCha20-Poly1305
// This implementation defaults to 20 rounds
use crate::poly1305::Poly1305;
use crate::util::randbytes;
use pyo3::exceptions::PyAssertionError;
use pyo3::prelude::*;
use std::borrow::Cow;

const ROUNDS: usize = 20;

fn from_le_bytes(x: &[u8]) -> u32 {
    u32::from_le_bytes([x[0], x[1], x[2], x[3]])
}

fn quarter_round(a: usize, b: usize, c: usize, d: usize, block: &mut [u32; 16]) {
    block[a] = block[a].wrapping_add(block[b]);
    block[d] ^= block[a];
    block[d] = block[d].rotate_left(16);

    block[c] = block[c].wrapping_add(block[d]);
    block[b] ^= block[c];
    block[b] = block[b].rotate_left(12);

    block[a] = block[a].wrapping_add(block[b]);
    block[d] ^= block[a];
    block[d] = block[d].rotate_left(8);

    block[c] = block[c].wrapping_add(block[d]);
    block[b] ^= block[c];
    block[b] = block[b].rotate_left(7);
}

fn double_round(mut block: [u32; 16]) -> [u32; 16] {
    quarter_round(0, 4, 8, 12, &mut block);
    quarter_round(1, 5, 9, 13, &mut block);
    quarter_round(2, 6, 10, 14, &mut block);
    quarter_round(3, 7, 11, 15, &mut block);

    quarter_round(0, 5, 10, 15, &mut block);
    quarter_round(1, 6, 11, 12, &mut block);
    quarter_round(2, 7, 8, 13, &mut block);
    quarter_round(3, 4, 9, 14, &mut block);

    block
}

pub struct ChaCha20 {
    key: Vec<u8>,
}

// An implementation of IETF ChaCha
impl ChaCha20 {
    pub fn new(key: Vec<u8>) -> ChaCha20 {
        ChaCha20 { key }
    }

    fn keystream(&self, nonce: &[u8], counter: u32) -> Vec<u8> {
        let mut state = [
            0x61707865,
            0x3320646e,
            0x79622d32,
            0x6b206574,
            from_le_bytes(&self.key[0..4]),
            from_le_bytes(&self.key[4..8]),
            from_le_bytes(&self.key[8..12]),
            from_le_bytes(&self.key[12..16]),
            from_le_bytes(&self.key[16..20]),
            from_le_bytes(&self.key[20..24]),
            from_le_bytes(&self.key[24..28]),
            from_le_bytes(&self.key[28..]),
            counter,
            from_le_bytes(&nonce[0..4]),
            from_le_bytes(&nonce[4..8]),
            from_le_bytes(&nonce[8..12]),
        ];

        let mut working_state = state.clone();
        for _ in 0..(ROUNDS / 2) {
            working_state = double_round(working_state);
        }

        for (chunk, working_chunk) in state.iter_mut().zip(working_state) {
            *chunk = chunk.wrapping_add(working_chunk);
        }

        let mut result: Vec<u8> = Vec::new();

        for chunk in state {
            result.extend_from_slice(&chunk.to_le_bytes());
        }

        result
    }

    fn encrypt(&self, plaintext: &[u8], nonce: &[u8], counter: u32) -> Vec<u8> {
        let mut ciphertext: Vec<u8> = Vec::new();

        for (index, block) in plaintext.chunks(64).enumerate() {
            let keystream = self.keystream(nonce, counter + index as u32);

            for (key, chunk) in block.iter().zip(keystream) {
                ciphertext.push(chunk ^ key);
            }
        }

        ciphertext
    }
}

// ChaCha20-Poly1305 implementation
#[pyclass]
struct ChaCha20Poly1305 {
    key: Vec<u8>,
}

#[pymethods]
impl ChaCha20Poly1305 {
    #[new]
    pub fn new(key: Vec<u8>) -> ChaCha20Poly1305 {
        ChaCha20Poly1305 { key }
    }

    pub fn encrypt(&self, plaintext: &[u8], nonce: &[u8], aead: &[u8], counter: u32) -> Vec<u8> {
        let chacha = ChaCha20::new(self.key.clone());

        let otk = &chacha.keystream(nonce, 0);
        let poly1305_key = otk[..32].to_vec();

        let mut poly1305 = Poly1305::new(poly1305_key);
        let ciphertext = chacha.encrypt(plaintext, nonce, counter);

        poly1305.update(&aead, true);
        poly1305.update(&ciphertext, true);

        let aead_len = aead.len() as u64;
        let ciphertext_len = ciphertext.len() as u64;

        poly1305.update(&aead_len.to_le_bytes(), false);
        poly1305.update(&ciphertext_len.to_le_bytes(), false);

        [ciphertext, poly1305.tag()].concat().into()
    }

    pub fn decrypt(
        &self,
        text: &[u8],
        nonce: &[u8],
        aead: &[u8],
        counter: u32,
    ) -> PyResult<Vec<u8>> {
        if text.len() < 17 {
            return Err(PyAssertionError::new_err("Invalid ciphertext"));
        }

        let ciphertext = &text[..text.len() - 16];
        let tag = &text[text.len() - 16..];
        let chacha = ChaCha20::new(self.key.clone());

        let otk = &chacha.keystream(nonce, 0);
        let poly1305_key = otk[..32].to_vec();

        let mut poly1305 = Poly1305::new(poly1305_key);
        let plaintext = chacha.encrypt(ciphertext, nonce, counter);

        poly1305.update(&aead, true);
        poly1305.update(&ciphertext, true);

        let aead_len = aead.len() as u64;
        let ciphertext_len = ciphertext.len() as u64;

        poly1305.update(&aead_len.to_le_bytes(), false);
        poly1305.update(&ciphertext_len.to_le_bytes(), false);

        if poly1305.verify(tag) {
            return Ok(plaintext.to_vec());
        }

        Err(PyAssertionError::new_err("Invalid MAC"))
    }
}

fn hchacha20(key: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut state = [
        0x61707865,
        0x3320646e,
        0x79622d32,
        0x6b206574,
        from_le_bytes(&key[0..4]),
        from_le_bytes(&key[4..8]),
        from_le_bytes(&key[8..12]),
        from_le_bytes(&key[12..16]),
        from_le_bytes(&key[16..20]),
        from_le_bytes(&key[20..24]),
        from_le_bytes(&key[24..28]),
        from_le_bytes(&key[28..]),
        from_le_bytes(&nonce[0..4]),
        from_le_bytes(&nonce[4..8]),
        from_le_bytes(&nonce[8..12]),
        from_le_bytes(&nonce[12..]),
    ];

    for _ in 0..(ROUNDS / 2) {
        state = double_round(state);
    }

    let mut result: Vec<u8> = Vec::new();

    for chunk in state[0..4].iter().chain(state[12..16].iter()) {
        result.extend_from_slice(&chunk.to_le_bytes());
    }

    result
}

#[pyclass]
struct XChaCha20Poly1305 {
    key: Vec<u8>,
}

#[pymethods]
impl XChaCha20Poly1305 {
    #[new]
    fn new(key: Vec<u8>) -> XChaCha20Poly1305 {
        XChaCha20Poly1305 { key }
    }

    fn key(&self, nonce: &[u8]) -> (Vec<u8>, [u8; 12]) {
        let mut chacha_nonce = [0u8; 12];
        chacha_nonce[4..].copy_from_slice(&nonce[16..24]);

        let subkey = hchacha20(&self.key, &nonce[..16]);

        (subkey, chacha_nonce)
    }

    fn encrypt(&self, plaintext: &[u8], nonce: &[u8], aead: &[u8], counter: u32) -> Cow<[u8]> {
        let (subkey, chacha_nonce) = self.key(nonce);

        let chacha = ChaCha20Poly1305::new(subkey);

        chacha
            .encrypt(plaintext, &chacha_nonce, aead, counter)
            .into()
    }

    fn decrypt(
        &self,
        ciphertext: &[u8],
        nonce: &[u8],
        aead: &[u8],
        counter: u32,
    ) -> PyResult<Cow<[u8]>> {
        let (subkey, chacha_nonce) = self.key(nonce);

        let chacha = ChaCha20Poly1305::new(subkey);

        let output = chacha.decrypt(ciphertext, &chacha_nonce, aead, counter);

        match output {
            Ok(output) => Ok(output.into()),
            Err(e) => Err(e),
        }
    }
}

#[pyfunction]
fn keygen() -> Vec<u8> {
    randbytes::<32>().to_vec()
}

#[pymodule]
pub fn chacha(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(keygen, m)?)?;
    m.add_class::<ChaCha20Poly1305>()?;
    m.add_class::<XChaCha20Poly1305>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_chacha() {
        let key = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];

        let nonce = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00,
        ];

        let expected_output = [
            0x6e, 0x2e, 0x35, 0x9a, 0x25, 0x68, 0xf9, 0x80, 0x41, 0xba, 0x07, 0x28, 0xdd, 0x0d,
            0x69, 0x81, 0xe9, 0x7e, 0x7a, 0xec, 0x1d, 0x43, 0x60, 0xc2, 0x0a, 0x27, 0xaf, 0xcc,
            0xfd, 0x9f, 0xae, 0x0b, 0xf9, 0x1b, 0x65, 0xc5, 0x52, 0x47, 0x33, 0xab, 0x8f, 0x59,
            0x3d, 0xab, 0xcd, 0x62, 0xb3, 0x57, 0x16, 0x39, 0xd6, 0x24, 0xe6, 0x51, 0x52, 0xab,
            0x8f, 0x53, 0x0c, 0x35, 0x9f, 0x08, 0x61, 0xd8, 0x07, 0xca, 0x0d, 0xbf, 0x50, 0x0d,
            0x6a, 0x61, 0x56, 0xa3, 0x8e, 0x08, 0x8a, 0x22, 0xb6, 0x5e, 0x52, 0xbc, 0x51, 0x4d,
            0x16, 0xcc, 0xf8, 0x06, 0x81, 0x8c, 0xe9, 0x1a, 0xb7, 0x79, 0x37, 0x36, 0x5a, 0xf9,
            0x0b, 0xbf, 0x74, 0xa3, 0x5b, 0xe6, 0xb4, 0x0b, 0x8e, 0xed, 0xf2, 0x78, 0x5e, 0x42,
            0x87, 0x4d,
        ];

        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

        let counter = 1u32;

        let chacha = ChaCha20::new(key.to_vec());

        let output = chacha.encrypt(plaintext, &nonce, counter);

        assert_eq!(output, expected_output);
    }

    #[test]
    fn test_chacha_aead() {
        let key = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];

        let nonce = [
            0x07, 0x00, 0x00, 0x00, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
            0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
        ];

        let aead = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];

        let expected_output = [
            0xd3, 0x1a, 0x8d, 0x34, 0x64, 0x8e, 0x60, 0xdb, 0x7b, 0x86, 0xaf, 0xbc, 0x53, 0xef,
            0x7e, 0xc2, 0xa4, 0xad, 0xed, 0x51, 0x29, 0x6e, 0x08, 0xfe, 0xa9, 0xe2, 0xb5, 0xa7,
            0x36, 0xee, 0x62, 0xd6, 0x3d, 0xbe, 0xa4, 0x5e, 0x8c, 0xa9, 0x67, 0x12, 0x82, 0xfa,
            0xfb, 0x69, 0xda, 0x92, 0x72, 0x8b, 0x1a, 0x71, 0xde, 0x0a, 0x9e, 0x06, 0x0b, 0x29,
            0x05, 0xd6, 0xa5, 0xb6, 0x7e, 0xcd, 0x3b, 0x36, 0x92, 0xdd, 0xbd, 0x7f, 0x2d, 0x77,
            0x8b, 0x8c, 0x98, 0x03, 0xae, 0xe3, 0x28, 0x09, 0x1b, 0x58, 0xfa, 0xb3, 0x24, 0xe4,
            0xfa, 0xd6, 0x75, 0x94, 0x55, 0x85, 0x80, 0x8b, 0x48, 0x31, 0xd7, 0xbc, 0x3f, 0xf4,
            0xde, 0xf0, 0x8e, 0x4b, 0x7a, 0x9d, 0xe5, 0x76, 0xd2, 0x65, 0x86, 0xce, 0xc6, 0x4b,
            0x61, 0x16, 0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb,
            0xd0, 0x60, 0x06, 0x91,
        ];

        let counter = 1u32;

        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

        let chacha = ChaCha20Poly1305::new(key.to_vec());
        let output = chacha.encrypt(plaintext, &nonce, &aead, counter);

        assert_eq!(output, expected_output.to_vec());
    }

    #[test]
    fn test_hchacha() {
        let key = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];

        let nonce = [
            0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00, 0x31, 0x41,
            0x59, 0x27,
        ];

        let expected_output = [
            130, 65, 59, 66, 39, 178, 123, 254, 211, 14, 66, 80, 138, 135, 125, 115, 160, 249, 228,
            213, 138, 116, 168, 83, 193, 46, 196, 19, 38, 211, 236, 220,
        ];

        let output = hchacha20(&key, &nonce);

        assert_eq!(output, expected_output.to_vec());
    }

    #[test]
    fn test_xchacha() {
        let key = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];
        let nonce = [
            0xf2, 0x8a, 0x50, 0xa7, 0x8a, 0x7e, 0x23, 0xc9, 0xcb, 0xa6, 0x78, 0x34, 0x66, 0xf8,
            0x03, 0x59, 0x0f, 0x04, 0xe9, 0x22, 0x31, 0xa3, 0x2d, 0x5d,
        ];
        let aad = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let plaintext = [
            0x4c, 0x61, 0x64, 0x69, 0x65, 0x73, 0x20, 0x61, 0x6e, 0x64, 0x20, 0x47, 0x65, 0x6e,
            0x74, 0x6c, 0x65, 0x6d, 0x65, 0x6e, 0x20, 0x6f, 0x66, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x63, 0x6c, 0x61, 0x73, 0x73, 0x20, 0x6f, 0x66, 0x20, 0x27, 0x39, 0x39, 0x3a, 0x20,
            0x49, 0x66, 0x20, 0x49, 0x20, 0x63, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x6f, 0x66, 0x66,
            0x65, 0x72, 0x20, 0x79, 0x6f, 0x75, 0x20, 0x6f, 0x6e, 0x6c, 0x79, 0x20, 0x6f, 0x6e,
            0x65, 0x20, 0x74, 0x69, 0x70, 0x20, 0x66, 0x6f, 0x72, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x66, 0x75, 0x74, 0x75, 0x72, 0x65, 0x2c, 0x20, 0x73, 0x75, 0x6e, 0x73, 0x63, 0x72,
            0x65, 0x65, 0x6e, 0x20, 0x77, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x62, 0x65, 0x20, 0x69,
            0x74, 0x2e,
        ];
        let expected_ct = [
            0x20, 0xf1, 0xae, 0x75, 0xe1, 0xe5, 0xe0, 0x00, 0x40, 0x29, 0x4f, 0x0f, 0xb1, 0x0e,
            0xbb, 0x08, 0x10, 0xc5, 0x93, 0xc7, 0xdb, 0xa4, 0xec, 0x10, 0x4c, 0x1e, 0x5e, 0xf9,
            0x50, 0x7f, 0xae, 0xef, 0x58, 0xfc, 0x28, 0x98, 0xbb, 0xd0, 0xe4, 0x7b, 0x2f, 0x53,
            0x31, 0xfb, 0xc3, 0x67, 0xd3, 0xc2, 0x78, 0x4e, 0x36, 0x48, 0xce, 0x1e, 0xaa, 0x77,
            0x87, 0xad, 0x18, 0x6d, 0xb2, 0x68, 0x5e, 0xe8, 0x9a, 0xe4, 0xd3, 0x44, 0x1f, 0x6e,
            0xa0, 0xb2, 0x22, 0x4c, 0xd5, 0xa1, 0x34, 0x16, 0x1b, 0x55, 0x4d, 0x8b, 0x48, 0x35,
            0x0b, 0x4a, 0xd4, 0x01, 0x15, 0xdb, 0x81, 0xea, 0x82, 0x09, 0x68, 0xe9, 0x43, 0x89,
            0x2f, 0x2b, 0x80, 0x51, 0xcb, 0x5f, 0x7a, 0x86, 0x66, 0xe7, 0xe7, 0xef, 0x7f, 0x84,
            0xc0, 0xa2, 0xf8, 0x0a, 0x12, 0xd0, 0x66, 0x80, 0xc8, 0xee, 0xbb, 0xd9, 0x30, 0x04,
            0x10, 0x9d, 0xe8, 0x42,
        ];

        let xchacha = XChaCha20Poly1305::new(key.to_vec());
        let output = xchacha.encrypt(&plaintext, &nonce, &aad, 1);

        assert_eq!(output, expected_ct.to_vec());
    }

    #[test]
    fn test_tag() {
        let key = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];

        let nonce = [
            0xf2, 0x8a, 0x50, 0xa7, 0x8a, 0x7e, 0x23, 0xc9, 0xcb, 0xa6, 0x78, 0x34, 0x66, 0xf8,
            0x03, 0x59, 0x0f, 0x04, 0xe9, 0x22, 0x31, 0xa3, 0x2d, 0x5d,
        ];

        let aad = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];

        let false_aad = [0x04, 0x03, 0x02, 0x01];

        let plaintext = [
            0x4c, 0x61, 0x64, 0x69, 0x65, 0x73, 0x20, 0x61, 0x6e, 0x64, 0x20, 0x47, 0x65, 0x6e,
            0x74, 0x6c, 0x65, 0x6d, 0x65, 0x6e, 0x20, 0x6f, 0x66, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x63, 0x6c, 0x61, 0x73, 0x73, 0x20, 0x6f, 0x66, 0x20, 0x27, 0x39, 0x39, 0x3a, 0x20,
            0x49, 0x66, 0x20, 0x49, 0x20, 0x63, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x6f, 0x66, 0x66,
            0x65, 0x72, 0x20, 0x79, 0x6f, 0x75, 0x20, 0x6f, 0x6e, 0x6c, 0x79, 0x20, 0x6f, 0x6e,
            0x65, 0x20, 0x74, 0x69, 0x70, 0x20, 0x66, 0x6f, 0x72, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x66, 0x75, 0x74, 0x75, 0x72, 0x65, 0x2c, 0x20, 0x73, 0x75, 0x6e, 0x73, 0x63, 0x72,
            0x65, 0x65, 0x6e, 0x20, 0x77, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x62, 0x65, 0x20, 0x69,
            0x74, 0x2e,
        ];

        let xchacha = XChaCha20Poly1305::new(key.to_vec());
        let output = xchacha.encrypt(&plaintext, &nonce, &aad, 1);

        let _ = match xchacha.decrypt(&output, &nonce, &false_aad, 1) {
            Ok(_) => Err(String::from("Tag checking failed")),
            Err(_) => Ok(()),
        };
    }

    #[test]
    fn test_decrypt() {
        let key = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];

        let nonce = [
            0xf2, 0x8a, 0x50, 0xa7, 0x8a, 0x7e, 0x23, 0xc9, 0xcb, 0xa6, 0x78, 0x34, 0x66, 0xf8,
            0x03, 0x59, 0x0f, 0x04, 0xe9, 0x22, 0x31, 0xa3, 0x2d, 0x5d,
        ];

        let aad = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];

        let ciphertext = [
            0x20, 0xf1, 0xae, 0x75, 0xe1, 0xe5, 0xe0, 0x00, 0x40, 0x29, 0x4f, 0x0f, 0xb1, 0x0e,
            0xbb, 0x08, 0x10, 0xc5, 0x93, 0xc7, 0xdb, 0xa4, 0xec, 0x10, 0x4c, 0x1e, 0x5e, 0xf9,
            0x50, 0x7f, 0xae, 0xef, 0x58, 0xfc, 0x28, 0x98, 0xbb, 0xd0, 0xe4, 0x7b, 0x2f, 0x53,
            0x31, 0xfb, 0xc3, 0x67, 0xd3, 0xc2, 0x78, 0x4e, 0x36, 0x48, 0xce, 0x1e, 0xaa, 0x77,
            0x87, 0xad, 0x18, 0x6d, 0xb2, 0x68, 0x5e, 0xe8, 0x9a, 0xe4, 0xd3, 0x44, 0x1f, 0x6e,
            0xa0, 0xb2, 0x22, 0x4c, 0xd5, 0xa1, 0x34, 0x16, 0x1b, 0x55, 0x4d, 0x8b, 0x48, 0x35,
            0x0b, 0x4a, 0xd4, 0x01, 0x15, 0xdb, 0x81, 0xea, 0x82, 0x09, 0x68, 0xe9, 0x43, 0x89,
            0x2f, 0x2b, 0x80, 0x51, 0xcb, 0x5f, 0x7a, 0x86, 0x66, 0xe7, 0xe7, 0xef, 0x7f, 0x84,
            0xc0, 0xa2, 0xf8, 0x0a, 0x12, 0xd0, 0x66, 0x80, 0xc8, 0xee, 0xbb, 0xd9, 0x30, 0x04,
            0x10, 0x9d, 0xe8, 0x42,
        ];

        let plaintext = [
            0x4c, 0x61, 0x64, 0x69, 0x65, 0x73, 0x20, 0x61, 0x6e, 0x64, 0x20, 0x47, 0x65, 0x6e,
            0x74, 0x6c, 0x65, 0x6d, 0x65, 0x6e, 0x20, 0x6f, 0x66, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x63, 0x6c, 0x61, 0x73, 0x73, 0x20, 0x6f, 0x66, 0x20, 0x27, 0x39, 0x39, 0x3a, 0x20,
            0x49, 0x66, 0x20, 0x49, 0x20, 0x63, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x6f, 0x66, 0x66,
            0x65, 0x72, 0x20, 0x79, 0x6f, 0x75, 0x20, 0x6f, 0x6e, 0x6c, 0x79, 0x20, 0x6f, 0x6e,
            0x65, 0x20, 0x74, 0x69, 0x70, 0x20, 0x66, 0x6f, 0x72, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x66, 0x75, 0x74, 0x75, 0x72, 0x65, 0x2c, 0x20, 0x73, 0x75, 0x6e, 0x73, 0x63, 0x72,
            0x65, 0x65, 0x6e, 0x20, 0x77, 0x6f, 0x75, 0x6c, 0x64, 0x20, 0x62, 0x65, 0x20, 0x69,
            0x74, 0x2e,
        ];

        let xchacha = XChaCha20Poly1305::new(key.to_vec());

        match xchacha.decrypt(&ciphertext, &nonce, &aad, 1) {
            Ok(pt) => assert_eq!(pt, plaintext.to_vec()),
            Err(_) => panic!("Decryption failed"),
        }
    }
}
