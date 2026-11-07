//! Death window: the last 5 kill / connect rows, the reason → GTAWEAPON3 glyph table and the
//! per-row layout ([`DeathWindow::layout`])
//! (docs/re/subsystems/ui-rendering.md, "Death window").

use std::collections::VecDeque;

use crate::fonts::{FontId, TextMeasure};
use crate::{Rect, round_i};

/// Rows kept.
pub const ROWS: usize = 5;
/// Name buffers are 25 bytes: at most 24 characters.
pub const MAX_NAME_LEN: usize = 24;
/// Sample string whose width sets the name column.
pub const NICK_SAMPLE: &[u8] = b"LONGESTNICKNICK_NICKNICK";
/// Glyph drawn as the black icon backdrop.
pub const BACKDROP_GLYPH: u8 = b'G';
/// Icon box width.
pub const ICON_BOX_W: i32 = 35;
/// Reason glyph colour (white) and backdrop colour (black).
pub const ICON_COLOUR: u32 = 0xFFFF_FFFF;
pub const BACKDROP_COLOUR: u32 = 0xFF00_0000;
/// Connect / disconnect reasons and their glyph colours.
pub const REASON_CONNECT: u8 = 200;
pub const REASON_DISCONNECT: u8 = 201;
pub const CONNECT_COLOUR: u32 = 0xFF11_11AA;
pub const DISCONNECT_COLOUR: u32 = 0xFFAA_1111;
/// Glyph for reasons not in the table.
pub const DEFAULT_GLYPH: u8 = b'J';

/// Reason → glyph. Mirrors samp.dll 0x10069e50.
pub fn glyph(reason: u8) -> u8 {
    match reason {
        0 => b'%',
        1 => b'B',
        2 => b'>',
        3 => b'(',
        4 => b'C',
        5 => b'?',
        6 => b'&',
        7 => b'"',
        8 => b'!',
        9 => b'1',
        10..=13 => b'E',
        14 => b'$',
        15 => b'#',
        16 => b'@',
        17 => b'D',
        22 => b'6',
        23 => b'2',
        24 => b'3',
        25 => b'=',
        26 => b'0',
        27 => b'+',
        28 => b'I',
        30 => b'H',
        31 => b'5',
        32 => b'7',
        33 => b'.',
        34 => b'A',
        35 => b'4',
        36 => b')',
        37 => b'P',
        38 => b'F',
        39 => b'<',
        40 => b';',
        41 => b'/',
        42 => b',',
        46 => b':',
        49 => b'L',
        50 => b'R',
        51 => b'Q',
        54 => b'K',
        200 | 201 => b'N',
        _ => DEFAULT_GLYPH,
    }
}

/// Colour of the reason glyph.
pub fn glyph_colour(reason: u8) -> u32 {
    match reason {
        REASON_CONNECT => CONNECT_COLOUR,
        REASON_DISCONNECT => DISCONNECT_COLOUR,
        _ => ICON_COLOUR,
    }
}

/// One row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeathRow {
    pub killer: Vec<u8>,
    pub victim: Vec<u8>,
    pub killer_colour: u32,
    pub victim_colour: u32,
    pub reason: u8,
}

/// Text placement; `align_right` = the text ends at `x` (`DT_RIGHT`), else starts at `x`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePlacement {
    pub x: i32,
    pub y: i32,
    pub align_right: bool,
    pub text: Vec<u8>,
    pub colour: u32,
}

/// What to draw for one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowLayout {
    pub left_name: NamePlacement,
    /// Icon box: backdrop glyph `G` in black, then the reason glyph centred.
    pub icon_box: Rect,
    pub glyph: u8,
    pub glyph_colour: u32,
    pub right_name: Option<NamePlacement>,
}

/// The death window. Mirrors the samp.dll global at 0x1026ea00.
#[derive(Clone, Debug)]
pub struct DeathWindow {
    rows: VecDeque<DeathRow>,
    visible: bool,
}

impl Default for DeathWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl DeathWindow {
    pub fn new() -> Self {
        Self { rows: VecDeque::with_capacity(ROWS), visible: true }
    }

    /// Visible flag (default on, F9 toggles, not persisted).
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn toggle_visible(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    /// Rows, oldest (top) first.
    pub fn rows(&self) -> impl Iterator<Item = &DeathRow> {
        self.rows.iter()
    }

    /// Adds a row at the bottom; the oldest is pushed out. No timeout. Mirrors samp.dll 0x1006a5d0.
    pub fn add(&mut self, killer: &[u8], killer_colour: u32, victim: &[u8], victim_colour: u32, reason: u8) {
        if self.rows.len() == ROWS {
            self.rows.pop_front();
        }
        let cut = |s: &[u8]| s[..s.len().min(MAX_NAME_LEN)].to_vec();
        self.rows.push_back(DeathRow {
            killer: cut(killer),
            victim: cut(victim),
            killer_colour,
            victim_colour,
            reason,
        });
    }

    /// Lays out the rows for video mode `w × h`. Rows are placed in slot order from the top; with
    /// fewer than 5 rows the stored rows fill from the top (assumed).
    pub fn layout(&self, m: &dyn TextMeasure, w: i32, h: i32) -> Vec<RowLayout> {
        let nick_w = m.measure(FontId::DeathNames, NICK_SAMPLE).w;
        let icon = m.measure(FontId::DeathIcon, &[DEFAULT_GLYPH]);
        let (x, y0) = origin(w, h, nick_w, icon.w);
        self.rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let y = y0 + i as i32 * (icon.h + 5);
                let icon_left = x + nick_w + 3;
                let icon_box = Rect::new(icon_left, y - 5, icon_left + ICON_BOX_W, y - 5 + icon.h);
                let glyph = glyph(r.reason);
                let glyph_colour = glyph_colour(r.reason);
                if r.killer.is_empty() {
                    RowLayout {
                        left_name: NamePlacement {
                            x: x + nick_w,
                            y,
                            align_right: true,
                            text: r.victim.clone(),
                            colour: r.victim_colour,
                        },
                        icon_box,
                        glyph,
                        glyph_colour,
                        right_name: None,
                    }
                } else {
                    RowLayout {
                        left_name: NamePlacement {
                            x: x + nick_w,
                            y,
                            align_right: true,
                            text: r.killer.clone(),
                            colour: r.killer_colour,
                        },
                        icon_box,
                        glyph,
                        glyph_colour,
                        right_name: Some(NamePlacement {
                            x: icon_left + icon.w,
                            y,
                            align_right: false,
                            text: r.victim.clone(),
                            colour: r.victim_colour,
                        }),
                    }
                }
            })
            .collect()
    }
}

/// Origin `(round(0.75·W), round(0.3·H))`, pulled left so `2·nickW + iconW` fits.
pub fn origin(w: i32, h: i32, nick_w: i32, icon_w: i32) -> (i32, i32) {
    let mut x = round_i(0.75 * f64::from(w));
    let y = round_i(0.3 * f64::from(h));
    if x + nick_w * 2 + icon_w > w {
        x = w - (nick_w * 2 + icon_w);
    }
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::MonoMeasure;

    #[test]
    fn ur_17_2_glyph_table() {
        let table: &[(u8, u8)] = &[
            (0, b'%'),
            (1, b'B'),
            (2, b'>'),
            (3, b'('),
            (4, b'C'),
            (5, b'?'),
            (6, b'&'),
            (7, b'"'),
            (8, b'!'),
            (9, b'1'),
            (10, b'E'),
            (13, b'E'),
            (14, b'$'),
            (15, b'#'),
            (16, b'@'),
            (17, b'D'),
            (22, b'6'),
            (23, b'2'),
            (24, b'3'),
            (25, b'='),
            (26, b'0'),
            (27, b'+'),
            (28, b'I'),
            (30, b'H'),
            (31, b'5'),
            (32, b'7'),
            (33, b'.'),
            (34, b'A'),
            (35, b'4'),
            (36, b')'),
            (37, b'P'),
            (38, b'F'),
            (39, b'<'),
            (40, b';'),
            (41, b'/'),
            (42, b','),
            (46, b':'),
            (49, b'L'),
            (50, b'R'),
            (51, b'Q'),
            (54, b'K'),
            (200, b'N'),
            (201, b'N'),
        ];
        for &(r, g) in table {
            assert_eq!(glyph(r), g, "reason {r}");
        }
        for r in [18, 19, 20, 21, 29, 43, 44, 45, 47, 48, 52, 53, 55, 199, 202, 255] {
            assert_eq!(glyph(r), b'J', "reason {r}");
        }
    }

    #[test]
    fn ur_17_1_glyph_colours() {
        assert_eq!(glyph_colour(200), 0xFF1111AA);
        assert_eq!(glyph_colour(201), 0xFFAA1111);
        assert_eq!(glyph_colour(24), 0xFFFFFFFF);
    }

    #[test]
    fn ur_17_1_five_rows_newest_at_bottom() {
        let mut d = DeathWindow::new();
        for i in 0..7u8 {
            d.add(b"k", 0, &[b'a' + i], 0, i);
        }
        let v: Vec<u8> = d.rows().map(|r| r.victim[0]).collect();
        assert_eq!(v, b"cdefg");
        d.add(&[b'x'; 30], 0, b"v", 0, 0);
        assert_eq!(d.rows().last().unwrap().killer.len(), 24);
        assert!(d.visible());
        assert!(!d.toggle_visible());
    }

    #[test]
    fn ur_17_1_origin_and_row_layout() {
        // 24-char sample at 8 px = 192; icon 20 × 18.
        assert_eq!(origin(1920, 1080, 192, 20), (1440, 324));
        assert_eq!(origin(1280, 720, 192, 20), (1280 - 404, 216));
        assert_eq!(origin(800, 600, 192, 20), (800 - 404, 180));
        let m = MonoMeasure { char_w: 8, line_h: 18 };
        let mut d = DeathWindow::new();
        d.add(b"Killer", 0xFF00FF00, b"Victim", 0xFFFF0000, 24);
        d.add(b"", 0, b"Joiner", 0xFF0000FF, 200);
        let l = d.layout(&m, 1920, 1080);
        let (x, y) = (1440, 324);
        assert_eq!(
            l[0].left_name,
            NamePlacement { x: x + 192, y, align_right: true, text: b"Killer".to_vec(), colour: 0xFF00FF00 }
        );
        assert_eq!(l[0].icon_box, Rect::new(x + 195, y - 5, x + 195 + 35, y - 5 + 18));
        assert_eq!((l[0].glyph, l[0].glyph_colour), (b'3', 0xFFFFFFFF));
        assert_eq!(l[0].right_name.as_ref().unwrap().x, x + 195 + 8);
        assert_eq!(l[1].left_name.text, b"Joiner");
        assert_eq!(l[1].left_name.y, y + 18 + 5);
        assert!(l[1].right_name.is_none());
        assert_eq!((l[1].glyph, l[1].glyph_colour), (b'N', 0xFF1111AA));
    }
}
