//! Platform-independent UI state of the in-game client, built on `samp-ui`: chat window and input, local
//! commands, `sa-mp.cfg`, dialogs, scoreboard, death window, bubbles, textdraws and menus.
//!
//! The Windows glue (`ui::glue`) owns one [`UiCore`], feeds it RPCs, window messages and control-channel
//! requests, and drains what it asks the host to do ([`Outgoing`] network messages, [`HostRequests`]).
//! Everything here is host-testable. Spec: docs/re/subsystems/ui-rendering.md and ui-chat-audio.md;
//! addresses in docs/re/addresses/ui-core.md.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use samp_proto::rpcs::ui;
use samp_ui::bubbles::ChatBubbles;
use samp_ui::chat::{ChatWindow, LocalTime};
use samp_ui::commands::MessageKind;
use samp_ui::config::{self, SampCfg, UiSettings};
use samp_ui::deathwindow::DeathWindow;
use samp_ui::dialog::Dialogs;
use samp_ui::input::ChatInput;
use samp_ui::menu::{MenuBuild, Menus};
use samp_ui::scoreboard::Scoreboard;
use samp_ui::textdraw::TextDraws;

use super::colours::PlayerColours;

mod input;
mod netstats;
mod reports;
mod rpc;
#[cfg(test)]
mod tests;

pub use netstats::{NetStats, net_stats_text};

/// Client-local strings from samp.dll's read-only data, all printed as type-8 (debug) chat lines.
pub mod msg {
    /// Printed when the netgame object is created.
    pub const STARTED: &str = "{FFFFFF}SA-MP {B9C9BF}0.3.7-R4 {FFFFFF}Started";
    /// Printed on each connect attempt.
    pub fn connecting(host: &str, port: u16) -> String {
        format!("Connecting to {host}:{port}...")
    }
    /// Printed on the `0x22` connection-accepted packet.
    pub const JOINING: &str = "Connected. Joining the game...";
    /// `Connected to {B9C9BF}%.64s`, printed on InitGame.
    pub fn connected_to(server: &[u8]) -> Vec<u8> {
        let mut v = b"Connected to {B9C9BF}".to_vec();
        v.extend_from_slice(&server[..server.len().min(64)]);
        v
    }
    /// Printed when the server closes the connection.
    pub const SERVER_CLOSED: &str = "Server closed the connection.";
}

/// Chat scroll position set by Enter in the input. Mirrors samp.dll 0x10067390.
const SUBMIT_SCROLL_POS: u32 = 90;
/// `INVALID_PLAYER_ID`.
const INVALID_PLAYER: u16 = 0xFFFF;

/// A message the UI wants sent (drained by the network session every frame).
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    /// RPC 101 PlayerRequestChatMessage.
    Chat(Vec<u8>),
    /// RPC 50 PlayerRequestCommandMessage (the whole typed line).
    Command(Vec<u8>),
    /// Packet 201 PlayerRconCommand.
    Rcon(Vec<u8>),
    /// RPC 62 OnPlayerDialogResponse (reliability 9).
    DialogResponse(ui::OnPlayerDialogResponse),
    /// RPC 83 OnPlayerSelectTextDraw (reliability 9).
    SelectTextDraw(u16),
    /// RPC 132 OnPlayerSelectedMenuRow (reliability 8).
    MenuRow(u8),
    /// RPC 140 OnPlayerExitedMenu (reliability 8).
    MenuExit,
    /// RPC 155 OnPlayerRequestScoresAndPings (reliability 8, empty).
    ScoresRequest,
    /// RPC 23 OnPlayerClickPlayer from the scoreboard (reliability 9).
    ClickPlayer(u16),
    /// RPC 119 OnPlayerClickMap (reliability 8).
    ClickMap([f32; 3]),
}

/// A remote player as the UI needs it (published by the player pool every tick).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteUi {
    pub id: u16,
    /// `CPed*` (0 = not streamed in).
    pub ped: u32,
    pub state: u8,
    pub health: f32,
    pub armour: f32,
    pub npc: bool,
    /// Milliseconds since the last sync (u64::MAX = never).
    pub ms_since_sync: u64,
}

/// Cursor / input-lock requests the host executes in order (ui-rendering.md, "5. Cursor and input-lock
/// modes").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorRequest {
    /// Mode 0 with the 10-frame delay (dialog close) or immediately.
    Mode0 { immediate: bool },
    /// Mode 3 (scoreboard open).
    Mode3,
}

/// GTA menu work for the host (script commands).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuRequest {
    /// Delete the current GTA menu (0x08DA), then build this one if given.
    Show {
        delete_previous: bool,
        build: MenuBuild,
    },
    Delete,
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
    /// Focus loss: run the input-restore step (`WM_KILLFOCUS`, ui-rendering.md, "6.3 Key and mouse table").
    pub input_restore: bool,
    pub cursor: Vec<CursorRequest>,
    pub menus: Vec<MenuRequest>,
    /// F8: take a screenshot at the next Present.
    pub screenshot: bool,
    /// DXUT edit box Ctrl+C / Ctrl+X / Ctrl+Insert with a selection: put this on the clipboard.
    pub clipboard_copy: Option<Vec<u8>>,
    /// DXUT edit box Ctrl+V / Shift+Insert: paste the clipboard into the focused edit box
    /// ([`UiCore::paste`]).
    pub paste: bool,
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
    connect_host: String,
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
    pub dialogs: Dialogs,
    /// Bumped whenever a dialog opens (the renderer caches its layout).
    pub dialog_serial: u64,
    pub scoreboard: Scoreboard,
    /// RPC 155 records: id → (score, ping).
    pub scores: BTreeMap<u16, (i32, u32)>,
    pub deaths: DeathWindow,
    pub bubbles: ChatBubbles,
    pub textdraws: TextDraws,
    pub menus: Menus,
    /// RPC 80 per player (default shown).
    pub hidden_tags: std::collections::BTreeSet<u16>,
    /// InitGame name-tag settings.
    pub show_name_tags: bool,
    pub name_tag_distance: f32,
    pub name_tag_los: bool,
    pub server_name: Vec<u8>,
    pub responses_sent: u32,
    /// Screen rects for click routing (set by the renderer side).
    pub input_rect: samp_ui::Rect,
    pub scoreboard_rect: samp_ui::Rect,
    /// Class selection running (blocks the scoreboard).
    pub class_selection: bool,
    /// Rows visible in the open list dialog (set by the renderer side).
    pub dialog_page_rows: usize,
}

/// Local wall-clock time for chat entries.
pub fn local_time_now() -> LocalTime {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: SYSTEMTIME is plain data; all-zero is a valid value.
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

/// Milliseconds since the UI started (samp uses `GetTickCount`).
pub fn tick_ms() -> u32 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u32
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
            // fpslimit and multicore are created if missing; audioproxyoff only after BASS has started
            // (`audio_proxy_key`). See ui-rendering.md, "9. Local commands — UI effects and persistence".
            cfg_dirty = cfg.apply_startup_defaults();
        }
        let settings = UiSettings::from_cfg(&cfg);
        let mut chat = ChatWindow::new();
        chat.set_page_size(settings.page_size);
        chat.set_timestamps(settings.timestamps);
        // chatlog.txt is truncated (mode "w") at construction; logging is on if that succeeded.
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
            connect_host: String::new(),
            colours: PlayerColours::new(),
            netgame: false,
            connected: false,
            debug_mode,
            outgoing: Vec::new(),
            requests: HostRequests::default(),
            last_game_text: None,
            dialogs: Dialogs::new(),
            dialog_serial: 0,
            scoreboard: Scoreboard::new(),
            scores: BTreeMap::new(),
            deaths: DeathWindow::new(),
            bubbles: ChatBubbles::new(),
            textdraws: TextDraws::new(),
            menus: Menus::new(),
            hidden_tags: Default::default(),
            // The netgame settings are zero until InitGame; samp's nametag passes first run after it.
            show_name_tags: false,
            name_tag_distance: 70.0,
            name_tag_los: true,
            server_name: Vec::new(),
            responses_sent: 0,
            input_rect: samp_ui::Rect::default(),
            scoreboard_rect: samp_ui::Rect::default(),
            class_selection: false,
            dialog_page_rows: 10,
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

    /// After a successful `BASS_Init`: `audioproxyoff` is created with 0 when missing (the file is
    /// rewritten), then read. Mirrors samp.dll 0x10066480.
    pub fn audio_proxy_key(&mut self) -> bool {
        if self.user_dir.is_some() && self.cfg.ensure_audio_proxy_key() {
            self.write_config();
        }
        self.cfg.get_int(config::keys::AUDIOPROXYOFF).unwrap_or(0) != 0
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

    /// Type-4 line in the info colour. Mirrors samp.dll 0x10068070.
    pub fn print_info(&mut self, text: &[u8]) {
        let l = self.chat.add_info(text, local_time_now());
        self.log_lines(l);
    }

    /// F1 held: the local help dialog through the normal dialog show path (which closes an open chat
    /// input first), only when no dialog is open. Mirrors samp.dll 0x1006BB30.
    pub fn open_help(&mut self) -> bool {
        if !self.dialogs.open_help() {
            return false;
        }
        self.input.close();
        self.dialog_serial += 1;
        true
    }

    /// F8 file name: the first `<user dir>\screens\sa-mp-%03i.png` (index 0..999) that does not exist.
    /// When all exist the loop ends at 1000 with the name of 999 still in the buffer, so the file 999 is
    /// overwritten and the message says 1000. Mirrors samp.dll 0x10062270.
    pub fn screenshot_target(&self) -> (PathBuf, u32) {
        let dir = self.user_dir.clone().unwrap_or_default().join("screens");
        let name = |i: u32| dir.join(format!("sa-mp-{i:03}.png"));
        let mut i = 0;
        loop {
            let p = name(i);
            if std::fs::metadata(&p).is_err() {
                return (p, i);
            }
            i += 1;
            if i >= 1000 {
                return (p, i);
            }
        }
    }

    /// F8 result line: type 4 on success, type 8 when the front buffer could not be read (the save
    /// result itself is not checked). Mirrors samp.dll 0x10075620.
    pub fn screenshot_done(&mut self, index: Option<u32>) {
        match index {
            Some(i) => self.print_info(format!("Screenshot Taken - sa-mp-{i:03}.png").as_bytes()),
            None => self.print_debug(b"Unable to save screenshot."),
        }
    }

    /// Type-8 line in the debug colour. Mirrors samp.dll 0x100680F0.
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

    /// The netgame object is created: "Started" (the connect lines come from
    /// `lifecycle.rs` through the client).
    pub fn on_netgame_start(&mut self, host: &str, port: u16, name: &[u8]) {
        self.netgame = true;
        self.local_name = name.to_vec();
        self.connect_host = format!("{host}:{port}");
        self.print_debug(msg::STARTED.as_bytes());
    }

    /// RPC 139 InitGame: netgame state 5.
    pub fn on_init_game(&mut self, g: &samp_proto::session::InitGame) {
        self.connected = true;
        self.local_id = Some(g.player_id);
        self.show_name_tags = g.show_name_tags;
        self.name_tag_distance = g.name_tag_distance;
        // samp reads the bit into settings +0x2F and then stores 1 there unconditionally, so the nametag
        // line-of-sight test always runs in 0.3.7-R4, whatever the server sends.
        self.name_tag_los = true;
        let _ = g.name_tag_los;
        self.server_name = g.server_name.as_bytes().to_vec();
        self.print_debug(&msg::connected_to(g.server_name.as_bytes()));
    }

    pub fn local_id(&self) -> Option<u16> {
        self.local_id
    }

    pub fn local_name(&self) -> &[u8] {
        &self.local_name
    }

    pub fn name(&self, id: u16) -> Option<&[u8]> {
        if Some(id) == self.local_id { Some(&self.local_name) } else { self.names.get(&id).map(Vec::as_slice) }
    }

    pub fn remote_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.names.keys().copied()
    }

    /// RPC 40 reset: the netgame leaves state 5, the textdraw and menu pools are recreated, the name-tag
    /// settings go back to samp's defaults (shown, distance 60.0, LOS on). Dialogs, scoreboard and chat
    /// are untouched. Mirrors samp.dll B 0x1000A530.
    pub fn on_reset(&mut self) {
        self.connected = false;
        self.textdraws.clear();
        self.menus.clear();
        self.show_name_tags = true;
        self.name_tag_distance = 60.0;
        self.name_tag_los = true;
    }

    /// Lost connection: the remote players are deleted from the pool.
    pub fn forget_players(&mut self) {
        self.names.clear();
        self.scores.clear();
        self.hidden_tags.clear();
    }
}
