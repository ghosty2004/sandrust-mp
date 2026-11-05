//! `window` control command (FEAT-B): the top-level windows of the game process as one line, so a
//! scenario can compare what both clients show (game window title and class style, the crash and
//! message-box dialogs with their control ids and texts).
//!
//! * `window` — `window N | hwnd=0x… class="…" title="…" style=0x… cstyle=0x… [ctl ID "CLASS" "TEXT" …] | …`
//!   (`cstyle` = the window class style, GCL_STYLE; control texts with CR/LF escaped as `\r` / `\n`).
//! * `window click ID` — posts `WM_COMMAND(ID, BN_CLICKED)` to every dialog (`#32770`) of the process that has a
//!   control `ID`. Refused for any button whose text mentions "Report": the original's crash dialog uploads a
//!   report to a third-party site from that button, which no test may ever press.

use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, EnumChildWindows, EnumWindows, GCL_STYLE, GWL_STYLE, GetClassLongA, GetClassNameA, GetDlgCtrlID,
    GetDlgItem, GetWindowLongA, GetWindowTextA, GetWindowThreadProcessId, IsWindowVisible, PostMessageA, WM_COMMAND,
};

fn text(h: HWND) -> String {
    let mut b = vec![0u8; 4096];
    // SAFETY: valid buffer of the given length.
    let n = unsafe { GetWindowTextA(h, b.as_mut_ptr(), b.len() as i32) };
    b.truncate(n.max(0) as usize);
    String::from_utf8_lossy(&b).replace('\r', "\\r").replace('\n', "\\n").replace('"', "'")
}

fn class(h: HWND) -> String {
    let mut b = [0u8; 256];
    // SAFETY: valid buffer.
    let n = unsafe { GetClassNameA(h, b.as_mut_ptr(), b.len() as i32) };
    String::from_utf8_lossy(&b[..n.max(0) as usize]).into_owned()
}

fn top_level() -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> i32 {
        let mut pid = 0u32;
        // SAFETY: `l` is the Vec passed below; the window handle comes from the system.
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid == GetCurrentProcessId() {
                (*(l as *mut Vec<HWND>)).push(h);
            }
        }
        TRUE
    }
    let mut v: Vec<HWND> = Vec::new();
    // SAFETY: callback only pushes into `v`, which outlives the call.
    unsafe { EnumWindows(Some(cb), &mut v as *mut Vec<HWND> as LPARAM) };
    v
}

fn children(h: HWND) -> Vec<HWND> {
    unsafe extern "system" fn cb(c: HWND, l: LPARAM) -> i32 {
        // SAFETY: `l` is the Vec passed below.
        unsafe { (*(l as *mut Vec<HWND>)).push(c) };
        TRUE
    }
    let mut v: Vec<HWND> = Vec::new();
    // SAFETY: as above.
    unsafe { EnumChildWindows(h, Some(cb), &mut v as *mut Vec<HWND> as LPARAM) };
    v
}

fn describe() -> String {
    let wins = top_level();
    let mut out = format!("window {}", wins.len());
    for h in wins {
        // SAFETY: plain queries on a window of this process.
        let (style, cstyle, visible) = unsafe {
            (GetWindowLongA(h, GWL_STYLE) as u32, GetClassLongA(h, GCL_STYLE), IsWindowVisible(h) != 0)
        };
        out += &format!(
            " | hwnd={:#x} class=\"{}\" title=\"{}\" style={style:#x} cstyle={cstyle:#x} visible={}",
            h as usize,
            class(h),
            text(h),
            u8::from(visible)
        );
        if class(h) == "#32770" {
            for c in children(h) {
                // SAFETY: as above.
                let id = unsafe { GetDlgCtrlID(c) };
                out += &format!(" [ctl {id} \"{}\" \"{}\"]", class(c), text(c));
            }
        }
    }
    out
}

fn click(id: i32) -> String {
    let mut sent = 0;
    for h in top_level() {
        if class(h) != "#32770" {
            continue;
        }
        // SAFETY: plain query.
        let c = unsafe { GetDlgItem(h, id) };
        if c.is_null() {
            continue;
        }
        if text(c).to_ascii_lowercase().contains("report") {
            return format!("error refused: control {id} is a report button");
        }
        // SAFETY: posting a button click to a dialog of this process.
        unsafe { PostMessageA(h, WM_COMMAND, ((BN_CLICKED as usize) << 16) | id as u16 as usize, c as LPARAM) };
        sent += 1;
    }
    format!("ok click {id} sent to {sent} dialog(s)")
}

pub fn command(args: &str) -> String {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => describe(),
        ["click", id] => match id.parse::<i32>() {
            Ok(id) => click(id),
            Err(_) => "error usage: window [click ID]".into(),
        },
        _ => "error usage: window [click ID]".into(),
    }
}
