//! Global UI state and the entry points used by `dll` and `client` (Windows only).
//!
//! Everything runs on the game's main thread (per-frame entry, Present, the
//! window procedure, the network session), except the control channel, which may
//! be serviced from the background thread: it only reads state or queues work.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Mutex;

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::Incoming;
use samp_proto::rpcs::ui::{PlayerRequestChatMessage, PlayerRequestCommandMessage};
use samp_proto::sync::rcon::RconCommand;
use samp_ui::chat::DisplayMode;
use samp_ui::commands::{GameInfo, PlayerPosition};
use samp_ui::text::KeyNameExpander;

use super::core::{Outgoing, UiCore};
use super::d3d::{self, HResult, Rect};
use super::inputlock::{InputLock, Op};
use super::render::Renderer;
use crate::game::{self, off};
use crate::netclient::{NetClient, Phase};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner};

/// GTA user files directory (`InitUserDirectories`), a C string.
const GTA_USER_DIR: u32 = 0xC92368;
/// `FrontEndMenuManager.m_bMenuActive` (UR §2.3 step 3).
const GTA_MENU_ACTIVE: u32 = 0xBA67A4;
/// `CMessages::AddBigMessage(text, time, style)` (UCA §73, A 0x100a0d40 → 0x69F2B0).
const ADD_BIG_MESSAGE: u32 = 0x69F2B0;
/// `CMessages::InsertPlayerControlKeysInString` (UCA chat model, `~k~`).
const INSERT_PLAYER_CONTROL_KEYS: u32 = 0x69E160;
/// `/mem` counter (UCA local commands).
const MEMORY_COUNTER: u32 = 0x8A5A80;
/// Script opcodes (gta-reversed `eScriptCommands.h`).
const OP_CLEAR_PRINTS: u16 = 0x00BE;
const OP_GET_AREA_VISIBLE: u16 = 0x077E;
/// `CPed::m_pVehicle`, vehicle colours (gta-reversed `Ped.h` / `Vehicle.h`).
const PED_VEHICLE: u32 = 0x58C;
const VEHICLE_COLOUR1: u32 = 0x434;
const VEHICLE_COLOUR2: u32 = 0x435;

struct State {
    core: UiCore,
    renderer: Option<Renderer>,
    renderer_failed: bool,
    lock: InputLock,
    runner: Runner,
    /// samp's 512-byte static game-text buffer (UCA §73).
    game_text: Box<[u8; 512]>,
    joining_seen: bool,
    /// HUD was switched off by chat mode 0 (see notes "HUD and chat mode").
    hud_off_by_chat: bool,
    say_queue: Vec<Vec<u8>>,
    device_proxy: bool,
    wndproc: bool,
}

// SAFETY: the raw D3D pointers inside are only used on the game's main thread;
// other threads only touch plain data under the mutex.
unsafe impl Send for State {}

static UI: Mutex<Option<State>> = Mutex::new(None);

fn user_dir() -> Option<PathBuf> {
    let mut buf = [0u8; 260];
    let _ = ProcessMemory.read(GTA_USER_DIR, &mut buf);
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    let dir = if n == 0 {
        std::env::current_dir().ok()?
    } else {
        PathBuf::from(String::from_utf8_lossy(&buf[..n]).into_owned()).join("SAMP")
    };
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Writes a byte value into game data/code for the input lock.
fn exec_ops(ops: Vec<Op>) {
    let mut mem = ProcessMemory;
    for op in ops {
        match op {
            Op::Write(a, b) => {
                if let Err(e) = mem.write(a, &b) {
                    crate::log!("ui: input lock write {a:#x} failed: {e}");
                }
            }
            Op::Zero(a) => {
                let _ = mem.write(a, &0u32.to_le_bytes());
            }
            Op::Call(a) => {
                // SAFETY: the game's cdecl `void f(void)` CPad helpers, main thread.
                let f: extern "C" fn() = unsafe { std::mem::transmute(a as usize) };
                f();
            }
            Op::ShowCursor(s) => d3d::show_cursor(s),
        }
    }
}

/// UR §2.3 "HUD off/on": menu HUD flag and the script radar flag.
fn set_hud(on: bool) {
    let mut m = ProcessMemory;
    let _ = m.write(game::MENU_HUD_ON, &[u8::from(on)]);
    let _ = m.write(game::HUD_SCRIPT_DONT_DISPLAY_RADAR, &[u8::from(!on)]);
}

fn gta_menu_active() -> bool {
    ProcessMemory.read_u8(GTA_MENU_ACTIVE).unwrap_or(0) != 0
}

/// GTA's `~k~` substitution on a private buffer.
struct GameExpander;

impl KeyNameExpander for GameExpander {
    fn expand(&self, text: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 2048];
        let n = text.len().min(1000);
        buf[..n].copy_from_slice(&text[..n]);
        // SAFETY: cdecl `void(char*)` editing a NUL-terminated buffer in place;
        // the input is < 400 chars (caller) and the buffer leaves room to grow.
        unsafe {
            let f: extern "C" fn(*mut u8) = std::mem::transmute(INSERT_PLAYER_CONTROL_KEYS as usize);
            f(buf.as_mut_ptr());
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(end);
        buf
    }
}

/// Game queries for local commands (read on the main thread).
struct LiveGame {
    connected: bool,
}

impl GameInfo for LiveGame {
    fn connected(&self) -> bool {
        self.connected
    }
    fn memory_counter(&self) -> u32 {
        ProcessMemory.read_u32(MEMORY_COUNTER).unwrap_or(0)
    }
    fn interior(&self) -> u32 {
        let mut r = Runner::new();
        r.run(OP_GET_AREA_VISIBLE, &[Arg::Local(0)]);
        r.local_i32(0) as u32
    }
    fn position(&self) -> Option<PlayerPosition> {
        let mem = ProcessMemory;
        let p = game::read_player(&mem).ok().flatten()?;
        let in_vehicle = mem.read_u32(p.ptr + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        let veh = mem.read_u32(p.ptr + PED_VEHICLE).unwrap_or(0);
        // TODO-unknown (U-SAVE-HEADING): heading source and units of /save; here the
        // ped heading (radians) as degrees in [0, 360).
        let deg = |r: f32| r.to_degrees().rem_euclid(360.0);
        if in_vehicle && veh != 0 {
            let m = mem.read_u32(veh + off::PLACEABLE_MATRIX).unwrap_or(0);
            let rd = |a: u32| mem.read_f32(a).unwrap_or(0.0);
            let (pos, fwd) = if m != 0 {
                ([rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)], (rd(m + 0x10), rd(m + 0x14)))
            } else {
                (p.pos, (0.0, 1.0))
            };
            // TODO-unknown (U-SAVE-HEADING): vehicle heading from the forward vector.
            let heading = (-fwd.0).atan2(fwd.1).to_degrees().rem_euclid(360.0);
            return Some(PlayerPosition::InCar {
                model: u32::from(mem.read_u16(veh + off::ENTITY_MODEL).unwrap_or(0)),
                x: pos[0],
                y: pos[1],
                z: pos[2],
                heading,
                colour1: u32::from(mem.read_u8(veh + VEHICLE_COLOUR1).unwrap_or(0)),
                colour2: u32::from(mem.read_u8(veh + VEHICLE_COLOUR2).unwrap_or(0)),
            });
        }
        Some(PlayerPosition::OnFoot {
            skin: u32::from(mem.read_u16(p.ptr + off::ENTITY_MODEL).unwrap_or(0)),
            x: p.pos[0],
            y: p.pos[1],
            z: p.pos[2],
            heading: deg(p.heading),
        })
    }
}

/// Creates the UI state on first use (main thread).
fn ensure(g: &mut Option<State>, debug_mode: bool) -> &mut State {
    g.get_or_insert_with(|| {
        let dir = user_dir();
        crate::log!("ui: user files directory {:?}", dir);
        let core = UiCore::new(dir, debug_mode);
        crate::log!(
            "ui: sa-mp.cfg page_size={} fontsize={} timestamp={}",
            core.settings.page_size,
            core.settings.font_size,
            core.settings.timestamps
        );
        State {
            core,
            renderer: None,
            renderer_failed: false,
            lock: InputLock::new(),
            runner: Runner::new(),
            game_text: Box::new([0u8; 512]),
            joining_seen: false,
            hud_off_by_chat: false,
            say_queue: Vec::new(),
            device_proxy: false,
            wndproc: false,
        }
    })
}

/// Main-thread access (creates the state if needed).
fn with_ui<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = UI.lock().unwrap_or_else(|e| e.into_inner());
    Some(f(ensure(&mut g, false)))
}

fn try_with_ui<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = UI.try_lock().ok()?;
    g.as_mut().map(f)
}

// ---- per-frame entry -------------------------------------------------------------

/// Called once per frame from the game's per-frame entry (after the session tick).
pub fn frame_entry(debug_mode: bool) {
    let _ = catch_unwind(AssertUnwindSafe(|| frame_entry_inner(debug_mode)));
}

fn frame_entry_inner(debug_mode: bool) {
    let mut g = UI.lock().unwrap_or_else(|e| e.into_inner());
    let st = ensure(&mut g, debug_mode);
    if !st.wndproc {
        st.wndproc = true;
        match super::wndproc::install() {
            Ok(()) => crate::log!("ui: window procedure subclassed"),
            Err(e) => crate::log!("ui: window procedure NOT subclassed: {e}"),
        }
    }
    if !st.device_proxy {
        match d3d::install_device_proxy(present_hook, reset_hook) {
            Ok(()) => st.device_proxy = true,
            Err(e) => crate::log!("ui: device proxy not installed yet: {e}"),
        }
    }
    // Input restore countdown (UR §2.4 step 4); focus loss runs it once more.
    let mut ops = st.lock.frame();
    if std::mem::take(&mut st.core.requests.input_restore) {
        ops.extend(st.lock.frame());
    }
    if !ops.is_empty() {
        exec_ops(ops);
    }
    for line in std::mem::take(&mut st.say_queue) {
        let game = LiveGame { connected: st.core.connected };
        st.core.say(&line, &game);
    }
    if let Some((style, time, text)) = st.core.requests.game_text.take() {
        show_game_text(st, style, time, &text);
    }
    let quit = st.core.requests.quit.take();
    drop(g);
    if let Some(disconnect) = quit {
        crate::log!("ui: /quit (disconnect={disconnect})");
        crate::dll::request_quit();
    }
}

/// RPC 73 (UCA §73): clear prints, copy into the static buffer, AddBigMessage.
fn show_game_text(st: &mut State, style: u8, time: u32, text: &[u8]) {
    st.runner.run(OP_CLEAR_PRINTS, &[]);
    let n = text.len().min(st.game_text.len() - 1);
    st.game_text[..n].copy_from_slice(&text[..n]);
    st.game_text[n] = 0;
    // SAFETY: GTA's cdecl `CMessages::AddBigMessage(char*, uint32, uint16)`; the
    // buffer is static (it keeps the pointer).
    unsafe {
        let f: extern "C" fn(*const u8, u32, u32) = std::mem::transmute(ADD_BIG_MESSAGE as usize);
        f(st.game_text.as_ptr(), time, u32::from(style));
    }
    crate::log!("ui: game text style {style} {time} ms {:?}", String::from_utf8_lossy(text));
}

// ---- Present / Reset wrappers ----------------------------------------------------------

extern "system" fn present_hook(
    _this: *mut std::ffi::c_void,
    a: *const Rect,
    b: *const Rect,
    c: isize,
    d: *const std::ffi::c_void,
) -> HResult {
    let _ = catch_unwind(present_pass);
    d3d::call_present(a, b, c, d)
}

extern "system" fn reset_hook(_this: *mut std::ffi::c_void, params: *mut std::ffi::c_void) -> HResult {
    let _ = catch_unwind(|| {
        try_with_ui(|st| {
            if let Some(r) = &st.renderer {
                r.on_lost();
            }
        })
    });
    let hr = d3d::call_reset(params);
    crate::log!("ui: device Reset -> {hr:#x}");
    if hr >= 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            try_with_ui(|st| {
                if let Some(r) = st.renderer.as_mut() {
                    r.on_reset();
                }
            })
        }));
    }
    hr
}

/// UR §2.3 (M4 subset): menu check, HUD visibility from the chat mode, chat window,
/// input cursor mode, GUI dialog with the scrollbar and the input box.
fn present_pass() {
    // Step 3: GTA pause menu → no samp UI.
    if gta_menu_active() {
        return;
    }
    let Ok(mut g) = UI.try_lock() else { return };
    let Some(st) = g.as_mut() else { return };
    let face = st.core.settings.font_face.clone().unwrap_or_else(|| samp_ui::fonts::DEFAULT_FACE.into()).into_bytes();
    let weight = samp_ui::fonts::weight(st.core.settings.font_weight);
    let fs = st.core.settings.font_size;
    if st.renderer.is_none() && !st.renderer_failed {
        match Renderer::new(&face, weight, fs) {
            Ok(r) => st.renderer = Some(r),
            Err(e) => {
                crate::log!("ui: renderer unavailable: {e}");
                st.renderer_failed = true;
            }
        }
    }
    // Step 9 (deviation, see notes): chat mode 0 hides the HUD every frame; leaving
    // mode 0 turns it back on once.
    if st.core.chat.mode() == DisplayMode::Hidden {
        set_hud(false);
        st.hud_off_by_chat = true;
    } else if st.hud_off_by_chat {
        st.hud_off_by_chat = false;
        set_hud(true);
    }
    // Step 11: the open input requests cursor mode 2 every frame; closing it
    // requests mode 0 with an immediate restore (UR §8).
    let ops = if st.core.input.is_open() {
        st.lock.request_mode2()
    } else if st.lock.mode() == 2 {
        st.lock.request_mode0(true)
    } else {
        Vec::new()
    };
    exec_ops(ops);
    let Some(r) = st.renderer.as_mut() else { return };
    if std::mem::take(&mut st.core.requests.rebuild_fonts) || r.needs_rebuild(fs) {
        r.rebuild_fonts(&face, weight, fs);
    }
    r.frame(&st.core.chat, &st.core.input);
}

// ---- window messages ---------------------------------------------------------------

/// A window event for the UI; returns whether it is consumed.
pub(super) fn on_window_event(ev: samp_ui::keys::Event, wheel_lines: u32) -> bool {
    let menu = gta_menu_active();
    try_with_ui(|st| {
        let game = LiveGame { connected: st.core.connected };
        st.core.handle_event(ev, menu, &game, wheel_lines)
    })
    .unwrap_or(false)
}

/// Edit-box key-down; true if the input is open (key consumed).
pub(super) fn on_edit_key(key: samp_ui::edit::EditKey, mods: samp_ui::edit::Mods) -> bool {
    try_with_ui(|st| {
        if st.core.input.is_open() {
            st.core.edit_key(key, mods);
            true
        } else {
            false
        }
    })
    .unwrap_or(false)
}

// ---- network session hooks (client.rs) ------------------------------------------------

/// The netgame is created (Client::start).
pub fn on_netgame_start(host: &str, port: u16, name: &str) {
    with_ui(|st| st.core.on_netgame_start(host, port, name.as_bytes()));
}

/// RPC 139 InitGame.
pub fn on_init_game(server_name: &[u8], player_id: u16) {
    with_ui(|st| st.core.on_init_game(server_name, player_id));
}

/// The session closed.
pub fn on_closed() {
    with_ui(|st| st.core.on_closed());
}

/// Every S→C RPC, before the game handlers.
pub fn on_rpc(r: &Incoming) {
    with_ui(|st| st.core.on_rpc(r, &GameExpander));
}

/// Once per session tick: connection lines and the UI's outgoing messages.
pub fn net_tick(net: &mut NetClient) {
    let out = with_ui(|st| {
        if *net.phase() == Phase::Joining && !st.joining_seen {
            st.joining_seen = true;
            st.core.on_joining();
        }
        st.core.take_outgoing()
    })
    .unwrap_or_default();
    for o in out {
        match o {
            Outgoing::Chat(message) => {
                crate::log!("ui: RPC 101 {:?}", String::from_utf8_lossy(&message));
                net.send_rpc(&PlayerRequestChatMessage { message }, Reliability::Reliable);
            }
            Outgoing::Command(command) => {
                crate::log!("ui: RPC 50 {:?}", String::from_utf8_lossy(&command));
                net.send_rpc(&PlayerRequestCommandMessage { command }, Reliability::Reliable);
            }
            Outgoing::Rcon(command) => net.send_sync(&RconCommand { command }),
        }
    }
}

// ---- control channel ---------------------------------------------------------------

/// `chat [N]`.
pub fn control_chat(n: usize) -> String {
    try_with_ui(|st| st.core.chat_report(n)).unwrap_or_else(|| "chat count=0 error=busy".into())
}

/// `input`.
pub fn control_input() -> String {
    try_with_ui(|st| {
        let mut s = st.core.input_report();
        s.push_str(&format!(
            " cursor_mode={} renderer={}",
            st.lock.mode(),
            if st.renderer.is_some() {
                "ok"
            } else if st.renderer_failed {
                "failed"
            } else {
                "none"
            }
        ));
        if let Some(g) = &st.core.last_game_text {
            s.push_str(&format!(
                " gametext_style={} gametext_ms={} gametext={}",
                g.style,
                g.time_ms,
                super::core::quote(&g.text)
            ));
        }
        s
    })
    .unwrap_or_else(|| "error ui busy".into())
}

/// `say TEXT` (debug helper): queued, run on the next frame as typed input.
pub fn control_say(text: &str) -> String {
    try_with_ui(|st| {
        st.say_queue.push(text.as_bytes().to_vec());
        "say queued".to_string()
    })
    .unwrap_or_else(|| "error ui busy".into())
}
