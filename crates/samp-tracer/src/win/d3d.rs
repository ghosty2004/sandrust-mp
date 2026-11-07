//! Direct3D9 / D3DX call-stream logger, runtime side (tables and formatting in `crate::d3d`), installed
//! from the first `CGame::Process` that finds a device in `_RwD3DDevice`:
//! * the real device is reached through `GetSwapChain(0)` → `GetDevice` on whatever object sits there (a
//!   client's proxy forwards these), so the hooks see every call that reaches the driver, whoever makes it;
//! * the device, D3DX font, sprite, render-to-surface and vertex-buffer class vtables are patched (one
//!   throw-away object of each D3DX class is created to find them);
//! * the D3DX factory exports of the loaded `d3dx9_NN.dll` are inline-hooked at their hot-patch prologue.
//!
//! Output: `d3d.jsonl` next to `trace.jsonl` (`SAMP_TRACER_D3D_OUT`).

use super::*;
use crate::d3d::{self as dd, Iface, METHODS, Method, Mode, Sp};
use std::cell::Cell;
use std::collections::HashMap;
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

/// `_RwD3DDevice`: the device pointer RenderWare draws through (a client may put a proxy here).
const RW_D3D_DEVICE: u32 = 0xC97C28;
/// `FrontEndMenuManager.m_bHudOn` (believed) and `CHud::bScriptDontDisplayRadar`
/// (ui-rendering.md, "Present: per-frame draw order").
const HUD_ON: u32 = 0xBA6769;
const HUD_NO_RADAR: u32 = 0xBAA3FB;
const DT_CALCRECT: u32 = 0x400;
const SHADOW_MAX: usize = 4 << 20;

static D3D_OUT: Mutex<Out> = Mutex::new(Out { buf: String::new(), file: None, seq: 0 });

static INSTALLED: AtomicBool = AtomicBool::new(false);
static INSTALL_TRIES: AtomicU32 = AtomicU32::new(0);
static MODE: AtomicU32 = AtomicU32::new(0);
static FRAMES_LEFT: AtomicU32 = AtomicU32::new(0);
/// Requested (mode code << 24 | frames), applied at the next Present; u32::MAX = none.
static PENDING: AtomicU32 = AtomicU32::new(u32::MAX);
static PRESENTS: AtomicU64 = AtomicU64::new(0);
/// 0 = no scene, 1 = scene begun by the game (or its proxy), 2 = scene begun by D3DX.
static SCENE: AtomicU32 = AtomicU32::new(0);
static RECORDING_SB: AtomicBool = AtomicBool::new(false);
static CUR_FVF: AtomicU32 = AtomicU32::new(0);
static STREAM0_VB: AtomicU32 = AtomicU32::new(0);
static STREAM0_OFF: AtomicU32 = AtomicU32::new(0);
static STREAM0_STRIDE: AtomicU32 = AtomicU32::new(0);
static TEX0: AtomicU32 = AtomicU32::new(0);
static CUR_DECL: AtomicU32 = AtomicU32::new(0);
static LAST_SHOW: AtomicU32 = AtomicU32::new(u32::MAX);
static SHOW_REPEATS: AtomicU32 = AtomicU32::new(0);
/// Last device VIEW (0..16) and PROJECTION (16..32) matrices and viewport (x, y, w, h as u32,
/// min/max z as f32 bits), whatever the logging mode (`xform` control command).
static XFORM: [AtomicU32; 38] = [const { AtomicU32::new(0) }; 38];
static D3D9_RANGE: AtomicU64 = AtomicU64::new(0);
static D3DX_RANGE: AtomicU64 = AtomicU64::new(0);

struct State {
    /// (vtable, slot) → original function.
    origs: HashMap<(u32, u32), u32>,
    patched: Vec<(Iface, u32)>,
    labels: HashMap<u32, String>,
    locks: HashMap<u32, (u32, u32, u32)>,
    shadow: HashMap<u32, Vec<u8>>,
    block: u32,
    used: u32,
    exports: HashMap<&'static str, u32>,
    decls: HashMap<u32, dd::Layout>,
}

static ST: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    static FACTORY_DEPTH: Cell<u32> = const { Cell::new(0) };
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().map(f)
}

fn mode() -> Mode {
    Mode::from_code(MODE.load(Ordering::Relaxed) as u8)
}

fn in_range(r: &AtomicU64, a: u32) -> bool {
    let v = r.load(Ordering::Relaxed);
    let (b, e) = ((v >> 32) as u32, v as u32);
    e != 0 && a >= b && a < e
}

fn set_range(r: &AtomicU64, addr: u32) {
    if let Some(m) = module_of(addr) {
        r.store((u64::from(m.base) << 32) | u64::from(m.end), Ordering::Relaxed);
    }
}

fn d3d_emit(mut rec: Val) {
    let mut g = D3D_OUT.lock().unwrap_or_else(|e| e.into_inner());
    g.seq += 1;
    let seq = g.seq;
    if let Val::Obj(f) = &mut rec {
        f.insert(0, ("n".into(), Val::Int(seq as i64)));
    }
    let line = rec.to_json();
    g.buf.push_str(&line);
    g.buf.push('\n');
}

pub fn flush() {
    let mut g = D3D_OUT.lock().unwrap_or_else(|e| e.into_inner());
    flush_locked(&mut g);
}

pub fn init(dir: &std::path::Path) {
    let p = std::env::var("SAMP_TRACER_D3D_OUT").map(Into::into).unwrap_or_else(|_| dir.join("d3d.jsonl"));
    match File::create(&p) {
        Ok(f) => D3D_OUT.lock().unwrap_or_else(|e| e.into_inner()).file = Some(f),
        Err(e) => tlog(&format!("d3d: cannot create {}: {e}", p.display())),
    }
    if let Ok(spec) = std::env::var("SAMP_TRACER_D3D_BOOT") {
        match dd::parse_cmd(&spec) {
            Some((m, frames)) if m != Mode::Off => {
                BOOT.store((u32::from(m.code()) << 24) | frames, Ordering::SeqCst);
                std::thread::spawn(boot_watch);
                tlog(&format!("d3d: boot logging '{spec}' armed"));
            }
            _ => tlog(&format!("d3d: SAMP_TRACER_D3D_BOOT '{spec}' ignored (want MODE [FRAMES])")),
        }
    }
}

// ------------------------------------------------------------------ boot logging ----------
//
// `SAMP_TRACER_D3D_BOOT=MODE [FRAMES]` logs from the first Present of the process (the
// loading screen), long before the first `CGame::Process`. A helper thread waits for a device
// in `_RwD3DDevice` and points the Present slot of its vtable at `boot_present`; that runs on
// the game thread, puts the slot back, installs the normal hooks and starts logging. With
// MODE `all`, logging drops to `ui` once `gGameState` reaches 9 (the world scene is large).
// Every change of `gGameState` seen at a Present is written as a `game_state` record.

const GAME_STATE: u32 = 0xC8D4C0;
/// Boot request (mode code << 24 | frames), 0 = none.
static BOOT: AtomicU32 = AtomicU32::new(0);
static BOOT_SLOT: AtomicU32 = AtomicU32::new(0);
static BOOT_ORIG: AtomicU32 = AtomicU32::new(0);
static LAST_GS: AtomicU32 = AtomicU32::new(u32::MAX);
/// The real device found by `install` (for a late D3DX install).
static REAL_DEV: AtomicU32 = AtomicU32::new(0);
static D3DX_LATE: AtomicBool = AtomicBool::new(false);

fn boot_watch() {
    for _ in 0..600_000 {
        let dev = ProcMem.u32(RW_D3D_DEVICE).unwrap_or(0);
        if dev != 0
            && let Some(vt) = ProcMem.u32(dev)
            && let Some(orig) = ProcMem.u32(vt + 4 * 17)
        {
            BOOT_ORIG.store(orig, Ordering::SeqCst);
            BOOT_SLOT.store(vt + 4 * 17, Ordering::SeqCst);
            let f = boot_present as extern "system" fn(u32, u32, u32, u32, u32) -> i32 as usize as u32;
            if !write_u32(vt + 4 * 17, f) {
                tlog("d3d: boot: cannot patch Present");
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

extern "system" fn boot_present(this: u32, a: u32, b: u32, c: u32, d: u32) -> i32 {
    let slot = BOOT_SLOT.load(Ordering::SeqCst);
    write_u32(slot, BOOT_ORIG.load(Ordering::SeqCst));
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !INSTALLED.swap(true, Ordering::SeqCst) {
            match install(this) {
                Ok(msg) => {
                    tlog(&format!("d3d: installed at the first Present: {msg}"));
                    d3d_emit(base_rec("d3d_installed").with("info", msg.as_str()).with("boot", true));
                }
                Err(e) => {
                    tlog(&format!("d3d: boot install FAILED: {e}"));
                    d3d_emit(base_rec("d3d_installed").with("error", e.as_str()));
                }
            }
        }
        PENDING.store(BOOT.load(Ordering::SeqCst), Ordering::SeqCst);
    }));
    // SAFETY: Present (slot 17) of the same object, now through the (hooked) vtable.
    match unsafe { vcall::<extern "system" fn(u32, u32, u32, u32, u32) -> i32>(this, 17) } {
        Some(f) if f as usize as u32 != boot_present as extern "system" fn(u32, u32, u32, u32, u32) -> i32 as usize as u32 => {
            f(this, a, b, c, d)
        }
        _ => {
            // SAFETY: the original Present read from the vtable before patching it.
            let f: extern "system" fn(u32, u32, u32, u32, u32) -> i32 =
                unsafe { std::mem::transmute(BOOT_ORIG.load(Ordering::SeqCst) as usize) };
            f(this, a, b, c, d)
        }
    }
}

/// Per Present: `game_state` records, the boot-mode switch from `all` to `ui`, and the D3DX
/// hooks once a client loads `d3dx9_NN.dll` after the first Present (every 16th Present).
fn boot_present_step() {
    if D3DX_RANGE.load(Ordering::Relaxed) == 0
        && PRESENTS.load(Ordering::Relaxed).is_multiple_of(16)
        && find_d3dx().is_some()
        && !D3DX_LATE.swap(true, Ordering::SeqCst)
    {
        let msg = install_d3dx(REAL_DEV.load(Ordering::SeqCst));
        tlog(&format!("d3d: late D3DX install: {msg}"));
        d3d_emit(base_rec("d3d_installed").with("info", msg.as_str()).with("late_d3dx", true));
    }
    let gs = ProcMem.u32(GAME_STATE).unwrap_or(u32::MAX);
    if LAST_GS.swap(gs, Ordering::Relaxed) != gs {
        d3d_emit(base_rec("game_state").with("gs", gs));
    }
    if BOOT.load(Ordering::Relaxed) != 0 && gs == 9 && mode() == Mode::All {
        BOOT.store(0, Ordering::Relaxed);
        MODE.store(u32::from(Mode::Ui.code()), Ordering::SeqCst);
        d3d_emit(base_rec("d3d_logging").with("mode", "ui").with("frames", FRAMES_LEFT.load(Ordering::Relaxed)));
    }
}

/// Mark records are copied into the D3D log so it can be cut by scenario step.
pub fn mark(text: &str) {
    d3d_emit(base_rec("mark").with("text", text));
    flush();
}

fn scene_name() -> &'static str {
    match SCENE.load(Ordering::Relaxed) {
        1 => "game",
        2 => "d3dx",
        _ => "none",
    }
}

fn base_rec(name: &str) -> Val {
    Val::obj()
        .with("pr", PRESENTS.load(Ordering::Relaxed) as i64)
        .with("f", super::frame())
        .with("t", ms())
        .with("sc", scene_name())
        .with("fn", name)
}

// ------------------------------------------------------------------ control ---------------

/// `d3d devreset N`: the real device's `TestCooperativeLevel` (slot 3) reports D3DERR_DEVICENOTRESET for the next N
/// calls, so RenderWare (and the client's device proxy in front of it) goes through a device loss and `Reset` —
/// the same input for the original client and ours.
static FORCE_LOST: AtomicU32 = AtomicU32::new(0);
static ORIG_TEST_COOP: AtomicU32 = AtomicU32::new(0);

extern "system" fn test_cooperative_level(this: u32) -> i32 {
    if FORCE_LOST.load(Ordering::Relaxed) > 0 {
        FORCE_LOST.fetch_sub(1, Ordering::Relaxed);
        return 0x8876_0869_u32 as i32;
    }
    let f = ORIG_TEST_COOP.load(Ordering::Relaxed);
    // SAFETY: the real IDirect3DDevice9::TestCooperativeLevel saved from the vtable.
    let f: extern "system" fn(u32) -> i32 = unsafe { std::mem::transmute(f as usize) };
    f(this)
}

fn devreset(n: u32) -> String {
    let dev = REAL_DEV.load(Ordering::SeqCst);
    if dev == 0 {
        return "error d3d hooks not installed".into();
    }
    if ORIG_TEST_COOP.load(Ordering::SeqCst) == 0 {
        let Some(vt) = ProcMem.u32(dev) else { return "error device unreadable".into() };
        let Some(orig) = ProcMem.u32(vt + 4 * 3) else { return "error vtable unreadable".into() };
        ORIG_TEST_COOP.store(orig, Ordering::SeqCst);
        if !write_u32(vt + 4 * 3, test_cooperative_level as extern "system" fn(u32) -> i32 as usize as u32) {
            return "error VirtualProtect failed".into();
        }
    }
    FORCE_LOST.store(n, Ordering::SeqCst);
    format!("ok device reports lost for {n} checks")
}

/// `d3d …` control command.
pub fn command(args: &str) -> String {
    if let Some(n) = args.strip_prefix("devreset") {
        let n = n.trim();
        return match if n.is_empty() { Ok(15) } else { n.parse::<u32>() } {
            Ok(n) => devreset(n.min(600)),
            Err(_) => "error usage: d3d devreset [CHECKS]".into(),
        };
    }
    let Some((m, frames)) = dd::parse_cmd(args) else {
        return "error usage: d3d off | d3d on|ui|all|text [FRAMES]".into();
    };
    if m == Mode::Off {
        PENDING.store(u32::MAX, Ordering::SeqCst);
        MODE.store(0, Ordering::SeqCst);
        FRAMES_LEFT.store(0, Ordering::SeqCst);
        return "ok d3d off".into();
    }
    PENDING.store((u32::from(m.code()) << 24) | frames, Ordering::SeqCst);
    format!(
        "ok d3d {} {frames} installed={} present={}",
        m.name(),
        INSTALLED.load(Ordering::SeqCst),
        PRESENTS.load(Ordering::SeqCst)
    )
}

/// `xform`: the last device VIEW and PROJECTION matrices (row-major, full precision) and the
/// last viewport, as `xform vp=X,Y,W,H,MINZ,MAXZ view=16 floats proj=16 floats`.
pub fn xform() -> String {
    let f = |i: usize| f32::from_bits(XFORM[i].load(Ordering::Relaxed));
    let u = |i: usize| XFORM[i].load(Ordering::Relaxed);
    let mat = |b: usize| (b..b + 16).map(|i| format!("{:e}", f(i))).collect::<Vec<_>>().join(",");
    format!(
        "xform vp={},{},{},{},{:e},{:e} view={} proj={}",
        u(32),
        u(33),
        u(34),
        u(35),
        f(36),
        f(37),
        mat(0),
        mat(16)
    )
}

/// Whether logging is (or is about to be) active; `d3d_busy` lets scripts wait for the end.
pub fn busy() -> bool {
    PENDING.load(Ordering::SeqCst) != u32::MAX || MODE.load(Ordering::SeqCst) != 0
}

// ------------------------------------------------------------------ COM helpers -----------

unsafe fn vcall<F: Copy>(obj: u32, slot: u32) -> Option<F> {
    let vt = ProcMem.u32(obj)?;
    let f = ProcMem.u32(vt + 4 * slot)?;
    if f == 0 {
        return None;
    }
    // SAFETY: caller supplies the right signature for the slot.
    Some(unsafe { std::mem::transmute_copy::<u32, F>(&f) })
}

type FnRelease = extern "system" fn(u32) -> u32;
type FnGetType = extern "system" fn(u32) -> u32;
type FnGetDesc = extern "system" fn(u32, *mut u32) -> i32;
type FnGetLevelDesc = extern "system" fn(u32, u32, *mut u32) -> i32;

fn release(obj: u32) {
    // SAFETY: IUnknown::Release on a live COM object we own a reference to.
    if let Some(f) = unsafe { vcall::<FnRelease>(obj, 2) } {
        f(obj);
    }
}

/// `WxH fN uX pN` of a texture (level 0), surface or vertex buffer.
fn resource_desc(obj: u32) -> Option<String> {
    // SAFETY: obj is a live IDirect3DResource9 passed to a device method; GetType is slot 10.
    let ty = unsafe { vcall::<FnGetType>(obj, 10)? }(obj);
    let mut d = [0u32; 8];
    match ty {
        3 => {
            // SAFETY: IDirect3DTexture9::GetLevelDesc (slot 17), D3DSURFACE_DESC = 8 dwords.
            let f = unsafe { vcall::<FnGetLevelDesc>(obj, 17)? };
            (f(obj, 0, d.as_mut_ptr()) >= 0).then(|| format!("tex {}x{} f{} u{:#x} p{}", d[6], d[7], d[0], d[2], d[3]))
        }
        1 => {
            // SAFETY: IDirect3DSurface9::GetDesc (slot 12).
            let f = unsafe { vcall::<FnGetDesc>(obj, 12)? };
            (f(obj, d.as_mut_ptr()) >= 0).then(|| format!("surf {}x{} f{} u{:#x} p{}", d[6], d[7], d[0], d[2], d[3]))
        }
        6 => {
            // SAFETY: IDirect3DVertexBuffer9::GetDesc (slot 13): Format, Type, Usage, Pool, Size, FVF.
            let f = unsafe { vcall::<FnGetDesc>(obj, 13)? };
            (f(obj, d.as_mut_ptr()) >= 0).then(|| format!("vb {}B u{:#x} p{} fvf{:#x}", d[4], d[2], d[3], d[5]))
        }
        4 => Some("cubetex".into()),
        7 => Some("ib".into()),
        t => Some(format!("res{t}")),
    }
}

fn vb_size(vb: u32) -> u32 {
    let mut d = [0u32; 6];
    // SAFETY: IDirect3DVertexBuffer9::GetDesc (slot 13).
    match unsafe { vcall::<FnGetDesc>(vb, 13) } {
        Some(f) if f(vb, d.as_mut_ptr()) >= 0 => d[4],
        _ => 0,
    }
}

fn font_label(font: u32) -> String {
    if let Some(Some(l)) = st(|s| s.labels.get(&font).cloned()) {
        return l;
    }
    let mut d = [0u8; 56];
    // SAFETY: ID3DXFont::GetDescA (slot 4) fills a D3DXFONT_DESCA (56 bytes).
    let l = match unsafe { vcall::<extern "system" fn(u32, *mut u8) -> i32>(font, 4) } {
        Some(f) if f(font, d.as_mut_ptr()) >= 0 => dd::font_desc_label(&d),
        _ => "font ?".into(),
    };
    st(|s| s.labels.insert(font, l.clone()));
    l
}

/// Label for an object argument: resource description (+ file name when known).
fn obj_label(p: u32, resource: bool) -> Option<String> {
    let name = st(|s| s.labels.get(&p).cloned()).flatten();
    if !resource {
        return name;
    }
    let desc = resource_desc(p);
    match (name, desc) {
        (Some(n), Some(d)) => Some(format!("{d} {n}")),
        (n, d) => n.or(d),
    }
}

// ------------------------------------------------------------------ patching --------------

fn alloc_thunk(s: &mut State) -> Option<u32> {
    if s.block == 0 {
        // SAFETY: plain RWX allocation for the thunks.
        s.block = unsafe { VirtualAlloc(std::ptr::null(), 0x10000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) }
            as usize as u32;
        s.used = 0;
        if s.block == 0 {
            return None;
        }
    }
    if s.used + 32 > 0x10000 {
        return None;
    }
    let a = s.block + s.used;
    s.used += 32;
    Some(a)
}

/// Generic pre-call thunk: `pushad; push esp; push idx; call dispatch; add esp,8; popad;
/// jmp [orig]` with the original pointer stored at +24.
fn pre_thunk(s: &mut State, idx: u32, orig: u32) -> Option<u32> {
    let at = alloc_thunk(s)?;
    let disp = dispatch as extern "C" fn(u32, *const Regs) as usize as u32;
    let mut t = vec![0x60, 0x54, 0x68];
    t.extend_from_slice(&idx.to_le_bytes());
    t.push(0xE8);
    t.extend_from_slice(&x86::rel32_to(at + 7, 5, disp).to_le_bytes());
    t.extend_from_slice(&[0x83, 0xC4, 0x08, 0x61, 0xFF, 0x25]);
    t.extend_from_slice(&(at + 24).to_le_bytes());
    t.resize(24, 0xCC);
    t.extend_from_slice(&orig.to_le_bytes());
    // SAFETY: `at` is a fresh 32-byte slot in our RWX block.
    unsafe { std::ptr::copy_nonoverlapping(t.as_ptr(), at as usize as *mut u8, t.len()) };
    Some(at)
}

fn typed_target(sp: Sp) -> Option<u32> {
    Some(match sp {
        Sp::DrawTextA => draw_text_a as extern "system" fn(u32, u32, u32, i32, u32, u32, u32) -> i32 as usize as u32,
        Sp::DrawTextW => draw_text_w as extern "system" fn(u32, u32, u32, i32, u32, u32, u32) -> i32 as usize as u32,
        Sp::VbLock => vb_lock as extern "system" fn(u32, u32, u32, u32, u32) -> i32 as usize as u32,
        _ => return None,
    })
}

fn write_u32(addr: u32, v: u32) -> bool {
    let mut old = 0u32;
    // SAFETY: vtable entry of a live COM class; made writable for the store and restored.
    unsafe {
        let p = addr as usize as *mut c_void;
        if VirtualProtect(p, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return false;
        }
        std::ptr::write_volatile(addr as usize as *mut u32, v);
        VirtualProtect(p, 4, old, &mut old);
    }
    true
}

/// Patches every hooked slot of the vtable of `obj` (once per vtable).
fn hook_vtable(iface: Iface, obj: u32) -> Result<u32, String> {
    let vt = ProcMem.u32(obj).ok_or("object unreadable")?;
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.as_mut().ok_or("no state")?;
    if s.patched.contains(&(iface, vt)) {
        return Ok(vt);
    }
    for (idx, m) in dd::methods_of(iface) {
        let entry = vt + 4 * m.slot;
        let orig = ProcMem.u32(entry).ok_or("vtable unreadable")?;
        let target = match typed_target(m.sp) {
            Some(t) => t,
            None => pre_thunk(s, idx as u32, orig).ok_or("thunk block full")?,
        };
        s.origs.insert((vt, m.slot), orig);
        if !write_u32(entry, target) {
            return Err(format!("{}.{}: VirtualProtect failed", iface.prefix(), m.name));
        }
    }
    s.patched.push((iface, vt));
    drop(g);
    tlog(&format!("d3d: hooked {} vtable {vt:#x} ({})", iface.prefix(), fmt_loc(vt)));
    Ok(vt)
}

fn orig_of(obj: u32, slot: u32) -> u32 {
    let vt = ProcMem.u32(obj).unwrap_or(0);
    st(|s| s.origs.get(&(vt, slot)).copied()).flatten().unwrap_or(0)
}

// ------------------------------------------------------------------ install ---------------

/// Called every frame until the hooks are in.
pub fn frame() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let n = INSTALL_TRIES.fetch_add(1, Ordering::Relaxed);
    if n > 600 || std::env::var("SAMP_TRACER_D3D").ok().as_deref() == Some("0") {
        return;
    }
    let dev = ProcMem.u32(RW_D3D_DEVICE).unwrap_or(0);
    if dev == 0 {
        return;
    }
    INSTALLED.store(true, Ordering::SeqCst);
    match install(dev) {
        Ok(msg) => {
            tlog(&format!("d3d: installed: {msg}"));
            d3d_emit(base_rec("d3d_installed").with("info", msg.as_str()));
        }
        Err(e) => {
            tlog(&format!("d3d: install FAILED: {e}"));
            d3d_emit(base_rec("d3d_installed").with("error", e.as_str()));
        }
    }
    flush();
}

fn install(dev_slot_obj: u32) -> Result<String, String> {
    *ST.lock().unwrap_or_else(|e| e.into_inner()) = Some(State {
        origs: HashMap::new(),
        patched: Vec::new(),
        labels: HashMap::new(),
        locks: HashMap::new(),
        shadow: HashMap::new(),
        block: 0,
        used: 0,
        exports: HashMap::new(),
        decls: HashMap::new(),
    });
    // Real device: GetSwapChain(0) (slot 14) → IDirect3DSwapChain9::GetDevice (slot 8).
    let mut sc = 0u32;
    // SAFETY: the object in _RwD3DDevice implements IDirect3DDevice9 (directly or as a proxy).
    let gsc =
        unsafe { vcall::<extern "system" fn(u32, u32, *mut u32) -> i32>(dev_slot_obj, 14) }.ok_or("GetSwapChain")?;
    if gsc(dev_slot_obj, 0, &mut sc) < 0 || sc == 0 {
        return Err("GetSwapChain failed".into());
    }
    let mut dev = 0u32;
    // SAFETY: IDirect3DSwapChain9::GetDevice.
    let gd = unsafe { vcall::<extern "system" fn(u32, *mut u32) -> i32>(sc, 8) }.ok_or("GetDevice")?;
    let r = gd(sc, &mut dev);
    release(sc);
    if r < 0 || dev == 0 {
        return Err("SwapChain::GetDevice failed".into());
    }
    release(dev);
    REAL_DEV.store(dev, Ordering::SeqCst);
    let dev_vt = ProcMem.u32(dev).unwrap_or(0);
    set_range(&D3D9_RANGE, dev_vt);
    let proxied = dev != dev_slot_obj;
    hook_vtable(Iface::Device, dev)?;
    let mut info = format!(
        "device {dev:#x} vtable {} (slot object {dev_slot_obj:#x}{})",
        fmt_loc(dev_vt),
        if proxied { ", a proxy" } else { "" }
    );
    // A vertex buffer of the driver's class (managed pool), for the Lock/Unlock vtable.
    let mut vb = 0u32;
    // SAFETY: IDirect3DDevice9::CreateVertexBuffer (slot 26) on the real device.
    if let Some(cvb) = unsafe { vcall::<extern "system" fn(u32, u32, u32, u32, u32, *mut u32, u32) -> i32>(dev, 26) }
        && cvb(dev, 64, 0, 0, 1, &mut vb, 0) >= 0
        && vb != 0
    {
        let _ = hook_vtable(Iface::VertexBuffer, vb);
        release(vb);
    }
    info.push_str(&format!("; {}", install_d3dx(dev)));
    info.push_str(&format!("; {}", video_modes()));
    Ok(info)
}

/// RenderWare's video mode list (`RwEngineGetNumVideoModes` 0x7F2CC0, `RwEngineGetVideoModeInfo`
/// 0x7F2CF0, plugin-sdk): the index stored in gta_sa.set (`m_nDisplayVideoMode`) selects one.
fn video_modes() -> String {
    // SAFETY: plain cdecl RenderWare engine queries, called on the game thread after RwEngineStart.
    let n: extern "C" fn() -> i32 = unsafe { std::mem::transmute(0x7F2CC0usize) };
    let info: extern "C" fn(*mut i32, i32) -> u32 = unsafe { std::mem::transmute(0x7F2CF0usize) };
    let count = n();
    let mut out = Vec::new();
    for i in 0..count.min(256) {
        let mut vm = [0i32; 6];
        info(vm.as_mut_ptr(), i);
        out.push(format!("{i}:{}x{}x{}{}", vm[0], vm[1], vm[2], if vm[3] & 1 != 0 { "" } else { "w" }));
    }
    let w = ProcMem.u32(0xC17044).unwrap_or(0);
    let h = ProcMem.u32(0xC17048).unwrap_or(0);
    format!("video {w}x{h}; modes [{}]", out.join(" "))
}

const D3DX_EXPORTS: &[&str] = &[
    "D3DXCreateFontA",
    "D3DXCreateFontW",
    "D3DXCreateFontIndirectA",
    "D3DXCreateFontIndirectW",
    "D3DXCreateSprite",
    "D3DXCreateRenderToSurface",
    "D3DXCreateTexture",
    "D3DXCreateTextureFromFileExA",
    "D3DXCreateTextureFromFileExW",
    "D3DXCreateTextureFromFileInMemoryEx",
    "D3DXLoadSurfaceFromFileA",
];

fn export_wrapper(name: &str) -> u32 {
    match name {
        "D3DXCreateFontA" => create_font_a as FnCreateFont as usize as u32,
        "D3DXCreateFontW" => create_font_w as FnCreateFont as usize as u32,
        "D3DXCreateFontIndirectA" => create_font_ind_a as FnCreateFontInd as usize as u32,
        "D3DXCreateFontIndirectW" => create_font_ind_w as FnCreateFontInd as usize as u32,
        "D3DXCreateSprite" => create_sprite as FnCreateSprite as usize as u32,
        "D3DXCreateRenderToSurface" => create_rts as FnCreateRts as usize as u32,
        "D3DXCreateTexture" => create_texture as FnCreateTexture as usize as u32,
        "D3DXCreateTextureFromFileExA" => tex_from_file_a as FnTexFromFileEx as usize as u32,
        "D3DXCreateTextureFromFileExW" => tex_from_file_w as FnTexFromFileEx as usize as u32,
        "D3DXCreateTextureFromFileInMemoryEx" => tex_from_mem as FnTexFromMemEx as usize as u32,
        "D3DXLoadSurfaceFromFileA" => load_surface_a as FnLoadSurface as usize as u32,
        _ => 0,
    }
}

fn find_d3dx() -> Option<(u32, String)> {
    for v in (24..=43).rev() {
        let name = format!("d3dx9_{v}.dll");
        let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated wide string.
        let h = unsafe { GetModuleHandleW(w.as_ptr()) };
        if !h.is_null() {
            return Some((h as usize as u32, name));
        }
    }
    None
}

fn proc_addr(module: u32, name: &str) -> u32 {
    let c: Vec<u8> = name.bytes().chain(std::iter::once(0)).collect();
    // SAFETY: valid module handle and NUL-terminated name.
    unsafe { GetProcAddress(module as usize as *mut c_void, c.as_ptr()) }.map(|f| f as usize as u32).unwrap_or(0)
}

/// Hooks the D3DX exports and the D3DX class vtables (found by creating one object each).
fn install_d3dx(dev: u32) -> String {
    let Some((module, name)) = find_d3dx() else { return "no d3dx9_NN.dll loaded".into() };
    set_range(&D3DX_RANGE, module);
    let mut done = Vec::new();
    let mut failed = Vec::new();
    // Probe objects first, through the unhooked exports.
    let cf = proc_addr(module, "D3DXCreateFontA");
    if cf != 0 {
        let mut font = 0u32;
        let face = b"Arial\0";
        // SAFETY: D3DXCreateFontA signature (12 stdcall args).
        let f: FnCreateFont = unsafe { std::mem::transmute(cf as usize) };
        if f(dev, 10, 0, 400, 1, 0, 1, 0, 0, 0, face.as_ptr() as usize as u32, &mut font as *mut u32 as usize as u32)
            >= 0
            && font != 0
        {
            if let Err(e) = hook_vtable(Iface::Font, font) {
                failed.push(format!("font vtable: {e}"));
            }
            release(font);
        }
    }
    let cs = proc_addr(module, "D3DXCreateSprite");
    if cs != 0 {
        let mut sp = 0u32;
        // SAFETY: D3DXCreateSprite(device, ID3DXSprite**).
        let f: FnCreateSprite = unsafe { std::mem::transmute(cs as usize) };
        if f(dev, &mut sp as *mut u32 as usize as u32) >= 0 && sp != 0 {
            if let Err(e) = hook_vtable(Iface::Sprite, sp) {
                failed.push(format!("sprite vtable: {e}"));
            }
            release(sp);
        }
    }
    let cr = proc_addr(module, "D3DXCreateRenderToSurface");
    if cr != 0 {
        let mut r = 0u32;
        // SAFETY: D3DXCreateRenderToSurface(device, w, h, format, depth, depth format, out).
        let f: FnCreateRts = unsafe { std::mem::transmute(cr as usize) };
        if f(dev, 16, 16, 21, 0, 0, &mut r as *mut u32 as usize as u32) >= 0 && r != 0 {
            if let Err(e) = hook_vtable(Iface::RenderToSurface, r) {
                failed.push(format!("rts vtable: {e}"));
            }
            release(r);
        }
    }
    for &e in D3DX_EXPORTS {
        let a = proc_addr(module, e);
        if a == 0 {
            failed.push(format!("{e}: not exported"));
            continue;
        }
        match inline_hook(a, export_wrapper(e)) {
            Ok(tramp) => {
                st(|s| s.exports.insert(e, tramp));
                done.push(e);
            }
            Err(x) => failed.push(format!("{e}: {x}")),
        }
    }
    format!("{name}: {} exports hooked; failed: [{}]", done.len(), failed.join("; "))
}

/// Inline hook of a hot-patchable export: entry → `jmp wrapper`; returns the trampoline.
fn inline_hook(addr: u32, wrapper: u32) -> Result<u32, String> {
    let mut code = [0u8; 16];
    if !ProcMem.read(addr, &mut code) {
        return Err("unreadable".into());
    }
    let tramp = {
        let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
        alloc_thunk(g.as_mut().ok_or("no state")?).ok_or("thunk block full")?
    };
    let (moved, n) = x86::relocate(&code, addr, tramp, 5).map_err(|e| e.to_string())?;
    let mut t = moved;
    let back = tramp + t.len() as u32;
    t.push(0xE9);
    t.extend_from_slice(&x86::rel32_to(back, 5, addr + n as u32).to_le_bytes());
    if t.len() > 32 {
        return Err("prologue too long".into());
    }
    // SAFETY: fresh slot in our RWX block.
    unsafe { std::ptr::copy_nonoverlapping(t.as_ptr(), tramp as usize as *mut u8, t.len()) };
    let mut patch = vec![0xE9];
    patch.extend_from_slice(&x86::rel32_to(addr, 5, wrapper).to_le_bytes());
    patch.resize(n, 0x90);
    let mut old = 0u32;
    // SAFETY: patching the export entry we just read and relocated (main thread, not running it).
    unsafe {
        let p = addr as usize as *mut c_void;
        if VirtualProtect(p, n, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return Err("VirtualProtect failed".into());
        }
        std::ptr::copy_nonoverlapping(patch.as_ptr(), p as *mut u8, n);
        VirtualProtect(p, n, old, &mut old);
        FlushInstructionCache(GetCurrentProcess(), p, n);
    }
    Ok(tramp)
}

fn export_tramp(name: &str) -> usize {
    st(|s| s.exports.get(name).copied()).flatten().unwrap_or(0) as usize
}

// ------------------------------------------------------------------ dispatch --------------

extern "C" fn dispatch(idx: u32, regs: *const Regs) {
    // SAFETY: the thunk passes a pointer to the pushad block on its own stack.
    let r = unsafe { &*regs };
    let _ = catch_unwind(AssertUnwindSafe(|| on_call(idx as usize, r)));
}

fn caller_fields(rec: &mut Val, ret: u32, esp: u32) {
    let c = module_of(ret);
    rec.put("by", c.as_ref().map(|c| format!("{}+{:#x}", c.name, ret - c.base)).unwrap_or(format!("{ret:#x}")));
    if !c.is_some_and(|c| c.client)
        && let Some(o) = client_origin(esp + 4)
    {
        rec.put("org", o);
    }
}

fn on_call(idx: usize, r: &Regs) {
    let Some(meth) = METHODS.get(idx) else { return };
    let m = ProcMem;
    let esp = r.esp;
    let ret = m.u32(esp).unwrap_or(0);
    if in_range(&D3D9_RANGE, ret) {
        return; // the driver calling itself
    }
    let arg = |i: u32| m.u32(esp + 8 + 4 * i).unwrap_or(0);
    let this = m.u32(esp + 4).unwrap_or(0);
    // State tracking, whether or not the call is written.
    match meth.sp {
        Sp::BeginScene => SCENE.store(if in_range(&D3DX_RANGE, ret) { 2 } else { 1 }, Ordering::Relaxed),
        Sp::SetFvf => CUR_FVF.store(arg(0), Ordering::Relaxed),
        Sp::SetVertexDeclaration => {
            CUR_FVF.store(0, Ordering::Relaxed);
            CUR_DECL.store(arg(0), Ordering::Relaxed);
        }
        Sp::SetStreamSource if arg(0) == 0 => {
            STREAM0_VB.store(arg(1), Ordering::Relaxed);
            STREAM0_OFF.store(arg(2), Ordering::Relaxed);
            STREAM0_STRIDE.store(arg(3), Ordering::Relaxed);
        }
        Sp::SetTexture if arg(0) == 0 => TEX0.store(arg(1), Ordering::Relaxed),
        Sp::BeginStateBlock => RECORDING_SB.store(true, Ordering::Relaxed),
        // VIEW and the last perspective PROJECTION (_34 ≠ 0): D3DX sprites set their own
        // orthographic one late in the frame, which is not the world camera.
        Sp::SetTransform
            if arg(0) == 2 || (arg(0) == 3 && m.u32(arg(1) + 4 * 11).is_some_and(|v| f32::from_bits(v) != 0.0)) =>
        {
            let base = if arg(0) == 2 { 0 } else { 16 };
            for i in 0..16 {
                XFORM[base + i].store(m.u32(arg(1) + 4 * i as u32).unwrap_or(0), Ordering::Relaxed);
            }
        }
        Sp::SetViewport => {
            for i in 0..6 {
                XFORM[32 + i].store(m.u32(arg(0) + 4 * i as u32).unwrap_or(0), Ordering::Relaxed);
            }
        }
        Sp::VbUnlock => return vb_unlock(this),
        _ => {}
    }
    let mode = mode();
    let rec_sb = RECORDING_SB.load(Ordering::Relaxed) && meth.iface == Iface::Device;
    let mut write = dd::wanted(mode, meth, SCENE.load(Ordering::Relaxed) == 1) || rec_sb;
    if meth.sp == Sp::ShowCursor {
        let v = arg(0);
        let last = LAST_SHOW.swap(v, Ordering::Relaxed);
        if last == v && mode == Mode::Off {
            SHOW_REPEATS.fetch_add(1, Ordering::Relaxed);
            write = false;
        }
    }
    if write {
        let mut rec = base_rec(&format!("{}.{}", meth.iface.prefix(), meth.name));
        if meth.iface != Iface::Device {
            rec.put("obj_ptr", Val::Hex(this));
            if meth.iface == Iface::Sprite {
                rec.put("sprite", obj_label(this, false));
            }
        }
        caller_fields(&mut rec, ret, esp);
        let mut lab = |p: u32| {
            let resource = !matches!(meth.sp, Sp::SetVertexDeclaration) && !meth.name.contains("Shader");
            obj_label(p, resource)
        };
        rec.put("args", dd::render_args(&m, meth, esp, &mut lab));
        if rec_sb {
            rec.put("recording_stateblock", true);
        }
        if meth.sp == Sp::ShowCursor {
            rec.put("repeats_before", SHOW_REPEATS.swap(0, Ordering::Relaxed));
        }
        draw_extras(meth, &mut rec, &arg);
        if meth.sp == Sp::Present {
            rec.put(
                "hud",
                Val::List(vec![m.u8(HUD_ON).map(u32::from).into(), m.u8(HUD_NO_RADAR).map(u32::from).into()]),
            );
        }
        d3d_emit(rec);
    }
    match meth.sp {
        Sp::EndScene => SCENE.store(0, Ordering::Relaxed),
        Sp::EndStateBlock => RECORDING_SB.store(false, Ordering::Relaxed),
        Sp::Present => on_present(),
        _ => {}
    }
}

fn read_bytes(addr: u32, n: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; n];
    ProcMem.read(addr, &mut b).then_some(b)
}

fn layout() -> Option<dd::Layout> {
    let f = CUR_FVF.load(Ordering::Relaxed);
    if f != 0 {
        return Some(dd::fvf_layout(f));
    }
    let d = CUR_DECL.load(Ordering::Relaxed);
    if d == 0 {
        return None;
    }
    if let Some(Some(l)) = st(|s| s.decls.get(&d).copied()) {
        return Some(l);
    }
    // IDirect3DVertexDeclaration9::GetDeclaration (slot 4): elements + count.
    let mut n = 0u32;
    // SAFETY: the declaration is the live object last passed to SetVertexDeclaration.
    let gd = unsafe { vcall::<extern "system" fn(u32, *mut u8, *mut u32) -> i32>(d, 4) }?;
    if gd(d, std::ptr::null_mut(), &mut n) < 0 || n == 0 || n > 64 {
        return None;
    }
    let mut buf = vec![0u8; 8 * n as usize];
    if gd(d, buf.as_mut_ptr(), &mut n) < 0 {
        return None;
    }
    let l = dd::decl_layout(&buf);
    st(|s| s.decls.insert(d, l));
    Some(l)
}

fn draw_extras(meth: &Method, rec: &mut Val, arg: &dyn Fn(u32) -> u32) {
    let fvf = CUR_FVF.load(Ordering::Relaxed);
    let from_vb = |first: u32, count: u32| -> Val {
        let vb = STREAM0_VB.load(Ordering::Relaxed);
        let stride = STREAM0_STRIDE.load(Ordering::Relaxed);
        let off = STREAM0_OFF.load(Ordering::Relaxed) as usize + first as usize * stride as usize;
        let n = count as usize * stride as usize;
        let data = st(|s| s.shadow.get(&vb).and_then(|b| b.get(off..off + n).map(|x| x.to_vec()))).flatten();
        match data {
            Some(d) => dd::vertices_val(&d, stride, count, layout().as_ref()),
            None => Val::obj().with("n", count).with("note", "vertex buffer not shadowed"),
        }
    };
    let tex = || {
        let t = TEX0.load(Ordering::Relaxed);
        if t == 0 { Val::Null } else { obj_label(t, true).map(Val::Str).unwrap_or(Val::Null) }
    };
    match meth.sp {
        Sp::DrawPrimitiveUp => {
            let (ty, prims, data, stride) = (arg(0), arg(1), arg(2), arg(3));
            let n = dd::vertex_count(ty, prims);
            let raw = read_bytes(data, (n * stride).min(1 << 20) as usize).unwrap_or_default();
            rec.put("fvf", Val::Str(format!("{fvf:#x}")));
            rec.put("tex0", tex());
            rec.put("vertices", dd::vertices_val(&raw, stride, n, layout().as_ref()));
        }
        Sp::DrawIndexedPrimitiveUp => {
            let (min, nv, idx, ifmt, data, stride) = (arg(1), arg(2), arg(4), arg(5), arg(6), arg(7));
            let ni = dd::vertex_count(arg(0), arg(3));
            let isz = if ifmt == 102 { 4 } else { 2 };
            if let Some(ib) = read_bytes(idx, (ni * isz).min(4096) as usize) {
                let list: Vec<Val> = ib
                    .chunks(isz as usize)
                    .take(dd::MAX_DECODED)
                    .map(|c| {
                        Val::Int(
                            if isz == 4 {
                                u32::from_le_bytes([c[0], c[1], c[2], c[3]])
                            } else {
                                u32::from(u16::from_le_bytes([c[0], c[1]]))
                            }
                            .into(),
                        )
                    })
                    .collect();
                rec.put("indices", Val::List(list));
            }
            let raw = read_bytes(data + min * stride, (nv * stride).min(1 << 20) as usize).unwrap_or_default();
            rec.put("fvf", Val::Str(format!("{fvf:#x}")));
            rec.put("tex0", tex());
            rec.put("vertices", dd::vertices_val(&raw, stride, nv, layout().as_ref()));
        }
        Sp::DrawPrimitive => {
            rec.put("fvf", Val::Str(format!("{fvf:#x}")));
            rec.put("tex0", tex());
            rec.put("vertices", from_vb(arg(1), dd::vertex_count(arg(0), arg(2))));
        }
        Sp::DrawIndexedPrimitive => {
            rec.put("fvf", Val::Str(format!("{fvf:#x}")));
            rec.put("tex0", tex());
            rec.put("vertices", from_vb((arg(1) as i32 + arg(2) as i32).max(0) as u32, arg(3)));
        }
        _ => {}
    }
}

fn on_present() {
    PRESENTS.fetch_add(1, Ordering::Relaxed);
    boot_present_step();
    if MODE.load(Ordering::Relaxed) != 0 {
        let left = FRAMES_LEFT.load(Ordering::Relaxed).saturating_sub(1);
        FRAMES_LEFT.store(left, Ordering::Relaxed);
        if left == 0 {
            MODE.store(0, Ordering::Relaxed);
            d3d_emit(base_rec("d3d_logging").with("mode", "off"));
            st(|s| {
                s.shadow.clear();
                s.locks.clear();
            });
        }
    }
    let p = PENDING.swap(u32::MAX, Ordering::SeqCst);
    if p != u32::MAX {
        let (m, frames) = (Mode::from_code((p >> 24) as u8), p & 0xFF_FFFF);
        MODE.store(u32::from(m.code()), Ordering::SeqCst);
        FRAMES_LEFT.store(frames, Ordering::SeqCst);
        d3d_emit(base_rec("d3d_logging").with("mode", m.name()).with("frames", frames));
    }
    flush();
}

// ------------------------------------------------------------------ vertex buffers --------

extern "system" fn vb_lock(this: u32, off: u32, size: u32, pp: u32, flags: u32) -> i32 {
    let o = orig_of(this, 11);
    if o == 0 {
        return -1;
    }
    // SAFETY: original IDirect3DVertexBuffer9::Lock.
    let f: extern "system" fn(u32, u32, u32, u32, u32) -> i32 = unsafe { std::mem::transmute(o as usize) };
    let r = f(this, off, size, pp, flags);
    if r >= 0 && MODE.load(Ordering::Relaxed) != 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let ptr = ProcMem.u32(pp).unwrap_or(0);
            st(|s| s.locks.insert(this, (off, size, ptr)));
        }));
    }
    r
}

fn vb_unlock(vb: u32) {
    let Some(Some((off, mut size, ptr))) = st(|s| s.locks.remove(&vb)) else { return };
    if ptr == 0 {
        return;
    }
    if size == 0 {
        size = vb_size(vb).saturating_sub(off);
    }
    let (off, size) = (off as usize, size as usize);
    if off + size > SHADOW_MAX {
        return;
    }
    if let Some(data) = read_bytes(ptr, size) {
        st(|s| {
            let b = s.shadow.entry(vb).or_default();
            if b.len() < off + size {
                b.resize(off + size, 0);
            }
            b[off..off + size].copy_from_slice(&data);
        });
    }
}

// ------------------------------------------------------------------ fonts -----------------

fn text_logging() -> bool {
    matches!(mode(), Mode::Ui | Mode::All | Mode::Text)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(wide: bool, this: u32, sprite: u32, s: u32, count: i32, rect: u32, fmt: u32, color: u32) -> i32 {
    let slot = if wide { 15 } else { 14 };
    let o = orig_of(this, slot);
    if o == 0 {
        return 0;
    }
    let log = text_logging();
    let name = if wide { "font.DrawTextW" } else { "font.DrawTextA" };
    if log {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let m = ProcMem;
            let text = if wide { dd::wide_str(&m, s, count, 4096) } else { dd::ansi_str(&m, s, count, 4096) };
            let mut rec = base_rec(name);
            rec.put("obj_ptr", Val::Hex(this));
            rec.put("font", font_label(this));
            // Our own return address is not on the stack here; use the frame's.
            rec.put("sprite_ptr", Val::Hex(sprite));
            if sprite != 0 {
                rec.put("sprite", obj_label(sprite, false));
            }
            rec.put(
                "args",
                Val::obj()
                    .with("text", text)
                    .with("count", count)
                    .with("rect", rect_val(rect))
                    .with("fmt", Val::Str(format!("{fmt:#x}")))
                    .with("color", Val::Str(format!("{color:#010x}"))),
            );
            let mut esp = 0u32;
            // SAFETY: reads our own stack pointer.
            unsafe { std::arch::asm!("mov {0}, esp", out(reg) esp, options(nomem, nostack)) };
            if let Some(o) = client_origin(esp) {
                rec.put("org", o);
            }
            d3d_emit(rec);
        }));
    }
    // SAFETY: original ID3DXFont::DrawTextA/W.
    let f: extern "system" fn(u32, u32, u32, i32, u32, u32, u32) -> i32 = unsafe { std::mem::transmute(o as usize) };
    let r = f(this, sprite, s, count, rect, fmt, color);
    if log {
        let mut rec = base_rec(&format!("{name}.ret"));
        rec.put("h", r);
        if fmt & DT_CALCRECT != 0 {
            rec.put("rect", rect_val(rect));
        }
        d3d_emit(rec);
    }
    r
}

fn rect_val(p: u32) -> Val {
    let m = ProcMem;
    if p == 0 {
        return Val::Null;
    }
    match (m.i32(p), m.i32(p + 4), m.i32(p + 8), m.i32(p + 12)) {
        (Some(l), Some(t), Some(r), Some(b)) => Val::List(vec![l.into(), t.into(), r.into(), b.into()]),
        _ => Val::Null,
    }
}

extern "system" fn draw_text_a(this: u32, sprite: u32, s: u32, count: i32, rect: u32, fmt: u32, color: u32) -> i32 {
    draw_text(false, this, sprite, s, count, rect, fmt, color)
}

extern "system" fn draw_text_w(this: u32, sprite: u32, s: u32, count: i32, rect: u32, fmt: u32, color: u32) -> i32 {
    draw_text(true, this, sprite, s, count, rect, fmt, color)
}

// ------------------------------------------------------------------ factories -------------

type FnCreateFont = extern "system" fn(u32, i32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> i32;
type FnCreateFontInd = extern "system" fn(u32, u32, u32) -> i32;
type FnCreateSprite = extern "system" fn(u32, u32) -> i32;
type FnCreateRts = extern "system" fn(u32, u32, u32, u32, u32, u32, u32) -> i32;
type FnCreateTexture = extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32) -> i32;
type FnTexFromFileEx = extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> i32;
type FnTexFromMemEx =
    extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> i32;
type FnLoadSurface = extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32) -> i32;

/// Runs a factory through its trampoline; only the outermost factory call is logged
/// (D3DX factories call each other).
fn factory<R>(call: impl FnOnce() -> R) -> (R, bool) {
    let depth = FACTORY_DEPTH.with(|d| {
        let v = d.get();
        d.set(v + 1);
        v
    });
    let r = call();
    FACTORY_DEPTH.with(|d| d.set(depth));
    (r, depth == 0)
}

fn caller_rec(name: &str) -> Val {
    let mut rec = base_rec(name);
    let mut esp = 0u32;
    // SAFETY: reads our own stack pointer.
    unsafe { std::arch::asm!("mov {0}, esp", out(reg) esp, options(nomem, nostack)) };
    if let Some(o) = client_origin(esp) {
        rec.put("org", o);
    }
    rec
}

fn out_obj(pp: u32) -> u32 {
    ProcMem.u32(pp).unwrap_or(0)
}

fn log_font(name: &str, hr: i32, pp: u32, label: String) {
    let font = if hr >= 0 { out_obj(pp) } else { 0 };
    if font != 0 {
        st(|s| s.labels.insert(font, label.clone()));
        let _ = hook_vtable(Iface::Font, font);
    }
    d3d_emit(
        caller_rec(name).with("hr", Val::Str(format!("{hr:#x}"))).with("font", label).with("obj_ptr", Val::Hex(font)),
    );
    flush();
}

#[allow(clippy::too_many_arguments)]
extern "system" fn create_font_a(
    dev: u32,
    h: i32,
    w: u32,
    wt: u32,
    mip: u32,
    it: u32,
    cs: u32,
    op: u32,
    q: u32,
    pf: u32,
    face: u32,
    pp: u32,
) -> i32 {
    // SAFETY: trampoline of D3DXCreateFontA.
    let f: FnCreateFont = unsafe { std::mem::transmute(export_tramp("D3DXCreateFontA")) };
    let (hr, outer) = factory(|| f(dev, h, w, wt, mip, it, cs, op, q, pf, face, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let fc = ProcMem.cstr(face, 64).unwrap_or_default();
            log_font(
                "D3DXCreateFontA",
                hr,
                pp,
                format!("{fc} h{h} w{w} wt{wt} mip{mip} it{it} cs{cs} op{op} q{q} pf{pf}"),
            );
        }));
    }
    hr
}

#[allow(clippy::too_many_arguments)]
extern "system" fn create_font_w(
    dev: u32,
    h: i32,
    w: u32,
    wt: u32,
    mip: u32,
    it: u32,
    cs: u32,
    op: u32,
    q: u32,
    pf: u32,
    face: u32,
    pp: u32,
) -> i32 {
    // SAFETY: trampoline of D3DXCreateFontW.
    let f: FnCreateFont = unsafe { std::mem::transmute(export_tramp("D3DXCreateFontW")) };
    let (hr, outer) = factory(|| f(dev, h, w, wt, mip, it, cs, op, q, pf, face, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let fc = dd::wide_str(&ProcMem, face, -1, 64).unwrap_or_default();
            log_font(
                "D3DXCreateFontW",
                hr,
                pp,
                format!("{fc} h{h} w{w} wt{wt} mip{mip} it{it} cs{cs} op{op} q{q} pf{pf}"),
            );
        }));
    }
    hr
}

fn font_ind(wide: bool, dev: u32, desc: u32, pp: u32) -> i32 {
    let name = if wide { "D3DXCreateFontIndirectW" } else { "D3DXCreateFontIndirectA" };
    // SAFETY: trampoline of D3DXCreateFontIndirectA/W.
    let f: FnCreateFontInd = unsafe { std::mem::transmute(export_tramp(name)) };
    let (hr, outer) = factory(|| f(dev, desc, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut d = [0u8; 56];
            let label = if wide {
                let mut b = [0u8; 24];
                ProcMem.read(desc, &mut b);
                d[..24].copy_from_slice(&b);
                let fc = dd::wide_str(&ProcMem, desc + 24, -1, 32).unwrap_or_default();
                let l = dd::font_desc_label(&d);
                l.replacen(' ', &format!("{fc} "), 1).trim_start().to_string()
            } else {
                ProcMem.read(desc, &mut d);
                dd::font_desc_label(&d)
            };
            log_font(name, hr, pp, label);
        }));
    }
    hr
}

extern "system" fn create_font_ind_a(dev: u32, desc: u32, pp: u32) -> i32 {
    font_ind(false, dev, desc, pp)
}

extern "system" fn create_font_ind_w(dev: u32, desc: u32, pp: u32) -> i32 {
    font_ind(true, dev, desc, pp)
}

static SPRITES: AtomicU32 = AtomicU32::new(0);

extern "system" fn create_sprite(dev: u32, pp: u32) -> i32 {
    // SAFETY: trampoline of D3DXCreateSprite.
    let f: FnCreateSprite = unsafe { std::mem::transmute(export_tramp("D3DXCreateSprite")) };
    let (hr, outer) = factory(|| f(dev, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let sp = if hr >= 0 { out_obj(pp) } else { 0 };
            let label = format!("sprite#{}", SPRITES.fetch_add(1, Ordering::Relaxed) + 1);
            if sp != 0 {
                st(|s| s.labels.insert(sp, label.clone()));
                let _ = hook_vtable(Iface::Sprite, sp);
            }
            d3d_emit(
                caller_rec("D3DXCreateSprite")
                    .with("hr", Val::Str(format!("{hr:#x}")))
                    .with("sprite", label)
                    .with("obj_ptr", Val::Hex(sp)),
            );
            flush();
        }));
    }
    hr
}

extern "system" fn create_rts(dev: u32, w: u32, h: u32, fmt: u32, depth: u32, dfmt: u32, pp: u32) -> i32 {
    // SAFETY: trampoline of D3DXCreateRenderToSurface.
    let f: FnCreateRts = unsafe { std::mem::transmute(export_tramp("D3DXCreateRenderToSurface")) };
    let (hr, outer) = factory(|| f(dev, w, h, fmt, depth, dfmt, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let o = if hr >= 0 { out_obj(pp) } else { 0 };
            if o != 0 {
                st(|s| s.labels.insert(o, format!("rts {w}x{h} f{fmt} depth{depth} f{dfmt}")));
                let _ = hook_vtable(Iface::RenderToSurface, o);
            }
            d3d_emit(
                caller_rec("D3DXCreateRenderToSurface")
                    .with("hr", Val::Str(format!("{hr:#x}")))
                    .with(
                        "args",
                        Val::obj()
                            .with("w", w)
                            .with("h", h)
                            .with("format", fmt)
                            .with("depth", depth)
                            .with("depth_format", dfmt),
                    )
                    .with("obj_ptr", Val::Hex(o)),
            );
            flush();
        }));
    }
    hr
}

fn log_texture(name: &str, hr: i32, pp: u32, label: Option<String>, args: Val) {
    let t = if hr >= 0 { out_obj(pp) } else { 0 };
    if t != 0
        && let Some(l) = &label
    {
        st(|s| s.labels.insert(t, l.clone()));
    }
    let desc = if t != 0 { resource_desc(t) } else { None };
    d3d_emit(
        caller_rec(name)
            .with("hr", Val::Str(format!("{hr:#x}")))
            .with("args", args)
            .with("result", desc)
            .with("obj_ptr", Val::Hex(t)),
    );
    flush();
}

#[allow(clippy::too_many_arguments)]
extern "system" fn create_texture(dev: u32, w: u32, h: u32, mip: u32, usage: u32, fmt: u32, pool: u32, pp: u32) -> i32 {
    // SAFETY: trampoline of D3DXCreateTexture.
    let f: FnCreateTexture = unsafe { std::mem::transmute(export_tramp("D3DXCreateTexture")) };
    let (hr, outer) = factory(|| f(dev, w, h, mip, usage, fmt, pool, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            log_texture(
                "D3DXCreateTexture",
                hr,
                pp,
                Some(format!("d3dx-tex {w}x{h}")),
                Val::obj()
                    .with("w", w)
                    .with("h", h)
                    .with("mip", mip)
                    .with("usage", Val::Str(format!("{usage:#x}")))
                    .with("format", fmt)
                    .with("pool", pool),
            );
        }));
    }
    hr
}

fn tex_file(wide: bool, a: [u32; 14]) -> i32 {
    let name = if wide { "D3DXCreateTextureFromFileExW" } else { "D3DXCreateTextureFromFileExA" };
    // SAFETY: trampoline of D3DXCreateTextureFromFileExA/W.
    let f: FnTexFromFileEx = unsafe { std::mem::transmute(export_tramp(name)) };
    let (hr, outer) =
        factory(|| f(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], a[10], a[11], a[12], a[13]));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let path =
                if wide { dd::wide_str(&ProcMem, a[1], -1, 260) } else { ProcMem.cstr(a[1], 260) }.unwrap_or_default();
            log_texture(
                name,
                hr,
                a[13],
                Some(format!("file:{path}")),
                Val::obj()
                    .with("path", path.as_str())
                    .with("w", a[2])
                    .with("h", a[3])
                    .with("mip", a[4])
                    .with("usage", Val::Str(format!("{:#x}", a[5])))
                    .with("format", a[6])
                    .with("pool", a[7])
                    .with("filter", Val::Str(format!("{:#x}", a[8])))
                    .with("mip_filter", Val::Str(format!("{:#x}", a[9])))
                    .with("colorkey", Val::Str(format!("{:#x}", a[10]))),
            );
        }));
    }
    hr
}

#[allow(clippy::too_many_arguments)]
extern "system" fn tex_from_file_a(
    a0: u32,
    a1: u32,
    a2: u32,
    a3: u32,
    a4: u32,
    a5: u32,
    a6: u32,
    a7: u32,
    a8: u32,
    a9: u32,
    a10: u32,
    a11: u32,
    a12: u32,
    a13: u32,
) -> i32 {
    tex_file(false, [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13])
}

#[allow(clippy::too_many_arguments)]
extern "system" fn tex_from_file_w(
    a0: u32,
    a1: u32,
    a2: u32,
    a3: u32,
    a4: u32,
    a5: u32,
    a6: u32,
    a7: u32,
    a8: u32,
    a9: u32,
    a10: u32,
    a11: u32,
    a12: u32,
    a13: u32,
) -> i32 {
    tex_file(true, [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13])
}

#[allow(clippy::too_many_arguments)]
extern "system" fn tex_from_mem(
    dev: u32,
    data: u32,
    len: u32,
    w: u32,
    h: u32,
    mip: u32,
    usage: u32,
    fmt: u32,
    pool: u32,
    filter: u32,
    mfilter: u32,
    ck: u32,
    info: u32,
    pal: u32,
    pp: u32,
) -> i32 {
    // SAFETY: trampoline of D3DXCreateTextureFromFileInMemoryEx.
    let f: FnTexFromMemEx = unsafe { std::mem::transmute(export_tramp("D3DXCreateTextureFromFileInMemoryEx")) };
    let (hr, outer) = factory(|| f(dev, data, len, w, h, mip, usage, fmt, pool, filter, mfilter, ck, info, pal, pp));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let hash = read_bytes(data, len.min(16 << 20) as usize)
                .map(|b| format!("{:08x}", dd::fnv1a(&b)))
                .unwrap_or_default();
            log_texture(
                "D3DXCreateTextureFromFileInMemoryEx",
                hr,
                pp,
                Some(format!("mem:{len}B#{hash}")),
                Val::obj()
                    .with("len", len)
                    .with("hash", hash.as_str())
                    .with("w", w)
                    .with("h", h)
                    .with("format", fmt)
                    .with("pool", pool),
            );
        }));
    }
    hr
}

#[allow(clippy::too_many_arguments)]
extern "system" fn load_surface_a(
    surf: u32,
    pal: u32,
    rect: u32,
    path: u32,
    src: u32,
    filter: u32,
    ck: u32,
    info: u32,
) -> i32 {
    // SAFETY: trampoline of D3DXLoadSurfaceFromFileA.
    let f: FnLoadSurface = unsafe { std::mem::transmute(export_tramp("D3DXLoadSurfaceFromFileA")) };
    let (hr, outer) = factory(|| f(surf, pal, rect, path, src, filter, ck, info));
    if outer {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let p = ProcMem.cstr(path, 260).unwrap_or_default();
            st(|s| s.labels.insert(surf, format!("file:{p}")));
            d3d_emit(
                caller_rec("D3DXLoadSurfaceFromFileA")
                    .with("hr", Val::Str(format!("{hr:#x}")))
                    .with(
                        "args",
                        Val::obj()
                            .with("path", p.as_str())
                            .with("dst_rect", rect_val(rect))
                            .with("src_rect", rect_val(src))
                            .with("filter", Val::Str(format!("{filter:#x}")))
                            .with("colorkey", Val::Str(format!("{ck:#x}"))),
                    )
                    .with("surface", resource_desc(surf))
                    .with("obj_ptr", Val::Hex(surf)),
            );
            flush();
        }));
    }
    hr
}
