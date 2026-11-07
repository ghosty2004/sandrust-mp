//! GUI panels: the scoreboard, the DXUT list box, dialogs and the death window.

use samp_ui::colour;
use samp_ui::deathwindow::{self, DeathWindow};
use samp_ui::dialog::{self, Dialog, DialogLayout, Style};
use samp_ui::fonts;

use super::{Renderer, SB_BACKGROUND, ScoreboardDraw, gdi_width, to_wide, video_height, video_width};
use crate::ui::d3d::{self, Font, Rect, dt};
use crate::ui::dxut::{self, EditView, ScrollBar};

/// Scoreboard text colour (server name, player count).
const SB_TEXT: u32 = 0xFFBE_BEBE;
/// Scoreboard column header colour.
const SB_HEADER: u32 = 0xFF95_B0D0;
/// Dialog background colour.
const DIALOG_BACKGROUND: u32 = 0xDC05_0505;

impl Renderer {
    // ---- scoreboard --------------------------------------------------------------------

    /// Scoreboard: DXUT dialog with the list box, then the texts (ui-rendering.md, "Scoreboard").
    pub(super) fn scoreboard(&self, sb: &ScoreboardDraw, cursor: (i32, i32)) {
        let (w, h) = (video_width(), video_height());
        let (sw, shh) = samp_ui::scoreboard::size(w);
        let (x, y) = (w / 2 - sw / 2, h / 2 - shh / 2);
        let rect = Rect { left: x, top: y, right: x + sw, bottom: y + shh };
        let g = &self.sprites.gui;
        dxut::dialog_begin(g, rect, SB_BACKGROUND, self.gui);
        let list = Rect { left: x, top: y + 60, right: x + sw, bottom: y + shh };
        let cw = [
            samp_ui::round_i(0.085 * f64::from(sw)),
            samp_ui::round_i(0.4375 * f64::from(sw)),
            samp_ui::round_i(0.265625 * f64::from(sw)),
        ];
        let cells: Vec<(u32, Vec<Vec<u8>>)> = sb.rows.iter().map(|r| (r.colour, r.cells.to_vec())).collect();
        let _ = cursor;
        self.list_box(list, &cells, 4, &cw, sb.selected, sb.scroll);
        dxut::dialog_end(g);
        let text_rect = Rect { left: x + 5, top: y + 5, right: x + sw, bottom: y + 60 };
        let flags = dt::SINGLELINE;
        self.small_text(sb.server_name, text_rect, flags, SB_TEXT);
        let mut m = Rect::default();
        self.fonts.small.draw_n(
            std::ptr::null_mut(),
            samp_ui::scoreboard::PLAYERS_SAMPLE,
            -1,
            &mut m,
            dt::CALCRECT,
            0xFF00_0000,
        );
        let px = x + sw - (m.right - m.left);
        self.small_text(&sb.players, Rect { left: px, ..text_rect }, flags, SB_TEXT);
        let top = y + 60 - self.fonts.small_h - 3;
        let mut hx = x + 10;
        for (i, hdr) in samp_ui::scoreboard::HEADERS.iter().enumerate() {
            let r = Rect { left: hx, top, right: hx + 46, bottom: y + 60 };
            self.small_text(hdr, r, flags, SB_HEADER);
            hx += cw.get(i).copied().unwrap_or(0);
        }
    }

    // ---- DXUT list box -----------------------------------------------------------------

    /// List box (DXUT `CDXUTListBox::Render` with samp's tablist columns): background,
    /// selection bar + rows from the scroll position, then the scrollbar. Each row is
    /// `(text colour, cells)`; `columns` cells are drawn at the column offsets.
    fn list_box(
        &self,
        list: Rect,
        rows: &[(u32, Vec<Vec<u8>>)],
        columns: usize,
        widths: &[i32],
        selected: Option<usize>,
        top_row: usize,
    ) {
        let g = &self.sprites.gui;
        let li = samp_ui::Rect::new(list.left, list.top, list.right, list.bottom);
        dxut::draw_sprite(g, self.gui, dxut::skin::LIST, list, dxut::colours::LIST_TEX);
        let sel = dialog::list_selection_rect(&li);
        let text = dialog::list_text_rect(&li);
        let row_h = self.fonts.gui1_height;
        let page = if row_h > 0 { (text.height() / row_h).max(1) } else { 1 };
        if !rows.is_empty() {
            let mut top = text.top;
            for (i, (colour, cells)) in rows.iter().enumerate().skip(top_row) {
                if top + row_h > text.bottom {
                    break;
                }
                if Some(i) == selected {
                    let r = Rect { left: sel.left, top, right: sel.right, bottom: top + row_h };
                    dxut::draw_sprite(g, self.gui, dxut::skin::LIST_SELECTION, r, dialog::SELECTION_COLOUR);
                }
                // DXUT draws the selected row with the selection element (white text).
                let c = if Some(i) == selected { dialog::ROW_COLOUR } else { *colour };
                let mut x = text.left;
                for k in 0..columns.max(1) {
                    let cell = cells.get(k).map(Vec::as_slice).unwrap_or(b"");
                    let r = Rect { left: x, top, right: text.right, bottom: top + row_h };
                    dxut::draw_text(g, &self.fonts.gui1, cell, r, dxut::fmt::LIST, c);
                    x += widths.get(k).copied().unwrap_or(0);
                }
                top += row_h;
            }
        }
        let sb = ScrollBar {
            rect: Rect {
                left: list.right - dialog::LIST_SCROLLBAR_W,
                top: list.top,
                right: list.right,
                bottom: list.bottom,
            },
            start: 0,
            end: rows.len() as i32,
            pos: top_row as i32,
            page,
        };
        dxut::scrollbar(g, self.gui, &sb);
    }

    // ---- dialog ------------------------------------------------------------------------

    /// Dialog: DXUT dialog (caption, buttons, list / edit box focused last), then the body or the style-5
    /// headers through the font renderer (ui-rendering.md, "Dialogs", "Per-frame draw").
    pub(super) fn dialog(&self, d: &Dialog, l: &DialogLayout, cursor: (i32, i32)) {
        let g = &self.sprites.gui;
        let rect = Rect { left: l.rect.left, top: l.rect.top, right: l.rect.right, bottom: l.rect.bottom };
        dxut::dialog_begin(g, rect, DIALOG_BACKGROUND, self.gui);
        if l.caption_h > 0 {
            dxut::caption(g, self.gui, &self.fonts.gui1, rect, l.caption_h, &d.title);
        }
        let rr = |r: samp_ui::Rect| Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let over = |r: samp_ui::Rect| r.contains(cursor.0, cursor.1);
        dxut::button(g, self.gui, &self.fonts.gui0, &d.button1, rr(l.button1), over(l.button1));
        if let Some(b2) = l.button2 {
            dxut::button(g, self.gui, &self.fonts.gui0, &d.button2, rr(b2), over(b2));
        }
        if let Some(list) = l.list {
            let rows: Vec<(u32, Vec<Vec<u8>>)> = d.rows().iter().map(|r| (dialog::ROW_COLOUR, r.clone())).collect();
            let cols = if d.style == Style::List { 1 } else { d.columns() };
            self.list_box(rr(list), &rows, cols, &l.columns, d.selected(), d.top_row());
        }
        if let Some(e) = l.edit {
            let wide = d.edit.display_wide(to_wide);
            let fv = dialog_first_visible(&self.fonts.gui0, &wide, d.edit.caret(), rr(e));
            let view = EditView {
                text: &wide,
                caret: d.edit.caret(),
                anchor: d.edit.anchor(),
                first_visible: fv,
                focused: true,
                insert_mode: d.edit.insert_mode(),
            };
            let f = &self.fonts.gui0;
            dxut::edit_box(g, self.gui, f, rr(e), &view, &|t| gdi_width(f, t));
        }
        dxut::dialog_end(g);
        if let Some(b) = &l.body {
            let r = Rect { left: b.x, top: b.y, right: rect.right, bottom: rect.bottom };
            self.small_text(&b.text, r, dt::EXPANDTABS, b.colour);
        }
        for hdr in &l.headers {
            let r = Rect { left: hdr.x, top: hdr.y, right: rect.right, bottom: rect.bottom };
            self.small_text(&hdr.text, r, dt::SINGLELINE, hdr.colour);
        }
    }

    // ---- death window --------------------------------------------------------------------

    /// Death window: 5 slots from `ftol(0.3·H)`, newest at the bottom (ui-rendering.md, "Death window").
    /// Mirrors samp.dll 0x1006A2E0.
    pub(super) fn death_window(&self, dw: &DeathWindow) {
        let s = &self.sprites.death;
        let f = &self.fonts;
        let (w, h) = (video_width(), video_height());
        let mut x = (0.75 * w as f32) as i32;
        let mut y = (0.3 * h as f32) as i32;
        if x + f.nick_w * 2 + f.icon_w > w {
            x = w - (f.icon_w + f.nick_w * 2);
        }
        s.begin(d3d::sprite::ALPHABLEND);
        let rows: Vec<_> = dw.rows().collect();
        let empty = deathwindow::ROWS - rows.len().min(deathwindow::ROWS);
        y += empty as i32 * (f.icon_h + 5);
        for r in rows {
            let (left_text, left_colour, right) = if r.killer.is_empty() {
                (&r.victim, r.victim_colour, None)
            } else {
                (&r.killer, r.killer_colour, Some((&r.victim, r.victim_colour)))
            };
            if left_text.is_empty() {
                y += f.icon_h + 5;
                continue;
            }
            let mut m = Rect::default();
            let mc = if r.killer.is_empty() { 0xFF00_0000 } else { 0xFFFF_FFFF };
            f.death_names.draw_n(std::ptr::null_mut(), left_text, -1, &mut m, dt::CALCRECT, mc);
            let lw = m.right - m.left;
            let rect = Rect { left: x + f.nick_w - lw, top: y, right: x + 60, bottom: y + 30 };
            self.death_name(left_text, rect, left_colour);
            let il = x + f.nick_w + 3;
            self.death_icon(Rect { left: il, top: y, right: il + 35, bottom: y + 30 }, r.reason);
            if let Some((vt, vc)) = right {
                let vl = il + f.icon_w;
                self.death_name(vt, Rect { left: vl, top: y, right: vl + 35, bottom: y + 30 }, vc);
            }
            y += f.icon_h + 5;
        }
        s.end();
    }

    /// Death window name with the 4-way outline in the same font. Mirrors samp.dll 0x10069C40.
    fn death_name(&self, text: &[u8], rect: Rect, colour: u32) {
        let s = &self.sprites.death;
        let f = &self.fonts.death_names;
        for &(dx, dy) in &fonts::OUTLINE_4 {
            let mut r = Rect { left: rect.left + dx, top: rect.top + dy, ..rect };
            f.draw_n(s.ptr, text, -1, &mut r, dt::NOCLIP, colour::OUTLINE);
        }
        let mut r = rect;
        f.draw_n(s.ptr, text, -1, &mut r, dt::NOCLIP, colour);
    }

    /// Death window icon box: backdrop `G`, its measure into the rect, the weapon glyph.
    /// Mirrors samp.dll 0x10069D40.
    fn death_icon(&self, rect: Rect, reason: u8) {
        let s = &self.sprites.death;
        let mut r = Rect { top: rect.top - 5, ..rect };
        self.fonts.death_backdrop.draw_n(s.ptr, b"G", -1, &mut r, dt::NOCLIP, 0xFF00_0000);
        self.fonts.death_backdrop.draw_n(s.ptr, b"G", -1, &mut r, dt::CALCRECT | dt::NOCLIP, 0xFF00_0000);
        let g = [deathwindow::glyph(reason)];
        let c = deathwindow::glyph_colour(reason);
        self.fonts.death_icon.draw_n(s.ptr, &g, -1, &mut r, dt::CENTER | dt::VCENTER | dt::NOCLIP, c);
    }
}

/// First visible character of a dialog edit box (the caret kept in view).
fn dialog_first_visible(font: &Font, text: &[u16], caret: usize, rect: Rect) -> usize {
    let width = rect.right - rect.left - 2 * (5 + 4);
    samp_ui::edit::place_caret(0, caret, text.len(), width, &|cp| gdi_width(font, &text[..cp]))
}
