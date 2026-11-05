//! Game-side state machine of the bot: class selection and spawn, the
//! scripted modes (walk / idle / drive / passenger / shoot), and the sync
//! senders with the original client's rates and filters.
//!
//! Specs: docs/re/subsystems/remote-players-sync.md (§0.4 rates and
//! duplicate suppression, §2 layouts), vehicles.md (26/154/70/71),
//! remote-control-vehicles.md §5, remote-control-combat.md §5, player-state.md
//! (spawn, death, 115). Values the specs leave open are listed in
//! docs/phase3/bot2-notes.md.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use samp_proto::rpcs::{Incoming, player, remote, ui, vehicle};
use samp_proto::session::{self, InitGame};
use samp_proto::sync::player::{AimSync, Animation, BulletSync, FootSync, SpectatorSync, StatsSync, hit_type};
use samp_proto::sync::vehicle::{PassengerSync, VehicleSync};
use samp_proto::sync::{IncomingSync, SyncPacket, WeaponByte, policy};
use samp_proto::types::Vec3;

use crate::client::Event;
use crate::gate::{DuplicateGate, RotationFilter};
use crate::geom::{self, add, forward, right, scale, sub, v3};
use crate::path::{Motion, Pedal, Shape};
use crate::weapons::{self, Inventory, WeaponsTracker};
use crate::world::World;
use crate::{Outbox, Result, send_rpc};

/// Key bits (remote-players-sync.md §0.3).
pub mod keys {
    pub const ACTION: u16 = 1;
    /// In a vehicle: horn.
    pub const CROUCH: u16 = 2;
    pub const FIRE: u16 = 4;
    /// In a vehicle: accelerate.
    pub const SPRINT: u16 = 8;
    pub const SECONDARY_ATTACK: u16 = 16;
    /// In a vehicle: brake / reverse.
    pub const JUMP: u16 = 32;
    /// On foot: aim; in a vehicle: handbrake.
    pub const AIM: u16 = 128;
}

/// Walk mode: radius (m) and angular speed (rad/s) of the circle around the spawn point.
const WALK_RADIUS: f32 = 3.0;
const WALK_SPEED: f32 = 0.5;
/// On-foot speed towards a vehicle door (m/s, running).
const RUN_SPEED: f32 = 4.0;
/// Distance to the vehicle at which the bot "presses" enter (driver key /
/// passenger key). The passenger poll requires < 8.0 (vehicles.md 26 C→S).
const ENTER_REQUEST_RANGE: f32 = 4.0;
const PASSENGER_POLL_RANGE: f32 = 8.0;
/// Duration of the get-in animation once at the door, and of the exit.
const GET_IN: Duration = Duration::from_millis(1500);
const GET_OUT: Duration = Duration::from_millis(1200);
/// Give up an enter task that has not reached the door after this long.
const ENTER_TIMEOUT: Duration = Duration::from_millis(6000);
/// Wasted sequence before the client respawns.
const WASTED: Duration = Duration::from_millis(4000);
/// Weapon raise time before the first shot.
const AIM_RAISE: Duration = Duration::from_millis(300);
/// Heading sway of an aiming bot (rad), in step with the camera sway of its aim sync.
const AIM_SWAY_YAW: f32 = 0.004;
/// Class selection: Shift (spawn request) only works this long after it began.
const CLASS_SELECTION_MIN: Duration = Duration::from_millis(2000);
/// Special action while an enter-vehicle task runs.
const SPECIAL_ACTION_ENTER_VEHICLE: u8 = 3;
/// Body part reported in 115 (torso).
const BODYPART_TORSO: u32 = 3;
/// Aim-sync zoom for FOV 70: trunc(((70 − 35) / 35) × 63).
const ZOOM_FOV70: u8 = 63;
/// Aim-sync aspect for 16:9: trunc((1.7778 − 1) × 255).
const ASPECT_16_9: u8 = 198;
const NONE_ID: u16 = 0xffff;

/// What the bot does once spawned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Walk a 3 m circle around the spawn point (the original behaviour).
    Walk,
    /// Stand still.
    Idle,
    /// Enter a vehicle as driver and drive a path.
    Drive,
    /// Enter a vehicle as passenger.
    Passenger,
    /// Aim at a player and fire.
    Shoot,
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        Ok(match s {
            "walk" => Mode::Walk,
            "idle" => Mode::Idle,
            "drive" => Mode::Drive,
            "passenger" => Mode::Passenger,
            "shoot" => Mode::Shoot,
            _ => return Err(format!("unknown mode {s:?} (walk|idle|drive|passenger|shoot)")),
        })
    }
}

/// A player given by id or by name (resolved through PlayerJoin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerRef {
    Id(u16),
    Name(String),
}

#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    pub class: u32,
    pub mode: Mode,
    /// Vehicle to enter (default: nearest streamed, undriven vehicle).
    pub vehicle: Option<u16>,
    /// Passenger seat as sent in 211 (1 = front passenger).
    pub seat: u8,
    pub shape: Shape,
    /// Cruise speed (m/s).
    pub speed: f32,
    /// Extra key bits held while driving (e.g. 2 horn, 128 handbrake).
    pub extra_keys: u16,
    /// Driving: tow this vehicle (200 trailer id + 210 every in-car tick, the trailer 9 m behind the cab).
    pub trailer: Option<u16>,
    pub siren: bool,
    /// Passenger drive-by (211 bit 6).
    pub drive_by: bool,
    /// Leave the vehicle (RPC 154) this long after sitting down.
    pub exit_after: Option<Duration>,
    pub target: Option<PlayerRef>,
    /// Weapon to shoot with (must be given by the server); default: armed.
    pub weapon: Option<u8>,
    /// 206 hit type (default 1 = player).
    pub hit_type: u8,
    /// 206 hit id override (vehicle/object ids for hit types 2/3).
    pub hit_id: Option<u16>,
    /// 206 offset override.
    pub offset: Option<Vec3>,
    /// Time between shots (default: per weapon).
    pub fire_ms: Option<u32>,
    /// Fixed foot/in-car sync interval instead of rate + N (measurement only).
    pub sync_ms: Option<u32>,
    /// Special action sent in every 207 on foot (measurement only, e.g. 24 = cuffed).
    pub special_action: Option<u8>,
    /// 207 animation words sent in turn, each for `anim_step` (measurement only, e.g. remote parachute anims).
    pub anim_seq: Vec<u32>,
    pub anim_step: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "RustBot".into(),
            class: 0,
            mode: Mode::Walk,
            vehicle: None,
            seat: 1,
            shape: Shape::Circle { radius: 10.0 },
            speed: 5.0,
            extra_keys: 0,
            trailer: None,
            siren: false,
            drive_by: false,
            exit_after: None,
            target: None,
            weapon: None,
            hit_type: hit_type::PLAYER,
            hit_id: None,
            offset: None,
            fire_ms: None,
            sync_ms: None,
            special_action: None,
            anim_seq: Vec::new(),
            anim_step: Duration::from_millis(2000),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    Joining,
    ClassSelection { since: Instant },
    SpawnRequested,
    Spawned,
    Dead { since: Instant },
}

/// What the spawned ped is doing.
#[derive(Clone, Copy, Debug)]
pub enum Activity {
    OnFoot,
    /// Walking towards a vehicle, enter not yet requested.
    Approach { vehicle: u16, passenger: bool },
    /// RPC 26 sent: walking to the door, then getting in.
    Entering { vehicle: u16, seat: u8, since: Instant, at_door: Option<Instant> },
    Driving { vehicle: u16, motion: Motion, since: Instant },
    Riding { vehicle: u16, seat: u8, since: Instant },
    /// Leave-car task running (still seated): 200/211 continue.
    Exiting { vehicle: u16, seat: u8, since: Instant },
}

/// Rates from InitGame.
#[derive(Clone, Copy, Debug)]
struct Rates {
    on_foot: u32,
    in_car: u32,
    weapon: u32,
    lan: bool,
    /// Lag-comp selector: InitGame 1 → 0, 2 → 1, else 2.
    lag_comp: u8,
}

impl Default for Rates {
    fn default() -> Self {
        Self {
            on_foot: policy::DEFAULT_ON_FOOT_RATE_MS,
            in_car: policy::DEFAULT_IN_CAR_RATE_MS,
            weapon: policy::DEFAULT_WEAPON_RATE_MS,
            lan: false,
            lag_comp: 0,
        }
    }
}

#[derive(Debug, Default)]
struct Timers {
    foot: Option<Instant>,
    car: Option<Instant>,
    aim: Option<Instant>,
    stats: Option<Instant>,
    spectate: Option<Instant>,
    last_tick: Option<Instant>,
}

/// Last foot keys sent (207 is also sent when these change).
type FootKeys = (u16, i16, i16);

pub struct Bot {
    pub cfg: Config,
    pub phase: Phase,
    pub player_id: u16,
    pub world: World,
    pub activity: Activity,
    pub inv: Inventory,
    pub pos: Vec3,
    /// Radians.
    pub heading: f32,
    pub health: f32,
    pub armour: f32,
    pub money: i32,
    rates: Rates,
    spawn: Option<player::SpawnInfo>,
    class_info: bool,
    force_class: bool,
    spectating: bool,
    /// Walk circle centre and start.
    centre: Vec3,
    walk_since: Instant,
    /// First 207 of the `--anim-seq` sequence.
    anim_t0: Option<Instant>,
    vel: Vec3,
    keys: u16,
    lr: i16,
    ud: i16,
    special_action: u8,
    gate: DuplicateGate,
    foot_rot: RotationFilter,
    car_rot: RotationFilter,
    last_foot_keys: Option<FootKeys>,
    weapons_tx: WeaponsTracker,
    t: Timers,
    /// The drive/passenger mode has already used its vehicle.
    vehicle_done: bool,
    shutting_down: bool,
    aim_since: Option<Instant>,
    next_shot: Option<Instant>,
    shots: u32,
    last_damage: Option<(u16, u8)>,
    waiting_note: Option<&'static str>,
    /// A shot is due this tick (fired from the sync step, before 207/203).
    pending_shot: bool,
}

impl Bot {
    pub fn new(cfg: Config, now: Instant) -> Self {
        Self {
            cfg,
            phase: Phase::Connecting,
            player_id: 0,
            world: World::default(),
            activity: Activity::OnFoot,
            inv: Inventory::default(),
            pos: Vec3::default(),
            heading: 0.0,
            health: 100.0,
            armour: 0.0,
            money: 0,
            rates: Rates::default(),
            spawn: None,
            class_info: false,
            force_class: false,
            spectating: false,
            centre: Vec3::default(),
            walk_since: now,
            anim_t0: None,
            vel: Vec3::default(),
            keys: 0,
            lr: 0,
            ud: 0,
            special_action: 0,
            gate: DuplicateGate::default(),
            foot_rot: RotationFilter::default(),
            car_rot: RotationFilter::default(),
            last_foot_keys: None,
            weapons_tx: WeaponsTracker::default(),
            t: Timers::default(),
            vehicle_done: false,
            shutting_down: false,
            aim_since: None,
            next_shot: None,
            shots: 0,
            last_damage: None,
            waiting_note: None,
            pending_shot: false,
        }
    }

    // ---------------------------------------------------------------- events

    /// Handle one event. Returns false when the session has ended.
    pub fn handle_event(&mut self, ev: Event, now: Instant, out: &mut impl Outbox) -> Result<bool> {
        match ev {
            Event::Joining => self.phase = Phase::Joining,
            Event::Closed => {
                println!("[conn] server closed the connection");
                return Ok(false);
            }
            Event::UndecodedRpc(id, bits) => println!("[rpc] undecoded id {id} ({bits} bits)"),
            Event::Sync(s) => self.handle_sync(&s, now, out),
            Event::Rpc(rpc) => {
                self.world.apply_rpc(&rpc);
                self.handle_rpc(rpc, now, out)?;
            }
        }
        Ok(true)
    }

    fn handle_rpc(&mut self, rpc: Incoming, now: Instant, out: &mut impl Outbox) -> Result<()> {
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        match rpc {
            Incoming::InitGame(g) => self.init_game(&g, now, out),
            // RPC 40 (gmx): like the client's reset routine, no sync until the server's next InitGame. A sync sent
            // in that gap reaches the restarted gamemode before the class request, the server then takes the
            // player as on foot and ignores its class request (soak, derby: stuck in class selection for good).
            Incoming::Session(samp_proto::rpcs::session::Incoming::PlayerClose(_)) => {
                println!("[game] server restarting; no sync until InitGame");
                self.phase = Phase::Joining;
                self.activity = Activity::OnFoot;
                self.spectating = false;
            }
            Incoming::ConnectionRejected { reason } => {
                return Err(format!("connection rejected: {}", session::rejection_reason(reason)).into());
            }
            Incoming::Player(p) => self.handle_player_rpc(p, now, out),
            Incoming::Vehicle(v) => self.handle_vehicle_rpc(v, now),
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
            Incoming::Remote(remote::Incoming::TogglePlayerSpectating(s)) => {
                let on = s.enable != 0;
                if self.spectating && !on {
                    // Leaving spectator mode: class selection if pending, else spawn.
                    self.spectating = false;
                    if self.force_class {
                        self.enter_class_selection(now, out);
                    } else {
                        self.do_spawn(now, out);
                    }
                }
                self.spectating = on;
            }
            _ => {}
        }
        Ok(())
    }

    fn init_game(&mut self, g: &InitGame, now: Instant, out: &mut impl Outbox) {
        self.player_id = g.player_id;
        self.rates = Rates {
            on_foot: g.on_foot_rate.clamp(10, 1000),
            in_car: g.in_car_rate.clamp(10, 1000),
            weapon: g.weapon_rate.clamp(10, 1000),
            lan: g.lan_mode,
            lag_comp: match g.lag_compensation {
                1 => 0,
                2 => 1,
                _ => 2,
            },
        };
        println!(
            "[game] InitGame: server {:?}, our id {}, {} class(es), rates foot {} car {} weapon {} ms, lagcomp {}",
            g.server_name, g.player_id, g.spawns_available, g.on_foot_rate, g.in_car_rate, g.weapon_rate,
            g.lag_compensation
        );
        self.enter_class_selection(now, out);
    }

    fn enter_class_selection(&mut self, now: Instant, out: &mut impl Outbox) {
        self.phase = Phase::ClassSelection { since: now };
        self.class_info = false;
        self.force_class = false;
        send_rpc(out, &player::PlayerRequestClass { class_index: self.cfg.class });
        println!("[class] requested class {}", self.cfg.class);
    }

    fn handle_player_rpc(&mut self, p: player::Incoming, now: Instant, out: &mut impl Outbox) {
        use player::Incoming as P;
        match p {
            P::PlayerRequestClassResponse(r) => {
                if r.selectable == 0 {
                    println!("[class] server refused class {}", self.cfg.class);
                    return;
                }
                println!("[class] skin {}, team {}, spawn at {:?}", r.spawn.skin, r.spawn.team, r.spawn.pos);
                self.spawn = Some(r.spawn);
                self.class_info = true;
            }
            P::SetSpawnInfo(s) => self.spawn = Some(s.spawn),
            P::PlayerRequestSpawnResponse(r) => {
                if r.response == 2 || (r.response != 0 && self.phase == Phase::SpawnRequested) {
                    self.do_spawn(now, out);
                } else if self.phase == Phase::SpawnRequested {
                    println!("[spawn] refused by server");
                    self.phase = Phase::ClassSelection { since: now };
                }
            }
            P::SetPlayerPosition(s) => {
                if self.on_foot() {
                    self.pos = s.pos;
                    self.centre = s.pos;
                    self.walk_since = now;
                }
                println!("[pos] teleported to {:?}", s.pos);
            }
            P::SetPlayerFacingAngle(a) => self.heading = a.angle.to_radians(),
            P::SetPlayerHealth(h) => self.health = h.health,
            P::SetPlayerArmour(a) => self.armour = a.armour,
            P::GivePlayerMoney(m) => self.money = self.money.wrapping_add(m.amount),
            P::ResetPlayerMoney(_) => self.money = 0,
            P::ForcePlayerClassSelection(_) => {
                self.force_class = true;
                println!("[class] server forces class selection on next death");
            }
            P::GivePlayerWeapon(g) => {
                if g.weapon <= 46 {
                    self.inv.give(g.weapon as u8, g.ammo);
                    println!("[weapon] given {} ({} ammo), armed {}", g.weapon, g.ammo, self.inv.armed);
                }
            }
            P::ResetPlayerWeapons(_) => self.inv.clear(),
            P::SetPlayerArmedWeapon(a) => {
                if a.weapon <= 46 {
                    self.inv.arm(a.weapon as u8);
                }
            }
            P::SetPlayerAmmo(a) => self.inv.set_ammo(a.weapon, a.ammo),
            _ => {}
        }
    }

    fn handle_vehicle_rpc(&mut self, v: vehicle::Incoming, now: Instant) {
        use vehicle::Incoming as V;
        match v {
            V::PutPlayerInVehicle(p) => {
                let Some(veh) = self.world.vehicles.get(&p.vehicle_id) else {
                    println!("[car] PutPlayerInVehicle {}: vehicle not streamed, ignored", p.vehicle_id);
                    return;
                };
                if p.seat == 0 {
                    let cruise = if self.cfg.mode == Mode::Idle { 0.0 } else { self.cfg.speed };
                    let motion = Motion::new(self.cfg.shape, veh.pos, veh.heading, cruise);
                    self.activity = Activity::Driving { vehicle: p.vehicle_id, motion, since: now };
                    println!("[car] put into vehicle {} as driver", p.vehicle_id);
                } else {
                    self.activity = Activity::Riding { vehicle: p.vehicle_id, seat: p.seat, since: now };
                    println!("[car] put into vehicle {} seat {}", p.vehicle_id, p.seat);
                }
                self.vehicle_done = true;
            }
            V::RemovePlayerFromVehicle(_) => {
                // Leave-car task without RPC 154 (the caller is not player input).
                if let Some((vehicle, seat)) = self.seated() {
                    self.activity = Activity::Exiting { vehicle, seat, since: now };
                    println!("[car] removed from vehicle {vehicle} by the server");
                }
            }
            // Occupants of a vehicle that streams out are put on foot.
            V::StreamOutVehicle(s) if self.seated().is_some_and(|(v, _)| v == s.vehicle_id) => {
                self.activity = Activity::OnFoot;
            }
            _ => {}
        }
    }

    fn handle_sync(&mut self, s: &IncomingSync, now: Instant, out: &mut impl Outbox) {
        self.world.apply_sync(s, now);
        if let IncomingSync::Bullet(b) = s
            && b.sync.hit_type == hit_type::PLAYER
            && b.sync.hit_id == self.player_id
            && b.player_id != self.player_id
            && self.phase == Phase::Spawned
            && !self.spectating
        {
            self.take_hit(b.player_id, b.sync.weapon_id, out);
        }
    }

    /// A remote shot hit the local ped: GTA damage → RPC 115 (taking).
    fn take_hit(&mut self, shooter: u16, weapon: u8, out: &mut impl Outbox) {
        let damage = weapons::reported_damage(weapon);
        if damage <= 0.0 {
            return;
        }
        let to_armour = damage.min(self.armour);
        self.armour -= to_armour;
        self.health -= damage - to_armour;
        self.last_damage = Some((shooter, weapon));
        send_rpc(
            out,
            &player::OnPlayerGiveTakeDamage {
                taking: true,
                player_id: shooter,
                damage,
                weapon_id: weapon.into(),
                bodypart: BODYPART_TORSO,
            },
        );
        println!("[damage] hit by {shooter} with {weapon}: -{damage:.1}, health {:.1}", self.health);
    }

    // ---------------------------------------------------------------- spawn / death

    fn do_spawn(&mut self, now: Instant, out: &mut impl Outbox) {
        let Some(s) = self.spawn else {
            println!("[spawn] allowed, but no spawn info yet");
            return;
        };
        self.pos = s.pos;
        self.centre = s.pos;
        self.heading = s.rotation.to_radians();
        self.health = 100.0;
        self.armour = 0.0;
        self.special_action = 0;
        self.activity = Activity::OnFoot;
        self.inv.clear();
        for i in [2, 1, 0] {
            if s.weapons[i] != player::SpawnInfo::NO_WEAPON && s.weapons[i] <= 46 {
                self.inv.give(s.weapons[i] as u8, s.ammo[i]);
            }
        }
        self.walk_since = now;
        self.phase = Phase::Spawned;
        self.spectating = false;
        send_rpc(out, &player::PlayerSpawn);
        println!("[spawn] spawned at {:?}, mode {:?}", s.pos, self.cfg.mode);
    }

    fn die(&mut self, now: Instant, out: &mut impl Outbox) {
        // One forced 200 when dying as driver.
        if let Activity::Driving { vehicle, motion, .. } = self.activity {
            let sync = self.driver_sync(vehicle, &motion);
            crate::send_sync(out, &sync);
        }
        let (reason, killer) = match self.last_damage {
            Some((p, w)) if w < 43 || w == 50 || w == 51 => (w, p),
            _ => (255, player::INVALID_PLAYER_ID),
        };
        send_rpc(out, &player::OnPlayerDeath { reason, killer_id: killer });
        println!("[death] died (reason {reason}, killer {killer})");
        self.phase = Phase::Dead { since: now };
        self.activity = Activity::OnFoot;
        self.vel = Vec3::default();
        self.last_damage = None;
    }

    // ---------------------------------------------------------------- tick

    /// Ask the bot to wind down: leave the vehicle if seated.
    pub fn request_shutdown(&mut self) {
        self.shutting_down = true;
    }

    /// True once the bot can disconnect (not inside a vehicle).
    pub fn ready_to_quit(&self) -> bool {
        self.phase != Phase::Spawned || matches!(self.activity, Activity::OnFoot | Activity::Approach { .. })
    }

    pub fn tick(&mut self, now: Instant, out: &mut impl Outbox) {
        let dt = self.t.last_tick.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.25);
        self.t.last_tick = Some(now);
        match self.phase {
            Phase::ClassSelection { since } if self.class_info && now.duration_since(since) >= CLASS_SELECTION_MIN => {
                self.phase = Phase::SpawnRequested;
                send_rpc(out, &player::PlayerRequestSpawn);
                println!("[spawn] requested");
            }
            Phase::Dead { since } if now.duration_since(since) >= WASTED && !self.spectating => {
                if self.force_class || !self.class_info {
                    self.enter_class_selection(now, out);
                } else {
                    self.do_spawn(now, out);
                }
            }
            _ => {}
        }
        if self.phase != Phase::Spawned {
            return;
        }
        if self.health <= 0.0 && !self.spectating {
            self.die(now, out);
            return;
        }
        self.inv.update(now);
        if self.spectating {
            self.send_spectator(now, out);
            return;
        }
        self.update_activity(now, dt, out);
        self.send_syncs(now, out);
    }

    fn on_foot(&self) -> bool {
        matches!(self.activity, Activity::OnFoot | Activity::Approach { .. } | Activity::Entering { .. })
    }

    /// (vehicle, seat) while seated (incl. exiting).
    fn seated(&self) -> Option<(u16, u8)> {
        match self.activity {
            Activity::Driving { vehicle, .. } => Some((vehicle, 0)),
            Activity::Riding { vehicle, seat, .. } | Activity::Exiting { vehicle, seat, .. } => Some((vehicle, seat)),
            _ => None,
        }
    }

    fn stand(&mut self) {
        self.vel = Vec3::default();
        self.keys = 0;
        self.lr = 0;
        self.ud = 0;
    }

    /// Run towards `to`; true once within `tol`.
    fn run_to(&mut self, to: Vec3, tol: f32, dt: f32) -> bool {
        let d = sub(v3(to.x, to.y, self.pos.z), self.pos);
        let dist = geom::len(d);
        if dist <= tol {
            self.stand();
            return true;
        }
        let step = (RUN_SPEED * dt).min(dist);
        self.heading = geom::heading_of(d);
        self.pos = add(self.pos, scale(geom::normalize(d), step));
        self.vel = scale(forward(self.heading), RUN_SPEED);
        self.keys = 0;
        self.lr = 0;
        self.ud = -128;
        false
    }

    /// Door position for a seat (driver: front left; 1: front right; 2/3: rear).
    fn door(&self, vehicle: u16, seat: u8) -> Option<Vec3> {
        let v = self.world.vehicles.get(&vehicle)?;
        let side = if seat.is_multiple_of(2) { -1.4 } else { 1.4 };
        let along = if seat >= 2 { -0.9 } else { 0.4 };
        Some(add(add(v.pos, scale(right(v.heading), side)), scale(forward(v.heading), along)))
    }

    fn note_once(&mut self, note: &'static str) {
        if self.waiting_note != Some(note) {
            println!("{note}");
            self.waiting_note = Some(note);
        }
    }

    fn update_activity(&mut self, now: Instant, dt: f32, out: &mut impl Outbox) {
        if !matches!(self.activity, Activity::Entering { .. }) {
            self.special_action = 0;
        }
        match self.activity {
            Activity::OnFoot => self.on_foot_mode(now),
            Activity::Approach { vehicle, passenger } => self.approach(vehicle, passenger, now, dt, out),
            Activity::Entering { vehicle, seat, since, at_door } => {
                self.special_action = SPECIAL_ACTION_ENTER_VEHICLE;
                let Some(door) = self.door(vehicle, seat) else {
                    println!("[car] vehicle {vehicle} vanished while entering");
                    self.activity = Activity::OnFoot;
                    return;
                };
                match at_door {
                    None => {
                        if self.run_to(door, 0.3, dt) {
                            self.activity = Activity::Entering { vehicle, seat, since, at_door: Some(now) };
                        } else if now.duration_since(since) > ENTER_TIMEOUT {
                            println!("[car] enter task timed out");
                            self.activity = Activity::OnFoot;
                        }
                    }
                    Some(t) if now.duration_since(t) >= GET_IN => self.sit_down(vehicle, seat, now),
                    Some(_) => {
                        self.stand();
                        if let Some(v) = self.world.vehicles.get(&vehicle) {
                            self.heading = v.heading;
                        }
                    }
                }
            }
            Activity::Driving { vehicle, mut motion, since } => {
                let exit_due = self.shutting_down || self.cfg.exit_after.is_some_and(|d| now.duration_since(since) >= d);
                if exit_due {
                    motion.braking = true;
                }
                motion.step(dt);
                let pose = motion.pose();
                if let Some(v) = self.world.vehicles.get_mut(&vehicle) {
                    v.pos = pose.pos;
                    v.heading = pose.heading;
                }
                self.pos = pose.pos;
                self.heading = pose.heading;
                self.activity = Activity::Driving { vehicle, motion, since };
                if exit_due && motion.stopped() {
                    self.leave_vehicle(vehicle, 0, now, out);
                }
            }
            Activity::Riding { vehicle, seat, since } => {
                if let Some(v) = self.world.vehicles.get(&vehicle) {
                    self.pos = v.pos;
                    self.heading = v.heading;
                }
                if self.shutting_down || self.cfg.exit_after.is_some_and(|d| now.duration_since(since) >= d) {
                    self.leave_vehicle(vehicle, seat, now, out);
                }
            }
            Activity::Exiting { vehicle, seat, since } => {
                if now.duration_since(since) >= GET_OUT {
                    if let Some(door) = self.door(vehicle, seat) {
                        self.pos = door;
                    }
                    self.centre = self.pos;
                    self.walk_since = now;
                    self.stand();
                    self.activity = Activity::OnFoot;
                    println!("[car] out of vehicle {vehicle} at {:?}", self.pos);
                }
            }
        }
    }

    fn sit_down(&mut self, vehicle: u16, seat: u8, now: Instant) {
        let Some(v) = self.world.vehicles.get(&vehicle).cloned() else {
            self.activity = Activity::OnFoot;
            return;
        };
        self.special_action = 0;
        self.stand();
        self.pos = v.pos;
        self.heading = v.heading;
        if seat == 0 {
            let motion = Motion::new(self.cfg.shape, v.pos, v.heading, self.cfg.speed);
            self.activity = Activity::Driving { vehicle, motion, since: now };
            println!("[car] seated as driver of {vehicle} (model {}), driving", v.model);
        } else {
            self.activity = Activity::Riding { vehicle, seat, since: now };
            println!("[car] seated in {vehicle} (model {}) seat {seat}", v.model);
        }
    }

    /// Player-input exit: RPC 154 (not for RC models), then the leave-car task.
    fn leave_vehicle(&mut self, vehicle: u16, seat: u8, now: Instant, out: &mut impl Outbox) {
        const RC_MODELS: [u32; 6] = [441, 464, 465, 501, 564, 594];
        let model = self.world.vehicles.get(&vehicle).map_or(0, |v| v.model);
        if !RC_MODELS.contains(&model) {
            send_rpc(out, &vehicle::OnPlayerExitVehicle { vehicle_id: vehicle });
            println!("[car] exit request (RPC 154) for vehicle {vehicle}");
        }
        self.activity = Activity::Exiting { vehicle, seat, since: now };
    }

    fn on_foot_mode(&mut self, now: Instant) {
        match self.cfg.mode {
            Mode::Walk => self.walk_circle(now),
            Mode::Idle => self.stand(),
            Mode::Drive | Mode::Passenger => {
                self.stand();
                if self.vehicle_done || self.shutting_down {
                    return;
                }
                let target = self.cfg.vehicle.filter(|v| self.world.vehicles.contains_key(v));
                let target = target.or_else(|| {
                    if self.cfg.vehicle.is_some() { None } else { self.world.nearest_free_vehicle(self.pos, now) }
                });
                match target {
                    Some(vehicle) => {
                        let passenger = self.cfg.mode == Mode::Passenger;
                        println!("[car] going to vehicle {vehicle} as {}", if passenger { "passenger" } else { "driver" });
                        self.activity = Activity::Approach { vehicle, passenger };
                    }
                    None => self.note_once("[car] waiting for a vehicle to stream in"),
                }
            }
            Mode::Shoot => self.shoot_mode(now),
        }
    }

    fn walk_circle(&mut self, now: Instant) {
        let t = now.duration_since(self.walk_since).as_secs_f32();
        let angle = (t * WALK_SPEED) % TAU;
        self.pos = v3(
            self.centre.x + WALK_RADIUS * angle.cos(),
            self.centre.y + WALK_RADIUS * angle.sin(),
            self.centre.z,
        );
        // GTA heading h faces (-sin h, cos h): the tangent of the
        // counter-clockwise circle at `angle`, so the heading equals `angle`.
        self.heading = angle;
        self.vel = scale(forward(angle), WALK_RADIUS * WALK_SPEED);
        self.keys = 0;
        self.lr = 0;
        self.ud = -128;
    }

    fn approach(&mut self, vehicle: u16, passenger: bool, now: Instant, dt: f32, out: &mut impl Outbox) {
        let Some(v) = self.world.vehicles.get(&vehicle).cloned() else {
            println!("[car] vehicle {vehicle} streamed out");
            self.activity = Activity::OnFoot;
            return;
        };
        if !self.run_to(v.pos, ENTER_REQUEST_RANGE, dt) {
            return;
        }
        if passenger {
            // Passenger-key poll: closest vehicle closer than 8.0.
            if geom::dist(self.pos, v.pos) >= PASSENGER_POLL_RANGE {
                return;
            }
        } else if v.locked || v.model == 570 {
            // Driver gate refuses: no RPC, no enter task.
            println!("[car] vehicle {vehicle} is locked (or model 570): enter refused");
            self.vehicle_done = true;
            self.activity = Activity::OnFoot;
            return;
        }
        let seat = if passenger { self.cfg.seat.max(1) } else { 0 };
        send_rpc(out, &vehicle::OnPlayerEnterVehicle { vehicle_id: vehicle, passenger: u8::from(passenger) });
        println!("[car] enter request (RPC 26) vehicle {vehicle} passenger={passenger}");
        self.vehicle_done = true;
        self.activity = Activity::Entering { vehicle, seat, since: now, at_door: None };
    }

    // ---------------------------------------------------------------- shooting

    fn target_id(&self) -> Option<u16> {
        match self.cfg.target.as_ref()? {
            PlayerRef::Id(id) => Some(*id),
            PlayerRef::Name(n) => self.world.player_by_name(n),
        }
    }

    /// (id, position, heading) of a streamed-in target.
    fn target(&self) -> Option<(u16, Vec3, f32)> {
        let id = self.target_id()?;
        let p = self.world.players.get(&id)?;
        if !p.streamed {
            return None;
        }
        Some((id, p.pos?, p.heading))
    }

    /// Choose the weapon to shoot with; None while it is not available.
    fn shoot_weapon(&mut self) -> Option<u8> {
        match self.cfg.weapon {
            Some(w) => {
                if !self.inv.carries(w) {
                    return None;
                }
                if self.inv.armed != w {
                    self.inv.arm(w);
                }
                Some(w)
            }
            None => {
                if weapons::is_hitscan(self.inv.armed) {
                    return Some(self.inv.armed);
                }
                let w = self.inv.slots.iter().map(|s| s.0).find(|&w| weapons::is_hitscan(w) && self.inv.carries(w))?;
                self.inv.arm(w);
                Some(w)
            }
        }
    }

    fn shoot_mode(&mut self, now: Instant) {
        self.stand();
        if self.shutting_down {
            self.aim_since = None;
            return;
        }
        let Some((_, tpos, _)) = self.target() else {
            self.aim_since = None;
            self.note_once("[shoot] waiting for the target to stream in");
            return;
        };
        let Some(weapon) = self.shoot_weapon() else {
            self.aim_since = None;
            self.note_once("[shoot] waiting for the server to give the weapon");
            return;
        };
        let since = *self.aim_since.get_or_insert(now);
        // The ped turns with the aiming camera, sway included (aim_sync): an aiming player's on-foot struct
        // changes with its aim. With a fixed heading the struct stays byte-identical while the swaying aim sync
        // keeps refreshing the shared duplicate-suppression timestamp, so 207 would never be re-sent and the
        // server (and every other client) would keep a stale on-foot state for this bot.
        let t = now.duration_since(since).as_secs_f32();
        self.heading = geom::heading_of(sub(tpos, self.pos)) + AIM_SWAY_YAW * (t * 3.7).sin();
        self.keys = keys::AIM;
        // FIRE only counts while the weapon has ammo.
        if self.inv.ammo(weapon) > 0 && now.duration_since(since) >= AIM_RAISE {
            self.keys |= keys::FIRE;
            if self.next_shot.is_none_or(|t| now >= t) {
                self.pending_shot = true;
                let ms = self.cfg.fire_ms.unwrap_or(weapons::info(weapon).fire_interval_ms);
                self.next_shot = Some(now + Duration::from_millis(ms.into()));
            }
        }
    }

    // ---------------------------------------------------------------- sync senders

    /// Foot / in-car interval: rate + N streamed players (15 ms in LAN mode),
    /// or the fixed `--sync-ms` override.
    fn interval(&self, rate: u32) -> Duration {
        let ms = match self.cfg.sync_ms {
            Some(ms) => ms,
            None if self.rates.lan => policy::LAN_INTERVAL_MS,
            None => rate + self.world.streamed_players(),
        };
        Duration::from_millis(ms.into())
    }

    /// Send a gated sync packet (duplicate suppression); true if sent.
    fn send_gated<S: SyncPacket>(&mut self, s: &S, now: Instant, out: &mut impl Outbox) -> bool {
        let bits = s.to_bits();
        if !self.gate.admit(bits.as_bytes(), now) {
            return false;
        }
        out.send(&bits, policy::send_params(S::PACKET_ID).expect("sync packet has send params"));
        true
    }

    fn send_syncs(&mut self, now: Instant, out: &mut impl Outbox) {
        match self.activity {
            Activity::Driving { vehicle, motion, .. } => self.send_driver(vehicle, &motion, now, out),
            Activity::Exiting { vehicle, seat: 0, .. } => {
                let mut stopped = Motion::new(Shape::Circle { radius: 1.0 }, self.pos, self.heading, 0.0);
                stopped.braking = true;
                self.send_driver(vehicle, &stopped, now, out);
            }
            Activity::Riding { vehicle, seat, .. } | Activity::Exiting { vehicle, seat, .. } => {
                self.send_passenger(vehicle, seat, now, out)
            }
            Activity::OnFoot | Activity::Approach { .. } | Activity::Entering { .. } => {
                if self.pending_shot {
                    self.fire_shot(now, out);
                }
                self.send_foot(now, out);
                self.send_aim_on_foot(now, out);
            }
        }
        self.send_stats_and_weapons(now, out);
    }

    /// 205 every > 1000 ms; 204 evaluated every tick.
    fn send_stats_and_weapons(&mut self, now: Instant, out: &mut impl Outbox) {
        if self.t.stats.is_none_or(|t| now.duration_since(t) > Duration::from_millis(policy::STATS_INTERVAL_MS.into())) {
            crate::send_sync(out, &StatsSync { money: self.money, drunk_level: 0 });
            self.t.stats = Some(now);
        }
        let target = match self.target() {
            Some((id, ..)) if self.cfg.mode == Mode::Shoot && self.keys & keys::AIM != 0 => id,
            _ => NONE_ID,
        };
        if let Some(p) = self.weapons_tx.evaluate(&self.inv, target, NONE_ID, now) {
            crate::send_sync(out, &p);
        }
    }

    /// The `--anim-seq` word for `now` (the sequence starts with the first 207; it holds the last word at the end).
    fn anim_word(&mut self, now: Instant) -> Animation {
        let Some(&last) = self.cfg.anim_seq.last() else { return Animation::default() };
        let t0 = *self.anim_t0.get_or_insert(now);
        let step = self.cfg.anim_step.as_millis().max(1);
        let i = (now.duration_since(t0).as_millis() / step) as usize;
        let v = self.cfg.anim_seq.get(i).copied().unwrap_or(last);
        Animation { index: v as u16, flags: (v >> 16) as u16 }
    }

    /// 207: when keys/lr/ud changed or the on-foot interval elapsed.
    fn send_foot(&mut self, now: Instant, out: &mut impl Outbox) {
        let k = (self.keys, self.lr, self.ud);
        let interval = self.interval(self.rates.on_foot);
        if self.last_foot_keys == Some(k) && self.t.foot.is_some_and(|t| now.duration_since(t) <= interval) {
            return;
        }
        self.t.foot = Some(now);
        self.last_foot_keys = Some(k);
        let sync = FootSync {
            left_right: self.lr,
            up_down: self.ud,
            keys: self.keys,
            position: self.pos,
            rotation: self.foot_rot.filter(geom::heading_to_quat(self.heading)),
            health: self.health.clamp(0.0, 255.0) as u8,
            armour: self.armour.clamp(0.0, 255.0) as u8,
            weapon: WeaponByte::new(self.inv.armed, 0),
            special_action: self.cfg.special_action.unwrap_or(self.special_action),
            velocity: geom::move_speed(self.vel),
            animation: self.anim_word(now),
            ..FootSync::default()
        };
        self.send_gated(&sync, now, out);
    }

    /// 203 on foot: every 100 ms (weapon rate if lag-comp selector 2) with
    /// FIRE or AIM held, else every 1000 ms.
    fn send_aim_on_foot(&mut self, now: Instant, out: &mut impl Outbox) {
        let ms = if self.keys & (keys::FIRE | keys::AIM) != 0 {
            if self.rates.lag_comp == 2 { self.rates.weapon } else { policy::AIM_ACTIVE_INTERVAL_MS }
        } else {
            policy::AIM_IDLE_INTERVAL_MS
        };
        if self.t.aim.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(ms.into())) {
            self.send_aim(now, out);
        }
    }

    fn send_aim(&mut self, now: Instant, out: &mut impl Outbox) {
        self.t.aim = Some(now);
        let aim = self.aim_sync(now);
        self.send_gated(&aim, now, out);
    }

    fn drive_by_active(&self) -> bool {
        self.cfg.drive_by && self.inv.armed != 0 && self.inv.armed != 46
    }

    fn aim_sync(&self, now: Instant) -> AimSync {
        let f = forward(self.heading);
        let (cam_mode, cam_pos, cam_front, aim_z) = match (self.seated(), self.target()) {
            (Some((_, seat)), _) => {
                let mode = if seat > 0 && self.drive_by_active() { 55 } else { 18 };
                (mode, add(add(self.pos, scale(f, -6.0)), v3(0.0, 0.0, 2.0)), geom::normalize(add(f, v3(0.0, 0.0, -0.25))), 0.0)
            }
            (None, Some((_, tpos, _))) if self.keys & keys::AIM != 0 => {
                let r = right(self.heading);
                let cam = add(add(add(self.pos, scale(f, -2.0)), scale(r, 0.5)), v3(0.0, 0.0, 0.7));
                // Small hand/camera sway, as a held mouse never is perfectly still.
                let t = self.aim_since.map_or(0.0, |s| now.duration_since(s).as_secs_f32());
                let sway = add(scale(r, 0.004 * (t * 3.7).sin()), v3(0.0, 0.0, 0.003 * (t * 2.3).sin()));
                let front = geom::normalize(add(geom::normalize(sub(add(tpos, v3(0.0, 0.0, 0.3)), cam)), sway));
                (cam_mode_for(self.inv.armed), cam, front, front.z.clamp(-1.0, 1.0).asin())
            }
            _ => (4, add(add(self.pos, scale(f, -3.5)), v3(0.0, 0.0, 1.0)), geom::normalize(add(f, v3(0.0, 0.0, -0.15))), 0.0),
        };
        AimSync {
            cam_mode,
            cam_front,
            cam_pos,
            aim_z,
            zoom: ZOOM_FOV70,
            weapon_state: self.inv.weapon_state(now),
            aspect: ASPECT_16_9,
        }
    }

    fn driver_sync(&self, vehicle: u16, m: &Motion) -> VehicleSync {
        let pose = m.pose();
        let (model, health) = self.world.vehicles.get(&vehicle).map_or((0, 1000.0), |v| (v.model, v.health));
        let pedal = match m.pedal() {
            Pedal::Accelerate => keys::SPRINT,
            Pedal::Brake => keys::JUMP,
            Pedal::None => 0,
        };
        let steer = m.steer();
        VehicleSync {
            vehicle_id: vehicle,
            left_right: steer,
            up_down: 0,
            keys: pedal | self.cfg.extra_keys,
            rotation: geom::heading_to_quat(pose.heading),
            position: pose.pos,
            velocity: geom::move_speed(m.velocity()),
            vehicle_health: health,
            player_health: self.health.clamp(0.0, 255.0) as u8,
            player_armour: self.armour.clamp(0.0, 255.0) as u8,
            weapon: WeaponByte::new(self.inv.armed, 0),
            siren: u8::from(self.cfg.siren),
            landing_gear: 0,
            trailer_id: self.cfg.trailer.unwrap_or(0),
            extra: extra_field(model, steer, m.v),
        }
    }

    /// 200 when the in-car interval elapsed; 203 after it for armed models,
    /// and from the 1000 ms tick otherwise.
    fn send_driver(&mut self, vehicle: u16, m: &Motion, now: Instant, out: &mut impl Outbox) {
        let interval = self.interval(self.rates.in_car);
        if self.t.car.is_none_or(|t| now.duration_since(t) > interval) {
            self.t.car = Some(now);
            let mut sync = self.driver_sync(vehicle, m);
            sync.rotation = self.car_rot.filter(sync.rotation);
            self.send_gated(&sync, now, out);
            if let Some(t) = self.cfg.trailer {
                // A towed trailer 9 m behind the cab on its heading (measurement input, M6-P).
                let pose = m.pose();
                let back = scale(geom::forward(pose.heading), -9.0);
                let ts = samp_proto::sync::vehicle::TrailerSync {
                    trailer_id: t,
                    position: add(pose.pos, back),
                    rotation: sync.rotation,
                    velocity: sync.velocity,
                    turn_velocity: v3(0.0, 0.0, 0.0),
                };
                self.send_gated(&ts, now, out);
            }
            let model = self.world.vehicles.get(&vehicle).map_or(0, |v| v.model);
            if policy::ARMED_VEHICLE_MODELS.iter().any(|&a| u32::from(a) == model) {
                self.send_aim(now, out);
            }
        }
        if self.t.aim.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(policy::AIM_IDLE_INTERVAL_MS.into())) {
            self.send_aim(now, out);
        }
    }

    /// 211 every in-car interval; 203 every 1000 ms (100 ms with drive-by).
    fn send_passenger(&mut self, vehicle: u16, seat: u8, now: Instant, out: &mut impl Outbox) {
        let interval = self.interval(self.rates.in_car);
        if self.t.car.is_none_or(|t| now.duration_since(t) > interval) {
            self.t.car = Some(now);
            let sync = PassengerSync {
                vehicle_id: vehicle,
                seat,
                drive_by: self.drive_by_active(),
                cuffed: false,
                weapon: WeaponByte::new(self.inv.armed, 0),
                health: self.health.clamp(0.0, 255.0) as u8,
                armour: self.armour.clamp(0.0, 255.0) as u8,
                left_right: 0,
                up_down: 0,
                keys: 0,
                position: self.pos,
            };
            self.send_gated(&sync, now, out);
        }
        let ms = if self.drive_by_active() { policy::AIM_ACTIVE_INTERVAL_MS } else { policy::AIM_IDLE_INTERVAL_MS };
        if self.t.aim.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(ms.into())) {
            self.send_aim(now, out);
        }
    }

    /// 212 every on-foot interval while spectating.
    fn send_spectator(&mut self, now: Instant, out: &mut impl Outbox) {
        let interval = self.interval(self.rates.on_foot);
        if self.t.spectate.is_none_or(|t| now.duration_since(t) > interval) {
            self.t.spectate = Some(now);
            crate::send_sync(out, &SpectatorSync { left_right: 0, up_down: 0, keys: 0, position: self.pos });
        }
    }

    /// Hit offset: entity-local frame (lag-comp selector 0) or hit − entity position.
    fn entity_offset(&self, hit: Vec3, epos: Vec3, eh: f32) -> Vec3 {
        if self.rates.lag_comp == 0 { geom::to_local(hit, epos, eh) } else { sub(hit, epos) }
    }

    /// One shot of the armed weapon at the target: 206 (hitscan), then 115
    /// (giving) for a player hit.
    fn fire_shot(&mut self, now: Instant, out: &mut impl Outbox) {
        self.pending_shot = false;
        let Some((tid, tpos, th)) = self.target() else { return };
        let weapon = self.inv.armed;
        if !self.inv.fire(now) || !weapons::is_hitscan(weapon) {
            return;
        }
        self.shots += 1;
        let (f, r) = (forward(self.heading), right(self.heading));
        let origin = add(add(add(self.pos, scale(f, 0.45)), scale(r, 0.15)), v3(0.0, 0.0, 0.55));
        let torso = v3(0.0, 0.0, 0.3);
        let ht = self.cfg.hit_type;
        let (hit_id, hit_pos, offset) = match ht {
            hit_type::PLAYER => {
                let hit = add(tpos, torso);
                (self.cfg.hit_id.unwrap_or(tid), hit, self.entity_offset(hit, tpos, th))
            }
            hit_type::VEHICLE | hit_type::OBJECT => {
                let id = self.cfg.hit_id.or(self.world.players.get(&tid).and_then(|p| p.vehicle)).unwrap_or(NONE_ID);
                match self.world.vehicles.get(&id).filter(|_| ht == hit_type::VEHICLE) {
                    Some(v) => {
                        let hit = add(v.pos, torso);
                        (id, hit, self.entity_offset(hit, v.pos, v.heading))
                    }
                    None => (id, add(tpos, torso), torso),
                }
            }
            // Nothing hit: offset carries world coordinates.
            _ => {
                let hit = add(tpos, torso);
                (NONE_ID, hit, hit)
            }
        };
        let offset = self.cfg.offset.unwrap_or(offset);
        if weapon == 34 && ht == hit_type::NONE {
            return; // sniper misses are not sent
        }
        crate::send_sync(out, &BulletSync { hit_type: ht, hit_id, origin, hit_pos, offset, weapon_id: weapon });
        if ht == hit_type::PLAYER {
            let damage = weapons::reported_damage(weapon);
            send_rpc(
                out,
                &player::OnPlayerGiveTakeDamage {
                    taking: false,
                    player_id: hit_id,
                    damage,
                    weapon_id: weapon.into(),
                    bodypart: BODYPART_TORSO,
                },
            );
        }
        if self.shots <= 3 || self.shots.is_multiple_of(10) {
            println!(
                "[shoot] shot {} weapon {weapon} hit type {ht} id {hit_id} offset ({:.2}, {:.2}, {:.2}), ammo {}",
                self.shots,
                offset.x,
                offset.y,
                offset.z,
                self.inv.ammo(weapon)
            );
        }
    }
}

/// Aim camera mode for the armed weapon (53 = third-person weapon aim).
fn cam_mode_for(weapon: u8) -> u8 {
    match weapon {
        34 => 7,
        35 => 8,
        36 => 51,
        43 => 46,
        _ => 53,
    }
}

/// Bikes (incl. BMX/bicycles).
fn is_bike(model: u32) -> bool {
    matches!(model, 448 | 461 | 462 | 463 | 468 | 471 | 481 | 509 | 510 | 521 | 522 | 523 | 581 | 586)
}

/// Model-dependent 200 tail: trains/tram f32 speed, bikes f32 lean, else 0.
fn extra_field(model: u32, steer: i16, speed: f32) -> u32 {
    match model {
        537 | 538 | 449 => (speed / 50.0).to_bits(),
        m if is_bike(m) => (f32::from(steer) / 128.0 * 0.5).to_bits(),
        _ => 0.0f32.to_bits(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samp_proto::bitstream::{BitReader, BitWriter};
    use samp_proto::raknet::{Reliability, id};
    use samp_proto::rpcs::{RpcPayload, SendParams};
    use samp_proto::sync::player::RemoteBulletSync;
    use samp_proto::sync::{ids, player::WeaponsUpdate};

    /// Records everything the bot sends, with the time of the tick.
    #[derive(Default)]
    struct Rec {
        now: Option<Instant>,
        msgs: Vec<(Instant, Vec<u8>, usize, SendParams)>,
    }

    impl Outbox for Rec {
        fn send(&mut self, bits: &BitWriter, params: SendParams) {
            self.msgs.push((self.now.expect("time set"), bits.as_bytes().to_vec(), bits.bit_len(), params));
        }
    }

    impl Rec {
        /// (time, bytes after the id, bit length) of sync packets with `pkt`.
        fn syncs(&self, pkt: u8) -> Vec<(Instant, Vec<u8>, SendParams)> {
            self.msgs.iter().filter(|m| m.1[0] == pkt).map(|m| (m.0, m.1[1..].to_vec(), m.3)).collect()
        }
        /// (time, decoded payload, params) of RPCs with `P::ID`.
        fn rpcs<P: RpcPayload>(&self) -> Vec<(Instant, P, SendParams)> {
            self.msgs
                .iter()
                .filter(|m| m.1[0] == id::RPC)
                .filter_map(|m| {
                    let rpc = samp_proto::rpc::decode(&m.1, m.2)?;
                    (rpc.id == P::ID).then(|| (m.0, P::read(&mut rpc.reader()).expect("decodes"), m.3))
                })
                .collect()
        }
    }

    fn spawned(cfg: Config, t0: Instant) -> Bot {
        let mut b = Bot::new(cfg, t0);
        b.phase = Phase::Spawned;
        b.class_info = true;
        b.player_id = 0;
        b
    }

    fn stream_vehicle(b: &mut Bot, id: u16, pos: Vec3, model: u32) {
        b.world.apply_rpc(&Incoming::Vehicle(vehicle::Incoming::StreamInVehicle(vehicle::StreamInVehicle {
            vehicle_id: id,
            model,
            position: pos,
            health: 1000.0,
            ..vehicle::StreamInVehicle::default()
        })));
    }

    fn run(b: &mut Bot, rec: &mut Rec, t0: Instant, from_ms: u64, to_ms: u64) {
        for ms in (from_ms..to_ms).step_by(5) {
            let t = t0 + Duration::from_millis(ms);
            rec.now = Some(t);
            b.tick(t, rec);
        }
    }

    fn ms(t0: Instant, t: Instant) -> u64 {
        t.duration_since(t0).as_millis() as u64
    }

    #[test]
    fn drive_enter_order_rates_and_exit() {
        let t0 = Instant::now();
        let mut b = spawned(Config { mode: Mode::Drive, ..Config::default() }, t0);
        stream_vehicle(&mut b, 5, v3(10.0, 0.0, 3.0), 560);
        let mut rec = Rec::default();
        run(&mut b, &mut rec, t0, 0, 8000);

        let enters = rec.rpcs::<vehicle::OnPlayerEnterVehicle>();
        assert_eq!(enters.len(), 1);
        let (t26, enter, p) = enters[0];
        assert_eq!((enter.vehicle_id, enter.passenger), (5, 0));
        assert_eq!((p.reliability, p.channel), (Reliability::ReliableSequenced, 0));

        let drv = rec.syncs(ids::VEHICLE_SYNC);
        assert!(!drv.is_empty());
        let first = drv[0].0;
        assert!(ms(t26, first) >= GET_IN.as_millis() as u64, "200 before the get-in animation ended");
        assert!(drv.iter().all(|d| d.2 == policy::SEQUENCED_SYNC));
        // Foot sync between the request and the seat carries special action 3.
        for (t, body, _) in rec.syncs(ids::FOOT_SYNC) {
            if t > t26 && t < first {
                let f = FootSync::read_body(&mut BitReader::new(&body)).unwrap();
                assert_eq!(f.special_action, SPECIAL_ACTION_ENTER_VEHICLE);
            }
        }
        // No 207 once seated; 200 at the in-car interval (> 30 ms).
        assert!(rec.syncs(ids::FOOT_SYNC).iter().all(|f| f.0 < first));
        for w in drv.windows(2) {
            assert!(w[1].0.duration_since(w[0].0) > Duration::from_millis(30));
        }
        let v: Vec<VehicleSync> = drv.iter().map(|d| VehicleSync::read_body(&mut BitReader::new(&d.1)).unwrap()).collect();
        assert!(v.iter().all(|s| s.vehicle_id == 5 && s.vehicle_health == 1000.0 && s.player_health == 100));
        assert!(geom::dist(v[0].position, v.last().unwrap().position) > 5.0);
        assert!(v.iter().skip(5).all(|s| s.keys & keys::SPRINT != 0 && s.left_right < 0));
        // Aim sync while driving: about once per second, camera mode 18.
        let aims = rec.syncs(ids::AIM_SYNC);
        let seated_aims: Vec<_> = aims.iter().filter(|a| a.0 > first).collect();
        assert!((3..=6).contains(&seated_aims.len()), "{}", seated_aims.len());
        assert!(seated_aims.iter().all(|a| a.1[0] == 18));

        // Shutdown: brake, RPC 154, still 200 while getting out, then 207.
        b.request_shutdown();
        assert!(!b.ready_to_quit());
        let n = rec.msgs.len();
        run(&mut b, &mut rec, t0, 8000, 12000);
        let exits = rec.rpcs::<vehicle::OnPlayerExitVehicle>();
        assert_eq!(exits.len(), 1);
        let t154 = exits[0].0;
        let after: Vec<_> = rec.msgs[n..].iter().filter(|m| m.0 < t154 && m.1[0] == ids::VEHICLE_SYNC).collect();
        let braking = VehicleSync::read_body(&mut BitReader::new(&after[0].1[1..])).unwrap();
        assert_eq!(braking.keys & keys::JUMP, keys::JUMP);
        assert!(rec.syncs(ids::VEHICLE_SYNC).iter().any(|d| d.0 > t154));
        assert!(rec.syncs(ids::FOOT_SYNC).iter().any(|d| ms(t154, d.0) >= GET_OUT.as_millis() as u64));
        assert!(b.ready_to_quit());
    }

    #[test]
    fn passenger_enters_and_sends_211() {
        let t0 = Instant::now();
        let mut b = spawned(Config { mode: Mode::Passenger, seat: 2, ..Config::default() }, t0);
        stream_vehicle(&mut b, 9, v3(-6.0, 0.0, 3.0), 560);
        let mut rec = Rec::default();
        run(&mut b, &mut rec, t0, 0, 6000);
        let enters = rec.rpcs::<vehicle::OnPlayerEnterVehicle>();
        assert_eq!(enters.len(), 1);
        assert_eq!((enters[0].1.vehicle_id, enters[0].1.passenger), (9, 1));
        let p = rec.syncs(ids::PASSENGER_SYNC);
        assert!(p.len() >= 3);
        let s = PassengerSync::read_body(&mut BitReader::new(&p[0].1)).unwrap();
        assert_eq!((s.vehicle_id, s.seat, s.drive_by), (9, 2, false));
        assert!(rec.syncs(ids::VEHICLE_SYNC).is_empty());
        // Identical 211 structs: duplicate suppression spaces them ≥ 501 ms.
        for w in p.windows(2) {
            assert!(w[1].0.duration_since(w[0].0) >= Duration::from_millis(501));
        }
    }

    #[test]
    fn shoot_cycle() {
        let t0 = Instant::now();
        let cfg = Config { mode: Mode::Shoot, target: Some(PlayerRef::Id(7)), weapon: Some(24), ..Config::default() };
        let mut b = spawned(cfg, t0);
        b.inv.give(24, 50);
        b.world.apply_rpc(&Incoming::Remote(remote::Incoming::PlayerStreamIn(remote::PlayerStreamIn {
            player_id: 7,
            position: v3(0.0, 10.0, 3.0),
            ..remote::PlayerStreamIn::default()
        })));
        let mut rec = Rec::default();
        run(&mut b, &mut rec, t0, 0, 3000);

        let shots = rec.syncs(ids::BULLET_SYNC);
        assert_eq!(shots.len(), 4, "Deagle: first shot after 300 ms, then every 800 ms");
        let dmg = rec.rpcs::<player::OnPlayerGiveTakeDamage>();
        assert_eq!(dmg.len(), 4);
        for ((ts, body, p), (td, d, _)) in shots.iter().zip(&dmg) {
            assert_eq!(*p, policy::BULLET);
            assert!(td >= ts);
            let s = BulletSync::read_body(&mut BitReader::new(body)).unwrap();
            assert_eq!((s.hit_type, s.hit_id, s.weapon_id), (1, 7, 24));
            assert!(geom::dist(s.offset, v3(0.0, 0.0, 0.3)) < 1e-3, "{:?}", s.offset);
            assert!(geom::dist(s.hit_pos, v3(0.0, 10.3, 3.0)) > 0.0 && (s.hit_pos.y - 10.0).abs() < 1e-3);
            assert!(!d.taking && d.player_id == 7 && (d.damage - 46.2).abs() < 1e-3 && d.weapon_id == 24);
        }
        // Aim sync every 100 ms while aiming, weapon camera mode 53.
        let aims = rec.syncs(ids::AIM_SYNC);
        assert!(aims.len() >= 25, "{}", aims.len());
        for w in aims.windows(2) {
            assert!(w[1].0.duration_since(w[0].0) >= Duration::from_millis(100));
        }
        assert!(aims.iter().skip(1).all(|a| a.1[0] == 53));
        // 204: first the given Deagle with the target, later the ammo drop.
        let wu: Vec<WeaponsUpdate> = rec
            .syncs(ids::WEAPONS_UPDATE)
            .iter()
            .map(|w| {
                let mut r = BitReader::new(&w.1);
                <WeaponsUpdate as SyncPacket>::read(&mut r).unwrap()
            })
            .collect();
        assert_eq!(wu[0].slots[0].weapon_id, 24);
        assert_eq!(wu[0].target_player, 7);
        assert_eq!(wu.last().unwrap().slots[0].ammo, 46);
        // Foot sync holds AIM, and FIRE once the weapon is raised.
        let feet: Vec<FootSync> =
            rec.syncs(ids::FOOT_SYNC).iter().map(|f| FootSync::read_body(&mut BitReader::new(&f.1)).unwrap()).collect();
        assert!(feet.iter().all(|f| f.keys & keys::AIM != 0 && f.weapon.weapon() == 24));
        assert!(feet.iter().any(|f| f.keys & keys::FIRE != 0));
        // Standing still while aiming at a still target, the on-foot struct still changes with the aim sway,
        // so 207 keeps going out (soak: a fixed heading starved it behind the always-changing 203).
        let late =
            rec.syncs(ids::FOOT_SYNC).iter().filter(|f| f.0.duration_since(t0) >= Duration::from_millis(1000)).count();
        assert!(late >= 20, "on-foot syncs after the first second: {late}");
    }

    #[test]
    fn idle_duplicate_suppression_and_stats() {
        let t0 = Instant::now();
        let mut b = spawned(Config { mode: Mode::Idle, ..Config::default() }, t0);
        let mut rec = Rec::default();
        run(&mut b, &mut rec, t0, 0, 4000);
        let feet = rec.syncs(ids::FOOT_SYNC);
        assert!(feet.len() >= 4);
        for w in feet.windows(2) {
            assert!(w[1].0.duration_since(w[0].0) >= Duration::from_millis(501));
        }
        let stats = rec.syncs(ids::STATS_SYNC);
        assert_eq!(stats.len(), 4);
        assert!(stats.iter().all(|s| s.2 == policy::WEAPONS_STATS));
        // Identical idle aim structs fall under the shared 501 ms window of
        // the foot sends: only the first one goes out.
        assert_eq!(rec.syncs(ids::AIM_SYNC).len(), 1);
    }

    #[test]
    fn victim_reports_damage_and_dies() {
        let t0 = Instant::now();
        let mut b = spawned(Config { mode: Mode::Idle, ..Config::default() }, t0);
        b.spawn = Some(player::SpawnInfo::default());
        let mut rec = Rec { now: Some(t0), ..Rec::default() };
        let hit = IncomingSync::Bullet(RemoteBulletSync {
            player_id: 3,
            sync: BulletSync { hit_type: 1, hit_id: 0, weapon_id: 24, ..BulletSync::default() },
        });
        for _ in 0..3 {
            b.handle_event(Event::Sync(hit.clone()), t0, &mut rec).unwrap();
        }
        let d = rec.rpcs::<player::OnPlayerGiveTakeDamage>();
        assert_eq!(d.len(), 3);
        assert!(d[0].1.taking && d[0].1.player_id == 3 && d[0].1.weapon_id == 24);
        b.tick(t0, &mut rec);
        let deaths = rec.rpcs::<player::OnPlayerDeath>();
        assert_eq!((deaths[0].1.reason, deaths[0].1.killer_id), (24, 3));
        assert!(matches!(b.phase, Phase::Dead { .. }));
        // Respawn after the wasted sequence (RPC 52).
        rec.now = Some(t0 + WASTED);
        b.tick(t0 + WASTED, &mut rec);
        assert_eq!(rec.rpcs::<player::PlayerSpawn>().len(), 1);
        assert_eq!(b.phase, Phase::Spawned);
    }
}
