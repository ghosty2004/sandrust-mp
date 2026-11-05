//! Vehicles inside the game (Windows only, milestone M5): the samp vehicle pool, the vehicle RPCs, remote
//! drivers/passengers, unoccupied sync and the local in-car senders. Runs on the game's main thread from the
//! client tick (samp: netgame tick → receive → player pool → local player → vehicle pool).
//!
//! Specs: vehicles.md (every RPC), remote-control-vehicles.md (§1 lifecycle, §2 remote drivers, §3 passengers and
//! unoccupied cars, §4 trailers/trains, §5 enter/exit), remote-players-sync.md (§0.4, §200, §209, §210, §211,
//! §2.1). Measured behaviour of the original client: docs/phase3/trace-notes.md "Vehicles (TRACE-4)" — stream-in
//! opcode order with the busy-wait load and z + 0.25, the per-frame proofs/tyres pair for every car, IS_CAR_IN_WATER
//! every 4th frame, blips, remote seating by WARP_CHAR_INTO_CAR on the first 200 and WARP_CHAR_FROM_CAR_TO_COORD
//! to the ped's own position, corrections after `CWorld::Process` (rotation slerp every frame, speed writes per
//! packet, no position writes in normal driving), RPC 26/154 at the moment of F, no sync during the exit
//! animation, stream-out order.
//!
//! Not implemented (TODO-unknown or later milestones, see docs/phase3/m5-notes.md): weapons / drive-by / cuffs of
//! remote occupants and the 203 aim sender (M7), "Car Jacked" ejection, the RC-driver world removal, plate
//! textures (D3D proxy), model release on stream-out (MR-5.9), SCM event type 4 (mod shop), spectating.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use samp_proto::rpcs::OutgoingRpc;
use samp_proto::rpcs::vehicle::{self as rv, Incoming as V};
use samp_proto::sync::IncomingSync;
use samp_proto::sync::SyncPacket;
use samp_proto::sync::vehicle::TrailerSync;
use samp_proto::types::Quat;

use crate::footsync::{self, Rotation, quat_from_matrix};
use crate::game::{self, off};
use crate::netclient::NetClient;
use crate::patch::{Patcher, ProcessMemory};
use crate::players::Players;
use crate::remote::{self, PadInput, accept_timestamp, normalise, rows_from_quat, slerp};
use crate::script::{Arg, Runner, op};
use crate::vehhooks::{self, HookEvent, VehInfo};
use crate::vehicle::{self as vm, DriverData, Kind, PassengerData, RemoteVeh, VehCorrection, state};

/// GTA addresses / fields (names via gta-reversed / plugin-sdk).
mod g {
    pub const VEHICLE_POOL: u32 = 0xB74494;
    /// `CPool<CVehicle>::GetAt(handle)` (thiscall).
    pub const POOL_GET_AT: u32 = 0x4048E0;
    pub const MOVING_LIST: u32 = 0xB4;
    pub const TURN_SPEED: u32 = 0x50;
    pub const VEH_FLAGS: u32 = 0x428;
    pub const VEH_SIREN_BYTE: u32 = 0x42D;
    pub const VEH_PRIMARY_COLOUR: u32 = 0x434;
    pub const VEH_SECONDARY_COLOUR: u32 = 0x435;
    pub const VEH_ALARM: u32 = 0x45C;
    pub const VEH_DRIVER: u32 = 0x460;
    pub const VEH_PASSENGERS: u32 = 0x464;
    pub const VEH_MAX_PASSENGERS: u32 = 0x488;
    pub const VEH_LIGHTS_FLAGS: u32 = 0x4A8;
    pub const VEH_DIRT: u32 = 0x4B0;
    pub const VEH_HEALTH: u32 = 0x4C0;
    pub const VEH_TRACTOR: u32 = 0x4C4;
    pub const VEH_TRAILER: u32 = 0x4C8;
    pub const VEH_DOOR_LOCK: u32 = 0x4F8;
    pub const VEH_HORN: u32 = 0x514;
    /// CAutomobile damage manager fields.
    pub const DMG_WHEELS: u32 = 0x5A5;
    pub const DMG_DOORS: u32 = 0x5A9;
    pub const DMG_LIGHTS: u32 = 0x5B0;
    pub const DMG_PANELS: u32 = 0x5B4;
    pub const BIKE_TYRE_FRONT: u32 = 0x65C;
    pub const BIKE_TYRE_REAR: u32 = 0x65D;
    pub const BIKE_LEAN: u32 = 0x648;
    pub const BIKE_DESIRED_LEAN: u32 = 0x64C;
    pub const TRAIN_SPEED: u32 = 0x5A4;
    pub const TRAIN_NEXT_CARRIAGE: u32 = 0x5D4;
    pub const HYDRA_NOZZLE: u32 = 0x86C;
    pub const PLANE_GEAR: u32 = 0x9CC;
    pub const PED_VEHICLE: u32 = 0x58C;
    pub const PED_TYPE: u32 = 0x598;
    pub const VTBL_CPLACEABLE: u32 = 0x863C40;
    pub const AUTOMOBILE_FIX: u32 = 0x6A3440;
    pub const AUTOMOBILE_SETUP_DAMAGE: u32 = 0x6B3E90;
    pub const AUTOMOBILE_OPEN_DOOR: u32 = 0x6A6AE0;
    pub const SET_WINDOW_OPEN: u32 = 0x6D3080;
    pub const CLEAR_WINDOW_OPEN: u32 = 0x6D30B0;
    pub const PLANE_GEAR_UP: u32 = 0x6CAC20;
    pub const PLANE_GEAR_DOWN: u32 = 0x6CAC70;
    /// Pads[0] DPadUp new / old state.
    pub const PAD_DPAD_UP_NEW: u32 = 0xB73458 + 0x10;
    pub const PAD_DPAD_UP_OLD: u32 = 0xB73458 + 0x40;
}

const PEDSTATE_IDLE: u32 = 1;
const PEDSTATE_SNIPER: u32 = 12;
const PEDSTATE_DRIVING: u32 = 50;
/// GTA's passenger state (passengers normally show 50 as well; accepted for safety).
const PEDSTATE_PASSENGER: u32 = 51;

fn rd<T: Copy + Default>(a: u32) -> T {
    if a < 0x10000 {
        return T::default();
    }
    // SAFETY: callers pass fields of live game objects / GTA globals.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

fn poke<T: Copy>(a: u32, v: T) {
    if a < 0x10000 {
        return;
    }
    // SAFETY: callers pass fields of live game objects.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) };
}

fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

fn put_v3(a: u32, v: [f32; 3]) {
    poke(a, v[0]);
    poke(a + 4, v[1]);
    poke(a + 8, v[2]);
}

fn matrix(ent: u32) -> u32 {
    rd(ent + off::PLACEABLE_MATRIX)
}

fn pos_of(ent: u32) -> Option<[f32; 3]> {
    let m = matrix(ent);
    if m == 0 { None } else { Some(v3(m + off::MATRIX_POS)) }
}

fn rows_of(m: u32) -> Rotation {
    Rotation { right: v3(m), forward: v3(m + 0x10), up: v3(m + 0x20) }
}

fn write_rows(m: u32, r: &Rotation) {
    put_v3(m, r.right);
    put_v3(m + 0x10, r.forward);
    put_v3(m + 0x20, r.up);
}

fn in_vehicle(ped: u32) -> bool {
    rd::<u32>(ped + off::PED_FLAGS) & (1 << 8) != 0
}

fn local_ped() -> u32 {
    rd(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED)
}

/// Current weapon type of a ped (`m_aWeapons[m_nActiveWeaponSlot].m_eWeaponType`).
pub fn current_weapon(ped: u32) -> u32 {
    let slot = u32::from(rd::<u8>(ped + off::PED_SELECTED_WEAPON_SLOT).min(12));
    rd(ped + off::PED_WEAPONS + slot * off::WEAPON_SIZE)
}

fn vehicle_at(handle: i32) -> u32 {
    let pool: u32 = rd(g::VEHICLE_POOL);
    if pool == 0 || handle < 0 {
        return 0;
    }
    // SAFETY: GTA's own pool lookup (thiscall) on the live vehicle pool.
    unsafe {
        let f: extern "thiscall" fn(u32, i32) -> u32 = std::mem::transmute(g::POOL_GET_AT as usize);
        f(pool, handle)
    }
}

fn thiscall0(f: u32, this: u32) {
    // SAFETY: GTA thiscall without arguments on a live object.
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(f as usize);
        f(this);
    }
}

fn kind_of(veh: u32) -> Kind {
    vm::kind_from_vtable(rd(veh))
}

fn send<P: OutgoingRpc>(net: &mut NetClient, p: &P) {
    net.send_rpc(p, P::RELIABILITY);
}

/// One samp vehicle object.
#[derive(Clone, Debug)]
struct Veh {
    id: u16,
    model: u32,
    handle: i32,
    ptr: u32,
    kind: Kind,
    engine: i32,
    lights: i32,
    locked: bool,
    objective: bool,
    siren: bool,
    colours: Option<(u8, u8)>,
    in_mod_shop: bool,
    plate: Vec<u8>,
    /// Attached trailer object (RPC 148 / packet 210).
    trailer: Option<u16>,
    wasted_at: Option<Instant>,
    blip: Option<i32>,
    objective_blip: Option<i32>,
    last_unocc_send: Option<Instant>,
    /// Last remote player whose 209 was applied (pool +0x8E34).
    last_unocc_sender: Option<u16>,
    /// Carriage of a locomotive: no own GTA vehicle created, deleted with the locomotive.
    carriage: bool,
}

impl Veh {
    fn new(id: u16, model: u32, handle: i32, ptr: u32) -> Self {
        Self {
            id,
            model,
            handle,
            ptr,
            kind: kind_of(ptr),
            engine: -1,
            lights: -1,
            locked: false,
            objective: false,
            siren: false,
            colours: None,
            in_mod_shop: false,
            plate: Vec::new(),
            trailer: None,
            wasted_at: None,
            blip: None,
            objective_blip: None,
            last_unocc_send: None,
            last_unocc_sender: None,
            carriage: false,
        }
    }
}

#[derive(Default)]
struct Counters {
    sent_200: u32,
    sent_210: u32,
    sent_211: u32,
    sent_209: u32,
    sent_26: u32,
    sent_154: u32,
    sent_106: u32,
    sent_136: u32,
    applied_209: u32,
}

#[derive(Default)]
pub struct Vehicles {
    pool: BTreeMap<u16, Veh>,
    remote: HashMap<u16, RemoteVeh>,
    manual: bool,
    in_car_rate: u32,
    local_id: Option<u16>,
    frame: u64,
    dup: vm::DupGate,
    driver_timer: vm::IntervalTimer,
    passenger_timer: vm::IntervalTimer,
    last_rotation: Option<Quat>,
    damage: vm::DamageCache,
    last_driven: Option<u16>,
    /// The vehicle the local ped drove in the previous tick.
    driving: Option<u16>,
    spawned_once: bool,
    stats_done: bool,
    patcher: Patcher,
    train_camera_since: Option<Instant>,
    n: Counters,
    /// Set when a 200 went out (combat: 203 after each 200 of an armed vehicle).
    driver_sent: Option<u16>,
    /// The local passenger drive-by is active (combat, local player +0x1CB): 211 drive-by bit.
    pub local_drive_by: bool,
    /// The local player is cuffed (special action 24): 211 cuffed bit.
    pub local_cuffed: bool,
    /// Remote +0x1E7 per player (see `remote_vehicle`).
    last_vehicle: HashMap<u16, u16>,
}

impl Vehicles {
    pub fn set_init_game(&mut self, g: &samp_proto::session::InitGame) {
        self.manual = g.manual_engine_and_lights;
        self.in_car_rate = g.in_car_rate.clamp(10, 1000);
        self.local_id = Some(g.player_id);
        if self.manual {
            // CSV rows 140–147, once.
            let r = crate::vehpatches::apply(&mut self.patcher, &mut ProcessMemory, crate::vehpatches::MANUAL_LIGHTS);
            crate::log!("vehicles: manual engine/lights: {} rows applied, {} failed", r.applied.len(), r.failed.len());
        }
    }

    pub fn set_spawned(&mut self) {
        self.spawned_once = true;
    }

    pub fn set_disable_collisions(&mut self, on: bool) {
        vehhooks::with_shared(|s| s.disable_collisions = on);
    }

    /// Script handle of a streamed vehicle (object attachments, M6).
    pub fn script_handle(&self, id: u16) -> Option<i32> {
        self.get(id).map(|v| v.handle)
    }

    /// GTA vehicle pointer of a streamed vehicle (3D label attachments, M6).
    pub fn gta_ptr(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| vehicle_at(v.handle)).filter(|&p| p != 0)
    }

    /// SA:MP id of a GTA vehicle of the pool (world bounds for the driver, M8b).
    pub fn id_of(&self, ptr: u32) -> Option<u16> {
        self.id_of_ptr(ptr)
    }

    fn id_of_ptr(&self, ptr: u32) -> Option<u16> {
        if ptr == 0 {
            return None;
        }
        self.pool.values().find(|v| v.ptr == ptr && v.id != 0).map(|v| v.id)
    }

    fn get(&self, id: u16) -> Option<&Veh> {
        self.pool.get(&id).filter(|v| v.ptr != 0)
    }

    /// First frame: GTA's vehicle- and weapon-skill cheats, then `CStats::SetStatValue := RET` (CSV row 356).
    pub fn first_frame(&mut self) {
        if self.stats_done {
            return;
        }
        self.stats_done = true;
        // SAFETY: GTA's own cdecl cheat functions (no arguments), main thread.
        unsafe {
            let f: extern "C" fn() = std::mem::transmute(crate::vehpatches::VEHICLE_SKILLS_CHEAT as usize);
            f();
            let f: extern "C" fn() = std::mem::transmute(crate::vehpatches::WEAPON_SKILLS_CHEAT as usize);
            f();
        }
        let r = crate::vehpatches::apply(&mut self.patcher, &mut ProcessMemory, crate::vehpatches::STATS);
        crate::log!(
            "vehicles: skill stats := 1000, SetStatValue ret ({} applied, {} failed)",
            r.applied.len(),
            r.failed.len()
        );
    }

    // ---- RPCs ---------------------------------------------------------------------------------------------

    pub fn on_rpc(&mut self, runner: &mut Runner, players: &mut Players, r: V) {
        match r {
            V::StreamInVehicle(s) => self.stream_in(runner, &s),
            V::StreamOutVehicle(s) => self.stream_out(runner, players, s.vehicle_id),
            V::SetVehicleParams(p) => self.set_params(p.vehicle_id, &p.params),
            V::SetVehicleHealth(h) => {
                if let Some(v) = self.get(h.vehicle_id) {
                    poke(v.ptr + g::VEH_HEALTH, h.health);
                }
            }
            V::SetVehicleDamageStatus(d) => {
                if let Some(v) = self.get(d.vehicle_id) {
                    apply_damage(v.ptr, v.kind, d.panels, d.doors, d.lights);
                    apply_tyres(v.ptr, v.kind, d.tyres);
                }
            }
            V::RemoveVehicleComponent(c) => {
                if let Some(v) = self.get(c.vehicle_id) {
                    runner.run(op::REMOVE_VEHICLE_MOD, &[Arg::Int(v.handle), Arg::Int(i32::from(c.component))]);
                }
            }
            V::LinkVehicleToInterior(l) => {
                if let Some(v) = self.get(l.vehicle_id) {
                    runner.run(op::SET_VEHICLE_AREA_VISIBLE, &[Arg::Int(v.handle), Arg::Int(i32::from(l.interior))]);
                }
            }
            V::SetVehicleVelocity(s) => {
                let ped = local_ped();
                if ped != 0 && in_vehicle(ped) {
                    let veh: u32 = rd(ped + g::PED_VEHICLE);
                    if self.id_of_ptr(veh).is_some() {
                        let v = vm::vec3(s.velocity);
                        match s.kind {
                            0 => put_v3(veh + off::MOVE_SPEED, v),
                            1 => put_v3(veh + g::TURN_SPEED, v),
                            _ => {}
                        }
                    }
                }
            }
            V::ScmEvent(e) => self.scm_event(runner, &e),
            V::SetVehiclePlate(p) => {
                // Text only; the texture is created once by the plate pass (`plates.rs`).
                if p.plate.len() <= 32
                    && let Some(v) = self.pool.get_mut(&p.vehicle_id)
                {
                    v.plate = p.plate.clone();
                }
            }
            V::AttachTrailer(a) => {
                let (Some(t), Some(c)) = (self.get(a.trailer_id), self.get(a.vehicle_id)) else { return };
                let (th, ch) = (t.handle, c.handle);
                if let Some(c) = self.pool.get_mut(&a.vehicle_id) {
                    c.trailer = Some(a.trailer_id);
                }
                runner.run(op::ATTACH_TRAILER_TO_CAB, &[Arg::Int(th), Arg::Int(ch)]);
            }
            V::DetachTrailer(d) => self.detach_trailer(runner, d.vehicle_id),
            V::SetVehiclePosition(p) => {
                if let Some(v) = self.get(p.vehicle_id) {
                    let (x, y, z) = (p.position.x, p.position.y, p.position.z);
                    if vm::TRAIN_SYNC_MODELS.contains(&v.model) {
                        runner.run(
                            op::SET_MISSION_TRAIN_COORDINATES,
                            &[Arg::Int(v.handle), Arg::Float(x), Arg::Float(y), Arg::Float(z)],
                        );
                    } else {
                        teleport(v.ptr, [x, y, z]);
                    }
                }
            }
            V::SetVehicleZAngle(a) => {
                if let Some(v) = self.get(a.vehicle_id) {
                    runner.run(op::SET_CAR_HEADING, &[Arg::Int(v.handle), Arg::Float(a.angle)]);
                }
            }
            V::PutPlayerInVehicle(p) => self.put_local_in(runner, p.vehicle_id, p.seat),
            V::RemovePlayerFromVehicle(_) => {
                let ped = local_ped();
                if ped != 0 && in_vehicle(ped) {
                    let veh: u32 = rd(ped + g::PED_VEHICLE);
                    if let Some(v) = self.id_of_ptr(veh).and_then(|id| self.get(id))
                        && !matches!(v.model, 538 | 570)
                        && let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten()
                    {
                        runner.run(op::TASK_LEAVE_CAR, &[Arg::Int(h), Arg::Int(v.handle)]);
                    }
                }
            }
            V::EnterVehicle(e) => self.remote_enter(runner, players, e.player_id, e.vehicle_id, e.passenger != 0),
            V::ExitVehicle(e) => self.remote_exit(runner, players, e.player_id),
        }
    }

    /// RPC 98 (tyres) and 161 (old objective / doors), session-misc.md.
    pub fn on_tyres(&mut self, id: u16, tyres: u8) {
        if let Some(v) = self.get(id) {
            apply_tyres(v.ptr, v.kind, tyres);
        }
    }

    pub fn on_old_params(&mut self, id: u16, objective: u8, doors: u8) {
        if let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) {
            if objective != 0 {
                v.objective = true;
            }
            v.locked = doors != 0;
            poke(v.ptr + g::VEH_DOOR_LOCK, if doors != 0 { 2u32 } else { 0 });
        }
    }

    /// RPC 164 (vehicles.md 164; remote-control-vehicles §1.2; TRACE-4 §1 opcode order).
    fn stream_in(&mut self, runner: &mut Runner, s: &rv::StreamInVehicle) {
        if s.vehicle_id >= vm::MAX_VEHICLES || !(vm::MODEL_MIN..=vm::MODEL_MAX).contains(&s.model) {
            return;
        }
        if self.pool.contains_key(&s.vehicle_id) {
            crate::log!("Warning: vehicle {} was not deleted", s.vehicle_id);
            self.delete(runner, s.vehicle_id);
        }
        let (x, y, z) = (s.position.x, s.position.y, s.position.z);
        let model = s.model as i32;
        let handle;
        if let Some(train) = vm::train_type(s.model) {
            for m in vm::RAIL_MODELS {
                busy_load(runner, m as i32);
            }
            let dir = i32::from(s.angle > 180.0);
            runner.run(
                op::CREATE_MISSION_TRAIN,
                &[Arg::Int(train), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Int(dir), Arg::Local(0)],
            );
            handle = runner.local_i32(0);
        } else {
            busy_load(runner, model);
            runner.run(op::CREATE_CAR, &[Arg::Int(model), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Local(0)]);
            handle = runner.local_i32(0);
            runner.run(op::SET_CAR_HEADING, &[Arg::Int(handle), Arg::Float(s.angle)]);
            runner.run(op::SET_PETROL_TANK_WEAKPOINT, &[Arg::Int(handle), Arg::Int(0)]);
            runner.run(op::SET_CAR_HYDRAULICS, &[Arg::Int(handle), Arg::Int(0)]);
            runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(handle), Arg::Int(0)]);
        }
        let ptr = vehicle_at(handle);
        if ptr == 0 {
            crate::log!("Warning: couldn't create vehicle type: {}", s.model);
            return;
        }
        let mut v = Veh::new(s.vehicle_id, s.model, handle, ptr);
        if vm::train_type(s.model).is_none() {
            poke(ptr + g::VEH_DOOR_LOCK, 0u32);
            poke(ptr + g::VEH_DIRT, 0.0f32);
            let m = matrix(ptr);
            if m != 0 {
                let dz = if matches!(v.kind, Kind::Bike | Kind::Bmx) { 0.0 } else { vm::CREATE_Z_OFFSET };
                put_v3(m + off::MATRIX_POS, [x, y, z + dz]);
            }
        }
        v.siren = s.siren != 0;
        if s.colour1 != rv::StreamInVehicle::COLOUR_DEFAULT || s.colour2 != rv::StreamInVehicle::COLOUR_DEFAULT {
            poke(ptr + g::VEH_PRIMARY_COLOUR, s.colour1);
            poke(ptr + g::VEH_SECONDARY_COLOUR, s.colour2);
            v.colours = Some((s.colour1, s.colour2));
        }
        poke(ptr + g::VEH_HEALTH, s.health);
        if s.interior != 0 {
            runner.run(op::SET_VEHICLE_AREA_VISIBLE, &[Arg::Int(handle), Arg::Int(i32::from(s.interior))]);
        }
        if s.panel_damage != 0 || s.door_damage != 0 || s.light_damage != 0 {
            apply_damage(ptr, v.kind, s.panel_damage, s.door_damage, s.light_damage);
        }
        apply_tyres(ptr, v.kind, s.tyre_damage);
        crate::log!(
            "vehicles: stream-in id={} model={} handle={handle:#x} ptr={ptr:#x} pos={x:.2},{y:.2},{z:.2} angle={:.1}",
            s.vehicle_id,
            s.model,
            s.angle
        );
        let kind = v.kind;
        self.pool.insert(s.vehicle_id, v);
        // Locomotive carriages: GTA created them with the train; samp objects take m_pNextCarriage in turn.
        if let Some(cm) = vm::carriage_of(s.model) {
            let mut prev = ptr;
            for i in 1..=3u16 {
                let next: u32 = rd(prev + g::TRAIN_NEXT_CARRIAGE);
                if next == 0 {
                    break;
                }
                let h = pool_ref(next);
                let mut c = Veh::new(s.vehicle_id + i, cm, h, next);
                c.carriage = true;
                self.pool.insert(s.vehicle_id + i, c);
                prev = next;
            }
        }
        if kind == Kind::Automobile {
            for slot in 0..s.mods.len() {
                if let Some(c) = s.component(slot) {
                    add_component(runner, handle, i32::from(c));
                }
            }
            if let Some(p) = s.paintjob()
                && p < 4
            {
                runner.run(op::GIVE_VEHICLE_PAINTJOB, &[Arg::Int(handle), Arg::Int(i32::from(p))]);
            }
        }
        if s.body_colour1 != rv::StreamInVehicle::BODY_COLOUR_UNSET
            || s.body_colour2 != rv::StreamInVehicle::BODY_COLOUR_UNSET
        {
            self.set_colours(s.vehicle_id, s.body_colour1 as u8, s.body_colour2 as u8);
        }
    }

    fn set_colours(&mut self, id: u16, c1: u8, c2: u8) {
        if let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) {
            poke(v.ptr + g::VEH_PRIMARY_COLOUR, c1);
            poke(v.ptr + g::VEH_SECONDARY_COLOUR, c2);
            v.colours = Some((c1, c2));
        }
    }

    /// RPC 165 (vehicles.md 165; TRACE-4 §5: occupants warped first, then DELETE_CAR).
    fn stream_out(&mut self, runner: &mut Runner, players: &mut Players, id: u16) {
        let Some(v) = self.get(id).cloned() else { return };
        crate::log!("vehicles: stream-out id={id}");
        let local = local_ped();
        if local != 0 && in_vehicle(local) && rd::<u32>(local + g::PED_VEHICLE) == v.ptr {
            // A 0x100AC5B0: the local ped goes to the vehicle position (interpretation: the warp opcode).
            if let (Some(p), Some(h)) = (pos_of(v.ptr), game::ped_handle(&ProcessMemory, local).ok().flatten()) {
                runner.run(
                    op::WARP_CHAR_FROM_CAR_TO_COORD,
                    &[Arg::Int(h), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2])],
                );
            }
        }
        for p in players.iter_mut() {
            if p.ped != 0 && in_vehicle(p.ped) && rd::<u32>(p.ped + g::PED_VEHICLE) == v.ptr {
                warp_out_own_position(runner, p.ped);
                if let Some(r) = self.remote.get_mut(&p.id) {
                    r.warps_out += 1;
                }
            }
        }
        let cabs: Vec<u16> = self.pool.values().filter(|c| c.trailer == Some(id)).map(|c| c.id).collect();
        for c in cabs {
            self.detach_trailer(runner, c);
        }
        self.delete(runner, id);
        if matches!(v.model, 537 | 538) {
            for i in 1..=3 {
                self.pool.remove(&(id + i));
            }
        }
    }

    /// The samp object's destructor (remote-control-vehicles §1.3).
    fn delete(&mut self, runner: &mut Runner, id: u16) {
        let Some(v) = self.pool.remove(&id) else { return };
        if v.carriage || v.ptr == 0 {
            return;
        }
        for b in [v.blip, v.objective_blip].into_iter().flatten() {
            runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
        }
        let pos = pos_of(v.ptr).unwrap_or_default();
        let mut occupants = vec![rd::<u32>(v.ptr + g::VEH_DRIVER)];
        occupants.extend((0..8).map(|i| rd::<u32>(v.ptr + g::VEH_PASSENGERS + 4 * i)));
        for ped in occupants.into_iter().filter(|&p| p != 0) {
            if let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() {
                let z = pos[2] + vm::EJECT_Z_OFFSET;
                runner.run(
                    op::WARP_CHAR_FROM_CAR_TO_COORD,
                    &[Arg::Int(h), Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(z)],
                );
            }
        }
        if let Some(t) = v.trailer.and_then(|t| self.get(t)) {
            let th = t.handle;
            vehhooks::with_shared(|s| s.detaching = true);
            runner.run(op::DETACH_TRAILER_FROM_CAB, &[Arg::Int(th), Arg::Int(v.handle)]);
            vehhooks::with_shared(|s| s.detaching = false);
        }
        if matches!(v.model, 537 | 538) {
            runner.run(op::DELETE_MISSION_TRAIN, &[Arg::Int(v.handle)]);
        } else {
            runner.run(op::DELETE_CAR, &[Arg::Int(v.handle)]);
        }
        // Model release (> 80 loaded vehicle models, object +0x87) not reproduced: MR-5.9.
        crate::log!("vehicles: id={id} handle={:#x} deleted", v.handle);
    }

    fn detach_trailer(&mut self, runner: &mut Runner, cab: u16) {
        let Some(c) = self.get(cab) else { return };
        let ch = c.handle;
        if let Some(t) = c.trailer.and_then(|t| self.get(t)) {
            let th = t.handle;
            vehhooks::with_shared(|s| s.detaching = true);
            runner.run(op::DETACH_TRAILER_FROM_CAB, &[Arg::Int(th), Arg::Int(ch)]);
            vehhooks::with_shared(|s| s.detaching = false);
        }
        if let Some(c) = self.pool.get_mut(&cab) {
            c.trailer = None;
        }
    }

    /// RPC 24 (vehicles.md 24; names per RE-10 "GTA function names").
    fn set_params(&mut self, id: u16, p: &rv::VehicleParams) {
        let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) else { return };
        let ptr = v.ptr;
        let bit = |b: u8| matches!(b, 0 | 1).then_some(b == 1);
        if let Some(on) = bit(p.engine) {
            v.engine = i32::from(on);
        }
        if let Some(on) = bit(p.lights) {
            v.lights = i32::from(on);
        }
        if let Some(on) = bit(p.alarm) {
            let s: u8 = rd(ptr + g::VEH_SIREN_BYTE);
            poke(ptr + g::VEH_SIREN_BYTE, if on { s | 0x80 } else { s & 0x7F });
            poke(ptr + g::VEH_ALARM, if on { 20000u16 } else { 0 });
        }
        if let Some(on) = bit(p.doors) {
            poke(ptr + g::VEH_DOOR_LOCK, if on { 2u32 } else { 0 });
            v.locked = on;
        }
        if let Some(on) = bit(p.objective) {
            v.objective = on;
        }
        if let Some(on) = bit(p.siren) {
            let s: u8 = rd(ptr + g::VEH_SIREN_BYTE);
            poke(ptr + g::VEH_SIREN_BYTE, if on { s | 0x80 } else { s & 0x7F });
        }
        if v.kind == Kind::Automobile {
            // (wire byte, node, door index)
            let doors = [
                (p.bonnet, 0x10, 0),
                (p.boot, 0x11, 1),
                (p.door_driver, 10, 2),
                (p.door_passenger, 8, 3),
                (p.door_back_left, 11, 4),
                (p.door_back_right, 9, 5),
            ];
            for (b, node, door) in doors {
                if let Some(open) = bit(b) {
                    // SAFETY: CAutomobile::OpenDoor(CPed*, node, door, ratio, playSound) on a live automobile.
                    // The ped / sound arguments samp passes are not documented: null / false here.
                    unsafe {
                        let f: extern "thiscall" fn(u32, u32, i32, i32, f32, u8) =
                            std::mem::transmute(g::AUTOMOBILE_OPEN_DOOR as usize);
                        f(ptr, 0, node, door, if open { 1.0 } else { 0.0 }, 0);
                    }
                }
            }
        }
        for (b, node) in
            [(p.window_driver, 10u32), (p.window_passenger, 8), (p.window_back_left, 11), (p.window_back_right, 9)]
        {
            if let Some(closed) = bit(b) {
                let f = if closed { g::CLEAR_WINDOW_OPEN } else { g::SET_WINDOW_OPEN };
                // SAFETY: CVehicle::Set/ClearWindowOpenFlag(uint8 node) on a live vehicle.
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(f as usize);
                    f(ptr, node);
                }
            }
        }
    }

    /// RPC 96 S→C (vehicles.md 96).
    fn scm_event(&mut self, runner: &mut Runner, e: &rv::ScmEvent) {
        if !self.spawned_once || e.vehicle_id >= u32::from(vm::MAX_VEHICLES) {
            return;
        }
        let id = e.vehicle_id as u16;
        let Some(v) = self.get(id) else { return };
        let (h, kind) = (v.handle, v.kind);
        match e.event_type {
            1 if kind == Kind::Automobile && e.arg1 < 4 => {
                runner.run(op::GIVE_VEHICLE_PAINTJOB, &[Arg::Int(h), Arg::Int(e.arg1 as i32)]);
            }
            2 if kind == Kind::Automobile => add_component(runner, h, e.arg1 as i32),
            3 => self.set_colours(id, e.arg1 as u8, e.arg2 as u8),
            // 4: mod-shop follow for passengers — TODO-unknown (A 0x100AC7D0 not specified).
            _ => {}
        }
    }

    /// RPC 70 for the local ped (A 0x100AC2D0).
    fn put_local_in(&mut self, runner: &mut Runner, id: u16, seat: u8) {
        let Some(v) = self.get(id).cloned() else { return };
        let ped = local_ped();
        let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
        if !seat_allowed(&v, seat) {
            return;
        }
        // samp reads the weapon with GET_CURRENT_CHAR_WEAPON (A 0x100abc90; efx RPC 70 diff).
        runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Local(0)]);
        if runner.local_i32(0) as u32 == u32::from(vm::WEAPON_PARACHUTE) {
            runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
        }
        warp_into(runner, h, &v, seat);
        if in_vehicle(ped) {
            camera_behind(runner);
            if vm::is_rail(v.model) {
                runner.run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(v.handle), Arg::Int(3), Arg::Int(2)]);
            }
        }
    }

    /// RPC 26 S→C relay (vehicles.md 26).
    fn remote_enter(&mut self, runner: &mut Runner, players: &mut Players, pid: u16, id: u16, passenger: bool) {
        let Some(v) = self.get(id).cloned() else { return };
        let Some(p) = players.get_mut(pid) else { return };
        if p.ped == 0 || !within_relay_range(p.ped) || in_vehicle(p.ped) {
            return;
        }
        clear_keys(p);
        let Some(h) = game::ped_handle(&ProcessMemory, p.ped).ok().flatten() else { return };
        if passenger {
            runner.run(
                op::TASK_ENTER_CAR_AS_PASSENGER,
                &[Arg::Int(h), Arg::Int(v.handle), Arg::Int(vm::ENTER_TASK_MS), Arg::Int(-1)],
            );
        } else {
            runner.run(op::TASK_ENTER_CAR_AS_DRIVER, &[Arg::Int(h), Arg::Int(v.handle), Arg::Int(vm::ENTER_TASK_MS)]);
        }
    }

    /// RPC 154 S→C relay: the vehicle id is ignored; the ped leaves whatever it is in.
    fn remote_exit(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get_mut(pid) else { return };
        if p.ped == 0 || !within_relay_range(p.ped) || !in_vehicle(p.ped) {
            return;
        }
        let veh: u32 = rd(p.ped + g::PED_VEHICLE);
        let model = if veh != 0 { u32::from(rd::<u16>(veh + off::ENTITY_MODEL)) } else { 0 };
        if matches!(model, 570 | 538) {
            return;
        }
        clear_keys(p);
        let vh = self.id_of_ptr(veh).and_then(|id| self.get(id)).map(|v| v.handle);
        if let (Some(h), Some(vh)) = (game::ped_handle(&ProcessMemory, p.ped).ok().flatten(), vh) {
            runner.run(op::TASK_LEAVE_CAR, &[Arg::Int(h), Arg::Int(vh)]);
        }
    }

    // ---- sync receive -------------------------------------------------------------------------------------

    /// 200/211 are stored, 209/210 applied immediately (remote-control-vehicles §6 step 3a).
    pub fn on_sync(&mut self, runner: &mut Runner, players: &mut Players, s: IncomingSync, now: Instant) {
        match s {
            IncomingSync::Vehicle { timestamp, sync } => {
                if Some(sync.player_id) == self.local_id {
                    return;
                }
                let Some(p) = players.get_mut(sync.player_id) else { return };
                if !accept_timestamp(p.last_ts, timestamp) {
                    return;
                }
                if timestamp.is_some() {
                    p.last_ts = timestamp;
                }
                let d = DriverData::from_sync(&sync);
                p.health = d.health;
                p.armour = d.armour;
                p.state = state::DRIVER;
                p.last_sync = Some(now);
                let r = self.remote.entry(sync.player_id).or_default();
                r.driver = Some(d);
                r.driver_new = true;
                self.last_vehicle.insert(sync.player_id, d.vehicle_id);
                r.passenger = None;
                let ped = p.ped;
                if ped != 0 && !in_vehicle(ped) && self.seat_remote(runner, ped, d.vehicle_id, 0) {
                    self.remote.entry(sync.player_id).or_default().warps_in += 1;
                }
            }
            IncomingSync::Passenger(ps) => {
                if Some(ps.player_id) == self.local_id {
                    return;
                }
                let Some(p) = players.get_mut(ps.player_id) else { return };
                let d = PassengerData::from_sync(&ps.sync);
                p.health = d.health;
                p.armour = d.armour;
                p.state = state::PASSENGER;
                p.last_sync = Some(now);
                let r = self.remote.entry(ps.player_id).or_default();
                r.passenger = Some(d);
                r.passenger_new = true;
                self.last_vehicle.insert(ps.player_id, d.vehicle_id);
                r.driver = None;
                let ped = p.ped;
                if ped != 0 && !in_vehicle(ped) && d.seat > 0 && self.seat_remote(runner, ped, d.vehicle_id, d.seat) {
                    self.remote.entry(ps.player_id).or_default().warps_in += 1;
                }
            }
            IncomingSync::Unoccupied(u) => self.apply_unoccupied(u.player_id, &u.sync),
            IncomingSync::Trailer(t) => self.apply_trailer(runner, t.player_id, &t.sync),
            _ => {}
        }
    }

    /// Driver/passenger seat warp (A 0x10014800 → A 0x100AC2D0). The weapon switch before it is M7.
    fn seat_remote(&mut self, runner: &mut Runner, ped: u32, id: u16, seat: u8) -> bool {
        let Some(v) = self.get(id).cloned() else { return false };
        if !seat_allowed(&v, seat) {
            return false;
        }
        let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return false };
        warp_into(runner, h, &v, seat);
        crate::log!("vehicles: remote ped {ped:#x} warped into vehicle {id} seat {seat}");
        true
    }

    /// After a 207 store: a ped still in a vehicle is pulled out unless entering/exiting (special action 3/4).
    pub fn after_foot_store(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get_mut(pid) else { return };
        let sa = p.foot.map(|f| f.special_action).unwrap_or(0);
        if let Some(r) = self.remote.get_mut(&pid) {
            r.driver = None;
            r.passenger = None;
        }
        // B 0x10017288…0x100172D4: not while the ped runs its leave-car task (RPC 154 exit animation).
        if p.ped != 0 && in_vehicle(p.ped) && sa != 3 && sa != 4 && !leaving_car(p.ped) {
            warp_out_own_position(runner, p.ped);
            if let Some(r) = self.remote.get_mut(&pid) {
                r.warps_out += 1;
            }
        }
    }

    /// Right after a remote ped was (re)created: seat it from the stored driver/passenger sync (TRACE-4 §5).
    pub fn after_stream_in(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get(pid) else { return };
        let ped = p.ped;
        if ped == 0 || in_vehicle(ped) {
            return;
        }
        let Some(r) = self.remote.get(&pid).copied() else { return };
        let ok = match (p.state, r.driver, r.passenger) {
            (state::DRIVER, Some(d), _) => self.seat_remote(runner, ped, d.vehicle_id, 0),
            (state::PASSENGER, _, Some(d)) if d.seat > 0 => self.seat_remote(runner, ped, d.vehicle_id, d.seat),
            _ => false,
        };
        if ok && let Some(r) = self.remote.get_mut(&pid) {
            r.warps_in += 1;
            r.driver_new = r.driver.is_some();
        }
    }

    pub fn forget_player(&mut self, pid: u16) {
        self.remote.remove(&pid);
        self.last_vehicle.remove(&pid);
    }

    /// samp's cache reset of a remote (A 0x10015FA0: RPC 166, stream-out) clears its vehicle id (+0x1E7).
    pub fn reset_remote_vehicle(&mut self, pid: u16) {
        self.last_vehicle.remove(&pid);
    }

    /// 209 apply (A 0x100158D0), immediately on receive.
    fn apply_unoccupied(&mut self, pid: u16, s: &samp_proto::sync::vehicle::UnoccupiedSync) {
        if !vm::unoccupied_valid(s) {
            return;
        }
        let Some(v) = self.pool.get_mut(&s.vehicle_id).filter(|v| v.ptr != 0) else { return };
        let ptr = v.ptr;
        let driver: u32 = rd(ptr + g::VEH_DRIVER);
        if driver != 0 && rd::<u32>(driver + g::PED_TYPE) == 0 {
            return;
        }
        let tractor: u32 = rd(ptr + g::VEH_TRACTOR);
        if tractor != 0 && rd::<u32>(tractor + g::VEH_DRIVER) != 0 {
            return;
        }
        v.last_unocc_sender = Some(pid);
        let m = matrix(ptr);
        if m == 0 {
            return;
        }
        let cur = v3(m + off::MATRIX_POS);
        let target = vm::vec3(s.position);
        let up = v3(m + 0x20);
        let rows = Rotation { right: vm::vec3(s.roll), forward: vm::vec3(s.direction), up };
        match vm::correct_unoccupied(cur, target, vm::vec3(s.velocity)) {
            VehCorrection::None => {}
            VehCorrection::Speed(speed) => {
                write_rows(m, &rows);
                put_v3(ptr + off::MOVE_SPEED, speed);
                put_v3(ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            }
            VehCorrection::Teleport => {
                write_rows(m, &rows);
                put_v3(m + off::MATRIX_POS, target);
                put_v3(ptr + off::MOVE_SPEED, vm::vec3(s.velocity));
                put_v3(ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            }
        }
        self.n.applied_209 += 1;
    }

    /// 210 apply (A 0x10015C90), immediately on receive.
    fn apply_trailer(&mut self, runner: &mut Runner, pid: u16, s: &TrailerSync) {
        let Some(cab_id) = self.remote.get(&pid).and_then(|r| r.driver).map(|d| d.vehicle_id) else { return };
        let (Some(cab), Some(t)) = (self.get(cab_id).cloned(), self.get(s.trailer_id).cloned()) else { return };
        if s.trailer_id == 0 || !vm::is_towable_list(t.model) {
            // The second towable test (A 0x100B7E10) is not specified: TODO-unknown.
            return;
        }
        if rd::<u32>(t.ptr + g::VEH_TRACTOR) != cab.ptr {
            runner.run(op::ATTACH_TRAILER_TO_CAB, &[Arg::Int(t.handle), Arg::Int(cab.handle)]);
        }
        if let Some(c) = self.pool.get_mut(&cab_id) {
            c.trailer = Some(s.trailer_id);
        }
        let m = matrix(t.ptr);
        if m == 0 {
            return;
        }
        let cur = v3(m + off::MATRIX_POS);
        let target = vm::vec3(s.position);
        let d = [0, 1, 2].map(|i| (target[i] - cur[i]).abs());
        let rows = rows_from_quat(normalise(s.rotation));
        if d.iter().all(|&x| x <= 0.5) {
            return;
        }
        if d[0] <= 6.0 && d[1] <= 6.0 && d[2] <= 3.0 {
            write_rows(m, &rows);
            put_v3(t.ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            let mut vel = v3(t.ptr + off::MOVE_SPEED);
            for i in 0..3 {
                if d[i] > 0.05 {
                    vel[i] += (target[i] - cur[i]) * 0.025;
                }
            }
            put_v3(t.ptr + off::MOVE_SPEED, vel);
        } else {
            write_rows(m, &rows);
            put_v3(m + off::MATRIX_POS, target);
            put_v3(t.ptr + off::MOVE_SPEED, vm::vec3(s.velocity));
            put_v3(t.ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
        }
    }

    // ---- per frame: remote occupants (A 0x100166B0, driver/passenger part) --------------------------------

    pub fn remote_tick(&mut self, runner: &mut Runner, players: &mut Players) {
        for p in players.iter_mut() {
            let Some(slot) = p.slot else { continue };
            if p.ped == 0 || !(p.state == state::DRIVER || p.state == state::PASSENGER) {
                continue;
            }
            let ped = p.ped;
            let seated = in_vehicle(ped);
            let Some(r) = self.remote.get_mut(&p.id) else { continue };
            if let (state::DRIVER, Some(d)) = (p.state, r.driver) {
                let Some(v) = self.pool.get(&d.vehicle_id).filter(|v| v.ptr != 0) else { continue };
                if !seated {
                    continue;
                }
                let ptr = v.ptr;
                let m = matrix(ptr);
                if r.driver_new && m != 0 {
                    r.driver_new = false;
                    if vm::TRAIN_SYNC_MODELS.contains(&v.model) {
                        let cur = v3(m + off::MATRIX_POS);
                        if vm::train_needs_teleport(cur, d.position) {
                            let a = [
                                Arg::Int(v.handle),
                                Arg::Float(d.position[0]),
                                Arg::Float(d.position[1]),
                                Arg::Float(d.position[2]),
                            ];
                            runner.run(op::SET_MISSION_TRAIN_COORDINATES, &a);
                        }
                        put_v3(ptr + off::MOVE_SPEED, d.velocity);
                        let sp = d.extra_f32();
                        if sp.abs() <= 100.0 {
                            poke(ptr + g::TRAIN_SPEED, sp);
                        }
                    } else {
                        put_v3(ptr + off::MOVE_SPEED, d.velocity);
                        let cur = v3(m + off::MATRIX_POS);
                        let moving = rd::<u32>(ptr + g::MOVING_LIST) != 0;
                        match vm::correct_driver(cur, d.position, d.velocity, moving, vm::z_threshold(v.kind)) {
                            VehCorrection::None => {}
                            VehCorrection::Speed(s) => {
                                r.corrections += 1;
                                put_v3(ptr + off::MOVE_SPEED, s);
                            }
                            VehCorrection::Teleport => {
                                r.teleports += 1;
                                put_v3(m + off::MATRIX_POS, d.position);
                                put_v3(ptr + off::MOVE_SPEED, d.velocity);
                            }
                        }
                    }
                    if v.model == vm::MODEL_HYDRA {
                        poke(ptr + g::HYDRA_NOZZLE, d.extra as u16);
                        poke(ptr + g::HYDRA_NOZZLE + 2, d.extra as u16);
                    } else if matches!(v.kind, Kind::Bike | Kind::Bmx) {
                        poke(ptr + g::BIKE_DESIRED_LEAN, d.extra_f32().clamp(-1.0, 1.0));
                    }
                    let s: u8 = rd(ptr + g::VEH_SIREN_BYTE);
                    poke(ptr + g::VEH_SIREN_BYTE, if d.siren { s | 0x80 } else { s & 0x7F });
                    if v.kind == Kind::Plane {
                        let up = rd::<f32>(ptr + g::PLANE_GEAR) != 0.0;
                        if up != d.landing_gear {
                            thiscall0(if d.landing_gear { g::PLANE_GEAR_UP } else { g::PLANE_GEAR_DOWN }, ptr);
                        }
                    }
                    poke(ptr + g::VEH_HEALTH, d.vehicle_health);
                    // Weapon, as on foot (remote-players-sync §2.1 one-shot apply, driver; M7).
                    crate::players::apply_weapon(runner, ped, slot, d.weapon);
                    if d.trailer_id == 0 || d.trailer_id >= vm::MAX_VEHICLES {
                        let tr: u32 = rd(ptr + g::VEH_TRAILER);
                        if tr != 0 && self.pool.values().any(|t| t.ptr == tr && t.id != 0) {
                            let id = v.id;
                            self.detach_trailer(runner, id);
                        }
                    }
                }
                let Some(v) = self.pool.get(&d.vehicle_id) else { continue };
                let ptr = v.ptr;
                let m = matrix(ptr);
                if m != 0 && !vm::TRAIN_SYNC_MODELS.contains(&v.model) {
                    put_v3(ptr + g::TURN_SPEED, vm::clamp_turn(v3(ptr + g::TURN_SPEED)));
                    let cur = quat_from_matrix(&rows_of(m));
                    let q = normalise(slerp(d.rotation, cur, remote::SLERP_KEEP));
                    write_rows(m, &rows_from_quat(q));
                }
                if p.npc && d.velocity == [0.0; 3] {
                    put_v3(ptr + off::MOVE_SPEED, [0.0; 3]);
                }
                let input = PadInput {
                    keys: d.keys,
                    left_right: d.left_right,
                    up_down: d.up_down,
                    in_vehicle: true,
                    jetpack: false,
                };
                crate::swap::with_table(|t| {
                    if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                        remote::apply_keys(&mut sh.pad, input);
                    }
                });
                poke(ptr + g::VEH_HORN, u32::from(d.keys & 2 != 0));
            } else if let (state::PASSENGER, Some(d)) = (p.state, r.passenger) {
                if r.passenger_new && seated {
                    // One-shot apply, passenger (§2.1 + RE-10): weapon, drive-by task, cuffs (M7).
                    crate::players::apply_weapon(runner, ped, slot, d.weapon);
                    remote_drive_by(runner, ped, slot, d.drive_by);
                }
                r.passenger_new = false;
                let input = PadInput { keys: d.keys, left_right: 0, up_down: 0, in_vehicle: true, jetpack: false };
                crate::swap::with_table(|t| {
                    if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                        remote::apply_keys(&mut sh.pad, input);
                    }
                });
            }
        }
    }

    // ---- combat (M7) --------------------------------------------------------------------------------------------

    /// Streamed vehicles as (GTA pointer, id) for the combat hooks.
    pub fn hook_vehicles(&self) -> Vec<(u32, u16)> {
        self.pool.values().filter(|v| v.ptr != 0 && v.id != 0).map(|v| (v.ptr, v.id)).collect()
    }

    /// samp's remote +0x1E7: the vehicle of the last stored driver / passenger sync. Written by both stores,
    /// kept after the remote leaves the vehicle, cleared only by the cache reset A 0x10015FA0 (RPC 166,
    /// stream-out, the reset branch of the remote tick) and the constructor.
    pub fn remote_vehicle(&self, pid: u16) -> Option<u16> {
        self.last_vehicle.get(&pid).copied().filter(|&v| v != 0)
    }

    pub fn ptr_of(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| v.ptr)
    }

    /// The in-car duplicate gate (shared timestamp of 200/211/210, also used for the in-car 203).
    pub fn dup_offer(&mut self, now: Instant, packet_id: u8, bytes: Vec<u8>) -> bool {
        self.dup.offer(now, packet_id, bytes)
    }

    /// A 200 was sent by the last `local_tick` (armed vehicles send a 203 after every 200).
    pub fn take_driver_sent(&mut self) -> Option<u16> {
        self.driver_sent.take()
    }

    /// Death while driving (A 0x10007080): one last 200, outside the interval.
    pub fn force_driver_send(&mut self, net: &mut NetClient, now: Instant) {
        let ped = local_ped();
        if ped == 0 || !in_vehicle(ped) {
            return;
        }
        let veh: u32 = rd(ped + g::PED_VEHICLE);
        if rd::<u32>(veh + g::VEH_DRIVER) != ped {
            return;
        }
        if let Some(id) = self.id_of_ptr(veh) {
            self.send_driver(net, ped, veh, id, now);
        }
    }

    /// The vehicle model of a pool id.
    pub fn model_of(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| v.model)
    }

    // ---- hook events and the local player -------------------------------------------------------------------

    /// Events queued by the enter/leave hooks since the last tick.
    pub fn hook_events(&mut self, runner: &mut Runner, net: &mut NetClient) {
        let events = vehhooks::with_shared(|s| std::mem::take(&mut s.events));
        let ped = local_ped();
        let h = game::ped_handle(&ProcessMemory, ped).ok().flatten();
        for ev in events {
            match ev {
                HookEvent::Enter { vehicle_id, parachute } => {
                    if parachute && let Some(h) = h {
                        runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
                    }
                    crate::log!("vehicles: enter request vehicle {vehicle_id} (driver)");
                    send(net, &rv::OnPlayerEnterVehicle { vehicle_id, passenger: 0 });
                    self.n.sent_26 += 1;
                    self.train_camera(runner, vehicle_id);
                }
                HookEvent::Exit { vehicle_id, driver, rc, train } => {
                    if driver {
                        self.last_driven = Some(vehicle_id);
                    }
                    if train {
                        // Warped inside the constructor hook (vehhooks::leave_filter); nothing else.
                    } else if !rc {
                        crate::log!("vehicles: exit vehicle {vehicle_id}");
                        send(net, &rv::OnPlayerExitVehicle { vehicle_id });
                        self.n.sent_154 += 1;
                    }
                }
            }
        }
    }

    fn train_camera(&mut self, runner: &mut Runner, id: u16) {
        if let Some(v) = self.get(id)
            && vm::is_rail(v.model)
        {
            runner.run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(v.handle), Arg::Int(3), Arg::Int(2)]);
            self.train_camera_since = Some(Instant::now());
        }
    }

    /// The local player's in-car part of the tick. Returns true when on-foot sync must not be sent (ped state
    /// not 1/12: driving, passenger, entering a seat or the exit animation; remote-players-sync §0.4).
    pub fn local_tick(&mut self, runner: &mut Runner, net: &mut NetClient, streamed: u32, now: Instant) -> bool {
        let ped = local_ped();
        if ped == 0 {
            return false;
        }
        if let Some(t) = self.train_camera_since
            && now.duration_since(t) > Duration::from_secs(5)
        {
            self.train_camera_since = None;
            if !in_vehicle(ped) {
                camera_behind(runner);
            }
        }
        let st: u32 = rd(ped + off::PED_STATE);
        let veh: u32 = rd(ped + g::PED_VEHICLE);
        let interval = vm::in_car_interval(self.in_car_rate.max(10), streamed);
        if (st == PEDSTATE_DRIVING || st == PEDSTATE_PASSENGER) && in_vehicle(ped) && veh != 0 {
            let Some(id) = self.id_of_ptr(veh) else { return true };
            let driver = rd::<u32>(veh + g::VEH_DRIVER) == ped && st == PEDSTATE_DRIVING;
            if driver {
                self.driving = Some(id);
                if self.driver_timer.due(now, interval) {
                    self.send_driver(net, ped, veh, id, now);
                }
                let d = damage_of(veh, kind_of(veh));
                if self.damage.update(id, d) {
                    send(
                        net,
                        &rv::SetVehicleDamageStatus {
                            vehicle_id: id,
                            panels: d.panels,
                            doors: d.doors,
                            lights: d.lights,
                            tyres: d.tyres,
                        },
                    );
                    self.n.sent_106 += 1;
                }
            } else {
                if let Some(prev) = self.driving.take() {
                    self.last_driven = Some(prev);
                }
                if self.passenger_timer.due(now, interval) {
                    self.send_passenger(net, ped, veh, id, now);
                }
            }
            return true;
        }
        if let Some(prev) = self.driving.take() {
            self.last_driven = Some(prev);
        }
        self.last_rotation = None;
        if st == PEDSTATE_IDLE || st == PEDSTATE_SNIPER {
            if !in_vehicle(ped) {
                self.passenger_poll(runner, net, ped);
            }
            return false;
        }
        true
    }

    fn send_driver(&mut self, net: &mut NetClient, ped: u32, veh: u32, id: u16, now: Instant) {
        let m = matrix(veh);
        if m == 0 {
            return;
        }
        let Ok(sample) = game::read_local_sample(&ProcessMemory, ped) else { return };
        let kind = kind_of(veh);
        let model = u32::from(rd::<u16>(veh + off::ENTITY_MODEL));
        let trailer: u32 = rd(veh + g::VEH_TRAILER);
        let extra = if model == vm::MODEL_HYDRA {
            u32::from(rd::<u16>(veh + g::HYDRA_NOZZLE))
        } else if vm::TRAIN_SYNC_MODELS.contains(&model) {
            rd::<f32>(veh + g::TRAIN_SPEED).to_bits()
        } else if matches!(kind, Kind::Bike | Kind::Bmx) {
            rd::<f32>(veh + g::BIKE_LEAN).to_bits()
        } else {
            0
        };
        let s = vm::DriverSample {
            vehicle_id: id,
            left_right: sample.pad.left_stick_x,
            up_down: sample.pad.left_stick_y,
            keys: footsync::key_word(&sample.pad, sample.key_ctx),
            additional_key: footsync::additional_key(&sample.pad),
            rotation: rows_of(m),
            position: v3(m + off::MATRIX_POS),
            velocity: v3(veh + off::MOVE_SPEED),
            vehicle_health: rd(veh + g::VEH_HEALTH),
            health: sample.health,
            armour: sample.armour,
            weapon: sample.weapon,
            siren: rd::<u8>(veh + g::VEH_SIREN_BYTE) & 0x80 != 0,
            landing_gear: kind == Kind::Plane && rd::<f32>(veh + g::PLANE_GEAR) != 0.0,
            trailer_id: self.id_of_ptr(trailer).unwrap_or(0),
            extra,
        };
        let p = vm::build_driver_sync(&s, self.last_rotation);
        if self.dup.offer(now, samp_proto::sync::ids::VEHICLE_SYNC, body(&p)) {
            self.last_rotation = Some(p.rotation);
            net.send_sync(&p);
            self.n.sent_200 += 1;
            self.driver_sent = Some(id);
        }
        // 210 goes out on every in-car tick while towing, even when the 200 was suppressed as a duplicate, and it
        // refreshes the shared send timestamp (TRACE-5: stationary 1 × 200 vs 119 × 210).
        if s.trailer_id != 0
            && let Some(t) = self.get(s.trailer_id)
        {
            let tm = matrix(t.ptr);
            if tm != 0 {
                let ts = TrailerSync {
                    trailer_id: s.trailer_id,
                    position: vm::to_vec3(v3(tm + off::MATRIX_POS)),
                    rotation: quat_from_matrix(&rows_of(tm)),
                    velocity: vm::to_vec3(v3(t.ptr + off::MOVE_SPEED)),
                    turn_velocity: vm::to_vec3(v3(t.ptr + g::TURN_SPEED)),
                };
                self.dup.touch(now);
                net.send_sync(&ts);
                self.n.sent_210 += 1;
            }
        }
    }

    fn send_passenger(&mut self, net: &mut NetClient, ped: u32, veh: u32, id: u16, now: Instant) {
        let Ok(sample) = game::read_local_sample(&ProcessMemory, ped) else { return };
        let seat =
            (0..8u32).find(|&i| rd::<u32>(veh + g::VEH_PASSENGERS + 4 * i) == ped).map(|i| i as u8 + 1).unwrap_or(0);
        let mut p = vm::build_passenger_sync(
            id,
            seat,
            sample.weapon,
            footsync::additional_key(&sample.pad),
            sample.health,
            sample.armour,
            sample.pad.left_stick_x,
            sample.pad.left_stick_y,
            footsync::key_word(&sample.pad, sample.key_ctx),
            sample.position,
        );
        // Drive-by bit (byte 2 bit 6): the local drive-by task is active (TRACE-7: set from the start frame on).
        p.drive_by = self.local_drive_by;
        p.cuffed = self.local_cuffed;
        if self.dup.offer(now, samp_proto::sync::ids::PASSENGER_SYNC, body(&p)) {
            net.send_sync(&p);
            self.n.sent_211 += 1;
        }
    }

    /// Passenger-enter poll (A 0x10006FE0): the GROUP_CONTROL_FWD (DPadUp) press edge.
    fn passenger_poll(&mut self, runner: &mut Runner, net: &mut NetClient, ped: u32) {
        let new: i16 = rd(g::PAD_DPAD_UP_NEW);
        let old: i16 = rd(g::PAD_DPAD_UP_OLD);
        if new == 0 || old != 0 {
            return;
        }
        let Some(me) = pos_of(ped) else { return };
        let closest = self
            .pool
            .values()
            .filter(|v| v.ptr != 0 && v.id != 0)
            .filter_map(|v| pos_of(v.ptr).map(|p| (vm::dist3d(p, me), v.id, v.handle, v.model)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((d, id, vh, _model)) = closest else { return };
        if d >= vm::PASSENGER_ENTER_RANGE {
            return;
        }
        let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
        // samp reads the weapon with GET_CURRENT_CHAR_WEAPON (A 0x100abc90; efx RPC 70 diff).
        runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Local(0)]);
        if runner.local_i32(0) as u32 == u32::from(vm::WEAPON_PARACHUTE) {
            runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
        }
        // Model 570 warps instead of animating (vehicles.md 26); the seat it uses is not specified: TODO-unknown.
        runner.run(
            op::TASK_ENTER_CAR_AS_PASSENGER,
            &[Arg::Int(h), Arg::Int(vh), Arg::Int(vm::ENTER_TASK_MS), Arg::Int(-1)],
        );
        crate::log!("vehicles: enter request vehicle {id} (passenger)");
        send(net, &rv::OnPlayerEnterVehicle { vehicle_id: id, passenger: 1 });
        self.n.sent_26 += 1;
        self.train_camera(runner, id);
    }

    // ---- per frame: the vehicle pool (A 0x1001F3C0) ----------------------------------------------------------

    /// (id, CVehicle*, plate text) of every vehicle in pool order (the plate pass, `plates.rs`).
    pub fn plate_list(&self) -> Vec<(u16, u32, Vec<u8>)> {
        self.pool.iter().map(|(id, v)| (*id, v.ptr, v.plate.clone())).collect()
    }

    pub fn pool_tick(&mut self, runner: &mut Runner, net: &mut NetClient, players: &Players, now: Instant) {
        self.frame += 1;
        let local = local_ped();
        // "Last driven": also when the local driver dies in the car (the sync tick does not run while dead).
        if let Some(id) = self.driving
            && self.get(id).is_none_or(|v| local == 0 || rd::<u32>(v.ptr + g::VEH_DRIVER) != local || dead(local))
        {
            self.driving = None;
            self.last_driven = Some(id);
        }
        let local_pos = if local != 0 { pos_of(local) } else { None };
        let remote_peds: Vec<(u16, u32)> = players.iter().filter(|p| p.ped != 0).map(|p| (p.id, p.ped)).collect();
        // The original runs this pool tick on every 4th frame only (TRACE-4 trace: the proofs/tyres pair,
        // IS_CAR_IN_WATER and the blip calls of every car are exactly 4 frames apart, 1123 of 1124 gaps; a car
        // created by RPC 164 is first processed on the next such frame).
        if !self.frame.is_multiple_of(vm::POOL_TICK_FRAMES) {
            self.publish_hook_state(players);
            return;
        }
        let mut sends = 0;
        let ids: Vec<u16> = self.pool.keys().copied().collect();
        for id in ids {
            let Some(v) = self.pool.get(&id) else { continue };
            if v.carriage || v.ptr == 0 {
                continue;
            }
            let ptr = vehicle_at(v.handle);
            if ptr == 0 || rd::<u32>(ptr) == g::VTBL_CPLACEABLE {
                continue;
            }
            if let Some(t) = v.wasted_at
                && now.duration_since(t) >= vm::WASTED_DELETE
            {
                self.delete(runner, id);
                continue;
            }
            let h = v.handle;
            let driver: u32 = rd(ptr + g::VEH_DRIVER);
            let local_drives = local != 0 && driver == local;
            let local_passenger = local != 0 && !local_drives && is_passenger(ptr, local);
            // Damage authority: proofs + tyres pair every frame (TRACE-4 §2); none at all for the car the local
            // ped sits in as a passenger (TRACE-5 MR-a).
            if local_passenger {
                // no pair
            } else if local_drives {
                runner.run(
                    op::SET_CAR_PROOFS,
                    &[Arg::Int(h), Arg::Int(0), Arg::Int(0), Arg::Int(0), Arg::Int(0), Arg::Int(0)],
                );
                runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(h), Arg::Int(1)]);
            } else {
                runner.run(
                    op::SET_CAR_PROOFS,
                    &[Arg::Int(h), Arg::Int(1), Arg::Int(1), Arg::Int(1), Arg::Int(1), Arg::Int(1)],
                );
                runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(h), Arg::Int(0)]);
            }
            let pos = pos_of(ptr);
            let dist = match (pos, local_pos) {
                (Some(a), Some(b)) => vm::dist3d(a, b),
                _ => f32::MAX,
            };
            let kind = kind_of(ptr);
            if rd::<f32>(ptr + g::VEH_HEALTH) == 0.0 {
                if v.wasted_at.is_none() {
                    self.vehicle_death(net, id);
                    if let Some(v) = self.pool.get_mut(&id) {
                        v.wasted_at = Some(now);
                    }
                }
                continue;
            }
            // Sunk check, not for boats, within 200 units.
            if kind != Kind::Boat && dist <= vm::NEAR_RANGE && runner.run(op::IS_CAR_IN_WATER, &[Arg::Int(h)]) {
                self.vehicle_death(net, id);
                continue;
            }
            // Unoccupied sync.
            if sends < vm::UNOCC_SENDS_PER_TICK
                && let Some(p) = pos
                && self.unoccupied_send(net, id, ptr, p, dist, &remote_peds, now)
            {
                sends += 1;
            }
            self.engine_lights(id, ptr, driver);
            let Some(v) = self.pool.get(&id) else { continue };
            if let Some((c1, c2)) = v.colours
                && !v.in_mod_shop
            {
                poke(ptr + g::VEH_PRIMARY_COLOUR, c1);
                poke(ptr + g::VEH_SECONDARY_COLOUR, c2);
            }
            self.blips(runner, id, driver != 0 || local_passenger, dist);
        }
        self.publish_hook_state(players);
    }

    fn vehicle_death(&mut self, net: &mut NetClient, id: u16) {
        if self.last_driven == Some(id) {
            self.last_driven = None;
            send(net, &rv::VehicleDeath { vehicle_id: id });
            self.n.sent_136 += 1;
            crate::log!("vehicles: vehicle {id} death reported");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn unoccupied_send(
        &mut self,
        net: &mut NetClient,
        id: u16,
        ptr: u32,
        pos: [f32; 3],
        local_dist: f32,
        remote_peds: &[(u16, u32)],
        now: Instant,
    ) -> bool {
        let Some(v) = self.pool.get(&id) else { return false };
        if vm::is_rail(v.model) || v.last_unocc_send.is_some_and(|t| now.duration_since(t) <= vm::UNOCC_SEND_INTERVAL) {
            return false;
        }
        let tractor: u32 = rd(ptr + g::VEH_TRACTOR);
        let towed = tractor != 0 && rd::<u32>(tractor + g::VEH_DRIVER) != 0;
        let driver: u32 = rd(ptr + g::VEH_DRIVER);
        let local = local_ped();
        let speed = v3(ptr + off::MOVE_SPEED);
        // Not "stopped" (A 0x1009FE40): stopped = in the moving list with a zero move speed.
        let moving = rd::<u32>(ptr + g::MOVING_LIST) == 0 || speed != [0.0; 3];
        // Only player peds (`m_nPedType` +0x598 == 0, local or remote) count as passengers; AI peds do not.
        let first = (0..8u32).find_map(|i| {
            let p: u32 = rd(ptr + g::VEH_PASSENGERS + 4 * i);
            (p != 0 && rd::<u32>(p + g::PED_TYPE) == 0).then_some((p == local, i as u8))
        });
        let has_driver = driver != 0 && in_vehicle(driver);
        let closest = first.is_none() && {
            // Peds count while in the world (A 0x1009F430: not a deleted placeable, in the moving list).
            let dist_of = |ped: u32| {
                (ped != 0 && rd::<u32>(ped) != g::VTBL_CPLACEABLE && rd::<u32>(ped + g::MOVING_LIST) != 0)
                    .then(|| pos_of(ped).map(|p| vm::dist3d(p, pos)))
                    .flatten()
            };
            let local_id = self.local_id.unwrap_or(0);
            let mut all: Vec<(u16, Option<f32>)> =
                remote_peds.iter().filter(|(pid, _)| *pid != local_id).map(|&(pid, ped)| (pid, dist_of(ped))).collect();
            all.push((local_id, dist_of(local)));
            all.sort_by_key(|e| e.0);
            vm::local_is_closest(local_id, all)
        };
        let Some(seat) = vm::unoccupied_seat(has_driver, towed, moving, local_dist, first, closest) else {
            return false;
        };
        let m = matrix(ptr);
        if m == 0 {
            return false;
        }
        let u = vm::build_unoccupied_sync(
            id,
            seat,
            &rows_of(m),
            pos,
            speed,
            v3(ptr + g::TURN_SPEED),
            rd(ptr + g::VEH_HEALTH),
        );
        net.send_sync(&u);
        self.n.sent_209 += 1;
        if let Some(v) = self.pool.get_mut(&id) {
            v.last_unocc_send = Some(now);
            v.last_unocc_sender = self.local_id;
        }
        true
    }

    fn engine_lights(&mut self, id: u16, ptr: u32, driver: u32) {
        let Some(v) = self.pool.get(&id) else { return };
        let flags: u8 = rd(ptr + g::VEH_FLAGS);
        let engine = vm::engine_wanted(self.manual, v.engine, driver != 0 && in_vehicle(driver));
        let mut f = if engine { flags | 0x10 } else { flags & !0x10 };
        if let Some(on) = vm::lights_wanted(self.manual, v.lights) {
            f = if on { f | 0x40 } else { f & !0x40 };
            let l: u8 = rd(ptr + g::VEH_LIGHTS_FLAGS);
            poke(ptr + g::VEH_LIGHTS_FLAGS, if on { l & !0x08 } else { l | 0x08 });
        }
        if f != flags {
            poke(ptr + g::VEH_FLAGS, f);
        }
    }

    /// Objective / unoccupied radar blips (A 0x100B7720). "Occupied" = a driver ped is set or the local ped sits in it
    /// as a passenger (TRACE-4: the blip goes when the local driver is seated and comes back after the exit
    /// animation; TRACE-5 MR-b: removed once after the local ped is seated as a passenger). Remote passengers in a
    /// driverless car are not measured.
    fn blips(&mut self, runner: &mut Runner, id: u16, occupied: bool, dist: f32) {
        let Some(v) = self.pool.get_mut(&id) else { return };
        let h = v.handle;
        if v.objective {
            if let Some(b) = v.blip.take() {
                runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
            }
            if v.objective_blip.is_none() {
                runner.run(op::ADD_BLIP_FOR_CAR_OLD, &[Arg::Int(h), Arg::Int(1), Arg::Int(3), Arg::Local(0)]);
                let b = runner.local_i32(0);
                runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(b), Arg::Int(vm::BLIP_COLOUR_OBJECTIVE)]);
                runner.run(op::CHANGE_BLIP_SCALE, &[Arg::Int(b), Arg::Int(3)]);
                v.objective_blip = Some(b);
            }
            return;
        }
        if let Some(b) = v.objective_blip.take() {
            runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
        }
        let want = !occupied && dist <= vm::NEAR_RANGE;
        match (want, v.blip) {
            (true, None) => {
                runner.run(op::ADD_BLIP_FOR_CAR_OLD, &[Arg::Int(h), Arg::Int(1), Arg::Int(2), Arg::Local(0)]);
                let b = runner.local_i32(0);
                runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(b), Arg::Int(vm::BLIP_COLOUR_UNOCCUPIED)]);
                v.blip = Some(b);
            }
            (false, Some(b)) => {
                runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
                v.blip = None;
            }
            _ => {}
        }
    }

    /// Mirrors what the GTA hooks need (ptr → id, lock, siren, trailer, NPC peds).
    fn publish_hook_state(&self, players: &Players) {
        let by_ptr: HashMap<u32, VehInfo> = self
            .pool
            .values()
            .filter(|v| v.ptr != 0)
            .map(|v| {
                (
                    v.ptr,
                    VehInfo {
                        id: v.id,
                        model: v.model,
                        locked: v.locked,
                        siren: v.siren,
                        has_trailer: v.trailer.is_some(),
                    },
                )
            })
            .collect();
        let npc: Vec<u32> = players.iter().filter(|p| p.npc && p.ped != 0).map(|p| p.ped).collect();
        vehhooks::with_shared(|s| {
            s.by_ptr = by_ptr;
            s.npc_peds = npc;
        });
    }

    /// Session ended: every vehicle goes.
    pub fn clear(&mut self, runner: &mut Runner) {
        let ids: Vec<u16> = self.pool.keys().copied().collect();
        for id in ids {
            self.delete(runner, id);
        }
        self.remote.clear();
        vehhooks::with_shared(|s| s.by_ptr.clear());
    }

    // ---- control channel ------------------------------------------------------------------------------------

    /// `vehicles` reply: a header line, then one line per vehicle.
    pub fn status(&self, players: &Players) -> String {
        let local = local_ped();
        let [a, b, c, d] = vehhooks::call_counts();
        let n = &self.n;
        let mut out = format!(
            "vehicles count={} process_control_calls={a}/{b}/{c}/{d} sent_200={} sent_210={} sent_211={} sent_209={} \
             sent_26={} sent_154={} sent_106={} sent_136={} applied_209={} last_driven={}",
            self.pool.len(),
            n.sent_200,
            n.sent_210,
            n.sent_211,
            n.sent_209,
            n.sent_26,
            n.sent_154,
            n.sent_106,
            n.sent_136,
            n.applied_209,
            self.last_driven.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
        );
        for v in self.pool.values() {
            let ptr = if v.ptr != 0 { vehicle_at(v.handle) } else { 0 };
            let (pos, heading, health, driver, occ) = if ptr != 0 {
                let m = matrix(ptr);
                let driver: u32 = rd(ptr + g::VEH_DRIVER);
                let drv = if driver == 0 {
                    "-".to_string()
                } else if driver == local {
                    "local".to_string()
                } else {
                    players
                        .iter()
                        .find(|p| p.ped == driver)
                        .map(|p| p.id.to_string())
                        .unwrap_or_else(|| format!("ped:{driver:#x}"))
                };
                let occ = if local != 0 && driver == local {
                    "driver"
                } else if local != 0 && (0..8u32).any(|i| rd::<u32>(ptr + g::VEH_PASSENGERS + 4 * i) == local) {
                    "passenger"
                } else {
                    "none"
                };
                (
                    (m != 0).then(|| v3(m + off::MATRIX_POS)),
                    (m != 0).then(|| vm::heading_deg(v3(m + 0x10))),
                    Some(rd::<f32>(ptr + g::VEH_HEALTH)),
                    drv,
                    occ,
                )
            } else {
                (None, None, None, "-".to_string(), "none")
            };
            out.push('\n');
            out.push_str(&vm::format_vehicle(v.id, v.model, v.handle, ptr, pos, heading, health, &driver, occ));
        }
        out
    }

    /// Extra fields of a `remote` line: the GTA vehicle/seat of the ped and the synced vehicle.
    pub fn remote_fields(&self, p: &remote::RemotePlayer) -> String {
        let (veh, seat) = if p.ped != 0 && in_vehicle(p.ped) {
            let ptr: u32 = rd(p.ped + g::PED_VEHICLE);
            let id = self.id_of_ptr(ptr).map(|i| i.to_string()).unwrap_or_else(|| "?".into());
            let seat = if rd::<u32>(ptr + g::VEH_DRIVER) == p.ped {
                "0".to_string()
            } else {
                (0..8u32)
                    .find(|&i| rd::<u32>(ptr + g::VEH_PASSENGERS + 4 * i) == p.ped)
                    .map(|i| (i + 1).to_string())
                    .unwrap_or_else(|| "?".into())
            };
            (id, seat)
        } else {
            ("-".into(), "-".into())
        };
        let r = self.remote.get(&p.id).copied().unwrap_or_default();
        let synced = r
            .driver
            .map(|d| format!("{}:{:.2},{:.2},{:.2}", d.vehicle_id, d.position[0], d.position[1], d.position[2]))
            .or_else(|| r.passenger.map(|d| format!("{}:seat{}", d.vehicle_id, d.seat)))
            .unwrap_or_else(|| "-".into());
        format!(
            " vehicle={veh} seat={seat} veh_sync={synced} veh_corrections={} veh_teleports={} warps_in={} warps_out={}",
            r.corrections, r.teleports, r.warps_in, r.warps_out
        )
    }
}

fn body<S: SyncPacket>(p: &S) -> Vec<u8> {
    p.to_bits().as_bytes().to_vec()
}

fn within_relay_range(ped: u32) -> bool {
    match (pos_of(ped), pos_of(local_ped())) {
        (Some(a), Some(b)) => vm::dist3d(a, b) < vm::RELAY_RANGE,
        _ => false,
    }
}

fn clear_keys(p: &mut remote::RemotePlayer) {
    if let Some(f) = p.foot.as_mut() {
        f.keys = 0;
        f.left_right = 0;
        f.up_down = 0;
    }
    if let Some(slot) = p.slot {
        crate::swap::with_table(|t| {
            if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                remote::apply_keys(&mut sh.pad, PadInput::default());
            }
        });
    }
}

/// Seat checks of A 0x100AC2D0: vehicle health 0, seat beyond the passenger count (automobile/bike kinds),
/// a deleted entity, or an occupied driver seat.
fn seat_allowed(v: &Veh, seat: u8) -> bool {
    let ptr = v.ptr;
    if rd::<u32>(ptr) == g::VTBL_CPLACEABLE || rd::<f32>(ptr + g::VEH_HEALTH) == 0.0 {
        return false;
    }
    if matches!(v.kind, Kind::Automobile | Kind::Bike)
        && u32::from(seat) > u32::from(rd::<u8>(ptr + g::VEH_MAX_PASSENGERS))
    {
        return false;
    }
    !(seat == 0 && rd::<u32>(ptr + g::VEH_DRIVER) != 0)
}

fn warp_into(runner: &mut Runner, ped_handle: i32, v: &Veh, seat: u8) {
    if seat == 0 {
        runner.run(op::WARP_CHAR_INTO_CAR, &[Arg::Int(ped_handle), Arg::Int(v.handle)]);
    } else {
        runner.run(
            op::WARP_CHAR_INTO_CAR_AS_PASSENGER,
            &[Arg::Int(ped_handle), Arg::Int(v.handle), Arg::Int(i32::from(seat) - 1)],
        );
    }
}

/// WARP_CHAR_FROM_CAR_TO_COORD to the ped's own current position (TRACE-4 §3).
fn warp_out_own_position(runner: &mut Runner, ped: u32) {
    let (Some(p), Some(h)) = (pos_of(ped), game::ped_handle(&ProcessMemory, ped).ok().flatten()) else { return };
    runner.run(op::WARP_CHAR_FROM_CAR_TO_COORD, &[Arg::Int(h), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2])]);
    crate::log!("vehicles: ped {ped:#x} warped out to its own position");
}

/// samp's camera reset (A 0x1009D7A0): 0930(0), 0925, 0373, 02EB.
fn camera_behind(runner: &mut Runner) {
    runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(0)]);
    runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
    runner.run(op::SET_CAMERA_BEHIND_PLAYER, &[]);
    runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
}

/// Model load before CREATE_CAR (TRACE-4 §1.1): HAS_MODEL_LOADED; if missing REQUEST_MODEL + LOAD_ALL_MODELS_NOW and
/// busy-wait on HAS_MODEL_LOADED with Sleep(0). samp waits without limit; we stop after 10 s and log.
fn busy_load(runner: &mut Runner, model: i32) {
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        return;
    }
    runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
    runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
    let t0 = Instant::now();
    while !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        if t0.elapsed() > Duration::from_secs(10) {
            crate::log!("vehicles: model {model} still not loaded after 10 s");
            return;
        }
        std::thread::sleep(Duration::ZERO);
    }
}

/// Add-component (vehicle method B 0x100B7FD0, used by RPC 65 and SCM event 2): not loaded → `REQUEST_MODEL`,
/// `LOAD_ALL_MODELS_NOW`, `REQUEST_VEHICLE_MOD`, then up to 20 polls with `Sleep(1)`; if loaded, `ADD_VEHICLE_MOD`.
fn add_component(runner: &mut Runner, handle: i32, component: i32) {
    if !runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
        runner.run(op::REQUEST_MODEL, &[Arg::Int(component)]);
        runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
        runner.run(op::REQUEST_VEHICLE_MOD, &[Arg::Int(component)]);
        for _ in 0..20 {
            if runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    if runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
        runner.run(op::ADD_VEHICLE_MOD, &[Arg::Int(handle), Arg::Int(component), Arg::Local(0)]);
    }
}

/// `CPool::GetRef` for a vehicle pointer (A 0x100B3C40 → GTA 0x424160).
fn pool_ref(ptr: u32) -> i32 {
    let pool: u32 = rd(g::VEHICLE_POOL);
    if pool == 0 {
        return -1;
    }
    // SAFETY: GTA's own pool function (thiscall) on the live vehicle pool.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(0x424160usize);
        f(pool, ptr)
    }
}

/// Entity teleport virtual (vtable +0x38: x, y, z, false).
fn teleport(ent: u32, p: [f32; 3]) {
    let vt: u32 = rd(ent);
    if vt == g::VTBL_CPLACEABLE {
        return;
    }
    let target: u32 = rd(vt + off::VT_TELEPORT);
    // SAFETY: the entity's own virtual (thiscall, CVector by value, bool).
    unsafe {
        let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
        f(ent, p[0], p[1], p[2], 0);
    }
}

/// Damage setter (A 0x100B7B00): automobiles and planes only.
fn apply_damage(ptr: u32, kind: Kind, panels: u32, doors: u32, lights: u8) {
    if !matches!(kind, Kind::Automobile | Kind::Plane) {
        return;
    }
    if panels == 0 && doors == 0 && lights == 0 {
        let has = rd::<u32>(ptr + g::DMG_PANELS) != 0
            || rd::<u32>(ptr + g::DMG_DOORS) != 0
            || rd::<u8>(ptr + g::DMG_LIGHTS) != 0;
        if has {
            thiscall0(g::AUTOMOBILE_FIX, ptr);
        }
        return;
    }
    poke(ptr + g::DMG_PANELS, panels);
    poke(ptr + g::DMG_DOORS, doors);
    poke(ptr + g::DMG_LIGHTS, u32::from(lights));
    thiscall0(g::AUTOMOBILE_SETUP_DAMAGE, ptr);
}

/// Tyres (A 0x100B7980).
fn apply_tyres(ptr: u32, kind: Kind, tyres: u8) {
    match kind {
        Kind::Automobile => {
            for (bit, off) in [(0, 0x5A8u32), (1, 0x5A7), (2, 0x5A6), (3, 0x5A5)] {
                poke(ptr + off, u8::from(tyres & (1 << bit) != 0));
            }
        }
        Kind::Bike => {
            poke(ptr + g::BIKE_TYRE_REAR, u8::from(tyres & 1 != 0));
            poke(ptr + g::BIKE_TYRE_FRONT, u8::from(tyres & 2 != 0));
        }
        _ => {}
    }
}

fn damage_of(ptr: u32, kind: Kind) -> vm::DamageStatus {
    let auto = matches!(kind, Kind::Automobile | Kind::Plane);
    vm::DamageStatus {
        panels: if auto { rd(ptr + g::DMG_PANELS) } else { 0 },
        doors: if auto { rd(ptr + g::DMG_DOORS) } else { 0 },
        lights: if kind == Kind::Automobile { rd(ptr + g::DMG_LIGHTS) } else { 0 },
        tyres: match kind {
            Kind::Automobile => {
                vm::car_tyre_bits([rd(ptr + g::DMG_WHEELS), rd(ptr + 0x5A6), rd(ptr + 0x5A7), rd(ptr + 0x5A8)])
            }
            Kind::Bike => {
                (u8::from(rd::<u8>(ptr + g::BIKE_TYRE_FRONT) != 0) << 1)
                    | u8::from(rd::<u8>(ptr + g::BIKE_TYRE_REAR) != 0)
            }
            _ => 0,
        },
    }
}

fn is_passenger(veh: u32, ped: u32) -> bool {
    (0..8u32).any(|i| rd::<u32>(veh + g::VEH_PASSENGERS + 4 * i) == ped)
}

/// Ped state DIE (54) / DEAD (55) or no health.
fn dead(ped: u32) -> bool {
    let st: u32 = rd(ped + off::PED_STATE);
    st == 54 || st == 55 || rd::<f32>(ped + off::PED_HEALTH) <= 0.0
}

/// `CPedIntelligence` task slot samp reads for the drive-by task (RE-10: intelligence +0x10).
const INTEL_DRIVE_BY_SLOT: u32 = 0x10;
/// `TASK_SIMPLE_GANG_DRIVEBY` task type and its deleting destructor.
const TASK_GANG_DRIVEBY: u32 = 0x3FE;
const GANG_DRIVEBY_DELETING_DTOR: u32 = 0x6275E0;

/// The task in the intelligence slot +0x10 (the task manager's primary slot 3) and its type, if any.
fn intel_slot_task(ped: u32) -> Option<(u32, u32)> {
    let intel: u32 = rd(ped + off::PED_INTELLIGENCE);
    if intel == 0 {
        return None;
    }
    let task: u32 = rd(intel + INTEL_DRIVE_BY_SLOT);
    if task == 0 {
        return None;
    }
    let vt: u32 = rd(task);
    if !(0x80_0000..=0x90_0000).contains(&vt) {
        return None;
    }
    let get_type: u32 = rd(vt + 0x10);
    // SAFETY: the task's own GetTaskType virtual (thiscall), main thread.
    let kind = unsafe {
        let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(get_type as usize);
        f(task)
    };
    Some((task, kind))
}

/// The drive-by task is active (task type 0x3FE in the intelligence slot +0x10).
pub(crate) fn drive_by_task(ped: u32) -> u32 {
    match intel_slot_task(ped) {
        Some((task, TASK_GANG_DRIVEBY)) => task,
        _ => 0,
    }
}

/// `TASK_COMPLEX_LEAVE_CAR`.
const TASK_LEAVE_CAR: u32 = 0x2C0;

/// The ped is running the leave-car task (B 0x100AC650: intelligence slot +0x10, type 0x2C0).
fn leaving_car(ped: u32) -> bool {
    matches!(intel_slot_task(ped), Some((_, TASK_LEAVE_CAR)))
}

/// Stops the drive-by task through its deleting destructor and clears the slot (A 0x100AD0C0).
pub(crate) fn stop_drive_by(ped: u32) {
    let task = drive_by_task(ped);
    if task == 0 {
        return;
    }
    // SAFETY: the task's deleting destructor (thiscall), main thread.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(GANG_DRIVEBY_DELETING_DTOR as usize);
        f(task, 1);
    }
    let intel: u32 = rd(ped + off::PED_INTELLIGENCE);
    poke(intel + INTEL_DRIVE_BY_SLOT, 0u32);
}

/// Starts the drive-by task (A 0x100ACFD0): in a vehicle only; with the parachute the weapon goes to fists and no
/// task starts; otherwise the current weapon is armed again and TASK_DRIVE_BY(ped, −1, −1, 0, 0, 0, 300, 8, 1, 20)
/// runs (TRACE-7: same parameters for the local and the remote passenger).
pub(crate) fn start_drive_by(runner: &mut Runner, ped: u32, arm: impl FnOnce(&mut Runner, u8)) -> bool {
    if !in_vehicle(ped) || rd::<u32>(ped + g::PED_VEHICLE) == 0 {
        return false;
    }
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return false };
    let w = current_weapon(ped) as u8;
    if w == vm::WEAPON_PARACHUTE {
        arm(runner, 0);
        return false;
    }
    arm(runner, w);
    let f = Arg::Float;
    runner.run(
        op::TASK_DRIVE_BY,
        &[Arg::Int(h), Arg::Int(-1), Arg::Int(-1), f(0.0), f(0.0), f(0.0), f(300.0), Arg::Int(8), Arg::Int(1), Arg::Int(20)],
    );
    true
}

/// Remote passenger drive-by bit (remote-players-sync RE-10): start when set and not active, stop when clear.
fn remote_drive_by(runner: &mut Runner, ped: u32, slot: u8, on: bool) {
    let active = drive_by_task(ped) != 0;
    if on && !active {
        start_drive_by(runner, ped, |r, w| crate::players::set_weapon(r, ped, slot, w));
    } else if !on && active {
        stop_drive_by(ped);
    }
}
