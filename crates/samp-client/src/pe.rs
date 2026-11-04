//! Minimal PE32 import-table walk: finds the import-address-table slot of a
//! function imported by name (used to hook gta_sa.exe's kernel32 file calls).

/// Reads `len` bytes at a virtual address (None if unreadable).
pub trait Reader {
    fn bytes(&self, va: u32, len: usize) -> Option<Vec<u8>>;

    fn u32_at(&self, va: u32) -> Option<u32> {
        self.bytes(va, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn cstr(&self, va: u32, max: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        for i in 0..max as u32 {
            let b = self.bytes(va + i, 1)?[0];
            if b == 0 {
                return Some(out);
            }
            out.push(b);
        }
        None
    }
}

/// Import-address-table slot (VA) of each of `names` imported from `dll`
/// (case-insensitive) by the image loaded at `base`, in the order of `names`.
/// Uses the import name table (`OriginalFirstThunk`); a descriptor without one
/// yields `None` for its functions.
pub fn iat_slots(r: &impl Reader, base: u32, dll: &str, names: &[&str]) -> Vec<Option<u32>> {
    let mut out = vec![None; names.len()];
    let Some(pe) = r.u32_at(base + 0x3C) else { return out };
    let nt = base + pe;
    // PE32 optional header at +24; data directory 1 (imports) at +96 + 8.
    let Some(import_rva) = r.u32_at(nt + 24 + 96 + 8) else { return out };
    if import_rva == 0 {
        return out;
    }
    let mut desc = base + import_rva;
    while let Some(d) = r.bytes(desc, 20) {
        let w = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let (oft, name_rva, ft) = (w(0), w(12), w(16));
        if oft == 0 && name_rva == 0 && ft == 0 {
            break;
        }
        let matches = r.cstr(base + name_rva, 260).is_some_and(|n| n.eq_ignore_ascii_case(dll.as_bytes()));
        if matches && oft != 0 {
            for i in 0.. {
                let Some(t) = r.u32_at(base + oft + 4 * i) else { break };
                if t == 0 {
                    break;
                }
                if t & 0x8000_0000 != 0 {
                    continue; // by ordinal
                }
                let Some(fname) = r.cstr(base + t + 2, 256) else { continue };
                if let Some(k) = names.iter().position(|n| n.as_bytes() == fname.as_slice()) {
                    out[k] = Some(base + ft + 4 * i);
                }
            }
        }
        desc += 20;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patches::testutil::Exe;

    /// Sections through `Exe::read_va`, the headers (first page) from the raw file.
    struct ExeReader(Exe, Vec<u8>);

    impl Reader for ExeReader {
        fn bytes(&self, va: u32, len: usize) -> Option<Vec<u8>> {
            let off = (va - 0x40_0000) as usize;
            if off + len <= 0x1000 {
                return Some(self.1[off..off + len].to_vec());
            }
            Some(self.0.read_va(va, len))
        }
    }

    #[test]
    fn kernel32_file_imports_of_the_reference_exe() {
        let Some(exe) = Exe::load() else { return };
        let raw = std::fs::read(crate::patches::testutil::root().join("re/gta/1.0-US/gta_sa.exe")).unwrap();
        let r = ExeReader(exe, raw);
        let names = ["CreateFileA", "ReadFile", "GetFileSize", "SetFilePointer", "CloseHandle", "GetFileType", "Nope"];
        let slots = iat_slots(&r, 0x40_0000, "KERNEL32.dll", &names);
        assert_eq!(
            slots,
            vec![Some(0x8580A0), Some(0x858098), Some(0x858074), Some(0x858090), Some(0x858078), Some(0x8581B4), None]
        );
        assert_eq!(iat_slots(&r, 0x40_0000, "kernel32.DLL", &names[..1]), vec![Some(0x8580A0)]);
    }
}
