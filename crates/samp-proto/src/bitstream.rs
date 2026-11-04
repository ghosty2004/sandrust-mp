//! Bit-level reader/writer matching the conventions used on the SA:MP wire.
//!
//! Spec: docs/re/handshake.md ("Conventions" and §3):
//! - bits are filled most-significant-bit first within each byte;
//! - multi-byte integers are little-endian and are NOT byte-aligned;
//! - "compressed" integers drop leading zero bytes (see [`BitWriter::write_compressed`]).

/// Append-only bit writer.
#[derive(Clone, Debug, Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of bits written so far.
    pub fn bit_len(&self) -> usize {
        self.bits
    }

    /// Written bytes; the last byte is zero-padded if `bit_len` is not a multiple of 8.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn write_bit(&mut self, bit: bool) {
        let offset = self.bits % 8;
        if offset == 0 {
            self.buf.push(0);
        }
        if bit {
            *self.buf.last_mut().unwrap() |= 0x80 >> offset;
        }
        self.bits += 1;
    }

    /// Write the low `n` bits of `value` (n ≤ 32), most significant first.
    pub fn write_bits(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32);
        for i in (0..n).rev() {
            self.write_bit((value >> i) & 1 == 1);
        }
    }

    /// Write whole bytes at the current (possibly unaligned) bit position.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if self.bits.is_multiple_of(8) {
            self.buf.extend_from_slice(bytes);
            self.bits += bytes.len() * 8;
        } else {
            for &b in bytes {
                self.write_bits(b.into(), 8);
            }
        }
    }

    /// Copy the first `n` bits of `src` (MSB-first order).
    pub fn write_bits_from(&mut self, src: &[u8], n: usize) {
        let mut r = BitReader::new(src);
        for _ in 0..n {
            self.write_bit(r.read_bit().expect("source shorter than bit count"));
        }
    }

    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }
    /// Two's-complement i16, little-endian (same bits as `write_u16`).
    pub fn write_i16(&mut self, v: i16) {
        self.write_bytes(&v.to_le_bytes());
    }
    /// Two's-complement i32, little-endian (same bits as `write_u32`).
    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_f32(&mut self, v: f32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Pad with zero bits up to the next byte boundary.
    pub fn align(&mut self) {
        self.bits = self.buf.len() * 8;
    }

    /// Align, then write the bytes.
    pub fn write_aligned_bytes(&mut self, bytes: &[u8]) {
        self.align();
        self.write_bytes(bytes);
    }

    /// Unsigned compressed integer, given its little-endian bytes.
    ///
    /// From the most significant byte down to the second-least significant:
    /// a zero byte is written as a single `1` bit; at the first non-zero byte
    /// a `0` bit is written followed by that byte and all lower bytes (in
    /// little-endian order), and encoding stops. If every upper byte was zero,
    /// the last byte is written as `1` + its low nibble when its high nibble is
    /// zero, otherwise `0` + the full byte.
    pub fn write_compressed(&mut self, le: &[u8]) {
        for k in (1..le.len()).rev() {
            if le[k] == 0 {
                self.write_bit(true);
            } else {
                self.write_bit(false);
                self.write_bytes(&le[..=k]);
                return;
            }
        }
        let last = le[0];
        if last & 0xF0 == 0 {
            self.write_bit(true);
            self.write_bits(last.into(), 4);
        } else {
            self.write_bit(false);
            self.write_bits(last.into(), 8);
        }
    }

    pub fn write_compressed_u16(&mut self, v: u16) {
        self.write_compressed(&v.to_le_bytes());
    }
    pub fn write_compressed_u32(&mut self, v: u32) {
        self.write_compressed(&v.to_le_bytes());
    }
}

/// Bit reader over a byte slice. All reads return `None` when data runs out.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, end: data.len() * 8 }
    }

    /// Reader limited to the first `bits` bits of `data`.
    pub fn with_bit_len(data: &'a [u8], bits: usize) -> Self {
        Self { data, pos: 0, end: bits.min(data.len() * 8) }
    }

    pub fn position(&self) -> usize {
        self.pos
    }
    pub fn remaining(&self) -> usize {
        self.end - self.pos
    }
    pub fn skip(&mut self, n: usize) -> Option<()> {
        (self.remaining() >= n).then(|| self.pos += n)
    }

    pub fn read_bit(&mut self) -> Option<bool> {
        if self.pos >= self.end {
            return None;
        }
        let bit = self.data[self.pos / 8] & (0x80 >> (self.pos % 8)) != 0;
        self.pos += 1;
        Some(bit)
    }

    /// Read `n` bits (n ≤ 32) into the low bits of the result, MSB first.
    pub fn read_bits(&mut self, n: u32) -> Option<u32> {
        debug_assert!(n <= 32);
        if self.remaining() < n as usize {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read_bit()? as u32;
        }
        Some(v)
    }

    pub fn read_bytes_into(&mut self, out: &mut [u8]) -> Option<()> {
        if self.remaining() < out.len() * 8 {
            return None;
        }
        if self.pos.is_multiple_of(8) {
            let start = self.pos / 8;
            out.copy_from_slice(&self.data[start..start + out.len()]);
            self.pos += out.len() * 8;
        } else {
            for b in out.iter_mut() {
                *b = self.read_bits(8)? as u8;
            }
        }
        Some(())
    }

    /// Read `n` whole bytes. Fails before allocating when fewer than `n`
    /// bytes remain, so a hostile length prefix cannot force a huge allocation.
    pub fn read_bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.remaining() / 8 < n {
            return None;
        }
        let mut v = vec![0; n];
        self.read_bytes_into(&mut v)?;
        Some(v)
    }

    /// Read `n` bits into a new buffer (MSB-first packing, last byte zero-padded).
    pub fn read_bits_to_vec(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.remaining() < n {
            return None;
        }
        let mut w = BitWriter::new();
        for _ in 0..n {
            w.write_bit(self.read_bit()?);
        }
        Some(w.into_bytes())
    }

    fn read_array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut a = [0u8; N];
        self.read_bytes_into(&mut a)?;
        Some(a)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        self.read_array::<1>().map(|a| a[0])
    }
    pub fn read_u16(&mut self) -> Option<u16> {
        self.read_array().map(u16::from_le_bytes)
    }
    pub fn read_u32(&mut self) -> Option<u32> {
        self.read_array().map(u32::from_le_bytes)
    }
    pub fn read_i16(&mut self) -> Option<i16> {
        self.read_array().map(i16::from_le_bytes)
    }
    pub fn read_i32(&mut self) -> Option<i32> {
        self.read_array().map(i32::from_le_bytes)
    }
    pub fn read_f32(&mut self) -> Option<f32> {
        self.read_array().map(f32::from_le_bytes)
    }

    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
        self.pos = self.pos.min(self.end);
    }

    pub fn read_aligned_bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        self.align();
        self.read_bytes(n)
    }

    /// Inverse of [`BitWriter::write_compressed`] for an `N`-byte unsigned value.
    pub fn read_compressed<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        for k in (1..N).rev() {
            if !self.read_bit()? {
                self.read_bytes_into(&mut out[..=k])?;
                return Some(out);
            }
        }
        out[0] = if self.read_bit()? { self.read_bits(4)? as u8 } else { self.read_bits(8)? as u8 };
        Some(out)
    }

    pub fn read_compressed_u16(&mut self) -> Option<u16> {
        self.read_compressed().map(u16::from_le_bytes)
    }
    pub fn read_compressed_u32(&mut self) -> Option<u32> {
        self.read_compressed().map(u32::from_le_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msb_first_bits() {
        let mut w = BitWriter::new();
        w.write_bit(true);
        w.write_bits(0b0101, 4);
        assert_eq!(w.as_bytes(), [0b1010_1000]);
        assert_eq!(w.bit_len(), 5);
    }

    #[test]
    fn unaligned_u16_is_little_endian() {
        let mut w = BitWriter::new();
        w.write_bit(true);
        w.write_u16(0x1234);
        // 1 | 00110100 | 00010010 -> 1001_1010 0000_1001 0000_0000
        assert_eq!(w.as_bytes(), [0x9a, 0x09, 0x00]);
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(r.read_bit(), Some(true));
        assert_eq!(r.read_u16(), Some(0x1234));
    }

    #[test]
    fn compressed_encodings() {
        let enc = |v: u16| {
            let mut w = BitWriter::new();
            w.write_compressed_u16(v);
            (w.bit_len(), w.into_bytes())
        };
        assert_eq!(enc(5), (6, vec![0b1101_0100])); // 1 (hi=0) 1 (nibble) 0101
        assert_eq!(enc(0xAB).0, 1 + 1 + 8);
        assert_eq!(enc(0x0100).0, 1 + 16);
    }

    #[test]
    fn compressed_roundtrip() {
        for v in [0u32, 1, 15, 16, 255, 256, 0xFFFF, 0x10000, 0x00FF_0000, u32::MAX] {
            let mut w = BitWriter::new();
            w.write_bit(true); // force misalignment
            w.write_compressed_u32(v);
            let mut r = BitReader::new(w.as_bytes());
            r.read_bit();
            assert_eq!(r.read_compressed_u32(), Some(v), "value {v:#x}");
        }
    }

    #[test]
    fn aligned_bytes() {
        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        w.write_aligned_bytes(&[0xde, 0xad]);
        assert_eq!(w.as_bytes(), [0xa0, 0xde, 0xad]);
        let mut r = BitReader::new(w.as_bytes());
        r.read_bits(3);
        assert_eq!(r.read_aligned_bytes(2), Some(vec![0xde, 0xad]));
    }

    #[test]
    fn short_read_is_none() {
        let mut r = BitReader::new(&[0xff]);
        assert_eq!(r.read_u16(), None);
        assert_eq!(r.read_bits(8), Some(0xff));
        assert_eq!(r.read_bit(), None);
    }
}
