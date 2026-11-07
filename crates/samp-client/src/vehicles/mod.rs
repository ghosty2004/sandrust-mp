//! Vehicles inside the game (Windows only): [`Vehicles`] owns the samp vehicle pool and handles the vehicle RPCs
//! ([`Vehicles::on_rpc`]), remote drivers and passengers ([`Vehicles::on_sync`], [`Vehicles::remote_tick`]), the
//! per-frame pool tick (damage authority, deaths, unoccupied sync, engine/lights, blips) and the local in-car
//! senders ([`Vehicles::local_tick`]). Runs on the game's main thread from the client tick. Remote weapons are
//! applied through `players.rs`, plate textures live in `plates.rs`; the pure rules are in `vehicle.rs`.
//! Specs: docs/re/subsystems/vehicles.md, docs/re/subsystems/remote-control-vehicles.md and
//! docs/re/subsystems/remote-players-sync.md; open differences are listed in docs/phase3/m5-notes.md.

mod damage;
mod debug;
mod driveby;
mod enter_exit;
mod local_sync;
mod mem;
mod modshop;
mod pool_tick;
mod remote_apply;
mod rpc;
mod stream;
mod trailers;

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

use damage::*;
use driveby::*;
use enter_exit::*;
use mem::*;
use modshop::*;
use trailers::*;

pub(crate) use driveby::{start_drive_by, stop_drive_by};
pub use mem::current_weapon;
pub(crate) use mem::pool_ref;
pub use stream::{create_at, create_detached, delete_handle, samp_type};

/// "Car Jacked" game text, shown for 1000 ms in style 5.
pub const CAR_JACKED_TEXT: &str = "~r~Car Jacked~w~!";
const CAR_JACKED_MS: u32 = 1000;
const CAR_JACKED_STYLE: u32 = 5;

/// GTA addresses / fields (names via gta-reversed / plugin-sdk).
mod g {
    /// `CPools::ms_pVehiclePool`.
    pub const VEHICLE_POOL: u32 = 0xB74494;
    /// `CPool<CVehicle>::GetAt(handle)` (thiscall).
    pub const POOL_GET_AT: u32 = 0x4048E0;
    /// `CPool<CVehicle>::GetRef(CVehicle*)` (thiscall).
    pub const POOL_GET_REF: u32 = 0x424160;
    /// `CEntity` moving-list link (non-zero while the entity is in the world's moving list).
    pub const MOVING_LIST: u32 = 0xB4;
    /// `CWorld::Remove(CEntity*)` (cdecl).
    pub const CWORLD_REMOVE: u32 = 0x563280;
    /// `CMatrix` rows: right, forward and up vectors.
    pub const MATRIX_RIGHT: u32 = 0x00;
    pub const MATRIX_FORWARD: u32 = 0x10;
    pub const MATRIX_UP: u32 = 0x20;
    /// `CPed` flag word bit `bInVehicle`.
    pub const PED_IN_VEHICLE: u32 = 1 << 8;
    pub const TURN_SPEED: u32 = 0x50;
    pub const VEH_FLAGS: u32 = 0x428;
    /// `VEH_FLAGS` bits: engine on, lights on.
    pub const VEH_FLAG_ENGINE_ON: u8 = 0x10;
    pub const VEH_FLAG_LIGHTS_ON: u8 = 0x40;
    pub const VEH_SIREN_BYTE: u32 = 0x42D;
    /// `VEH_SIREN_BYTE` bit: siren / alarm on.
    pub const VEH_SIREN_ON: u8 = 0x80;
    pub const VEH_PRIMARY_COLOUR: u32 = 0x434;
    pub const VEH_SECONDARY_COLOUR: u32 = 0x435;
    pub const VEH_ALARM: u32 = 0x45C;
    pub const VEH_DRIVER: u32 = 0x460;
    pub const VEH_PASSENGERS: u32 = 0x464;
    pub const VEH_MAX_PASSENGERS: u32 = 0x488;
    pub const VEH_LIGHTS_FLAGS: u32 = 0x4A8;
    /// `VEH_LIGHTS_FLAGS` bit: lights forced off.
    pub const VEH_LIGHTS_FORCED_OFF: u8 = 0x08;
    pub const VEH_DIRT: u32 = 0x4B0;
    pub const VEH_HEALTH: u32 = 0x4C0;
    pub const VEH_TRACTOR: u32 = 0x4C4;
    pub const VEH_TRAILER: u32 = 0x4C8;
    pub const VEH_DOOR_LOCK: u32 = 0x4F8;
    pub const VEH_HORN: u32 = 0x514;
    /// CAutomobile damage manager fields; the four wheel-status bytes are in address order.
    pub const DMG_WHEEL_STATUS: [u32; 4] = [0x5A5, 0x5A6, 0x5A7, 0x5A8];
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
    /// `CTask::GetTaskType` vtable slot.
    pub const VT_TASK_GET_TYPE: u32 = 0x10;
}

const PEDSTATE_IDLE: u32 = 1;
const PEDSTATE_SNIPER: u32 = 12;
const PEDSTATE_DRIVING: u32 = 50;
/// GTA's passenger state (passengers normally show 50 as well; accepted for safety).
const PEDSTATE_PASSENGER: u32 = 51;
/// GTA's dying and dead ped states.
const PEDSTATE_DIE: u32 = 54;
const PEDSTATE_DEAD: u32 = 55;

fn send<P: OutgoingRpc>(net: &mut NetClient, p: &P) {
    net.send_rpc(p, P::RELIABILITY);
}

fn body<S: SyncPacket>(p: &S) -> Vec<u8> {
    p.to_bits().as_bytes().to_vec()
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
    /// Last player whose 209 was applied or sent.
    last_unocc_sender: Option<u16>,
    /// Carriage of a locomotive: no own GTA vehicle created, deleted with the locomotive.
    carriage: bool,
    /// The stream-in position (shown by `/dl` as "sPos").
    spawn_pos: [f32; 3],
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
            spawn_pos: [0.0; 3],
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
    sent_96: u32,
    jacked: u32,
    applied_209: u32,
}

/// The samp vehicle pool and the in-car state of the local and remote players.
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
    /// The local passenger drive-by is active (set by the combat code): 211 drive-by bit.
    pub local_drive_by: bool,
    /// The local player is cuffed (special action 24): 211 cuffed bit.
    pub local_cuffed: bool,
    /// Last synced vehicle per remote player (see `remote_vehicle`).
    last_vehicle: HashMap<u16, u16>,
}

impl Vehicles {
    /// InitGame: manual engine/lights mode (patches `DoVehicleLights` once), in-car send rate, local id.
    pub fn set_init_game(&mut self, g: &samp_proto::session::InitGame) {
        self.manual = g.manual_engine_and_lights;
        self.in_car_rate = g.in_car_rate.clamp(10, 1000);
        self.local_id = Some(g.player_id);
        if self.manual {
            let r = crate::vehpatches::apply(&mut self.patcher, &mut ProcessMemory, crate::vehpatches::MANUAL_LIGHTS);
            crate::log!("vehicles: manual engine/lights: {} rows applied, {} failed", r.applied.len(), r.failed.len());
        }
    }

    /// The local player spawned at least once (RPC 96 is ignored before that).
    pub fn set_spawned(&mut self) {
        self.spawned_once = true;
    }

    /// RPC 167: vehicle-vehicle collisions off / on.
    pub fn set_disable_collisions(&mut self, on: bool) {
        vehhooks::with_shared(|s| s.disable_collisions = on);
    }

    /// Script handle of a streamed vehicle (object attachments).
    pub fn script_handle(&self, id: u16) -> Option<i32> {
        self.get(id).map(|v| v.handle)
    }

    /// GTA vehicle pointer of a streamed vehicle (3D label attachments).
    pub fn gta_ptr(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| vehicle_at(v.handle)).filter(|&p| p != 0)
    }

    /// SA:MP id of a GTA vehicle of the pool (world bounds for the driver).
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

    /// First frame: GTA's vehicle- and weapon-skill cheats, then `CStats::SetStatValue := RET`.
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

    /// Streamed vehicles as (GTA pointer, id) for the combat hooks.
    pub fn hook_vehicles(&self) -> Vec<(u32, u16)> {
        self.pool.values().filter(|v| v.ptr != 0 && v.id != 0).map(|v| (v.ptr, v.id)).collect()
    }

    /// The vehicle of the remote's last stored driver / passenger sync. Written by both stores, kept after the
    /// remote leaves the vehicle, cleared only by the cache reset (RPC 166, stream-out, the reset branch of the
    /// remote tick) and when the player is created.
    pub fn remote_vehicle(&self, pid: u16) -> Option<u16> {
        self.last_vehicle.get(&pid).copied().filter(|&v| v != 0)
    }

    /// `CVehicle*` of a pool vehicle (0 while it has no GTA vehicle).
    pub fn ptr_of(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| v.ptr)
    }

    /// The vehicle model of a pool id.
    pub fn model_of(&self, id: u16) -> Option<u32> {
        self.get(id).map(|v| v.model)
    }

    /// (id, CVehicle*, plate text) of every vehicle in pool order (the plate pass, `plates.rs`).
    pub fn plate_list(&self) -> Vec<(u16, u32, Vec<u8>)> {
        self.pool.iter().map(|(id, v)| (*id, v.ptr, v.plate.clone())).collect()
    }
}
