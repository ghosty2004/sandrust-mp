//! Platform-independent UI state of the in-game client (milestone M4): the chat
//! window, the chat input, local commands and `sa-mp.cfg`, built on `samp-ui`.
//!
//! The Windows glue (`ui::glue`) owns one [`UiCore`], feeds it RPCs, window
//! messages and control-channel requests, and drains what it asks the host to do
//! ([`Outgoing`] network messages, [`HostRequests`]). Everything here is
//! host-testable.
//!
//! Specs: `docs/re/subsystems/ui-rendering.md` (RE-5, "UR §n"),
//! `docs/re/subsystems/ui-chat-audio.md` ("UCA"), `docs/phase3/m4-notes.md`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use samp_proto::rpcs::{Incoming, player, remote, ui};
use samp_ui::chat::{ChatEntry, ChatWindow, ClientMessageResult, DisplayMode, EntryType, LocalTime};
use samp_ui::colour;
use samp_ui::commands::{self, Effect, GameInfo, MessageKind};
use samp_ui::config::{self, SampCfg, UiSettings};
use samp_ui::edit::{CharResult, EditKey, Mods};
use samp_ui::gametext::GameText;
use samp_ui::input::{ChatInput, Submit};
use samp_ui::keys::{self, Action, Event, UiState};
use samp_ui::text::KeyNameExpander;

use super::colours::PlayerColours;

/// Client-local strings (samp.dll 2021 .rdata; all printed through the type-8
/// helper A 0x100680f0, verified from the call after each string push).
pub mod msg {
    /// A 0x100e596c, printed when the netgame object is created (A 0x1000bafe).
    pub const STARTED: &str = "{FFFFFF}SA-MP {B9C9BF}0.3.7-R4 {FFFFFF}Started";
    /// A 0x100e599c, the connect tick (A 0x1000896d).
    pub fn connecting(host: &str, port: u16) -> String {
        format!("Connecting to {host}:{port}...")
    }
    /// A 0x100e5b98, on the `0x22` connection-accepted packet (A 0x1000ae70).
    pub const JOINING: &str = "Connected. Joining the game...";
    /// A 0x100e6060 `Connected to {B9C9BF}%.64s`, on InitGame (A 0x10010a87).
    pub fn connected_to(server: &[u8]) -> Vec<u8> {
        let mut v = b"Connected to {B9C9BF}".to_vec();
        v.extend_from_slice(&server[..server.len().min(64)]);
        v
    }
    /// A 0x100e5a80.
    pub const SERVER_CLOSED: &str = "Server closed the connection.";
}

/// Chat scroll position set by Enter in the input (A 0x10067390).
const SUBMIT_SCROLL_POS: u32 = 90;

/// A message the UI wants sent (drained by the network session every frame).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    /// RPC 101 PlayerRequestChatMessage.
    Chat(Vec<u8>),
    /// RPC 50 PlayerRequestCommandMessage (the whole typed line).
    Command(Vec<u8>),
    /// Packet 201 PlayerRconCommand.
    Rcon(Vec<u8>),
}

/// Host-side requests collected while handling input or commands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostRequests {
    /// `/q`: quit (disconnect first when true).
    pub quit: Option<bool>,
    /// `/fontsize`: rebuild the font renderer fonts and chat metrics.
    pub rebuild_fonts: bool,
    /// RPC 73 accepted: (style, time ms, text) for `CMessages::AddBigMessage`.
    pub game_text: Option<(u8, u32, Vec<u8>)>,
    /// Focus loss: run the input-restore step (UR §6.3 `WM_KILLFOCUS`).
    pub input_restore: bool,
}

/// One accepted game text, for the control channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownGameText {
    pub style: u8,
    pub time_ms: u32,
    pub text: Vec<u8>,
}

/// The UI state of the client.
pub struct UiCore {
    pub chat: ChatWindow,
    pub input: ChatInput,
    pub cfg: SampCfg,
    pub settings: UiSettings,
    user_dir: Option<PathBuf>,
    chatlog: Option<PathBuf>,
    /// Remote player names by id (RPC 137/138/11), for RPC 101.
    names: BTreeMap<u16, Vec<u8>>,
    local_name: Vec<u8>,
    local_id: Option<u16>,
    pub colours: PlayerColours,
    /// A netgame object exists (client started with `-c`).
    pub netgame: bool,
    /// Netgame state 5 (InitGame received).
    pub connected: bool,
    /// Started with `-d`.
    pub debug_mode: bool,
    outgoing: Vec<Outgoing>,
    pub requests: HostRequests,
    pub last_game_text: Option<ShownGameText>,
}

/// Local wall-clock time for chat entries.
pub fn local_time_now() -> LocalTime {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: plain Win32 query into a valid struct.
        unsafe { GetLocalTime(&mut st) };
        let unix =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        LocalTime { unix, hour: st.wHour as u8, minute: st.wMinute as u8, second: st.wSecond as u8 }
    }
    #[cfg(not(windows))]
    {
        let unix =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let day = unix.rem_euclid(86_400);
        LocalTime { unix, hour: (day / 3600) as u8, minute: (day / 60 % 60) as u8, second: (day % 60) as u8 }
    }
}

/// What a C stream opened in text mode writes for `b`: every `\n` becomes `\r\n`.
/// chatlog.txt and savedpositions.txt are written that way (the original's files
/// end their lines in `\r\r\n` and `\r\n`).
pub fn text_mode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 4);
    for &c in b {
        if c == b'\n' {
            out.push(b'\r');
        }
        out.push(c);
    }
    out
}

/// Escapes a byte string for one control-channel line: `"` and `\` escaped,
/// non-printable bytes as `\xNN`.
pub fn quote(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() + 2);
    s.push('"');
    for &c in b {
        match c {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7E => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s.push('"');
    s
}

impl UiCore {
    /// Creates the UI. `user_dir` is samp's user files directory (`<GTA user
    /// files>\SAMP`); `None` keeps everything in memory (tests).
    pub fn new(user_dir: Option<PathBuf>, debug_mode: bool) -> Self {
        let mut cfg = SampCfg::default();
        let mut cfg_dirty = false;
        if let Some(dir) = &user_dir {
            if let Ok(bytes) = std::fs::read(dir.join(config::FILE_NAME)) {
                cfg = SampCfg::parse(&bytes);
            }
            // UR §9 / U-CFG-STARTUP: fpslimit and audioproxyoff are created if missing.
            cfg_dirty = cfg.apply_startup_defaults();
        }
        let settings = UiSettings::from_cfg(&cfg);
        let mut chat = ChatWindow::new();
        chat.set_page_size(settings.page_size);
        chat.set_timestamps(settings.timestamps);
        // UR §7.1: chatlog.txt is truncated (mode "w") at construction; logging is on
        // if that succeeded.
        let chatlog = user_dir.as_ref().map(|d| d.join(samp_ui::chat::CHATLOG_FILE));
        let chatlog = chatlog.filter(|p| std::fs::File::create(p).is_ok());
        chat.set_logging(chatlog.is_some());
        let mut ui = Self {
            chat,
            input: ChatInput::new(),
            cfg,
            settings,
            user_dir,
            chatlog,
            names: BTreeMap::new(),
            local_name: Vec::new(),
            local_id: None,
            colours: PlayerColours::new(),
            netgame: false,
            connected: false,
            debug_mode,
            outgoing: Vec::new(),
            requests: HostRequests::default(),
            last_game_text: None,
        };
        if cfg_dirty {
            ui.write_config();
        }
        ui
    }

    pub fn user_dir(&self) -> Option<&PathBuf> {
        self.user_dir.as_ref()
    }

    pub fn cfg_path(&self) -> Option<PathBuf> {
        self.user_dir.as_ref().map(|d| d.join(config::FILE_NAME))
    }

    fn write_config(&mut self) {
        if let Some(p) = self.cfg_path()
            && let Err(e) = std::fs::write(&p, self.cfg.to_bytes())
        {
            crate::log!("ui: cannot write {}: {e}", p.display());
        }
    }

    fn log_lines(&self, lines: impl IntoIterator<Item = Vec<u8>>) {
        let Some(p) = &self.chatlog else { return };
        let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(p) else { return };
        for l in lines {
            let _ = f.write_all(&text_mode(&l));
        }
    }

    // ---- printing --------------------------------------------------------------------

    /// Type-4 line in the info colour (A 0x10068070).
    pub fn print_info(&mut self, text: &[u8]) {
        let l = self.chat.add_info(text, local_time_now());
        self.log_lines(l);
    }

    /// Type-8 line in the debug colour (A 0x100680f0).
    pub fn print_debug(&mut self, text: &[u8]) {
        let l = self.chat.add_debug(text, local_time_now());
        self.log_lines(l);
    }

    fn print(&mut self, kind: MessageKind, text: &[u8]) {
        match kind {
            MessageKind::Info => self.print_info(text),
            MessageKind::Debug => self.print_debug(text),
        }
    }

    // ---- session -----------------------------------------------------------------

    /// The netgame object is created (connect started): "Started", then the
    /// first "Connecting to" line.
    pub fn on_netgame_start(&mut self, host: &str, port: u16, name: &[u8]) {
        self.netgame = true;
        self.local_name = name.to_vec();
        self.print_debug(msg::STARTED.as_bytes());
        self.print_debug(msg::connecting(host, port).as_bytes());
    }

    /// `0x22` accepted: PlayerConnect is being sent.
    pub fn on_joining(&mut self) {
        self.print_debug(msg::JOINING.as_bytes());
    }

    /// RPC 139 InitGame: netgame state 5.
    pub fn on_init_game(&mut self, server_name: &[u8], player_id: u16) {
        self.connected = true;
        self.local_id = Some(player_id);
        self.print_debug(&msg::connected_to(server_name));
    }

    /// The session ended.
    pub fn on_closed(&mut self) {
        if self.connected {
            self.print_debug(msg::SERVER_CLOSED.as_bytes());
        }
        self.connected = false;
        self.names.clear();
    }

    // ---- RPCs --------------------------------------------------------------------

    /// Every decoded S→C RPC passes through here (before the game handlers).
    pub fn on_rpc(&mut self, r: &Incoming, expander: &dyn KeyNameExpander) {
        match r {
            Incoming::Ui(ui::Incoming::SendClientMessage(m)) => {
                if let ClientMessageResult::Added(l) =
                    self.chat.add_client_message(m.colour, &m.message, expander, local_time_now())
                {
                    self.log_lines(l);
                }
            }
            Incoming::Ui(ui::Incoming::PlayerChatMessage(m)) => self.on_player_chat(m.player_id, &m.message),
            Incoming::Ui(ui::Incoming::SendGameText(g)) => match GameText::from_rpc(g) {
                GameText::Show { style, time_ms, text } => {
                    self.last_game_text = Some(ShownGameText { style: style as u8, time_ms, text: text.clone() });
                    self.requests.game_text = Some((style as u8, time_ms, text));
                }
                other => crate::log!("ui: game text ignored ({other:?})"),
            },
            Incoming::Remote(remote::Incoming::PlayerJoin(j)) => {
                if j.name.len() <= samp_ui::chat::MAX_PREFIX_LEN && j.player_id < 1005 {
                    self.names.insert(j.player_id, j.name.clone());
                    // remote-players-sync.md §137: colour != 0 and id < 1004 sets it.
                    if j.colour != 0 && j.player_id < 1004 {
                        self.colours.set(j.player_id, j.colour);
                    }
                }
            }
            Incoming::Remote(remote::Incoming::PlayerQuit(q)) => {
                self.names.remove(&q.player_id);
            }
            Incoming::Remote(remote::Incoming::SetPlayerName(n)) => {
                if Some(n.player_id) == self.local_id {
                    self.local_name = n.name.clone();
                } else if let Some(e) = self.names.get_mut(&n.player_id) {
                    *e = n.name.clone();
                }
            }
            // player-state.md §72: local id, or a remote id with an entry.
            Incoming::Player(player::Incoming::SetPlayerColor(c))
                if Some(c.player_id) == self.local_id || self.names.contains_key(&c.player_id) =>
            {
                self.colours.set(c.player_id, c.colour);
            }
            _ => {}
        }
    }

    /// RPC 101 S→C (UCA §101): only in netgame state 5; local id → local name,
    /// else an active remote slot (< 1005).
    fn on_player_chat(&mut self, id: u16, text: &[u8]) {
        if !self.connected {
            return;
        }
        let name = if Some(id) == self.local_id {
            self.local_name.clone()
        } else if id < 1005
            && let Some(n) = self.names.get(&id)
        {
            n.clone()
        } else {
            return;
        };
        let colour = self.colours.chat_argb(id);
        let l = self.chat.add_player_chat(&name, colour, text, local_time_now());
        self.log_lines(l);
    }

    // ---- input -------------------------------------------------------------------

    /// Routing state for `samp_ui::keys::route`.
    pub fn key_state(&self, gta_menu: bool) -> UiState {
        UiState {
            input_exists: self.netgame,
            input_open: self.input.is_open(),
            input_rect: samp_ui::Rect::default(),
            dialog_open: false,
            dialog_has_edit: false,
            scoreboard_exists: false,
            scoreboard_open: false,
            scoreboard_rect: samp_ui::Rect::default(),
            class_selection_visible: false,
            other_modal: false,
            gta_menu,
            ime_composing: false,
            netgame: self.netgame,
            chat_mode: self.chat.mode(),
            debug_mode: self.debug_mode,
        }
    }

    /// Applies routed actions.
    pub fn apply(&mut self, actions: &[Action], game: &dyn GameInfo, wheel_lines: u32) {
        for &a in actions {
            match a {
                Action::OpenInput => {
                    self.input.open();
                }
                Action::CloseInput => self.input.close(),
                Action::InputChar(c) => match self.input.char(c) {
                    CharResult::Enter => self.submit(game),
                    CharResult::Edited | CharResult::Ignored | CharResult::Copy(_) => {}
                },
                Action::SubmitInput => self.submit(game),
                Action::HistoryOlder => self.input.history_older(),
                Action::HistoryNewer => self.input.history_newer(),
                Action::ChatPageUp => self.chat.page_up(),
                Action::ChatPageDown => self.chat.page_down(),
                Action::ChatWheel(n) => self.chat.wheel(n, wheel_lines),
                Action::CycleChatMode => {
                    self.chat.cycle_mode();
                }
                Action::InputRestore => self.requests.input_restore = true,
                // No scoreboard, dialog or class-selection GUI before M8a.
                Action::CloseScoreboard | Action::CursorModeZeroImmediate => {}
                other => crate::log!("ui: action {other:?} not handled in M4"),
            }
        }
    }

    /// Edit-box key-down while the input is open (DXUT edit box; UR §8).
    pub fn edit_key(&mut self, key: EditKey, mods: Mods) {
        if self.input.is_open() {
            let _ = self.input.key(key, mods);
        }
    }

    /// Routes one window event; returns whether it is consumed.
    pub fn handle_event(&mut self, ev: Event, gta_menu: bool, game: &dyn GameInfo, wheel_lines: u32) -> bool {
        let r = keys::route(ev, &self.key_state(gta_menu));
        self.apply(&r.actions, game, wheel_lines);
        r.consumed
    }

    /// Enter in the chat input (UCA "Local chat input").
    pub fn submit(&mut self, game: &dyn GameInfo) {
        let was_open = self.input.is_open();
        let r = self.input.submit(self.netgame, self.debug_mode);
        if was_open {
            // A 0x10067390: chat scroll position 90, clamped by the scrollbar.
            self.chat.scroll_to(SUBMIT_SCROLL_POS);
        }
        match r {
            Submit::Empty | Submit::Nothing => {}
            Submit::Chat(c) => self.outgoing.push(Outgoing::Chat(c.message)),
            Submit::Command(c) => self.outgoing.push(Outgoing::Command(c.command)),
            Submit::UnknownCommand => self.print_debug(commands::msg::UNKNOWN_COMMAND.as_bytes()),
            Submit::Local(cmd) => {
                let effects = commands::execute(&cmd, &mut self.settings, &mut self.cfg, game);
                for e in effects {
                    self.effect(e);
                }
            }
        }
    }

    /// Control-channel `say TEXT`: as if TEXT were typed into the input and Enter pressed.
    pub fn say(&mut self, text: &[u8], game: &dyn GameInfo) {
        self.input.open();
        self.input.set_text(text);
        self.submit(game);
    }

    fn effect(&mut self, e: Effect) {
        match e {
            Effect::Print(kind, text) => self.print(kind, &text),
            Effect::WriteConfig => self.write_config(),
            Effect::Quit { disconnect } => self.requests.quit = Some(disconnect),
            Effect::SendRcon(t) => self.outgoing.push(Outgoing::Rcon(t)),
            Effect::AppendFile { file, line, ok, ok_kind, failed } => {
                let written = self
                    .user_dir
                    .as_ref()
                    .map(|d| d.join(file.file_name()))
                    .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok())
                    .is_some_and(|mut f| f.write_all(&text_mode(&line)).is_ok());
                if written {
                    self.print(ok_kind, &ok);
                } else {
                    self.print_debug(&failed);
                }
            }
            Effect::SetChatPageSize(n) => {
                self.chat.set_page_size(n);
            }
            Effect::RebuildFonts(_) => self.requests.rebuild_fonts = true,
            Effect::SetTimestamps(on) => self.chat.set_timestamps(on),
            // Stored in `settings`; their game effects belong to later milestones.
            other => crate::log!("ui: effect {other:?} stored only (not applied in M4)"),
        }
    }

    /// Takes the queued network messages.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outgoing)
    }

    // ---- control channel -----------------------------------------------------------

    /// `chat [N]`: header plus the last N non-empty entries (oldest first), each
    /// with its colour runs as drawn (UR §3.3 glyph colouring).
    pub fn chat_report(&self, n: usize) -> String {
        let all: Vec<&ChatEntry> = self.chat.entries().flatten().collect();
        let take = n.min(all.len());
        let visible: Vec<Option<&ChatEntry>> = self.chat.visible().collect();
        let mut out = format!(
            "chat count={take} total={} page_size={} scroll={} at_bottom={} mode={} timestamps={} visible_lines={}",
            all.len(),
            self.chat.page_size(),
            self.chat.scroll_pos(),
            self.chat.at_bottom() as u8,
            mode_num(self.chat.mode()),
            self.chat.timestamps() as u8,
            visible.iter().filter(|e| e.is_some()).count(),
        );
        for e in &all[all.len() - take..] {
            out.push('\n');
            out.push_str(&entry_line(e));
        }
        out
    }

    /// `input`: chat input state.
    pub fn input_report(&self) -> String {
        let e = self.input.edit();
        format!(
            "input open={} text={} caret={} history={} netgame={} connected={} page_size={} cfg={}",
            self.input.is_open() as u8,
            quote(e.text()),
            e.caret(),
            self.input.history().count(),
            self.netgame as u8,
            self.connected as u8,
            self.chat.page_size(),
            self.cfg_path().map(|p| quote(p.to_string_lossy().as_bytes())).unwrap_or_else(|| "-".into()),
        )
    }
}

fn mode_num(m: DisplayMode) -> u8 {
    match m {
        DisplayMode::Hidden => 0,
        DisplayMode::Plain => 1,
        DisplayMode::Outlined => 2,
    }
}

fn entry_line(e: &ChatEntry) -> String {
    let kind = match e.kind {
        EntryType::PlayerChat => 2,
        EntryType::Info => 4,
        EntryType::Debug => 8,
    };
    let runs: Vec<String> = colour::colour_runs(&e.text, e.text_colour)
        .iter()
        .map(|r| format!("{:08X}:{}", r.colour, quote(&r.text)))
        .collect();
    format!(
        "line type={kind} colour={:08X} name_colour={:08X} name={} text={} runs=[{}]",
        e.text_colour,
        e.name_colour,
        quote(&e.prefix()),
        quote(&e.text),
        runs.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use samp_proto::rpcs::ui::{PlayerChatMessage, SendClientMessage, SendGameText};
    use samp_ui::commands::PlayerPosition;
    use samp_ui::text::NoExpansion;

    struct Game;
    impl GameInfo for Game {
        fn connected(&self) -> bool {
            true
        }
        fn memory_counter(&self) -> u32 {
            7
        }
        fn interior(&self) -> u32 {
            0
        }
        fn position(&self) -> Option<PlayerPosition> {
            Some(PlayerPosition::OnFoot { skin: 0, x: 1.0, y: 2.0, z: 3.0, heading: 90.0 })
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("samp-rs-ui-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn connected_ui(dir: Option<PathBuf>) -> UiCore {
        let mut u = UiCore::new(dir, false);
        u.on_netgame_start("127.0.0.1", 7777, b"RustClient");
        u.on_init_game(b"srv", 0);
        u
    }

    fn typed(u: &mut UiCore, s: &[u8]) {
        for &c in s {
            u.handle_event(Event::Char(c), false, &Game, 3);
        }
    }

    #[test]
    fn connect_lines_are_type_8() {
        let u = connected_ui(None);
        let lines: Vec<_> = u.chat.entries().flatten().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|e| e.kind == EntryType::Debug && e.text_colour == 0xFFA9C4E4));
        assert_eq!(lines[1].text, b"Connecting to 127.0.0.1:7777...");
        assert_eq!(lines[2].text, b"Connected to {B9C9BF}srv");
    }

    #[test]
    fn client_message_and_player_chat() {
        let mut u = connected_ui(None);
        let r = Incoming::Ui(ui::Incoming::SendClientMessage(SendClientMessage {
            colour: 0x11223344,
            message: b"hi {FF0000}red".to_vec(),
        }));
        u.on_rpc(&r, &NoExpansion);
        let e = u.chat.last().unwrap();
        assert_eq!((e.kind, e.text_colour), (EntryType::Info, 0xFF112233));
        let r = Incoming::Ui(ui::Incoming::PlayerChatMessage(PlayerChatMessage {
            player_id: 0,
            message: b"hello world".to_vec(),
        }));
        u.on_rpc(&r, &NoExpansion);
        let e = u.chat.last().unwrap();
        assert_eq!(e.kind, EntryType::PlayerChat);
        assert_eq!(e.name, b"RustClient");
        assert_eq!(e.name_colour, 0xFFFF8C13);
        // Unknown remote id: ignored.
        let r =
            Incoming::Ui(ui::Incoming::PlayerChatMessage(PlayerChatMessage { player_id: 5, message: b"x".to_vec() }));
        u.on_rpc(&r, &NoExpansion);
        assert_eq!(u.chat.last().unwrap().text, b"hello world");
        let rep = u.chat_report(2);
        assert!(rep.starts_with("chat count=2 "), "{rep}");
        assert!(rep.contains("runs=[FF112233:\"hi \",FFFF0000:\"red\"]"), "{rep}");
        assert!(rep.contains("name=\"RustClient:\""), "{rep}");
    }

    #[test]
    fn typing_chat_and_commands() {
        let mut u = connected_ui(None);
        typed(&mut u, b"t");
        assert!(u.input.is_open());
        typed(&mut u, b"hello world\r");
        assert!(!u.input.is_open());
        typed(&mut u, b"t/foo bar\r");
        assert_eq!(
            u.take_outgoing(),
            vec![Outgoing::Chat(b"hello world".to_vec()), Outgoing::Command(b"/foo bar".to_vec())]
        );
        u.say(b"/rcon login x", &Game);
        assert_eq!(u.take_outgoing(), vec![Outgoing::Rcon(b"login x".to_vec())]);
    }

    #[test]
    fn pagesize_persists_and_save_appends() {
        let d = tmpdir("cfg");
        std::fs::write(d.join("sa-mp.cfg"), b"pagesize=12\r\n").unwrap();
        let mut u = UiCore::new(Some(d.clone()), false);
        assert_eq!(u.chat.page_size(), 12);
        // Start-up defaults were appended.
        let cfg = std::fs::read_to_string(d.join("sa-mp.cfg")).unwrap();
        assert!(cfg.contains("fpslimit=48"), "{cfg}");
        u.netgame = true;
        u.say(b"/pagesize 15", &Game);
        assert_eq!(u.chat.page_size(), 15);
        let cfg = std::fs::read_to_string(d.join("sa-mp.cfg")).unwrap();
        assert!(cfg.starts_with("pagesize=15\r\n"), "{cfg}");
        u.say(b"/pagesize 25", &Game);
        assert_eq!(u.chat.last().unwrap().text, b"pagesize [10-20] (lines)");
        assert_eq!(u.chat.last().unwrap().kind, EntryType::Debug);
        u.say(b"/save here", &Game);
        let s = std::fs::read_to_string(d.join("savedpositions.txt")).unwrap();
        assert_eq!(s, "AddPlayerClass(0,1.0000,2.0000,3.0000,90.0000,0,0,0,0,0,0); // here\r\n");
        assert_eq!(u.chat.last().unwrap().text, b"-> OnFoot position saved");
        assert_eq!(u.chat.last().unwrap().kind, EntryType::Info);
        u.say(b"/timestamp", &Game);
        assert!(u.chat.timestamps());
        u.say(b"/q", &Game);
        assert_eq!(u.requests.quit, Some(true));
        let log = std::fs::read(d.join("chatlog.txt")).unwrap();
        assert!(log.ends_with(b"\r\r\n"), "{log:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn game_text_request() {
        let mut u = connected_ui(None);
        let r =
            Incoming::Ui(ui::Incoming::SendGameText(SendGameText { style: 3, time_ms: 5000, text: b"~r~hi".to_vec() }));
        u.on_rpc(&r, &NoExpansion);
        assert_eq!(u.requests.game_text, Some((3, 5000, b"~r~hi".to_vec())));
    }

    #[test]
    fn colour_rpc_and_join() {
        let mut u = connected_ui(None);
        let j = Incoming::Remote(remote::Incoming::PlayerJoin(remote::PlayerJoin {
            player_id: 3,
            colour: 0x00FF00FF,
            is_npc: 0,
            name: b"Bob".to_vec(),
        }));
        u.on_rpc(&j, &NoExpansion);
        assert_eq!(u.colours.chat_argb(3), 0xFF00FF00);
        let c = Incoming::Player(player::Incoming::SetPlayerColor(player::SetPlayerColor {
            player_id: 0,
            colour: 0xFF0000FF,
        }));
        u.on_rpc(&c, &NoExpansion);
        assert_eq!(u.colours.chat_argb(0), 0xFFFF0000);
    }
}
