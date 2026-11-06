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
use samp_ui::keys::Overlay;
use samp_ui::commands::{GameInfo, PlayerPosition};
use samp_ui::text::KeyNameExpander;

use super::core::{CursorRequest, MenuRequest, Outgoing, UiCore};
use super::d3d::{self, HResult, Rect};
use super::inputlock::{InputLock, Op};
use super::render::{self, BubbleDraw, CursorReq, Frame, Renderer, ScoreboardDraw, TagDraw};
use crate::game::{self, off};
use crate::netclient::NetClient;
use crate::objedit;
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
/// `GET_CHAR_HEADING` / `GET_CAR_HEADING` of `/save` and `/rs` (descriptors A 0x100e9dc0 / 0x100e9d98).
const OP_GET_CHAR_HEADING: u16 = 0x0172;
const OP_GET_CAR_HEADING: u16 = 0x0174;
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
    /// The class-selection panel (GUI dialog 0x1026eaa4, visible flag +0x13) was made visible: samp's class-selection
    /// step (A 0x10006100) does it on the first frame without an open dialog and keeps it until the spawn.
    class_panel_shown: bool,
    /// Last click on a dialog list row (double-click detection).
    last_list_click: Option<(usize, std::time::Instant)>,
    /// Last scoreboard row click (double-click detection).
    last_scoreboard_click: Option<(usize, std::time::Instant)>,
    /// The spawn ends class selection: its mode-0 request restores input only after
    /// 10 frames (A 0x10003c20 → 0x100a0750(0, 0); measured: 0x541DF5 restored 10
    /// frames after the spawn frame, docs/phase3/parity-notes.md).
    spawn_release: bool,
    /// samp's netgame object exists (from the game frame after the UI construction).
    netgame_object: bool,
    /// F5 counters published by the session tick (`net_tick`).
    net_stats: super::core::NetStats,
    /// Wire byte totals (sent, received) published by the session tick.
    net_bytes: (u64, u64),
    /// The F5 rate sampler (samp's overlay object): totals and deltas of the last sample.
    rates: RateSampler,
}

/// The F5 download/upload rates (A 0x10060d20): while the overlay draws, once more than 1000 ms
/// have passed since the last sample, the byte deltas since that sample are taken; each rate is
/// the stored delta / 1024 (KB per sample) until the next sample.
#[derive(Default)]
struct RateSampler {
    last: Option<std::time::Instant>,
    sent: u64,
    received: u64,
    d_sent: u64,
    d_received: u64,
}

impl RateSampler {
    fn sample(&mut self, now: std::time::Instant, sent: u64, received: u64) -> (f32, f32) {
        if self.last.is_none_or(|t| now.duration_since(t).as_millis() > 1000) {
            self.last = Some(now);
            self.d_received = received.wrapping_sub(self.received);
            self.received = received;
            self.d_sent = sent.wrapping_sub(self.sent);
            self.sent = sent;
        }
        (self.d_received as f32 / 1024.0, self.d_sent as f32 / 1024.0)
    }
}

// SAFETY: the raw D3D pointers inside are only used on the game's main thread;
// other threads only touch plain data under the mutex.
unsafe impl Send for State {}

static UI: Mutex<Option<State>> = Mutex::new(None);

fn user_dir() -> Option<PathBuf> {
    let mut buf = [0u8; 260];
    let _ = ProcessMemory.read(GTA_USER_DIR, &mut buf);
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    // A 0x100c3cb0: the current directory when GTA has no user directory; otherwise
    // `<user files>\SAMP`, created with its `screens` folder if missing.
    if n == 0 {
        return std::env::current_dir().ok();
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&buf[..n]).into_owned()).join("SAMP");
    let _ = std::fs::create_dir(&dir);
    let _ = std::fs::create_dir(dir.join("screens"));
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
            Op::MouseSink(a) => {
                let sink = std::ptr::addr_of!(MOUSE_SINK) as usize as u32;
                if let Err(e) = mem.write(a, &sink.to_le_bytes()) {
                    crate::log!("ui: input lock redirect {a:#x} failed: {e}");
                }
            }
        }
    }
}

/// Mode 4's dummy mouse store (samp: A 0x101501a8); GTA writes floats into it.
static mut MOUSE_SINK: [u32; 4] = [0; 4];

/// samp's input-lock setter (A 0x100a0750) for other subsystems (object editing/selection):
/// `mode` 0–4, `flag` = immediate restore for mode 0. Runs the operations at once (main thread).
pub fn set_input_mode(mode: u8, flag: bool) {
    if let Some(ops) = with_ui(|st| st.lock.set_mode(mode, flag)) {
        exec_ops(ops);
    }
}

/// Editor start (A 0x10072480): mode 0 and the restore at once.
pub fn restore_input_now(flag: bool) {
    if let Some(ops) = with_ui(|st| st.lock.restore_now(flag)) {
        exec_ops(ops);
    }
}

/// The plate painter (B 0x1006D840) for the vehicle pool pass; 0 when the renderer is not up.
pub fn paint_plate(text: &[u8]) -> u32 {
    with_ui(|st| st.renderer.as_mut().map(|r| r.paint_plate(text))).flatten().unwrap_or(0)
}

/// RPC 27 / 117 (B 0x1000E880 / 0x10011880): textdraw selection off when it is active (B 0x10071550).
pub fn cancel_textdraw_selection() {
    with_ui(|st| {
        if st.core.textdraws.selection.active {
            st.core.cancel_textdraw_select();
        }
    });
}

/// The chat input closed when the editor or selection ends (B 0x10069540 / 0x10069440).
pub fn close_chat_input() {
    with_ui(|st| {
        if st.core.input.is_open() {
            st.core.input.close();
        }
    });
}

// ---- object editor and selection (Present step 12, window messages) ------------------------------------------

/// Pad 0 `Cross` (the sprint key; samp tests key bit 8 of its on-foot key word, B 0x100AF5E0).
fn sprint_held() -> bool {
    crate::worldhooks::rd::<i16>(crate::game::PAD0 + 0x20) != 0
}

fn matrix_of(ent: u32) -> Option<[f32; 16]> {
    use crate::worldhooks::rd;
    let m: u32 = rd(ent + 0x14);
    (m != 0).then(|| std::array::from_fn(|i| rd(m + 4 * i as u32)))
}

fn camera_distance(p: [f32; 3]) -> f32 {
    dist(p, camera_pos())
}

/// The cursor ray of the selection pick (B 0x10073F70) from samp's copies of WORLD, VIEW and PROJECTION.
fn pick_ray(cursor: (i32, i32)) -> Option<([f32; 3], [f32; 3])> {
    let lock = |m: &Mutex<[f32; 16]>| *m.lock().unwrap_or_else(|e| e.into_inner());
    let (world, view, proj) = (lock(&d3d::WORLD), lock(&d3d::VIEW), lock(&d3d::PROJECTION));
    let inv = objedit::mat_inverse(&objedit::mat_mul(&world, &view))?;
    let size = (render::video_width().max(1) as u32, render::video_height().max(1) as u32);
    Some(objedit::pick_ray(cursor, size, &proj, &inv))
}

/// Editor per-frame (B 0x100737B0) then selection per-frame (B 0x1006DBC0).
fn editor_pass(lock: &mut InputLock, r: &Renderer) {
    let sprint = sprint_held();
    let cursor = render::cursor_pos();
    objedit::with(|sh| {
        let frame = sh.frame;
        let ed = &mut sh.editor;
        ed.drawn = false;
        if ed.active {
            if ed.dragging {
                d3d::show_cursor(false);
            } else if sprint {
                exec_ops(lock.set_mode(0, true));
            } else {
                d3d::show_cursor(true);
                exec_ops(lock.set_mode(3, true));
            }
            if let Some(f) = frame
                && let Some(mut m) = matrix_of(f.object)
            {
                let s = camera_distance([m[12], m[13], m[14]]) * objedit::SCALE_PER_METRE;
                if f.ped != 0
                    && ed.mode == 0
                    && let Some(b) = crate::worldhooks::bone_matrix(f.ped, f.bone)
                {
                    m[..12].copy_from_slice(&b[..12]);
                }
                ed.drawn = r.editor_gizmo(ed, &m, s);
            }
            if !ed.dragging {
                ed.hover(cursor, lock.mode());
            }
        }
        let sel = &mut sh.selection;
        if sel.active {
            if sprint {
                exec_ops(lock.set_mode(0, true));
            } else {
                d3d::show_cursor(true);
                exec_ops(lock.set_mode(3, false));
            }
            sel.hovered = 0xFFFF;
            if lock.mode() != 0
                && let Some((start, end)) = pick_ray(cursor)
                && let Some((id, ptr, model)) = crate::objects::pick(start, end, &sh.pickable)
                && let Some(m) = matrix_of(ptr)
            {
                let d = camera_distance([m[12], m[13], m[14]]);
                if d < objedit::PICK_DISTANCE {
                    sel.hovered = id;
                    r.select_highlight(&m, objedit::select_label(id, model, d).as_bytes());
                }
            }
        }
    });
}

/// Editor / selection window messages (B 0x10073580, 0x1006DE00, and the ESC `WM_CHAR` checks of samp's window
/// procedure B 0x100618A1 / 0x100618FA). Returns whether the message is consumed.
pub(super) fn editor_message(msg: u32, w: usize) -> bool {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ClipCursor, GetCursorPos, SetCursorPos, ShowCursor};
    const WM_CHAR: u32 = 0x102;
    let mut screen = POINT { x: 0, y: 0 };
    // SAFETY: plain Win32 query.
    unsafe { GetCursorPos(&mut screen) };
    let client = render::cursor_pos();
    let hwnd = render::hwnd();
    let mut deselect = false;
    let (consumed, fx) = objedit::with(|sh| {
        if sh.editor.active {
            let fx = sh.editor.on_mouse(msg, (screen.x, screen.y), client);
            if fx.consumed {
                return (true, Some(fx));
            }
            if msg == WM_CHAR && w == 0x1B {
                sh.editor.stop(0);
                return (true, None);
            }
        }
        if sh.selection.active {
            if msg == objedit::WM_LBUTTONUP {
                if sh.selection.hovered != 0xFFFF {
                    let id = sh.selection.hovered;
                    sh.editor.actions.push(objedit::Action::Select { id });
                }
                return (true, None);
            }
            if msg == WM_CHAR && w == 0x1B {
                sh.selection.active = false;
                deselect = true;
                return (true, None);
            }
        }
        (false, None)
    });
    if deselect {
        set_input_mode(0, true);
        close_chat_input();
    }
    let Some(fx) = fx else { return consumed };
    // SAFETY: plain Win32 calls on the game window.
    unsafe {
        if fx.capture {
            SetCapture(hwnd as _);
            ShowCursor(0);
            ClipCursor(std::ptr::null());
        }
        if let Some((x, y)) = fx.release_to {
            ReleaseCapture();
            ShowCursor(1);
            let mut p = POINT { x, y };
            ClientToScreen(hwnd as _, &mut p);
            SetCursorPos(p.x, p.y);
        }
        if fx.recentre {
            let (cw, ch) = render::client_size();
            let c = (cw / 2, ch / 2);
            objedit::with(|sh| sh.editor.recentred(c));
            SetCursorPos(c.0, c.1);
        }
    }
    if fx.hover {
        let mode = try_with_ui(|st| st.lock.mode()).unwrap_or(0);
        let client = render::cursor_pos();
        objedit::with(|sh| sh.editor.hover(client, mode));
    }
    consumed
}

/// GTA 1.0 US: `CStreaming::ms_aInfoForModel` (20-byte entries, load state at +0x10).
const STREAMING_INFO: u32 = 0x8E4CC0;
/// `CModelInfo::ms_modelInfoPtrs` (20000 entries).
const MODEL_INFO_PTRS: u32 = 0xA9B0C8;
/// The RenderWare object counter F5 shows as RWObjects.
const RW_OBJECT_COUNT: u32 = 0xB71804;

/// F5 values: memory from the process, GTA's streaming / model tables (vehicle models 400…610
/// with a non-zero load state, null model-info slots among ids 1000…19999, the RW object
/// counter), the rest from what our client counts (pools from the session tick, the network
/// block from our reliability layer).
fn net_stats_values(base: &super::core::NetStats) -> super::core::NetStats {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut s = base.clone();
    let m = ProcessMemory;
    // CStreaming::ms_memoryUsed / ms_memoryAvailable (1.0 US).
    s.streaming_mb = m.read_u32(0x8E4CB4).unwrap_or(0) / (1024 * 1024);
    s.streaming_total_mb = m.read_u32(0x8A5A80).unwrap_or(0) / (1024 * 1024);
    let mut info = vec![0u8; 211 * 20];
    if m.read(STREAMING_INFO + 400 * 20, &mut info).is_ok() {
        s.vehicle_models_loaded = info.as_chunks::<20>().0.iter().filter(|e| e[0x10..0x14] != [0; 4]).count() as u32;
    }
    s.rw_objects = m.read_u32(RW_OBJECT_COUNT).unwrap_or(0);
    let mut ptrs = vec![0u8; 19000 * 4];
    if m.read(MODEL_INFO_PTRS + 1000 * 4, &mut ptrs).is_ok() {
        s.free_ides = ptrs.as_chunks::<4>().0.iter().filter(|p| **p == [0; 4]).count() as u32;
    }
    // SAFETY: plain Win32 queries into correctly sized structs.
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            s.process_kb = (pmc.PagefileUsage / 1024) as u32;
            s.working_set_kb = (pmc.WorkingSetSize / 1024) as u32;
        }
        let mut ms: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of_val(&ms) as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut ms) != 0 {
            s.system_mb = (ms.ullTotalPhys / (1024 * 1024)) as u32;
        }
    }
    s
}

/// `GetAsyncKeyState(vk) != 0` as samp's Present pass tests it (either bit).
fn key_polled(vk: i32) -> bool {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) != 0 }
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
pub(crate) struct GameExpander;

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
    /// `/save` and `/rs` (A 0x10068a00 / 0x10068b80): in a vehicle (ped flags bit 8) the vehicle's
    /// matrix position, `GET_CAR_HEADING` (0x0174), model and colours +0x434/+0x435; on foot the
    /// ped's matrix position and `GET_CHAR_HEADING` (0x0172). Both commands read the same values.
    fn position(&self) -> Option<PlayerPosition> {
        let mem = ProcessMemory;
        let p = game::read_player(&mem).ok().flatten()?;
        let in_vehicle = mem.read_u32(p.ptr + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        let veh = mem.read_u32(p.ptr + PED_VEHICLE).unwrap_or(0);
        let mut r = Runner::new();
        if in_vehicle && veh != 0 {
            let m = mem.read_u32(veh + off::PLACEABLE_MATRIX).unwrap_or(0);
            let rd = |a: u32| mem.read_f32(a).unwrap_or(0.0);
            let pos = if m != 0 { [rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)] } else { p.pos };
            r.run(OP_GET_CAR_HEADING, &[Arg::Int(crate::vehicles::pool_ref(veh)), Arg::Local(0)]);
            return Some(PlayerPosition::InCar {
                model: u32::from(mem.read_u16(veh + off::ENTITY_MODEL).unwrap_or(0)),
                x: pos[0],
                y: pos[1],
                z: pos[2],
                heading: r.local_f32(0),
                colour1: u32::from(mem.read_u8(veh + VEHICLE_COLOUR1).unwrap_or(0)),
                colour2: u32::from(mem.read_u8(veh + VEHICLE_COLOUR2).unwrap_or(0)),
            });
        }
        let handle = game::ped_handle(&mem, p.ptr).ok().flatten().unwrap_or(-1);
        r.run(OP_GET_CHAR_HEADING, &[Arg::Int(handle), Arg::Local(0)]);
        Some(PlayerPosition::OnFoot {
            skin: u32::from(mem.read_u16(p.ptr + off::ENTITY_MODEL).unwrap_or(0)),
            x: p.pos[0],
            y: p.pos[1],
            z: p.pos[2],
            heading: r.local_f32(0),
        })
    }
}

/// Creates the UI state on first use (main thread).
fn ensure(g: &mut Option<State>, debug_mode: bool) -> &mut State {
    g.get_or_insert_with(|| {
        let dir = user_dir();
        crate::log!("ui: user files directory {:?}", dir);
        let core = UiCore::new(dir, debug_mode);
        // samp clears the HUD scale fix flag only for `nohudscalefix` exactly 1 (A 0x100C49DD).
        crate::mainhooks::set_enabled(core.cfg.get_int("nohudscalefix") != Some(1));
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
            last_scoreboard_click: None,
            class_buttons: None,
            class_panel_shown: false,
            spawn_release: false,
            netgame_object: false,
            net_stats: super::core::NetStats::default(),
            net_bytes: (0, 0),
            rates: RateSampler::default(),
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

/// B 0x100e9d64, type 8 (B 0x100680b0).
const DIRECT_MODE_LINE: &str = "ChatWindow: Using direct drawing mode.";

/// The rest of samp's first-frame boot routine after the UI objects (B 0x100c4e2c..0x100c4efa):
/// `multicore` = 0 → process affinity mask 1; the audio object and BASS start-up
/// (`audioproxyoff`). `fpslimit` and `disableheadmove` are read from the settings directly.
fn boot_settings(st: &mut State) {
    if !st.core.settings.multicore {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessAffinityMask};
        // SAFETY: plain Win32 calls on our own process.
        let ok = unsafe { SetProcessAffinityMask(GetCurrentProcess(), 1) };
        crate::log!("ui: multicore=0 -> SetProcessAffinityMask(1) = {ok}");
    }
    let core = &mut st.core;
    crate::audio::init(|| core.audio_proxy_key());
}

/// Before `Render2dStuff` (samp's pre-step A 0x100c4fb0, after the netgame tick).
pub fn frame_entry(debug_mode: bool) {
    let _ = catch_unwind(AssertUnwindSafe(|| frame_entry_inner(debug_mode)));
}

fn frame_entry_inner(debug_mode: bool) {
    let mut g = UI.lock().unwrap_or_else(|e| e.into_inner());
    let st = ensure(&mut g, debug_mode);
    flush_pending_debug(st);
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
    // samp creates its netgame object on the game frame after the UI objects (A 0x100c47d0).
    if st.renderer.is_some() {
        st.netgame_object = true;
    }
    // First-frame UI construction (A 0x100c47d0), inside GTA's scene like samp's.
    if st.device_proxy && st.renderer.is_none() && !st.renderer_failed {
        let face =
            st.core.settings.font_face.clone().unwrap_or_else(|| samp_ui::fonts::DEFAULT_FACE.into()).into_bytes();
        let weight = samp_ui::fonts::weight(st.core.settings.font_weight);
        let direct = st.core.settings.direct_mode;
        match Renderer::new(&face, weight, st.core.settings.font_size, direct) {
            Ok(r) => {
                // Chat construction (B 0x10068190): `directmode` prints its line instead of
                // creating the render target, before the netgame's "Started".
                if direct {
                    st.core.print_debug(DIRECT_MODE_LINE.as_bytes());
                }
                st.renderer = Some(r);
                // The model-preview camera set (A 0x1006ccc0, created in the same first-frame pass).
                super::preview::init_env();
                boot_settings(st);
            }
            Err(e) => {
                crate::log!("ui: renderer unavailable: {e}");
                st.renderer_failed = true;
            }
        }
    }
    for line in std::mem::take(&mut st.say_queue) {
        let game = LiveGame { connected: st.core.connected };
        st.core.say(&line, &game);
    }
    if let Some((style, time, text)) = st.core.requests.game_text.take() {
        show_game_text(st, style, time, &text);
    }
    // Class selection locks input from the game frame (measured: its mode-2 requests come
    // from samp's game-frame code, inside GTA's scene).
    if st.core.class_selection && !st.core.scoreboard.is_open() {
        exec_ops(st.lock.request_mode2());
    }
    // A 0x10006100: while a dialog is open (dialog +0x28) nothing else happens; otherwise the first such frame makes
    // the panel visible. It stays visible when a dialog opens later and goes with the class selection (A 0x10003c20).
    if !st.core.class_selection {
        st.class_panel_shown = false;
    } else if !st.core.dialogs.is_open() {
        st.class_panel_shown = true;
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
    let mut previews = Vec::new();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut g) = UI.try_lock() else { return };
        let Some(st) = g.as_mut() else { return };
        let menu = gta_menu_active();
        if !st.core.settings.direct_mode
            && let Some(r) = st.renderer.as_mut()
        {
            r.update_chat_rt(&mut st.core.chat, menu);
        }
        // Model previews after CGame::Process (A 0x100a27d0 → A 0x1001e870): netgame and textdraw pool present.
        if st.core.netgame {
            previews = super::preview::pending(&st.core.textdraws, super::textdraws::sprite_count());
        }
    }));
    let _ = catch_unwind(AssertUnwindSafe(|| super::preview::render_pending(previews)));
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
        let ops = try_with_ui(|st| {
            if let Some(r) = st.renderer.as_mut() {
                r.on_lost();
            }
            crate::plates::on_lost();
            // Lost device (A 0x100c3f90): an open chat input is closed (A 0x10069580: cursor mode 0,
            // immediate) and an open scoreboard too (A 0x1006e9e0(1): cursor mode 0, delayed).
            let mut ops = Vec::new();
            if st.core.input.is_open() {
                st.core.input.close();
                ops.extend(st.lock.request_mode0(true));
            }
            if st.core.scoreboard.is_open() {
                st.core.scoreboard.close();
                ops.extend(st.lock.request_mode0(false));
            }
            ops
        });
        if let Some(ops) = ops {
            exec_ops(ops);
        }
        // Then the "SetCursor(NULL) once" flag of WM_SETCURSOR is re-armed (store at A 0x100c419b).
        super::wndproc::on_lost_device();
        crate::ui::mattext::on_lost();
        // Last in samp's lost-device pass (A 0x100c41e5): the model-preview textures.
        super::preview::on_lost();
    }));
    let hr = d3d::call_reset(params);
    crate::crash::note(crate::crash::crumb::DEVICE_RESET, 0, hr as u32);
    crate::log!("ui: device Reset -> {hr:#x}");
    if hr >= 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            super::wndproc::on_reset_device();
            let ops = try_with_ui(|st| {
                crate::plates::on_reset();
                if let Some(r) = st.renderer.as_mut() {
                    r.on_reset();
                    // The chat render target is re-created through the same routine (B 0x10068190).
                    if st.core.settings.direct_mode {
                        st.core.print_debug(DIRECT_MODE_LINE.as_bytes());
                    }
                }
                // Last step of A 0x100c41f0: unless the GTA menu is active, cursor mode 2, then mode 0
                // (immediate) with the input restore run at once (A 0x100c43ff..0x100c4431).
                if ProcessMemory.read_u32(GTA_MENU_ACTIVE).unwrap_or(0) == 0 {
                    let mut ops = st.lock.set_mode(2, false);
                    ops.extend(st.lock.restore_now(true));
                    ops
                } else {
                    Vec::new()
                }
            });
            if let Some(ops) = ops {
                exec_ops(ops);
            }
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
        // A 0x10074270: only the driver (samp state 0x13) of an RC vehicle (A 0x100b7e40: valid
        // pool handle and model 441/464/465/501/564/594) is anchored at the vehicle's matrix
        // position; every other player, drivers of ordinary cars included, needs to be in the
        // moving list (A 0x1009f430, `CPhysical` +0xB4 ≠ 0) and is anchored at bone 8 (head).
        let vehicle = if r.state == crate::vehicle::state::DRIVER {
            let v = ProcessMemory.read_u32(r.ped + PED_VEHICLE).unwrap_or(0);
            let model = ped_valid(v)
                .then(|| ProcessMemory.read_u16(v + off::ENTITY_MODEL).ok())
                .flatten()
                .map(u32::from);
            if crate::vehicle::tag_at_vehicle(r.state, model) { v } else { 0 }
        } else {
            0
        };
        let anchor = if vehicle != 0 {
            let Some(v) = ped_pos(vehicle) else { continue };
            v
        } else {
            if ProcessMemory.read_u32(r.ped + 0xB4).unwrap_or(0) == 0 {
                continue;
            }
            bone_pos(r.ped, 8)
        };
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

/// Pause-menu map right-click (B 0x10075B7B, session-misc.md "119"): with the map page shown (0xBA6774), the
/// right button's "pressed since the last call" bit set and 0xBA8298 clear, the radar-trace entries 1..=175
/// (0xBA86F0, stride 0x28) are scanned for the u16 0x1203 at +0x25 (the waypoint; the last match wins) and its x/y
/// go out with z = `CWorld::FindGroundZForCoord(x, y)` + 2. samp sends uninitialised x/y without a waypoint;
/// we send nothing then (ROADMAP decision, feata-notes.md).
fn map_click() -> Option<[f32; 3]> {
    use crate::worldhooks::rd;
    if rd::<u32>(MENU_MAP_PAGE) == 0 {
        return None;
    }
    // SAFETY: plain Win32 query.
    let pressed = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x02) } & 1 != 0;
    if !pressed || rd::<u8>(MENU_MAP_BLOCK) != 0 {
        return None;
    }
    let mut found = None;
    for i in 1..=RADAR_TRACE_SCAN as u32 {
        let e = RADAR_TRACE + i * RADAR_TRACE_STRIDE;
        if rd::<u16>(e + 0x25) == WAYPOINT_TAG {
            found = Some((rd::<f32>(e + 8), rd::<f32>(e + 0xC)));
        }
    }
    let (x, y) = found?;
    // SAFETY: CWorld::FindGroundZForCoord(float, float) (cdecl, returns float), main thread.
    let z = unsafe {
        let f: extern "C" fn(f32, f32) -> f32 = std::mem::transmute(FIND_GROUND_Z_FOR_COORD as usize);
        f(x, y)
    };
    Some([x, y, z + 2.0])
}

const MENU_MAP_PAGE: u32 = 0xBA6774;
const MENU_MAP_BLOCK: u32 = 0xBA8298;
const RADAR_TRACE: u32 = 0xBA86F0;
const RADAR_TRACE_STRIDE: u32 = 0x28;
const RADAR_TRACE_SCAN: usize = 175;
const WAYPOINT_TAG: u16 = 0x1203;
const FIND_GROUND_Z_FOR_COORD: u32 = 0x569660;

/// UR §2.3: menu check, HUD flags, then the drawing steps.
fn present_pass() {
    let Ok(mut g) = UI.try_lock() else { return };
    let Some(st) = g.as_mut() else { return };
    // Step 1: F8 screenshot (A 0x10075620), before anything of this frame is drawn.
    if std::mem::take(&mut st.core.requests.screenshot) {
        let (path, index) = st.core.screenshot_target();
        let ok = d3d::screenshot(path.to_string_lossy().as_bytes(), render::hwnd());
        st.core.screenshot_done(ok.then_some(index));
    }
    // Step 3: GTA pause menu → no samp UI; only the map right-click (B 0x10075B7B).
    if gta_menu_active() {
        if st.core.netgame
            && let Some(p) = map_click()
        {
            // samp sends from this pass and RakNet's own thread transmits it; our session tick does not run
            // while the menu is up, so send and flush here (queued for the next tick if the session is busy).
            if !crate::dll::send_click_map(p) {
                st.core.push_outgoing(Outgoing::ClickMap(p));
            }
        }
        return;
    }
    let face = st.core.settings.font_face.clone().unwrap_or_else(|| samp_ui::fonts::DEFAULT_FACE.into()).into_bytes();
    let weight = samp_ui::fonts::weight(st.core.settings.font_weight);
    let fs = st.core.settings.font_size;
    for req in std::mem::take(&mut st.core.requests.cursor) {
        let ops = match req {
            CursorRequest::Mode0 { immediate } => st.lock.request_mode0(immediate),
            CursorRequest::Mode3 => st.lock.request_mode3(),
        };
        exec_ops(ops);
    }
    // Steps 5–8 (A 0x10075790): scoreboard, else F5 (connected) / F10 held → HUD off and
    // only the nametag passes; else F1 held opens the help dialog. Keys are polled with
    // GetAsyncKeyState (any bit set), each only when the earlier ones did not apply.
    let sb_open = st.core.scoreboard.is_open();
    let mut overlay = None;
    if st.core.netgame && !sb_open {
        if key_polled(0x74) && st.core.connected {
            overlay = Some(Overlay::NetStats);
        } else if key_polled(0x79) {
            overlay = Some(Overlay::F10);
        } else if key_polled(0x70) {
            st.core.open_help();
        }
    }
    // Step 5/9 HUD (TRACE-3 U-HUD-STEP9: written every Present).
    set_hud(!sb_open && overlay.is_none() && st.core.chat.mode() != DisplayMode::Hidden);
    let scoreboard_shown = sb_open;
    let sb_open = sb_open || overlay.is_some();
    let tags = nametags(st);
    let bubbles = if sb_open { Vec::new() } else { bubbles(st) };
    // Step 4b: the audio stream title line (B 0x10066a70), when the netgame exists.
    let audio_title = if st.core.netgame { crate::audio::title_text() } else { None };
    let net_stats = (overlay == Some(Overlay::NetStats)).then(|| {
        let (sent, received) = st.net_bytes;
        let (down, up) = st.rates.sample(std::time::Instant::now(), sent, received);
        let mut base = st.net_stats.clone();
        base.download_kbs = down;
        base.upload_kbs = up;
        super::core::net_stats_text(&net_stats_values(&base))
    });
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
    let sb_draw = scoreboard_shown.then(|| ScoreboardDraw {
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
        overlay,
        net_stats: net_stats.as_deref(),
        netgame_object: st.core.netgame && st.netgame_object,
        name_tag_passes: st.core.netgame && st.netgame_object && st.core.show_name_tags,
        audio_title: audio_title.as_deref(),
    };
    let lock = &mut st.lock;
    r.frame(&frame, &mut |req| match req {
        CursorReq::Mode2 => exec_ops(lock.request_mode2()),
    });
    // Step 12: the class-selection GUI dialog after the chat GUI dialog (UR §2.3, §4.2),
    // with cursor mode 2 while it is shown (TRACE-3: class selection locks input).
    if st.core.class_selection && st.class_panel_shown && !sb_open {
        st.class_buttons = Some(r.class_panel(cursor));
    } else {
        st.class_buttons = None;
    }
    // Step 12: object editor and object selection (B 0x100737B0, 0x1006DBC0).
    if !sb_open {
        editor_pass(&mut st.lock, r);
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
        let immediate = !std::mem::take(&mut st.spawn_release);
        exec_ops(st.lock.request_mode0(immediate));
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
        let consumed = st.core.handle_event(ev, menu, &game, wheel_lines);
        clipboard_requests(st);
        consumed
    })
    .unwrap_or(false)
}

/// DXUT edit-box clipboard: copy (`CF_UNICODETEXT` from the ANSI selection) and paste
/// (`CF_UNICODETEXT` converted to the ANSI code page).
fn clipboard_requests(st: &mut State) {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar, WideCharToMultiByte};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    if let Some(sel) = st.core.requests.clipboard_copy.take() {
        // SAFETY: Win32 clipboard sequence; the global block is handed to the clipboard.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                EmptyClipboard();
                let n = MultiByteToWideChar(CP_ACP, 0, sel.as_ptr(), sel.len() as i32, std::ptr::null_mut(), 0);
                let h = GlobalAlloc(GMEM_MOVEABLE, (n as usize + 1) * 2);
                if !h.is_null() {
                    let p = GlobalLock(h) as *mut u16;
                    if !p.is_null() {
                        MultiByteToWideChar(CP_ACP, 0, sel.as_ptr(), sel.len() as i32, p, n);
                        *p.add(n as usize) = 0;
                        GlobalUnlock(h);
                        SetClipboardData(CF_UNICODETEXT, h as _);
                    }
                }
                CloseClipboard();
            }
        }
    }
    if std::mem::take(&mut st.core.requests.paste) {
        let mut text = Vec::new();
        // SAFETY: as above; the clipboard keeps ownership of its data.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                let h = GetClipboardData(CF_UNICODETEXT);
                if !h.is_null() {
                    let p = GlobalLock(h as _) as *const u16;
                    if !p.is_null() {
                        let mut len = 0usize;
                        while *p.add(len) != 0 {
                            len += 1;
                        }
                        let n = WideCharToMultiByte(CP_ACP, 0, p, len as i32, std::ptr::null_mut(), 0, std::ptr::null(), std::ptr::null_mut());
                        text.resize(n.max(0) as usize, 0);
                        WideCharToMultiByte(CP_ACP, 0, p, len as i32, text.as_mut_ptr(), n, std::ptr::null(), std::ptr::null_mut());
                        GlobalUnlock(h as _);
                    }
                }
                CloseClipboard();
            }
        }
        st.core.paste(&text);
    }
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
    if st.core.scoreboard.is_open() {
        return scoreboard_click(st, x, y);
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

/// A click on the scoreboard's list box (DXUT list box: a click selects the row, a double click on it activates
/// it → the scoreboard's GUI callback, B 0x100C5590 on event 0x701, sends RPC 23 and closes; B 0x1006EBD0).
/// Clicks outside the scoreboard fall through (they close it, keys.rs).
fn scoreboard_click(st: &mut State, x: i32, y: i32) -> bool {
    let r = st.core.scoreboard_rect;
    if !r.contains(x, y) {
        return false;
    }
    let row_h = st.renderer.as_ref().map(|r| r.fonts.gui1_height).unwrap_or(0);
    let list = samp_ui::Rect::new(r.left, r.top + 60, r.right, r.bottom);
    let sel = samp_ui::dialog::list_selection_rect(&list);
    let text = samp_ui::dialog::list_text_rect(&list);
    if row_h <= 0 || x < sel.left || x >= sel.right || y < text.top || y >= sel.bottom {
        return true;
    }
    let row = st.core.scoreboard.scroll() + ((y - text.top) / row_h) as usize;
    if row >= st.core.scoreboard.rows().len() {
        return true;
    }
    let now = std::time::Instant::now();
    // SAFETY: plain Win32 query.
    let dbl = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    let double = st
        .last_scoreboard_click
        .is_some_and(|(r, t)| r == row && now.duration_since(t).as_millis() <= u128::from(dbl));
    st.core.scoreboard.set_selected(Some(row));
    if double {
        st.last_scoreboard_click = None;
        st.core.scoreboard_activate();
    } else {
        st.last_scoreboard_click = Some((row, now));
    }
    true
}

/// A click on the class-selection panel: 1 = `<<`, 2 = `>>`, 3 = `Spawn` (0 = none).
static CLASS_CLICK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Takes the pending class-selection panel click (session tick).
pub fn take_class_click() -> u8 {
    CLASS_CLICK.swap(0, std::sync::atomic::Ordering::SeqCst)
}

/// Edit-box key-down; true if the input or a dialog edit box has focus (consumed).
pub(super) fn on_edit_key(key: samp_ui::edit::EditKey, mods: samp_ui::edit::Mods) -> bool {
    try_with_ui(|st| {
        let r = st.core.edit_key(key, mods);
        clipboard_requests(st);
        r
    })
    .unwrap_or(false)
}

// ---- network session hooks (client.rs) ------------------------------------------------

/// The netgame is created (Client::start).
pub fn on_netgame_start(host: &str, port: u16, name: &str) {
    with_ui(|st| st.core.on_netgame_start(host, port, name.as_bytes()));
}

/// The configured frame limit (`fpslimit`, 20..90), if the UI exists and is not busy.
pub fn fps_limit() -> Option<u32> {
    try_with_ui(|st| st.core.settings.fps_limit).flatten()
}

/// Spawn (A 0x10003c20): the mode-0 request that ends class selection's input lock
/// uses the delayed (10-frame) restore.
pub fn class_selection_release() {
    let _ = with_ui(|st| st.spawn_release = true);
}

/// A type-8 (debug colour) chat line, e.g. samp's "Warning(…)" lines.
pub fn print_debug(text: &str) {
    with_ui(|st| st.core.print_debug(text.as_bytes()));
}

/// Debug chat lines raised while the UI state is locked (renderer construction, the object pre-step): printed
/// at the end of the next frame entry, in order.
static PENDING_DEBUG: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn print_debug_later(text: String) {
    PENDING_DEBUG.lock().unwrap_or_else(|e| e.into_inner()).push(text);
}

fn flush_pending_debug(st: &mut State) {
    let lines = std::mem::take(&mut *PENDING_DEBUG.lock().unwrap_or_else(|e| e.into_inner()));
    for l in lines {
        st.core.print_debug(l.as_bytes());
    }
}

/// A type-4 (info colour) chat line.
/// `/ctd` is on (B flag 0x101189A8).
pub fn camera_target_labels_on() -> bool {
    try_with_ui(|st| st.core.settings.camera_target_labels).unwrap_or(false)
}

/// `/ctd` labels, drawn from the local tick inside GTA's scene like samp's (B 0x10004830).
pub fn draw_camera_target_labels(items: &[([f32; 16], Vec<u8>)]) {
    let _ = try_with_ui(|st| {
        if st.core.settings.camera_target_labels
            && let Some(r) = st.renderer.as_ref()
        {
            r.camera_target_labels(items);
        }
    });
}

pub fn print_info(text: &[u8]) {
    with_ui(|st| st.core.print_info(text));
}

/// `audiomsgoff` (sa-mp.cfg / `/audiomsg`).
pub fn audio_messages_off() -> bool {
    with_ui(|st| st.core.settings.audio_messages_off).unwrap_or(false)
}

/// `disableheadmove` (sa-mp.cfg / `/headmove`): samp's Game+0x59 head-tracking flag is off.
/// `/dl` vehicle debug labels on (samp's flag B 0x1026EA30).
pub fn vehicle_labels_on() -> bool {
    try_with_ui(|st| st.core.settings.vehicle_labels).unwrap_or(false)
}

pub fn head_move_disabled() -> bool {
    try_with_ui(|st| st.core.settings.head_move_disabled).unwrap_or(false)
}

/// RPC 139 InitGame.
pub fn on_init_game(g: &samp_proto::session::InitGame) {
    with_ui(|st| {
        st.net_stats.send_rate = g.on_foot_rate;
        // samp's lag-comp selector (A 0x10118990: InitGame 1 → 0, 2 → 1, else 2).
        st.net_stats.shot_comp_mode = u32::from(crate::combat::lag_comp_selector(g.lag_compensation));
        st.core.on_init_game(g)
    });
}

/// RPC 40 / lost connection reset (B 0x1000a530): pools and server settings.
pub fn on_reset() {
    with_ui(|st| st.core.on_reset());
}

/// RPC 40 reset (A 0x100A72C0): the player colour table back to the defaults.
pub fn reset_colours() {
    with_ui(|st| st.core.colours = super::colours::PlayerColours::new());
}

/// Lost connection: every remote player is deleted from the pool.
pub fn forget_players() {
    with_ui(|st| st.core.forget_players());
}

/// Every S→C RPC, before the game handlers.
pub fn on_rpc(r: &Incoming) {
    with_ui(|st| st.core.on_rpc(r, &GameExpander));
}

/// F5 pool block (A 0x10060d20), once per session tick: vehicles in the pool, distinct models
/// among those with a GTA vehicle, remote ped slots in use, objects in the pool.
pub fn set_pool_stats(vehicles: u32, vehicle_models: u32, ped_slots: u32, objects: u32) {
    let _ = try_with_ui(|st| {
        let s = &mut st.net_stats;
        s.vehicles = vehicles;
        s.vehicle_models = vehicle_models;
        s.ped_slots_used = ped_slots;
        s.object_slots_used = objects;
    });
}

/// Remote players for nametags, bubbles and the scoreboard (once per session tick).
pub fn set_remotes(remotes: Vec<super::core::RemoteUi>) {
    let _ = try_with_ui(|st| st.remotes = remotes);
}

/// Class selection state (the scoreboard does not open during it).
pub fn set_class_selection(active: bool) {
    let _ = try_with_ui(|st| st.core.class_selection = active);
}

/// A dialog is open (samp's dialog object +0x28, A 0x1026e9c8): class selection's tick does nothing
/// while it is (A 0x10006100).
pub fn dialog_open() -> bool {
    with_ui(|st| st.core.dialogs.is_open()).unwrap_or(false)
}

/// The chat input is open (chat input object +0x14e0, A 0x1026e9fc): Shift does not request a spawn.
pub fn chat_input_open() -> bool {
    with_ui(|st| st.core.input.is_open()).unwrap_or(false)
}

/// Sends the UI's queued messages now. The original sends them from its window procedure (a
/// dialog's RPC 62 when Enter is released, chat lines, clicks), i.e. before the frame's local
/// tick: the session tick calls this before the local player's part, `net_tick` again at its end.
pub fn send_outgoing(net: &mut NetClient) {
    let out = with_ui(|st| st.core.take_outgoing()).unwrap_or_default();
    for o in out {
        send_one(net, o);
    }
}

/// Once per session tick: connection lines and the UI's outgoing messages.
pub fn net_tick(net: &mut NetClient) {
    let c = net.counters();
    let l = net.link_stats().unwrap_or_default();
    let _ = with_ui(|st| {
        st.net_bytes = (c.bytes_sent, c.bytes_received);
        let s = &mut st.net_stats;
        s.messages_in_send_buffer = l.messages_in_send_buffer;
        s.messages_sent = l.messages_sent as u32;
        s.bytes_sent = (l.bits_sent / 8) as u32;
        s.acks_sent = l.acks_sent as u32;
        s.acks_in_send_buffer = l.acks_in_send_buffer;
        s.messages_waiting_for_ack = l.messages_waiting_for_ack;
        s.messages_resent = l.messages_resent as u32;
        s.bytes_resent = (l.bits_resent / 8) as u32;
        s.packet_loss = if l.bits_sent == 0 { 0.0 } else { l.bits_resent as f32 * 100.0 / l.bits_sent as f32 };
        s.messages_received = l.messages_received as u32;
        s.bytes_received = c.bytes_received as u32;
        s.acks_received = l.acks_received as u32;
        s.duplicate_acks_received = l.duplicate_acks_received as u32;
    });
    send_outgoing(net);
}

fn send_one(net: &mut NetClient, o: Outgoing) {
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
        Outgoing::ClickPlayer(player_id) => {
            crate::log!("ui: RPC 23 player {player_id}");
            let p = samp_proto::rpcs::session::OnPlayerClickPlayer {
                player_id,
                source: samp_proto::rpcs::session::OnPlayerClickPlayer::SOURCE_SCOREBOARD,
            };
            net.send_rpc(&p, Reliability::ReliableOrdered);
        }
        Outgoing::ClickMap(pos) => {
            crate::log!("ui: RPC 119 {pos:?}");
            let p = samp_proto::rpcs::session::OnPlayerClickMap {
                pos: samp_proto::types::Vec3 { x: pos[0], y: pos[1], z: pos[2] },
            };
            net.send_rpc(&p, Reliability::Reliable);
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

/// `bubbles`: the chat bubbles held per player (DIFF-3).
pub fn control_bubbles() -> String {
    try_with_ui(|st| st.core.bubbles_report()).unwrap_or_else(|| "error ui busy".into())
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
