//! Font settings and the pluggable text-measure interface (UR §3).

/// The fonts the layouts measure with (UR §3.2 table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontId {
    /// Font renderer main font (chat lines, net stats).
    Main,
    /// Font renderer small font (main − 2): nametags, bubbles, dialog body, scoreboard texts.
    Small,
    /// GUI font 0: buttons, edit boxes, captions.
    Gui0,
    /// GUI font 1 (GUI − 2): static texts, dialog lists.
    Gui1,
    /// Death-window names font (aux height).
    DeathNames,
    /// Death-window GTAWEAPON3 glyph font (aux + 8).
    DeathIcon,
}

/// Width and height in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

/// Text metrics supplied by the renderer (D3DX `DrawText` with `DT_CALCRECT`).
///
/// `text` never contains colour codes: callers strip them first, as the original
/// does (UR §3.3).
pub trait TextMeasure {
    /// Size of a single line (`DT_SINGLELINE`).
    fn measure(&self, font: FontId, text: &[u8]) -> Size;

    /// Height of `"Y"`, the line height every layout uses (UR §3.1 +0x20/+0x24, §7.2).
    fn line_height(&self, font: FontId) -> i32 {
        self.measure(font, b"Y").h
    }

    /// Width of the chat's timestamp sample (`[19:58:34]`, main font), measured once by samp's
    /// chat window at construction; hosts that cached it override this.
    fn timestamp_width(&self) -> i32 {
        self.measure(FontId::Main, crate::chat::TIMESTAMP_SAMPLE).w
    }

    /// Size of a multi-line block (`DT_EXPANDTABS`, lines split on `\n`).
    /// Default: widest line × (lines · line height).
    fn measure_block(&self, font: FontId, text: &[u8]) -> Size {
        let mut w = 0;
        let mut n = 0;
        for line in text.split(|&b| b == b'\n') {
            w = w.max(self.measure(font, line).w);
            n += 1;
        }
        Size { w, h: n * self.line_height(font) }
    }
}

/// Fixed-pitch measure for tests and headless use: every byte is `char_w` wide.
#[derive(Clone, Copy, Debug)]
pub struct MonoMeasure {
    pub char_w: i32,
    pub line_h: i32,
}

impl TextMeasure for MonoMeasure {
    fn measure(&self, _font: FontId, text: &[u8]) -> Size {
        Size { w: self.char_w * text.len() as i32, h: self.line_h }
    }
}

/// Default `fontface` (UR §3.2, A 0x100c53b0).
pub const DEFAULT_FACE: &str = "Arial";
/// `/fontsize` range (UCA local commands).
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<i32> = -3..=5;

/// Main font height for video-mode width `w` and config `fontsize` `fs` (A 0x100c52d0).
pub fn main_height(w: i32, fs: i32) -> i32 {
    let base = if w < 1024 {
        14
    } else if w < 1400 {
        16
    } else if w < 1600 {
        18
    } else {
        20
    };
    base + 2 * fs
}

/// Small font height: main − 2 (UR §3.2 table).
pub fn small_height(w: i32, fs: i32) -> i32 {
    main_height(w, fs) - 2
}

/// Aux height (death window names) (A 0x100c5320): never below its base.
pub fn aux_height(w: i32, fs: i32) -> i32 {
    let base = if w < 1024 { 12 } else { 14 };
    (base + 2 * fs).max(base)
}

/// GUI font 0 height, independent of resolution (A 0x100c5360). GUI font 1 is 2 less.
pub fn gui_height(fs: i32) -> i32 {
    20 + 2 * fs
}

/// Font weight from config `fontweight` (A 0x100c5380): 1 → 400, anything else → 700.
pub fn weight(fontweight: Option<i32>) -> u32 {
    if fontweight == Some(1) { 400 } else { 700 }
}

/// Font face from config `fontface` (A 0x100c53b0).
pub fn face(fontface: Option<&str>) -> &str {
    fontface.unwrap_or(DEFAULT_FACE)
}

/// Four-direction outline (UR §3.4).
pub const OUTLINE_4: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
/// Eight-offset outline used by chat lines when the video width is above 1280 (UR §3.4).
pub const OUTLINE_8: [(i32, i32); 8] = [(0, -2), (-1, -1), (1, -1), (0, 2), (-1, 1), (1, 1), (-2, 0), (2, 0)];

/// Chat line outline offsets for video-mode width `w` (UR §3.4).
pub fn chat_outline(w: i32) -> &'static [(i32, i32)] {
    if w <= 1280 { &OUTLINE_4 } else { &OUTLINE_8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ur_3_2_main_height_table() {
        assert_eq!(main_height(800, 0), 14);
        assert_eq!(main_height(1023, 0), 14);
        assert_eq!(main_height(1024, 0), 16);
        assert_eq!(main_height(1399, 0), 16);
        assert_eq!(main_height(1400, 0), 18);
        assert_eq!(main_height(1600, 0), 20);
        assert_eq!(main_height(1920, 5), 30);
        assert_eq!(main_height(800, -3), 8);
        assert_eq!(small_height(1280, 0), 14);
    }

    #[test]
    fn ur_3_2_aux_gui_weight_face() {
        assert_eq!(aux_height(800, 0), 12);
        assert_eq!(aux_height(800, -3), 12);
        assert_eq!(aux_height(1280, 2), 18);
        assert_eq!(gui_height(0), 20);
        assert_eq!(gui_height(-3), 14);
        assert_eq!(weight(Some(1)), 400);
        assert_eq!(weight(Some(0)), 700);
        assert_eq!(weight(None), 700);
        assert_eq!(face(None), "Arial");
        assert_eq!(face(Some("Tahoma")), "Tahoma");
    }

    #[test]
    fn ur_3_4_chat_outline_by_width() {
        assert_eq!(chat_outline(1280).len(), 4);
        assert_eq!(chat_outline(1281).len(), 8);
    }

    #[test]
    fn measure_block_default() {
        let m = MonoMeasure { char_w: 7, line_h: 15 };
        assert_eq!(m.measure_block(FontId::Small, b"abc\nabcdef"), Size { w: 42, h: 30 });
    }
}
