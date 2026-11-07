//! Game-frame hooks: start-up, the per-frame entry before `Render2dStuff` (first-frame UI construction,
//! queued work, class selection, menus, scoreboard), the step after it, and the `CGame::Process` hook.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::adapters::{LiveGame, gta_menu_active};
use super::present::{present_hook, reset_hook};
use super::session::flush_pending_debug;
use super::{State, UI, ensure, exec_ops};
use crate::script::Arg;
use crate::ui::core::{MenuRequest, Outgoing};
use crate::ui::d3d;
use crate::ui::render::Renderer;

/// `CMessages::AddBigMessage(text, time, style)`, used for game text.
const ADD_BIG_MESSAGE: u32 = 0x69F2B0;
/// Script opcodes (gta-reversed `eScriptCommands.h`).
const OP_CLEAR_PRINTS: u16 = 0x00BE;
const OP_DELETE_MENU: u16 = 0x08DA;
const OP_GET_MENU_ITEM_SELECTED: u16 = 0x08D7;

/// At start-up (init thread): load d3dx9_25.dll and register the SA:MP fonts.
pub fn preload() {
    // SAFETY: LoadLibraryA with a NUL-terminated name; the module stays loaded.
    let h = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryA(d3d::D3DX_DLL.as_ptr()) };
    crate::log!("ui: d3dx9_25.dll {}", if h.is_null() { "NOT loaded" } else { "loaded" });
    d3d::add_font_resources();
}

/// The debug-colour chat line samp prints in `directmode`.
pub(super) const DIRECT_MODE_LINE: &str = "ChatWindow: Using direct drawing mode.";

/// The rest of samp's first-frame boot routine after the UI objects: `multicore` = 0 sets the process affinity
/// mask to 1; then the audio object and BASS start-up (`audioproxyoff`). `fpslimit` and `disableheadmove` are
/// read from the settings where they are used.
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

/// Per-frame entry before `Render2dStuff`, after the netgame tick: hooks, first-frame UI construction, queued
/// work, class-selection input lock, menus and the scoreboard. Mirrors samp.dll 0x100C4FB0.
pub fn frame_entry(debug_mode: bool) {
    let _ = catch_unwind(AssertUnwindSafe(|| frame_entry_inner(debug_mode)));
}

fn frame_entry_inner(debug_mode: bool) {
    let mut g = UI.lock().unwrap_or_else(|e| e.into_inner());
    let st = ensure(&mut g, debug_mode);
    flush_pending_debug(st);
    if !st.fonts_added {
        st.fonts_added = true;
        match crate::ui::menus::install_gxt_hook() {
            Ok(()) => crate::log!("ui: CText::Get hook installed"),
            Err(e) => crate::log!("ui: CText::Get hook NOT installed: {e}"),
        }
    }
    if !st.wndproc {
        st.wndproc = true;
        match crate::ui::wndproc::install() {
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
    // samp creates its netgame object on the game frame after the UI objects.
    if st.renderer.is_some() {
        st.netgame_object = true;
    }
    // First-frame UI construction, inside GTA's scene like samp's.
    if st.device_proxy && st.renderer.is_none() && !st.renderer_failed {
        let face =
            st.core.settings.font_face.clone().unwrap_or_else(|| samp_ui::fonts::DEFAULT_FACE.into()).into_bytes();
        let weight = samp_ui::fonts::weight(st.core.settings.font_weight);
        let direct = st.core.settings.direct_mode;
        match Renderer::new(&face, weight, st.core.settings.font_size, direct) {
            Ok(r) => {
                // Chat construction: in `directmode` samp prints this line instead of creating the
                // render target, before the netgame's "Started" line.
                if direct {
                    st.core.print_debug(DIRECT_MODE_LINE.as_bytes());
                }
                st.renderer = Some(r);
                // The model-preview camera set is created in the same first-frame pass.
                crate::ui::preview::init_env();
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
    // Class selection locks input from the game frame: samp's mode-2 requests for it come from its
    // game-frame code, inside GTA's scene.
    if st.core.class_selection && !st.core.scoreboard.is_open() {
        exec_ops(st.lock.request_mode2());
    }
    // While a dialog is open the class-selection step does nothing; otherwise its first frame makes the panel
    // visible. It stays visible when a dialog opens later and goes away with the class selection.
    if !st.core.class_selection {
        st.class_panel_shown = false;
    } else if !st.core.dialogs.is_open() {
        st.class_panel_shown = true;
    }
    menu_work(st);
    // Scoreboard: rows rebuilt on open and when the player count changes; RPC 155 on the opening frame, then
    // every 3000 ms (docs/re/subsystems/ui-rendering.md, "15.3 Behaviour").
    let count = st.core.remote_ids().count();
    let (rebuild, send) = st.core.scoreboard.frame(u64::from(crate::ui::core::tick_ms()), count);
    if rebuild {
        st.core.rebuild_scoreboard();
    }
    if send {
        st.core.push_outgoing(Outgoing::ScoresRequest);
    }
    st.core.bubbles.expire(crate::ui::core::tick_ms());
    let quit = st.core.requests.quit.take();
    drop(g);
    if let Some(disconnect) = quit {
        crate::log!("ui: /quit (disconnect={disconnect})");
        crate::dll::request_quit();
    }
}

/// After `Render2dStuff`: textdraws, then the input-restore step
/// (docs/re/subsystems/ui-rendering.md, "2.4 Per-frame game entry").
pub fn frame_after_2d() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut g) = UI.try_lock() else { return };
        let Some(st) = g.as_mut() else { return };
        if !gta_menu_active() && st.core.netgame && !st.core.scoreboard.is_open() {
            crate::ui::textdraws::draw_all(&mut st.core.textdraws);
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

/// From the `CGame::Process` hook: the chat render-target update, then the model previews still to render
/// (docs/re/subsystems/ui-rendering.md, "7.6 Render-to-texture cache" and "14.3a Model previews (font 5)").
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
        // Model previews after CGame::Process, only once the netgame (and so the textdraw pool) exists.
        if st.core.netgame {
            previews = crate::ui::preview::pending(&st.core.textdraws, crate::ui::textdraws::sprite_count());
        }
    }));
    let _ = catch_unwind(AssertUnwindSafe(|| crate::ui::preview::render_pending(previews)));
}

/// Menus: build / delete requests and the selection poll (docs/re/subsystems/ui-chat-audio.md, RPCs 77, 78,
/// 132 and 140).
fn menu_work(st: &mut State) {
    use samp_ui::menu::PollAction;
    for req in std::mem::take(&mut st.core.requests.menus) {
        match req {
            MenuRequest::Show { delete_previous, build } => {
                if delete_previous && let Some(h) = st.menu_handle.take() {
                    st.runner.run(OP_DELETE_MENU, &[Arg::Int(h)]);
                }
                crate::ui::menus::set_gxt(st.core.menus.current_menu());
                st.menu_handle = crate::ui::menus::build(&mut st.runner, &build);
            }
            MenuRequest::Delete => {
                if let Some(h) = st.menu_handle.take() {
                    st.runner.run(OP_DELETE_MENU, &[Arg::Int(h)]);
                }
            }
        }
    }
    let (cross, triangle) = crate::ui::menus::pad_buttons();
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

/// RPC 73 SendGameText: clear prints, copy into the long-lived buffer, `AddBigMessage`.
fn show_game_text(st: &mut State, style: u8, time: u32, text: &[u8]) {
    st.runner.run(OP_CLEAR_PRINTS, &[]);
    let n = text.len().min(st.game_text.len() - 1);
    st.game_text[..n].copy_from_slice(&text[..n]);
    st.game_text[n] = 0;
    // SAFETY: GTA's cdecl `CMessages::AddBigMessage(char*, uint32, uint16)`; the buffer lives in the UI state, which GTA's kept pointer needs.
    unsafe {
        let f: extern "C" fn(*const u8, u32, u32) = std::mem::transmute(ADD_BIG_MESSAGE as usize);
        f(st.game_text.as_ptr(), time, u32::from(style));
    }
    crate::log!("ui: game text style {style} {time} ms {:?}", String::from_utf8_lossy(text));
}
