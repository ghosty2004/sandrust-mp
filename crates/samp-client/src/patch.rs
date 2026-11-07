//! Minimal in-house code patching: byte writes, NOP runs, rel32 call/jmp
//! redirection and pointer (vtable slot) swaps, with the original bytes kept
//! so every patch can be undone.
//!
//! The arithmetic and bookkeeping are pure and unit-tested on the host through
//! the [`Memory`] trait; the real implementation (VirtualProtect + raw writes)
//! lives in `ProcessMemory` behind `cfg(windows)`.

use std::fmt;

/// x86 opcode of `call rel32`.
pub const OP_CALL: u8 = 0xE8;
/// x86 opcode of `jmp rel32`.
pub const OP_JMP: u8 = 0xE9;
/// x86 `nop`.
pub const OP_NOP: u8 = 0x90;

/// rel32 displacement for a branch instruction of `insn_len` bytes at `site`
/// that should land on `target` (wraps like the CPU does).
pub fn rel32(site: u32, insn_len: u32, target: u32) -> i32 {
    target.wrapping_sub(site.wrapping_add(insn_len)) as i32
}

/// Absolute target of a branch instruction of `insn_len` bytes at `site` with
/// displacement `rel`.
pub fn rel32_target(site: u32, insn_len: u32, rel: i32) -> u32 {
    site.wrapping_add(insn_len).wrapping_add(rel as u32)
}

/// Encodes a 5-byte `call rel32` (opcode 0xE8) or `jmp rel32` (0xE9).
pub fn encode_branch(opcode: u8, site: u32, target: u32) -> [u8; 5] {
    let r = rel32(site, 5, target).to_le_bytes();
    [opcode, r[0], r[1], r[2], r[3]]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// Memory could not be read or made writable.
    Access { addr: u32, len: usize },
    /// The bytes at the site are not what the patch expects.
    Mismatch { addr: u32, expected: Vec<u8>, found: Vec<u8> },
    /// The site does not hold the expected instruction (e.g. no 0xE8 at a call site).
    NotABranch { addr: u32, found: u8 },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Access { addr, len } => write!(f, "cannot access {len} bytes at {addr:#x}"),
            PatchError::Mismatch { addr, expected, found } => {
                write!(f, "unexpected bytes at {addr:#x}: expected {}, found {}", hex(expected), hex(found))
            }
            PatchError::NotABranch { addr, found } => write!(f, "no call/jmp rel32 at {addr:#x} (opcode {found:#04x})"),
        }
    }
}

impl std::error::Error for PatchError {}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Access to the patched address space (32-bit addresses).
pub trait Memory {
    /// Reads `buf.len()` bytes at `addr`.
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), PatchError>;
    /// Writes `bytes` at `addr`, making the range writable first if needed.
    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), PatchError>;

    fn read_u8(&self, addr: u32) -> Result<u8, PatchError> {
        let mut b = [0u8; 1];
        self.read(addr, &mut b)?;
        Ok(b[0])
    }
    fn read_u16(&self, addr: u32) -> Result<u16, PatchError> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    fn read_u32(&self, addr: u32) -> Result<u32, PatchError> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn read_i32(&self, addr: u32) -> Result<i32, PatchError> {
        Ok(self.read_u32(addr)? as i32)
    }
    fn read_f32(&self, addr: u32) -> Result<f32, PatchError> {
        Ok(f32::from_bits(self.read_u32(addr)?))
    }
}

/// One applied patch: where, and what was there before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub addr: u32,
    pub original: Vec<u8>,
    pub label: &'static str,
}

/// Applies patches to a [`Memory`] and remembers the original bytes.
#[derive(Debug, Default)]
pub struct Patcher {
    applied: Vec<Applied>,
}

impl Patcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn applied(&self) -> &[Applied] {
        &self.applied
    }

    /// Writes `bytes` at `addr`. If `expect` is given, the current bytes must
    /// match it (same length as `bytes`) or nothing is written.
    pub fn write<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        addr: u32,
        bytes: &[u8],
        expect: Option<&[u8]>,
    ) -> Result<(), PatchError> {
        let mut original = vec![0u8; bytes.len()];
        mem.read(addr, &mut original)?;
        if let Some(e) = expect
            && e != original.as_slice()
        {
            return Err(PatchError::Mismatch { addr, expected: e.to_vec(), found: original });
        }
        mem.write(addr, bytes)?;
        self.applied.push(Applied { addr, original, label });
        Ok(())
    }

    /// Fills `len` bytes at `addr` with NOPs.
    pub fn nop<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        addr: u32,
        len: usize,
        expect: Option<&[u8]>,
    ) -> Result<(), PatchError> {
        self.write(mem, label, addr, &vec![OP_NOP; len], expect)
    }

    /// Writes a 5-byte `call target` at `site` (overwrites whatever is there).
    pub fn hook_call<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        site: u32,
        target: u32,
        expect: Option<&[u8]>,
    ) -> Result<(), PatchError> {
        self.write(mem, label, site, &encode_branch(OP_CALL, site, target), expect)
    }

    /// Writes a 5-byte `jmp target` at `site`.
    pub fn hook_jmp<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        site: u32,
        target: u32,
        expect: Option<&[u8]>,
    ) -> Result<(), PatchError> {
        self.write(mem, label, site, &encode_branch(OP_JMP, site, target), expect)
    }

    /// Changes only the rel32 of an existing `call`/`jmp rel32` at `site` so it
    /// lands on `new_target`. Returns the previous target. If
    /// `expect_old_target` is given, the current target must equal it.
    pub fn retarget_branch<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        site: u32,
        new_target: u32,
        expect_old_target: Option<u32>,
    ) -> Result<u32, PatchError> {
        let op = mem.read_u8(site)?;
        if op != OP_CALL && op != OP_JMP {
            return Err(PatchError::NotABranch { addr: site, found: op });
        }
        let old = rel32_target(site, 5, mem.read_i32(site + 1)?);
        if let Some(e) = expect_old_target
            && e != old
        {
            return Err(PatchError::Mismatch {
                addr: site + 1,
                expected: rel32(site, 5, e).to_le_bytes().to_vec(),
                found: rel32(site, 5, old).to_le_bytes().to_vec(),
            });
        }
        self.write(mem, label, site + 1, &rel32(site, 5, new_target).to_le_bytes(), None)?;
        Ok(old)
    }

    /// Replaces the 4-byte pointer at `slot` (vtable entry, operand pointer, …)
    /// with `new_ptr`. Returns the previous pointer.
    pub fn swap_pointer<M: Memory>(
        &mut self,
        mem: &mut M,
        label: &'static str,
        slot: u32,
        new_ptr: u32,
    ) -> Result<u32, PatchError> {
        let old = mem.read_u32(slot)?;
        self.write(mem, label, slot, &new_ptr.to_le_bytes(), None)?;
        Ok(old)
    }

    /// Restores every patch in reverse order of application.
    pub fn restore_all<M: Memory>(&mut self, mem: &mut M) -> Result<(), PatchError> {
        while let Some(p) = self.applied.pop() {
            mem.write(p.addr, &p.original)?;
        }
        Ok(())
    }
}

/// The live process address space (gta_sa.exe and everything loaded into it).
#[cfg(windows)]
pub struct ProcessMemory;

#[cfg(windows)]
impl Memory for ProcessMemory {
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), PatchError> {
        if addr == 0 {
            return Err(PatchError::Access { addr, len: buf.len() });
        }
        // SAFETY: callers pass addresses inside gta_sa.exe's (never relocated) image or game objects reached from it.
        unsafe { std::ptr::copy_nonoverlapping(addr as usize as *const u8, buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }

    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), PatchError> {
        use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
        use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let p = addr as usize as *mut core::ffi::c_void;
        let mut old = 0u32;
        // SAFETY: plain Win32 calls on our own address space.
        unsafe {
            if VirtualProtect(p, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                return Err(PatchError::Access { addr, len: bytes.len() });
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
            let mut ignored = 0u32;
            VirtualProtect(p, bytes.len(), old, &mut ignored);
            FlushInstructionCache(GetCurrentProcess(), p, bytes.len());
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// A flat fake address space starting at `base`.
    pub struct FakeMemory {
        pub base: u32,
        pub bytes: Vec<u8>,
    }

    impl FakeMemory {
        pub fn new(base: u32, len: usize) -> Self {
            Self { base, bytes: vec![0; len] }
        }
        fn range(&self, addr: u32, len: usize) -> Result<std::ops::Range<usize>, PatchError> {
            let start = addr.checked_sub(self.base).ok_or(PatchError::Access { addr, len })? as usize;
            if start + len > self.bytes.len() {
                return Err(PatchError::Access { addr, len });
            }
            Ok(start..start + len)
        }
        pub fn set(&mut self, addr: u32, b: &[u8]) {
            let r = self.range(addr, b.len()).unwrap();
            self.bytes[r].copy_from_slice(b);
        }
        pub fn get(&self, addr: u32, len: usize) -> &[u8] {
            &self.bytes[self.range(addr, len).unwrap()]
        }
    }

    impl Memory for FakeMemory {
        fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), PatchError> {
            let r = self.range(addr, buf.len())?;
            buf.copy_from_slice(&self.bytes[r]);
            Ok(())
        }
        fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), PatchError> {
            let r = self.range(addr, bytes.len())?;
            self.bytes[r].copy_from_slice(bytes);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeMemory;
    use super::*;

    #[test]
    fn rel32_matches_game_call() {
        // 0x53EB12: E8 19 F7 FF FF = call 0x53E230 (Render2dStuff).
        assert_eq!(rel32(0x53EB12, 5, 0x53E230), i32::from_le_bytes([0x19, 0xF7, 0xFF, 0xFF]));
        assert_eq!(rel32_target(0x53EB12, 5, i32::from_le_bytes([0x19, 0xF7, 0xFF, 0xFF])), 0x53E230);
        // 0x748063: E8 F8 3B DF FF = call 0x53BC60.
        assert_eq!(rel32_target(0x748063, 5, i32::from_le_bytes([0xF8, 0x3B, 0xDF, 0xFF])), 0x53BC60);
    }

    #[test]
    fn rel32_forward_and_wrapping() {
        assert_eq!(rel32(0x1000, 5, 0x1005), 0);
        assert_eq!(rel32(0x1000, 5, 0x2000), 0xFFB);
        // A DLL far above the exe and back again: wraps modulo 2^32.
        let site = 0x0040_1000;
        let target = 0x7000_0000;
        let r = rel32(site, 5, target);
        assert_eq!(rel32_target(site, 5, r), target);
        let r = rel32(0xFFFF_FFF0, 5, 0x10);
        assert_eq!(rel32_target(0xFFFF_FFF0, 5, r), 0x10);
    }

    #[test]
    fn encode_call_and_jmp() {
        assert_eq!(encode_branch(OP_CALL, 0x53EB12, 0x53E230), [0xE8, 0x19, 0xF7, 0xFF, 0xFF]);
        assert_eq!(encode_branch(OP_JMP, 0x100, 0x105), [0xE9, 0, 0, 0, 0]);
    }

    #[test]
    fn write_checks_expected_and_restores() {
        let mut m = FakeMemory::new(0x1000, 0x100);
        m.set(0x1010, &[1, 2, 3, 4]);
        let mut p = Patcher::new();
        let e = p.write(&mut m, "bad", 0x1010, &[9, 9], Some(&[7, 7])).unwrap_err();
        assert!(matches!(e, PatchError::Mismatch { .. }));
        assert_eq!(m.get(0x1010, 4), &[1, 2, 3, 4]);
        p.write(&mut m, "ok", 0x1010, &[9, 9], Some(&[1, 2])).unwrap();
        p.nop(&mut m, "nop", 0x1011, 3, None).unwrap();
        assert_eq!(m.get(0x1010, 4), &[9, 0x90, 0x90, 0x90]);
        assert_eq!(p.applied().len(), 2);
        p.restore_all(&mut m).unwrap();
        assert_eq!(m.get(0x1010, 4), &[1, 2, 3, 4]);
        assert!(p.applied().is_empty());
    }

    #[test]
    fn retarget_call_keeps_opcode() {
        let mut m = FakeMemory::new(0x53E000, 0x1000);
        m.set(0x53EB12, &[0xE8, 0x19, 0xF7, 0xFF, 0xFF]);
        let mut p = Patcher::new();
        let old = p.retarget_branch(&mut m, "frame", 0x53EB12, 0x1234_5678, Some(0x53E230)).unwrap();
        assert_eq!(old, 0x53E230);
        assert_eq!(m.get(0x53EB12, 1), &[0xE8]);
        assert_eq!(rel32_target(0x53EB12, 5, m.read_i32(0x53EB13).unwrap()), 0x1234_5678);
        // Wrong expected target → refused.
        let e = p.retarget_branch(&mut m, "frame", 0x53EB12, 1, Some(0x53E230)).unwrap_err();
        assert!(matches!(e, PatchError::Mismatch { .. }));
        p.restore_all(&mut m).unwrap();
        assert_eq!(m.get(0x53EB12, 5), &[0xE8, 0x19, 0xF7, 0xFF, 0xFF]);
    }

    #[test]
    fn retarget_rejects_non_branch() {
        let mut m = FakeMemory::new(0, 16);
        m.set(0, &[0x90; 5]);
        let e = Patcher::new().retarget_branch(&mut m, "x", 0, 5, None).unwrap_err();
        assert_eq!(e, PatchError::NotABranch { addr: 0, found: 0x90 });
    }

    #[test]
    fn hook_and_pointer_swap() {
        let mut m = FakeMemory::new(0x400000, 0x100);
        let mut p = Patcher::new();
        p.hook_jmp(&mut m, "j", 0x400010, 0x400080, None).unwrap();
        assert_eq!(m.get(0x400010, 5), &encode_branch(OP_JMP, 0x400010, 0x400080));
        p.hook_call(&mut m, "c", 0x400020, 0x400000, None).unwrap();
        assert_eq!(m.get(0x400020, 1), &[OP_CALL]);
        m.set(0x400040, &0x00851234u32.to_le_bytes());
        assert_eq!(p.swap_pointer(&mut m, "vt", 0x400040, 0x10002000).unwrap(), 0x00851234);
        assert_eq!(m.read_u32(0x400040).unwrap(), 0x10002000);
        p.restore_all(&mut m).unwrap();
        assert_eq!(m.read_u32(0x400040).unwrap(), 0x00851234);
        assert_eq!(m.get(0x400010, 5), &[0; 5]);
    }
}
