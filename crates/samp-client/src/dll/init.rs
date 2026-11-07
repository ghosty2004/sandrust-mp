//! The init thread: wait for `gGameState == 7`, apply samp's main block and our hooks (one step per install, in
//! samp's order), bind the control channel, leave the frontend, then service the control channel while the game
//! is not ticking.

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{
    CAMERA_PROCESS, CAMERA_PROCESS_CALL, CONTROL, LAST_TICK_MS, ORIGINAL_CAMERA_PROCESS, ORIGINAL_GAME_PROCESS,
    ORIGINAL_RENDER_2D, PATCHER, QUIT, REMOTE_PEDS_READY, SCANNER_DUMMY_PED, SECTORS_RELOCATED, TICKS,
    camera_process_hook, frame_hook, frame_hook_installed, game_process_hook, game_shutdown_hook, log_report,
    open_main_scm_hook, scanner_find_player_ped, since_start_ms, write_main_scm,
};
use crate::control;
use crate::game;
use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::patches::{self, Phase};

pub(super) fn init_thread() {
    // samp imports d3dx9_25.dll (loaded with samp.dll, before the game's first Present) and registers its fonts
    // at attach (docs/re/subsystems/ui-rendering.md, "Font assets"): done first thing here, outside the loader lock.
    crate::ui::preload();
    let t0 = Instant::now();
    wait_for_frontend_idle(t0);
    let mut mem = ProcessMemory;
    apply_main_block(&mut mem);
    bind_control();
    leave_frontend(&mut mem);
    crate::log!("init done after {:.2}s", t0.elapsed().as_secs_f32());
    service_while_idle(t0);
}

/// Wait for FRONTEND_IDLE, like samp's init thread.
fn wait_for_frontend_idle(t0: Instant) {
    let mem = ProcessMemory;
    let mut last_state = -1;
    let mut last_report = Instant::now();
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
            // Rare long stalls before state 7: record where the threads are.
            if t0.elapsed() < Duration::from_secs(35) {
                crate::diag::dump_threads("slow start");
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// samp's main block and our start-up hooks, in order, under the patcher lock.
fn apply_main_block(mem: &mut ProcessMemory) {
    let mut g = PATCHER.lock().unwrap_or_else(|e| e.into_inner());
    let patcher = g.get_or_insert_with(Patcher::new);
    relocate_sectors(patcher, mem);
    relocate_stores(patcher, mem);
    let r = patches::apply_phase(patcher, mem, Phase::Main, 0);
    log_report("main", &r);
    install_cj_walk_gate(patcher, mem);
    install_swap_hooks(patcher, mem);
    // Combat: the start-up part of the damage / projectile / weapon-render hooks.
    log_rows("combat rows", "applied", &crate::combathooks::install_main(patcher, mem));
    // HUD scale fix around the radar, Idle Sleep(1), no cheats.
    log_rows("main hooks", "installed", &crate::mainhooks::install(patcher, mem));
    install_script_opcodes(patcher, mem);
    install_scanner_hook(patcher, mem);
    install_shutdown_hook(patcher, mem);
    install_game_process_hook(patcher, mem);
    apply_vehicle_patches(patcher, mem);
    log_rows("vehicle hooks", "installed", &crate::vehhooks::install(patcher, mem));
    // Number plates.
    log_rows("plate rows", "installed", &crate::plates::install(patcher, mem));
    install_weather_audio_hook(patcher, mem);
    // World entities: byte patches and hooks.
    log_rows("world rows", "applied", &crate::worldhooks::install(patcher, mem));
    // Behaviour: the anim recorders, and the camera follow before CCamera::Process.
    log_rows("behaviour rows", "applied", &crate::behaviourclient::install(patcher, mem));
    install_camera_process_hook(patcher, mem);
    install_frame_hook(patcher, mem);
    install_main_scm(patcher, mem);
}

/// One log line for a table of install rows: how many went in, and the failed rows.
fn log_rows(what: &str, verb: &str, rows: &[(u32, Result<(), PatchError>)]) {
    let failed: Vec<String> =
        rows.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
    crate::log!("main: {what}: {} {verb}, failed: {failed:?}", rows.len() - failed.len());
}

/// Sector grid first (samp moves it before everything else in its main block); remote peds depend on it.
fn relocate_sectors(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let buffer: &'static mut [u8] = Box::leak(vec![0u8; crate::sectors::BUFFER_BYTES as usize].into_boxed_slice());
    match crate::sectors::relocate(patcher, mem, buffer.as_mut_ptr() as u32) {
        Ok(n) => {
            SECTORS_RELOCATED.store(true, Ordering::SeqCst);
            crate::log!(
                "main: sector grid relocated to {:#x} ({n} references), original zeroed",
                buffer.as_ptr() as u32
            );
        }
        Err(e) => crate::log!("main: sector relocation FAILED, remote players disabled: {e}"),
    }
}

/// Enlarged model-info stores, needed by samp.saa's IDE files.
fn relocate_stores(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    for store in crate::stores::STORES {
        let buf: &'static mut [u8] = Box::leak(crate::stores::new_entries(store).into_boxed_slice());
        match crate::stores::relocate(patcher, mem, store, buf.as_ptr() as u32) {
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
}

/// samp's CJ-walk gate over the move-anim-group store that the main block NOPs.
fn install_cj_walk_gate(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    match crate::pedhooks::install(patcher, mem) {
        Ok(()) => crate::log!("main: ProcessAnimGroups CJ-walk gate at {:#x}", crate::pedhooks::STORE_SITE),
        Err(e) => crate::log!("main: ProcessAnimGroups CJ-walk gate FAILED: {e}"),
    }
}

/// Remote-player context switch (docs/re/subsystems/remote-control-onfoot.md, "`CPlayerPed::ProcessControl` hook").
fn install_swap_hooks(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    match crate::swap::install(patcher, mem) {
        Ok(()) if SECTORS_RELOCATED.load(Ordering::SeqCst) => {
            REMOTE_PEDS_READY.store(true, Ordering::SeqCst);
            crate::log!("main: CPlayerPed::ProcessControl / CTaskSimpleUseGun::SetPedPosition hooks installed");
        }
        Ok(()) => crate::log!("main: swap hooks installed, but remote peds stay disabled (no sector relocation)"),
        Err(e) => crate::log!("main: swap hooks FAILED, remote players disabled: {e}"),
    }
}

/// samp's custom script opcodes 0x09C6 / 0x09D3 (used by samp.saa's scripts).
fn install_script_opcodes(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    match crate::scmops::install(patcher, mem) {
        Ok(()) => crate::log!("main: script opcodes 0x09C6 / 0x09D3 installed"),
        Err(e) => crate::log!("main: script opcodes 0x09C6 / 0x09D3 FAILED: {e}"),
    }
}

/// The police scanner's FindPlayerPed call gets a zeroed stand-in ped (samp: a zeroed static of the same size), so
/// scanner reports never refer to the player.
fn install_scanner_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let dummy: &'static mut [u8] = Box::leak(vec![0u8; game::SCANNER_DUMMY_PED_BYTES].into_boxed_slice());
    SCANNER_DUMMY_PED.store(dummy.as_ptr() as u32, Ordering::SeqCst);
    let hook = scanner_find_player_ped as extern "C" fn(i32) -> u32 as usize as u32;
    match patcher.retarget_branch(
        mem,
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
}

/// CGame::Shutdown: like samp, never run the game's own shutdown; give the network ~2 s and end the process
/// (CModelInfo::ShutDown does not know the enlarged model-info stores and would walk past the stock ped array).
/// Same form as samp: `jmp dword [slot]` + NOP, the handler stored in the padding slot in front of the function.
fn install_shutdown_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    match crate::worldhooks::jmp_indirect(
        patcher,
        mem,
        "CGame::Shutdown -> exit",
        game::CGAME_SHUTDOWN,
        game::CGAME_SHUTDOWN_SLOT,
        game_shutdown_hook as extern "C" fn() as usize as u32,
        &game::CGAME_SHUTDOWN_ORIGINAL,
    ) {
        Ok(()) => crate::log!("main: CGame::Shutdown at {:#x} -> exit after 2 s", game::CGAME_SHUTDOWN),
        Err(e) => crate::log!("main: CGame::Shutdown hook FAILED: {e}"),
    }
}

/// The CGame::Process call in Idle: runs every frame, also while the screen is faded out (samp.saa's main.scm keeps
/// it faded for the first frames).
fn install_game_process_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let hook = game_process_hook as extern "C" fn() as usize as u32;
    match patcher.retarget_branch(
        mem,
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
}

/// Vehicles: start-up byte patches and the 256-colour palette.
fn apply_vehicle_patches(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let r = crate::vehpatches::apply(patcher, mem, crate::vehpatches::INIT);
    let c = crate::vehpatches::apply_colour_table(patcher, mem);
    for (row, addr, e) in r.failed.iter().chain(c.failed.iter()) {
        crate::log!("main: vehicle row {row} at {addr:#x} SKIPPED: {e}");
    }
    crate::log!(
        "main: vehicle rows applied: {} byte patches, {} colour-table operands",
        r.applied.len(),
        c.applied.len()
    );
}

/// Weather audio muted while PlayerPlaySound(1)'s flag is set.
fn install_weather_audio_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    match crate::audio::install_weather_hook(patcher, mem) {
        Ok(()) => crate::log!("main: weather audio hook installed"),
        Err(e) => crate::log!("main: weather audio hook FAILED: {e}"),
    }
}

/// The camera follow before `CCamera::Process` in `CGame::Process`.
fn install_camera_process_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let hook = camera_process_hook as extern "thiscall" fn(u32) as usize as u32;
    match patcher.retarget_branch(
        mem,
        "CGame::Process -> CCamera::Process hook",
        CAMERA_PROCESS_CALL,
        hook,
        Some(CAMERA_PROCESS),
    ) {
        Ok(old) => ORIGINAL_CAMERA_PROCESS.store(old, Ordering::SeqCst),
        Err(e) => crate::log!("main: CCamera::Process hook FAILED: {e}"),
    }
}

/// Retarget the Render2dStuff call in Idle to our per-frame entry.
fn install_frame_hook(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    let hook = frame_hook as extern "C" fn() as usize as u32;
    match patcher.retarget_branch(
        mem,
        "Idle -> frame hook",
        game::IDLE_RENDER_2D_CALL,
        hook,
        Some(game::RENDER_2D_STUFF),
    ) {
        Ok(old) => {
            ORIGINAL_RENDER_2D.store(old, Ordering::SeqCst);
            crate::log!("main: per-frame hook at {:#x} -> {hook:#x} (original {old:#x})", game::IDLE_RENDER_2D_CALL);
        }
        Err(e) => crate::log!("main: per-frame hook FAILED: {e}"),
    }
}

/// Fallback only: our own replacement for the story script (scm.rs), served instead of data\script\main.scm through
/// the open call in CTheScripts::Init. With samp.saa the game opens samp's own main.scm.
fn install_main_scm(patcher: &mut Patcher, mem: &mut ProcessMemory) {
    if crate::filehook::archive_active() {
        crate::log!("main: main.scm comes from samp.saa (samp's script); our replacement script is not used");
        return;
    }
    match write_main_scm() {
        Ok(path) => {
            let hook = open_main_scm_hook as extern "C" fn(*const u8, *const u8) -> *mut c_void as usize as u32;
            match patcher.retarget_branch(
                mem,
                "CTheScripts::Init main.scm open",
                game::MAIN_SCM_OPEN_CALL,
                hook,
                Some(game::CFILEMGR_OPEN_FILE),
            ) {
                Ok(_) => crate::log!("main: main.scm open at {:#x} -> our script {}", game::MAIN_SCM_OPEN_CALL, path),
                Err(e) => crate::log!("main: main.scm redirect FAILED: {e}"),
            }
        }
        Err(e) => crate::log!("main: cannot write our main.scm: {e}; the stock story script will run"),
    }
}

fn bind_control() {
    let ctl_port = control::port_from_env();
    match control::Server::bind(ctl_port) {
        Ok(s) => {
            crate::log!("control: listening on 127.0.0.1:{ctl_port}");
            *CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
        }
        Err(e) => crate::log!("control: bind 127.0.0.1:{ctl_port} failed: {e}"),
    }
}

/// Leave the frontend, like samp: menu flags, then gGameState := 8 (LOADING_STARTED).
fn leave_frontend(mem: &mut ProcessMemory) {
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
}

/// Service the control channel from here while the game is not ticking (loading, paused, frontend), so tests can
/// always query the state; slow loads and a missing frame tick are diagnosed once.
fn service_while_idle(t0: Instant) {
    let mem = ProcessMemory;
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
            report_missing_frame_tick(&mem);
        }
        let last = LAST_TICK_MS.load(Ordering::SeqCst);
        if last == 0 || since_start_ms().saturating_sub(last) > 1000 {
            super::service::service_control("bg");
        }
    }
}

/// No frame tick 10 s after state 9: log the call site's bytes and the threads.
fn report_missing_frame_tick(mem: &ProcessMemory) {
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
