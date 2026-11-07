//! The chat window (render-target cache or direct draw) and the full-screen chat GUI dialog with the chat
//! scrollbar, the input edit box and the class-selection panel.

use samp_ui::chat::{ChatLayout, ChatWindow};
use samp_ui::input::{self, ChatInput};

use super::{Renderer, SB_BACKGROUND, client_size, gdi_width, to_wide, video_width};
use crate::ui::d3d::{self, Rect, devcall, dt};
use crate::ui::dxut::{self, EditView, ScrollBar};

/// Chat GUI dialog background: drawn like samp does, but fully transparent.
const CHAT_GUI_BACKGROUND: u32 = 0x0000_0000;

impl Renderer {
    /// Draws the visible chat lines with the chat sprite (ui-rendering.md, "Chat window", "Layout").
    fn chat_lines(&self, layout: &ChatLayout) {
        let s = &self.sprites.chat;
        for line in &layout.lines {
            for item in &line.items {
                let rect = Rect { left: item.x, top: item.y, right: line.rect.right, bottom: line.rect.bottom };
                Self::outlined(
                    s,
                    &self.fonts.main,
                    &self.fonts.main_outline,
                    &item.text,
                    rect,
                    dt::SINGLELINE,
                    item.colour,
                    layout.outline,
                );
            }
        }
    }

    /// Render-target update, once per game frame from the `CGame::Process` hook: invalid while the GTA menu is
    /// on; redrawn when the chat is dirty or the scroll position changed. Mirrors samp.dll 0x10067ED0.
    pub fn update_chat_rt(&mut self, chat: &mut ChatWindow, menu: bool) {
        if menu {
            self.rt_valid = false;
            return;
        }
        if !chat.mode().chat_visible() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        if self.rt_valid && !chat.is_dirty() && self.rt_scroll == chat.scroll_pos() {
            return;
        }
        let layout = chat.layout(&self.fonts, video_width());
        if rt.begin() < 0 {
            return;
        }
        devcall::clear(3, 0, 0.0, 0);
        let s = &self.sprites.chat;
        s.begin(d3d::sprite::ALPHABLEND);
        devcall::set_render_state(20, 14); // D3DRS_DESTBLEND = D3DBLEND_BLENDFACTOR, as samp sets it
        self.chat_lines(&layout);
        s.end();
        rt.end();
        self.rt_valid = true;
        self.rt_scroll = chat.scroll_pos();
        chat.clear_dirty();
    }

    /// Present step: the chat window, a blit of the render target or a direct draw (ui-rendering.md,
    /// "Draw path").
    pub(super) fn chat(&self, chat: &ChatWindow) {
        if !chat.mode().chat_visible() {
            return;
        }
        if let (Some(rt), true) = (&self.rt, self.rt_valid) {
            let s = &self.sprites.blit;
            s.begin(d3d::sprite::ALPHABLEND);
            s.draw(rt.tex, None, None, None, 0xFFFF_FFFF);
            s.end();
        } else {
            let layout = chat.layout(&self.fonts, video_width());
            let s = &self.sprites.chat;
            s.begin(d3d::sprite::ALPHABLEND);
            self.chat_lines(&layout);
            s.end();
        }
    }

    /// The full-screen chat / input GUI dialog: the chat scrollbar and the input edit box (ui-rendering.md,
    /// "Scrollbar and paging", "Chat input").
    pub(super) fn chat_gui(&mut self, chat: &ChatWindow, inp: &ChatInput, first_visible: usize) {
        let bottom = chat.bottom(self.fonts.main_h);
        let (cw, _) = client_size();
        let box_r = input::box_rect(cw, bottom, self.fonts.gui0_height);
        if inp.is_open() {
            self.host_size = input::host_dialog_size(box_r.width(), bottom);
        }
        let g = &self.sprites.gui;
        let host = Rect { left: 0, top: 0, right: self.host_size.0, bottom: self.host_size.1 };
        dxut::dialog_begin(g, host, CHAT_GUI_BACKGROUND, self.gui);
        if chat.scrollbar_visible(inp.is_open()) {
            let r = chat.scrollbar_rect(self.fonts.main_h);
            let sb = ScrollBar {
                rect: Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
                start: samp_ui::chat::SCROLL_MIN as i32,
                end: samp_ui::chat::SCROLL_END as i32,
                pos: chat.scroll_pos() as i32,
                page: chat.page_size() as i32,
            };
            dxut::scrollbar(g, self.gui, &sb);
        }
        if inp.is_open() {
            let e = inp.edit();
            let wide = to_wide(e.text());
            let view = EditView {
                text: &wide,
                caret: e.caret(),
                anchor: e.anchor(),
                first_visible,
                focused: true,
                insert_mode: e.insert_mode(),
            };
            let r = Rect { left: box_r.left, top: box_r.top, right: box_r.right, bottom: box_r.bottom };
            let f = &self.fonts.gui0;
            dxut::edit_box(g, self.gui, f, r, &view, &|t| gdi_width(f, t));
        }
        dxut::dialog_end(g);
    }

    /// Class-selection panel (ui-rendering.md, "GUI dialogs created by samp"): 310×40 on the scoreboard
    /// background, centred at `y = clientH − 40 − 50`; buttons `<<` (10,5), `>>` (110,5), `Spawn` (210,5), 90×30.
    /// Returns the three button rects.
    pub fn class_panel(&self, cursor: (i32, i32)) -> [samp_ui::Rect; 3] {
        let (cw, ch) = client_size();
        let (x, y) = ((cw - 310) / 2, ch - 40 - 50);
        let rects = [10, 110, 210].map(|dx| samp_ui::Rect::xywh(x + dx, y + 5, 90, 30));
        let g = &self.sprites.gui;
        dxut::dialog_begin(g, Rect { left: x, top: y, right: x + 310, bottom: y + 40 }, SB_BACKGROUND, self.gui);
        for (r, label) in rects.iter().zip([&b"<<"[..], b">>", b"Spawn"]) {
            let rr = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
            dxut::button(g, self.gui, &self.fonts.gui0, label, rr, r.contains(cursor.0, cursor.1));
        }
        dxut::dialog_end(g);
        rects
    }

    /// The chat input's first visible character after a caret move (DXUT `PlaceCaret`).
    pub fn input_first_visible(&self, chat: &ChatWindow, inp: &ChatInput, first: usize) -> usize {
        let (cw, _) = client_size();
        let r = input::box_rect(cw, chat.bottom(self.fonts.main_h), self.fonts.gui0_height);
        let wide = to_wide(inp.edit().text());
        let width = r.width() - 2 * (5 + 4);
        let f = &self.fonts.gui0;
        samp_ui::edit::place_caret(first, inp.edit().caret(), wide.len(), width, &|cp| gdi_width(f, &wide[..cp]))
    }
}
