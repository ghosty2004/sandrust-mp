//! Vehicles: the platform-independent part (milestone M5).
//!
//! Specs: vehicles.md (RPC layouts and client reactions), remote-control-vehicles.md (§1 lifecycle, §2 remote
//! drivers, §3 passengers / unoccupied, §5 enter/exit), remote-players-sync.md (§0.4 send rates, §200/§209/§211,
//! §2.1 apply rules) and the measured behaviour of the original client in docs/phase3/trace-notes.md "Vehicles
//! (TRACE-4)".
//!
//! Everything here is pure data and arithmetic so it can be unit-tested on the host; the GTA side (script
//! commands, hooks, memory writes) lives in `vehicles.rs` / `vehhooks.rs` (Windows only).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use samp_proto::sync::encoding::WeaponByte;
use samp_proto::sync::vehicle::{PassengerSync, RemoteVehicleSync, UnoccupiedSync, VehicleSync};
use samp_proto::types::{Quat, Vec3};

use crate::footsync::{JITTER_EPSILON, Rotation, quat_from_matrix};
use crate::remote::normalise;

/// Pool size: ids 0..1999 (id 0 is never found by the reverse lookup).
pub const MAX_VEHICLES: u16 = 2000;
/// Models accepted by RPC 164.
pub const MODEL_MIN: u32 = 400;
pub const MODEL_MAX: u32 = 611;
/// Trains and trams (vehicles.md "Model sets").
pub const RAIL_MODELS: [u32; 5] = [537, 538, 569, 570, 449];
/// Models whose driver sync goes through the train path (remote-control-vehicles §2.2 step 2).
pub const TRAIN_SYNC_MODELS: [u32; 3] = [537, 538, 449];
/// RC models (no RPC 154 on exit; the remote "driver" is taken out of the world).
pub const RC_MODELS: [u32; 6] = [441, 464, 465, 501, 564, 594];
/// Hydra: `extra` carries the thrust nozzle angle.
pub const MODEL_HYDRA: u32 = 520;
/// Freight carriage: cannot be entered as driver.
pub const MODEL_FREIGHT_CARRIAGE: u32 = 570;
/// Parachute weapon id (dropped before entering a car).
pub const WEAPON_PARACHUTE: u8 = 46;

/// Driver correction (A 0x10015140, remote-players-sync §2.1).
pub const DRIVER_DEAD_ZONE: f32 = 0.05;
pub const DRIVER_GAIN: f32 = 0.06;
pub const DRIVER_TELEPORT_XY: f32 = 8.0;
pub const DRIVER_TELEPORT_Z: f32 = 0.5;
pub const DRIVER_TELEPORT_Z_AIR_WATER: f32 = 2.0;
/// A nudged move speed is only written when one component exceeds this.
pub const DRIVER_MIN_SPEED: f32 = 0.01;
/// Turn speed clamp per axis (A 0x100FE4B8 switch = 1).
pub const TURN_CLAMP: f32 = 0.02;
/// Unoccupied apply (A 0x100158D0).
pub const UNOCC_DEAD_ZONE: f32 = 0.1;
pub const UNOCC_TELEPORT_XY: f32 = 6.0;
pub const UNOCC_TELEPORT_Z: f32 = 3.0;
pub const UNOCC_AXIS_ZONE: f32 = 0.05;
pub const UNOCC_GAIN: f32 = 0.06;
/// Unoccupied send rules (remote-control-vehicles §3.2).
pub const UNOCC_SEND_RANGE: f32 = 90.0;
/// Per-vehicle 209 interval ("> 100 ms"). Checked only on pool-tick frames (every 4th frame), which gives the measured
/// ≈ 150 ms spacing of the original (TRACE-4: 150 ms; TRACE-5: ≈ 153 ms at 49 fps).
pub const UNOCC_SEND_INTERVAL: Duration = Duration::from_millis(100);
/// The vehicle pool tick runs on every 4th frame (measured, TRACE-4 trace orig-20261104-204143).
pub const POOL_TICK_FRAMES: u64 = 4;
pub const UNOCC_SENDS_PER_TICK: u32 = 3;
/// Distance gates of the pool tick and the hooks (2-D for the ProcessControl hook).
pub const NEAR_RANGE: f32 = 200.0;
/// The relays 26/154 are dropped beyond this distance.
pub const RELAY_RANGE: f32 = 200.0;
/// Wasted vehicles are deleted locally after this.
pub const WASTED_DELETE: Duration = Duration::from_millis(15001);
/// Passenger-enter poll: closest car must be nearer than this.
pub const PASSENGER_ENTER_RANGE: f32 = 8.0;
/// Enter tasks started by the relays / the passenger poll.
pub const ENTER_TASK_MS: i32 = 3000;
/// z offset written after CREATE_CAR (not for bikes / BMX).
pub const CREATE_Z_OFFSET: f32 = 0.25;
/// Stream-out / destructor: occupants are warped to the car position + this on z.
pub const EJECT_Z_OFFSET: f32 = 2.0;
/// Train exit: the local ped is placed beside the train (+2.5 on x and y).
pub const TRAIN_EXIT_OFFSET: f32 = 2.5;
/// Radar blip colours (samp's own codes, vehicles.md RE-10 "radar blips").
pub const BLIP_COLOUR_UNOCCUPIED: i32 = 0x3EC;
pub const BLIP_COLOUR_OBJECTIVE: i32 = 0x3EE;

/// samp's vehicle kind classifier (A 0x100B6220, by vtable). Many setters only act on [`Kind::Automobile`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Other = 0,
    Automobile = 1,
    Bike = 2,
    Heli = 3,
    Boat = 4,
    Plane = 5,
    Bmx = 6,
    Train = 7,
}

/// GTA vehicle vtables (named via gta-reversed / plugin-sdk).
pub mod vtbl {
    pub const AUTOMOBILE: u32 = 0x871120;
    pub const BIKE: u32 = 0x871360;
    pub const BMX: u32 = 0x871528;
    pub const HELI: u32 = 0x871680;
    pub const MONSTER_TRUCK: u32 = 0x8717D8;
    pub const PLANE: u32 = 0x871948;
    pub const QUAD_BIKE: u32 = 0x871AE8;
    pub const TRAILER: u32 = 0x871C28;
    pub const BOAT: u32 = 0x8721A0;
    pub const TRAIN: u32 = 0x872370;
}

pub fn kind_from_vtable(vt: u32) -> Kind {
    match vt {
        vtbl::AUTOMOBILE => Kind::Automobile,
        vtbl::BIKE => Kind::Bike,
        vtbl::HELI => Kind::Heli,
        vtbl::BOAT => Kind::Boat,
        vtbl::PLANE => Kind::Plane,
        vtbl::BMX => Kind::Bmx,
        vtbl::TRAIN => Kind::Train,
        _ => Kind::Other,
    }
}

pub fn is_rail(model: u32) -> bool {
    RAIL_MODELS.contains(&model)
}

pub fn is_rc(model: u32) -> bool {
    RC_MODELS.contains(&model)
}

/// Nametag anchor (A 0x10074270 / 0x10075390 with A 0x100b7e40): only a remote driving an RC
/// vehicle is tagged at the vehicle's matrix position; every other player, drivers of ordinary
/// vehicles included, at the ped's bone 8 (head). Measured on the original: a driver of an
/// Infernus is tagged at its head, 0.3 m above the car's matrix position.
pub fn tag_at_vehicle(remote_state: u8, vehicle_model: Option<u32>) -> bool {
    remote_state == state::DRIVER && vehicle_model.is_some_and(is_rc)
}

/// Towable trailer models (A 0x100B7DB0 list; remote-players-sync §210).
pub fn is_towable_list(model: u32) -> bool {
    [435, 450, 584, 591, 606, 607, 608, 610, 611].contains(&model)
}

/// Locomotive → carriage model created by GTA with the train (ids id+1..id+3).
pub fn carriage_of(model: u32) -> Option<u32> {
    match model {
        537 => Some(569),
        538 => Some(570),
        _ => None,
    }
}

/// CREATE_MISSION_TRAIN type for a locomotive / tram model (remote-control-vehicles §1.2.5).
pub fn train_type(model: u32) -> Option<i32> {
    match model {
        538 => Some(5),
        537 => Some(3),
        449 => Some(9),
        _ => None,
    }
}

pub fn dist2d(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub fn dist3d(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn vec3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

pub fn to_vec3(v: [f32; 3]) -> Vec3 {
    Vec3 { x: v[0], y: v[1], z: v[2] }
}

// ---- remote driver / passenger data ------------------------------------------------------------------

/// Remote state byte values used by vehicles (remote-players-sync §2.1).
pub mod state {
    pub const PASSENGER: u8 = 18;
    pub const DRIVER: u8 = 19;
}

/// The decoded fields of the last accepted 200 (remote-players-sync §200 S→C).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriverData {
    pub vehicle_id: u16,
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    pub rotation: Quat,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub vehicle_health: f32,
    pub health: f32,
    pub armour: f32,
    pub weapon: u8,
    pub siren: bool,
    pub landing_gear: bool,
    /// 0 when absent.
    pub extra: u32,
    /// 0 when absent.
    pub trailer_id: u16,
}

impl DriverData {
    pub fn from_sync(s: &RemoteVehicleSync) -> Self {
        let v = s.velocity.to_vec3();
        Self {
            vehicle_id: s.vehicle_id,
            left_right: s.left_right,
            up_down: s.up_down,
            keys: s.keys,
            rotation: normalise(s.rotation.to_quat()),
            position: vec3(s.position),
            velocity: vec3(v),
            vehicle_health: f32::from(s.vehicle_health),
            health: s.health_armour.health(),
            armour: s.health_armour.armour(),
            weapon: s.weapon.weapon(),
            siren: s.siren,
            landing_gear: s.landing_gear,
            extra: s.extra.unwrap_or(0),
            trailer_id: s.trailer_id.unwrap_or(0),
        }
    }

    /// `extra` as the f32 the train / bike paths read.
    pub fn extra_f32(&self) -> f32 {
        f32::from_bits(self.extra)
    }
}

/// The decoded fields of the last 211 (remote-players-sync §211).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassengerData {
    pub vehicle_id: u16,
    /// 1… = passenger seat (seat − 1 for WARP_CHAR_INTO_CAR_AS_PASSENGER).
    pub seat: u8,
    pub drive_by: bool,
    pub cuffed: bool,
    pub weapon: u8,
    pub health: f32,
    pub armour: f32,
    pub keys: u16,
    pub position: [f32; 3],
}

impl PassengerData {
    pub fn from_sync(s: &PassengerSync) -> Self {
        Self {
            vehicle_id: s.vehicle_id,
            seat: s.seat & 0x3F,
            drive_by: s.drive_by,
            cuffed: s.cuffed,
            weapon: s.weapon.weapon(),
            health: f32::from(s.health),
            armour: f32::from(s.armour),
            keys: s.keys,
            position: vec3(s.position),
        }
    }
}

/// Per remote player: what the vehicle stores and the per-frame apply need.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteVeh {
    pub driver: Option<DriverData>,
    pub passenger: Option<PassengerData>,
    /// A 200 was stored and not yet applied (update type 17).
    pub driver_new: bool,
    pub passenger_new: bool,
    /// Timestamp of the last accepted 200 (shared rule with 207 in samp: one field per player).
    pub corrections: u32,
    pub teleports: u32,
    pub warps_in: u32,
    pub warps_out: u32,
}

// ---- corrections ---------------------------------------------------------------------------------------

/// Result of a per-packet vehicle correction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VehCorrection {
    /// Nothing written.
    None,
    /// Write this move speed.
    Speed([f32; 3]),
    /// Write the matrix translation (and the synced velocity).
    Teleport,
}

/// Driver sync correction (A 0x10015140), applied right after `move speed := synced velocity`.
/// `synced` is that velocity; `z_threshold` is 2.0 for heli/plane/boat and 0.5 otherwise.
pub fn correct_driver(
    current: [f32; 3],
    target: [f32; 3],
    synced: [f32; 3],
    moving: bool,
    z_threshold: f32,
) -> VehCorrection {
    if !moving {
        return VehCorrection::Teleport;
    }
    let d = [0, 1, 2].map(|i| (target[i] - current[i]).abs());
    if d.iter().all(|&v| v <= DRIVER_DEAD_ZONE) {
        return VehCorrection::None;
    }
    if d[0] <= DRIVER_TELEPORT_XY && d[1] <= DRIVER_TELEPORT_XY && d[2] <= z_threshold {
        let mut s = synced;
        for i in 0..3 {
            if d[i] > DRIVER_DEAD_ZONE {
                s[i] += (target[i] - current[i]) * DRIVER_GAIN;
            }
        }
        if s.iter().any(|c| c.abs() > DRIVER_MIN_SPEED) { VehCorrection::Speed(s) } else { VehCorrection::None }
    } else {
        VehCorrection::Teleport
    }
}

pub fn z_threshold(kind: Kind) -> f32 {
    match kind {
        Kind::Heli | Kind::Plane | Kind::Boat => DRIVER_TELEPORT_Z_AIR_WATER,
        _ => DRIVER_TELEPORT_Z,
    }
}

/// Turn speed clamped to ±0.02 per axis (remote-control-vehicles §2.2, every frame).
pub fn clamp_turn(t: [f32; 3]) -> [f32; 3] {
    t.map(|c| c.clamp(-TURN_CLAMP, TURN_CLAMP))
}

/// Train path of the driver apply (A 0x10015650): teleport beyond 10 units on x or y.
pub fn train_needs_teleport(current: [f32; 3], target: [f32; 3]) -> bool {
    (target[0] - current[0]).abs() > 10.0 || (target[1] - current[1]).abs() > 10.0
}

/// 209 receive validation (remote-players-sync §209 S→C).
pub fn unoccupied_valid(s: &UnoccupiedSync) -> bool {
    let unit = |v: Vec3| [v.x, v.y, v.z].iter().all(|c| (-1.0..=1.0).contains(c));
    let small = |v: Vec3| [v.x, v.y, v.z].iter().all(|c| *c > -100.0 && *c < 100.0);
    (1..MAX_VEHICLES).contains(&s.vehicle_id)
        && unit(s.roll)
        && unit(s.direction)
        && s.position.x > -20000.0
        && s.position.x < 20000.0
        && s.position.y > -20000.0
        && s.position.y < 20000.0
        && s.position.z > -10000.0
        && s.position.z < 100000.0
        && small(s.velocity)
        && small(s.turn_velocity)
}

/// 209 apply (A 0x100158D0): `None`, `Speed(v)` = keep the position, set rotation, turn speed and this velocity,
/// or `Teleport` (position, rotation, velocity and turn speed from the packet).
pub fn correct_unoccupied(current: [f32; 3], target: [f32; 3], synced: [f32; 3]) -> VehCorrection {
    let d = [0, 1, 2].map(|i| (target[i] - current[i]).abs());
    if d.iter().all(|&v| v <= UNOCC_DEAD_ZONE) {
        return VehCorrection::None;
    }
    if d[0] <= UNOCC_TELEPORT_XY && d[1] <= UNOCC_TELEPORT_XY && d[2] <= UNOCC_TELEPORT_Z {
        let mut s = synced;
        for i in 0..3 {
            if d[i] > UNOCC_AXIS_ZONE {
                s[i] += (target[i] - current[i]) * UNOCC_GAIN;
            }
        }
        VehCorrection::Speed(s)
    } else {
        VehCorrection::Teleport
    }
}

/// Matrix rows from the 209 roll (right) and direction (forward) vectors; up = right × forward.
pub fn rows_from_roll_direction(right: [f32; 3], forward: [f32; 3]) -> Rotation {
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    Rotation { right, forward, up }
}

// ---- local sending ------------------------------------------------------------------------------------

/// Duplicate suppression shared by the in-car packets (remote-players-sync §0.4): one timestamp for all types,
/// one "last struct" copy per type. A struct identical to the last one of its type is not sent while fewer
/// than 501 ms have passed since the last send of any of them.
#[derive(Debug, Default)]
pub struct DupGate {
    last_send: Option<Instant>,
    last: HashMap<u8, Vec<u8>>,
}

impl DupGate {
    pub fn offer(&mut self, now: Instant, packet_id: u8, bytes: Vec<u8>) -> bool {
        let recent = self.last_send.is_some_and(|t| now.duration_since(t) < crate::footsync::DUPLICATE_WINDOW);
        if recent && self.last.get(&packet_id) == Some(&bytes) {
            return false;
        }
        self.last_send = Some(now);
        self.last.insert(packet_id, bytes);
        true
    }

    /// A send that bypasses the comparison but still refreshes the shared timestamp (210 while towing).
    pub fn touch(&mut self, now: Instant) {
        self.last_send = Some(now);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// In-car interval: `inCarRate + N` ms, N = streamed-in remote player peds (§0.4).
pub fn in_car_interval(in_car_rate_ms: u32, streamed_peds: u32) -> Duration {
    Duration::from_millis(u64::from(in_car_rate_ms) + u64::from(streamed_peds))
}

/// Interval timer of one packet type ("elapsed > interval" since the last attempt).
#[derive(Debug, Default)]
pub struct IntervalTimer {
    last: Option<Instant>,
}

impl IntervalTimer {
    pub fn due(&mut self, now: Instant, interval: Duration) -> bool {
        if self.last.is_none_or(|t| now.duration_since(t) > interval) {
            self.last = Some(now);
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Everything read from the game for one 200 packet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriverSample {
    pub vehicle_id: u16,
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    pub additional_key: u8,
    pub rotation: Rotation,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub vehicle_health: f32,
    pub health: f32,
    pub armour: f32,
    pub weapon: u8,
    pub siren: bool,
    pub landing_gear: bool,
    pub trailer_id: u16,
    pub extra: u32,
}

fn trunc_u8(v: f32) -> u8 {
    (v as i32) as u8
}

/// Quaternion of the vehicle matrix with the 1e-5 jitter filter against the last sent one.
pub fn filtered_quat(r: &Rotation, last: Option<Quat>) -> Quat {
    let q = quat_from_matrix(r);
    if let Some(old) = last {
        let close = |a: f32, b: f32| (a - b).abs() < JITTER_EPSILON;
        if close(q.w, old.w) && close(q.x, old.x) && close(q.y, old.y) && close(q.z, old.z) {
            return old;
        }
    }
    q
}

/// Builds the 200 struct (remote-players-sync §200 C→S).
pub fn build_driver_sync(s: &DriverSample, last_rotation: Option<Quat>) -> VehicleSync {
    VehicleSync {
        vehicle_id: s.vehicle_id,
        left_right: s.left_right,
        up_down: s.up_down,
        keys: s.keys,
        rotation: filtered_quat(&s.rotation, last_rotation),
        position: to_vec3(s.position),
        velocity: to_vec3(s.velocity),
        vehicle_health: s.vehicle_health,
        player_health: trunc_u8(s.health),
        player_armour: trunc_u8(s.armour),
        weapon: WeaponByte::new(s.weapon, s.additional_key),
        siren: u8::from(s.siren),
        landing_gear: u8::from(s.landing_gear),
        trailer_id: s.trailer_id,
        extra: s.extra,
    }
}

/// Builds the 211 struct (remote-players-sync §211 C→S). Drive-by and cuffs are not tracked locally yet.
#[allow(clippy::too_many_arguments)]
pub fn build_passenger_sync(
    vehicle_id: u16,
    seat: u8,
    weapon: u8,
    additional_key: u8,
    health: f32,
    armour: f32,
    left_right: i16,
    up_down: i16,
    keys: u16,
    position: [f32; 3],
) -> PassengerSync {
    PassengerSync {
        vehicle_id,
        seat: seat & 0x3F,
        drive_by: false,
        cuffed: false,
        weapon: WeaponByte::new(weapon, additional_key),
        health: trunc_u8(health),
        armour: trunc_u8(armour),
        left_right,
        up_down,
        keys,
        position: to_vec3(position),
    }
}

/// Who sends 209 for one vehicle (A 0x10006E00, remote-control-vehicles §3.2): `Some(seat)` to send.
/// `moving`: not "stopped", where stopped = in GTA's moving list with a zero move speed (A 0x1009FE40).
/// `first_passenger`: the first passenger seat holding a player ped (`m_nPedType` +0x598 == 0, local or remote; AI
/// peds do not count): `None` = none, `Some((true, idx))` = the local ped in seat idx, `Some((false, _))`
/// = another ped. `local_closest`: [`local_is_closest`].
pub fn unoccupied_seat(
    has_seated_driver: bool,
    towed: bool,
    moving: bool,
    local_dist: f32,
    first_passenger: Option<(bool, u8)>,
    local_closest: bool,
) -> Option<u8> {
    if has_seated_driver || towed || !moving || local_dist > UNOCC_SEND_RANGE {
        return None;
    }
    match first_passenger {
        Some((true, idx)) => Some(idx + 1),
        Some((false, _)) => None,
        None => local_closest.then_some(0),
    }
}

/// The closest-player test of an empty car (A 0x10006E00): players in id order (ids 0 ..= max(highest id, local id)),
/// each with its distance to the car while its ped is in the world (`None` otherwise, A 0x1009F430). Id 0 seeds the
/// minimum unconditionally; later ids replace it only when strictly closer; the initial minimum is id 0 at 100000.
/// The local id must hold the minimum, at most 90 units away.
pub fn local_is_closest(local_id: u16, players: impl IntoIterator<Item = (u16, Option<f32>)>) -> bool {
    let (mut best_id, mut best) = (0u16, 100_000.0f32);
    for (id, d) in players {
        let Some(d) = d else { continue };
        if id == 0 {
            best = d;
        } else if d < best {
            best = d;
            best_id = id;
        }
    }
    best_id == local_id && best <= UNOCC_SEND_RANGE
}

/// Builds the 209 struct from the vehicle matrix and physics.
pub fn build_unoccupied_sync(
    vehicle_id: u16,
    seat: u8,
    rows: &Rotation,
    position: [f32; 3],
    velocity: [f32; 3],
    turn: [f32; 3],
    health: f32,
) -> UnoccupiedSync {
    UnoccupiedSync {
        vehicle_id,
        seat,
        roll: to_vec3(rows.right),
        direction: to_vec3(rows.forward),
        position: to_vec3(position),
        velocity: to_vec3(velocity),
        turn_velocity: to_vec3(turn),
        health,
    }
}

/// Damage-status cache of the 106 poll (vehicles.md "106 C→S").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageStatus {
    pub panels: u32,
    pub doors: u32,
    pub lights: u8,
    pub tyres: u8,
}

#[derive(Debug, Default)]
pub struct DamageCache {
    id: Option<u16>,
    last: DamageStatus,
}

impl DamageCache {
    /// True if RPC 106 must be sent (same vehicle, values changed). A vehicle change only refreshes the cache.
    pub fn update(&mut self, id: u16, now: DamageStatus) -> bool {
        if self.id != Some(id) {
            self.id = Some(id);
            self.last = now;
            return false;
        }
        if self.last != now {
            self.last = now;
            return true;
        }
        false
    }
}

/// Automobile tyre bits (bit3 +0x5A5 … bit0 +0x5A8) from the four wheel-status bytes at +0x5A5..+0x5A8.
pub fn car_tyre_bits(wheels: [u8; 4]) -> u8 {
    let mut t = 0;
    for (i, w) in wheels.iter().enumerate() {
        if *w != 0 {
            t |= 1 << (3 - i);
        }
    }
    t
}

/// Engine application per frame (vehicles.md 24 "Engine/lights application"). `param` −1/0/1.
pub fn engine_wanted(manual: bool, param: i32, driver_seated: bool) -> bool {
    if manual {
        param > 0
    } else {
        match param {
            0 => false,
            1 => true,
            _ => driver_seated,
        }
    }
}

/// Lights: `Some(on)` or `None` (untouched).
pub fn lights_wanted(manual: bool, param: i32) -> Option<bool> {
    if manual {
        Some(param > 0)
    } else {
        match param {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

/// One `vehicles` control-channel line.
#[allow(clippy::too_many_arguments)]
pub fn format_vehicle(
    id: u16,
    model: u32,
    handle: i32,
    ptr: u32,
    pos: Option<[f32; 3]>,
    heading_deg: Option<f32>,
    health: Option<f32>,
    driver: &str,
    local: &str,
) -> String {
    let pos = pos.map(|v| format!("{:.2},{:.2},{:.2}", v[0], v[1], v[2])).unwrap_or_else(|| "-".into());
    let heading = heading_deg.map(|h| format!("{h:.1}")).unwrap_or_else(|| "-".into());
    let health = health.map(|h| format!("{h:.0}")).unwrap_or_else(|| "-".into());
    format!(
        "vehicle id={id} model={model} handle={handle:#x} ptr={ptr:#x} pos={pos} heading={heading} health={health} \
         driver={driver} local={local}"
    )
}

/// Heading in degrees [0, 360) from the forward row (same convention as RPC 160 / SET_CAR_HEADING).
pub fn heading_deg(forward: [f32; 3]) -> f32 {
    let d = (-forward[0]).atan2(forward[1]).to_degrees().rem_euclid(360.0);
    if d >= 359.95 { 0.0 } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::rows_from_quat;

    #[test]
    fn kinds_and_sets() {
        assert_eq!(kind_from_vtable(vtbl::AUTOMOBILE), Kind::Automobile);
        assert_eq!(kind_from_vtable(vtbl::BMX), Kind::Bmx);
        assert_eq!(kind_from_vtable(vtbl::QUAD_BIKE), Kind::Other, "quads are not automobiles for samp");
        assert_eq!(kind_from_vtable(vtbl::TRAIN), Kind::Train);
        assert!(is_rail(570) && !is_rail(560));
        assert!(is_rc(441));
        assert!(tag_at_vehicle(state::DRIVER, Some(441)));
        assert!(!tag_at_vehicle(state::DRIVER, Some(411)), "an Infernus driver is tagged at the head");
        assert!(!tag_at_vehicle(state::PASSENGER, Some(441)));
        assert!(!tag_at_vehicle(state::DRIVER, None));
        assert_eq!(carriage_of(538), Some(570));
        assert_eq!(
            (train_type(538), train_type(537), train_type(449), train_type(560)),
            (Some(5), Some(3), Some(9), None)
        );
        assert!((z_threshold(Kind::Heli) - 2.0).abs() < 1e-6 && (z_threshold(Kind::Automobile) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn driver_correction() {
        let c = [0.0, 0.0, 0.0];
        let v = [0.1, 0.0, 0.0];
        assert_eq!(correct_driver(c, [1.0, 0.0, 0.0], v, false, 0.5), VehCorrection::Teleport);
        assert_eq!(correct_driver(c, [0.05, -0.05, 0.0], v, true, 0.5), VehCorrection::None);
        match correct_driver(c, [1.0, 0.04, 0.0], v, true, 0.5) {
            VehCorrection::Speed(s) => assert!((s[0] - 0.16).abs() < 1e-6 && s[1] == 0.0),
            o => panic!("{o:?}"),
        }
        // Small result → nothing written.
        assert_eq!(correct_driver(c, [0.1, 0.0, 0.0], [0.0; 3], true, 0.5), VehCorrection::None);
        assert_eq!(correct_driver(c, [8.5, 0.0, 0.0], v, true, 0.5), VehCorrection::Teleport);
        assert_eq!(correct_driver(c, [0.0, 0.0, 0.6], v, true, 0.5), VehCorrection::Teleport);
        assert!(matches!(correct_driver(c, [0.0, 0.0, 0.6], v, true, 2.0), VehCorrection::Speed(_)));
        assert_eq!(clamp_turn([0.5, -0.5, 0.01]), [0.02, -0.02, 0.01]);
        assert!(train_needs_teleport(c, [10.5, 0.0, 0.0]) && !train_needs_teleport(c, [9.0, 9.0, 50.0]));
    }

    #[test]
    fn unoccupied_apply_and_validation() {
        let c = [0.0, 0.0, 0.0];
        assert_eq!(correct_unoccupied(c, [0.1, 0.0, 0.0], [0.0; 3]), VehCorrection::None);
        match correct_unoccupied(c, [1.0, 0.04, 0.0], [0.2, 0.0, 0.0]) {
            VehCorrection::Speed(s) => assert!((s[0] - 0.26).abs() < 1e-6 && s[1] == 0.0),
            o => panic!("{o:?}"),
        }
        assert_eq!(correct_unoccupied(c, [0.0, 0.0, 3.5], [0.0; 3]), VehCorrection::Teleport);
        let mut s = UnoccupiedSync {
            vehicle_id: 5,
            roll: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            direction: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            ..Default::default()
        };
        assert!(unoccupied_valid(&s));
        s.velocity.x = 100.0;
        assert!(!unoccupied_valid(&s));
        s.velocity.x = 0.0;
        s.vehicle_id = 0;
        assert!(!unoccupied_valid(&s));
        let r = rows_from_roll_direction([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(r.up, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn dup_gate_shared_timestamp() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut g = DupGate::default();
        assert!(g.offer(t0, 200, vec![1]));
        assert!(!g.offer(at(30), 200, vec![1]), "identical within 501 ms");
        assert!(g.offer(at(60), 200, vec![2]));
        assert!(g.offer(at(90), 211, vec![2]), "other type, first copy");
        assert!(!g.offer(at(400), 200, vec![2]));
        assert!(g.offer(at(600), 200, vec![2]), "501 ms after the last send of any type");
        g.touch(at(1000));
        assert!(!g.offer(at(1400), 200, vec![2]), "a touch keeps identical structs suppressed");
        let mut t = IntervalTimer::default();
        let iv = in_car_interval(30, 1);
        assert!(t.due(t0, iv));
        assert!(!t.due(at(31), iv));
        assert!(t.due(at(32), iv));
    }

    #[test]
    fn driver_sync_fields() {
        let h = 1.0f32;
        let q = Quat { w: (h / 2.0).cos(), x: 0.0, y: 0.0, z: -(h / 2.0).sin() };
        let s = DriverSample {
            vehicle_id: 7,
            up_down: -128,
            keys: 8,
            rotation: rows_from_quat(q),
            position: [1.0, 2.0, 3.0],
            velocity: [0.1, 0.0, 0.0],
            vehicle_health: 1000.0,
            health: 99.9,
            armour: 0.0,
            weapon: 24,
            additional_key: 2,
            ..Default::default()
        };
        let p = build_driver_sync(&s, None);
        assert_eq!((p.vehicle_id, p.up_down, p.keys, p.player_health), (7, -128, 8, 99));
        assert!((p.rotation.w - q.w).abs() < 1e-5 && (p.rotation.z - q.z).abs() < 1e-5);
        assert_eq!((p.weapon.weapon(), p.weapon.additional_key()), (24, 2));
        let old = Quat { w: p.rotation.w + 5e-6, ..p.rotation };
        assert_eq!(build_driver_sync(&s, Some(old)).rotation, old);
        let ps = build_passenger_sync(7, 1, 0, 0, 100.0, 50.0, 0, 0, 0, [1.0, 2.0, 3.0]);
        assert_eq!((ps.seat, ps.health, ps.armour), (1, 100, 50));
    }

    #[test]
    fn unoccupied_send_rules() {
        // Nobody inside, local closest → seat 0.
        assert_eq!(unoccupied_seat(false, false, true, 10.0, None, true), Some(0));
        // Someone else is closest → no send.
        assert_eq!(unoccupied_seat(false, false, true, 10.0, None, false), None);
        // Not moving / out of range / driver / towed.
        assert_eq!(unoccupied_seat(false, false, false, 10.0, None, true), None);
        assert_eq!(unoccupied_seat(false, false, true, 95.0, None, true), None);
        assert_eq!(unoccupied_seat(true, false, true, 1.0, None, true), None);
        assert_eq!(unoccupied_seat(false, true, true, 1.0, None, true), None);
        // First occupied passenger seat decides.
        assert_eq!(unoccupied_seat(false, false, true, 1.0, Some((true, 2)), false), Some(3));
        assert_eq!(unoccupied_seat(false, false, true, 1.0, Some((false, 0)), true), None);
    }

    #[test]
    fn unoccupied_closest_player() {
        assert!(local_is_closest(3, [(3, Some(10.0))]));
        assert!(local_is_closest(3, [(1, Some(20.0)), (3, Some(10.0))]));
        assert!(!local_is_closest(3, [(1, Some(5.0)), (3, Some(10.0))]));
        // Ties go to the lower id; a ped outside the world does not count.
        assert!(!local_is_closest(3, [(1, Some(10.0)), (3, Some(10.0))]));
        assert!(local_is_closest(3, [(1, None), (3, Some(10.0))]));
        // Beyond 90 units, or the local ped itself not in the world.
        assert!(!local_is_closest(3, [(3, Some(90.5))]));
        assert!(local_is_closest(3, [(3, Some(90.0))]));
        assert!(!local_is_closest(3, [(3, None)]));
        // Id 0 seeds the minimum even when farther than the start value would allow others.
        assert!(local_is_closest(0, [(0, Some(50.0)), (4, Some(60.0))]));
        assert!(!local_is_closest(0, [(0, None), (4, Some(60.0))]));
        assert!(!local_is_closest(0, [(0, None)]));
    }

    #[test]
    fn damage_cache_and_tyres() {
        let mut c = DamageCache::default();
        let d = DamageStatus { panels: 1, ..Default::default() };
        assert!(!c.update(3, d), "first sight of a vehicle only refreshes");
        assert!(!c.update(3, d));
        assert!(c.update(3, DamageStatus { tyres: 1, ..d }));
        assert!(!c.update(4, DamageStatus::default()), "vehicle change");
        assert_eq!(car_tyre_bits([1, 0, 0, 1]), 0b1001);
        assert_eq!(car_tyre_bits([0, 0, 0, 1]), 0b0001);
    }

    #[test]
    fn engine_and_lights_rules() {
        assert!(engine_wanted(false, -1, true));
        assert!(!engine_wanted(false, -1, false));
        assert!(!engine_wanted(false, 0, true));
        assert!(engine_wanted(false, 1, false));
        assert!(!engine_wanted(true, -1, true));
        assert_eq!(lights_wanted(false, -1), None);
        assert_eq!(lights_wanted(true, -1), Some(false));
        assert_eq!(lights_wanted(false, 1), Some(true));
    }

    #[test]
    fn format_and_heading() {
        let l = format_vehicle(3, 560, 0x101, 0x1234, Some([1.0, 2.0, 3.0]), Some(90.0), Some(1000.0), "5", "driver");
        assert_eq!(
            l,
            "vehicle id=3 model=560 handle=0x101 ptr=0x1234 pos=1.00,2.00,3.00 heading=90.0 health=1000 driver=5 local=driver"
        );
        assert!((heading_deg([-1.0, 0.0, 0.0]) - 90.0).abs() < 1e-4);
        assert!((heading_deg([1.0, 0.0, 0.0]) - 270.0).abs() < 1e-4);
        assert_eq!(heading_deg([1e-7, 1.0, 0.0]), 0.0);
    }
}
