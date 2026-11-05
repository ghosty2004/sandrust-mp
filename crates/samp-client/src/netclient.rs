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
/// Open-connection requests per attempt (TRACE-8: 6 requests 1.0 s apart per socket).
const OPEN_REQUESTS: u32 = 6;
/// The attempt fails when more than this passed since its last request (RakNet's update cycle
/// A 0x10041656: the request time + 1000 ms is the deadline, `0x1D` once it lies in the past).
/// Measured: the next attempt's first request follows the 6th by 1.03–1.07 s (6.05–6.07 s per
/// attempt), the rest is samp's frame and RakClient::Connect's start-up ([`RAKNET_STOP`]).
const OPEN_FAIL_AFTER: Duration = Duration::from_millis(1000);
/// RakClient::Connect / Disconnect block samp's main thread while RakNet's update thread is
/// stopped and a new one started: RakPeer::Disconnect polls the thread's exit with `Sleep(15)`
/// (A 0x10040700), RakPeer::Initialize polls the new thread's start with `Sleep(10)` (A 0x10042934).
/// Our network code runs on the main thread, so the same waits are made here (one poll each:
/// the update thread sleeps 2 ms per cycle, samp's `threadSleepTimer`).
const RAKNET_STOP: Duration = Duration::from_millis(15);
const RAKNET_START: Duration = Duration::from_millis(10);
/// RakNet `ID_RSA_PUBLIC_KEY_MISMATCH` (samp prints "Failed to initialize encryption.").
const ENCRYPTION_FAILED: u8 = 0x23;
/// How many RPC ids `net` reports.
const LAST_RPCS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No connection attempt running (before the first connect, after a failed attempt, a
    /// lost connection or a disconnect): no socket, no traffic.
    Idle,
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
            Phase::Idle => "idle",
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
    /// A RakNet packet the netgame's packet handler (B 0x1000b250) reacts to.
    Packet(NetPacket),
    /// RPC 130 ConnectionRejected with its reason.
    Rejected(u8),
}

/// RakNet-level outcomes (packet ids as seen by samp's packet handler).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetPacket {
    /// 0x1D: no answer to the open-connection requests.
    AttemptFailed,
    /// 0x1F: the server has no free slot.
    ServerFull,
    /// 0x20: the server closed the connection (kick, ban, server shutdown).
    ServerClosed,
    /// 0x21: no packet from the server for 12 s.
    ConnectionLost,
    /// 0x22: connection accepted (PlayerConnect is being sent).
    Accepted,
    /// 0x23.
    EncryptionFailed,
    /// 0x24.
    Banned,
    /// 0x25.
    InvalidPassword,
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
    /// The attempt's socket; every connect opens a new one (TRACE-8: new source port).
    sock: Option<UdpSocket>,
    /// RakNet's update thread would be running (between Connect and Disconnect).
    rak_running: bool,
    /// Drop the connection after the next flush (its acks still go out).
    closing: bool,
    server: SocketAddrV4,
    key: CipherKey,
    name: String,
    auth: AuthTable,
    phase: Phase,
    cookie: u16,
    open_attempts: u32,
    /// When the attempt's first request went out (requests are due at whole seconds from it).
    first_open: Option<Instant>,
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
    /// A session for `server`; nothing is sent before [`Self::connect`].
    pub fn new(server: SocketAddrV4, name: &str, auth: AuthTable) -> std::io::Result<Self> {
        Ok(Self {
            sock: None,
            rak_running: false,
            closing: false,
            server,
            key: CipherKey::from_port(server.port()),
            name: name.to_string(),
            auth,
            phase: Phase::Idle,
            cookie: 0,
            open_attempts: 0,
            first_open: None,
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

    /// RakClient `Connect` (A 0x10034130, called by samp's connect tick A 0x10008940): RakNet is
    /// shut down and started again (the waits of [`RAKNET_STOP`] / [`RAKNET_START`]), a new
    /// socket and a new attempt; the first open-connection request goes out on the next tick.
    /// Returns when the attempt has started; samp takes the attempt time after this returns.
    pub fn connect(&mut self) -> std::io::Result<()> {
        self.rak_stop();
        self.drop_connection();
        rak_wait(RAKNET_START);
        self.rak_running = true;
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        sock.connect(self.server)?;
        sock.set_nonblocking(true)?;
        crate::log!("net: connect to {} from {:?}", self.server, sock.local_addr().ok());
        self.sock = Some(sock);
        self.phase = Phase::Opening;
        self.cookie = 0;
        self.open_attempts = 0;
        self.first_open = None;
        self.player_id = None;
        Ok(())
    }

    /// Stops RakNet's update thread, if it runs (RakPeer::Disconnect's exit poll).
    fn rak_stop(&mut self) {
        if std::mem::take(&mut self.rak_running) {
            rak_wait(RAKNET_STOP);
        }
    }

    /// RakClient `Disconnect` from samp's packet handlers (no notification is sent in the
    /// measured cases): pending acks go out with the next flush, then the connection and
    /// its socket are dropped.
    pub fn disconnect_silent(&mut self) {
        self.rak_stop();
        if self.sock.is_some() {
            self.closing = true;
        }
        if !matches!(self.phase, Phase::Disconnected(_)) {
            self.phase = Phase::Idle;
        }
    }

    fn drop_connection(&mut self) {
        self.conn = None;
        self.sock = None;
        self.closing = false;
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
        if self.sock.is_none() {
            return events;
        }
        if self.phase == Phase::Opening {
            // Requests at whole seconds from the first one (RakNet's own timer: measured 0, 1.001,
            // 2.003, … s), so frame granularity does not add up.
            let since_first = self.first_open.map(|t| now.duration_since(t));
            let due = since_first.is_none_or(|d| d >= OPEN_RETRY * self.open_attempts);
            if due && self.open_attempts < OPEN_REQUESTS {
                self.send_open_request(now);
            } else if self.open_attempts >= OPEN_REQUESTS
                && since_first.is_some_and(|d| d > OPEN_RETRY * (OPEN_REQUESTS - 1) + OPEN_FAIL_AFTER)
            {
                crate::log!("net: no answer to {} open-connection requests", self.open_attempts);
                self.end_attempt(NetPacket::AttemptFailed, &mut events);
                return events;
            }
        }
        let mut buf = [0u8; 2048];
        while let Some(sock) = self.sock.as_ref() {
            match sock.recv(&mut buf) {
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
        if !self.closing && self.conn.as_ref().is_some_and(|c| c.is_dead(now)) {
            crate::log!(
                "net: no ack for {} ms while reliable messages are pending: connection lost",
                samp_proto::connection::TIMEOUT.as_millis()
            );
            self.drop_connection();
            self.phase = Phase::Idle;
            events.push(Event::Packet(NetPacket::ConnectionLost));
            return events;
        }
        self.flush(now);
        events
    }

    /// The attempt ended before a connection existed (no answer, full, banned).
    fn end_attempt(&mut self, p: NetPacket, events: &mut Vec<Event>) {
        self.drop_connection();
        self.phase = Phase::Idle;
        events.push(Event::Packet(p));
    }

    fn send_open_request(&mut self, now: Instant) {
        self.open_attempts += 1;
        self.first_open.get_or_insert(now);
        if self.open_attempts == 1 || self.open_attempts.is_multiple_of(10) {
            crate::log!("net: open-connection request #{} to {}", self.open_attempts, self.server);
        }
        let req = open_connection_request(self.cookie);
        self.send_plain(&req);
    }

    fn send_plain(&mut self, plain: &[u8]) {
        cipher::encrypt(plain, self.key, &mut self.wire);
        let Some(sock) = self.sock.as_ref() else { return };
        match sock.send(&self.wire) {
            Ok(_) => self.counters.datagrams_sent += 1,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => crate::log!("net: send error: {e}"),
        }
    }

    /// Sends everything queued in the connection.
    pub fn flush(&mut self, now: Instant) {
        if let Some(conn) = self.conn.as_mut() {
            for dg in conn.poll(now) {
                self.send_plain(&dg);
            }
        }
        if self.closing {
            self.drop_connection();
        }
    }

    fn on_datagram(&mut self, data: &[u8], now: Instant, events: &mut Vec<Event>) {
        if self.phase == Phase::Opening {
            match parse_reply(data) {
                // The original stores the cookie and answers with the next request of its
                // 1000 ms retry timer (TRACE-4 capture orig-20261104-204143, net-59452: request
                // at 0 s, cookie at 0.001 s, next request at 1.001 s).
                Some(ServerReply::Cookie(c)) => self.cookie = c,
                Some(ServerReply::Accepted) => {
                    crate::log!("net: open-connection accepted after {} request(s)", self.open_attempts);
                    let mut conn = Connection::new(now);
                    conn.send_bytes(&[id::CONNECTION_REQUEST], Reliability::Reliable, 0);
                    self.conn = Some(conn);
                    self.phase = Phase::Connecting;
                }
                Some(ServerReply::ServerFull) => self.end_attempt(NetPacket::ServerFull, events),
                Some(ServerReply::Banned) => self.end_attempt(NetPacket::Banned, events),
                Some(ServerReply::Failed) => self.end_attempt(NetPacket::AttemptFailed, events),
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

    /// The connection ends (acks of this datagram still go out with the next flush).
    fn close(&mut self, reason: String) {
        crate::log!("net: session closed: {reason}");
        self.phase = Phase::Disconnected(reason);
        self.closing = true;
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
                    None => {
                        crate::log!("net: auth challenge {challenge} not in table");
                        self.close("auth challenge not in table".into());
                    }
                }
            }
            id::CONNECTION_REQUEST_ACCEPTED => {
                self.on_accepted(body, now);
                events.push(Event::Packet(NetPacket::Accepted));
            }
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
                        crate::log!("net: connection rejected: {}", session::rejection_reason(reason));
                        events.push(Event::Rejected(reason))
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
            // RakNet ends the connection itself on these; samp's handlers only add their own
            // lines / Disconnect calls (B 0x1000b250).
            id::DISCONNECTION_NOTIFICATION => {
                self.close("server closed the connection".into());
                events.push(Event::Packet(NetPacket::ServerClosed));
            }
            id::CONNECTION_BANNED => {
                self.close("banned from this server".into());
                events.push(Event::Packet(NetPacket::Banned));
            }
            id::INVALID_PASSWORD => {
                self.close("wrong server password".into());
                events.push(Event::Packet(NetPacket::InvalidPassword));
            }
            samp_proto::unconnected::id::NO_FREE_INCOMING_CONNECTIONS => {
                self.close("server full".into());
                events.push(Event::Packet(NetPacket::ServerFull));
            }
            ENCRYPTION_FAILED => events.push(Event::Packet(NetPacket::EncryptionFailed)),
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

    /// RakClient `Disconnect(500, 0)` of samp's `/quit` (A 0x100c3ec0): the disconnection
    /// notification goes out UNRELIABLE on channel 0 (RakNet A 0x1003e8f0, reliability 6; measured
    /// in every original capture, e.g. `orig-20261105-003749` net-42564: a 5-byte datagram
    /// `rel6 0x20`), RakNet waits for the connection slot to close (one `Sleep(15)` poll) and
    /// stops; nothing is sent afterwards.
    pub fn disconnect(&mut self, now: Instant) {
        if let Some(c) = self.conn.as_mut() {
            c.send_bytes(&[id::DISCONNECTION_NOTIFICATION], Reliability::Unreliable, 0);
            self.flush(now);
            rak_wait(RAKNET_STOP);
        }
        self.rak_stop();
        self.drop_connection();
        if !matches!(self.phase, Phase::Disconnected(_)) {
            self.phase = Phase::Idle;
        }
    }
}

/// One of RakNet's blocking polls on the calling (main) thread.
fn rak_wait(d: Duration) {
    #[cfg(windows)]
    // SAFETY: plain Win32 Sleep, like RakNet's own polls.
    unsafe {
        windows_sys::Win32::System::Threading::Sleep(d.as_millis() as u32)
    };
    #[cfg(not(windows))]
    std::thread::sleep(d);
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

    /// TRACE-8: 6 open-connection requests 1 s apart on a new socket, then 0x1D 1.05 s after
    /// the last; the next connect uses another source port.
    #[test]
    fn attempt_fails_after_six_requests_and_reconnect_uses_a_new_socket() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let addr = match server.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let mut c = NetClient::new(addr, "Tester", AuthTable::parse("").unwrap()).unwrap();
        c.connect().unwrap();
        let t0 = Instant::now();
        let mut buf = [0u8; 64];
        let mut first_port = None;
        for i in 0..6u64 {
            assert!(c.tick(t0 + Duration::from_millis(1000 * i)).is_empty());
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            first_port.get_or_insert(peer.port());
        }
        assert!(c.tick(t0 + Duration::from_millis(6000)).is_empty());
        assert!(server.recv_from(&mut buf).is_err(), "a 7th request");
        let ev = c.tick(t0 + Duration::from_millis(6001));
        assert!(matches!(ev.as_slice(), [Event::Packet(NetPacket::AttemptFailed)]));
        assert_eq!(c.phase(), &Phase::Idle);
        // Connect stops RakNet (15 ms) and starts it again (10 ms) before returning.
        let before = Instant::now();
        c.connect().unwrap();
        assert!(before.elapsed() >= RAKNET_STOP + RAKNET_START);
        c.tick(Instant::now());
        let (_, peer) = server.recv_from(&mut buf).unwrap();
        assert_ne!(Some(peer.port()), first_port);
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
        assert_eq!(c.phase(), &Phase::Idle);
        c.connect().unwrap();
        assert!(c.tick(t0).is_empty());
        let mut buf = [0u8; 64];
        let (n, peer) = server.recv_from(&mut buf).unwrap();
        assert!(n >= 3);
        // Cookie reply (unencrypted S→C): 0x1a + u16 cookie.
        server.send_to(&[0x1a, 0x34, 0x12], peer).unwrap();
        // The cookie is stored; the answer waits for the 1000 ms retry timer.
        std::thread::sleep(Duration::from_millis(20));
        c.tick(t0 + Duration::from_millis(500));
        server.set_nonblocking(true).unwrap();
        assert!(server.recv_from(&mut buf).is_err(), "answered the cookie before the retry timer");
        let mut got_second = false;
        for _ in 0..200 {
            c.tick(t0 + OPEN_RETRY);
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
