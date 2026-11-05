//! Object editor and object selection (RPC 27/28/116/117): state, gizmo layout and input rules.
//!
//! Static source: samp.dll B (the build the lanes run). Editor object (global 0x1026E9D8, 0x117 bytes, ctor
//! B 0x10071B90): per-frame draw B 0x100737B0, gizmo B 0x10072C60, handle painter B 0x10072570, hover test
//! B 0x100720D0, mouse handler B 0x10073580, drag step B 0x100734B0, object apply B 0x10073240, attached apply
//! B 0x10072240, update/final send B 0x100728E0, stop B 0x10072BF0. Selection object (global 0x1026E9DC, ctor B 0x1006D9C0): per-frame
//! B 0x1006DBC0, pick B 0x1006D9D0 → pool ray test B 0x10012E40, highlight B 0x1006DA20, click B 0x1006DE00.
//!
//! Measured on the original (run `orig-20261105-042300`, docs/phase3/m6p-notes.md): the handle glyphs, colours
//! and rects of the D3D stream, 117 response 2 only while the mouse moves during a drag (≥ 250 ms apart), the
//! position lagging behind the drag (the object is moved with MoveObject at speed 1), no 116 during an
//! attached-object drag, 116/117 response 1 from the save button, 27 on the click.
//!
//! This module is platform independent; the Windows side (drawing, window messages, GTA calls) lives in
//! `ui::render`, `ui::glue` and `objects`.

use std::sync::Mutex;

/// Handle indices of samp's hover test (B 0x100720D0): 0–2 axes, 3–5 mode buttons, 10 save. The rects are kept
/// in this order (index 10 is the seventh rect).
pub const HANDLE_SAVE: i32 = 10;
pub const NO_HANDLE: i32 = -1;

/// Background glyph of every handle and the glyph-size probe (SAMPAUX3 28, B 0x100E6304).
pub const GLYPH_BACKGROUND: &[u8] = b"C";
/// Axis glyphs per mode (translate, rotate, scale) and axis (B 0x10071FD0).
pub const AXIS_GLYPHS: [[&[u8]; 3]; 3] = [[b"6", b"7", b"8"], [b"3", b"4", b"5"], [b"9", b"A", b"B"]];
/// Button glyphs: translate (3), rotate (4), scale (5), save (10) (B 0x10072701…0x100728B2).
pub const BUTTON_GLYPHS: [(i32, &[u8]); 4] = [(3, b"2"), (4, b"0"), (5, b"1"), (HANDLE_SAVE, b"H")];
/// Line and axis-glyph colours (X, Y, Z).
pub const AXIS_COLOURS: [u32; 3] = [0xFF99_99CC, 0xFFCC_9999, 0xFF99_CC99];
pub const BUTTON_GLYPH_COLOUR: u32 = 0xFF99_9999;
pub const BACKGROUND: u32 = 0x9922_2222;
pub const BACKGROUND_HOVER: u32 = 0xFFAA_AAAA;
pub const BACKGROUND_ACTIVE: u32 = 0xFF77_4444;
/// Glyph-size probe colour (DT_CALCRECT).
pub const PROBE_COLOUR: u32 = 0xFF00_0000;
/// DrawText formats: probe (CALCRECT | NOCLIP | VCENTER | CENTER) and handles (NOCLIP | VCENTER | CENTER).
pub const FMT_PROBE: u32 = 0x505;
pub const FMT_HANDLE: u32 = 0x105;
/// Gizmo half length per metre of camera distance (B 0x100EA740).
pub const SCALE_PER_METRE: f32 = 0.125;
/// Pixels → drag units (B 0x10073546: × 0.01).
pub const PIXEL_UNIT: f32 = 0.01;
/// Minimum interval of 117 response 2 (B 0x100728E0: `GetTickCount() - last > 0xF9`).
pub const UPDATE_INTERVAL_MS: u32 = 250;
/// Selection: pick distance and highlight / accept limit (B 0x1006D9D0, 0x1006DC00: 150 m), axis cross length
/// (B 0x1006DAD1: 1.0) and colour, label colour (B 0x1006DA20).
pub const PICK_DISTANCE: f32 = 150.0;
pub const SELECT_CROSS: f32 = 1.0;
pub const SELECT_LINE_COLOUR: u32 = 0xFFCC_9999;
pub const SELECT_LABEL_COLOUR: u32 = 0xFFD4_7B75;
/// Pick spheres of the ray test (B 0x10012E99: radius 0.25).
pub const PICK_SPHERE_RADIUS: f32 = 0.25;

/// Mouse messages handled by the editor / selection (WM_*).
pub const WM_MOUSEMOVE: u32 = 0x200;
pub const WM_LBUTTONDOWN: u32 = 0x201;
pub const WM_LBUTTONUP: u32 = 0x202;

pub type Rect = [i32; 4];

/// What the editor edits (+0x78: 1 object, 2 attached object).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    None,
    Object {
        id: u16,
        player_object: bool,
    },
    Attached {
        slot: u32,
    },
}

/// Requests for the client tick (it owns the object pool, the script runner and the network).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Drag step on `axis` by `d` units (B 0x10073490).
    Drag { axis: usize, d: f32 },
    /// Button press on an axis handle: copy the object matrix into the edit matrix (B 0x100735F0).
    Press,
    /// End editing with this response (0 cancel, 1 save) (B 0x10072BF0).
    Stop { response: u32 },
    /// Selection click (B 0x1006DC90): send 27 for this object.
    Select { id: u16 },
}

/// What the Windows side does after a message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseEffects {
    pub consumed: bool,
    /// SetCapture + ShowCursor(FALSE) + ClipCursor(NULL).
    pub capture: bool,
    /// ReleaseCapture + ShowCursor(TRUE) + SetCursorPos(client → screen of this point).
    pub release_to: Option<(i32, i32)>,
    /// Re-centre the cursor in the client area after a drag step (B 0x100723F0).
    pub recentre: bool,
    /// Run the hover test again (B 0x100720D0) with the current cursor.
    pub hover: bool,
}

/// The editor object (+0x78…+0x113 of B 0x10071B90).
#[derive(Clone, Debug)]
pub struct Editor {
    pub active: bool,
    pub target: Target,
    /// +0x7C: 0 translate, 1 rotate, 2 scale.
    pub mode: u32,
    /// +0xA3.
    pub dragging: bool,
    /// +0x113.
    pub hovered: i32,
    /// +0x84: the gizmo was drawn this frame.
    pub drawn: bool,
    /// +0x00/+0x04: size of the background glyph.
    pub glyph: (i32, i32),
    /// +0x08 + 16·i: hit rects of handles 0–5 and save.
    pub rects: [Rect; 7],
    /// +0xA4…: drag direction inverted, +0xB7…: the axis is mostly vertical on screen.
    pub invert: [bool; 3],
    pub vertical: [bool; 3],
    /// +0xA7: cursor (screen) at the press / after the last re-centre; +0xAF: cursor (client) at the press.
    pub press: (i32, i32),
    pub press_client: (i32, i32),
    /// +0xA2: the scale button is shown (attached objects).
    pub show_scale: bool,
    /// +0x92: the editor's rotation copy (object mode).
    pub rotation: [f32; 3],
    /// +0x9E: tick of the last 117 send.
    pub last_send_ms: u32,
    pub actions: Vec<Action>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            active: false,
            target: Target::None,
            mode: 0,
            dragging: false,
            hovered: NO_HANDLE,
            drawn: false,
            glyph: (0, 0),
            rects: [[0; 4]; 7],
            invert: [false; 3],
            vertical: [false; 3],
            press: (0, 0),
            press_client: (0, 0),
            show_scale: false,
            rotation: [0.0; 3],
            last_send_ms: 0,
            actions: Vec::new(),
        }
    }
}

/// Selection mode (B 0x1006D9C0: +0 active, +4 hovered id).
#[derive(Clone, Debug)]
pub struct Selection {
    pub active: bool,
    pub hovered: u16,
}

impl Default for Selection {
    fn default() -> Self {
        Self { active: false, hovered: 0xFFFF }
    }
}

/// What the Present pass draws the gizmo for (published by the client tick): the edited GTA object and, for an
/// attached object, the local ped and the slot's bone (translate mode uses the bone's axes, B 0x10073930).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditFrame {
    pub object: u32,
    pub ped: u32,
    pub bone: i32,
}

#[derive(Default)]
pub struct Shared {
    pub editor: Editor,
    pub selection: Selection,
    pub frame: Option<EditFrame>,
    /// Objects of the pool (id, `CObject*`, model) for the selection pick.
    pub pickable: Vec<(u16, u32, i32)>,
}

pub static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Shared::default))
}

fn slot(index: i32) -> Option<usize> {
    match index {
        0..=5 => Some(index as usize),
        HANDLE_SAVE => Some(6),
        _ => None,
    }
}

/// `x / 2` as samp computes it (cdq; sub; sar: rounds toward zero).
fn half(x: i32) -> i32 {
    x / 2
}

impl Editor {
    /// Editor start (B 0x10072450 object, B 0x10072510 attached): mode translate, no drag.
    pub fn start(&mut self, target: Target, rotation: [f32; 3]) {
        self.active = true;
        self.target = target;
        self.set_mode(0);
        self.dragging = false;
        self.show_scale = matches!(target, Target::Attached { .. });
        if let Target::Object { .. } = target {
            self.rotation = rotation;
        }
    }

    /// B 0x10072220: objects have no scale mode.
    pub fn set_mode(&mut self, mode: u32) {
        self.mode = if matches!(self.target, Target::Object { .. }) && mode == 2 { 0 } else { mode };
    }

    /// Background colour of a handle (B 0x10072570).
    pub fn background(&self, index: i32) -> u32 {
        match index {
            0..=2 => {
                if self.hovered == index {
                    if self.dragging { BACKGROUND_ACTIVE } else { BACKGROUND_HOVER }
                } else {
                    BACKGROUND
                }
            }
            3..=5 => {
                if self.mode == (index - 3) as u32 {
                    BACKGROUND_ACTIVE
                } else if self.hovered == index {
                    BACKGROUND_HOVER
                } else {
                    BACKGROUND
                }
            }
            _ => {
                if self.hovered == index {
                    BACKGROUND_HOVER
                } else {
                    BACKGROUND
                }
            }
        }
    }

    /// Glyph and colour of a handle.
    pub fn glyph_of(&self, index: i32) -> (&'static [u8], u32) {
        match index {
            0..=2 => (AXIS_GLYPHS[self.mode.min(2) as usize][index as usize], AXIS_COLOURS[index as usize]),
            _ => (BUTTON_GLYPHS.iter().find(|(i, _)| *i == index).map(|(_, g)| *g).unwrap_or(b""), BUTTON_GLYPH_COLOUR),
        }
    }

    /// Hit rect of a handle drawn at `(x, y)` (B 0x10072070: centred on the point, glyph size).
    pub fn place(&mut self, index: i32, x: i32, y: i32) {
        let (w, h) = self.glyph;
        let l = x - half(w);
        let t = y - half(h);
        if let Some(s) = slot(index) {
            self.rects[s] = [l, t, l + w, t + h];
        }
    }

    pub fn clear_rect(&mut self, index: i32) {
        if let Some(s) = slot(index) {
            self.rects[s] = [0; 4];
        }
    }

    /// Drag direction of an axis from its projected end points `a` (+s) and `b` (−s) (B 0x10072D8F…).
    pub fn set_direction(&mut self, axis: usize, a: [f32; 2], b: [f32; 2]) {
        if self.dragging {
            return;
        }
        let (dx, dy) = ((a[0] - b[0]).abs(), (a[1] - b[1]).abs());
        if dy >= dx {
            self.invert[axis] = a[1] < b[1];
            self.vertical[axis] = true;
        } else {
            self.invert[axis] = a[0] < b[0];
            self.vertical[axis] = false;
        }
    }

    /// Hover test (B 0x100720D0): needs a drawn gizmo and a visible cursor (cursor mode ≠ 0).
    pub fn hover(&mut self, cursor: (i32, i32), cursor_mode: u8) {
        if !self.drawn || cursor_mode == 0 {
            self.hovered = NO_HANDLE;
            return;
        }
        let inside = |r: &Rect| cursor.0 >= r[0] && cursor.0 < r[2] && cursor.1 >= r[1] && cursor.1 < r[3];
        self.hovered = NO_HANDLE;
        for (i, r) in self.rects.iter().enumerate() {
            if inside(r) {
                self.hovered = if i == 6 { HANDLE_SAVE } else { i as i32 };
                return;
            }
        }
    }

    /// Mouse message (B 0x10073580). `cursor` = screen position, `client` = client position.
    pub fn on_mouse(&mut self, msg: u32, cursor: (i32, i32), client: (i32, i32)) -> MouseEffects {
        let mut fx = MouseEffects::default();
        if !self.active {
            return fx;
        }
        match msg {
            WM_LBUTTONDOWN => {
                if !self.dragging && (0..=2).contains(&self.hovered) {
                    self.dragging = true;
                    self.press = cursor;
                    self.press_client = client;
                    self.actions.push(Action::Press);
                    fx.consumed = true;
                    fx.capture = true;
                }
            }
            WM_LBUTTONUP => {
                if self.dragging {
                    self.dragging = false;
                    let to = slot(self.hovered)
                        .filter(|&s| s < 3)
                        .map(|s| (self.rects[s][0], self.rects[s][1]))
                        .unwrap_or(self.press_client);
                    fx.release_to = Some(to);
                    fx.hover = true;
                    fx.consumed = true;
                } else {
                    match self.hovered {
                        3..=5 => {
                            self.set_mode((self.hovered - 3) as u32);
                            fx.consumed = true;
                        }
                        HANDLE_SAVE => {
                            self.stop(1);
                            fx.consumed = true;
                        }
                        _ => {}
                    }
                }
            }
            WM_MOUSEMOVE if self.dragging => {
                if let Some(d) = self.drag_delta(cursor) {
                    let axis = self.hovered as usize;
                    self.actions.push(Action::Drag { axis, d });
                    fx.recentre = true;
                }
                fx.consumed = true;
            }
            _ => {}
        }
        fx
    }

    /// Drag step (B 0x100734B0): the axis' screen direction picks x or y (swapped in rotate mode), the inversion
    /// flag the sign; `None` when the cursor did not move on that coordinate.
    pub fn drag_delta(&self, cursor: (i32, i32)) -> Option<f32> {
        let axis = usize::try_from(self.hovered).ok().filter(|&a| a < 3);
        let (inv, vert) = axis.map(|a| (self.invert[a], self.vertical[a])).unwrap_or((false, false));
        let use_x = if self.mode == 1 { vert } else { !vert };
        let (cur, start) = if use_x { (cursor.0, self.press.0) } else { (cursor.1, self.press.1) };
        let delta = if inv { start - cur } else { cur - start };
        (delta != 0).then_some(delta as f32 * PIXEL_UNIT)
    }

    /// Re-centre (B 0x100723F0): the stored press point becomes the client centre.
    pub fn recentred(&mut self, centre: (i32, i32)) {
        self.press = centre;
    }

    /// Stop (B 0x10072BF0): the client tick sends the final 116/117.
    pub fn stop(&mut self, response: u32) {
        if self.active {
            self.active = false;
            self.actions.push(Action::Stop { response });
        }
    }
}

/// Angle wrap of the rotate drag (B 0x100B48F0): one step of ±360 outside (−180, 180].
pub fn wrap_degrees(v: f32) -> f32 {
    let mut v = v;
    if v > 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Object drag in rotate mode (B 0x10073240): rotation copy += d × 10, wrapped.
pub fn rotate(rot: &mut [f32; 3], axis: usize, d: f32) {
    rot[axis] = wrap_degrees(rot[axis] + d * 10.0);
}

/// Attached-object drag (B 0x10072240) on the slot's offset / rotation / scale.
pub fn attached_drag(
    offset: &mut [f32; 3],
    rotation: &mut [f32; 3],
    scale: &mut [f32; 3],
    mode: u32,
    axis: usize,
    d: f32,
) {
    match mode {
        0 => offset[axis] += d * 0.1,
        1 => rotation[axis] = wrap_degrees(rotation[axis] + d * 10.0),
        2 => {
            scale[axis] += d * 0.1;
            if scale[axis] < 0.0 {
                scale[axis] = 0.0;
            }
        }
        _ => {}
    }
}

/// `M · v` with samp's matrix helper (B 0x100B4D20): rows right (0), forward (4), up (8), position (12).
pub fn transform(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12],
        m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13],
        m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14],
    ]
}

/// 117 response-2 rate rule.
pub fn update_due(now_ms: u32, last_ms: u32) -> bool {
    now_ms.wrapping_sub(last_ms) as i32 >= UPDATE_INTERVAL_MS as i32
}

/// Selection label (B 0x1006DA20).
pub fn select_label(id: u16, model: i32, distance: f32) -> String {
    format!("[id: {id}, type: {model}]\nDistance: {distance:.2}m\n")
}

/// Pick ray through the cursor (B 0x10073F70): `inv` = inverse of world × view, `proj` = projection, `size` =
/// back-buffer size. Returns (start, end) with end = start + 150 · direction (direction not normalised).
pub fn pick_ray(cursor: (i32, i32), size: (u32, u32), proj: &[f32; 16], inv: &[f32; 16]) -> ([f32; 3], [f32; 3]) {
    let nx = ((cursor.0 as f32 * 2.0) / size.0 as f32 - 1.0) / proj[0];
    let ny = -(((cursor.1 as f32 * 2.0) / size.1 as f32 - 1.0) / proj[5]);
    let dir =
        [nx * inv[0] + ny * inv[4] + inv[8], nx * inv[1] + ny * inv[5] + inv[9], nx * inv[2] + ny * inv[6] + inv[10]];
    let start = [inv[12], inv[13], inv[14]];
    let end = [start[0] + PICK_DISTANCE * dir[0], start[1] + PICK_DISTANCE * dir[1], start[2] + PICK_DISTANCE * dir[2]];
    (start, end)
}

/// `a × b` for row-major D3D matrices (D3DXMatrixMultiply).
pub fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        (0..4).map(|k| a[4 * r + k] * b[4 * k + c]).sum()
    })
}

/// General 4×4 inverse (D3DXMatrixInverse); `None` when singular.
pub fn mat_inverse(m: &[f32; 16]) -> Option<[f32; 16]> {
    let m: [f64; 16] = m.map(f64::from);
    let mut inv = [0.0f64; 16];
    inv[0] =
        m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];
    inv[4] =
        -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];
    inv[8] =
        m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];
    inv[12] =
        -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];
    inv[1] =
        -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];
    inv[5] =
        m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];
    inv[9] =
        -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];
    inv[13] =
        m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];
    inv[2] =
        m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];
    inv[6] =
        -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];
    inv[10] =
        m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];
    inv[14] =
        -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];
    inv[3] =
        -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7]
        - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7]
        + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6]
        - m[8] * m[2] * m[5];
    let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
    if det == 0.0 {
        return None;
    }
    Some(inv.map(|v| (v / det) as f32))
}

/// Pick sphere radius of an object's bounding test (B 0x100B65E0): < 0.5 → 0.5 then × 1.5, < 1 → × 1.5.
pub fn pick_radius(col_radius: f32) -> f32 {
    if col_radius < 0.5 {
        0.75
    } else if col_radius < 1.0 {
        col_radius * 1.5
    } else {
        col_radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing() -> Editor {
        let mut e = Editor::default();
        e.start(Target::Object { id: 1, player_object: false }, [0.0; 3]);
        e.glyph = (20, 27);
        e
    }

    #[test]
    fn colours_and_glyphs_as_measured() {
        let mut e = editing();
        e.hovered = 1;
        assert_eq!(e.background(0), BACKGROUND);
        assert_eq!(e.background(1), BACKGROUND_HOVER);
        assert_eq!(e.background(3), BACKGROUND_ACTIVE);
        assert_eq!(e.glyph_of(0), (&b"6"[..], 0xFF99_99CC));
        assert_eq!(e.glyph_of(HANDLE_SAVE), (&b"H"[..], BUTTON_GLYPH_COLOUR));
        e.set_mode(1);
        assert_eq!(e.glyph_of(2), (&b"5"[..], 0xFF99_CC99));
        e.dragging = true;
        assert_eq!(e.background(1), BACKGROUND_ACTIVE);
        e.set_mode(2);
        assert_eq!(e.mode, 0, "objects have no scale mode");
    }

    #[test]
    fn rects_centre_on_the_point() {
        let mut e = editing();
        e.place(0, 754, 364);
        assert_eq!(e.rects[0], [744, 351, 764, 378]);
        e.place(HANDLE_SAVE, 640, 449);
        e.drawn = true;
        e.hover((640, 449), 3);
        assert_eq!(e.hovered, HANDLE_SAVE);
        e.hover((640, 449), 0);
        assert_eq!(e.hovered, NO_HANDLE);
    }

    #[test]
    fn drag_follows_the_screen_direction() {
        let mut e = editing();
        e.set_direction(0, [754.0, 364.0], [526.0, 355.0]);
        assert!(!e.vertical[0] && !e.invert[0]);
        e.set_direction(2, [640.0, 278.0], [640.0, 449.0]);
        assert!(e.vertical[2] && e.invert[2]);
        e.place(0, 754, 364);
        e.hovered = 0;
        e.drawn = true;
        let fx = e.on_mouse(WM_LBUTTONDOWN, (754, 364), (754, 364));
        assert!(fx.consumed && fx.capture && e.dragging);
        let fx = e.on_mouse(WM_MOUSEMOVE, (757, 364), (757, 364));
        assert!(fx.recentre);
        assert_eq!(e.actions.last(), Some(&Action::Drag { axis: 0, d: 0.03 }));
        e.recentred((640, 360));
        assert_eq!(e.drag_delta((640, 365)), None, "x only for a horizontal axis");
        let fx = e.on_mouse(WM_LBUTTONUP, (640, 360), (640, 360));
        assert_eq!(fx.release_to, Some((744, 351)));
        assert!(!e.dragging);
    }

    #[test]
    fn rotate_mode_swaps_the_coordinate() {
        let mut e = editing();
        e.set_mode(1);
        e.set_direction(2, [640.0, 278.0], [640.0, 449.0]);
        e.hovered = 2;
        e.press = (640, 360);
        assert!((e.drag_delta((643, 360)).unwrap() + 0.03).abs() < 1e-6);
        let mut r = [0.0, 0.0, 0.0];
        rotate(&mut r, 2, -0.03);
        assert!((r[2] + 0.3).abs() < 1e-5);
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-181.0), 179.0);
    }

    #[test]
    fn buttons_and_save() {
        let mut e = editing();
        e.hovered = 4;
        assert!(e.on_mouse(WM_LBUTTONUP, (0, 0), (0, 0)).consumed);
        assert_eq!(e.mode, 1);
        e.hovered = HANDLE_SAVE;
        assert!(e.on_mouse(WM_LBUTTONUP, (0, 0), (0, 0)).consumed);
        assert!(!e.active);
        assert_eq!(e.actions.last(), Some(&Action::Stop { response: 1 }));
    }

    #[test]
    fn attached_drag_rules() {
        let (mut o, mut r, mut s) = ([0.1, 0.0, 0.0], [0.0; 3], [1.0; 3]);
        attached_drag(&mut o, &mut r, &mut s, 0, 0, -0.4);
        assert!((o[0] - 0.06).abs() < 1e-6, "measured: 0.1 → 0.06");
        attached_drag(&mut o, &mut r, &mut s, 2, 1, -20.0);
        assert_eq!(s[1], 0.0);
    }

    #[test]
    fn misc() {
        assert!(update_due(1250, 1000));
        assert!(!update_due(1249, 1000));
        assert_eq!(select_label(1, 1271, 7.8), "[id: 1, type: 1271]\nDistance: 7.80m\n");
        assert_eq!(pick_radius(0.2), 0.75);
        let id = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let (s, e) = pick_ray((640, 360), (1280, 720), &id, &id);
        assert_eq!(s, [0.0; 3]);
        assert_eq!(e, [0.0, 0.0, 150.0]);
        assert_eq!(transform(&id, [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        let t = [1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 5.0, 6.0, 7.0, 1.0];
        let p = mat_mul(&t, &mat_inverse(&t).unwrap());
        assert!(p.iter().zip(id.iter()).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
