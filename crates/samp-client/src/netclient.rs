//! The client's network session: samp-proto's sans-IO RakNet [`Connection`]
//! driven by the per-frame tick over a non-blocking UDP socket.
//!
//! Flow (docs/re/handshake.md §1–§5, same order as the original client and
//! `samp-bot`): open-connection request/cookie → `0x0b` connection request →
//! auth key (`0x0c`) → `0x22` accepted → `0x1e` + ping + `0x29` + keep-alive +
//! PlayerConnect (RPC 25) → InitGame (RPC 139). Afterwards every delivered
//! RPC is decoded with `samp_proto::rpcs::decode` and handed to the caller as
//! an [`Event`]. Nothing here blocks: [`NetClient::tick`] drains the socket,
//! runs the retry timers and flushes queued datagrams.
//!
//! The socket is `std::net::UdpSocket` (on Windows that is Winsock), so the
//! same code runs in host tests and in the DLL.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use samp_proto::auth::{self, AuthTable};
use samp_proto::bitstream::BitWriter;
use samp_proto::cipher::{self, CipherKey};
use samp_proto::connection::{Connection, Delivered};
use samp_proto::raknet::{Reliability, id, strip_timestamp};
use samp_proto::rpc;
use samp_proto::rpcs::{self, Incoming, RpcPayload};
use samp_proto::session::{self, InitGame};
use samp_proto::sync::{SyncPacket, policy};
use samp_proto::unconnected::{ServerReply, open_connection_request, parse_reply};

/// Interval between open-connection requests while the server is silent.
const OPEN_RETRY: Duration = Duration::from_millis(1000);
/// How many RPC ids `net` reports.
const LAST_RPCS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Sending open-connection requests (cookie exchange).
    Opening,
    /// RakNet connection requested; waiting for auth / `0x22`.
    Connecting,
    /// PlayerConnect sent; waiting for InitGame.
    Joining,
    /// InitGame received.
    Connected,
    /// Closed by the server or refused; carries the reason.
    Disconnected(String),
}

impl Phase {
    pub fn name(&self) -> &str {
        match self {
            Phase::Opening => "opening",
            Phase::Connecting => "connecting",
            Phase::Joining => "joining",
            Phase::Connected => "connected",
            Phase::Disconnected(_) => "disconnected",
        }
    }
}

/// Something the game side has to react to.
#[derive(Debug)]
pub enum Event {
    /// RPC 139: we are in.
    InitGame(Box<InitGame>),
    /// Any other decoded S→C RPC.
    Rpc(Incoming),
    /// A decoded S→C sync packet (207, 203, …; remote-players-sync.md §2).
    Sync(samp_proto::sync::IncomingSync),
    /// The session ended.
    Closed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
    pub messages_received: u64,
    pub rpcs_received: u64,
    pub syncs_sent: u64,
    pub syncs_received: u64,
}

pub struct NetClient {
    sock: UdpSocket,
    server: SocketAddrV4,
    key: CipherKey,
    name: String,
    auth: AuthTable,
    phase: Phase,
    cookie: u16,
    open_attempts: u32,
    last_open: Option<Instant>,
    conn: Option<Connection>,
    player_id: Option<u16>,
    last_rpcs: VecDeque<u8>,
    counters: Counters,
    wire: Vec<u8>,
}

/// Resolves `host:port` to an IPv4 address (RakNet 2.x is IPv4 only).
pub fn resolve(host: &str, port: u16) -> std::io::Result<SocketAddrV4> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(SocketAddrV4::new(ip, port));
    }
    (host, port)
        .to_socket_addrs()?
        .find_map(|a| match a {
            std::net::SocketAddr::V4(v4) => Some(v4),
            std::net::SocketAddr::V6(_) => None,
        })
        .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, format!("no IPv4 address for {host}")))
}

impl NetClient {
    pub fn new(server: SocketAddrV4, name: &str, auth: AuthTable) -> std::io::Result<Self> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        sock.connect(server)?;
        sock.set_nonblocking(true)?;
        Ok(Self {
            sock,
            server,
            key: CipherKey::from_port(server.port()),
            name: name.to_string(),
            auth,
            phase: Phase::Opening,
            cookie: 0,
            open_attempts: 0,
            last_open: None,
            conn: None,
            player_id: None,
            last_rpcs: VecDeque::new(),
            counters: Counters::default(),
            wire: Vec::new(),
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn player_id(&self) -> Option<u16> {
        self.player_id
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// The `net` control-channel reply.
    pub fn status_line(&self) -> String {
        let pid = self.player_id.map(|p| p.to_string()).unwrap_or_else(|| "none".into());
        let rpcs: Vec<String> = self.last_rpcs.iter().map(|r| r.to_string()).collect();
        let c = self.counters;
        let reason = match &self.phase {
            Phase::Disconnected(r) => format!(" reason={r:?}"),
            _ => String::new(),
        };
        format!(
            "net phase={}{reason} server={} player_id={pid} last_rpcs=[{}] sent={} received={} messages={} rpcs={} syncs={} syncs_in={}",
            self.phase.name(),
            self.server,
            rpcs.join(","),
            c.datagrams_sent,
            c.datagrams_received,
            c.messages_received,
            c.rpcs_received,
            c.syncs_sent,
            c.syncs_received
        )
    }

    /// Drains the socket, runs timers and flushes outgoing datagrams.
    pub fn tick(&mut self, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        if self.phase == Phase::Opening && self.last_open.is_none_or(|t| now.duration_since(t) >= OPEN_RETRY) {
            self.send_open_request(now);
        }
        let mut buf = [0u8; 2048];
        loop {
            match self.sock.recv(&mut buf) {
                Ok(n) => {
                    self.counters.datagrams_received += 1;
                    self.on_datagram(&buf[..n], now, &mut events);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // ICMP port unreachable (server not up yet) shows up as a reset on Windows.
                Err(e) if matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused) => continue,
                Err(e) => {
                    crate::log!("net: recv error: {e}");
                    break;
                }
            }
        }
        self.flush(now);
        events
    }

    fn send_open_request(&mut self, now: Instant) {
        self.open_attempts += 1;
        self.last_open = Some(now);
        if self.open_attempts == 1 || self.open_attempts.is_multiple_of(10) {
            crate::log!("net: open-connection request #{} to {}", self.open_attempts, self.server);
        }
        let req = open_connection_request(self.cookie);
        self.send_plain(&req);
    }

    fn send_plain(&mut self, plain: &[u8]) {
        cipher::encrypt(plain, self.key, &mut self.wire);
        match self.sock.send(&self.wire) {
            Ok(_) => self.counters.datagrams_sent += 1,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => crate::log!("net: send error: {e}"),
        }
    }

    /// Sends everything queued in the connection.
    pub fn flush(&mut self, now: Instant) {
        let Some(conn) = self.conn.as_mut() else { return };
        for dg in conn.poll(now) {
            self.send_plain(&dg);
        }
    }

    fn on_datagram(&mut self, data: &[u8], now: Instant, events: &mut Vec<Event>) {
        if self.phase == Phase::Opening {
            match parse_reply(data) {
                Some(ServerReply::Cookie(c)) => {
                    self.cookie = c;
                    self.send_open_request(now);
                }
                Some(ServerReply::Accepted) => {
                    crate::log!("net: open-connection accepted after {} request(s)", self.open_attempts);
                    let mut conn = Connection::new(now);
                    conn.send_bytes(&[id::CONNECTION_REQUEST], Reliability::Reliable, 0);
                    self.conn = Some(conn);
                    self.phase = Phase::Connecting;
                }
                Some(other) => self.close(format!("server refused: {other:?}"), events),
                None => {}
            }
            return;
        }
        let Some(conn) = self.conn.as_mut() else { return };
        let Ok(delivered) = conn.handle_datagram(data, now) else { return };
        for msg in delivered {
            self.counters.messages_received += 1;
            self.on_message(&msg, now, events);
        }
    }

    fn close(&mut self, reason: String, events: &mut Vec<Event>) {
        crate::log!("net: session closed: {reason}");
        self.phase = Phase::Disconnected(reason.clone());
        events.push(Event::Closed(reason));
    }

    fn on_message(&mut self, msg: &Delivered, now: Instant, events: &mut Vec<Event>) {
        let (_, body) = strip_timestamp(&msg.data);
        let Some(&first) = body.first() else { return };
        match first {
            id::AUTH_KEY => {
                let Some(challenge) = auth::parse_challenge(body) else {
                    crate::log!("net: malformed auth challenge");
                    return;
                };
                match self.auth.response(&challenge) {
                    Some(resp) => {
                        let reply = auth::build_response(resp);
                        if let Some(c) = self.conn.as_mut() {
                            c.send_bytes(&reply, Reliability::Reliable, 0);
                        }
                        crate::log!("net: auth challenge {challenge} answered");
                    }
                    None => self.close(format!("auth challenge {challenge} not in table"), events),
                }
            }
            id::CONNECTION_REQUEST_ACCEPTED => self.on_accepted(body, now),
            id::RPC => {
                let Some(r) = rpc::decode(&msg.data, msg.bit_len) else { return };
                self.counters.rpcs_received += 1;
                if self.last_rpcs.len() == LAST_RPCS {
                    self.last_rpcs.pop_front();
                }
                self.last_rpcs.push_back(r.id);
                match rpcs::decode(r.id, &mut r.reader()) {
                    Some(Incoming::InitGame(g)) => {
                        self.player_id = Some(g.player_id);
                        self.phase = Phase::Connected;
                        events.push(Event::InitGame(g));
                    }
                    Some(Incoming::ConnectionRejected { reason }) => {
                        self.close(format!("connection rejected: {}", session::rejection_reason(reason)), events)
                    }
                    Some(other) => events.push(Event::Rpc(other)),
                    None => {}
                }
            }
            b if samp_proto::sync::INCOMING_IDS.contains(&b) => {
                self.counters.syncs_received += 1;
                if let Some(s) = samp_proto::sync::decode(&msg.data, msg.bit_len) {
                    events.push(Event::Sync(s));
                }
            }
            id::DISCONNECTION_NOTIFICATION => self.close("server closed the connection".into(), events),
            id::CONNECTION_LOST => self.close("connection lost".into(), events),
            id::CONNECTION_BANNED => self.close("banned from this server".into(), events),
            id::INVALID_PASSWORD => self.close("wrong server password".into(), events),
            _ => {}
        }
    }

    fn on_accepted(&mut self, body: &[u8], now: Instant) {
        let Some(acc) = session::parse_accepted(body) else {
            crate::log!("net: malformed connection-accepted");
            return;
        };
        crate::log!("net: connection accepted, player id {}", acc.player_id);
        self.player_id = Some(acc.player_id);
        let Some(conn) = self.conn.as_mut() else { return };
        let mut w = BitWriter::new();
        w.write_u8(id::NEW_INCOMING_CONNECTION);
        w.write_bytes(&self.server.ip().octets());
        w.write_u16(self.server.port());
        conn.send_bits(&w, Reliability::Reliable, 0);
        // Same order as the original client (handshake.md §1).
        conn.ping(now);
        conn.send_bytes(&[id::RECEIVED_STATIC_DATA], Reliability::Reliable, 0);
        conn.send_bytes(&[id::KEEP_ALIVE], Reliability::Reliable, 0);
        let serial = session::make_serial(&pseudo_random_hex40());
        let payload = session::build_player_connect(&session::R4, &self.name, acc.challenge, &serial);
        conn.send_bits(&rpc::encode(rpc::ids::PLAYER_CONNECT, &payload), Reliability::Reliable, 0);
        self.phase = Phase::Joining;
        crate::log!("net: PlayerConnect sent as {:?}", self.name);
    }

    /// Queues a C→S RPC (priority/channel 0 as in player-state.md).
    pub fn send_rpc<P: RpcPayload>(&mut self, p: &P, reliability: Reliability) {
        if let Some(c) = self.conn.as_mut() {
            c.send_bits(&p.to_rpc(), reliability, 0);
        }
    }

    /// Queues a sync packet with its spec'd reliability and channel.
    pub fn send_sync<S: SyncPacket>(&mut self, s: &S) {
        let Some(params) = policy::send_params(S::PACKET_ID) else { return };
        if let Some(c) = self.conn.as_mut() {
            c.send_bits(&s.to_bits(), params.reliability, params.channel);
            self.counters.syncs_sent += 1;
        }
    }

    /// Sends a disconnection notification right away (best effort).
    pub fn disconnect(&mut self, now: Instant) {
        if let Some(c) = self.conn.as_mut() {
            c.send_bytes(&[id::DISCONNECTION_NOTIFICATION], Reliability::ReliableOrdered, 0);
        }
        self.flush(now);
    }
}

/// 40 hex digits of non-cryptographic randomness for the client serial.
fn pseudo_random_hex40() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut s = String::with_capacity(48);
    for i in 0..3u8 {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        s.push_str(&format!("{:016X}", h.finish()));
    }
    s.truncate(40);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_literal_ipv4() {
        assert_eq!(resolve("127.0.0.1", 7777).unwrap(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7777));
    }

    #[test]
    fn serial_seed_is_40_hex_digits() {
        let s = pseudo_random_hex40();
        assert_eq!(s.len(), 40);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Opening phase: a cookie reply makes the client repeat the request with
    /// the cookie; an accept starts the RakNet connection.
    #[test]
    fn open_connection_exchange_over_loopback() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let addr = match server.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let mut c = NetClient::new(addr, "Tester", AuthTable::parse("").unwrap()).unwrap();
        let t0 = Instant::now();
        assert!(c.tick(t0).is_empty());
        let mut buf = [0u8; 64];
        let (n, peer) = server.recv_from(&mut buf).unwrap();
        assert!(n >= 3);
        // Cookie reply (unencrypted S→C): 0x1a + u16 cookie.
        server.send_to(&[0x1a, 0x34, 0x12], peer).unwrap();
        let mut got_second = false;
        for _ in 0..200 {
            c.tick(Instant::now());
            server.set_nonblocking(true).unwrap();
            if let Ok((n, _)) = server.recv_from(&mut buf) {
                assert!(n >= 3);
                got_second = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(got_second, "no second open-connection request after the cookie");
        server.send_to(&[0x19, 0x00], peer).unwrap();
        for _ in 0..200 {
            c.tick(Instant::now());
            if c.phase() == &Phase::Connecting {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(c.phase(), &Phase::Connecting);
        assert!(c.status_line().starts_with("net phase=connecting server=127.0.0.1:"));
    }
}
