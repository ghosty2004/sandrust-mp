//! Global UI state and the entry points used by `dll` and `client` (Windows only).
//!
//! Entry points: `frame_entry` / `frame_after_2d` / `game_frame` (game-frame hooks), the Present and Reset
//! wrappers, window messages, and the network-session hooks (`on_rpc`, `net_tick`, …). Everything runs on the
//! game's main thread except the control channel, which may run on the background thread: it only reads state
//! or queues work. This module holds the state and the input-lock plumbing; `frame`, `present`, `input`,
//! `session` and `control` hold the entry points, `overlays`, `editor` and `adapters` what they draw and read.
//! Spec: docs/re/subsystems/ui-rendering.md and ui-chat-audio.md; addresses in docs/re/addresses/ui-glue.md.

use std::path::PathBuf;
use std::sync::Mutex;

use super::core::UiCore;
use super::d3d;
use super::inputlock::{InputLock, Op};
use super::render::Renderer;
use crate::patch::{Memory, ProcessMemory};
use crate::script::Runner;

mod adapters;
mod control;
mod editor;
mod frame;
mod input;
mod overlays;
mod present;
mod session;

pub(crate) use adapters::GameExpander;
pub use control::*;
pub(in crate::ui) use editor::editor_message;
pub use frame::{frame_after_2d, frame_entry, game_frame, preload};
pub use input::take_class_click;
pub(in crate::ui) use input::{on_edit_key, on_window_event};
pub use session::*;

/// GTA user files directory (`InitUserDirectories`), a C string.
const GTA_USER_DIR: u32 = 0xC92368;

struct State {
    core: UiCore,
    renderer: Option<Renderer>,
    renderer_failed: bool,
    lock: InputLock,
    runner: Runner,
    /// samp's 512-byte game-text buffer; GTA keeps the pointer, so it must outlive the message.
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
    /// The class-selection panel was made visible. samp's class-selection step shows it on the first frame without
    /// an open dialog and keeps it until the spawn.
    class_panel_shown: bool,
    /// Last click on a dialog list row (double-click detection).
    last_list_click: Option<(usize, std::time::Instant)>,
    /// Last scoreboard row click (double-click detection).
    last_scoreboard_click: Option<(usize, std::time::Instant)>,
    /// The spawn ends class selection: its mode-0 request restores input only after 10 frames
    /// (docs/phase3/parity-notes.md).
    spawn_release: bool,
    /// samp's netgame object exists (from the game frame after the UI construction).
    netgame_object: bool,
    /// F5 network-statistics counters published by the session tick (`net_tick`).
    net_stats: super::core::NetStats,
    /// Wire byte totals (sent, received) published by the session tick.
    net_bytes: (u64, u64),
    /// The F5 rate sampler: totals and deltas of the last sample.
    rates: RateSampler,
}

/// The F5 download/upload rates: while the overlay draws, once more than 1000 ms have passed since the last
/// sample, the byte deltas since that sample are taken; each rate is the stored delta / 1024 (KB per sample)
/// until the next sample. Mirrors samp.dll 0x10060D20.
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

// SAFETY: the raw D3D pointers inside are used only on the main thread; other threads touch plain data under the mutex.
unsafe impl Send for State {}

static UI: Mutex<Option<State>> = Mutex::new(None);

fn user_dir() -> Option<PathBuf> {
    let mut buf = [0u8; 260];
    let _ = ProcessMemory.read(GTA_USER_DIR, &mut buf);
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    // Like samp: the current directory when GTA has no user directory, otherwise `<user files>\SAMP`,
    // created with its `screens` folder if missing.
    if n == 0 {
        return std::env::current_dir().ok();
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&buf[..n]).into_owned()).join("SAMP");
    let _ = std::fs::create_dir(&dir);
    let _ = std::fs::create_dir(dir.join("screens"));
    Some(dir)
}

/// Carries out the input lock's operations: memory writes, CPad helper calls, cursor visibility and the mouse
/// redirect.
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

/// Input-lock mode 4 points GTA's mouse writes here instead of the real mouse state; GTA writes floats into it.
static mut MOUSE_SINK: [u32; 4] = [0; 4];

/// samp's input-lock setter for other subsystems (object editing / selection): `mode` 0–4, `flag` = immediate
/// restore for mode 0. Runs the operations at once (main thread). Mirrors samp.dll 0x100A0750.
pub fn set_input_mode(mode: u8, flag: bool) {
    if let Some(ops) = with_ui(|st| st.lock.set_mode(mode, flag)) {
        exec_ops(ops);
    }
}

/// Object-editor start: input-lock mode 0 with the restore run at once.
pub fn restore_input_now(flag: bool) {
    if let Some(ops) = with_ui(|st| st.lock.restore_now(flag)) {
        exec_ops(ops);
    }
}

/// Paints a number-plate texture for the vehicle pool pass; 0 when the renderer is not up.
pub fn paint_plate(text: &[u8]) -> u32 {
    with_ui(|st| st.renderer.as_mut().map(|r| r.paint_plate(text))).flatten().unwrap_or(0)
}

/// RPC 27 / 117: turns textdraw selection off when it is active.
pub fn cancel_textdraw_selection() {
    with_ui(|st| {
        if st.core.textdraws.selection.active {
            st.core.cancel_textdraw_select();
        }
    });
}

/// Closes the chat input when the object editor or object selection ends.
pub fn close_chat_input() {
    with_ui(|st| {
        if st.core.input.is_open() {
            st.core.input.close();
        }
    });
}

/// Creates the UI state on first use (main thread).
fn ensure(g: &mut Option<State>, debug_mode: bool) -> &mut State {
    g.get_or_insert_with(|| {
        let dir = user_dir();
        crate::log!("ui: user files directory {:?}", dir);
        let core = UiCore::new(dir, debug_mode);
        // samp turns the HUD scale fix off only when `nohudscalefix` is exactly 1.
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
