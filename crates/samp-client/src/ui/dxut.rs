//! The GUI layer drawn the way samp's statically linked DXUT does (UR §4), so that the
//! Direct3D call stream matches the original's (TRACE-3 D3D logs, docs/phase3/m8a-notes.md):
//!
//! - a dialog render is `sprite.Begin(DONOTSAVESTATE)`, the fixed render/texture-stage
//!   states, the background fan (`DrawPrimitiveUP`, FVF 0x144, also when transparent),
//!   the texture states, `SetTexture(sampgui.png)`, the controls, `sprite.End`;
//! - an element sprite is `SetTransform(scale)` + `Draw(src, NULL, pos / scale, colour)`
//!   (skipped when the colour's alpha is 0);
//! - text is `SetTransform(identity)` + the colour-code wrapper's `DrawTextA` (or the
//!   font's `DrawTextW` for edit boxes);
//! - solid rectangles (caret, selection) flush the sprite and draw a fan with the
//!   texture stage set to "diffuse only", restoring the vertex declaration.
//!
//! Control internals follow the stock DXUT library (list box, scrollbar, edit box,
//! button), with the colours and texture rects of samp's skin (UR §4.1).

use std::ffi::c_void;

use samp_ui::colour;

use super::d3d::{self, D3dx, Font, IDENTITY, Rect, Sprite, devcall};

/// sampgui.png element rects (UR §4.1 table).
pub mod skin {
    use super::Rect;
    pub const fn r(l: i32, t: i32, rr: i32, b: i32) -> Rect {
        Rect { left: l, top: t, right: rr, bottom: b }
    }
    /// Edit box: text area, then the 8 frame pieces in DXUT element order
    /// (TL, T, TR, L, R, BL, B, BR).
    pub const EDIT: [Rect; 9] = [
        r(14, 90, 241, 113),
        r(8, 82, 14, 90),
        r(14, 82, 241, 90),
        r(241, 82, 246, 90),
        r(8, 90, 14, 113),
        r(241, 90, 246, 113),
        r(8, 113, 14, 121),
        r(14, 113, 241, 121),
        r(241, 113, 246, 121),
    ];
    pub const SCROLL_TRACK: Rect = r(243, 144, 265, 155);
    pub const SCROLL_UP: Rect = r(243, 124, 265, 144);
    pub const SCROLL_DOWN: Rect = r(243, 155, 265, 176);
    pub const SCROLL_THUMB: Rect = r(266, 123, 286, 167);
    pub const CAPTION: Rect = r(17, 269, 241, 287);
    pub const BUTTON: Rect = r(0, 0, 136, 54);
    pub const LIST: Rect = r(13, 124, 241, 265);
    pub const LIST_SELECTION: Rect = r(17, 269, 241, 287);
}

/// Colours of samp's skin (UR §4.1, TRACE-3).
pub mod colours {
    pub const CAPTION_TEX: u32 = 0xC80A_0A0A;
    pub const CAPTION_TEXT: u32 = 0xFFD2_D2D2;
    pub const BUTTON_TEX: u32 = 0xC8FF_FFFF;
    pub const BUTTON_TEX_OVER: u32 = 0xFFB9_2228;
    pub const BUTTON_TEXT: u32 = 0xFFC8_C8C8;
    pub const BUTTON_TEXT_OVER: u32 = 0xFFFF_FFFF;
    /// Fill layer: transparent unless hovered/focused.
    pub const BUTTON_FILL_OVER: u32 = 0xFFB9_2228;
    pub const LIST_TEX: u32 = 0xC8FF_FFFF;
    pub const WHITE: u32 = 0xFFFF_FFFF;
    pub const THUMB: u32 = 0xFFB9_2228;
    /// Thumb tint while nothing scrolls (disabled state).
    pub const THUMB_DISABLED: u32 = 0xFF95_B0D0;
    pub const EDIT_TEXT: u32 = 0xFFFF_FFFF;
    pub const EDIT_FRAME: u32 = 0xF005_0505;
    pub const EDIT_BORDER: u32 = 0xFFFF_FFFF;
    pub const CARET: u32 = 0xFF96_9696;
    pub const SELECTION_BG: u32 = 0xFFB9_2228;
    pub const SELECTED_TEXT: u32 = 0xFF0A_0A0F;
}

/// Format flags of the elements.
pub mod fmt {
    pub const CAPTION: u32 = 0x4;
    pub const BUTTON: u32 = 0x5;
    pub const LIST: u32 = 0x40;
    pub const EDIT: u32 = 0x4;
}

const FVF_SCREEN: u32 = 0x144;
const FVF_COLOUR: u32 = 0x44;
const TRIANGLE_FAN: u32 = 6;

/// `DXUT_SCREEN_VERTEX`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ScreenVertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    colour: u32,
    u: f32,
    v: f32,
}

/// Pre-transformed coloured vertex (FVF 0x44), used by the nametag bars.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColourVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rhw: f32,
    pub colour: u32,
}

/// The render/texture-stage states DXUT sets at the start of a dialog render; the
/// nametag bar pass sets the same list (TRACE-3).
pub fn ui_states() {
    devcall::set_render_state(27, 1); // ALPHABLENDENABLE
    devcall::set_render_state(19, 5); // SRCBLEND SRCALPHA
    devcall::set_render_state(20, 6); // DESTBLEND INVSRCALPHA
    devcall::set_render_state(15, 0); // ALPHATESTENABLE
    devcall::set_tss(0, 1, 3); // COLOROP SELECTARG2
    devcall::set_tss(0, 3, 0); // COLORARG2 DIFFUSE
    devcall::set_tss(0, 4, 2); // ALPHAOP SELECTARG1
    devcall::set_tss(0, 5, 0); // ALPHAARG1 DIFFUSE
    devcall::set_vertex_shader_null();
    devcall::set_pixel_shader_null();
    devcall::set_render_state(7, 0); // ZENABLE
}

/// Start of `CDXUTDialog::OnRender` for a dialog at `rect` with a uniform background.
pub fn dialog_begin(sprite: &Sprite, rect: Rect, background: u32, gui_tex: *mut c_void) {
    sprite.begin(d3d::sprite::DONOTSAVESTATE);
    ui_states();
    let (l, t, r, b) = (rect.left as f32, rect.top as f32, rect.right as f32, rect.bottom as f32);
    let v = [
        ScreenVertex { x: l, y: t, z: 0.5, rhw: 1.0, colour: background, u: 0.0, v: 0.5 },
        ScreenVertex { x: r, y: t, z: 0.5, rhw: 1.0, colour: background, u: 1.0, v: 0.5 },
        ScreenVertex { x: r, y: b, z: 0.5, rhw: 1.0, colour: background, u: 1.0, v: 1.0 },
        ScreenVertex { x: l, y: b, z: 0.5, rhw: 1.0, colour: background, u: 0.0, v: 1.0 },
    ];
    devcall::set_fvf(FVF_SCREEN);
    devcall::draw_primitive_up(TRIANGLE_FAN, 2, v.as_ptr().cast(), std::mem::size_of::<ScreenVertex>() as u32);
    devcall::set_tss(0, 1, 4); // COLOROP MODULATE
    devcall::set_tss(0, 2, 2); // COLORARG1 TEXTURE
    devcall::set_tss(0, 3, 0); // COLORARG2 DIFFUSE
    devcall::set_tss(0, 4, 4); // ALPHAOP MODULATE
    devcall::set_tss(0, 5, 2); // ALPHAARG1 TEXTURE
    devcall::set_tss(0, 6, 0); // ALPHAARG2 DIFFUSE
    devcall::set_sampler_state(0, 6, 2); // MINFILTER LINEAR
    devcall::set_texture(0, gui_tex);
}

pub fn dialog_end(sprite: &Sprite) {
    sprite.end();
}

fn scale_matrix(sx: f32, sy: f32) -> [f32; 16] {
    let mut m = IDENTITY;
    m[0] = sx;
    m[5] = sy;
    m
}

/// `CDXUTDialog::DrawSprite`: `src` of `tex` scaled into the screen rect `dst`.
pub fn draw_sprite(sprite: &Sprite, tex: *mut c_void, src: Rect, dst: Rect, colour: u32) {
    if colour >> 24 == 0 {
        return;
    }
    let sx = (dst.right - dst.left) as f32 / (src.right - src.left) as f32;
    let sy = (dst.bottom - dst.top) as f32 / (src.bottom - src.top) as f32;
    sprite.set_transform(&scale_matrix(sx, sy));
    let pos = [dst.left as f32 / sx, dst.top as f32 / sy, 0.0];
    sprite.draw(tex, Some(&src), None, Some(&pos), colour);
}

/// `CDXUTDialog::DrawText` through the colour-code wrapper (GUI fonts are wrapper fonts:
/// `{RRGGBB}` recolours glyphs; the stripped text is drawn).
pub fn draw_text(sprite: &Sprite, font: &Font, text: &[u8], rect: Rect, format: u32, colour: u32) {
    if colour >> 24 == 0 {
        return;
    }
    sprite.set_transform(&IDENTITY);
    let stripped = colour::strip_codes(text);
    let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
    let mut r = rect;
    d3d::draw_glyph_coloured(font, sprite, &stripped, &mut r, format, colour, glyphs);
}

/// `CDXUTDialog::DrawText` of wide text with the real sprite (edit boxes).
pub fn draw_text_w(sprite: &Sprite, font: &Font, text: &[u16], count: i32, rect: Rect, format: u32, colour: u32) {
    if colour >> 24 == 0 {
        return;
    }
    sprite.set_transform(&IDENTITY);
    let mut r = rect;
    font.draw_w(sprite.ptr, text, count, &mut r, format, colour);
}

/// `CDXUTDialog::DrawRect`.
pub fn draw_rect(sprite: &Sprite, rect: Rect, colour: u32) {
    let (l, t, r, b) = (rect.left as f32 - 0.5, rect.top as f32 - 0.5, rect.right as f32 - 0.5, rect.bottom as f32 - 0.5);
    let v = [
        ScreenVertex { x: l, y: t, z: 0.5, rhw: 1.0, colour, u: 0.0, v: 0.0 },
        ScreenVertex { x: r, y: t, z: 0.5, rhw: 1.0, colour, u: 0.0, v: 0.0 },
        ScreenVertex { x: r, y: b, z: 0.5, rhw: 1.0, colour, u: 0.0, v: 0.0 },
        ScreenVertex { x: l, y: b, z: 0.5, rhw: 1.0, colour, u: 0.0, v: 0.0 },
    ];
    sprite.flush();
    let decl = devcall::get_vertex_declaration();
    devcall::set_fvf(FVF_SCREEN);
    devcall::set_tss(0, 1, 3);
    devcall::set_tss(0, 4, 3);
    devcall::draw_primitive_up(TRIANGLE_FAN, 2, v.as_ptr().cast(), std::mem::size_of::<ScreenVertex>() as u32);
    devcall::set_tss(0, 1, 4);
    devcall::set_tss(0, 4, 4);
    devcall::set_vertex_declaration(decl);
    // SAFETY: GetVertexDeclaration AddRef'd it.
    unsafe { d3d::release(decl) };
}

/// A coloured fan (FVF 0x44 was set by the caller), quad corners in the nametag order
/// (x0,y0), (x0,y1), (x1,y1), (x1,y0).
pub fn colour_quad(x0: f32, y0: f32, x1: f32, y1: f32, colour: u32) {
    let v = [
        ColourVertex { x: x0, y: y0, z: 0.0, rhw: 1.0, colour },
        ColourVertex { x: x0, y: y1, z: 0.0, rhw: 1.0, colour },
        ColourVertex { x: x1, y: y1, z: 0.0, rhw: 1.0, colour },
        ColourVertex { x: x1, y: y0, z: 0.0, rhw: 1.0, colour },
    ];
    devcall::draw_primitive_up(TRIANGLE_FAN, 2, v.as_ptr().cast(), std::mem::size_of::<ColourVertex>() as u32);
}

pub fn set_colour_fvf() {
    devcall::set_fvf(FVF_COLOUR);
}

/// Caption bar and text (`CDXUTDialog::OnRender` with a caption).
pub fn caption(sprite: &Sprite, tex: *mut c_void, font1: &Font, rect: Rect, caption_h: i32, title: &[u8]) {
    let bar = Rect { left: rect.left, top: rect.top, right: rect.right, bottom: rect.top + caption_h };
    draw_sprite(sprite, tex, skin::CAPTION, bar, colours::CAPTION_TEX);
    let r = Rect { left: bar.left + 5, ..bar };
    draw_text(sprite, font1, title, r, fmt::CAPTION, colours::CAPTION_TEXT);
}

/// `CDXUTButton::Render` (face layer + text, fill layer + text).
pub fn button(sprite: &Sprite, tex: *mut c_void, font0: &Font, label: &[u8], rect: Rect, mouse_over: bool) {
    let (rc, tex_c, text_c, fill_c) = if mouse_over {
        (
            Rect { left: rect.left - 1, top: rect.top - 2, right: rect.right - 1, bottom: rect.bottom - 2 },
            colours::BUTTON_TEX_OVER,
            colours::BUTTON_TEXT_OVER,
            colours::BUTTON_FILL_OVER,
        )
    } else {
        (rect, colours::BUTTON_TEX, colours::BUTTON_TEXT, 0x00FF_FFFF)
    };
    draw_sprite(sprite, tex, skin::BUTTON, rc, tex_c);
    draw_text(sprite, font0, label, rc, fmt::BUTTON, text_c);
    draw_sprite(sprite, tex, skin::BUTTON, rc, fill_c);
    draw_text(sprite, font0, label, rc, fmt::BUTTON, text_c);
}

/// DXUT scrollbar state.
#[derive(Clone, Copy, Debug)]
pub struct ScrollBar {
    pub rect: Rect,
    pub start: i32,
    pub end: i32,
    pub pos: i32,
    pub page: i32,
}

impl ScrollBar {
    /// Up, down, track and thumb rects; `None` thumb height when nothing scrolls.
    pub fn rects(&self) -> (Rect, Rect, Rect, Rect, bool) {
        let b = self.rect;
        let w = b.right - b.left;
        let up = Rect { bottom: b.top + w, ..b };
        let down = Rect { top: b.bottom - w, ..b };
        let track = Rect { top: up.bottom, bottom: down.top, ..b };
        let th = track.bottom - track.top;
        if self.end - self.start > self.page {
            let thumb_h = (th * self.page / (self.end - self.start)).max(8);
            let max_pos = self.end - self.start - self.page;
            let top = track.top + (self.pos - self.start) * (th - thumb_h) / max_pos;
            (up, down, track, Rect { top, bottom: top + thumb_h, ..b }, true)
        } else {
            // No content to scroll: the thumb keeps its top and has no height.
            (up, down, track, Rect { top: track.top, bottom: track.top, ..b }, false)
        }
    }
}

/// `CDXUTScrollBar::Render`: track, up, down, thumb.
pub fn scrollbar(sprite: &Sprite, tex: *mut c_void, sb: &ScrollBar) {
    let (up, down, track, thumb, shown) = sb.rects();
    draw_sprite(sprite, tex, skin::SCROLL_TRACK, track, colours::WHITE);
    draw_sprite(sprite, tex, skin::SCROLL_UP, up, colours::WHITE);
    draw_sprite(sprite, tex, skin::SCROLL_DOWN, down, colours::WHITE);
    let c = if shown { colours::THUMB } else { colours::THUMB_DISABLED };
    draw_sprite(sprite, tex, skin::SCROLL_THUMB, thumb, c);
}

/// The edit box's text as drawn plus caret/selection state.
pub struct EditView<'a> {
    pub text: &'a [u16],
    pub caret: usize,
    pub anchor: usize,
    pub first_visible: usize,
    pub focused: bool,
    pub insert_mode: bool,
}

/// `CDXUTEditBox::Render` for the box `rect` (border 5, spacing 4). `width_of` measures
/// UTF-16 text like DXUT's Uniscribe buffer (GDI, not a D3DX call).
pub fn edit_box(sprite: &Sprite, tex: *mut c_void, font: &Font, rect: Rect, e: &EditView, width_of: &dyn Fn(&[u16]) -> i32) {
    let t = Rect { left: rect.left + 5, top: rect.top + 5, right: rect.right - 5, bottom: rect.bottom - 5 };
    let dst = [
        t,
        Rect { left: rect.left, top: rect.top, right: t.left, bottom: t.top },
        Rect { left: t.left, top: rect.top, right: t.right, bottom: t.top },
        Rect { left: t.right, top: rect.top, right: rect.right, bottom: t.top },
        Rect { left: rect.left, top: t.top, right: t.left, bottom: t.bottom },
        Rect { left: t.right, top: t.top, right: rect.right, bottom: t.bottom },
        Rect { left: rect.left, top: t.bottom, right: t.left, bottom: rect.bottom },
        Rect { left: t.left, top: t.bottom, right: t.right, bottom: rect.bottom },
        Rect { left: t.right, top: t.bottom, right: rect.right, bottom: rect.bottom },
    ];
    for (i, (src, d)) in skin::EDIT.iter().zip(dst).enumerate() {
        draw_sprite(sprite, tex, *src, d, if i == 0 { colours::EDIT_FRAME } else { colours::EDIT_BORDER });
    }
    let area = Rect { left: t.left + 4, top: t.top + 4, right: t.right - 4, bottom: t.bottom - 4 };
    let x_of = |cp: usize| width_of(&e.text[..cp.min(e.text.len())]);
    let x_first = x_of(e.first_visible);
    let caret_x = x_of(e.caret);
    let mut sel_rect = None;
    if e.caret != e.anchor {
        let sel_x = x_of(e.anchor);
        let (a, b) = if caret_x <= sel_x { (caret_x, sel_x) } else { (sel_x, caret_x) };
        let mut r = Rect {
            left: a + area.left - x_first,
            top: area.top,
            right: b + area.left - x_first,
            bottom: area.bottom,
        };
        r.left = r.left.max(area.left);
        r.right = r.right.min(area.right);
        if r.right > r.left {
            draw_rect(sprite, r, colours::SELECTION_BG);
        }
        sel_rect = Some(r);
    }
    let first = e.first_visible.min(e.text.len());
    draw_text_w(sprite, font, &e.text[first..], -1, area, fmt::EDIT, colours::EDIT_TEXT);
    if let Some(r) = sel_rect {
        let lo = e.caret.min(e.anchor).max(first);
        let hi = e.caret.max(e.anchor);
        if hi > lo {
            draw_text_w(sprite, font, &e.text[lo..hi], (hi - lo) as i32, r, fmt::EDIT, colours::SELECTED_TEXT);
        }
    }
    if e.focused {
        // TRACE-3: the caret does not blink.
        let cx = area.left - x_first + caret_x;
        let mut r = Rect { left: cx - 1, top: area.top, right: cx + 1, bottom: area.bottom };
        if !e.insert_mode {
            r.right = area.left - x_first + x_of(e.caret + 1);
        }
        draw_rect(sprite, r, colours::CARET);
    }
}

/// Loads the GUI skin texture and the D3DX objects the GUI needs.
pub fn load_skin(d3dx: &D3dx) -> *mut c_void {
    match d3d::load_texture(d3dx, b"sampgui.png") {
        Ok(t) => t,
        Err(e) => {
            crate::log!("ui: {e}; GUI controls are not drawn");
            std::ptr::null_mut()
        }
    }
}
