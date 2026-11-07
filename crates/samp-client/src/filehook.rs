//! samp.saa file redirection, Windows side (docs/re/samp-saa.md, "8. Redirection layer").
//!
//! [`init`] runs at attach while the main thread is still suspended: it opens `samp.saa` next to
//! gta_sa.exe and, if the signature verifies, points gta_sa.exe's import-table slots for the six kernel32
//! file functions at the hooks below; the rules themselves live in `saafs.rs`. samp instead inline-hooks the
//! kernel32 exports, but every file the archive covers is opened by gta_sa.exe itself, so the same opens
//! are served. Without a usable archive nothing is hooked and the client falls back to its own main.scm.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{SetLastError, TRUE};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::SetEvent;

use crate::patch::{Memory, Patcher, ProcessMemory};
use crate::saafs::{self, OpenResult, Redirector};

/// Which data the game gets.
#[derive(Debug)]
pub enum Mode {
    /// Files are served from samp.saa (incl. samp's main.scm and script.img).
    Archive,
    /// No usable samp.saa: stock files plus our own main.scm.
    Fallback(String),
}

static MODE: OnceLock<Mode> = OnceLock::new();
static REDIRECTOR: Mutex<Option<Redirector>> = Mutex::new(None);

const NAMES: [&str; 6] = ["CreateFileA", "ReadFile", "GetFileSize", "SetFilePointer", "CloseHandle", "GetFileType"];
const CREATE: usize = 0;
const READ: usize = 1;
const SIZE: usize = 2;
const SEEK: usize = 3;
const CLOSE: usize = 4;
const TYPE: usize = 5;

/// The real kernel32 functions (read from the import table before patching).
static ORIG: [AtomicU32; 6] = [const { AtomicU32::new(0) }; 6];

/// True when files are served from samp.saa.
pub fn archive_active() -> bool {
    matches!(MODE.get(), Some(Mode::Archive))
}

/// An archive entry by its name hash (samp reads `logo.png` this way, hash 0xBADDEA6B).
pub fn archive_read_hash(hash: u32) -> Option<Vec<u8>> {
    let g = REDIRECTOR.lock().ok()?;
    g.as_ref()?.archive().read_hash(hash)
}

/// Reply of the `files` control command.
pub fn report() -> String {
    match MODE.get() {
        Some(Mode::Archive) => match REDIRECTOR.lock() {
            Ok(g) => g.as_ref().map(Redirector::report).unwrap_or_else(|| saafs::report_fallback("no-archive")),
            Err(_) => "error redirector poisoned".into(),
        },
        Some(Mode::Fallback(why)) => saafs::report_fallback(&why.replace(' ', "_")),
        None => saafs::report_fallback("not-initialised"),
    }
}

/// Opens the archive and installs the hooks; decides the mode once.
pub fn init(patcher: &mut Patcher, mem: &mut ProcessMemory) -> &'static Mode {
    let mode = match load_and_install(patcher, mem) {
        Ok(n) => {
            crate::log!(
                "files: ARCHIVE mode: serving {n} samp.saa entries by basename (incl. samp's main.scm and script.img)"
            );
            Mode::Archive
        }
        Err(why) => {
            crate::log!("files: FALLBACK mode: {why}; the game reads stock files and our own minimal main.scm");
            Mode::Fallback(why)
        }
    };
    MODE.get_or_init(|| mode)
}

/// Set to 1 to ignore samp.saa (tests of the fallback mode).
pub const DISABLE_ENV: &str = "SAMP_RS_NO_SAA";

fn load_and_install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Result<usize, String> {
    if std::env::var(DISABLE_ENV).is_ok_and(|v| v == "1") {
        return Err(format!("{DISABLE_ENV}=1"));
    }
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("samp.saa")))
        .ok_or("cannot locate gta_sa.exe's directory")?;
    if !path.exists() {
        return Err(format!("no samp.saa next to gta_sa.exe ({})", path.display()));
    }
    let saa = samp_saa::SaaArchive::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let entries = saa.list();
    crate::log!(
        "files: {} opened: signature {}, {} live entries",
        path.display(),
        if saa.signature_valid() { "valid" } else { "INVALID" },
        entries.len()
    );
    if !saa.signature_valid() {
        // samp keeps going with a directory decoded without the hash term, i.e. garbage sizes and links
        // (docs/re/samp-saa.md, "6. Signature"). We report it and use the stock files instead.
        return Err("samp.saa signature does not verify (samp would decode garbage)".into());
    }
    for e in &entries {
        crate::log!(
            "files:   slot {:3} hash {:#010x} size {:8} {}",
            e.index,
            e.hash,
            e.size,
            e.name.unwrap_or("(unnamed)")
        );
    }
    // SAFETY: GetModuleHandleW(NULL) returns the base of the main executable.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as u32;
    let slots = crate::pe::iat_slots(&ImageReader, base, "kernel32.dll", &NAMES);
    let hooks: [u32; 6] = [
        create_file_a as unsafe extern "system" fn(_, _, _, _, _, _, _) -> _ as usize as u32,
        read_file as unsafe extern "system" fn(_, _, _, _, _) -> _ as usize as u32,
        get_file_size as unsafe extern "system" fn(_, _) -> _ as usize as u32,
        set_file_pointer as unsafe extern "system" fn(_, _, _, _) -> _ as usize as u32,
        close_handle as unsafe extern "system" fn(_) -> _ as usize as u32,
        get_file_type as unsafe extern "system" fn(_) -> _ as usize as u32,
    ];
    let mut found = [0u32; 6];
    for (i, s) in slots.iter().enumerate() {
        let slot = s.ok_or_else(|| format!("gta_sa.exe does not import {} from kernel32", NAMES[i]))?;
        found[i] = slot;
        let orig = mem.read_u32(slot).map_err(|e| e.to_string())?;
        ORIG[i].store(orig, Ordering::SeqCst);
    }
    *REDIRECTOR.lock().unwrap_or_else(|e| e.into_inner()) = Some(Redirector::new(saa));
    for i in 0..6 {
        let orig = ORIG[i].load(Ordering::SeqCst);
        patcher
            .write(mem, "kernel32 import (samp.saa)", found[i], &hooks[i].to_le_bytes(), Some(&orig.to_le_bytes()))
            .map_err(|e| format!("import slot of {}: {e}", NAMES[i]))?;
        crate::log!("files: {} import at {:#x}: {orig:#x} -> {:#x}", NAMES[i], found[i], hooks[i]);
    }
    Ok(entries.len())
}

struct ImageReader;

impl crate::pe::Reader for ImageReader {
    fn bytes(&self, va: u32, len: usize) -> Option<Vec<u8>> {
        let mut b = vec![0u8; len];
        ProcessMemory.read(va, &mut b).ok().map(|_| b)
    }
}

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Relaxed) as usize
}

fn with_redirector<T>(f: impl FnOnce(&mut Redirector) -> Option<T>) -> Option<T> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut g = REDIRECTOR.lock().unwrap_or_else(|e| e.into_inner());
        g.as_mut().and_then(f)
    }))
    .ok()
    .flatten()
}

/// True if the byte at `p` may be written (samp writes it unconditionally).
fn writable(p: *const u8) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY,
        VirtualQuery,
    };
    // SAFETY: VirtualQuery only inspects the address space.
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let n = unsafe { VirtualQuery(p as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
    n != 0 && mbi.Protect & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY) != 0
}

type CreateFileAFn = unsafe extern "system" fn(*const u8, u32, u32, *mut c_void, u32, u32, usize) -> usize;

unsafe extern "system" fn create_file_a(
    name: *const u8,
    access: u32,
    share: u32,
    security: *mut c_void,
    disposition: u32,
    flags: u32,
    template: usize,
) -> usize {
    if !name.is_null() {
        // SAFETY: the caller passes a NUL-terminated ANSI path.
        let path = unsafe { std::ffi::CStr::from_ptr(name.cast()) }.to_bytes();
        let n = saafs::trim_trailing_space(path);
        if n < path.len() {
            // samp cuts a trailing space in the caller's buffer (see `saafs::trim_trailing_space`).
            let p = name.wrapping_add(n) as *mut u8;
            if writable(p) {
                // SAFETY: inside the caller's string, checked writable.
                unsafe { p.write(0) };
            }
        }
        let path = &path[..n];
        if let Some(OpenResult::Fake(h)) = with_redirector(|r| Some(r.open(path))) {
            crate::log!("files: CreateFileA({}) -> archive handle {h:#010x}", String::from_utf8_lossy(path));
            return h as usize;
        }
    }
    // SAFETY: the real CreateFileA, same signature and arguments.
    unsafe {
        let f: CreateFileAFn = std::mem::transmute(orig(CREATE));
        f(name, access, share, security, disposition, flags, template)
    }
}

type ReadFileFn = unsafe extern "system" fn(usize, *mut u8, u32, *mut u32, *mut u8) -> i32;

unsafe extern "system" fn read_file(h: usize, buf: *mut u8, n: u32, read: *mut u32, overlapped: *mut u8) -> i32 {
    if saafs::in_fake_range(h as u32) {
        // OVERLAPPED: Offset at +8, hEvent at +16.
        // SAFETY: a non-null OVERLAPPED passed by the caller of ReadFile.
        let offset = (!overlapped.is_null()).then(|| unsafe { overlapped.add(8).cast::<u32>().read_unaligned() });
        let out = with_redirector(|r| {
            let dst: &mut [u8] = if n == 0 || buf.is_null() {
                &mut []
            } else {
                // SAFETY: the caller's buffer holds `n` bytes.
                unsafe { std::slice::from_raw_parts_mut(buf, n as usize) }
            };
            r.files().read(h as u32, offset, dst)
        });
        if let Some(o) = out {
            // SAFETY: caller-provided out pointers, checked for NULL.
            unsafe {
                if !read.is_null() {
                    read.write_unaligned(o.copied as u32);
                }
                if !overlapped.is_null() {
                    let ev = overlapped.add(16).cast::<usize>().read_unaligned();
                    if ev != 0 {
                        SetEvent(ev as _);
                    }
                }
                if o.eof {
                    SetLastError(saafs::ERROR_HANDLE_EOF);
                }
            }
            return TRUE;
        }
    }
    // SAFETY: the real ReadFile.
    unsafe {
        let f: ReadFileFn = std::mem::transmute(orig(READ));
        f(h, buf, n, read, overlapped)
    }
}

type GetFileSizeFn = unsafe extern "system" fn(usize, *mut u32) -> u32;

unsafe extern "system" fn get_file_size(h: usize, high: *mut u32) -> u32 {
    if saafs::in_fake_range(h as u32)
        && let Some(size) = with_redirector(|r| r.files().size(h as u32))
    {
        if !high.is_null() {
            // SAFETY: caller-provided pointer.
            unsafe { high.write_unaligned(0) };
        }
        return size;
    }
    // SAFETY: the real GetFileSize.
    unsafe {
        let f: GetFileSizeFn = std::mem::transmute(orig(SIZE));
        f(h, high)
    }
}

type SetFilePointerFn = unsafe extern "system" fn(usize, i32, *mut i32, u32) -> u32;

unsafe extern "system" fn set_file_pointer(h: usize, distance: i32, high: *mut i32, method: u32) -> u32 {
    if saafs::in_fake_range(h as u32)
        && let Some(pos) = with_redirector(|r| r.files().seek(h as u32, distance, method))
    {
        return pos;
    }
    // SAFETY: the real SetFilePointer.
    unsafe {
        let f: SetFilePointerFn = std::mem::transmute(orig(SEEK));
        f(h, distance, high, method)
    }
}

type HandleFn = unsafe extern "system" fn(usize) -> u32;

unsafe extern "system" fn close_handle(h: usize) -> u32 {
    if saafs::in_fake_range(h as u32) && with_redirector(|r| r.files().close(h as u32).then_some(())).is_some() {
        return TRUE as u32;
    }
    // SAFETY: the real CloseHandle.
    unsafe {
        let f: HandleFn = std::mem::transmute(orig(CLOSE));
        f(h)
    }
}

unsafe extern "system" fn get_file_type(h: usize) -> u32 {
    if saafs::in_fake_range(h as u32) && with_redirector(|r| r.files().is_open(h as u32).then_some(())).is_some() {
        return 1; // FILE_TYPE_DISK
    }
    // SAFETY: the real GetFileType.
    unsafe {
        let f: HandleFn = std::mem::transmute(orig(TYPE));
        f(h)
    }
}
