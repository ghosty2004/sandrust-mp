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

use samp_proto::rpcs::{Incoming, player, remote, session, ui};
use samp_ui::chat::{ChatEntry, ChatWindow, ClientMessageResult, DisplayMode, EntryType, LocalTime};
use samp_ui::colour;
use samp_ui::commands::{self, Effect, GameInfo, MessageKind};
use samp_ui::config::{self, SampCfg, UiSettings};
use samp_ui::edit::{CharResult, EditKey, Mods};
use samp_ui::gametext::GameText;
use samp_ui::input::{ChatInput, Submit};
use samp_ui::bubbles::ChatBubbles;
use samp_ui::deathwindow::DeathWindow;
use samp_ui::dialog::{Button, Dialogs, ShowOutcome};
use samp_ui::keys::{self, Action, Event, UiState};
use samp_ui::menu::{MenuBuild, Menus};
use samp_ui::scoreboard::{PlayerInfo, Scoreboard};
use samp_ui::textdraw::TextDraws;
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
/// `INVALID_PLAYER_ID`.
const INVALID_PLAYER: u16 = 0xFFFF;

/// A message the UI wants sent (drained by the network session every frame).
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Cursor/input-lock requests (UR §5) the host executes in order.
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
    Show { delete_previous: bool, build: MenuBuild },
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
    /// Focus loss: run the input-restore step (UR §6.3 `WM_KILLFOCUS`).
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

/// Values of the F5 overlay (A 0x10060d70). Fields our client has no counter for stay 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetStats {
    pub download_kbs: f32,
    pub upload_kbs: f32,
    pub vehicles: u32,
    pub vehicle_models: u32,
    pub vehicle_models_loaded: u32,
    pub rw_objects: u32,
    pub ped_slots_used: u32,
    pub object_slots_used: u32,
    pub shot_comp_mode: u32,
    pub free_ides: u32,
    pub in_car: bool,
    pub send_rate: u32,
    pub streaming_mb: u32,
    pub streaming_total_mb: u32,
    pub system_mb: u32,
    pub process_kb: u32,
    pub working_set_kb: u32,
    pub messages_in_send_buffer: u32,
    pub messages_sent: u32,
    pub bytes_sent: u32,
    pub acks_sent: u32,
    pub acks_in_send_buffer: u32,
    pub messages_waiting_for_ack: u32,
    pub messages_resent: u32,
    pub bytes_resent: u32,
    pub packet_loss: f32,
    pub messages_received: u32,
    pub bytes_received: u32,
    pub acks_received: u32,
    pub duplicate_acks_received: u32,
}

/// The F5 text, line by line as samp concatenates it (A 0x10060d70; the labels and their
/// order measured on the original): rates, the pool block, the send rate, memory, then the
/// network statistics block. Every line ends with `\n`.
pub fn net_stats_text(s: &NetStats) -> Vec<u8> {
    let mut t = format!("Download Rate: {:.2} KB/s\nUpload Rate: {:.2} KB/s\n", s.download_kbs, s.upload_kbs);
    t += &format!(
        "Vehicles: {}\nVehicle Models: {}\nVehicle Models Loaded: {}\nRWObjects: {}\nPedSlotsUsed: {}\nObjectSlotsUsed: {}\nShotCompMode: {}\nFreeIDEs: {}\n",
        s.vehicles,
        s.vehicle_models,
        s.vehicle_models_loaded,
        s.rw_objects,
        s.ped_slots_used,
        s.object_slots_used,
        s.shot_comp_mode,
        s.free_ides
    );
    t += &format!("{} Send Rate: {}\n", if s.in_car { "InCar" } else { "OnFoot" }, s.send_rate);
    t += &format!(
        "Streaming Mem: {}MB Total: {}MB System: {}MB\n",
        s.streaming_mb, s.streaming_total_mb, s.system_mb
    );
    t += &format!("Process Mem: {}KB Working Set: {}KB\n", s.process_kb, s.working_set_kb);
    t += &format!(
        "Messages in Send buffer: {}\nMessages sent: {}\nBytes sent: {}\nAcks sent: {}\nAcks in send buffer: {}\nMessages waiting for ack: {}\nMessages resent: {}\nBytes resent: {}\nPacketloss: {:.1}%\nMessages received: {}\nBytes received: {}\nAcks received: {}\nDuplicate acks received: {}\n",
        s.messages_in_send_buffer,
        s.messages_sent,
        s.bytes_sent,
        s.acks_sent,
        s.acks_in_send_buffer,
        s.messages_waiting_for_ack,
        s.messages_resent,
        s.bytes_resent,
        s.packet_loss,
        s.messages_received,
        s.bytes_received,
        s.acks_received,
        s.duplicate_acks_received
    );
    t.into_bytes()
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
            // The netgame settings are zero until InitGame (measured: samp's nametag passes
            // first run after RPC 139).
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

    /// F1 held (A 0x1006bb30): the local help dialog through the normal show path (A 0x10070010,
    /// which closes an open chat input first), only when no dialog is open.
    pub fn open_help(&mut self) -> bool {
        if !self.dialogs.open_help() {
            return false;
        }
        self.input.close();
        self.dialog_serial += 1;
        true
    }

    /// F8 file name (A 0x10062270): the first `<user dir>\screens\sa-mp-%03i.png` (index
    /// 0..999) that does not exist. When all exist the loop ends at 1000 with the name of 999
    /// still in the buffer, so the file 999 is overwritten and the message says 1000.
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

    /// F8 result line (A 0x10075620): type 4 on success, type 8 when the front buffer
    /// could not be read (the save result itself is not checked).
    pub fn screenshot_done(&mut self, index: Option<u32>) {
        match index {
            Some(i) => self.print_info(format!("Screenshot Taken - sa-mp-{i:03}.png").as_bytes()),
            None => self.print_debug(b"Unable to save screenshot."),
        }
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
        // samp reads the bit into settings +0x2F and then stores 1 there unconditionally (B 0x10010778): the
        // nametag LOS test always runs in R4 (measured: 0x06BD per pass with the bit 0 from open.mp).
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

    /// RPC 40 reset (B 0x1000a530): the netgame leaves state 5, the textdraw and menu pools
    /// are recreated, the name-tag settings go back to samp's defaults (+0x1f shown = 1,
    /// distance 60.0, LOS on). Dialogs, scoreboard and chat are untouched.
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
                self.bubbles.clear(q.player_id);
                self.hidden_tags.remove(&q.player_id);
                self.scores.remove(&q.player_id);
            }
            Incoming::Remote(remote::Incoming::ShowPlayerNameTagForPlayer(t)) => {
                if t.show != 0 {
                    self.hidden_tags.remove(&t.player_id);
                } else {
                    self.hidden_tags.insert(t.player_id);
                }
            }
            Incoming::Remote(remote::Incoming::SetPlayerChatBubble(b)) => {
                if self.names.contains_key(&b.player_id) {
                    self.bubbles.set(b.player_id, b.colour, b.draw_distance, b.expire_time, &b.text, tick_ms());
                }
            }
            Incoming::Ui(ui::Incoming::ShowDialog(d)) => match self.dialogs.show(d) {
                ShowOutcome::Opened => {
                    self.input.close();
                    self.dialog_serial += 1;
                }
                ShowOutcome::Closed { was_open: true } => {
                    self.requests.cursor.push(CursorRequest::Mode0 { immediate: false })
                }
                _ => {}
            },
            Incoming::Ui(ui::Incoming::PlayerShowTextDraw(t)) => {
                if t.text.len() >= samp_ui::textdraw::TEXT_MAX {
                    self.print_debug(format!("Warning: ignoring large TextDraw size={}", t.text.len()).as_bytes());
                } else {
                    self.textdraws.show(t.textdraw_id, t.data, &t.text);
                }
            }
            Incoming::Ui(ui::Incoming::PlayerHideTextDraw(t)) => self.textdraws.hide(t.textdraw_id),
            Incoming::Ui(ui::Incoming::PlayerTextDrawSetString(t)) => {
                self.textdraws.set_string(t.textdraw_id, &t.text);
            }
            Incoming::Ui(ui::Incoming::PlayerBeginTextDrawSelect(s)) => {
                if s.enable {
                    self.textdraws.begin_select(s.hover_colour);
                } else {
                    self.cancel_textdraw_select();
                }
            }
            Incoming::Ui(ui::Incoming::PlayerInitMenu(m)) => {
                self.menus.init(m);
            }
            Incoming::Ui(ui::Incoming::PlayerShowMenu(m)) => {
                if let Some((delete_previous, build)) = self.menus.show(m.menu_id) {
                    self.requests.menus.push(MenuRequest::Show { delete_previous, build });
                }
            }
            Incoming::Ui(ui::Incoming::PlayerHideMenu(m)) => {
                if self.menus.hide(m.menu_id) {
                    self.requests.menus.push(MenuRequest::Delete);
                }
            }
            Incoming::Session(session::Incoming::SendPlayerScoresAndPings(s)) => {
                for p in &s.players {
                    if p.player_id < 1004 {
                        self.scores.insert(p.player_id, (p.score, p.ping));
                    }
                }
            }
            Incoming::Player(player::Incoming::SendDeathMessage(d)) => self.on_death_message(d),
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

    /// RPC 55 (player-state §55): names and colours of the ids; victim 0xFFFF drops it.
    fn on_death_message(&mut self, d: &player::SendDeathMessage) {
        if d.victim_id == INVALID_PLAYER {
            return;
        }
        let who = |u: &Self, id: u16| -> (Vec<u8>, u32) {
            match u.name(id) {
                Some(n) if id != INVALID_PLAYER => (n.to_vec(), u.colours.chat_argb(id)),
                _ => (Vec::new(), 0),
            }
        };
        let (kn, kc) = who(self, d.killer_id);
        let (vn, vc) = who(self, d.victim_id);
        self.deaths.add(&kn, kc, &vn, vc, d.reason);
    }

    /// The textdraw-selection disable path (UCA §83): mode off, cursor mode 0, chat
    /// input closed, RPC 83 with 0xFFFF.
    pub fn cancel_textdraw_select(&mut self) {
        self.textdraws.cancel_select();
        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
        self.input.close();
        self.outgoing.push(Outgoing::SelectTextDraw(samp_ui::textdraw::NONE));
    }

    /// Answers the open dialog (UCA §62) and closes it (cursor mode 0, 10-frame delay).
    pub fn dialog_respond(&mut self, button: Button) {
        if !self.dialogs.is_open() {
            return;
        }
        if let Some(r) = self.dialogs.respond(button) {
            self.responses_sent += 1;
            self.outgoing.push(Outgoing::DialogResponse(r));
        }
        self.requests.cursor.push(CursorRequest::Mode0 { immediate: false });
    }

    /// Scoreboard rows (local first) from the names, colours and RPC 155 records.
    pub fn rebuild_scoreboard(&mut self) {
        let info = |u: &Self, id: u16, name: &[u8]| {
            let (score, ping) = u.scores.get(&id).copied().unwrap_or((0, 0));
            PlayerInfo { id, name: name.to_vec(), score, ping, colour_rgba: u.colours.rgba(id), is_npc: false }
        };
        let local = info(self, self.local_id.unwrap_or(0), &self.local_name);
        let remotes: Vec<PlayerInfo> = self.names.iter().map(|(&id, n)| info(self, id, n)).collect();
        self.scoreboard.rebuild(&local, &remotes);
        // TRACE-3: the first row is selected (selection bar on the local player's row).
        if self.scoreboard.selected().is_none() && !self.scoreboard.rows().is_empty() {
            self.scoreboard.set_selected(Some(0));
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
        let d = self.dialogs.current();
        UiState {
            textdraw_select: self.textdraws.selection.active,
            dialog_is_list: d.is_some_and(|d| d.style.is_list()),
            input_exists: self.netgame,
            input_open: self.input.is_open(),
            input_rect: self.input_rect,
            dialog_open: d.is_some(),
            dialog_has_edit: d.is_some_and(|d| d.style.has_edit()),
            scoreboard_exists: self.netgame,
            scoreboard_open: self.scoreboard.is_open(),
            scoreboard_rect: self.scoreboard_rect,
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
                Action::InputChar(0x16) => self.requests.paste = true,
                Action::InputChar(c) => match self.input.char(c) {
                    CharResult::Enter => self.submit(game),
                    CharResult::Copy(sel) => self.copy(sel),
                    CharResult::Edited | CharResult::Ignored => {}
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
                Action::DialogChar(0x16) => self.requests.paste = true,
                Action::DialogChar(c) => {
                    if let Some(d) = self.dialogs.current_mut() {
                        // Enter in the edit box: response 1 (UCA §62).
                        match d.edit.char(c) {
                            CharResult::Enter => self.dialog_respond(Button::Ok),
                            CharResult::Copy(sel) => self.copy(sel),
                            _ => {}
                        }
                    }
                }
                Action::DialogListKey(k) => {
                    if let Some(d) = self.dialogs.current_mut() {
                        d.list_key(k, self.dialog_page_rows);
                    }
                }
                Action::DialogResponse(b) => self.dialog_respond(b),
                Action::CancelTextDrawSelect => self.cancel_textdraw_select(),
                Action::ToggleScoreboard => {
                    let was = self.scoreboard.is_open();
                    let open = self.scoreboard.toggle(self.class_selection);
                    if open && !was {
                        self.rebuild_scoreboard();
                        self.requests.cursor.push(CursorRequest::Mode3);
                    } else if was && !open {
                        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
                    }
                }
                Action::CloseScoreboard => {
                    if self.scoreboard.is_open() {
                        self.scoreboard.close();
                        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
                    }
                }
                Action::ToggleDeathWindow => {
                    self.deaths.toggle_visible();
                }
                Action::Screenshot => self.requests.screenshot = true,
                Action::CursorModeZeroImmediate => self.requests.cursor.push(CursorRequest::Mode0 { immediate: true }),
                other => crate::log!("ui: action {other:?} not handled in M4"),
            }
        }
    }

    /// Edit-box key-down while the input or a dialog edit box has focus (DXUT edit box).
    pub fn edit_key(&mut self, key: EditKey, mods: Mods) -> bool {
        let copied = if self.input.is_open() {
            self.input.key(key, mods)
        } else if let Some(d) = self.dialogs.current_mut()
            && d.style.has_edit()
            && !self.scoreboard.is_open()
        {
            d.edit.key(key, mods)
        } else {
            return false;
        };
        if let Some(sel) = copied {
            self.copy(sel);
        } else if key == EditKey::Insert && mods.shift && !mods.ctrl {
            self.requests.paste = true;
        }
        true
    }

    /// DXUT `CopyToClipboard`: only when something is selected.
    fn copy(&mut self, sel: Vec<u8>) {
        if !sel.is_empty() {
            self.requests.clipboard_copy = Some(sel);
        }
    }

    /// DXUT `PasteFromClipboard` into the focused edit box (chat input, else a dialog edit box).
    pub fn paste(&mut self, clip: &[u8]) {
        if self.input.is_open() {
            self.input.paste(clip);
        } else if let Some(d) = self.dialogs.current_mut()
            && d.style.has_edit()
        {
            d.edit.paste(clip);
        }
    }

    /// `ui` control report.
    pub fn ui_report(&self) -> String {
        let mut s = String::from("ui");
        match self.dialogs.current() {
            Some(d) => {
                let sel = d.selected().map_or(-1, |v| v as i32);
                s.push_str(&format!(
                    " dialog_open=1 dialog_id={} dialog_style={:?} dialog_rows={} dialog_columns={} dialog_selected={sel} dialog_text={}",
                    d.id,
                    d.style,
                    d.rows().len(),
                    d.columns(),
                    quote(d.edit.text())
                ));
            }
            None => s.push_str(" dialog_open=0"),
        }
        s.push_str(&format!(
            " responses={} scoreboard_open={} scoreboard_rows={} textdraws={} textdraw_ids={:?} select={} hovered={} menu={} deaths={} death_visible={} bubbles={} hidden_tags={}",
            self.responses_sent,
            self.scoreboard.is_open() as u8,
            self.scoreboard.rows().len(),
            self.textdraws.count(),
            self.textdraws.ids(),
            self.textdraws.selection.active as u8,
            self.textdraws.selection.hovered.map_or(-1, i32::from),
            self.menus.current().map_or(-1, i32::from),
            self.deaths.rows().count(),
            self.deaths.visible() as u8,
            (0..samp_ui::bubbles::SLOTS as u16).filter(|&i| self.bubbles.get(i).is_some()).count(),
            self.hidden_tags.len(),
        ));
        s
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
                // TRACE-8: `/rs` in a car writes `rawvehicles.txt` relative to the game's current
                // directory (B string `rawvehicles.txt`, no user-files prefix); the other files go
                // to the user files directory.
                let path = if file == samp_ui::commands::SaveFile::RawVehicles {
                    Some(PathBuf::from(file.file_name()))
                } else {
                    self.user_dir.as_ref().map(|d| d.join(file.file_name()))
                };
                let written = path
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

    pub fn push_outgoing(&mut self, o: Outgoing) {
        self.outgoing.push(o);
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
        u.print_debug(b"Connecting to 127.0.0.1:7777...");
        let g = samp_proto::session::InitGame { server_name: "srv".into(), player_id: 0, ..Default::default() };
        u.on_init_game(&g);
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
    fn net_stats_text_matches_the_measured_labels() {
        let t = String::from_utf8(net_stats_text(&NetStats { send_rate: 30, ..NetStats::default() })).unwrap();
        let lines: Vec<&str> = t.split('\n').collect();
        assert_eq!(lines.len(), 27, "{t}");
        assert_eq!(lines[0], "Download Rate: 0.00 KB/s");
        assert_eq!(lines[10], "OnFoot Send Rate: 30");
        assert_eq!(lines[11], "Streaming Mem: 0MB Total: 0MB System: 0MB");
        assert_eq!(lines[21], "Packetloss: 0.0%");
        assert_eq!(lines[25], "Duplicate acks received: 0");
        assert_eq!(lines[26], "");
    }

    #[test]
    fn clipboard_requests_from_the_input() {
        let mut u = UiCore::new(None, false);
        u.netgame = true;
        u.apply(&[Action::OpenInput], &Game, 3);
        u.apply(&[Action::InputChar(0x16)], &Game, 3);
        assert!(std::mem::take(&mut u.requests.paste));
        u.paste(b"clip text 1");
        assert_eq!(u.input.edit().text(), b"clip text 1");
        // Nothing selected: Ctrl+C copies nothing (DXUT CopyToClipboard).
        u.apply(&[Action::InputChar(0x03)], &Game, 3);
        assert_eq!(u.requests.clipboard_copy, None);
        u.edit_key(EditKey::Home, Mods { shift: true, ctrl: false });
        u.apply(&[Action::InputChar(0x03)], &Game, 3);
        assert_eq!(u.requests.clipboard_copy.take(), Some(b"clip text 1".to_vec()));
        u.edit_key(EditKey::Insert, Mods { shift: true, ctrl: false });
        assert!(u.requests.paste);
    }

    #[test]
    fn screenshot_index_is_the_first_free_name() {
        let d = tmpdir("shots");
        let mut u = UiCore::new(Some(d.clone()), false);
        std::fs::create_dir_all(d.join("screens")).unwrap();
        assert_eq!(u.screenshot_target(), (d.join("screens").join("sa-mp-000.png"), 0));
        std::fs::write(d.join("screens").join("sa-mp-000.png"), b"x").unwrap();
        std::fs::write(d.join("screens").join("sa-mp-002.png"), b"x").unwrap();
        assert_eq!(u.screenshot_target().1, 1);
        u.screenshot_done(Some(1));
        assert_eq!(u.chat.last().unwrap().text, b"Screenshot Taken - sa-mp-001.png");
        assert_eq!(u.chat.last().unwrap().kind, EntryType::Info);
        u.screenshot_done(None);
        assert_eq!(u.chat.last().unwrap().text, b"Unable to save screenshot.");
        assert_eq!(u.chat.last().unwrap().kind, EntryType::Debug);
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
