//! Debug control channel: a non-blocking TCP line protocol on 127.0.0.1,
//! serviced by polling (from the per-frame tick in the game).
//!
//! Commands (one per line, replies are one line):
//! * `ping`  → `pong`
//! * `state` → game state and local player position/heading/health/armour
//! * `quit`  → `bye`, then the game is asked to exit
//! * `peek ADDR LEN` → `peek ADDR HEXBYTES` (LEN ≤ 256; ADDR hex with 0x or decimal)
//! * `net`   → connection phase, player id, last RPC ids, packet counters
//! * `remote` → `remote count=N …` followed by N lines, one per remote player
//!   (id, name, ped slot, ped pointer, position, heading, health, last sync age)
//! * `files` → `files mode=archive|fallback …` followed by one line per
//!   samp.saa entry served so far (name, hash, size, opens, first path)
//! * `vehicles` → `vehicles count=N …` followed by N lines, one per streamed vehicle (id, model, script
//!   handle, CVehicle pointer, position, heading, health, driver player id / `local`, local occupancy)
//! * `world` → `world count=N objects=… pickups=… actors=… …` followed by N lines, one per world entity
//!   (objects, attached objects, pickups, actors, checkpoints, gang zones, map icons, 3D labels)
//! * `spawn` → debugging aid: run the local spawn routine with the stored
//!   spawn info (as if the server had answered RPC 129); not used by tests
//! * `chat [N]` → `chat count=K …` header plus K lines `line type=… colour=…
//!   name=… text=… runs=[…]` (the newest K ≤ N chat entries, default 20)
//! * `input` → chat input state (`input open=… text=… caret=… …`)
//! * `say TEXT` → debugging aid: TEXT is handled as if typed into the chat
//!   input and sent with Enter (next frame); replies `say queued`
//! * anything else → `error unknown command`

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

pub const DEFAULT_PORT: u16 = 47700;
/// Environment variable overriding [`DEFAULT_PORT`] (e.g. 47701 in the trace lane).
pub const PORT_ENV: &str = "SAMP_RS_CTL_PORT";

/// The control port: `SAMP_RS_CTL_PORT` if set to a valid port, else 47700.
pub fn port_from_env() -> u16 {
    parse_port(std::env::var(PORT_ENV).ok().as_deref())
}

fn parse_port(v: Option<&str>) -> u16 {
    v.and_then(|s| s.trim().parse::<u16>().ok()).filter(|&p| p != 0).unwrap_or(DEFAULT_PORT)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ping,
    State,
    Net,
    Remote,
    Files,
    Vehicles,
    /// Combat counters (M7).
    Combat,
    World,
    /// `behaviour`: camera / spectating / bounds / drunk / animation state (M8b).
    Behaviour,
    Spawn,
    Quit,
    Chat(usize),
    Input,
    /// `ui`: dialog / scoreboard / textdraw / menu / death window state.
    Ui,
    Say(String),
    /// `audio`: audio stream / PlayerPlaySound state (M9).
    Audio,
    Peek { addr: u32, len: usize },
    /// `devreset [frames]`: test hook, the device reports "lost, reset possible" for that many
    /// `TestCooperativeLevel` calls (default 15), so RenderWare runs its lost-device path (CRASH-1).
    DevReset(u32),
    Unknown(String),
}

pub const MAX_PEEK: usize = 256;

fn parse_num(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

pub fn parse(line: &str) -> Option<Command> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if t.len() > 4 && t[..4].eq_ignore_ascii_case("say ") {
        return Some(Command::Say(t[4..].to_string()));
    }
    let lower = t.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if let ["peek", a, n] = words.as_slice()
        && let (Some(addr), Some(len)) = (parse_num(a), parse_num(n))
        && addr <= u64::from(u32::MAX)
        && (1..=MAX_PEEK as u64).contains(&len)
    {
        return Some(Command::Peek { addr: addr as u32, len: len as usize });
    }
    match words.as_slice() {
        ["devreset"] => return Some(Command::DevReset(15)),
        ["devreset", n] if n.parse::<u32>().is_ok() => return Some(Command::DevReset(n.parse::<u32>().unwrap_or(15).min(600))),
        _ => {}
    }
    if let ["chat", n] = words.as_slice()
        && let Ok(n) = n.parse::<usize>()
    {
        return Some(Command::Chat(n.min(100)));
    }
    Some(match lower.as_str() {
        "ping" => Command::Ping,
        "state" => Command::State,
        "net" => Command::Net,
        "remote" => Command::Remote,
        "files" => Command::Files,
        "vehicles" => Command::Vehicles,
        "combat" => Command::Combat,
        "world" => Command::World,
        "behaviour" => Command::Behaviour,
        "spawn" => Command::Spawn,
        "chat" => Command::Chat(20),
        "input" => Command::Input,
        "ui" => Command::Ui,
        "audio" => Command::Audio,
        "quit" | "exit" => Command::Quit,
        _ => Command::Unknown(t.to_string()),
    })
}

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    peer: SocketAddr,
}

pub struct Server {
    listener: TcpListener,
    clients: Vec<Client>,
}

/// Lines longer than this without a newline drop the client.
const MAX_LINE: usize = 4096;

impl Server {
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, clients: Vec::new() })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts new connections, reads complete lines and answers each with
    /// `handle`. Never blocks. Returns the commands that were handled.
    pub fn poll(&mut self, mut handle: impl FnMut(&Command) -> String) -> Vec<Command> {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if stream.set_nonblocking(true).is_ok() {
                        let _ = stream.set_nodelay(true);
                        crate::log!("control: client {peer} connected");
                        self.clients.push(Client { stream, buf: Vec::new(), peer });
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    crate::log!("control: accept error: {e}");
                    break;
                }
            }
        }

        let mut handled = Vec::new();
        self.clients.retain_mut(|c| {
            let mut alive = true;
            let mut tmp = [0u8; 1024];
            loop {
                match c.stream.read(&mut tmp) {
                    Ok(0) => {
                        alive = false;
                        break;
                    }
                    Ok(n) => c.buf.extend_from_slice(&tmp[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        alive = false;
                        break;
                    }
                }
            }
            while let Some(nl) = c.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = c.buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&line);
                if let Some(cmd) = parse(&line) {
                    let mut reply = handle(&cmd);
                    reply.push('\n');
                    // Replies are short; a blocking write of a few bytes into an
                    // empty socket buffer does not stall the frame.
                    let _ = c.stream.set_nonblocking(false);
                    if c.stream.write_all(reply.as_bytes()).is_err() {
                        alive = false;
                    }
                    let _ = c.stream.set_nonblocking(true);
                    handled.push(cmd);
                }
            }
            if c.buf.len() > MAX_LINE {
                alive = false;
            }
            if !alive {
                crate::log!("control: client {} disconnected", c.peer);
            }
            alive
        });
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    #[test]
    fn parse_commands() {
        assert_eq!(parse("ping\r\n"), Some(Command::Ping));
        assert_eq!(parse(" STATE "), Some(Command::State));
        assert_eq!(parse("quit"), Some(Command::Quit));
        assert_eq!(parse("net"), Some(Command::Net));
        assert_eq!(parse("remote"), Some(Command::Remote));
        assert_eq!(parse("FILES"), Some(Command::Files));
        assert_eq!(parse("world"), Some(Command::World));
        assert_eq!(parse("behaviour"), Some(Command::Behaviour));
        assert_eq!(parse("Spawn\n"), Some(Command::Spawn));
        assert_eq!(parse(""), None);
        assert_eq!(parse("chat"), Some(Command::Chat(20)));
        assert_eq!(parse("chat 5"), Some(Command::Chat(5)));
        assert_eq!(parse("input"), Some(Command::Input));
        assert_eq!(parse("say Hello World"), Some(Command::Say("Hello World".into())));
        assert_eq!(parse("SAY /pagesize 15\n"), Some(Command::Say("/pagesize 15".into())));
        assert_eq!(parse("foo bar"), Some(Command::Unknown("foo bar".into())));
        assert_eq!(parse("peek 0x53EB12 5"), Some(Command::Peek { addr: 0x53EB12, len: 5 }));
        assert_eq!(parse("peek 16 1"), Some(Command::Peek { addr: 16, len: 1 }));
        assert_eq!(parse("devreset"), Some(Command::DevReset(15)));
        assert_eq!(parse("DevReset 40"), Some(Command::DevReset(40)));
        assert!(matches!(parse("peek 0x10 999"), Some(Command::Unknown(_))));
        assert!(matches!(parse("peek 0x1ffffffff 1"), Some(Command::Unknown(_))));
    }

    #[test]
    fn port_override() {
        assert_eq!(parse_port(None), DEFAULT_PORT);
        assert_eq!(parse_port(Some("47701")), 47701);
        assert_eq!(parse_port(Some("0")), DEFAULT_PORT);
        assert_eq!(parse_port(Some("nope")), DEFAULT_PORT);
    }

    #[test]
    fn serves_lines_over_tcp() {
        let mut srv = Server::bind(0).unwrap();
        let addr = srv.local_addr().unwrap();
        let mut c = TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Two commands in one write, the second split across writes.
        c.write_all(b"ping\nsta").unwrap();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut sent_rest = false;
        while got.len() < 2 && Instant::now() < deadline {
            got.extend(srv.poll(|cmd| match cmd {
                Command::Ping => "pong".into(),
                Command::State => "state ok".into(),
                _ => "error".into(),
            }));
            if !got.is_empty() && !sent_rest {
                c.write_all(b"te\n").unwrap();
                sent_rest = true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got, vec![Command::Ping, Command::State]);
        let mut r = BufReader::new(c);
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "pong\n");
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "state ok\n");
    }
}
