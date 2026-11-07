//! Single-line edit box used by the chat input and the dialog edit box.
//!
//! Both are DXUT IME edit boxes in samp. The keys follow the stock DXUT edit box: Home/End,
//! Left/Right (Ctrl = word), Shift extends the selection, Delete/Backspace, Insert toggles
//! overwrite, Ctrl+Insert copies, Shift+Insert pastes, Ctrl+A/C/X/V, other control characters
//! ignored (docs/re/subsystems/ui-rendering.md, "Chat input").

/// Keys the edit box handles on key-down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Left,
    Right,
    Home,
    End,
    Delete,
    Insert,
}

/// Modifier state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

/// Result of a typed character (`WM_CHAR`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharResult {
    /// Handled (text, caret or selection may have changed).
    Edited,
    /// Enter: the edit box raises its "string" event (chat send / dialog response 1).
    Enter,
    /// Ctrl+C / Ctrl+X: copy this text to the clipboard.
    Copy(Vec<u8>),
    /// Ignored control character.
    Ignored,
}

/// Masking character for password edit boxes (dialog style 3): U+25CF `●`, drawn with
/// `DrawTextW`.
pub const PASSWORD_MASK: u16 = 0x25CF;

/// The edit buffer: bytes in the ANSI code page, caret and selection anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineEdit {
    text: Vec<u8>,
    caret: usize,
    anchor: usize,
    insert_mode: bool,
    max_len: usize,
    masked: bool,
}

fn is_word_stop(t: &[u8], i: usize) -> bool {
    i < t.len() && !t[i].is_ascii_whitespace() && (i == 0 || t[i - 1].is_ascii_whitespace())
}

impl LineEdit {
    /// Empty edit box accepting at most `max_len` bytes.
    pub fn new(max_len: usize) -> Self {
        Self { text: Vec::new(), caret: 0, anchor: 0, insert_mode: true, max_len, masked: false }
    }

    pub fn text(&self) -> &[u8] {
        &self.text
    }
    pub fn caret(&self) -> usize {
        self.caret
    }
    pub fn max_len(&self) -> usize {
        self.max_len
    }
    pub fn insert_mode(&self) -> bool {
        self.insert_mode
    }
    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }
    pub fn masked(&self) -> bool {
        self.masked
    }

    /// Selected byte range (empty when nothing is selected).
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// Text as drawn (UTF-16, `DrawTextW`): masked boxes show one U+25CF per byte;
    /// otherwise `to_wide` converts the ANSI bytes (the host's code page).
    pub fn display_wide(&self, to_wide: impl Fn(&[u8]) -> Vec<u16>) -> Vec<u16> {
        if self.masked { vec![PASSWORD_MASK; self.text.len()] } else { to_wide(&self.text) }
    }

    /// Selection anchor (DXUT `m_nSelStart`).
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// Replaces the text (truncated to the maximum) and puts the caret at the end.
    pub fn set_text(&mut self, t: &[u8]) {
        self.text = t[..t.len().min(self.max_len)].to_vec();
        self.caret = self.text.len();
        self.anchor = self.caret;
    }

    pub fn clear(&mut self) {
        self.set_text(b"");
    }

    fn place(&mut self, pos: usize, extend: bool) {
        self.caret = pos.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    fn delete_selection(&mut self) -> bool {
        let r = self.selection();
        if r.is_empty() {
            return false;
        }
        self.text.drain(r.clone());
        self.caret = r.start;
        self.anchor = r.start;
        true
    }

    fn prior_word(&self) -> usize {
        (1..self.caret).rev().find(|&i| is_word_stop(&self.text, i)).unwrap_or(0)
    }

    fn next_word(&self) -> usize {
        (self.caret + 1..self.text.len()).find(|&i| is_word_stop(&self.text, i)).unwrap_or(self.text.len())
    }

    /// Key-down handling. Returns text to put on the clipboard for Ctrl+Insert.
    pub fn key(&mut self, key: EditKey, mods: Mods) -> Option<Vec<u8>> {
        match key {
            EditKey::Home => self.place(0, mods.shift),
            EditKey::End => self.place(self.text.len(), mods.shift),
            EditKey::Left => {
                let p = if mods.ctrl { self.prior_word() } else { self.caret.saturating_sub(1) };
                self.place(p, mods.shift);
            }
            EditKey::Right => {
                let p = if mods.ctrl { self.next_word() } else { self.caret + 1 };
                self.place(p, mods.shift);
            }
            EditKey::Delete => {
                if !self.delete_selection() && self.caret < self.text.len() {
                    self.text.remove(self.caret);
                }
            }
            EditKey::Insert => {
                if mods.ctrl {
                    return Some(self.text[self.selection()].to_vec());
                } else if mods.shift {
                    return None; // the caller pastes with `paste`
                }
                self.insert_mode = !self.insert_mode;
            }
        }
        None
    }

    /// Typed character (`WM_CHAR`).
    pub fn char(&mut self, c: u8) -> CharResult {
        match c {
            0x08 => {
                if !self.delete_selection() && self.caret > 0 {
                    self.caret -= 1;
                    self.text.remove(self.caret);
                    self.anchor = self.caret;
                }
                CharResult::Edited
            }
            0x0D => CharResult::Enter,
            0x01 => {
                self.anchor = 0;
                self.caret = self.text.len();
                CharResult::Edited
            }
            0x03 | 0x18 => {
                let sel = self.text[self.selection()].to_vec();
                if c == 0x18 {
                    self.delete_selection();
                }
                CharResult::Copy(sel)
            }
            0x16 => CharResult::Ignored, // Ctrl+V: the caller supplies the clipboard via `paste`
            0..=0x1F => CharResult::Ignored,
            _ => {
                self.delete_selection();
                if !self.insert_mode && self.caret < self.text.len() {
                    self.text[self.caret] = c;
                    self.caret += 1;
                } else if self.text.len() < self.max_len {
                    self.text.insert(self.caret, c);
                    self.caret += 1;
                }
                self.anchor = self.caret;
                CharResult::Edited
            }
        }
    }

    /// Paste (Ctrl+V / Shift+Insert): replaces the selection; stops at the maximum
    /// length and at the first control character.
    pub fn paste(&mut self, clip: &[u8]) {
        self.delete_selection();
        for &b in clip {
            if b < 0x20 || self.text.len() >= self.max_len {
                break;
            }
            self.text.insert(self.caret, b);
            self.caret += 1;
        }
        self.anchor = self.caret;
    }
}

/// DXUT edit box `PlaceCaret`: the first visible character after moving the caret to
/// `caret`: the view scrolls only when the caret leaves the
/// text rect. `x_of(cp)` = pixel x of the leading edge of character `cp` (width of
/// the text before it); `len` = text length; `width` = text rect width.
pub fn place_caret(first_visible: usize, caret: usize, len: usize, width: i32, x_of: &dyn Fn(usize) -> i32) -> usize {
    let first = first_visible.min(len);
    let caret = caret.min(len);
    let x_first = x_of(first);
    let x = x_of(caret);
    // Trailing edge of the caret character (leading edge at the end of the text).
    let x2 = if caret == len { x } else { x_of(caret + 1) };
    if x < x_first {
        return caret;
    }
    if x2 > x_first + width {
        let new_left = x2 - width;
        // XtoCP: the character containing `new_left`.
        let mut cp = (0..len).find(|&i| x_of(i + 1) > new_left).unwrap_or(len);
        if x_of(cp) < new_left {
            cp += 1;
        }
        return cp;
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &[u8], max: usize) -> LineEdit {
        let mut e = LineEdit::new(max);
        for &c in s {
            e.char(c);
        }
        e
    }

    #[test]
    fn typing_respects_max_length() {
        let e = typed(&[b'a'; 200], 128);
        assert_eq!(e.text().len(), 128);
    }

    #[test]
    fn caret_moves_and_editing() {
        let mut e = typed(b"hello world", 128);
        e.key(EditKey::Home, Mods::default());
        e.char(b'>');
        assert_eq!(e.text(), b">hello world");
        e.key(EditKey::End, Mods::default());
        e.char(0x08);
        assert_eq!(e.text(), b">hello worl");
        e.key(EditKey::Home, Mods::default());
        e.key(EditKey::Delete, Mods::default());
        assert_eq!(e.text(), b"hello worl");
    }

    #[test]
    fn word_moves() {
        let mut e = typed(b"one two  three", 128);
        let ctrl = Mods { ctrl: true, shift: false };
        e.key(EditKey::Left, ctrl);
        assert_eq!(e.caret(), 9);
        e.key(EditKey::Left, ctrl);
        assert_eq!(e.caret(), 4);
        e.key(EditKey::Left, ctrl);
        assert_eq!(e.caret(), 0);
        e.key(EditKey::Right, ctrl);
        assert_eq!(e.caret(), 4);
        e.key(EditKey::Right, ctrl);
        e.key(EditKey::Right, ctrl);
        assert_eq!(e.caret(), 14);
    }

    #[test]
    fn selection_overwrite_and_clipboard() {
        let mut e = typed(b"abcdef", 128);
        let shift = Mods { shift: true, ctrl: false };
        e.key(EditKey::Left, shift);
        e.key(EditKey::Left, shift);
        assert_eq!(e.selection(), 4..6);
        assert_eq!(e.char(0x18), CharResult::Copy(b"ef".to_vec()));
        assert_eq!(e.text(), b"abcd");
        e.char(0x01);
        e.paste(b"xy\nz");
        assert_eq!(e.text(), b"xy");
        e.key(EditKey::Insert, Mods::default());
        e.key(EditKey::Home, Mods::default());
        e.char(b'Q');
        assert_eq!(e.text(), b"Qy");
        assert_eq!(e.char(0x0D), CharResult::Enter);
        assert_eq!(e.char(0x1B), CharResult::Ignored);
    }

    #[test]
    fn password_mask() {
        let mut e = typed(b"secret", 128);
        e.set_masked(true);
        assert_eq!(e.display_wide(|b| b.iter().map(|&c| u16::from(c)).collect()), vec![0x25CF; 6]);
        assert_eq!(e.text(), b"secret");
    }

    #[test]
    fn dxut_first_visible() {
        // 10 px per char, 100 px box.
        let x = |cp: usize| cp as i32 * 10;
        // Caret at the end of 30 chars: last 10 visible.
        assert_eq!(place_caret(0, 30, 30, 100, &x), 20);
        // Caret moves left inside the box: no scroll.
        assert_eq!(place_caret(20, 25, 30, 100, &x), 20);
        // Caret left of the first visible char: it becomes the first.
        assert_eq!(place_caret(20, 4, 30, 100, &x), 4);
        // Caret right of the box: trailing edge brought to the right border.
        assert_eq!(place_caret(0, 12, 30, 100, &x), 3);
    }
}
