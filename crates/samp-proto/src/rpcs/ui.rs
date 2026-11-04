//! UI, chat and audio RPCs: chat messages and commands, game text, textdraws
//! (including click-to-select), dialogs, GTA menus and the internet audio stream.
//!
//! Spec: `docs/re/subsystems/ui-chat-audio.md` (section names below are `§<id>`).
//! Layouts were cross-checked against open.mp's NetCode; no wire-layout
//! disagreement exists for this module, only client-side limits, which are
//! documented per payload and are *not* applied by the decoders.
//!
//! Strings are raw bytes (Windows code page). The only Huffman-compressed
//! string here is the dialog body (§61), via [`crate::strcomp`].

use crate::bitstream::{BitReader, BitWriter};
use crate::rpcs::RpcPayload;
use crate::strcomp::{read_compressed_string, write_compressed_string};
use crate::types::{Vec2, Vec3, read_str8, read_str16, read_str32, write_str8, write_str16, write_str32};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of global textdraw ids (0..2047); per-player ids follow (2048..2303).
pub const GLOBAL_TEXTDRAW_POOL_SIZE: u16 = 2048;
/// Size of the client's textdraw pool (0x900). Ids at or above this are not
/// bounds-checked by the original client in show/hide; a hardened client must
/// ignore them (§134, §135).
pub const TEXTDRAW_POOL_SIZE: u16 = 0x900;
/// Textdraw id meaning "none / selection cancelled" (§83).
pub const INVALID_TEXTDRAW: u16 = 0xFFFF;
/// Textdraw texts must be shorter than this (§134 rejects `>= 800`; §105
/// accepts `<= 800` on the wire but the setter ignores `>= 800`).
pub const MAX_TEXTDRAW_TEXT: usize = 800;

/// Valid menu ids are `0..MAX_MENUS` (§76: show/hide ignore ids ≥ 128).
pub const MAX_MENUS: u8 = 128;
/// Rows per menu column (§76).
pub const MAX_MENU_ITEMS: usize = 12;
/// Size of every fixed menu string on the wire (title, headers, items).
pub const MENU_TEXT_LEN: usize = 32;
/// Row value meaning "no row" (returned by a non-interactive menu, §132).
pub const INVALID_MENU_ROW: u8 = 0xFF;

/// Receiver buffer for the decoded dialog body (§61).
pub const DIALOG_BODY_MAX: usize = 4096;

/// Dialog styles (§61).
pub mod dialog_style {
    pub const MSGBOX: u8 = 0;
    pub const INPUT: u8 = 1;
    pub const LIST: u8 = 2;
    pub const PASSWORD: u8 = 3;
    pub const TABLIST: u8 = 4;
    pub const TABLIST_HEADERS: u8 = 5;
}

/// Bits of [`TextDrawData::flags`] (§134, byte 0 of the 63-byte block).
pub mod textdraw_flags {
    pub const BOX: u8 = 1 << 0;
    pub const ALIGN_LEFT: u8 = 1 << 1;
    pub const ALIGN_RIGHT: u8 = 1 << 2;
    pub const ALIGN_CENTRE: u8 = 1 << 3;
    pub const PROPORTIONAL: u8 = 1 << 4;
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn read_menu_text(r: &mut BitReader) -> Option<MenuText> {
    let mut a = [0u8; MENU_TEXT_LEN];
    r.read_bytes_into(&mut a)?;
    Some(a)
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/// RPC 93 SendClientMessage (S→C): a coloured server line in the chat window.
///
/// Spec: ui-chat-audio.md §93. Wire: u32 colour (RGBA, `0xRRGGBBAA`), u32
/// length, text (not NUL-terminated).
///
/// Client behaviour: messages with length ≥ 256 are dropped; texts of
/// 145..255 chars are read but not shown; ≤ 144 chars get `~k~` expansion,
/// control characters → spaces, and a type-4 chat entry with colour
/// `(colour >> 8) | 0xFF000000`. open.mp's 128-char hint is storage only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendClientMessage {
    /// RGBA colour value.
    pub colour: u32,
    pub message: Vec<u8>,
}

impl RpcPayload for SendClientMessage {
    const ID: u8 = 93;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { colour: r.read_u32()?, message: read_str32(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.colour);
        write_str32(w, &self.message);
    }
}

/// RPC 101 PlayerChatMessage (S→C): a player's chat line (also the server's
/// echo of our own chat; the client never shows its own chat locally).
///
/// Spec: ui-chat-audio.md §101. Wire: u16 playerId, u8 length, text.
///
/// Client behaviour: handled only when fully connected; playerId must be the
/// local id or an existing remote slot < 1005, else ignored. Shown as a type-2
/// entry (name prefix, 72-char wrap), no `~k~` expansion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerChatMessage {
    pub player_id: u16,
    pub message: Vec<u8>,
}

impl RpcPayload for PlayerChatMessage {
    const ID: u8 = 101;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, message: read_str8(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        write_str8(w, &self.message);
    }
}

/// RPC 101 PlayerRequestChatMessage (C→S): chat line typed by the local player.
///
/// Spec: ui-chat-audio.md §101 (C→S). Wire: u8 length, text. Reliability
/// RELIABLE, priority 1, channel 0. The client's input line caps text at 128
/// chars; longer text is truncated to 255 bytes by [`write_str8`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerRequestChatMessage {
    pub message: Vec<u8>,
}

impl RpcPayload for PlayerRequestChatMessage {
    const ID: u8 = 101;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { message: read_str8(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        write_str8(w, &self.message);
    }
}

/// RPC 50 PlayerRequestCommandMessage (C→S): a `/command` line not handled locally.
///
/// Spec: ui-chat-audio.md §50. Wire: u32 length, text = the whole line as
/// typed, including the leading `/`. Reliability RELIABLE.
///
/// open.mp rejects length < 2 or > 256; the client's 128-char input cap keeps
/// it in range. The S→C form of id 50 is not registered by the client, so it
/// is not part of [`Incoming`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerRequestCommandMessage {
    pub command: Vec<u8>,
}

impl RpcPayload for PlayerRequestCommandMessage {
    const ID: u8 = 50;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { command: read_str32(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        write_str32(w, &self.command);
    }
}

// ---------------------------------------------------------------------------
// Game text
// ---------------------------------------------------------------------------

/// RPC 73 SendGameText (S→C): GTA "big message" centred text.
///
/// Spec: ui-chat-audio.md §73. Wire: u32 style, u32 time (ms), u32 length, text.
///
/// Client behaviour: the whole RPC is ignored unless length is 1..200; shown
/// only if style, as a signed int, is < 201 (the client does not restrict it
/// to GTA's 0..6 — a hardened client should ignore or clamp larger styles).
/// Clears current prints first; GTA `~x~` formatting applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendGameText {
    /// GTA big-message style; signed because the client compares it signed.
    pub style: i32,
    pub time_ms: u32,
    pub text: Vec<u8>,
}

impl RpcPayload for SendGameText {
    const ID: u8 = 73;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { style: r.read_u32()? as i32, time_ms: r.read_u32()?, text: read_str32(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.style as u32);
        w.write_u32(self.time_ms);
        write_str32(w, &self.text);
    }
}

// ---------------------------------------------------------------------------
// Textdraws
// ---------------------------------------------------------------------------

/// The fixed 63-byte textdraw property block of RPC 134 (§134), in wire order.
///
/// Colours are u32 values read little-endian from bytes laid out R,G,B,A in
/// memory, i.e. the value is `0xAABBGGRR` (open.mp writes `Colour::ABGR()`).
/// Positions/sizes are on the 640×448 virtual canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextDrawData {
    /// See [`textdraw_flags`]: box, align left/right/centre, proportional.
    /// Orientation: right if ALIGN_RIGHT, else centre if ALIGN_CENTRE, else left.
    pub flags: u8,
    pub letter_size: Vec2,
    pub letter_colour: u32,
    /// x = wrap width / box extent, y = centre size.
    pub text_size: Vec2,
    pub box_colour: u32,
    /// Used only when `outline` is 0.
    pub shadow: u8,
    pub outline: u8,
    /// Shadow/outline colour.
    pub background_colour: u32,
    /// 0..3 GTA fonts, 4 = sprite (`txd:name` text), 5 = model preview.
    pub style: u8,
    pub selectable: u8,
    pub position: Vec2,
    pub preview_model: u16,
    pub preview_rot: Vec3,
    pub preview_zoom: f32,
    pub preview_colour1: i16,
    pub preview_colour2: i16,
}

impl TextDrawData {
    /// Size of the block on the wire.
    pub const WIRE_LEN: usize = 63;

    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            flags: r.read_u8()?,
            letter_size: Vec2::read(r)?,
            letter_colour: r.read_u32()?,
            text_size: Vec2::read(r)?,
            box_colour: r.read_u32()?,
            shadow: r.read_u8()?,
            outline: r.read_u8()?,
            background_colour: r.read_u32()?,
            style: r.read_u8()?,
            selectable: r.read_u8()?,
            position: Vec2::read(r)?,
            preview_model: r.read_u16()?,
            preview_rot: Vec3::read(r)?,
            preview_zoom: r.read_f32()?,
            preview_colour1: r.read_i16()?,
            preview_colour2: r.read_i16()?,
        })
    }

    pub fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.flags);
        self.letter_size.write(w);
        w.write_u32(self.letter_colour);
        self.text_size.write(w);
        w.write_u32(self.box_colour);
        w.write_u8(self.shadow);
        w.write_u8(self.outline);
        w.write_u32(self.background_colour);
        w.write_u8(self.style);
        w.write_u8(self.selectable);
        self.position.write(w);
        w.write_u16(self.preview_model);
        self.preview_rot.write(w);
        w.write_f32(self.preview_zoom);
        w.write_i16(self.preview_colour1);
        w.write_i16(self.preview_colour2);
    }

    pub fn has_box(&self) -> bool {
        self.flags & textdraw_flags::BOX != 0
    }
    pub fn is_proportional(&self) -> bool {
        self.flags & textdraw_flags::PROPORTIONAL != 0
    }
}

/// RPC 134 PlayerShowTextDraw (S→C): create (or replace) a textdraw.
///
/// Spec: ui-chat-audio.md §134. Wire: u16 textDrawId, 63-byte
/// [`TextDrawData`] block, u16 text length, text.
///
/// Client behaviour: ids 0..2047 are global, 2048..2303 per-player; the
/// client does not bounds-check the id (hardened client: ignore ≥
/// [`TEXTDRAW_POOL_SIZE`]). Text length must be < 800, else ignored with a
/// debug warning. An existing id is destroyed first. Text validation (tilde
/// pairs, `~k~` in long strings) replaces bad text with an error string; that
/// is display behaviour, not applied here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerShowTextDraw {
    pub textdraw_id: u16,
    pub data: TextDrawData,
    pub text: Vec<u8>,
}

impl RpcPayload for PlayerShowTextDraw {
    const ID: u8 = 134;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { textdraw_id: r.read_u16()?, data: TextDrawData::read(r)?, text: read_str16(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.textdraw_id);
        self.data.write(w);
        write_str16(w, &self.text);
    }
}

/// RPC 135 PlayerHideTextDraw (S→C): destroy a textdraw ("hide" is a delete;
/// a later show must resend everything).
///
/// Spec: ui-chat-audio.md §135. Wire: u16 textDrawId. No id bound check in the
/// client (hardened client: ignore ≥ [`TEXTDRAW_POOL_SIZE`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerHideTextDraw {
    pub textdraw_id: u16,
}

impl RpcPayload for PlayerHideTextDraw {
    const ID: u8 = 135;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { textdraw_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.textdraw_id);
    }
}

/// RPC 105 PlayerTextDrawSetString (S→C): replace a textdraw's text.
///
/// Spec: ui-chat-audio.md §105. Wire: u16 textDrawId, u16 length, text.
///
/// Client behaviour: id must be < 0x900 and exist; the wire check accepts
/// length ≤ 800 but the setter ignores texts ≥ 800 (off-by-one in the
/// original) — treat ≥ [`MAX_TEXTDRAW_TEXT`] as invalid. Same text rules as §134.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerTextDrawSetString {
    pub textdraw_id: u16,
    pub text: Vec<u8>,
}

impl RpcPayload for PlayerTextDrawSetString {
    const ID: u8 = 105;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { textdraw_id: r.read_u16()?, text: read_str16(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.textdraw_id);
        write_str16(w, &self.text);
    }
}

/// RPC 83 PlayerBeginTextDrawSelect (S→C): SelectTextDraw / CancelSelectTextDraw.
///
/// Spec: ui-chat-audio.md §83 (S→C). Wire: a single **bit** `enable`, then
/// u32 hover colour (RGBA value `0xRRGGBBAA`). Nothing is byte-aligned.
///
/// Client behaviour: first cancels object selection/editing (which may send
/// RPC 116/117). enable = 1 → selection mode with the hover colour.
/// enable = 0 → *unconditionally* runs the disable path, which sends
/// [`OnPlayerSelectTextDraw`] with [`INVALID_TEXTDRAW`] even if selection
/// was not active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerBeginTextDrawSelect {
    pub enable: bool,
    /// RGBA colour value.
    pub hover_colour: u32,
}

impl RpcPayload for PlayerBeginTextDrawSelect {
    const ID: u8 = 83;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { enable: r.read_bit()?, hover_colour: r.read_u32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_bit(self.enable);
        w.write_u32(self.hover_colour);
    }
}

/// RPC 83 OnPlayerSelectTextDraw (C→S): textdraw clicked in select mode, or
/// [`INVALID_TEXTDRAW`] when selection was cancelled.
///
/// Spec: ui-chat-audio.md §83 (C→S). Wire: u16 textDrawId. Reliability
/// RELIABLE_ORDERED. Sent on left-button release over a hovered textdraw
/// (selection stays on), and with 0xFFFF on ESC, on server RPC 83 with
/// enable = 0, and when RPC 27/117 cancel an active selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnPlayerSelectTextDraw {
    pub textdraw_id: u16,
}

impl Default for OnPlayerSelectTextDraw {
    fn default() -> Self {
        Self { textdraw_id: INVALID_TEXTDRAW }
    }
}

impl RpcPayload for OnPlayerSelectTextDraw {
    const ID: u8 = 83;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { textdraw_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.textdraw_id);
    }
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// RPC 61 ShowDialog (S→C).
///
/// Spec: ui-chat-audio.md §61 and "Shared: compressed string". Wire: i16
/// dialogId, u8 style ([`dialog_style`]), u8-length title, u8-length button1,
/// u8-length button2, body as a Huffman-compressed string (decoded up to
/// [`DIALOG_BODY_MAX`] bytes).
///
/// Client behaviour: if the decoded title **or** body is empty the RPC is
/// ignored entirely (open.mp's hide therefore sends id -1 with single-space
/// strings). dialogId < 0 closes an open dialog without a response. The client
/// keeps the first 64 title chars; button2 empty → single button. Styles 2/4/5
/// split rows on `\n` (4/5 columns on `\t`, up to 4; 5 has a header row).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowDialog {
    pub dialog_id: i16,
    pub style: u8,
    pub title: Vec<u8>,
    pub button1: Vec<u8>,
    pub button2: Vec<u8>,
    pub body: Vec<u8>,
}

impl RpcPayload for ShowDialog {
    const ID: u8 = 61;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            dialog_id: r.read_i16()?,
            style: r.read_u8()?,
            title: read_str8(r)?,
            button1: read_str8(r)?,
            button2: read_str8(r)?,
            body: read_compressed_string(r, DIALOG_BODY_MAX)?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_i16(self.dialog_id);
        w.write_u8(self.style);
        write_str8(w, &self.title);
        write_str8(w, &self.button1);
        write_str8(w, &self.button2);
        write_compressed_string(w, &self.body);
    }
}

/// RPC 62 OnPlayerDialogResponse (C→S).
///
/// Spec: ui-chat-audio.md §62. Wire: u16 dialogId (low 16 bits of the id
/// from §61), u8 response, i16 listItem, u8 text length, text. Reliability
/// RELIABLE_ORDERED. open.mp reads the id as i16 (same bits); the client's
/// unsigned form is used here.
///
/// Values the client sends: response 1 = button1/Enter, 0 = button2/ESC;
/// listItem = selected data row for styles 2/4/5 (header excluded), else -1;
/// text = edit box (styles 1/3, ≤ 128 chars), the selected row's first column
/// with `{RRGGBB}` codes stripped (2/4/5, ≤ 128), or empty (style 0). The
/// client closes the dialog locally right after sending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnPlayerDialogResponse {
    pub dialog_id: u16,
    pub response: u8,
    pub list_item: i16,
    pub text: Vec<u8>,
}

impl Default for OnPlayerDialogResponse {
    fn default() -> Self {
        Self { dialog_id: 0, response: 0, list_item: -1, text: Vec::new() }
    }
}

impl RpcPayload for OnPlayerDialogResponse {
    const ID: u8 = 62;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { dialog_id: r.read_u16()?, response: r.read_u8()?, list_item: r.read_i16()?, text: read_str8(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.dialog_id);
        w.write_u8(self.response);
        w.write_i16(self.list_item);
        write_str8(w, &self.text);
    }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/// A fixed 32-byte, NUL-padded menu string (title, header or item).
pub type MenuText = [u8; MENU_TEXT_LEN];

/// Build a [`MenuText`] from bytes (truncated to 32, NUL-padded).
pub fn menu_text(s: &[u8]) -> MenuText {
    let mut a = [0u8; MENU_TEXT_LEN];
    let n = s.len().min(MENU_TEXT_LEN);
    a[..n].copy_from_slice(&s[..n]);
    a
}

/// The bytes of a [`MenuText`] up to the first NUL (all 32 if none).
pub fn menu_text_str(t: &MenuText) -> &[u8] {
    let n = t.iter().position(|&b| b == 0).unwrap_or(MENU_TEXT_LEN);
    &t[..n]
}

/// One menu column of [`PlayerInitMenu`]: fixed 32-byte header, u8 item
/// count, then that many fixed 32-byte items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuColumn {
    pub header: MenuText,
    /// At most [`MAX_MENU_ITEMS`] in a well-formed menu.
    pub items: Vec<MenuText>,
}

impl MenuColumn {
    /// Read header and items. Returns `(column, complete)`; `complete` is
    /// false when the item count exceeded 12, in which case the client stops
    /// parsing the RPC there (items are not read and `items` is empty).
    fn read(r: &mut BitReader) -> Option<(Self, bool)> {
        let header = read_menu_text(r)?;
        let count = r.read_u8()?;
        if usize::from(count) > MAX_MENU_ITEMS {
            return Some((Self { header, items: Vec::new() }, false));
        }
        let items = (0..count).map(|_| read_menu_text(r)).collect::<Option<Vec<_>>>()?;
        Some((Self { header, items }, true))
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_bytes(&self.header);
        let items = &self.items[..self.items.len().min(u8::MAX.into())];
        w.write_u8(items.len() as u8);
        for it in items {
            w.write_bytes(it);
        }
    }
}

/// RPC 76 PlayerInitMenu (S→C): define a GTA menu (not shown until §77).
///
/// Spec: ui-chat-audio.md §76. Wire: u8 menuId, u32 twoColumns, title[32],
/// f32 posX, f32 posY, f32 col1Width, f32 col2Width (**only if twoColumns ≠
/// 0**), u32 interactive, 12 × u32 rowEnabled, column 1 ([`MenuColumn`]),
/// column 2 (only if twoColumns ≠ 0).
///
/// Wire-parsing quirk mirrored by [`read`](RpcPayload::read): if a column's
/// item count is > 12, the client skips its items **and the whole second
/// column**; the decoder then returns that column with no items and
/// `column2 = None`, leaving the rest unread.
///
/// Client behaviour: columns = (low byte of twoColumns) + 1; strings accepted
/// if < 33 chars; an existing menu with the same id is replaced. The create
/// path does not range-check menuId, but show/hide ignore ids ≥ 128 — a
/// hardened client should reject ids ≥ [`MAX_MENUS`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInitMenu {
    pub menu_id: u8,
    /// Raw u32; any non-zero value means two columns on the wire.
    pub two_columns: u32,
    pub title: MenuText,
    pub position: Vec2,
    pub col1_width: f32,
    /// Only on the wire when `two_columns != 0` (decoded as 0.0 otherwise).
    pub col2_width: f32,
    /// 0 = display-only menu (no row can be selected).
    pub interactive: u32,
    pub row_enabled: [u32; MAX_MENU_ITEMS],
    pub column1: MenuColumn,
    /// Written only when `two_columns != 0` (as an empty column if `None`).
    pub column2: Option<MenuColumn>,
}

impl PlayerInitMenu {
    pub fn has_two_columns(&self) -> bool {
        self.two_columns != 0
    }
}

impl RpcPayload for PlayerInitMenu {
    const ID: u8 = 76;
    fn read(r: &mut BitReader) -> Option<Self> {
        let menu_id = r.read_u8()?;
        let two_columns = r.read_u32()?;
        let title = read_menu_text(r)?;
        let position = Vec2::read(r)?;
        let col1_width = r.read_f32()?;
        let col2_width = if two_columns != 0 { r.read_f32()? } else { 0.0 };
        let interactive = r.read_u32()?;
        let mut row_enabled = [0u32; MAX_MENU_ITEMS];
        for v in &mut row_enabled {
            *v = r.read_u32()?;
        }
        let (column1, complete) = MenuColumn::read(r)?;
        let column2 = if two_columns != 0 && complete {
            // An over-long second column is returned with no items; nothing follows it.
            Some(MenuColumn::read(r)?.0)
        } else {
            None
        };
        Some(Self {
            menu_id,
            two_columns,
            title,
            position,
            col1_width,
            col2_width,
            interactive,
            row_enabled,
            column1,
            column2,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.menu_id);
        w.write_u32(self.two_columns);
        w.write_bytes(&self.title);
        self.position.write(w);
        w.write_f32(self.col1_width);
        if self.two_columns != 0 {
            w.write_f32(self.col2_width);
        }
        w.write_u32(self.interactive);
        for &v in &self.row_enabled {
            w.write_u32(v);
        }
        self.column1.write(w);
        if self.two_columns != 0 {
            self.column2.clone().unwrap_or_default().write(w);
        }
    }
}

/// RPC 77 PlayerShowMenu (S→C).
///
/// Spec: ui-chat-audio.md §77. Wire: u8 menuId. Client: id < 128 and existing
/// → replaces any current GTA menu and builds this one; clears the
/// "awaiting server" flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerShowMenu {
    pub menu_id: u8,
}

impl RpcPayload for PlayerShowMenu {
    const ID: u8 = 77;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { menu_id: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.menu_id);
    }
}

/// RPC 78 PlayerHideMenu (S→C).
///
/// Spec: ui-chat-audio.md §78. Wire: u8 menuId. Client: if id < 128, a menu is
/// current and the slot exists, the *current* GTA menu is hidden — even when
/// menuId names a different existing menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerHideMenu {
    pub menu_id: u8,
}

impl RpcPayload for PlayerHideMenu {
    const ID: u8 = 78;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { menu_id: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.menu_id);
    }
}

/// RPC 132 OnPlayerSelectedMenuRow (C→S).
///
/// Spec: ui-chat-audio.md §132/§140. Wire: u8 row. Reliability RELIABLE.
/// Sent when the select button is newly pressed on an interactive menu and
/// the active row is not [`INVALID_MENU_ROW`]; the client hides the menu
/// itself one frame later regardless of the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerSelectedMenuRow {
    pub row: u8,
}

impl RpcPayload for OnPlayerSelectedMenuRow {
    const ID: u8 = 132;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { row: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.row);
    }
}

/// RPC 140 OnPlayerExitedMenu (C→S): empty payload.
///
/// Spec: ui-chat-audio.md §132/§140. Reliability RELIABLE. Sent when the
/// exit button is newly pressed while a menu is current; the client hides
/// the menu itself one frame later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerExitedMenu;

impl RpcPayload for OnPlayerExitedMenu {
    const ID: u8 = 140;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

// ---------------------------------------------------------------------------
// Audio stream
// ---------------------------------------------------------------------------

/// RPC 41 PlayAudioStreamForPlayer (S→C): start a BASS internet audio stream.
///
/// Spec: ui-chat-audio.md §41. Wire: u8-length URL, f32 x, y, z, f32
/// distance, u8 usePos.
///
/// Client behaviour: ignored if audio is not initialised; stops any current
/// stream first; nothing plays if GTA's radio volume is 0. URL kept up to 256
/// chars, no scheme validation (a hardened client may whitelist). Prints
/// "Audio stream: <url>" unless `audiomsgoff`. Volume: usePos = 1 → radio
/// volume × 8000 × (1 − d/distance) within `distance` of `position`, else 0;
/// usePos = 0 → radio volume × 7000 (formula: medium confidence).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayAudioStreamForPlayer {
    pub url: Vec<u8>,
    pub position: Vec3,
    pub distance: f32,
    /// Raw byte; 0 = global stream, 1 = positional.
    pub use_pos: u8,
}

impl RpcPayload for PlayAudioStreamForPlayer {
    const ID: u8 = 41;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { url: read_str8(r)?, position: Vec3::read(r)?, distance: r.read_f32()?, use_pos: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        write_str8(w, &self.url);
        self.position.write(w);
        w.write_f32(self.distance);
        w.write_u8(self.use_pos);
    }
}

/// RPC 42 StopAudioStreamForPlayer (S→C): empty payload.
///
/// Spec: ui-chat-audio.md §42. Client: if a stream is playing, signal the
/// worker to stop and free the BASS stream immediately (no wait).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StopAudioStreamForPlayer;

impl RpcPayload for StopAudioStreamForPlayer {
    const ID: u8 = 42;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Every S→C payload of this module.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    PlayAudioStreamForPlayer(PlayAudioStreamForPlayer),
    StopAudioStreamForPlayer(StopAudioStreamForPlayer),
    ShowDialog(ShowDialog),
    SendGameText(SendGameText),
    PlayerInitMenu(PlayerInitMenu),
    PlayerShowMenu(PlayerShowMenu),
    PlayerHideMenu(PlayerHideMenu),
    PlayerBeginTextDrawSelect(PlayerBeginTextDrawSelect),
    SendClientMessage(SendClientMessage),
    PlayerChatMessage(PlayerChatMessage),
    PlayerTextDrawSetString(PlayerTextDrawSetString),
    PlayerShowTextDraw(PlayerShowTextDraw),
    PlayerHideTextDraw(PlayerHideTextDraw),
}

/// S→C RPC ids handled by [`decode`].
pub const INCOMING_IDS: &[u8] = &[41, 42, 61, 73, 76, 77, 78, 83, 93, 101, 105, 134, 135];

/// Decode an S→C payload of this module; `None` for other ids or malformed data.
pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
    Some(match id {
        PlayAudioStreamForPlayer::ID => Incoming::PlayAudioStreamForPlayer(RpcPayload::read(r)?),
        StopAudioStreamForPlayer::ID => Incoming::StopAudioStreamForPlayer(RpcPayload::read(r)?),
        ShowDialog::ID => Incoming::ShowDialog(RpcPayload::read(r)?),
        SendGameText::ID => Incoming::SendGameText(RpcPayload::read(r)?),
        PlayerInitMenu::ID => Incoming::PlayerInitMenu(RpcPayload::read(r)?),
        PlayerShowMenu::ID => Incoming::PlayerShowMenu(RpcPayload::read(r)?),
        PlayerHideMenu::ID => Incoming::PlayerHideMenu(RpcPayload::read(r)?),
        PlayerBeginTextDrawSelect::ID => Incoming::PlayerBeginTextDrawSelect(RpcPayload::read(r)?),
        SendClientMessage::ID => Incoming::SendClientMessage(RpcPayload::read(r)?),
        PlayerChatMessage::ID => Incoming::PlayerChatMessage(RpcPayload::read(r)?),
        PlayerTextDrawSetString::ID => Incoming::PlayerTextDrawSetString(RpcPayload::read(r)?),
        PlayerShowTextDraw::ID => Incoming::PlayerShowTextDraw(RpcPayload::read(r)?),
        PlayerHideTextDraw::ID => Incoming::PlayerHideTextDraw(RpcPayload::read(r)?),
        _ => return None,
    })
}

impl Incoming {
    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::PlayAudioStreamForPlayer(_) => PlayAudioStreamForPlayer::ID,
            Self::StopAudioStreamForPlayer(_) => StopAudioStreamForPlayer::ID,
            Self::ShowDialog(_) => ShowDialog::ID,
            Self::SendGameText(_) => SendGameText::ID,
            Self::PlayerInitMenu(_) => PlayerInitMenu::ID,
            Self::PlayerShowMenu(_) => PlayerShowMenu::ID,
            Self::PlayerHideMenu(_) => PlayerHideMenu::ID,
            Self::PlayerBeginTextDrawSelect(_) => PlayerBeginTextDrawSelect::ID,
            Self::SendClientMessage(_) => SendClientMessage::ID,
            Self::PlayerChatMessage(_) => PlayerChatMessage::ID,
            Self::PlayerTextDrawSetString(_) => PlayerTextDrawSetString::ID,
            Self::PlayerShowTextDraw(_) => PlayerShowTextDraw::ID,
            Self::PlayerHideTextDraw(_) => PlayerHideTextDraw::ID,
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::PlayAudioStreamForPlayer(p) => p.write(w),
            Self::StopAudioStreamForPlayer(p) => p.write(w),
            Self::ShowDialog(p) => p.write(w),
            Self::SendGameText(p) => p.write(w),
            Self::PlayerInitMenu(p) => p.write(w),
            Self::PlayerShowMenu(p) => p.write(w),
            Self::PlayerHideMenu(p) => p.write(w),
            Self::PlayerBeginTextDrawSelect(p) => p.write(w),
            Self::SendClientMessage(p) => p.write(w),
            Self::PlayerChatMessage(p) => p.write(w),
            Self::PlayerTextDrawSetString(p) => p.write(w),
            Self::PlayerShowTextDraw(p) => p.write(w),
            Self::PlayerHideTextDraw(p) => p.write(w),
        }
    }
}

// ---------------------------------------------------------------------------
// Outgoing (C→S) send parameters
// ---------------------------------------------------------------------------

outgoing! {
    Ui;
    /// RPC 101: sender A 0x10005a10 / B 0x10005a00, reliability 8 (RELIABLE),
    /// priority 1, channel 0 (ui-chat-audio.md §101 and its C→S preamble).
    /// Static analysis; not yet captured.
    PlayerRequestChatMessage => Reliable;
    /// RPC 50: sender A 0x10069900 / B 0x100698c0, reliability 8, priority 1,
    /// channel 0 (ui-chat-audio.md §50). Static analysis; not yet captured.
    PlayerRequestCommandMessage => Reliable;
    /// RPC 83: sender A 0x100714e0 / B 0x100714b0, reliability 9
    /// (RELIABLE_ORDERED), priority 1, channel 0 (ui-chat-audio.md §83).
    /// Static analysis; not yet captured.
    OnPlayerSelectTextDraw => ReliableOrdered;
    /// RPC 62: sender A 0x10070690 / B 0x10070660, reliability 9, priority 1,
    /// channel 0 (ui-chat-audio.md §62). Static analysis; not yet captured.
    OnPlayerDialogResponse => ReliableOrdered;
    /// RPC 132: menu poll A 0x10008200 / B 0x100081f0, reliability 8,
    /// priority 1, channel 0 (ui-chat-audio.md §132/§140). Static analysis;
    /// not yet captured.
    OnPlayerSelectedMenuRow => Reliable;
    /// RPC 140: menu poll A 0x10008200 / B 0x100081f0, reliability 8,
    /// priority 1, channel 0 (ui-chat-audio.md §132/§140). Static analysis;
    /// not yet captured.
    OnPlayerExitedMenu => Reliable;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;

    fn encode<T: RpcPayload>(v: &T) -> BitWriter {
        let mut w = BitWriter::new();
        v.write(&mut w);
        w
    }

    /// Write, read back, compare, and check every bit was consumed.
    fn roundtrip<T: RpcPayload + PartialEq + Debug>(v: &T) {
        let w = encode(v);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        let back = T::read(&mut r).expect("decode failed");
        assert_eq!(&back, v);
        assert_eq!(r.remaining(), 0, "unread bits");
    }

    /// Check the writer against hand-built bytes and the reader on them.
    fn exact<T: RpcPayload + PartialEq + Debug>(v: &T, bytes: &[u8], bits: usize) {
        let w = encode(v);
        assert_eq!(w.bit_len(), bits, "bit length");
        assert_eq!(w.as_bytes(), bytes, "bytes");
        let mut r = BitReader::with_bit_len(bytes, bits);
        assert_eq!(&T::read(&mut r).expect("decode failed"), v);
        assert_eq!(r.remaining(), 0);
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    // ---- chat ----

    #[test]
    fn send_client_message_exact() {
        let v = SendClientMessage { colour: 0xFF00_00AA, message: b"hi".to_vec() };
        let b = cat(&[&[0xAA, 0x00, 0x00, 0xFF], &[2, 0, 0, 0], b"hi"]);
        exact(&v, &b, b.len() * 8);
    }

    #[test]
    fn send_client_message_huge_length_is_none() {
        // Length 0xFFFFFFFF with 2 bytes of data must fail without allocating.
        let b = cat(&[&[0, 0, 0, 0], &[0xFF; 4], b"ab"]);
        assert_eq!(SendClientMessage::read(&mut BitReader::new(&b)), None);
    }

    #[test]
    fn player_chat_message_exact() {
        let v = PlayerChatMessage { player_id: 0x0102, message: b"yo".to_vec() };
        exact(&v, &[0x02, 0x01, 2, b'y', b'o'], 40);
        roundtrip(&PlayerChatMessage { player_id: 1004, message: vec![b'x'; 144] });
    }

    #[test]
    fn player_request_chat_message_exact() {
        exact(&PlayerRequestChatMessage { message: b"abc".to_vec() }, &[3, b'a', b'b', b'c'], 32);
        roundtrip(&PlayerRequestChatMessage { message: vec![] });
    }

    #[test]
    fn player_request_command_message_exact() {
        let v = PlayerRequestCommandMessage { command: b"/help me".to_vec() };
        let b = cat(&[&[8, 0, 0, 0], b"/help me"]);
        exact(&v, &b, b.len() * 8);
    }

    // ---- game text ----

    #[test]
    fn send_game_text_exact() {
        let v = SendGameText { style: 3, time_ms: 5000, text: b"~r~X".to_vec() };
        let b = cat(&[&[3, 0, 0, 0], &[0x88, 0x13, 0, 0], &[4, 0, 0, 0], b"~r~X"]);
        exact(&v, &b, b.len() * 8);
        // Signed style survives the u32 wire form.
        roundtrip(&SendGameText { style: -1, time_ms: 0, text: b"a".to_vec() });
    }

    // ---- textdraws ----

    fn sample_td() -> TextDrawData {
        TextDrawData {
            flags: textdraw_flags::BOX | textdraw_flags::ALIGN_CENTRE | textdraw_flags::PROPORTIONAL,
            letter_size: Vec2 { x: 0.5, y: 1.0 },
            letter_colour: 0xFF00_00FF,
            text_size: Vec2 { x: 640.0, y: 20.0 },
            box_colour: 0x8000_0000,
            shadow: 1,
            outline: 2,
            background_colour: 0xFF11_2233,
            style: 5,
            selectable: 1,
            position: Vec2 { x: 320.0, y: 224.0 },
            preview_model: 411,
            preview_rot: Vec3 { x: -10.0, y: 0.0, z: 45.0 },
            preview_zoom: 1.5,
            preview_colour1: -1,
            preview_colour2: 3,
        }
    }

    #[test]
    fn show_textdraw_exact() {
        let v = PlayerShowTextDraw { textdraw_id: 2050, data: sample_td(), text: b"Hi".to_vec() };
        // Hand-built from the §134 table (byte offsets of the 63-byte block).
        let mut block = Vec::new();
        block.push(0b0001_1001); // 0: flags box | centre | proportional
        block.extend(0.5f32.to_le_bytes()); // 1
        block.extend(1.0f32.to_le_bytes()); // 5
        block.extend([0xFF, 0x00, 0x00, 0xFF]); // 9 letter colour
        block.extend(640.0f32.to_le_bytes()); // 13
        block.extend(20.0f32.to_le_bytes()); // 17
        block.extend([0x00, 0x00, 0x00, 0x80]); // 21 box colour
        block.push(1); // 25 shadow
        block.push(2); // 26 outline
        block.extend([0x33, 0x22, 0x11, 0xFF]); // 27 background colour
        block.push(5); // 31 style
        block.push(1); // 32 selectable
        block.extend(320.0f32.to_le_bytes()); // 33
        block.extend(224.0f32.to_le_bytes()); // 37
        block.extend([0x9B, 0x01]); // 41 model 411
        block.extend((-10.0f32).to_le_bytes()); // 43
        block.extend(0.0f32.to_le_bytes()); // 47
        block.extend(45.0f32.to_le_bytes()); // 51
        block.extend(1.5f32.to_le_bytes()); // 55
        block.extend([0xFF, 0xFF]); // 59 colour1 = -1
        block.extend([0x03, 0x00]); // 61 colour2 = 3
        assert_eq!(block.len(), TextDrawData::WIRE_LEN);
        let b = cat(&[&[0x02, 0x08], &block, &[2, 0], b"Hi"]);
        exact(&v, &b, b.len() * 8);
        let d = v.data;
        assert!(d.has_box() && d.is_proportional());
    }

    #[test]
    fn show_textdraw_roundtrip_default() {
        roundtrip(&PlayerShowTextDraw::default());
    }

    #[test]
    fn hide_textdraw_exact() {
        exact(&PlayerHideTextDraw { textdraw_id: 0x08FF }, &[0xFF, 0x08], 16);
    }

    #[test]
    fn textdraw_set_string_exact() {
        let v = PlayerTextDrawSetString { textdraw_id: 7, text: b"abc".to_vec() };
        exact(&v, &[7, 0, 3, 0, b'a', b'b', b'c'], 56);
        roundtrip(&PlayerTextDrawSetString { textdraw_id: 2303, text: vec![b'z'; 800] });
    }

    #[test]
    fn begin_textdraw_select_is_bit_packed() {
        // 1 | 0x44 0x33 0x22 0x11 shifted by one bit.
        let v = PlayerBeginTextDrawSelect { enable: true, hover_colour: 0x1122_3344 };
        exact(&v, &[0xA2, 0x19, 0x91, 0x08, 0x80], 33);
        let v = PlayerBeginTextDrawSelect { enable: false, hover_colour: 0xFFFF_FFFF };
        exact(&v, &[0x7F, 0xFF, 0xFF, 0xFF, 0x80], 33);
    }

    #[test]
    fn select_textdraw_exact() {
        exact(&OnPlayerSelectTextDraw::default(), &[0xFF, 0xFF], 16);
        exact(&OnPlayerSelectTextDraw { textdraw_id: 2048 }, &[0x00, 0x08], 16);
    }

    // ---- dialogs ----

    #[test]
    fn dialog_response_exact() {
        let v = OnPlayerDialogResponse { dialog_id: 0x1234, response: 1, list_item: -1, text: b"pw".to_vec() };
        exact(&v, &[0x34, 0x12, 1, 0xFF, 0xFF, 2, b'p', b'w'], 64);
        // Empty text: only the zero length byte follows.
        let v = OnPlayerDialogResponse { dialog_id: 5, response: 0, list_item: 3, text: vec![] };
        exact(&v, &[5, 0, 0, 3, 0, 0], 48);
    }

    #[test]
    fn show_dialog_truncated_header_is_none() {
        // id, style, title "T" and button1 "OK" but no button2 / body.
        let b = [0x01, 0x00, 2, 1, b'T', 2, b'O', b'K'];
        assert_eq!(ShowDialog::read(&mut BitReader::new(&b)), None);
    }

    #[test]
    fn show_dialog_roundtrip() {
        roundtrip(&ShowDialog {
            dialog_id: 7,
            style: dialog_style::LIST,
            title: b"Pick".to_vec(),
            button1: b"OK".to_vec(),
            button2: b"Cancel".to_vec(),
            body: b"one\ntwo\nthree".to_vec(),
        });
        roundtrip(&ShowDialog {
            dialog_id: -1,
            style: 0,
            title: b" ".to_vec(),
            button1: b" ".to_vec(),
            button2: b" ".to_vec(),
            body: b" ".to_vec(),
        });
    }

    #[test]
    fn show_dialog_header_exact() {
        let v = ShowDialog {
            dialog_id: -2,
            style: dialog_style::INPUT,
            title: b"T".to_vec(),
            button1: b"OK".to_vec(),
            button2: vec![],
            body: b"x".to_vec(),
        };
        let w = encode(&v);
        // Header bytes before the compressed body.
        assert_eq!(&w.as_bytes()[..8], &[0xFE, 0xFF, 1, 1, b'T', 2, b'O', b'K']);
        assert_eq!(w.as_bytes()[8], 0);
    }

    // ---- menus ----

    fn one_col_menu() -> PlayerInitMenu {
        PlayerInitMenu {
            menu_id: 3,
            two_columns: 0,
            title: menu_text(b"Shop"),
            position: Vec2 { x: 200.0, y: 100.0 },
            col1_width: 150.0,
            col2_width: 0.0,
            interactive: 1,
            row_enabled: [1; MAX_MENU_ITEMS],
            column1: MenuColumn { header: menu_text(b"Item"), items: vec![menu_text(b"A"), menu_text(b"B")] },
            column2: None,
        }
    }

    #[test]
    fn init_menu_one_column_exact() {
        let v = one_col_menu();
        let mut b = vec![3];
        b.extend([0, 0, 0, 0]);
        b.extend(menu_text(b"Shop"));
        b.extend(200.0f32.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(150.0f32.to_le_bytes());
        // No col2 width.
        b.extend([1, 0, 0, 0]);
        for _ in 0..12 {
            b.extend([1, 0, 0, 0]);
        }
        b.extend(menu_text(b"Item"));
        b.push(2);
        b.extend(menu_text(b"A"));
        b.extend(menu_text(b"B"));
        // No second column.
        assert_eq!(b.len(), 1 + 4 + 32 + 12 + 52 + 32 + 1 + 64);
        exact(&v, &b, b.len() * 8);
    }

    #[test]
    fn init_menu_two_columns_exact() {
        let mut v = one_col_menu();
        v.two_columns = 1;
        v.col2_width = 80.0;
        v.column2 = Some(MenuColumn { header: menu_text(b"Price"), items: vec![menu_text(b"$1"), menu_text(b"$2")] });
        let mut b = vec![3];
        b.extend([1, 0, 0, 0]);
        b.extend(menu_text(b"Shop"));
        b.extend(200.0f32.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(150.0f32.to_le_bytes());
        b.extend(80.0f32.to_le_bytes());
        b.extend([1, 0, 0, 0]);
        for _ in 0..12 {
            b.extend([1, 0, 0, 0]);
        }
        b.extend(menu_text(b"Item"));
        b.push(2);
        b.extend(menu_text(b"A"));
        b.extend(menu_text(b"B"));
        b.extend(menu_text(b"Price"));
        b.push(2);
        b.extend(menu_text(b"$1"));
        b.extend(menu_text(b"$2"));
        exact(&v, &b, b.len() * 8);
        assert!(v.has_two_columns());
    }

    #[test]
    fn init_menu_overlong_column_stops_parsing() {
        let mut v = one_col_menu();
        v.two_columns = 1;
        v.column1.items = vec![menu_text(b"x"); 13];
        v.column2 = Some(MenuColumn { header: menu_text(b"H2"), items: vec![] });
        let w = encode(&v);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        let got = PlayerInitMenu::read(&mut r).unwrap();
        assert!(got.column1.items.is_empty());
        assert_eq!(menu_text_str(&got.column1.header), b"Item");
        assert_eq!(got.column2, None);
        // 13 items + second column left unread.
        assert_eq!(r.remaining(), (13 * 32 + 33) * 8);
    }

    #[test]
    fn menu_text_helpers() {
        let t = menu_text(&[b'a'; 40]);
        assert_eq!(menu_text_str(&t).len(), 32);
        assert_eq!(menu_text_str(&menu_text(b"ab")), b"ab");
    }

    #[test]
    fn show_hide_menu_exact() {
        exact(&PlayerShowMenu { menu_id: 127 }, &[127], 8);
        exact(&PlayerHideMenu { menu_id: 5 }, &[5], 8);
    }

    #[test]
    fn menu_row_and_exit_exact() {
        exact(&OnPlayerSelectedMenuRow { row: 11 }, &[11], 8);
        exact(&OnPlayerExitedMenu, &[], 0);
    }

    // ---- audio ----

    #[test]
    fn play_audio_stream_exact() {
        let v = PlayAudioStreamForPlayer {
            url: b"http://a/b".to_vec(),
            position: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            distance: 50.0,
            use_pos: 1,
        };
        let b = cat(&[
            &[10],
            b"http://a/b",
            &1.0f32.to_le_bytes(),
            &2.0f32.to_le_bytes(),
            &3.0f32.to_le_bytes(),
            &50.0f32.to_le_bytes(),
            &[1],
        ]);
        exact(&v, &b, b.len() * 8);
    }

    #[test]
    fn stop_audio_stream_is_empty() {
        exact(&StopAudioStreamForPlayer, &[], 0);
    }

    // ---- dispatch ----

    #[test]
    fn decode_dispatch() {
        let w = encode(&SendClientMessage { colour: 1, message: b"m".to_vec() });
        let got = decode(93, &mut BitReader::with_bit_len(w.as_bytes(), w.bit_len()));
        assert_eq!(got, Some(Incoming::SendClientMessage(SendClientMessage { colour: 1, message: b"m".to_vec() })));

        let w = encode(&PlayerBeginTextDrawSelect { enable: true, hover_colour: 2 });
        assert!(matches!(
            decode(83, &mut BitReader::with_bit_len(w.as_bytes(), w.bit_len())),
            Some(Incoming::PlayerBeginTextDrawSelect(PlayerBeginTextDrawSelect { enable: true, hover_colour: 2 }))
        ));

        // Not ours / C→S-only ids.
        for id in [50u8, 62, 132, 140, 30, 59, 111] {
            assert_eq!(decode(id, &mut BitReader::new(&[0u8; 64])), None, "id {id}");
        }
    }

    #[test]
    fn decode_every_incoming_id() {
        // Enough zero bytes for every payload (an all-zero cstr is empty).
        let zeros = [0u8; 256];
        for &id in INCOMING_IDS {
            assert!(decode(id, &mut BitReader::new(&zeros)).is_some(), "id {id}");
        }
    }

    #[test]
    fn decode_truncated_is_none() {
        assert_eq!(decode(134, &mut BitReader::new(&[0u8; 10])), None);
        assert_eq!(decode(76, &mut BitReader::new(&[0u8; 40])), None);
        assert_eq!(decode(41, &mut BitReader::new(&[3, b'a'])), None);
    }

    #[test]
    fn to_rpc_frames_payload() {
        let msg = PlayerRequestChatMessage { message: b"hi".to_vec() }.to_rpc();
        let rpc = crate::rpc::decode(msg.as_bytes(), msg.bit_len()).unwrap();
        assert_eq!((rpc.id, rpc.bit_len), (101, 24));
        assert_eq!(PlayerRequestChatMessage::read(&mut rpc.reader()).unwrap().message, b"hi");
    }
}
