//! Combat hooks inside GTA (Windows only, milestone M7).
//!
//! Specs: remote-control-combat.md §3 (H3 `FireInstantHit` / `FireSniper` call sites, H4 the line-of-sight hook,
//! H5 `AddBullet`), §6 (H6 `ComputeDamageResponse`, H7 `CEventDamage::AffectsPed`, H8 `CVehicle::InflictDamage`),
//! §1.3 (H9 `GetWeaponSkill` while rendering weapons) and the CSV rows 4, 5, 8, 22, 45, 46, 53, 56, 57, 80, 98–106,
//! 191, 200, 201, 210, 240–243, 247. Static details read in samp.dll A for this module:
//! * the call-site stubs return their 8th / 1st argument ("fired") and only the local ped fires (A 0x100A5820,
//!   0x100A58E0); a local shot ignores GTA's origin arguments and fires through A 0x100AFAB0;
//! * the hooks of rows 4/5 and 98–106 are installed by InitGame only for lag-comp 1 and 2 (A 0x10010909);
//! * the LOS hook returns 0 when it suppresses a hit on the local ped (A 0x100A57D8);
//! * a satchel whose owner is null is removed in `CProjectileInfo::Update` (A 0x100A3FC0), and the detonator only
//!   blows charges whose owner is the detonating ped (A 0x100A4000);
//! * `CVehicle::InflictDamage` is cancelled with `ret 0x18` (A 0x100A53BE).
//!
//! Like vehhooks.rs, the hooks never touch the client session: they read [`SHARED`] (written by the client tick)
//! and queue [`Outgoing`] messages that the client sends in the same frame, in the order they were queued.
//!
//! Deviations (docs/phase3/m7-notes.md): stubs keep their state on the stack instead of samp's fixed globals
//! (spec P1); the remote 206 shot is fired with the shooter's camera swapped and restored to the saved values.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use samp_proto::sync::player::BulletSync;

use crate::combat::{self, DamageCtx, HitTarget, Matrix, Placement, RemoteRef, Report, V3};
use crate::patch::{PatchError, Patcher, ProcessMemory, encode_branch};
use crate::remote::{AIM_FLOATS, SKILLS};
use crate::swap::addr as sw;

/// GTA addresses (names via gta-reversed / plugin-sdk).
pub mod g {
    pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
    pub const PROCESS_LINE_OF_SIGHT: u32 = 0x56BA00;
    pub const FIRE_INSTANT_HIT: u32 = 0x73FB10;
    pub const FIRE_SNIPER: u32 = 0x73AAC0;
    pub const ADD_BULLET: u32 = 0x736010;
    pub const BULLET_UPDATE: u32 = 0x7360D0;
    /// `CWeaponInfo::GetWeaponInfo(eWeaponType, eWeaponSkill)` (cdecl).
    pub const GET_WEAPON_INFO: u32 = 0x743C60;
    pub const WEAPON_INFO_FIRE_OFFSET: u32 = 0x24;
    /// `CPed::GetBonePosition(RwV3d&, bone, bool)` / `GetTransformedBonePosition(RwV3d&, bone, bool)` (thiscall).
    pub const GET_BONE_POSITION: u32 = 0x5E4280;
    pub const GET_TRANSFORMED_BONE_POSITION: u32 = 0x5E01C0;
    /// `CPed::GetWeaponSkill()` (thiscall).
    pub const GET_WEAPON_SKILL: u32 = 0x5E6580;
    /// `CHud::DrawCrossHairs` (cdecl).
    pub const DRAW_CROSSHAIRS: u32 = 0x58E020;
    /// `CProjectileInfo::RemoveIfThisIsAProjectile(CObject*)` (cdecl).
    pub const REMOVE_IF_PROJECTILE: u32 = 0x739A40;
    /// `CFire::Extinguish` (thiscall).
    pub const FIRE_EXTINGUISH: u32 = 0x5393F0;
    // Fields.
    pub const ENTITY_FLAGS: u32 = 0x1C;
    pub const PLACEABLE_MATRIX: u32 = 0x14;
    pub const PED_FLAGS: u32 = 0x46C;
    pub const PED_VEHICLE: u32 = 0x58C;
    pub const PED_WEAPONS: u32 = 0x5A0;
    pub const PED_SELECTED_SLOT: u32 = 0x718;
    pub const PED_FIRE: u32 = 0x730;
    pub const VEH_DRIVER: u32 = 0x460;
    pub const VEH_FIRE: u32 = 0x490;
    /// `CCivilianPed` vtable (samp actors), `CPlaceable` vtable (a deleted entity).
    pub const VTBL_CIVILIAN_PED: u32 = 0x86C0A8;
    pub const VTBL_CPLACEABLE: u32 = 0x863C40;
}

/// Call sites (CSV rows).
pub mod site {
    pub const FIRE_INSTANT_HIT: [(u32, u32); 3] = [(98, 0x742495), (99, 0x7424EC), (100, 0x742548)];
    pub const LOS: [(u32, u32); 3] = [(101, 0x740721), (102, 0x740B69), (103, 0x736247)];
    pub const FIRE_SNIPER: u32 = 0x7424CB; // row 104
    pub const ADD_BULLET: u32 = 0x73AC4B; // row 105
    pub const INFLICT_DAMAGE: u32 = 0x6D7C90; // row 106
    pub const INFLICT_DAMAGE_ORIGINAL: [u8; 7] = [0x6A, 0xFF, 0xE9, 0x45, 0xD0, 0xD2, 0xFF];
    /// Target of the original `jmp` at 0x6D7C92.
    pub const INFLICT_DAMAGE_CONTINUE: u32 = 0x404CDC;
    pub const COMPUTE_DAMAGE_RESPONSE: u32 = 0x4B5AC0; // row 53, `jmp [0x4B5ABC]`
    pub const COMPUTE_DAMAGE_RESPONSE_SLOT: u32 = 0x4B5ABC;
    pub const COMPUTE_DAMAGE_RESPONSE_ORIGINAL: [u8; 6] = [0x64, 0xA1, 0x00, 0x00, 0x00, 0x00];
    pub const COMPUTE_DAMAGE_RESPONSE_RESUME: u32 = 0x4B5AC6;
    pub const AFFECTS_PED: u32 = 0x4B35A0; // row 80, `jmp [0x4B3433]`
    pub const AFFECTS_PED_SLOT: u32 = 0x4B3433;
    pub const AFFECTS_PED_ORIGINAL: [u8; 6] = [0x83, 0xEC, 0x0C, 0x56, 0x8B, 0xF1];
    pub const AFFECTS_PED_RESUME: u32 = 0x4B35A6;
    pub const RENDER_WEAPON_SKILL: u32 = 0x7330A2; // row 22
    pub const CROSSHAIRS: u32 = 0x58FBBF; // row 8
    pub const SNIPER_CAM_SHAKE: u32 = 0x73ACE2; // row 46
    pub const CAM_SHAKE_NO_POS: u32 = 0x50A970;
    pub const TAKE_PHOTO: u32 = 0x73C252; // row 45 (jmp over `mov [bPhotographHasBeenTaken], 1`)
    pub const TAKE_PHOTO_ORIGINAL: [u8; 7] = [0xC6, 0x05, 0xC0, 0xA7, 0xC8, 0x00, 0x01];
    pub const TAKE_PHOTO_RESUME: u32 = 0x73C260;
    pub const PHOTO_TAKEN: u32 = 0xC8A7C0;
    pub const PHOTO_TAKEN_2: u32 = 0xC8A7C1;
    pub const SATCHEL_UPDATE: u32 = 0x738F3A; // row 56, `jmp [0x738B1B]`
    pub const SATCHEL_UPDATE_SLOT: u32 = 0x738B1B;
    pub const SATCHEL_UPDATE_ORIGINAL: [u8; 6] = [0x8B, 0x4B, 0x04, 0x8A, 0x51, 0x36];
    pub const SATCHEL_UPDATE_RESUME: u32 = 0x738F40;
    pub const DETONATOR: u32 = 0x738877; // row 57, `jmp [0x73885B]`
    pub const DETONATOR_SLOT: u32 = 0x73885B;
    pub const DETONATOR_ORIGINAL: [u8; 6] = [0x83, 0x7E, 0xE0, 0x27, 0x8B, 0x7D];
    pub const DETONATOR_BLOW: u32 = 0x738880;
    pub const DETONATOR_SKIP: u32 = 0x7388DB;
    /// Lock-on RETs (rows 4/5, InitGame).
    pub const FIND_LOCK_ON: u32 = 0x60DC50;
    pub const FIND_NEXT_LOCK_ON: u32 = 0x60E530;
}

/// The pad pad-slot bytes of the `jmp [slot]` rows (gta_sa.exe padding before the function).
const SLOT_PADDING: [u8; 4] = [0x90; 4];

// ---- shared state (written by the client tick) -------------------------------------------------------------

/// A streamed remote player as the hooks see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteHook {
    pub ped: u32,
    pub id: u16,
    pub npc: bool,
    pub team: u8,
    /// Remote +0x1E7: the vehicle of the last stored driver/passenger sync.
    pub vehicle: Option<u16>,
}

#[derive(Clone, Debug, Default)]
pub struct Shared {
    pub local_team: u8,
    /// Remote players in pool (id) order.
    pub remotes: Vec<RemoteHook>,
    /// Streamed vehicles: (GTA pointer, id).
    pub vehicles: Vec<(u32, u16)>,
    /// Actors: (ped, id), filled by the world-entity code (RPC 177 of the damage hook).
    pub actors: Vec<(u32, u16)>,
    /// The vehicle the local player surfs (0 = none). TODO-unknown until local surfing exists.
    pub surf_vehicle: u32,
}

impl Shared {
    fn remote_by_ped(&self, ped: u32) -> Option<&RemoteHook> {
        if ped == 0 {
            return None;
        }
        self.remotes.iter().find(|r| r.ped == ped)
    }

    fn vehicle_id(&self, ptr: u32) -> Option<u16> {
        if ptr == 0 {
            return None;
        }
        self.vehicles.iter().find(|v| v.0 == ptr).map(|v| v.1)
    }

    /// Entity → player (A 0x100A3B20): a remote ped, else the first remote (pool order) whose synced vehicle is
    /// that pool vehicle.
    fn player_of_entity(&self, e: u32) -> Option<&RemoteHook> {
        if let Some(r) = self.remote_by_ped(e) {
            return Some(r);
        }
        let vid = self.vehicle_id(e)?;
        self.remotes.iter().find(|r| r.vehicle == Some(vid))
    }

    fn rref(r: &RemoteHook) -> RemoteRef {
        RemoteRef { id: r.id, npc: r.npc, team: r.team }
    }
}

pub static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

pub fn set_shared(s: Shared) {
    *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

fn with_shared<R>(f: impl FnOnce(&Shared) -> R) -> Option<R> {
    let g = SHARED.try_lock().ok()?;
    g.as_ref().map(f)
}

/// What the hooks ask the client to send, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Bullet(BulletSync),
    /// RPC 115.
    Damage {
        taking: bool,
        player: u16,
        damage: f32,
        weapon: u32,
        body_part: u32,
    },
    /// RPC 177.
    ActorDamage {
        actor: u16,
        damage: f32,
        weapon: u32,
        body_part: u32,
    },
}

static OUTBOX: Mutex<Vec<Outgoing>> = Mutex::new(Vec::new());

pub fn take_outgoing() -> Vec<Outgoing> {
    std::mem::take(&mut *OUTBOX.lock().unwrap_or_else(|e| e.into_inner()))
}

fn queue(o: Outgoing) {
    OUTBOX.lock().unwrap_or_else(|e| e.into_inner()).push(o);
}

/// Diagnostics: the first this many damage responses / remote shots are logged.
const LOG_LIMIT: u32 = 64;

/// Counters for the control channel.
pub static N_LOCAL_SHOTS: AtomicU32 = AtomicU32::new(0);
pub static N_REMOTE_SHOTS: AtomicU32 = AtomicU32::new(0);
pub static N_DROPPED_REMOTE: AtomicU32 = AtomicU32::new(0);
pub static N_SUPPRESSED: AtomicU32 = AtomicU32::new(0);
pub static N_REDIRECTED: AtomicU32 = AtomicU32::new(0);
pub static N_DAMAGE: AtomicU32 = AtomicU32::new(0);

/// `combat` control-channel reply.
pub fn status_line() -> String {
    let n = |a: &AtomicU32| a.load(Ordering::Relaxed);
    format!(
        "combat selector={} local_shots={} remote_shots={} dropped_remote={} redirected={} suppressed={} damage_events={} queued={}",
        selector(),
        n(&N_LOCAL_SHOTS),
        n(&N_REMOTE_SHOTS),
        n(&N_DROPPED_REMOTE),
        n(&N_REDIRECTED),
        n(&N_SUPPRESSED),
        n(&N_DAMAGE),
        OUTBOX.lock().map(|o| o.len()).unwrap_or(0)
    )
}

// ---- raw memory --------------------------------------------------------------------------------------------

fn rd<T: Copy>(a: u32) -> T {
    // SAFETY: callers pass addresses of live game objects / globals.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

fn wr<T: Copy>(a: u32, v: T) {
    // SAFETY: callers pass writable game data.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) }
}

fn v3(a: u32) -> V3 {
    [rd(a), rd(a + 4), rd(a + 8)]
}

pub fn local_ped() -> u32 {
    rd(g::CWORLD_PLAYERS)
}

/// The entity's matrix, if it is live (not a deleted `CPlaceable`) and has one.
fn matrix_of(e: u32) -> Option<Matrix> {
    if e == 0 || rd::<u32>(e) == g::VTBL_CPLACEABLE {
        return None;
    }
    let m: u32 = rd(e + g::PLACEABLE_MATRIX);
    (m != 0).then(|| Matrix { right: v3(m), forward: v3(m + 0x10), up: v3(m + 0x20), pos: v3(m + 0x30) })
}

/// The ped's current `CWeapon` (`m_aWeapons[m_nSelectedWepSlot]`).
pub fn current_weapon_ptr(ped: u32) -> u32 {
    ped + g::PED_WEAPONS + u32::from(rd::<u8>(ped + g::PED_SELECTED_SLOT)) * 0x1C
}

pub fn current_weapon(ped: u32) -> u8 {
    rd::<u32>(current_weapon_ptr(ped)) as u8
}

// ---- the shot (A 0x100AFAB0) --------------------------------------------------------------------------------

/// Shooter and record published for the LOS hook while a shot is fired.
static SHOOTER: AtomicU32 = AtomicU32::new(0);
static RECORD_VALID: AtomicBool = AtomicBool::new(false);
static RECORD_ENTITY: AtomicU32 = AtomicU32::new(0);
static RECORD_OFFSET: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
/// Lag-comp selector cached for the hooks (InitGame).
static SELECTOR: AtomicU32 = AtomicU32::new(2);

pub fn selector() -> u8 {
    SELECTOR.load(Ordering::Relaxed) as u8
}

/// The camera context of a remote shot (no pad, no PlayerInFocus; A 0x100AFAB0).
struct ShotContext {
    cam_mode: u8,
    weapon_mode: u16,
    fov: f32,
    aspect: f32,
    vectors: [f32; AIM_FLOATS],
    skills: [f32; SKILLS],
}

fn read_floats<const N: usize>(a: u32) -> [f32; N] {
    std::array::from_fn(|i| rd(a + 4 * i as u32))
}

fn write_floats(a: u32, v: &[f32]) {
    for (i, f) in v.iter().enumerate() {
        wr(a + 4 * i as u32, *f);
    }
}

fn enter_shot_context(slot: u8) -> Option<ShotContext> {
    let shadow = crate::swap::with_table(|t| t.shadows.get(usize::from(slot)).copied())?;
    let saved = ShotContext {
        cam_mode: rd(sw::CAM0_MODE),
        weapon_mode: rd(sw::PLAYER_WEAPON_MODE),
        fov: rd(sw::CAM0_FOV),
        aspect: rd(sw::ASPECT_RATIO),
        vectors: read_floats(sw::CAM0_VECTORS),
        skills: read_floats(sw::WEAPON_SKILLS),
    };
    wr(sw::CAM0_MODE, shadow.cam_mode);
    wr(sw::PLAYER_WEAPON_MODE, shadow.weapon_mode());
    wr(sw::CAM0_FOV, shadow.fov());
    wr(sw::ASPECT_RATIO, shadow.aspect + 1.0);
    write_floats(sw::CAM0_VECTORS, &shadow.aim);
    write_floats(sw::WEAPON_SKILLS, &shadow.skills);
    Some(saved)
}

fn leave_shot_context(s: &ShotContext) {
    wr(sw::CAM0_MODE, s.cam_mode);
    wr(sw::PLAYER_WEAPON_MODE, s.weapon_mode);
    wr(sw::CAM0_FOV, s.fov);
    wr(sw::ASPECT_RATIO, s.aspect);
    write_floats(sw::CAM0_VECTORS, &s.vectors);
    write_floats(sw::WEAPON_SKILLS, &s.skills);
}

/// Origin and muzzle of a shot (A 0x100AFA00): the right-hand bone, z + fire offset z + 0.15; the muzzle is the
/// weapon's fire offset transformed by the same bone.
fn origin_and_muzzle(ped: u32, weapon: u8) -> (V3, V3) {
    // SAFETY: GTA's weapon-info lookup and bone queries (cdecl / thiscall) on a live ped, main thread.
    unsafe {
        let info: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(g::GET_WEAPON_INFO as usize);
        let wi = info(u32::from(weapon), 1);
        let fire_offset = if wi != 0 { v3(wi + g::WEAPON_INFO_FIRE_OFFSET) } else { [0.0; 3] };
        let mut origin = [0f32; 3];
        let bone: extern "thiscall" fn(u32, *mut f32, u32, u32) = std::mem::transmute(g::GET_BONE_POSITION as usize);
        bone(ped, origin.as_mut_ptr(), combat::BONE_RIGHT_HAND, 0);
        origin[2] += fire_offset[2] + combat::ORIGIN_Z_EXTRA;
        let mut muzzle = fire_offset;
        let tbone: extern "thiscall" fn(u32, *mut f32, u32, u32) =
            std::mem::transmute(g::GET_TRANSFORMED_BONE_POSITION as usize);
        tbone(ped, muzzle.as_mut_ptr(), combat::BONE_RIGHT_HAND, 0);
        (origin, muzzle)
    }
}

/// Fires one shot for `ped` (A 0x100AFAB0). `slot` 0 = the local ped (no swap). `record` = the remote 206's
/// target entity and offset (the LOS hook's redirect / suppression input).
pub fn fire_shot(ped: u32, slot: u8, record: Option<(u32, V3)>) {
    if ped == 0 {
        return;
    }
    let ctx = if slot != 0 { enter_shot_context(slot) } else { None };
    SHOOTER.store(ped, Ordering::SeqCst);
    match record {
        Some((e, off)) => {
            RECORD_ENTITY.store(e, Ordering::SeqCst);
            for (a, v) in RECORD_OFFSET.iter().zip(off) {
                a.store(v.to_bits(), Ordering::SeqCst);
            }
            RECORD_VALID.store(true, Ordering::SeqCst);
        }
        None => RECORD_VALID.store(false, Ordering::SeqCst),
    }
    let weapon = current_weapon(ped);
    let wptr = current_weapon_ptr(ped);
    // SAFETY: GTA's own fire functions (thiscall on the ped's CWeapon), main thread, outside any ped context.
    unsafe {
        if weapon == combat::weapon::SNIPER {
            let f: extern "thiscall" fn(u32, u32, u32, u32) -> u32 = std::mem::transmute(g::FIRE_SNIPER as usize);
            f(wptr, ped, 0, 0);
        } else {
            let (mut origin, mut muzzle) = origin_and_muzzle(ped, weapon);
            let f: extern "thiscall" fn(u32, u32, *mut f32, *mut f32, u32, u32, u32, u32, u32) -> u32 =
                std::mem::transmute(g::FIRE_INSTANT_HIT as usize);
            f(wptr, ped, origin.as_mut_ptr(), muzzle.as_mut_ptr(), 0, 0, 0, 0, 1);
        }
    }
    SHOOTER.store(0, Ordering::SeqCst);
    RECORD_VALID.store(false, Ordering::SeqCst);
    if let Some(c) = ctx {
        leave_shot_context(&c);
    }
}

/// Before a local instant-hit shot every streamed on-foot remote ped gets collision (A 0x10013C90); the saved
/// state is put back afterwards (A 0x10013D10).
fn force_remote_collisions() -> Vec<(u32, bool)> {
    let peds = with_shared(|s| s.remotes.iter().map(|r| r.ped).collect::<Vec<_>>()).unwrap_or_default();
    peds.into_iter()
        .filter(|&p| p != 0 && rd::<u32>(p + g::PED_FLAGS) & (1 << 8) == 0)
        .map(|p| {
            let flags: u32 = rd(p + g::ENTITY_FLAGS);
            wr(p + g::ENTITY_FLAGS, crate::remote::collision_flags(flags, true));
            (p, flags & 1 != 0)
        })
        .collect()
}

fn restore_remote_collisions(saved: Vec<(u32, bool)>) {
    for (p, on) in saved {
        let flags: u32 = rd(p + g::ENTITY_FLAGS);
        wr(p + g::ENTITY_FLAGS, crate::remote::collision_flags(flags, on));
    }
}

/// H3: replaces `call CWeapon::FireInstantHit` in `CWeapon::Fire` (A 0x100A5820). Returns the 8th argument.
extern "thiscall" fn fire_instant_hit_site(
    _weapon: u32,
    firing: u32,
    _a2: u32,
    _a3: u32,
    _a4: u32,
    _a5: u32,
    _a6: u32,
    _a7: u32,
    a8: u32,
) -> u32 {
    let local = local_ped();
    if firing != local || local == 0 {
        N_DROPPED_REMOTE.fetch_add(1, Ordering::Relaxed);
        return a8;
    }
    N_LOCAL_SHOTS.fetch_add(1, Ordering::Relaxed);
    let saved = force_remote_collisions();
    fire_shot(local, 0, None);
    restore_remote_collisions(saved);
    a8
}

/// H3: replaces `call CWeapon::FireSniper` in `CWeapon::Fire` (A 0x100A58E0). Returns the shooter.
extern "thiscall" fn fire_sniper_site(_weapon: u32, shooter: u32, _victim: u32, _target: u32) -> u32 {
    let local = local_ped();
    if shooter == local && local != 0 {
        N_LOCAL_SHOTS.fetch_add(1, Ordering::Relaxed);
        fire_shot(local, 0, None);
    } else {
        N_DROPPED_REMOTE.fetch_add(1, Ordering::Relaxed);
    }
    shooter
}

/// H5: replaces `call CBulletInfo::AddBullet` in `FireSniper` (A 0x100A5930): velocity × 50, then
/// `CBulletInfo::Update` at once, while the shooter globals are still set.
extern "C" fn add_bullet_site(creator: u32, weapon: u32, px: f32, py: f32, pz: f32, vx: f32, vy: f32, vz: f32) {
    let k = combat::SNIPER_VELOCITY_SCALE;
    // SAFETY: GTA's static bullet functions (cdecl), main thread.
    unsafe {
        let add: extern "C" fn(u32, u32, f32, f32, f32, f32, f32, f32) = std::mem::transmute(g::ADD_BULLET as usize);
        add(creator, weapon, px, py, pz, vx * k, vy * k, vz * k);
        let update: extern "C" fn() = std::mem::transmute(g::BULLET_UPDATE as usize);
        update();
    }
}

type Los = extern "C" fn(*const f32, *mut f32, *mut f32, *mut u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32;

/// H4: replaces `call CWorld::ProcessLineOfSight` at the three sites (A 0x100A5640).
#[allow(clippy::too_many_arguments)]
extern "C" fn los_site(
    origin: *const f32,
    target: *mut f32,
    colpoint: *mut f32,
    entity: *mut u32,
    b1: u32,
    b2: u32,
    b3: u32,
    b4: u32,
    b5: u32,
    b6: u32,
    b7: u32,
    b8: u32,
) -> u32 {
    let s = selector();
    let shooter = SHOOTER.load(Ordering::SeqCst);
    let local = local_ped();
    let remote_shot = s != 2 && shooter != 0 && shooter != local;
    // SAFETY: GTA passes valid pointers to its own vectors / col point / entity slot.
    let start: V3 = unsafe { [*origin, *origin.add(1), *origin.add(2)] };
    let record_entity =
        if RECORD_VALID.load(Ordering::SeqCst) { Some(RECORD_ENTITY.load(Ordering::SeqCst)) } else { None };
    if remote_shot
        && let Some(e) = record_entity
        && let Some(m) = matrix_of(e)
    {
        let off: V3 = std::array::from_fn(|i| f32::from_bits(RECORD_OFFSET[i].load(Ordering::SeqCst)));
        if let Some(end) = combat::redirect_end(s, start, &m, off) {
            N_REDIRECTED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: as above.
            unsafe { std::ptr::copy_nonoverlapping(end.as_ptr(), target, 3) };
        }
    }
    // SAFETY: the original cdecl function with the caller's arguments.
    let r = unsafe {
        let f: Los = std::mem::transmute(g::PROCESS_LINE_OF_SIGHT as usize);
        f(origin, target, colpoint, entity, b1, b2, b3, b4, b5, b6, b7, b8)
    };
    if remote_shot && N_REMOTE_SHOTS.load(Ordering::Relaxed) <= LOG_LIMIT {
        // SAFETY: as above.
        let (end, hit): (V3, u32) = unsafe { ([*target, *target.add(1), *target.add(2)], *entity) };
        crate::log!(
            "combat: remote LOS start {start:?} end {end:?} record {record_entity:?} -> {r} entity {hit:#x} (local {local:#x})"
        );
    }
    if remote_shot && record_entity == Some(0) {
        // SAFETY: as above.
        let hit: u32 = unsafe { *entity };
        let in_car = local != 0 && rd::<u32>(local + g::PED_FLAGS) & (1 << 8) != 0;
        if hit != 0 && (hit == local || (in_car && hit == rd::<u32>(local + g::PED_VEHICLE))) {
            N_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: as above.
            unsafe {
                *entity = 0;
                std::ptr::write_bytes(colpoint, 0, 3);
            }
            return 0;
        }
    }
    if shooter != 0 && shooter == local {
        // SAFETY: as above.
        let (hit_pos, hit): (V3, u32) = unsafe { ([*colpoint, *colpoint.add(1), *colpoint.add(2)], *entity) };
        capture_local(s, start, hit_pos, hit);
    }
    r
}

/// Local capture (A 0x100A5450 → 0x10006BC0): build the record and queue the 206.
fn capture_local(s: u8, origin: V3, hit_pos: V3, hit: u32) {
    let m = if hit != 0 { matrix_of(hit) } else { None };
    let rec = combat::capture(s, origin, hit_pos, m.as_ref());
    let entity = if m.is_some() { hit } else { 0 };
    let target = if entity == 0 {
        HitTarget::None
    } else {
        with_shared(|sh| {
            if let Some(r) = sh.remote_by_ped(entity) {
                HitTarget::Player(r.id)
            } else if let Some(id) = sh.vehicle_id(entity) {
                HitTarget::Vehicle(id)
            } else {
                // Objects (hit type 3) belong to the world-entity pool (M6): TODO-unknown here.
                HitTarget::Other
            }
        })
        .unwrap_or(HitTarget::Other)
    };
    let other = (target == HitTarget::Other).then(|| match m {
        Some(m) => Placement::Matrix(m),
        None => Placement::Position(v3(entity + 4)),
    });
    let weapon = current_weapon(local_ped());
    if let Some(b) = combat::bullet_sync(&rec, target, other, weapon) {
        queue(Outgoing::Bullet(b));
    }
}

// ---- H6: ComputeDamageResponse -----------------------------------------------------------------------------

/// Decision for `CPedDamageResponseCalculator` `calc` and `victim` (A 0x100A3BF0). Returns 1 = cancel.
extern "C" fn damage_decide(calc: u32, victim: u32) -> u32 {
    if calc == 0 || victim == 0 {
        return 0;
    }
    N_DAMAGE.fetch_add(1, Ordering::Relaxed);
    let local = local_ped();
    let damager: u32 = rd(calc);
    let damage: f32 = rd(calc + 4);
    let body_part: u32 = rd(calc + 8);
    let weapon: u32 = rd(calc + 0xC);
    let decision = with_shared(|sh| {
        let victim_local = victim == local;
        let damager_local = damager == local && local != 0;
        let ctx = DamageCtx {
            victim_local,
            damager_local,
            victim_civilian: rd::<u32>(victim) == g::VTBL_CIVILIAN_PED,
            surf_vehicle_is_damager: sh.surf_vehicle != 0 && sh.surf_vehicle == damager,
            surfing: sh.surf_vehicle != 0,
            weapon,
            attacker: if victim_local { sh.player_of_entity(damager).map(Shared::rref) } else { None },
            victim_remote: if damager_local { sh.player_of_entity(victim).map(Shared::rref) } else { None },
            victim_actor: if damager_local { sh.actors.iter().find(|a| a.0 == victim).map(|a| a.1) } else { None },
            local_team: sh.local_team,
        };
        combat::decide(&ctx)
    });
    if N_DAMAGE.load(Ordering::Relaxed) <= LOG_LIMIT {
        crate::log!(
            "combat: damage response victim {victim:#x} (local {}) damager {damager:#x} weapon {weapon} damage {damage} part {body_part} -> {decision:?}",
            victim == local
        );
    }
    let Some(d) = decision else { return 0 };
    match d.report {
        Some(Report::Take { attacker }) => queue(Outgoing::Damage {
            taking: true,
            player: attacker,
            damage: damage * combat::DAMAGE_SCALE,
            weapon,
            body_part,
        }),
        Some(Report::Give { victim }) => queue(Outgoing::Damage {
            taking: false,
            player: victim,
            damage: damage * combat::DAMAGE_SCALE,
            weapon,
            body_part,
        }),
        Some(Report::Actor { actor }) => queue(Outgoing::ActorDamage { actor, damage, weapon, body_part }),
        None => {}
    }
    u32::from(d.cancel)
}

/// Entry stub of `ComputeDamageResponse(CPed* victim, CPedDamageResponse&, bool)` (thiscall, 3 arguments).
#[unsafe(naked)]
unsafe extern "C" fn damage_stub() {
    std::arch::naked_asm!(
        "test ecx, ecx",
        "jz 2f",
        "push ecx",
        "push edx",
        "push dword ptr [esp + 12]",
        "push ecx",
        "call {decide}",
        "add esp, 8",
        "pop edx",
        "pop ecx",
        "test eax, eax",
        "jz 2f",
        "ret 12",
        "2:",
        "mov eax, dword ptr fs:[0]",
        "push {resume}",
        "ret",
        decide = sym damage_decide,
        resume = const site::COMPUTE_DAMAGE_RESPONSE_RESUME,
    )
}

// ---- H7: CEventDamage::AffectsPed ---------------------------------------------------------------------------

/// Entry stub of `CEventDamage::AffectsPed(CPed*)` (thiscall): true at once for the local ped while a remote
/// context is active (A 0x100A53E0).
#[unsafe(naked)]
unsafe extern "C" fn affects_ped_stub() {
    std::arch::naked_asm!(
        "push eax",
        "mov eax, dword ptr [{players}]",
        "test eax, eax",
        "jz 2f",
        "cmp eax, dword ptr [esp + 8]",
        "jne 2f",
        "cmp byte ptr [{focus}], 0",
        "je 2f",
        "pop eax",
        "mov eax, 1",
        "ret 4",
        "2:",
        "pop eax",
        "sub esp, 12",
        "push esi",
        "mov esi, ecx",
        "push {resume}",
        "ret",
        players = const g::CWORLD_PLAYERS,
        focus = const sw::PLAYER_IN_FOCUS,
        resume = const site::AFFECTS_PED_RESUME,
    )
}

// ---- H8: CVehicle::InflictDamage ---------------------------------------------------------------------------

/// A 0x100A5280: cancel when the vehicle is the one the local player drives and the damager maps to a player of
/// the local team.
extern "C" fn inflict_decide(vehicle: u32, damager: u32) -> u32 {
    let local = local_ped();
    if local == 0 || vehicle == 0 || rd::<u32>(vehicle + g::VEH_DRIVER) != local {
        return 0;
    }
    with_shared(|sh| {
        let team = sh.local_team;
        let r = sh.player_of_entity(damager);
        u32::from(team != 255 && r.is_some_and(|r| r.team == team))
    })
    .unwrap_or(0)
}

/// Entry stub of `CVehicle::InflictDamage(CEntity*, eWeaponType, float, CVector)` (thiscall, 6 dwords).
#[unsafe(naked)]
unsafe extern "C" fn inflict_stub() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push dword ptr [esp + 12]",
        "push ecx",
        "call {decide}",
        "add esp, 8",
        "pop edx",
        "pop ecx",
        "test eax, eax",
        "jz 2f",
        "xor eax, eax",
        "ret 0x18",
        "2:",
        "push -1",
        "push {resume}",
        "ret",
        decide = sym inflict_decide,
        resume = const site::INFLICT_DAMAGE_CONTINUE,
    )
}

// ---- H9 and small hooks ------------------------------------------------------------------------------------

/// Replaces `call CPed::GetWeaponSkill` in `RenderWeaponPedsForPC` (A 0x100A2D80): a remote ped's skill comes
/// from its slot's skill table.
extern "thiscall" fn render_weapon_skill(ped: u32) -> u32 {
    // SAFETY: GTA's own function with the same signature.
    let f: extern "thiscall" fn(u32) -> u32 = unsafe { std::mem::transmute(g::GET_WEAPON_SKILL as usize) };
    let skills = crate::swap::with_table(|t| t.slot_of(ped).map(|s| t.shadows[s].skills));
    match skills {
        Some(sk) => {
            let saved: [f32; SKILLS] = read_floats(sw::WEAPON_SKILLS);
            write_floats(sw::WEAPON_SKILLS, &sk);
            let r = f(ped);
            write_floats(sw::WEAPON_SKILLS, &saved);
            r
        }
        None => f(ped),
    }
}

/// Replaces `call CamShakeNoPos` in `FireSniper` (A 0x100A3DA0: a bare `ret`).
extern "C" fn no_cam_shake() {}

/// Crosshair correction (row 8, A 0x100A2590): only active with samp's flag A 0x10117470, which is 0 unless
/// sa-mp.cfg sets `nohudscalefix` to a non-zero value (A 0x100C49DD). That configuration path is not wired yet, so
/// the wrapper only calls the original (identical to samp with the default configuration).
extern "C" fn crosshairs_site() {
    // TODO-unknown: the corrected branch (A 0x100A2440) for `nohudscalefix` = 1 is not implemented.
    // SAFETY: the original cdecl function.
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::DRAW_CROSSHAIRS as usize);
        f();
    }
}

/// `CWeapon::TakePhotograph` (row 45, A 0x100A3DB0): the photo flags only for the local photographer.
/// Entered with the photographer at [esp+8] (the jmp replaces an instruction inside the function body).
#[unsafe(naked)]
unsafe extern "C" fn take_photo_stub() {
    std::arch::naked_asm!(
        "push eax",
        "mov eax, dword ptr [esp + 12]",
        "cmp eax, dword ptr [{players}]",
        "jne 2f",
        "mov byte ptr [{f1}], 1",
        "mov byte ptr [{f2}], 1",
        "2:",
        "pop eax",
        "push {resume}",
        "ret",
        players = const g::CWORLD_PLAYERS,
        f1 = const site::PHOTO_TAKEN,
        f2 = const site::PHOTO_TAKEN_2,
        resume = const site::TAKE_PHOTO_RESUME,
    )
}

/// `CProjectileInfo::Update` at the type-39 test (row 56, A 0x100A3FC0): ESI = the projectile object, EBX = the
/// projectile info; an owner-less satchel is removed, then the loop continues with DL = 0xFF.
#[unsafe(naked)]
unsafe extern "C" fn satchel_update_stub() {
    std::arch::naked_asm!(
        "cmp dword ptr [ebx + 4], 0",
        "jne 2f",
        "pushad",
        "push esi",
        "mov eax, {remove}",
        "call eax",
        "add esp, 4",
        "popad",
        "mov dl, 0xFF",
        "jmp 3f",
        "2:",
        "mov ecx, dword ptr [ebx + 4]",
        "mov dl, byte ptr [ecx + 0x36]",
        "3:",
        "push {resume}",
        "ret",
        remove = const g::REMOVE_IF_PROJECTILE,
        resume = const site::SATCHEL_UPDATE_RESUME,
    )
}

/// `CProjectileInfo::RemoveDetonatorProjectiles` at the type-39 test (row 57, A 0x100A4000): only satchels owned
/// by the detonating ped ([esp]) are blown.
#[unsafe(naked)]
unsafe extern "C" fn detonator_stub() {
    std::arch::naked_asm!(
        "cmp dword ptr [esi - 0x20], 0x27",
        "mov edi, dword ptr [ebp]",
        "jne 2f",
        "mov eax, dword ptr [esp]",
        "cmp dword ptr [esi - 0x1C], eax",
        "jne 2f",
        "push {blow}",
        "ret",
        "2:",
        "push {skip}",
        "ret",
        blow = const site::DETONATOR_BLOW,
        skip = const site::DETONATOR_SKIP,
    )
}

/// Constants samp points GTA at (rows 210, 240–243).
static TWO: f32 = 2.0;
static BOUND_MIN: f32 = -20000.0;
static BOUND_MAX: f32 = 20000.0;

/// Extinguishes the ped's fire and its vehicle's (A 0x100AD130).
pub fn extinguish(ped: u32) {
    let fire: u32 = rd(ped + g::PED_FIRE);
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(g::FIRE_EXTINGUISH as usize) };
    if fire != 0 {
        f(fire);
    }
    let veh: u32 = rd(ped + g::PED_VEHICLE);
    if rd::<u32>(ped + g::PED_FLAGS) & (1 << 8) != 0 && veh != 0 {
        let vf: u32 = rd(veh + g::VEH_FIRE);
        if vf != 0 {
            f(vf);
        }
    }
}

// ---- install -----------------------------------------------------------------------------------------------

fn fn_addr(f: usize) -> u32 {
    f as u32
}

/// `jmp [slot]` + pointer at `slot` (the samp pattern of rows 53, 56, 57, 80).
fn jmp_indirect(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    what: &'static str,
    at: u32,
    slot: u32,
    original: &[u8; 6],
    target: u32,
) -> Result<(), PatchError> {
    patcher.write(mem, what, slot, &target.to_le_bytes(), Some(&SLOT_PADDING))?;
    let mut b = vec![0xFF, 0x25];
    b.extend_from_slice(&slot.to_le_bytes());
    patcher.write(mem, what, at, &b, Some(original))
}

/// The init-main rows (applied once at start-up). Returns one result per row.
#[allow(clippy::vec_init_then_push)] // one push per CSV row reads like the table it implements
pub fn install_main(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    out.push((
        53,
        jmp_indirect(
            patcher,
            mem,
            "ComputeDamageResponse hook",
            site::COMPUTE_DAMAGE_RESPONSE,
            site::COMPUTE_DAMAGE_RESPONSE_SLOT,
            &site::COMPUTE_DAMAGE_RESPONSE_ORIGINAL,
            fn_addr(damage_stub as *const () as usize),
        ),
    ));
    out.push((
        80,
        jmp_indirect(
            patcher,
            mem,
            "CEventDamage::AffectsPed hook",
            site::AFFECTS_PED,
            site::AFFECTS_PED_SLOT,
            &site::AFFECTS_PED_ORIGINAL,
            fn_addr(affects_ped_stub as *const () as usize),
        ),
    ));
    out.push((
        56,
        jmp_indirect(
            patcher,
            mem,
            "CProjectileInfo::Update satchel",
            site::SATCHEL_UPDATE,
            site::SATCHEL_UPDATE_SLOT,
            &site::SATCHEL_UPDATE_ORIGINAL,
            fn_addr(satchel_update_stub as *const () as usize),
        ),
    ));
    out.push((
        57,
        jmp_indirect(
            patcher,
            mem,
            "RemoveDetonatorProjectiles",
            site::DETONATOR,
            site::DETONATOR_SLOT,
            &site::DETONATOR_ORIGINAL,
            fn_addr(detonator_stub as *const () as usize),
        ),
    ));
    let r = patcher
        .retarget_branch(
            mem,
            "RenderWeaponPedsForPC GetWeaponSkill",
            site::RENDER_WEAPON_SKILL,
            fn_addr(render_weapon_skill as *const () as usize),
            Some(g::GET_WEAPON_SKILL),
        )
        .map(|_| ());
    out.push((22, r));
    let r = patcher
        .retarget_branch(
            mem,
            "CHud::DrawCrossHairs",
            site::CROSSHAIRS,
            fn_addr(crosshairs_site as *const () as usize),
            Some(g::DRAW_CROSSHAIRS),
        )
        .map(|_| ());
    out.push((8, r));
    let r = patcher
        .retarget_branch(
            mem,
            "FireSniper CamShakeNoPos",
            site::SNIPER_CAM_SHAKE,
            fn_addr(no_cam_shake as *const () as usize),
            Some(site::CAM_SHAKE_NO_POS),
        )
        .map(|_| ());
    out.push((46, r));
    let mut jmp =
        encode_branch(crate::patch::OP_JMP, site::TAKE_PHOTO, fn_addr(take_photo_stub as *const () as usize)).to_vec();
    jmp.extend_from_slice(&[0x90, 0x90]);
    out.push((
        45,
        patcher.write(mem, "TakePhotograph flags", site::TAKE_PHOTO, &jmp[..5], Some(&site::TAKE_PHOTO_ORIGINAL[..5])),
    ));
    // Byte rows.
    let nops: [(u32, u32, usize, &[u8], &'static str); 4] = [
        (191, 0x56E5AD, 5, &[0xE8, 0xFE, 0x0C, 0x13, 0x00], "KillPlayer: no Wasted message"),
        (200, 0x60F289, 8, &[0x68, 0x00, 0x00, 0x80, 0x3F, 0x50, 0x6A, 0x29], "sniper fire-failed sound"),
        (
            201,
            0x60F29D,
            19,
            &[
                0x68, 0x00, 0x00, 0x80, 0x3F, 0x6A, 0x00, 0x6A, 0x2A, 0xB9, 0x90, 0xBC, 0xB6, 0x00, 0xE8, 0xF0, 0x7B,
                0xEF, 0xFF,
            ],
            "rocket fire-failed sound",
        ),
        (247, 0x4C3A5B, 10, &[0x55, 0x53, 0x57, 0x8B, 0xCE, 0xE8, 0xDB, 0xED, 0xFF, 0xFF], "ComputeGunAimedAtResponse"),
    ];
    for (row, addr, len, orig, what) in nops {
        out.push((row, patcher.nop(mem, what, addr, len, Some(orig))));
    }
    let ptrs: [(u32, u32, u32, u32, &'static str); 5] = [
        (210, 0x61E0C2, 0xB7CB5C, &TWO as *const f32 as u32, "ControlGunMove time step := 2.0"),
        (240, 0x7361B2, 0x859A90, &BOUND_MIN as *const f32 as u32, "bullet bound -20000"),
        (241, 0x7361CB, 0x859A94, &BOUND_MAX as *const f32 as u32, "bullet bound 20000"),
        (242, 0x7361E0, 0x859A90, &BOUND_MIN as *const f32 as u32, "bullet bound -20000"),
        (243, 0x7361F5, 0x859A94, &BOUND_MAX as *const f32 as u32, "bullet bound 20000"),
    ];
    for (row, addr, orig, new, what) in ptrs {
        out.push((row, patcher.write(mem, what, addr, &new.to_le_bytes(), Some(&orig.to_le_bytes()))));
    }
    out
}

static INIT_GAME_DONE: AtomicBool = AtomicBool::new(false);

/// The InitGame rows (A 0x10010909): the selector, then for lag-comp 1/2 the bullet hooks and the lock-on RETs.
/// Installed once per process (samp never removes them either).
pub fn install_init_game(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    selector: u8,
) -> Vec<(u32, Result<(), PatchError>)> {
    SELECTOR.store(u32::from(selector), Ordering::SeqCst);
    let mut out = Vec::new();
    if !combat::bullet_hooks_active(selector) || INIT_GAME_DONE.swap(true, Ordering::SeqCst) {
        return out;
    }
    for (row, at) in site::FIRE_INSTANT_HIT {
        let r = patcher
            .retarget_branch(
                mem,
                "CWeapon::Fire FireInstantHit",
                at,
                fn_addr(fire_instant_hit_site as *const () as usize),
                Some(g::FIRE_INSTANT_HIT),
            )
            .map(|_| ());
        out.push((row, r));
    }
    for (row, at) in site::LOS {
        let r = patcher
            .retarget_branch(
                mem,
                "bullet ProcessLineOfSight",
                at,
                fn_addr(los_site as *const () as usize),
                Some(g::PROCESS_LINE_OF_SIGHT),
            )
            .map(|_| ());
        out.push((row, r));
    }
    let r = patcher
        .retarget_branch(
            mem,
            "CWeapon::Fire FireSniper",
            site::FIRE_SNIPER,
            fn_addr(fire_sniper_site as *const () as usize),
            Some(g::FIRE_SNIPER),
        )
        .map(|_| ());
    out.push((104, r));
    let r = patcher
        .retarget_branch(
            mem,
            "FireSniper AddBullet",
            site::ADD_BULLET,
            fn_addr(add_bullet_site as *const () as usize),
            Some(g::ADD_BULLET),
        )
        .map(|_| ());
    out.push((105, r));
    let jmp = encode_branch(crate::patch::OP_JMP, site::INFLICT_DAMAGE, fn_addr(inflict_stub as *const () as usize));
    out.push((
        106,
        patcher.write(
            mem,
            "CVehicle::InflictDamage hook",
            site::INFLICT_DAMAGE,
            &jmp,
            Some(&site::INFLICT_DAMAGE_ORIGINAL[..5]),
        ),
    ));
    out.push((4, patcher.write(mem, "FindWeaponLockOnTarget := ret", site::FIND_LOCK_ON, &[0xC3], Some(&[0x81]))));
    out.push((
        5,
        patcher.write(
            mem,
            "FindNextWeaponLockOnTarget := ret 8",
            site::FIND_NEXT_LOCK_ON,
            &[0xC2, 0x08, 0x00],
            Some(&[0x6A, 0xFF, 0xE9]),
        ),
    ));
    out
}
