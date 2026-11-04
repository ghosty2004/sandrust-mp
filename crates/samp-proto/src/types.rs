//! Small shared value types and string helpers used by RPC and sync codecs.
//!
//! Strings on the SA:MP wire are raw bytes in the Windows code page (usually
//! Windows-1252), not UTF-8, so they are kept as `Vec<u8>`.

use crate::bitstream::{BitReader, BitWriter};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Quaternion in the order the client uses on the wire: w, x, y, z.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec2 {
    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { x: r.read_f32()?, y: r.read_f32()? })
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_f32(self.x);
        w.write_f32(self.y);
    }
}

impl Vec3 {
    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { x: r.read_f32()?, y: r.read_f32()?, z: r.read_f32()? })
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_f32(self.x);
        w.write_f32(self.y);
        w.write_f32(self.z);
    }
}

impl Quat {
    /// Four raw f32 in w, x, y, z order.
    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { w: r.read_f32()?, x: r.read_f32()?, y: r.read_f32()?, z: r.read_f32()? })
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_f32(self.w);
        w.write_f32(self.x);
        w.write_f32(self.y);
        w.write_f32(self.z);
    }
}

/// String with a u8 length prefix.
pub fn read_str8(r: &mut BitReader) -> Option<Vec<u8>> {
    let n = r.read_u8()?;
    r.read_bytes(n.into())
}
/// String with a u16 length prefix.
pub fn read_str16(r: &mut BitReader) -> Option<Vec<u8>> {
    let n = r.read_u16()?;
    r.read_bytes(n.into())
}
/// String with a u32 length prefix.
pub fn read_str32(r: &mut BitReader) -> Option<Vec<u8>> {
    let n = r.read_u32()?;
    r.read_bytes(usize::try_from(n).ok()?)
}

pub fn write_str8(w: &mut BitWriter, s: &[u8]) {
    let s = &s[..s.len().min(u8::MAX.into())];
    w.write_u8(s.len() as u8);
    w.write_bytes(s);
}
pub fn write_str16(w: &mut BitWriter, s: &[u8]) {
    let s = &s[..s.len().min(u16::MAX.into())];
    w.write_u16(s.len() as u16);
    w.write_bytes(s);
}
pub fn write_str32(w: &mut BitWriter, s: &[u8]) {
    w.write_u32(s.len() as u32);
    w.write_bytes(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_roundtrip() {
        let mut w = BitWriter::new();
        w.write_bit(true);
        write_str8(&mut w, b"abc");
        write_str32(&mut w, b"hello");
        let mut r = BitReader::new(w.as_bytes());
        r.read_bit();
        assert_eq!(read_str8(&mut r).as_deref(), Some(&b"abc"[..]));
        assert_eq!(read_str32(&mut r).as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn oversized_length_prefix_is_rejected() {
        // A u32 length far beyond the input must fail (without allocating it).
        let mut r = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, b'a']);
        assert_eq!(read_str32(&mut r), None);
    }
}
