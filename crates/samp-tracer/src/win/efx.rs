//! EFX-1 — effect tracer, Windows side (see `crate::efx` for the idea and the record format,
//! and docs/phase3/efx-notes.md for the design).
//!
//! * One vectored exception handler serves both modes: `int3` hits (armed targets, window
//!   call hooks, the return stub), single-step traps and guard-page faults.
//! * **Return stub**: to find out when a function returns (the armed target, a game call that
//!   is stepped over, a foreign call), the return address on the stack is replaced with the
//!   address of a one-byte `int3` stub. When it is reached, the real return address (kept in a
//!   per-thread LIFO) is put back into EIP. No code outside the stub is patched for this.
//! * Output: `<GTA_DIR>/efx.jsonl` (`SAMP_TRACER_EFX_OUT`), written when a capture or a
//!   window ends.

use super::*;
use crate::efx::{self as fx, Access, ArmOpts, Cpu, EfxCmd, Flow, SpecLine, Target, WatchOpts, World};
use iced_x86::InstructionInfoFactory;
use std::cell::RefCell;
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, CONTEXT, EXCEPTION_POINTERS};
use windows_sys::Win32::System::Memory::{
    MEM_IMAGE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY,
};

const BREAKPOINT: i32 = 0x8000_0003u32 as i32;
const SINGLE_STEP: i32 = 0x8000_0004u32 as i32;
const GUARD: i32 = 0x8000_0001u32 as i32;
const ACCESS_VIOLATION: i32 = 0xC000_0005u32 as i32;
const CONTINUE_EXECUTION: i32 = -1;
const CONTINUE_SEARCH: i32 = 0;
const TF: u32 = 0x100;
/// gta_sa.exe 1.0 US code section (window pages are never taken from it).
const GTA_CODE: (u32, u32) = (0x401000, 0x857000);

// ------------------------------------------------------------------ output ----------------

struct EOut {
    buf: String,
    file: Option<File>,
    seq: u64,
}

static EOUT: Mutex<EOut> = Mutex::new(EOut { buf: String::new(), file: None, seq: 0 });

fn eout(lines: &mut Vec<Val>) {
    let mut g = EOUT.lock().unwrap_or_else(|e| e.into_inner());
    for mut rec in lines.drain(..) {
        g.seq += 1;
        let seq = g.seq;
        if let Val::Obj(f) = &mut rec {
            f.insert(0, ("n".into(), Val::Int(seq as i64)));
        }
        let line = rec.to_json();
        g.buf.push_str(&line);
        g.buf.push('\n');
    }
    let data = std::mem::take(&mut g.buf);
    if let Some(f) = g.file.as_mut() {
        let _ = f.write_all(data.as_bytes());
        let _ = f.flush();
    }
}

fn rec(kind: &str, cap: u32) -> Val {
    Val::obj().with("fn", kind).with("cap", cap).with("frame", super::frame()).with("t", ms())
}

/// `mark` records are copied into efx.jsonl so runs can be cut by scenario step.
pub fn mark(text: &str) {
    eout(&mut vec![rec("mark", 0).with("text", text)]);
}

static STATUS: Mutex<String> = Mutex::new(String::new());
static CAPS: AtomicU32 = AtomicU32::new(0);

fn set_status(s: String) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

// ------------------------------------------------------------------ memory helpers --------

/// Every address we ever put an `int3` on: a late hit (another thread, just before the byte
/// was restored) is resumed at the same address instead of crashing the game.
/// (address, original byte).
static INT3_SITES: Mutex<Vec<(u32, u8)>> = Mutex::new(Vec::new());

fn write_code_byte(addr: u32, b: u8) -> bool {
    if b == 0xCC {
        let mut g = INT3_SITES.lock().unwrap_or_else(|e| e.into_inner());
        if !g.iter().any(|x| x.0 == addr)
            && let Some(cur) = ProcMem.u8(addr).filter(|&c| c != 0xCC)
        {
            g.push((addr, cur));
        }
    }
    let mut old = 0u32;
    // SAFETY: one byte of committed code we decided to patch (entry of a function); the
    // protection is restored afterwards.
    unsafe {
        let p = addr as usize as *mut c_void;
        if VirtualProtect(p, 1, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return false;
        }
        std::ptr::write_volatile(p as *mut u8, b);
        VirtualProtect(p, 1, old, &mut old);
        FlushInstructionCache(GetCurrentProcess(), p, 1);
    }
    true
}

fn query(addr: u32) -> Option<MEMORY_BASIC_INFORMATION> {
    // SAFETY: VirtualQuery only inspects the address space.
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let n = unsafe {
        VirtualQuery(addr as usize as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
    };
    (n != 0 && mbi.State == MEM_COMMIT).then_some(mbi)
}

fn writable(addr: u32, len: u32) -> bool {
    let mut p = addr as u64;
    let end = addr as u64 + len as u64;
    while p < end {
        let Some(m) = query(p as u32) else { return false };
        let w = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
        if m.Protect & w == 0 || m.Protect & PAGE_GUARD != 0 {
            return false;
        }
        p = m.BaseAddress as u64 + m.RegionSize as u64;
    }
    true
}

fn read_vec(addr: u32, len: u32) -> Option<Vec<u8>> {
    let mut b = vec![0u8; len as usize];
    ProcMem.read(addr, &mut b).then_some(b)
}

/// Read for the exception handler while guard pages are armed: every page is checked with an
/// uncached VirtualQuery (the tracer's readability cache may predate a guard), so the handler
/// never faults on a watched page itself.
fn veh_read(addr: u32, len: u32) -> Option<Vec<u8>> {
    if addr < 0x10000 || len == 0 {
        return None;
    }
    let mut p = addr & !0xFFF;
    while p < addr + len {
        let m = query(p)?;
        let r = PAGE_READWRITE
            | PAGE_WRITECOPY
            | PAGE_EXECUTE_READ
            | PAGE_EXECUTE_READWRITE
            | PAGE_EXECUTE_WRITECOPY
            | 0x02;
        if m.Protect & PAGE_GUARD != 0 || m.Protect & r == 0 {
            return None;
        }
        p += 0x1000;
    }
    let mut b = vec![0u8; len as usize];
    // SAFETY: every page of the range was just checked to be committed, readable, unguarded.
    unsafe { std::ptr::copy_nonoverlapping(addr as usize as *const u8, b.as_mut_ptr(), b.len()) };
    Some(b)
}

/// The window state from inside the exception handler: bounded spinning instead of blocking,
/// so a fault raised while this thread already holds the lock can never deadlock the game.
fn win_lock() -> Option<std::sync::MutexGuard<'static, Option<Win>>> {
    for i in 0..20_000u32 {
        match WIN.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                if i > 100 {
                    std::thread::yield_now();
                }
            }
        }
    }
    None
}

fn win_pages() -> Vec<(u32, u32)> {
    WIN_PAGES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn teb() -> u32 {
    let t: u32;
    // SAFETY: fs:[0x18] is the TEB self pointer of the current thread.
    unsafe { std::arch::asm!("mov {0}, dword ptr fs:[0x18]", out(reg) t, options(nostack, readonly)) };
    t
}

fn cpu_of(c: &CONTEXT) -> Cpu {
    Cpu {
        eax: c.Eax,
        ecx: c.Ecx,
        edx: c.Edx,
        ebx: c.Ebx,
        esp: c.Esp,
        ebp: c.Ebp,
        esi: c.Esi,
        edi: c.Edi,
        eip: c.Eip,
        eflags: c.EFlags,
        fs_base: teb(),
    }
}

/// ST(0) if the x87 stack is not empty (float return values).
fn st0(c: &CONTEXT) -> Option<f64> {
    let f = &c.FloatSave;
    let top = (f.StatusWord >> 11) & 7;
    if (f.TagWord >> (2 * top)) & 3 == 3 {
        return None;
    }
    let mut b = [0u8; 10];
    b.copy_from_slice(&f.RegisterArea[..10]);
    Some(fx::f80_to_f64(&b))
}

fn gta_range() -> (u32, u32) {
    let m = ProcMem;
    let e = m.u32(0x400000 + 0x3C).unwrap_or(0);
    let size = m.u32(0x400000 + e + 0x50).unwrap_or(0x1177000);
    (0x400000, 0x400000 + size)
}

/// Loaded client module (samp.dll or samp_client.dll).
fn client_module() -> Option<Module> {
    refresh_client_modules();
    MODULES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|m| m.client).cloned()
}

/// Window attribution: the innermost client-module return address on the stack above `esp`
/// (formatted), and the outermost game return address below it — a return address inside the
/// game function the client called (e.g. `CGame::Process` when the client's frame hook runs
/// the whole game frame), so the diff can tell game processing from the client's own work.
fn client_via(esp: u32, client: (u32, u32)) -> (Option<String>, Option<u32>) {
    let (_, top) = stack_bounds();
    let end = top.min(esp.saturating_add(0x8000));
    let mut p = esp;
    let mut outer = None;
    while p + 4 <= end {
        // SAFETY: p lies inside the current thread's committed stack (between esp and StackBase).
        let d = unsafe { std::ptr::read_unaligned(p as usize as *const u32) };
        if d >= client.0 + 7 && d < client.1 && after_call(d) {
            return (Some(fmt_loc(d)), outer);
        }
        if (GTA_CODE.0 + 7..GTA_CODE.1).contains(&d) && after_call(d) {
            outer = Some(d);
        }
        p += 4;
    }
    (None, outer)
}

// ------------------------------------------------------------------ code regions ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Client,
    Gta,
    /// Private executable memory (trampolines): stepped like the code that jumped there.
    Private,
    /// Other modules (system DLLs, d3d9, the tracer itself): stepped over.
    Foreign,
}

struct Regions {
    client: (u32, u32),
    gta: (u32, u32),
    cache: Vec<(u32, u32, Kind)>,
}

impl Regions {
    fn new(client: (u32, u32)) -> Self {
        Regions { client, gta: (0x400000, GTA_CODE.1.max(gta_range().1)), cache: Vec::new() }
    }

    fn kind(&mut self, a: u32) -> Kind {
        if a >= self.client.0 && a < self.client.1 {
            return Kind::Client;
        }
        if a >= self.gta.0 && a < self.gta.1 {
            return Kind::Gta;
        }
        if let Some(&(_, _, k)) = self.cache.iter().find(|(b, e, _)| a >= *b && a < *e) {
            return k;
        }
        let Some(m) = query(a) else { return Kind::Foreign };
        let (b, e, k) = if m.Type == MEM_IMAGE {
            match module_of(a) {
                Some(md) => (md.base, md.end, Kind::Foreign),
                None => (m.BaseAddress as u32, m.BaseAddress as u32 + m.RegionSize as u32, Kind::Foreign),
            }
        } else {
            let exec = PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
            let k = if m.Protect & exec != 0 { Kind::Private } else { Kind::Foreign };
            (m.BaseAddress as u32, m.BaseAddress as u32 + m.RegionSize as u32, k)
        };
        self.cache.push((b, e, k));
        k
    }
}

/// Code bytes at `eip` (up to 15; fewer at the end of a readable region).
fn code_at(eip: u32) -> Option<([u8; 16], usize)> {
    let mut b = [0u8; 16];
    for n in [16usize, 8, 4, 2, 1] {
        if ProcMem.read(eip, &mut b[..n]) {
            return Some((b, n));
        }
    }
    None
}

/// Tracer inline-hook thunk entry → its trampoline (skips the tracer's own logging, so the
/// hooked game function is stepped like any other).
fn hook_thunk_tramp(eip: u32) -> Option<u32> {
    rt().iter().map(|h| h.tramp.load(Ordering::Relaxed)).find(|&t| t != 0 && t - 21 == eip)
}

// ------------------------------------------------------------------ return stub -----------

static STUB: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StubKind {
    /// The armed target's own return: ends the capture.
    Target,
    /// A foreign call stepped over; stepping resumes after it.
    Foreign,
    /// A game call stepped over (or truncated) with this call id.
    Over(u32),
    /// A window-mode call hook's return (call id).
    WCall(u32),
}

#[derive(Debug, Clone, Copy)]
struct Stub {
    slot: u32,
    ret: u32,
    kind: StubKind,
}

thread_local! {
    static STUBS: RefCell<Vec<Stub>> = const { RefCell::new(Vec::new()) };
    static WPEND: RefCell<Option<WPend>> = const { RefCell::new(None) };
}

/// Replaces the return address at `[slot]` with the stub.
fn push_stub(slot: u32, kind: StubKind) -> bool {
    let Some(ret) = ProcMem.u32(slot) else { return false };
    let stub = STUB.load(Ordering::Relaxed);
    if stub == 0 || ret == stub {
        return false;
    }
    // SAFETY: `slot` is the return-address cell of a frame on the current thread's stack.
    unsafe { std::ptr::write_volatile(slot as usize as *mut u32, stub) };
    STUBS.with(|s| s.borrow_mut().push(Stub { slot, ret, kind }));
    true
}

/// The stub was reached with `esp` (after the `ret`): the entry with the highest slot below
/// `esp` is the frame that returned (deeper stale entries are dropped).
fn pop_stub(esp: u32) -> Option<Stub> {
    STUBS.with(|s| {
        let mut v = s.borrow_mut();
        let i = v.iter().enumerate().filter(|(_, x)| x.slot < esp).max_by_key(|(_, x)| x.slot).map(|(i, _)| i)?;
        let st = v[i];
        v.retain(|x| x.slot > st.slot);
        Some(st)
    })
}

// ------------------------------------------------------------------ step capture ----------

struct Arm {
    va: u32,
    orig: u8,
    target: Target,
    opts: ArmOpts,
    left: u32,
}

static ARM: Mutex<Option<Arm>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FKind {
    Client,
    Gta,
}

struct Frame {
    /// Stack cell holding the return address (ESP at entry).
    slot: u32,
    kind: FKind,
    id: u32,
    steps0: u64,
}

struct Pend {
    addr: u32,
    size: u32,
    old: Option<Vec<u8>>,
    sym: String,
    at: u32,
    by: Kind,
}

struct Cap {
    n: u32,
    opts: ArmOpts,
    world: World,
    stack: (u32, u32),
    teb: u32,
    regions: Regions,
    info: InstructionInfoFactory,
    steps: u64,
    frames: Vec<Frame>,
    pend: Vec<Pend>,
    /// (target, call-site eip) of the call just executed.
    call: Option<(u32, u32)>,
    prev: Flow,
    /// Last game-memory read by client code: (sym, size, value hex, at, age).
    last_read: Option<(String, u32, String, u32, u32)>,
    branch: Option<(String, u32, String, u32, u32)>,
    next_id: u32,
    out: Vec<Val>,
    draining: bool,
    done: bool,
    t0: Instant,
    stats: CapStats,
}

#[derive(Default)]
struct CapStats {
    writes: u32,
    skipped_stack: u32,
    skipped_client: u32,
    skipped_heap: u32,
    gta_calls: u32,
    ext_calls: u32,
    icalls: u32,
    branches: u32,
    emulated: u32,
    truncated: u32,
    stepped_over: u32,
}

static CAP: Mutex<Option<Cap>> = Mutex::new(None);
static CAP_TID: AtomicU32 = AtomicU32::new(0);

enum Next {
    /// Keep single-stepping.
    Step,
    /// Run freely (a stub will bring us back, or the capture is over).
    Run,
}

fn hexs(b: &Option<Vec<u8>>) -> Val {
    match b {
        Some(v) => Val::Str(fx::hex_bytes(v)),
        None => Val::Null,
    }
}

impl Cap {
    fn depth(&self) -> u32 {
        self.frames.iter().filter(|f| f.kind == FKind::Gta).count() as u32
    }

    /// Stack arguments and `this` of a call at its entry.
    fn call_rec(&mut self, kind: &str, to: u32, from: u32, c: &CONTEXT) -> (u32, Val) {
        self.next_id += 1;
        let id = self.next_id;
        let args: Vec<Val> =
            (1..=6).map(|i| Val::Str(format!("{:#x}", ProcMem.u32(c.Esp + 4 * i).unwrap_or(0)))).collect();
        let args_sym: Vec<Val> = (1..=6)
            .map(|i| ProcMem.u32(c.Esp + 4 * i).unwrap_or(0))
            .map(|v| self.world.sym(v).map(Val::Str).unwrap_or(Val::Null))
            .collect();
        let mut r = rec(kind, self.n)
            .with("id", id)
            .with("d", self.depth() + 1)
            .with("to", Val::Hex(to))
            .with("loc", fmt_loc(to))
            .with("from", fmt_loc(from))
            .with("ecx", Val::Hex(c.Ecx))
            .with("args", Val::List(args))
            .with("args_sym", Val::List(args_sym));
        if let Some(s) = self.world.sym(c.Ecx) {
            r.put("this", s);
        }
        (id, r)
    }

    fn ret_rec(&mut self, id: u32, c: &CONTEXT, note: Option<&str>) {
        let mut r = rec("efx_ret", self.n).with("id", id).with("eax", Val::Hex(c.Eax));
        if let Some(s) = self.world.sym(c.Eax) {
            r.put("eax_sym", s);
        }
        if let Some(f) = st0(c) {
            r.put("st0", Val::Str(format!("{f:.4}")));
        }
        if let Some(n) = note {
            r.put("note", n);
        }
        self.out.push(r);
    }

    /// Classifies a write destination: the symbol to log it under, or None (skipped).
    fn write_sym(&mut self, a: u32, by: Kind) -> Option<String> {
        // The thread's stack and TEB (SEH frame links written through fs:).
        if (a >= self.stack.0 && a < self.stack.1) || a.wrapping_sub(self.teb) < 0x2000 {
            self.stats.skipped_stack += 1;
            return None;
        }
        if let Some(s) = self.world.sym(a) {
            return Some(s);
        }
        if a >= self.regions.client.0 && a < self.regions.client.1 {
            self.stats.skipped_client += 1;
            return None;
        }
        if by == Kind::Gta || self.opts.heap {
            return Some(format!("heap:{a:#x}"));
        }
        self.stats.skipped_heap += 1;
        None
    }

    /// Finishes the instruction executed since the last trap.
    fn complete(&mut self, c: &CONTEXT) {
        for p in std::mem::take(&mut self.pend) {
            let new = read_vec(p.addr, p.size);
            self.stats.writes += 1;
            let mut r = rec("efx_write", self.n)
                .with("d", self.depth())
                .with("at", fmt_loc(p.at))
                .with("by", if p.by == Kind::Gta { "game" } else { "client" })
                .with("sym", p.sym)
                .with("addr", Val::Hex(p.addr))
                .with("size", p.size)
                .with("old", hexs(&p.old))
                .with("new", hexs(&new));
            if p.old.is_some() && p.old == new {
                r.put("same", true);
            }
            self.out.push(r);
        }
        if let Some((sym, size, val, at, target)) = self.branch.take() {
            self.stats.branches += 1;
            self.out.push(
                rec("efx_branch", self.n)
                    .with("at", fmt_loc(at))
                    .with("sym", sym)
                    .with("size", size)
                    .with("val", val)
                    .with("taken", c.Eip == target),
            );
        }
        if self.prev == Flow::Ret {
            while let Some(f) = self.frames.last() {
                if f.slot >= c.Esp {
                    break;
                }
                let f = self.frames.pop().unwrap_or(Frame { slot: 0, kind: FKind::Client, id: 0, steps0: 0 });
                if f.kind == FKind::Gta {
                    self.ret_rec(f.id, c, None);
                }
            }
        }
    }

    /// Decides what to do at the current EIP (entering calls, stepping over, budgets) and
    /// prepares the pending records of the next instruction.
    fn dispatch(&mut self, c: &mut CONTEXT) -> Next {
        loop {
            if c.Eip == STUB.load(Ordering::Relaxed) {
                match self.stub_return(c) {
                    Some(n) => return n,
                    None => continue,
                }
            }
            let kind = self.regions.kind(c.Eip);
            if kind == Kind::Private
                && let Some(t) = hook_thunk_tramp(c.Eip)
            {
                c.Eip = t;
                continue;
            }
            let call = self.call.take();
            match kind {
                Kind::Foreign => {
                    if !matches!(self.prev, Flow::Call(_) | Flow::Jump(_)) {
                        self.abort(c, "entered foreign code without a call");
                        return Next::Run;
                    }
                    self.stats.ext_calls += 1;
                    if self.stats.ext_calls <= 400 {
                        let from = call.map(|x| x.1).unwrap_or(0);
                        self.out.push(
                            rec("efx_ext", self.n)
                                .with("d", self.depth())
                                .with("loc", fmt_loc(c.Eip))
                                .with("from", fmt_loc(from)),
                        );
                    }
                    if !push_stub(c.Esp, StubKind::Foreign) {
                        self.abort(c, "cannot step over a foreign call");
                    }
                    return Next::Run;
                }
                Kind::Gta => {
                    if let Some((to, from)) = call {
                        self.stats.gta_calls += 1;
                        let (id, r) = self.call_rec("efx_call", to, from, c);
                        self.out.push(r);
                        let over = self.opts.shallow
                            || fx::DEFAULT_STEP_OVER.contains(&to)
                            || self.opts.over.contains(&to)
                            || self.depth() >= self.opts.depth;
                        if over && push_stub(c.Esp, StubKind::Over(id)) {
                            self.stats.stepped_over += 1;
                            return Next::Run;
                        }
                        self.frames.push(Frame { slot: c.Esp, kind: FKind::Gta, id, steps0: self.steps });
                    }
                }
                Kind::Client | Kind::Private => {
                    if let Some((to, from)) = call {
                        self.next_id += 1;
                        let id = self.next_id;
                        self.frames.push(Frame { slot: c.Esp, kind: FKind::Client, id, steps0: self.steps });
                        if kind == Kind::Client {
                            self.stats.icalls += 1;
                            if self.stats.icalls <= self.opts.icalls {
                                self.out.push(
                                    rec("efx_icall", self.n)
                                        .with("d", self.depth())
                                        .with("to", fmt_loc(to))
                                        .with("from", fmt_loc(from)),
                                );
                            }
                        }
                    }
                }
            }
            if self.steps >= self.opts.budget {
                self.abort(c, "step budget");
                return Next::Run;
            }
            // Per top-level game call budget: run the rest of it freely.
            if let Some(i) = self.frames.iter().position(|f| f.kind == FKind::Gta)
                && self.steps - self.frames[i].steps0 > self.opts.sub
            {
                let (slot, id) = (self.frames[i].slot, self.frames[i].id);
                if push_stub(slot, StubKind::Over(id)) {
                    self.frames.truncate(i);
                    self.stats.truncated += 1;
                    self.out.push(rec("efx_truncated", self.n).with("id", id).with("steps", self.steps as u32));
                    return Next::Run;
                }
            }
            let Some((code, n)) = code_at(c.Eip) else {
                self.abort(c, "code unreadable");
                return Next::Run;
            };
            let cpu = cpu_of(c);
            let Some(st) = fx::analyse(&code[..n], &cpu, &ProcMem, &mut self.info) else {
                self.abort(c, "undecodable instruction");
                return Next::Run;
            };
            if let Some(rep) = st.rep
                && self.emulate_rep(c, rep, kind)
            {
                c.Eip = st.next_ip;
                self.prev = Flow::Next;
                continue;
            }
            self.prepare(c, &st, kind);
            if st.writes_tf {
                // popf: keep the trap flag in the value it loads.
                if let Some(v) = ProcMem.u32(c.Esp) {
                    // SAFETY: [esp] is the flags image on the stepped thread's own stack.
                    unsafe { std::ptr::write_volatile(c.Esp as usize as *mut u32, v | TF) };
                }
            }
            if st.flow == Flow::Other {
                self.abort(c, "interrupt/system instruction");
                return Next::Run;
            }
            return Next::Step;
        }
    }

    fn prepare(&mut self, c: &CONTEXT, st: &fx::Step, kind: Kind) {
        let by = if kind == Kind::Gta { Kind::Gta } else { Kind::Client };
        for a in &st.acc {
            let Access { addr, size, write, read } = *a;
            if write && let Some(sym) = self.write_sym(addr, by) {
                let size = size.min(64);
                self.pend.push(Pend { addr, size, old: read_vec(addr, size), sym, at: c.Eip, by });
            }
            if read && !write && by == Kind::Client && size <= 8 && self.stats.branches < self.opts.branches {
                let in_stack = addr >= self.stack.0 && addr < self.stack.1;
                if !in_stack && let Some(sym) = self.world.sym(addr) {
                    let v = read_vec(addr, size).map(|b| fx::hex_bytes(&b)).unwrap_or_default();
                    self.last_read = Some((sym, size, v, c.Eip, 0));
                }
            }
        }
        if let Some(lr) = self.last_read.as_mut() {
            lr.4 += 1;
            if lr.4 > 4 {
                self.last_read = None;
            }
        }
        if st.cond_use
            && by == Kind::Client
            && let Some((sym, size, v, _, _)) = self.last_read.take()
        {
            let target = match st.flow {
                Flow::Cond(t) => t,
                _ => st.next_ip.wrapping_add(0xFFFF_0000),
            };
            self.branch = Some((sym, size, v, c.Eip, target));
        }
        self.call = match st.flow {
            Flow::Call(Some(t)) => Some((t, c.Eip)),
            _ => None,
        };
        self.prev = st.flow;
    }

    /// `rep movs/stos` with DF = 0: done here in one go and logged as one write.
    fn emulate_rep(&mut self, c: &mut CONTEXT, r: fx::RepString, kind: Kind) -> bool {
        let total = u64::from(r.count) * u64::from(r.elem);
        if total > 16 << 20 || (total > 0 && !writable(r.dst, total as u32)) {
            return false;
        }
        let total = total as u32;
        if !r.stos && total > 0 && !ProcMem.read(r.src, &mut vec![0u8; total as usize]) {
            return false;
        }
        let by = if kind == Kind::Gta { Kind::Gta } else { Kind::Client };
        let sym = if total > 0 { self.write_sym(r.dst, by) } else { None };
        let old = sym.as_ref().and_then(|_| read_vec(r.dst, total.min(64)));
        let fill = c.Eax.to_le_bytes();
        // SAFETY: both ranges were checked to be committed (destination writable); element by
        // element, front to back, like the CPU does with DF = 0.
        unsafe {
            for i in 0..r.count {
                let d = (r.dst + i * r.elem) as usize as *mut u8;
                for (k, &f) in fill.iter().enumerate().take(r.elem as usize) {
                    let v = if r.stos {
                        f
                    } else {
                        std::ptr::read_volatile(((r.src + i * r.elem) as usize + k) as *const u8)
                    };
                    std::ptr::write_volatile(d.add(k), v);
                }
            }
        }
        c.Edi = r.dst + total;
        if !r.stos {
            c.Esi = r.src + total;
        }
        c.Ecx = 0;
        self.stats.emulated += 1;
        if let Some(sym) = sym {
            self.stats.writes += 1;
            let new = read_vec(r.dst, total.min(64));
            self.out.push(
                rec("efx_write", self.n)
                    .with("d", self.depth())
                    .with("at", fmt_loc(c.Eip))
                    .with("by", if by == Kind::Gta { "game" } else { "client" })
                    .with("sym", sym)
                    .with("addr", Val::Hex(r.dst))
                    .with("size", total)
                    .with("old", hexs(&old))
                    .with("new", hexs(&new))
                    .with("rep", if r.stos { "stos" } else { "movs" }),
            );
        }
        true
    }

    /// The stub was reached (by a trap or a breakpoint). Returns Some(next) if the capture
    /// decided, None to continue dispatching at the restored EIP.
    fn stub_return(&mut self, c: &mut CONTEXT) -> Option<Next> {
        let Some(s) = pop_stub(c.Esp) else {
            self.abort(c, "stub without entry");
            return Some(Next::Run);
        };
        c.Eip = s.ret;
        self.call = None;
        self.prev = Flow::Next;
        match s.kind {
            StubKind::Target => {
                self.finish(c, "return");
                Some(Next::Run)
            }
            StubKind::Over(id) => {
                while self.frames.last().is_some_and(|f| f.slot < c.Esp) {
                    self.frames.pop();
                }
                self.ret_rec(id, c, None);
                if self.draining { Some(Next::Run) } else { None }
            }
            StubKind::Foreign | StubKind::WCall(_) => {
                if self.draining {
                    Some(Next::Run)
                } else {
                    None
                }
            }
        }
    }

    fn abort(&mut self, c: &CONTEXT, why: &str) {
        if !self.draining {
            self.draining = true;
            self.out.push(
                rec("efx_stop", self.n).with("why", why).with("eip", fmt_loc(c.Eip)).with("steps", self.steps as u32),
            );
            tlog(&format!("efx: capture {} stops stepping: {why} at {}", self.n, fmt_loc(c.Eip)));
        }
    }

    fn finish(&mut self, c: &CONTEXT, reason: &str) {
        let s = &self.stats;
        let mut r = rec("efx_end", self.n)
            .with("reason", if self.draining { "budget/stop" } else { reason })
            .with("steps", self.steps as u32)
            .with("ms", self.t0.elapsed().as_millis() as u32)
            .with("eax", Val::Hex(c.Eax))
            .with("writes", s.writes)
            .with("gta_calls", s.gta_calls)
            .with("stepped_over", s.stepped_over)
            .with("truncated", s.truncated)
            .with("ext_calls", s.ext_calls)
            .with("icalls", s.icalls)
            .with("branches", s.branches)
            .with("rep_emulated", s.emulated)
            .with("skipped_stack", s.skipped_stack)
            .with("skipped_client_image", s.skipped_client)
            .with("skipped_client_heap", s.skipped_heap);
        if let Some(f) = st0(c) {
            r.put("st0", Val::Str(format!("{f:.4}")));
        }
        self.out.push(r);
        self.draining = true;
        self.done = true;
    }
}

/// Resolves a target to an absolute address.
fn resolve_target(t: &Target) -> Result<u32, String> {
    let rebase = |va: u32| -> Result<u32, String> {
        let m = client_module().ok_or("no client module loaded")?;
        let a = m.base + (va - fx::SAMP_IMAGE_BASE);
        if a >= m.end {
            return Err(format!("{va:#x} is outside {}", m.name));
        }
        Ok(a)
    };
    match t {
        Target::Rpc(id) => {
            let m = client_module().ok_or("no client module loaded")?;
            if m.name != "samp.dll" {
                return Err("rpc:N targets need samp.dll (our client: use `efx watch`)".into());
            }
            let e = ProcMem.u32(m.base + 0x3C).unwrap_or(0);
            let stamp = ProcMem.u32(m.base + e + 8).unwrap_or(0);
            let build = fx::build_of_stamp(stamp).ok_or(format!("unknown samp.dll build (PE stamp {stamp:#x})"))?;
            let va = fx::rpc_handler(*id, build).ok_or(format!("rpc {id} has no handler in rpc-ids.md"))?;
            rebase(va)
        }
        Target::ClientVa(va) => rebase(*va),
        Target::ModRva(name, rva) => {
            let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: NUL-terminated wide string.
            let h = unsafe { GetModuleHandleW(w.as_ptr()) };
            if h.is_null() {
                return Err(format!("module {name} not loaded"));
            }
            Ok(h as usize as u32 + rva)
        }
        Target::Gta(a) => Ok(*a),
        Target::Event(e) => {
            let site = e.call_site();
            if ProcMem.u8(site) != Some(0xE8) {
                return Err(format!("{site:#x} is not a call"));
            }
            let to = x86::rel32_target(site, 5, ProcMem.i32(site + 1).unwrap_or(0));
            let m = client_module().ok_or("no client module loaded")?;
            if to < m.base || to >= m.end {
                return Err(format!("call at {site:#x} goes to {to:#x}, not into {}", m.name));
            }
            Ok(to)
        }
    }
}

fn arm(t: Target, opts: ArmOpts) -> String {
    if WIN_ON.load(Ordering::SeqCst) || CAP_TID.load(Ordering::SeqCst) != 0 {
        return "error busy (window or capture running)".into();
    }
    disarm();
    let va = match resolve_target(&t) {
        Ok(v) => v,
        Err(e) => return format!("error {e}"),
    };
    let Some(orig) = ProcMem.u8(va) else { return format!("error {va:#x} unreadable") };
    if orig == 0xCC {
        return format!("error {va:#x} already holds int3");
    }
    if !write_code_byte(va, 0xCC) {
        return format!("error cannot patch {va:#x}");
    }
    let s = format!("armed {t} at {}", fmt_loc(va));
    tlog(&format!("efx: {s} ({opts:?})"));
    let left = opts.count;
    *ARM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arm { va, orig, target: t, opts, left });
    set_status(s.clone());
    format!("ok {s}")
}

fn disarm() {
    if let Some(a) = ARM.lock().unwrap_or_else(|e| e.into_inner()).take() {
        write_code_byte(a.va, a.orig);
    }
}

/// `int3` at the armed target: start a step capture on this thread.
fn start_capture(c: &mut CONTEXT, va: u32) -> i32 {
    let (target, opts, rearm) = {
        let mut g = ARM.lock().unwrap_or_else(|e| e.into_inner());
        let Some(a) = g.as_mut() else { return CONTINUE_SEARCH };
        write_code_byte(a.va, a.orig);
        a.left = a.left.saturating_sub(1);
        let r = (a.target.to_string(), a.opts.clone(), a.left > 0);
        if !r.2 {
            *g = None;
        }
        r
    };
    c.Eip = va;
    let me = tid();
    if CAP_TID.compare_exchange(0, me, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return CONTINUE_EXECUTION;
    }
    let client = client_module().map(|m| (m.base, m.end)).unwrap_or((0, 0));
    let n = CAPS.fetch_add(1, Ordering::SeqCst) + 1;
    let (lo, hi) = stack_bounds();
    let mut cap = Cap {
        n,
        opts,
        world: World::read(&ProcMem, gta_range()),
        stack: (lo, hi),
        teb: teb(),
        regions: Regions::new(client),
        info: InstructionInfoFactory::new(),
        steps: 0,
        frames: Vec::new(),
        pend: Vec::new(),
        call: None,
        prev: Flow::Next,
        last_read: None,
        branch: None,
        next_id: 0,
        out: Vec::new(),
        draining: false,
        done: false,
        t0: Instant::now(),
        stats: CapStats::default(),
    };
    let (_, mut begin) = cap.call_rec("efx_begin", va, ProcMem.u32(c.Esp).unwrap_or(0), c);
    begin.put("target", target.as_str());
    begin.put("mode", "step");
    begin.put("tid", me);
    let objs: Vec<Val> = cap.world.named.iter().map(|(nm, b, _)| Val::Str(format!("{nm}={b:#x}"))).collect();
    begin.put("objects", Val::List(objs));
    begin.put("world", cap.world.note.as_str());
    cap.out.push(begin);
    set_status(format!("capturing {} ({target})", n));
    if !push_stub(c.Esp, StubKind::Target) {
        cap.abort(c, "cannot hook the target's return");
        cap.finish(c, "no-return-hook");
        eout(&mut cap.out);
        CAP_TID.store(0, Ordering::SeqCst);
        return CONTINUE_EXECUTION;
    }
    cap.prev = Flow::Next;
    let next = cap.dispatch(c);
    after_dispatch(cap, next, c, rearm.then_some(va))
}

/// Applies the dispatch result: trap flag on/off, and ends the capture once the target's
/// return stub was handled.
fn after_dispatch(cap: Cap, next: Next, c: &mut CONTEXT, rearm: Option<u32>) -> i32 {
    let mut cap = cap;
    match next {
        Next::Step if !cap.draining => c.EFlags |= TF,
        _ => c.EFlags &= !TF,
    }
    if cap.done {
        eout(&mut cap.out);
        let n = cap.n;
        set_status(format!("idle done={n} steps={}", cap.steps));
        tlog(&format!("efx: capture {n} done ({} steps, {} ms)", cap.steps, cap.t0.elapsed().as_millis()));
        *CAP.lock().unwrap_or_else(|e| e.into_inner()) = None;
        CAP_TID.store(0, Ordering::SeqCst);
        if let Some(va) = rearm.or_else(|| ARM.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|a| a.va)) {
            write_code_byte(va, 0xCC);
        }
    } else {
        if cap.out.len() > 4096 {
            eout(&mut cap.out);
        }
        *CAP.lock().unwrap_or_else(|e| e.into_inner()) = Some(cap);
    }
    CONTINUE_EXECUTION
}

fn on_capture_trap(c: &mut CONTEXT, via_stub: bool) -> i32 {
    let Some(mut cap) = CAP.lock().unwrap_or_else(|e| e.into_inner()).take() else { return CONTINUE_SEARCH };
    if !via_stub {
        cap.steps += 1;
        cap.complete(c);
    }
    let next = if cap.draining && c.Eip != STUB.load(Ordering::Relaxed) { Next::Run } else { cap.dispatch(c) };
    after_dispatch(cap, next, c, None)
}

// ------------------------------------------------------------------ window mode -----------

/// Memory that reads nothing: the window fault handler must not touch other watched pages
/// (indirect branch targets are not needed there).
struct NoMem;

impl Mem for NoMem {
    fn read(&self, _: u32, _: &mut [u8]) -> bool {
        false
    }
}

struct WFn {
    addr: u32,
    name: String,
    orig: u8,
    hits: u32,
    logged: u32,
    armed: bool,
}

struct WMem {
    sym: String,
    addr: u32,
    size: u32,
    before: Option<Vec<u8>>,
}

struct Win {
    n: u32,
    pages: Vec<(u32, u32)>,
    mems: Vec<WMem>,
    fns: Vec<WFn>,
    world: World,
    opts: WatchOpts,
    frames: u32,
    t0: Instant,
    faults: u32,
    writes: u32,
    next_id: u32,
    out: Vec<Val>,
    client: (u32, u32),
}

struct WPend {
    pages: Vec<u32>,
    writes: Vec<Pend>,
    rearm: Option<u32>,
    by: &'static str,
    via: Option<String>,
    outer: Option<u32>,
}

static WIN: Mutex<Option<Win>> = Mutex::new(None);
static WIN_ON: AtomicBool = AtomicBool::new(false);
/// Pages of the current window (lock-free membership test for the fault handler).
static WIN_PAGES: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
static WIN_CLIENT: AtomicU64 = AtomicU64::new(0);
/// Every page any window ever guarded, with its normal protection: a guard left behind by a
/// race (a thread re-arming a page while the window closes) is recognised and dropped instead
/// of crashing the thread that touches it, and unguarded again one frame after the close.
static EVER_PAGES: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
static UNGUARD_AGAIN: AtomicBool = AtomicBool::new(false);

thread_local! {
    static WINFO: RefCell<InstructionInfoFactory> = RefCell::new(InstructionInfoFactory::new());
}

fn guard_page(page: u32, prot: u32, on: bool) {
    let mut old = 0u32;
    // SAFETY: changes the protection of one committed game data page we chose to watch.
    unsafe {
        VirtualProtect(page as usize as *const c_void, 0x1000, if on { prot | PAGE_GUARD } else { prot }, &mut old)
    };
}

fn watch(path: &str, opts: WatchOpts) -> String {
    if WIN_ON.load(Ordering::SeqCst) || CAP_TID.load(Ordering::SeqCst) != 0 {
        return "error busy (window or capture running)".into();
    }
    let p = std::path::PathBuf::from(path);
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&p))).unwrap_or(p)
    };
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => return format!("error cannot read {}: {e}", p.display()),
    };
    let world = World::read(&ProcMem, gta_range());
    let (lo, hi) = stack_bounds();
    let mut mems = Vec::new();
    let mut fns = Vec::new();
    let mut unresolved = 0;
    for l in fx::parse_spec(&text) {
        match l {
            SpecLine::Mem { sym, size } => match world.resolve(&sym) {
                Some(a) => mems.push(WMem { before: read_vec(a, size), sym, addr: a, size }),
                None => unresolved += 1,
            },
            SpecLine::Fn { addr, name } => {
                if (GTA_CODE.0..GTA_CODE.1).contains(&addr)
                    && ProcMem.u8(addr).is_some_and(|b| b != 0xCC)
                    && !fns.iter().any(|f: &WFn| f.addr == addr)
                {
                    fns.push(WFn { addr, name, orig: ProcMem.u8(addr).unwrap_or(0), hits: 0, logged: 0, armed: false });
                }
            }
        }
    }
    let mut pages: Vec<(u32, u32)> = Vec::new();
    for m in &mems {
        for page in [m.addr & !0xFFF, (m.addr + m.size - 1) & !0xFFF] {
            let code = page >= GTA_CODE.0 && page < GTA_CODE.1;
            let stack = page >= lo && page < hi;
            if code || stack || pages.iter().any(|x| x.0 == page) {
                continue;
            }
            let ever = EVER_PAGES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|x| x.0 == page).map(|x| x.1);
            if let Some(q) = query(page) {
                let prot = match ever {
                    Some(p) if q.Protect & PAGE_GUARD != 0 => p,
                    _ => q.Protect,
                };
                if prot & PAGE_GUARD == 0 && prot & (PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE) != 0 {
                    pages.push((page, prot));
                }
            }
        }
    }
    let n = CAPS.fetch_add(1, Ordering::SeqCst) + 1;
    let client = client_module().map(|m| (m.base, m.end)).unwrap_or((0, 0));
    let mut w = Win {
        n,
        pages: pages.clone(),
        mems,
        fns,
        world,
        opts,
        frames: 0,
        t0: Instant::now(),
        faults: 0,
        writes: 0,
        next_id: 0,
        out: Vec::new(),
        client,
    };
    let objs: Vec<Val> = w.world.named.iter().map(|(nm, b, _)| Val::Str(format!("{nm}={b:#x}"))).collect();
    w.out.push(
        rec("efx_begin", n)
            .with("mode", "window")
            .with("spec", path)
            .with("mems", w.mems.len() as u32)
            .with("unresolved", unresolved)
            .with("pages", Val::List(pages.iter().map(|p| Val::Hex(p.0)).collect()))
            .with("fns", Val::List(w.fns.iter().map(|f| Val::Str(format!("{:#x} {}", f.addr, f.name))).collect()))
            .with("objects", Val::List(objs))
            .with("world", w.world.note.as_str()),
    );
    let msg = format!("ok window {n}: {} mems on {} pages, {} fns", w.mems.len(), pages.len(), w.fns.len());
    *WIN_PAGES.lock().unwrap_or_else(|e| e.into_inner()) = pages.clone();
    {
        let mut ev = EVER_PAGES.lock().unwrap_or_else(|e| e.into_inner());
        for p in &pages {
            if !ev.iter().any(|x| x.0 == p.0) {
                ev.push(*p);
            }
        }
    }
    WIN_CLIENT.store((u64::from(client.0) << 32) | u64::from(client.1), Ordering::SeqCst);
    for f in &mut w.fns {
        f.armed = write_code_byte(f.addr, 0xCC);
    }
    *WIN.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
    WIN_ON.store(true, Ordering::SeqCst);
    for (page, prot) in &pages {
        guard_page(*page, *prot, true);
    }
    LAST_REGION.store(0, Ordering::Relaxed);
    set_status(format!("window {n}"));
    tlog(&format!("efx: {msg}"));
    msg
}

/// Frame boundary (main thread): closes the window once its frames and time have passed.
pub fn frame() {
    if UNGUARD_AGAIN.swap(false, Ordering::SeqCst) && !WIN_ON.load(Ordering::SeqCst) {
        for (page, prot) in EVER_PAGES.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            if query(*page).is_some_and(|q| q.Protect & PAGE_GUARD != 0) {
                guard_page(*page, *prot, false);
            }
        }
        LAST_REGION.store(0, Ordering::Relaxed);
    }
    if !WIN_ON.load(Ordering::SeqCst) {
        return;
    }
    let close = {
        let mut g = WIN.lock().unwrap_or_else(|e| e.into_inner());
        let Some(w) = g.as_mut() else { return };
        w.frames += 1;
        w.frames >= w.opts.frames && w.t0.elapsed().as_millis() as u32 >= w.opts.ms
    };
    if close {
        close_window("window end");
    }
}

fn close_window(why: &str) {
    if !WIN_ON.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some(mut w) = WIN.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    for (page, prot) in &w.pages {
        guard_page(*page, *prot, false);
    }
    WIN_PAGES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    LAST_REGION.store(0, Ordering::Relaxed);
    UNGUARD_AGAIN.store(true, Ordering::SeqCst);
    for f in &w.fns {
        if f.armed {
            write_code_byte(f.addr, f.orig);
        }
    }
    for m in &w.mems {
        let after = read_vec(m.addr, m.size);
        if after != m.before {
            w.out.push(
                rec("efx_net", w.n)
                    .with("sym", m.sym.as_str())
                    .with("size", m.size)
                    .with("old", hexs(&m.before))
                    .with("new", hexs(&after)),
            );
        }
    }
    let calls: Vec<Val> = w
        .fns
        .iter()
        .filter(|f| f.hits > 0)
        .map(|f| Val::Str(format!("{:#x} {} x{}", f.addr, f.name, f.hits)))
        .collect();
    w.out.push(
        rec("efx_end", w.n)
            .with("mode", "window")
            .with("reason", why)
            .with("frames", w.frames)
            .with("ms", w.t0.elapsed().as_millis() as u32)
            .with("faults", w.faults)
            .with("writes", w.writes)
            .with("hits", Val::List(calls)),
    );
    eout(&mut w.out);
    set_status(format!("idle done={}", w.n));
    tlog(&format!("efx: window {} closed ({why}): {} faults, {} writes", w.n, w.faults, w.writes));
}

/// Attribution of an effect seen in window mode: `client` if the instruction is in the client
/// module, else `game`; `via` = client return addresses on the stack (innermost first).
fn attribution(c: &CONTEXT, client: (u32, u32)) -> (&'static str, Option<String>, Option<u32>) {
    let by = if c.Eip >= client.0 && c.Eip < client.1 { "client" } else { "game" };
    let (via, outer) = client_via(c.Esp, client);
    (by, via, outer)
}

fn on_window_guard(rec_addr: u32, c: &mut CONTEXT) -> i32 {
    let page = rec_addr & !0xFFF;
    let pages = win_pages();
    if !pages.iter().any(|x| x.0 == page) {
        // A stale guard of an earlier window: the fault consumed it; just retry.
        let ours = EVER_PAGES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|x| x.0 == page);
        if !ours {
            tlog(&format!("efx: guard fault at {} on {rec_addr:#x}: not a watched page", fmt_loc(c.Eip)));
        }
        return if ours { CONTINUE_EXECUTION } else { CONTINUE_SEARCH };
    }
    // Nested fault of the same instruction on a second watched page.
    let nested = WPEND.with(|p| {
        let mut p = p.borrow_mut();
        match p.as_mut() {
            Some(x) => {
                x.pages.push(page);
                true
            }
            None => false,
        }
    });
    if nested {
        c.EFlags |= TF;
        return CONTINUE_EXECUTION;
    }
    let cl = WIN_CLIENT.load(Ordering::Relaxed);
    let client = ((cl >> 32) as u32, cl as u32);
    // Decode and read without holding the window lock (reads only the now unguarded page).
    let mut acc = Vec::new();
    if let Some((code, n)) = code_at(c.Eip) {
        let cpu = cpu_of(c);
        WINFO.with(|f| {
            if let Some(st) = fx::analyse(&code[..n], &cpu, &NoMem, &mut f.borrow_mut()) {
                acc = st.acc.into_iter().filter(|a| a.write && pages.iter().any(|x| x.0 == a.addr & !0xFFF)).collect();
            }
        });
    }
    let olds: Vec<Option<Vec<u8>>> = acc
        .iter()
        .map(|a| {
            let size = a.size.min(64);
            let same_page = a.addr & !0xFFF == page && (a.addr + size - 1) & !0xFFF == page;
            if same_page { veh_read(a.addr, size) } else { None }
        })
        .collect();
    let (by, via, outer) = if acc.is_empty() { ("", None, None) } else { attribution(c, client) };
    let mut writes = Vec::new();
    {
        let Some(mut g) = win_lock() else {
            // Could not record it; still re-arm the page after the step.
            WPEND.with(|p| *p.borrow_mut() = Some(WPend { pages: vec![page], writes, rearm: None, by, via, outer }));
            c.EFlags |= TF;
            return CONTINUE_EXECUTION;
        };
        let Some(w) = g.as_mut() else { return CONTINUE_EXECUTION };
        w.faults += 1;
        if w.faults > w.opts.faults {
            if w.faults == w.opts.faults + 1 {
                w.out.push(rec("efx_stop", w.n).with("why", "fault budget"));
            }
            return CONTINUE_EXECUTION;
        }
        for (a, old) in acc.iter().zip(olds) {
            let sym = w.world.sym(a.addr).unwrap_or_else(|| format!("{:#x}", a.addr));
            writes.push(Pend { addr: a.addr, size: a.size.min(64), old, sym, at: c.Eip, by: Kind::Gta });
        }
    }
    WPEND.with(|p| *p.borrow_mut() = Some(WPend { pages: vec![page], writes, rearm: None, by, via, outer }));
    c.EFlags |= TF;
    CONTINUE_EXECUTION
}

/// Safety net: an access violation on a page a window guarded (seen under Wine on a second
/// thread touching a page while it was being re-armed) gets the page's normal protection back
/// and the access is retried, instead of killing the game.
fn on_access_violation(addr: u32, c: &mut CONTEXT) -> i32 {
    let page = addr & !0xFFF;
    let Some(prot) = EVER_PAGES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|x| x.0 == page).map(|x| x.1)
    else {
        return CONTINUE_SEARCH;
    };
    let now = query(page).map(|q| q.Protect).unwrap_or(0);
    static LOGGED: AtomicU32 = AtomicU32::new(0);
    if LOGGED.fetch_add(1, Ordering::Relaxed) < 20 {
        tlog(&format!(
            "efx: access violation at {} on watched page {page:#x} (protect {now:#x}, normal {prot:#x}, thread {}); restoring",
            fmt_loc(c.Eip),
            tid()
        ));
    }
    guard_page(page, prot, false);
    LAST_REGION.store(0, Ordering::Relaxed);
    CONTINUE_EXECUTION
}

/// Single step after a window guard fault or a window call hook.
fn on_window_step(c: &mut CONTEXT) -> Option<i32> {
    let p = WPEND.with(|p| p.borrow_mut().take())?;
    c.EFlags &= !TF;
    // New values first: the pages are still unguarded.
    let news: Vec<Option<Vec<u8>>> = p.writes.iter().map(|x| veh_read(x.addr, x.size)).collect();
    if WIN_ON.load(Ordering::SeqCst) {
        let pages = win_pages();
        for page in &p.pages {
            if let Some(&(_, prot)) = pages.iter().find(|x| x.0 == *page) {
                guard_page(*page, prot, true);
            }
        }
        LAST_REGION.store(0, Ordering::Relaxed);
    }
    let Some(mut g) = win_lock() else { return Some(CONTINUE_EXECUTION) };
    let Some(w) = g.as_mut() else { return Some(CONTINUE_EXECUTION) };
    if let Some(a) = p.rearm
        && WIN_ON.load(Ordering::SeqCst)
        && w.fns.iter().any(|f| f.addr == a && f.armed)
    {
        write_code_byte(a, 0xCC);
    }
    for (x, new) in p.writes.into_iter().zip(news) {
        w.writes += 1;
        let mut r = rec("efx_write", w.n)
            .with("mode", "window")
            .with("at", fmt_loc(x.at))
            .with("by", p.by)
            .with("sym", x.sym)
            .with("addr", Val::Hex(x.addr))
            .with("size", x.size)
            .with("old", hexs(&x.old))
            .with("new", hexs(&new));
        if let Some(v) = &p.via {
            r.put("via", v.as_str());
        }
        if let Some(o) = p.outer {
            r.put("gta_outer", Val::Hex(o));
        }
        if x.old.is_some() && x.old == new {
            r.put("same", true);
        }
        w.out.push(r);
    }
    Some(CONTINUE_EXECUTION)
}

/// A window call hook hit while the window state is busy: run the function unrecorded (the
/// hook is put back after one step, like a recorded hit).
fn late_int3(c: &mut CONTEXT, addr: u32) -> i32 {
    let orig = INT3_SITES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|x| x.0 == addr).map(|x| x.1);
    if let Some(b) = orig {
        write_code_byte(addr, b);
    }
    c.Eip = addr;
    WPEND.with(|p| {
        *p.borrow_mut() =
            Some(WPend { pages: Vec::new(), writes: Vec::new(), rearm: Some(addr), by: "", via: None, outer: None })
    });
    c.EFlags |= TF;
    CONTINUE_EXECUTION
}

/// `int3` of a window call hook.
fn on_window_call(c: &mut CONTEXT, addr: u32) -> i32 {
    let Some(mut g) = win_lock() else {
        // Busy: run the function unrecorded (the int3 stays; retried at the next call).
        return late_int3(c, addr);
    };
    let Some(w) = g.as_mut() else { return CONTINUE_SEARCH };
    let Some(i) = w.fns.iter().position(|f| f.addr == addr && f.armed) else { return CONTINUE_SEARCH };
    write_code_byte(addr, w.fns[i].orig);
    c.Eip = addr;
    w.fns[i].hits += 1;
    let keep = w.fns[i].hits < 20 * w.opts.calls.max(1);
    if w.fns[i].logged < w.opts.calls {
        w.fns[i].logged += 1;
        w.next_id += 1;
        let id = w.next_id;
        let (by, via, outer) = attribution(c, w.client);
        let args: Vec<Val> =
            (1..=6).map(|k| Val::Str(format!("{:#x}", ProcMem.u32(c.Esp + 4 * k).unwrap_or(0)))).collect();
        let args_sym: Vec<Val> = (1..=6)
            .map(|k| ProcMem.u32(c.Esp + 4 * k).unwrap_or(0))
            .map(|v| w.world.sym(v).map(Val::Str).unwrap_or(Val::Null))
            .collect();
        let mut r = rec("efx_call", w.n)
            .with("mode", "window")
            .with("id", id)
            .with("to", Val::Hex(addr))
            .with("loc", fmt_loc(addr))
            .with("name", w.fns[i].name.as_str())
            .with("from", fmt_loc(ProcMem.u32(c.Esp).unwrap_or(0)))
            .with("by", by)
            .with("ecx", Val::Hex(c.Ecx))
            .with("args", Val::List(args))
            .with("args_sym", Val::List(args_sym));
        if let Some(s) = w.world.sym(c.Ecx) {
            r.put("this", s);
        }
        if let Some(v) = via {
            r.put("via", v);
        }
        if let Some(o) = outer {
            r.put("gta_outer", Val::Hex(o));
        }
        w.out.push(r);
        push_stub(c.Esp, StubKind::WCall(id));
    }
    if !keep {
        w.fns[i].armed = false;
        w.out.push(rec("efx_unhook", w.n).with("to", Val::Hex(addr)).with("hits", w.fns[i].hits));
    }
    drop(g);
    if keep {
        WPEND.with(|p| {
            *p.borrow_mut() =
                Some(WPend { pages: Vec::new(), writes: Vec::new(), rearm: Some(addr), by: "", via: None, outer: None })
        });
        c.EFlags |= TF;
    }
    CONTINUE_EXECUTION
}

fn on_window_stub(c: &mut CONTEXT, s: Stub, id: u32) {
    c.Eip = s.ret;
    let Some(mut g) = win_lock() else { return };
    if let Some(w) = g.as_mut() {
        let mut r = rec("efx_ret", w.n).with("mode", "window").with("id", id).with("eax", Val::Hex(c.Eax));
        if let Some(sy) = w.world.sym(c.Eax) {
            r.put("eax_sym", sy);
        }
        if let Some(f) = st0(c) {
            r.put("st0", Val::Str(format!("{f:.4}")));
        }
        w.out.push(r);
    }
}

// ------------------------------------------------------------------ exception handler -----

unsafe extern "system" fn veh(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes valid exception and context records.
    let (rec, ctx) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
    let code = rec.ExceptionCode;
    if code != BREAKPOINT && code != SINGLE_STEP && code != GUARD && code != ACCESS_VIOLATION {
        return CONTINUE_SEARCH;
    }
    let ex_addr = rec.ExceptionAddress as usize as u32;
    let p1 = if rec.NumberParameters >= 2 { rec.ExceptionInformation[1] as u32 } else { 0 };
    catch_unwind(AssertUnwindSafe(|| handle_exception(code, ex_addr, p1, ctx))).unwrap_or(CONTINUE_SEARCH)
}

fn handle_exception(code: i32, ex_addr: u32, p1: u32, c: &mut CONTEXT) -> i32 {
    let stub = STUB.load(Ordering::Relaxed);
    let me = tid();
    match code {
        ACCESS_VIOLATION => on_access_violation(p1, c),
        GUARD => on_window_guard(p1, c),
        SINGLE_STEP => {
            if let Some(r) = on_window_step(c) {
                return r;
            }
            if CAP_TID.load(Ordering::SeqCst) == me {
                return on_capture_trap(c, false);
            }
            CONTINUE_SEARCH
        }
        _ => {
            // int3: the address of the int3 itself (Windows/Wine back EIP up by one; accept both).
            let at = |a: u32| ex_addr == a || c.Eip == a || c.Eip == a.wrapping_add(1);
            if stub != 0 && at(stub) {
                c.Eip = stub;
                if CAP_TID.load(Ordering::SeqCst) == me {
                    return on_capture_trap(c, true);
                }
                let Some(s) = pop_stub(c.Esp) else { return CONTINUE_SEARCH };
                match s.kind {
                    StubKind::WCall(id) => on_window_stub(c, s, id),
                    _ => c.Eip = s.ret,
                }
                return CONTINUE_EXECUTION;
            }
            let armed = ARM.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|a| a.va);
            if let Some(va) = armed
                && at(va)
            {
                return start_capture(c, va);
            }
            if WIN_ON.load(Ordering::Relaxed) {
                let hit = match win_lock() {
                    Some(g) => g.as_ref().and_then(|w| w.fns.iter().find(|f| f.armed && at(f.addr)).map(|f| f.addr)),
                    None => INT3_SITES.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|x| x.0).find(|&a| at(a)),
                };
                if let Some(a) = hit {
                    return on_window_call(c, a);
                }
            }
            let late = INT3_SITES.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|x| x.0).find(|&a| at(a));
            if let Some(a) = late {
                if ProcMem.u8(a) == Some(0xCC) {
                    // Being restored by the thread that owns it.
                    std::thread::sleep(Duration::from_millis(1));
                }
                c.Eip = a;
                return CONTINUE_EXECUTION;
            }
            CONTINUE_SEARCH
        }
    }
}

// ------------------------------------------------------------------ setup / control -------

pub fn init(dir: &std::path::Path) {
    let p = std::env::var("SAMP_TRACER_EFX_OUT").map(Into::into).unwrap_or_else(|_| dir.join("efx.jsonl"));
    match File::create(&p) {
        Ok(f) => EOUT.lock().unwrap_or_else(|e| e.into_inner()).file = Some(f),
        Err(e) => tlog(&format!("efx: cannot create {}: {e}", p.display())),
    }
    // SAFETY: plain allocation of 16 bytes of int3 for the return stub.
    let stub =
        unsafe { VirtualAlloc(std::ptr::null(), 16, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize as u32;
    if stub == 0 {
        tlog("efx: VirtualAlloc for the stub failed");
        return;
    }
    // SAFETY: the block was just allocated with 16 writable bytes.
    unsafe { std::ptr::write_bytes(stub as usize as *mut u8, 0xCC, 16) };
    STUB.store(stub, Ordering::SeqCst);
    // SAFETY: registers our handler first in the vectored chain.
    let h = unsafe { AddVectoredExceptionHandler(1, Some(veh)) };
    if h.is_null() {
        tlog("efx: AddVectoredExceptionHandler failed");
        STUB.store(0, Ordering::SeqCst);
        return;
    }
    set_status("idle done=0".into());
}

/// Control-channel `efx …` command.
pub fn command(args: &str) -> String {
    if STUB.load(Ordering::SeqCst) == 0 {
        return "error efx not initialised".into();
    }
    match fx::parse_cmd(args) {
        Err(e) => format!("error {e}"),
        Ok(EfxCmd::Status) => STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        Ok(EfxCmd::Disarm) => {
            disarm();
            close_window("disarm");
            set_status(format!("idle done={}", CAPS.load(Ordering::SeqCst)));
            "ok".into()
        }
        Ok(EfxCmd::Arm(t, o)) => arm(t, o),
        Ok(EfxCmd::Watch(spec, o)) => watch(&spec, o),
    }
}
