//! Object editor gizmo, object-selection highlight and the `/ctd` camera-target labels.

use samp_ui::fonts;

use super::Renderer;
use crate::objedit::{self, Editor};
use crate::ui::d3d::{self, Rect, dt};

impl Renderer {
    /// One object-editor handle: the background glyph (SAMPAUX3 28), then the handle glyph (SAMPAUX3 22), one
    /// character each at the point, format 0x105; the hit rect is centred on the point.
    /// Mirrors samp.dll B 0x10072570.
    fn editor_handle(&self, ed: &mut Editor, index: i32, x: i32, y: i32) {
        let (Some(small), Some(big)) = (&self.unused.editor_fonts.0, &self.unused.editor_fonts.1) else { return };
        let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        big.draw_n(
            std::ptr::null_mut(),
            objedit::GLYPH_BACKGROUND,
            1,
            &mut r,
            objedit::FMT_HANDLE,
            ed.background(index),
        );
        let (glyph, argb) = ed.glyph_of(index);
        let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        small.draw_n(std::ptr::null_mut(), glyph, 1, &mut r, objedit::FMT_HANDLE, argb);
        ed.place(index, x, y);
    }

    /// Both ends of `axis` (±`s` in the matrix frame) on screen; `None` when either z > 1.
    fn axis_ends(&self, m: &[f32; 16], axis: usize, s: f32) -> Option<([f32; 3], [f32; 3])> {
        let mut v = [0.0f32; 3];
        v[axis] = s;
        let a = self.project(objedit::transform(m, v));
        if a[2] > 1.0 {
            return None;
        }
        v[axis] = -s;
        let b = self.project(objedit::transform(m, v));
        (b[2] <= 1.0).then_some((a, b))
    }

    /// The object-editor gizmo: glyph probe, antialiased line per axis (aborting at the first end point behind the
    /// camera), axis handle at the + end; save / scale / rotate / translate buttons left of the − end of Z.
    /// Returns whether all of it was drawn. Mirrors samp.dll B 0x10072C60.
    pub fn editor_gizmo(&self, ed: &mut Editor, m: &[f32; 16], s: f32) -> bool {
        let Some(line) = &self.unused.editor_line else { return false };
        if let Some(big) = &self.unused.editor_fonts.1 {
            // samp passes an uninitialised rect; the size of the result is what it keeps.
            let mut r = Rect::default();
            big.draw_n(
                std::ptr::null_mut(),
                objedit::GLYPH_BACKGROUND,
                -1,
                &mut r,
                objedit::FMT_PROBE,
                objedit::PROBE_COLOUR,
            );
            ed.glyph = (r.right - r.left, r.bottom - r.top);
        }
        line.set_antialias(true);
        let mut last_b = [0.0f32; 3];
        for axis in 0..3 {
            let Some((a, b)) = self.axis_ends(m, axis, s) else { return false };
            line.segment([a[0], a[1]], [b[0], b[1]], objedit::AXIS_COLOURS[axis]);
            ed.set_direction(axis, [a[0], a[1]], [b[0], b[1]]);
            self.editor_handle(ed, axis as i32, a[0] as i32, a[1] as i32);
            last_b = b;
        }
        let (mut x, y) = (last_b[0] as i32, last_b[1] as i32);
        self.editor_handle(ed, objedit::HANDLE_SAVE, x, y);
        let step = -2 - ed.glyph.0;
        if ed.show_scale {
            x += step;
            self.editor_handle(ed, 5, x, y);
        } else {
            ed.clear_rect(5);
        }
        x += step;
        self.editor_handle(ed, 4, x, y);
        x += step;
        self.editor_handle(ed, 3, x, y);
        true
    }

    /// Selection highlight: the object's axis cross (length 1, one colour) and the id / model / distance label
    /// through the 3D-text painter with the outline on and no line-of-sight test (outlined small text, rect
    /// [x, y, x+1, y+1], format 0x101). Mirrors samp.dll B 0x1006DA20.
    pub fn select_highlight(&self, m: &[f32; 16], label: &[u8]) {
        if let Some(line) = &self.unused.editor_line {
            line.set_antialias(true);
            for axis in 0..3 {
                let Some((a, b)) = self.axis_ends(m, axis, objedit::SELECT_CROSS) else { break };
                line.segment([a[0], a[1]], [b[0], b[1]], objedit::SELECT_LINE_COLOUR);
            }
        }
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        let pos = [m[12], m[13], m[14]];
        {
            let p = self.project(pos);
            if p[2] <= 1.0 {
                let (x, y) = (p[0] as i32, p[1] as i32);
                let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
                Self::outlined(
                    s,
                    &self.fonts.small,
                    &self.fonts.small_outline,
                    label,
                    rect,
                    dt::CENTER,
                    objedit::SELECT_LABEL_COLOUR,
                    &fonts::OUTLINE_4,
                );
            }
        }
        s.end();
    }

    /// `/ctd`: one sprite begin; per camera target the editor's axis cross on the entity matrix (length 1) and its
    /// label at the matrix position (3D-text painter, outline on, no line-of-sight test); one sprite end.
    /// Mirrors samp.dll B 0x10004830.
    pub fn camera_target_labels(&self, items: &[([f32; 16], Vec<u8>)]) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for (m, label) in items {
            if let Some(line) = &self.unused.editor_line {
                line.set_antialias(true);
                for axis in 0..3 {
                    let Some((a, b)) = self.axis_ends(m, axis, objedit::SELECT_CROSS) else { break };
                    line.segment([a[0], a[1]], [b[0], b[1]], objedit::SELECT_LINE_COLOUR);
                }
            }
            let p = self.project([m[12], m[13], m[14]]);
            if p[2] <= 1.0 {
                let (x, y) = (p[0] as i32, p[1] as i32);
                let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
                Self::outlined(
                    s,
                    &self.fonts.small,
                    &self.fonts.small_outline,
                    label,
                    rect,
                    dt::CENTER,
                    crate::camtarget::LABEL_COLOUR,
                    &fonts::OUTLINE_4,
                );
            }
        }
        s.end();
    }
}
