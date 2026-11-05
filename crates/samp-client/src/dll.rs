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
/// This DLL's image base (DllMain's module handle; samp keeps its own the same way, A 0x100c5100).
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
        MODULE_BASE.store(hinst as usize as u32, Ordering::SeqCst);
        // SAFETY: valid module handle passed by the loader.
        unsafe { DisableThreadLibraryCalls(hinst) };
        let _ = catch_unwind(attach);
    }
    TRUE
}

/// Runs under the loader lock: only cheap work (file open, a few byte writes
/// through kernel32, starting a thread).
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
            // samp's check (B 0x100A08C0 → 0x100AA720) runs on its start-up thread: it polls the probe, and an
            // executable that is neither 1.0 US nor EU ends with the "Version Error" box and ExitProcess(1).
            std::thread::spawn(version_error_thread);
        }
        return;
    }
    crate::log!("gta_sa.exe 1.0 US detected (byte {:#x} == 0x89)", game::VERSION_PROBE_ADDR);
    // samp's top-level exception filter and crash dialog (FEAT-12; no upload).
    crate::crash::install();

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
    // SAA-2: serve files from samp.saa like samp (filehook.rs), before the game opens anything.
    if let crate::filehook::Mode::Archive = crate::filehook::init(&mut patcher, &mut mem) {
        let r = patches::apply_phase(&mut patcher, &mut mem, Phase::EarlyArchive, ram_mb);
        log_report("early", &r);
    }
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
    // samp imports d3dx9_25.dll (loaded with samp.dll, before the game's first Present) and
    // registers its fonts at attach (UR §3.1, A 0x100c5100): done first thing here, outside
    // the loader lock.
    crate::ui::preload();
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
            // Rare long stalls before state 7 (seen ~50 s): record where the threads are.
            if t0.elapsed() < Duration::from_secs(35) {
                crate::diag::dump_threads("slow start");
            }
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
                crate::log!(
                    "main: sector grid relocated to {:#x} ({n} references), original zeroed",
                    buffer.as_ptr() as u32
                );
            }
            Err(e) => crate::log!("main: sector relocation FAILED, remote players disabled: {e}"),
        }
        // Enlarged model-info stores (gta-patches.md §4.2), needed by samp.saa's IDE files.
        for store in crate::stores::STORES {
            let buf: &'static mut [u8] = Box::leak(crate::stores::new_entries(store).into_boxed_slice());
            match crate::stores::relocate(patcher, &mut mem, store, buf.as_ptr() as u32) {
                Ok(()) => crate::log!(
                    "main: {} moved to {:#x} ({} entries, {} references)",
                    store.name,
                    buf.as_ptr() as u32,
                    store.entries,
                    store.refs.len()
                ),
                Err(e) => crate::log!("main: {} relocation FAILED: {e}", store.name),
            }
        }
        let r = patches::apply_phase(patcher, &mut mem, Phase::Main, 0);
        log_report("main", &r);
        // CSV row 43 — samp's CJ-walk gate over the move-anim-group store NOPed by row 187.
        match crate::pedhooks::install(patcher, &mut mem) {
            Ok(()) => crate::log!("main: ProcessAnimGroups CJ-walk gate at {:#x}", crate::pedhooks::STORE_SITE),
            Err(e) => crate::log!("main: ProcessAnimGroups CJ-walk gate FAILED: {e}"),
        }
        // Remote-player context switch (CSV rows 19 and 21; remote-control-onfoot.md §2.2).
        match crate::swap::install(patcher, &mut mem) {
            Ok(()) if SECTORS_RELOCATED.load(Ordering::SeqCst) => {
                REMOTE_PEDS_READY.store(true, Ordering::SeqCst);
                crate::log!("main: CPlayerPed::ProcessControl / CTaskSimpleUseGun::SetPedPosition hooks installed");
            }
            Ok(()) => crate::log!("main: swap hooks installed, but remote peds stay disabled (no sector relocation)"),
            Err(e) => crate::log!("main: swap hooks FAILED, remote players disabled: {e}"),
        }
        // Combat (M7): the init-main rows of the damage / projectile / weapon-render hooks.
        let combat = crate::combathooks::install_main(patcher, &mut mem);
        let failed: Vec<String> =
            combat.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: combat rows: {} applied, failed: {failed:?}", combat.len() - failed.len());
        // HUD scale fix around the radar (rows 7/368/369), Idle Sleep(1) (row 16), no cheats (row 50).
        let hooks = crate::mainhooks::install(patcher, &mut mem);
        let failed: Vec<String> =
            hooks.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: main hooks: {} installed, failed: {failed:?}", hooks.len() - failed.len());
        // CSV rows 2/3 — samp's custom script opcodes 0x09C6 / 0x09D3 (used by samp.saa's scripts).
        match crate::scmops::install(patcher, &mut mem) {
            Ok(()) => crate::log!("main: script opcodes 0x09C6 / 0x09D3 installed"),
            Err(e) => crate::log!("main: script opcodes 0x09C6 / 0x09D3 FAILED: {e}"),
        }
        // CSV row 55 — the police scanner's FindPlayerPed call gets a zeroed stand-in ped
        // (samp: a 0x7A0-byte zeroed static), so scanner reports never refer to the player.
        let dummy: &'static mut [u8] = Box::leak(vec![0u8; game::SCANNER_DUMMY_PED_BYTES].into_boxed_slice());
        SCANNER_DUMMY_PED.store(dummy.as_ptr() as u32, Ordering::SeqCst);
        let hook = scanner_find_player_ped as extern "C" fn(i32) -> u32 as usize as u32;
        match patcher.retarget_branch(
            &mut mem,
            "police scanner FindPlayerPed",
            game::POLICE_SCANNER_FIND_PLAYER_CALL,
            hook,
            Some(game::FIND_PLAYER_PED),
        ) {
            Ok(_) => crate::log!(
                "main: police scanner FindPlayerPed at {:#x} -> stand-in ped",
                game::POLICE_SCANNER_FIND_PLAYER_CALL
            ),
            Err(e) => crate::log!("main: police scanner hook FAILED: {e}"),
        }
        // CSV row 52 — CGame::Shutdown: like samp, never run the game's own shutdown; give
        // the network ~2 s and end the process (the enlarged model-info stores are not
        // known to CModelInfo::ShutDown, which would walk 311 ped entries of a 278-entry array).
        let mut jmp = crate::patch::encode_branch(
            crate::patch::OP_JMP,
            game::CGAME_SHUTDOWN,
            game_shutdown_hook as extern "C" fn() as usize as u32,
        )
        .to_vec();
        jmp.extend_from_slice(&[crate::patch::OP_NOP; 2]);
        match patcher.write(
            &mut mem,
            "CGame::Shutdown -> exit",
            game::CGAME_SHUTDOWN,
            &jmp,
            Some(&game::CGAME_SHUTDOWN_ORIGINAL),
        ) {
            Ok(()) => crate::log!("main: CGame::Shutdown at {:#x} -> exit after 2 s", game::CGAME_SHUTDOWN),
            Err(e) => crate::log!("main: CGame::Shutdown hook FAILED: {e}"),
        }
        // CSV row 10 — the CGame::Process call in Idle: runs every frame, also while the
        // screen is faded out (samp.saa's main.scm keeps it faded until frame ~34).
        let hook = game_process_hook as extern "C" fn() as usize as u32;
        match patcher.retarget_branch(
            &mut mem,
            "Idle -> CGame::Process hook",
            game::IDLE_GAME_PROCESS_CALL,
            hook,
            Some(game::CGAME_PROCESS),
        ) {
            Ok(old) => {
                ORIGINAL_GAME_PROCESS.store(old, Ordering::SeqCst);
                crate::log!("main: CGame::Process call at {:#x} -> {hook:#x}", game::IDLE_GAME_PROCESS_CALL);
            }
            Err(e) => crate::log!("main: CGame::Process hook FAILED: {e}"),
        }
        // Vehicles (M5): init-main byte rows, the 256-colour palette (rows 148–154) and the hooks.
        let r = crate::vehpatches::apply(patcher, &mut mem, crate::vehpatches::INIT);
        let c = crate::vehpatches::apply_colour_table(patcher, &mut mem);
        for (row, addr, e) in r.failed.iter().chain(c.failed.iter()) {
            crate::log!("main: vehicle row {row} at {addr:#x} SKIPPED: {e}");
        }
        crate::log!(
            "main: vehicle rows applied: {} byte patches, {} colour-table operands",
            r.applied.len(),
            c.applied.len()
        );
        let hooks = crate::vehhooks::install(patcher, &mut mem);
        let failed: Vec<String> =
            hooks.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: vehicle hooks: {} installed, failed: {failed:?}", hooks.len() - failed.len());
        // Number plates (rows 59/60).
        let hooks = crate::plates::install(patcher, &mut mem);
        let failed: Vec<String> =
            hooks.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: plate rows: {} installed, failed: {failed:?}", hooks.len() - failed.len());
        // World entities (M6): byte rows and hooks.
        let hooks = crate::worldhooks::install(patcher, &mut mem);
        let failed: Vec<String> =
            hooks.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: world rows: {} applied, failed: {failed:?}", hooks.len() - failed.len());
        // Remaining behaviour (M8b): the anim recorders (rows 17/18) and the camera follow before CCamera::Process
        // (row 9).
        let hooks = crate::behaviourclient::install(patcher, &mut mem);
        let failed: Vec<String> =
            hooks.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!("main: behaviour rows: {} applied, failed: {failed:?}", hooks.len() - failed.len());
        let hook = camera_process_hook as extern "thiscall" fn(u32) as usize as u32;
        match patcher.retarget_branch(
            &mut mem,
            "CGame::Process -> CCamera::Process hook",
            CAMERA_PROCESS_CALL,
            hook,
            Some(CAMERA_PROCESS),
        ) {
            Ok(old) => ORIGINAL_CAMERA_PROCESS.store(old, Ordering::SeqCst),
            Err(e) => crate::log!("main: CCamera::Process hook (row 9) FAILED: {e}"),
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
        // Fallback only: our own replacement for the story script (scm.rs), served
        // instead of data\script\main.scm through the open call in CTheScripts::Init.
        // With samp.saa the game opens samp's own main.scm from the archive.
        if crate::filehook::archive_active() {
            crate::log!("main: main.scm comes from samp.saa (samp's script); our replacement script is not used");
        } else {
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
    let mut load_checked = false;
    while !QUIT.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
        let s = mem.read_i32(game::G_GAME_STATE).unwrap_or(-1);
        if s != last_state {
            crate::log!("gGameState = {s} ({}) after {:.2}s", game::game_state_name(s), t0.elapsed().as_secs_f32());
            last_state = s;
        }
        if s == 8 && !load_checked && t0.elapsed() > Duration::from_secs(30) {
            load_checked = true;
            crate::log!("still loading (gGameState 8) {:.0} s after start", t0.elapsed().as_secs_f32());
            crate::diag::dump_threads("slow load");
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

/// Runs `f` with the process patcher (InitGame-time patches, M7).
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

/// Replaces `CGame::Shutdown` (samp B 0x100A3AB0: its own cleanup, then up to
/// 2000 ms of 100 ms sleeps, then `ExitProcess(0)`).
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

/// Zeroed stand-in ped for the police scanner (CSV row 55).
static SCANNER_DUMMY_PED: AtomicU32 = AtomicU32::new(0);

/// Replaces `FindPlayerPed(player)` in `CAEPoliceScannerAudioEntity::AddAudioEvent` (cdecl).
extern "C" fn scanner_find_player_ped(_player: i32) -> u32 {
    SCANNER_DUMMY_PED.load(Ordering::Relaxed)
}

/// `CCamera::Process` (thiscall) and its call in `CGame::Process` (CSV row 9, samp B 0x100A2640).
const CAMERA_PROCESS: u32 = 0x52B730;
const CAMERA_PROCESS_CALL: u32 = 0x53C104;
static ORIGINAL_CAMERA_PROCESS: AtomicU32 = AtomicU32::new(0);

/// Replaces `TheCamera.Process()` in `CGame::Process`: samp's camera follow first (attached object), then the
/// original.
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

/// Original target of the call at 0x53E981 (`CGame::Process`).
static ORIGINAL_GAME_PROCESS: AtomicU32 = AtomicU32::new(0);
/// The first-frame ped flags were set from [`game_process_hook`].
pub(crate) static PED_FLAGS_DONE: AtomicBool = AtomicBool::new(false);

/// Replaces the `call CGame::Process` in `Idle` (cdecl, no arguments). After
/// the first game frame with a local ped (state 9) it sets the two ped flags
/// the original sets in its first frame (TRACE-1: frame 1, after the scripts
/// ran), even while the screen is still faded out and `Render2dStuff` (our
/// per-frame tick) is skipped.
extern "C" fn game_process_hook() {
    if ARGS.get().is_some_and(|a| a.connect) {
        let _ = catch_unwind(frame_timer_step);
    }
    // World-entity pre-step (objects: streaming, attachments, MoveObject; world-entities.md RE-10 §3).
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut g) = CLIENT.try_lock()
            && let Some(c) = g.as_mut()
        {
            c.pre_process();
        }
    }));
    let orig = ORIGINAL_GAME_PROCESS.load(Ordering::Relaxed);
    if orig != 0 {
        // CGame::Process (void __cdecl(void), read from the call we retargeted) inside the shared SEH guard, like
        // samp's per-frame hook (B 0x100A2670: scope "s007"); `this` (ECX) is ignored by the callee.
        // SAFETY: a no-argument cdecl function called through the guard's thiscall shape.
        let code = unsafe { crate::swap::guarded_thiscall(orig, 0) };
        if code != 0 {
            let at = crate::swap::last_fault_address();
            crate::log!("main: CGame::Process faulted: {code:#x} at {at:#x}");
            crate::swap::push_fault(crate::swap::FaultSite::Named("s007"), code, at);
        }
    }
    if !PED_FLAGS_DONE.load(Ordering::Relaxed) {
        let _ = catch_unwind(AssertUnwindSafe(first_frame_ped_flags));
    }
    // The 0x53E981 hook also updates the chat render target (UR §2.5).
    crate::ui::game_frame();
}

static FRAME_TIMER: Mutex<Option<crate::frametimer::FrameTimer>> = Mutex::new(None);
/// Frame limit (samp's Game object +0x5d): 90 from the Game constructor (A 0x1009ffe0)
/// until the first-frame init sets the sa-mp.cfg `fpslimit` (only when it is in 20..90;
/// otherwise 90 stays), then `/fpslimit`.
static FPS_LIMIT: AtomicU32 = AtomicU32::new(GAME_DEFAULT_FPS_LIMIT);
const GAME_DEFAULT_FPS_LIMIT: u32 = 90;

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

/// samp's step before `CGame::Process` (frametimer.rs, A 0x100A1C50).
fn frame_timer_step() {
    use crate::frametimer::{self as ft, Limiter};
    let mut g = FRAME_TIMER.lock().unwrap_or_else(|e| e.into_inner());
    let t = g.get_or_insert_with(|| {
        // samp's first-frame init: 1 ms timer resolution (A 0x100c47d0).
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
        // samp calls kernel32 `Sleep` (std's sleep may use a high-resolution waitable timer instead, which wakes
        // later under Wine: ~19.2 ms frames instead of the original's 19.0 at the same limit).
        // SAFETY: plain Win32 call.
        unsafe { windows_sys::Win32::System::Threading::Sleep(s.sleep_ms) };
    }
    t.finish(since_start_ms());
}

/// The local ped is on foot and its intelligence +0xC task reports type 0x10C (A 0x100acd10).
fn limiter_task_gate() -> bool {
    let m = ProcessMemory;
    let ped = m.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0);
    if ped == 0 || m.read_u32(ped + game::off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0 {
        return false;
    }
    let intel = m.read_u32(ped + game::off::PED_INTELLIGENCE).unwrap_or(0);
    let task = if intel != 0 { m.read_u32(intel + 0xC).unwrap_or(0) } else { 0 };
    let vtbl = if task != 0 { m.read_u32(task).unwrap_or(0) } else { 0 };
    // samp's sanity range for the vtable (A 0x100b40e0).
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

fn first_frame_ped_flags() {
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

/// Replaces the `call Render2dStuff` in `Idle` (cdecl, no arguments).
/// samp's order (UR §2.4; also onfoot spec §6 step 5, A 0x100C5030 → 0x100C4FB0):
/// pre-step (netgame tick, UI work), `Render2dStuff`, textdraws, input-restore
/// countdown. The netgame's HUD switches therefore apply to the same frame (the
/// player-parity measurement of 0xBA6769/0xBAA3FB, docs/phase3/parity-notes.md).
extern "C" fn frame_hook() {
    let _ = catch_unwind(AssertUnwindSafe(tick));
    let orig = ORIGINAL_RENDER_2D.load(Ordering::Relaxed);
    if orig != 0 {
        // SAFETY: `orig` is the game's Render2dStuff (void __cdecl(void)),
        // read from the call we retargeted.
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
    // samp's per-frame step after the netgame tick: radio calls while a stream plays, the
    // mission-audio poll (audio.rs, B 0x100c4ff0).
    if ARGS.get().is_some_and(|a| a.connect) {
        crate::audio::frame_tick();
    }
    // In-game UI (M4): first-frame setup, input restore, queued UI work.
    crate::ui::frame_entry(false);
    quit_tick();
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
        // samp's Render2dStuff hook does its one-time init on its first call and creates
        // the netgame on the second (A 0x100c47d0; TRACE-1: netgame constructor's 09BA in
        // frame 35, ours was one frame earlier).
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
        Command::DevReset(n) => {
            crate::ui::force_device_lost(*n);
            format!("ok device reports lost for {n} checks")
        }
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
        Command::Files => crate::filehook::report(),
        Command::Vehicles => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.vehicle_lines()).unwrap_or_else(|| "vehicles count=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::World => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.world_lines()).unwrap_or_else(|| "world count=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Behaviour => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.behaviour_line()).unwrap_or_else(|| "behaviour none".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Spawn => match CLIENT.try_lock() {
            Ok(mut g) => g.as_mut().map(|c| c.request_debug_spawn()).unwrap_or_else(|| "error no session".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Combat => crate::combathooks::status_line(),
        Command::Chat(n) => crate::ui::control_chat(*n),
        Command::Input => crate::ui::control_input(),
        Command::Ui => crate::ui::control_ui(),
        Command::Say(t) => crate::ui::control_say(t),
        Command::Audio => crate::audio::status(),
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
    // samp's quit (A 0x100c3ec0): Disconnect(500) in netgame state 5, then the quit flag and its
    // GetTickCount; the frame routine (A 0x100c5030) keeps running the game and, once more than
    // 1000 ms passed, deletes the netgame and calls ExitProcess(0) ([`quit_tick`]).
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

/// The quit step of samp's frame routine (A 0x100c5030, after the pre-step): once more than
/// 1000 ms passed since `/quit` (`< 1001` returns), the netgame is deleted and the process ends.
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
