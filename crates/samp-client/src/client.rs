//! The in-game client session (Windows only): network session and connection lifecycle, class selection, local
//! spawn and death, local sync, the S→C player RPCs, and the per-frame tick that drives the pools (remote players,
//! vehicles, objects, entities, combat, behaviour). Entry points: [`Client::start`] and [`Client::tick`].
//! Everything runs on the game's main thread from the per-frame hook, except [`Client::status_line`] /
//! [`Client::request_debug_spawn`], which only touch plain data.
//! Specs: docs/re/subsystems/player-state.md, docs/re/subsystems/remote-players-sync.md,
//! docs/re/subsystems/session-misc.md.
//!
//! Layout: `frame` (the per-frame tick in samp's main-loop order and the CGame::Process steps), `session`
//! (network events, lifecycle actions, the reset routine, the connecting scene), `rpc` / `rpc_player` (the RPC
//! dispatch and its handlers), `spawn` (class selection, spawn, death and respawn), `sync` (the local tick's
//! per-frame commands and sync senders, surfing, head look, camera target), `ped` (operations on the local ped),
//! `status` (control-channel queries and requests) and `gta` (free helpers over game memory and functions).

mod frame;
mod gta;
mod ped;
mod rpc;
mod rpc_player;
mod session;
mod spawn;
mod status;
mod sync;

use std::time::Instant;

use samp_proto::auth::AuthTable;
use samp_proto::rpcs::player;
use samp_proto::sync::policy;

use crate::classsel::ClassSelection;
use crate::footsync::FootSender;
use crate::lifecycle::NetGame;
use crate::netclient::NetClient;
use crate::script::Runner;

pub(crate) use gta::{load_model, ped_in_world, set_hud, set_ped_skin};

/// Environment variable with the (Windows) path of `auth_table.txt`.
pub const AUTH_TABLE_ENV: &str = "SAMP_RS_AUTH_TABLE";

/// Ped state `PEDSTATE_IDLE` (gta-reversed `Enums/ePedState.h`, like the other `PEDSTATE_*`).
const PEDSTATE_IDLE: u32 = 1;
const PEDSTATE_SNIPER: u32 = 12;
const PEDSTATE_DRIVING: u32 = 50;
const PEDSTATE_DIE: u32 = 54;
const PEDSTATE_DEAD: u32 = 55;
/// `TheCamera.m_aCams[0].Front` (what samp's camera-front pointer holds).
const CAMERA_FRONT: u32 = 0xB6F32C;
/// The in-vehicle bit of the first `CPed` flag word.
const PED_FLAG_IN_VEHICLE: u32 = 1 << 8;

/// F4, polled with `GetAsyncKeyState` at the end of the local tick.
const VK_F4: i32 = 0x73;
/// The type-4 client line F4 prints.
pub const F4_LINE: &str = "Returning to class selection after next death";

pub struct Client {
    net: NetClient,
    runner: Runner,
    class_sel: ClassSelection,
    sender: FootSender,
    spawn_info: Option<player::SpawnInfo>,
    spawned: bool,
    /// RPC 74 / F4 flag: re-enter class selection on the next death.
    force_class_selection: bool,
    on_foot_rate: u32,
    last_stats: Option<Instant>,
    debug_spawn: bool,
    fade_patched: bool,
    last_net_log: Option<Instant>,
    spawned_once: bool,
    first_frame_done: bool,
    /// Clock and weather (world.rs): settings hour/minute/weather, clock state.
    world: crate::world::World,
    /// InitGame "zone names" (settings +0x21): DISPLAY_ZONE_NAMES every frame from 1000 ms after the spawn.
    zone_names: bool,
    zone_names_from: Option<Instant>,
    /// Last interior reported with RPC 118 (local player +0x1cf, starts at 0).
    last_interior: u8,
    /// Phone special action active (local ped object +0x48).
    phone: bool,
    /// Local surf state (local-player object +0x2D4..+0x2EE, surf.rs).
    surf: crate::surf::LocalSurf,
    /// Held item / piss state of the local ped (`specials.rs`).
    specials: crate::specials::PedExtras,
    /// Local head look: offset refreshed every 1000 ms, LookAt every 200 ms.
    look_offset: [f32; 3],
    look_refresh: Option<Instant>,
    look_last: Option<Instant>,
    /// Remote players (players.rs).
    players: crate::players::Players,
    /// Vehicles (vehicles.rs).
    vehicles: crate::vehicles::Vehicles,
    /// Combat (combatclient.rs).
    combat: crate::combatclient::Combat,
    /// Local player "dead" flag (local player +0xF4): set with RPC 53, cleared by the respawn.
    dead: bool,
    /// InitGame class count (class selection after a forced reselection).
    class_count: u32,
    /// samp's netgame state and connect timer (lifecycle.rs).
    netgame: NetGame,
    /// Start of the netgame clock (GetTickCount stand-in).
    clock: Instant,
    /// `host`, `port` for the `Connecting to` line.
    connect_target: (String, u16),
    /// Objects (objects.rs).
    objects: crate::objects::Objects,
    /// Pickups, actors, checkpoints, gang zones, map icons, 3D labels (entities.rs).
    entities: crate::entities::Entities,
    /// Camera, spectating, bounds, drunk, animations, ClientCheck (behaviourclient.rs).
    behaviour: crate::behaviourclient::Behaviour,
    /// Camera target reporting (RPC 170 → 168, camtarget.rs).
    cam_target: crate::camtarget::CamTarget,
}

impl Client {
    /// Creates the session from the command line (`-n/-h/-p`) and the auth table named by [`AUTH_TABLE_ENV`].
    pub fn start(args: &crate::cmdline::ConnectArgs) -> Result<Self, String> {
        let host = args.host.clone().unwrap_or_else(|| "127.0.0.1".into());
        let port = args.port.unwrap_or(7777);
        let name = args.name.clone().unwrap_or_else(|| "RustClient".into());
        let path = std::env::var(AUTH_TABLE_ENV)
            .map_err(|_| format!("{AUTH_TABLE_ENV} is not set (path of data/auth_table.txt)"))?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read auth table {path}: {e}"))?;
        let auth = AuthTable::parse(&text).map_err(|e| format!("auth table {path}: parse error at line {}", e.line))?;
        let server = crate::netclient::resolve(&host, port).map_err(|e| format!("resolve {host}:{port}: {e}"))?;
        crate::log!("client: connecting to {server} as {name:?} (auth table {path}, {} entries)", auth.len());
        let net = NetClient::new(server, &name, auth).map_err(|e| format!("udp socket: {e}"))?;
        // "Started"; the connect tick starts the session more than 3000 ms after the netgame constructor
        // (lifecycle.rs).
        crate::ui::on_netgame_start(&host, port, &name);
        let now = Instant::now();
        Ok(Self {
            net,
            runner: Runner::new(),
            class_sel: ClassSelection::new(now),
            sender: FootSender::default(),
            spawn_info: None,
            spawned: false,
            force_class_selection: false,
            on_foot_rate: policy::DEFAULT_ON_FOOT_RATE_MS,
            last_stats: None,
            debug_spawn: false,
            fade_patched: false,
            last_net_log: None,
            spawned_once: false,
            first_frame_done: false,
            world: crate::world::World::default(),
            zone_names: false,
            zone_names_from: None,
            last_interior: 0,
            phone: false,
            surf: Default::default(),
            specials: Default::default(),
            look_offset: [0.0; 3],
            look_refresh: None,
            look_last: None,
            players: crate::players::Players::default(),
            vehicles: crate::vehicles::Vehicles::default(),
            combat: crate::combatclient::Combat::new(),
            dead: false,
            class_count: 0,
            netgame: NetGame::new(0),
            clock: now,
            connect_target: (host.clone(), port),
            objects: crate::objects::Objects::default(),
            entities: crate::entities::Entities::default(),
            behaviour: crate::behaviourclient::Behaviour::new(),
            cam_target: crate::camtarget::CamTarget::default(),
        })
    }
}
