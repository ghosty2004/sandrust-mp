//! Read-only memory access, abstracted so record formatting can be tested on
//! the host with a fake address space.

pub trait Mem {
    /// Fills `buf` from `addr`; false if any byte is unreadable.
    fn read(&self, addr: u32, buf: &mut [u8]) -> bool;

    fn u8(&self, addr: u32) -> Option<u8> {
        let mut b = [0u8; 1];
        self.read(addr, &mut b).then_some(b[0])
    }
    fn u16(&self, addr: u32) -> Option<u16> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b).then(|| u16::from_le_bytes(b))
    }
    fn i16(&self, addr: u32) -> Option<i16> {
        self.u16(addr).map(|v| v as i16)
    }
    fn u32(&self, addr: u32) -> Option<u32> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b).then(|| u32::from_le_bytes(b))
    }
    fn i32(&self, addr: u32) -> Option<i32> {
        self.u32(addr).map(|v| v as i32)
    }
    fn f32(&self, addr: u32) -> Option<f32> {
        self.u32(addr).map(f32::from_bits)
    }
    /// NUL-terminated string of at most `max` bytes (lossy UTF-8).
    fn cstr(&self, addr: u32, max: usize) -> Option<String> {
        let mut out = Vec::new();
        for i in 0..max as u32 {
            let b = self.u8(addr.wrapping_add(i))?;
            if b == 0 {
                break;
            }
            out.push(b);
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Sparse fake address space for tests.
#[cfg(test)]
pub mod fake {
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct FakeMem {
        bytes: BTreeMap<u32, u8>,
    }

    impl FakeMem {
        pub fn set(&mut self, addr: u32, data: &[u8]) {
            for (i, b) in data.iter().enumerate() {
                self.bytes.insert(addr + i as u32, *b);
            }
        }
        pub fn set_u32(&mut self, addr: u32, v: u32) {
            self.set(addr, &v.to_le_bytes());
        }
        pub fn set_f32(&mut self, addr: u32, v: f32) {
            self.set(addr, &v.to_le_bytes());
        }
    }

    impl super::Mem for FakeMem {
        fn read(&self, addr: u32, buf: &mut [u8]) -> bool {
            for (i, b) in buf.iter_mut().enumerate() {
                match self.bytes.get(&(addr + i as u32)) {
                    Some(v) => *b = *v,
                    None => return false,
                }
            }
            true
        }
    }
}
