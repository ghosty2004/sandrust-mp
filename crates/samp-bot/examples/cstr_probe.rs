//! Compressed-string probe: joins a server (same flow as samp-bot), then
//! decodes ShowDialog (RPC 61) and PlayerShowTextLabel (RPC 36), prints the
//! Huffman-decoded strings, compares them with what the test filterscript
//! sends, and checks that `strcomp::write_compressed_string` reproduces the
//! wire bits exactly.
//!
//! Usage: cstr_probe [host:port] [--name NAME] [--seconds N] [--auth-table PATH]

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use samp_proto::auth::{self, AuthTable};
use samp_proto::bitstream::{BitReader, BitWriter};
use samp_proto::cipher::{self, CipherKey};
use samp_proto::connection::{Connection, Delivered};
use samp_proto::raknet::{Reliability, id, strip_timestamp};
use samp_proto::rpc::{self, ids};
use samp_proto::session;
use samp_proto::strcomp;
use samp_proto::unconnected::{ServerReply, open_connection_request, parse_reply};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Args {
    target: String,
    name: String,
    seconds: u64,
    auth_table: String,
}

fn parse_args() -> Args {
    let mut a = Args {
        target: "127.0.0.1:7791".into(),
        name: "RustBot".into(),
        seconds: 6,
        auth_table: "data/auth_table.txt".into(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => a.name = it.next().expect("--name needs a value"),
            "--seconds" => a.seconds = it.next().and_then(|s| s.parse().ok()).expect("--seconds needs a number"),
            "--auth-table" => a.auth_table = it.next().expect("--auth-table needs a path"),
            other => a.target = other.into(),
        }
    }
    a
}

struct Client {
    sock: UdpSocket,
    server: SocketAddr,
    key: CipherKey,
    wire: Vec<u8>,
}

impl Client {
    fn send_plain(&mut self, plain: &[u8]) -> Result<()> {
        cipher::encrypt(plain, self.key, &mut self.wire);
        self.sock.send(&self.wire)?;
        Ok(())
    }

    fn recv(&self, buf: &mut [u8], timeout: Duration) -> Result<Option<usize>> {
        self.sock.set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        match self.sock.recv(buf) {
            Ok(n) => Ok(Some(n)),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn main() -> Result<()> {
    let args = parse_args();
    let table = AuthTable::parse(&std::fs::read_to_string(&args.auth_table).map_err(|e| {
        format!("cannot read auth table {}: {e} (generate it with tools/extract_auth_table.py)", args.auth_table)
    })?)
    .map_err(|e| format!("auth table parse error at line {}", e.line))?;

    let server = args.target.to_socket_addrs()?.next().ok_or("could not resolve address")?;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(server)?;
    let mut client = Client { sock, server, key: CipherKey::from_port(server.port()), wire: Vec::new() };
    let mut buf = [0u8; 1500];

    // 1. Unconnected open-connection handshake (cookie exchange).
    open_connection(&mut client, &mut buf)?;

    // 2. Connected mode: reliable connection request with an empty password.
    let mut conn = Connection::new(Instant::now());
    conn.send_bytes(&[id::CONNECTION_REQUEST], Reliability::Reliable, 0);

    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut joined = false;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        for dg in conn.poll(now) {
            client.send_plain(&dg)?;
        }
        let Some(n) = client.recv(&mut buf, Duration::from_millis(10))? else { continue };
        let delivered = match conn.handle_datagram(&buf[..n], Instant::now()) {
            Ok(d) => d,
            Err(_) => continue, // stray unconnected reply (e.g. duplicate 0x19)
        };
        for msg in delivered {
            if !handle_message(&msg, &mut conn, &client, &table, &args.name, &mut joined)? {
                return Ok(());
            }
        }
    }

    // Leave politely.
    conn.send_bytes(&[id::DISCONNECTION_NOTIFICATION], Reliability::ReliableOrdered, 0);
    for dg in conn.poll(Instant::now()) {
        client.send_plain(&dg)?;
    }
    println!("done ({}s elapsed), disconnected", args.seconds);
    Ok(())
}

fn open_connection(client: &mut Client, buf: &mut [u8]) -> Result<()> {
    let mut cookie = 0u16;
    for attempt in 1..=6 {
        client.send_plain(&open_connection_request(cookie))?;
        let deadline = Instant::now() + Duration::from_millis(1000);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(n) = client.recv(buf, left)? else { break };
            match parse_reply(&buf[..n]) {
                Some(ServerReply::Cookie(c)) => cookie = c,
                Some(ServerReply::Accepted) => {
                    println!("[open] accepted after {attempt} request(s)");
                    return Ok(());
                }
                Some(other) => return Err(format!("server refused: {other:?}").into()),
                None => {}
            }
        }
    }
    Err("server did not respond".into())
}

/// Returns false when the session should end.
fn handle_message(
    msg: &Delivered,
    conn: &mut Connection,
    client: &Client,
    table: &AuthTable,
    name: &str,
    joined: &mut bool,
) -> Result<bool> {
    let (_, body) = strip_timestamp(&msg.data);
    let Some(&first) = body.first() else { return Ok(true) };
    match first {
        id::AUTH_KEY => {
            let challenge = auth::parse_challenge(body).ok_or("malformed auth challenge")?;
            let response = table.response(&challenge).ok_or_else(|| format!("challenge {challenge} not in table"))?;
            println!("[auth] challenge {challenge} -> answered");
            conn.send_bytes(&auth::build_response(response), Reliability::Reliable, 0);
        }
        id::CONNECTION_REQUEST_ACCEPTED => {
            let acc = session::parse_accepted(body).ok_or("malformed connection-accepted")?;
            println!("[conn] accepted: player id {}, challenge {:#010x}", acc.player_id, acc.challenge);
            // New-incoming-connection notice with the server address, then an internal ping.
            let mut w = BitWriter::new();
            w.write_u8(id::NEW_INCOMING_CONNECTION);
            match client.server.ip() {
                std::net::IpAddr::V4(ip) => w.write_bytes(&ip.octets()),
                std::net::IpAddr::V6(_) => return Err("IPv6 servers are not supported".into()),
            }
            w.write_u16(client.server.port());
            conn.send_bits(&w, Reliability::Reliable, 0);
            conn.ping(Instant::now());

            let serial = session::make_serial(&pseudo_random_hex40());
            let payload = session::build_player_connect(&session::R4, name, acc.challenge, &serial);
            conn.send_bits(&rpc::encode(ids::PLAYER_CONNECT, &payload), Reliability::Reliable, 0);
            println!("[join] sent PlayerConnect as {name:?}");
        }
        id::RPC => {
            let Some(rpc) = rpc::decode(&msg.data, msg.bit_len) else { return Ok(true) };
            return handle_rpc(&rpc, joined);
        }
        id::DISCONNECTION_NOTIFICATION => {
            println!("[conn] server closed the connection");
            return Ok(false);
        }
        id::CONNECTION_BANNED => return Err("banned from this server".into()),
        id::INVALID_PASSWORD => return Err("wrong server password".into()),
        _ => {}
    }
    Ok(true)
}

fn handle_rpc(rpc: &rpc::Rpc, joined: &mut bool) -> Result<bool> {
    let mut r = rpc.reader();
    match rpc.id {
        ids::PLAYER_INIT => {
            let g = session::parse_init_game(&mut r).ok_or("malformed InitGame")?;
            *joined = true;
            println!("[game] InitGame: server {:?}, our id {}", g.server_name, g.player_id);
        }
        ids::CONNECTION_REJECTED => {
            let code = r.read_u8().unwrap_or(0);
            return Err(format!("connection rejected: {}", session::rejection_reason(code)).into());
        }
        SHOW_DIALOG => {
            let (Some(dialog_id), Some(style)) = (r.read_u16(), r.read_u8()) else { return Ok(true) };
            let title = read_str8(&mut r).ok_or("dialog title")?;
            let b1 = read_str8(&mut r).ok_or("dialog button1")?;
            let b2 = read_str8(&mut r).ok_or("dialog button2")?;
            println!(
                "[dialog] id {} style {style} title {:?} buttons {:?}/{:?}",
                dialog_id as i16,
                lossy(&title),
                lossy(&b1),
                lossy(&b2)
            );
            report_cstr("dialog body", &rpc.payload, r, 4096, EXPECTED_DIALOG);
        }
        SHOW_TEXT_LABEL => {
            let (Some(label_id), Some(colour)) = (r.read_u16(), r.read_u32()) else { return Ok(true) };
            let (Some(x), Some(y), Some(z), Some(dist), Some(los), Some(pl), Some(veh)) =
                (r.read_f32(), r.read_f32(), r.read_f32(), r.read_f32(), r.read_u8(), r.read_u16(), r.read_u16())
            else {
                return Ok(true);
            };
            println!(
                "[label] id {label_id} colour {colour:08x} pos ({x}, {y}, {z}) dist {dist} los {los} player {pl:#x} vehicle {veh:#x}"
            );
            report_cstr("label text", &rpc.payload, r, 2048, EXPECTED_LABEL);
        }
        other => println!("[rpc] id {other} ({} bits)", rpc.bit_len),
    }
    Ok(true)
}

const SHOW_DIALOG: u8 = 61;
const SHOW_TEXT_LABEL: u8 = 36;

/// What the probe filterscript sends (Windows-1252 bytes).
const EXPECTED_DIALOG: &[u8] = b"Hello, World! 0123456789\nLine 2:\tTabbed {FF00AA}colour ~r~ (x) [y] <z> 50% #$&*+-=/?;'`|\\ quote\" end\n\x80uro caf\xE9 na\xEFve \xA9 \xDF \xFF.";
const EXPECTED_LABEL: &[u8] = b"Label: \xC4rger 3D-text #7 (ok)!\nsecond line";

fn read_str8(r: &mut BitReader) -> Option<Vec<u8>> {
    let n = r.read_u8()?;
    r.read_bytes(n.into())
}

fn lossy(b: &[u8]) -> String {
    // Windows-1252 bytes shown as Latin-1 code points (close enough for a probe).
    b.iter().map(|&c| c as char).collect()
}

/// Decode a compressed string at the reader's position, print it, compare with
/// the expected text, and check that our encoder reproduces the wire bits.
fn report_cstr(what: &str, payload: &[u8], r: BitReader, max_len: usize, expected: &[u8]) {
    let start = r.position();
    let mut rr = r.clone();
    let Some(text) = strcomp::read_compressed_string(&mut rr, max_len) else {
        println!("[{what}] DECODE FAILED at bit {start}");
        return;
    };
    let end = rr.position();
    let mut raw = BitReader::new(payload);
    raw.skip(start);
    let wire = raw.read_bits_to_vec(end - start).unwrap_or_default();
    let mut ours = BitWriter::new();
    strcomp::write_compressed_string(&mut ours, &text);
    println!("[{what}] {} bytes: {:?}", text.len(), lossy(&text));
    println!("[{what}] wire cstr ({} bits): {}", end - start, hex(&wire));
    println!("[{what}] matches script text: {}", text == expected);
    println!(
        "[{what}] re-encode bit-identical: {}",
        ours.bit_len() == end - start && ours.as_bytes() == wire.as_slice()
    );
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 40 hex digits of non-cryptographic randomness for the client serial.
fn pseudo_random_hex40() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..3)
        .map(|_| {
            let mut h = RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
            );
            format!("{:016X}", h.finish())
        })
        .collect::<String>()[..40]
        .to_owned()
}
