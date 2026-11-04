//! Decoding of one SCM command (opcode + typed parameters) as GTA's script
//! interpreter reads it. Parameter type bytes follow the public SA SCM format
//! (as used by gta-reversed `CRunningScript::CollectParameters` and the
//! Sanny Builder documentation): 1 = int32, 2 = global var, 3 = local var,
//! 4 = int8, 5 = int16, 6 = float32, 7/8 = global/local array,
//! 9 = 8-byte string, 0xA/0xB = global/local 8-byte-string var,
//! 0xC/0xD = their arrays, 0xE = length-prefixed string, 0xF = 16-byte string,
//! 0x10/0x11 = 16-byte-string vars, 0x12/0x13 = their arrays.

use crate::json::Val;

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Int(i32),
    Float(f32),
    Str(String),
    GlobalVar(u16),
    LocalVar(u16),
    Array { global: bool, offset: u16, index_var: u16 },
}

impl Param {
    pub fn to_val(&self) -> Val {
        match self {
            Param::Int(i) => Val::Int((*i).into()),
            Param::Float(f) => Val::Float(*f),
            Param::Str(s) => Val::Str(s.clone()),
            Param::GlobalVar(o) => Val::Str(format!("g{o}")),
            Param::LocalVar(i) => Val::Str(format!("l{i}")),
            Param::Array { global, offset, index_var } => {
                Val::Str(format!("{}{offset}[{index_var}]", if *global { "g" } else { "l" }))
            }
        }
    }
}

fn str_of(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Decodes parameters from `b` (the bytes after the opcode). Stops at the
/// end of the slice, at a 0 type byte, or at an unknown type. Returns the
/// parameters and whether decoding stopped cleanly.
pub fn decode_params(b: &[u8], max: usize) -> (Vec<Param>, bool) {
    let mut out = Vec::new();
    let mut i = 0;
    let rd16 = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let rd32 = |i: usize| b.get(i..i + 4).map(|s| [s[0], s[1], s[2], s[3]]);
    while i < b.len() && out.len() < max {
        let t = b[i];
        i += 1;
        let (p, n) = match t {
            0x01 => match rd32(i) {
                Some(x) => (Param::Int(i32::from_le_bytes(x)), 4),
                None => return (out, false),
            },
            0x02 | 0x0A | 0x10 => match rd16(i) {
                Some(x) => (Param::GlobalVar(x), 2),
                None => return (out, false),
            },
            0x03 | 0x0B | 0x11 => match rd16(i) {
                Some(x) => (Param::LocalVar(x), 2),
                None => return (out, false),
            },
            0x04 => match b.get(i) {
                Some(&x) => (Param::Int(i32::from(x as i8)), 1),
                None => return (out, false),
            },
            0x05 => match rd16(i) {
                Some(x) => (Param::Int(i32::from(x as i16)), 2),
                None => return (out, false),
            },
            0x06 => match rd32(i) {
                Some(x) => (Param::Float(f32::from_le_bytes(x)), 4),
                None => return (out, false),
            },
            0x07 | 0x08 | 0x0C | 0x0D | 0x12 | 0x13 => match (rd16(i), rd16(i + 2)) {
                (Some(offset), Some(index_var)) if i + 6 <= b.len() => {
                    (Param::Array { global: matches!(t, 0x07 | 0x0C | 0x12), offset, index_var }, 6)
                }
                _ => return (out, false),
            },
            0x09 => match b.get(i..i + 8) {
                Some(s) => (Param::Str(str_of(s)), 8),
                None => return (out, false),
            },
            0x0F => match b.get(i..i + 16) {
                Some(s) => (Param::Str(str_of(s)), 16),
                None => return (out, false),
            },
            0x0E => {
                let Some(&len) = b.get(i) else { return (out, false) };
                match b.get(i + 1..i + 1 + len as usize) {
                    Some(s) => (Param::Str(str_of(s)), 1 + len as usize),
                    None => return (out, false),
                }
            }
            0x00 => return (out, true),
            _ => return (out, false),
        };
        out.push(p);
        i += n;
    }
    (out, i == b.len())
}

/// Splits a raw opcode word into (opcode, NOT flag).
pub fn opcode(word: u16) -> (u16, bool) {
    (word & 0x7FFF, word & 0x8000 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_typed_params() {
        let mut b = vec![0x01];
        b.extend_from_slice(&123i32.to_le_bytes());
        b.push(0x06);
        b.extend_from_slice(&2.5f32.to_le_bytes());
        b.extend_from_slice(&[0x03, 0x02, 0x00]); // local var 2
        b.extend_from_slice(&[0x04, 0xFF]); // int8 -1
        b.extend_from_slice(&[0x05, 0x34, 0x12]); // int16
        b.push(0x09);
        b.extend_from_slice(b"intro\0\0\0");
        b.extend_from_slice(&[0x0E, 3, b'a', b'b', b'c']);
        let (p, clean) = decode_params(&b, 16);
        assert!(clean);
        assert_eq!(
            p,
            vec![
                Param::Int(123),
                Param::Float(2.5),
                Param::LocalVar(2),
                Param::Int(-1),
                Param::Int(0x1234),
                Param::Str("intro".into()),
                Param::Str("abc".into()),
            ]
        );
        assert_eq!(p[2].to_val().to_json(), "\"l2\"");
    }

    #[test]
    fn stops_on_end_or_garbage() {
        let (p, clean) = decode_params(&[0x04, 7, 0x00, 0x01], 16);
        assert_eq!((p, clean), (vec![Param::Int(7)], true));
        let (p, clean) = decode_params(&[0x04, 7, 0x77], 16);
        assert_eq!((p, clean), (vec![Param::Int(7)], false));
        let (p, clean) = decode_params(&[0x01, 1, 2], 16);
        assert_eq!((p, clean), (vec![], false));
        assert_eq!(opcode(0x8256), (0x0256, true));
    }
}
