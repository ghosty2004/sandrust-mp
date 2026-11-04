//! Local `/commands` of the chat input: table, parser and effects (UCA "Local chat
//! input and built-in commands", UR §9).
//!
//! Lookup is case-insensitive (A 0x10069710). The debug-only commands are registered
//! only when the client runs with `-d`. Printed texts are the client's own strings;
//! all command feedback is a type-4 info line (UR §7.1) except "I don't know that
//! command." (type 8).

use crate::chat::PAGE_SIZE_COMMAND_RANGE;
use crate::config::{FPS_LIMIT_RANGE, SampCfg, UiSettings, keys};
use crate::fonts::FONT_SIZE_RANGE;
use crate::text::{atoi, scan_ints};

/// Command names are shorter than this (UCA).
pub const MAX_NAME_LEN: usize = 32;

/// Printed texts (samp.dll string table, A ≈ 0x100e9e00..0x100ea2ec).
pub mod msg {
    pub const PAGESIZE_USAGE: &str = "pagesize [10-20] (lines)";
    pub const FONTSIZE_USAGE: &str = "Valid fontsize: -3 to 5";
    pub const FPSLIMIT_USAGE: &str = "-> Frame Limiter: valid amounts are 20-90";
    pub const HEAD_MOVE_ENABLED: &str = "-> Head movements enabled";
    pub const HEAD_MOVE_DISABLED: &str = "-> Head movements disabled";
    pub const NAMETAG_STATUS_ON: &str = "NameTag Player Status: ON";
    pub const NAMETAG_STATUS_OFF: &str = "NameTag Player Status: OFF";
    pub const AUDIO_MESSAGES_ON: &str = "Audio messages: On";
    pub const AUDIO_MESSAGES_OFF: &str = "Audio messages: Off";
    pub const URL_MESSAGES_ON: &str = "URL messages: On";
    pub const URL_MESSAGES_OFF: &str = "URL messages: Off";
    pub const SAVED_IN_CAR: &str = "-> InCar position saved";
    pub const SAVED_ON_FOOT: &str = "-> OnFoot position saved";
    pub const SAVE_OPEN_FAILED: &str = "I can't open the savepositions.txt file for append.";
    pub const RAW_SAVED_IN_CAR: &str = "-> InCar pos saved";
    pub const RAW_SAVED_ON_FOOT: &str = "-> OnFoot pos saved";
    /// Unknown (notes U-RS-ERROR): the only raw-save error string; assumed for both files.
    pub const RAW_OPEN_FAILED: &str = "I can't open the rawvehicles.txt file for append.";
    pub const UNKNOWN_COMMAND: &str = "I don't know that command.";
    pub const USAGE_V: &str = "Usage: /v (vehicle id).";
    pub const USAGE_PLAYER_SKIN: &str = "Usage: player_skin (skin number).";
    pub const USAGE_SET_WEATHER: &str = "Usage: set_weather (weather number)";
    pub const USAGE_SET_TIME: &str = "Usage: set_time (hour) (minute)";
    pub const INVALID_TIME: &str = "Invalid Time. Use /set_time (hour 0-23) (minute 0-59)";

    pub fn frame_limiter(n: u32) -> String {
        format!("-> Frame Limiter: {n}")
    }
    pub fn memory(v: u32) -> String {
        format!("Memory: {v}")
    }
    pub fn interior(v: u32) -> String {
        format!("Current Interior: {v}")
    }
    /// Widescreen RPC 111 debug line (UCA §111).
    pub fn widescreen(v: i32) -> String {
        format!("Widescreen = {v}")
    }
    /// Audio stream line (UCA §41), suppressed by `/audiomsg`.
    pub fn audio_stream(url: &str) -> String {
        format!("Audio stream: {url}")
    }
    /// Screenshot feedback (UR §6.4).
    pub fn screenshot_taken(n: u32) -> String {
        format!("Screenshot Taken - sa-mp-{n:03}.png")
    }
    pub const SCREENSHOT_FAILED: &str = "Unable to save screenshot.";
}

/// The registered commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandKind {
    Quit,
    Save,
    RawSave,
    Rcon,
    Mem,
    FpsLimit,
    PageSize,
    FontSize,
    NameTagStatus,
    Timestamp,
    HeadMove,
    HudScaleFix,
    AudioMsg,
    LogUrls,
    Interior,
    TogObjLight,
    Dl,
    Ctd,
    CmpStat,
    TestDw,
    // Debug mode (`-d`) only:
    Vsel,
    V,
    Vehicle,
    PlayerSkin,
    SetWeather,
    SetTime,
}

/// Name → command, `true` = registered only in debug mode (UCA table).
pub const TABLE: &[(&str, CommandKind, bool)] = &[
    ("quit", CommandKind::Quit, false),
    ("q", CommandKind::Quit, false),
    ("save", CommandKind::Save, false),
    ("rs", CommandKind::RawSave, false),
    ("rcon", CommandKind::Rcon, false),
    ("mem", CommandKind::Mem, false),
    ("fpslimit", CommandKind::FpsLimit, false),
    ("pagesize", CommandKind::PageSize, false),
    ("fontsize", CommandKind::FontSize, false),
    ("nametagstatus", CommandKind::NameTagStatus, false),
    ("timestamp", CommandKind::Timestamp, false),
    ("headmove", CommandKind::HeadMove, false),
    ("hudscalefix", CommandKind::HudScaleFix, false),
    ("audiomsg", CommandKind::AudioMsg, false),
    ("logurls", CommandKind::LogUrls, false),
    ("interior", CommandKind::Interior, false),
    ("togobjlight", CommandKind::TogObjLight, false),
    ("dl", CommandKind::Dl, false),
    ("ctd", CommandKind::Ctd, false),
    ("cmpstat", CommandKind::CmpStat, false),
    ("testdw", CommandKind::TestDw, false),
    ("vsel", CommandKind::Vsel, true),
    ("v", CommandKind::V, true),
    ("vehicle", CommandKind::Vehicle, true),
    ("player_skin", CommandKind::PlayerSkin, true),
    ("set_weather", CommandKind::SetWeather, true),
    ("set_time", CommandKind::SetTime, true),
];

/// Case-insensitive lookup of a command word.
pub fn lookup(word: &[u8], debug_mode: bool) -> Option<CommandKind> {
    if word.len() >= MAX_NAME_LEN {
        return None;
    }
    TABLE
        .iter()
        .find(|(name, _, dbg)| (debug_mode || !dbg) && name.as_bytes().eq_ignore_ascii_case(word))
        .map(|&(_, k, _)| k)
}

/// Splits a `/` line: command word = after `/` up to the first space; arguments = the
/// rest after that space (or empty). `None` if the line does not start with `/`.
pub fn split_command(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let body = line.strip_prefix(b"/")?;
    Some(match body.iter().position(|&b| b == b' ') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, &b""[..]),
    })
}

/// A local command with its argument string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalCommand {
    pub kind: CommandKind,
    pub args: Vec<u8>,
}

impl LocalCommand {
    /// Parses a typed line into a local command, if its word is registered.
    pub fn parse(line: &[u8], debug_mode: bool) -> Option<Self> {
        let (word, args) = split_command(line)?;
        lookup(word, debug_mode).map(|kind| Self { kind, args: args.to_vec() })
    }
}

/// Chat line kind for printed feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// Type 4, colour 0xFF88AA62.
    Info,
    /// Type 8, colour 0xFFA9C4E4.
    Debug,
}

/// Where a saved position goes (user files directory).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFile {
    SavedPositions,
    RawVehicles,
    RawPositions,
}

impl SaveFile {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::SavedPositions => "savedpositions.txt",
            Self::RawVehicles => "rawvehicles.txt",
            Self::RawPositions => "rawpositions.txt",
        }
    }
}

/// Local player state `/save` and `/rs` need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerPosition {
    OnFoot { skin: u32, x: f32, y: f32, z: f32, heading: f32 },
    InCar { model: u32, x: f32, y: f32, z: f32, heading: f32, colour1: u32, colour2: u32 },
}

/// Game queries the commands need (supplied by the host).
pub trait GameInfo {
    /// A netgame exists and is connected (used by `/quit`).
    fn connected(&self) -> bool;
    /// The u32 at GTA 0x8A5A80 (`/mem`).
    fn memory_counter(&self) -> u32;
    /// Active interior from script opcode 0x077E (`/interior`).
    fn interior(&self) -> u32;
    /// Local player position, or `None` if there is no local ped.
    fn position(&self) -> Option<PlayerPosition>;
}

/// Debug-mode actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugAction {
    VehicleSelector,
    SpawnVehicle(i32),
    PlayerSkin(i32),
    SetWeather(i32),
    SetTime { hour: i32, minute: i32 },
}

/// What a command asks the host to do. Settings are already updated in
/// [`UiSettings`] / [`SampCfg`] when these are returned.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Print(MessageKind, Vec<u8>),
    /// `sa-mp.cfg` changed: rewrite it from the [`SampCfg`].
    WriteConfig,
    /// Disconnect first (RakNet, 500 ms wait) if connected, then quit the game.
    Quit {
        disconnect: bool,
    },
    /// Send packet 201 (PlayerRconCommand) with this text.
    SendRcon(Vec<u8>),
    /// Append `line` to `file`; print `ok` on success, `failed` if it cannot be opened.
    AppendFile {
        file: SaveFile,
        line: Vec<u8>,
        ok: Vec<u8>,
        /// Line kind of `ok`: `/save` prints type 4, `/rs` type 8. `failed` is always type 8.
        ok_kind: MessageKind,
        failed: Vec<u8>,
    },
    SetChatPageSize(u32),
    /// Rebuild font renderer, chat metrics, scrollbar and death-window fonts.
    RebuildFonts(i32),
    SetTimestamps(bool),
    SetFpsLimit(u32),
    SetHeadMoveDisabled(bool),
    SetHudScaleFix(bool),
    SetNameTagStatus(bool),
    SetObjectLightFlag(bool),
    SetVehicleLabels(bool),
    SetCameraTargetLabels(bool),
    /// `/testdw`: fill the death window with the client's test rows.
    FillTestDeathWindow,
    Debug(DebugAction),
}

/// Type-4 line (helper A 0x10068070).
fn info(s: impl Into<String>) -> Effect {
    Effect::Print(MessageKind::Info, s.into().into_bytes())
}

/// Type-8 line (helper A 0x100680f0). Which feedback string goes through which helper
/// was read from the call after each string push in the 2021 build.
fn debug(s: impl Into<String>) -> Effect {
    Effect::Print(MessageKind::Debug, s.into().into_bytes())
}

fn persist(cfg: &mut SampCfg, out: &mut Vec<Effect>, key: &str, v: i32) {
    cfg.set_int(key, v);
    out.push(Effect::WriteConfig);
}

/// `AddStaticVehicle(...)` / `AddPlayerClass(...)` line of `/save` (format strings A ≈
/// 0x100e9f78 / 0x100e9fcc). Floats `%.4f`, ints `%u`.
pub fn save_line(pos: &PlayerPosition, comment: &[u8]) -> Vec<u8> {
    let mut s = match *pos {
        PlayerPosition::InCar { model, x, y, z, heading, colour1, colour2 } => format!(
            "AddStaticVehicle({model},{:.4},{:.4},{:.4},{:.4},{colour1},{colour2}); // ",
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(heading)
        ),
        PlayerPosition::OnFoot { skin, x, y, z, heading } => format!(
            "AddPlayerClass({skin},{:.4},{:.4},{:.4},{:.4},0,0,0,0,0,0); // ",
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(heading)
        ),
    }
    .into_bytes();
    s.extend_from_slice(comment);
    s.push(b'\n');
    s
}

/// `/rs` line: `model,x,y,z,heading,col1,col2 ; comment` or `x,y,z,heading ; comment`.
pub fn raw_save_line(pos: &PlayerPosition, comment: &[u8]) -> Vec<u8> {
    let mut s = match *pos {
        PlayerPosition::InCar { model, x, y, z, heading, colour1, colour2 } => format!(
            "{model},{:.4},{:.4},{:.4},{:.4},{colour1},{colour2} ; ",
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(heading)
        ),
        PlayerPosition::OnFoot { x, y, z, heading, .. } => {
            format!("{:.4},{:.4},{:.4},{:.4} ; ", f64::from(x), f64::from(y), f64::from(z), f64::from(heading))
        }
    }
    .into_bytes();
    s.extend_from_slice(comment);
    s.push(b'\n');
    s
}

/// Runs a local command: updates settings and config, returns host effects.
pub fn execute(cmd: &LocalCommand, settings: &mut UiSettings, cfg: &mut SampCfg, game: &dyn GameInfo) -> Vec<Effect> {
    let mut out = Vec::new();
    let args = cmd.args.as_slice();
    match cmd.kind {
        CommandKind::Quit => out.push(Effect::Quit { disconnect: game.connected() }),
        CommandKind::Save | CommandKind::RawSave => {
            // Unknown (notes U-SAVE-NOPED): behaviour without a local ped; nothing is done.
            if let Some(pos) = game.position() {
                let in_car = matches!(pos, PlayerPosition::InCar { .. });
                let e = if cmd.kind == CommandKind::Save {
                    Effect::AppendFile {
                        file: SaveFile::SavedPositions,
                        line: save_line(&pos, args),
                        ok: if in_car { msg::SAVED_IN_CAR } else { msg::SAVED_ON_FOOT }.into(),
                        ok_kind: MessageKind::Info,
                        failed: msg::SAVE_OPEN_FAILED.into(),
                    }
                } else {
                    Effect::AppendFile {
                        file: if in_car { SaveFile::RawVehicles } else { SaveFile::RawPositions },
                        line: raw_save_line(&pos, args),
                        ok: if in_car { msg::RAW_SAVED_IN_CAR } else { msg::RAW_SAVED_ON_FOOT }.into(),
                        ok_kind: MessageKind::Debug,
                        failed: msg::RAW_OPEN_FAILED.into(),
                    }
                };
                out.push(e);
            }
        }
        CommandKind::Rcon => out.push(Effect::SendRcon(args.to_vec())),
        CommandKind::Mem => out.push(debug(msg::memory(game.memory_counter()))),
        CommandKind::Interior => out.push(debug(msg::interior(game.interior()))),
        CommandKind::FpsLimit => {
            let n = atoi(args);
            if FPS_LIMIT_RANGE.contains(&n) {
                settings.fps_limit = Some(n as u32);
                out.push(Effect::SetFpsLimit(n as u32));
                persist(cfg, &mut out, keys::FPSLIMIT, n);
                out.push(info(msg::frame_limiter(n as u32)));
            } else {
                out.push(info(msg::FPSLIMIT_USAGE));
            }
        }
        CommandKind::PageSize => {
            let n = atoi(args);
            if n >= 0 && PAGE_SIZE_COMMAND_RANGE.contains(&(n as u32)) {
                settings.page_size = n as u32;
                out.push(Effect::SetChatPageSize(n as u32));
                persist(cfg, &mut out, keys::PAGESIZE, n);
            } else {
                out.push(debug(msg::PAGESIZE_USAGE));
            }
        }
        CommandKind::FontSize => {
            let n = atoi(args);
            if FONT_SIZE_RANGE.contains(&n) {
                settings.font_size = n;
                persist(cfg, &mut out, keys::FONTSIZE, n);
                out.push(Effect::RebuildFonts(n));
            } else {
                out.push(debug(msg::FONTSIZE_USAGE));
            }
        }
        CommandKind::Timestamp => {
            settings.timestamps = !settings.timestamps;
            out.push(Effect::SetTimestamps(settings.timestamps));
            persist(cfg, &mut out, keys::TIMESTAMP, settings.timestamps.into());
        }
        CommandKind::HeadMove => {
            settings.head_move_disabled = !settings.head_move_disabled;
            let d = settings.head_move_disabled;
            out.push(Effect::SetHeadMoveDisabled(d));
            persist(cfg, &mut out, keys::DISABLEHEADMOVE, d.into());
            out.push(info(if d { msg::HEAD_MOVE_DISABLED } else { msg::HEAD_MOVE_ENABLED }));
        }
        CommandKind::HudScaleFix => {
            settings.hud_scale_fix = !settings.hud_scale_fix;
            out.push(Effect::SetHudScaleFix(settings.hud_scale_fix));
            persist(cfg, &mut out, keys::NOHUDSCALEFIX, (!settings.hud_scale_fix).into());
        }
        CommandKind::NameTagStatus => {
            settings.nametag_status = !settings.nametag_status;
            let on = settings.nametag_status;
            out.push(Effect::SetNameTagStatus(on));
            persist(cfg, &mut out, keys::NONAMETAGSTATUS, (!on).into());
            out.push(info(if on { msg::NAMETAG_STATUS_ON } else { msg::NAMETAG_STATUS_OFF }));
        }
        CommandKind::AudioMsg => {
            settings.audio_messages_off = !settings.audio_messages_off;
            let off = settings.audio_messages_off;
            persist(cfg, &mut out, keys::AUDIOMSGOFF, off.into());
            out.push(info(if off { msg::AUDIO_MESSAGES_OFF } else { msg::AUDIO_MESSAGES_ON }));
        }
        CommandKind::LogUrls => {
            settings.log_urls = !settings.log_urls;
            let on = settings.log_urls;
            persist(cfg, &mut out, keys::LOGURLS, on.into());
            out.push(info(if on { msg::URL_MESSAGES_ON } else { msg::URL_MESSAGES_OFF }));
        }
        CommandKind::TogObjLight => {
            settings.object_light_flag = !settings.object_light_flag;
            out.push(Effect::SetObjectLightFlag(settings.object_light_flag));
        }
        CommandKind::Dl => {
            settings.vehicle_labels = !settings.vehicle_labels;
            out.push(Effect::SetVehicleLabels(settings.vehicle_labels));
        }
        CommandKind::Ctd => {
            settings.camera_target_labels = !settings.camera_target_labels;
            out.push(Effect::SetCameraTargetLabels(settings.camera_target_labels));
        }
        CommandKind::CmpStat => {}
        CommandKind::TestDw => out.push(Effect::FillTestDeathWindow),
        CommandKind::Vsel => out.push(Effect::Debug(DebugAction::VehicleSelector)),
        // Unknown (notes U-DEBUGCMDS): argument checks of the debug commands.
        CommandKind::V | CommandKind::Vehicle => match scan_ints(args, 1).first() {
            Some(&id) => out.push(Effect::Debug(DebugAction::SpawnVehicle(id))),
            None => out.push(debug(msg::USAGE_V)),
        },
        CommandKind::PlayerSkin => match scan_ints(args, 1).first() {
            Some(&id) => out.push(Effect::Debug(DebugAction::PlayerSkin(id))),
            None => out.push(debug(msg::USAGE_PLAYER_SKIN)),
        },
        CommandKind::SetWeather => match scan_ints(args, 1).first() {
            Some(&id) => out.push(Effect::Debug(DebugAction::SetWeather(id))),
            None => out.push(debug(msg::USAGE_SET_WEATHER)),
        },
        CommandKind::SetTime => match scan_ints(args, 2)[..] {
            [h, m] if (0..=23).contains(&h) && (0..=59).contains(&m) => {
                out.push(Effect::Debug(DebugAction::SetTime { hour: h, minute: m }))
            }
            [_, _] => out.push(debug(msg::INVALID_TIME)),
            _ => out.push(debug(msg::USAGE_SET_TIME)),
        },
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Game(Option<PlayerPosition>);
    impl GameInfo for Game {
        fn connected(&self) -> bool {
            true
        }
        fn memory_counter(&self) -> u32 {
            1234
        }
        fn interior(&self) -> u32 {
            3
        }
        fn position(&self) -> Option<PlayerPosition> {
            self.0
        }
    }

    fn run(line: &[u8], s: &mut UiSettings, cfg: &mut SampCfg) -> Vec<Effect> {
        let cmd = LocalCommand::parse(line, false).expect("local command");
        execute(&cmd, s, cfg, &Game(None))
    }

    fn printed(e: &[Effect]) -> Vec<String> {
        e.iter()
            .filter_map(|e| match e {
                Effect::Print(_, t) => Some(String::from_utf8_lossy(t).into_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn uca_local_input_table_lookup_case_insensitive_debug_gated() {
        assert_eq!(lookup(b"Q", false), Some(CommandKind::Quit));
        assert_eq!(lookup(b"QUIT", false), Some(CommandKind::Quit));
        assert_eq!(lookup(b"PageSize", false), Some(CommandKind::PageSize));
        assert_eq!(lookup(b"set_time", false), None);
        assert_eq!(lookup(b"set_time", true), Some(CommandKind::SetTime));
        assert_eq!(lookup(b"help", false), None);
        assert_eq!(TABLE.iter().filter(|t| !t.2).count(), 21);
    }

    #[test]
    fn uca_local_input_word_and_args_split() {
        assert_eq!(split_command(b"/save my spot"), Some((&b"save"[..], &b"my spot"[..])));
        assert_eq!(split_command(b"/rs"), Some((&b"rs"[..], &b""[..])));
        assert_eq!(split_command(b"/"), Some((&b""[..], &b""[..])));
        assert_eq!(split_command(b"hi"), None);
        assert_eq!(LocalCommand::parse(b"/RCON login x", false).unwrap().args, b"login x");
    }

    #[test]
    fn ur_9_pagesize_10_to_20_persists() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        let e = run(b"/pagesize 20", &mut s, &mut c);
        assert!(e.contains(&Effect::SetChatPageSize(20)) && e.contains(&Effect::WriteConfig));
        assert_eq!(c.get_int("pagesize"), Some(20));
        for bad in [&b"/pagesize 21"[..], b"/pagesize 9", b"/pagesize", b"/pagesize x"] {
            assert_eq!(printed(&run(bad, &mut s, &mut c)), vec![msg::PAGESIZE_USAGE]);
        }
        assert_eq!(s.page_size, 20);
    }

    #[test]
    fn ur_9_fontsize_minus3_to_5() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        assert!(run(b"/fontsize -3", &mut s, &mut c).contains(&Effect::RebuildFonts(-3)));
        assert_eq!(c.get_int("fontsize"), Some(-3));
        assert_eq!(printed(&run(b"/fontsize 6", &mut s, &mut c)), vec![msg::FONTSIZE_USAGE]);
        assert_eq!(s.font_size, -3);
    }

    #[test]
    fn ur_9_fpslimit_20_to_90() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        let e = run(b"/fpslimit 60", &mut s, &mut c);
        assert_eq!(printed(&e), vec!["-> Frame Limiter: 60"]);
        assert_eq!(c.get_int("fpslimit"), Some(60));
        assert_eq!(printed(&run(b"/fpslimit 19", &mut s, &mut c)), vec![msg::FPSLIMIT_USAGE]);
        assert_eq!(printed(&run(b"/fpslimit 91", &mut s, &mut c)), vec![msg::FPSLIMIT_USAGE]);
    }

    #[test]
    fn ur_9_toggles_persist_keys_and_print() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        run(b"/timestamp", &mut s, &mut c);
        assert_eq!((s.timestamps, c.get_int("timestamp")), (true, Some(1)));
        assert_eq!(printed(&run(b"/headmove", &mut s, &mut c)), vec![msg::HEAD_MOVE_DISABLED]);
        assert_eq!(c.get_int("disableheadmove"), Some(1));
        assert_eq!(printed(&run(b"/headmove", &mut s, &mut c)), vec![msg::HEAD_MOVE_ENABLED]);
        // hudscalefix: default on, stored inverted.
        assert!(run(b"/hudscalefix", &mut s, &mut c).contains(&Effect::SetHudScaleFix(false)));
        assert_eq!(c.get_int("nohudscalefix"), Some(1));
        assert_eq!(printed(&run(b"/nametagstatus", &mut s, &mut c)), vec![msg::NAMETAG_STATUS_OFF]);
        assert_eq!(c.get_int("nonametagstatus"), Some(1));
        assert_eq!(printed(&run(b"/audiomsg", &mut s, &mut c)), vec![msg::AUDIO_MESSAGES_OFF]);
        assert_eq!(c.get_int("audiomsgoff"), Some(1));
        assert_eq!(printed(&run(b"/logurls", &mut s, &mut c)), vec![msg::URL_MESSAGES_ON]);
        assert_eq!(c.get_int("logurls"), Some(1));
        // Session-only toggles do not touch the config.
        let before = c.clone();
        assert_eq!(run(b"/dl", &mut s, &mut c), vec![Effect::SetVehicleLabels(true)]);
        assert_eq!(run(b"/ctd", &mut s, &mut c), vec![Effect::SetCameraTargetLabels(true)]);
        assert_eq!(run(b"/togobjlight", &mut s, &mut c), vec![Effect::SetObjectLightFlag(true)]);
        assert!(run(b"/cmpstat", &mut s, &mut c).is_empty());
        assert_eq!(run(b"/testdw", &mut s, &mut c), vec![Effect::FillTestDeathWindow]);
        assert_eq!(c, before);
    }

    #[test]
    fn uca_local_input_quit_rcon_mem_interior() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        assert_eq!(run(b"/q", &mut s, &mut c), vec![Effect::Quit { disconnect: true }]);
        assert_eq!(run(b"/rcon login pw", &mut s, &mut c), vec![Effect::SendRcon(b"login pw".to_vec())]);
        assert_eq!(run(b"/rcon", &mut s, &mut c), vec![Effect::SendRcon(Vec::new())]);
        assert_eq!(printed(&run(b"/mem", &mut s, &mut c)), vec!["Memory: 1234"]);
        assert_eq!(printed(&run(b"/interior", &mut s, &mut c)), vec!["Current Interior: 3"]);
    }

    #[test]
    fn uca_local_input_save_and_rs_line_formats() {
        let car = PlayerPosition::InCar { model: 411, x: 1.0, y: -2.5, z: 3.25, heading: 90.0, colour1: 1, colour2: 0 };
        let foot = PlayerPosition::OnFoot { skin: 0, x: 1.0, y: 2.0, z: 3.0, heading: 180.5 };
        assert_eq!(save_line(&car, b"car"), b"AddStaticVehicle(411,1.0000,-2.5000,3.2500,90.0000,1,0); // car\n");
        assert_eq!(save_line(&foot, b""), b"AddPlayerClass(0,1.0000,2.0000,3.0000,180.5000,0,0,0,0,0,0); // \n");
        assert_eq!(raw_save_line(&car, b"x"), b"411,1.0000,-2.5000,3.2500,90.0000,1,0 ; x\n");
        assert_eq!(raw_save_line(&foot, b"y"), b"1.0000,2.0000,3.0000,180.5000 ; y\n");
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        let cmd = LocalCommand::parse(b"/rs spot", false).unwrap();
        let e = execute(&cmd, &mut s, &mut c, &Game(Some(foot)));
        assert_eq!(
            e,
            vec![Effect::AppendFile {
                file: SaveFile::RawPositions,
                line: b"1.0000,2.0000,3.0000,180.5000 ; spot\n".to_vec(),
                ok: msg::RAW_SAVED_ON_FOOT.into(),
                ok_kind: MessageKind::Debug,
                failed: msg::RAW_OPEN_FAILED.into(),
            }]
        );
        let cmd = LocalCommand::parse(b"/save", false).unwrap();
        let e = execute(&cmd, &mut s, &mut c, &Game(Some(car)));
        assert!(
            matches!(&e[0], Effect::AppendFile { file: SaveFile::SavedPositions, ok, .. } if ok == msg::SAVED_IN_CAR.as_bytes())
        );
        assert_eq!(SaveFile::RawVehicles.file_name(), "rawvehicles.txt");
    }

    #[test]
    fn uca_local_input_debug_commands() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        let mut dbg = |l: &[u8]| execute(&LocalCommand::parse(l, true).unwrap(), &mut s, &mut c, &Game(None));
        assert_eq!(dbg(b"/set_time 12 30"), vec![Effect::Debug(DebugAction::SetTime { hour: 12, minute: 30 })]);
        assert_eq!(printed(&dbg(b"/set_time 24 0")), vec![msg::INVALID_TIME]);
        assert_eq!(printed(&dbg(b"/set_time 1")), vec![msg::USAGE_SET_TIME]);
        assert_eq!(dbg(b"/v 411"), vec![Effect::Debug(DebugAction::SpawnVehicle(411))]);
        assert_eq!(printed(&dbg(b"/vehicle")), vec![msg::USAGE_V]);
    }

    #[test]
    fn feedback_line_kinds_follow_the_original_helpers() {
        let (mut s, mut c) = (UiSettings::default(), SampCfg::default());
        let kind = |e: &[Effect]| {
            e.iter()
                .find_map(|x| match x {
                    Effect::Print(k, _) => Some(*k),
                    _ => None,
                })
                .expect("a printed line")
        };
        assert_eq!(kind(&run(b"/pagesize 5", &mut s, &mut c)), MessageKind::Debug);
        assert_eq!(kind(&run(b"/fontsize 9", &mut s, &mut c)), MessageKind::Debug);
        assert_eq!(kind(&run(b"/mem", &mut s, &mut c)), MessageKind::Debug);
        assert_eq!(kind(&run(b"/interior", &mut s, &mut c)), MessageKind::Debug);
        assert_eq!(kind(&run(b"/fpslimit 5", &mut s, &mut c)), MessageKind::Info);
        assert_eq!(kind(&run(b"/headmove", &mut s, &mut c)), MessageKind::Info);
    }
}
