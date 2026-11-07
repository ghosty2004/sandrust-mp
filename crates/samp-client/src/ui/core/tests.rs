use super::*;
use samp_proto::rpcs::ui::{PlayerChatMessage, SendClientMessage, SendGameText};
use samp_proto::rpcs::{Incoming, player, remote};
use samp_ui::chat::EntryType;
use samp_ui::commands::{GameInfo, PlayerPosition};
use samp_ui::edit::{EditKey, Mods};
use samp_ui::keys::{Action, Event};
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
        if c == b'\r' {
            enter_down(u);
            enter_up(u);
        } else {
            u.handle_event(Event::Char(c), false, &Game, 3);
        }
    }
}

/// Enter as Windows delivers it: `WM_KEYDOWN`, then the `WM_CHAR` CR from TranslateMessage.
fn enter_down(u: &mut UiCore) {
    u.handle_event(Event::KeyDown(samp_ui::keys::Vk::Enter), false, &Game, 3);
    u.handle_event(Event::Char(b'\r'), false, &Game, 3);
}

fn enter_up(u: &mut UiCore) {
    u.handle_event(Event::KeyUp(samp_ui::keys::Vk::Enter), false, &Game, 3);
}

fn show_dialog(u: &mut UiCore, id: i16, style: u8) {
    let d = ui::ShowDialog {
        dialog_id: id,
        style,
        title: b"T".to_vec(),
        button1: b"OK".to_vec(),
        button2: b"Cancel".to_vec(),
        body: b"Body".to_vec(),
    };
    u.on_rpc(&Incoming::Ui(ui::Incoming::ShowDialog(d)), &NoExpansion);
}

/// The Enter that sends a chat line (on its key-up, like samp's edit box) must not answer the
/// dialog the server shows in reply, nor may auto-repeat or a dialog chain.
#[test]
fn chat_enter_does_not_answer_the_reply_dialog() {
    for style in [0u8, 1, 2, 3] {
        let mut u = connected_ui(None);
        typed(&mut u, b"t/dlg");
        enter_down(&mut u);
        // Key held: nothing is sent yet; auto-repeat (key-down + CR) sends nothing either.
        assert!(u.take_outgoing().is_empty());
        enter_down(&mut u);
        assert!(u.take_outgoing().is_empty());
        enter_up(&mut u);
        assert_eq!(u.take_outgoing(), vec![Outgoing::Command(b"/dlg".to_vec())]);
        // The reply dialog arrives after the key-up; a fast server reply racing the
        // auto-repeat of a still-held key must not answer it either.
        show_dialog(&mut u, 90, style);
        enter_down(&mut u);
        assert!(u.take_outgoing().is_empty(), "style {style}");
        assert!(u.dialogs.is_open());
        // The next real key-up answers it.
        enter_up(&mut u);
        assert!(matches!(u.take_outgoing().as_slice(), [Outgoing::DialogResponse(r)] if r.response == 1));
        // Dialog chain: the server shows the next dialog in OnDialogResponse.
        show_dialog(&mut u, 91, style);
        enter_down(&mut u);
        assert!(u.take_outgoing().is_empty() && u.dialogs.is_open(), "chain, style {style}");
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
    let r = Incoming::Ui(ui::Incoming::PlayerChatMessage(PlayerChatMessage { player_id: 5, message: b"x".to_vec() }));
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
    let r = Incoming::Ui(ui::Incoming::SendGameText(SendGameText { style: 3, time_ms: 5000, text: b"~r~hi".to_vec() }));
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
    let c =
        Incoming::Player(player::Incoming::SetPlayerColor(player::SetPlayerColor { player_id: 0, colour: 0xFF0000FF }));
    u.on_rpc(&c, &NoExpansion);
    assert_eq!(u.colours.chat_argb(0), 0xFFFF0000);
}
