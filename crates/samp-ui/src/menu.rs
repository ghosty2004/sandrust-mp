//! Menus (RPC 76/77/78, 132/140): data, the GXT keys samp hands to GTA's
//! `CMenuSystem` and the per-frame selection poll (UR §12; UCA §76–§78, §132/§140).
//!
//! Static analysis of samp.dll (A build) gave the exact script sequence of the menu
//! build (A 0x100a7780) and the `CText::Get` hook (A 0x100a4070 → A 0x10008060):
//! - 0x08D4 CREATE_MENU(title key, x, y, (col1W + col2W) / columns, columns,
//!   interactive, 1, 1) → handle;
//! - 0x08DB SET_MENU_COLUMN(handle, 0, header key, 12 row keys), 0x09DB
//!   SET_MENU_COLUMN_WIDTH(handle, 0, (int) col1W); for two columns the same with
//!   column 1, where samp also passes **col1W** as the width (observable quirk);
//! - 0x08D9 ACTIVATE_MENU_ITEM(handle, row, enabled) for rows 0..11.
//!
//! Keys: title `SAMPHED`, headers `SAMPRW1`/`SAMPRW2`, cells `SAMP0rr` / `SAMP1rr`
//! (rr = 00..11, key table A 0x100fe0c8), `DUMMY` for empty strings. The hook answers a
//! key starting with `SAMP` from the current menu; anything else goes to GTA.

use samp_proto::rpcs::ui::{MAX_MENU_ITEMS, PlayerInitMenu, menu_text_str};

/// Menu ids are 0..127; "no current menu" is 0x80.
pub const MAX_MENUS: usize = 128;
pub const DUMMY: &str = "DUMMY";
pub const TITLE_KEY: &str = "SAMPHED";
pub const HEADER_KEYS: [&str; 2] = ["SAMPRW1", "SAMPRW2"];
/// `SAMP0rr` / `SAMP1rr`.
pub const CELL_KEYS: [[&str; MAX_MENU_ITEMS]; 2] = [
    [
        "SAMP000", "SAMP001", "SAMP002", "SAMP003", "SAMP004", "SAMP005", "SAMP006", "SAMP007", "SAMP008", "SAMP009",
        "SAMP010", "SAMP011",
    ],
    [
        "SAMP100", "SAMP101", "SAMP102", "SAMP103", "SAMP104", "SAMP105", "SAMP106", "SAMP107", "SAMP108", "SAMP109",
        "SAMP110", "SAMP111",
    ],
];
/// "No row" from the active-row query (also for display-only menus).
pub const NO_ROW: u8 = 0xFF;

/// One menu (A 0x100a7460 object).
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: Vec<u8>,
    pub x: f32,
    pub y: f32,
    pub col_widths: [f32; 2],
    pub columns: u8,
    pub interactive: bool,
    pub row_enabled: [bool; MAX_MENU_ITEMS],
    pub headers: [Vec<u8>; 2],
    /// `cells[column][row]`.
    pub cells: [[Vec<u8>; MAX_MENU_ITEMS]; 2],
}

fn s32(t: &[u8; 32]) -> Vec<u8> {
    menu_text_str(t).to_vec()
}

impl Menu {
    pub fn from_rpc(m: &PlayerInitMenu) -> Self {
        let mut cells: [[Vec<u8>; MAX_MENU_ITEMS]; 2] = Default::default();
        for (r, it) in m.column1.items.iter().take(MAX_MENU_ITEMS).enumerate() {
            cells[0][r] = s32(it);
        }
        let mut headers = [s32(&m.column1.header), Vec::new()];
        if let Some(c2) = &m.column2 {
            headers[1] = s32(&c2.header);
            for (r, it) in c2.items.iter().take(MAX_MENU_ITEMS).enumerate() {
                cells[1][r] = s32(it);
            }
        }
        Self {
            title: s32(&m.title),
            x: m.position.x,
            y: m.position.y,
            col_widths: [m.col1_width, m.col2_width],
            columns: (m.two_columns as u8).wrapping_add(1),
            interactive: m.interactive != 0,
            row_enabled: m.row_enabled.map(|v| v != 0),
            headers,
            cells,
        }
    }

    /// GXT key for a string: the samp key, or `DUMMY` when the string is empty.
    fn key(text: &[u8], key: &'static str) -> &'static str {
        if text.is_empty() { DUMMY } else { key }
    }

    /// The script commands that build the GTA menu, in order (A 0x100a7780).
    pub fn build(&self) -> MenuBuild {
        let width = (self.col_widths[1] + self.col_widths[0]) / f32::from(self.columns.max(1));
        let mut columns = Vec::new();
        for c in 0..usize::from(self.columns.min(2)) {
            let rows = std::array::from_fn(|r| Self::key(&self.cells[c][r], CELL_KEYS[c][r]));
            // Both columns get col1W as their width (samp passes +0x384 twice).
            columns.push(MenuColumnBuild {
                column: c as u8,
                header: Self::key(&self.headers[c], HEADER_KEYS[c]),
                rows,
                width: self.col_widths[0] as i32,
            });
        }
        MenuBuild {
            title: Self::key(&self.title, TITLE_KEY),
            x: self.x,
            y: self.y,
            width,
            columns: i32::from(self.columns),
            interactive: i32::from(self.interactive),
            column_builds: columns,
            rows_enabled: self.row_enabled.map(i32::from),
        }
    }

    /// The text for the part of a `SAMP…` key after `SAMP` (A 0x10008060).
    pub fn gxt(&self, rest: &[u8]) -> Option<&[u8]> {
        let rest = crate::text::c_str(rest);
        match rest {
            b"HED" => return Some(&self.title),
            b"RW1" => return Some(&self.headers[0]),
            b"RW2" => return Some(&self.headers[1]),
            _ => {}
        }
        (0..MAX_MENU_ITEMS)
            .flat_map(|r| (0..2).map(move |c| (r, c)))
            .find(|&(r, c)| rest == &CELL_KEYS[c][r].as_bytes()[4..])
            .map(|(r, c)| self.cells[c][r].as_slice())
    }
}

/// One `SET_MENU_COLUMN` + `SET_MENU_COLUMN_WIDTH` pair.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuColumnBuild {
    pub column: u8,
    pub header: &'static str,
    pub rows: [&'static str; MAX_MENU_ITEMS],
    pub width: i32,
}

/// The whole build sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuBuild {
    pub title: &'static str,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub columns: i32,
    pub interactive: i32,
    pub column_builds: Vec<MenuColumnBuild>,
    pub rows_enabled: [i32; MAX_MENU_ITEMS],
}

/// What the per-frame poll asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollAction {
    /// Send RPC 132 with this row.
    Selected(u8),
    /// Send RPC 140.
    Exited,
    /// Delete the GTA menu locally (one frame after a selection/exit).
    HideLocal,
}

/// The menu pool (player-pool sub-object +0x20).
#[derive(Clone, Debug, Default)]
pub struct Menus {
    menus: Vec<Option<Menu>>,
    current: Option<u8>,
    awaiting: bool,
}

impl Menus {
    pub fn new() -> Self {
        Self { menus: vec![None; MAX_MENUS], current: None, awaiting: false }
    }

    /// RPC 76: replaces the menu (ids ≥ 128 rejected, hardened).
    pub fn init(&mut self, m: &PlayerInitMenu) -> bool {
        let Some(slot) = self.menus.get_mut(usize::from(m.menu_id)) else { return false };
        *slot = Some(Menu::from_rpc(m));
        true
    }

    pub fn get(&self, id: u8) -> Option<&Menu> {
        self.menus.get(usize::from(id)).and_then(|m| m.as_ref())
    }

    pub fn current(&self) -> Option<u8> {
        self.current
    }

    pub fn current_menu(&self) -> Option<&Menu> {
        self.current.and_then(|i| self.get(i))
    }

    /// RPC 77: returns `(delete the previous GTA menu, build this one)`.
    pub fn show(&mut self, id: u8) -> Option<(bool, MenuBuild)> {
        let build = self.get(id)?.build();
        let had = self.current.is_some();
        self.current = Some(id);
        self.awaiting = false;
        Some((had, build))
    }

    /// RPC 78: hides the *current* menu if `id` is a valid existing menu. Returns
    /// whether the GTA menu must be deleted.
    pub fn hide(&mut self, id: u8) -> bool {
        if usize::from(id) >= MAX_MENUS || self.current.is_none() || self.get(id).is_none() {
            return false;
        }
        self.current = None;
        true
    }

    /// Per-frame poll (A 0x10008200). `cross`/`triangle` = just pressed (new ≠ 0, old = 0);
    /// `active_row` = GET_MENU_ITEM_SELECTED (only asked for interactive menus).
    pub fn poll(&mut self, cross: bool, triangle: bool, active_row: impl FnOnce() -> u8) -> Option<PollAction> {
        let menu = self.current_menu()?;
        if cross {
            let row = if menu.interactive { active_row() } else { NO_ROW };
            if row != NO_ROW {
                self.awaiting = true;
                return Some(PollAction::Selected(row));
            }
            None
        } else if triangle {
            self.awaiting = true;
            Some(PollAction::Exited)
        } else if self.awaiting {
            self.awaiting = false;
            self.current = None;
            Some(PollAction::HideLocal)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.menus.iter_mut().for_each(|m| *m = None);
        self.current = None;
        self.awaiting = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samp_proto::rpcs::ui::{MenuColumn, menu_text};
    use samp_proto::types::Vec2;

    fn rpc(two: bool) -> PlayerInitMenu {
        let col = |h: &[u8], items: &[&[u8]]| MenuColumn {
            header: menu_text(h),
            items: items.iter().map(|i| menu_text(i)).collect(),
        };
        PlayerInitMenu {
            menu_id: 3,
            two_columns: u32::from(two),
            title: menu_text(b"Shop"),
            position: Vec2 { x: 30.0, y: 140.0 },
            col1_width: 150.0,
            col2_width: if two { 100.0 } else { 0.0 },
            interactive: 1,
            row_enabled: [1; 12],
            column1: col(b"Item", &[b"Apple", b"Pear"]),
            column2: two.then(|| col(b"", &[b"$1"])),
        }
    }

    #[test]
    fn build_sequence_and_keys() {
        let m = Menu::from_rpc(&rpc(true));
        let b = m.build();
        assert_eq!((b.title, b.columns, b.width), ("SAMPHED", 2, 125.0));
        assert_eq!(b.column_builds.len(), 2);
        assert_eq!(b.column_builds[0].header, "SAMPRW1");
        assert_eq!(b.column_builds[1].header, DUMMY);
        assert_eq!(&b.column_builds[0].rows[..3], &["SAMP000", "SAMP001", DUMMY]);
        assert_eq!(&b.column_builds[1].rows[..2], &["SAMP100", DUMMY]);
        assert_eq!(b.column_builds[1].width, 150);
        assert_eq!(m.gxt(b"HED"), Some(&b"Shop"[..]));
        assert_eq!(m.gxt(b"001"), Some(&b"Pear"[..]));
        assert_eq!(m.gxt(b"100"), Some(&b"$1"[..]));
        assert_eq!(m.gxt(b"XYZ"), None);
        let one = Menu::from_rpc(&rpc(false)).build();
        assert_eq!((one.columns, one.width, one.column_builds.len()), (1, 150.0, 1));
    }

    #[test]
    fn show_hide_and_poll() {
        let mut ms = Menus::new();
        assert!(ms.init(&rpc(false)));
        assert!(ms.show(9).is_none());
        let (had, _) = ms.show(3).unwrap();
        assert!(!had);
        assert_eq!(ms.poll(false, false, || 0), None);
        assert_eq!(ms.poll(true, false, || 1), Some(PollAction::Selected(1)));
        assert_eq!(ms.poll(false, false, || 0), Some(PollAction::HideLocal));
        assert_eq!(ms.current(), None);
        ms.show(3);
        assert_eq!(ms.poll(false, true, || 0), Some(PollAction::Exited));
        ms.show(3);
        assert!(ms.hide(3));
        assert!(!ms.hide(3));
    }
}
