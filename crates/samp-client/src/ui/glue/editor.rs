//! The object editor and object selection: per-frame work at the end of the Present pass, and their
//! window messages.

use std::sync::Mutex;

use super::adapters::{camera_pos, dist};
use super::{close_chat_input, exec_ops, set_input_mode, try_with_ui};
use crate::game::off;
use crate::objedit;
use crate::ui::d3d;
use crate::ui::inputlock::InputLock;
use crate::ui::render::{self, Renderer};
use crate::worldhooks::rd;

/// Offset of pad 0's `Cross` button state in `CPad`.
const PAD_CROSS: u32 = 0x20;

/// Pad 0 `Cross`, the sprint key (samp tests the sprint bit of its on-foot key word).
fn sprint_held() -> bool {
    rd::<i16>(crate::game::PAD0 + PAD_CROSS) != 0
}

fn matrix_of(ent: u32) -> Option<[f32; 16]> {
    let m: u32 = rd(ent + off::PLACEABLE_MATRIX);
    (m != 0).then(|| std::array::from_fn(|i| rd(m + 4 * i as u32)))
}

fn camera_distance(p: [f32; 3]) -> f32 {
    dist(p, camera_pos())
}

/// The cursor ray of the object-selection pick, from our copies of the WORLD, VIEW and PROJECTION transforms.
fn pick_ray(cursor: (i32, i32)) -> Option<([f32; 3], [f32; 3])> {
    let lock = |m: &Mutex<[f32; 16]>| *m.lock().unwrap_or_else(|e| e.into_inner());
    let (world, view, proj) = (lock(&d3d::WORLD), lock(&d3d::VIEW), lock(&d3d::PROJECTION));
    let inv = objedit::mat_inverse(&objedit::mat_mul(&world, &view))?;
    let size = (render::video_width().max(1) as u32, render::video_height().max(1) as u32);
    Some(objedit::pick_ray(cursor, size, &proj, &inv))
}

/// Per-frame work of the object editor (gizmo, hover), then of object selection (pick and highlight).
pub(super) fn editor_pass(lock: &mut InputLock, r: &Renderer) {
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

/// Window messages for the object editor and object selection, including ESC (`WM_CHAR` 0x1B), which ends
/// either. Returns whether the message is consumed.
pub(in crate::ui) fn editor_message(msg: u32, w: usize) -> bool {
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
