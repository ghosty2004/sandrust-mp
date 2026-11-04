//! The D3D UI pass (Windows only): fonts (UR §3.2), the colour-code text path
//! (UR §3.3/§3.4), the chat window (UR §7), its scrollbar (UR §7.4) and the chat
//! input box (UR §8). Called from the Present wrapper with the real device.
//!
//! Edit box, caret, selection and scrollbar geometry/colours follow TRACE-3 batch 1
//! (D3D call log of the original at 1280×720, docs/phase3/m4-notes.md). Still
//! TODO-unknown: how text longer than the box scrolls (U-EDIT-SCROLL).

use std::ffi::c_void;

use samp_ui::chat::{ChatLayout, ChatWindow};
use samp_ui::colour;
use samp_ui::fonts::{self, FontId, Size, TextMeasure};
use samp_ui::input::{self, ChatInput};

use super::d3d::{self, D3dx, Font, FontArgs, Rect, Sprite, dt};

/// `RsGlobal.maximumWidth` / `maximumHeight` (UR §3.2 `W`).
const RS_MAX_WIDTH: u32 = 0xC17044;
/// GTA main window handle.
const GTA_HWND: u32 = 0xC97C1C;

/// sampgui.png element rects (UR §4.1 table).
mod skin {
    use super::Rect;
    const fn r(l: i32, t: i32, rr: i32, b: i32) -> Rect {
        Rect { left: l, top: t, right: rr, bottom: b }
    }
    /// Edit box: text area, then the 8 border pieces of the frame (8,82)–(246,121)
    /// in DXUT element order (TL, T, TR, L, R, BL, B, BR).
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
}

/// Chat input edit-box colours (UR §4.1, "Edit boxes in samp's own dialogs and in
/// the chat input").
mod edit_colours {
    pub const TEXT: u32 = 0xFFFF_FFFF;
    /// Text area (element 0).
    pub const FRAME: u32 = 0xF005_0505;
    /// The eight border pieces (TRACE-3: 0xFFFFFFFF).
    pub const BORDER: u32 = 0xFFFF_FFFF;
    /// TRACE-3: caret, selection background (quad drawn before the text) and
    /// selected text (substring redrawn after the full text).
    pub const CARET: u32 = 0xFF96_9696;
    pub const SELECTION_BG: u32 = 0xFFB9_2228;
    pub const SELECTED_TEXT: u32 = 0xFF0A_0A0F;
}

/// Scrollbar thumb tint (TRACE-3; track and buttons 0xFFFFFFFF).
const SCROLL_THUMB_COLOUR: u32 = 0xFFB9_2228;

/// Edit-box border (outer rect → centre piece) and text inset (centre → text rect),
/// both on all four sides (TRACE-3: outer 40,197–808,241, centre 45,202–803,236,
/// text 49,206–799,232).
const EDIT_BORDER: i32 = 5;
const EDIT_SPACING: i32 = 4;
/// Caret: a 2-px quad from caretX − 1.5 to caretX + 0.5 over the text rect height,
/// i.e. pixels [caretX − 1, caretX + 1); drawn every frame (no blink) (TRACE-3).
const CARET_LEFT: i32 = 1;
const CARET_W: i32 = 2;

pub struct Fonts {
    main: Font,
    main_outline: Font,
    small: Font,
    small_outline: Font,
    gui0: Font,
    /// Height of "Y" in the main font (font renderer +0x20).
    pub main_h: i32,
    pub gui0_height: i32,
}

/// Builds the font-renderer and GUI fonts (UR §3.2 table).
fn build_fonts(d3dx: &D3dx, video_w: i32, fs: i32, face: &[u8], weight: u32) -> Result<Fonts, String> {
    let main_h = fonts::main_height(video_w, fs);
    let small_h = fonts::small_height(video_w, fs);
    let gui_h = fonts::gui_height(fs);
    let args =
        |height: i32, quality: u32| FontArgs { height, width: 0, weight, charset: 1, quality, face: face.to_vec() };
    let main = Font::create(d3dx, args(main_h, 4))?;
    let main_outline = Font::create(d3dx, args(main_h, 4))?;
    let small = Font::create(d3dx, args(small_h, 4))?;
    let small_outline = Font::create(d3dx, args(small_h, 4))?;
    let gui0 = Font::create(d3dx, args(gui_h, 0))?;
    let y = colour::strip_codes(b"Y");
    let main_line = main.measure(&y, dt::SINGLELINE).1;
    crate::log!(
        "ui: fonts face={:?} weight={weight} main={main_h} small={small_h} gui={gui_h} (fontsize {fs}, W {video_w}); \"Y\" main {}x{}",
        String::from_utf8_lossy(face),
        main.measure(&y, dt::SINGLELINE).0,
        main_line
    );
    Ok(Fonts { main, main_outline, small, small_outline, gui0, main_h: main_line, gui0_height: gui_h })
}

impl Fonts {
    fn font(&self, id: FontId) -> &Font {
        match id {
            FontId::Main => &self.main,
            FontId::Small => &self.small,
            FontId::Gui0 | FontId::Gui1 => &self.gui0,
            FontId::DeathNames | FontId::DeathIcon => &self.small,
        }
    }
}

impl TextMeasure for Fonts {
    fn measure(&self, font: FontId, text: &[u8]) -> Size {
        let (w, h) = self.font(font).measure(text, dt::SINGLELINE);
        Size { w, h }
    }
}

/// The renderer: D3DX objects created on the first Present.
pub struct Renderer {
    d3dx: D3dx,
    sprite: Sprite,
    pub fonts: Fonts,
    gui: *mut c_void,
    white: *mut c_void,
    cursor: *mut c_void,
    built_for: (i32, i32),
    /// First visible character of the input text.
    first_visible: usize,
    glyph_checks: u32,
}

pub fn video_width() -> i32 {
    // SAFETY: RsGlobal field read on the main thread.
    unsafe { std::ptr::read_volatile(RS_MAX_WIDTH as usize as *const i32) }
}

/// Client rect of the GTA window.
pub fn client_size() -> (i32, i32) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    // SAFETY: reading the game's HWND global and a Win32 query.
    unsafe {
        let hwnd = std::ptr::read_volatile(GTA_HWND as usize as *const usize) as *mut c_void;
        let mut r: RECT = std::mem::zeroed();
        if hwnd.is_null() || GetClientRect(hwnd, &mut r) == 0 {
            return (video_width(), 0);
        }
        (r.right - r.left, r.bottom - r.top)
    }
}

impl Renderer {
    pub fn new(face: &[u8], weight: u32, font_size: i32) -> Result<Self, String> {
        let d3dx = D3dx::load()?;
        let sprite = Sprite::create(&d3dx)?;
        let w = video_width();
        let fonts = build_fonts(&d3dx, w, font_size, face, weight)?;
        let gui = match d3d::load_texture(&d3dx, b"sampgui.png") {
            Ok(t) => t,
            Err(e) => {
                crate::log!("ui: {e}; the input box and scrollbar are not drawn");
                std::ptr::null_mut()
            }
        };
        let white = d3d::white_texture().unwrap_or_else(|e| {
            crate::log!("ui: {e}");
            std::ptr::null_mut()
        });
        let cursor = match d3d::create_cursor(&d3dx, b"mouse.png") {
            Ok(s) => s,
            Err(e) => {
                crate::log!("ui: cursor: {e}");
                std::ptr::null_mut()
            }
        };
        Ok(Self {
            d3dx,
            sprite,
            fonts,
            gui,
            white,
            cursor,
            built_for: (w, font_size),
            first_visible: 0,
            glyph_checks: 0,
        })
    }

    /// `/fontsize` or a video-mode change: rebuild the font-renderer fonts.
    pub fn rebuild_fonts(&mut self, face: &[u8], weight: u32, font_size: i32) {
        let w = video_width();
        match build_fonts(&self.d3dx, w, font_size, face, weight) {
            Ok(f) => {
                self.fonts = f;
                self.built_for = (w, font_size);
            }
            Err(e) => crate::log!("ui: font rebuild failed: {e}"),
        }
    }

    pub fn needs_rebuild(&self, font_size: i32) -> bool {
        self.built_for != (video_width(), font_size)
    }

    pub fn on_lost(&self) {
        self.sprite.on_lost();
        for f in
            [&self.fonts.main, &self.fonts.main_outline, &self.fonts.small, &self.fonts.small_outline, &self.fonts.gui0]
        {
            f.on_lost();
        }
    }

    pub fn on_reset(&mut self) {
        self.sprite.on_reset();
        for f in
            [&self.fonts.main, &self.fonts.main_outline, &self.fonts.small, &self.fonts.small_outline, &self.fonts.gui0]
        {
            f.on_reset();
        }
        // UR §2.2: the cursor surface is re-created after a reset.
        // SAFETY: we own the old surface.
        unsafe { d3d::release(self.cursor) };
        self.cursor = d3d::create_cursor(&self.d3dx, b"mouse.png").unwrap_or(std::ptr::null_mut());
    }

    /// Text through the colour-code wrapper with an outline pass (UR §3.3, §3.4).
    fn draw_text(&mut self, main: bool, text: &[u8], rect: Rect, flags: u32, colour: u32, outline: &[(i32, i32)]) {
        let stripped = colour::strip_codes(text);
        let (font, outline_font) = if main {
            (&self.fonts.main, &self.fonts.main_outline)
        } else {
            (&self.fonts.small, &self.fonts.small_outline)
        };
        for &(dx, dy) in outline {
            let mut r =
                Rect { left: rect.left + dx, top: rect.top + dy, right: rect.right + dx, bottom: rect.bottom + dy };
            outline_font.draw(self.sprite.ptr, &stripped, &mut r, flags | dt::NOCLIP, colour::OUTLINE);
        }
        let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
        let expected = glyphs.len();
        let mut r = rect;
        let drawn = d3d::draw_glyph_coloured(font, &self.sprite, &stripped, &mut r, flags, colour, glyphs);
        if self.glyph_checks < 5 && expected > 0 {
            self.glyph_checks += 1;
            crate::log!(
                "ui: glyph check {:?}: {expected} visible glyphs, D3DX made {drawn} sprite draws{}",
                String::from_utf8_lossy(&stripped),
                if drawn == expected { "" } else { " (MISMATCH: per-glyph colours are off)" }
            );
        }
    }

    /// Chat window (UR §7.2, §7.5; direct mode — see notes for the RT cache).
    fn draw_chat(&mut self, layout: &ChatLayout) {
        if !layout.visible {
            return;
        }
        for line in &layout.lines {
            for item in &line.items {
                let rect = Rect { left: item.x, top: item.y, right: line.rect.right, bottom: line.rect.bottom };
                self.draw_text(true, &item.text, rect, dt::SINGLELINE | dt::NOCLIP, item.colour, layout.outline);
            }
        }
    }

    /// Solid rectangle (DXUT `DrawRect`).
    fn fill(&self, dst: Rect, colour: u32) {
        if !self.white.is_null() && dst.right > dst.left && dst.bottom > dst.top {
            self.sprite.draw_scaled(self.white, Rect { left: 0, top: 0, right: 1, bottom: 1 }, dst, colour);
        }
    }

    fn draw_skin(&self, src: Rect, dst: Rect, colour: u32) {
        if !self.gui.is_null() && dst.right > dst.left && dst.bottom > dst.top {
            self.sprite.draw_scaled(self.gui, src, dst, colour);
        }
    }

    /// Chat scrollbar (UR §7.4): square buttons, track between them, thumb height
    /// `max(8, trackH·page/99)` at `track.top + (pos − 1)·(trackH − thumbH)/(99 − page)`
    /// (TRACE-3: page 10 → thumb 20×8, top 60 + trunc((pos − 1)·62/89)).
    fn draw_scrollbar(&self, chat: &ChatWindow) {
        let r = chat.scrollbar_rect(self.fonts.main_h);
        let b = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let w = b.right - b.left;
        if b.bottom - b.top < 2 * w {
            return;
        }
        let up = Rect { bottom: b.top + w, ..b };
        let down = Rect { top: b.bottom - w, ..b };
        let track = Rect { top: up.bottom, bottom: down.top, ..b };
        self.draw_skin(skin::SCROLL_TRACK, track, 0xFFFF_FFFF);
        self.draw_skin(skin::SCROLL_UP, up, 0xFFFF_FFFF);
        self.draw_skin(skin::SCROLL_DOWN, down, 0xFFFF_FFFF);
        let (start, end, page) = (1i32, 100i32, chat.page_size() as i32);
        if end - start > page {
            let th = track.bottom - track.top;
            let thumb_h = (th * page / (end - start)).max(8);
            let max_pos = end - start - page;
            let pos = (chat.scroll_pos() as i32 - start).clamp(0, max_pos);
            let top = track.top + pos * (th - thumb_h) / max_pos;
            self.draw_skin(skin::SCROLL_THUMB, Rect { top, bottom: top + thumb_h, ..b }, SCROLL_THUMB_COLOUR);
        }
    }

    /// Chat input edit box (UR §8, §4.1).
    fn draw_input(&mut self, input: &ChatInput, chat_bottom: i32) {
        let (cw, _) = client_size();
        let r = input::box_rect(cw, chat_bottom, self.fonts.gui0_height);
        let bb = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let t = Rect {
            left: bb.left + EDIT_BORDER,
            top: bb.top + EDIT_BORDER,
            right: bb.right - EDIT_BORDER,
            bottom: bb.bottom - EDIT_BORDER,
        };
        let dst = [
            t,
            Rect { left: bb.left, top: bb.top, right: t.left, bottom: t.top },
            Rect { left: t.left, top: bb.top, right: t.right, bottom: t.top },
            Rect { left: t.right, top: bb.top, right: bb.right, bottom: t.top },
            Rect { left: bb.left, top: t.top, right: t.left, bottom: t.bottom },
            Rect { left: t.right, top: t.top, right: bb.right, bottom: t.bottom },
            Rect { left: bb.left, top: t.bottom, right: t.left, bottom: bb.bottom },
            Rect { left: t.left, top: t.bottom, right: t.right, bottom: bb.bottom },
            Rect { left: t.right, top: t.bottom, right: bb.right, bottom: bb.bottom },
        ];
        for (i, (src, d)) in skin::EDIT.iter().zip(dst).enumerate() {
            self.draw_skin(*src, d, if i == 0 { edit_colours::FRAME } else { edit_colours::BORDER });
        }
        // Text, scrolled so the caret stays visible (TODO-unknown U-EDIT-SCROLL: the
        // first-visible rule for text wider than the box).
        let edit = input.edit();
        let text = edit.display_text();
        let caret = edit.caret().min(text.len());
        let area = Rect {
            left: t.left + EDIT_SPACING,
            top: t.top + EDIT_SPACING,
            right: t.right - EDIT_SPACING,
            bottom: t.bottom - EDIT_SPACING,
        };
        let width = (area.right - area.left).max(1);
        let f = &self.fonts.gui0;
        let wof = |a: usize, b: usize| f.measure(&text[a..b], dt::SINGLELINE).0;
        if caret < self.first_visible || self.first_visible > text.len() {
            self.first_visible = caret;
        }
        while self.first_visible < caret && wof(self.first_visible, caret) > width {
            self.first_visible += 1;
        }
        let first = self.first_visible;
        let mut end = text.len();
        while end > first && wof(first, end) > width {
            end -= 1;
        }
        let sel = edit.selection();
        if !sel.is_empty() {
            let a = sel.start.clamp(first, end);
            let b = sel.end.clamp(first, end);
            if a < b {
                let x0 = area.left + wof(first, a);
                let x1 = area.left + wof(first, b);
                self.fill(Rect { left: x0, top: area.top, right: x1, bottom: area.bottom }, edit_colours::SELECTION_BG);
            }
        }
        let mut rr = area;
        // TRACE-3: DrawTextW, GUI font 0, DT_VCENTER only.
        f.draw(self.sprite.ptr, &text[first..end], &mut rr, dt::VCENTER, edit_colours::TEXT);
        // Selected characters again in the selected-text colour (U-EDIT-SLOTS).
        let (a, b) = (sel.start.clamp(first, end), sel.end.clamp(first, end));
        if a < b {
            let mut rs = Rect { left: area.left + wof(first, a), ..area };
            f.draw(self.sprite.ptr, &text[a..b], &mut rs, dt::VCENTER, edit_colours::SELECTED_TEXT);
        }
        let cx = area.left + wof(first, caret.max(first));
        let cl = cx - CARET_LEFT;
        self.fill(Rect { left: cl, top: area.top, right: cl + CARET_W, bottom: area.bottom }, edit_colours::CARET);
    }

    /// The chat part of the Present pass (UR §2.3 steps 11–12).
    pub fn frame(&mut self, chat: &ChatWindow, input: &ChatInput) {
        let layout = chat.layout(&self.fonts, video_width());
        if d3d::begin_scene() < 0 {
            return;
        }
        if self.sprite.begin(d3d::sprite::ALPHABLEND) >= 0 {
            self.draw_chat(&layout);
            // GUI dialog 0x1026ea9c: scrollbar, then the input edit box.
            if chat.scrollbar_visible(input.is_open()) {
                self.draw_scrollbar(chat);
            }
            if input.is_open() {
                self.draw_input(input, layout.bottom);
            } else {
                self.first_visible = 0;
            }
            self.sprite.end();
        }
        d3d::end_scene();
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: we own these references.
        unsafe {
            d3d::release(self.gui);
            d3d::release(self.white);
            d3d::release(self.cursor);
        }
    }
}
