//! Decode SA:MP 0.3.7 traffic from a capture into a normalized, diffable log.
//!
//! Usage: samp-dump CAPTURE [--port 7777] [--raw]
//!
//! One line per logical message. By default, transport noise (acks,
//! message numbers, ordering indices, internal pings, keep-alives, resends)
//! is dropped so that captures of two different clients performing the same
//! actions can be compared with `diff`. `--raw` keeps transport details.
//!
//! Client→server datagrams are de-obfuscated with the port-keyed cipher
//! (docs/re/handshake.md §2); server→client datagrams are plaintext.

mod capture;

use std::collections::HashSet;
use std::fmt::Write as _;

use samp_proto::bitstream::BitReader;
use samp_proto::cipher::{self, CipherKey};
use samp_proto::raknet::{Datagram, Message, id, strip_timestamp};
use samp_proto::{rpc, rpcs, sync};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    ClientToServer,
    ServerToClient,
}

impl Dir {
    fn arrow(self) -> &'static str {
        match self {
            Dir::ClientToServer => "C>S",
            Dir::ServerToClient => "S>C",
        }
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut path, mut port, mut raw) = (None, 7777u16, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).ok_or("--port needs a number")?,
            "--raw" => raw = true,
            _ => path = Some(a),
        }
    }
    let path = path.ok_or("usage: samp-dump CAPTURE [--port 7777] [--raw]")?;
    let packets = capture::read(&std::fs::read(&path)?)?;
    let key = CipherKey::from_port(port);

    let mut seen: [HashSet<u16>; 2] = [HashSet::new(), HashSet::new()];
    let mut plain = Vec::new();
    let mut lines = 0usize;
    let t0 = packets.first().map_or(0, |p| p.ts_us);

    for p in &packets {
        let dir = if p.dst.port() == port {
            Dir::ClientToServer
        } else if p.src.port() == port {
            Dir::ServerToClient
        } else {
            continue;
        };
        let body: &[u8] = if dir == Dir::ClientToServer {
            if cipher::decrypt(&p.payload, key, &mut plain).is_err() {
                println!("{} !! bad checksum ({} bytes)", dir.arrow(), p.payload.len());
                continue;
            }
            &plain
        } else {
            &p.payload
        };
        let prefix = if raw { format!("{:>9.3} ", (p.ts_us - t0) as f64 / 1e6) } else { String::new() };

        if let Some(line) = unconnected(dir, body, raw) {
            println!("{prefix}{} {line}", dir.arrow());
            lines += 1;
            continue;
        }
        let Ok(dg) = Datagram::decode(body) else {
            println!("{prefix}{} ?? undecodable datagram: {}", dir.arrow(), hex(body));
            continue;
        };
        if raw && !dg.acks.is_empty() {
            println!("{prefix}{} acks {:?}", dir.arrow(), dg.acks.iter().map(|a| (a.min, a.max)).collect::<Vec<_>>());
        }
        let seen = &mut seen[(dir == Dir::ServerToClient) as usize];
        for m in &dg.messages {
            // Reliable resends repeat the message number: show each once.
            if m.reliability.is_reliable() && !seen.insert(m.number) {
                continue;
            }
            if let Some(line) = describe(dir, m, raw) {
                println!("{prefix}{} {line}", dir.arrow());
                lines += 1;
            }
        }
    }
    eprintln!("{} UDP packets, {lines} messages", packets.len());
    Ok(())
}

fn unconnected(dir: Dir, b: &[u8], raw: bool) -> Option<String> {
    // --raw: the cookie a request carries (0x6969 ^ cookie on the wire) and the one a cookie reply issues.
    let cookie = |x: u16| if raw { format!(" cookie={x:#06x}") } else { String::new() };
    Some(match (dir, b) {
        (Dir::ClientToServer, [0x18, lo, hi]) => {
            format!("open-connection-request{}", cookie(u16::from_le_bytes([*lo, *hi]) ^ 0x6969))
        }
        (Dir::ServerToClient, [0x1a, lo, hi]) => {
            format!("open-connection-cookie{}", cookie(u16::from_le_bytes([*lo, *hi])))
        }
        (Dir::ServerToClient, [0x19, ..]) if b.len() <= 2 => "open-connection-reply".into(),
        (Dir::ServerToClient, [x @ (0x1d | 0x1f | 0x24), ..]) if b.len() <= 2 => {
            format!("open-connection-refused {x:#04x}")
        }
        _ => return None,
    })
}

fn describe(dir: Dir, m: &Message, raw: bool) -> Option<String> {
    let mut s = String::new();
    if raw {
        let _ = write!(s, "#{} rel{} ", m.number, m.reliability as u8);
        if let Some(o) = m.ordering {
            let _ = write!(s, "ch{}:{} ", o.channel, o.index);
        }
    } else {
        let _ = write!(s, "rel{} ", m.reliability as u8);
        if let Some(o) = m.ordering {
            let _ = write!(s, "ch{} ", o.channel);
        }
    }
    if let Some(sp) = m.split {
        let _ = write!(s, "split {}/{} of id {}: {} bits", sp.index + 1, sp.count, sp.id, m.bit_len);
        return Some(s);
    }
    let bits = usize::from(m.bit_len);
    let (_, body) = strip_timestamp(&m.data);
    let first = *body.first()?;
    match first {
        id::INTERNAL_PING | id::CONNECTED_PONG | id::KEEP_ALIVE if !raw => return None,
        id::RPC => {
            let Some(r) = rpc::decode(&m.data, bits) else {
                let _ = write!(s, "rpc ?? {}", hex(&m.data));
                return Some(s);
            };
            let _ = write!(s, "rpc {:3} ", r.id);
            match (dir, rpcs::decode(r.id, &mut r.reader())) {
                (Dir::ServerToClient, Some(decoded)) => {
                    let _ = write!(s, "{decoded:?}");
                }
                _ => {
                    let _ = write!(s, "{} bits {}", r.bit_len, hex(&r.payload));
                }
            }
        }
        0xc8..=0xd4 => match dir {
            Dir::ServerToClient => match sync::decode(&m.data, bits) {
                Some(p) => {
                    let _ = write!(s, "sync {p:?}");
                }
                None => {
                    let _ = write!(s, "sync {first} ?? {}", hex(body));
                }
            },
            Dir::ClientToServer => {
                let _ = write!(s, "sync {first} {} bytes {}", body.len() - 1, hex(&body[1..]));
            }
        },
        id::AUTH_KEY => {
            let mut r = BitReader::new(&body[1..]);
            let n = r.read_u8().unwrap_or(0);
            let txt = r.read_bytes(n.into()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let _ = write!(s, "auth-key len {n} {txt:?}");
        }
        _ => {
            let _ = write!(s, "msg {first:#04x} {}", hex(body));
        }
    }
    Some(s)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
}
