//! Vehicle hooks inside GTA (Windows only, milestone M5).
//!
//! Specs: remote-control-vehicles.md §2.3 (the `ProcessControl` redirect of all ten vehicle classes with its three
//! branches and the per-player context swap), §2.4 (siren/horn, UsesSiren, BreakTowLink, ProcessEntityCollision,
//! train derail clamp), §5 (enter/leave task constructor hooks with return-address filtering) and vehicles.md
//! RE-10 additions (radar zoom while a passenger). Measured by TRACE-4 (docs/phase3/trace-notes.md "Vehicles"):
//! inside GTA's `CAutomobile::ProcessControl` of a remote-driven car PlayerInFocus = slot, `Pads[0]` = the remote
//! keys, camera mode 4, camera front (0,0,0), look-left/right 0; unoccupied cars far away are not processed.
//!
//! The hooks never touch the client session: they read [`SHARED`] (written by `vehicles.rs` on the main thread
//! between frames) and queue [`HookEvent`]s that the client tick sends in the same frame.
//!
//! Deviations (docs/phase3/m5-notes.md): every swapped global is restored to its saved value (as in swap.rs);
//! on an enter refusal the unconstructed task memory goes back to the task pool instead of samp's call through a
//! stale vtable (TRACE-5 MR-c).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::remote::{PAD_OLD_STATE, PAD_SIZE};
use crate::swap::addr as sw;
use crate::vehicle::{self, Kind, NEAR_RANGE, is_rail};

/// GTA addresses and offsets (names via gta-reversed / plugin-sdk).
pub mod g {
    pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
    /// `CGame::currArea`.
    pub const CURR_AREA: u32 = 0xB72914;
    /// `CAEVehicleAudioEntity::Service` (thiscall).
    pub const AUDIO_SERVICE: u32 = 0x502280;
    /// `CColStore::HasCollisionLoaded(const CVector&, int32 area)` (cdecl).
    pub const HAS_COLLISION_LOADED: u32 = 0x410CE0;
    /// `CAEVehicleAudioEntity::ProcessVehicleSirenAlarmHorn` (thiscall).
    pub const PROCESS_SIREN_ALARM_HORN: u32 = 0x5002C0;
    /// `CVehicle::UsesSiren` (thiscall).
    pub const USES_SIREN: u32 = 0x6D8470;
    /// `CAutomobile::BreakTowLink` (shared by the five classes).
    pub const BREAK_TOW_LINK: u32 = 0x6A4400;
    /// `FindPlayerSpeed` (cdecl, returns `CVector&`).
    pub const FIND_PLAYER_SPEED: u32 = 0x56E090;
    // CEntity / CPhysical / CVehicle / CPed fields.
    pub const ENTITY_TYPE_STATUS: u32 = 0x36;
    pub const ENTITY_MODEL: u32 = 0x22;
    pub const PLACEABLE_MATRIX: u32 = 0x14;
    pub const MOVE_SPEED: u32 = 0x44;
    pub const VEH_AUDIO: u32 = 0x138;
    pub const VEH_FLAGS: u32 = 0x428;
    pub const VEH_ENGINE_ON: u8 = 0x10;
    pub const VEH_DRIVER: u32 = 0x460;
    pub const VEH_HEALTH: u32 = 0x4C0;
    pub const VEH_TRACTOR: u32 = 0x4C4;
    pub const BIKE_LEAN: u32 = 0x648;
    pub const BIKE_DESIRED_LEAN: u32 = 0x64C;
    pub const TRAIN_SPEED: u32 = 0x5A4;
    pub const PED_FLAGS: u32 = 0x46C;
    pub const PED_IN_VEHICLE: u32 = 1 << 8;
    pub const PED_VEHICLE: u32 = 0x58C;
    pub const PED_TYPE: u32 = 0x598;
    /// `CPed::m_nPedType` value of a civilian male (used for the audio "AI car" service).
    pub const PEDTYPE_CIVMALE: u32 = 4;
    /// Entity type 2 (vehicle) + status 3 (STATUS_PHYSICS).
    pub const TYPE_STATUS_VEHICLE_PHYSICS: u8 = 0x1A;
    /// Status bit of STATUS_ABANDONED (4 << 3).
    pub const STATUS_ABANDONED_BIT: u8 = 0x20;
    /// Pad words zeroed for an engine-off car: Square (brake) and Cross (accelerate).
    pub const PAD_SQUARE: usize = 0x0E;
    pub const PAD_CROSS: usize = 0x10;
}

/// `ProcessControl` vtable slots (+0x28) of the ten vehicle classes and their originals (CSV rows 26–35).
pub const PROCESS_CONTROL: [(u32, u32, u32, u32); 10] = [
    // (csv row, slot, original, vtable)
    (26, 0x871148, 0x6B1880, vehicle::vtbl::AUTOMOBILE),
    (27, 0x8721C8, 0x6F1770, vehicle::vtbl::BOAT),
    (28, 0x871388, 0x6B9250, vehicle::vtbl::BIKE),
    (29, 0x871970, 0x6C9260, vehicle::vtbl::PLANE),
    (30, 0x8716A8, 0x6C7050, vehicle::vtbl::HELI),
    (31, 0x871550, 0x6BFA30, vehicle::vtbl::BMX),
    (32, 0x871800, 0x6C8250, vehicle::vtbl::MONSTER_TRUCK),
    (33, 0x871B10, 0x6CDCC0, vehicle::vtbl::QUAD_BIKE),
    (34, 0x872398, 0x6F86A0, vehicle::vtbl::TRAIN),
    (35, 0x871C50, 0x6CED20, vehicle::vtbl::TRAILER),
];

/// `BreakTowLink` slots (+0xF8), CSV rows 70–74 (all point at 0x6A4400).
pub const BREAK_TOW_LINK_SLOTS: [(u32, u32); 5] =
    [(70, 0x871218), (71, 0x871778), (72, 0x8718D0), (73, 0x871A40), (74, 0x871BE0)];

/// `ProcessEntityCollision` slots (+0x58), CSV rows 84–91: (row, slot, original).
pub const ENTITY_COLLISION_SLOTS: [(u32, u32, u32); 8] = [
    (84, 0x871178, 0x6ACE70),
    (85, 0x8716D8, 0x6ACE70),
    (86, 0x8719A0, 0x6ACE70),
    (87, 0x871B40, 0x6ACE70),
    (88, 0x8713B8, 0x6BDEA0),
    (89, 0x871580, 0x6BDEA0),
    (90, 0x871830, 0x6C8AE0),
    (91, 0x8721F8, 0x546D00),
];

/// Call sites of `ProcessVehicleSirenAlarmHorn` (rows 36–40) and `UsesSiren` (rows 92–94).
pub const SIREN_HORN_CALLS: [(u32, u32); 5] =
    [(36, 0x501B1D), (37, 0x501B42), (38, 0x501FC2), (39, 0x502067), (40, 0x5021AE)];
pub const USES_SIREN_CALLS: [(u32, u32); 3] = [(92, 0x6E0954), (93, 0x6B2BCB), (94, 0x4F77DA)];
/// `call FindPlayerSpeed` in `CRadar::DrawMap` (row 64).
pub const RADAR_SPEED_CALL: u32 = 0x586C0A;

/// Enter/leave task constructors (rows 48/49) and the pointer slots of their `jmp [slot]`.
pub const ENTER_CTOR: u32 = 0x6402F0;
pub const ENTER_CTOR_SLOT: u32 = 0x6919BB;
pub const ENTER_CTOR_RESUME: u32 = 0x6402F7;
pub const ENTER_CTOR_ORIGINAL: [u8; 7] = [0x8B, 0x44, 0x24, 0x04, 0x56, 0x6A, 0x00];
pub const LEAVE_CTOR: u32 = 0x63B8C0;
pub const LEAVE_CTOR_SLOT: u32 = 0x63B8BA;
pub const LEAVE_CTOR_RESUME: u32 = 0x63B8C7;
/// The constant the original pushes second (`push 0x841618`, the SEH scope table).
pub const LEAVE_CTOR_SEH_SCOPE: u32 = 0x841618;
pub const LEAVE_CTOR_ORIGINAL: [u8; 7] = [0x6A, 0xFF, 0x68, 0x18, 0x16, 0x84, 0x00];
/// Return addresses of the player-input callers (`CPlayerInfo::Process` area).
pub const ENTER_CALLERS: [u32; 2] = [0x570A20, 0x570A99];
pub const LEAVE_CALLERS: [u32; 2] = [0x5704A1, 0x5703FC];
/// `CTask::operator delete` (cdecl, named via gta-reversed `Task.cpp`).
pub const TASK_OPERATOR_DELETE: u32 = 0x61A5B0;
/// Where a refused enter continues (no SetTask).
pub const ENTER_REFUSED_CONTINUE: u32 = 0x570A9E;
/// Train derail block (rows 47 + 188).
pub const TRAIN_DERAIL_SITE: u32 = 0x6F8CF8;

/// What a hook asks the client to do in the next tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    /// Driver enter accepted: send RPC 26 (passenger 0). `parachute`: arm weapon 0 first.
    Enter { vehicle_id: u16, parachute: bool },
    /// Player-input exit: `driver` = the local ped was the driver; `rc` = no RPC 154; `train` = warp beside it.
    Exit { vehicle_id: u16, driver: bool, rc: bool, train: bool },
}

/// One streamed vehicle as the hooks see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct VehInfo {
    pub id: u16,
    pub model: u32,
    pub locked: bool,
    pub siren: bool,
    /// The samp object has an attached trailer object (RPC 148 / 210).
    pub has_trailer: bool,
}

#[derive(Default)]
pub struct Shared {
    pub by_ptr: HashMap<u32, VehInfo>,
    /// Remote NPC peds (player-pool entry isNPC).
    pub npc_peds: Vec<u32>,
    /// RPC 167 (netgame +0x232).
    pub disable_collisions: bool,
    /// samp is detaching a trailer itself (BreakTowLink lets it through).
    pub detaching: bool,
    pub events: Vec<HookEvent>,
}

pub static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

pub fn with_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Shared::default))
}

/// For hooks that may run while the client holds the lock: never blocks.
pub fn try_with_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    let mut g = match SHARED.try_lock() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    Some(f(g.get_or_insert_with(Shared::default)))
}

/// (remote, local, none-run, none-skipped) ProcessControl calls (diagnostics).
static PC_REMOTE: AtomicU32 = AtomicU32::new(0);
static PC_LOCAL: AtomicU32 = AtomicU32::new(0);
static PC_NONE_RUN: AtomicU32 = AtomicU32::new(0);
static PC_NONE_SKIP: AtomicU32 = AtomicU32::new(0);

pub fn call_counts() -> [u32; 4] {
    [&PC_REMOTE, &PC_LOCAL, &PC_NONE_RUN, &PC_NONE_SKIP].map(|a| a.load(Ordering::Relaxed))
}

// ---- raw access ---------------------------------------------------------------------------------------

unsafe fn rd<T: Copy>(a: u32) -> T {
    // SAFETY: caller passes a readable address of a T.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

unsafe fn wr<T: Copy>(a: u32, v: T) {
    // SAFETY: caller passes a writable address of a T.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) }
}

unsafe fn copy_in(a: u32, src: &[u8]) {
    // SAFETY: caller passes a writable range.
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), a as usize as *mut u8, src.len()) }
}

unsafe fn copy_out(a: u32, dst: &mut [u8]) {
    // SAFETY: caller passes a readable range.
    unsafe { std::ptr::copy_nonoverlapping(a as usize as *const u8, dst.as_mut_ptr(), dst.len()) }
}

fn local_ped() -> u32 {
    // SAFETY: CWorld::Players[0].m_pPed.
    unsafe { rd(g::CWORLD_PLAYERS) }
}

fn pos_of(ent: u32) -> Option<[f32; 3]> {
    // SAFETY: live entity; the matrix pointer may be null.
    unsafe {
        let m: u32 = rd(ent + g::PLACEABLE_MATRIX);
        if m == 0 {
            return None;
        }
        Some([rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)])
    }
}

fn seated(ped: u32) -> bool {
    // SAFETY: live ped.
    ped != 0 && unsafe { rd::<u32>(ped + g::PED_FLAGS) } & g::PED_IN_VEHICLE != 0
}

fn model_of(ent: u32) -> u32 {
    // SAFETY: live entity.
    u32::from(unsafe { rd::<u16>(ent + g::ENTITY_MODEL) })
}

fn has_collision_loaded(pos: [f32; 3]) -> bool {
    // SAFETY: GTA's cdecl function with a valid vector; the area is the current one (samp passes an
    // uninitialised stack value there, MR-5.12).
    unsafe {
        let f: extern "C" fn(*const f32, i32) -> u8 = std::mem::transmute(g::HAS_COLLISION_LOADED as usize);
        f(pos.as_ptr(), rd::<i32>(g::CURR_AREA)) != 0
    }
}

/// Within 200 units (2-D) of the local ped and collision loaded at the car (§2.3 A.7 / C).
fn near_with_collision(veh: u32) -> bool {
    let (Some(p), Some(l)) = (pos_of(veh), pos_of(local_ped())) else { return false };
    vehicle::dist2d(p, l) <= NEAR_RANGE && has_collision_loaded(p)
}

fn audio_service(veh: u32) {
    // SAFETY: GTA's own thiscall on the vehicle's audio entity.
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(g::AUDIO_SERVICE as usize);
        f(veh + g::VEH_AUDIO);
    }
}

fn original_for(vt: u32) -> Option<u32> {
    PROCESS_CONTROL.iter().find(|e| e.3 == vt).map(|e| e.2)
}

// ---- ProcessControl (rows 26–35) ----------------------------------------------------------------------

/// Zeroes Cross and Square (new and old state) of the pad at `pad` (engine off).
unsafe fn zero_throttle(pad: u32) {
    for w in [g::PAD_CROSS, g::PAD_SQUARE] {
        // SAFETY: Pads[0] words.
        unsafe {
            wr::<i16>(pad + 2 * w as u32, 0);
            wr::<i16>(pad + (PAD_OLD_STATE + 2 * w) as u32, 0);
        }
    }
}

/// Engine-off handling of §2.3 A.3 (also B): helis get status ABANDONED, the pad's throttle is zeroed.
/// Engine on clears the heli's ABANDONED bit.
unsafe fn engine_off_rules(veh: u32, kind: Kind) -> bool {
    // SAFETY: live vehicle.
    unsafe {
        let engine_on = rd::<u8>(veh + g::VEH_FLAGS) & g::VEH_ENGINE_ON != 0;
        let st: u8 = rd(veh + g::ENTITY_TYPE_STATUS);
        if kind == Kind::Heli {
            wr(
                veh + g::ENTITY_TYPE_STATUS,
                if engine_on { st & !g::STATUS_ABANDONED_BIT } else { st | g::STATUS_ABANDONED_BIT },
            );
        }
        !engine_on
    }
}

struct SwapSaved {
    pad: [u8; PAD_SIZE],
    looking: (u8, u8),
    cam_mode: u8,
    weapon_mode: u16,
    vectors: [u8; 0x30],
    focus: u8,
}

/// Replacement for `ProcessControl` of every vehicle class (thiscall, vtable slot +0x28).
pub extern "thiscall" fn vehicle_process_control(veh: u32) {
    // SAFETY: GTA calls us with a live vehicle; every address below is a GTA global or a field of it.
    unsafe {
        let vt: u32 = rd(veh);
        let Some(orig_addr) = original_for(vt) else { return };
        let original: extern "thiscall" fn(u32) = std::mem::transmute(orig_addr as usize);
        let kind = vehicle::kind_from_vtable(vt);
        let model = model_of(veh);
        let focus: u8 = rd(sw::PLAYER_IN_FOCUS);
        let driver: u32 = rd(veh + g::VEH_DRIVER);
        let local = local_ped();
        let player_driver = driver != 0 && rd::<u32>(driver + g::PED_TYPE) == 0;

        // A. remote player driver.
        if player_driver && driver != local && focus == 0 {
            let ctx = crate::swap::TABLE.try_lock().ok().and_then(|g| {
                let t = g.as_ref()?;
                let slot = t.slot_of(driver)?;
                Some((slot as u8, t.shadows[slot]))
            });
            if let Some((slot, shadow)) = ctx {
                PC_REMOTE.fetch_add(1, Ordering::Relaxed);
                // 2. swap in.
                let mut saved = SwapSaved {
                    pad: [0; PAD_SIZE],
                    looking: (rd(sw::CAM0_LOOKING_LEFT), rd(sw::CAM0_LOOKING_RIGHT)),
                    cam_mode: rd(sw::CAM0_MODE),
                    weapon_mode: rd(sw::PLAYER_WEAPON_MODE),
                    vectors: [0; 0x30],
                    focus,
                };
                copy_out(sw::PAD0, &mut saved.pad);
                copy_out(sw::CAM0_VECTORS, &mut saved.vectors);
                copy_in(sw::PAD0, &shadow.pad);
                let (l, r) = shadow.looking_left_right();
                wr(sw::CAM0_LOOKING_LEFT, l);
                wr(sw::CAM0_LOOKING_RIGHT, r);
                wr(sw::CAM0_MODE, shadow.cam_mode);
                wr(sw::PLAYER_WEAPON_MODE, shadow.weapon_mode());
                let aim: Vec<u8> = shadow.aim.iter().flat_map(|f| f.to_le_bytes()).collect();
                copy_in(sw::CAM0_VECTORS, &aim);
                // 3. engine off.
                if engine_off_rules(veh, kind) {
                    zero_throttle(sw::PAD0);
                }
                // 4. bike lean values.
                let bike = matches!(kind, Kind::Bike | Kind::Bmx);
                let lean: (f32, f32) =
                    if bike { (rd(veh + g::BIKE_LEAN), rd(veh + g::BIKE_DESIRED_LEAN)) } else { (0.0, 0.0) };
                // 5. audio service as an AI car.
                wr(sw::PLAYER_IN_FOCUS, 0u8);
                let st: u8 = rd(veh + g::ENTITY_TYPE_STATUS);
                wr(driver + g::PED_TYPE, g::PEDTYPE_CIVMALE);
                wr(veh + g::ENTITY_TYPE_STATUS, g::TYPE_STATUS_VEHICLE_PHYSICS);
                audio_service(veh);
                wr(veh + g::ENTITY_TYPE_STATUS, st);
                wr(driver + g::PED_TYPE, 0u32);
                // 6. focus := slot, save matrix z and move-speed z, run GTA's function.
                wr(sw::PLAYER_IN_FOCUS, slot);
                let m: u32 = rd(veh + g::PLACEABLE_MATRIX);
                let z: f32 = if m != 0 { rd(m + 0x38) } else { 0.0 };
                let vz: f32 = rd(veh + g::MOVE_SPEED + 8);
                original(veh);
                // 7. z protection (far away / no collision, or always for boats and aircraft).
                let restore = if !is_rail(model) && !near_with_collision(veh) {
                    true
                } else {
                    matches!(kind, Kind::Boat | Kind::Plane | Kind::Heli)
                };
                if restore {
                    let m2: u32 = rd(veh + g::PLACEABLE_MATRIX);
                    if m2 != 0 && m2 == m {
                        wr(m2 + 0x38, z);
                    }
                    wr(veh + g::MOVE_SPEED + 8, vz);
                }
                if bike {
                    wr(veh + g::BIKE_LEAN, lean.0);
                    wr(veh + g::BIKE_DESIRED_LEAN, lean.1);
                    wr(veh + g::BIKE_LEAN, lean.0 + (lean.1 - lean.0) * 0.5);
                }
                // 8. swap out (every value restored to what it was).
                wr(sw::PLAYER_IN_FOCUS, saved.focus);
                copy_in(sw::PAD0, &saved.pad);
                wr(sw::CAM0_LOOKING_LEFT, saved.looking.0);
                wr(sw::CAM0_LOOKING_RIGHT, saved.looking.1);
                wr(sw::CAM0_MODE, saved.cam_mode);
                wr(sw::PLAYER_WEAPON_MODE, saved.weapon_mode);
                copy_in(sw::CAM0_VECTORS, &saved.vectors);
                return;
            }
        }
        // B. local driver, a nested call while a switch is active, or a player driver without a slot.
        if (driver != 0 && driver == local) || focus != 0 || player_driver {
            PC_LOCAL.fetch_add(1, Ordering::Relaxed);
            if engine_off_rules(veh, kind) {
                let mut pad = [0u8; PAD_SIZE];
                copy_out(sw::PAD0, &mut pad);
                zero_throttle(sw::PAD0);
                audio_service(veh);
                original(veh);
                copy_in(sw::PAD0, &pad);
            } else {
                audio_service(veh);
                original(veh);
            }
            return;
        }
        // C. no driver or a non-player driver: simulate only when towed by a seated driver, rail, or near.
        let tractor: u32 = rd(veh + g::VEH_TRACTOR);
        let towed = tractor != 0 && seated(rd::<u32>(tractor + g::VEH_DRIVER));
        if towed || is_rail(model) || near_with_collision(veh) {
            PC_NONE_RUN.fetch_add(1, Ordering::Relaxed);
            audio_service(veh);
            original(veh);
        } else {
            PC_NONE_SKIP.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ---- enter / leave task constructors (rows 48/49) -----------------------------------------------------

/// Entry of `CTaskComplexEnterCarAsDriver::CTaskComplexEnterCarAsDriver(CVehicle*)` (thiscall, ret 4).
#[unsafe(naked)]
unsafe extern "C" fn enter_ctor_stub() {
    core::arch::naked_asm!(
        "push ecx",
        "push dword ptr [esp+8]",
        "push ecx",
        "push dword ptr [esp+12]",
        "call {filter}",
        "add esp, 12",
        "pop ecx",
        "test eax, eax",
        "jz 2f",
        "mov eax, [esp+4]",
        "push esi",
        "push 0",
        "push {resume}",
        "ret",
        "2:",
        "add esp, 8",
        "push {refused}",
        "ret",
        filter = sym enter_filter,
        resume = const ENTER_CTOR_RESUME,
        refused = const ENTER_REFUSED_CONTINUE,
    )
}

/// Entry of `CTaskComplexLeaveCar::CTaskComplexLeaveCar(CVehicle*, …)` (thiscall, 5 arguments).
#[unsafe(naked)]
unsafe extern "C" fn leave_ctor_stub() {
    core::arch::naked_asm!(
        "push ecx",
        "push dword ptr [esp+8]",
        "push ecx",
        "push dword ptr [esp+12]",
        "call {filter}",
        "add esp, 12",
        "pop ecx",
        "push eax",
        "push {seh}",
        "push {resume}",
        "ret",
        filter = sym leave_filter,
        seh = const LEAVE_CTOR_SEH_SCOPE,
        resume = const LEAVE_CTOR_RESUME,
    )
}

/// The driver-enter gate (A 0x100A3700). Returns 1 to let the constructor run, 0 after deleting the task.
extern "C" fn enter_filter(ret: u32, task: u32, veh: u32) -> u32 {
    if !ENTER_CALLERS.contains(&ret) {
        return 1;
    }
    let accept = gate_driver_enter(veh);
    match accept {
        Some(ev) => {
            with_shared(|s| s.events.push(ev));
            1
        }
        None => {
            // TRACE-5 MR-c: samp calls slot 0 of whatever stale vtable the unconstructed task memory holds (a samp
            // quirk). We only return the raw memory to the task pool (`CTask::operator delete`, 0x61A5B0); the
            // observable effect is the same: no task, no RPC.
            // SAFETY: the memory came from `CTask::operator new` in the caller; nothing was constructed in it.
            unsafe {
                let f: extern "C" fn(u32) = std::mem::transmute(TASK_OPERATOR_DELETE as usize);
                f(task);
            }
            0
        }
    }
}

/// Refusal rules of §5: no samp id, locked, model 570, current driver not a player or a remote NPC.
/// Surfing (the 4th rule) is not tracked by this client yet (TODO-unknown: local surf state, M3 deviation).
fn gate_driver_enter(veh: u32) -> Option<HookEvent> {
    let info = SHARED.try_lock().ok()?.as_ref()?.by_ptr.get(&veh).copied()?;
    if info.locked || info.model == vehicle::MODEL_FREIGHT_CARRIAGE {
        return None;
    }
    // SAFETY: live vehicle / ped fields.
    let driver: u32 = unsafe { rd(veh + g::VEH_DRIVER) };
    if driver != 0 {
        if unsafe { rd::<u32>(driver + g::PED_TYPE) } != 0 {
            return None;
        }
        if with_shared(|s| s.npc_peds.contains(&driver)) {
            return None;
        }
    }
    let local = local_ped();
    let parachute = local != 0 && crate::vehicles::current_weapon(local) == u32::from(vehicle::WEAPON_PARACHUTE);
    Some(HookEvent::Enter { vehicle_id: info.id, parachute })
}

/// The exit hook (A 0x100A38C0): returns the value pushed as the SEH try-level (−1, or 1 for trains).
extern "C" fn leave_filter(ret: u32, _task: u32, veh: u32) -> i32 {
    if !LEAVE_CALLERS.contains(&ret) {
        return -1;
    }
    let local = local_ped();
    // SAFETY: local ped field.
    if local == 0 || unsafe { rd::<u32>(local + g::PED_VEHICLE) } != veh {
        return -1;
    }
    let Some(info) = SHARED.try_lock().ok().and_then(|g| g.as_ref().and_then(|s| s.by_ptr.get(&veh).copied())) else {
        return -1;
    };
    let train = is_rail(info.model);
    // SAFETY: live vehicle.
    let driver = unsafe { rd::<u32>(veh + g::VEH_DRIVER) } == local && !matches!(info.model, 569 | 570);
    if train {
        // B 0x100A3992…0x100A39E0: inside the constructor, WARP_CHAR_FROM_CAR_TO_COORD to the train's position
        // + (2.5, 2.5, 0); no camera call and no RPC 154 (measured, run orig-20261105-042300 mark `trainexit`:
        // the warp precedes GTA's SetTask of the leave task in the same frame).
        if let Ok(Some(h)) = crate::game::ped_handle(&crate::patch::ProcessMemory, local) {
            // SAFETY: live vehicle matrix.
            let m: u32 = unsafe { rd(veh + 0x14) };
            if m != 0 {
                let p: [f32; 3] = unsafe { [rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)] };
                use crate::script::{Arg, Runner, op};
                Runner::new().run(
                    op::WARP_CHAR_FROM_CAR_TO_COORD,
                    &[
                        Arg::Int(h),
                        Arg::Float(p[0] + vehicle::TRAIN_EXIT_OFFSET),
                        Arg::Float(p[1] + vehicle::TRAIN_EXIT_OFFSET),
                        Arg::Float(p[2]),
                    ],
                );
            }
        }
    }
    with_shared(|s| {
        s.events.push(HookEvent::Exit { vehicle_id: info.id, driver, rc: vehicle::is_rc(info.model), train })
    });
    if train { 1 } else { -1 }
}

// ---- other hooks ----------------------------------------------------------------------------------------

/// `ProcessVehicleSirenAlarmHorn` call sites (rows 36–40): entity status PLAYER while a driver is seated,
/// else ABANDONED, for the duration of the call.
extern "thiscall" fn siren_alarm_horn(audio: u32, params: u32) {
    // SAFETY: GTA's audio entity (m_pEntity at +4) and its own function.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(g::PROCESS_SIREN_ALARM_HORN as usize);
        let veh: u32 = rd(audio + 4);
        if veh == 0 {
            f(audio, params);
            return;
        }
        let st: u8 = rd(veh + g::ENTITY_TYPE_STATUS);
        let status = if seated(rd(veh + g::VEH_DRIVER)) { 0 } else { 4 };
        wr(veh + g::ENTITY_TYPE_STATUS, (st & 0x07) | (status << 3));
        f(audio, params);
        wr(veh + g::ENTITY_TYPE_STATUS, st);
    }
}

/// `UsesSiren` call sites (rows 92–94): GTA's answer, else the stream-in siren flag. (The one-shot "force true"
/// global of samp has no known writer; not reproduced.)
extern "thiscall" fn uses_siren(veh: u32) -> bool {
    // SAFETY: GTA's own thiscall.
    let orig = unsafe {
        let f: extern "thiscall" fn(u32) -> bool = std::mem::transmute(g::USES_SIREN as usize);
        f(veh)
    };
    orig || SHARED
        .try_lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|s| s.by_ptr.get(&veh).map(|i| i.siren)))
        .unwrap_or(false)
}

/// `BreakTowLink` (rows 70–74): only the cab's driver (or the server) can break a tow.
extern "thiscall" fn break_tow_link(veh: u32) -> bool {
    // SAFETY: live vehicle fields; GTA's own function.
    unsafe {
        let original: extern "thiscall" fn(u32) -> bool = std::mem::transmute(g::BREAK_TOW_LINK as usize);
        let tractor: u32 = rd(veh + g::VEH_TRACTOR);
        let suppress = SHARED.try_lock().ok().and_then(|g| {
            let s = g.as_ref()?;
            if s.detaching || rd::<f32>(veh + g::VEH_HEALTH) <= 0.0 || tractor == 0 {
                return Some(false);
            }
            let cab = s.by_ptr.get(&tractor)?;
            let local_drives = rd::<u32>(tractor + g::VEH_DRIVER) == local_ped() && local_ped() != 0;
            Some(!local_drives && cab.has_trailer)
        });
        if suppress == Some(true) { false } else { original(veh) }
    }
}

fn entity_collision_blocked(veh: u32, other: u32) -> bool {
    let Some(on) = SHARED.try_lock().ok().and_then(|g| g.as_ref().map(|s| s.disable_collisions)) else { return false };
    if !on || other == 0 {
        return false;
    }
    // SAFETY: live entities.
    unsafe {
        let is_vehicle = rd::<u8>(other + g::ENTITY_TYPE_STATUS) & 7 == 2;
        let m = model_of(other);
        is_vehicle
            && (vehicle::MODEL_MIN..=vehicle::MODEL_MAX).contains(&m)
            && rd::<u32>(veh + g::VEH_DRIVER) != 0
            && rd::<u32>(other + g::VEH_DRIVER) != 0
    }
}

macro_rules! entity_collision_hook {
    ($name:ident, $orig:expr) => {
        extern "thiscall" fn $name(veh: u32, other: u32, colpoints: u32) -> i32 {
            if entity_collision_blocked(veh, other) {
                return 0;
            }
            // SAFETY: GTA's own thiscall with the same signature.
            unsafe {
                let f: extern "thiscall" fn(u32, u32, u32) -> i32 = std::mem::transmute($orig as usize);
                f(veh, other, colpoints)
            }
        }
    };
}
entity_collision_hook!(pec_automobile, 0x6ACE70u32);
entity_collision_hook!(pec_bike, 0x6BDEA0u32);
entity_collision_hook!(pec_monster_truck, 0x6C8AE0u32);
entity_collision_hook!(pec_physical, 0x546D00u32);

fn pec_for(original: u32) -> u32 {
    let f: extern "thiscall" fn(u32, u32, u32) -> i32 = match original {
        0x6ACE70 => pec_automobile,
        0x6BDEA0 => pec_bike,
        0x6C8AE0 => pec_monster_truck,
        _ => pec_physical,
    };
    f as usize as u32
}

/// Train derail block (rows 47/188): clamp `m_fTrainSpeed` to ±1.0 (sign kept); the patched code then jumps past
/// the derail check.
extern "C" fn clamp_train_speed(train: u32) {
    if train == 0 {
        return;
    }
    // SAFETY: live CTrain (esi of CTrain::ProcessControl).
    unsafe {
        let s: f32 = rd(train + g::TRAIN_SPEED);
        if s.abs() > 1.0 {
            wr(train + g::TRAIN_SPEED, 1.0f32.copysign(s));
        }
    }
}

#[unsafe(naked)]
unsafe extern "C" fn train_derail_stub() {
    core::arch::naked_asm!("push esi", "call {f}", "add esp, 4", "ret", f = sym clamp_train_speed)
}

static ZERO_SPEED: [f32; 3] = [0.0; 3];

/// `call CVehicleModelInfo::SetDirtTextures` in `CVehicle::Render` (row 62).
const RENDER_DIRT_CALL: u32 = 0x6D0E7E;
const SET_DIRT_TEXTURES: u32 = 0x5D5DB0;

/// The vehicle `CVehicle::Render` is drawing (esi at row 62's call; samp B 0x10151550, read by samp's plate
/// texture code).
static RENDER_VEHICLE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The vehicle `CVehicle::Render` is drawing now (plate substitution, `plates.rs`).
pub fn render_vehicle() -> u32 {
    RENDER_VEHICLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Row 62 (B 0x100A4570): replaces `SetDirtTextures(modelInfo, dirt)` in `CVehicle::Render`. It only records the
/// vehicle (esi) and returns: the dirt textures are never switched per vehicle (the caller pops the arguments).
#[unsafe(naked)]
unsafe extern "C" fn render_dirt_stub() {
    core::arch::naked_asm!("mov dword ptr [{v}], esi", "ret", v = sym RENDER_VEHICLE)
}

/// `FindPlayerSpeed` in `CRadar::DrawMap` (row 64): a zero vector while the local ped is a passenger.
extern "C" fn radar_find_player_speed(id: i32) -> *const f32 {
    let local = local_ped();
    if local != 0 && seated(local) {
        // SAFETY: local ped and its vehicle.
        let veh: u32 = unsafe { rd(local + g::PED_VEHICLE) };
        if veh != 0 {
            let driver: u32 = unsafe { rd(veh + g::VEH_DRIVER) };
            if driver != local || matches!(model_of(veh), 569 | 570) {
                return ZERO_SPEED.as_ptr();
            }
        }
    }
    // SAFETY: GTA's own cdecl function.
    unsafe {
        let f: extern "C" fn(i32) -> *const f32 = std::mem::transmute(g::FIND_PLAYER_SPEED as usize);
        f(id)
    }
}

// ---- install --------------------------------------------------------------------------------------------

fn expect_ptr(slot: u32, want: u32) -> Result<(), PatchError> {
    let found = ProcessMemory.read_u32(slot)?;
    if found == want {
        Ok(())
    } else {
        Err(PatchError::Mismatch {
            addr: slot,
            expected: want.to_le_bytes().to_vec(),
            found: found.to_le_bytes().to_vec(),
        })
    }
}

/// `jmp dword [slot]` + NOP at `site`, the handler address stored in `slot` (samp's hook-jmp-indirect form).
fn hook_jmp_indirect(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    label: &'static str,
    site: u32,
    slot: u32,
    handler: u32,
    original: &[u8; 7],
) -> Result<(), PatchError> {
    patcher.write(mem, label, slot, &handler.to_le_bytes(), Some(&[0x90; 4]))?;
    let s = slot.to_le_bytes();
    patcher.write(mem, label, site, &[0xFF, 0x25, s[0], s[1], s[2], s[3], 0x90], Some(original))
}

/// Installs every vehicle hook. Returns (row, result) per CSV row for the log.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    let pc = vehicle_process_control as extern "thiscall" fn(u32) as usize as u32;
    for (row, slot, orig, _) in PROCESS_CONTROL {
        let r = expect_ptr(slot, orig)
            .and_then(|()| patcher.swap_pointer(mem, "vehicle ProcessControl", slot, pc).map(|_| ()));
        out.push((row, r));
    }
    let btl = break_tow_link as extern "thiscall" fn(u32) -> bool as usize as u32;
    for (row, slot) in BREAK_TOW_LINK_SLOTS {
        let r = expect_ptr(slot, g::BREAK_TOW_LINK)
            .and_then(|()| patcher.swap_pointer(mem, "BreakTowLink", slot, btl).map(|_| ()));
        out.push((row, r));
    }
    for (row, slot, orig) in ENTITY_COLLISION_SLOTS {
        let r = expect_ptr(slot, orig)
            .and_then(|()| patcher.swap_pointer(mem, "ProcessEntityCollision", slot, pec_for(orig)).map(|_| ()));
        out.push((row, r));
    }
    let sah = siren_alarm_horn as extern "thiscall" fn(u32, u32) as usize as u32;
    for (row, site) in SIREN_HORN_CALLS {
        out.push((
            row,
            patcher
                .retarget_branch(mem, "ProcessVehicleSirenAlarmHorn call", site, sah, Some(g::PROCESS_SIREN_ALARM_HORN))
                .map(|_| ()),
        ));
    }
    let us = uses_siren as extern "thiscall" fn(u32) -> bool as usize as u32;
    for (row, site) in USES_SIREN_CALLS {
        out.push((row, patcher.retarget_branch(mem, "UsesSiren call", site, us, Some(g::USES_SIREN)).map(|_| ())));
    }
    let rs = radar_find_player_speed as extern "C" fn(i32) -> *const f32 as usize as u32;
    out.push((
        64,
        patcher
            .retarget_branch(mem, "CRadar::DrawMap FindPlayerSpeed", RADAR_SPEED_CALL, rs, Some(g::FIND_PLAYER_SPEED))
            .map(|_| ()),
    ));
    let rd_stub = render_dirt_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        62,
        patcher
            .retarget_branch(mem, "CVehicle::Render SetDirtTextures", RENDER_DIRT_CALL, rd_stub, Some(SET_DIRT_TEXTURES))
            .map(|_| ()),
    ));
    let enter = enter_ctor_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        48,
        hook_jmp_indirect(
            patcher,
            mem,
            "enter-as-driver ctor",
            ENTER_CTOR,
            ENTER_CTOR_SLOT,
            enter,
            &ENTER_CTOR_ORIGINAL,
        ),
    ));
    let leave = leave_ctor_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        49,
        hook_jmp_indirect(patcher, mem, "leave-car ctor", LEAVE_CTOR, LEAVE_CTOR_SLOT, leave, &LEAVE_CTOR_ORIGINAL),
    ));
    // Row 47 over row 188 (vehpatches INIT wrote NOP5 + mov eax,0x6F8F89; jmp eax).
    let td = train_derail_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        47,
        patcher.hook_call(mem, "CTrain::ProcessControl derail clamp", TRAIN_DERAIL_SITE, td, Some(&[0x90; 5])),
    ));
    with_shared(|_| ());
    out
}
