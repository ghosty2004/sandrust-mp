//! Textdraws: pool, text rules, drawing parameters for GTA's `CFont`, hit rectangles
//! and the selection mode (UR §14; UCA §134, §135, §105, §83).
//!
//! Static analysis of samp.dll (A build) settled what RE-5 left open:
//! - set-text (A 0x100b2fa0): texts of 800 or more characters leave the textdraw
//!   empty; a `~` code longer than 24 characters or an unmatched `~` replaces the text
//!   with "Error: unmatched tilde"; a `~k~` text longer than 255 characters becomes
//!   "Error: can't use k-codes in long string".
//! - hit rectangles (A 0x100b33f5.., 0x100b3179..) per alignment, see [`hit_rect`]
//!   (the well-known swap: centred textdraws use `textSizeY` as width and `textSizeX`
//!   as height).

use samp_proto::rpcs::ui::TextDrawData;

use crate::Rect;

/// Pool size: 0..2047 global, 2048..2303 per-player.
pub const POOL_SIZE: usize = 0x900;
/// Text buffer size: texts of this length or longer are not stored.
pub const TEXT_MAX: usize = 800;
/// A `~` code may hold at most this many characters.
pub const TILDE_CODE_MAX: usize = 24;
/// `~k~` texts longer than this are refused.
pub const KEY_TEXT_MAX: usize = 255;
pub const ERR_UNMATCHED_TILDE: &[u8] = b"Error: unmatched tilde";
pub const ERR_LONG_K_CODES: &[u8] = b"Error: can't use k-codes in long string";
/// Hovered id when nothing is hovered / the cancel id sent by RPC 83.
pub const NONE: u16 = 0xFFFF;

/// Flags byte of the 63-byte block (UCA §134).
pub mod flags {
    pub const BOX: u8 = 0x01;
    pub const LEFT: u8 = 0x02;
    pub const RIGHT: u8 = 0x04;
    pub const CENTRE: u8 = 0x08;
    pub const PROPORTIONAL: u8 = 0x10;
}

/// Font/style 4: sprite (`txd:name`); 5: model preview.
pub const FONT_SPRITE: u8 = 4;
pub const FONT_PREVIEW: u8 = 5;

/// Result of the set-text rules (A 0x100b2fa0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRules {
    pub text: Vec<u8>,
    /// The text has a `~k~`/`~K~` code (InsertPlayerControlKeysInString is called).
    pub has_keys: bool,
}

/// Applies samp's set-text rules.
pub fn set_text(input: &[u8]) -> TextRules {
    let input = crate::text::c_str(input);
    if input.len() >= TEXT_MAX {
        return TextRules { text: Vec::new(), has_keys: false };
    }
    let text = input.to_vec();
    let mut has_keys = false;
    let mut in_code = false;
    let mut count = 0usize;
    let mut ok = true;
    for (i, &c) in text.iter().enumerate() {
        if c == b'~' {
            if in_code {
                count = 0;
                in_code = false;
                continue;
            }
            in_code = true;
            if matches!(text.get(i + 1), Some(b'k' | b'K')) {
                has_keys = true;
            }
        } else if !in_code {
            continue;
        }
        count += 1;
        if count > TILDE_CODE_MAX {
            ok = false;
        }
    }
    if !ok || in_code {
        return TextRules { text: ERR_UNMATCHED_TILDE.to_vec(), has_keys: false };
    }
    if has_keys && text.len() > KEY_TEXT_MAX {
        return TextRules { text: ERR_LONG_K_CODES.to_vec(), has_keys: false };
    }
    TextRules { text, has_keys }
}

/// GTA `CFont` parameters of one text textdraw for video mode `w × h`, with the
/// unit constants `ux` (0x859520, 1/640) and `uy` (0x859524, 1/448) read at draw time
/// (UR §14.2, A 0x100b31e0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontParams {
    pub scale: (f32, f32),
    /// `CRGBA` as stored (R, G, B, A in memory order).
    pub colour: u32,
    /// 0 = centre, 1 = left, 2 = right (`CFont::SetOrientation`).
    pub orientation: u8,
    pub wrap_x: f32,
    pub centre_size: f32,
    pub background: bool,
    pub background_colour: u32,
    pub proportional: bool,
    pub drop_colour: u32,
    /// `SetEdge(outline)` when non-zero, else `SetDropShadowPosition(shadow)`.
    pub outline: u8,
    pub shadow: u8,
    pub font: u8,
    /// `PrintString` position.
    pub x: f32,
    pub y: f32,
}

/// Computes the draw parameters (same float operation order as the original).
pub fn font_params(d: &TextDrawData, hovered: Option<u32>, w: i32, h: i32, ux: f32, uy: f32) -> FontParams {
    let fw = w as f32;
    let fh = h as f32;
    let orientation = if d.flags & flags::RIGHT != 0 {
        2
    } else if d.flags & flags::CENTRE != 0 {
        0
    } else {
        1
    };
    FontParams {
        scale: (fw * d.letter_size.x * ux, fh * d.letter_size.y * uy * 0.5),
        colour: hovered.unwrap_or(d.letter_colour),
        orientation,
        wrap_x: fw * d.text_size.x * ux,
        centre_size: fw * d.text_size.y * ux,
        background: d.flags & flags::BOX != 0,
        background_colour: d.box_colour,
        proportional: d.flags & flags::PROPORTIONAL != 0,
        drop_colour: d.background_colour,
        outline: d.outline,
        shadow: d.shadow,
        font: d.style,
        x: fw - fw * ux * (640.0 - d.position.x),
        y: fh - fh * uy * (448.0 - d.position.y),
    }
}

/// MSVC `_ftol2`: truncation toward zero.
fn ftol(v: f32) -> i32 {
    v as i32
}

/// Hit rectangle stored after drawing a text textdraw (A 0x100b33f5..0x100b34aa).
pub fn hit_rect(p: &FontParams) -> Rect {
    let (x, y) = (p.x, p.y);
    match p.orientation {
        // Right: from 2x − wrap to x; height = centre size.
        2 => Rect::new(ftol(x - (p.wrap_x - x)), ftol(y), ftol(x), ftol(y + p.centre_size)),
        // Centre: width = centre size around x; height = wrap.
        0 => {
            let left = ftol(x - p.centre_size * 0.5);
            Rect::new(left, ftol(y), ftol(p.centre_size) + left, ftol(y + p.wrap_x))
        }
        // Left: x to the wrap edge; height = centre size.
        _ => Rect::new(ftol(x), ftol(y), ftol((p.wrap_x - x) + x), ftol(y + p.centre_size)),
    }
}

/// Sprite/model rectangle (A 0x100b30d0): `(x·W·ux, y·H·uy) – ((x + tsx)·W·ux, (y + tsy)·H·uy)`.
pub fn sprite_rect(d: &TextDrawData, w: i32, h: i32, ux: f32, uy: f32) -> [f32; 4] {
    let sx = w as f32 * ux;
    let sy = h as f32 * uy;
    let (x, y) = (d.position.x, d.position.y);
    [x * sx, y * sy, (x + d.text_size.x) * sx, (y + d.text_size.y) * sy]
}

/// Hit rectangle of a sprite/model textdraw (A 0x100b3179..0x100b31ce).
pub fn sprite_hit_rect(d: &TextDrawData, w: i32, h: i32, ux: f32, uy: f32) -> Rect {
    let sx = w as f32 * ux;
    let sy = h as f32 * uy;
    let left = ftol(d.position.x * sx);
    let top = ftol(d.position.y * sy);
    Rect::new(left, top, ftol(sx * d.text_size.x + left as f32), ftol(sy * d.text_size.y + top as f32))
}

/// One textdraw.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDraw {
    pub data: TextDrawData,
    pub text: Vec<u8>,
    pub has_keys: bool,
    /// Drawn in the hover colour (selection mode).
    pub hovered: bool,
    /// Rectangle of the last draw (selection hit test).
    pub hit: Option<Rect>,
    /// Creation number: every RPC 134 makes a new textdraw object in samp (A 0x100b3720), even with the same id
    /// and data; per-object resources (model-preview textures) follow it.
    pub serial: u64,
}

impl TextDraw {
    pub fn new(data: TextDrawData, text: &[u8]) -> Self {
        let r = set_text(text);
        Self { data, text: r.text, has_keys: r.has_keys, hovered: false, hit: None, serial: 0 }
    }

    pub fn set_text(&mut self, text: &[u8]) {
        let r = set_text(text);
        self.text = r.text;
        self.has_keys = r.has_keys;
    }

    pub fn is_sprite(&self) -> bool {
        self.data.style == FONT_SPRITE
    }

    /// `txd:name` of a sprite textdraw.
    pub fn sprite_name(&self) -> Option<(&[u8], &[u8])> {
        let i = self.text.iter().position(|&b| b == b':')?;
        Some((&self.text[..i], &self.text[i + 1..]))
    }
}

/// Selection mode state (global A 0x1026e9e0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub active: bool,
    /// Hover colour, `CRGBA` memory order.
    pub hover_colour: u32,
    pub hovered: Option<u16>,
}

/// The textdraw pool.
#[derive(Clone, Debug)]
pub struct TextDraws {
    slots: Vec<Option<TextDraw>>,
    pub selection: Selection,
    /// Serial of the next created textdraw.
    next_serial: u64,
}

impl Default for TextDraws {
    fn default() -> Self {
        Self { slots: vec![None; POOL_SIZE], selection: Selection::default(), next_serial: 1 }
    }
}

impl TextDraws {
    pub fn new() -> Self {
        Self::default()
    }

    /// RPC 134: replaces any existing textdraw with that id. Ids ≥ 2304 are ignored
    /// (hardened; the original has no bound check). Texts with length ≥ 800 are refused
    /// by the RPC handler (debug "Warning: ignoring large TextDraw size=%u").
    pub fn show(&mut self, id: u16, data: TextDrawData, text: &[u8]) -> bool {
        let Some(slot) = self.slots.get_mut(usize::from(id)) else { return false };
        let mut t = TextDraw::new(data, text);
        t.serial = self.next_serial;
        self.next_serial += 1;
        *slot = Some(t);
        true
    }

    /// RPC 135 (a hide is a delete).
    pub fn hide(&mut self, id: u16) {
        if let Some(s) = self.slots.get_mut(usize::from(id)) {
            *s = None;
        }
    }

    /// RPC 105: id < 0x900 and existing; texts ≥ 800 are invalid.
    pub fn set_string(&mut self, id: u16, text: &[u8]) -> bool {
        match self.slots.get_mut(usize::from(id)).and_then(|s| s.as_mut()) {
            Some(t) if text.len() < TEXT_MAX => {
                t.set_text(text);
                true
            }
            _ => false,
        }
    }

    pub fn get(&self, id: u16) -> Option<&TextDraw> {
        self.slots.get(usize::from(id)).and_then(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, id: u16) -> Option<&mut TextDraw> {
        self.slots.get_mut(usize::from(id)).and_then(|s| s.as_mut())
    }

    /// Present textdraws in id order (the draw order).
    pub fn iter(&self) -> impl Iterator<Item = (u16, &TextDraw)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|t| (i as u16, t)))
    }

    pub fn ids(&self) -> Vec<u16> {
        self.iter().map(|(i, _)| i).collect()
    }

    pub fn count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = None);
        self.selection = Selection::default();
    }

    /// RPC 83 enable: mode on, hover colour stored, nothing hovered.
    pub fn begin_select(&mut self, hover_colour: u32) {
        self.selection = Selection { active: true, hover_colour, hovered: None };
    }

    /// The disable path (RPC 83 enable = 0, ESC, RPC 27/117): mode off, hover flags
    /// cleared. The caller sends RPC 83 with 0xFFFF and closes the chat input.
    pub fn cancel_select(&mut self) {
        self.selection.active = false;
        self.selection.hovered = None;
        for t in self.slots.iter_mut().flatten() {
            t.hovered = false;
        }
    }

    /// Per-frame hit test while selection is on (A 0x10071470): the highest selectable,
    /// drawn textdraw under the cursor is hovered.
    pub fn hover(&mut self, cx: i32, cy: i32) -> Option<u16> {
        if !self.selection.active {
            return None;
        }
        let mut hovered = None;
        for (i, s) in self.slots.iter_mut().enumerate() {
            let Some(t) = s else { continue };
            let hit = t.data.selectable != 0 && t.hit.is_some_and(|r| r.contains(cx, cy));
            t.hovered = false;
            if hit {
                hovered = Some(i as u16);
            }
        }
        if let Some(i) = hovered
            && let Some(t) = self.get_mut(i)
        {
            t.hovered = true;
        }
        self.selection.hovered = hovered;
        hovered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samp_proto::types::{Vec2, Vec3};

    fn data(flags: u8, pos: (f32, f32), ts: (f32, f32)) -> TextDrawData {
        TextDrawData {
            flags,
            letter_size: Vec2 { x: 0.5, y: 1.0 },
            letter_colour: 0xFFFF_FFFF,
            text_size: Vec2 { x: ts.0, y: ts.1 },
            box_colour: 0x8000_0000,
            shadow: 1,
            outline: 0,
            background_colour: 0xFF00_0000,
            style: 1,
            selectable: 1,
            position: Vec2 { x: pos.0, y: pos.1 },
            preview_model: 0,
            preview_rot: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            preview_zoom: 1.0,
            preview_colour1: 0,
            preview_colour2: 0,
        }
    }

    #[test]
    fn set_text_rules() {
        assert_eq!(set_text(b"~r~hi~w~ok"), TextRules { text: b"~r~hi~w~ok".to_vec(), has_keys: false });
        assert!(set_text(b"press ~k~~PED_FIREWEAPON~").has_keys);
        assert_eq!(set_text(b"bad ~r").text, ERR_UNMATCHED_TILDE);
        let long = [b"~".as_slice(), &[b'a'; 30], b"~"].concat();
        assert_eq!(set_text(&long).text, ERR_UNMATCHED_TILDE);
        let k = [b"~k~~PED_FIREWEAPON~".as_slice(), &[b'x'; 250]].concat();
        assert_eq!(set_text(&k).text, ERR_LONG_K_CODES);
        assert_eq!(set_text(&[b'x'; 800]).text, b"");
        assert_eq!(set_text(&[b'x'; 799]).text.len(), 799);
    }

    #[test]
    fn params_and_hit_rects() {
        let (ux, uy) = (1.0 / 640.0, 1.0 / 448.0);
        // Left aligned at (320, 224) on 1280×720, text size (400, 20).
        let d = data(flags::LEFT, (320.0, 224.0), (400.0, 20.0));
        let p = font_params(&d, None, 1280, 720, ux, uy);
        assert_eq!(p.orientation, 1);
        assert!((p.x - 640.0).abs() < 0.01 && (p.y - 360.0).abs() < 0.01);
        assert_eq!(p.scale, (1280.0 * 0.5 * ux, 720.0 * uy * 0.5));
        let r = hit_rect(&p);
        assert_eq!((r.left, r.top, r.right), (640, 360, 800));
        assert_eq!(r.bottom, (p.y + p.centre_size) as i32);
        // Centred: width from textSizeY.
        let d = data(flags::CENTRE, (320.0, 224.0), (20.0, 100.0));
        let p = font_params(&d, Some(0x1122_3344), 1280, 720, ux, uy);
        assert_eq!((p.orientation, p.colour), (0, 0x1122_3344));
        let r = hit_rect(&p);
        assert_eq!((r.left, r.right), (540, 740));
        // Right aligned: from 2x − wrap.
        let d = data(flags::RIGHT, (320.0, 224.0), (200.0, 20.0));
        let r = hit_rect(&font_params(&d, None, 1280, 720, ux, uy));
        assert_eq!((r.left, r.right), (880, 640));
    }

    #[test]
    fn pool_and_selection() {
        let mut t = TextDraws::new();
        assert!(!t.show(2304, data(0, (0.0, 0.0), (0.0, 0.0)), b"x"));
        assert!(t.show(5, data(flags::LEFT, (0.0, 0.0), (100.0, 10.0)), b"a"));
        assert!(t.show(7, data(flags::LEFT, (0.0, 0.0), (100.0, 10.0)), b"b"));
        assert_eq!(t.ids(), vec![5, 7]);
        assert!(t.set_string(5, b"new"));
        assert!(!t.set_string(6, b"new"));
        assert_eq!(t.get(5).unwrap().text, b"new");
        t.get_mut(5).unwrap().hit = Some(Rect::new(0, 0, 100, 100));
        t.get_mut(7).unwrap().hit = Some(Rect::new(50, 50, 150, 150));
        assert_eq!(t.hover(60, 60), None);
        t.begin_select(0xFF00_00FF);
        assert_eq!(t.hover(60, 60), Some(7));
        assert!(t.get(7).unwrap().hovered && !t.get(5).unwrap().hovered);
        assert_eq!(t.hover(10, 10), Some(5));
        t.cancel_select();
        assert!(!t.selection.active && !t.get(5).unwrap().hovered);
        t.hide(5);
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn every_show_is_a_new_object() {
        let mut t = TextDraws::new();
        let d = data(flags::LEFT, (0.0, 0.0), (100.0, 10.0));
        t.show(3, d, b"_");
        let first = t.get(3).unwrap().serial;
        // Same id, same data: still a new textdraw (samp deletes and re-creates it).
        t.show(3, d, b"_");
        let second = t.get(3).unwrap().serial;
        assert!(first != 0 && second > first);
        t.set_string(3, b"x");
        assert_eq!(t.get(3).unwrap().serial, second);
        t.clear();
        t.show(3, d, b"_");
        assert!(t.get(3).unwrap().serial > second);
    }
}
