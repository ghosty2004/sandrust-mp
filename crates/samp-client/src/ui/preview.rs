//! Model-preview textdraws (font 5, `TEXT_DRAW_FONT_MODEL_PREVIEW`). Windows only.
//!
//! A camera set is built once on the first frame (`init_env`). After every `CGame::Process`, each preview
//! textdraw without a texture is rendered once off-screen into its own 256×256 texture (`pending` +
//! `render_pending`): a ped, a vehicle or an object, by model id. Until then the textdraw is drawn as text.
//! Textures are freed with their textdraw and on device loss (`on_lost`), so they are rendered again.
//! Spec: docs/re/subsystems/ui-rendering.md, "14.3a Model previews (font 5)"; addresses in
//! docs/re/addresses/ui-preview.md.

use std::collections::BTreeMap;
use std::sync::Mutex;

use samp_ui::textdraw::{self, TextDraws};

use crate::script::{Arg, Runner, op};
use crate::worldhooks::{rd, wr};

/// GTA / RenderWare 1.0 US (gta-reversed names).
mod g {
    pub const RW_RASTER_CREATE: usize = 0x7FB230;
    pub const RW_TEXTURE_CREATE: usize = 0x7F37C0;
    pub const RP_LIGHT_CREATE: usize = 0x752110;
    pub const RP_LIGHT_SET_COLOR: usize = 0x751A90;
    pub const RW_CAMERA_CREATE: usize = 0x7EE4F0;
    pub const RW_FRAME_CREATE: usize = 0x7F0410;
    pub const RW_FRAME_TRANSLATE: usize = 0x7F0E30;
    pub const RW_FRAME_ROTATE: usize = 0x7F1010;
    pub const RW_OBJECT_SET_FRAME: usize = 0x804EF0;
    pub const RW_CAMERA_SET_FAR: usize = 0x7EE2A0;
    pub const RW_CAMERA_SET_NEAR: usize = 0x7EE1D0;
    pub const RW_CAMERA_SET_VIEW_WINDOW: usize = 0x7EE410;
    pub const RW_CAMERA_SET_PROJECTION: usize = 0x7EE3A0;
    pub const RP_WORLD_ADD_CAMERA: usize = 0x750F20;
    pub const SET_RENDERWARE_CAMERA: usize = 0x7328C0;
    pub const RW_CAMERA_CLEAR: usize = 0x7EE340;
    pub const RW_CAMERA_BEGIN_UPDATE: usize = 0x7EE190;
    pub const RW_CAMERA_END_UPDATE: usize = 0x7EE180;
    pub const RP_WORLD_ADD_LIGHT: usize = 0x751910;
    pub const RP_WORLD_REMOVE_LIGHT: usize = 0x751960;
    pub const RW_MATRIX_ROTATE: usize = 0x7F1FD0;
    pub const ANIM_BLEND_UPDATE: usize = 0x4D34F0;
    pub const SETUP_VEHICLE_VARIABLES: usize = 0x733160;
    pub const RENDER_ONE_NON_ROAD: usize = 0x553260;
    pub const WORLD_ADD: usize = 0x563220;
    pub const MATRIX_UPDATE_RW: usize = 0x59AD70;
    pub const ENTITY_UPDATE_RW_FRAME: usize = 0x532B00;
    pub const RP_CLUMP_RENDER: usize = 0x749B20;
    pub const RP_ATOMIC_DESTROY: usize = 0x749DC0;
    pub const RW_FRAME_DESTROY: usize = 0x7F05A0;
    /// `CSprite2d::Delete`; on a one-pointer sprite this is `RwTextureDestroy`.
    pub const SPRITE_DELETE: usize = 0x7281E0;
    /// `Scene.m_pRpWorld`.
    pub const SCENE_WORLD: u32 = 0xC17038;
    /// `RwEngineInstance` (+0x20 `fpRenderStateSet`).
    pub const RW_ENGINE: u32 = 0xC97B24;
    pub const MODEL_INFO_PTRS: u32 = 0xA9B0C8;
    /// `CPedModelInfo` vtable (how samp tells a ped model apart).
    pub const VT_PED_MODEL_INFO: u32 = 0x85BDC0;
    pub const VT_CPLACEABLE: u32 = 0x863C40;
}

/// CEntity fields.
mod e {
    pub const MATRIX: u32 = 0x14;
    pub const RW_OBJECT: u32 = 0x18;
    pub const FLAGS: u32 = 0x1C;
    pub const MODEL: u32 = 0x22;
    pub const MOVE_SPEED: u32 = 0x44;
    pub const MOVING_LIST: u32 = 0xB4;
    /// vtable slots (byte offsets): Add, Remove, Teleport, PreRender.
    pub const VT_ADD: u32 = 0x08;
    pub const VT_REMOVE: u32 = 0x0C;
    pub const VT_TELEPORT: u32 = 0x38;
    pub const VT_PRE_RENDER: u32 = 0x44;
}

/// Preview render size and camera-texture raster flags (`rwRASTERTYPECAMERATEXTURE | rwRASTERFORMAT8888`).
const SIZE: i32 = 256;
const RASTER_CAMERA_TEXTURE: i32 = 0x505;
const RASTER_ZBUFFER: i32 = 0x1;
/// Size of samp's texture slot table, shared by sprite textdraws and previews.
const TEXTURE_SLOTS: usize = 200;
/// The object samp renders for an unknown model.
const FALLBACK_OBJECT: i32 = 18631;
/// The one skin samp renders 0.1 higher (z 50.15 instead of 50.05).
const TALL_SKIN: i32 = 162;

/// The parts of samp's preview camera set we use again after construction. Mirrors samp.dll 0x1006BE40.
#[derive(Clone, Copy, Default)]
struct Env {
    camera: u32,
    light: u32,
    zraster: u32,
}

struct State {
    env: Option<Env>,
    /// Textdraw id → (textdraw serial, RwTexture*).
    textures: BTreeMap<u16, (u64, u32)>,
}

static STATE: Mutex<State> = Mutex::new(State { env: None, textures: BTreeMap::new() });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

// ---- calls ---------------------------------------------------------------------------------------------------------

fn c1(f: usize, a: u32) -> u32 {
    // SAFETY: a RenderWare / GTA cdecl function taking one pointer-sized argument, main thread.
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32) -> u32>(f)(a) }
}

fn c2(f: usize, a: u32, b: u32) -> u32 {
    // SAFETY: as `c1`, two pointer-sized arguments.
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32, u32) -> u32>(f)(a, b) }
}

fn c3(f: usize, a: u32, b: u32, c: u32) -> u32 {
    // SAFETY: as `c1`, three pointer-sized arguments.
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32, u32, u32) -> u32>(f)(a, b, c) }
}

fn c_obj_f(f: usize, obj: u32, v: f32) -> u32 {
    // SAFETY: a RenderWare cdecl setter (object, float).
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32, f32) -> u32>(f)(obj, v) }
}

fn rotate(f: usize, obj: u32, axis: &[f32; 3], angle: f32) {
    // SAFETY: RwFrameRotate / RwMatrixRotate (object, const RwV3d* axis, RwReal angle, RwOpCombineType), cdecl.
    unsafe {
        std::mem::transmute::<usize, extern "C" fn(u32, *const f32, f32, u32) -> u32>(f)(obj, axis.as_ptr(), angle, 1);
    }
}

fn thiscall0(f: u32, this: u32) {
    // SAFETY: a GTA thiscall function / virtual without arguments on a live object.
    unsafe { std::mem::transmute::<usize, extern "thiscall" fn(u32)>(f as usize)(this) }
}

fn vcall0(this: u32, slot: u32) {
    let vt: u32 = rd(this);
    thiscall0(rd(vt + slot), this);
}

/// `RwRenderStateSet` through `RwEngineInstance`.
fn render_state(state: u32, value: u32) {
    let engine: u32 = rd(g::RW_ENGINE);
    let f: u32 = rd(engine + 0x20);
    if f != 0 {
        c2(f as usize, state, value);
    }
}

const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn model_info(model: i32) -> u32 {
    if !(0..20000).contains(&model) {
        return 0;
    }
    rd(g::MODEL_INFO_PTRS + 4 * model as u32)
}

fn is_ped_model(model: i32) -> bool {
    let mi = model_info(model);
    mi != 0 && rd::<u32>(mi) == g::VT_PED_MODEL_INFO
}

/// `CSprite2d` delete on a temporary sprite: `RwTextureDestroy`, which releases the raster too.
/// Mirrors samp.dll 0x100B39A0.
fn destroy_texture(tex: u32) {
    if tex != 0 {
        let mut s = tex;
        // SAFETY: CSprite2d::Delete (thiscall) on a one-pointer sprite holding our texture reference.
        unsafe { std::mem::transmute::<usize, extern "thiscall" fn(*mut u32)>(g::SPRITE_DELETE)(&mut s) };
    }
}

// ---- environment ---------------------------------------------------------------------------------------------------

/// Builds the preview camera set once, on the first frame: an ambient white light, a camera at (0, 0, 50)
/// turned 90° about X (far 300, near 0.01, view window 0.5 × 0.5, perspective) with a 256×256 Z raster.
pub fn init_env() {
    if with(|s| s.env.is_some()) {
        return;
    }
    let light = c1(g::RP_LIGHT_CREATE, 2);
    if light == 0 {
        return;
    }
    let white = [1.0f32; 4];
    c2(g::RP_LIGHT_SET_COLOR, light, white.as_ptr() as u32);
    let raster = create_raster(RASTER_CAMERA_TEXTURE);
    let zraster = create_raster(RASTER_ZBUFFER);
    let camera = c1(g::RW_CAMERA_CREATE, 0);
    let frame = c1(g::RW_FRAME_CREATE, 0);
    let pos = [0.0f32, 0.0, 50.0];
    c3(g::RW_FRAME_TRANSLATE, frame, pos.as_ptr() as u32, 1);
    rotate(g::RW_FRAME_ROTATE, frame, &AXES[0], 90.0);
    if camera == 0 || frame == 0 || raster == 0 || zraster == 0 {
        crate::log!(
            "preview: camera set incomplete (camera {camera:#x} frame {frame:#x} raster {raster:#x} z {zraster:#x})"
        );
        return;
    }
    set_camera_rasters(camera, raster, zraster);
    c2(g::RW_OBJECT_SET_FRAME, camera, frame);
    c_obj_f(g::RW_CAMERA_SET_FAR, camera, 300.0);
    c_obj_f(g::RW_CAMERA_SET_NEAR, camera, 0.01);
    let window = [0.5f32, 0.5];
    c2(g::RW_CAMERA_SET_VIEW_WINDOW, camera, window.as_ptr() as u32);
    c2(g::RW_CAMERA_SET_PROJECTION, camera, 1);
    let world: u32 = rd(g::SCENE_WORLD);
    if world != 0 {
        c2(g::RP_WORLD_ADD_CAMERA, world, camera);
    }
    // samp also creates a texture for the camera set's raster and never uses it; done for parity.
    c1(g::RW_TEXTURE_CREATE, raster);
    with(|s| s.env = Some(Env { camera, light, zraster }));
    crate::log!("preview: camera set ready (camera {camera:#x})");
}

fn create_raster(flags: i32) -> u32 {
    // SAFETY: RwRasterCreate(width, height, depth, flags), cdecl.
    unsafe {
        std::mem::transmute::<usize, extern "C" fn(i32, i32, i32, i32) -> u32>(g::RW_RASTER_CREATE)(
            SIZE, SIZE, 0, flags,
        )
    }
}

/// `RwCameraSetRaster` / `RwCameraSetZRaster`: samp writes the camera's +0x60 / +0x64 fields directly.
fn set_camera_rasters(camera: u32, raster: u32, zraster: u32) {
    wr(camera + 0x60, raster);
    wr(camera + 0x64, zraster);
}

// ---- textures ------------------------------------------------------------------------------------------------------

/// The preview texture of textdraw `id` when it belongs to that textdraw object.
pub fn texture(id: u16, serial: u64) -> Option<u32> {
    with(|s| s.textures.get(&id).filter(|(sr, _)| *sr == serial).map(|(_, t)| *t))
}

/// Frees the textures of textdraws that are gone or were re-created (samp frees them with the textdraw object).
fn sync(pool: &TextDraws) -> Vec<u32> {
    with(|s| {
        let mut gone = Vec::new();
        s.textures.retain(|id, (serial, tex)| {
            let keep = pool.get(*id).is_some_and(|t| t.serial == *serial && t.data.style == textdraw::FONT_PREVIEW);
            if !keep {
                gone.push(*tex);
            }
            keep
        });
        gone
    })
}

/// Device lost: every preview texture is released; the next game frame renders them again.
/// Mirrors samp.dll 0x1001E8A0.
pub fn on_lost() {
    let all: Vec<u32> = with(|s| std::mem::take(&mut s.textures).into_values().map(|(_, t)| t).collect());
    for t in &all {
        destroy_texture(*t);
    }
    if !all.is_empty() {
        crate::log!("preview: device lost: {} preview textures released", all.len());
    }
}

/// Called from the game-frame hook after `CGame::Process`: every model preview that has no texture, in id order
/// (textures of gone textdraws are freed first). `sprites` is the number of textures held by sprite textdraws,
/// which share samp's 200 slots. Mirrors samp.dll 0x1001E870.
pub fn pending(pool: &TextDraws, sprites: usize) -> Vec<Pending> {
    for t in sync(pool) {
        destroy_texture(t);
    }
    let held = with(|s| s.textures.len()) + sprites;
    pool.iter()
        .filter(|(id, t)| t.data.style == textdraw::FONT_PREVIEW && texture(*id, t.serial).is_none())
        .take(TEXTURE_SLOTS.saturating_sub(held))
        .map(|(id, t)| {
            let d = &t.data;
            Pending {
                id,
                serial: t.serial,
                req: Request {
                    model: i32::from(d.preview_model as i16),
                    background: d.background_colour.to_le_bytes(),
                    rot: [d.preview_rot.x, d.preview_rot.y, d.preview_rot.z],
                    zoom: d.preview_zoom,
                    colours: (i32::from(d.preview_colour1), i32::from(d.preview_colour2)),
                },
            }
        })
        .collect()
}

/// A preview to render. Collected under the UI lock and rendered without it, because a device loss inside the
/// render runs the Reset hook, which needs the UI state.
pub struct Pending {
    id: u16,
    serial: u64,
    req: Request,
}

/// The preview renders' script runner.
static RUNNER: Mutex<Option<Runner>> = Mutex::new(None);

/// Renders the pending previews in id order; the textures are kept for their textdraw objects.
pub fn render_pending(list: Vec<Pending>) {
    if list.is_empty() {
        return;
    }
    let Some(env) = with(|s| s.env) else { return };
    let mut g = RUNNER.lock().unwrap_or_else(|e| e.into_inner());
    let runner = g.get_or_insert_with(Runner::new);
    for p in list {
        let tex = render(runner, &env, &p.req);
        if tex != 0 {
            with(|s| s.textures.insert(p.id, (p.serial, tex)));
        }
    }
}

struct Request {
    model: i32,
    /// `RwRGBA` bytes of the textdraw's background colour, as received.
    background: [u8; 4],
    rot: [f32; 3],
    zoom: f32,
    colours: (i32, i32),
}

/// Picks the renderer by model id (ped, vehicle 400..611, object / weapon 321..373 or ≥ 615, negative ids),
/// with samp's model substitutions. Returns 0 when nothing was rendered; the textdraw then stays on the text
/// path. Mirrors samp.dll 0x100B34E0.
fn render(runner: &mut Runner, env: &Env, r: &Request) -> u32 {
    let model = r.model;
    if is_ped_model(model) {
        return render_ped(runner, env, r, model);
    }
    if (400..=611).contains(&model) {
        return render_vehicle(runner, env, r, model);
    }
    let object = if (321..=373).contains(&model) || model >= 615 {
        if model_info(model) != 0 { model } else { FALLBACK_OBJECT }
    } else if model < 0 {
        // 0.3.7-R4 has no custom-model table, so a negative id is always unknown.
        FALLBACK_OBJECT
    } else {
        return 0;
    };
    render_object(runner, env, r, object)
}

/// `RwRasterCreate` + `RwTextureCreate` of one preview (each render has its own raster).
fn new_target() -> (u32, u32) {
    let raster = create_raster(RASTER_CAMERA_TEXTURE);
    let tex = if raster != 0 { c1(g::RW_TEXTURE_CREATE, raster) } else { 0 };
    (raster, tex)
}

/// Camera on the new raster, cleared to the background colour, update begun, light added, then samp's render
/// states: ZTEST 1, ZWRITE 1, SHADEMODE 2, then `extra`, then FOG 0.
fn begin(env: &Env, raster: u32, bg: &[u8; 4], extra: &[(u32, u32)]) {
    set_camera_rasters(env.camera, raster, env.zraster);
    c1(g::SET_RENDERWARE_CAMERA, env.camera);
    c3(g::RW_CAMERA_CLEAR, env.camera, bg.as_ptr() as u32, 3);
    c1(g::RW_CAMERA_BEGIN_UPDATE, env.camera);
    let world: u32 = rd(g::SCENE_WORLD);
    if world != 0 {
        c2(g::RP_WORLD_ADD_LIGHT, world, env.light);
    }
    render_state(6, 1);
    render_state(8, 1);
    render_state(7, 2);
    for &(s, v) in extra {
        render_state(s, v);
    }
    // FOG 0 goes through samp's render-state hook: it sets D3D fog itself and passes FALSE to RenderWare.
    crate::worldhooks::samp_fog_render_state(false);
}

fn end(env: &Env) {
    c1(g::RW_CAMERA_END_UPDATE, env.camera);
    let world: u32 = rd(g::SCENE_WORLD);
    if world != 0 {
        c2(g::RP_WORLD_REMOVE_LIGHT, world, env.light);
    }
}

/// samp's entity-wrapper position set: `Teleport(x, y, z, false)`.
fn set_position(ent: u32, pos: [f32; 3]) {
    let vt: u32 = rd(ent);
    if vt == 0 || vt == g::VT_CPLACEABLE {
        return;
    }
    let f: u32 = rd(vt + e::VT_TELEPORT);
    // SAFETY: CEntity::Teleport (thiscall: x, y, z, bool) on a live entity.
    unsafe {
        std::mem::transmute::<usize, extern "thiscall" fn(u32, f32, f32, f32, u32)>(f as usize)(
            ent, pos[0], pos[1], pos[2], 0,
        )
    };
}

/// The flags samp gives a preview entity: |= 0x80000002, then bit 0 cleared.
fn preview_flags(ent: u32) {
    let f: u32 = rd(ent + e::FLAGS);
    wr(ent + e::FLAGS, f | 0x8000_0002);
    let f: u32 = rd(ent + e::FLAGS);
    wr(ent + e::FLAGS, f & !1);
}

/// Turns the entity's matrix by `rot`: copied into a zeroed RwMatrix, `RwMatrixRotate` pre-concat for each
/// non-zero axis, then written back the way samp does it (Remove, rows, `CMatrix::UpdateRW`, `UpdateRwFrame`, Add).
fn rotate_entity(ent: u32, rot: [f32; 3]) {
    let m: u32 = rd(ent + e::MATRIX);
    if m == 0 {
        return;
    }
    let mut local = [0f32; 16];
    for o in [0u32, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14] {
        local[o as usize] = rd(m + 4 * o);
    }
    for (axis, &angle) in rot.iter().enumerate() {
        if angle != 0.0 {
            rotate(g::RW_MATRIX_ROTATE, local.as_mut_ptr() as u32, &AXES[axis], angle);
        }
    }
    vcall0(ent, e::VT_REMOVE);
    let m: u32 = rd(ent + e::MATRIX);
    for o in [0u32, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14] {
        wr(m + 4 * o, local[o as usize]);
    }
    let vt: u32 = rd(ent);
    let rw: u32 = rd(ent + e::RW_OBJECT);
    if vt != g::VT_CPLACEABLE && rw != 0 {
        let frame: u32 = rd(rw + 4);
        // SAFETY: CMatrix::UpdateRW(RwMatrix*) (thiscall) on the entity's matrix with its frame's matrix.
        unsafe {
            std::mem::transmute::<usize, extern "thiscall" fn(u32, u32)>(g::MATRIX_UPDATE_RW)(m, frame + 0x10);
        }
        thiscall0(g::ENTITY_UPDATE_RW_FRAME as u32, ent);
    }
    vcall0(ent, e::VT_ADD);
}

/// samp's entity-wrapper "add": an entity outside the moving list gets its speeds zeroed, `CWorld::Add`, and is
/// teleported to its own position.
fn add_to_world(ent: u32) {
    let vt: u32 = rd(ent);
    if vt == 0 || vt == g::VT_CPLACEABLE || rd::<u32>(ent + e::MOVING_LIST) != 0 {
        return;
    }
    for i in 0..6 {
        wr(ent + e::MOVE_SPEED + 4 * i, 0.0f32);
    }
    c1(g::WORLD_ADD, ent);
    let m: u32 = rd(ent + e::MATRIX);
    if m != 0 {
        set_position(ent, [rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)]);
    }
}

/// Renders one entity: `SetupVehicleVariables` for vehicles, then `PreRender`, then `RenderOneNonRoad` when the
/// entity has an RW object.
fn render_entity(ent: u32) {
    let rw: u32 = rd(ent + e::RW_OBJECT);
    let model: u16 = rd(ent + e::MODEL);
    if (400..=611).contains(&model) && rw != 0 {
        c1(g::SETUP_VEHICLE_VARIABLES, rw);
    }
    vcall0(ent, e::VT_PRE_RENDER);
    if rw != 0 {
        c1(g::RENDER_ONE_NON_ROAD, ent);
    }
}

/// `MARK_MODEL_AS_NO_LONGER_NEEDED` when the model is loaded.
fn release_model(runner: &mut Runner, model: i32) {
    if (0..20000).contains(&model) && runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
    }
}

/// Model-info reference count (+8); 1 when the model has no info.
fn model_refs(model: i32) -> u16 {
    let mi = model_info(model);
    if mi != 0 { rd(mi + 8) } else { 1 }
}

/// Ped preview: a temporary ped with the skin, placed at (0, −2.25·zoom, 50.05), rotated, rendered and destroyed.
/// Mirrors samp.dll 0x1006C140.
fn render_ped(runner: &mut Runner, env: &Env, r: &Request, model: i32) -> u32 {
    let (raster, tex) = new_target();
    let ped = crate::players::create_preview_ped(runner);
    let Some(ped) = ped.filter(|_| raster != 0 && tex != 0) else { return 0 };
    let z = if model == TALL_SKIN { f32::from_bits(0x4248_9999) } else { f32::from_bits(0x4248_3333) };
    set_position(ped, [0.0, r.zoom * -2.25, z]);
    crate::client::set_ped_skin(runner, ped, model as u32);
    preview_flags(ped);
    rotate_entity(ped, r.rot);
    begin(env, raster, &r.background, &[]);
    add_to_world(ped);
    let clump: u32 = rd(ped + e::RW_OBJECT);
    if clump != 0 {
        // SAFETY: RpAnimBlendClumpUpdateAnimations(clump, step, onScreen), cdecl.
        unsafe {
            std::mem::transmute::<usize, extern "C" fn(u32, f32, u32)>(g::ANIM_BLEND_UPDATE)(clump, 100.0, 1);
        }
    }
    render_entity(ped);
    end(env);
    crate::players::destroy_preview_ped(ped);
    if model_refs(model) == 0 {
        release_model(runner, model);
    }
    tex
}

/// Vehicle preview, at a distance from the collision radius and zoom. Trains (449, 537/538 and 569/570, which
/// samp maps to 537/538) need samp's mission-train constructor, which we do not reproduce: they get no preview.
/// Mirrors samp.dll 0x1006C3C0.
fn render_vehicle(runner: &mut Runner, env: &Env, r: &Request, model: i32) -> u32 {
    let model = match model {
        570 => 538,
        569 => 537,
        m => m,
    };
    let (raster, tex) = new_target();
    if matches!(model, 449 | 537 | 538) {
        crate::log!("preview: train model {model} not rendered");
        return 0;
    }
    let made = crate::vehicles::create_at(runner, model as u16, [0.0, 0.0, 50.0]);
    let Some((handle, veh)) = made.filter(|_| raster != 0 && tex != 0) else { return 0 };
    preview_flags(veh);
    // samp computes this in x87 80-bit precision with one rounding at the store; f64 gives the same result.
    let radius = f64::from(col_radius(model));
    let mut y = ((-1.0 - (radius + radius)) * f64::from(r.zoom)) as f32;
    if crate::vehicles::samp_type(veh) == 4 {
        y = (-5.5 - radius * 2.5) as f32;
    }
    set_position(veh, [0.0, y, 50.0]);
    if r.colours.0 != -1 && r.colours.1 != -1 {
        wr(veh + 0x434, r.colours.0 as u8);
        wr(veh + 0x435, r.colours.1 as u8);
    }
    rotate_entity(veh, r.rot);
    begin(env, raster, &r.background, &[]);
    add_to_world(veh);
    crate::worldhooks::set_preview_vehicle(true);
    render_entity(veh);
    crate::worldhooks::set_preview_vehicle(false);
    end(env);
    crate::vehicles::delete_handle(runner, handle);
    tex
}

/// Collision-model bounding sphere: model info +0x14 is the collision model, its +0x18 the centre, +0x24 the radius.
fn col_sphere(model: i32) -> Option<([f32; 3], f32)> {
    let mi = model_info(model);
    let col: u32 = if mi != 0 { rd(mi + 0x14) } else { 0 };
    (col != 0).then(|| ([rd(col + 0x18), rd(col + 0x1C), rd(col + 0x20)], rd(col + 0x24)))
}

fn col_radius(model: i32) -> f32 {
    col_sphere(model).map_or(0.0, |(_, r)| r)
}

/// Object / weapon preview: an RW instance of the model, no entity. Mirrors samp.dll 0x1006C9B0.
fn render_object(runner: &mut Runner, env: &Env, r: &Request, model: i32) -> u32 {
    let (raster, tex) = new_target();
    if raster == 0 || tex == 0 {
        return 0;
    }
    let model = if matches!(model, 1373 | 3118 | 3552 | 3553) { FALLBACK_OBJECT } else { model };
    let mut requested = false;
    if !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        requested = true;
        runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
        runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
        // samp waits without limit (Sleep(1) loop); we stop after 10 s and render nothing.
        let t0 = std::time::Instant::now();
        while !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            if t0.elapsed() > std::time::Duration::from_secs(10) {
                crate::log!("preview: model {model} still not loaded after 10 s");
                return 0;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let mi = model_info(model);
    let obj = if mi != 0 {
        let vt: u32 = rd(mi);
        let f: u32 = rd(vt + 0x2C);
        // SAFETY: CBaseModelInfo::CreateInstance() (thiscall) on a live model info.
        unsafe { std::mem::transmute::<usize, extern "thiscall" fn(u32) -> u32>(f as usize)(mi) }
    } else {
        0
    };
    if obj == 0 {
        // samp returns the (uncleared) texture here.
        return tex;
    }
    let (centre, radius) = col_sphere(model).unwrap_or(([0.0; 3], 0.0));
    let d = ((f64::from(-0.1f32) - f64::from(radius) * 2.25) * f64::from(r.zoom)) as f32;
    let frame: u32 = rd(obj + 4);
    if frame != 0 {
        let v = [-centre[0], d, (50.0 - f64::from(centre[2])) as f32];
        c3(g::RW_FRAME_TRANSLATE, frame, v.as_ptr() as u32, 1);
        if model == FALLBACK_OBJECT {
            rotate(g::RW_FRAME_ROTATE, frame, &AXES[2], 180.0);
        } else {
            for (axis, &angle) in r.rot.iter().enumerate() {
                if angle != 0.0 {
                    rotate(g::RW_FRAME_ROTATE, frame, &AXES[axis], angle);
                }
            }
        }
    }
    begin(env, raster, &r.background, &[(30, 0), (20, 0)]);
    let kind: u8 = rd(obj);
    if kind == 1 {
        let cb: u32 = rd(obj + 0x48);
        if cb != 0 {
            c1(cb as usize, obj);
        }
    } else if kind == 2 {
        c1(g::RP_CLUMP_RENDER, obj);
    }
    end(env);
    if kind == 1 {
        let frame: u32 = rd(obj + 4);
        c1(g::RP_ATOMIC_DESTROY, obj);
        if frame != 0 {
            c1(g::RW_FRAME_DESTROY, frame);
        }
    }
    if requested {
        release_model(runner, model);
    }
    tex
}
