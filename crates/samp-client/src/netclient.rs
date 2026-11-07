//! The client's network session: samp-proto's sans-IO RakNet [`Connection`] over a non-blocking UDP socket
//! (`std::net`, so the same code runs in host tests and in the DLL), updated every 2 ms on RakNet's update thread.
//! Join order, as in the original client: open-connection request / cookie → `0x0b` connection request → auth key
//! (`0x0c`) → `0x22` accepted → `0x1e` + ping + `0x29` + keep-alive → PlayerConnect (RPC 25) → InitGame (RPC 139).
//! Afterwards [`NetClient::tick`] hands every decoded RPC and sync packet to the caller as an [`Event`]; sends are
//! queued and go out with the next update.
//! Spec: docs/re/handshake.md, "Connection establishment" to "Join"; docs/phase3/net6-notes.md.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use samp_proto::auth::{self, AuthTable};
use samp_proto::bitstream::BitWriter;
use samp_proto::cipher::{self, CipherKey};
use samp_proto::connection::{Connection, Delivered, priority};
use samp_proto::raknet::{Reliability, id, strip_timestamp};
use samp_proto::rpc;
use samp_proto::rpcs::{self, Incoming, RpcPayload};
use samp_proto::session::{self, InitGame};
use samp_proto::sync::{SyncPacket, policy};
use samp_proto::unconnected::{ServerReply, open_connection_request, parse_reply};

/// Interval between open-connection requests while the server is silent: the next request (or, after the 6th,
/// the failure `0x1D`) is due in the first update cycle whose RakNet time (whole ms) is strictly greater than the
/// last request's + 1000; the first is due at the Connect time (docs/phase3/reconnect-notes.md, "What the
/// original does"). Mirrors samp.dll 0x1004165D.
const OPEN_RETRY_MS: u64 = 1000;
/// Open-connection requests per attempt (one attempt = one socket).
const OPEN_REQUESTS: u32 = 6;
/// RakClient::Connect / Disconnect block samp's main thread while RakNet's update thread is stopped and a new one
/// started; we make the same waits around stopping / starting our update thread (one poll each, since the thread
/// sleeps only 2 ms per cycle). RakPeer::Disconnect polls the thread's exit with `Sleep(15)`.
/// Mirrors samp.dll 0x10040700.
const RAKNET_STOP: Duration = Duration::from_millis(15);
/// RakPeer::Initialize polls the new thread's start with `Sleep(10)`. Mirrors samp.dll 0x10042934.
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
    /// A decoded S→C sync packet (207, 203, …; docs/re/subsystems/remote-players-sync.md).
    Sync(samp_proto::sync::IncomingSync),
    /// A RakNet packet the netgame's packet handler reacts to (see `lifecycle`).
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
    /// Datagram bytes on the wire (encrypted size), both directions.
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub messages_received: u64,
    pub rpcs_received: u64,
    pub syncs_sent: u64,
    pub syncs_received: u64,
    /// Incoming sync packets the decoder rejected (diagnostics).
    pub syncs_bad: u64,
}

/// The session. RakNet's part (socket, open-connection timers, reliability layer, internal messages) lives in
/// [`Core`] and runs on an update thread every [`UPDATE_SLEEP`], like samp's RakNet; the game's main thread queues
/// sends and takes the packets in [`NetClient::tick`], as samp's packet handler does with RakClient::Receive.
/// Host tests drive the update from `tick`.
pub struct NetClient {
    core: Arc<Mutex<Core>>,
    worker: Option<Worker>,
    /// Run RakNet's update on its own thread (off in unit tests, which use synthetic times).
    threaded: bool,
    /// RakNet's update thread would be running (between Connect and Disconnect).
    rak_running: bool,
}

struct Worker {
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

/// The open-connection cookie: one value for the whole process (initially 0), written only by a cookie reply and
/// read by every request. Nothing resets it on Connect, so after a lost connection the first request of every new
/// attempt already carries the cookie of the last join; while the server's cookie is unchanged (open.mp reseeds
/// every 300 s) that first request is accepted at once (docs/phase3/reconnect-notes.md). Mirrors samp.dll 0x10118E20.
static COOKIE: AtomicU16 = AtomicU16::new(0);

/// RakNet's update thread sleep (the `threadSleepTimer` samp's connect tick passes). Mirrors samp.dll 0x10008988.
const UPDATE_SLEEP: Duration = Duration::from_millis(2);

struct Core {
    /// The attempt's socket; every connect opens a new one (new source port), like samp.
    sock: Option<UdpSocket>,
    /// Drop the connection after the current update (its acks still go out).
    closing: bool,
    server: SocketAddrV4,
    key: CipherKey,
    name: String,
    auth: AuthTable,
    phase: Phase,
    open_attempts: u32,
    /// RakNet time (ms) after which the next request / the failure is due.
    next_open_ms: u64,
    /// Origin of the RakNet millisecond clock of the open-connection timer.
    epoch: Instant,
    conn: Option<Connection>,
    player_id: Option<u16>,
    last_rpcs: VecDeque<u8>,
    counters: Counters,
    wire: Vec<u8>,
    /// Packets for the main thread (RakNet's packet queue).
    events: Vec<Event>,
    /// The challenge of `0x22`, for PlayerConnect sent by the main thread.
    accepted_challenge: Option<u32>,
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

fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

impl NetClient {
    /// A session for `server`; nothing is sent before [`Self::connect`].
    pub fn new(server: SocketAddrV4, name: &str, auth: AuthTable) -> std::io::Result<Self> {
        let core = Core {
            sock: None,
            closing: false,
            server,
            key: CipherKey::from_port(server.port()),
            name: name.to_string(),
            auth,
            phase: Phase::Idle,
            open_attempts: 0,
            next_open_ms: 0,
            epoch: Instant::now(),
            conn: None,
            player_id: None,
            last_rpcs: VecDeque::new(),
            counters: Counters::default(),
            wire: Vec::new(),
            events: Vec::new(),
            accepted_challenge: None,
        };
        Ok(Self { core: Arc::new(Mutex::new(core)), worker: None, threaded: !cfg!(test), rak_running: false })
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        lock(&self.core)
    }

    pub fn phase(&self) -> Phase {
        self.core().phase.clone()
    }

    /// RakClient `Connect`, called by the connect tick: RakNet is shut down and started again (the waits of
    /// [`RAKNET_STOP`] / [`RAKNET_START`]), with a new socket and a new attempt; the first open-connection request
    /// goes out with the next update. Returns when the attempt has started; samp takes the attempt time after
    /// this returns. Mirrors samp.dll 0x10034130.
    pub fn connect(&mut self) -> std::io::Result<()> {
        self.rak_stop();
        self.core().drop_connection();
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        sock.connect(self.core().server)?;
        sock.set_nonblocking(true)?;
        {
            let mut c = self.core();
            crate::log!("net: connect to {} from {:?}", c.server, sock.local_addr().ok());
            c.sock = Some(sock);
            c.phase = Phase::Opening;
            c.open_attempts = 0;
            c.next_open_ms = c.rak_ms(Instant::now());
            c.player_id = None;
        }
        self.rak_start();
        rak_wait(RAKNET_START);
        Ok(())
    }

    /// Starts RakNet's update thread (RakPeer::Initialize).
    fn rak_start(&mut self) {
        self.rak_running = true;
        if !self.threaded {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (core, flag) = (Arc::clone(&self.core), Arc::clone(&stop));
        let spawned = std::thread::Builder::new().name("raknet-update".into()).spawn(move || {
            while !flag.load(AtomicOrdering::Relaxed) {
                lock(&core).update(Instant::now());
                // RakNet's `Sleep(threadSleepTimer)` (kernel32, as rak_wait; std's sleep wakes later under Wine).
                rak_wait(UPDATE_SLEEP);
            }
        });
        match spawned {
            Ok(handle) => self.worker = Some(Worker { stop, handle }),
            Err(e) => {
                crate::log!("net: update thread not started ({e}); updating from the frame");
                self.threaded = false;
            }
        }
    }

    /// Stops RakNet's update thread, if it runs (RakPeer::Disconnect's exit poll).
    fn rak_stop(&mut self) {
        if std::mem::take(&mut self.rak_running) {
            if let Some(w) = self.worker.take() {
                w.stop.store(true, AtomicOrdering::Relaxed);
                rak_wait(RAKNET_STOP);
                let _ = w.handle.join();
            } else {
                rak_wait(RAKNET_STOP);
            }
        }
    }

    /// RakClient `Disconnect` from samp's packet handlers, without a notification: the update that delivered the
    /// packet already sent its acks; the connection and its socket are dropped.
    pub fn disconnect_silent(&mut self) {
        self.rak_stop();
        let mut c = self.core();
        if c.sock.is_some() {
            c.closing = true;
        }
        if !matches!(c.phase, Phase::Disconnected(_)) {
            c.phase = Phase::Idle;
        }
    }

    pub fn player_id(&self) -> Option<u16> {
        self.core().player_id
    }

    pub fn counters(&self) -> Counters {
        self.core().counters
    }

    /// The reliability layer's statistics of the current connection (RakNet's statistics block).
    pub fn link_stats(&self) -> Option<samp_proto::connection::Stats> {
        self.core().conn.as_ref().map(|c| c.stats())
    }

    /// The `net` control-channel reply.
    pub fn status_line(&self) -> String {
        let c = self.core();
        let pid = c.player_id.map(|p| p.to_string()).unwrap_or_else(|| "none".into());
        let rpcs: Vec<String> = c.last_rpcs.iter().map(|r| r.to_string()).collect();
        let n = c.counters;
        let reason = match &c.phase {
            Phase::Disconnected(r) => format!(" reason={r:?}"),
            _ => String::new(),
        };
        format!(
            "net phase={}{reason} server={} player_id={pid} last_rpcs=[{}] sent={} received={} messages={} rpcs={} syncs={} syncs_in={} syncs_bad={}",
            c.phase.name(),
            c.server,
            rpcs.join(","),
            n.datagrams_sent,
            n.datagrams_received,
            n.messages_received,
            n.rpcs_received,
            n.syncs_sent,
            n.syncs_received,
            n.syncs_bad
        )
    }

    /// The main thread's packet handling (samp's netgame tick): the packets the update thread received since
    /// the last frame; on `0x22` it sends PlayerConnect (RPC 25). Without the thread (host tests) the update runs
    /// here first.
    pub fn tick(&mut self, now: Instant) -> Vec<Event> {
        let mut c = self.core();
        if self.worker.is_none() && c.sock.is_some() {
            c.update(now);
        }
        let events = std::mem::take(&mut c.events);
        if events.iter().any(|e| matches!(e, Event::Packet(NetPacket::Accepted))) {
            c.send_player_connect();
        }
        events
    }

    /// Without the update thread: runs an update now (sends what this frame queued). With it,
    /// the thread sends within [`UPDATE_SLEEP`], like RakNet's buffered sends.
    pub fn flush(&mut self, now: Instant) {
        if self.worker.is_none() {
            let mut c = self.core();
            if c.sock.is_some() {
                c.update(now);
            }
        }
    }

    /// Queues a C→S RPC on channel 0 (docs/re/subsystems/player-state.md).
    pub fn send_rpc<P: RpcPayload>(&mut self, p: &P, reliability: Reliability) {
        if let Some(c) = self.core().conn.as_mut() {
            c.send_bits(&p.to_rpc(), reliability, 0);
        }
    }

    /// Queues a sync packet with its spec'd reliability and channel.
    pub fn send_sync<S: SyncPacket>(&mut self, s: &S) {
        let Some(params) = policy::send_params(S::PACKET_ID) else { return };
        let mut c = self.core();
        if let Some(conn) = c.conn.as_mut() {
            conn.send_with(
                s.to_bits().as_bytes(),
                s.to_bits().bit_len(),
                params.reliability,
                params.channel,
                params.priority,
            );
            c.counters.syncs_sent += 1;
        }
    }

    /// RakClient `Disconnect(500, 0)` of samp's `/quit`: the disconnection notification goes out UNRELIABLE on
    /// channel 0 at once (RakNet sends it from the calling thread), RakNet waits for the connection slot to close
    /// (one `Sleep(15)` poll) and stops; nothing is sent afterwards (docs/phase3/m10-notes.md).
    /// Mirrors samp.dll 0x100C3EC0.
    pub fn disconnect(&mut self, now: Instant) {
        let sent = {
            let mut c = self.core();
            if let Some(conn) = c.conn.as_mut() {
                conn.send_with(&[id::DISCONNECTION_NOTIFICATION], 8, Reliability::Unreliable, 0, priority::SYSTEM);
                c.flush_now(now);
                crate::log!("net: disconnection notification sent");
                true
            } else {
                false
            }
        };
        if sent {
            rak_wait(RAKNET_STOP);
        }
        self.rak_stop();
        let mut c = self.core();
        c.drop_connection();
        if !matches!(c.phase, Phase::Disconnected(_)) {
            c.phase = Phase::Idle;
        }
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.stop.store(true, AtomicOrdering::Relaxed);
            let _ = w.handle.join();
        }
    }
}

impl Core {
    fn drop_connection(&mut self) {
        self.conn = None;
        self.sock = None;
        self.closing = false;
    }

    /// One RakNet update cycle: received datagrams into the reliability layer, the open-connection timers, the
    /// reliability-layer update (acks, resends, new messages), the dead-connection test, then the received
    /// messages: RakNet's internal ones (auth key, `0x22`) are answered here and go out with the next update, the
    /// others are queued for the main thread. Mirrors samp.dll 0x10041450.
    fn update(&mut self, now: Instant) {
        if self.sock.is_none() {
            return;
        }
        if self.phase == Phase::Opening {
            let now_ms = self.rak_ms(now);
            if now_ms > self.next_open_ms {
                if self.open_attempts < OPEN_REQUESTS {
                    self.next_open_ms = now_ms + OPEN_RETRY_MS;
                    self.send_open_request();
                } else {
                    crate::log!("net: no answer to {} open-connection requests", self.open_attempts);
                    self.end_attempt(NetPacket::AttemptFailed);
                    return;
                }
            }
        }
        let mut delivered = Vec::new();
        let mut buf = [0u8; 2048];
        while let Some(sock) = self.sock.as_ref() {
            match sock.recv(&mut buf) {
                Ok(n) => {
                    self.counters.datagrams_received += 1;
                    self.counters.bytes_received += n as u64;
                    self.on_datagram(&buf[..n], now, &mut delivered);
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
            self.events.push(Event::Packet(NetPacket::ConnectionLost));
            return;
        }
        self.flush_now(now);
        for msg in delivered {
            self.counters.messages_received += 1;
            self.on_message(&msg, now);
        }
        if self.closing {
            self.drop_connection();
        }
    }

    /// The reliability-layer update: everything due goes out now.
    fn flush_now(&mut self, now: Instant) {
        if let Some(conn) = self.conn.as_mut() {
            for dg in conn.poll(now) {
                self.send_plain_dg(&dg);
            }
        }
    }

    /// The attempt ended before a connection existed (no answer, full, banned).
    fn end_attempt(&mut self, p: NetPacket) {
        self.drop_connection();
        self.phase = Phase::Idle;
        self.events.push(Event::Packet(p));
    }

    /// RakNet's time in whole milliseconds.
    fn rak_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.epoch).as_millis() as u64
    }

    fn send_open_request(&mut self) {
        self.open_attempts += 1;
        if self.open_attempts == 1 || self.open_attempts.is_multiple_of(10) {
            crate::log!("net: open-connection request #{} to {}", self.open_attempts, self.server);
        }
        let req = open_connection_request(COOKIE.load(AtomicOrdering::Relaxed));
        self.send_plain_dg(&req);
    }

    fn send_plain_dg(&mut self, plain: &[u8]) {
        cipher::encrypt(plain, self.key, &mut self.wire);
        let Some(sock) = self.sock.as_ref() else { return };
        match sock.send(&self.wire) {
            Ok(n) => {
                self.counters.datagrams_sent += 1;
                self.counters.bytes_sent += n as u64;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => crate::log!("net: send error: {e}"),
        }
    }

    fn on_datagram(&mut self, data: &[u8], now: Instant, delivered: &mut Vec<Delivered>) {
        if self.phase == Phase::Opening {
            match parse_reply(data) {
                // Like the original: store the cookie and answer with the next request of the 1000 ms retry
                // timer, not at once (docs/phase3/parity-notes.md, "Connect cookie").
                Some(ServerReply::Cookie(c)) => COOKIE.store(c, AtomicOrdering::Relaxed),
                Some(ServerReply::Accepted) => {
                    crate::log!("net: open-connection accepted after {} request(s)", self.open_attempts);
                    let mut conn = Connection::connecting(now);
                    conn.send_with(&[id::CONNECTION_REQUEST], 8, Reliability::Reliable, 0, priority::SYSTEM);
                    self.conn = Some(conn);
                    self.phase = Phase::Connecting;
                }
                Some(ServerReply::ServerFull) => self.end_attempt(NetPacket::ServerFull),
                Some(ServerReply::Banned) => self.end_attempt(NetPacket::Banned),
                Some(ServerReply::Failed) => self.end_attempt(NetPacket::AttemptFailed),
                None => {}
            }
            return;
        }
        let Some(conn) = self.conn.as_mut() else { return };
        let Ok(got) = conn.handle_datagram(data, now) else { return };
        delivered.extend(got);
    }

    /// The connection ends (the acks of this update already went out).
    fn close(&mut self, reason: String) {
        crate::log!("net: session closed: {reason}");
        self.phase = Phase::Disconnected(reason);
        self.closing = true;
    }

    fn on_message(&mut self, msg: &Delivered, now: Instant) {
        let (_, body) = strip_timestamp(&msg.data);
        let Some(&first) = body.first() else { return };
        match first {
            // samp's RakNet answers the auth key itself while connecting.
            id::AUTH_KEY => {
                let Some(challenge) = auth::parse_challenge(body) else {
                    crate::log!("net: malformed auth challenge");
                    return;
                };
                match self.auth.response(&challenge) {
                    Some(resp) => {
                        let reply = auth::build_response(resp);
                        if let Some(c) = self.conn.as_mut() {
                            c.send_with(&reply, reply.len() * 8, Reliability::Reliable, 0, priority::SYSTEM);
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
                self.events.push(Event::Packet(NetPacket::Accepted));
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
                        self.events.push(Event::InitGame(g));
                    }
                    Some(Incoming::ConnectionRejected { reason }) => {
                        crate::log!("net: connection rejected: {}", session::rejection_reason(reason));
                        self.events.push(Event::Rejected(reason))
                    }
                    Some(other) => self.events.push(Event::Rpc(other)),
                    None => {}
                }
            }
            b if samp_proto::sync::INCOMING_IDS.contains(&b) => {
                self.counters.syncs_received += 1;
                if let Some(s) = samp_proto::sync::decode(&msg.data, msg.bit_len) {
                    self.events.push(Event::Sync(s));
                } else {
                    self.counters.syncs_bad += 1;
                    if self.counters.syncs_bad <= 20 {
                        let hex: String = msg.data.iter().take(64).map(|b| format!("{b:02x}")).collect();
                        crate::log!("net: undecodable sync packet ({} bits): {hex}", msg.bit_len);
                    }
                }
            }
            // RakNet ends the connection itself on these; samp's packet handler only adds its own lines /
            // Disconnect calls (`lifecycle`).
            id::DISCONNECTION_NOTIFICATION => {
                self.close("server closed the connection".into());
                self.events.push(Event::Packet(NetPacket::ServerClosed));
            }
            id::CONNECTION_BANNED => {
                self.close("banned from this server".into());
                self.events.push(Event::Packet(NetPacket::Banned));
            }
            id::INVALID_PASSWORD => {
                self.close("wrong server password".into());
                self.events.push(Event::Packet(NetPacket::InvalidPassword));
            }
            samp_proto::unconnected::id::NO_FREE_INCOMING_CONNECTIONS => {
                self.close("server full".into());
                self.events.push(Event::Packet(NetPacket::ServerFull));
            }
            ENCRYPTION_FAILED => self.events.push(Event::Packet(NetPacket::EncryptionFailed)),
            _ => {}
        }
    }

    /// RakNet's own answer to `0x22`, in the original client's order (docs/re/handshake.md, "Connection
    /// establishment"): `0x1e`, a ping, `0x29`, a keep-alive, all system priority except the low-priority
    /// keep-alive. They go out with the next update, and the update cycle pings once more after that
    /// (docs/phase3/net6-notes.md).
    fn on_accepted(&mut self, body: &[u8], now: Instant) {
        let Some(acc) = session::parse_accepted(body) else {
            crate::log!("net: malformed connection-accepted");
            return;
        };
        crate::log!("net: connection accepted, player id {}", acc.player_id);
        self.player_id = Some(acc.player_id);
        self.accepted_challenge = Some(acc.challenge);
        let Some(conn) = self.conn.as_mut() else { return };
        let mut w = BitWriter::new();
        w.write_u8(id::NEW_INCOMING_CONNECTION);
        w.write_bytes(&self.server.ip().octets());
        w.write_u16(self.server.port());
        conn.send_with(w.as_bytes(), w.bit_len(), Reliability::Reliable, 0, priority::SYSTEM);
        conn.ping(now);
        conn.send_with(&[id::RECEIVED_STATIC_DATA], 8, Reliability::Reliable, 0, priority::SYSTEM);
        conn.send_with(&[id::KEEP_ALIVE], 8, Reliability::Reliable, 0, priority::LOW);
        conn.mark_connected(now);
    }

    /// samp's packet handler on `0x22` (main thread): PlayerConnect (RPC 25). Mirrors samp.dll 0x1000AD80.
    fn send_player_connect(&mut self) {
        let Some(challenge) = self.accepted_challenge.take() else { return };
        let Some(conn) = self.conn.as_mut() else { return };
        let serial = client_serial();
        let payload = session::build_player_connect(&session::R4, &self.name, challenge, &serial);
        conn.send_bits(&rpc::encode(rpc::ids::PLAYER_CONNECT, &payload), Reliability::Reliable, 0);
        self.phase = Phase::Joining;
        crate::log!("net: PlayerConnect sent as {:?}", self.name);
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

/// The game string samp's client serial hashes: GTA's user-files path from its fifth character on, e.g.
/// `sers\<user>\Documents\GTA San Andreas User Files` (docs/re/handshake.md, "Client serial").
#[cfg(windows)]
const SERIAL_INPUT: u32 = 0x00C9_236C;

/// samp's client serial (`session::samp_serial` of the game string); outside the game a random one.
fn client_serial() -> String {
    #[cfg(windows)]
    {
        use crate::patch::Memory;
        let mut buf = [0u8; 256];
        if crate::patch::ProcessMemory.read(SERIAL_INPUT, &mut buf).is_ok() {
            let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return session::samp_serial(&buf[..n]);
        }
    }
    session::make_serial(&pseudo_random_hex40())
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

    /// Like the original: 6 open-connection requests 1 s apart on a new socket, then 0x1D; the next connect uses
    /// another source port (docs/phase3/reconnect-notes.md).
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
        // Each request is due once RakNet's ms time is past the previous one's + 1000.
        let ms = |n: u64| t0 + Duration::from_millis(n);
        server.set_nonblocking(true).unwrap();
        let mut at = 2;
        for i in 0..6u64 {
            if i > 0 {
                assert!(c.tick(ms(at - 1)).is_empty());
                assert!(server.recv_from(&mut buf).is_err(), "request {} only 1000 ms after the last", i + 1);
            }
            assert!(c.tick(ms(at)).is_empty());
            std::thread::sleep(Duration::from_millis(5));
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            first_port.get_or_insert(peer.port());
            at += 1001;
        }
        assert!(c.tick(ms(at - 1)).is_empty());
        assert!(server.recv_from(&mut buf).is_err(), "a 7th request");
        let ev = c.tick(ms(at));
        assert!(matches!(ev.as_slice(), [Event::Packet(NetPacket::AttemptFailed)]));
        assert_eq!(c.phase(), Phase::Idle);
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
        assert!(c.tick(Instant::now()).is_empty());
        assert_eq!(c.phase(), Phase::Idle);
        c.connect().unwrap();
        let t0 = Instant::now();
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
            c.tick(t0 + Duration::from_millis(OPEN_RETRY_MS + 2));
            if let Ok((n, _)) = server.recv_from(&mut buf) {
                assert_eq!(open_request_cookie(&buf[..n], addr.port()), Some(0x1234));
                got_second = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(got_second, "no second open-connection request after the cookie");
        server.send_to(&[0x19, 0x00], peer).unwrap();
        for _ in 0..200 {
            c.tick(Instant::now());
            if c.phase() == Phase::Connecting {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(c.phase(), Phase::Connecting);
        assert!(c.status_line().starts_with("net phase=connecting server=127.0.0.1:"));

        // The cookie is process-global: a new attempt (new socket) and even a new session start with it
        // instead of 0.
        for fresh in [false, true] {
            if fresh {
                c = NetClient::new(addr, "Tester", AuthTable::parse("").unwrap()).unwrap();
            }
            c.connect().unwrap();
            server.set_nonblocking(true).unwrap();
            while server.recv_from(&mut buf).is_ok() {} // the old connection's datagrams
            server.set_nonblocking(false).unwrap();
            c.tick(Instant::now());
            let (n, _) = server.recv_from(&mut buf).unwrap();
            assert_eq!(open_request_cookie(&buf[..n], addr.port()), Some(0x1234), "fresh session: {fresh}");
            c.disconnect_silent();
        }
    }

    /// The cookie an encrypted open-connection request carries.
    fn open_request_cookie(wire: &[u8], port: u16) -> Option<u16> {
        let mut plain = Vec::new();
        cipher::decrypt(wire, CipherKey::from_port(port), &mut plain).ok()?;
        match plain.as_slice() {
            [0x18, lo, hi] => Some(u16::from_le_bytes([*lo, *hi]) ^ samp_proto::unconnected::COOKIE_MASK),
            _ => None,
        }
    }
}
