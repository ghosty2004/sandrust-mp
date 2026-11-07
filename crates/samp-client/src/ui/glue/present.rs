//! The Present and Reset wrappers of the device proxy, and samp's per-frame Present pass.

use std::panic::{AssertUnwindSafe, catch_unwind};

use samp_ui::chat::DisplayMode;
use samp_ui::keys::Overlay;

use super::adapters::{GTA_MENU_ACTIVE, gta_menu_active, key_polled, net_stats_values, set_hud};
use super::editor::editor_pass;
use super::frame::DIRECT_MODE_LINE;
use super::overlays::{bubbles, map_click, nametags};
use super::{State, UI, exec_ops, try_with_ui};
use crate::patch::{Memory, ProcessMemory};
use crate::ui::core::{CursorRequest, Outgoing};
use crate::ui::d3d::{self, HResult, Rect};
use crate::ui::render::{self, BubbleDraw, CursorReq, Frame, ScoreboardDraw, TagDraw};

pub(super) extern "system" fn present_hook(
    _this: *mut std::ffi::c_void,
    a: *const Rect,
    b: *const Rect,
    c: isize,
    d: *const std::ffi::c_void,
) -> HResult {
    let _ = catch_unwind(present_pass);
    d3d::call_present(a, b, c, d)
}

pub(super) extern "system" fn reset_hook(_this: *mut std::ffi::c_void, params: *mut std::ffi::c_void) -> HResult {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let ops = try_with_ui(|st| {
            if let Some(r) = st.renderer.as_mut() {
                r.on_lost();
            }
            crate::plates::on_lost();
            // Lost device: an open chat input is closed (cursor mode 0, immediate restore) and so is an open
            // scoreboard (cursor mode 0, delayed restore).
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
        // Then the "SetCursor(NULL) once" flag of WM_SETCURSOR is re-armed.
        crate::ui::wndproc::on_lost_device();
        crate::ui::mattext::on_lost();
        // Last in samp's lost-device pass: the model-preview textures.
        crate::ui::preview::on_lost();
    }));
    let hr = d3d::call_reset(params);
    crate::crash::note(crate::crash::crumb::DEVICE_RESET, 0, hr as u32);
    crate::log!("ui: device Reset -> {hr:#x}");
    if hr >= 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            crate::ui::wndproc::on_reset_device();
            let ops = try_with_ui(|st| {
                crate::plates::on_reset();
                if let Some(r) = st.renderer.as_mut() {
                    r.on_reset();
                    // samp re-creates the chat render target through its construction routine, which prints
                    // the direct-mode line again.
                    if st.core.settings.direct_mode {
                        st.core.print_debug(DIRECT_MODE_LINE.as_bytes());
                    }
                }
                // Last step of samp's reset handling: unless the GTA menu is active, cursor mode 2, then mode 0
                // (immediate) with the input restore run at once.
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

/// What the Present pass gathers before it draws.
struct PassInput {
    overlay: Option<Overlay>,
    /// The scoreboard is open (it is drawn this frame).
    scoreboard_shown: bool,
    /// The scoreboard or an F5 / F10 overlay hides the rest of the UI.
    sb_open: bool,
    tags: Vec<TagDraw>,
    bubbles: Vec<BubbleDraw>,
    audio_title: Option<Vec<u8>>,
    net_stats: Option<Vec<u8>>,
}

/// samp's per-frame Present pass: screenshot, menu check, HUD flags, then the drawing steps in samp's order
/// (docs/re/subsystems/ui-rendering.md, "2.3 Present: per-frame draw order"; steps below use its numbering).
fn present_pass() {
    let Ok(mut g) = UI.try_lock() else { return };
    let Some(st) = g.as_mut() else { return };
    // Step 1: F8 screenshot, before anything of this frame is drawn.
    take_screenshot(st);
    // Step 3: GTA pause menu → no samp UI; only the map right-click.
    if gta_menu_active() {
        pause_menu_map_click(st);
        return;
    }
    let face = st.core.settings.font_face.clone().unwrap_or_else(|| samp_ui::fonts::DEFAULT_FACE.into()).into_bytes();
    let weight = samp_ui::fonts::weight(st.core.settings.font_weight);
    let fs = st.core.settings.font_size;
    run_cursor_requests(st);
    let input = gather(st);
    if st.renderer.is_none() {
        return;
    }
    update_layouts(st, &face, weight, fs);
    let cursor = render::cursor_pos();
    draw_frame(st, &input, cursor);
    after_frame(st, input.sb_open, cursor);
}

/// Step 1: the F8 screenshot.
fn take_screenshot(st: &mut State) {
    if std::mem::take(&mut st.core.requests.screenshot) {
        let (path, index) = st.core.screenshot_target();
        let ok = d3d::screenshot(path.to_string_lossy().as_bytes(), render::hwnd());
        st.core.screenshot_done(ok.then_some(index));
    }
}

/// Step 3: with the GTA pause menu up, only the map right-click (RPC 119).
fn pause_menu_map_click(st: &mut State) {
    if st.core.netgame
        && let Some(p) = map_click()
    {
        // samp sends from this pass and RakNet's own thread transmits it; our session tick does not run
        // while the menu is up, so send and flush here (queued for the next tick if the session is busy).
        if !crate::dll::send_click_map(p) {
            st.core.push_outgoing(Outgoing::ClickMap(p));
        }
    }
}

/// The cursor-mode requests queued by input and RPC handling, in order.
fn run_cursor_requests(st: &mut State) {
    for req in std::mem::take(&mut st.core.requests.cursor) {
        let ops = match req {
            CursorRequest::Mode0 { immediate } => st.lock.request_mode0(immediate),
            CursorRequest::Mode3 => st.lock.request_mode3(),
        };
        exec_ops(ops);
    }
}

/// Steps 4b–9 up to the drawing: overlay keys, HUD flags, nametags, bubbles, the audio title and the F5 text.
fn gather(st: &mut State) -> PassInput {
    // Steps 5–8: scoreboard, else F5 (connected) / F10 held → HUD off and
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
    // Steps 5 / 9: the HUD flags are written on every Present.
    set_hud(!sb_open && overlay.is_none() && st.core.chat.mode() != DisplayMode::Hidden);
    let scoreboard_shown = sb_open;
    let sb_open = sb_open || overlay.is_some();
    let tags = nametags(st);
    let bubbles = if sb_open { Vec::new() } else { bubbles(st) };
    // Step 4b: the audio stream title line, when the netgame exists.
    let audio_title = if st.core.netgame { crate::audio::title_text() } else { None };
    let net_stats = (overlay == Some(Overlay::NetStats)).then(|| net_stats_text(st));
    PassInput { overlay, scoreboard_shown, sb_open, tags, bubbles, audio_title, net_stats }
}

/// The F5 text with the download / upload rates sampled now.
fn net_stats_text(st: &mut State) -> Vec<u8> {
    let (sent, received) = st.net_bytes;
    let (down, up) = st.rates.sample(std::time::Instant::now(), sent, received);
    let mut base = st.net_stats.clone();
    base.download_kbs = down;
    base.upload_kbs = up;
    crate::ui::core::net_stats_text(&net_stats_values(&base))
}

/// Font rebuild when asked or when the size changed, the dialog layout (once per open), the chat input's first
/// visible character and the click rects of the input and the scoreboard.
fn update_layouts(st: &mut State, face: &[u8], weight: u32, fs: i32) {
    let Some(r) = st.renderer.as_mut() else { return };
    if std::mem::take(&mut st.core.requests.rebuild_fonts) || r.needs_rebuild(fs) {
        r.rebuild_fonts(face, weight, fs);
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
        st.core.input_rect = samp_ui::input::box_rect(cw, st.core.chat.bottom(r.fonts.main_h), r.fonts.gui0_height);
    } else {
        st.input_caret = None;
    }
    let (w, h) = (render::video_width(), render::video_height());
    let (sw, shh) = samp_ui::scoreboard::size(w);
    st.core.scoreboard_rect = samp_ui::Rect::xywh(w / 2 - sw / 2, h / 2 - shh / 2, sw, shh);
}

/// Steps 4–12: the renderer's frame (chat, nametags, bubbles, scoreboard, death window, dialog, overlays).
fn draw_frame(st: &mut State, input: &PassInput, cursor: (i32, i32)) {
    let Some(r) = st.renderer.as_mut() else { return };
    let sb_draw = input.scoreboard_shown.then(|| ScoreboardDraw {
        rows: st.core.scoreboard.rows(),
        server_name: &st.core.server_name,
        players: samp_ui::scoreboard::players_text(st.core.remote_ids().count()),
        selected: st.core.scoreboard.selected(),
        scroll: st.core.scoreboard.scroll(),
    });
    let dialog = if st.core.dialogs.drawn(st.core.input.is_open(), input.sb_open) {
        st.core.dialogs.current().zip(st.dialog_layout.as_ref().map(|(_, l)| l))
    } else {
        None
    };
    let frame = Frame {
        chat: &st.core.chat,
        input: &st.core.input,
        input_first_visible: st.input_first_visible,
        tags: &input.tags,
        nametag_status: st.core.settings.nametag_status,
        bubbles: &input.bubbles,
        scoreboard: sb_draw,
        death: st.core.deaths.visible().then_some(&st.core.deaths),
        dialog,
        cursor,
        overlay: input.overlay,
        net_stats: input.net_stats.as_deref(),
        netgame_object: st.core.netgame && st.netgame_object,
        name_tag_passes: st.core.netgame && st.netgame_object && st.core.show_name_tags,
        audio_title: input.audio_title.as_deref(),
    };
    let lock = &mut st.lock;
    r.frame(&frame, &mut |req| match req {
        CursorReq::Mode2 => exec_ops(lock.request_mode2()),
    });
}

/// Step 12 after the frame: the class-selection panel, the object editor and selection, textdraw selection,
/// then leaving cursor mode 2 when no mode-2 UI is left.
fn after_frame(st: &mut State, sb_open: bool, cursor: (i32, i32)) {
    let Some(r) = st.renderer.as_mut() else { return };
    // Step 12: the class-selection GUI dialog after the chat GUI dialog (spec "4.2 GUI dialogs created by
    // samp"); class selection holds cursor mode 2 meanwhile.
    if st.core.class_selection && st.class_panel_shown && !sb_open {
        st.class_buttons = Some(r.class_panel(cursor));
    } else {
        st.class_buttons = None;
    }
    // Step 12: object editor and object selection.
    if !sb_open {
        editor_pass(&mut st.lock, r);
    }
    // End of step 12: textdraw selection per frame (spec "14.4 Selection mode").
    if st.core.textdraws.selection.active && !sb_open {
        // Cursor mode 2, HUD off, then the hover test.
        exec_ops(st.lock.request_mode2());
        set_hud(false);
        st.core.textdraws.hover(cursor.0, cursor.1);
    }
    // Leaving a mode-2 UI: back to mode 0 with an immediate restore, except after the spawn (delayed restore).
    let wants2 = st.core.input.is_open()
        || st.core.class_selection
        || st.core.dialogs.is_open()
        || st.core.textdraws.selection.active;
    if !wants2 && st.lock.mode() == 2 {
        let immediate = !std::mem::take(&mut st.spawn_release);
        exec_ops(st.lock.request_mode0(immediate));
    }
}
