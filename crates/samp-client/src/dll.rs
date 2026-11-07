//! Windows entry point: DllMain, the init thread, the per-frame hooks and the control-channel servicing.
//! Start-up follows samp's order: version probe and the early patch block at attach (the launcher keeps the
//! main thread suspended until LoadLibrary returns), then wait for `gGameState == 7`, apply the main block and
//! the per-frame hooks, then `gGameState := 8` plus the three menu flags.
//! Spec: docs/re/subsystems/gta-patches.md, "When patches are applied" and "What the Rust client must do at startup".
//!
//! Layout: this file holds the entry point, the attach-time work, the hooks and the per-frame tick; `init` the init
//! thread (one function per install step); `service` the control-channel commands.

mod init;
mod service;

pub(crate) use service::readable;

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HINSTANCE, TRUE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

use crate::client::Client;
use crate::control;
use crate::game;
use crate::log;
use crate::patch::{Memory, Patcher, ProcessMemory};
use crate::patches::{self, Phase};

static PATCHER: Mutex<Option<Patcher>> = Mutex::new(None);
static CONTROL: Mutex<Option<control::Server>> = Mutex::new(None);
/// Original target of the `Render2dStuff` call in `Idle` ([`game::IDLE_RENDER_2D_CALL`]).
static ORIGINAL_RENDER_2D: AtomicU32 = AtomicU32::new(0);
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Milliseconds since attach at the last tick (0 = never ticked).
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
/// This DLL's image base (DllMain's module handle; samp keeps its own the same way). Mirrors samp.dll 0x100C5100.
static MODULE_BASE: AtomicU32 = AtomicU32::new(0);

/// This client module's image base.
pub(crate) fn module_base() -> u32 {
    MODULE_BASE.load(Ordering::SeqCst)
}
/// `since_start_ms` of the `/quit` (0 = none).
static QUIT_AT_MS: AtomicU64 = AtomicU64::new(0);
/// `/quit` arrived while the session was locked: Disconnect on the next frame.
static QUIT_PENDING: AtomicBool = AtomicBool::new(false);
static START: Mutex<Option<Instant>> = Mutex::new(None);
/// Connection arguments from the command line (parsed at attach).
static ARGS: std::sync::OnceLock<crate::cmdline::ConnectArgs> = std::sync::OnceLock::new();
/// The network/game session, created on the first frame with a local ped.
static CLIENT: Mutex<Option<Client>> = Mutex::new(None);
static CLIENT_TRIED: AtomicBool = AtomicBool::new(false);
static READY_SEEN: AtomicBool = AtomicBool::new(false);
/// Set once `CWorld::ms_aSectors` has been moved (sectors.rs); remote peds (player indices ≥ 2) are only created
/// after that.
pub(crate) static SECTORS_RELOCATED: AtomicBool = AtomicBool::new(false);
/// Sector relocation and the swap hooks are both in place: remote peds may be created.
pub(crate) static REMOTE_PEDS_READY: AtomicBool = AtomicBool::new(false);

fn since_start_ms() -> u64 {
    let g = START.lock().unwrap_or_else(|e| e.into_inner());
    g.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0)
}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        MODULE_BASE.store(hinst as usize as u32, Ordering::SeqCst);
        // SAFETY: valid module handle passed by the loader.
        unsafe { DisableThreadLibraryCalls(hinst) };
        let _ = catch_unwind(attach);
    }
    TRUE
}

/// The rest of samp's version check for an executable that was not 1.0 US at attach (see [`attach`]).
fn version_error_thread() {
    let mem = ProcessMemory;
    for _ in 0..game::VERSION_POLLS {
        let probe = mem.read_u8(game::VERSION_PROBE_ADDR).unwrap_or(0);
        let give_up = mem.read_u8(game::VERSION_GIVE_UP_FLAG).unwrap_or(0);
        match game::version_poll(probe, give_up) {
            game::VersionPoll::Us | game::VersionPoll::Eu => {
                crate::log!("version probe became {probe:#04x}; the client stays unpatched");
                return;
            }
            game::VersionPoll::Fail => break,
            game::VersionPoll::Wait => std::thread::sleep(std::time::Duration::from_millis(game::VERSION_POLL_MS)),
        }
    }
    crate::log!("version check failed: \"{}\"", game::VERSION_ERROR_CAPTION);
    let text = format!("{}\0", game::VERSION_ERROR_TEXT);
    let caption = format!("{}\0", game::VERSION_ERROR_CAPTION);
    // SAFETY: NUL-terminated strings; plain Win32 calls (MB_ICONEXCLAMATION, no owner), then the process ends.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxA(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION,
        );
        windows_sys::Win32::System::Threading::ExitProcess(1);
    }
}

/// Runs under the loader lock: only cheap work (file open, a few byte writes through kernel32, starting a thread).
fn attach() {
    *START.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("samp-rs.log")))
        .unwrap_or_else(|| "samp-rs.log".into());
    let _ = log::init(&log_path);
    std::panic::set_hook(Box::new(|info| {
        crate::log!("PANIC: {info}");
    }));
    crate::log!("samp-client {} attached; log {}", env!("CARGO_PKG_VERSION"), log_path.display());
    let args = crate::cmdline::parse(std::env::args().skip(1));
    crate::log!("command line: {:?}", args);
    let _ = ARGS.set(args);

    let mut mem = ProcessMemory;
    let probe = mem.read_u8(game::VERSION_PROBE_ADDR).unwrap_or(0);
    if probe != game::VERSION_PROBE_US10 {
        let which = if probe == game::VERSION_PROBE_EU10 { "1.0 EU" } else { "unknown" };
        crate::log!(
            "unsupported gta_sa.exe: byte at {:#x} is {probe:#04x} ({which}); only 1.0 US (0x89) is supported — not patching",
            game::VERSION_PROBE_ADDR
        );
        if probe != game::VERSION_PROBE_EU10 {
            // samp's check runs on its start-up thread: it polls the probe, and an executable that is neither
            // 1.0 US nor EU ends with the "Version Error" box and ExitProcess(1).
            std::thread::spawn(version_error_thread);
        }
        return;
    }
    crate::log!("gta_sa.exe 1.0 US detected (byte {:#x} == 0x89)", game::VERSION_PROBE_ADDR);
    // samp's top-level exception filter and crash dialog (without the upload).
    crate::crash::install();

    let ram_mb = physical_ram_mb();
    let mut patcher = Patcher::new();
    // gGameState := 5 (FRONTEND_LOADING): skip logos and intro movies.
    match patcher.write(&mut mem, "gGameState := 5", game::G_GAME_STATE, &5i32.to_le_bytes(), Some(&[0; 4])) {
        Ok(()) => crate::log!("early: gGameState := 5"),
        Err(e) => crate::log!("early: gGameState := 5 FAILED: {e}"),
    }
    let r = patches::apply_phase(&mut patcher, &mut mem, Phase::Early, ram_mb);
    log_report("early", &r);
    // Not a SA:MP patch: fix the CdStreamSync lost-wakeup race (see cdstream.rs).
    let wait = crate::cdstream::hook::sync_wait as extern "system" fn(_, _) -> u32 as usize as u32;
    let mut call = crate::patch::encode_branch(crate::patch::OP_CALL, crate::cdstream::SYNC_WAIT_CALL, wait).to_vec();
    call.push(crate::patch::OP_NOP);
    match patcher.write(
        &mut mem,
        "CdStreamSync wait",
        crate::cdstream::SYNC_WAIT_CALL,
        &call,
        Some(&crate::cdstream::SYNC_WAIT_CALL_ORIGINAL),
    ) {
        Ok(()) => crate::log!("early: CdStreamSync wait at {:#x} -> {wait:#x}", crate::cdstream::SYNC_WAIT_CALL),
        Err(e) => crate::log!("early: CdStreamSync wait FAILED: {e}"),
    }
    // samp's ShowCursor stub (cursorhook.rs), before GTA creates and activates its window.
    if let Err(e) = crate::cursorhook::install(&mut patcher, &mut mem) {
        crate::log!("early: ShowCursor import FAILED: {e}");
    }
    crate::log!("early: physical RAM {ram_mb} MB -> streaming budget {:#x}", patches::streaming_budget(ram_mb));
    // Serve files from samp.saa like samp (filehook.rs), before the game opens anything.
    if let crate::filehook::Mode::Archive = crate::filehook::init(&mut patcher, &mut mem) {
        let r = patches::apply_phase(&mut patcher, &mut mem, Phase::EarlyArchive, ram_mb);
        log_report("early", &r);
    }
    *PATCHER.lock().unwrap_or_else(|e| e.into_inner()) = Some(patcher);

    if let Err(e) = std::thread::Builder::new().name("samp-init".into()).spawn(init::init_thread) {
        crate::log!("cannot start init thread: {e}");
    }
}

fn log_report(phase: &str, r: &patches::Report) {
    for (addr, what) in &r.applied {
        crate::log!("{phase}: applied {addr:#x} {what}");
    }
    for (addr, what, err) in &r.failed {
        crate::log!("{phase}: SKIPPED {addr:#x} {what}: {err}");
    }
}

fn physical_ram_mb() -> u64 {
    // SAFETY: plain Win32 query into a correctly sized struct.
    unsafe {
        let mut st: MEMORYSTATUSEX = std::mem::zeroed();
        st.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut st) == 0 {
            return 0;
        }
        st.ullTotalPhys / (1024 * 1024)
    }
}

/// Runs `f` with the process patcher (patches applied later, e.g. at InitGame).
pub(crate) fn with_patcher<R>(f: impl FnOnce(&mut Patcher, &mut ProcessMemory) -> R) -> R {
    let mut g = PATCHER.lock().unwrap_or_else(|e| e.into_inner());
    let p = g.get_or_insert_with(Patcher::new);
    f(p, &mut ProcessMemory)
}

/// ANSI path of our generated main.scm (NUL-terminated).
static MAIN_SCM_PATH: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// Writes our SCM (scm.rs) to the temp directory; returns its path.
fn write_main_scm() -> std::io::Result<String> {
    let path = std::env::temp_dir().join("samp-rs-main.scm");
    std::fs::write(&path, crate::scm::build_main_scm(game::INITIAL_PLAYER_POS))?;
    let s = path.to_string_lossy().into_owned();
    let mut bytes = s.clone().into_bytes();
    bytes.push(0);
    let _ = MAIN_SCM_PATH.set(bytes);
    Ok(s)
}

/// Replaces `CFileMgr::OpenFile("main.scm", "rb")` in `CTheScripts::Init` (cdecl). `CFileMgr::OpenFile` is a thin
/// wrapper around the CRT `fopen`, so opening our absolute path through it yields a `FILE*` the caller can read and
/// close as usual.
extern "C" fn open_main_scm_hook(path: *const u8, mode: *const u8) -> *mut c_void {
    // SAFETY: the game's cdecl fopen wrapper / fopen with valid C strings.
    unsafe {
        let open: extern "C" fn(*const u8, *const u8) -> *mut c_void =
            std::mem::transmute(game::CFILEMGR_OPEN_FILE as usize);
        match MAIN_SCM_PATH.get() {
            Some(p) => {
                let f = open(p.as_ptr(), mode);
                crate::log!(
                    "CTheScripts::Init: serving our main.scm ({})",
                    if f.is_null() { "open FAILED" } else { "ok" }
                );
                if f.is_null() { open(path, mode) } else { f }
            }
            None => open(path, mode),
        }
    }
}

/// Replaces `CGame::Shutdown`: our own cleanup, then 2000 ms in 100 ms sleeps, then exit, like samp.
/// Mirrors samp.dll B 0x100A3AB0.
extern "C" fn game_shutdown_hook() {
    let start = Instant::now();
    crate::log!("CGame::Shutdown: ending the process like samp");
    // A `quit` request has already disconnected.
    if !QUIT.load(Ordering::SeqCst)
        && let Ok(mut g) = CLIENT.try_lock()
        && let Some(c) = g.as_mut()
    {
        c.shutdown();
    }
    while start.elapsed() < Duration::from_millis(2000) {
        std::thread::sleep(Duration::from_millis(100));
    }
    crate::log!("exit");
    std::process::exit(0);
}

/// Zeroed stand-in ped for the police scanner.
static SCANNER_DUMMY_PED: AtomicU32 = AtomicU32::new(0);

/// RPC 119 straight from the Present pass (pause-menu map click): sent and flushed now; false if the session is
/// busy or absent.
pub(crate) fn send_click_map(pos: [f32; 3]) -> bool {
    let Ok(mut g) = CLIENT.try_lock() else { return false };
    let Some(c) = g.as_mut() else { return false };
    c.send_click_map(pos);
    true
}

/// The stand-in ped's address (0 before the patches ran).
pub(crate) fn scanner_dummy_ped() -> u32 {
    SCANNER_DUMMY_PED.load(Ordering::Relaxed)
}

/// Replaces `FindPlayerPed(player)` in `CAEPoliceScannerAudioEntity::AddAudioEvent` (cdecl).
extern "C" fn scanner_find_player_ped(_player: i32) -> u32 {
    SCANNER_DUMMY_PED.load(Ordering::Relaxed)
}

/// `CCamera::Process` (thiscall).
const CAMERA_PROCESS: u32 = 0x52B730;
/// The `call CCamera::Process` in `CGame::Process`, hooked like samp.
const CAMERA_PROCESS_CALL: u32 = 0x53C104;
static ORIGINAL_CAMERA_PROCESS: AtomicU32 = AtomicU32::new(0);

/// Replaces `TheCamera.Process()` in `CGame::Process`: samp's camera follow first (attached object), then the
/// original. Mirrors samp.dll B 0x100A2640.
extern "thiscall" fn camera_process_hook(this: u32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut g) = CLIENT.try_lock()
            && let Some(c) = g.as_mut()
        {
            c.camera_process();
        }
    }));
    let orig = ORIGINAL_CAMERA_PROCESS.load(Ordering::Relaxed);
    if orig != 0 {
        // SAFETY: `orig` is CCamera::Process (thiscall, no arguments), read from the call we retargeted.
        let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(orig as usize) };
        f(this);
    }
}

/// Original target of the `CGame::Process` call in `Idle` ([`game::IDLE_GAME_PROCESS_CALL`]).
static ORIGINAL_GAME_PROCESS: AtomicU32 = AtomicU32::new(0);
/// The first-frame ped flags were set from [`game_process_hook`].
pub(crate) static PED_FLAGS_DONE: AtomicBool = AtomicBool::new(false);

/// Replaces the `call CGame::Process` in `Idle` (cdecl, no arguments): the frame timer step, the world-entity
/// pre-step, then the original inside the SEH guard. After the first game frame with a local ped (state 9) it
/// sets the two ped flags the original sets in its first frame, even while the screen is still faded out and
/// `Render2dStuff` (our per-frame tick) is skipped. Mirrors samp.dll B 0x100A2670.
extern "C" fn game_process_hook() {
    if ARGS.get().is_some_and(|a| a.connect) {
        let _ = catch_unwind(frame_timer_step);
    }
    // World-entity pre-step (objects: streaming, attachments, MoveObject; docs/re/subsystems/world-entities.md,
    // "Per-frame object process").
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut g) = CLIENT.try_lock()
            && let Some(c) = g.as_mut()
        {
            c.pre_process();
        }
    }));
    let orig = ORIGINAL_GAME_PROCESS.load(Ordering::Relaxed);
    if orig != 0 {
        // CGame::Process inside the shared SEH guard, like samp's hook (fault scope "s007").
        // SAFETY: `orig` is CGame::Process (cdecl, no arguments, read from the call we retargeted), called through
        // the guard's thiscall shape; the callee ignores ECX.
        let code = unsafe { crate::swap::guarded_thiscall(orig, 0) };
        if code != 0 {
            let at = crate::swap::last_fault_address();
            crate::log!("main: CGame::Process faulted: {code:#x} at {at:#x} ({})", crate::swap::last_fault_details());
            crate::swap::push_fault(crate::swap::FaultSite::Named("s007"), code, at);
        }
    }
    if !PED_FLAGS_DONE.load(Ordering::Relaxed) {
        let _ = catch_unwind(AssertUnwindSafe(first_frame_ped_flags));
    }
    // samp's CGame::Process hook also updates the chat render target (docs/re/subsystems/ui-rendering.md,
    // "Other hooks that matter for UI").
    crate::ui::game_frame();
}

static FRAME_TIMER: Mutex<Option<crate::frametimer::FrameTimer>> = Mutex::new(None);
/// Frame limit (samp's Game object +0x5d): 90 from the Game constructor until the first-frame init sets the
/// sa-mp.cfg `fpslimit` (only when it is in 20..90; otherwise 90 stays), then `/fpslimit`.
static FPS_LIMIT: AtomicU32 = AtomicU32::new(GAME_DEFAULT_FPS_LIMIT);
const GAME_DEFAULT_FPS_LIMIT: u32 = 90;

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

/// samp's step before `CGame::Process` (frametimer.rs): reads the limiter inputs, writes the time step and pad
/// counter, sleeps.
fn frame_timer_step() {
    use crate::frametimer::{self as ft, Limiter};
    let mut g = FRAME_TIMER.lock().unwrap_or_else(|e| e.into_inner());
    let t = g.get_or_insert_with(|| {
        // Like samp's first-frame init: 1 ms timer resolution.
        // SAFETY: plain winmm call.
        unsafe { timeBeginPeriod(1) };
        Default::default()
    });
    if let Some(l) = crate::ui::fps_limit() {
        FPS_LIMIT.store(l, Ordering::Relaxed);
    }
    let mem = ProcessMemory;
    let limiter = Limiter {
        menu_limiter_on: mem.read_u8(ft::MENU_FRAME_LIMITER).unwrap_or(0) != 0,
        fps_limit: FPS_LIMIT.load(Ordering::Relaxed),
        task_gate: limiter_task_gate(),
    };
    let s = t.step(since_start_ms(), limiter);
    if let Some(ts) = s.time_step {
        let _ = ProcessMemory.write(ft::TIME_STEP_NON_CLIPPED, &ts.to_le_bytes());
        let _ = ProcessMemory.write(ft::TIME_STEP, &ts.to_le_bytes());
    }
    if let Some(c) = s.pad_counter {
        // SAFETY: a byte inside CPad::Pads[0] (.bss), main thread.
        unsafe { std::ptr::write_volatile(ft::PAD_COUNTER as usize as *mut u8, c) };
    }
    if s.sleep_ms > 0 {
        // kernel32 `Sleep` like samp: std's sleep may use a high-resolution waitable timer, which wakes later under
        // Wine and makes the frames longer than the original's at the same limit.
        // SAFETY: plain Win32 call.
        unsafe { windows_sys::Win32::System::Threading::Sleep(s.sleep_ms) };
    }
    t.finish(since_start_ms());
}

/// The local ped is on foot and its intelligence +0xC task reports type 0x10C. Mirrors samp.dll 0x100ACD10.
fn limiter_task_gate() -> bool {
    let m = ProcessMemory;
    let ped = m.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0);
    if ped == 0 || m.read_u32(ped + game::off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0 {
        return false;
    }
    let intel = m.read_u32(ped + game::off::PED_INTELLIGENCE).unwrap_or(0);
    let task = if intel != 0 { m.read_u32(intel + 0xC).unwrap_or(0) } else { 0 };
    let vtbl = if task != 0 { m.read_u32(task).unwrap_or(0) } else { 0 };
    // samp's sanity range for the vtable.
    if !(0x80_0000..=0x90_0000).contains(&vtbl) {
        return false;
    }
    let Ok(get_type) = m.read_u32(vtbl + 0x10) else { return false };
    // SAFETY: the task's own GetTaskType virtual (thiscall), main thread.
    let kind = unsafe {
        let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(get_type as usize);
        f(task)
    };
    kind == crate::frametimer::LIMITER_TASK_TYPE
}

/// Sets the two ped flags samp sets when it first creates its local-ped object, which happens on demand in the
/// local `CPlayerPed::ProcessControl` hook of frame 1 (after the scripts, before the frame's object streaming;
/// docs/phase3/close-notes.md). Called from there (swap.rs), with the post-`CGame::Process` call as the fallback.
/// Mirrors samp.dll 0x100ABAB0.
pub(crate) fn first_frame_ped_flags() {
    if !ARGS.get().is_some_and(|a| a.connect) {
        return;
    }
    let mem = ProcessMemory;
    if mem.read_i32(game::G_GAME_STATE).ok() != Some(9) {
        return;
    }
    let ped = mem.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0);
    let Some(h) = game::ped_handle(&mem, ped).ok().flatten() else { return };
    use crate::script::{Arg, Runner, op};
    let mut r = Runner::new();
    r.run(op::SET_CHAR_DROPS_WEAPONS_WHEN_DEAD, &[Arg::Int(h), Arg::Int(1)]);
    r.run(op::SET_CHAR_SUFFERS_CRITICAL_HITS, &[Arg::Int(h), Arg::Int(0)]);
    PED_FLAGS_DONE.store(true, Ordering::SeqCst);
    crate::log!("first game frame with a local ped: ped flags set (frame tick count {})", TICKS.load(Ordering::SeqCst));
}

/// Replaces the `call Render2dStuff` in `Idle` (cdecl, no arguments). samp's order: pre-step (netgame tick, UI
/// work), `Render2dStuff`, textdraws, input-restore countdown, so the netgame's HUD switches apply to the same
/// frame (docs/re/subsystems/ui-rendering.md, "Per-frame game entry"; docs/phase3/parity-notes.md).
/// Mirrors samp.dll 0x100C5030.
extern "C" fn frame_hook() {
    let _ = catch_unwind(AssertUnwindSafe(tick));
    let orig = ORIGINAL_RENDER_2D.load(Ordering::Relaxed);
    if orig != 0 {
        // SAFETY: `orig` is the game's Render2dStuff (cdecl, no arguments), read from the call we retargeted.
        let f: extern "C" fn() = unsafe { std::mem::transmute(orig as usize) };
        f();
    }
    crate::ui::frame_after_2d();
}

struct TickStats {
    window_start: Instant,
    window_ticks: u64,
}

static TICK_STATS: Mutex<Option<TickStats>> = Mutex::new(None);

fn tick() {
    let n = TICKS.fetch_add(1, Ordering::SeqCst) + 1;
    if n.is_multiple_of(64) {
        crate::crash::note(crate::crash::crumb::TICK, 0, n as u32);
    }
    LAST_TICK_MS.store(since_start_ms().max(1), Ordering::SeqCst);
    if n == 1 {
        crate::log!("first frame tick");
        // RsGlobal.frameLimit := 200 at the first frame, like samp.
        let _ = ProcessMemory.write(game::RS_GLOBAL_FRAME_LIMIT, &200u32.to_le_bytes());
    }
    {
        let mut g = TICK_STATS.lock().unwrap_or_else(|e| e.into_inner());
        let st = g.get_or_insert_with(|| TickStats { window_start: Instant::now(), window_ticks: 0 });
        st.window_ticks += 1;
        let el = st.window_start.elapsed();
        if el >= Duration::from_secs(1) {
            let snap = game::snapshot(&ProcessMemory).ok();
            let pos = snap
                .and_then(|s| s.player)
                .map(|p| format!("{:.1},{:.1},{:.1}", p.pos[0], p.pos[1], p.pos[2]))
                .unwrap_or_else(|| "none".into());
            crate::log!("tick {n} ({:.1} fps) player pos {pos}", st.window_ticks as f32 / el.as_secs_f32());
            st.window_start = Instant::now();
            st.window_ticks = 0;
        }
    }
    client_tick();
    // samp's per-frame step after the netgame tick: radio calls while a stream plays, the mission-audio poll
    // (audio.rs).
    if ARGS.get().is_some_and(|a| a.connect) {
        crate::audio::frame_tick();
    }
    // In-game UI: first-frame setup, input restore, queued UI work.
    crate::ui::frame_entry(false);
    quit_tick();
    service::service_control("tick");
}

/// Starts the session once the world is up, then runs it every frame.
fn client_tick() {
    let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        if CLIENT_TRIED.load(Ordering::SeqCst) {
            return;
        }
        let ready = game::snapshot(&ProcessMemory).ok().is_some_and(|s| s.game_state == 9 && s.player.is_some());
        if !ready {
            return;
        }
        // samp's Render2dStuff hook does its one-time init on its first call and creates the netgame on the
        // second, so we wait one frame too (docs/phase3/m2-notes.md).
        if !READY_SEEN.swap(true, Ordering::SeqCst) {
            return;
        }
        CLIENT_TRIED.store(true, Ordering::SeqCst);
        let Some(args) = ARGS.get() else { return };
        if !args.connect {
            crate::log!("client: no -c on the command line; not connecting");
            return;
        }
        match Client::start(args) {
            Ok(c) => *g = Some(c),
            Err(e) => {
                crate::log!("client: cannot start: {e}");
                return;
            }
        }
    }
    if let Some(c) = g.as_mut() {
        c.tick();
    }
}

/// Whether the `Render2dStuff` call in `Idle` still targets our frame hook.
fn frame_hook_installed() -> bool {
    let hook = frame_hook as extern "C" fn() as usize as u32;
    let m = ProcessMemory;
    m.read_u8(game::IDLE_RENDER_2D_CALL).ok() == Some(crate::patch::OP_CALL)
        && m.read_i32(game::IDLE_RENDER_2D_CALL + 1)
            .map(|r| crate::patch::rel32_target(game::IDLE_RENDER_2D_CALL, 5, r) == hook)
            .unwrap_or(false)
}

pub(crate) fn request_quit() {
    if QUIT.swap(true, Ordering::SeqCst) {
        return;
    }
    // samp's quit: Disconnect(500) in netgame state 5, then the quit flag and its GetTickCount; the frame routine
    // keeps running the game and, once more than 1000 ms passed, deletes the netgame and calls ExitProcess(0)
    // ([`quit_tick`]).
    QUIT_AT_MS.store(since_start_ms().max(1), Ordering::SeqCst);
    crate::log!("quit requested");
    match CLIENT.try_lock() {
        Ok(mut g) => {
            if let Some(c) = g.as_mut() {
                c.begin_quit();
            }
        }
        // Busy (control thread): the next frame does it.
        Err(_) => QUIT_PENDING.store(true, Ordering::SeqCst),
    }
    // Without frame ticks (loading, GTA's menu) the frame routine never ends the process: let
    // GTA shut down, then terminate.
    let _ = std::thread::Builder::new().name("samp-quit".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(4));
        crate::log!("no quit from the frame routine 4 s after /quit; setting RsGlobal.quit");
        let _ = ProcessMemory.write(game::RS_GLOBAL_QUIT, &1u32.to_le_bytes());
        std::thread::sleep(Duration::from_secs(8));
        crate::log!("game still running after quit; terminating the process");
        // SAFETY: terminating our own process.
        unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    });
}

/// The quit step of samp's frame routine (after the pre-step): once more than 1000 ms passed since `/quit`
/// (`< 1001` returns), the netgame is deleted and the process ends.
fn quit_tick() {
    let at = QUIT_AT_MS.load(Ordering::SeqCst);
    if at == 0 {
        return;
    }
    if QUIT_PENDING.swap(false, Ordering::SeqCst)
        && let Ok(mut g) = CLIENT.lock()
        && let Some(c) = g.as_mut()
    {
        c.begin_quit();
    }
    if since_start_ms().saturating_sub(at) < 1001 {
        return;
    }
    if let Some(mut c) = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).take() {
        c.quit_teardown();
    }
    crate::log!("exit");
    // SAFETY: ending our own process like samp.
    unsafe { windows_sys::Win32::System::Threading::ExitProcess(0) };
}
