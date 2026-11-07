//! Dialogs: all six styles, list/tablist parsing, keyboard and selection, the RPC 62 response,
//! geometry and the local F1 help dialog. Entry points: [`Dialogs::show`] (RPC 61),
//! [`Dialog::layout`] and [`Dialogs::respond`]. Specs: docs/re/subsystems/ui-rendering.md
//! ("Dialogs") and ui-chat-audio.md ("61 ShowDialog", "62 OnPlayerDialogResponse");
//! measured geometry in docs/phase3/trace-notes.md ("Dialogs").

use samp_proto::rpcs::ui::{OnPlayerDialogResponse, ShowDialog};

use crate::colour::{self, strip_codes};
use crate::edit::LineEdit;
use crate::fonts::{FontId, TextMeasure};
use crate::text::c_str;
use crate::{Rect, round_i};

/// The client keeps the first 64 title characters.
pub const TITLE_MAX: usize = 64;
/// Button size `BW × BH` (dialog object fields +0x14/+0x18).
pub const BUTTON_W: i32 = 100;
pub const BUTTON_H: i32 = 30;
/// List rows are lines of at most 256 characters.
pub const LINE_MAX: usize = 256;
/// Tablist cells are at most 128 characters.
pub const CELL_MAX: usize = 128;
/// Up to 4 columns.
pub const MAX_COLUMNS: usize = 4;
/// Response text limit (edit box contents or first list column).
pub const RESPONSE_TEXT_MAX: usize = 128;
/// Characters the dialog edit box accepts (assumed).
pub const EDIT_CAPACITY: usize = 128;
/// DXUT list-box border: the first row is drawn at list top + 6.
pub const LIST_BORDER: i32 = 6;
/// DXUT list-box text margin and scrollbar width: row text is drawn from list left plus 6 + 5
/// to list right minus 16 + 6 + 5; the selection bar from left + 6 to right − 22.
pub const LIST_MARGIN: i32 = 5;
pub const LIST_SCROLLBAR_W: i32 = 16;
/// List height clamp and list/tablist width limits.
pub const LIST_MIN_H: i32 = 200;
pub const LIST_MAX_H: i32 = 400;
pub const LIST_MIN_W: i32 = 400;
pub const LIST_MAX_W: i32 = 800;
/// Minimum width for all styles: `2·BW + 30`.
pub const MIN_W: i32 = 2 * BUTTON_W + 30;
/// Empty lines make no row: neither a trailing `\n` nor an inner empty line (`a\n\nb` = 2 rows).
pub const SKIP_EMPTY_ROWS: bool = true;
/// List rows (styles 2/4/5) are drawn through the colour-code wrapper: `{RRGGBB}` recolours per
/// glyph, the stripped text is drawn, base colour 0xFFFFFFFF.
pub const LIST_ROWS_INTERPRET_COLOUR_CODES: bool = true;
/// Row text colour (selected and unselected rows).
pub const ROW_COLOUR: u32 = 0xFFFF_FFFF;
/// Selection bar tint (drawn with the sampgui texel rect (17,269,241,287)).
pub const SELECTION_COLOUR: u32 = 0xFFB9_2228;
/// Background colour (all four corners) and caption text colour.
pub const BACKGROUND: u32 = 0xDC05_0505;
pub const CAPTION_COLOUR: u32 = 0xFFD2_D2D2;
/// List item colours.
pub const ITEM_COLOURS: (u32, u32) = (0xC8FF_FFFF, 0xC880_8080);

/// F1 help dialog: local, id 1, style 0, no button 2, nothing sent.
pub const HELP_ID: i16 = 1;
pub const HELP_TITLE: &[u8] = b"SA-MP Keys:";
pub const HELP_BUTTON: &[u8] = b"OK";
/// Help body. Mirrors the samp.dll string at 0x100ea470.
pub const HELP_BODY: &[u8] = b"{BB9999}F1\t{BBBBBB}Display this help dialog\n\
{BB9999}Tab\t{BBBBBB}Display the scoreboard\n\
{BB9999}F4\t{BBBBBB}Allows you to change class next time you respawn\n\
{BB9999}F5\t{BBBBBB}Show bandwidth statistics\n\
{BB9999}F7\t{BBBBBB}Toggle the chat box\n\
{BB9999}F8\t{BBBBBB}Take a screenshot\n\
{BB9999}F9\t{BBBBBB}Toggle the deathwindow\n\
{BB9999}T/F6\t{BBBBBB}Allows you to enter a chat message\n\
{BB9999}G\t{BBBBBB}Enter vehicle as passenger\n";

/// Dialog styles (the RPC 61 style byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    MsgBox,
    Input,
    List,
    Password,
    Tablist,
    TablistHeaders,
    /// Style byte above 5: treated as a message box whose body is not drawn (assumed).
    Other(u8),
}

impl Style {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::MsgBox,
            1 => Self::Input,
            2 => Self::List,
            3 => Self::Password,
            4 => Self::Tablist,
            5 => Self::TablistHeaders,
            o => Self::Other(o),
        }
    }
    pub fn is_list(self) -> bool {
        matches!(self, Self::List | Self::Tablist | Self::TablistHeaders)
    }
    pub fn has_edit(self) -> bool {
        matches!(self, Self::Input | Self::Password)
    }
    /// Body text is drawn for styles 0, 1 and 3.
    pub fn draws_body(self) -> bool {
        matches!(self, Self::MsgBox | Self::Input | Self::Password)
    }
}

/// Which button answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    /// Button 2 or ESC: response 0.
    Cancel = 0,
    /// Button 1, Enter, list activation (double click): response 1.
    Ok = 1,
}

/// List keys (stock DXUT list-box behaviour, inferred).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Splits a body into rows on `\n`, each truncated to [`LINE_MAX`]; empty lines
/// make no row.
pub fn split_rows(body: &[u8]) -> Vec<&[u8]> {
    body.split(|&b| b == b'\n')
        .map(|l| &l[..l.len().min(LINE_MAX)])
        .filter(|l| !(SKIP_EMPTY_ROWS && l.is_empty()))
        .collect()
}

/// Splits a tablist row on `\t`: runs of tabs are one separator (leading and trailing
/// tabs produce no cell); at most 4 cells of ≤ 128 chars.
pub fn split_cells(row: &[u8]) -> Vec<Vec<u8>> {
    row.split(|&b| b == b'\t')
        .filter(|c| !c.is_empty())
        .take(MAX_COLUMNS)
        .map(|c| c[..c.len().min(CELL_MAX)].to_vec())
        .collect()
}

/// An open dialog.
#[derive(Clone, Debug)]
pub struct Dialog {
    pub id: i16,
    pub style: Style,
    pub title: Vec<u8>,
    pub button1: Vec<u8>,
    pub button2: Vec<u8>,
    pub body: Vec<u8>,
    /// False for local dialogs (F1): answering sends nothing.
    pub to_server: bool,
    /// List rows (cells); style 2 rows have one cell (the whole line).
    rows: Vec<Vec<Vec<u8>>>,
    /// Style 5 header cells.
    header: Vec<Vec<u8>>,
    columns: usize,
    selected: Option<usize>,
    top: usize,
    pub edit: LineEdit,
}

impl Dialog {
    /// Builds a dialog and parses its body for the style.
    pub fn new(id: i16, style: u8, title: &[u8], button1: &[u8], button2: &[u8], body: &[u8], to_server: bool) -> Self {
        let style = Style::from_byte(style);
        let title = c_str(title);
        let mut d = Self {
            id,
            style,
            title: title[..title.len().min(TITLE_MAX)].to_vec(),
            button1: c_str(button1).to_vec(),
            button2: c_str(button2).to_vec(),
            body: c_str(body).to_vec(),
            to_server,
            rows: Vec::new(),
            header: Vec::new(),
            columns: 0,
            selected: None,
            top: 0,
            edit: LineEdit::new(EDIT_CAPACITY),
        };
        match style {
            Style::List => {
                d.rows = split_rows(&d.body).into_iter().map(|r| vec![r.to_vec()]).collect();
                d.columns = 1;
            }
            Style::Tablist | Style::TablistHeaders => {
                let mut rows: Vec<Vec<Vec<u8>>> = split_rows(&d.body).into_iter().map(split_cells).collect();
                // The column count is the first row's cell count
                // (the header row for style 5); extra cells of later rows are not drawn
                // (they still count towards the column widths, see `layout`).
                d.columns = rows.first().map(Vec::len).unwrap_or(0);
                if style == Style::TablistHeaders && !rows.is_empty() {
                    d.header = rows.remove(0);
                }
                d.rows = rows;
            }
            _ => {}
        }
        if style.is_list() && !d.rows.is_empty() {
            d.selected = Some(0);
        }
        d.edit.set_masked(style == Style::Password);
        d
    }

    /// The F1 help dialog.
    pub fn help() -> Self {
        Self::new(HELP_ID, 0, HELP_TITLE, HELP_BUTTON, b"", HELP_BODY, false)
    }

    pub fn rows(&self) -> &[Vec<Vec<u8>>] {
        &self.rows
    }
    pub fn header(&self) -> &[Vec<u8>] {
        &self.header
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    /// First visible list row.
    pub fn top_row(&self) -> usize {
        self.top
    }
    /// Button 2 is shown only when its label is non-empty.
    pub fn has_button2(&self) -> bool {
        !self.button2.is_empty()
    }

    /// Selects a data row (mouse click).
    pub fn select(&mut self, row: usize) {
        if row < self.rows.len() {
            self.selected = Some(row);
        }
    }

    /// List keyboard navigation; `page_rows` = rows visible in the list box.
    pub fn list_key(&mut self, key: ListKey, page_rows: usize) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let cur = self.selected.unwrap_or(0) as isize;
        let step = page_rows.saturating_sub(1).max(1) as isize;
        let next = match key {
            ListKey::Up => cur - 1,
            ListKey::Down => cur + 1,
            ListKey::Home => 0,
            ListKey::End => n as isize - 1,
            ListKey::PageUp => cur - step,
            ListKey::PageDown => cur + step,
        };
        let s = next.clamp(0, n as isize - 1) as usize;
        self.selected = Some(s);
        if s < self.top {
            self.top = s;
        } else if page_rows > 0 && s >= self.top + page_rows {
            self.top = s + 1 - page_rows;
        }
    }

    /// The RPC 62 payload for a button: `listItem` = selected data row for
    /// list styles (else −1); text = edit box (1, 3), first column of the selected row
    /// with codes stripped (2, 4, 5), or empty; both ≤ 128 chars.
    pub fn response(&self, button: Button) -> OnPlayerDialogResponse {
        let (list_item, text) = if self.style.is_list() {
            match self.selected.and_then(|i| self.rows.get(i).map(|r| (i, r))) {
                Some((i, row)) => {
                    let first = row.first().map(|c| strip_codes(c)).unwrap_or_default();
                    (i as i16, first[..first.len().min(RESPONSE_TEXT_MAX)].to_vec())
                }
                None => (-1, Vec::new()),
            }
        } else if self.style.has_edit() {
            let t = self.edit.text();
            (-1, t[..t.len().min(RESPONSE_TEXT_MAX)].to_vec())
        } else {
            (-1, Vec::new())
        };
        OnPlayerDialogResponse { dialog_id: self.id as u16, response: button as u8, list_item, text }
    }

    /// Geometry for a client rect of `client_w × client_h`. Mirrors samp.dll 0x1006f500.
    pub fn layout(&self, m: &dyn TextMeasure, client_w: i32, client_h: i32) -> DialogLayout {
        // Caption height C is the main font's "Y" height + 4 (20 at width 1280); the style-5
        // header height is the small font's.
        let main_h = m.line_height(FontId::Main);
        let small_h = m.line_height(FontId::Small);
        let h0 = m.line_height(FontId::Gui0);
        let h1 = m.line_height(FontId::Gui1);
        let c = if self.title.is_empty() { 0 } else { main_h + 4 };
        let list_h = (self.rows.len() as i32 * h1 + 2 * LIST_BORDER).clamp(LIST_MIN_H, LIST_MAX_H);
        let edit_h = round_i(1.5 * f64::from(h0));
        let mut columns = Vec::new();
        let mut header_h = 0;
        let (mut w, h) = match self.style {
            Style::List => {
                let widest =
                    self.rows.iter().map(|r| m.measure(FontId::Gui1, &strip_codes(&r[0])).w).max().unwrap_or(0);
                ((widest + 20 + 40).clamp(LIST_MIN_W, LIST_MAX_W), c + BUTTON_H + 15 + list_h)
            }
            Style::Tablist | Style::TablistHeaders => {
                // Widths come from up to 4 cells of every row, header included, even beyond the
                // drawn column count.
                let mut raw = [0i32; MAX_COLUMNS];
                let all = std::iter::once(&self.header).chain(self.rows.iter());
                for row in all {
                    for (k, cell) in row.iter().enumerate().take(MAX_COLUMNS) {
                        raw[k] = raw[k].max(m.measure(FontId::Gui1, &strip_codes(cell)).w);
                    }
                }
                let total = raw.iter().sum::<i32>() + 80;
                let n = self.columns.max(1);
                columns = (0..n.min(3)).map(|k| raw[k] + 20).collect();
                let mut w = (total + 40).max(LIST_MIN_W);
                if total + 40 < LIST_MIN_W {
                    w = LIST_MIN_W;
                    if n > 1 {
                        let extra = (360 - total) / (n as i32 - 1);
                        for cw in columns.iter_mut().take(n - 1) {
                            *cw += extra;
                        }
                    }
                }
                if n == MAX_COLUMNS {
                    // The 4th column runs to the list's text edge.
                    columns.push(w - 10 - columns.iter().sum::<i32>());
                }
                if self.style == Style::TablistHeaders {
                    header_h = small_h;
                }
                (w, c + BUTTON_H + 15 + list_h + header_h)
            }
            _ => {
                let b = m.measure_block(FontId::Small, &strip_codes(&self.body));
                let mut h = c + b.h + 25 + BUTTON_H;
                if self.style.has_edit() {
                    // Truncated, not rounded: 42 for h1 = 18.
                    h += (1.6 * f64::from(h1) + 14.0) as i32;
                }
                (b.w + 40, h)
            }
        };
        w = w.max(MIN_W);
        // Centred with rounding (227.5 → 228).
        let x = round_i(f64::from(client_w - w) / 2.0);
        let y = round_i(f64::from(client_h - h) / 2.0);
        // Control coordinates are relative to the client area below the caption.
        let ctl = |lx: i32, ly: i32, cw: i32, ch: i32| Rect::xywh(x + lx, y + c + ly, cw, ch);
        let by = h - c - BUTTON_H - 5;
        let (button1, button2) = if self.has_button2() {
            (ctl(w / 2 - BUTTON_W - 10, by, BUTTON_W, BUTTON_H), Some(ctl(w / 2 + 10, by, BUTTON_W, BUTTON_H)))
        } else {
            (ctl(w / 2 - BUTTON_W / 2, by, BUTTON_W, BUTTON_H), None)
        };
        let edit = self.style.has_edit().then(|| ctl(5, h - c - edit_h - BUTTON_H - 24, w - 10, 14 + edit_h));
        let list = self.style.is_list().then(|| ctl(5, header_h + 5, w - 10, list_h));
        let body = self.style.draws_body().then(|| DialogText {
            x: x + 20,
            y: y + c + 5,
            text: self.body.clone(),
            colour: colour::DIALOG_BODY,
        });
        let mut headers = Vec::new();
        if self.style == Style::TablistHeaders {
            let mut hx = x + 16;
            for (k, cell) in self.header.iter().take(self.columns).enumerate() {
                headers.push(DialogText { x: hx, y: y + c + 3, text: cell.clone(), colour: colour::DIALOG_BODY });
                hx += columns.get(k).copied().unwrap_or(0);
            }
        }
        DialogLayout {
            rect: Rect::xywh(x, y, w, h),
            caption_h: c,
            body,
            list,
            list_row_h: h1,
            columns,
            headers,
            edit,
            button1,
            button2,
        }
    }

    /// Rows visible in the list box (DXUT page size: text-rect height / row height).
    pub fn page_rows(&self, list: &Rect, row_h: i32) -> usize {
        let text_h = list.height() - 2 * LIST_BORDER;
        if row_h > 0 { (text_h / row_h).max(0) as usize } else { 0 }
    }

    /// The list row under client point `(px, py)`, if any (mouse selection).
    pub fn row_at(&self, list: &Rect, row_h: i32, px: i32, py: i32) -> Option<usize> {
        let sel = list_selection_rect(list);
        if row_h <= 0 || px < sel.left || px >= sel.right || py < sel.top || py >= sel.bottom {
            return None;
        }
        let i = self.top + ((py - sel.top) / row_h) as usize;
        (i < self.rows.len()).then_some(i)
    }
}

/// DXUT list-box selection rect: bounding box minus the scrollbar, deflated by the border.
pub fn list_selection_rect(list: &Rect) -> Rect {
    Rect::new(
        list.left + LIST_BORDER,
        list.top + LIST_BORDER,
        list.right - LIST_SCROLLBAR_W - LIST_BORDER,
        list.bottom - LIST_BORDER,
    )
}

/// DXUT list-box text rect: the selection rect minus the margin left and right.
pub fn list_text_rect(list: &Rect) -> Rect {
    let s = list_selection_rect(list);
    Rect::new(s.left + LIST_MARGIN, s.top, s.right - LIST_MARGIN, s.bottom)
}

/// Text drawn by the font renderer (small font, outline on, `{RRGGBB}` recolours).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogText {
    pub x: i32,
    pub y: i32,
    pub text: Vec<u8>,
    pub colour: u32,
}

/// Dialog geometry in screen pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogLayout {
    /// Outer rect including the caption, centred in the client rect.
    pub rect: Rect,
    /// Caption height `C` (main-font `"Y"` height + 4 when the title is non-empty).
    pub caption_h: i32,
    /// Body text (styles 0, 1, 3) at `(X + 20, Y + C + 5)`, colour 0xFFA9C4E4.
    pub body: Option<DialogText>,
    pub list: Option<Rect>,
    pub list_row_h: i32,
    /// Tablist column widths.
    pub columns: Vec<i32>,
    /// Style 5 header texts at `x = X + 16 + Σ previous widths`, `y = Y + C + 3`.
    pub headers: Vec<DialogText>,
    pub edit: Option<Rect>,
    pub button1: Rect,
    pub button2: Option<Rect>,
}

/// Outcome of RPC 61.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowOutcome {
    /// Title or body empty: nothing shown, nothing closed.
    Ignored,
    /// Negative id: an open dialog (if any) was closed without a response.
    Closed { was_open: bool },
    /// Shown. The host closes the chat input and focuses the edit box (1, 3) or the
    /// list (2, 4, 5); cursor mode 2 is requested every frame while drawn.
    Opened,
}

/// The dialog object. Mirrors the samp.dll global at 0x1026e9c8.
#[derive(Clone, Debug, Default)]
pub struct Dialogs {
    current: Option<Dialog>,
}

impl Dialogs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }
    pub fn current(&self) -> Option<&Dialog> {
        self.current.as_ref()
    }
    pub fn current_mut(&mut self) -> Option<&mut Dialog> {
        self.current.as_mut()
    }

    /// RPC 61 ShowDialog.
    pub fn show(&mut self, rpc: &ShowDialog) -> ShowOutcome {
        if c_str(&rpc.title).is_empty() || c_str(&rpc.body).is_empty() {
            return ShowOutcome::Ignored;
        }
        if rpc.dialog_id < 0 {
            let was_open = self.current.take().is_some();
            return ShowOutcome::Closed { was_open };
        }
        self.current =
            Some(Dialog::new(rpc.dialog_id, rpc.style, &rpc.title, &rpc.button1, &rpc.button2, &rpc.body, true));
        ShowOutcome::Opened
    }

    /// F1 (polled while held): opens the local help dialog if no dialog is open.
    pub fn open_help(&mut self) -> bool {
        if self.current.is_some() {
            return false;
        }
        self.current = Some(Dialog::help());
        true
    }

    /// Answers and closes the dialog (cursor mode 0 with the 10-frame restore delay). Mirrors
    /// samp.dll 0x10070690. Returns the RPC 62 payload if the dialog goes to the
    /// server; the client never waits for the server.
    pub fn respond(&mut self, button: Button) -> Option<OnPlayerDialogResponse> {
        let d = self.current.take()?;
        d.to_server.then(|| d.response(button))
    }

    /// Close without answering.
    pub fn close(&mut self) {
        self.current = None;
    }

    /// Drawn only while open, the chat input is closed and the scoreboard is closed.
    pub fn drawn(&self, input_open: bool, scoreboard_open: bool) -> bool {
        self.is_open() && !input_open && !scoreboard_open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::MonoMeasure;

    const M: MonoMeasure = MonoMeasure { char_w: 8, line_h: 16 };

    fn rpc(id: i16, style: u8, body: &[u8], b2: &[u8]) -> ShowDialog {
        ShowDialog {
            dialog_id: id,
            style,
            title: b"Title".to_vec(),
            button1: b"OK".to_vec(),
            button2: b2.to_vec(),
            body: body.to_vec(),
        }
    }

    #[test]
    fn uca_61_empty_title_or_body_ignored_negative_closes() {
        let mut d = Dialogs::new();
        let mut r = rpc(5, 0, b"", b"");
        assert_eq!(d.show(&r), ShowOutcome::Ignored);
        r.body = b"x".to_vec();
        r.title.clear();
        assert_eq!(d.show(&r), ShowOutcome::Ignored);
        assert_eq!(d.show(&rpc(5, 0, b"x", b"")), ShowOutcome::Opened);
        assert_eq!(d.show(&rpc(-1, 0, b" ", b"")), ShowOutcome::Closed { was_open: true });
        assert!(!d.is_open());
        assert_eq!(d.show(&rpc(-1, 0, b" ", b"")), ShowOutcome::Closed { was_open: false });
        // Ignored RPCs do not close an open dialog.
        d.show(&rpc(5, 0, b"x", b""));
        d.show(&rpc(-1, 0, b"", b""));
        assert!(d.is_open());
    }

    #[test]
    fn uca_61_title_kept_to_64() {
        let mut r = rpc(1, 0, b"x", b"");
        r.title = vec![b't'; 100];
        let mut d = Dialogs::new();
        d.show(&r);
        assert_eq!(d.current().unwrap().title.len(), 64);
    }

    #[test]
    fn ur_13_2_list_rows_and_tablist_cells() {
        assert_eq!(split_rows(b"a\nb\n"), vec![&b"a"[..], b"b"]);
        assert_eq!(split_rows(b"a\n\nb"), vec![&b"a"[..], b"b"]);
        assert_eq!(split_rows(&[b'x'; 300])[0].len(), 256);
        assert_eq!(split_cells(b"a\t\t\tb\tc\td\te"), vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        assert_eq!(split_cells(&[b'y'; 200])[0].len(), 128);
        let d = Dialog::new(1, 5, b"T", b"OK", b"", b"H1\tH2\n{FF0000}r1\tx\nr2\ty\n", true);
        assert_eq!(d.header(), &[b"H1".to_vec(), b"H2".to_vec()]);
        assert_eq!(d.columns(), 2);
        assert_eq!(d.rows().len(), 2);
        assert_eq!(d.selected(), Some(0));
    }

    #[test]
    fn uca_62_responses_per_style() {
        // List: data row index, first column, codes stripped.
        let mut d = Dialog::new(7, 5, b"T", b"OK", b"", b"H1\tH2\n{FF0000}r1\tx\nr2\ty", true);
        d.list_key(ListKey::Down, 10);
        let r = d.response(Button::Ok);
        assert_eq!((r.dialog_id, r.response, r.list_item, r.text.as_slice()), (7, 1, 1, &b"r2"[..]));
        d.list_key(ListKey::Up, 10);
        assert_eq!(d.response(Button::Cancel).text, b"r1");
        assert_eq!(d.response(Button::Cancel).response, 0);
        // Input: edit text, list item -1.
        let mut d = Dialog::new(-2, 1, b"T", b"OK", b"Cancel", b"Enter:", true);
        for &c in b"hello" {
            d.edit.char(c);
        }
        let r = d.response(Button::Ok);
        assert_eq!((r.dialog_id, r.list_item, r.text.as_slice()), (0xFFFE, -1, &b"hello"[..]));
        // Msgbox: empty text.
        let d = Dialog::new(3, 0, b"T", b"OK", b"", b"hi", true);
        assert_eq!(d.response(Button::Ok).text, b"");
        // Password masks but sends the clear text.
        let mut d = Dialog::new(3, 3, b"T", b"OK", b"", b"pw", true);
        d.edit.char(b'a');
        assert_eq!(d.edit.display_wide(|b| b.iter().map(|&c| u16::from(c)).collect()), vec![0x25CF]);
        assert_eq!(d.response(Button::Ok).text, b"a");
        // List style 2 keeps tabs in the row; response limited to 128.
        let body = [vec![b'z'; 200], b"\nb".to_vec()].concat();
        let d = Dialog::new(3, 2, b"T", b"OK", b"", &body, true);
        assert_eq!(d.response(Button::Ok).text.len(), 128);
    }

    #[test]
    fn ur_13_6_help_dialog_is_local() {
        let mut d = Dialogs::new();
        assert!(d.open_help());
        assert!(!d.open_help());
        let h = d.current().unwrap();
        assert_eq!((h.id, h.style, h.title.as_slice()), (1, Style::MsgBox, HELP_TITLE));
        assert!(!h.has_button2());
        assert_eq!(d.respond(Button::Ok), None);
        assert!(!d.is_open());
        assert!(HELP_BODY.starts_with(b"{BB9999}F1\t{BBBBBB}Display this help dialog\n"));
        assert_eq!(HELP_BODY.iter().filter(|&&b| b == b'\n').count(), 9);
    }

    #[test]
    fn uca_62_respond_sends_and_closes() {
        let mut d = Dialogs::new();
        d.show(&rpc(9, 0, b"x", b"No"));
        let r = d.respond(Button::Cancel).unwrap();
        assert_eq!((r.dialog_id, r.response, r.list_item), (9, 0, -1));
        assert!(!d.is_open());
        assert_eq!(d.respond(Button::Ok), None);
        d.show(&rpc(9, 0, b"x", b"No"));
        assert!(d.drawn(false, false));
        assert!(!d.drawn(true, false) && !d.drawn(false, true));
    }

    #[test]
    fn list_keys_clamp_and_page() {
        let body: Vec<u8> = (0..30).flat_map(|i| format!("row{i}\n").into_bytes()).collect();
        let mut d = Dialog::new(1, 2, b"T", b"OK", b"", &body, true);
        d.list_key(ListKey::Up, 10);
        assert_eq!(d.selected(), Some(0));
        d.list_key(ListKey::PageDown, 10);
        assert_eq!(d.selected(), Some(9));
        d.list_key(ListKey::Down, 10);
        assert_eq!((d.selected(), d.top_row()), (Some(10), 1));
        d.list_key(ListKey::End, 10);
        assert_eq!((d.selected(), d.top_row()), (Some(29), 20));
        d.list_key(ListKey::PageUp, 10);
        assert_eq!(d.selected(), Some(20));
        d.list_key(ListKey::Home, 10);
        assert_eq!((d.selected(), d.top_row()), (Some(0), 0));
        d.select(99);
        assert_eq!(d.selected(), Some(0));
    }

    #[test]
    fn ur_13_2_msgbox_and_input_geometry() {
        // C = 16 + 4 = 20; body 2 lines of max 10 chars → 80 × 32.
        let d = Dialog::new(1, 0, b"T", b"OK", b"Cancel", b"0123456789\nabc", true);
        let l = d.layout(&M, 1280, 720);
        let (w, h) = (230, 20 + 32 + 25 + 30); // bw + 40 = 120 < 230
        assert_eq!(l.rect, Rect::xywh((1280 - w) / 2, (720 - h + 1) / 2, w, h));
        let (x, y) = (l.rect.left, l.rect.top);
        assert_eq!(l.button1, Rect::xywh(x + w / 2 - 110, y + 20 + h - 20 - 35, 100, 30));
        assert_eq!(l.button2, Some(Rect::xywh(x + w / 2 + 10, y + 20 + h - 20 - 35, 100, 30)));
        assert_eq!(l.body.as_ref().unwrap().x, x + 20);
        assert_eq!(l.body.as_ref().unwrap().y, y + 25);
        assert_eq!(l.body.as_ref().unwrap().colour, 0xFFA9C4E4);
        assert!(l.edit.is_none() && l.list.is_none());
        // Wide body; one button centred.
        let d = Dialog::new(1, 1, b"T", b"OK", b"", &[b'w'; 50], true);
        let l = d.layout(&M, 1280, 720);
        let w = 50 * 8 + 40;
        let h = 20 + 16 + 25 + 30 + 39; // trunc(1.6·16 + 14)
        assert_eq!((l.rect.width(), l.rect.height()), (w, h));
        assert_eq!(l.button1.left, l.rect.left + w / 2 - 50);
        assert!(l.button2.is_none());
        let e = l.edit.unwrap();
        assert_eq!(e, Rect::xywh(l.rect.left + 5, l.rect.top + 20 + h - 20 - 24 - 30 - 24, w - 10, 14 + 24));
    }

    #[test]
    fn ur_13_2_list_geometry() {
        let d = Dialog::new(1, 2, b"T", b"OK", b"", b"a\nbb\nccc", true);
        let l = d.layout(&M, 1280, 720);
        // Widest 24 + 60 < 400 → 400; LH = 3·16 + 12 → 200.
        assert_eq!((l.rect.width(), l.rect.height()), (400, 20 + 30 + 15 + 200));
        assert_eq!(l.list, Some(Rect::xywh(l.rect.left + 5, l.rect.top + 20 + 5, 390, 200)));
        let body: Vec<u8> = (0..40).flat_map(|_| [vec![b'q'; 120], b"\n".to_vec()].concat()).collect();
        let l = Dialog::new(1, 2, b"T", b"OK", b"", &body, true).layout(&M, 1920, 1080);
        assert_eq!(l.rect.width(), 800);
        assert_eq!(l.list.unwrap().height(), 400);
    }

    #[test]
    fn ur_13_2_tablist_column_widths() {
        // raw = [8, 16, 0, 0] → total 104; +40 < 400 → W 400, extra (360 − 104)/(2 − 1) to col 0.
        let d = Dialog::new(1, 4, b"T", b"OK", b"", b"a\tbb\nb\tc", true);
        let l = d.layout(&M, 1280, 720);
        assert_eq!(l.rect.width(), 400);
        assert_eq!(l.columns, vec![8 + 20 + 256, 16 + 20]);
        // Wide 4-column tablist: no extra; 4th column takes the rest.
        let row = [vec![b'a'; 30], b"\t".to_vec(), vec![b'b'; 20], b"\tc\td".to_vec()].concat();
        let l = Dialog::new(1, 4, b"T", b"OK", b"", &row, true).layout(&M, 1280, 720);
        let total = 240 + 160 + 8 + 8 + 80;
        assert_eq!(l.rect.width(), total + 40);
        assert_eq!(l.columns, vec![260, 180, 28, total + 40 - 10 - 468]);
        // Headers (style 5): list moves down by the header height; header x positions.
        let d = Dialog::new(1, 5, b"T", b"OK", b"", b"Name\tScore\nbob\t5", true);
        let l = d.layout(&M, 1280, 720);
        assert_eq!(l.list.unwrap().top, l.rect.top + 20 + 16 + 5);
        assert_eq!(l.rect.height(), 20 + 30 + 15 + 200 + 16);
        assert_eq!(l.headers[0].x, l.rect.left + 16);
        assert_eq!(l.headers[1].x, l.rect.left + 16 + l.columns[0]);
        assert_eq!(l.headers[0].y, l.rect.top + 20 + 3);
    }
}
