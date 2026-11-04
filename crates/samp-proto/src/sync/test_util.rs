//! Test helpers: build expected wire data by hand, independently of `BitWriter`.

/// Parse a string of `0`/`1` digits (whitespace, `_` and `|` ignored) into
/// MSB-first packed bytes; returns the bytes and the bit count.
pub fn bits(s: &str) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut n = 0usize;
    for ch in s.chars() {
        let bit = match ch {
            '0' => false,
            '1' => true,
            c if c.is_whitespace() || c == '_' || c == '|' => continue,
            c => panic!("bad bit char {c:?}"),
        };
        if n.is_multiple_of(8) {
            out.push(0);
        }
        if bit {
            *out.last_mut().unwrap() |= 0x80 >> (n % 8);
        }
        n += 1;
    }
    (out, n)
}

/// Render bytes as a bit string (each byte MSB first), for composing [`bits`] input.
pub fn b(bytes: &[u8]) -> String {
    bytes.iter().map(|x| format!("{x:08b} ")).collect()
}

/// Append little-endian bytes of the given values to a byte vector.
#[derive(Default)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i16(self, v: i16) -> Self {
        self.u16(v as u16)
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32s(mut self, vs: &[f32]) -> Self {
        for v in vs {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        self
    }
}
