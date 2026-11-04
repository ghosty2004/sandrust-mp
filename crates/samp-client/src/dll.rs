//! Windows entry point: DllMain, the init thread, the per-frame hook and the
//! control-channel servicing.
//!
//! Start-up order follows gta-patches.md §2 (and "What the Rust client must
//! do at startup"): version probe and the early block at attach (the launcher
//! keeps the main thread suspended until LoadLibrary returns), then wait for
//! `gGameState == 7`, apply the main block and the per-frame hook, then
//! `gGameState := 8` plus the three menu flags.

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
use crate::control::{self, Command};
use crate::game;
use crate::log;
use crate::patch::{Memory, Patcher, ProcessMemory};
use crate::patches::{self, Phase};

static PATCHER: Mutex<Option<Patcher>> = Mutex::new(None);
static CONTROL: Mutex<Option<control::Server>> = Mutex::new(None);
/// Original target of the call at 0x53EB12 (`Render2dStuff`).
static ORIGINAL_RENDER_2D: AtomicU32 = AtomicU32::new(0);
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Milliseconds since attach at the last tick (0 = never ticked).
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
static START: Mutex<Option<Instant>> = Mutex::new(None);
/// Connection arguments from the command line (parsed at attach).
static ARGS: std::sync::OnceLock<crate::cmdline::ConnectArgs> = std::sync::OnceLock::new();
/// The network/game session, created on the first frame with a local ped.
static CLIENT: Mutex<Option<Client>> = Mutex::new(None);
static CLIENT_TRIED: AtomicBool = AtomicBool::new(false);
/// Set once `CWorld::ms_aSectors` has been moved (sectors.rs); remote peds
/// (player indices ≥ 2) are only created after that.
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
        // SAFETY: valid module handle passed by the loader.
        unsafe { DisableThreadLibraryCalls(hinst) };
        let _ = catch_unwind(attach);
    }
    TRUE
}

/// Runs under the loader lock: only cheap work (file open, a few byte writes
/// through kernel32, starting a thread).
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
        return;
    }
    crate::log!("gta_sa.exe 1.0 US detected (byte {:#x} == 0x89)", game::VERSION_PROBE_ADDR);

    let ram_mb = physical_ram_mb();
    let mut patcher = Patcher::new();
    // CSV row 120 — gGameState := 5 (FRONTEND_LOADING): skip logos and intro movies.
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
    crate::log!("early: physical RAM {ram_mb} MB -> streaming budget {:#x}", patches::streaming_budget(ram_mb));
    *PATCHER.lock().unwrap_or_else(|e| e.into_inner()) = Some(patcher);

    if let Err(e) = std::thread::Builder::new().name("samp-init".into()).spawn(init_thread) {
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

fn init_thread() {
    let mem = ProcessMemory;
    let t0 = Instant::now();
    let mut last_state = -1;
    let mut last_report = Instant::now();
    // Spec §2 step 3: wait for FRONTEND_IDLE.
    loop {
        let s = mem.read_i32(game::G_GAME_STATE).unwrap_or(-1);
        if s != last_state {
            crate::log!("gGameState = {s} ({}) after {:.2}s", game::game_state_name(s), t0.elapsed().as_secs_f32());
            last_state = s;
        }
        if s == 7 {
            break;
        }
        if s > 7 {
            crate::log!("game passed state 7 without us seeing it; continuing anyway");
            break;
        }
        if last_report.elapsed() > Duration::from_secs(10) {
            crate::log!("still waiting for gGameState 7 (now {s})");
            last_report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut mem = ProcessMemory;
    {
        let mut g = PATCHER.lock().unwrap_or_else(|e| e.into_inner());
        let patcher = g.get_or_insert_with(Patcher::new);
        // Sector grid first (gta-patches.md §2 step 3: 0x100aaac0 runs before
        // everything else in init-main); remote peds depend on it.
        let buffer: &'static mut [u8] = Box::leak(vec![0u8; crate::sectors::BUFFER_BYTES as usize].into_boxed_slice());
        match crate::sectors::relocate(patcher, &mut mem, buffer.as_mut_ptr() as u32) {
            Ok(n) => {
                SECTORS_RELOCATED.store(true, Ordering::SeqCst);
                crate::log!("main: sector grid relocated to {:#x} ({n} references), original zeroed", buffer.as_ptr() as u32);
            }
            Err(e) => crate::log!("main: sector relocation FAILED, remote players disabled: {e}"),
        }
        let r = patches::apply_phase(patcher, &mut mem, Phase::Main, 0);
        log_report("main", &r);
        // Remote-player context switch (CSV rows 19 and 21; remote-control-onfoot.md §2.2).
        match crate::swap::install(patcher, &mut mem) {
            Ok(()) if SECTORS_RELOCATED.load(Ordering::SeqCst) => {
                REMOTE_PEDS_READY.store(true, Ordering::SeqCst);
                crate::log!("main: CPlayerPed::ProcessControl / CTaskSimpleUseGun::SetPedPosition hooks installed");
            }
            Ok(()) => crate::log!("main: swap hooks installed, but remote peds stay disabled (no sector relocation)"),
            Err(e) => crate::log!("main: swap hooks FAILED, remote players disabled: {e}"),
        }
        // CSV row 6 — retarget the Render2dStuff call in Idle to our per-frame entry.
        let hook = frame_hook as extern "C" fn() as usize as u32;
        match patcher.retarget_branch(
            &mut mem,
            "Idle -> frame hook",
            game::IDLE_RENDER_2D_CALL,
            hook,
            Some(game::RENDER_2D_STUFF),
        ) {
            Ok(old) => {
                ORIGINAL_RENDER_2D.store(old, Ordering::SeqCst);
                crate::log!(
                    "main: per-frame hook at {:#x} -> {hook:#x} (original {old:#x})",
                    game::IDLE_RENDER_2D_CALL
                );
            }
            Err(e) => crate::log!("main: per-frame hook FAILED: {e}"),
        }
        // Our own replacement for the story script (scm.rs), served instead of
        // data\script\main.scm through the open call in CTheScripts::Init.
        match write_main_scm() {
            Ok(path) => {
                let hook = open_main_scm_hook as extern "C" fn(*const u8, *const u8) -> *mut c_void as usize as u32;
                match patcher.retarget_branch(
                    &mut mem,
                    "CTheScripts::Init main.scm open",
                    game::MAIN_SCM_OPEN_CALL,
                    hook,
                    Some(game::CFILEMGR_OPEN_FILE),
                ) {
                    Ok(_) => {
                        crate::log!("main: main.scm open at {:#x} -> our script {}", game::MAIN_SCM_OPEN_CALL, path)
                    }
                    Err(e) => crate::log!("main: main.scm redirect FAILED: {e}"),
                }
            }
            Err(e) => crate::log!("main: cannot write our main.scm: {e}; the stock story script will run"),
        }
    }

    let ctl_port = control::port_from_env();
    match control::Server::bind(ctl_port) {
        Ok(s) => {
            crate::log!("control: listening on 127.0.0.1:{ctl_port}");
            *CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
        }
        Err(e) => crate::log!("control: bind 127.0.0.1:{ctl_port} failed: {e}"),
    }

    // CSV rows 377/378 — leave the frontend: LOADING_STARTED plus menu flags
    // (m_bMainMenuSwitch := 1, m_bMenuActive := 0, m_bActivateMenuNextFrame := 0).
    let mm = game::FRONTEND_MENU_MANAGER;
    let writes: [(u32, &[u8], &str); 4] = [
        (mm + 0xE9, &[1], "FrontEndMenuManager.m_bMainMenuSwitch := 1"),
        (mm + 0x5C, &[0], "FrontEndMenuManager.m_bMenuActive := 0"),
        (mm + 0x33, &[0], "FrontEndMenuManager.m_bActivateMenuNextFrame := 0"),
        (game::G_GAME_STATE, &8i32.to_le_bytes(), "gGameState := 8"),
    ];
    for (addr, bytes, what) in writes {
        match mem.write(addr, bytes) {
            Ok(()) => crate::log!("main: {what}"),
            Err(e) => crate::log!("main: {what} FAILED: {e}"),
        }
    }
    crate::log!("init done after {:.2}s", t0.elapsed().as_secs_f32());

    // Service the control channel from here while the game is not ticking
    // (loading, paused, frontend), so tests can always query the state.
    let mut last_state = 8;
    let mut since_state9: Option<Instant> = None;
    let mut hook_checked = false;
    while !QUIT.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
        let s = mem.read_i32(game::G_GAME_STATE).unwrap_or(-1);
        if s != last_state {
            crate::log!("gGameState = {s} ({}) after {:.2}s", game::game_state_name(s), t0.elapsed().as_secs_f32());
            last_state = s;
        }
        if s == 9 && since_state9.is_none() {
            since_state9 = Some(Instant::now());
        }
        if let Some(t9) = since_state9
            && !hook_checked
            && TICKS.load(Ordering::SeqCst) == 0
            && t9.elapsed() > Duration::from_secs(10)
        {
            hook_checked = true;
            let mut b = [0u8; 5];
            let _ = mem.read(game::IDLE_RENDER_2D_CALL, &mut b);
            crate::log!(
                "no frame tick 10 s after IDLE; call site {:#x} = {} (hook {})",
                game::IDLE_RENDER_2D_CALL,
                crate::patch::hex(&b),
                if frame_hook_installed() { "installed" } else { "ABSENT" }
            );
            crate::diag::dump_threads("no frame tick");
        }
        let last = LAST_TICK_MS.load(Ordering::SeqCst);
        if last == 0 || since_start_ms().saturating_sub(last) > 1000 {
            service_control("bg");
        }
    }
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

/// Replaces `CFileMgr::OpenFile("main.scm", "rb")` in `CTheScripts::Init`
/// (cdecl). `CFileMgr::OpenFile` is a thin wrapper around the CRT `fopen`, so
/// opening our absolute path with the game's own `fopen` yields a `FILE*` the
/// caller can read and close as usual.
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

/// Replaces the `call Render2dStuff` in `Idle` (cdecl, no arguments).
extern "C" fn frame_hook() {
    let orig = ORIGINAL_RENDER_2D.load(Ordering::Relaxed);
    if orig != 0 {
        // SAFETY: `orig` is the game's Render2dStuff (void __cdecl(void)),
        // read from the call we retargeted.
        let f: extern "C" fn() = unsafe { std::mem::transmute(orig as usize) };
        f();
    }
    let _ = catch_unwind(AssertUnwindSafe(tick));
}

struct TickStats {
    window_start: Instant,
    window_ticks: u64,
}

static TICK_STATS: Mutex<Option<TickStats>> = Mutex::new(None);

fn tick() {
    let n = TICKS.fetch_add(1, Ordering::SeqCst) + 1;
    LAST_TICK_MS.store(since_start_ms().max(1), Ordering::SeqCst);
    if n == 1 {
        crate::log!("first frame tick");
        // CSV row 357 — RsGlobal.frameLimit := 200 at the first frame.
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
    // In-game UI (M4): first-frame setup, input restore, queued UI work.
    crate::ui::frame_entry(false);
    service_control("tick");
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

fn service_control(_from: &str) {
    let mut g = match CONTROL.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(server) = g.as_mut() else { return };
    let handled = server.poll(|cmd| match cmd {
        Command::Ping => "pong".into(),
        Command::State => match game::snapshot(&ProcessMemory) {
            Ok(s) => format!(
                "{} frame_hook={}",
                game::format_state(&s, TICKS.load(Ordering::SeqCst)),
                if frame_hook_installed() { "installed" } else { "absent" }
            ),
            Err(e) => format!("error {e}"),
        },
        Command::Peek { addr, len } => {
            if !readable(*addr, *len) {
                format!("error {addr:#x}+{len} is not readable")
            } else {
                let mut buf = vec![0u8; *len];
                match ProcessMemory.read(*addr, &mut buf) {
                    Ok(()) => format!("peek {addr:#x} {}", crate::patch::hex(&buf)),
                    Err(e) => format!("error {e}"),
                }
            }
        }
        Command::Net => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.status_line()).unwrap_or_else(|| "net phase=none".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Remote => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.remote_lines()).unwrap_or_else(|| "remote count=0 streamed=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Spawn => match CLIENT.try_lock() {
            Ok(mut g) => g.as_mut().map(|c| c.request_debug_spawn()).unwrap_or_else(|| "error no session".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Chat(n) => crate::ui::control_chat(*n),
        Command::Input => crate::ui::control_input(),
        Command::Say(t) => crate::ui::control_say(t),
        Command::Quit => "bye".into(),
        Command::Unknown(c) => format!("error unknown command {c:?}"),
    });
    drop(g);
    if handled.contains(&Command::Quit) {
        request_quit();
    }
}

/// True if every page of `addr..addr+len` is committed and readable.
pub(crate) fn readable(addr: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    let end = addr as usize + len;
    let mut p = addr as usize;
    while p < end {
        // SAFETY: VirtualQuery only inspects the address space.
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let n = unsafe { VirtualQuery(p as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 || mbi.State != MEM_COMMIT || mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
            return false;
        }
        p = mbi.BaseAddress as usize + mbi.RegionSize;
    }
    true
}

/// Whether the call at 0x53EB12 still targets our frame hook.
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
    crate::log!("quit requested: setting RsGlobal.quit");
    if let Ok(mut g) = CLIENT.try_lock()
        && let Some(c) = g.as_mut()
    {
        c.shutdown();
    }
    let _ = ProcessMemory.write(game::RS_GLOBAL_QUIT, &1u32.to_le_bytes());
    // If the game does not shut down by itself, end the process.
    let _ = std::thread::Builder::new().name("samp-quit".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(8));
        crate::log!("game still running 8s after quit; terminating the process");
        // SAFETY: terminating our own process.
        unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    });
}
