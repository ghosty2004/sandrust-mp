//! Subclass of the GTA window procedure (Windows only).
//!
//! [`install`] replaces the game window's procedure with `SetWindowLongA`. Keyboard, mouse-wheel, click and
//! focus messages are translated into `samp_ui::keys::Event`s and routed to the UI in `glue`; messages the
//! UI does not consume go on to GTA's procedure. Spec: docs/re/subsystems/ui-rendering.md, "6. Window
//! procedure and keys".

use std::ffi::c_void;
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use samp_ui::edit::{EditKey, Mods};
use samp_ui::keys::{Event, Vk};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CallWindowProcA, GCL_STYLE, GWL_WNDPROC, GetClassLongA, SPI_GETWHEELSCROLLLINES, SetClassLongA,
    SetCursor, SetWindowLongA, SetWindowTextA, SystemParametersInfoA, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS,
    WM_LBUTTONUP, WM_MOUSEWHEEL, WM_SETCURSOR, WM_SYSKEYUP, WNDPROC,
};

use crate::patch::{Memory, ProcessMemory};

/// GTA main window handle.
const GTA_HWND: u32 = 0xC97C1C;

static OLD_PROC: AtomicUsize = AtomicUsize::new(0);
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn install() -> Result<(), String> {
    let hwnd = ProcessMemory.read_u32(GTA_HWND).unwrap_or(0) as usize as HWND;
    // SAFETY: SetWindowLongA and the class / cursor calls on our own process's window.
    unsafe {
        if hwnd.is_null() {
            return Err("no GTA window yet".into());
        }
        let old = SetWindowLongA(hwnd, GWL_WNDPROC, window_proc as *const () as usize as i32);
        if old == 0 {
            return Err("SetWindowLongA failed".into());
        }
        OLD_PROC.store(old as u32 as usize, Ordering::SeqCst);
        // Like samp's window setup: the class gets CS_DBLCLKS (the second click of a double click arrives as
        // WM_LBUTTONDBLCLK) and the title becomes "GTA:SA:MP".
        let style = GetClassLongA(hwnd, GCL_STYLE);
        SetClassLongA(hwnd, GCL_STYLE, (style | CS_DBLCLKS) as i32);
        SetWindowTextA(hwnd, WINDOW_TITLE.as_ptr());
    }
    Ok(())
}

/// Called on device loss: re-arms the "SetCursor(NULL) once" flag of the `WM_SETCURSOR` answer, so the next
/// `WM_SETCURSOR` hides the Windows cursor again. Mirrors samp.dll 0x100C3F90.
pub fn on_lost_device() {
    CURSOR_HIDDEN.store(false, Ordering::SeqCst);
}

/// Called after a device reset; like samp's handler, its first action is `SetCursor(NULL)`.
/// Mirrors samp.dll 0x100C41F0.
pub fn on_reset_device() {
    // SAFETY: plain Win32 call.
    unsafe { SetCursor(std::ptr::null_mut()) };
}

/// Window title set at start-up (string at samp.dll B 0x10061EF4).
const WINDOW_TITLE: &[u8] = b"GTA:SA:MP\0";

fn key_down(vk: i32) -> bool {
    // SAFETY: plain Win32 query.
    (unsafe { GetKeyState(vk) } as u16) & 0x8000 != 0
}

fn wheel_lines() -> u32 {
    let mut n: u32 = 3;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes one UINT.
    unsafe { SystemParametersInfoA(SPI_GETWHEELSCROLLLINES, 0, &mut n as *mut u32 as *mut c_void, 0) };
    n
}

/// Edit-box keys handled on key-down by the DXUT edit box while it has focus.
fn edit_key(vk: u32) -> Option<EditKey> {
    Some(match vk {
        0x25 => EditKey::Left,
        0x27 => EditKey::Right,
        0x24 => EditKey::Home,
        0x23 => EditKey::End,
        0x2E => EditKey::Delete,
        0x2D => EditKey::Insert,
        _ => return None,
    })
}

/// Returns true if the UI consumed the message.
fn translate(msg: u32, w: WPARAM, l: LPARAM) -> bool {
    let route = |ev| super::glue::on_window_event(ev, 0);
    // Object editor / selection first: samp's window procedure checks them before the chat input.
    if matches!(msg, 0x200..=0x202 | WM_CHAR) && super::glue::editor_message(msg, w) {
        return true;
    }
    match msg {
        WM_CHAR => route(Event::Char(w as u8)),
        WM_KEYDOWN => {
            if let Some(k) = edit_key(w as u32) {
                let mods = Mods { shift: key_down(0x10), ctrl: key_down(0x11) };
                // Not known whether samp also passes these keys on to GTA; we consume them when handled.
                if super::glue::on_edit_key(k, mods) {
                    return true;
                }
            }
            route(Event::KeyDown(Vk::from_code(w as u32)))
        }
        WM_KEYUP => route(Event::KeyUp(Vk::from_code(w as u32))),
        WM_SYSKEYUP => route(Event::SysKeyUp(Vk::from_code(w as u32))),
        WM_MOUSEWHEEL => {
            let delta = ((w >> 16) & 0xFFFF) as u16 as i16 as i32;
            super::glue::on_window_event(Event::Wheel(delta / 120), wheel_lines())
        }
        WM_LBUTTONUP => {
            let x = (l & 0xFFFF) as u16 as i16 as i32;
            let y = ((l >> 16) & 0xFFFF) as u16 as i16 as i32;
            route(Event::LeftButtonUp { x, y })
        }
        WM_KILLFOCUS => route(Event::KillFocus),
        _ => false,
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_SETCURSOR {
        // Hide the Windows cursor once, then always answer TRUE (ui-rendering.md, "5. Cursor and
        // input-lock modes").
        if !CURSOR_HIDDEN.swap(true, Ordering::SeqCst) {
            // SAFETY: plain Win32 call.
            unsafe { SetCursor(std::ptr::null_mut()) };
        }
        return 1;
    }
    if catch_unwind(|| translate(msg, w, l)).unwrap_or(false) {
        return 0;
    }
    let old = OLD_PROC.load(Ordering::SeqCst);
    // SAFETY: `old` is the procedure SetWindowLongA returned.
    unsafe {
        let p: WNDPROC = std::mem::transmute::<usize, WNDPROC>(old);
        CallWindowProcA(p, hwnd, msg, w, l)
    }
}
