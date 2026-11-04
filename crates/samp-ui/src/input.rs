//! Chat input line: open/close, editing, history and what Enter does (UR §8; UCA
//! "Local chat input").

use std::collections::VecDeque;

use samp_proto::rpcs::ui::{PlayerRequestChatMessage, PlayerRequestCommandMessage};

use crate::commands::{LocalCommand, split_command};
use crate::edit::{CharResult, EditKey, LineEdit, Mods};
use crate::{Rect, round_i};

/// Characters taken from the edit box on Enter (UCA step 1).
pub const MAX_LINE: usize = 128;
/// Unknown (notes U-INPUT-CAP): whether the edit box itself stops at 128 chars or only
/// Enter truncates. Assumed: the box stops accepting characters at this length.
pub const EDIT_CAPACITY: usize = MAX_LINE;
/// History entries (UR §8: 10 × 129 bytes).
pub const HISTORY_SIZE: usize = 10;
/// Characters kept per history entry (129 bytes incl. NUL).
pub const HISTORY_ENTRY_MAX: usize = 128;
/// Unknown (notes U-HISTORY-DUP): whether a line equal to the newest entry is pushed again.
pub const HISTORY_SKIPS_DUPLICATES: bool = false;
/// Maximum input box width (UR §8).
pub const MAX_WIDTH: i32 = 800;
/// Input box x (UR §8).
pub const BOX_X: i32 = 40;

/// What Enter produced (the input is closed afterwards in every case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submit {
    /// Empty line: just close.
    Empty,
    /// Plain text with a netgame: send RPC 101.
    Chat(PlayerRequestChatMessage),
    /// `/` line that is not local, with a netgame: send RPC 50 with the whole line.
    Command(PlayerRequestCommandMessage),
    /// A registered local command: run it, nothing is sent.
    Local(LocalCommand),
    /// `/` line, not local, no netgame: print debug line "I don't know that command.".
    UnknownCommand,
    /// Plain text without a netgame: nothing happens.
    Nothing,
}

/// The chat input (global A 0x1026e9fc).
#[derive(Clone, Debug)]
pub struct ChatInput {
    open: bool,
    edit: LineEdit,
    history: VecDeque<Vec<u8>>,
    /// −1 = the live line.
    index: isize,
}

impl Default for ChatInput {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatInput {
    pub fn new() -> Self {
        Self { open: false, edit: LineEdit::new(EDIT_CAPACITY), history: VecDeque::new(), index: -1 }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn text(&self) -> &[u8] {
        self.edit.text()
    }
    pub fn edit(&self) -> &LineEdit {
        &self.edit
    }
    pub fn history(&self) -> impl Iterator<Item = &[u8]> {
        self.history.iter().map(Vec::as_slice)
    }

    /// Open (A 0x10069480): show, enable, focus. Returns false if already open.
    /// Opening does not touch the scoreboard or the chat mode. While open the host
    /// requests cursor mode 2 every frame.
    pub fn open(&mut self) -> bool {
        if self.open {
            return false;
        }
        self.open = true;
        true
    }

    /// Close (A 0x10069580): focus out, disable and hide the edit box, cursor mode 0
    /// with immediate restore. The text and the history position are kept: the
    /// close routine does not touch them (2021 build), and the original client was
    /// seen to keep "not sent" after ESC and send it with the next line typed after T.
    /// Enter clears the text itself ([`Self::submit`]).
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Typed character while open.
    pub fn char(&mut self, c: u8) -> CharResult {
        self.edit.char(c)
    }
    pub fn key(&mut self, key: EditKey, mods: Mods) -> Option<Vec<u8>> {
        self.edit.key(key, mods)
    }
    pub fn paste(&mut self, clip: &[u8]) {
        self.edit.paste(clip);
    }
    /// Replaces the edit-box text (caret at the end).
    pub fn set_text(&mut self, t: &[u8]) {
        self.edit.set_text(t);
    }

    /// Up: older entries (A 0x10069630). TRACE-3 U-HISTORY-RESET: the live line is
    /// not saved.
    pub fn history_older(&mut self) {
        let next = self.index + 1;
        if (next as usize) < self.history.len() {
            self.index = next;
            let t = self.history[next as usize].clone();
            self.edit.set_text(&t);
        }
    }

    /// Down: newer entries; past the newest shows an empty line (A 0x100696a0;
    /// TRACE-3 U-HISTORY-RESET).
    pub fn history_newer(&mut self) {
        match self.index {
            -1 => {}
            0 => {
                self.index = -1;
                self.edit.clear();
            }
            i => {
                self.index = i - 1;
                let t = self.history[(i - 1) as usize].clone();
                self.edit.set_text(&t);
            }
        }
    }

    fn push_history(&mut self, line: &[u8]) {
        let line = &line[..line.len().min(HISTORY_ENTRY_MAX)];
        if HISTORY_SKIPS_DUPLICATES && self.history.front().is_some_and(|h| h == line) {
            return;
        }
        self.history.push_front(line.to_vec());
        self.history.truncate(HISTORY_SIZE);
    }

    /// Enter (input processor A 0x100699d0): take up to 128 chars; a non-empty line is
    /// pushed to the history (the position is kept) and classified; then
    /// the edit box is cleared and the input closed. The caller also scrolls the chat
    /// (position 90, clamped by the scrollbar) when the input was open.
    pub fn submit(&mut self, netgame: bool, debug_mode: bool) -> Submit {
        let line = self.edit.text()[..self.edit.text().len().min(MAX_LINE)].to_vec();
        let out = if line.is_empty() {
            Submit::Empty
        } else {
            // TRACE-3 U-HISTORY-RESET: the history position survives sending.
            self.push_history(&line);
            if split_command(&line).is_none() {
                if netgame { Submit::Chat(PlayerRequestChatMessage { message: line }) } else { Submit::Nothing }
            } else if let Some(cmd) = LocalCommand::parse(&line, debug_mode) {
                Submit::Local(cmd)
            } else if netgame {
                Submit::Command(PlayerRequestCommandMessage { command: line })
            } else {
                Submit::UnknownCommand
            }
        };
        self.edit.clear();
        self.close();
        out
    }
}

/// Input box rect (UR §8): width `min(800, round(0.6 · clientWidth))` at
/// `(40, chatBottom)`, height `14 + round(1.5 · h0)` with `h0` = GUI font 0 height.
pub fn box_rect(client_w: i32, chat_bottom: i32, gui0_h: i32) -> Rect {
    let w = MAX_WIDTH.min(round_i(0.6 * f64::from(client_w)));
    Rect::xywh(BOX_X, chat_bottom, w, 14 + round_i(1.5 * f64::from(gui0_h)))
}

/// Size of the host GUI dialog while the input is open: `(width + 100) × (chatBottom + 50)`.
pub fn host_dialog_size(box_w: i32, chat_bottom: i32) -> (i32, i32) {
    (box_w + 100, chat_bottom + 50)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandKind;

    fn typed(i: &mut ChatInput, s: &[u8]) {
        for &c in s {
            i.char(c);
        }
    }

    #[test]
    fn uca_local_input_enter_classification() {
        let mut i = ChatInput::new();
        i.open();
        typed(&mut i, b"hello");
        assert_eq!(i.submit(true, false), Submit::Chat(PlayerRequestChatMessage { message: b"hello".to_vec() }));
        assert!(!i.is_open() && i.text().is_empty());
        typed(&mut i, b"/kill now");
        assert_eq!(
            i.submit(true, false),
            Submit::Command(PlayerRequestCommandMessage { command: b"/kill now".to_vec() })
        );
        typed(&mut i, b"/kill");
        assert_eq!(i.submit(false, false), Submit::UnknownCommand);
        typed(&mut i, b"hi");
        assert_eq!(i.submit(false, false), Submit::Nothing);
        typed(&mut i, b"/PAGESIZE 15");
        assert!(matches!(i.submit(true, false), Submit::Local(LocalCommand { kind: CommandKind::PageSize, .. })));
        // Debug commands are sent to the server when not in debug mode.
        typed(&mut i, b"/set_time 1 2");
        assert!(matches!(i.submit(true, false), Submit::Command(_)));
        assert_eq!(i.submit(true, false), Submit::Empty);
    }

    #[test]
    fn ur_8_line_capped_at_128() {
        let mut i = ChatInput::new();
        typed(&mut i, &[b'a'; 300]);
        match i.submit(true, false) {
            Submit::Chat(m) => assert_eq!(m.message.len(), 128),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn ur_8_history_10_entries_live_line_not_kept() {
        let mut i = ChatInput::new();
        for n in 0..12 {
            typed(&mut i, format!("line{n}").as_bytes());
            i.submit(true, false);
        }
        assert_eq!(i.history().count(), 10);
        assert_eq!(i.history().next(), Some(&b"line11"[..]));
        typed(&mut i, b"draft");
        i.history_older();
        assert_eq!(i.text(), b"line11");
        i.history_older();
        assert_eq!(i.text(), b"line10");
        for _ in 0..20 {
            i.history_older();
        }
        assert_eq!(i.text(), b"line2");
        i.history_newer();
        assert_eq!(i.text(), b"line3");
        for _ in 0..9 {
            i.history_newer();
        }
        // TRACE-3 U-HISTORY-RESET: past the newest entry the line is empty.
        assert_eq!(i.text(), b"");
        i.history_newer();
        assert_eq!(i.text(), b"");
    }

    #[test]
    fn ur_8_box_geometry() {
        assert_eq!(box_rect(1280, 170, 20), Rect::xywh(40, 170, 768, 44));
        assert_eq!(box_rect(1920, 170, 20).width(), 800);
        assert_eq!(host_dialog_size(768, 170), (868, 220));
    }

    #[test]
    fn close_keeps_text_and_history_position_enter_clears() {
        let mut i = ChatInput::new();
        i.open();
        typed(&mut i, b"one");
        i.submit(true, false);
        i.open();
        typed(&mut i, b"not sent");
        i.close();
        assert!(!i.is_open());
        i.open();
        assert_eq!(i.text(), b"not sent");
        i.history_older();
        assert_eq!(i.text(), b"one");
        i.close();
        i.open();
        assert_eq!(i.text(), b"one");
        i.history_newer();
        assert_eq!(i.text(), b"");
        typed(&mut i, b"two");
        i.submit(true, false);
        assert_eq!(i.text(), b"");
        // The position (−1 here) survives sending: Up shows the newest entry.
        i.open();
        i.history_older();
        assert_eq!(i.text(), b"two");
    }
}
