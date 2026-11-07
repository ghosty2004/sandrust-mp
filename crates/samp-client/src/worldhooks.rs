//! World-entity hooks inside GTA (Windows only): samp's object render passes and materials, building-removal
//! replay, pickup collision, radar gang zones and trace colours, pinned actors, ped fog, objects attached to peds
//! and the camera-collision tests; [`install`] applies them. "Patch table row N" is line N (header = line 1) of
//! `docs/re/data/gta-patches-r4-annotated.csv`, the samp patch a hook reproduces. Spec:
//! `docs/re/subsystems/world-entities.md`; measured behaviour in `docs/phase3/trace-notes.md` ("World entities").
//!
//! The hooks never touch the client session: they read [`SHARED`] (written by `objects.rs` / `entities.rs` on the
//! main thread, never while calling into GTA) with `try_lock`, and queue [`Event`]s that the client tick handles.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::worldents::{self as world, FlashPhase, GangZone, MATERIAL_SLOTS};

/// GTA addresses (names via gta-reversed / plugin-sdk).
pub mod g {
    pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
    /// `TheCamera` matrix position (what samp's camera-distance helper reads).
    pub const CAMERA_POS: u32 = 0xB6F9CC;
    /// `CGame::currArea`.
    pub const CURR_AREA: u32 = 0xB72914;
    /// `CTimer::ms_fTimeStep`.
    pub const TIME_STEP: u32 = 0xB7CB5C;
    /// `CModelInfo::ms_modelInfoPtrs`.
    pub const MODEL_INFO_PTRS: u32 = 0xA9B0C8;
    /// `CPools::ms_pObjectPool`, element size sizeof(CObject).
    pub const OBJECT_POOL: u32 = 0xB7449C;
    pub const OBJECT_SIZE: u32 = 0x19C;
    pub const VTBL_OBJECT: u32 = 0x866F60;
    pub const VTBL_PLACEABLE: u32 = 0x863C40;
    /// `RwEngineInstance` (+0x20 render-state set).
    pub const RW_ENGINE: u32 = 0xC97B24;
    /// `Scene.m_pRwCamera`.
    pub const SCENE_CAMERA: u32 = 0xC1703C;
    /// `_RwD3DDevice`.
    pub const RW_D3D_DEVICE: u32 = 0xC97C28;
    /// `CMenuManager` "in menu" byte, passed as DrawAreaOnRadar's menu flag.
    pub const RADAR_IN_MENU: u32 = 0xBA67A4;
    // Functions.
    pub const ENTITY_CREATE_RW: u32 = 0x533D30;
    pub const ENTITY_DELETE_RW: u32 = 0x534030;
    pub const OBJECT_RENDER: u32 = 0x59F180;
    pub const PED_RENDER: u32 = 0x5E7680;
    pub const CIVILIAN_PROCESS_CONTROL: u32 = 0x5DDBF0;
    pub const RENDER_ONE_NON_ROAD: u32 = 0x553260;
    pub const RENDER_EVERYTHING_BAR_ROADS: u32 = 0x553AA0;
    pub const RENDER_FADING_IN: u32 = 0x5531E0;
    pub const DRAW_AREA_ON_RADAR: u32 = 0x5853D0;
    pub const DRAW_RADAR_GANG_OVERLAY: u32 = 0x586650;
    pub const POSITION_ATTACHED_ENTITY: u32 = 0x546FF0;
    pub const ESCALATOR_UPDATE: u32 = 0x717D30;
    pub const ROADSIGN_RENDER_ATOMIC: u32 = 0x6FF350;
    pub const CHECK_CAMERA_COLLISION_VEHICLES: u32 = 0x41A990;
    pub const CAMERA_TO_IGNORE_THIS_OBJECT: u32 = 0x563F40;
    pub const PROCESS_PEDS_AFTER_PRE_RENDER: u32 = 0x563430;
    pub const UPDATE_RW_MATRIX: u32 = 0x59AD70;
    pub const UPDATE_RW_FRAME: u32 = 0x532B00;
    pub const UPDATE_RP_HANIM: u32 = 0x532B20;
    pub const RP_CLUMP_FOR_ALL_ATOMICS: u32 = 0x749B70;
    pub const RP_CLUMP_RENDER: u32 = 0x749B20;
    pub const RW_CAMERA_FRUSTUM_TEST_SPHERE: u32 = 0x7EE2D0;
    pub const GET_ANIM_HIERARCHY_FROM_SKIN_CLUMP: u32 = 0x734A40;
    pub const RP_HANIM_ID_GET_INDEX: u32 = 0x7C51A0;
    // Fields.
    pub const ENTITY_RW_OBJECT: u32 = 0x18;
    pub const ENTITY_FLAGS: u32 = 0x1C;
    pub const ENTITY_MODEL: u32 = 0x22;
    pub const ENTITY_MATRIX: u32 = 0x14;
    pub const PHYSICAL_FLAGS: u32 = 0x40;
    pub const MOVE_SPEED: u32 = 0x44;
    pub const TURN_SPEED: u32 = 0x50;
    pub const MI_FLAGS: u32 = 0x12;
    pub const MI_DRAW_DISTANCE: u32 = 0x18;
    pub const MI_REF_COUNT: u32 = 0x08;
    pub const MI_COL_MODEL: u32 = 0x14;
    pub const PED_BONES: u32 = 0x488;
}

/// Entity flag samp sets on every object it creates. Mirrors samp.dll 0x1009F780.
pub const ENTITY_FLAG_CLIENT_OBJECT: u32 = 0x8000_0000;
/// Entity flag set by samp's lighting wrapper around a coloured object's draw. Mirrors samp.dll 0x100A9098.
pub const ENTITY_FLAG_LIGHTING: u32 = 0x1000_0000;

/// What a hook asks the client to do in the next tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The collision path reported pickup `id` (RPC 131, rel 9).
    PickupHit(i32),
}

/// A material slot as the render hooks apply it (samp keeps texture and colour at object +0x217 / +0x257;
/// world-entities.md, "Materials (RPC 44/84) at render time").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MatSlot {
    /// RwTexture* (0 = keep the model's texture).
    pub texture: u32,
    /// RGBA bytes as stored (0 = keep the model's colour).
    pub colour: u32,
}

/// One samp object (pool object or attached object) as the hooks see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct ObjInfo {
    pub ptr: u32,
    pub model: i32,
    /// samp wrapper +0x54.
    pub draw_distance: f32,
    pub has_materials: bool,
    /// samp wrapper +0x49: the RW object was created by samp's own streaming.
    pub drawn_by_samp: bool,
    /// samp wrapper +0x6C: model flag "draw last".
    pub draw_last: bool,
    /// Bounding sphere (+0x5C centre, +0x58 radius) and camera distance (+0x68).
    pub centre: [f32; 3],
    pub radius: f32,
    pub cam_dist: f32,
    pub materials: [MatSlot; MATERIAL_SLOTS],
    /// Material text slots exist (`ui::mattext`).
    pub has_text: bool,
    /// Object-to-object attachment with sync rotation 0 keeps its own rotation (patch table row 75).
    pub keep_rotation: Option<[f32; 3]>,
    /// An object of samp's object pool (not a player-attached slot); the camera test of patch table row 81 only
    /// checks these.
    pub pool_object: bool,
    /// Wrapper +0x53: no camera collision (RPC 44 field / RPC 169).
    pub no_camera_collision: bool,
}

/// A building removal (RPC 43).
#[derive(Clone, Copy, Debug)]
pub struct Removal {
    pub model: i32,
    pub centre: [f32; 3],
    pub radius: f32,
}

/// One player-attached object slot (RPC 113) placed per frame on a ped bone (patch table row 25).
#[derive(Clone, Copy, Debug)]
pub struct Attached {
    pub ped: u32,
    pub object: u32,
    pub bone: i32,
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Default)]
pub struct Shared {
    /// samp objects by `CObject*`, plus the pool order of the samp-drawn ones.
    pub objects: HashMap<u32, ObjInfo>,
    pub draw_order: Vec<u32>,
    pub zones: BTreeMap<u16, GangZone>,
    pub flash: FlashPhase,
    pub removals: Vec<Removal>,
    /// GTA pickup index → samp pickup id, and the per-id collision cooldown.
    pub pickups: HashMap<u32, i32>,
    pub cooldown: HashMap<i32, u32>,
    pub colours: Option<crate::ui::colours::PlayerColours>,
    pub attached: Vec<Attached>,
    pub events: Vec<Event>,
    /// The netgame exists (samp tests its netgame pointer in most hooks).
    pub active: bool,
}

pub static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

pub fn with_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Shared::default))
}

/// For hooks: never blocks (the client never calls into GTA while holding the lock, so this only fails if a hook
/// re-enters itself).
fn try_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    let mut g = match SHARED.try_lock() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    Some(f(g.get_or_insert_with(Shared::default)))
}

/// Diagnostics: objects drawn by our own passes, and objects the render-list filter kept out of GTA's list.
pub static PASS_DRAWS: AtomicU32 = AtomicU32::new(0);
pub static FILTERED: AtomicU32 = AtomicU32::new(0);
/// Diagnostics: radar gang-overlay hook calls, calls that found the shared state busy, zones drawn.
pub static ZONE_CALLS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

// ---- raw access ---------------------------------------------------------------------------------------

pub(crate) fn rd<T: Copy + Default>(a: u32) -> T {
    if a < 0x10000 {
        return T::default();
    }
    // SAFETY: callers pass addresses inside live game objects or game globals.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

pub(crate) fn wr<T: Copy>(a: u32, v: T) {
    if a < 0x10000 {
        return;
    }
    // SAFETY: callers pass writable addresses of live game objects / globals made writable at install.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) }
}

fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

fn thiscall0(f: u32, this: u32) -> u32 {
    // SAFETY: GTA thiscall function without arguments on a live object.
    unsafe {
        let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(f as usize);
        f(this)
    }
}

fn vcall0(this: u32, slot: u32) -> u32 {
    let vt: u32 = rd(this);
    thiscall0(rd(vt + slot), this)
}

// Patch table row 53 (ComputeDamageResponse) is installed once, by combathooks.rs: its rules include the actor
// rules (others hitting a CCivilianPed → cancel; local hitting an actor → RPC 177, cancel), fed with the actors
// entities.rs publishes every tick.

mod buildings;
mod collision;
mod entity;
mod fog;
mod install;
mod materials;
mod peds;
mod pickups;
mod radar;
mod render;

pub use entity::update_rw;
pub(crate) use fog::{samp_fog_render_state, set_preview_vehicle};
pub use install::install;
pub(crate) use install::jmp_indirect;
pub use materials::forget_material_rw;
pub use peds::bone_matrix;
pub use render::{RENDERING, model_info};
