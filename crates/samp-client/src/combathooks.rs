//! Combat hooks inside GTA (Windows only): firing (H3 `FireInstantHit` / `FireSniper` call sites, H5 `AddBullet`),
//! the line-of-sight hook (H4), damage (H6 `ComputeDamageResponse`, H7 `CEventDamage::AffectsPed`, H8
//! `CVehicle::InflictDamage`), per-slot weapon skills while rendering (H9) and smaller weapon patches. Hook names
//! H3–H9 are those of docs/re/subsystems/remote-control-combat.md §9.1.
//!
//! Like `vehhooks.rs`, the hooks never touch the client session: they read [`SHARED`] (written by the client tick)
//! and queue [`Outgoing`] messages that the client sends in the same frame, in order. Unlike samp, the stubs keep
//! their state on the stack instead of fixed globals (docs/phase3/m7-notes.md).

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

/// Call sites and patch locations in gta_sa.exe; the numbers paired with a site are its rows in
/// docs/re/data/gta-patches-r4.csv.
pub mod site {
    pub const FIRE_INSTANT_HIT: [(u32, u32); 3] = [(98, 0x742495), (99, 0x7424EC), (100, 0x742548)];
    pub const LOS: [(u32, u32); 3] = [(101, 0x740721), (102, 0x740B69), (103, 0x736247)];
    pub const FIRE_SNIPER: u32 = 0x7424CB; // call FireSniper in CWeapon::Fire
    pub const ADD_BULLET: u32 = 0x73AC4B; // call AddBullet in FireSniper
    pub const INFLICT_DAMAGE: u32 = 0x6D7C90; // CVehicle::InflictDamage entry
    pub const INFLICT_DAMAGE_ORIGINAL: [u8; 7] = [0x6A, 0xFF, 0xE9, 0x45, 0xD0, 0xD2, 0xFF];
    /// Target of the original `jmp` at 0x6D7C92.
    pub const INFLICT_DAMAGE_CONTINUE: u32 = 0x404CDC;
    pub const COMPUTE_DAMAGE_RESPONSE: u32 = 0x4B5AC0; // `jmp [slot]` at the entry
    pub const COMPUTE_DAMAGE_RESPONSE_SLOT: u32 = 0x4B5ABC;
    pub const COMPUTE_DAMAGE_RESPONSE_ORIGINAL: [u8; 6] = [0x64, 0xA1, 0x00, 0x00, 0x00, 0x00];
    pub const COMPUTE_DAMAGE_RESPONSE_RESUME: u32 = 0x4B5AC6;
    pub const AFFECTS_PED: u32 = 0x4B35A0; // `jmp [slot]` at the entry
    pub const AFFECTS_PED_SLOT: u32 = 0x4B3433;
    pub const AFFECTS_PED_ORIGINAL: [u8; 6] = [0x83, 0xEC, 0x0C, 0x56, 0x8B, 0xF1];
    pub const AFFECTS_PED_RESUME: u32 = 0x4B35A6;
    pub const RENDER_WEAPON_SKILL: u32 = 0x7330A2; // call GetWeaponSkill in RenderWeaponPedsForPC
    pub const CROSSHAIRS: u32 = 0x58FBBF; // call DrawCrossHairs
    pub const SNIPER_CAM_SHAKE: u32 = 0x73ACE2; // call CamShakeNoPos in FireSniper
    pub const CAM_SHAKE_NO_POS: u32 = 0x50A970;
    pub const TAKE_PHOTO: u32 = 0x73C252; // jmp over `mov [bPhotographHasBeenTaken], 1`
    pub const TAKE_PHOTO_ORIGINAL: [u8; 7] = [0xC6, 0x05, 0xC0, 0xA7, 0xC8, 0x00, 0x01];
    pub const TAKE_PHOTO_RESUME: u32 = 0x73C260;
    pub const PHOTO_TAKEN: u32 = 0xC8A7C0;
    pub const PHOTO_TAKEN_2: u32 = 0xC8A7C1;
    pub const SATCHEL_UPDATE: u32 = 0x738F3A; // `jmp [slot]` at the type-39 test
    pub const SATCHEL_UPDATE_SLOT: u32 = 0x738B1B;
    pub const SATCHEL_UPDATE_ORIGINAL: [u8; 6] = [0x8B, 0x4B, 0x04, 0x8A, 0x51, 0x36];
    pub const SATCHEL_UPDATE_RESUME: u32 = 0x738F40;
    pub const DETONATOR: u32 = 0x738877; // `jmp [slot]` at the type-39 test
    pub const DETONATOR_SLOT: u32 = 0x73885B;
    pub const DETONATOR_ORIGINAL: [u8; 6] = [0x83, 0x7E, 0xE0, 0x27, 0x8B, 0x7D];
    pub const DETONATOR_BLOW: u32 = 0x738880;
    pub const DETONATOR_SKIP: u32 = 0x7388DB;
    /// Lock-on functions patched to return at once (InitGame, lag-comp 1 and 2).
    pub const FIND_LOCK_ON: u32 = 0x60DC50;
    pub const FIND_NEXT_LOCK_ON: u32 = 0x60E530;
}

/// Expected bytes of a `jmp [slot]` pointer slot (gta_sa.exe padding before the function).
const SLOT_PADDING: [u8; 4] = [0x90; 4];

// ---- shared state (written by the client tick) -------------------------------------------------------------

/// A streamed remote player as the hooks see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteHook {
    pub ped: u32,
    pub id: u16,
    pub npc: bool,
    pub team: u8,
    /// The vehicle of the last stored driver/passenger sync.
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
    /// Pool objects: (GTA pointer, id), for the 206 hit type 3.
    pub objects: Vec<(u32, u16)>,
    /// The GTA entity of the surface in the local surf state (a pool vehicle or object), `None` when the state has
    /// no surface.
    pub surface: Option<u32>,
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

    /// Entity → player: a remote ped, else the first remote (pool order) whose synced vehicle is that pool
    /// vehicle. Mirrors samp.dll 0x100A3B20.
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

fn read_floats<const N: usize>(a: u32) -> [f32; N] {
    std::array::from_fn(|i| rd(a + 4 * i as u32))
}

fn write_floats(a: u32, v: &[f32]) {
    for (i, f) in v.iter().enumerate() {
        wr(a + 4 * i as u32, *f);
    }
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

mod damage;
mod install;
mod shot;
mod weapons;

pub use install::{install_init_game, install_main, install_vehicle_friendly_fire};
pub use shot::{fire_shot, selector};
pub use weapons::extinguish;
