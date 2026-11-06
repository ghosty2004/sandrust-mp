//! Windows side: DllMain, hook installation (thunks + trampolines), the call
//! dispatcher, the trace writer and the control-channel thread.
//!
//! Hook shape (one 64-byte slot per hook in an RWX block):
//! ```text
//! thunk:  pushad; push esp; push idx; call dispatch; add esp,8; popad; jmp cont
//! tramp:  <relocated prologue>; jmp addr+len
//! ```
//! The function entry gets `jmp thunk` (+ NOPs). `cont` is the trampoline,
//! except for `CRunningScript::ProcessOneCommand`, which continues into a
//! typed wrapper so the script-command context can be closed after the call.

use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HINSTANCE, TRUE};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_GUARD, PAGE_NOACCESS, VirtualAlloc,
    VirtualProtect, VirtualQuery,
};
use windows_sys::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId, TerminateProcess};

use crate::control::{self, Command};
use crate::gta;
use crate::hooks::{self, Call, HOOKS, HookSpec, Policy, Special};
use crate::json::Val;
use crate::mem::Mem;
use crate::scm;
use crate::watch;
use crate::x86;

mod audio;
mod combat;
mod cov;
mod d3d;
mod efx;
mod fileio;
mod remote;
mod vehicles;
mod window;
mod world;

/// Max records per hook per frame (the rest are counted as suppressed).
const CAP_PER_FRAME: u32 = 200;
const SLOT: usize = 64;

// ---------------------------------------------------------------- memory ----------

/// Reads the live process memory, checking readability with VirtualQuery
/// (the last readable region is cached).
struct ProcMem;
static LAST_REGION: AtomicU64 = AtomicU64::new(0);

fn readable(addr: u32, len: usize) -> bool {
    let end = addr as u64 + len as u64;
    let cached = LAST_REGION.load(Ordering::Relaxed);
    let (cb, ce) = (cached >> 32, cached & 0xFFFF_FFFF);
    if addr as u64 >= cb && end <= ce && ce != 0 {
        return true;
    }
    let mut p = addr as u64;
    while p < end {
        // SAFETY: VirtualQuery only inspects the address space.
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let n = unsafe {
            VirtualQuery(p as usize as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
        };
        if n == 0 || mbi.State != MEM_COMMIT || mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
            return false;
        }
        let rb = mbi.BaseAddress as u64;
        let re = rb + mbi.RegionSize as u64;
        if re <= 0xFFFF_FFFF {
            LAST_REGION.store((rb << 32) | re, Ordering::Relaxed);
        }
        p = re;
    }
    true
}

impl Mem for ProcMem {
    fn read(&self, addr: u32, buf: &mut [u8]) -> bool {
        if addr < 0x10000 || !readable(addr, buf.len()) {
            return false;
        }
        // SAFETY: the range was just checked to be committed and readable.
        unsafe { std::ptr::copy_nonoverlapping(addr as usize as *const u8, buf.as_mut_ptr(), buf.len()) };
        true
    }
}

// ---------------------------------------------------------------- output ----------

struct Out {
    buf: String,
    file: Option<File>,
    seq: u64,
}

static OUT: Mutex<Out> = Mutex::new(Out { buf: String::new(), file: None, seq: 0 });
static START: OnceLock<Instant> = OnceLock::new();
static TEXT_LOG: Mutex<Option<File>> = Mutex::new(None);

fn ms() -> u64 {
    START.get().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0)
}

/// Plain-text diagnostics (`samp-tracer.log` next to gta_sa.exe).
fn tlog(msg: &str) {
    if let Ok(mut g) = TEXT_LOG.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = writeln!(f, "[+{:.3}] {msg}", ms() as f64 / 1000.0);
        let _ = f.flush();
    }
}

fn frame() -> u32 {
    ProcMem.u32(gta::FRAME_COUNTER).unwrap_or(0)
}

/// Builds the common record envelope; `seq` is assigned when written.
fn record(name: &str, caller: Option<String>, origin: Option<String>, via: Option<String>, args: Val) -> Val {
    let mut v = Val::obj().with("frame", frame()).with("t", ms()).with("fn", name);
    if let Some(c) = caller {
        v.put("caller", c);
    }
    if let Some(o) = origin {
        v.put("origin", o);
    }
    if let Some(w) = via {
        v.put("via", w);
    }
    v.put("args", args);
    v
}

fn emit(rec: Val) {
    let mut g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    push_locked(&mut g, rec);
}

/// `emit` with the output lock already held.
fn push_locked(g: &mut Out, rec: Val) {
    g.seq += 1;
    let seq = g.seq;
    let mut line = format!("{{\"seq\":{seq},");
    let body = rec.to_json();
    line.push_str(&body[1..]);
    line.push('\n');
    g.buf.push_str(&line);
}

fn flush() {
    let mut g = OUT.lock().unwrap_or_else(|e| e.into_inner());
    flush_locked(&mut g);
}

fn flush_locked(g: &mut Out) {
    if g.buf.is_empty() {
        return;
    }
    let data = std::mem::take(&mut g.buf);
    if let Some(f) = g.file.as_mut() {
        let _ = f.write_all(data.as_bytes());
        let _ = f.flush();
    }
}

// ---------------------------------------------------------------- modules ---------

#[derive(Clone)]
struct Module {
    base: u32,
    end: u32,
    name: String,
    client: bool,
}

static MODULES: Mutex<Vec<Module>> = Mutex::new(Vec::new());

fn is_client_name(n: &str) -> bool {
    n == "samp.dll" || n == "samp_client.dll"
}

fn module_name(base: u32) -> String {
    let mut buf = [0u16; 260];
    // SAFETY: base is a module base from VirtualQuery/GetModuleHandle; buffer is sized.
    let n = unsafe { GetModuleFileNameW(base as usize as *mut c_void, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let full = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    full.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase()
}

fn register_module(base: u32) -> Option<Module> {
    let m = ProcMem;
    if m.u16(base)? != 0x5A4D {
        return None;
    }
    let e = m.u32(base + 0x3C)?;
    let size = m.u32(base + e + 0x50)?;
    let name = module_name(base);
    if name.is_empty() {
        return None;
    }
    let md = Module { base, end: base + size, client: is_client_name(&name), name };
    let mut g = MODULES.lock().unwrap_or_else(|e| e.into_inner());
    if !g.iter().any(|x| x.base == base) {
        g.push(md.clone());
        drop(g);
        if md.client {
            emit(record(
                "module",
                None,
                None,
                None,
                Val::obj().with("name", md.name.as_str()).with("base_ptr", Val::Hex(md.base)),
            ));
            tlog(&format!("client module {} at {:#x}..{:#x}", md.name, md.base, md.end));
        }
    }
    Some(md)
}

fn module_of(addr: u32) -> Option<Module> {
    {
        let g = MODULES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = g.iter().find(|m| addr >= m.base && addr < m.end) {
            return Some(m.clone());
        }
    }
    // SAFETY: VirtualQuery only inspects the address space.
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let n = unsafe {
        VirtualQuery(addr as usize as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
    };
    if n == 0 || mbi.AllocationBase.is_null() {
        return None;
    }
    register_module(mbi.AllocationBase as usize as u32).filter(|m| addr >= m.base && addr < m.end)
}

fn fmt_loc(addr: u32) -> String {
    match module_of(addr) {
        Some(m) => format!("{}+{:#x}", m.name, addr - m.base),
        None => format!("{addr:#x}"),
    }
}

fn refresh_client_modules() {
    for n in ["samp.dll", "samp_client.dll"] {
        let w: Vec<u16> = n.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated wide string.
        let h = unsafe { GetModuleHandleW(w.as_ptr()) };
        if !h.is_null() {
            let base = h as usize as u32;
            let known = MODULES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|m| m.base == base);
            if !known {
                register_module(base);
            }
        }
    }
}

fn client_ranges() -> Vec<(u32, u32)> {
    MODULES.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|m| m.client).map(|m| (m.base, m.end)).collect()
}

/// Whether the bytes before `ret` look like a call instruction.
fn after_call(ret: u32) -> bool {
    let m = ProcMem;
    let mut b = [0u8; 7];
    if !m.read(ret - 7, &mut b) {
        return false;
    }
    let ff2 = |op: u8, modrm: u8| op == 0xFF && (modrm >> 3) & 7 == 2;
    b[2] == 0xE8 // call rel32
        || (ff2(b[5], b[6]) && b[6] >> 6 == 3) // call reg
        || (ff2(b[4], b[5]) && b[5] >> 6 == 1) // call [reg+d8]
        || (ff2(b[1], b[2]) && (b[2] >> 6 == 2 || (b[2] >> 6 == 0 && b[2] & 7 == 5))) // call [reg+d32] / [abs]
        || (ff2(b[3], b[4]) && b[4] >> 6 == 1 && b[4] & 7 == 4) // call [sib+d8]
        || (ff2(b[0], b[1]) && b[1] >> 6 == 2 && b[1] & 7 == 4) // call [sib+d32]
}

fn stack_bounds() -> (u32, u32) {
    let (base, limit): (u32, u32);
    // SAFETY: reads StackBase/StackLimit from the current thread's TIB (fs:[4], fs:[8]).
    unsafe {
        std::arch::asm!("mov {0}, dword ptr fs:[4]", "mov {1}, dword ptr fs:[8]", out(reg) base, out(reg) limit, options(nostack, readonly));
    }
    (limit, base)
}

/// Up to 3 return addresses into client modules found on the stack above
/// `esp` (innermost first), formatted as `samp.dll+0x… < samp.dll+0x…`.
fn client_origin(esp: u32) -> Option<String> {
    let ranges = client_ranges();
    if ranges.is_empty() {
        return None;
    }
    let (_, top) = stack_bounds();
    let end = top.min(esp.saturating_add(0x4000));
    let mut p = esp;
    let mut found = Vec::new();
    while p + 4 <= end && found.len() < 3 {
        // SAFETY: p lies inside the current thread's committed stack (between esp and StackBase).
        let d = unsafe { std::ptr::read_unaligned(p as usize as *const u32) };
        if ranges.iter().any(|&(a, b)| d >= a + 7 && d < b) && after_call(d) {
            found.push(fmt_loc(d));
        }
        p += 4;
    }
    (!found.is_empty()).then(|| found.join(" < "))
}

/// Up to `n` gta_sa.exe return addresses found on the stack above `esp` (heuristic, like
/// `client_origin`): which GTA code path led to a ped/vehicle creation (TRACE-4).
fn gta_stack(esp: u32, n: usize) -> Option<String> {
    let (_, top) = stack_bounds();
    let end = top.min(esp.saturating_add(0x2000));
    let mut p = esp;
    let mut found = Vec::new();
    while p + 4 <= end && found.len() < n {
        // SAFETY: p lies inside the current thread's committed stack (between esp and StackBase).
        let d = unsafe { std::ptr::read_unaligned(p as usize as *const u32) };
        if (0x401007..0x857000).contains(&d) && after_call(d) {
            found.push(format!("{d:#x}"));
        }
        p += 4;
    }
    (!found.is_empty()).then(|| found.join(" < "))
}

/// Hooks whose records carry `gta_stack` (creation paths of peds and vehicles).
fn wants_gta_stack(addr: u32) -> bool {
    matches!(addr, 0x5E8030 | 0x5DDC60 | 0x6D5F10 | 0x613A00 | 0x614210 | 0x562470 | 0x562570 | 0x562410 | 0x562430)
}

// ---------------------------------------------------------------- hooks -----------

struct HookRt {
    tramp: AtomicU32,
    installed: AtomicBool,
    calls: AtomicU32,
    logged: AtomicU32,
    suppressed: AtomicU32,
}

static RT: OnceLock<Vec<HookRt>> = OnceLock::new();
static FRAME_TICKS: AtomicU64 = AtomicU64::new(0);
static STOCK_CMDS: AtomicU32 = AtomicU32::new(0);
static FRAME_HOOK_SITE: AtomicU32 = AtomicU32::new(0);
static THUNK_OF_FRAME: AtomicU32 = AtomicU32::new(0);

fn rt() -> &'static [HookRt] {
    RT.get().map(|v| v.as_slice()).unwrap_or(&[])
}

/// Registers saved by `pushad`, lowest address first.
#[repr(C)]
struct Regs {
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

extern "C" fn dispatch(idx: u32, regs: *const Regs) {
    // SAFETY: the thunk passes a pointer to the pushad block on its own stack.
    let r = unsafe { &*regs };
    let _ = catch_unwind(AssertUnwindSafe(|| on_call(idx as usize, r)));
}

/// Script-command context (main thread only in practice; tagged by thread id).
struct ScmFrame {
    tid: u32,
    this: u32,
    ip: u32,
    word: u16,
    script: String,
    client: Option<ClientCmd>,
}

struct ClientCmd {
    frame: u32,
    t: u64,
    caller: String,
    origin: Option<String>,
    pending: Vec<Val>,
}

static SCM: Mutex<Vec<ScmFrame>> = Mutex::new(Vec::new());

fn tid() -> u32 {
    // SAFETY: trivial Win32 call.
    unsafe { GetCurrentThreadId() }
}

fn on_call(idx: usize, r: &Regs) {
    audio::poll();
    if idx & TABLE_TAG != 0 {
        return stock_cmd(r);
    }
    let (Some(spec), Some(h)) = (HOOKS.get(idx), rt().get(idx)) else { return };
    h.calls.fetch_add(1, Ordering::Relaxed);
    match spec.special {
        Special::Frame => return on_frame(),
        Special::Scm => return scm_enter(r),
        Special::StockScript => return stock_enter(r),
        Special::FileSeek => return capture::on_seek(r.esp),
        Special::FileClose => return capture::on_close(r.esp),
        Special::FileRead | Special::FileReadLine => return,
        Special::FileOpen | Special::FileLoad => {}
        Special::PedProcessControl => return remote::on_pc_call(r),
        Special::WorldProcess => return,
        Special::VehProcessControl => return vehicles::on_vpc_call(r),
        Special::ObjRender => return world::on_obj_render(spec, r),
        Special::Los => return world::on_los_call(r),
        Special::PedRender => return,
        Special::WndProc => {
            if !(0x100..=0x109).contains(&ProcMem.u32(r.esp + 8).unwrap_or(0)) {
                return;
            }
        }
        Special::None => {}
    }
    match spec.addr {
        0x5E8030 => remote::on_ped_ctor(r.ecx),
        0x5E8620 => remote::on_ped_dtor(r.ecx),
        0x52B730 => combat::on_camera_process(),
        _ => {}
    }
    if spec.policy == Policy::Count {
        return;
    }
    let m = ProcMem;
    let call = Call { ecx: r.ecx, esp: r.esp };
    let ret = m.u32(r.esp).unwrap_or(0);
    let caller = module_of(ret);
    let caller_is_client = caller.as_ref().is_some_and(|c| c.client);
    let me = tid();
    let (via, in_client_cmd) = {
        let scm = SCM.lock().unwrap_or_else(|e| e.into_inner());
        match scm.iter().rev().find(|f| f.tid == me) {
            Some(f) => (Some(via_of(f)), f.client.is_some()),
            None => (None, false),
        }
    };
    if spec.policy == Policy::Relevant && !caller_is_client && via.is_none() && !hooks::involves_player(&m, spec, &call)
    {
        return;
    }
    if h.logged.fetch_add(1, Ordering::Relaxed) >= CAP_PER_FRAME {
        h.suppressed.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let caller_s = caller.map(|c| format!("{}+{:#x}", c.name, ret - c.base)).or(Some(format!("{ret:#x}")));
    let origin = if caller_is_client { None } else { client_origin(r.esp + 4) };
    let mut rec = record(spec.name, caller_s, origin, via, hooks::render_args(&m, spec, &call));
    if wants_gta_stack(spec.addr)
        && let Some(st) = gta_stack(r.esp + 4, 6)
    {
        rec.put("gta_stack", st);
    }
    if in_client_cmd {
        // Written after the command's own `scm` record (see scm_leave).
        let mut scm = SCM.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = scm.iter_mut().rev().find(|f| f.tid == me).and_then(|f| f.client.as_mut()) {
            c.pending.push(rec);
            return;
        }
        drop(scm);
    }
    emit(rec);
}

/// Marks a stock script's turn (no single opcode known: the interpreter loop is inlined).
const NO_OPCODE: u16 = 0xFFFF;

fn via_of(f: &ScmFrame) -> String {
    let (op, not) = scm::opcode(f.word);
    let n = if not { "!" } else { "" };
    match (&f.client, f.word) {
        (Some(_), _) => format!("scm {n}{op:#06x}"),
        (None, NO_OPCODE) => format!("stock {}", f.script),
        (None, _) => format!("stock {} {n}{op:#06x}", f.script),
    }
}

fn stock_enter(r: &Regs) {
    STOCK_CMDS.fetch_add(1, Ordering::Relaxed);
    let m = ProcMem;
    let this = r.ecx;
    let script = m.cstr(this + gta::script::NAME, 8).unwrap_or_default();
    let ip = m.u32(this + gta::script::CURRENT_IP).unwrap_or(0);
    SCM.lock().unwrap_or_else(|e| e.into_inner()).push(ScmFrame {
        tid: tid(),
        this,
        ip,
        word: NO_OPCODE,
        script,
        client: None,
    });
}

fn scm_enter(r: &Regs) {
    let m = ProcMem;
    let this = r.ecx;
    let ip = m.u32(this + gta::script::CURRENT_IP).unwrap_or(0);
    let word = m.u16(ip).unwrap_or(0xFFFF);
    let ret = m.u32(r.esp).unwrap_or(0);
    let caller = module_of(ret);
    let client = if caller.as_ref().is_some_and(|c| c.client) {
        Some(ClientCmd {
            frame: frame(),
            t: ms(),
            caller: fmt_loc(ret),
            origin: client_origin(r.esp + 4),
            pending: Vec::new(),
        })
    } else {
        STOCK_CMDS.fetch_add(1, Ordering::Relaxed);
        None
    };
    let script = m.cstr(this + gta::script::NAME, 8).unwrap_or_default();
    SCM.lock().unwrap_or_else(|e| e.into_inner()).push(ScmFrame { tid: tid(), this, ip, word, script, client });
}

fn scm_leave(this: u32) {
    let me = tid();
    let f = {
        let mut g = SCM.lock().unwrap_or_else(|e| e.into_inner());
        match g.iter().rposition(|f| f.tid == me && f.this == this) {
            Some(i) => g.remove(i),
            None => return,
        }
    };
    let Some(c) = f.client else { return };
    let m = ProcMem;
    let after = m.u32(this + gta::script::CURRENT_IP).unwrap_or(0);
    let len = after.wrapping_sub(f.ip);
    let exact = (2..256).contains(&len);
    let n = if exact { len as usize - 2 } else { 64 };
    let mut raw = vec![0u8; n];
    let ok = n == 0 || m.read(f.ip + 2, &mut raw);
    let (params, clean) = if ok { scm::decode_params(&raw, 32) } else { (Vec::new(), false) };
    let (op, not) = scm::opcode(f.word);
    let mut args = Val::obj().with("op", format!("{op:#06x}"));
    if not {
        args.put("not", true);
    }
    args.put("params", Val::List(params.iter().map(|p| p.to_val()).collect()));
    // Values of variable parameters after the command (outputs such as created handles).
    let outs: Vec<Val> = params
        .iter()
        .filter_map(|p| match p {
            scm::Param::LocalVar(i) => Some(Val::Str(format!(
                "l{i}={}",
                m.i32(this + gta::script::LOCAL_VARS + 4 * u32::from(*i)).unwrap_or(0)
            ))),
            scm::Param::GlobalVar(o) => {
                Some(Val::Str(format!("g{o}={}", m.i32(gta::SCRIPT_SPACE + u32::from(*o)).unwrap_or(0))))
            }
            _ => None,
        })
        .collect();
    if !outs.is_empty() {
        args.put("out", Val::List(outs));
    }
    if !exact || !clean {
        args.put("decode", if exact { "partial" } else { "greedy" });
    }
    if !f.script.is_empty() {
        args.put("script", f.script.as_str());
    }
    let mut rec = Val::obj().with("frame", c.frame).with("t", c.t).with("fn", "scm").with("caller", c.caller);
    if let Some(o) = c.origin {
        rec.put("origin", o);
    }
    rec.put("args", args);
    emit(rec);
    for p in c.pending {
        emit(p);
    }
}

fn tramp_of(special: Special) -> u32 {
    HOOKS
        .iter()
        .position(|s| s.special == special)
        .and_then(|i| rt().get(i))
        .map(|h| h.tramp.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// Typed continuation for `CRunningScript::ProcessOneCommand` and
/// `CRunningScript::Process` (thiscall, no arguments, result in AL; the whole
/// EAX is passed through).
extern "thiscall" fn process_one_command_wrap(this: u32) -> u32 {
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: extern "thiscall" fn(u32) -> u32 = unsafe { std::mem::transmute(tramp_of(Special::Scm) as usize) };
    let r = f(this);
    let _ = catch_unwind(AssertUnwindSafe(|| scm_leave(this)));
    r
}

extern "thiscall" fn running_script_process_wrap(this: u32) -> u32 {
    // SAFETY: as above.
    let f: extern "thiscall" fn(u32) -> u32 = unsafe { std::mem::transmute(tramp_of(Special::StockScript) as usize) };
    let r = f(this);
    let _ = catch_unwind(AssertUnwindSafe(|| scm_leave(this)));
    r
}

// ---------------------------------------------------------------- per frame -------

struct FrameState {
    prev: watch::Snap,
    last_pos_frame: u64,
    globals: Vec<u32>,
    globals_off: u32,
}

fn read_globals(m: &ProcMem) -> (Vec<u32>, u32) {
    let Some((a, b)) = watch::globals_range(m) else { return (Vec::new(), 0) };
    let mut raw = vec![0u8; (b - a) as usize];
    if !m.read(gta::SCRIPT_SPACE + a, &mut raw) {
        return (Vec::new(), 0);
    }
    (raw.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(), a)
}

static FRAME_STATE: Mutex<Option<FrameState>> = Mutex::new(None);

fn on_frame() {
    fileio::arm("frame");
    let n = FRAME_TICKS.fetch_add(1, Ordering::SeqCst) + 1;
    let m = ProcMem;
    let mut extra: Vec<(&'static str, String)> = Vec::new();
    let stock = STOCK_CMDS.swap(0, Ordering::Relaxed);
    extra.push(("stock_script_cmds", if stock > 0 { "on" } else { "off" }.into()));
    for (spec, h) in HOOKS.iter().zip(rt()) {
        let calls = h.calls.swap(0, Ordering::Relaxed);
        h.logged.store(0, Ordering::Relaxed);
        let sup = h.suppressed.swap(0, Ordering::Relaxed);
        if sup > 0 {
            emit(record("suppressed", None, None, None, Val::obj().with("hook", spec.name).with("count", sup)));
        }
        if spec.policy == Policy::Count && spec.special == Special::None {
            extra.push((spec.name, if calls > 0 { "on" } else { "off" }.into()));
        }
    }
    let cur = watch::collect(&m, &extra);
    let mut g = FRAME_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = g.get_or_insert_with(|| FrameState {
        prev: Vec::new(),
        last_pos_frame: 0,
        globals: Vec::new(),
        globals_off: 0,
    });
    if let Some(d) = watch::diff(&st.prev, &cur) {
        emit(record("watch", None, None, None, d));
    }
    st.prev = cur;
    let (gl, goff) = read_globals(&m);
    if goff == st.globals_off && gl.len() == st.globals.len() {
        if let Some(d) = watch::diff_words(&st.globals, &gl, goff, 64) {
            emit(record("globals", None, None, None, d));
        }
    } else if !gl.is_empty() {
        emit(record("globals_base", None, None, None, Val::obj().with("bytes", (gl.len() * 4) as u32)));
    }
    st.globals = gl;
    st.globals_off = goff;
    if n == 1 {
        emit(record("first_frame", None, None, None, Val::obj()));
    }
    if n >= st.last_pos_frame + 30 {
        st.last_pos_frame = n;
        let p = gta::player_ped(&m);
        if p != 0
            && let Some(pos) = gta::entity_pos(&m, p)
        {
            emit(record(
                "pos",
                None,
                None,
                None,
                Val::obj()
                    .with("pos", Val::List(pos.iter().map(|&f| f.into()).collect()))
                    .with("heading", m.f32(p + gta::off::PED_HEADING)),
            ));
        }
    }
    drop(g);
    remote::frame(n);
    vehicles::frame();
    world::frame(n);
    combat::frame();
    audio::on_frame();
    d3d::frame();
    efx::frame();
    flush();
}

// ---------------------------------------------------------------- install ---------

fn build_thunk(slot: u32, idx: u32, cont: Option<u32>, spec: &HookSpec) -> Result<(Vec<u8>, u32), String> {
    let dispatch_addr = dispatch as extern "C" fn(u32, *const Regs) as usize as u32;
    let tramp = slot + 21;
    let mut t = vec![0x60, 0x54, 0x68];
    t.extend_from_slice(&idx.to_le_bytes());
    t.push(0xE8);
    t.extend_from_slice(&x86::rel32_to(slot + 7, 5, dispatch_addr).to_le_bytes());
    t.extend_from_slice(&[0x83, 0xC4, 0x08, 0x61, 0xE9]);
    t.extend_from_slice(&x86::rel32_to(slot + 16, 5, cont.unwrap_or(tramp)).to_le_bytes());
    debug_assert_eq!(t.len(), 21);
    let (moved, n) = x86::relocate(spec.prologue, spec.addr, tramp, 5).map_err(|e| e.to_string())?;
    if n != spec.prologue.len() {
        return Err(format!("decoder steals {n} bytes, table says {}", spec.prologue.len()));
    }
    t.extend_from_slice(&moved);
    let back = slot + t.len() as u32;
    t.push(0xE9);
    t.extend_from_slice(&x86::rel32_to(back, 5, spec.addr + n as u32).to_le_bytes());
    if t.len() > SLOT {
        return Err("thunk too long".into());
    }
    Ok((t, tramp))
}

// ------------------------------------------------- optional stock-script command trace ------
//
// `CRunningScript::Process` dispatches each command through a table of 27
// handler pointers (`ProcessCommands0To99` … `2600To2699`) at 0x8A6168
// (indexed by opcode / 100; handlers named via plugin-sdk / gta-reversed
// `RunningScript.cpp`). With `SAMP_TRACER_SCRIPTS=name,name|*` the tracer
// points these entries at thunks and logs every command of the named stock
// scripts as `stock_cmd` (opcode + IP offset) for the first
// `SAMP_TRACER_SCRIPT_FRAMES` frames (default 3000). Off by default.

const HANDLER_TABLE: u32 = 0x8A6168;
const HANDLER_COUNT: u32 = 27;
const TABLE_TAG: usize = 0x10000;

struct ScriptFilter {
    names: Vec<String>,
    all: bool,
    frames: u64,
}

static SCRIPT_FILTER: OnceLock<ScriptFilter> = OnceLock::new();

fn stock_cmd(r: &Regs) {
    let Some(f) = SCRIPT_FILTER.get() else { return };
    if FRAME_TICKS.load(Ordering::Relaxed) > f.frames {
        return;
    }
    let m = ProcMem;
    let this = r.ecx;
    let name = m.cstr(this + gta::script::NAME, 8).unwrap_or_default();
    if !f.all && !f.names.contains(&name) {
        return;
    }
    let op = m.u32(r.esp + 4).unwrap_or(0) & 0x7FFF;
    let ip = m.u32(this + gta::script::CURRENT_IP).unwrap_or(0).wrapping_sub(gta::SCRIPT_SPACE).wrapping_sub(2);
    let not = m.u8(this + 0xD2).unwrap_or(0) != 0;
    let mut args = Val::obj().with("script", name.as_str()).with("op", format!("{op:#06x}")).with("ip", ip);
    if not {
        args.put("not", true);
    }
    emit(record("stock_cmd", None, None, None, args));
}

fn install_table_thunks() -> Result<u32, String> {
    let Ok(spec) = std::env::var("SAMP_TRACER_SCRIPTS") else { return Ok(0) };
    let names: Vec<String> = spec.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let frames = std::env::var("SAMP_TRACER_SCRIPT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let _ = SCRIPT_FILTER.set(ScriptFilter { all: names.iter().any(|n| n == "*"), names, frames });
    // SAFETY: plain allocation of an executable block for the table thunks.
    let block = unsafe {
        VirtualAlloc(std::ptr::null(), (HANDLER_COUNT * 32) as usize, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
    } as usize as u32;
    if block == 0 {
        return Err("VirtualAlloc failed".into());
    }
    let dispatch_addr = dispatch as extern "C" fn(u32, *const Regs) as usize as u32;
    for k in 0..HANDLER_COUNT {
        let entry = HANDLER_TABLE + 4 * k;
        let orig = ProcMem.u32(entry).ok_or("handler table unreadable")?;
        if !(0x401000..0x857000).contains(&orig) {
            return Err(format!("handler table entry {k} = {orig:#x} is not game code"));
        }
        let slot = block + 32 * k;
        let mut t = vec![0x60, 0x54, 0x68];
        t.extend_from_slice(&((TABLE_TAG as u32) | k).to_le_bytes());
        t.push(0xE8);
        t.extend_from_slice(&x86::rel32_to(slot + 7, 5, dispatch_addr).to_le_bytes());
        t.extend_from_slice(&[0x83, 0xC4, 0x08, 0x61, 0xE9]);
        t.extend_from_slice(&x86::rel32_to(slot + 16, 5, orig).to_le_bytes());
        // SAFETY: slot is inside our RWX block; the table lives in the game's writable .data.
        unsafe {
            std::ptr::copy_nonoverlapping(t.as_ptr(), slot as usize as *mut u8, t.len());
            std::ptr::write_volatile(entry as usize as *mut u32, slot);
        }
    }
    Ok(HANDLER_COUNT)
}

// ------------------------------------------------- optional stock boot (no client) ----------
//
// `SAMP_TRACER_BOOT=1` (only for runs WITHOUT a client DLL): boot straight
// into a new single-player game the same way samp-client M1 does
// (gta-patches CSV rows 120, 377, 378): gGameState := 5 at attach; at
// state 7 the three FrontEndMenuManager flags and gGameState := 8. Gives a
// stock baseline trace of main.scm on the same boot path. Row 121 (keep
// gGameState in psInitialize) is applied too, otherwise the 5 is overwritten.

fn stock_boot_attach() -> bool {
    if std::env::var("SAMP_TRACER_BOOT").ok().as_deref() != Some("1") {
        return false;
    }
    // CSV row 121: psInitialize would reset gGameState to 0; NOP that store (6 bytes).
    const PS_INIT_STORE: u32 = 0x747483;
    if ProcMem.u8(PS_INIT_STORE + 1) == Some(0x35) {
        let mut old = 0u32;
        // SAFETY: verified game code; the main thread is still suspended.
        unsafe {
            let p = PS_INIT_STORE as usize as *mut c_void;
            VirtualProtect(p, 6, PAGE_EXECUTE_READWRITE, &mut old);
            std::ptr::write_bytes(p as *mut u8, 0x90, 6);
            VirtualProtect(p, 6, old, &mut old);
            FlushInstructionCache(GetCurrentProcess(), p, 6);
        }
    }
    // SAFETY: plain int in the game's .data; the main thread is still suspended.
    unsafe { std::ptr::write_volatile(gta::G_GAME_STATE as usize as *mut i32, 5) };
    let _ = std::thread::Builder::new().name("tracer-boot".into()).spawn(|| {
        let t0 = Instant::now();
        while ProcMem.i32(gta::G_GAME_STATE) != Some(7) {
            if t0.elapsed() > Duration::from_secs(120) {
                tlog("stock boot: state 7 not reached");
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mm = gta::MENU_MANAGER;
        // SAFETY: plain bytes/ints in the game's .data.
        unsafe {
            std::ptr::write_volatile((mm + 0xE9) as usize as *mut u8, 1);
            std::ptr::write_volatile((mm + 0x5C) as usize as *mut u8, 0);
            std::ptr::write_volatile((mm + 0x33) as usize as *mut u8, 0);
            std::ptr::write_volatile(gta::G_GAME_STATE as usize as *mut i32, 8);
        }
        tlog("stock boot: menu flags + gGameState := 8");
    });
    true
}

fn install_all() -> (usize, Vec<String>) {
    let mut failed = Vec::new();
    // SAFETY: plain allocation of an executable block for the thunks.
    let block =
        unsafe { VirtualAlloc(std::ptr::null(), HOOKS.len() * SLOT, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) }
            as usize as u32;
    let rts: Vec<HookRt> = HOOKS
        .iter()
        .map(|_| HookRt {
            tramp: AtomicU32::new(0),
            installed: AtomicBool::new(false),
            calls: AtomicU32::new(0),
            logged: AtomicU32::new(0),
            suppressed: AtomicU32::new(0),
        })
        .collect();
    let _ = RT.set(rts);
    if block == 0 {
        return (0, vec!["VirtualAlloc failed".into()]);
    }
    let wrap_scm = process_one_command_wrap as extern "thiscall" fn(u32) -> u32 as usize as u32;
    let wrap_stock = running_script_process_wrap as extern "thiscall" fn(u32) -> u32 as usize as u32;
    let mut ok = 0;
    for (i, spec) in HOOKS.iter().enumerate() {
        let slot = block + (i * SLOT) as u32;
        let mut cur = vec![0u8; spec.prologue.len()];
        if !ProcMem.read(spec.addr, &mut cur) || cur != spec.prologue {
            failed.push(format!("{} {:#x}: unexpected bytes {:02x?}", spec.name, spec.addr, cur));
            continue;
        }
        let cont = match spec.special {
            Special::Scm => Some(wrap_scm),
            Special::StockScript => Some(wrap_stock),
            Special::FileOpen => Some(capture::open_wrap as extern "C" fn(u32, u32) -> u32 as usize as u32),
            Special::FileRead => Some(capture::read_wrap as extern "C" fn(u32, u32, u32) -> u32 as usize as u32),
            Special::FileReadLine => {
                Some(capture::read_line_wrap as extern "C" fn(u32, u32, u32) -> u32 as usize as u32)
            }
            Special::FileLoad => {
                Some(capture::load_file_wrap as extern "C" fn(u32, u32, u32, u32) -> u32 as usize as u32)
            }
            Special::PedProcessControl => Some(remote::pc_inner_wrap as extern "thiscall" fn(u32) as usize as u32),
            Special::WorldProcess => Some(remote::world_process_wrap as extern "C" fn() as usize as u32),
            Special::VehProcessControl => Some(vehicles::vpc_inner_wrap as extern "thiscall" fn(u32) as usize as u32),
            Special::Los => Some(world::los_wrap as world::LosWrapFn as usize as u32),
            Special::PedRender => Some(world::ped_render_wrap as extern "thiscall" fn(u32) as usize as u32),
            _ => None,
        };
        let (code, tramp) = match build_thunk(slot, i as u32, cont, spec) {
            Ok(x) => x,
            Err(e) => {
                failed.push(format!("{}: {e}", spec.name));
                continue;
            }
        };
        // SAFETY: slot lies in our RWX block and has SLOT bytes.
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), slot as usize as *mut u8, code.len()) };
        let h = &rt()[i];
        h.tramp.store(tramp, Ordering::SeqCst);
        let mut patch = vec![0xE9];
        patch.extend_from_slice(&x86::rel32_to(spec.addr, 5, slot).to_le_bytes());
        patch.resize(spec.prologue.len(), 0x90);
        let mut old = 0u32;
        // SAFETY: patching game code we just verified; the game's main thread is
        // still suspended by the launcher while DllMain runs.
        unsafe {
            let p = spec.addr as usize as *mut c_void;
            if VirtualProtect(p, patch.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                failed.push(format!("{}: VirtualProtect failed", spec.name));
                continue;
            }
            std::ptr::copy_nonoverlapping(patch.as_ptr(), p as *mut u8, patch.len());
            VirtualProtect(p, patch.len(), old, &mut old);
            FlushInstructionCache(GetCurrentProcess(), p, patch.len());
        }
        h.installed.store(true, Ordering::SeqCst);
        if spec.special == Special::Frame {
            FRAME_HOOK_SITE.store(spec.addr, Ordering::SeqCst);
            THUNK_OF_FRAME.store(slot, Ordering::SeqCst);
        }
        ok += 1;
    }
    (ok, failed)
}

fn frame_hook_installed() -> bool {
    let site = FRAME_HOOK_SITE.load(Ordering::SeqCst);
    let thunk = THUNK_OF_FRAME.load(Ordering::SeqCst);
    site != 0
        && ProcMem.u8(site) == Some(0xE9)
        && ProcMem.i32(site + 1).map(|r| x86::rel32_target(site, 5, r)) == Some(thunk)
}

// ---------------------------------------------------------------- control ---------

fn handle(cmd: &Command) -> (String, bool) {
    match cmd {
        Command::Ping => ("pong".into(), false),
        Command::State => match gta::snapshot(&ProcMem) {
            Some(s) => (
                format!(
                    "{} frame_hook={}",
                    gta::format_state(&s, FRAME_TICKS.load(Ordering::SeqCst)),
                    if frame_hook_installed() { "installed" } else { "absent" }
                ),
                false,
            ),
            None => ("error game state unreadable".into(), false),
        },
        Command::Peek { addr, len } => {
            let mut buf = vec![0u8; *len];
            if ProcMem.read(*addr, &mut buf) {
                let hex: Vec<String> = buf.iter().map(|b| format!("{b:02x}")).collect();
                (format!("peek {addr:#x} {}", hex.join(" ")), false)
            } else {
                (format!("error {addr:#x}+{len} is not readable"), false)
            }
        }
        Command::Remote => (gta::format_remote(&ProcMem, &gta::scan_cached_remote()), false),
        Command::Mark(text) => {
            emit(record("mark", None, None, None, Val::obj().with("text", text.as_str())));
            flush();
            d3d::mark(text);
            efx::mark(text);
            ("ok".into(), false)
        }
        Command::D3d(args) if args == "busy" => ((if d3d::busy() { "busy" } else { "idle" }).into(), false),
        Command::D3d(args) => (d3d::command(args), false),
        Command::Cov(args) => (cov::command(args), false),
        Command::Window(args) => (window::command(args), false),
        Command::HidePed(on) => {
            world::HIDE_LOCAL_PED.store(*on, Ordering::SeqCst);
            emit(record("hideped", None, None, None, Val::obj().with("on", *on)));
            flush();
            (format!("ok hideped {}", if *on { "on" } else { "off" }), false)
        }
        Command::Efx(args) => (efx::command(args), false),
        Command::Poke { addr, bytes } => {
            if !ProcMem.read(*addr, &mut vec![0u8; bytes.len()]) {
                return (format!("error {addr:#x} is not readable"), false);
            }
            let mut old = 0u32;
            // SAFETY: test-harness write to a committed, readable range (made writable for the store).
            unsafe {
                let p = *addr as usize as *mut c_void;
                VirtualProtect(p, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
                VirtualProtect(p, bytes.len(), old, &mut old);
            }
            emit(record(
                "poke",
                None,
                None,
                None,
                Val::obj().with("addr", Val::Hex(*addr)).with("bytes", bytes.len() as u32),
            ));
            (format!("ok poke {addr:#x} {}", bytes.len()), false)
        }
        Command::Quit => {
            request_quit();
            ("bye".into(), false)
        }
        Command::Unknown(c) => (format!("error unknown command {c:?}"), false),
    }
}

static QUIT: AtomicBool = AtomicBool::new(false);

fn request_quit() {
    if QUIT.swap(true, Ordering::SeqCst) {
        return;
    }
    tlog("quit requested: setting RsGlobal.quit");
    remote::summaries();
    fileio::end();
    emit(record("quit", None, None, None, Val::obj()));
    flush();
    d3d::flush();
    // SAFETY: RsGlobal.quit is a plain int in the game's .data.
    unsafe { std::ptr::write_volatile(gta::RS_GLOBAL_QUIT as usize as *mut u32, 1) };
    let _ = std::thread::Builder::new().name("tracer-quit".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(8));
        tlog("game still running 8 s after quit; terminating");
        flush();
        // SAFETY: terminating our own process.
        unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    });
}

fn control_thread(port: u16) {
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => {
            tlog(&format!("control: listening on 127.0.0.1:{port}"));
            control::serve(l, |c| catch_unwind(AssertUnwindSafe(|| handle(c))).unwrap_or(("error".into(), false)));
        }
        Err(e) => tlog(&format!("control: bind 127.0.0.1:{port} failed: {e}")),
    }
}

fn flusher_thread() {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let _ = catch_unwind(|| {
            refresh_client_modules();
            flush();
            d3d::flush();
        });
    }
}

// ---------------------------------------------------------------- entry -----------

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // SAFETY: valid module handle passed by the loader.
        unsafe { DisableThreadLibraryCalls(hinst) };
        let _ = catch_unwind(attach);
    } else if reason == DLL_PROCESS_DETACH {
        // Other threads may have been killed while holding the lock: never block here.
        if let Ok(mut g) = OUT.try_lock() {
            fileio::end_at_exit(&mut g);
            flush_locked(&mut g);
        }
    }
    TRUE
}

fn attach() {
    let _ = START.set(Instant::now());
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if let Ok(f) = File::create(dir.join("samp-tracer.log")) {
        *TEXT_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
    std::panic::set_hook(Box::new(|info| tlog(&format!("PANIC: {info}"))));
    let out_path = std::env::var("SAMP_TRACER_OUT").map(Into::into).unwrap_or_else(|_| dir.join("trace.jsonl"));
    match File::create(&out_path) {
        Ok(f) => OUT.lock().unwrap_or_else(|e| e.into_inner()).file = Some(f),
        Err(e) => tlog(&format!("cannot create {}: {e}", out_path.display())),
    }
    let port = control::port_from_env(std::env::var("SAMP_TRACER_PORT").ok().as_deref());
    tlog(&format!("samp-tracer {} attached; trace {}", env!("CARGO_PKG_VERSION"), out_path.display()));

    if ProcMem.u8(gta::VERSION_PROBE_ADDR) != Some(gta::VERSION_PROBE_US10) {
        tlog("not gta_sa.exe 1.0 US: no hooks installed");
        emit(record("tracer_start", None, None, None, Val::obj().with("error", "unsupported exe")));
        flush();
        return;
    }
    if std::env::var("SAMP_TRACER_MODE").is_ok_and(|m| m == "watch") {
        attach_watch_only(&dir, port);
        return;
    }
    capture::init();
    fileio::init();
    remote::init();
    d3d::init(&dir);
    efx::init(&dir);
    audio::init();
    let boot = stock_boot_attach();
    let (ok, mut failed) = install_all();
    let table = install_table_thunks().unwrap_or_else(|e| {
        failed.push(format!("handler table: {e}"));
        0
    });
    for f in &failed {
        tlog(&format!("hook FAILED: {f}"));
    }
    // After our hooks (their entries are skipped), before the client DLL loads.
    cov::init();
    tlog(&format!("{ok}/{} hooks installed", HOOKS.len()));
    emit(record(
        "tracer_start",
        None,
        None,
        None,
        Val::obj()
            .with("version", env!("CARGO_PKG_VERSION"))
            .with("hooks", ok as u32)
            .with("failed", Val::List(failed.into_iter().map(Val::Str).collect()))
            .with("port", u32::from(port))
            .with("stock_boot", boot)
            .with("script_trace_handlers", table)
            .with("cmdline", std::env::args().skip(1).filter(|a| !a.is_empty()).collect::<Vec<_>>().join(" ").as_str()),
    ));
    flush();
    let _ = std::thread::Builder::new().name("tracer-ctl".into()).spawn(move || control_thread(port));
    let _ = std::thread::Builder::new().name("tracer-flush".into()).spawn(flusher_thread);
}

/// `SAMP_TRACER_MODE=watch`: only the control channel (`peek`, `efx watch|status|disarm`) and the
/// vectored handler of the effect tracer. No game code byte is changed (no inline hook, no thunk
/// table, no D3D/audio wrapper, no coverage int3s), so a client loaded next to it keeps exactly the
/// bytes it wrote itself (F5 runs the tracer this way next to our client). Without the frame hook,
/// a thread closes `efx watch` windows on `CTimer::m_FrameCounter` changes.
fn attach_watch_only(dir: &std::path::Path, port: u16) {
    efx::init(dir);
    tlog("watch-only mode: no hooks installed");
    emit(record(
        "tracer_start",
        None,
        None,
        None,
        Val::obj()
            .with("version", env!("CARGO_PKG_VERSION"))
            .with("mode", "watch")
            .with("hooks", 0u32)
            .with("port", u32::from(port)),
    ));
    flush();
    let _ = std::thread::Builder::new().name("tracer-ctl".into()).spawn(move || control_thread(port));
    let _ = std::thread::Builder::new().name("tracer-flush".into()).spawn(flusher_thread);
    let _ = std::thread::Builder::new().name("tracer-frames".into()).spawn(|| {
        let mut last = ProcMem.u32(gta::FRAME_COUNTER).unwrap_or(0);
        loop {
            std::thread::sleep(Duration::from_millis(1));
            let now = ProcMem.u32(gta::FRAME_COUNTER).unwrap_or(last);
            if now != last {
                last = now;
                let _ = catch_unwind(efx::frame);
            }
        }
    });
}

// ------------------------------------------------- optional file-content capture ------------
//
// `SAMP_TRACER_CAPTURE_DIR=<dir>`: every file the game reads through
// `CFileMgr` (OpenFile → Read/ReadLine/Seek → CloseFile, and LoadFile) is
// reassembled from what the reads returned and written to
// `<dir>/<NNN>_<path with separators replaced>` when it is closed. This shows
// the content the game actually got, whichever layer served it. Off by default.
mod capture {
    use super::*;
    use std::collections::HashMap;

    struct Open {
        path: String,
        data: Vec<u8>,
        pos: usize,
        lines: bool,
    }

    struct State {
        dir: std::path::PathBuf,
        open: HashMap<u32, Open>,
        n: u32,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);
    static ENABLED: AtomicBool = AtomicBool::new(false);

    pub fn init() {
        if let Ok(d) = std::env::var("SAMP_TRACER_CAPTURE_DIR") {
            let dir = std::path::PathBuf::from(d);
            let _ = std::fs::create_dir_all(&dir);
            *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(State { dir, open: HashMap::new(), n: 0 });
            ENABLED.store(true, Ordering::SeqCst);
            tlog("file capture enabled");
        }
    }

    fn tramp(special: Special) -> usize {
        tramp_of(special) as usize
    }

    fn put(o: &mut Open, bytes: &[u8]) {
        let end = o.pos + bytes.len();
        if o.data.len() < end {
            o.data.resize(end, 0);
        }
        o.data[o.pos..end].copy_from_slice(bytes);
        o.pos = end;
    }

    fn with_file(fd: u32, f: impl FnOnce(&mut Open)) {
        if !ENABLED.load(Ordering::Relaxed) || fd == 0 {
            return;
        }
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(o) = g.as_mut().and_then(|s| s.open.get_mut(&fd)) {
            f(o);
        }
    }

    fn save(st: &mut State, path: &str, data: &[u8], lines: bool) {
        st.n += 1;
        let clean: String = path
            .trim()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
            .collect();
        let name = format!("{:03}_{clean}", st.n);
        let _ = std::fs::write(st.dir.join(&name), data);
        emit(record(
            "file_captured",
            None,
            None,
            None,
            Val::obj()
                .with("path", path)
                .with("bytes", data.len() as u32)
                .with("via", if lines { "readline" } else { "read" })
                .with("file", name.as_str()),
        ));
    }

    pub extern "C" fn open_wrap(path: u32, mode: u32) -> u32 {
        // SAFETY: trampoline of CFileMgr::OpenFile (cdecl FILESTREAM(const char*, const char*)).
        let f: extern "C" fn(u32, u32) -> u32 = unsafe { std::mem::transmute(tramp(Special::FileOpen)) };
        let r = f(path, mode);
        if ENABLED.load(Ordering::Relaxed) && r != 0 {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let p = ProcMem.cstr(path, 260).unwrap_or_default();
                let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(st) = g.as_mut() {
                    st.open.insert(r, Open { path: p, data: Vec::new(), pos: 0, lines: false });
                }
            }));
        }
        r
    }

    pub extern "C" fn read_wrap(fd: u32, buf: u32, size: u32) -> u32 {
        // SAFETY: trampoline of CFileMgr::Read (cdecl size_t(FILESTREAM, void*, size_t)).
        let f: extern "C" fn(u32, u32, u32) -> u32 = unsafe { std::mem::transmute(tramp(Special::FileRead)) };
        let r = f(fd, buf, size);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let n = r.min(size).min(64 << 20) as usize;
            let mut b = vec![0u8; n];
            if n > 0 && ProcMem.read(buf, &mut b) {
                with_file(fd, |o| put(o, &b));
            }
        }));
        r
    }

    pub extern "C" fn read_line_wrap(fd: u32, buf: u32, num: u32) -> u32 {
        // SAFETY: trampoline of CFileMgr::ReadLine (cdecl bool(FILESTREAM, char*, int)); EAX passed through.
        let f: extern "C" fn(u32, u32, u32) -> u32 = unsafe { std::mem::transmute(tramp(Special::FileReadLine)) };
        let r = f(fd, buf, num);
        if r & 0xFF != 0 {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let line = ProcMem.cstr(buf, num.min(4096) as usize).unwrap_or_default();
                with_file(fd, |o| {
                    o.lines = true;
                    put(o, line.as_bytes());
                });
            }));
        }
        r
    }

    pub extern "C" fn load_file_wrap(path: u32, buf: u32, size: u32, mode: u32) -> u32 {
        // SAFETY: trampoline of CFileMgr::LoadFile (cdecl int32(const char*, uint8*, int32, const char*)).
        let f: extern "C" fn(u32, u32, u32, u32) -> u32 = unsafe { std::mem::transmute(tramp(Special::FileLoad)) };
        let r = f(path, buf, size, mode);
        if ENABLED.load(Ordering::Relaxed) && (r as i32) > 0 {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let p = ProcMem.cstr(path, 260).unwrap_or_default();
                let mut b = vec![0u8; r.min(size) as usize];
                if ProcMem.read(buf, &mut b) {
                    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(st) = g.as_mut() {
                        save(st, &p, &b, false);
                    }
                }
            }));
        }
        r
    }

    /// Pre-call of CFileMgr::Seek(FILESTREAM, long offset, int origin).
    pub fn on_seek(esp: u32) {
        let m = ProcMem;
        let (fd, off, origin) =
            (m.u32(esp + 4).unwrap_or(0), m.i32(esp + 8).unwrap_or(0), m.i32(esp + 12).unwrap_or(0));
        with_file(fd, |o| match origin {
            0 => o.pos = off.max(0) as usize,
            1 => o.pos = (o.pos as i64 + i64::from(off)).max(0) as usize,
            _ => {}
        });
    }

    /// Pre-call of CFileMgr::CloseFile(FILESTREAM).
    pub fn on_close(esp: u32) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let fd = ProcMem.u32(esp + 4).unwrap_or(0);
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(st) = g.as_mut()
            && let Some(o) = st.open.remove(&fd)
        {
            save(st, &o.path, &o.data, o.lines);
        }
    }
}
