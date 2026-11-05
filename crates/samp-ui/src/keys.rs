//! Window-message routing for the UI keys (UR §6.2–6.3) and the per-frame Present
//! plan (UR §2.3). Pure decisions: the host translates Windows messages into
//! [`Event`]s and executes the returned [`Action`]s on the models of this crate.

use crate::Rect;
use crate::chat::DisplayMode;
use crate::dialog::{Button, ListKey};

/// Virtual keys the UI cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vk {
    Escape,
    Enter,
    PageUp,
    PageDown,
    Up,
    Down,
    Tab,
    F6,
    F7,
    F8,
    F9,
    NumpadMinus,
    Other(u32),
}

impl Vk {
    /// From a Windows virtual-key code.
    pub fn from_code(vk: u32) -> Self {
        match vk {
            0x1B => Self::Escape,
            0x0D => Self::Enter,
            0x21 => Self::PageUp,
            0x22 => Self::PageDown,
            0x26 => Self::Up,
            0x28 => Self::Down,
            0x09 => Self::Tab,
            0x75 => Self::F6,
            0x76 => Self::F7,
            0x77 => Self::F8,
            0x78 => Self::F9,
            0x6D => Self::NumpadMinus,
            o => Self::Other(o),
        }
    }
}

/// Input events (one per window message).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `WM_CHAR`.
    Char(u8),
    KeyDown(Vk),
    KeyUp(Vk),
    /// `WM_SYSKEYUP`.
    SysKeyUp(Vk),
    /// Mouse wheel, in notches (positive = away from the user).
    Wheel(i32),
    /// Left button released at client coordinates.
    LeftButtonUp {
        x: i32,
        y: i32,
    },
    KillFocus,
}

/// UI state the routing depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    /// Textdraw selection mode on.
    pub textdraw_select: bool,
    /// The open dialog is a list (styles 2, 4, 5).
    pub dialog_is_list: bool,
    pub input_exists: bool,
    pub input_open: bool,
    pub input_rect: Rect,
    pub dialog_open: bool,
    /// The open dialog has an edit box (styles 1, 3).
    pub dialog_has_edit: bool,
    pub scoreboard_exists: bool,
    pub scoreboard_open: bool,
    pub scoreboard_rect: Rect,
    pub class_selection_visible: bool,
    /// Slider panel, object edit/select or textdraw selection active.
    pub other_modal: bool,
    pub gta_menu: bool,
    pub ime_composing: bool,
    pub netgame: bool,
    pub chat_mode: DisplayMode,
    pub debug_mode: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            textdraw_select: false,
            dialog_is_list: false,
            input_exists: true,
            input_open: false,
            input_rect: Rect::default(),
            dialog_open: false,
            dialog_has_edit: false,
            scoreboard_exists: true,
            scoreboard_open: false,
            scoreboard_rect: Rect::default(),
            class_selection_visible: false,
            other_modal: false,
            gta_menu: false,
            ime_composing: false,
            netgame: true,
            chat_mode: DisplayMode::Outlined,
            debug_mode: false,
        }
    }
}

/// What the host should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenInput,
    CloseInput,
    /// Character for the chat input edit box.
    InputChar(u8),
    /// Enter in the chat input edit box: run [`crate::input::ChatInput::submit`].
    SubmitInput,
    HistoryOlder,
    HistoryNewer,
    /// Character for the dialog edit box.
    DialogChar(u8),
    /// List navigation in a list dialog (key-down, DXUT list box).
    DialogListKey(ListKey),
    /// ESC while textdraw selection is on: the cancel path (RPC 83 0xFFFF).
    CancelTextDrawSelect,
    DialogResponse(Button),
    ToggleScoreboard,
    CloseScoreboard,
    ChatPageUp,
    ChatPageDown,
    /// Scroll the chat by `−(notches × SPI_GETWHEELSCROLLLINES)` lines.
    ChatWheel(i32),
    CycleChatMode,
    /// Set the screenshot flag (taken at the next Present).
    Screenshot,
    ToggleDeathWindow,
    /// `/rs` with an empty comment (debug mode only).
    RawSaveEmpty,
    /// Alt+Enter: toggle windowed mode of the current video mode.
    ToggleWindowed,
    /// Re-apply cursor mode 0 with no delay (focus loss during class selection).
    CursorModeZeroImmediate,
    /// Run the GTA input-restore step.
    InputRestore,
}

/// Routing result: actions, and whether the message is consumed (not passed to GTA).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Routed {
    pub actions: Vec<Action>,
    pub consumed: bool,
}

impl Routed {
    fn one(a: Action) -> Self {
        Self { actions: vec![a], consumed: true }
    }
    fn none() -> Self {
        Self::default()
    }
}

/// Routes one event (UR §6.2 order, §6.3 table).
pub fn route(ev: Event, s: &UiState) -> Routed {
    match ev {
        Event::Char(0x1B) => {
            if s.input_open {
                Routed::one(Action::CloseInput)
            } else if s.scoreboard_open {
                Routed::one(Action::CloseScoreboard)
            } else if s.dialog_open {
                Routed::one(Action::DialogResponse(Button::Cancel))
            } else if s.textdraw_select {
                Routed::one(Action::CancelTextDrawSelect)
            } else {
                Routed::none()
            }
        }
        // samp's edit box ignores a typed CR (`WM_CHAR` 0x0D): Enter acts on its key-up only (INPUT-1),
        // so neither key auto-repeat nor the character of the Enter that sent a chat line can
        // answer a dialog.
        Event::Char(b'\r') if s.input_open => Routed { actions: Vec::new(), consumed: true },
        Event::Char(c) if s.input_open => Routed::one(Action::InputChar(c)),
        Event::Char(b'\r') if s.dialog_open && !s.scoreboard_open && !s.gta_menu => {
            Routed { actions: Vec::new(), consumed: s.dialog_has_edit }
        }
        Event::Char(c) if s.dialog_open && !s.scoreboard_open && !s.gta_menu => {
            if s.dialog_has_edit { Routed::one(Action::DialogChar(c)) } else { Routed::none() }
        }
        Event::Char(b'T' | b't' | b'`')
            if s.input_exists && !s.input_open && !s.dialog_open && !s.gta_menu && !s.scoreboard_open =>
        {
            Routed::one(Action::OpenInput)
        }
        Event::Char(_) => Routed::none(),
        Event::KeyDown(Vk::Escape) => Routed {
            actions: Vec::new(),
            consumed: s.input_open
                || s.scoreboard_open
                || s.dialog_open
                || s.class_selection_visible
                || s.other_modal
                || s.textdraw_select,
        },
        // TRACE-3 U-EDITKEY-CONSUME: with the input open no key-down reaches GTA.
        Event::KeyDown(_) if s.input_open => Routed { actions: Vec::new(), consumed: true },
        Event::KeyDown(k) if s.dialog_open && !s.scoreboard_open && !s.gta_menu => {
            let lk = match k {
                Vk::Up => Some(ListKey::Up),
                Vk::Down => Some(ListKey::Down),
                Vk::PageUp => Some(ListKey::PageUp),
                Vk::PageDown => Some(ListKey::PageDown),
                Vk::Other(0x24) => Some(ListKey::Home),
                Vk::Other(0x23) => Some(ListKey::End),
                _ => None,
            };
            match lk {
                Some(l) if s.dialog_is_list => Routed::one(Action::DialogListKey(l)),
                _ => Routed { actions: Vec::new(), consumed: s.dialog_has_edit },
            }
        }
        Event::KeyDown(_) => Routed::none(),
        // TRACE-3 U-EDITKEY-CONSUME: every key-up reaches GTA (actions still run).
        Event::KeyUp(k) => Routed { consumed: false, ..key_up(k, s) },
        Event::SysKeyUp(Vk::Enter) => Routed::one(Action::ToggleWindowed),
        Event::SysKeyUp(_) => Routed::none(),
        Event::Wheel(n) if s.input_open && s.chat_mode.chat_visible() => Routed::one(Action::ChatWheel(n)),
        Event::Wheel(_) => Routed::none(),
        Event::LeftButtonUp { x, y } => {
            let mut r = Routed::none();
            if s.scoreboard_open && !s.scoreboard_rect.contains(x, y) {
                r.actions.push(Action::CloseScoreboard);
            }
            if s.input_open && !s.other_modal && !s.input_rect.contains(x, y) {
                r.actions.push(Action::CloseInput);
            }
            r
        }
        Event::KillFocus => {
            let mut actions = vec![Action::CloseScoreboard, Action::CloseInput];
            if s.class_selection_visible {
                actions.push(Action::CursorModeZeroImmediate);
            }
            actions.push(Action::InputRestore);
            Routed { actions, consumed: false }
        }
    }
}

fn key_up(k: Vk, s: &UiState) -> Routed {
    match k {
        // Unknown (notes U-F6): no condition besides the input existing.
        Vk::F6 if s.input_exists => Routed::one(if s.input_open { Action::CloseInput } else { Action::OpenInput }),
        Vk::Escape if s.input_open && !s.ime_composing => Routed::one(Action::CloseInput),
        // INPUT-1: samp's edit box raises its "string" event on the Enter key-up (A 0x10089380),
        // so the chat line is sent on the key-up, never on the key-down or the typed CR. A dialog the
        // server shows in answer arrives after the key-up and stays open.
        Vk::Enter if s.input_open => Routed::one(Action::SubmitInput),
        // Dialog edit box (same key-up event) or, for the other styles, the window procedure's key-up.
        Vk::Enter if s.dialog_open => Routed::one(Action::DialogResponse(Button::Ok)),
        Vk::PageUp | Vk::PageDown if s.netgame && !s.ime_composing && !s.gta_menu => {
            Routed::one(if k == Vk::PageUp { Action::ChatPageUp } else { Action::ChatPageDown })
        }
        Vk::Up if s.input_open && !s.ime_composing => Routed::one(Action::HistoryOlder),
        Vk::Down if s.input_open && !s.ime_composing => Routed::one(Action::HistoryNewer),
        Vk::F7 => Routed::one(Action::CycleChatMode),
        Vk::F8 => Routed::one(Action::Screenshot),
        Vk::F9 => Routed::one(Action::ToggleDeathWindow),
        Vk::NumpadMinus if s.debug_mode => Routed::one(Action::RawSaveEmpty),
        // Opening also needs the class selection to be inactive (Scoreboard::toggle).
        Vk::Tab if s.scoreboard_exists && !s.gta_menu => Routed::one(Action::ToggleScoreboard),
        _ => Routed::none(),
    }
}

/// Inputs of the per-frame Present plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInput {
    /// GTA pause menu active (0xBA67A4 ≠ 0).
    pub gta_menu: bool,
    pub netgame: bool,
    /// Netgame state 5 (connected).
    pub connected: bool,
    pub scoreboard_open: bool,
    pub dialog_open: bool,
    pub f1_held: bool,
    pub f5_held: bool,
    pub f10_held: bool,
    pub chat_mode: DisplayMode,
    /// Local ped field +0x2c9 > 5000 (meaning unknown, UR §21 item 6).
    pub ped_field_over_5000: bool,
}

/// Which exclusive UI the frame shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// GTA menu: only the map right-click handler (waypoint → RPC 119) runs.
    GtaMenu,
    /// Scoreboard only (no chat, dialogs, bubbles, death window).
    Scoreboard,
    /// F5 net stats only.
    NetStats,
    /// F10 held: nothing drawn (its draw is empty).
    F10,
    /// Normal: bubbles, 3D labels, chat, input, death window, dialog, GUI.
    World,
}

/// Per-frame decisions of the D3D pass (UR §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePlan {
    pub overlay: Overlay,
    /// Nametag bars/names, audio title and `/dl` labels (step 4).
    pub nametags: bool,
    /// HUD + radar on/off (`None` = left unchanged, GTA menu frames).
    pub hud_on: Option<bool>,
    /// F1: open the help dialog this frame.
    pub open_help: bool,
}

/// Computes the Present plan.
pub fn frame_plan(f: &FrameInput) -> FramePlan {
    if f.gta_menu {
        return FramePlan { overlay: Overlay::GtaMenu, nametags: false, hud_on: None, open_help: false };
    }
    let nametags = f.netgame;
    let (overlay, hud_on, open_help) = if f.scoreboard_open {
        (Overlay::Scoreboard, false, false)
    } else if f.f5_held && f.connected {
        (Overlay::NetStats, false, false)
    } else if f.f10_held {
        (Overlay::F10, false, false)
    } else {
        let hud = f.chat_mode.hud_visible() && !f.ped_field_over_5000;
        (Overlay::World, hud, f.f1_held && !f.dialog_open)
    };
    FramePlan { overlay, nametags, hud_on: Some(hud_on), open_help }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> UiState {
        UiState::default()
    }

    #[test]
    fn ur_6_3_t_opens_input_only_when_free() {
        assert_eq!(route(Event::Char(b't'), &st()).actions, vec![Action::OpenInput]);
        assert_eq!(route(Event::Char(b'`'), &st()).actions, vec![Action::OpenInput]);
        for s in [
            UiState { dialog_open: true, ..st() },
            UiState { gta_menu: true, ..st() },
            UiState { scoreboard_open: true, ..st() },
            UiState { input_exists: false, ..st() },
        ] {
            assert!(!route(Event::Char(b'T'), &s).actions.contains(&Action::OpenInput));
        }
        // With the input open the character is typed.
        let s = UiState { input_open: true, ..st() };
        assert_eq!(route(Event::Char(b't'), &s).actions, vec![Action::InputChar(b't')]);
        // INPUT-1: the typed CR does nothing; the Enter key-up sends the line.
        let r = route(Event::Char(b'\r'), &s);
        assert!(r.actions.is_empty() && r.consumed);
        assert_eq!(route(Event::KeyUp(Vk::Enter), &s).actions, vec![Action::SubmitInput]);
    }

    #[test]
    fn ur_6_3_esc_char_priority() {
        let all = UiState { input_open: true, scoreboard_open: true, dialog_open: true, ..st() };
        assert_eq!(route(Event::Char(0x1B), &all).actions, vec![Action::CloseInput]);
        let s = UiState { input_open: false, ..all };
        assert_eq!(route(Event::Char(0x1B), &s).actions, vec![Action::CloseScoreboard]);
        let s = UiState { scoreboard_open: false, ..s };
        assert_eq!(route(Event::Char(0x1B), &s).actions, vec![Action::DialogResponse(Button::Cancel)]);
    }

    #[test]
    fn ur_6_3_esc_keydown_swallowed_when_ui_active() {
        assert!(!route(Event::KeyDown(Vk::Escape), &st()).consumed);
        for s in [
            UiState { input_open: true, ..st() },
            UiState { dialog_open: true, ..st() },
            UiState { class_selection_visible: true, ..st() },
            UiState { other_modal: true, ..st() },
        ] {
            assert!(route(Event::KeyDown(Vk::Escape), &s).consumed);
        }
        let s = UiState { input_open: true, ..st() };
        assert_eq!(route(Event::KeyUp(Vk::Escape), &s).actions, vec![Action::CloseInput]);
        let s = UiState { ime_composing: true, ..s };
        assert!(route(Event::KeyUp(Vk::Escape), &s).actions.is_empty());
    }

    #[test]
    fn ur_6_3_function_keys_and_paging() {
        assert_eq!(route(Event::KeyUp(Vk::F6), &st()).actions, vec![Action::OpenInput]);
        let open = UiState { input_open: true, ..st() };
        assert_eq!(route(Event::KeyUp(Vk::F6), &open).actions, vec![Action::CloseInput]);
        assert_eq!(route(Event::KeyUp(Vk::F7), &st()).actions, vec![Action::CycleChatMode]);
        assert_eq!(route(Event::KeyUp(Vk::F8), &st()).actions, vec![Action::Screenshot]);
        assert_eq!(route(Event::KeyUp(Vk::F9), &st()).actions, vec![Action::ToggleDeathWindow]);
        assert!(route(Event::KeyUp(Vk::NumpadMinus), &st()).actions.is_empty());
        let dbg = UiState { debug_mode: true, ..st() };
        assert_eq!(route(Event::KeyUp(Vk::NumpadMinus), &dbg).actions, vec![Action::RawSaveEmpty]);
        assert_eq!(route(Event::KeyUp(Vk::PageUp), &st()).actions, vec![Action::ChatPageUp]);
        assert!(route(Event::KeyUp(Vk::PageDown), &UiState { gta_menu: true, ..st() }).actions.is_empty());
        assert!(route(Event::KeyUp(Vk::PageDown), &UiState { ime_composing: true, ..st() }).actions.is_empty());
        assert_eq!(route(Event::KeyUp(Vk::Up), &open).actions, vec![Action::HistoryOlder]);
        assert_eq!(route(Event::KeyUp(Vk::Down), &open).actions, vec![Action::HistoryNewer]);
        assert!(route(Event::KeyUp(Vk::Up), &st()).actions.is_empty());
        assert_eq!(route(Event::KeyUp(Vk::Tab), &st()).actions, vec![Action::ToggleScoreboard]);
        assert_eq!(route(Event::SysKeyUp(Vk::Enter), &st()).actions, vec![Action::ToggleWindowed]);
        assert_eq!(Vk::from_code(0x76), Vk::F7);
    }

    #[test]
    fn ur_6_3_enter_and_dialog_chars() {
        let d = UiState { dialog_open: true, ..st() };
        assert_eq!(route(Event::KeyUp(Vk::Enter), &d).actions, vec![Action::DialogResponse(Button::Ok)]);
        let both = UiState { input_open: true, ..d };
        // TRACE-3 U-EDITKEY-CONSUME: key-ups always reach GTA; key-downs not while typing.
        let r = route(Event::KeyUp(Vk::Enter), &both);
        assert!(r.actions == vec![Action::SubmitInput] && !r.consumed);
        // INPUT-1: a typed CR never answers a dialog (only the Enter key-up does).
        assert!(route(Event::Char(b'\r'), &d).actions.is_empty());
        let de = UiState { dialog_has_edit: true, ..d };
        let r = route(Event::Char(b'\r'), &de);
        assert!(r.actions.is_empty() && r.consumed);
        assert!(route(Event::KeyDown(Vk::Other(0x41)), &both).consumed);
        let list = UiState { dialog_is_list: true, ..d };
        assert_eq!(route(Event::KeyDown(Vk::Down), &list).actions, vec![Action::DialogListKey(ListKey::Down)]);
        let sel = UiState { textdraw_select: true, ..st() };
        assert_eq!(route(Event::Char(0x1B), &sel).actions, vec![Action::CancelTextDrawSelect]);
        assert_eq!(route(Event::Char(b'x'), &de).actions, vec![Action::DialogChar(b'x')]);
        assert!(route(Event::Char(b't'), &d).actions.is_empty());
    }

    #[test]
    fn ur_6_3_wheel_clicks_focus() {
        let open = UiState { input_open: true, input_rect: Rect::xywh(40, 170, 700, 40), ..st() };
        assert_eq!(route(Event::Wheel(2), &open).actions, vec![Action::ChatWheel(2)]);
        assert!(route(Event::Wheel(2), &st()).actions.is_empty());
        assert!(route(Event::Wheel(2), &UiState { chat_mode: DisplayMode::Hidden, ..open }).actions.is_empty());
        assert!(route(Event::LeftButtonUp { x: 50, y: 180 }, &open).actions.is_empty());
        assert_eq!(route(Event::LeftButtonUp { x: 5, y: 5 }, &open).actions, vec![Action::CloseInput]);
        let sb = UiState { scoreboard_open: true, scoreboard_rect: Rect::xywh(100, 100, 640, 480), ..st() };
        assert_eq!(route(Event::LeftButtonUp { x: 5, y: 5 }, &sb).actions, vec![Action::CloseScoreboard]);
        let r = route(Event::KillFocus, &UiState { class_selection_visible: true, ..st() });
        assert_eq!(
            r.actions,
            vec![Action::CloseScoreboard, Action::CloseInput, Action::CursorModeZeroImmediate, Action::InputRestore]
        );
    }

    #[test]
    fn ur_2_3_frame_plan() {
        let base = FrameInput {
            gta_menu: false,
            netgame: true,
            connected: true,
            scoreboard_open: false,
            dialog_open: false,
            f1_held: false,
            f5_held: false,
            f10_held: false,
            chat_mode: DisplayMode::Outlined,
            ped_field_over_5000: false,
        };
        assert_eq!(
            frame_plan(&base),
            FramePlan { overlay: Overlay::World, nametags: true, hud_on: Some(true), open_help: false }
        );
        assert_eq!(frame_plan(&FrameInput { gta_menu: true, ..base }).overlay, Overlay::GtaMenu);
        let p = frame_plan(&FrameInput { scoreboard_open: true, f5_held: true, ..base });
        assert_eq!((p.overlay, p.hud_on), (Overlay::Scoreboard, Some(false)));
        assert_eq!(frame_plan(&FrameInput { f5_held: true, ..base }).overlay, Overlay::NetStats);
        assert_eq!(frame_plan(&FrameInput { f5_held: true, connected: false, ..base }).overlay, Overlay::World);
        assert_eq!(frame_plan(&FrameInput { f10_held: true, ..base }).overlay, Overlay::F10);
        assert_eq!(frame_plan(&FrameInput { chat_mode: DisplayMode::Hidden, ..base }).hud_on, Some(false));
        assert_eq!(frame_plan(&FrameInput { ped_field_over_5000: true, ..base }).hud_on, Some(false));
        assert!(frame_plan(&FrameInput { f1_held: true, ..base }).open_help);
        assert!(!frame_plan(&FrameInput { f1_held: true, dialog_open: true, ..base }).open_help);
    }
}
