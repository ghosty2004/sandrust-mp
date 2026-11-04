//! TRACE-2: remote players.
//!
//! * `CPlayerPed::ProcessControl` is wrapped twice. The inline hook at
//!   0x60EA90 (inner) sees the game context the client set up for the ped;
//!   once a client module has redirected the CPlayerPed vtable slot
//!   (0x86D190), an outer wrapper is put in front of the client's function,
//!   so the context is also sampled before the client changes it and after it
//!   restores it. One `pc` record per call: `pre` (before the client's
//!   wrapper), `in`/`out` (around GTA's function), `post`.
//! * At every frame boundary `CWorld::Players[0..210].m_pPed` is scanned
//!   (`players` record on change) and the memory of `Players[2..209]` is
//!   diffed against the previous frame (`players_mem` for offsets that change
//!   for the first time, `players_mem_summary` every 300 frames).
//! * `SAMP_TRACER_GUARD=<budget>`: the pages of `Players[2..209]` (the former
//!   sector array) get PAGE_GUARD; a vectored handler records the instruction
//!   address of every access (`guard` record per new site, `guard_summary`
//!   every 300 frames), single-steps it and re-arms the page, up to <budget>
//!   accesses per frame.

use super::*;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};
use windows_sys::Win32::System::Memory::PAGE_READWRITE;

/// `CPlayerPed` vtable slot of `ProcessControl` (vtable 0x86D168 + 10 × 4; gta-reversed PlayerPed.cpp).
const PC_VTABLE_SLOT: u32 = 0x86D190;
const PC_ORIGINAL: u32 = 0x60EA90;
/// Call site of `CPed::UpdatePosition` inside `CPed::ProcessControl` and the two
/// `m_fAimingRotation` stores samp toggles around remote/local processing
/// (remote-control-combat.md §1.3); sampled as raw bytes.
const UPDATE_POSITION_CALL: u32 = 0x5E92F4;
const AIM_ROT_STORE_A: u32 = 0x6884C4;
const AIM_ROT_STORE_B: u32 = 0x688200;
const SUMMARY_EVERY: u64 = 300;

// ------------------------------------------------------------- context sampling ---

fn hex(m: &ProcMem, a: u32, n: usize) -> Val {
    let mut b = vec![0u8; n];
    if m.read(a, &mut b) { Val::Str(b.iter().map(|x| format!("{x:02x}")).collect()) } else { Val::Null }
}

fn vec3(m: &ProcMem, a: u32) -> Val {
    match (m.f32(a), m.f32(a + 4), m.f32(a + 8)) {
        (Some(x), Some(y), Some(z)) => Val::List(vec![x.into(), y.into(), z.into()]),
        _ => Val::Null,
    }
}

/// Non-zero 16-bit fields of `CPad::Pads[0].NewState` (24 × i16), as "index:value".
pub(super) fn pad_state(m: &ProcMem) -> Val {
    let mut b = [0u8; 0x30];
    if !m.read(gta::PAD0, &mut b) {
        return Val::Null;
    }
    let nz: Vec<Val> = b
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let v = i16::from_le_bytes(*c);
            (v != 0).then(|| Val::Str(format!("{i}:{v}")))
        })
        .collect();
    Val::List(nz)
}

/// Game context + the ped's own state.
fn sample(ped: u32) -> Val {
    let m = ProcMem;
    let mut v = Val::obj()
        .with("pif", m.i32(gta::PLAYER_IN_FOCUS))
        .with("pad", pad_state(&m))
        .with("cam_mode", m.u16(gta::CAM0_MODE).map(u32::from))
        .with("weapon_mode", m.u16(gta::CAM_WEAPON_MODE).map(u32::from))
        .with("fov", m.f32(gta::CAM0_FOV))
        .with("aspect", m.f32(gta::ASPECT_RATIO))
        .with("skill0", m.f32(gta::STAT_PISTOL_SKILL))
        .with("front", vec3(&m, gta::CAM0_FRONT))
        .with("upd_call", hex(&m, UPDATE_POSITION_CALL, 5))
        .with("aimrot_a", hex(&m, AIM_ROT_STORE_A, 6))
        .with("aimrot_b", hex(&m, AIM_ROT_STORE_B, 6));
    if let Some(p) = gta::entity_pos(&m, ped) {
        v.put("pos", Val::List(p.iter().map(|&f| f.into()).collect()));
    }
    v.put("speed", vec3(&m, ped + gta::off::PHYSICAL_MOVE_SPEED));
    v.put("heading", m.f32(ped + gta::off::PED_HEADING));
    v.put("heading_goal", m.f32(ped + gta::off::PED_HEADING_GOAL));
    v.put("ped_state", m.u32(ped + gta::off::PED_STATE));
    v.put("move_state", m.u32(ped + gta::off::PED_MOVE_STATE));
    v
}

// ------------------------------------------------------------- ProcessControl -----

/// Return address of the current inner call (set by the dispatcher before the wrapper runs).
static PC_CALLER: AtomicU32 = AtomicU32::new(0);
/// The client's function that the vtable slot pointed to before our outer wrapper.
static CLIENT_PC: AtomicU32 = AtomicU32::new(0);
static LOCAL_PC_FRAMES: AtomicU64 = AtomicU64::new(0);

struct PcRec {
    ped: u32,
    pre: Option<Val>,
    inner: Option<(String, Val, Val)>,
}

thread_local! {
    static OUTER: std::cell::RefCell<Vec<PcRec>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn on_pc_call(r: &Regs) {
    PC_CALLER.store(ProcMem.u32(r.esp).unwrap_or(0), Ordering::Relaxed);
}

fn wanted(ped: u32) -> bool {
    // Every remote ped call; the local ped every 30th frame as a baseline.
    if gta::player_slot_of(&ProcMem, ped) != Some(0) {
        return true;
    }
    let f = FRAME_TICKS.load(Ordering::Relaxed);
    let last = LOCAL_PC_FRAMES.load(Ordering::Relaxed);
    if f >= last + 30 || f < last {
        LOCAL_PC_FRAMES.store(f, Ordering::Relaxed);
        return true;
    }
    false
}

fn emit_pc(rec: PcRec, outer_caller: Option<String>) {
    let m = ProcMem;
    let mut a = Val::obj().with("ped", gta::describe_entity(&m, rec.ped));
    if let Some(c) = outer_caller {
        a.put("vtable_target", c);
    }
    if let Some(p) = rec.pre {
        a.put("pre", p);
    }
    match rec.inner {
        Some((caller, i, o)) => {
            a.put("orig_caller", caller);
            a.put("in", i);
            a.put("out", o);
        }
        None => a.put("orig_called", false),
    }
    emit(record("pc", None, None, None, a));
}

/// Continuation of the inline hook at 0x60EA90 (thiscall, no arguments).
pub extern "thiscall" fn pc_inner_wrap(this: u32) {
    let caller = PC_CALLER.load(Ordering::Relaxed);
    let in_outer = OUTER.with(|o| o.borrow().last().is_some_and(|r| r.ped == this));
    let want = in_outer || catch_unwind(AssertUnwindSafe(|| wanted(this))).unwrap_or(false);
    let pre = want.then(|| catch_unwind(AssertUnwindSafe(|| sample(this))).ok()).flatten();
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(tramp_of(Special::PedProcessControl) as usize) };
    f(this);
    let Some(pre) = pre else { return };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let post = sample(this);
        let inner = (fmt_loc(caller), pre, post);
        if in_outer {
            OUTER.with(|o| {
                if let Some(r) = o.borrow_mut().last_mut() {
                    r.inner = Some(inner);
                }
            });
        } else {
            emit_pc(PcRec { ped: this, pre: None, inner: Some(inner) }, None);
        }
    }));
}

/// Our function in the CPlayerPed vtable, in front of the client's.
extern "thiscall" fn pc_outer_wrap(this: u32) {
    let target = CLIENT_PC.load(Ordering::Relaxed);
    let want = catch_unwind(AssertUnwindSafe(|| wanted(this))).unwrap_or(false);
    if want {
        let pre = catch_unwind(AssertUnwindSafe(|| sample(this))).ok();
        OUTER.with(|o| o.borrow_mut().push(PcRec { ped: this, pre, inner: None }));
    }
    // SAFETY: the client's ProcessControl replacement, same signature as GTA's.
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(target as usize) };
    f(this);
    if want {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let post = sample(this);
            if let Some(mut r) = OUTER.with(|o| o.borrow_mut().pop()) {
                let mut p = r.pre.take().unwrap_or(Val::Null);
                p = Val::obj().with("before", p).with("after", post);
                r.pre = Some(p);
                emit_pc(r, Some(fmt_loc(target)));
            }
        }));
    }
}

/// Once a client module owns the vtable slot, put our outer wrapper in front of it.
fn install_outer() {
    if CLIENT_PC.load(Ordering::Relaxed) != 0 {
        return;
    }
    let cur = ProcMem.u32(PC_VTABLE_SLOT).unwrap_or(0);
    let ours = pc_outer_wrap as extern "thiscall" fn(u32) as usize as u32;
    if cur == 0 || cur == PC_ORIGINAL || cur == ours || !module_of(cur).is_some_and(|m| m.client) {
        return;
    }
    CLIENT_PC.store(cur, Ordering::SeqCst);
    let mut old = 0u32;
    // SAFETY: one pointer in the game's vtable (.rdata); we are on the main thread at the frame boundary.
    unsafe {
        let p = PC_VTABLE_SLOT as usize as *mut c_void;
        VirtualProtect(p, 4, PAGE_EXECUTE_READWRITE, &mut old);
        std::ptr::write_volatile(p as *mut u32, ours);
        VirtualProtect(p, 4, old, &mut old);
    }
    emit(record(
        "vtable_wrap",
        None,
        None,
        None,
        Val::obj().with("slot_ptr", Val::Hex(PC_VTABLE_SLOT)).with("client_target", fmt_loc(cur)),
    ));
    tlog(&format!("CPlayerPed::ProcessControl vtable slot: client target {}", fmt_loc(cur)));
}

// ------------------------------------------------------------- per-frame ped samples

/// Position / speed / heading of one ped (no game context).
fn ped_sample(ped: u32) -> Val {
    let m = ProcMem;
    let mut v = Val::obj();
    if let Some(p) = gta::entity_pos(&m, ped) {
        v.put("pos", Val::List(p.iter().map(|&f| f.into()).collect()));
    }
    v.put("speed", vec3(&m, ped + gta::off::PHYSICAL_MOVE_SPEED));
    v.put("heading", m.f32(ped + gta::off::PED_HEADING));
    v.put("heading_goal", m.f32(ped + gta::off::PED_HEADING_GOAL));
    v
}

/// slot → (ped, samples taken so far this frame).
static RPED: Mutex<Vec<(u32, u32, Val)>> = Mutex::new(Vec::new());

fn rped_sample(at: &str) {
    let peds = gta::scan_cached_remote();
    let mut g = RPED.lock().unwrap_or_else(|e| e.into_inner());
    for (slot, ped) in peds {
        let smp = ped_sample(ped);
        match g.iter_mut().find(|(s, p, _)| *s == slot && *p == ped) {
            Some((_, _, v)) => v.put(at, smp),
            None => g.push((slot, ped, Val::obj().with(at, smp))),
        }
    }
}

fn rped_emit() {
    let rows = std::mem::take(&mut *RPED.lock().unwrap_or_else(|e| e.into_inner()));
    for (slot, ped, v) in rows {
        let mut a = Val::obj().with("slot", slot).with("ped_ptr", Val::Hex(ped));
        if let Val::Obj(f) = v {
            for (k, x) in f {
                a.put(&k, x);
            }
        }
        a.put("timestep", ProcMem.f32(gta::TIME_STEP));
        emit(record("rped", None, None, None, a));
    }
}

/// Continuation of the inline hook at `CWorld::Process` (cdecl, no arguments).
pub extern "C" fn world_process_wrap() {
    let _ = catch_unwind(|| rped_sample("world_in"));
    let _ = catch_unwind(|| super::vehicles::sample_stage("world_in"));
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: extern "C" fn() = unsafe { std::mem::transmute(tramp_of(Special::WorldProcess) as usize) };
    f();
    let _ = catch_unwind(|| {
        rped_sample("world_out");
        rped_emit();
        super::vehicles::sample_stage("world_out");
        super::vehicles::emit_stage();
    });
}

// ------------------------------------------------------------- Players[] memory ---

static DEAD: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

pub fn on_ped_ctor(this: u32) {
    if let Some(s) = DEAD.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.remove(&this);
    }
}

pub fn on_ped_dtor(this: u32) {
    // The slot cache keeps the pointer until the next frame boundary, so the
    // destruction's own calls (CWorld::Remove, …) are still labelled with the slot.
    DEAD.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(this);
}

const MEM_FIRST_SLOT: u32 = 2;

#[derive(Default)]
struct PlayersState {
    peds: Vec<(u32, u32)>,
    mem: Vec<u32>,
    /// (slot, offset) → number of frames it changed in.
    changed: BTreeMap<(u32, u32), u32>,
    dirty: bool,
}

static PLAYERS: Mutex<Option<PlayersState>> = Mutex::new(None);

fn players_frame(n: u64) {
    let m = ProcMem;
    let mut g = PLAYERS.lock().unwrap_or_else(|e| e.into_inner());
    let st = g.get_or_insert_with(PlayersState::default);
    // Slot → ped. Pointers to destroyed peds (samp may leave them behind) are
    // listed as "(dead)" and kept out of the remote-ped cache.
    let dead = DEAD.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    let all = gta::scan_player_peds(&m);
    if all != st.peds {
        let list: Vec<Val> = all
            .iter()
            .map(|&(s, p)| Val::Str(format!("{s}:{p:#x}{}", if dead.contains(&p) { "(dead)" } else { "" })))
            .collect();
        emit(record("players", None, None, None, Val::obj().with("slots", Val::List(list))));
        st.peds = all.clone();
    }
    gta::set_remote_peds(all.into_iter().filter(|&(s, p)| s != 0 && !dead.contains(&p)).collect());
    // Memory of Players[2..210].
    let base = gta::player_info(MEM_FIRST_SLOT);
    let len = ((gta::SAMP_PLAYER_SLOTS - MEM_FIRST_SLOT) * gta::PLAYER_INFO_SIZE) as usize;
    let mut raw = vec![0u8; len];
    if !m.read(base, &mut raw) {
        return;
    }
    let cur: Vec<u32> = raw.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    if st.mem.len() == cur.len() {
        let mut fresh = Val::obj();
        let mut nfresh = 0;
        for (i, (a, b)) in st.mem.iter().zip(&cur).enumerate() {
            if a == b {
                continue;
            }
            let byte = i as u32 * 4;
            let key = (MEM_FIRST_SLOT + byte / gta::PLAYER_INFO_SIZE, byte % gta::PLAYER_INFO_SIZE);
            let c = st.changed.entry(key).or_insert(0);
            *c += 1;
            st.dirty = true;
            // New offsets, and every change of m_pPed (+0x0).
            if *c == 1 || key.1 == 0 {
                nfresh += 1;
                if nfresh <= 48 {
                    fresh.put(&format!("{}+{:#x}", key.0, key.1), format!("{a:#x}->{b:#x}"));
                }
            }
        }
        if nfresh > 48 {
            fresh.put("more", nfresh - 48);
        }
        if nfresh > 0 {
            emit(record("players_mem", None, None, None, fresh));
        }
    } else {
        // First snapshot: which slots hold anything at all.
        let nz: BTreeMap<u32, u32> =
            cur.iter().enumerate().filter(|(_, w)| **w != 0).fold(BTreeMap::new(), |mut acc, (i, _)| {
                *acc.entry(MEM_FIRST_SLOT + i as u32 * 4 / gta::PLAYER_INFO_SIZE).or_insert(0) += 1;
                acc
            });
        let v: Vec<Val> = nz.iter().map(|(s, c)| Val::Str(format!("{s}:{c}"))).collect();
        emit(record(
            "players_mem_base",
            None,
            None,
            None,
            Val::obj()
                .with("base_ptr", Val::Hex(base))
                .with("bytes", len as u32)
                .with("nonzero_words_by_slot", Val::List(v)),
        ));
    }
    st.mem = cur;
    if n.is_multiple_of(SUMMARY_EVERY) {
        players_summary(st);
    }
}

fn players_summary(st: &mut PlayersState) {
    if !st.dirty {
        return;
    }
    st.dirty = false;
    let mut v = Val::obj();
    for ((s, o), c) in st.changed.iter().take(400) {
        v.put(&format!("{s}+{o:#x}"), *c);
    }
    emit(record("players_mem_summary", None, None, None, v));
}

/// Final summaries (quit).
pub fn summaries() {
    if let Some(st) = PLAYERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        players_summary(st);
    }
    if let Some(agg) = GUARD_AGG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        guard_summary(agg);
    }
}

// ------------------------------------------------------------- guard pages --------

const GUARD_CAP: usize = 16384;
static G_EIP: [AtomicU32; GUARD_CAP] = [const { AtomicU32::new(0) }; GUARD_CAP];
static G_ADDR: [AtomicU32; GUARD_CAP] = [const { AtomicU32::new(0) }; GUARD_CAP];
/// bit 0 = write; bits 8.. = PlayerInFocus at the time of the access.
static G_INFO: [AtomicU32; GUARD_CAP] = [const { AtomicU32::new(0) }; GUARD_CAP];
static G_N: AtomicU32 = AtomicU32::new(0);
static G_HITS: AtomicU32 = AtomicU32::new(0);
static G_BUDGET: AtomicU32 = AtomicU32::new(0);
static G_ON: AtomicBool = AtomicBool::new(false);
static G_SELF: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static G_PENDING: Cell<u32> = const { Cell::new(0) };
}

fn guard_range() -> (u32, u32) {
    let a = gta::CWORLD_SECTORS & !0xFFF;
    let b = (gta::player_info(gta::SAMP_PLAYER_SLOTS) + 0xFFF) & !0xFFF;
    (a, b)
}

fn protect(page_from: u32, page_to: u32, guard: bool) {
    let prot = if guard { PAGE_READWRITE | PAGE_GUARD } else { PAGE_READWRITE };
    let mut old = 0u32;
    // SAFETY: changes the protection of the game's own .bss pages (Players[2..] / former sector array).
    unsafe { VirtualProtect(page_from as usize as *const c_void, (page_to - page_from) as usize, prot, &mut old) };
}

unsafe extern "system" fn guard_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    const GUARD: i32 = 0x8000_0001u32 as i32;
    const SINGLE_STEP: i32 = 0x8000_0004u32 as i32;
    const CONTINUE_EXECUTION: i32 = -1;
    const CONTINUE_SEARCH: i32 = 0;
    // SAFETY: the system passes valid exception and context records.
    let (rec, ctx) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
    let (lo, hi) = guard_range();
    match rec.ExceptionCode {
        GUARD => {
            let addr = rec.ExceptionInformation[1] as u32;
            if rec.NumberParameters < 2 || addr < lo || addr >= hi {
                return CONTINUE_SEARCH;
            }
            let eip = ctx.Eip;
            let sb = G_SELF.load(Ordering::Relaxed);
            let own = eip as u64 >= sb >> 32 && (eip as u64) < (sb & 0xFFFF_FFFF);
            if !own {
                let i = G_N.fetch_add(1, Ordering::Relaxed) as usize;
                if i < GUARD_CAP {
                    // SAFETY: plain .data read (page not guarded).
                    let pif = unsafe { std::ptr::read_volatile(gta::PLAYER_IN_FOCUS as usize as *const i32) };
                    G_EIP[i].store(eip, Ordering::Relaxed);
                    G_ADDR[i].store(addr, Ordering::Relaxed);
                    G_INFO[i].store(
                        (rec.ExceptionInformation[0] as u32 & 1) | ((pif as u32 & 0xFF_FFFF) << 8),
                        Ordering::Relaxed,
                    );
                }
            }
            let hits = if own { 0 } else { G_HITS.fetch_add(1, Ordering::Relaxed) + 1 };
            if G_ON.load(Ordering::Relaxed) && hits <= G_BUDGET.load(Ordering::Relaxed) {
                G_PENDING.with(|p| p.set(addr & !0xFFF));
                ctx.EFlags |= 0x100;
            }
            CONTINUE_EXECUTION
        }
        SINGLE_STEP => {
            let page = G_PENDING.with(|p| p.replace(0));
            if page == 0 {
                return CONTINUE_SEARCH;
            }
            if G_ON.load(Ordering::Relaxed) {
                protect(page, page + 0x1000, true);
            }
            ctx.EFlags &= !0x100;
            CONTINUE_EXECUTION
        }
        _ => CONTINUE_SEARCH,
    }
}

#[derive(Default)]
struct GuardAgg {
    seen: HashMap<(u32, u32, u32, u32), u32>,
    frames: u32,
    hits: u64,
    over_budget_frames: u32,
    dirty: bool,
}

static GUARD_AGG: Mutex<Option<GuardAgg>> = Mutex::new(None);

pub fn init() {
    let Some(budget) = std::env::var("SAMP_TRACER_GUARD").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    if budget == 0 {
        return;
    }
    let me = guard_handler as unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32 as usize as u32;
    if let Some(md) = module_of(me) {
        G_SELF.store((u64::from(md.base) << 32) | u64::from(md.end), Ordering::SeqCst);
    }
    G_BUDGET.store(budget, Ordering::SeqCst);
    // SAFETY: registers our handler first in the vectored chain.
    let h = unsafe { AddVectoredExceptionHandler(1, Some(guard_handler)) };
    if h.is_null() {
        tlog("guard: AddVectoredExceptionHandler failed");
        return;
    }
    *GUARD_AGG.lock().unwrap_or_else(|e| e.into_inner()) = Some(GuardAgg::default());
    let (a, b) = guard_range();
    tlog(&format!("guard: watching {a:#x}..{b:#x}, budget {budget}/frame"));
    emit(record(
        "guard_start",
        None,
        None,
        None,
        Val::obj().with("from_ptr", Val::Hex(a)).with("to_ptr", Val::Hex(b)).with("budget", budget),
    ));
}

fn guard_enabled() -> bool {
    GUARD_AGG.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Frame boundary, before reading Players memory: disarm and collect.
fn guard_drain(n: u64) {
    if !guard_enabled() {
        return;
    }
    G_ON.store(false, Ordering::SeqCst);
    let (a, b) = guard_range();
    protect(a, b, false);
    let cnt = (G_N.swap(0, Ordering::SeqCst) as usize).min(GUARD_CAP);
    let hits = G_HITS.swap(0, Ordering::SeqCst);
    let mut g = GUARD_AGG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(agg) = g.as_mut() else { return };
    agg.frames += 1;
    agg.hits += u64::from(hits);
    if hits > G_BUDGET.load(Ordering::Relaxed) {
        agg.over_budget_frames += 1;
    }
    for i in 0..cnt {
        let eip = G_EIP[i].load(Ordering::Relaxed);
        let addr = G_ADDR[i].load(Ordering::Relaxed);
        let info = G_INFO[i].load(Ordering::Relaxed);
        let rel = addr.wrapping_sub(gta::CWORLD_PLAYERS);
        let (slot, off) = (rel / gta::PLAYER_INFO_SIZE, rel % gta::PLAYER_INFO_SIZE);
        let key = (eip, slot, off, info & 1);
        let c = agg.seen.entry(key).or_insert(0);
        *c += 1;
        agg.dirty = true;
        if *c == 1 {
            emit(record(
                "guard",
                None,
                None,
                None,
                Val::obj()
                    .with("eip", fmt_loc(eip))
                    .with("addr_ptr", Val::Hex(addr))
                    .with("slot", slot)
                    .with("off", Val::Hex(off))
                    .with("write", info & 1 == 1)
                    .with("pif", (info >> 8) as i32),
            ));
        }
    }
    if n.is_multiple_of(SUMMARY_EVERY) {
        guard_summary(agg);
    }
}

fn guard_summary(agg: &mut GuardAgg) {
    if !agg.dirty {
        return;
    }
    agg.dirty = false;
    let mut rows: Vec<_> = agg.seen.iter().collect();
    rows.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
    let list: Vec<Val> = rows
        .iter()
        .take(400)
        .map(|((eip, s, o, w), c)| {
            Val::Str(format!("{} slot{s}+{o:#x} {} x{c}", fmt_loc(*eip), if *w == 1 { "W" } else { "R" }))
        })
        .collect();
    emit(record(
        "guard_summary",
        None,
        None,
        None,
        Val::obj()
            .with("frames", agg.frames)
            .with("hits", agg.hits as u32)
            .with("over_budget_frames", agg.over_budget_frames)
            .with("sites", Val::List(list)),
    ));
}

fn guard_arm() {
    if !guard_enabled() {
        return;
    }
    let (a, b) = guard_range();
    G_ON.store(true, Ordering::SeqCst);
    protect(a, b, true);
}

// ------------------------------------------------------------- frame hook ----------

/// Called from `on_frame` (main thread, CGame::Process entry).
pub fn frame(n: u64) {
    guard_drain(n);
    install_outer();
    players_frame(n);
    rped_sample("frame_start");
    guard_arm();
}
