//! Scoreboard (Tab): rows, sorting, open/close, RPC 155 refresh timing and geometry
//! (UR §15).

use crate::colour::rgba_to_argb;
use crate::fonts::{FontId, TextMeasure};
use crate::{Rect, round_i};

/// RPC 155 (score/ping request) is sent at most this often while open (A 0x10008f10).
pub const REFRESH_INTERVAL_MS: u64 = 3000;
/// Unknown (notes U-SB-FIRST-REQUEST): whether a request goes out on the first open frame
/// (assumed: yes, the timer starts expired).
pub const REQUEST_ON_OPEN: bool = true;
/// Background colour.
pub const BACKGROUND: u32 = 0x960A_0A0A;
/// Server name and player count colour.
pub const TITLE_COLOUR: u32 = 0xFFBE_BEBE;
/// Column header colour.
pub const HEADER_COLOUR: u32 = 0xFF95_B0D0;
/// List item colours (normal / selected [I]).
pub const ITEM_COLOURS: (u32, u32) = (0xC8FF_FFFF, 0xC880_8080);
/// Header texts.
pub const HEADERS: [&[u8]; 4] = [b"id", b"name", b"score", b"ping"];
/// String measured to right-align the player count.
pub const PLAYERS_SAMPLE: &[u8] = b"Players: 10000";

/// Sort mode (field +0x40). Nothing in R4 sets anything but `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortMode {
    #[default]
    None,
    ByName,
    ByScoreDescending,
}

/// A player as the scoreboard needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerInfo {
    pub id: u16,
    pub name: Vec<u8>,
    pub score: i32,
    pub ping: u32,
    /// Player colour, wire RGBA.
    pub colour_rgba: u32,
    pub is_npc: bool,
}

/// One list row: main text (id) in the player colour, cells name / score / ping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u16,
    /// ARGB, alpha forced to FF.
    pub colour: u32,
    pub cells: [Vec<u8>; 4],
}

/// The scoreboard (global A 0x1026e9c4).
#[derive(Clone, Debug, Default)]
pub struct Scoreboard {
    open: bool,
    rows: Vec<Row>,
    pub sort: SortMode,
    selected: Option<usize>,
    scroll: usize,
    last_request_ms: Option<u64>,
    built_for_count: Option<usize>,
}

impl Scoreboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    pub fn set_selected(&mut self, i: Option<usize>) {
        self.selected = i;
    }
    pub fn scroll(&self) -> usize {
        self.scroll
    }
    pub fn set_scroll(&mut self, s: usize) {
        self.scroll = s;
    }

    /// Tab: toggles. Opening needs the class selection to be inactive. Returns the new state.
    /// Open: dialog visible, list enabled, rows rebuilt (by the next [`Self::frame`]),
    /// cursor mode 3.
    pub fn toggle(&mut self, class_selection_active: bool) -> bool {
        if self.open {
            self.close();
        } else if !class_selection_active {
            self.open = true;
            self.built_for_count = None;
            if REQUEST_ON_OPEN {
                self.last_request_ms = None;
            }
        }
        self.open
    }

    /// Close: ESC, Tab, click outside, focus loss.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Rebuilds the rows: local player first, then connected remote players except NPCs,
    /// in id order (A 0x1006ed30). Selection and scroll position are kept.
    pub fn rebuild<'a>(&mut self, local: &PlayerInfo, remotes: impl IntoIterator<Item = &'a PlayerInfo>) {
        let mut remotes: Vec<&PlayerInfo> = remotes.into_iter().filter(|p| !p.is_npc).collect();
        remotes.sort_by_key(|p| p.id);
        let make = |p: &PlayerInfo| Row {
            id: p.id,
            colour: rgba_to_argb(p.colour_rgba),
            cells: [
                p.id.to_string().into_bytes(),
                p.name.clone(),
                p.score.to_string().into_bytes(),
                p.ping.to_string().into_bytes(),
            ],
        };
        let mut rows: Vec<Row> = std::iter::once(local).chain(remotes).map(make).collect();
        // Unknown (notes U-SB-SORT): how sorting treats the local row; sorting is stable
        // over all rows.
        match self.sort {
            SortMode::None => {}
            SortMode::ByName => rows.sort_by(|a, b| a.cells[1].cmp(&b.cells[1])),
            SortMode::ByScoreDescending => rows.sort_by_key(|r| std::cmp::Reverse(crate::text::atoi(&r.cells[2]))),
        }
        self.rows = rows;
        if let Some(s) = self.selected
            && s >= self.rows.len()
        {
            self.selected = self.rows.len().checked_sub(1);
        }
    }

    /// Per-frame step while open. `player_count` = remote pool count. Returns
    /// `(rebuild_rows, send_rpc_155)`: rows are rebuilt on open and whenever the count
    /// changes; RPC 155 is requested at most every 3000 ms.
    pub fn frame(&mut self, now_ms: u64, player_count: usize) -> (bool, bool) {
        if !self.open {
            return (false, false);
        }
        let rebuild = self.built_for_count != Some(player_count);
        self.built_for_count = Some(player_count);
        let send = match self.last_request_ms {
            None => true,
            Some(t) => now_ms.saturating_sub(t) >= REFRESH_INTERVAL_MS,
        };
        if send {
            self.last_request_ms = Some(now_ms);
        }
        (rebuild, send)
    }
}

/// Scoreboard geometry for video mode `w × h` (UR §15.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub rect: Rect,
    pub list: Rect,
    /// id, name, score, ping.
    pub column_widths: [i32; 4],
    /// Server name text rect.
    pub server_name: Rect,
    /// Left edge of `Players: %d`.
    pub players_x: i32,
    /// Header text origins (x, top).
    pub headers: [(i32, i32); 4],
}

/// Size: 640 × 480, or 800 × 600 when the video width is above 800.
pub fn size(w: i32) -> (i32, i32) {
    if w > 800 { (800, 600) } else { (640, 480) }
}

/// Text of the player-count label: pool count + 1 (the local player).
pub fn players_text(remote_count: usize) -> Vec<u8> {
    format!("Players: {}", remote_count + 1).into_bytes()
}

pub fn layout(m: &dyn TextMeasure, w: i32, h: i32) -> Layout {
    let (sw, sh) = size(w);
    let x = w / 2 - sw / 2;
    let y = h / 2 - sh / 2;
    let id_w = round_i(0.085 * f64::from(sw));
    let name_w = round_i(0.4375 * f64::from(sw));
    let score_w = round_i(0.265625 * f64::from(sw));
    let column_widths = [id_w, name_w, score_w, sw - id_w - name_w - score_w];
    let small_h = m.line_height(FontId::Small);
    let header_top = y + 60 - small_h - 3;
    let mut headers = [(0, 0); 4];
    let mut hx = x + 10;
    for (i, h) in headers.iter_mut().enumerate() {
        *h = (hx, header_top);
        hx += column_widths[i];
    }
    Layout {
        rect: Rect::xywh(x, y, sw, sh),
        list: Rect::xywh(x, y + 60, sw, sh - 60),
        column_widths,
        server_name: Rect::new(x + 5, y + 5, x + sw, y + 60),
        players_x: x + sw - m.measure(FontId::Small, PLAYERS_SAMPLE).w,
        headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::MonoMeasure;

    fn p(id: u16, name: &str, score: i32, npc: bool) -> PlayerInfo {
        PlayerInfo { id, name: name.into(), score, ping: 50, colour_rgba: 0x11223344, is_npc: npc }
    }

    #[test]
    fn ur_15_2_rows_local_first_then_ids_no_npcs() {
        let mut s = Scoreboard::new();
        let remotes = [p(9, "c", 1, false), p(2, "a", 5, false), p(4, "bot", 0, true)];
        s.rebuild(&p(5, "me", 3, false), &remotes);
        let ids: Vec<u16> = s.rows().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![5, 2, 9]);
        assert_eq!(s.rows()[0].colour, 0xFF112233);
        assert_eq!(s.rows()[1].cells, [b"2".to_vec(), b"a".to_vec(), b"5".to_vec(), b"50".to_vec()]);
        s.sort = SortMode::ByScoreDescending;
        s.rebuild(&p(5, "me", 3, false), &remotes);
        let ids: Vec<u16> = s.rows().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![2, 5, 9]);
    }

    #[test]
    fn ur_15_3_toggle_and_refresh_every_3000ms() {
        let mut s = Scoreboard::new();
        assert!(!s.toggle(true));
        assert!(s.toggle(false));
        assert_eq!(s.frame(10_000, 3), (true, true));
        assert_eq!(s.frame(11_000, 3), (false, false));
        assert_eq!(s.frame(12_999, 4), (true, false));
        assert_eq!(s.frame(13_000, 4), (false, true));
        assert!(!s.toggle(false));
        assert_eq!(s.frame(20_000, 4), (false, false));
    }

    #[test]
    fn ur_15_1_geometry() {
        assert_eq!(size(800), (640, 480));
        assert_eq!(size(801), (800, 600));
        let m = MonoMeasure { char_w: 7, line_h: 14 };
        let l = layout(&m, 1280, 720);
        assert_eq!(l.rect, Rect::xywh(240, 60, 800, 600));
        assert_eq!(l.list, Rect::xywh(240, 120, 800, 540));
        assert_eq!(l.column_widths, [68, 350, 213, 169]);
        assert_eq!(l.headers[0], (250, 60 + 60 - 14 - 3));
        assert_eq!(l.headers[1].0, 250 + 68);
        assert_eq!(l.players_x, 240 + 800 - 14 * 7);
        assert_eq!(players_text(3), b"Players: 4");
        let l = layout(&m, 800, 600);
        assert_eq!(l.column_widths, [54, 280, 170, 136]);
    }
}
