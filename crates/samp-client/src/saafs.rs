//! samp.saa file redirection, platform-independent part (SAA-2).
//!
//! samp.dll serves every file whose lower-case basename matches a live entry of
//! its archive from memory instead of the disk (docs/re/samp-saa.md §7–8). The
//! Windows hooks (`filehook.rs`) only translate the kernel32 calls; the fake
//! handle table, the read/seek/size rules and the decrypted-entry cache live
//! here so they can be unit-tested on the host.
//!
//! Rules reproduced from samp.dll 0.3.7-R4 build B (addresses there):
//! * handles start at 0xFF000001 and step by 2 on every successful open, also
//!   once they leave the recognised range (0x10102350, 0x100625B0);
//! * only handles in 0xFF000001..=0xFF000100 are treated as fake by the other
//!   hooks (0x10062540); at most 50 files are open at once (0x10062560); a full
//!   table makes the open return handle 0;
//! * `ReadFile`: an `OVERLAPPED` argument sets the position from its `Offset`;
//!   a read that reaches the end of the entry copies what is left, reports
//!   that count and sets the last error to 38 (`ERROR_HANDLE_EOF`); a read that
//!   ends before the end copies everything and leaves the last error alone;
//!   the event of the `OVERLAPPED` is signalled either way; the result is TRUE
//!   (0x10062630);
//! * `SetFilePointer`: FILE_BEGIN sets, FILE_CURRENT adds (32-bit wrap),
//!   FILE_END jumps to the size ignoring the distance, other methods keep the
//!   position; returns the position; the high word is ignored (0x10062800);
//! * `GetFileSize` returns the size and zeroes the high word (0x100627B0);
//! * `CloseHandle` frees the slot; the decrypted entry stays cached
//!   (0x100628A0, 0x10064F20); `GetFileType` answers 1, disk (0x100628E0).

use std::collections::HashMap;
use std::sync::Arc;

use samp_saa::{EntryInfo, SaaArchive};

/// First fake handle samp hands out.
pub const FIRST_HANDLE: u32 = 0xFF00_0001;
/// Handles advance by this much per open.
pub const HANDLE_STEP: u32 = 2;
/// Last handle value the read/seek/size/close hooks recognise.
pub const LAST_RECOGNISED_HANDLE: u32 = 0xFF00_0100;
/// Size of samp's fake-file table.
pub const MAX_OPEN: usize = 50;
/// `ERROR_HANDLE_EOF`, set after a read that reaches the end of an entry.
pub const ERROR_HANDLE_EOF: u32 = 38;

/// Whether the other hooks would look `h` up in the fake-file table.
pub fn in_fake_range(h: u32) -> bool {
    (FIRST_HANDLE..=LAST_RECOGNISED_HANDLE).contains(&h)
}

/// `SetFilePointer` move methods.
pub const FILE_BEGIN: u32 = 0;
pub const FILE_CURRENT: u32 = 1;
pub const FILE_END: u32 = 2;

struct Slot {
    handle: u32,
    pos: u32,
    data: Arc<[u8]>,
}

/// Result of a read on a fake handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOutcome {
    /// Bytes copied into the caller's buffer (also the reported count).
    pub copied: usize,
    /// The read reached the end of the entry: last error := 38.
    pub eof: bool,
}

/// samp's fake-file table.
pub struct FakeFiles {
    slots: Vec<Option<Slot>>,
    next_handle: u32,
}

impl Default for FakeFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeFiles {
    pub fn new() -> Self {
        Self { slots: (0..MAX_OPEN).map(|_| None).collect(), next_handle: FIRST_HANDLE }
    }

    /// Opens a fake file over `data`. Returns the handle, or 0 when all 50
    /// slots are in use (what samp's `CreateFileA` hook then returns).
    pub fn open(&mut self, data: Arc<[u8]>) -> u32 {
        let Some(i) = self.slots.iter().position(Option::is_none) else { return 0 };
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(HANDLE_STEP);
        self.slots[i] = Some(Slot { handle, pos: 0, data });
        handle
    }

    fn slot(&mut self, h: u32) -> Option<&mut Slot> {
        if !in_fake_range(h) {
            return None;
        }
        self.slots.iter_mut().flatten().find(|s| s.handle == h)
    }

    /// Whether `h` is one of ours (in range and open); otherwise the call goes
    /// to the real kernel32 function.
    pub fn is_open(&mut self, h: u32) -> bool {
        self.slot(h).is_some()
    }

    /// Number of open fake files.
    pub fn open_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// `ReadFile`. `offset` is the `OVERLAPPED.Offset` when an `OVERLAPPED` was
    /// passed. `None` if `h` is not an open fake handle.
    pub fn read(&mut self, h: u32, offset: Option<u32>, out: &mut [u8]) -> Option<ReadOutcome> {
        let s = self.slot(h)?;
        if let Some(o) = offset {
            s.pos = o;
        }
        let size = s.data.len() as u32;
        if s.pos >= size {
            return Some(ReadOutcome { copied: 0, eof: true });
        }
        let n = out.len() as u32;
        let left = size - s.pos;
        let (copied, eof) = if n < left { (n, false) } else { (left, true) };
        let start = s.pos as usize;
        out[..copied as usize].copy_from_slice(&s.data[start..start + copied as usize]);
        s.pos += copied;
        Some(ReadOutcome { copied: copied as usize, eof })
    }

    /// `SetFilePointer`; returns the new position.
    pub fn seek(&mut self, h: u32, distance: i32, method: u32) -> Option<u32> {
        let s = self.slot(h)?;
        match method {
            FILE_BEGIN => s.pos = distance as u32,
            FILE_CURRENT => s.pos = s.pos.wrapping_add(distance as u32),
            FILE_END => s.pos = s.data.len() as u32,
            _ => {}
        }
        Some(s.pos)
    }

    /// `GetFileSize` (low word; the high word is 0).
    pub fn size(&mut self, h: u32) -> Option<u32> {
        self.slot(h).map(|s| s.data.len() as u32)
    }

    /// `CloseHandle`; true if `h` was ours.
    pub fn close(&mut self, h: u32) -> bool {
        if !in_fake_range(h) {
            return false;
        }
        match self.slots.iter_mut().find(|s| s.as_ref().is_some_and(|s| s.handle == h)) {
            Some(s) => {
                *s = None;
                true
            }
            None => false,
        }
    }
}

/// Per-entry open statistics for the `files` control command.
#[derive(Debug, Clone)]
pub struct Served {
    pub hash: u32,
    pub name: Option<&'static str>,
    pub size: u32,
    pub opens: u32,
    /// Path of the first open, as the game passed it.
    pub first_path: String,
    /// Handle of the most recent open.
    pub last_handle: u32,
}

/// What a `CreateFileA` hook should do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenResult {
    /// Not an archive entry: call the real function.
    PassThrough,
    /// Served from the archive with this handle (0 = table full, as samp).
    Fake(u32),
}

/// The archive plus the open-file state: everything the hooks need.
pub struct Redirector {
    archive: SaaArchive,
    files: FakeFiles,
    cache: HashMap<u32, Arc<[u8]>>,
    served: Vec<Served>,
    /// Opens in total, including failed ones (table full / unreadable entry).
    pub opens: u32,
    pub failed_opens: u32,
}

impl Redirector {
    pub fn new(archive: SaaArchive) -> Self {
        Self { archive, files: FakeFiles::new(), cache: HashMap::new(), served: Vec::new(), opens: 0, failed_opens: 0 }
    }

    pub fn archive(&self) -> &SaaArchive {
        &self.archive
    }

    pub fn files(&mut self) -> &mut FakeFiles {
        &mut self.files
    }

    /// Decrypted entry, cached for the life of the process like samp.
    fn data(&mut self, e: &EntryInfo) -> Option<Arc<[u8]>> {
        if let Some(d) = self.cache.get(&e.hash) {
            return Some(d.clone());
        }
        let d: Arc<[u8]> = self.archive.read_entry(e)?.into();
        self.cache.insert(e.hash, d.clone());
        Some(d)
    }

    /// The `CreateFileA` decision for `path` (raw ANSI bytes without the NUL;
    /// a trailing space must already be cut, as the hook does in place).
    pub fn open(&mut self, path: &[u8]) -> OpenResult {
        let Some(e) = self.archive.find_bytes(path) else { return OpenResult::PassThrough };
        self.opens += 1;
        let Some(data) = self.data(&e) else {
            // A live slot whose data lies outside the file (broken archive).
            // samp would read past its buffer; we let the real file open instead.
            self.failed_opens += 1;
            return OpenResult::PassThrough;
        };
        let h = self.files.open(data);
        if h == 0 {
            self.failed_opens += 1;
        }
        match self.served.iter_mut().find(|s| s.hash == e.hash) {
            Some(s) => {
                s.opens += 1;
                s.last_handle = h;
            }
            None => self.served.push(Served {
                hash: e.hash,
                name: e.name,
                size: e.size,
                opens: 1,
                first_path: String::from_utf8_lossy(path).into_owned(),
                last_handle: h,
            }),
        }
        OpenResult::Fake(h)
    }

    pub fn served(&self) -> &[Served] {
        &self.served
    }

    /// Reply of the `files` control command: a header line, then one line per
    /// distinct entry served (in order of first open).
    pub fn report(&self) -> String {
        let mut out = format!(
            "files mode=archive signature={} entries={} distinct={} opens={} failed={} open_now={}",
            if self.archive.signature_valid() { "valid" } else { "invalid" },
            self.archive.list().len(),
            self.served.len(),
            self.opens,
            self.failed_opens,
            self.files.open_count(),
        );
        for s in &self.served {
            out.push_str(&format!(
                "\nfile name={} hash={:#010x} size={} opens={} handle={:#010x} path={}",
                s.name.unwrap_or("?"),
                s.hash,
                s.size,
                s.opens,
                s.last_handle,
                s.first_path
            ));
        }
        out
    }
}

/// Reply of `files` when no archive is in use.
pub fn report_fallback(reason: &str) -> String {
    format!("files mode=fallback distinct=0 opens=0 reason={reason}")
}

/// `CreateFileA` path preprocessing done in place by samp (0x10062950): a
/// trailing space is replaced by NUL. Returns the length to use.
pub fn trim_trailing_space(path: &[u8]) -> usize {
    match path.last() {
        Some(b' ') => path.len() - 1,
        _ => path.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize) -> Arc<[u8]> {
        (0..n).map(|i| i as u8).collect::<Vec<u8>>().into()
    }

    #[test]
    fn handles_step_by_two_and_table_is_bounded() {
        let mut f = FakeFiles::new();
        let mut hs = Vec::new();
        for _ in 0..MAX_OPEN {
            hs.push(f.open(data(4)));
        }
        assert_eq!(hs[0], 0xFF00_0001);
        assert_eq!(hs[1], 0xFF00_0003);
        assert_eq!(hs[49], 0xFF00_0001 + 49 * 2);
        assert_eq!(f.open(data(4)), 0, "51st concurrent open fails with handle 0");
        assert!(f.close(hs[10]));
        assert!(!f.close(hs[10]));
        // Counter did not advance on the failed open.
        assert_eq!(f.open(data(4)), 0xFF00_0001 + 50 * 2);
    }

    #[test]
    fn handles_leave_the_recognised_range_after_128_opens() {
        let mut f = FakeFiles::new();
        for i in 0..128u32 {
            let h = f.open(data(1));
            assert_eq!(h, FIRST_HANDLE + 2 * i);
            assert!(f.is_open(h));
            assert!(f.close(h));
        }
        let h = f.open(data(1));
        assert_eq!(h, 0xFF00_0101);
        assert!(!in_fake_range(h));
        assert!(!f.is_open(h), "out-of-range handles go to the real kernel32");
        assert_eq!(f.read(h, None, &mut [0; 1]), None);
        assert!(!f.close(h), "and the slot is never freed");
        assert_eq!(f.open_count(), 1);
    }

    #[test]
    fn read_rules() {
        let mut f = FakeFiles::new();
        let h = f.open(data(10));
        let mut b = [0u8; 4];
        assert_eq!(f.read(h, None, &mut b), Some(ReadOutcome { copied: 4, eof: false }));
        assert_eq!(b, [0, 1, 2, 3]);
        // Ends exactly at the end: EOF error is set.
        let mut b6 = [0u8; 6];
        assert_eq!(f.read(h, None, &mut b6), Some(ReadOutcome { copied: 6, eof: true }));
        assert_eq!(b6, [4, 5, 6, 7, 8, 9]);
        assert_eq!(f.read(h, None, &mut b), Some(ReadOutcome { copied: 0, eof: true }));
        // Overlapped offset sets the position; a short read past the end.
        let mut b8 = [0xEEu8; 8];
        assert_eq!(f.read(h, Some(7), &mut b8), Some(ReadOutcome { copied: 3, eof: true }));
        assert_eq!(&b8[..4], &[7, 8, 9, 0xEE]);
        assert_eq!(f.read(h, Some(100), &mut b8), Some(ReadOutcome { copied: 0, eof: true }));
        assert_eq!(f.read(h ^ 2, None, &mut b8), None);
    }

    #[test]
    fn seek_rules() {
        let mut f = FakeFiles::new();
        let h = f.open(data(10));
        assert_eq!(f.seek(h, 3, FILE_BEGIN), Some(3));
        assert_eq!(f.seek(h, 2, FILE_CURRENT), Some(5));
        assert_eq!(f.seek(h, -1, FILE_CURRENT), Some(4));
        assert_eq!(f.seek(h, -4, FILE_END), Some(10), "FILE_END ignores the distance");
        assert_eq!(f.seek(h, 1, 7), Some(10), "unknown method keeps the position");
        assert_eq!(f.seek(h, -11, FILE_CURRENT), Some(u32::MAX));
        assert_eq!(f.read(h, None, &mut [0; 2]), Some(ReadOutcome { copied: 0, eof: true }));
        assert_eq!(f.size(h), Some(10));
        assert_eq!(f.seek(0x1234, 0, FILE_BEGIN), None);
    }

    #[test]
    fn trailing_space() {
        assert_eq!(trim_trailing_space(b"data\\gta.dat "), 12);
        assert_eq!(trim_trailing_space(b"data\\gta.dat"), 12);
        assert_eq!(trim_trailing_space(b""), 0);
    }

    /// The user's archive, if present locally (third-party data, gitignored).
    fn real_archive() -> Option<SaaArchive> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let p = std::env::var_os("SAMP_SAA").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("re/samp-installer/extracted/samp.saa"));
        SaaArchive::open(p).ok()
    }

    #[test]
    fn redirector_on_the_real_archive() {
        let Some(saa) = real_archive() else { return };
        let mut r = Redirector::new(saa);
        assert_eq!(r.open(b"MODELS\\GTA3.IMG"), OpenResult::PassThrough);
        let OpenResult::Fake(h) = r.open(b"data\\script\\main.scm") else { panic!("main.scm not served") };
        assert_eq!(h, FIRST_HANDLE);
        let mut head = [0u8; 8];
        let out = r.files().read(h, None, &mut head).unwrap();
        assert_eq!(out.copied, 8);
        assert_eq!(head[0], 0x02, "SCM starts with a GOTO (0x0002)");
        // Any directory, any case.
        let OpenResult::Fake(h2) = r.open(b"C:\\elsewhere\\MAIN.SCM") else { panic!() };
        assert_eq!(h2, FIRST_HANDLE + 2);
        let OpenResult::Fake(_) = r.open(b"models\\txd\\LOADSCS.txd") else { panic!() };
        assert!(r.files().close(h));
        let rep = r.report();
        assert!(rep.starts_with("files mode=archive signature=valid entries=28 distinct=2 opens=3"), "{rep}");
        assert!(rep.contains("\nfile name=main.scm hash=0xd83f24dd size=103876 opens=2"), "{rep}");
        assert!(rep.contains("path=data\\script\\main.scm"));
    }
}
