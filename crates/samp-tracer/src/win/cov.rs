//! First-hit coverage, Windows side (formats and rules in `crate::cov`). `SAMP_TRACER_COV=<arm list>`
//! arms, at attach (after the tracer's hooks, before the client loads), an `int3` on every listed gta_sa.exe
//! entry whose byte is unmodified, in window `SAMP_TRACER_COV_WINDOW` (default `boot`). The vectored handler
//! records the first hit, restores the byte and resumes; it takes no locks and allocates nothing.
//! Module mode: `SAMP_TRACER_COV_MOD=<RVA list>` arms a second set inside a client module
//! (`SAMP_TRACER_COV_MOD_NAME`, default `samp.dll`) once it loads. `SAMP_TRACER_COV_DUMP=<path>` rewrites
//! the dump every 10 s and at each window start (`{pid}` = process id), so scenarios that never send
//! `cov dump` still leave their coverage behind.

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
/// Module-mode entries (absolute addresses after the module loaded).
static MODST: OnceLock<State> = OnceLock::new();
/// Module name and base once found.
static MODINFO: OnceLock<(String, u32)> = OnceLock::new();
static VEH: AtomicBool = AtomicBool::new(false);
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
    let at = rec.ExceptionAddress as usize as u32;
    let found = [ST.get(), MODST.get()].into_iter().flatten().find_map(|st| {
        [at, ctx.Eip.wrapping_sub(1), ctx.Eip].into_iter().find_map(|a| index_of(st, a)).map(|i| (st, i))
    });
    let Some((st, i)) = found else {
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
    // SAFETY: one byte of gta_sa.exe (or client module) code that we armed ourselves; made writable first.
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

fn states() -> impl Iterator<Item = &'static State> {
    [ST.get(), MODST.get()].into_iter().flatten()
}

fn start_window(name: &str) -> String {
    if ST.get().is_none() && !VEH.load(Ordering::Acquire) {
        return "error cov not enabled (SAMP_TRACER_COV / SAMP_TRACER_COV_MOD)".into();
    }
    let mut ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cur) = ws.last_mut() {
        for st in states() {
            collect(st, cur, true);
        }
    }
    let mut w = Window { name: model::clean_name(name), start_frame: frame(), start_ms: ms(), ..Default::default() };
    for st in states() {
        arm(st, &mut w);
    }
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
    drop(ws);
    autodump();
    msg
}

/// `SAMP_TRACER_COV_DUMP` (`{pid}` is replaced by the process id, for scenarios that run
/// several games); written only once something is armed, so a game without the module never
/// overwrites another game's dump.
fn autodump() {
    if ST.get().is_none() && MODST.get().is_none() {
        return;
    }
    if let Ok(p) = std::env::var("SAMP_TRACER_COV_DUMP") {
        let p = p.replace("{pid}", &std::process::id().to_string());
        let r = dump(&p);
        if r.starts_with("error") {
            tlog(&format!("cov: {r}"));
        }
    }
}

fn caller(ret: u32) -> String {
    if (0x40_0000..0x160_0000).contains(&ret) { format!("{ret:#x}") } else { fmt_loc(ret) }
}

fn dump(path: &str) -> String {
    if ST.get().is_none() && !VEH.load(Ordering::Acquire) {
        return "error cov not enabled (SAMP_TRACER_COV / SAMP_TRACER_COV_MOD)".into();
    }
    let ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = ws.clone();
    drop(ws);
    if let Some(cur) = all.last_mut() {
        for st in states() {
            collect(st, cur, false);
        }
    }
    let hits: usize = all.iter().map(|w| w.hits.len()).sum();
    let mut text = model::format_dump(&all, caller);
    if let Some((n, b)) = MODINFO.get() {
        text.insert_str(0, &format!("# module {n} base={b:#x}\n"));
    }
    match std::fs::write(path, text) {
        Ok(()) => format!("ok cov dump windows={} hits={hits} handled={}", all.len(), HANDLED.load(Ordering::Relaxed)),
        Err(e) => format!("error cov dump {path}: {e}"),
    }
}

fn status() -> String {
    if ST.get().is_none() && !VEH.load(Ordering::Acquire) {
        return "cov off".into();
    }
    let ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let cur = ws.last().map(|w| w.name.as_str()).unwrap_or("-");
    let hit: usize = states().map(|st| st.hit_frame.iter().filter(|f| f.load(Ordering::Relaxed) != 0).count()).sum();
    let entries: usize = states().map(|st| st.entries.len()).sum();
    let module = MODINFO.get().map(|(n, b)| format!(" module={n}@{b:#x}")).unwrap_or_default();
    format!(
        "cov on entries={entries}{module} window={cur} windows={} hit_now={hit} handled={}",
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

fn new_state(entries: Vec<ArmEntry>) -> State {
    let n = entries.len();
    let mk = || (0..n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>();
    State { addrs: entries.iter().map(|e| e.addr).collect(), entries, hit_frame: mk(), hit_ms: mk(), hit_tid: mk(), hit_ret: mk() }
}

fn ensure_veh() -> bool {
    if VEH.load(Ordering::Acquire) {
        return true;
    }
    // SAFETY: registers our handler first in the vectored chain.
    if unsafe { AddVectoredExceptionHandler(1, Some(handler)) }.is_null() {
        tlog("cov: AddVectoredExceptionHandler failed");
        return false;
    }
    VEH.store(true, Ordering::Release);
    true
}

/// At attach, after the tracer's hooks are installed.
pub fn init() {
    if let Ok(path) = std::env::var("SAMP_TRACER_COV") {
        init_exe(&path);
    }
    if let Ok(path) = std::env::var("SAMP_TRACER_COV_MOD") {
        init_module(&path);
    }
    if std::env::var("SAMP_TRACER_COV_DUMP").is_ok() && VEH.load(Ordering::Acquire) {
        let _ = std::thread::Builder::new().name("tracer-cov-dump".into()).spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(10));
                autodump();
            }
        });
    }
}

fn init_exe(path: &str) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tlog(&format!("cov: cannot read {path}: {e}"));
            return;
        }
    };
    let entries: Vec<ArmEntry> =
        model::parse_list(&text).into_iter().filter(|e| (CODE_LO..CODE_HI).contains(&e.addr)).collect();
    let n = entries.len();
    if ST.set(new_state(entries)).is_err() || !ensure_veh() {
        return;
    }
    let w = std::env::var("SAMP_TRACER_COV_WINDOW").unwrap_or_else(|_| "boot".into());
    let msg = start_window(&w);
    tlog(&format!("cov: {n} entries from {path}; {msg}"));
}

/// Module mode: wait for the module, rebase the RVA list, arm it into the current window.
fn init_module(path: &str) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tlog(&format!("cov: cannot read {path}: {e}"));
            return;
        }
    };
    let rvas = model::parse_list(&text);
    if rvas.is_empty() || !ensure_veh() {
        return;
    }
    let name = std::env::var("SAMP_TRACER_COV_MOD_NAME").unwrap_or_else(|_| "samp.dll".into());
    let path = path.to_string();
    let _ = std::thread::Builder::new().name("tracer-cov-mod".into()).spawn(move || {
        let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let t0 = Instant::now();
        let base = loop {
            // SAFETY: NUL-terminated wide string.
            let h = unsafe { GetModuleHandleW(w.as_ptr()) };
            if !h.is_null() {
                break h as usize as u32;
            }
            if t0.elapsed() > Duration::from_secs(600) {
                tlog(&format!("cov: module {name} never loaded"));
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let entries: Vec<ArmEntry> =
            rvas.iter().map(|e| ArmEntry { addr: base.wrapping_add(e.addr), orig: e.orig }).collect();
        let n = entries.len();
        let _ = MODINFO.set((name.clone(), base));
        if MODST.set(new_state(entries)).is_err() {
            return;
        }
        let st = MODST.get().expect("just set");
        let mut ws = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        let msg = if let Some(cur) = ws.last_mut() {
            let before = cur.armed;
            arm(st, cur);
            format!("armed into window {}: {} (skipped now {})", cur.name, cur.armed - before, cur.skipped.len())
        } else {
            let mut cur = Window { name: model::clean_name("boot"), start_frame: frame(), start_ms: ms(), ..Default::default() };
            arm(st, &mut cur);
            let m = format!("armed into new window boot: {} (skipped {})", cur.armed, cur.skipped.len());
            ws.push(cur);
            m
        };
        drop(ws);
        tlog(&format!("cov: module {name} at {base:#x} after {} ms; {n} entries from {path}; {msg}", t0.elapsed().as_millis()));
    });
}
