//! Chat window model: the 100-entry buffer, entry types, paging, F7 display modes, timestamps,
//! line layout and chatlog lines. Entry points: [`ChatWindow::add_client_message`] (RPC 93),
//! [`ChatWindow::add_player_chat`] (RPC 101) and [`ChatWindow::layout`]. Specs:
//! docs/re/subsystems/ui-rendering.md ("Chat window") and ui-chat-audio.md ("Shared: chat window
//! model", "93 SendClientMessage", "101 PlayerChatMessage").

use std::collections::VecDeque;
use std::ops::RangeInclusive;

use crate::Rect;
use crate::colour::{self, rgba_to_argb};
use crate::fonts::{FontId, TextMeasure, chat_outline};
use crate::text::{self, KeyNameExpander, c_str, expand_key_names, replace_control_chars, split_line};

/// Scroll-back size: 100 entries, newest last.
pub const CAPACITY: usize = 100;
/// Longest accepted prefix (player name); a longer one drops the whole entry.
pub const MAX_PREFIX_LEN: usize = 24;
/// Entry text is truncated to this many characters (`strncpy`).
pub const MAX_TEXT_LEN: usize = 144;
/// Player chat wraps after this many characters.
pub const PLAYER_CHAT_WRAP: usize = 72;
/// How far back from the wrap column a space is searched.
pub const PLAYER_CHAT_WRAP_BACKTRACK: usize = 12;
/// RPC 93 messages this long or longer are dropped.
pub const CLIENT_MESSAGE_DROP_LEN: usize = 256;
/// RPC 93 messages longer than this are read but not shown.
pub const CLIENT_MESSAGE_SHOW_MAX: usize = 144;

/// Default page size (chat object field +0x00).
pub const DEFAULT_PAGE_SIZE: u32 = 10;
/// Values the page-size setter ([`ChatWindow::set_page_size`]) accepts; also used at start-up.
pub const PAGE_SIZE_SETTER_RANGE: RangeInclusive<u32> = 10..=100;
/// Values `/pagesize` accepts.
pub const PAGE_SIZE_COMMAND_RANGE: RangeInclusive<u32> = 10..=20;
/// Scrollbar track range start; entry 0 is therefore never visible.
pub const SCROLL_MIN: u32 = 1;
/// Scrollbar track range end.
pub const SCROLL_END: u32 = 100;
/// Whether re-laying out the scrollbar after a page-size change scrolls to the bottom (assumed)
/// or only clamps the position; not confirmed on the original.
pub const PAGESIZE_CHANGE_SCROLLS_TO_BOTTOM: bool = true;

/// Left edge of chat lines.
pub const LINE_LEFT: i32 = 45;
/// Right edge of the line rect (unused because of `DT_NOCLIP`).
pub const LINE_RIGHT: i32 = 550;
/// Top of the first line.
pub const FIRST_LINE_TOP: i32 = 10;
/// Bottom of the first line's rect, and the chat bottom when the page size is 0.
pub const FIRST_LINE_BOTTOM: i32 = 110;
/// Gap between a player name and the chat text.
pub const NAME_GAP: i32 = 5;
/// With timestamps on, content starts at `timestamp width + 50`.
pub const TIMESTAMP_CONTENT_OFFSET: i32 = 50;
/// Sample string whose width sets the timestamp column.
pub const TIMESTAMP_SAMPLE: &[u8] = b"[19:58:34]";
/// Line terminator of chatlog lines (format strings `"%s <%s> %s\r\n"` / `"%s %s\r\n"`).
/// The original appends in text mode, so its file holds CR CR LF; the host reproduces that by
/// converting `\n` to `\r\n` when writing.
pub const CHATLOG_LINE_END: &[u8] = b"\r\n";
/// Chatlog file name in the user files directory.
pub const CHATLOG_FILE: &str = "chatlog.txt";
/// Scrollbar geometry: x, y, width; height is `(h + 1)·pageSize − 60`.
pub const SCROLLBAR_X: i32 = 10;
pub const SCROLLBAR_Y: i32 = 40;
pub const SCROLLBAR_W: i32 = 20;

/// Entry type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EntryType {
    /// Player chat: name prefix + text.
    PlayerChat = 2,
    /// Info / client message (RPC 93 and local feedback).
    Info = 4,
    /// Debug message.
    Debug = 8,
}

/// Wall-clock time of an entry: C `time()` plus its `localtime` fields, both
/// supplied by the host so this crate stays clock-free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub unix: i64,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    /// `strftime("[%H:%M:%S]")`.
    pub fn timestamp(&self) -> Vec<u8> {
        format!("[{:02}:{:02}:{:02}]", self.hour, self.minute, self.second).into_bytes()
    }
}

/// One stored chat line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatEntry {
    pub time: LocalTime,
    pub kind: EntryType,
    /// Player name without the `:` (empty for no prefix).
    pub name: Vec<u8>,
    pub text: Vec<u8>,
    pub text_colour: u32,
    pub name_colour: u32,
}

impl ChatEntry {
    /// The prefix as stored and drawn: the name with `:` appended.
    pub fn prefix(&self) -> Vec<u8> {
        if self.name.is_empty() {
            Vec::new()
        } else {
            let mut p = self.name.clone();
            p.push(b':');
            p
        }
    }

    /// Chatlog line: `[HH:MM:SS] <name> text` for named entries, else `[HH:MM:SS] text`; the name is
    /// logged without the `:`, as on the original. Mirrors samp.dll 0x100677d0.
    pub fn chatlog_line(&self) -> Vec<u8> {
        let mut out = self.time.timestamp();
        out.push(b' ');
        if !self.name.is_empty() {
            out.push(b'<');
            out.extend_from_slice(&self.name);
            out.extend_from_slice(b"> ");
        }
        out.extend_from_slice(&self.text);
        out.extend_from_slice(CHATLOG_LINE_END);
        out
    }
}

/// F7 display modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DisplayMode {
    /// Chat and scrollbar hidden; HUD and radar hidden too.
    Hidden = 0,
    /// Chat shown as plain text.
    Plain = 1,
    /// Default: chat shown with outline.
    Outlined = 2,
}

impl DisplayMode {
    /// F7: 2 → 1 → 0 → 2.
    pub fn next(self) -> Self {
        match self {
            Self::Outlined => Self::Plain,
            Self::Plain => Self::Hidden,
            Self::Hidden => Self::Outlined,
        }
    }
    pub fn chat_visible(self) -> bool {
        self != Self::Hidden
    }
    pub fn outline(self) -> bool {
        self == Self::Outlined
    }
    /// HUD + radar visibility follows the chat mode.
    pub fn hud_visible(self) -> bool {
        self != Self::Hidden
    }
}

/// Result of RPC 93 handling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessageResult {
    /// Length ≥ 256: dropped.
    Dropped,
    /// 145..255 chars: read but not shown.
    NotShown,
    /// Added as a type-4 entry; carries the chatlog lines to append.
    Added(Vec<Vec<u8>>),
}

/// A piece of text to draw: position, colour-coded text and base colour.
/// Draw it through the colour-code wrapper ([`colour::glyph_colours`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextItem {
    pub x: i32,
    pub y: i32,
    pub text: Vec<u8>,
    pub colour: u32,
}

/// One chat line on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    pub rect: Rect,
    /// Timestamp, name, text: in draw order. Empty for an empty slot.
    pub items: Vec<TextItem>,
}

/// What the renderer draws for the chat window this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLayout {
    pub visible: bool,
    pub lines: Vec<ChatLine>,
    /// Black outline offsets to draw (stripped text) before each item; empty in mode 1.
    pub outline: &'static [(i32, i32)],
    /// Bottom of the last line (chat object field +0x12e); the input box hangs below it.
    pub bottom: i32,
}

/// The chat window. Mirrors the samp.dll global at 0x1026e9f8.
#[derive(Clone, Debug)]
pub struct ChatWindow {
    entries: VecDeque<Option<ChatEntry>>,
    page_size: u32,
    pos: u32,
    mode: DisplayMode,
    timestamps: bool,
    logging: bool,
    dirty: bool,
    pub info_colour: u32,
    pub debug_colour: u32,
    pub player_text_colour: u32,
}

impl Default for ChatWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatWindow {
    pub fn new() -> Self {
        Self {
            entries: std::iter::repeat_n(None, CAPACITY).collect(),
            page_size: DEFAULT_PAGE_SIZE,
            pos: SCROLL_END - DEFAULT_PAGE_SIZE,
            mode: DisplayMode::Outlined,
            timestamps: false,
            logging: false,
            dirty: true,
            info_colour: colour::CHAT_INFO,
            debug_colour: colour::CHAT_DEBUG,
            player_text_colour: colour::CHAT_PLAYER_TEXT,
        }
    }

    /// Enables chatlog lines. The original enables logging when truncating `chatlog.txt`
    /// (mode `"w"`) at construction succeeded.
    pub fn set_logging(&mut self, on: bool) {
        self.logging = on;
    }
    pub fn logging(&self) -> bool {
        self.logging
    }

    /// All 100 slots, oldest first; `None` = never filled.
    pub fn entries(&self) -> impl Iterator<Item = Option<&ChatEntry>> {
        self.entries.iter().map(Option::as_ref)
    }

    /// The newest entry.
    pub fn last(&self) -> Option<&ChatEntry> {
        self.entries.back().and_then(Option::as_ref)
    }

    /// Adds one entry. Mirrors samp.dll 0x10068020. A prefix longer than 24 chars
    /// drops the entry silently; control characters become spaces; text is
    /// truncated to 144 chars. Returns the chatlog line if logging is on.
    pub fn add_entry(
        &mut self,
        kind: EntryType,
        name: &[u8],
        text: &[u8],
        text_colour: u32,
        name_colour: u32,
        time: LocalTime,
    ) -> Option<Vec<u8>> {
        let name = c_str(name);
        if name.len() > MAX_PREFIX_LEN {
            return None;
        }
        let mut name = name.to_vec();
        replace_control_chars(&mut name);
        let mut t = text::truncate(c_str(text), MAX_TEXT_LEN).to_vec();
        replace_control_chars(&mut t);
        let entry = ChatEntry { time, kind, name, text: t, text_colour, name_colour };
        let log = self.logging.then(|| entry.chatlog_line());
        self.entries.pop_front();
        self.entries.push_back(Some(entry));
        self.dirty = true;
        log
    }

    /// RPC 93 SendClientMessage: length limits, `~k~` expansion, then a type-4 entry in the server
    /// colour with alpha forced to FF.
    pub fn add_client_message(
        &mut self,
        colour_rgba: u32,
        wire_text: &[u8],
        expander: &dyn KeyNameExpander,
        time: LocalTime,
    ) -> ClientMessageResult {
        if wire_text.len() >= CLIENT_MESSAGE_DROP_LEN {
            return ClientMessageResult::Dropped;
        }
        let t = c_str(wire_text);
        if t.len() > CLIENT_MESSAGE_SHOW_MAX {
            return ClientMessageResult::NotShown;
        }
        let t = expand_key_names(t, expander);
        let argb = rgba_to_argb(colour_rgba);
        let log = self.add_entry(EntryType::Info, b"", &t, argb, argb, time);
        ClientMessageResult::Added(log.into_iter().collect())
    }

    /// RPC 101 PlayerChatMessage: type-2 entry, name in `name_colour`
    /// (already ARGB), text in the player-text colour, wrapped at 72 chars into a
    /// second entry without prefix. The caller checks connection state and resolves
    /// the id to name/colour. No `~k~` expansion.
    pub fn add_player_chat(&mut self, name: &[u8], name_colour: u32, text: &[u8], time: LocalTime) -> Vec<Vec<u8>> {
        // Assumed: the two lines are added independently, so a name longer than 24 chars drops
        // only the first one.
        let text = c_str(text);
        let (first, rest) = split_line(text, PLAYER_CHAT_WRAP, PLAYER_CHAT_WRAP_BACKTRACK);
        let tc = self.player_text_colour;
        let mut logs = Vec::new();
        logs.extend(self.add_entry(EntryType::PlayerChat, name, first, tc, name_colour, time));
        if let Some(rest) = rest {
            logs.extend(self.add_entry(EntryType::PlayerChat, b"", rest, tc, name_colour, time));
        }
        logs
    }

    /// Local printf-style info line (type 4). Mirrors samp.dll 0x10068070.
    pub fn add_info(&mut self, text: &[u8], time: LocalTime) -> Option<Vec<u8>> {
        let c = self.info_colour;
        self.add_entry(EntryType::Info, b"", text, c, c, time)
    }

    /// Local debug line (type 8). Mirrors samp.dll 0x100680f0.
    pub fn add_debug(&mut self, text: &[u8], time: LocalTime) -> Option<Vec<u8>> {
        let c = self.debug_colour;
        self.add_entry(EntryType::Debug, b"", text, c, c, time)
    }

    // ----- display mode, timestamps --------------------------------------------------

    pub fn mode(&self) -> DisplayMode {
        self.mode
    }
    /// F7: next display mode. Not persisted. Mirrors samp.dll 0x100612c0.
    pub fn cycle_mode(&mut self) -> DisplayMode {
        self.mode = self.mode.next();
        self.dirty = true;
        self.mode
    }
    pub fn timestamps(&self) -> bool {
        self.timestamps
    }
    pub fn set_timestamps(&mut self, on: bool) {
        self.timestamps = on;
        self.dirty = true;
    }

    /// Dirty flag for the render-to-texture cache: the renderer redraws the chat texture when set.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    // ----- paging ---------------------------------------------------------------------

    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    /// Scrollbar position: first visible entry index.
    pub fn scroll_pos(&self) -> u32 {
        self.pos
    }

    fn max_pos(&self) -> u32 {
        SCROLL_END.saturating_sub(self.page_size).max(SCROLL_MIN)
    }

    /// Page-size setter: accepts 10..100, re-lays out the scrollbar and marks the chat dirty.
    /// Returns false (and changes nothing) outside the range. Mirrors samp.dll 0x100672a0.
    pub fn set_page_size(&mut self, n: u32) -> bool {
        if !PAGE_SIZE_SETTER_RANGE.contains(&n) {
            return false;
        }
        self.page_size = n;
        self.pos = if PAGESIZE_CHANGE_SCROLLS_TO_BOTTOM { self.max_pos() } else { self.pos.min(self.max_pos()) };
        self.dirty = true;
        true
    }

    /// True when the newest entry is on the last line.
    pub fn at_bottom(&self) -> bool {
        self.pos >= self.max_pos()
    }

    /// PageUp: `pos := max(1, pos − pageSize)`. Mirrors samp.dll 0x100672d0.
    pub fn page_up(&mut self) {
        self.pos = self.pos.saturating_sub(self.page_size).max(SCROLL_MIN);
    }

    /// PageDown: `pos := pageSize` if `pos == 1`, else `pos + pageSize`, capped at 100, then clamped
    /// to `100 − pageSize` by the scrollbar. Mirrors samp.dll 0x10067330.
    pub fn page_down(&mut self) {
        let p = if self.pos == SCROLL_MIN { self.page_size } else { self.pos + self.page_size };
        self.pos = p.min(SCROLL_END).min(self.max_pos());
    }

    /// Mouse wheel: scroll by `−(notches × lines_per_notch)` lines. Mirrors samp.dll 0x100673c0.
    pub fn wheel(&mut self, notches: i32, lines_per_notch: u32) {
        let delta = -i64::from(notches) * i64::from(lines_per_notch);
        let p = (i64::from(self.pos) + delta).clamp(i64::from(SCROLL_MIN), i64::from(self.max_pos()));
        self.pos = p as u32;
    }

    /// Sets the scroll position, clamped like the scrollbar (`1..=100 − pageSize`).
    /// Enter in the chat input sets 90.
    pub fn scroll_to(&mut self, pos: u32) {
        self.pos = pos.clamp(SCROLL_MIN, self.max_pos());
    }

    /// Scroll to the newest entry.
    pub fn scroll_to_bottom(&mut self) {
        self.pos = self.max_pos();
    }

    /// The `pageSize` visible slots: entries `[pos, pos + pageSize)`.
    pub fn visible(&self) -> impl Iterator<Item = Option<&ChatEntry>> {
        let start = self.pos as usize;
        let end = (start + self.page_size as usize).min(CAPACITY);
        (start..end).map(|i| self.entries[i].as_ref())
    }

    /// Scrollbar shown: mode ≠ 0 and (not at the bottom or the input is open).
    pub fn scrollbar_visible(&self, input_open: bool) -> bool {
        self.mode.chat_visible() && (!self.at_bottom() || input_open)
    }

    /// Scrollbar rect for main-font line height `h`. Mirrors samp.dll 0x10067200.
    pub fn scrollbar_rect(&self, h: i32) -> Rect {
        Rect::xywh(SCROLLBAR_X, SCROLLBAR_Y, SCROLLBAR_W, (h + 1) * self.page_size as i32 - 60)
    }

    // ----- layout -----------------------------------------------------------------------

    /// Bottom of the chat (chat object field +0x12e) for main-font line height `h`.
    ///
    /// The draw loop runs once per page line, filled or not; at the end of each iteration it
    /// advances the rect (`top += h + 1`, `bottom = top + h + 1`) and finally stores the bottom of
    /// the rect prepared for the line *after* the last one: `10 + (pageSize + 1)·(h + 1)`; 110 with
    /// page size 0. Mirrors samp.dll 0x10067940.
    pub fn bottom(&self, h: i32) -> i32 {
        match self.page_size {
            0 => FIRST_LINE_BOTTOM,
            n => FIRST_LINE_TOP + (n as i32 + 1) * (h + 1),
        }
    }

    /// Lays out the visible lines for video-mode width `video_w`. Mirrors samp.dll 0x10067470.
    pub fn layout(&self, m: &dyn TextMeasure, video_w: i32) -> ChatLayout {
        let h = m.line_height(FontId::Main);
        let tw = m.timestamp_width();
        let mut lines = Vec::with_capacity(self.page_size as usize);
        for (k, e) in self.visible().enumerate() {
            let top = FIRST_LINE_TOP + k as i32 * (h + 1);
            let bottom = if k == 0 { FIRST_LINE_BOTTOM } else { top + h + 1 };
            let mut items = Vec::new();
            if let Some(e) = e {
                let mut x = LINE_LEFT;
                if self.timestamps {
                    items.push(TextItem { x: LINE_LEFT, y: top, text: e.time.timestamp(), colour: e.text_colour });
                    x = tw + TIMESTAMP_CONTENT_OFFSET;
                }
                if e.kind == EntryType::PlayerChat && !e.name.is_empty() {
                    let prefix = e.prefix();
                    let nw = m.measure(FontId::Main, &colour::strip_codes(&prefix)).w;
                    items.push(TextItem { x, y: top, text: prefix, colour: e.name_colour });
                    x += nw + NAME_GAP;
                }
                items.push(TextItem { x, y: top, text: e.text.clone(), colour: e.text_colour });
            }
            lines.push(ChatLine { rect: Rect::new(LINE_LEFT, top, LINE_RIGHT, bottom), items });
        }
        ChatLayout {
            visible: self.mode.chat_visible(),
            lines,
            outline: if self.mode.outline() { chat_outline(video_w) } else { &[] },
            bottom: self.bottom(h),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::MonoMeasure;
    use crate::text::NoExpansion;

    const T: LocalTime = LocalTime { unix: 0, hour: 19, minute: 58, second: 4 };

    fn texts(c: &ChatWindow) -> Vec<Vec<u8>> {
        c.entries().flatten().map(|e| e.text.clone()).collect()
    }

    #[test]
    fn uca_chat_model_capacity_100_oldest_dropped() {
        let mut c = ChatWindow::new();
        for i in 0..105 {
            c.add_info(format!("m{i}").as_bytes(), T);
        }
        let t = texts(&c);
        assert_eq!(t.len(), CAPACITY);
        assert_eq!(t[0], b"m5");
        assert_eq!(t[99], b"m104");
    }

    #[test]
    fn uca_chat_model_long_prefix_drops_entry_and_text_truncates_144() {
        let mut c = ChatWindow::new();
        assert!(c.add_entry(EntryType::PlayerChat, &[b'n'; 25], b"x", 0, 0, T).is_none());
        assert_eq!(texts(&c).len(), 0);
        c.add_entry(EntryType::PlayerChat, &[b'n'; 24], &[b'y'; 200], 0, 0, T);
        let e = c.last().unwrap();
        assert_eq!(e.text.len(), 144);
        assert_eq!(e.prefix().len(), 25);
        assert_eq!(*e.prefix().last().unwrap(), b':');
    }

    #[test]
    fn uca_93_limits_and_colour() {
        let mut c = ChatWindow::new();
        assert_eq!(c.add_client_message(0x112233AA, &[b'a'; 256], &NoExpansion, T), ClientMessageResult::Dropped);
        assert_eq!(c.add_client_message(0x112233AA, &[b'a'; 145], &NoExpansion, T), ClientMessageResult::NotShown);
        assert!(matches!(
            c.add_client_message(0x112233AA, &[b'a'; 144], &NoExpansion, T),
            ClientMessageResult::Added(_)
        ));
        let e = c.last().unwrap();
        assert_eq!((e.kind, e.text_colour), (EntryType::Info, 0xFF112233));
        // Control characters become spaces.
        c.add_client_message(0xFFFFFFFF, b"a\tb\nc", &NoExpansion, T);
        assert_eq!(c.last().unwrap().text, b"a b c");
    }

    #[test]
    fn uca_101_player_chat_wraps_into_second_entry() {
        let mut c = ChatWindow::new();
        let mut text = vec![b'w'; 90];
        text[65] = b' ';
        c.add_player_chat(b"Bob", 0xFF00FF00, &text, T);
        let e: Vec<&ChatEntry> = c.entries().flatten().collect();
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].name.as_slice(), e[0].text.len()), (&b"Bob"[..], 65));
        assert_eq!((e[1].name.as_slice(), e[1].text.len()), (&b""[..], 24));
        assert_eq!(e[0].text_colour, colour::CHAT_PLAYER_TEXT);
        assert_eq!(e[0].name_colour, 0xFF00FF00);
        assert_eq!(e[1].kind, EntryType::PlayerChat);
    }

    #[test]
    fn ur_7_1_default_colours_for_local_lines() {
        let mut c = ChatWindow::new();
        c.add_info(b"i", T);
        assert_eq!((c.last().unwrap().kind, c.last().unwrap().text_colour), (EntryType::Info, 0xFF88AA62));
        c.add_debug(b"d", T);
        assert_eq!((c.last().unwrap().kind, c.last().unwrap().text_colour), (EntryType::Debug, 0xFFA9C4E4));
    }

    #[test]
    fn uca_chat_model_chatlog_line_format() {
        let mut c = ChatWindow::new();
        c.set_logging(true);
        let l = c.add_player_chat(b"Bob", 0, b"hi", T);
        assert_eq!(l, vec![b"[19:58:04] <Bob> hi\r\n".to_vec()]);
        let l = c.add_info(b"server", T).unwrap();
        assert_eq!(l, b"[19:58:04] server\r\n");
        c.set_logging(false);
        assert!(c.add_info(b"x", T).is_none());
    }

    #[test]
    fn ur_7_3_f7_cycle_and_effects() {
        let mut c = ChatWindow::new();
        assert_eq!(c.mode(), DisplayMode::Outlined);
        assert_eq!(c.cycle_mode(), DisplayMode::Plain);
        assert!(c.mode().chat_visible() && !c.mode().outline() && c.mode().hud_visible());
        assert_eq!(c.cycle_mode(), DisplayMode::Hidden);
        assert!(!c.mode().chat_visible() && !c.mode().hud_visible());
        assert!(!c.scrollbar_visible(true));
        assert_eq!(c.cycle_mode(), DisplayMode::Outlined);
    }

    #[test]
    fn ur_7_4_paging() {
        let mut c = ChatWindow::new();
        assert_eq!(c.scroll_pos(), 90);
        assert!(c.at_bottom());
        c.page_up();
        assert_eq!(c.scroll_pos(), 80);
        for _ in 0..20 {
            c.page_up();
        }
        assert_eq!(c.scroll_pos(), 1);
        // PageDown from 1 goes to pageSize, not 1 + pageSize.
        c.page_down();
        assert_eq!(c.scroll_pos(), 10);
        c.page_down();
        assert_eq!(c.scroll_pos(), 20);
        for _ in 0..20 {
            c.page_down();
        }
        assert_eq!(c.scroll_pos(), 90);
    }

    #[test]
    fn ur_7_4_wheel_and_pagesize_setter() {
        let mut c = ChatWindow::new();
        c.wheel(1, 3);
        assert_eq!(c.scroll_pos(), 87);
        c.wheel(-10, 3);
        assert_eq!(c.scroll_pos(), 90);
        c.wheel(100, 3);
        assert_eq!(c.scroll_pos(), 1);
        assert!(!c.set_page_size(9));
        assert!(!c.set_page_size(101));
        assert!(c.set_page_size(100));
        assert_eq!(c.scroll_pos(), 1);
        assert!(c.set_page_size(20));
        assert_eq!(c.scroll_pos(), 80);
        assert_eq!(c.visible().count(), 20);
    }

    #[test]
    fn ur_7_4_new_message_does_not_move_pos() {
        let mut c = ChatWindow::new();
        for i in 0..100 {
            c.add_info(format!("{i}").as_bytes(), T);
        }
        c.page_up();
        let first = c.visible().next().unwrap().unwrap().text.clone();
        assert_eq!(first, b"80");
        c.add_info(b"new", T);
        assert_eq!(c.scroll_pos(), 80);
        assert_eq!(c.visible().next().unwrap().unwrap().text, b"81");
        // Scrollbar visible when scrolled up, or at the bottom with the input open.
        assert!(c.scrollbar_visible(false));
        c.scroll_to_bottom();
        assert!(!c.scrollbar_visible(false));
        assert!(c.scrollbar_visible(true));
        assert_eq!(c.scrollbar_rect(15), Rect::xywh(10, 40, 20, 16 * 10 - 60));
    }

    #[test]
    fn ur_7_2_layout_lines_timestamps_and_names() {
        let m = MonoMeasure { char_w: 8, line_h: 15 };
        let mut c = ChatWindow::new();
        c.add_player_chat(b"Bob", 0xFF00FF00, b"hello", T);
        let l = c.layout(&m, 1280);
        assert_eq!(l.lines.len(), 10);
        assert_eq!(l.lines[0].rect, Rect::new(45, 10, 550, 110));
        assert_eq!(l.lines[1].rect, Rect::new(45, 26, 550, 42));
        let last = &l.lines[9];
        assert_eq!(last.rect.top, 10 + 9 * 16);
        assert_eq!(last.items[0], TextItem { x: 45, y: 154, text: b"Bob:".to_vec(), colour: 0xFF00FF00 });
        assert_eq!(last.items[1].x, 45 + 4 * 8 + 5);
        assert_eq!(l.bottom, 10 + 11 * 16);
        assert_eq!(l.outline.len(), 4);
        c.set_timestamps(true);
        let l = c.layout(&m, 1920);
        let last = &l.lines[9];
        assert_eq!(last.items[0].text, b"[19:58:04]");
        assert_eq!(last.items[1].x, 80 + 50);
        assert_eq!(l.outline.len(), 8);
        c.cycle_mode();
        assert!(c.layout(&m, 1920).outline.is_empty());
    }
}
