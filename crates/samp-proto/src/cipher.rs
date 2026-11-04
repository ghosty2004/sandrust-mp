//! Datagram obfuscation applied by 0.3.7 clients to every client→server datagram.
//!
//! Spec: docs/re/handshake.md §2. Server→client datagrams are plaintext.
//!
//! Wire = 1 checksum byte followed by the substituted payload:
//! - checksum = XOR of `(b & 0xAA)` over all plaintext bytes
//! - every byte goes through [`SBOX`]; odd positions are additionally
//!   XORed with the key byte `(port & 0xFF) ^ 0xCC`.

use crate::sbox::SBOX;

const fn invert(table: &[u8; 256]) -> [u8; 256] {
    let mut inv = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        inv[table[i] as usize] = i as u8;
        i += 1;
    }
    inv
}

const SBOX_INV: [u8; 256] = invert(&SBOX);

/// Key byte derived from the server port the client is connecting to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherKey(u8);

impl CipherKey {
    pub fn from_port(port: u16) -> Self {
        Self(((port ^ 0xCCCC) & 0xFF) as u8)
    }

    pub fn byte(self) -> u8 {
        self.0
    }
}

fn checksum(plain: &[u8]) -> u8 {
    plain.iter().fold(0, |acc, &b| acc ^ (b & 0xAA))
}

/// Obfuscate a plaintext datagram (client→server direction).
pub fn encrypt(plain: &[u8], key: CipherKey, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(plain.len() + 1);
    out.push(checksum(plain));
    out.extend(plain.iter().enumerate().map(|(i, &b)| {
        let t = SBOX[b as usize];
        if i % 2 == 1 { t ^ key.0 } else { t }
    }));
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecryptError {
    Empty,
    BadChecksum { expected: u8, actual: u8 },
}

/// Reverse [`encrypt`]. Used by tests and by tooling that inspects client traffic.
pub fn decrypt(wire: &[u8], key: CipherKey, out: &mut Vec<u8>) -> Result<(), DecryptError> {
    let (&sum, body) = wire.split_first().ok_or(DecryptError::Empty)?;
    out.clear();
    out.extend(body.iter().enumerate().map(|(i, &b)| {
        let t = if i % 2 == 1 { b ^ key.0 } else { b };
        SBOX_INV[t as usize]
    }));
    let actual = checksum(out);
    if actual != sum {
        return Err(DecryptError::BadChecksum { expected: sum, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_from_port() {
        assert_eq!(CipherKey::from_port(7777).byte(), 0xAD);
    }

    /// Worked example from docs/re/handshake.md §2.
    #[test]
    fn spec_example_cookie_request() {
        let mut out = Vec::new();
        encrypt(&[0x18, 0x69, 0x69], CipherKey::from_port(7777), &mut out);
        assert_eq!(out, [0x08, 0x1e, 0x77, 0xda]);
    }

    #[test]
    fn roundtrip_all_bytes() {
        let plain: Vec<u8> = (0..=255).chain((0..=255).rev()).collect();
        for port in [7777u16, 8192, 1, 65535] {
            let key = CipherKey::from_port(port);
            let (mut wire, mut back) = (Vec::new(), Vec::new());
            encrypt(&plain, key, &mut wire);
            assert_eq!(wire.len(), plain.len() + 1);
            decrypt(&wire, key, &mut back).unwrap();
            assert_eq!(back, plain);
        }
    }

    #[test]
    fn checksum_mismatch_detected() {
        let key = CipherKey::from_port(7777);
        let mut wire = Vec::new();
        encrypt(&[1, 2, 3], key, &mut wire);
        wire[0] ^= 0x02;
        assert!(matches!(decrypt(&wire, key, &mut Vec::new()), Err(DecryptError::BadChecksum { .. })));
    }
}
