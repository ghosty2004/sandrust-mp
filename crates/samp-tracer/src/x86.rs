//! Minimal 32-bit x86 instruction-length decoder and relocator, just enough
//! to steal the first ≥ 5 bytes of a function prologue for an inline hook.
//!
//! Supported: the integer/x87 instructions that appear in MSVC prologues
//! (push/pop, mov/lea/arith with ModRM, immediates, moffs, SEH frame setup
//! with the `fs:` prefix, `0x66` operand-size prefix) and `call`/`jmp rel32`,
//! which are re-encoded for the trampoline. Short or conditional branches and
//! returns inside the stolen range are rejected rather than relocated.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran past the end of the supplied bytes.
    Truncated,
    /// Opcode the decoder does not know (byte, offset).
    Unknown(u8, usize),
    /// A relative branch or return that cannot be moved (offset).
    NotRelocatable(usize),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "instruction runs past the buffer"),
            DecodeError::Unknown(b, o) => write!(f, "unknown opcode {b:#04x} at +{o}"),
            DecodeError::NotRelocatable(o) => write!(f, "relative branch/return at +{o} cannot be relocated"),
        }
    }
}

/// `call rel32` / `jmp rel32`: the 4-byte displacement starts at offset 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rel {
    Call,
    Jmp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub len: usize,
    pub rel: Option<Rel>,
}

fn byte(code: &[u8], i: usize) -> Result<u8, DecodeError> {
    code.get(i).copied().ok_or(DecodeError::Truncated)
}

/// Length of a ModRM operand (ModRM byte + SIB + displacement) at `code[i]`.
fn modrm_len(code: &[u8], i: usize) -> Result<(usize, u8), DecodeError> {
    let m = byte(code, i)?;
    let (md, reg, rm) = (m >> 6, (m >> 3) & 7, m & 7);
    let mut len = 1;
    if md == 3 {
        return Ok((len, reg));
    }
    if rm == 4 {
        let sib = byte(code, i + 1)?;
        len += 1;
        if md == 0 && sib & 7 == 5 {
            len += 4;
        }
    } else if md == 0 && rm == 5 {
        len += 4;
    }
    len += match md {
        1 => 1,
        2 => 4,
        _ => 0,
    };
    Ok((len, reg))
}

/// Decodes one instruction at the start of `code`.
pub fn decode(code: &[u8]) -> Result<Insn, DecodeError> {
    let mut i = 0;
    let mut op16 = false;
    loop {
        match byte(code, i)? {
            0x66 => op16 = true,
            0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 => {}
            _ => break,
        }
        i += 1;
    }
    let imm = if op16 { 2 } else { 4 };
    let op = byte(code, i)?;
    let start = i;
    i += 1;
    let plain = |n: usize| Ok(Insn { len: n, rel: None });
    let with_modrm = |extra: usize| -> Result<Insn, DecodeError> {
        let (l, _) = modrm_len(code, start + 1)?;
        Ok(Insn { len: start + 1 + l + extra, rel: None })
    };
    let r = match op {
        0x50..=0x5F | 0x90 | 0x60 | 0x61 | 0x9C | 0x9D | 0x99 | 0x98 | 0xFC | 0xCC => plain(i),
        0x6A => plain(i + 1),
        0x68 => plain(i + imm),
        0xB0..=0xB7 => plain(i + 1),
        0xB8..=0xBF => plain(i + imm),
        0xA0..=0xA3 => plain(i + 4),
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C | 0xA8 => plain(i + 1),
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D | 0xA9 => plain(i + imm),
        0x00..=0x03
        | 0x08..=0x0B
        | 0x10..=0x13
        | 0x18..=0x1B
        | 0x20..=0x23
        | 0x28..=0x2B
        | 0x30..=0x33
        | 0x38..=0x3B
        | 0x84..=0x8B
        | 0x8D
        | 0x8F
        | 0xD0..=0xD3
        | 0xD8..=0xDF
        | 0xFE
        | 0xFF => with_modrm(0),
        0x80 | 0x83 | 0x6B | 0xC0 | 0xC1 | 0xC6 => with_modrm(1),
        0x81 | 0x69 | 0xC7 => with_modrm(imm),
        0xF6 | 0xF7 => {
            let (l, reg) = modrm_len(code, i)?;
            let extra = if reg <= 1 { if op == 0xF6 { 1 } else { imm } } else { 0 };
            plain(i + l + extra)
        }
        0x0F => {
            let op2 = byte(code, i)?;
            i += 1;
            match op2 {
                0x80..=0x8F => Err(DecodeError::NotRelocatable(start)),
                0x90..=0x9F
                | 0xB6
                | 0xB7
                | 0xBE
                | 0xBF
                | 0xAF
                | 0x10
                | 0x11
                | 0x28
                | 0x29
                | 0x2E
                | 0x2F
                | 0x57
                | 0x58
                | 0x59
                | 0x5C
                | 0x5E => {
                    let (l, _) = modrm_len(code, i)?;
                    plain(i + l)
                }
                _ => Err(DecodeError::Unknown(op2, i - 1)),
            }
        }
        0xE8 => Ok(Insn { len: i + 4, rel: Some(Rel::Call) }),
        0xE9 => Ok(Insn { len: i + 4, rel: Some(Rel::Jmp) }),
        0x70..=0x7F | 0xEB | 0xE0..=0xE3 | 0xC2 | 0xC3 | 0xCA | 0xCB => Err(DecodeError::NotRelocatable(start)),
        _ => Err(DecodeError::Unknown(op, start)),
    }?;
    if r.len > code.len() {
        return Err(DecodeError::Truncated);
    }
    if r.rel.is_some() && start != 0 {
        // A prefixed rel32 branch would need different handling; never seen in prologues.
        return Err(DecodeError::Unknown(op, start));
    }
    Ok(r)
}

/// Number of whole-instruction bytes ≥ `min` at the start of `code`.
pub fn steal_len(code: &[u8], min: usize) -> Result<usize, DecodeError> {
    let mut n = 0;
    while n < min {
        n += decode(&code[n..])?.len;
    }
    Ok(n)
}

pub fn rel32_target(site: u32, len: u32, rel: i32) -> u32 {
    site.wrapping_add(len).wrapping_add(rel as u32)
}

pub fn rel32_to(site: u32, len: u32, target: u32) -> i32 {
    target.wrapping_sub(site.wrapping_add(len)) as i32
}

/// Copies the first ≥ `min` bytes of `code` (located at `src`) so that they
/// run correctly at `dst`; `call/jmp rel32` are re-targeted. Returns the
/// relocated bytes and the number of source bytes consumed. The caller
/// appends a `jmp src+len`.
pub fn relocate(code: &[u8], src: u32, dst: u32, min: usize) -> Result<(Vec<u8>, usize), DecodeError> {
    let mut out = Vec::new();
    let mut n = 0;
    while n < min {
        let insn = decode(&code[n..])?;
        let bytes = &code[n..n + insn.len];
        match insn.rel {
            Some(_) => {
                let rel = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
                let target = rel32_target(src + n as u32, 5, rel);
                let here = dst + out.len() as u32;
                out.push(bytes[0]);
                out.extend_from_slice(&rel32_to(here, 5, target).to_le_bytes());
            }
            None => out.extend_from_slice(bytes),
        }
        n += insn.len;
    }
    Ok((out, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(b: &[u8]) -> usize {
        decode(b).unwrap().len
    }

    #[test]
    fn lengths_of_prologue_instructions() {
        assert_eq!(len(&[0x53]), 1); // push ebx
        assert_eq!(len(&[0x83, 0xEC, 0x0C]), 3); // sub esp,0xc
        assert_eq!(len(&[0x8B, 0x5C, 0x24, 0x0C]), 4); // mov ebx,[esp+0xc]
        assert_eq!(len(&[0x6A, 0xFF]), 2); // push -1
        assert_eq!(len(&[0x68, 0x58, 0xED, 0x83, 0x00]), 5); // push imm32
        assert_eq!(len(&[0x64, 0xA1, 0, 0, 0, 0]), 6); // mov eax,fs:[0]
        assert_eq!(len(&[0x66, 0xFF, 0x05, 0xF4, 0x47, 0xA4, 0x00]), 7); // inc word [abs]
        assert_eq!(len(&[0x81, 0x4E, 0x1C, 0x80, 0, 0, 0]), 7); // or dword [esi+0x1c],0x80
        assert_eq!(len(&[0x8B, 0x86, 0x98, 0x05, 0, 0]), 6); // mov eax,[esi+0x598]
        assert_eq!(len(&[0xA0, 0x88, 0x30, 0xA4, 0x00]), 5); // mov al,[abs]
        assert_eq!(len(&[0x69, 0xF6, 0x90, 0x01, 0, 0]), 6); // imul esi,esi,0x190
        assert_eq!(len(&[0xC6, 0x41, 0x3D, 0x01]), 4); // mov byte [ecx+0x3d],1
        assert_eq!(len(&[0x0F, 0x94, 0xC0]), 3); // sete al
        assert_eq!(len(&[0xF6, 0xC4, 0x06]), 3); // test ah,6
        assert_eq!(len(&[0xF7, 0xD8]), 2); // neg eax (no immediate)
        assert_eq!(len(&[0xD9, 0x40, 0x08]), 3); // fld dword [eax+8]
        assert_eq!(len(&[0x8D, 0x3C, 0xB6]), 3); // lea edi,[esi+esi*4]
        assert_eq!(len(&[0x8B, 0x04, 0x25, 1, 2, 3, 4]), 7); // SIB with disp32, no base
        assert_eq!(len(&[0x80, 0x79, 0x3D, 0x01]), 4); // cmp byte [ecx+0x3d],1
    }

    #[test]
    fn rejects_unmovable() {
        assert_eq!(decode(&[0x74, 0x05]), Err(DecodeError::NotRelocatable(0)));
        assert_eq!(decode(&[0xC3]), Err(DecodeError::NotRelocatable(0)));
        assert_eq!(decode(&[0x0F, 0x84, 0, 0, 0, 0]), Err(DecodeError::NotRelocatable(0)));
        assert_eq!(decode(&[0x8B]), Err(DecodeError::Truncated));
        assert!(matches!(decode(&[0xCD, 0x03]), Err(DecodeError::Unknown(0xCD, 0))));
    }

    #[test]
    fn steal_whole_instructions() {
        // CTheScripts::Init: sub esp,0x18; push ebx; push esi
        assert_eq!(steal_len(&[0x83, 0xEC, 0x18, 0x53, 0x56, 0x57], 5).unwrap(), 5);
        // CRunningScript::Process: push ebx; push esi; mov esi,ecx; mov eax,[esi+0xd8]
        let p = [0x53, 0x56, 0x8B, 0xF1, 0x8B, 0x86, 0xD8, 0, 0, 0, 0x85];
        assert_eq!(steal_len(&p, 5).unwrap(), 10);
    }

    #[test]
    fn relocates_rel32() {
        // push -1; jmp 0x40136c   at 0x5E8620 (CPed::~CPed on this exe)
        let code = [0x6A, 0xFF, 0xE9, 0x45, 0x8D, 0xE1, 0xFF, 0x64];
        let (out, n) = relocate(&code, 0x5E8620, 0x1000_0000, 5).unwrap();
        assert_eq!(n, 7);
        assert_eq!(&out[..3], &[0x6A, 0xFF, 0xE9]);
        let rel = i32::from_le_bytes([out[3], out[4], out[5], out[6]]);
        assert_eq!(rel32_target(0x1000_0002, 5, rel), 0x40136C);
    }
}
