//! World-anchored drawing: nametag bars and names, chat bubbles, 3D text labels, the `/dl` vehicle labels,
//! and the number-plate painter.

use std::ffi::c_void;

use samp_ui::colour;
use samp_ui::fonts;
use samp_ui::nametags;

use super::{BubbleDraw, Renderer, TagDraw};
use crate::ui::d3d::{self, Font, FontArgs, Rect, Sprite, devcall, dt};
use crate::ui::dxut;

impl Renderer {
    /// World point to screen `(x, y, z)` with samp's copies of the view and projection matrices.
    pub(super) fn project(&self, p: [f32; 3]) -> [f32; 3] {
        let vp = devcall::viewport();
        let proj = *d3d::PROJECTION.lock().unwrap_or_else(|e| e.into_inner());
        let view = *d3d::VIEW.lock().unwrap_or_else(|e| e.into_inner());
        self.d3dx.project(p, &vp, &proj, &view)
    }

    /// Screen point of a tag: `(x, y, z + 0.0475·d + 0.2)`, truncated; `None` when the
    /// projected z is above 1.
    fn tag_point(&self, t: &TagDraw) -> Option<(i32, i32)> {
        let p = [t.anchor[0], t.anchor[1], t.anchor[2] + t.distance * nametags::RAISE_PER_METRE + nametags::RAISE_BASE];
        let s = self.project(p);
        (s[2] <= 1.0).then_some((s[0] as i32, s[1] as i32))
    }

    /// Nametag health / armour bars pass. The painter's begin creates its `D3DSBT_ALL` state block on first use
    /// (after the sprite's `OnResetDevice`) and captures it; its end applies it after the sprite's `End`
    /// (ui-rendering.md, "Bars"). Mirrors samp.dll 0x10074270.
    pub(super) fn tag_bars(&mut self, tags: &[TagDraw]) {
        let s = &self.sprites.tags;
        if self.tags_state_block.is_null() {
            s.on_reset();
            self.tags_state_block = d3d::devcall::create_state_block(1);
        }
        d3d::state_block_capture(self.tags_state_block);
        s.begin(d3d::sprite::DONOTSAVESTATE);
        dxut::ui_states();
        for t in tags {
            let Some((x, y)) = self.tag_point(t) else { continue };
            for group in nametags::bars(x, y, t.health, t.armour) {
                dxut::set_colour_fvf();
                for q in group {
                    dxut::colour_quad(q.x0, q.y0, q.x1, q.y1, q.colour);
                }
            }
        }
        s.end();
        d3d::state_block_apply(self.tags_state_block);
    }

    /// Nametag names pass, including the AFK icon (ui-rendering.md, "Name text"). Mirrors samp.dll 0x10075390.
    pub(super) fn tag_names(&mut self, tags: &[TagDraw], nametag_status: bool) {
        if !tags.is_empty() && self.aux.is_none() {
            let f = |h: i32| {
                Font::create(
                    &self.d3dx,
                    FontArgs { height: h, width: 0, weight: 400, charset: 2, quality: 0, face: b"SAMPAUX3".to_vec() },
                )
            };
            match (f(20), f(22)) {
                (Ok(a), Ok(b)) => self.aux = Some((a, b)),
                (Err(e), _) | (_, Err(e)) => crate::log!("ui: SAMPAUX3 fonts: {e}"),
            }
        }
        let s = &self.sprites.tags;
        s.begin(d3d::sprite::ALPHABLEND);
        for t in tags {
            let Some((x, y)) = self.tag_point(t) else { continue };
            let mut m = Rect::default();
            self.fonts.small.draw_n(std::ptr::null_mut(), &t.name, -1, &mut m, dt::CALCRECT, 0xFF00_0000);
            let (w, h) = (m.right - m.left, m.bottom - m.top);
            let r = nametags::name_rect(x, y, w, h);
            let rect = Rect { left: r[0], top: r[1], right: r[2], bottom: r[3] };
            Self::outlined(
                s,
                &self.fonts.small,
                &self.fonts.small_outline,
                &t.name,
                rect,
                0,
                t.colour,
                &fonts::OUTLINE_4,
            );
            if let Some((a20, a22)) = &self.aux {
                let mut c = Rect::default();
                a22.draw_n(
                    std::ptr::null_mut(),
                    b"C",
                    1,
                    &mut c,
                    dt::CALCRECT | dt::NOCLIP | dt::CENTER | dt::VCENTER,
                    0,
                );
                if t.afk && nametag_status {
                    let (cr, er) = nametags::afk_rects(r[0], y, [c.left, c.top, c.right, c.bottom]);
                    let flags = dt::CENTER | dt::VCENTER | dt::NOCLIP;
                    let mut rc = Rect { left: cr[0], top: cr[1], right: cr[2], bottom: cr[3] };
                    a22.draw_n(std::ptr::null_mut(), b"C", 1, &mut rc, flags, nametags::AFK_BACKDROP);
                    let mut re = Rect { left: er[0], top: er[1], right: er[2], bottom: er[3] };
                    a20.draw_n(std::ptr::null_mut(), b"E", 1, &mut re, flags, nametags::AFK_ICON);
                }
            }
        }
        s.end();
    }

    /// The 3D text painter shared by labels and chat bubbles: project with samp's view/projection copies, skip
    /// when z > 1, truncate to a rect [x, y, x+1, y+1], then the font renderer's small draw with format 0x101:
    /// when `outline` is set, four black passes of the stripped text (offsets move only left/top), then the main
    /// pass of the raw text through the colour-code wrapper (per-glyph `{RRGGBB}` colours, the base alpha only
    /// up to the first code). Mirrors samp.dll 0x1006BC90.
    pub(super) fn text3d(&self, s: &Sprite, pos: [f32; 3], text: &[u8], argb: u32, outline: bool) {
        let p = self.project(pos);
        if p[2] > 1.0 {
            return;
        }
        let (x, y) = (p[0] as i32, p[1] as i32);
        let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        let passes: &[(i32, i32)] = if outline { &fonts::OUTLINE_4 } else { &[] };
        Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, text, rect, dt::CENTER, argb, passes);
    }

    /// Chat bubbles (ui-rendering.md, "Chat bubbles"): the 3D text painter with the outline always on and no
    /// line-of-sight test. Mirrors samp.dll 0x10066ED0.
    pub(super) fn bubbles(&self, bubbles: &[BubbleDraw]) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for b in bubbles {
            self.text3d(s, b.anchor, &b.text, b.colour, true);
        }
        s.end();
    }

    /// 3D text labels: one batch on the 3D-text sprite through the 3D text painter. The outline is on only when
    /// the label colour's alpha is FF; the text keeps its `{RRGGBB}` codes for the colour wrapper. Placement, draw
    /// distance and line of sight come from `entities.rs` (ui-rendering.md, "Smaller overlays").
    /// Mirrors samp.dll 0x10001350.
    pub(super) fn labels(&self) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for l in crate::entities::label_draws() {
            self.text3d(s, l.pos, &l.text, l.argb, l.argb & 0xFF00_0000 == 0xFF00_0000);
        }
        s.end();
    }

    /// `/dl` vehicle labels on the 3D-text painter with the outline on: four black outline draws of the stripped
    /// text, then the main draw in [`crate::vehlabels::LABEL_ARGB`] through the colour wrapper.
    pub(super) fn vehicle_labels(&self, rows: &[([f32; 3], String)]) {
        const OUTLINE: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for (pos, text) in rows {
            let p = self.project(*pos);
            if p[2] > 1.0 {
                continue;
            }
            let (x, y) = (p[0] as i32, p[1] as i32);
            let text = colour::strip_codes(text.as_bytes());
            let font = &self.fonts.small;
            for (dx, dy) in OUTLINE {
                let mut r = Rect { left: x + dx, top: y + dy, right: x + 1, bottom: y + 1 };
                font.draw(s.ptr, &text, &mut r, dt::CENTER | dt::NOCLIP, colour::OUTLINE);
            }
            // The main pass goes through the colour-code font wrapper, which samp calls without a sprite pointer.
            let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
            let argb = crate::vehlabels::LABEL_ARGB;
            let glyphs: Vec<u32> = colour::glyph_colours(&text, argb).iter().map(|g| g.colour).collect();
            d3d::draw_glyph_coloured(font, s, &text, &mut r, dt::CENTER | dt::NOCLIP, argb, glyphs);
        }
        s.end();
    }

    /// The number-plate painter: the 128×32 render target is cleared to 0xFFBEB6A8 and the plate text drawn by the
    /// font renderer (Arial 38/10 bold, four 0xB4F0EEE4 outline passes at (0,−2)/(+2,0)/(0,+2)/(−2,0) around
    /// (0, 3), then 0xEE444470; rect right / bottom 128 / 32, format 0x105, colour codes removed), then copied with
    /// a linear filter into a new 64×32 texture (default pool). Returns the texture, 0 on failure.
    /// Mirrors samp.dll B 0x1006D840.
    pub fn paint_plate(&self, text: &[u8]) -> u32 {
        const FMT: u32 = 0x105;
        const OUTLINE: u32 = 0xB4F0_EEE4;
        const TEXT: u32 = 0xEE44_4470;
        let Some(rt) = &self.unused.plate_rt else { return 0 };
        let mut tex = std::ptr::null_mut();
        // 22 = D3DFMT_X8R8G8B8, the format samp's plate painter uses.
        let hr = (self.d3dx.create_texture)(d3d::real_device(), 64, 32, 1, 0, 22, 0, &mut tex);
        if hr < 0 || tex.is_null() {
            return 0;
        }
        rt.begin();
        devcall::clear(3, 0xFFBE_B6A8, 1.0, 0);
        let stripped = colour::strip_codes(text);
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        let font = &self.unused.plate_font;
        // Every pass goes through the font renderer's colour proxy, as in samp: the glyphs land on the renderer's
        // sprite.
        for (dx, dy, argb) in [(0, -2, OUTLINE), (2, 0, OUTLINE), (0, 2, OUTLINE), (-2, 0, OUTLINE), (0, 0, TEXT)] {
            let mut r = Rect { left: dx, top: 3 + dy, right: 128, bottom: 32 };
            let glyphs: Vec<u32> = colour::glyph_colours(&stripped, argb).iter().map(|g| g.colour).collect();
            d3d::draw_glyph_coloured(font, s, &stripped, &mut r, FMT, argb, glyphs);
        }
        s.end();
        rt.end();
        // SAFETY: `tex` is the live texture just created (slot 14 SetAutoGenFilterType, slot 18 GetSurfaceLevel);
        // `surf` is checked for null and released after the copy.
        unsafe {
            let f: extern "system" fn(*mut c_void, u32) -> d3d::HResult = d3d::vfn(tex, 14);
            f(tex, 2);
            let mut surf = std::ptr::null_mut();
            let f: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> d3d::HResult = d3d::vfn(tex, 18);
            f(tex, 0, &mut surf);
            if !surf.is_null() {
                (self.d3dx.load_surface_from_surface)(
                    surf,
                    std::ptr::null(),
                    std::ptr::null(),
                    rt.surface,
                    std::ptr::null(),
                    std::ptr::null(),
                    3,
                    0,
                );
                d3d::release(surf);
            }
        }
        tex as u32
    }
}
