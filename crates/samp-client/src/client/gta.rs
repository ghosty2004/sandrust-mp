//! Free helpers of the session over game memory and game functions: raw writes, key polling, the ped-in-world test,
//! HUD switch, model loading, the skin change, entity matrices, the local surf probe and the camera-target search.

use std::time::Duration;

use super::{CAMERA_FRONT, PED_FLAG_IN_VEHICLE};
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner, op};

/// `CPlaceable` vtable: what a deleted entity's vtable reads as.
const VTBL_CPLACEABLE: u32 = 0x863C40;
/// `CPhysical::m_pMovingList`.
const PHYSICAL_MOVING_LIST: u32 = 0xB4;
/// `TheCamera` matrix position (what samp's camera distance reads).
const CAMERA_POS: u32 = 0xB6F9CC;
/// `CWorld::ProcessLineOfSight` (cdecl).
const PROCESS_LINE_OF_SIGHT: usize = 0x56BA00;
/// `CClothes::RebuildPlayer`, made a plain `ret` before every skin change.
const CLOTHES_REBUILD_PLAYER: u32 = 0x5A82C0;

/// Raw write into game data (heap objects, .data/.bss); no protection change.
pub(super) fn poke<T: Copy>(addr: u32, v: T) {
    if addr == 0 {
        return;
    }
    // SAFETY: callers pass addresses of live game objects / game globals.
    unsafe { std::ptr::write_unaligned(addr as usize as *mut T, v) };
}

/// Three consecutive floats (a `CVector`) at `a`.
pub(super) fn read_vec3(a: u32) -> [f32; 3] {
    let m = ProcessMemory;
    [m.read_f32(a).unwrap_or(0.0), m.read_f32(a + 4).unwrap_or(0.0), m.read_f32(a + 8).unwrap_or(0.0)]
}

/// The first `CPed` flag word has the in-vehicle bit.
pub(super) fn ped_in_vehicle(ped: u32) -> bool {
    ProcessMemory.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & PED_FLAG_IN_VEHICLE != 0
}

/// `GetAsyncKeyState(vk) != 0` (either bit), like samp's F4 test.
pub(super) fn async_key_any(vk: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // SAFETY: plain Win32 query.
    (unsafe { GetAsyncKeyState(vk) }) != 0
}

pub(super) fn async_key_down(vk: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // Only the "held now" bit, like the original (a stale "pressed since the last call" bit must not trigger a
    // spawn request after the guard).
    // SAFETY: plain Win32 query.
    (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0
}

/// `GetTickCount` (samp's clock for the camera-target rate).
pub(super) fn tick_count() -> u32 {
    // SAFETY: plain Win32 call.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

/// samp's "ped is live" test: not a deleted entity, in the moving list. Mirrors samp.dll 0x1009F430.
pub(crate) fn ped_in_world(ped: u32) -> bool {
    let m = ProcessMemory;
    let vt = m.read_u32(ped + off::VTABLE).unwrap_or(0);
    vt != 0 && vt != VTBL_CPLACEABLE && m.read_u32(ped + PHYSICAL_MOVING_LIST).unwrap_or(0) != 0
}

/// HUD drawing on/off (the menu "HUD on" flag and the script radar flag). Mirrors samp.dll 0x100A1DF0.
pub(crate) fn set_hud(on: bool) {
    poke(game::MENU_HUD_ON, u8::from(on));
    poke(game::HUD_SCRIPT_DONT_DISPLAY_RADAR, u8::from(!on));
}

/// Requests a model and waits (briefly) until it is loaded.
pub(crate) fn load_model(runner: &mut Runner, model: i32) -> bool {
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        return true;
    }
    runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
    runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
    for _ in 0..200 {
        if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    crate::log!("client: model {model} did not load in time");
    false
}

/// Skin change as the original does it, always (even for the current model): load the model, `DeleteRwObject` +
/// `SetModelIndex` through the ped's vtable, then re-initialise its speech audio entity.
/// `CClothes::RebuildPlayer := ret` first, so CJ's clothes never override it. Mirrors samp.dll 0x100AFF90.
pub(crate) fn set_ped_skin(runner: &mut Runner, ped: u32, skin: u32) {
    // An invalid model becomes 0.
    let skin = if game::is_ped_model(&ProcessMemory, skin) { skin } else { 0 };
    let mut mem = ProcessMemory;
    let _ = mem.write(CLOTHES_REBUILD_PLAYER, &[0xC3]);
    delete_ik_task(ped);
    if !load_model(runner, skin as i32) {
        // samp prints a debug chat line here.
        crate::ui::print_debug(&format!("Warning: Model {skin} wouldn't load in time!"));
        return;
    }
    let vtbl = ProcessMemory.read_u32(ped + off::VTABLE).unwrap_or(0);
    let (Ok(delete_rw), Ok(set_model)) = (
        ProcessMemory.read_u32(vtbl + off::VT_DELETE_RW_OBJECT),
        ProcessMemory.read_u32(vtbl + off::VT_SET_MODEL_INDEX),
    ) else {
        return;
    };
    // Animations the ped's tasks still hold survive the clump (the original's tasks keep pointing at them).
    crate::pedanims::keep_before_delete(ped);
    // SAFETY: the ped's own virtuals and the game's speech-entity init (thiscall), main thread.
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(delete_rw as usize);
        f(ped);
        poke(ped + off::ENTITY_MODEL, skin as u16);
        let g: extern "thiscall" fn(u32, u32) = std::mem::transmute(set_model as usize);
        g(ped, skin);
        let h: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::PED_SPEECH_INITIALISE as usize);
        h(ped + off::PED_SPEECH, ped);
    }
}

/// The skin change's first step: the IK secondary task (head look) is deleted.
fn delete_ik_task(ped: u32) {
    let intel = ProcessMemory.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
    let ik = if intel != 0 { ProcessMemory.read_u32(intel + game::INTELLIGENCE_SECONDARY_IK).unwrap_or(0) } else { 0 };
    if ik != 0 {
        // SAFETY: GTA's scalar deleting destructor of that task (thiscall), main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::TASK_IK_DELETING_DTOR as usize);
            f(ik, 1);
        }
        poke(intel + game::INTELLIGENCE_SECONDARY_IK, 0u32);
    }
}

/// An entity's matrix as 16 floats (CMatrix rows right / forward / up / position, padded).
pub(super) fn entity_matrix(ent: u32) -> Option<[f32; 16]> {
    let m = ProcessMemory;
    let mat = m.read_u32(ent + off::PLACEABLE_MATRIX).unwrap_or(0);
    (mat != 0).then(|| std::array::from_fn(|i| m.read_f32(mat + 4 * i as u32).unwrap_or(0.0)))
}

pub(super) fn entity_pos(ent: u32) -> Option<[f32; 3]> {
    entity_matrix(ent).map(|m| [m[12], m[13], m[14]])
}

pub(super) fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// samp's local surf probe: on foot, a vertical `CWorld::ProcessLineOfSight` from 0.25 m to 2 m below the ped's
/// position, vehicles and objects only; returns the entity hit (0 = none). `Client::surf_step` turns a hit into
/// surfing a pool vehicle or object (docs/re/subsystems/remote-players-sync.md, "Local surfing").
/// Mirrors samp.dll 0x100ACA50.
pub(super) fn surf_probe(ped: u32) -> u32 {
    type Los = extern "C" fn(*const f32, *const f32, *mut u8, *mut u32, u8, u8, u8, u8, u8, u8, u8, u8) -> u8;
    let m = ProcessMemory;
    if ped_in_vehicle(ped) {
        return 0;
    }
    let matrix = m.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
    if matrix == 0 {
        return 0;
    }
    let p = [0, 4, 8].map(|o| m.read_f32(matrix + off::MATRIX_POS + o).unwrap_or(0.0));
    let from = [p[0], p[1], p[2] - 0.25];
    let to = [p[0], p[1], p[2] - 0.25 - 1.75];
    let mut colpoint = [0u8; 64];
    let mut entity = 0u32;
    // SAFETY: GTA's cdecl line-of-sight query with valid in/out buffers, main thread.
    unsafe {
        let f: Los = std::mem::transmute(PROCESS_LINE_OF_SIGHT);
        f(from.as_ptr(), to.as_ptr(), colpoint.as_mut_ptr(), &mut entity, 0, 1, 0, 1, 0, 0, 0, 0);
    }
    entity
}

/// `CWorld::ProcessLineOfSight(start, end, …, buildings, vehicles, peds, objects, 0, 0, 0, 0)` of the camera-target
/// search: the hit point, or `end` when nothing was hit.
fn camera_los(start: [f32; 3], end: [f32; 3], vehicles: u8, objects: u8) -> [f32; 3] {
    type Los = extern "C" fn(*const f32, *const f32, *mut f32, *mut u32, u8, u8, u8, u8, u8, u8, u8, u8) -> u8;
    let mut colpoint = [0f32; 16];
    let mut entity = 0u32;
    // SAFETY: GTA's cdecl line-of-sight query with valid in/out buffers, main thread.
    unsafe {
        let f: Los = std::mem::transmute(PROCESS_LINE_OF_SIGHT);
        f(start.as_ptr(), end.as_ptr(), colpoint.as_mut_ptr(), &mut entity, 1, vehicles, 0, objects, 0, 0, 0, 0);
    }
    if entity != 0 { [colpoint[0], colpoint[1], colpoint[2]] } else { end }
}

/// The camera-target search (camtarget.rs): object, vehicle, player, actor. Mirrors samp.dll B 0x10004420.
pub(super) fn camera_targets(
    objects: &[(u16, u32, i32)],
    vehicles: &[(u32, u16)],
    peds: &[(u32, u16)],
    actors: &[(u32, u16)],
) -> [u16; 4] {
    use crate::camtarget as ct;
    let m = ProcessMemory;
    let (start, end) = ct::ray(read_vec3(CAMERA_FRONT), read_vec3(CAMERA_FRONT + 0xC));
    let cam = read_vec3(CAMERA_POS);
    let mut out = [ct::NONE; 4];
    let e = camera_los(start, end, 0, 0);
    if let Some((id, _, _)) = crate::objects::pick_within(start, e, objects, ct::OBJECT_RADIUS) {
        out[0] = id;
    }
    // The local ped's own vehicle is skipped.
    let ped = m.read_u32(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED).unwrap_or(0);
    let own = if ped != 0 && ped_in_vehicle(ped) { m.read_u32(ped + off::PED_VEHICLE).unwrap_or(0) } else { 0 };
    let search = |list: &[(u32, u16)], radius: f32, scale: f32, e: [f32; 3], skip: u32| -> u16 {
        ct::nearest(
            start,
            list.iter().filter_map(|&(ent, id)| {
                if !ped_in_world(ent) || ent == skip {
                    return None;
                }
                let p = entity_pos(ent)?;
                (dist3(p, cam) < radius && crate::objects::bound_sphere_hit(start, e, ent, scale)).then_some((id, p))
            }),
        )
    };
    let e = camera_los(start, end, 0, 1);
    out[1] = search(vehicles, ct::VEHICLE_RADIUS, ct::VEHICLE_SCALE, e, own);
    let e = camera_los(start, end, 1, 1);
    out[2] = search(peds, ct::PED_RADIUS, ct::PED_SCALE, e, 0);
    out[3] = search(actors, ct::PED_RADIUS, ct::PED_SCALE, e, 0);
    out
}
