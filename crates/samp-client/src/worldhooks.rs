//! World-entity hooks inside GTA (Windows only, milestone M6), table-driven from
//! `docs/re/data/gta-patches-r4-annotated.csv` (row = 1-based line number).
//!
//! Specs: world-entities.md RE-10 §2 (render-list filter, own render passes, Create/DeleteRwObject), §4 (materials
//! at render time), §5 (attached objects on peds), §6 (actors: pinned invulnerable actors, ped fog), §7 (building
//! removal replay), §8 (radar trace colours), §10 (attached-entity rotation, roadsign guard); remote-control-combat.md
//! §6.1 (ComputeDamageResponse: rules 0, 1, 2, 7, 8 — the player rules 3–6 and RPC 115 belong to M7); the
//! disassembly notes of samp A are cited per hook. TRACE-6 (trace-notes "World entities"): render paths by draw
//! distance (GTA list / samp pass after RenderEverythingBarRoads / after RenderFadingInEntities), bit-31 flags,
//! 1000-frame RW release, pickups via the collision path, gang zones per frame in id order.
//!
//! The hooks never touch the client session: they read [`SHARED`] (written by `objects.rs` / `entities.rs` on the
//! main thread, never while calling into GTA) with `try_lock`, and queue [`Event`]s that the client tick handles.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::world::{self, FlashPhase, GangZone, MATERIAL_SLOTS};

/// GTA addresses (names via gta-reversed / plugin-sdk).
pub mod g {
    pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
    /// `TheCamera` matrix position (samp's distance helper A 0x1009D760 reads it).
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
    pub const VTBL_CIVILIAN_PED: u32 = 0x86C0A8;
    pub const VTBL_PLACEABLE: u32 = 0x863C40;
    /// `RwEngineInstance` (+0x20 render-state set).
    pub const RW_ENGINE: u32 = 0xC97B24;
    /// `Scene.m_pRwCamera`.
    pub const SCENE_CAMERA: u32 = 0xC1703C;
    /// `_RwD3DDevice`.
    pub const RW_D3D_DEVICE: u32 = 0xC97C28;
    /// `CMenuManager` "in menu" byte DrawAreaOnRadar's flag comes from (A 0x100a1486).
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

/// Entity flag samp sets on every object it creates (RE-10 §2; A 0x1009F780).
pub const ENTITY_FLAG_CLIENT_OBJECT: u32 = 0x8000_0000;
/// Entity flag set by samp's lighting wrapper (A 0x100a9098).
pub const ENTITY_FLAG_LIGHTING: u32 = 0x1000_0000;

/// What a hook asks the client to do in the next tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The collision path reported pickup `id` (RPC 131, rel 9).
    PickupHit(i32),
    /// The local ped damaged an actor (RPC 177).
    ActorDamage { actor: u16, damage: f32, weapon: u32, part: u32 },
}

/// A material slot as the render hooks apply it (RE-10 §4: texture +0x217, colour +0x257).
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
    /// Object-to-object attachment with sync rotation 0 keeps its own rotation (row 75).
    pub keep_rotation: Option<[f32; 3]>,
}

/// A building removal (RPC 43).
#[derive(Clone, Copy, Debug)]
pub struct Removal {
    pub model: i32,
    pub centre: [f32; 3],
    pub radius: f32,
}

/// One player-attached object slot (RPC 113) placed per frame on a ped bone (row 25).
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
    /// Actor peds → actor id.
    pub actor_peds: HashMap<u32, u16>,
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

/// Diagnostics: calls of the own passes and the render filter.
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

fn local_ped() -> u32 {
    rd(g::CWORLD_PLAYERS)
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

/// `RwRenderStateSet(state, value)` through `RwEngineInstance` (+0x20), like samp's A 0x100c3e60.
fn rw_render_state(state: u32, value: u32) {
    let engine: u32 = rd(g::RW_ENGINE);
    let f: u32 = rd(engine + 0x20);
    if f == 0 {
        return;
    }
    // SAFETY: RenderWare's render-state setter (cdecl, state, value).
    unsafe {
        let f: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(f as usize);
        f(state, value);
    }
}

/// `IDirect3DDevice9::SetRenderState` on `_RwD3DDevice` (vtable +0xE4), like samp's fog setter A 0x100c3d70.
fn d3d_render_state(state: u32, value: u32) {
    let dev: u32 = rd(g::RW_D3D_DEVICE);
    if dev == 0 {
        return;
    }
    let vt: u32 = rd(dev);
    // SAFETY: COM call on the live device (stdcall this, state, value).
    unsafe {
        let f: extern "system" fn(u32, u32, u32) -> i32 = std::mem::transmute(rd::<u32>(vt + 0xE4) as usize);
        f(dev, state, value);
    }
}

/// samp's fog setter (A 0x100c3d70): FOGENABLE := on, FOGTABLEMODE := NONE, FOGVERTEXMODE := LINEAR.
fn set_fog(on: bool) {
    d3d_render_state(28, u32::from(on));
    d3d_render_state(35, 0);
    d3d_render_state(140, 3);
}

/// RenderWare's current fog state (`rwRENDERSTATEFOGENABLE` = 14 via `RwEngineInstance` +0x24 get). samp tracks
/// the same value from its own render-state hook (A 0x100c3dc0 → flag A 0x1026ea54).
fn rw_fog_enabled() -> bool {
    let engine: u32 = rd(g::RW_ENGINE);
    let f: u32 = rd(engine + 0x24);
    if f == 0 {
        return false;
    }
    let mut v: u32 = 0;
    // SAFETY: RenderWare's render-state getter (cdecl, state, out pointer).
    unsafe {
        let f: extern "C" fn(u32, *mut u32) -> u32 = std::mem::transmute(f as usize);
        f(14, &mut v);
    }
    v != 0
}

// ---- materials (RE-10 §4) -------------------------------------------------------------------------------

/// What the pre-pass changed, for the post-pass.
#[derive(Default)]
pub struct SavedMaterials {
    /// (material, texture, colour, ambient, specular, diffuse)
    mats: Vec<(u32, u32, u32, [u32; 3])>,
    /// (geometry, flags)
    geoms: Vec<(u32, u32)>,
    pub coloured: bool,
    /// A material-text substitution was armed for this render.
    pub text: bool,
}

struct ApplyCtx<'a> {
    slots: &'a [MatSlot; MATERIAL_SLOTS],
    saved: &'a mut SavedMaterials,
    /// Material-text slots: (slot, render target) in, (original D3D texture, render target) out.
    texts: &'a [(usize, u32)],
    pairs: Vec<(u32, u32)>,
}

extern "C" fn apply_atomic_cb(atomic: u32, data: *mut ApplyCtx) -> u32 {
    // SAFETY: `data` is the context passed to RpClumpForAllAtomics by `apply_materials`.
    let ctx = unsafe { &mut *data };
    apply_atomic(atomic, ctx);
    atomic
}

fn apply_atomic(atomic: u32, ctx: &mut ApplyCtx) {
    let geom: u32 = rd(atomic + 0x18);
    if geom == 0 {
        return;
    }
    let mats: u32 = rd(geom + 0x20);
    let n: i32 = rd(geom + 0x24);
    let flags: u32 = rd(geom + 8);
    let mut geom_saved = false;
    for i in 0..(n.max(0) as usize).min(MATERIAL_SLOTS) {
        if let Some(&(_, rt)) = ctx.texts.iter().find(|(s, _)| *s == i) {
            let m: u32 = rd(mats + 4 * i as u32);
            let orig = if m != 0 { crate::ui::mattext::d3d_texture_of(rd(m)) } else { 0 };
            if orig != 0 {
                ctx.pairs.push((orig, rt));
            }
        }
        let slot = ctx.slots[i];
        if slot.texture == 0 && slot.colour == 0 {
            continue;
        }
        let m: u32 = rd(mats + 4 * i as u32);
        if m == 0 {
            continue;
        }
        ctx.saved.mats.push((m, rd(m), rd(m + 4), [rd(m + 0xC), rd(m + 0x10), rd(m + 0x14)]));
        if slot.texture != 0 {
            wr(m, slot.texture);
        }
        if slot.colour != 0 {
            if !geom_saved {
                ctx.saved.geoms.push((geom, flags));
                geom_saved = true;
            }
            // MODULATEMATERIALCOLOR on, PRELIT off; surface properties (1, 0, 1); material colour = slot colour.
            let f: u32 = rd(geom + 8);
            wr(geom + 8, (f | 0x40) & !0x08);
            wr(m + 0xC, 1.0f32);
            wr(m + 0x10, 0.0f32);
            wr(m + 0x14, 1.0f32);
            wr(m + 4, slot.colour);
            ctx.saved.coloured = true;
        }
    }
}

/// Pre-pass (A 0x100A8C40, simplified to "save and apply every render"): slot textures and colours onto the RW
/// materials in geometry order.
pub fn apply_materials(entity: u32, slots: &[MatSlot; MATERIAL_SLOTS], texts: &[(usize, u32)]) -> SavedMaterials {
    let mut saved = SavedMaterials::default();
    let rw: u32 = rd(entity + g::ENTITY_RW_OBJECT);
    if rw == 0 || (texts.is_empty() && slots.iter().all(|s| s.texture == 0 && s.colour == 0)) {
        return saved;
    }
    let mut ctx = ApplyCtx { slots, saved: &mut saved, texts, pairs: Vec::new() };
    match rd::<u8>(rw) {
        1 => apply_atomic(rw, &mut ctx),
        2 => {
            // SAFETY: RpClumpForAllAtomics(clump, callback, data) on the live clump; the callback only touches
            // the context for the duration of the call.
            unsafe {
                let f: extern "C" fn(u32, extern "C" fn(u32, *mut ApplyCtx) -> u32, *mut ApplyCtx) -> u32 =
                    std::mem::transmute(g::RP_CLUMP_FOR_ALL_ATOMICS as usize);
                f(rw, apply_atomic_cb, &mut ctx);
            }
        }
        _ => {}
    }
    let pairs = std::mem::take(&mut ctx.pairs);
    if !pairs.is_empty() {
        saved.text = true;
        crate::ui::mattext::set_active(pairs);
    }
    saved
}

/// Post-pass (A 0x100A9EB0): restore what the pre-pass changed.
pub fn restore_materials(saved: &SavedMaterials) {
    if saved.text {
        crate::ui::mattext::set_active(Vec::new());
    }
    for &(m, tex, col, sp) in saved.mats.iter().rev() {
        wr(m, tex);
        wr(m + 4, col);
        wr(m + 0xC, sp[0]);
        wr(m + 0x10, sp[1]);
        wr(m + 0x14, sp[2]);
    }
    for &(geom, flags) in &saved.geoms {
        wr(geom + 8, flags);
    }
}

// ---- row 63: CObject::Render ----------------------------------------------------------------------------

extern "thiscall" fn object_render(this: u32) {
    let info = if rd::<u32>(this + g::ENTITY_FLAGS) & ENTITY_FLAG_CLIENT_OBJECT != 0 {
        try_shared(|s| s.objects.get(&this).copied()).flatten()
    } else {
        None
    };
    let saved = info.map(|i| {
        let texts = if i.has_text { crate::ui::mattext::textures(this) } else { Vec::new() };
        apply_materials(this, &i.materials, &texts)
    });
    thiscall0(g::OBJECT_RENDER, this);
    if let Some(s) = saved {
        restore_materials(&s);
    }
}

// ---- rows 67/68/69 + 372/373: Create/DeleteRwObject -----------------------------------------------------

/// `call [edx+0x20]` in `CStreaming::AddEntity` (row 372).
const ADD_ENTITY_CALL: u32 = 0x4096AA;
const ADD_ENTITY_CALL_BYTES: [u8; 3] = [0xFF, 0x52, 0x20];
/// `call CEscalator::SwitchOff` in `CEntity::RemoveEscalatorsForEntity` (row 373).
const ESCALATOR_SWITCH_OFF_CALL: u32 = 0x71791F;
const ESCALATOR_SWITCH_OFF_BYTES: [u8; 5] = [0xE8, 0x3C, 0xFF, 0xFF, 0xFF];
/// Escalator models (A 0x100a4bb8: 0xE02, 0xE9F, 0x2314, 0x2313).
const ESCALATOR_MODELS: [u16; 4] = [3586, 3743, 8980, 8979];

fn raw_patch(addr: u32, bytes: &[u8]) {
    // SAFETY: the range was made writable at install (unprotect) and is code no other thread executes now.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as usize as *mut u8, bytes.len()) }
}

/// Row 67 (A 0x100A4B30): `CEntity::CreateRwObject` with the AddEntity `call [edx+0x20]` NOPed for the call.
extern "thiscall" fn object_create_rw(this: u32) {
    raw_patch(ADD_ENTITY_CALL, &[0x90; 3]);
    thiscall0(g::ENTITY_CREATE_RW, this);
    raw_patch(ADD_ENTITY_CALL, &ADD_ENTITY_CALL_BYTES);
}

/// Rows 68/69 (A 0x100A4BA0): escalator models keep running when their RW object goes.
extern "thiscall" fn entity_delete_rw(this: u32) {
    let model: u16 = rd(this + g::ENTITY_MODEL);
    if ESCALATOR_MODELS.contains(&model) {
        raw_patch(ESCALATOR_SWITCH_OFF_CALL, &[0x90; 5]);
        thiscall0(g::ENTITY_DELETE_RW, this);
        raw_patch(ESCALATOR_SWITCH_OFF_CALL, &ESCALATOR_SWITCH_OFF_BYTES);
    } else {
        thiscall0(g::ENTITY_DELETE_RW, this);
    }
}

// ---- row 66 + 371: escalator speed constants -------------------------------------------------------------

const ESCALATOR_FLOATS: [(u32, f32); 2] = [(0x858BA4, 40.0), (0x858F84, 50.0)];

/// Row 66 (A 0x100A4C40): `CEscalator::Update` with the two constants raised to 40.0 / 50.0 for the call.
extern "thiscall" fn escalator_update(this: u32) {
    let saved = ESCALATOR_FLOATS.map(|(a, _)| rd::<u32>(a));
    for (a, v) in ESCALATOR_FLOATS {
        wr(a, v);
    }
    thiscall0(g::ESCALATOR_UPDATE, this);
    for ((a, _), v) in ESCALATOR_FLOATS.iter().zip(saved) {
        wr(*a, v);
    }
}

// ---- row 75: attached entity rotation (sync rotation 0) ----------------------------------------------------

/// Row 75 (A 0x100A4F10): after GTA positions an attached entity, an object attached to an object with sync
/// rotation 0 gets its own stored rotation back (A 0x100A7E60).
extern "thiscall" fn position_attached_entity(this: u32) -> u32 {
    let r = thiscall0(g::POSITION_ATTACHED_ENTITY, this);
    if rd::<u32>(this + g::ENTITY_FLAGS) & ENTITY_FLAG_CLIENT_OBJECT != 0
        && let Some(Some(rot)) = try_shared(|s| s.objects.get(&this).map(|o| o.keep_rotation)).flatten()
    {
        let m: u32 = rd(this + g::ENTITY_MATRIX);
        if m != 0 {
            let rows = world::rotation_rows(rot);
            for (i, row) in [rows.right, rows.forward, rows.up].iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    wr(m + 0x10 * i as u32 + 4 * j as u32, *v);
                }
            }
            update_rw(this);
        }
    }
    r
}

/// `CMatrix::UpdateRwMatrix` into the RW frame + `CEntity::UpdateRwFrame` (samp A 0x1009ECE0).
pub fn update_rw(ent: u32) {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    let rw: u32 = rd(ent + g::ENTITY_RW_OBJECT);
    if m == 0 || rw == 0 {
        return;
    }
    let frame: u32 = rd(rw + 4);
    if frame == 0 {
        return;
    }
    // SAFETY: CMatrix::UpdateRwMatrix(RwMatrix*) (thiscall on the entity matrix) and CEntity::UpdateRwFrame.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(g::UPDATE_RW_MATRIX as usize);
        f(m, frame + 0x10);
    }
    thiscall0(g::UPDATE_RW_FRAME, ent);
}

// ---- rows 76/77/78: render-list filter and samp's own passes -----------------------------------------------

/// Row 78 decision (A 0x100A5150): a samp object with materials, a draw distance > 300 or timed hours is not
/// added to GTA's render list.
extern "C" fn render_list_skip(entity: u32) -> u32 {
    if entity == 0
        || rd::<u32>(entity) != g::VTBL_OBJECT
        || rd::<u32>(entity + g::ENTITY_FLAGS) & ENTITY_FLAG_CLIENT_OBJECT == 0
    {
        return 0;
    }
    let skip = try_shared(|s| {
        s.active
            && s.objects.get(&entity).is_some_and(|o| world::drawn_by_samp(o.has_materials, o.draw_distance, false))
    })
    .unwrap_or(false);
    if skip {
        FILTERED.fetch_add(1, Ordering::Relaxed);
    }
    u32::from(skip)
}

/// `CRenderer::AddEntityToRenderList(CEntity*, float)` entry (cdecl). The original's first instructions
/// (`push esi; mov esi,[esp+8]; movsx eax, word [esi+0x22]`) run here, then it continues at 0x5534B9.
#[unsafe(naked)]
unsafe extern "C" fn add_entity_to_render_list_stub() {
    core::arch::naked_asm!(
        "push esi",
        "mov esi, [esp+8]",
        "pushad",
        "push esi",
        "call {skip}",
        "add esp, 4",
        "test eax, eax",
        "popad",
        "jz 2f",
        "pop esi",
        "ret",
        "2:",
        "movsx eax, word ptr [esi+0x22]",
        "push {cont}",
        "ret",
        skip = sym render_list_skip,
        cont = const 0x5534B9u32,
    )
}

fn camera_pos() -> [f32; 3] {
    v3(g::CAMERA_POS)
}

/// One manual draw (A 0x100A9FD0).
fn draw_object(o: &ObjInfo) {
    let ent = o.ptr;
    let rw: u32 = rd(ent + g::ENTITY_RW_OBJECT);
    if rw == 0 {
        return;
    }
    // Visibility: inside the bounding sphere → always; else the RW camera frustum test of the sphere.
    let dist = world::dist3(o.centre, camera_pos());
    if dist > o.radius {
        let cam: u32 = rd(g::SCENE_CAMERA);
        if cam == 0 {
            return;
        }
        let sphere = [o.centre[0], o.centre[1], o.centre[2], o.radius];
        // SAFETY: RwCameraFrustumTestSphere(camera, sphere) (cdecl) with a sphere on our stack.
        let r = unsafe {
            let f: extern "C" fn(u32, *const f32) -> u32 =
                std::mem::transmute(g::RW_CAMERA_FRUSTUM_TEST_SPHERE as usize);
            f(cam, sphere.as_ptr())
        };
        if r == 0 {
            return;
        }
    }
    let mi = model_info(o.model);
    let mflags: u16 = if mi != 0 { rd(mi + g::MI_FLAGS) } else { 0 };
    if o.draw_last {
        let alpha_ref = if rd::<u32>(g::CURR_AREA) == 0 && mflags & 0x8 == 0 { 100 } else { 0 };
        rw_render_state(30, alpha_ref);
    }
    rw_render_state(20, if mflags & 0x40 != 0 { 2 } else { 1 });
    // Material text render targets are created once, from this pass (TRACE-6 batch 3).
    let texts = if o.has_text { crate::ui::mattext::ensure(ent) } else { Vec::new() };
    let saved = o.has_materials.then(|| apply_materials(ent, &o.materials, &texts));
    set_fog(true);
    let coloured = saved.as_ref().is_some_and(|s| s.coloured);
    if coloured && o.cam_dist < world::DIRECT_RENDER_DISTANCE {
        // Lighting wrapper (A 0x100A9080): entity bit 28, SetupLighting; RW render; RemoveLighting(1).
        wr(ent + g::ENTITY_FLAGS, rd::<u32>(ent + g::ENTITY_FLAGS) | ENTITY_FLAG_LIGHTING);
        vcall0(ent, 0x4C);
        render_rw(rw);
        let vt: u32 = rd(ent);
        // SAFETY: the entity's RemoveLighting(bool) virtual (thiscall).
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(rd::<u32>(vt + 0x50) as usize);
            f(ent, 1);
        }
    } else if !coloured && o.cam_dist < world::DIRECT_RENDER_DISTANCE {
        // PreRender + CRenderer::RenderOneNonRoad (A 0x100A8FA0).
        vcall0(ent, 0x44);
        // SAFETY: CRenderer::RenderOneNonRoad(CEntity*) (cdecl).
        unsafe {
            let f: extern "C" fn(u32) = std::mem::transmute(g::RENDER_ONE_NON_ROAD as usize);
            f(ent);
        }
    } else {
        render_rw(rw);
    }
    if let Some(s) = saved {
        restore_materials(&s);
    }
    PASS_DRAWS.fetch_add(1, Ordering::Relaxed);
}

/// RW object render (A 0x100B6370): atomic → its render callback, clump → RpClumpRender.
fn render_rw(rw: u32) {
    match rd::<u8>(rw) {
        1 => {
            let cb: u32 = rd(rw + 0x48);
            if cb != 0 {
                thiscall0_cdecl(cb, rw);
            }
        }
        2 => thiscall0_cdecl(g::RP_CLUMP_RENDER, rw),
        _ => {}
    }
}

fn thiscall0_cdecl(f: u32, arg: u32) {
    // SAFETY: a RenderWare cdecl function taking one object pointer.
    unsafe {
        let f: extern "C" fn(u32) -> u32 = std::mem::transmute(f as usize);
        f(arg);
    }
}

pub fn model_info(model: i32) -> u32 {
    if !(0..20000).contains(&model) {
        return 0;
    }
    rd(g::MODEL_INFO_PTRS + 4 * model as u32)
}

fn samp_pass(draw_last: bool) {
    let list: Vec<ObjInfo> = match try_shared(|s| {
        if !s.active {
            return Vec::new();
        }
        s.draw_order
            .iter()
            .filter_map(|p| s.objects.get(p).copied())
            .filter(|o| o.drawn_by_samp && o.draw_last == draw_last)
            .collect()
    }) {
        Some(l) => l,
        None => return,
    };
    if !draw_last {
        for o in &list {
            draw_object(o);
        }
    } else {
        // Draw-last objects far → near (A 0x10012BD0).
        let mut l = list;
        l.sort_by(|a, b| b.cam_dist.partial_cmp(&a.cam_dist).unwrap_or(std::cmp::Ordering::Equal));
        for o in &l {
            draw_object(o);
        }
    }
}

/// Row 76 (A 0x100A50D0).
extern "C" fn after_render_everything_bar_roads() {
    // SAFETY: CRenderer::RenderEverythingBarRoads (cdecl, no arguments).
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::RENDER_EVERYTHING_BAR_ROADS as usize);
        f();
    }
    samp_pass(false);
}

/// Row 77 (A 0x100A5110).
extern "C" fn after_render_fading_in() {
    // SAFETY: CRenderer::RenderFadingInEntities (cdecl, no arguments).
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::RENDER_FADING_IN as usize);
        f();
    }
    samp_pass(true);
}

// ---- row 79: roadsign guard ---------------------------------------------------------------------------

#[unsafe(naked)]
unsafe extern "C" fn roadsign_guard_stub() {
    core::arch::naked_asm!(
        "mov eax, [esp+4]",
        "test eax, eax",
        "jz 2f",
        "push {orig}",
        "ret",
        "2:",
        "ret",
        orig = const g::ROADSIGN_RENDER_ATOMIC,
    )
}

// ---- rows 82/83: camera collision --------------------------------------------------------------------------

/// Row 82 (A 0x100A5B00): no camera collision with vehicles while RPC 167 disabled vehicle collisions.
extern "C" fn camera_collision_vehicles_off() -> u32 {
    u32::from(crate::vehhooks::with_shared(|s| s.disable_collisions))
}

#[unsafe(naked)]
unsafe extern "C" fn check_camera_collision_vehicles_stub() {
    core::arch::naked_asm!(
        "pushad",
        "call {off}",
        "test eax, eax",
        "popad",
        "jz 2f",
        "xor eax, eax",
        "ret",
        "2:",
        "push {orig}",
        "ret",
        off = sym camera_collision_vehicles_off,
        orig = const g::CHECK_CAMERA_COLLISION_VEHICLES,
    )
}

/// Row 83 (A 0x100A5B30): the camera ignores attached objects (`m_pAttachedTo` set).
#[unsafe(naked)]
unsafe extern "C" fn camera_ignore_object_stub() {
    core::arch::naked_asm!(
        "mov eax, [esp+4]",
        "test eax, eax",
        "jz 2f",
        "mov ecx, [eax+0xFC]",
        "cmp ecx, 0",
        "jne 3f",
        "2:",
        "push {orig}",
        "ret",
        "3:",
        "mov eax, 1",
        "ret",
        orig = const g::CAMERA_TO_IGNORE_THIS_OBJECT,
    )
}

// ---- row 65: building removal replay -----------------------------------------------------------------------

/// `CFileObjectInstance`: position +0, model id +0x1C (gta-reversed FileObjectInstance.h).
extern "C" fn load_object_instance_filter(inst: u32) {
    if inst == 0 {
        return;
    }
    let model: i32 = rd(inst + 0x1C);
    let pos = v3(inst);
    let hit =
        try_shared(|s| s.removals.iter().any(|r| world::removal_matches(r.model, r.centre, r.radius, model, pos)))
            .unwrap_or(false);
    if hit {
        wr(inst + 0x1C, world::REMOVED_BUILDING_MODEL);
    }
}

/// `CFileLoader::LoadObjectInstance(CFileObjectInstance*, const char*)` entry (row 65, A 0x100A4AB0): replays the
/// removals, then runs the overwritten `push -1; push 0x83C931` and continues at 0x538097.
#[unsafe(naked)]
unsafe extern "C" fn load_object_instance_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x24]",
        "call {f}",
        "add esp, 4",
        "popad",
        "push -1",
        "push 0x83C931",
        "push {cont}",
        "ret",
        f = sym load_object_instance_filter,
        cont = const 0x538097u32,
    )
}

// ---- row 44: pickup collision --------------------------------------------------------------------------------

/// A 0x100a3f80 → 0x10013440: GTA pickup index → samp id; report when live, cooldown 0 and not a dropped weapon.
extern "C" fn pickup_touched(pickup: u32) {
    let index = pickup.wrapping_sub(world::GTA_PICKUPS) / world::GTA_PICKUP_SIZE;
    try_shared(|s| {
        if !s.active {
            return;
        }
        if let Some(&id) = s.pickups.get(&index) {
            let cd = s.cooldown.entry(id).or_insert(0);
            if *cd == 0 {
                *cd = world::PICKUP_COOLDOWN;
                s.events.push(Event::PickupHit(id));
            }
        }
    });
}

#[unsafe(naked)]
unsafe extern "C" fn pickup_update_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push esi",
        "call {f}",
        "add esp, 4",
        "popad",
        "mov al, [esi+0x1C]",
        "cmp al, 6",
        "push {cont}",
        "ret",
        f = sym pickup_touched,
        cont = const 0x4579CBu32,
    )
}

// ---- row 51: radar trace colour --------------------------------------------------------------------------------

extern "C" fn radar_trace_colour(index: u32) -> u32 {
    try_shared(|s| {
        let c = s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new);
        world::radar_trace_colour(index, |id| c.rgba(id))
    })
    .unwrap_or_else(|| world::radar_trace_colour(index, |_| 0))
}

// ---- rows 41/42: gang zones ----------------------------------------------------------------------------------

/// Replaces the `CRadar::DrawRadarGangOverlay` calls (A 0x100A3F40 → 0x10002270): every shown zone, in id order,
/// through `CRadar::DrawAreaOnRadar(rect, colour, in_menu)` with the 500 ms flash phase.
extern "C" fn draw_gang_zones() {
    let now = tick_ms();
    ZONE_CALLS[0].fetch_add(1, Ordering::Relaxed);
    let list: Vec<([f32; 4], u32)> = try_shared(|s| {
        if !s.active {
            return Vec::new();
        }
        let phase = s.flash.update(now);
        s.zones.values().map(|z| (z.rect, z.colour_now(phase))).collect()
    })
    .unwrap_or_else(|| {
        ZONE_CALLS[1].fetch_add(1, Ordering::Relaxed);
        Vec::new()
    });
    ZONE_CALLS[2].fetch_add(list.len() as u32, Ordering::Relaxed);
    let in_menu = u32::from(rd::<u8>(g::RADAR_IN_MENU));
    for (rect, colour) in list {
        let crgba = world::crgba_bytes(colour);
        // SAFETY: CRadar::DrawAreaOnRadar(const CRect&, const CRGBA&, bool) (cdecl) with locals.
        unsafe {
            let f: extern "C" fn(*const f32, *const u8, u32) = std::mem::transmute(g::DRAW_AREA_ON_RADAR as usize);
            f(rect.as_ptr(), crgba.as_ptr(), in_menu);
        }
    }
}

pub fn tick_ms() -> u32 {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

// ---- row 20: CCivilianPed::ProcessControl ---------------------------------------------------------------------

/// Row 20 (A 0x100A2CA0): invulnerable actors (bullet + explosion + melee proof) cannot be pushed.
extern "thiscall" fn civilian_process_control(ped: u32) {
    let flags: u32 = rd(ped + g::PHYSICAL_FLAGS);
    let pinned = world::invulnerable_flags(flags);
    if pinned {
        wr(ped + g::MOVE_SPEED, 0.0f32);
        wr(ped + g::MOVE_SPEED + 4, 0.0f32);
    }
    let m: u32 = rd(ped + g::ENTITY_MATRIX);
    let saved = (m != 0).then(|| v3(m + 0x30));
    thiscall0(g::CIVILIAN_PROCESS_CONTROL, ped);
    if world::invulnerable_flags(rd(ped + g::PHYSICAL_FLAGS)) {
        let m: u32 = rd(ped + g::ENTITY_MATRIX);
        if let (Some(p), true) = (saved, m != 0) {
            wr(m + 0x30, p[0]);
            wr(m + 0x34, p[1]);
            wr(m + 0x38, p[2]);
        }
        wr(ped + g::MOVE_SPEED, 0.0f32);
        wr(ped + g::MOVE_SPEED + 4, 0.0f32);
    }
}

// ---- rows 11–15: ped Render fog ----------------------------------------------------------------------------------

/// Rows 11–15 (A 0x100A3E70): fog off for the ped draw, then back to RenderWare's fog state.
extern "thiscall" fn ped_render(ped: u32) {
    set_fog(false);
    thiscall0(g::PED_RENDER, ped);
    set_fog(rw_fog_enabled());
}

// ---- row 25: attached objects on peds (RE-10 §5) -------------------------------------------------------------

extern "C" fn process_peds_after_pre_render() {
    // SAFETY: CWorld::ProcessPedsAfterPreRender (cdecl, no arguments).
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::PROCESS_PEDS_AFTER_PRE_RENDER as usize);
        f();
    }
    let list = try_shared(|s| if s.active { s.attached.clone() } else { Vec::new() }).unwrap_or_default();
    let mut animated: Vec<u32> = Vec::new();
    for a in list {
        place_attached(a, &mut animated);
    }
}

fn place_attached(a: Attached, animated: &mut Vec<u32>) {
    let obj = a.object;
    if obj == 0 || rd::<u32>(obj) != g::VTBL_OBJECT {
        return;
    }
    let ped = a.ped;
    if ped == 0 || rd::<u32>(ped) == g::VTBL_PLACEABLE || rd::<u32>(ped + g::ENTITY_RW_OBJECT) == 0 {
        // samp moves the object to (0,0,0) when the entity is invalid.
        set_position(obj, [0.0; 3]);
        return;
    }
    if !animated.contains(&ped) {
        thiscall0(g::UPDATE_RP_HANIM, ped);
        animated.push(ped);
    }
    let Some(bone) = bone_matrix(ped, a.bone) else { return };
    // bone × offset, then rotate X, Y, Z (zero components skipped), then scale.
    let mut rows = [[bone[0], bone[1], bone[2]], [bone[4], bone[5], bone[6]], [bone[8], bone[9], bone[10]]];
    let bpos = [bone[12], bone[13], bone[14]];
    let pos =
        [0, 1, 2].map(|i| bpos[i] + rows[0][i] * a.offset[0] + rows[1][i] * a.offset[1] + rows[2][i] * a.offset[2]);
    for (axis, deg) in a.rotation.iter().enumerate() {
        if *deg != 0.0 {
            rows = rotate_local(rows, axis, deg.to_radians());
        }
    }
    for (i, s) in a.scale.iter().enumerate() {
        rows[i] = rows[i].map(|c| c * s);
    }
    vcall0(obj, 0x0C); // Remove()
    let m: u32 = rd(obj + g::ENTITY_MATRIX);
    if m != 0 {
        for (i, row) in rows.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                wr(m + 0x10 * i as u32 + 4 * j as u32, *v);
            }
        }
        wr(m + 0x30, pos[0]);
        wr(m + 0x34, pos[1]);
        wr(m + 0x38, pos[2]);
        update_rw(obj);
    }
    vcall0(obj, 0x08); // Add()
}

/// Rotation about the matrix's own axis `axis` (0 = right/X, 1 = forward/Y, 2 = up/Z).
fn rotate_local(r: [[f32; 3]; 3], axis: usize, a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    let (i, j) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let mut out = r;
    out[i] = [0, 1, 2].map(|k| r[i][k] * c + r[j][k] * s);
    out[j] = [0, 1, 2].map(|k| r[j][k] * c - r[i][k] * s);
    out
}

/// LTM of the ped bone `bone` (SA:MP bone id 1…18 = `ePedNode`): the frame data in `CPed::m_apBones` (+0x488)
/// gives the node id; its matrix comes from the skin clump's animation hierarchy.
fn bone_matrix(ped: u32, bone: i32) -> Option<[f32; 16]> {
    if !(1..=18).contains(&bone) {
        return None;
    }
    let frame_data: u32 = rd(ped + g::PED_BONES + 4 * bone as u32);
    if frame_data == 0 {
        return None;
    }
    let node_id: i32 = rd(frame_data + 0x14);
    let clump: u32 = rd(ped + g::ENTITY_RW_OBJECT);
    // SAFETY: GetAnimHierarchyFromSkinClump / RpHAnimIDGetIndex (cdecl) on the ped's live clump.
    let hier = unsafe {
        let f: extern "C" fn(u32) -> u32 = std::mem::transmute(g::GET_ANIM_HIERARCHY_FROM_SKIN_CLUMP as usize);
        f(clump)
    };
    if hier == 0 {
        return None;
    }
    let idx = unsafe {
        let f: extern "C" fn(u32, i32) -> i32 = std::mem::transmute(g::RP_HANIM_ID_GET_INDEX as usize);
        f(hier, node_id)
    };
    if idx < 0 {
        return None;
    }
    let mats: u32 = rd(hier + 8);
    if mats == 0 {
        return None;
    }
    let base = mats + 0x40 * idx as u32;
    Some(std::array::from_fn(|i| rd(base + 4 * i as u32)))
}

/// Entity matrix position (Remove, write, UpdateRW, Add — samp A 0x1009FD60).
pub fn set_position(ent: u32, p: [f32; 3]) {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    if m == 0 {
        return;
    }
    vcall0(ent, 0x0C);
    wr(m + 0x30, p[0]);
    wr(m + 0x34, p[1]);
    wr(m + 0x38, p[2]);
    update_rw(ent);
    vcall0(ent, 0x08);
}

// ---- row 53: ComputeDamageResponse (actor rules) ------------------------------------------------------------

/// Decision of remote-control-combat.md §6.1 for the rules M6 needs: 1 (damager ≠ local and victim is a
/// CCivilianPed → cancel) and 7 (damager local, victim an actor → RPC 177, cancel). 1 = cancel.
extern "C" fn damage_decision(calc: u32, victim: u32) -> u32 {
    let local = local_ped();
    let damager: u32 = rd(calc);
    try_shared(|s| {
        if !s.active {
            return 0;
        }
        if damager != local && rd::<u32>(victim) == g::VTBL_CIVILIAN_PED {
            return 1;
        }
        if damager == local
            && local != 0
            && let Some(&actor) = s.actor_peds.get(&victim)
        {
            s.events.push(Event::ActorDamage {
                actor,
                damage: rd(calc + 4),
                weapon: rd(calc + 0xC),
                part: rd(calc + 8),
            });
            return 1;
        }
        0
    })
    .unwrap_or(0)
}

#[unsafe(naked)]
unsafe extern "C" fn compute_damage_response_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x24]",
        "push ecx",
        "call {d}",
        "add esp, 8",
        "test eax, eax",
        "popad",
        "jnz 2f",
        "mov eax, fs:[0]",
        "push {cont}",
        "ret",
        "2:",
        "ret 0xC",
        d = sym damage_decision,
        cont = const 0x4B5AC6u32,
    )
}

// ---- install ------------------------------------------------------------------------------------------------------

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

/// samp's hook-jmp-indirect form: the handler address in `slot`, `jmp dword [slot]` (+ NOP padding) at `site`.
fn jmp_indirect(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    label: &'static str,
    site: u32,
    slot: u32,
    handler: u32,
    original: &[u8],
) -> Result<(), PatchError> {
    let mut b = vec![0xFF, 0x25];
    b.extend_from_slice(&slot.to_le_bytes());
    b.resize(original.len(), 0x90);
    let mut cur = vec![0u8; original.len()];
    mem.read(site, &mut cur)?;
    if cur != original {
        return Err(PatchError::Mismatch { addr: site, expected: original.to_vec(), found: cur });
    }
    patcher.write(mem, label, slot, &handler.to_le_bytes(), None)?;
    patcher.write(mem, label, site, &b, Some(original))
}

fn unprotect(addr: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    let mut old = 0u32;
    // SAFETY: changes the protection of a range of gta_sa.exe's image.
    unsafe { VirtualProtect(addr as usize as *const _, len, PAGE_EXECUTE_READWRITE, &mut old) != 0 }
}

/// `pickup visibility` constant samp points `CPickup::IsVisible`'s `fcomp` at (row 211, A .data 0x1011671c).
static PICKUP_VISIBLE_DISTANCE: f32 = 10000.0;

/// Byte rows (all init-main): (row, address, new bytes, original bytes, label).
/// (row, address, new bytes, original bytes, label).
type ByteRow = (u32, u32, &'static [u8], &'static [u8], &'static str);
/// (row, site, slot, handler, original bytes, label).
type IndirectRow = (u32, u32, u32, u32, &'static [u8], &'static str);

const BYTE_ROWS: &[ByteRow] = &[
    (113, 0x551097, &[0xB8, 0x0B, 0x00, 0x00], &[0x5E, 0x01, 0x00, 0x00], "CPools::Initialise object pool 350 -> 3000"),
    (208, 0x593D7C, &[0x90; 5], &[0xE8, 0x0F, 0xE2, 0xFF, 0xFF], "Interior_c::Init: no AddPickups"),
];

/// Installs every M6 row. Returns (row, result) per CSV row for the log.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    for &(row, addr, new, orig, label) in BYTE_ROWS {
        out.push((row, patcher.write(mem, label, addr, new, Some(orig))));
    }
    let vis = &PICKUP_VISIBLE_DISTANCE as *const f32 as u32;
    out.push((
        211,
        patcher.write(
            mem,
            "CPickup::IsVisible distance 10000",
            0x454CC9,
            &vis.to_le_bytes(),
            Some(&0x858628u32.to_le_bytes()),
        ),
    ));
    // Vtable slots.
    let render = object_render as extern "thiscall" fn(u32) as usize as u32;
    out.push((
        63,
        expect_ptr(0x866FA8, g::OBJECT_RENDER)
            .and_then(|()| patcher.swap_pointer(mem, "CObject::Render", 0x866FA8, render).map(|_| ())),
    ));
    let create = object_create_rw as extern "thiscall" fn(u32) as usize as u32;
    out.push((
        67,
        expect_ptr(0x866F7C, 0x59F110)
            .and_then(|()| patcher.swap_pointer(mem, "CObject::CreateRwObject", 0x866F7C, create).map(|_| ())),
    ));
    let delete = entity_delete_rw as extern "thiscall" fn(u32) as usize as u32;
    for (row, slot) in [(68, 0x866F80), (69, 0x8585E8)] {
        out.push((
            row,
            expect_ptr(slot, g::ENTITY_DELETE_RW)
                .and_then(|()| patcher.swap_pointer(mem, "DeleteRwObject", slot, delete).map(|_| ())),
        ));
    }
    let prc = ped_render as extern "thiscall" fn(u32) as usize as u32;
    for (row, slot) in [(11, 0x86D1B0), (12, 0x86C0F0), (13, 0x86C168), (14, 0x86C248), (15, 0x86C3A0)] {
        out.push((
            row,
            expect_ptr(slot, g::PED_RENDER)
                .and_then(|()| patcher.swap_pointer(mem, "ped Render", slot, prc).map(|_| ())),
        ));
    }
    let cpc = civilian_process_control as extern "thiscall" fn(u32) as usize as u32;
    out.push((
        20,
        expect_ptr(0x86C0D0, g::CIVILIAN_PROCESS_CONTROL)
            .and_then(|()| patcher.swap_pointer(mem, "CCivilianPed::ProcessControl", 0x86C0D0, cpc).map(|_| ())),
    ));
    // Call sites.
    let calls: [(u32, u32, u32, u32, &'static str); 10] = [
        (
            25,
            0x53EA03,
            process_peds_after_pre_render as extern "C" fn() as usize as u32,
            g::PROCESS_PEDS_AFTER_PRE_RENDER,
            "Idle ProcessPedsAfterPreRender",
        ),
        (
            41,
            0x5869BF,
            draw_gang_zones as extern "C" fn() as usize as u32,
            g::DRAW_RADAR_GANG_OVERLAY,
            "DrawRadarMap gang overlay",
        ),
        (
            42,
            0x5759E4,
            draw_gang_zones as extern "C" fn() as usize as u32,
            g::DRAW_RADAR_GANG_OVERLAY,
            "PrintMap gang overlay",
        ),
        (
            66,
            0x718599,
            escalator_update as extern "thiscall" fn(u32) as usize as u32,
            g::ESCALATOR_UPDATE,
            "CEscalators::Update",
        ),
        (
            75,
            0x5648D3,
            position_attached_entity as extern "thiscall" fn(u32) -> u32 as usize as u32,
            g::POSITION_ATTACHED_ENTITY,
            "ProcessAttachedEntities",
        ),
        (
            76,
            0x53DFDD,
            after_render_everything_bar_roads as extern "C" fn() as usize as u32,
            g::RENDER_EVERYTHING_BAR_ROADS,
            "RenderScene pass 1",
        ),
        (
            77,
            0x53E019,
            after_render_fading_in as extern "C" fn() as usize as u32,
            g::RENDER_FADING_IN,
            "RenderScene pass 2",
        ),
        (
            79,
            0x5342F9,
            roadsign_guard_stub as unsafe extern "C" fn() as usize as u32,
            g::ROADSIGN_RENDER_ATOMIC,
            "RenderEffects roadsign guard",
        ),
        (
            82,
            0x41AF80,
            check_camera_collision_vehicles_stub as unsafe extern "C" fn() as usize as u32,
            g::CHECK_CAMERA_COLLISION_VEHICLES,
            "camera collision vehicles",
        ),
        (
            83,
            0x41AB78,
            camera_ignore_object_stub as unsafe extern "C" fn() as usize as u32,
            g::CAMERA_TO_IGNORE_THIS_OBJECT,
            "camera ignores attached objects",
        ),
    ];
    for (row, site, target, orig, label) in calls {
        out.push((row, patcher.retarget_branch(mem, label, site, target, Some(orig)).map(|_| ())));
    }
    // Row 371: the escalator floats are written around each call (made writable once here).
    out.push((
        371,
        if ESCALATOR_FLOATS.iter().all(|(a, _)| unprotect(*a, 4)) {
            Ok(())
        } else {
            Err(PatchError::Mismatch { addr: 0x858BA4, expected: vec![], found: vec![] })
        },
    ));
    // Rows 372/373: temporary NOPs around the calls (made writable once here).
    let ok372 = unprotect(ADD_ENTITY_CALL, 3) && ProcessMemory.read_u8(ADD_ENTITY_CALL).ok() == Some(0xFF);
    out.push((
        372,
        if ok372 {
            Ok(())
        } else {
            Err(PatchError::Mismatch { addr: ADD_ENTITY_CALL, expected: ADD_ENTITY_CALL_BYTES.to_vec(), found: vec![] })
        },
    ));
    let ok373 =
        unprotect(ESCALATOR_SWITCH_OFF_CALL, 5) && ProcessMemory.read_u8(ESCALATOR_SWITCH_OFF_CALL).ok() == Some(0xE8);
    out.push((
        373,
        if ok373 {
            Ok(())
        } else {
            Err(PatchError::Mismatch {
                addr: ESCALATOR_SWITCH_OFF_CALL,
                expected: ESCALATOR_SWITCH_OFF_BYTES.to_vec(),
                found: vec![],
            })
        },
    ));
    // jmp-indirect entries.
    let ind: [IndirectRow; 4] = [
        (
            51,
            0x584770,
            0x584A79,
            radar_trace_colour as extern "C" fn(u32) -> u32 as usize as u32,
            &[0x8B, 0x44, 0x24, 0x04, 0x83, 0xF8, 0x08],
            "CRadar::GetRadarTraceColour",
        ),
        (
            53,
            0x4B5AC0,
            0x4B5ABC,
            compute_damage_response_stub as unsafe extern "C" fn() as usize as u32,
            &[0x64, 0xA1, 0x00, 0x00, 0x00, 0x00],
            "ComputeDamageResponse",
        ),
        (
            65,
            0x538090,
            0x538084,
            load_object_instance_stub as unsafe extern "C" fn() as usize as u32,
            &[0x6A, 0xFF, 0x68, 0x31, 0xC9, 0x83, 0x00],
            "CFileLoader::LoadObjectInstance",
        ),
        (
            78,
            0x5534B0,
            0x5534A6,
            add_entity_to_render_list_stub as unsafe extern "C" fn() as usize as u32,
            &[0x56, 0x8B, 0x74, 0x24, 0x08, 0x0F, 0xBF, 0x46, 0x22],
            "CRenderer::AddEntityToRenderList",
        ),
    ];
    for (row, site, slot, handler, orig, label) in ind {
        out.push((row, jmp_indirect(patcher, mem, label, site, slot, handler, orig)));
    }
    // Row 44: the pickup-collision jmp over `mov al,[esi+0x1C]; cmp al,6`.
    let pk = pickup_update_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        44,
        patcher.hook_jmp(mem, "CPickup::Update collision", 0x4579C6, pk, Some(&[0x8A, 0x46, 0x1C, 0x3C, 0x06])),
    ));
    with_shared(|_| ());
    out
}
