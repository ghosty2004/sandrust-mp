//! Font-renderer text: outlined draws and the small-font overlays (audio stream title, F5 network
//! statistics).

use samp_ui::colour;
use samp_ui::fonts;

use super::{Renderer, video_height, video_width};
use crate::ui::d3d::{self, Font, Rect, Sprite, dt};

impl Renderer {
    /// Outline passes, then the main pass through the colour-code wrapper, all with `sprite`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn outlined(
        sprite: &Sprite,
        font: &Font,
        outline_font: &Font,
        text: &[u8],
        rect: Rect,
        flags: u32,
        colour: u32,
        outline: &[(i32, i32)],
    ) {
        let stripped = colour::strip_codes(text);
        // As in samp, the 4-way passes move only left/top; the 8-way chat passes (W > 1280) move the whole rect.
        let whole = outline.len() > 4;
        for &(dx, dy) in outline {
            let (rx, ry) = if whole { (dx, dy) } else { (0, 0) };
            let mut r =
                Rect { left: rect.left + dx, top: rect.top + dy, right: rect.right + rx, bottom: rect.bottom + ry };
            outline_font.draw(sprite.ptr, &stripped, &mut r, flags | dt::NOCLIP, colour::OUTLINE);
        }
        let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
        let mut r = rect;
        d3d::draw_glyph_coloured(font, sprite, &stripped, &mut r, flags | dt::NOCLIP, colour, glyphs);
    }

    /// Font-renderer small-font draw without the outline passes, in its own sprite Begin/End.
    /// Mirrors samp.dll 0x1006B4E0 with the outline off.
    fn small_text_plain(&self, text: &[u8], rect: Rect, flags: u32, colour: u32) {
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        let stripped = colour::strip_codes(text);
        let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
        let mut r = rect;
        d3d::draw_glyph_coloured(&self.fonts.small, s, &stripped, &mut r, flags | dt::NOCLIP, colour, glyphs);
        s.end();
    }

    /// Audio stream title: small font at (15, H − 20)–(W, H + 10) (ui-rendering.md, "Smaller overlays").
    /// Mirrors samp.dll 0x10066AB0.
    pub(super) fn audio_title(&self, text: &[u8]) {
        let (w, h) = (video_width(), video_height());
        let rect = Rect { left: 15, top: h - 20, right: w, bottom: h + 10 };
        self.small_text_plain(text, rect, crate::audio::TITLE_FLAGS, crate::audio::TITLE_COLOUR);
    }

    /// Font-renderer small-font draw with the 4-way outline, in its own sprite Begin/End.
    /// Mirrors samp.dll 0x1006B4E0.
    pub(super) fn small_text(&self, text: &[u8], rect: Rect, flags: u32, colour: u32) {
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, text, rect, flags, colour, &fonts::OUTLINE_4);
        s.end();
    }

    /// F5 overlay: the whole text and "Y" measured (`DT_CALCRECT`, no sprite), then the title
    /// `Client Network Stats` at (10, 10)–(W − 150, 10 + h) in 0xFF8888EE and every `\n`-terminated line below
    /// it in white, each through the font renderer with its 4-way outline and its own sprite Begin/End.
    /// Mirrors samp.dll 0x10060D70.
    pub(super) fn net_stats(&self, text: &[u8]) {
        let _ = self.fonts.main.measure_z(&colour::strip_codes(text), 0);
        let h = self.fonts.main.measure_z(b"Y", 0).1;
        let right = video_width() - 150;
        let line = |t: &[u8], top: i32, c: u32| {
            let s = &self.sprites.text;
            s.begin(d3d::sprite::ALPHABLEND);
            let rect = Rect { left: 10, top, right, bottom: top + h };
            Self::outlined(s, &self.fonts.main, &self.fonts.main_outline, t, rect, 0, c, &fonts::OUTLINE_4);
            s.end();
        };
        line(b"Client Network Stats", 10, 0xFF88_88EE);
        let mut top = 10 + h;
        let mut rest = text;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            line(&rest[..i], top, 0xFFFF_FFFF);
            top += h;
            rest = &rest[i + 1..];
        }
    }
}
