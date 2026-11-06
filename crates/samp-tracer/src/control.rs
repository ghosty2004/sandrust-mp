//! Control channel, same line protocol as samp-client (`ping`, `state`,
//! `quit`, `peek ADDR LEN`, `remote`) plus tracer-only `mark TEXT` (writes a
//! marker record into the trace, used by scenario scripts to label steps).
//! `hideped on|off` hides the local player ped from rendering (DIFF-2 screenshots).
//! `remote` answers a header line plus one line per remote player ped, like
//! samp-client's (DIFF-3). `bubbles` / `synced` read samp.dll's chat bubbles and synced
//! health/armour (`crate::sampstate`), same reply format as samp-client's.
//!
//! Served from a background thread with blocking sockets, so it also answers
//! while the game is loading or paused.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47701;
pub const MAX_PEEK: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ping,
    State,
    Quit,
    Peek {
        addr: u32,
        len: usize,
    },
    Mark(String),
    Remote,
    /// `bubbles`: the chat bubbles samp holds per player (DIFF-3, `crate::sampstate`).
    Bubbles,
    /// `synced`: samp's synced health/armour per remote player (DIFF-3, `crate::sampstate`).
    Synced,
    /// `d3d off|on|ui|all|text [FRAMES]` (Direct3D call-stream logger, `crate::d3d`).
    D3d(String),
    /// `efx arm|watch|status|disarm …` (effect tracer, `crate::efx`).
    Efx(String),
    /// `poke ADDR HEXBYTES`: writes bytes (test harness only, e.g. to see whether a client
    /// rewrites a flag every frame).
    Poke {
        addr: u32,
        bytes: Vec<u8>,
    },
    /// `cov start NAME|dump PATH|status` (first-hit coverage, `crate::cov`). Arguments keep
    /// their case (paths, window names).
    Cov(String),
    /// `window [click ID]` (top-level windows of the process, FEAT-B; `win/window.rs`).
    Window(String),
    /// `hideped on|off`: leave the local player's ped out of rendering (DIFF-2 screenshots).
    HidePed(bool),
    Unknown(String),
}

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
    if let Some(rest) = t.strip_prefix("mark ").or_else(|| t.strip_prefix("MARK ")) {
        return Some(Command::Mark(rest.trim().to_string()));
    }
    if let Some(rest) = t.strip_prefix("cov ").or_else(|| t.strip_prefix("COV ")) {
        return Some(Command::Cov(rest.trim().to_string()));
    }
    let lower = t.to_ascii_lowercase();
    if lower == "window" || lower.starts_with("window ") {
        return Some(Command::Window(lower[6..].trim().to_string()));
    }
    if let Some(rest) = lower.strip_prefix("d3d ") {
        return Some(Command::D3d(rest.trim().to_string()));
    }
    if lower.starts_with("efx ") {
        // Original case: `efx watch` takes a file path.
        return Some(Command::Efx(t[4..].trim().to_string()));
    }
    if let ["poke", a, hex] = lower.split_whitespace().collect::<Vec<_>>().as_slice()
        && let Some(addr) = parse_num(a).filter(|&a| a <= u64::from(u32::MAX))
        && hex.len() % 2 == 0
        && (2..=2 * MAX_PEEK).contains(&hex.len())
        && let Some(bytes) = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()
    {
        return Some(Command::Poke { addr: addr as u32, bytes });
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    if let ["peek", a, n] = words.as_slice()
        && let (Some(addr), Some(len)) = (parse_num(a), parse_num(n))
        && addr <= u64::from(u32::MAX)
        && (1..=MAX_PEEK as u64).contains(&len)
    {
        return Some(Command::Peek { addr: addr as u32, len: len as usize });
    }
    Some(match lower.as_str() {
        "ping" => Command::Ping,
        "state" => Command::State,
        "remote" => Command::Remote,
        "bubbles" => Command::Bubbles,
        "synced" => Command::Synced,
        "hideped on" => Command::HidePed(true),
        "hideped off" => Command::HidePed(false),
        "quit" | "exit" => Command::Quit,
        _ => Command::Unknown(t.to_string()),
    })
}

/// Port from `SAMP_TRACER_PORT` (default 47701).
pub fn port_from_env(v: Option<&str>) -> u16 {
    v.and_then(|s| s.trim().parse().ok()).filter(|&p| p != 0).unwrap_or(DEFAULT_PORT)
}

/// Accepts connections forever; each line gets one reply line from `handle`.
/// `handle` returns (reply, stop_after_reply).
pub fn serve(listener: TcpListener, mut handle: impl FnMut(&Command) -> (String, bool)) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_nodelay(true);
        let Ok(mut w) = stream.try_clone() else { continue };
        let mut r = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if line.len() > 4096 {
                break;
            }
            let Some(cmd) = parse(&line) else { continue };
            let (mut reply, stop) = handle(&cmd);
            reply.push('\n');
            if w.write_all(reply.as_bytes()).is_err() {
                break;
            }
            if stop {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn parse_commands() {
        assert_eq!(parse("ping\r\n"), Some(Command::Ping));
        assert_eq!(parse(" STATE "), Some(Command::State));
        assert_eq!(parse("quit"), Some(Command::Quit));
        assert_eq!(parse("Remote\n"), Some(Command::Remote));
        assert_eq!(parse("bubbles"), Some(Command::Bubbles));
        assert_eq!(parse("SYNCED"), Some(Command::Synced));
        assert_eq!(parse("hideped on"), Some(Command::HidePed(true)));
        assert_eq!(parse("HIDEPED off"), Some(Command::HidePed(false)));
        assert!(matches!(parse("hideped"), Some(Command::Unknown(_))));
        assert_eq!(parse(""), None);
        assert_eq!(parse("mark class selection"), Some(Command::Mark("class selection".into())));
        assert_eq!(parse("peek 0x53EB12 5"), Some(Command::Peek { addr: 0x53EB12, len: 5 }));
        assert!(matches!(parse("peek 0x10 999"), Some(Command::Unknown(_))));
        assert_eq!(parse("d3d all 3"), Some(Command::D3d("all 3".into())));
        assert_eq!(parse("cov start Grove_Idle"), Some(Command::Cov("start Grove_Idle".into())));
        assert_eq!(parse("cov dump Z:\\x\\c.tsv\n"), Some(Command::Cov("dump Z:\\x\\c.tsv".into())));
        assert_eq!(parse("efx watch C:\\X.txt"), Some(Command::Efx("watch C:\\X.txt".into())));
        assert_eq!(parse("window"), Some(Command::Window(String::new())));
        assert_eq!(parse("window click 1001"), Some(Command::Window("click 1001".into())));
        assert_eq!(parse("poke 0xBA6769 00ff"), Some(Command::Poke { addr: 0xBA6769, bytes: vec![0, 0xFF] }));
        assert!(matches!(parse("poke 0xBA6769 0"), Some(Command::Unknown(_))));
        assert_eq!(port_from_env(Some("47701")), 47701);
        assert_eq!(port_from_env(Some("x")), DEFAULT_PORT);
        assert_eq!(port_from_env(None), DEFAULT_PORT);
    }

    #[test]
    fn serves_lines() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            serve(l, |c| match c {
                Command::Ping => ("pong".into(), false),
                Command::Quit => ("bye".into(), true),
                _ => ("error".into(), false),
            })
        });
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(b"ping\nquit\n").unwrap();
        let mut r = BufReader::new(s);
        let mut a = String::new();
        r.read_line(&mut a).unwrap();
        let mut b = String::new();
        r.read_line(&mut b).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("pong\n", "bye\n"));
        t.join().unwrap();
    }
}
