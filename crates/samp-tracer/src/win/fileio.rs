//! F6 file-I/O records (`SAMP_TRACER_FILEIO=1`, off by default): wraps gta_sa.exe's import slots
//! of CreateFileA / ReadFile / GetFileSize / SetFilePointer / CloseHandle on top of whatever the
//! client installed, and records what the game got (`crate::fileio`):
//!
//! * `fileio_create` per CreateFileA: `n` (open ordinal), `path` (as the game passed it), `access`,
//!   `disposition`, `class` = `archive` (samp-style fake handle) / `disk` / `fail`.
//! * `fileio_close` per CloseHandle of a tracked handle, and `fileio_end` for handles still open
//!   at quit / exit: `n`, `path`, `class`, `reads`, `bytes`, `extent`, `size` (last GetFileSize
//!   answer, or null), `sha1` (of the reassembled bytes; archive handles, and read-only disk files
//!   up to 8 MB), `holes`.
//!
//! Armed from WinMain's first import call (SystemParametersInfoA), i.e. after every injected DLL's
//! attach, so the wrappers see the client's redirection. Only digests are written, never content.
//! Every wrapper keeps the callee's last-error value.

use super::*;
use crate::fileio::{self as fio, Image};
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{GetLastError, SetLastError};

const ARCHIVE_CAP: u64 = 64 << 20;
const DISK_CAP: u64 = 8 << 20;
const GENERIC_WRITE: u32 = 0x4000_0000;

struct Open {
    n: u32,
    path: String,
    archive: bool,
    write: bool,
    size: Option<u64>,
    img: Image,
}

struct State {
    open: HashMap<u32, Open>,
    n: u32,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static ENDED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Real (next-in-chain) functions, read from the slots when armed.
static NEXT_CREATE: AtomicU32 = AtomicU32::new(0);
static NEXT_READ: AtomicU32 = AtomicU32::new(0);
static NEXT_SIZE: AtomicU32 = AtomicU32::new(0);
static NEXT_SEEK: AtomicU32 = AtomicU32::new(0);
static NEXT_CLOSE: AtomicU32 = AtomicU32::new(0);
static NEXT_SPI: AtomicU32 = AtomicU32::new(0);

fn slot_write(slot: u32, value: u32) -> bool {
    let mut old = 0u32;
    // SAFETY: an import-table slot of the main image (checked readable by the caller), made
    // writable for the 4-byte store and restored.
    unsafe {
        let p = slot as usize as *mut c_void;
        if VirtualProtect(p, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return false;
        }
        (slot as usize as *mut u32).write_volatile(value);
        VirtualProtect(p, 4, old, &mut old);
    }
    true
}

/// Swaps `slot` to `wrapper`, remembering the old value in `next`.
fn wrap(slot: u32, next: &AtomicU32, wrapper: usize) -> bool {
    let Some(cur) = ProcMem.u32(slot).filter(|&v| v != 0) else { return false };
    next.store(cur, Ordering::SeqCst);
    slot_write(slot, wrapper as u32)
}

pub fn init() {
    if std::env::var("SAMP_TRACER_FILEIO").ok().as_deref() != Some("1") {
        return;
    }
    ENABLED.store(true, Ordering::SeqCst);
    if wrap(fio::IAT_SYSTEM_PARAMETERS_INFO_A, &NEXT_SPI, spi_wrap as *const () as usize) {
        tlog("fileio: enabled; arms at WinMain (SystemParametersInfoA)");
    } else {
        tlog("fileio: could not wrap SystemParametersInfoA; arms at the first frame");
    }
}

/// Installs the file wrappers (once). Also called from the frame hook as a fallback.
pub fn arm(from: &str) {
    if !ENABLED.load(Ordering::Relaxed) || ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let ok = [
        wrap(fio::IAT_CREATE_FILE_A, &NEXT_CREATE, create_file_a as *const () as usize),
        wrap(fio::IAT_READ_FILE, &NEXT_READ, read_file as *const () as usize),
        wrap(fio::IAT_GET_FILE_SIZE, &NEXT_SIZE, get_file_size as *const () as usize),
        wrap(fio::IAT_SET_FILE_POINTER, &NEXT_SEEK, set_file_pointer as *const () as usize),
        wrap(fio::IAT_CLOSE_HANDLE, &NEXT_CLOSE, close_handle as *const () as usize),
    ];
    let n = ok.iter().filter(|&&b| b).count();
    tlog(&format!("fileio: armed from {from}: {n}/5 slots wrapped"));
    emit(record("fileio_armed", None, None, None, Val::obj().with("from", from).with("slots", n as u32)));
}

fn state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { open: HashMap::new(), n: 0 }))
}

fn summary(name: &str, o: &Open) -> Val {
    let digest = if o.archive || (!o.write && o.img.kept()) { o.img.digest() } else { None };
    record(
        name,
        None,
        None,
        None,
        Val::obj()
            .with("n", o.n)
            .with("path", o.path.as_str())
            .with("class", if o.archive { "archive" } else { "disk" })
            .with("write", o.write)
            .with("reads", o.img.reads)
            .with("bytes", Val::Int(o.img.bytes as i64))
            .with("extent", Val::Int(o.img.extent as i64))
            .with("size", o.size.map_or(Val::Null, |s| Val::Int(s as i64)))
            .with("sha1", digest.map_or(Val::Null, Val::Str))
            .with("holes", o.img.has_holes()),
    )
}

/// Summaries of the handles still open (`fileio_end`), once; at quit.
pub fn end() {
    if !ARMED.load(Ordering::Relaxed) || ENDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let recs = state(|s| {
        let mut v: Vec<&Open> = s.open.values().collect();
        v.sort_by_key(|o| o.n);
        v.into_iter().map(|o| summary("fileio_end", o)).collect::<Vec<_>>()
    });
    for r in recs {
        emit(r);
    }
    emit(record("fileio_done", None, None, None, Val::obj()));
}

/// Like `end`, from DLL_PROCESS_DETACH: never blocks (other threads may be gone holding a lock).
pub fn end_at_exit(out: &mut Out) {
    if !ARMED.load(Ordering::Relaxed) || ENDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(g) = STATE.try_lock() else { return };
    let Some(s) = g.as_ref() else { return };
    let mut v: Vec<&Open> = s.open.values().collect();
    v.sort_by_key(|o| o.n);
    for o in v {
        push_locked(out, summary("fileio_end", o));
    }
    push_locked(out, record("fileio_done", None, None, None, Val::obj().with("at", "exit")));
}

// ------------------------------------------------------------------ wrappers ----------

type CreateFileAFn = unsafe extern "system" fn(u32, u32, u32, u32, u32, u32, u32) -> u32;
type ReadFileFn = unsafe extern "system" fn(u32, u32, u32, u32, u32) -> i32;
type GetFileSizeFn = unsafe extern "system" fn(u32, u32) -> u32;
type SetFilePointerFn = unsafe extern "system" fn(u32, i32, u32, u32) -> u32;
type HandleFn = unsafe extern "system" fn(u32) -> i32;
type SpiFn = unsafe extern "system" fn(u32, u32, u32, u32) -> i32;

fn next<T: Copy>(a: &AtomicU32) -> T {
    let v = a.load(Ordering::SeqCst) as usize;
    // SAFETY: `v` is the slot's previous value, a function of type T (same import).
    unsafe { std::mem::transmute_copy::<usize, T>(&v) }
}

/// Runs `f` (bookkeeping) without disturbing the thread's last-error value.
fn keep_error(f: impl FnOnce()) {
    // SAFETY: trivial Win32 calls.
    let e = unsafe { GetLastError() };
    let _ = catch_unwind(AssertUnwindSafe(f));
    unsafe { SetLastError(e) };
}

unsafe extern "system" fn spi_wrap(a: u32, b: u32, c: u32, d: u32) -> i32 {
    keep_error(|| arm("WinMain"));
    // SAFETY: the real SystemParametersInfoA, same arguments.
    unsafe { next::<SpiFn>(&NEXT_SPI)(a, b, c, d) }
}

unsafe extern "system" fn create_file_a(
    name: u32,
    access: u32,
    share: u32,
    sec: u32,
    disposition: u32,
    flags: u32,
    template: u32,
) -> u32 {
    // The path as the game passed it (a client may cut a trailing space in place).
    let path = ProcMem.cstr(name, 260).unwrap_or_default();
    // SAFETY: the next CreateFileA in the chain, same arguments.
    let h = unsafe { next::<CreateFileAFn>(&NEXT_CREATE)(name, access, share, sec, disposition, flags, template) };
    keep_error(|| {
        let ok = h != 0 && h != u32::MAX;
        let archive = ok && fio::is_archive_handle(h);
        let write = access & GENERIC_WRITE != 0;
        let class = match (ok, archive) {
            (false, _) => "fail",
            (true, true) => "archive",
            (true, false) => "disk",
        };
        let n = state(|s| {
            s.n += 1;
            let n = s.n;
            if ok {
                let cap = if archive { ARCHIVE_CAP } else { DISK_CAP };
                s.open.insert(h, Open { n, path: path.clone(), archive, write, size: None, img: Image::new(cap) });
            }
            n
        });
        emit(record(
            "fileio_create",
            None,
            None,
            None,
            Val::obj()
                .with("n", n)
                .with("path", path.as_str())
                .with("access", Val::Hex(access))
                .with("disposition", disposition)
                .with("class", class),
        ));
    });
    h
}

unsafe extern "system" fn read_file(h: u32, buf: u32, n: u32, read: u32, ov: u32) -> i32 {
    let tracked = state(|s| s.open.contains_key(&h));
    // Archive reads complete at once; give them a byte count even when the caller passes none.
    let mut local = 0u32;
    let read_ptr = if tracked && read == 0 && fio::is_archive_handle(h) { &mut local as *mut u32 as u32 } else { read };
    // SAFETY: the next ReadFile in the chain.
    let r = unsafe { next::<ReadFileFn>(&NEXT_READ)(h, buf, n, read_ptr, ov) };
    if tracked && r != 0 {
        keep_error(|| {
            let got = if read_ptr != 0 { ProcMem.u32(read_ptr).unwrap_or(0) } else { 0 }.min(n);
            let offset = (ov != 0).then(|| {
                u64::from(ProcMem.u32(ov + 8).unwrap_or(0)) | u64::from(ProcMem.u32(ov + 12).unwrap_or(0)) << 32
            });
            let mut b = vec![0u8; got as usize];
            if got > 0 && !ProcMem.read(buf, &mut b) {
                return;
            }
            state(|s| {
                if let Some(o) = s.open.get_mut(&h) {
                    o.img.read(offset, &b);
                }
            });
        });
    }
    r
}

unsafe extern "system" fn get_file_size(h: u32, high: u32) -> u32 {
    // SAFETY: the next GetFileSize in the chain.
    let r = unsafe { next::<GetFileSizeFn>(&NEXT_SIZE)(h, high) };
    keep_error(|| {
        if r == u32::MAX {
            return;
        }
        let hi = if high != 0 { ProcMem.u32(high).unwrap_or(0) } else { 0 };
        state(|s| {
            if let Some(o) = s.open.get_mut(&h) {
                o.size = Some(u64::from(hi) << 32 | u64::from(r));
            }
        });
    });
    r
}

unsafe extern "system" fn set_file_pointer(h: u32, dist: i32, high: u32, method: u32) -> u32 {
    // SAFETY: the next SetFilePointer in the chain.
    let r = unsafe { next::<SetFilePointerFn>(&NEXT_SEEK)(h, dist, high, method) };
    keep_error(|| {
        if r == u32::MAX {
            return;
        }
        let hi = if high != 0 && !fio::is_archive_handle(h) { ProcMem.u32(high).unwrap_or(0) } else { 0 };
        state(|s| {
            if let Some(o) = s.open.get_mut(&h) {
                o.img.seek(u64::from(hi) << 32 | u64::from(r));
            }
        });
    });
    r
}

unsafe extern "system" fn close_handle(h: u32) -> i32 {
    let mut closed = None;
    keep_error(|| closed = state(|s| s.open.remove(&h)));
    // SAFETY: the next CloseHandle in the chain.
    let r = unsafe { next::<HandleFn>(&NEXT_CLOSE)(h) };
    if let Some(o) = closed {
        keep_error(|| emit(summary("fileio_close", &o)));
    }
    r
}
