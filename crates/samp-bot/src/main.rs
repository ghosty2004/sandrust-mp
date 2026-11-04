//! Headless test client: cookie exchange → RakNet connection → auth key →
//! join (RPC 25) → InitGame (RPC 139) → class selection (128) → spawn
//! (129 → 52) → on-foot sync (packet 207), walking in a small circle around
//! the spawn point. Prints chat, game texts, dialogs and player joins/quits.
//!
//! Usage: samp-bot [host:port] [--name NAME] [--seconds N] [--auth-table PATH] [--class N]

use std::f32::consts::TAU;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use samp_proto::auth::{self, AuthTable};
use samp_proto::bitstream::BitWriter;
use samp_proto::cipher::{self, CipherKey};
use samp_proto::connection::{Connection, Delivered};
use samp_proto::raknet::{Reliability, id, strip_timestamp};
use samp_proto::rpc::{self, ids};
use samp_proto::rpcs::{self, Incoming, RpcPayload, player, remote, ui};
use samp_proto::session;
use samp_proto::sync::player::{FootSync, StatsSync};
use samp_proto::sync::{SyncPacket, policy};
use samp_proto::types::{Quat, Vec3};
use samp_proto::unconnected::{ServerReply, open_connection_request, parse_reply};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Radius (m) and angular speed (rad/s) of the demo walk around the spawn point.
const WALK_RADIUS: f32 = 3.0;
const WALK_SPEED: f32 = 0.5;

struct Args {
    target: String,
    name: String,
    seconds: u64,
    auth_table: String,
    class: u32,
}

fn parse_args() -> Args {
    let mut a = Args {
        target: "127.0.0.1:7777".into(),
        name: "RustBot".into(),
        seconds: 15,
        auth_table: "data/auth_table.txt".into(),
        class: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => a.name = it.next().expect("--name needs a value"),
            "--seconds" => a.seconds = it.next().and_then(|s| s.parse().ok()).expect("--seconds needs a number"),
            "--auth-table" => a.auth_table = it.next().expect("--auth-table needs a path"),
            "--class" => a.class = it.next().and_then(|s| s.parse().ok()).expect("--class needs a number"),
            other => a.target = other.into(),
        }
    }
    a
}

struct Net {
    sock: UdpSocket,
    server: SocketAddr,
    key: CipherKey,
    wire: Vec<u8>,
}

impl Net {
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

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Connecting,
    Joining,
    ClassSelection,
    SpawnRequested,
    Spawned,
}

/// Game-side state of the bot.
struct Bot {
    name: String,
    class: u32,
    phase: Phase,
    player_id: u16,
    on_foot_rate: Duration,
    spawn: Option<player::SpawnInfo>,
    /// Centre of the walk circle (spawn point or last server teleport).
    centre: Vec3,
    heading_deg: f32,
    health: f32,
    armour: f32,
    spawned_at: Instant,
    last_sync: Option<Instant>,
    last_stats: Option<Instant>,
    money: i32,
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
    let mut net = Net { sock, server, key: CipherKey::from_port(server.port()), wire: Vec::new() };
    let mut buf = [0u8; 1500];

    open_connection(&mut net, &mut buf)?;

    let start = Instant::now();
    let mut conn = Connection::new(start);
    conn.send_bytes(&[id::CONNECTION_REQUEST], Reliability::Reliable, 0);
    let mut bot = Bot {
        name: args.name.clone(),
        class: args.class,
        phase: Phase::Connecting,
        player_id: 0,
        on_foot_rate: Duration::from_millis(policy::DEFAULT_ON_FOOT_RATE_MS.into()),
        spawn: None,
        centre: Vec3::default(),
        heading_deg: 0.0,
        health: 100.0,
        armour: 0.0,
        spawned_at: start,
        last_sync: None,
        last_stats: None,
        money: 0,
    };

    let deadline = start + Duration::from_secs(args.seconds);
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if bot.phase == Phase::Spawned && bot.last_sync.is_none_or(|t| now.duration_since(t) >= bot.on_foot_rate) {
            send_foot_sync(&mut bot, &mut conn, now);
        }
        // Stats (205) once per second while spawned, like the original client.
        if bot.phase == Phase::Spawned
            && bot
                .last_stats
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(policy::STATS_INTERVAL_MS.into()))
        {
            let stats = StatsSync { money: bot.money, drunk_level: 0 };
            let p = policy::send_params(StatsSync::PACKET_ID).expect("stats has send params");
            conn.send_bits(&stats.to_bits(), p.reliability, p.channel);
            bot.last_stats = Some(now);
        }
        for dg in conn.poll(now) {
            net.send_plain(&dg)?;
        }
        let Some(n) = net.recv(&mut buf, Duration::from_millis(5))? else { continue };
        let Ok(delivered) = conn.handle_datagram(&buf[..n], Instant::now()) else { continue };
        for msg in delivered {
            if !handle_message(&msg, &mut conn, &net, &table, &mut bot)? {
                return Ok(());
            }
        }
    }

    conn.send_bytes(&[id::DISCONNECTION_NOTIFICATION], Reliability::ReliableOrdered, 0);
    for dg in conn.poll(Instant::now()) {
        net.send_plain(&dg)?;
    }
    println!("done ({}s, phase {:?}), disconnected", args.seconds, bot.phase);
    Ok(())
}

fn open_connection(net: &mut Net, buf: &mut [u8]) -> Result<()> {
    let mut cookie = 0u16;
    for attempt in 1..=6 {
        net.send_plain(&open_connection_request(cookie))?;
        let deadline = Instant::now() + Duration::from_millis(1000);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(n) = net.recv(buf, left)? else { break };
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

fn send_rpc<P: RpcPayload>(conn: &mut Connection, p: &P, reliability: Reliability) {
    conn.send_bits(&p.to_rpc(), reliability, 0);
}

/// Returns false when the session should end.
fn handle_message(msg: &Delivered, conn: &mut Connection, net: &Net, table: &AuthTable, bot: &mut Bot) -> Result<bool> {
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
            let mut w = BitWriter::new();
            w.write_u8(id::NEW_INCOMING_CONNECTION);
            match net.server.ip() {
                std::net::IpAddr::V4(ip) => w.write_bytes(&ip.octets()),
                std::net::IpAddr::V6(_) => return Err("IPv6 servers are not supported".into()),
            }
            w.write_u16(net.server.port());
            conn.send_bits(&w, Reliability::Reliable, 0);
            // Same order as the original client (docs/re/handshake.md §1,
            // differential capture): ping, empty static data, keep-alive.
            conn.ping(Instant::now());
            conn.send_bytes(&[id::RECEIVED_STATIC_DATA], Reliability::Reliable, 0);
            conn.send_bytes(&[id::KEEP_ALIVE], Reliability::Reliable, 0);

            let serial = session::make_serial(&pseudo_random_hex40());
            let payload = session::build_player_connect(&session::R4, &bot.name, acc.challenge, &serial);
            conn.send_bits(&rpc::encode(ids::PLAYER_CONNECT, &payload), Reliability::Reliable, 0);
            bot.phase = Phase::Joining;
            println!("[join] sent PlayerConnect as {:?}", bot.name);
        }
        id::RPC => {
            let Some(rpc) = rpc::decode(&msg.data, msg.bit_len) else { return Ok(true) };
            match rpcs::decode(rpc.id, &mut rpc.reader()) {
                Some(incoming) => return handle_rpc(incoming, conn, bot),
                None => println!("[rpc] undecoded id {} ({} bits)", rpc.id, rpc.bit_len),
            }
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

fn handle_rpc(incoming: Incoming, conn: &mut Connection, bot: &mut Bot) -> Result<bool> {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match incoming {
        Incoming::InitGame(g) => {
            bot.player_id = g.player_id;
            bot.on_foot_rate = Duration::from_millis(g.on_foot_rate.clamp(10, 1000).into());
            println!(
                "[game] InitGame: server {:?}, our id {}, {} class(es), foot rate {} ms",
                g.server_name, g.player_id, g.spawns_available, g.on_foot_rate
            );
            bot.phase = Phase::ClassSelection;
            send_rpc(conn, &player::PlayerRequestClass { class_index: bot.class }, Reliability::Reliable);
            println!("[class] requested class {}", bot.class);
        }
        Incoming::ConnectionRejected { reason } => {
            return Err(format!("connection rejected: {}", session::rejection_reason(reason)).into());
        }
        Incoming::Player(p) => handle_player_rpc(p, conn, bot),
        Incoming::Ui(ui::Incoming::SendClientMessage(m)) => println!("[chat {:08x}] {}", m.colour, text(&m.message)),
        Incoming::Ui(ui::Incoming::SendGameText(g)) => println!("[gametext] {}", text(&g.text)),
        Incoming::Ui(ui::Incoming::ShowDialog(d)) => {
            println!("[dialog {}] {}: {}", d.dialog_id, text(&d.title), text(&d.body))
        }
        Incoming::Remote(remote::Incoming::PlayerJoin(j)) => {
            println!("[players] {} joined (id {})", text(&j.name), j.player_id)
        }
        Incoming::Remote(remote::Incoming::PlayerQuit(q)) => {
            println!("[players] id {} left (reason {})", q.player_id, q.reason)
        }
        _ => {}
    }
    Ok(true)
}

fn handle_player_rpc(p: player::Incoming, conn: &mut Connection, bot: &mut Bot) {
    use player::Incoming as P;
    match p {
        P::PlayerRequestClassResponse(r) => {
            if r.selectable == 0 {
                println!("[class] server refused class {}", bot.class);
                return;
            }
            println!("[class] skin {}, team {}, spawn at {:?}", r.spawn.skin, r.spawn.team, r.spawn.pos);
            bot.spawn = Some(r.spawn);
            if bot.phase == Phase::ClassSelection {
                bot.phase = Phase::SpawnRequested;
                send_rpc(conn, &player::PlayerRequestSpawn, Reliability::Reliable);
                println!("[spawn] requested");
            }
        }
        P::SetSpawnInfo(s) => bot.spawn = Some(s.spawn),
        P::PlayerRequestSpawnResponse(r) => {
            if r.response != 0 && (r.response == 2 || bot.phase == Phase::SpawnRequested) {
                spawn(bot, conn);
            } else if bot.phase == Phase::SpawnRequested {
                println!("[spawn] refused by server");
                bot.phase = Phase::ClassSelection;
            }
        }
        P::SetPlayerPosition(s) => {
            bot.centre = s.pos;
            println!("[pos] teleported to {:?}", s.pos);
        }
        P::SetPlayerFacingAngle(a) => bot.heading_deg = a.angle,
        P::SetPlayerHealth(h) => bot.health = h.health,
        P::SetPlayerArmour(a) => bot.armour = a.armour,
        P::GivePlayerMoney(m) => bot.money = bot.money.wrapping_add(m.amount),
        P::ResetPlayerMoney(_) => bot.money = 0,
        P::ForcePlayerClassSelection(_) => println!("[class] server forces class selection on next death"),
        _ => {}
    }
}

fn spawn(bot: &mut Bot, conn: &mut Connection) {
    let Some(s) = bot.spawn else {
        println!("[spawn] allowed, but no spawn info yet");
        return;
    };
    bot.centre = Vec3 { z: s.pos.z, ..s.pos };
    bot.heading_deg = s.rotation;
    bot.health = 100.0;
    bot.spawned_at = Instant::now();
    bot.phase = Phase::Spawned;
    send_rpc(conn, &player::PlayerSpawn, Reliability::ReliableOrdered);
    println!("[spawn] spawned at {:?}, sending on-foot sync every {:?}", s.pos, bot.on_foot_rate);
}

/// Walk around `centre` and send one 207 packet.
fn send_foot_sync(bot: &mut Bot, conn: &mut Connection, now: Instant) {
    let t = now.duration_since(bot.spawned_at).as_secs_f32();
    let angle = (t * WALK_SPEED) % TAU;
    let position = Vec3 {
        x: bot.centre.x + WALK_RADIUS * angle.cos(),
        y: bot.centre.y + WALK_RADIUS * angle.sin(),
        z: bot.centre.z,
    };
    // GTA heading h faces (-sin h, cos h), which is the tangent of the
    // counter-clockwise circle at `angle`, so the heading equals `angle`.
    let heading = angle;
    let speed = WALK_RADIUS * WALK_SPEED;
    let velocity_per_frame = speed / 50.0; // GTA move speed is in units per 1/50 s
    let sync = FootSync {
        up_down: -128, // forward on the left stick
        position,
        // Heading h <-> quaternion (cos h/2, 0, 0, -sin h/2): remote-control-onfoot.md.
        rotation: Quat { w: (heading / 2.0).cos(), x: 0.0, y: 0.0, z: -(heading / 2.0).sin() },
        health: bot.health.clamp(0.0, 255.0) as u8,
        armour: bot.armour.clamp(0.0, 255.0) as u8,
        velocity: Vec3 { x: -heading.sin() * velocity_per_frame, y: heading.cos() * velocity_per_frame, z: 0.0 },
        ..FootSync::default()
    };
    let params = policy::send_params(FootSync::PACKET_ID).expect("foot sync has send params");
    conn.send_bits(&sync.to_bits(), params.reliability, params.channel);
    bot.last_sync = Some(now);
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
