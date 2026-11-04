//! COV-1 first-hit coverage (Windows side; formats and rules in `crate::cov`).
//!
//! `SAMP_TRACER_COV=<arm list>` (Windows path; `tools/cov/inventory.sh` writes
//! `target/cov/arm.txt`) enables it: at attach — after the tracer's own hooks, before the
//! client DLL loads — every listed entry whose live byte is still the exe's byte gets an
//! `int3`, in window `SAMP_TRACER_COV_WINDOW` (default `boot`). A vectored handler records
//! the first hit (frame, ms, thread, return address), writes the original byte back and
//! resumes at the entry. Control commands: `cov start NAME` (close the window, re-arm all),
//! `cov dump PATH`, `cov status`.
//!
//! The handler takes no locks and allocates nothing: per-entry atomics only.

use super::*;
use crate::cov::{self as model, ArmEntry, Hit, Skip, Window};
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};

struct State {
    entries: Vec<ArmEntry>,
    /// Sorted entry addresses (binary search in the handler).
    addrs: Vec<u32>,
    /// 0 = not hit in the current window, else frame + 1.
    hit_frame: Vec<AtomicU32>,
    hit_ms: Vec<AtomicU32>,
    hit_tid: Vec<AtomicU32>,
    hit_ret: Vec<AtomicU32>,
}

static ST: OnceLock<State> = OnceLock::new();
/// Closed windows followed by the current one (last).
static WINDOWS: Mutex<Vec<Window>> = Mutex::new(Vec::new());
static HANDLED: AtomicU64 = AtomicU64::new(0);
/// gta_sa.exe 1.0 US `.text` + `_rwcseg` (the only sections the arm list may name).
const CODE_LO: u32 = 0x40_1000;
const CODE_HI: u32 = 0x85_7600;

fn hook_ranges() -> Vec<(u32, u32)> {
    HOOKS.iter().map(|s| (s.addr, s.addr + s.prologue.len() as u32)).collect()
}

fn index_of(st: &State, a: u32) -> Option<usize> {
    st.addrs.binary_search(&a).ok()
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    const BREAKPOINT: i32 = 0x8000_0003u32 as i32;
    const CONTINUE_EXECUTION: i32 = -1;
    const CONTINUE_SEARCH: i32 = 0;
    // SAFETY: the system passes valid exception and context records.
    let (rec, ctx) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
    if rec.ExceptionCode != BREAKPOINT {
        return CONTINUE_SEARCH;
    }
    let Some(st) = ST.get() else { return CONTINUE_SEARCH };
    let at = rec.ExceptionAddress as usize as u32;
    let Some(i) = [at, ctx.Eip.wrapping_sub(1), ctx.Eip].into_iter().find_map(|a| index_of(st, a)) else {
        return CONTINUE_SEARCH;
    };
    let e = st.entries[i];
    if st.hit_frame[i].load(Ordering::Relaxed) == 0 {
        // SAFETY: FRAME_COUNTER is a plain int in the game's .data; ESP points at the return
        // address on function entry (the thread's own stack).
        let (f, ret) = unsafe {
            (
                std::ptr::read_volatile(gta::FRAME_COUNTER as usize as *const u32),
                std::ptr::read_volatile(ctx.Esp as usize as *const u32),
            )
        };
        st.hit_ms[i].store(ms() as u32, Ordering::Relaxed);
        st.hit_tid[i].store(tid(), Ordering::Relaxed);
        st.hit_ret[i].store(ret, Ordering::Relaxed);
        st.hit_frame[i].store(f.wrapping_add(1).max(1), Ordering::Release);
    }
    let mut old = 0u32;
    // SAFETY: one byte of gta_sa.exe code that we armed ourselves; made writable first.
    unsafe {
        let p = e.addr as usize as *mut c_void;
        VirtualProtect(p, 1, PAGE_EXECUTE_READWRITE, &mut old);
        std::ptr::write_volatile(p as *mut u8, e.orig);
        FlushInstructionCache(GetCurrentProcess(), p, 1);
    }
    ctx.Eip = e.addr;
    HANDLED.fetch_add(1, Ordering::Relaxed);
    CONTINUE_EXECUTION
}

/// Moves the current window's hits out of the per-entry slots (resetting them).
fn collect(st: &State, w: &mut Window, reset: bool) {
    for (i, e) in st.entries.iter().enumerate() {
        let f = if reset { st.hit_frame[i].swap(0, Ordering::AcqRel) } else { st.hit_frame[i].load(Ordering::Acquire) };
        if f != 0 {
            w.hits.push(Hit {
                addr: e.addr,
                frame: f - 1,
                t_ms: st.hit_ms[i].load(Ordering::Relaxed),
                tid: st.hit_tid[i].load(Ordering::Relaxed),
                ret: st.hit_ret[i].load(Ordering::Relaxed),
            });
        }
    }
}

/// Arms every entry for window `w` (records armed/skipped counts).
fn arm(st: &State, w: &mut Window) {
    let (Some(first), Some(last)) = (st.entries.first(), st.entries.last()) else { return };
    let mut old = 0u32;
    let len = (last.addr - first.addr + 1) as usize;
    // SAFETY: the range lies inside gta_sa.exe's image (.text/_rwcseg); left writable so the
    // handler's restores never fault.
    let ok = unsafe { VirtualProtect(first.addr as usize as *const c_void, len, PAGE_EXECUTE_READWRITE, &mut old) };
    if ok == 0 {
        tlog(&format!("cov: VirtualProtect {:#x}+{len:#x} failed", first.addr));
    }
    let hooks = hook_ranges();
    for e in &st.entries {
        let Some(live) = ProcMem.u8(e.addr) else {
            w.skipped.push((e.addr, Skip::Patched(0)));
            continue;
        };
        match model::arm_decision(*e, live, &hooks) {
            Ok(write) => {
                if write {
                    // SAFETY: writable code byte (see above).
                    unsafe { std::ptr::write_volatile(e.addr as usize as *mut u8, model::INT3) };
                }
                w.armed += 1;
            }
            Err(s) => w.skipped.push((e.addr, s)),
        }
    }
    // SAFETY: trivial Win32 call on our own process.
    unsafe { FlushInstructionCache(GetCurrentProcess(), first.addr as usize as *const c_void, len) };
}

fn start_window(name: &str) -> String {
    let Some(st) = ST.get() else { return "error cov not enabled (SAMP_TRACER_COV)".into() };
    let mut ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cur) = ws.last_mut() {
        collect(st, cur, true);
    }
    let mut w = Window { name: model::clean_name(name), start_frame: frame(), start_ms: ms(), ..Default::default() };
    arm(st, &mut w);
    let msg = format!("ok cov window {} armed={} skipped={}", w.name, w.armed, w.skipped.len());
    tlog(&format!("cov: {msg}"));
    emit(record(
        "cov_window",
        None,
        None,
        None,
        Val::obj().with("name", w.name.as_str()).with("armed", w.armed).with("skipped", w.skipped.len() as u32),
    ));
    ws.push(w);
    msg
}

fn caller(ret: u32) -> String {
    if (0x40_0000..0x160_0000).contains(&ret) { format!("{ret:#x}") } else { fmt_loc(ret) }
}

fn dump(path: &str) -> String {
    let Some(st) = ST.get() else { return "error cov not enabled (SAMP_TRACER_COV)".into() };
    let ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = ws.clone();
    if let Some(cur) = all.last_mut() {
        collect(st, cur, false);
    }
    let hits: usize = all.iter().map(|w| w.hits.len()).sum();
    let text = model::format_dump(&all, caller);
    match std::fs::write(path, text) {
        Ok(()) => format!("ok cov dump windows={} hits={hits} handled={}", all.len(), HANDLED.load(Ordering::Relaxed)),
        Err(e) => format!("error cov dump {path}: {e}"),
    }
}

fn status() -> String {
    let Some(st) = ST.get() else { return "cov off".into() };
    let ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let cur = ws.last().map(|w| w.name.as_str()).unwrap_or("-");
    let hit = st.hit_frame.iter().filter(|f| f.load(Ordering::Relaxed) != 0).count();
    format!(
        "cov on entries={} window={cur} windows={} hit_now={hit} handled={}",
        st.entries.len(),
        ws.len(),
        HANDLED.load(Ordering::Relaxed)
    )
}

/// `cov start NAME` | `cov dump PATH` | `cov status`.
pub fn command(args: &str) -> String {
    let (verb, rest) = args.split_once(' ').unwrap_or((args, ""));
    match verb {
        "start" => start_window(rest),
        "dump" if !rest.trim().is_empty() => dump(rest.trim()),
        "status" | "" => status(),
        _ => format!("error cov: unknown {args:?} (start NAME | dump PATH | status)"),
    }
}

/// At attach, after the tracer's hooks are installed.
pub fn init() {
    let Ok(path) = std::env::var("SAMP_TRACER_COV") else { return };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            tlog(&format!("cov: cannot read {path}: {e}"));
            return;
        }
    };
    let entries: Vec<ArmEntry> =
        model::parse_list(&text).into_iter().filter(|e| (CODE_LO..CODE_HI).contains(&e.addr)).collect();
    let n = entries.len();
    let mk = || (0..n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>();
    let st = State {
        addrs: entries.iter().map(|e| e.addr).collect(),
        entries,
        hit_frame: mk(),
        hit_ms: mk(),
        hit_tid: mk(),
        hit_ret: mk(),
    };
    if ST.set(st).is_err() {
        return;
    }
    // SAFETY: registers our handler first in the vectored chain.
    if unsafe { AddVectoredExceptionHandler(1, Some(handler)) }.is_null() {
        tlog("cov: AddVectoredExceptionHandler failed");
        return;
    }
    let w = std::env::var("SAMP_TRACER_COV_WINDOW").unwrap_or_else(|_| "boot".into());
    let msg = start_window(&w);
    tlog(&format!("cov: {n} entries from {path}; {msg}"));
}
