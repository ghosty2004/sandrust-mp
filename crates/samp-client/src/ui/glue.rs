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
use samp_proto::rpcs::session::OnPlayerRequestScoresAndPings;
use samp_proto::rpcs::ui::{
    OnPlayerExitedMenu, OnPlayerSelectTextDraw, OnPlayerSelectedMenuRow, PlayerRequestChatMessage,
    PlayerRequestCommandMessage,
};
use samp_proto::sync::rcon::RconCommand;
use samp_ui::chat::DisplayMode;
use samp_ui::commands::{GameInfo, PlayerPosition};
use samp_ui::text::KeyNameExpander;

use super::core::{CursorRequest, MenuRequest, Outgoing, UiCore};
use super::d3d::{self, HResult, Rect};
use super::inputlock::{InputLock, Op};
use super::render::{self, BubbleDraw, CursorReq, Frame, Renderer, ScoreboardDraw, TagDraw};
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
    say_queue: Vec<Vec<u8>>,
    device_proxy: bool,
    wndproc: bool,
    fonts_added: bool,
    /// Cached dialog layout (computed once per open, like the original).
    dialog_layout: Option<(u64, samp_ui::dialog::DialogLayout)>,
    input_first_visible: usize,
    input_caret: Option<(usize, usize)>,
    /// Remote players published by the session tick.
    remotes: Vec<super::core::RemoteUi>,
    /// GTA menu handle of the current menu.
    menu_handle: Option<i32>,
    /// Pad buttons last frame (cross, triangle) for the menu poll.
    menu_pad: (bool, bool),
    /// Class-selection panel buttons (`<<`, `>>`, `Spawn`) drawn this frame.
    class_buttons: Option<[samp_ui::Rect; 3]>,
    /// Last click on a dialog list row (double-click detection).
    last_list_click: Option<(usize, std::time::Instant)>,
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
            say_queue: Vec::new(),
            device_proxy: false,
            wndproc: false,
            fonts_added: false,
            dialog_layout: None,
            input_first_visible: 0,
            input_caret: None,
            remotes: Vec::new(),
            menu_handle: None,
            menu_pad: (false, false),
            last_list_click: None,
            class_buttons: None,
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

/// At start-up (init thread): load d3dx9_25.dll and register the SA:MP fonts.
pub fn preload() {
    // SAFETY: LoadLibraryA with a NUL-terminated name; the module stays loaded.
    let h = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryA(d3d::D3DX_DLL.as_ptr()) };
    crate::log!("ui: d3dx9_25.dll {}", if h.is_null() { "NOT loaded" } else { "loaded" });
    d3d::add_font_resources();
}

/// Before `Render2dStuff` (samp's pre-step A 0x100c4fb0, after the netgame tick).
pub fn frame_entry(debug_mode: bool) {
    let _ = catch_unwind(AssertUnwindSafe(|| frame_entry_inner(debug_mode)));
}

fn frame_entry_inner(debug_mode: bool) {
    let mut g = UI.lock().unwrap_or_else(|e| e.into_inner());
    let st = ensure(&mut g, debug_mode);
    if !st.fonts_added {
        st.fonts_added = true;
        match super::menus::install_gxt_hook() {
            Ok(()) => crate::log!("ui: CText::Get hook installed"),
            Err(e) => crate::log!("ui: CText::Get hook NOT installed: {e}"),
        }
    }
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
    for line in std::mem::take(&mut st.say_queue) {
        let game = LiveGame { connected: st.core.connected };
        st.core.say(&line, &game);
    }
    if let Some((style, time, text)) = st.core.requests.game_text.take() {
        show_game_text(st, style, time, &text);
    }
    menu_work(st);
    // Scoreboard: rows rebuilt on open / count change, RPC 155 every 3000 ms (UR §15.3).
    let count = st.core.remote_ids().count();
    // TRACE-3 U-SB-FIRST-REQUEST: RPC 155 on the opening frame, then every 3000 ms.
    let (rebuild, send) = st.core.scoreboard.frame(u64::from(super::core::tick_ms()), count);
    if rebuild {
        st.core.rebuild_scoreboard();
    }
    if send {
        st.core.push_outgoing(Outgoing::ScoresRequest);
    }
    st.core.bubbles.expire(super::core::tick_ms());
    let quit = st.core.requests.quit.take();
    drop(g);
    if let Some(disconnect) = quit {
        crate::log!("ui: /quit (disconnect={disconnect})");
        crate::dll::request_quit();
    }
}

/// After `Render2dStuff` (UR §2.4 steps 3–4): textdraws, then the input-restore step.
pub fn frame_after_2d() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut g) = UI.try_lock() else { return };
        let Some(st) = g.as_mut() else { return };
        if !gta_menu_active() && st.core.netgame && !st.core.scoreboard.is_open() {
            super::textdraws::draw_all(&mut st.core.textdraws);
        }
        let mut ops = st.lock.frame();
        if std::mem::take(&mut st.core.requests.input_restore) {
            ops.extend(st.lock.frame());
        }
        if !ops.is_empty() {
            exec_ops(ops);
        }
    }));
}

/// From the `CGame::Process` hook: the chat render-target update (UR §7.6).
pub fn game_frame() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut g) = UI.try_lock() else { return };
        let Some(st) = g.as_mut() else { return };
        if st.core.settings.direct_mode {
            return;
        }
        let menu = gta_menu_active();
        if let Some(r) = st.renderer.as_mut() {
            r.update_chat_rt(&mut st.core.chat, menu);
        }
    }));
}

/// Menus: build/delete requests and the selection poll (UCA §77/§78, §132/§140).
fn menu_work(st: &mut State) {
    use samp_ui::menu::PollAction;
    for req in std::mem::take(&mut st.core.requests.menus) {
        match req {
            MenuRequest::Show { delete_previous, build } => {
                if delete_previous && let Some(h) = st.menu_handle.take() {
                    st.runner.run(OP_DELETE_MENU, &[Arg::Int(h)]);
                }
                super::menus::set_gxt(st.core.menus.current_menu());
                st.menu_handle = super::menus::build(&mut st.runner, &build);
            }
            MenuRequest::Delete => {
                if let Some(h) = st.menu_handle.take() {
                    st.runner.run(OP_DELETE_MENU, &[Arg::Int(h)]);
                }
            }
        }
    }
    let (cross, triangle) = super::menus::pad_buttons();
    let (old_c, old_t) = st.menu_pad;
    st.menu_pad = (cross, triangle);
    let handle = st.menu_handle;
    let runner = &mut st.runner;
    let action = st.core.menus.poll(cross && !old_c, triangle && !old_t, || match handle {
        Some(h) => {
            runner.run(OP_GET_MENU_ITEM_SELECTED, &[Arg::Int(h), Arg::Local(0)]);
            runner.local_i32(0) as u8
        }
        None => samp_ui::menu::NO_ROW,
    });
    match action {
        Some(PollAction::Selected(row)) => st.core.push_outgoing(Outgoing::MenuRow(row)),
        Some(PollAction::Exited) => st.core.push_outgoing(Outgoing::MenuExit),
        Some(PollAction::HideLocal) => {
            if let Some(h) = st.menu_handle.take() {
                st.runner.run(OP_DELETE_MENU, &[Arg::Int(h)]);
            }
        }
        None => {}
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
    let _ = catch_unwind(AssertUnwindSafe(|| {
        try_with_ui(|st| {
            if let Some(r) = st.renderer.as_mut() {
                r.on_lost();
            }
        })
    }));
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

/// Camera position (TheCamera, read by samp's distance helper A 0x1009f1f0).
const CAMERA_POS: u32 = 0xB6F9CC;
/// `CPed::GetBonePosition(RwV3d&, uint32 bone, bool)` (thiscall).
const PED_GET_BONE_POSITION: u32 = 0x5E4280;
/// `IS_LINE_OF_SIGHT_CLEAR` (0x06BD), the name-tag LOS test.
const OP_IS_LINE_OF_SIGHT_CLEAR: u16 = 0x06BD;
const OP_DELETE_MENU: u16 = 0x08DA;
const OP_GET_MENU_ITEM_SELECTED: u16 = 0x08D7;

fn camera_pos() -> [f32; 3] {
    let m = ProcessMemory;
    [0, 4, 8].map(|o| m.read_f32(CAMERA_POS + o).unwrap_or(0.0))
}

fn ped_pos(ped: u32) -> Option<[f32; 3]> {
    let m = ProcessMemory;
    let mat = m.read_u32(ped + off::PLACEABLE_MATRIX).ok()?;
    let a = if mat != 0 { mat + off::MATRIX_POS } else { ped + off::PLACEABLE_PLACEMENT };
    Some([m.read_f32(a).ok()?, m.read_f32(a + 4).ok()?, m.read_f32(a + 8).ok()?])
}

fn bone_pos(ped: u32, bone: u32) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    // SAFETY: GTA's thiscall CPed::GetBonePosition on a live ped, main thread.
    unsafe {
        let f: extern "thiscall" fn(u32, *mut [f32; 3], u32, u8) = std::mem::transmute(PED_GET_BONE_POSITION as usize);
        f(ped, &mut out, bone, 0);
    }
    out
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// `CPlaceable` vtable: a deleted entity (samp's validity test).
const VTBL_CPLACEABLE: u32 = 0x863C40;

fn ped_valid(ped: u32) -> bool {
    ped != 0 && ProcessMemory.read_u32(ped).is_ok_and(|v| v != VTBL_CPLACEABLE)
}

/// Nametags to draw this frame (UR §16.1).
fn nametags(st: &mut State) -> Vec<TagDraw> {
    let core = &st.core;
    if !core.show_name_tags {
        return Vec::new();
    }
    let cam = camera_pos();
    let mut out = Vec::new();
    for r in &st.remotes {
        if r.ped == 0 || r.state == 0 || r.npc || core.hidden_tags.contains(&r.id) || !ped_valid(r.ped) {
            continue;
        }
        let Some(p) = ped_pos(r.ped) else { continue };
        let d = dist(p, cam);
        if d >= core.name_tag_distance {
            continue;
        }
        // A 0x1009f430: the ped is in the moving list (`CPhysical` +0xB4 ≠ 0).
        if ProcessMemory.read_u32(r.ped + 0xB4).unwrap_or(0) == 0 {
            continue;
        }
        let anchor = bone_pos(r.ped, 8);
        if core.name_tag_los {
            let los = |run: &mut Runner| {
                let a = [anchor[0], anchor[1], anchor[2], cam[0], cam[1], cam[2]].map(Arg::Float);
                let flags = [1, 0, 0, 1, 0].map(Arg::Int);
                let args: Vec<Arg> = a.into_iter().chain(flags).collect();
                run.run(OP_IS_LINE_OF_SIGHT_CLEAR, &args)
            };
            // Tested once per pass (bars and names) like the original.
            let a = los(&mut st.runner);
            let b = los(&mut st.runner);
            if !(a && b) {
                continue;
            }
        }
        let Some(name) = core.name(r.id) else { continue };
        let mut text = name.to_vec();
        text.extend_from_slice(format!(" ({})", r.id).as_bytes());
        out.push(TagDraw {
            anchor,
            distance: d,
            name: text,
            colour: core.colours.chat_argb(r.id) | 0xFF00_0000,
            health: r.health,
            armour: r.armour,
            afk: r.ms_since_sync >= samp_ui::nametags::AFK_AFTER_MS,
        });
    }
    out
}

/// Chat bubbles to draw (UR §11).
fn bubbles(st: &State) -> Vec<BubbleDraw> {
    let local = game::read_player(&ProcessMemory).ok().flatten();
    let cam = camera_pos();
    let mut out = Vec::new();
    for r in &st.remotes {
        let Some(b) = st.core.bubbles.get(r.id) else { continue };
        if r.ped == 0 || r.state == 0 || !ped_valid(r.ped) {
            continue;
        }
        let Some(p) = ped_pos(r.ped) else { continue };
        let d_local = local.map_or(f32::MAX, |l| dist(p, l.pos));
        if d_local >= b.distance {
            continue;
        }
        let head = bone_pos(r.ped, samp_ui::bubbles::ANCHOR_BONE);
        let raise = b.anchor_raise(dist(p, cam));
        out.push(BubbleDraw { anchor: [head[0], head[1], head[2] + raise], text: b.text.clone(), colour: b.colour });
    }
    out
}

/// UR §2.3: menu check, HUD flags, then the drawing steps.
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
    for req in std::mem::take(&mut st.core.requests.cursor) {
        let ops = match req {
            CursorRequest::Mode0 { immediate } => st.lock.request_mode0(immediate),
            CursorRequest::Mode3 => st.lock.request_mode3(),
        };
        exec_ops(ops);
    }
    // Step 5/9 HUD (TRACE-3 U-HUD-STEP9: written every Present).
    let sb_open = st.core.scoreboard.is_open();
    set_hud(!sb_open && st.core.chat.mode() != DisplayMode::Hidden);
    let tags = nametags(st);
    let bubbles = if sb_open { Vec::new() } else { bubbles(st) };
    let Some(r) = st.renderer.as_mut() else { return };
    if std::mem::take(&mut st.core.requests.rebuild_fonts) || r.needs_rebuild(fs) {
        r.rebuild_fonts(&face, weight, fs);
        st.dialog_layout = None;
    }
    // Dialog layout once per open (and after a font rebuild).
    let serial = st.core.dialog_serial;
    if let Some(d) = st.core.dialogs.current() {
        if st.dialog_layout.as_ref().is_none_or(|(s, _)| *s != serial) {
            let (cw, ch) = render::client_size();
            st.dialog_layout = Some((serial, d.layout(&r.fonts, cw, ch)));
        }
        if let Some((_, l)) = &st.dialog_layout
            && let Some(list) = l.list
        {
            st.core.dialog_page_rows = d.page_rows(&list, l.list_row_h);
        }
    }
    // Chat input first visible character (DXUT PlaceCaret on caret moves).
    if st.core.input.is_open() {
        let key = (st.core.input.edit().caret(), st.core.input.edit().text().len());
        if st.input_caret != Some(key) {
            st.input_caret = Some(key);
            st.input_first_visible = r.input_first_visible(&st.core.chat, &st.core.input, st.input_first_visible);
        }
        let (cw, _) = render::client_size();
        st.core.input_rect =
            samp_ui::input::box_rect(cw, st.core.chat.bottom(r.fonts.main_h), r.fonts.gui0_height);
    } else {
        st.input_caret = None;
    }
    let (w, h) = (render::video_width(), render::video_height());
    let (sw, shh) = samp_ui::scoreboard::size(w);
    st.core.scoreboard_rect = samp_ui::Rect::xywh(w / 2 - sw / 2, h / 2 - shh / 2, sw, shh);
    let cursor = render::cursor_pos();
    let sb_draw = sb_open.then(|| ScoreboardDraw {
        rows: st.core.scoreboard.rows(),
        server_name: &st.core.server_name,
        players: samp_ui::scoreboard::players_text(st.core.remote_ids().count()),
        selected: st.core.scoreboard.selected(),
        scroll: st.core.scoreboard.scroll(),
    });
    let dialog = if st.core.dialogs.drawn(st.core.input.is_open(), sb_open) {
        st.core.dialogs.current().zip(st.dialog_layout.as_ref().map(|(_, l)| l))
    } else {
        None
    };
    let frame = Frame {
        chat: &st.core.chat,
        input: &st.core.input,
        input_first_visible: st.input_first_visible,
        tags: &tags,
        nametag_status: st.core.settings.nametag_status,
        bubbles: &bubbles,
        scoreboard: sb_draw,
        death: st.core.deaths.visible().then_some(&st.core.deaths),
        dialog,
        cursor,
    };
    let lock = &mut st.lock;
    r.frame(&frame, &mut |req| match req {
        CursorReq::Mode2 => exec_ops(lock.request_mode2()),
    });
    // Step 12: the class-selection GUI dialog after the chat GUI dialog (UR §2.3, §4.2),
    // with cursor mode 2 while it is shown (TRACE-3: class selection locks input).
    if st.core.class_selection && !sb_open {
        exec_ops(st.lock.request_mode2());
        st.class_buttons = Some(r.class_panel(cursor));
    } else {
        st.class_buttons = None;
    }
    // Step 12 end: textdraw selection per-frame logic (UR §14.4).
    if st.core.textdraws.selection.active && !sb_open {
        // A 0x10071470: cursor mode 2, HUD off (measured: HUD flags 0/1 while selecting),
        // then the hover test.
        exec_ops(st.lock.request_mode2());
        set_hud(false);
        st.core.textdraws.hover(cursor.0, cursor.1);
    }
    // Leaving a mode-2 UI: back to mode 0 with an immediate restore (UR §8).
    let wants2 = st.core.input.is_open()
        || st.core.class_selection
        || st.core.dialogs.is_open()
        || st.core.textdraws.selection.active;
    if !wants2 && st.lock.mode() == 2 {
        exec_ops(st.lock.request_mode0(true));
    }
}

// ---- window messages ---------------------------------------------------------------

/// A window event for the UI; returns whether it is consumed.
pub(super) fn on_window_event(ev: samp_ui::keys::Event, wheel_lines: u32) -> bool {
    use samp_ui::keys::Event;
    let menu = gta_menu_active();
    try_with_ui(|st| {
        // Mouse on the dialog / textdraws first (GUI dialog MsgProc, selection handler).
        if let Event::LeftButtonUp { x, y } = ev
            && !menu
            && mouse_up(st, x, y)
        {
            return true;
        }
        let game = LiveGame { connected: st.core.connected };
        st.core.handle_event(ev, menu, &game, wheel_lines)
    })
    .unwrap_or(false)
}

/// Left button released over a dialog control or a hovered textdraw.
fn mouse_up(st: &mut State, x: i32, y: i32) -> bool {
    use samp_ui::dialog::Button;
    if st.core.textdraws.selection.active
        && let Some(id) = st.core.textdraws.selection.hovered
    {
        // UCA §83: the hovered id is sent; selection stays on.
        st.core.push_outgoing(Outgoing::SelectTextDraw(id));
        return true;
    }
    if let Some(b) = st.class_buttons
        && let Some(i) = b.iter().position(|r| r.contains(x, y))
    {
        // `<<` previous class, `>>` next class, `Spawn` (consumed by the session tick).
        CLASS_CLICK.store(i as u8 + 1, std::sync::atomic::Ordering::SeqCst);
        return true;
    }
    if !st.core.dialogs.drawn(st.core.input.is_open(), st.core.scoreboard.is_open()) {
        return false;
    }
    let Some((_, l)) = st.dialog_layout.clone() else { return false };
    if l.button1.contains(x, y) {
        st.core.dialog_respond(Button::Ok);
        return true;
    }
    if l.button2.is_some_and(|b| b.contains(x, y)) {
        st.core.dialog_respond(Button::Cancel);
        return true;
    }
    if let (Some(list), Some(d)) = (l.list, st.core.dialogs.current_mut()) {
        if let Some(row) = d.row_at(&list, l.list_row_h, x, y) {
            // Measured on the original (MR-M8A-3): a click selects; a double click
            // activates (response 1); a second single click 1.5 s later does nothing.
            // DXUT's list box reacts to WM_LBUTTONDBLCLK: two clicks within the
            // system double-click time on the same row.
            let now = std::time::Instant::now();
            // SAFETY: plain Win32 query.
            let dbl = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
            let double = st.last_list_click.is_some_and(|(r, t)| {
                r == row && now.duration_since(t).as_millis() <= u128::from(dbl)
            });
            d.select(row);
            if double {
                st.last_list_click = None;
                st.core.dialog_respond(Button::Ok);
            } else {
                st.last_list_click = Some((row, now));
            }
        }
        return list.contains(x, y);
    }
    l.rect.contains(x, y)
}

/// A click on the class-selection panel: 1 = `<<`, 2 = `>>`, 3 = `Spawn` (0 = none).
static CLASS_CLICK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Takes the pending class-selection panel click (session tick).
pub fn take_class_click() -> u8 {
    CLASS_CLICK.swap(0, std::sync::atomic::Ordering::SeqCst)
}

/// Edit-box key-down; true if the input or a dialog edit box has focus (consumed).
pub(super) fn on_edit_key(key: samp_ui::edit::EditKey, mods: samp_ui::edit::Mods) -> bool {
    try_with_ui(|st| st.core.edit_key(key, mods)).unwrap_or(false)
}

// ---- network session hooks (client.rs) ------------------------------------------------

/// The netgame is created (Client::start).
pub fn on_netgame_start(host: &str, port: u16, name: &str) {
    with_ui(|st| st.core.on_netgame_start(host, port, name.as_bytes()));
}

/// RPC 139 InitGame.
pub fn on_init_game(g: &samp_proto::session::InitGame) {
    with_ui(|st| st.core.on_init_game(g));
}

/// The session closed.
pub fn on_closed() {
    with_ui(|st| st.core.on_closed());
}

/// Every S→C RPC, before the game handlers.
pub fn on_rpc(r: &Incoming) {
    with_ui(|st| st.core.on_rpc(r, &GameExpander));
}

/// Remote players for nametags, bubbles and the scoreboard (once per session tick).
pub fn set_remotes(remotes: Vec<super::core::RemoteUi>) {
    let _ = try_with_ui(|st| st.remotes = remotes);
}

/// Class selection state (the scoreboard does not open during it).
pub fn set_class_selection(active: bool) {
    let _ = try_with_ui(|st| st.core.class_selection = active);
}

/// The connect tick: "Connecting to" lines (TRACE-3 U-CONNECT-TIMING). Returns true
/// when the session may start (or restart) connecting.
pub fn connect_tick(answered: bool) -> bool {
    with_ui(|st| st.core.connect_tick(answered)).unwrap_or(true)
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
            Outgoing::DialogResponse(r) => {
                crate::log!(
                    "ui: RPC 62 id={} response={} listitem={} text={:?}",
                    r.dialog_id,
                    r.response,
                    r.list_item,
                    String::from_utf8_lossy(&r.text)
                );
                net.send_rpc(&r, Reliability::ReliableOrdered);
            }
            Outgoing::SelectTextDraw(textdraw_id) => {
                crate::log!("ui: RPC 83 textdraw {textdraw_id:#x}");
                net.send_rpc(&OnPlayerSelectTextDraw { textdraw_id }, Reliability::ReliableOrdered);
            }
            Outgoing::MenuRow(row) => {
                crate::log!("ui: RPC 132 row {row}");
                net.send_rpc(&OnPlayerSelectedMenuRow { row }, Reliability::Reliable);
            }
            Outgoing::MenuExit => {
                crate::log!("ui: RPC 140");
                net.send_rpc(&OnPlayerExitedMenu, Reliability::Reliable);
            }
            Outgoing::ScoresRequest => net.send_rpc(&OnPlayerRequestScoresAndPings, Reliability::Reliable),
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

/// `ui`: dialog, scoreboard, textdraws, menus, death window, bubbles, cursor mode.
pub fn control_ui() -> String {
    try_with_ui(|st| format!("{} cursor_mode={} tags={}", st.core.ui_report(), st.lock.mode(), st.remotes.len()))
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
