//! File-I/O view at the Win32 layer (`SAMP_TRACER_FILEIO=1`): what gta_sa.exe's own
//! `CreateFileA` / `ReadFile` / `SetFilePointer` / `GetFileSize` / `CloseHandle` calls get,
//! whichever layer serves them (disk, samp.dll's samp.saa redirection or ours).
//! This part is platform independent: per-handle reassembly of the bytes the game read (by offset,
//! so overlapped CdStream reads and seeks land where they belong) and classification of samp-style
//! archive handles. The Windows side (`win/fileio.rs`) wraps the game's import-table slots and feeds
//! it. Only digests leave the process: the reassembled content is hashed (SHA-1), never written.

/// gta_sa.exe 1.0 US import-address-table slots (kernel32 / user32), checked against the
/// reference exe's import directory in the tests.
pub const IAT_CREATE_FILE_A: u32 = 0x8580A0;
pub const IAT_READ_FILE: u32 = 0x858098;
pub const IAT_GET_FILE_SIZE: u32 = 0x858074;
pub const IAT_SET_FILE_POINTER: u32 = 0x858090;
pub const IAT_CLOSE_HANDLE: u32 = 0x858078;
/// `SystemParametersInfoA`: WinMain's first call (0x748727), i.e. the first game code after every
/// injected DLL has attached. Wrapped at tracer attach to arm the file wrappers there, so they sit
/// on top of whatever the client installed (samp: inline hooks in kernel32; ours: these slots).
pub const IAT_SYSTEM_PARAMETERS_INFO_A: u32 = 0x85826C;

/// The slots and their import names.
pub const IAT_SLOTS: [(u32, &str, &str); 6] = [
    (IAT_CREATE_FILE_A, "KERNEL32.dll", "CreateFileA"),
    (IAT_READ_FILE, "KERNEL32.dll", "ReadFile"),
    (IAT_GET_FILE_SIZE, "KERNEL32.dll", "GetFileSize"),
    (IAT_SET_FILE_POINTER, "KERNEL32.dll", "SetFilePointer"),
    (IAT_CLOSE_HANDLE, "KERNEL32.dll", "CloseHandle"),
    (IAT_SYSTEM_PARAMETERS_INFO_A, "USER32.dll", "SystemParametersInfoA"),
];

/// samp's fake archive handles start at 0xFF000001 and grow by 2 per open
/// (docs/re/samp-saa.md, "Redirection layer");
/// real Win32 handles are small multiples of 4.
pub fn is_archive_handle(h: u32) -> bool {
    (0xFF00_0000..=0xFF00_FFFF).contains(&h)
}

/// Reassembled content of one open handle.
#[derive(Debug, Clone)]
pub struct Image {
    data: Vec<u8>,
    /// Current file position (for reads without an OVERLAPPED offset).
    pub pos: u64,
    /// Read calls that returned data.
    pub reads: u32,
    /// Total bytes returned by those reads.
    pub bytes: u64,
    /// Highest end offset read.
    pub extent: u64,
    /// Bytes beyond `cap` are counted but not kept (no digest then).
    cap: u64,
    overflow: bool,
    /// A read started past the bytes seen so far (out-of-order or sparse reads).
    holes: bool,
}

impl Image {
    pub fn new(cap: u64) -> Self {
        Image { data: Vec::new(), pos: 0, reads: 0, bytes: 0, extent: 0, cap, overflow: false, holes: false }
    }

    /// A read returned `got`, at `offset` (OVERLAPPED) or at the current position.
    /// The position moves to the end of the read either way (samp's rule for OVERLAPPED reads,
    /// and the Win32 one for synchronous handles).
    pub fn read(&mut self, offset: Option<u64>, got: &[u8]) {
        let at = offset.unwrap_or(self.pos);
        let end = at + got.len() as u64;
        self.pos = end;
        if got.is_empty() {
            return;
        }
        self.reads += 1;
        self.bytes += got.len() as u64;
        self.extent = self.extent.max(end);
        if end > self.cap {
            self.overflow = true;
            self.data = Vec::new();
            return;
        }
        if self.overflow {
            return;
        }
        let (a, e) = (at as usize, end as usize);
        if self.data.len() < a {
            self.holes = true;
        }
        if self.data.len() < e {
            self.data.resize(e, 0);
        }
        self.data[a..e].copy_from_slice(got);
    }

    /// `SetFilePointer` returned the new position.
    pub fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// Lower-case hex SHA-1 of the reassembled bytes `[0, extent)`, if they were kept.
    pub fn digest(&self) -> Option<String> {
        if self.overflow || self.reads == 0 {
            return None;
        }
        Some(samp_saa::verify::sha1(&self.data).iter().map(|b| format!("{b:02x}")).collect())
    }

    /// True if a read started past the bytes seen so far (gaps, if any, hash as zeros).
    pub fn has_holes(&self) -> bool {
        self.holes
    }

    pub fn kept(&self) -> bool {
        !self.overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_handles() {
        assert!(is_archive_handle(0xFF00_0001));
        assert!(is_archive_handle(0xFF00_0101));
        assert!(!is_archive_handle(0x0000_0124));
        assert!(!is_archive_handle(0xFFFF_FFFF));
    }

    #[test]
    fn sequential_reads_and_seek() {
        let mut im = Image::new(1 << 20);
        im.read(None, b"abc");
        im.read(None, b"def");
        assert_eq!((im.pos, im.extent, im.bytes, im.reads), (6, 6, 6, 2));
        im.seek(1);
        im.read(None, b"X");
        im.read(None, b"");
        assert_eq!(im.reads, 3);
        let want: String = samp_saa::verify::sha1(b"aXcdef").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(im.digest().as_deref(), Some(want.as_str()));
        assert!(!im.has_holes());
    }

    #[test]
    fn overlapped_offsets_and_holes() {
        let mut im = Image::new(1 << 20);
        im.read(Some(4), b"ef");
        assert!(im.has_holes());
        im.read(Some(0), b"abcd");
        assert_eq!(im.extent, 6);
        assert_eq!(im.pos, 4);
        let want: String = samp_saa::verify::sha1(b"abcdef").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(im.digest().as_deref(), Some(want.as_str()));
    }

    #[test]
    fn cap_drops_content() {
        let mut im = Image::new(4);
        im.read(None, b"abcdef");
        assert!(!im.kept());
        assert_eq!(im.digest(), None);
        assert_eq!(im.bytes, 6);
        assert_eq!(Image::new(4).digest(), None);
    }

    /// Resolves the import slots of the reference exe by name (skipped if it is absent).
    #[test]
    fn iat_slots_match_reference_exe() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../re/gta/1.0-US/gta_sa.exe");
        let Ok(d) = std::fs::read(&p) else {
            eprintln!("re/gta/1.0-US/gta_sa.exe not present; skipping");
            return;
        };
        let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let u16_at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as usize;
        let e = u32_at(0x3C) as usize;
        let (n, opt) = (u16_at(e + 6), u16_at(e + 20));
        let secs: Vec<(u32, u32, u32)> = (0..n)
            .map(|i| {
                let o = e + 24 + opt + 40 * i;
                (u32_at(o + 12), u32_at(o + 8).max(u32_at(o + 16)), u32_at(o + 20))
            })
            .collect();
        let off = |rva: u32| {
            let &(va, _, raw) = secs.iter().find(|&&(va, sz, _)| rva >= va && rva < va + sz).unwrap();
            (rva - va + raw) as usize
        };
        let cstr = |o: usize| {
            let end = o + d[o..].iter().position(|&b| b == 0).unwrap();
            String::from_utf8_lossy(&d[o..end]).into_owned()
        };
        let mut found = Vec::new();
        let mut desc = off(u32_at(e + 24 + 96 + 8));
        loop {
            let (oft, name, ft) = (u32_at(desc), u32_at(desc + 12), u32_at(desc + 16));
            if name == 0 {
                break;
            }
            let dll = cstr(off(name));
            for i in 0.. {
                let t = u32_at(off(oft) + 4 * i);
                if t == 0 {
                    break;
                }
                if t & 0x8000_0000 == 0 {
                    found.push((0x40_0000 + ft + 4 * i as u32, dll.clone(), cstr(off(t) + 2)));
                }
            }
            desc += 20;
        }
        for (slot, dll, fname) in IAT_SLOTS {
            assert!(
                found.iter().any(|(s, d, f)| *s == slot && d.eq_ignore_ascii_case(dll) && f == fname),
                "{dll}!{fname} is not at {slot:#x}"
            );
        }
    }
}
