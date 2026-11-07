//! Window messages for the UI: routed key and mouse events, clicks on dialogs, the scoreboard and the
//! class-selection panel, and the edit-box clipboard.

use super::adapters::{LiveGame, gta_menu_active};
use super::{State, try_with_ui};
use crate::ui::core::Outgoing;

/// A window event for the UI; returns whether it is consumed.
pub(in crate::ui) fn on_window_event(ev: samp_ui::keys::Event, wheel_lines: u32) -> bool {
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
                        let n = WideCharToMultiByte(
                            CP_ACP,
                            0,
                            p,
                            len as i32,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null(),
                            std::ptr::null_mut(),
                        );
                        text.resize(n.max(0) as usize, 0);
                        WideCharToMultiByte(
                            CP_ACP,
                            0,
                            p,
                            len as i32,
                            text.as_mut_ptr(),
                            n,
                            std::ptr::null(),
                            std::ptr::null_mut(),
                        );
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
        // RPC 83 OnPlayerSelectTextDraw with the hovered id; selection stays on.
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
            // A click selects; a double click (two clicks on the same row within the system double-click
            // time, DXUT's WM_LBUTTONDBLCLK) activates with response 1.
            let now = std::time::Instant::now();
            // SAFETY: plain Win32 query.
            let dbl = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
            let double = st
                .last_list_click
                .is_some_and(|(r, t)| r == row && now.duration_since(t).as_millis() <= u128::from(dbl));
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

/// A click on the scoreboard's list box: a click selects the row; a double click activates it, and the
/// scoreboard's GUI callback sends RPC 23 OnPlayerClickPlayer and closes. Clicks outside the scoreboard fall
/// through (they close it, see `samp_ui::keys`).
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
    let double =
        st.last_scoreboard_click.is_some_and(|(r, t)| r == row && now.duration_since(t).as_millis() <= u128::from(dbl));
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
pub(in crate::ui) fn on_edit_key(key: samp_ui::edit::EditKey, mods: samp_ui::edit::Mods) -> bool {
    try_with_ui(|st| {
        let r = st.core.edit_key(key, mods);
        clipboard_requests(st);
        r
    })
    .unwrap_or(false)
}
