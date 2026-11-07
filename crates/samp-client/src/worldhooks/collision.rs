//! Camera collision tests and the vertical-line test that also sees client objects.

use super::render::model_info;
use super::*;

// ---- camera collision (patch table rows 81/82/83) ---------------------------------------------------------------

/// No camera collision with vehicles while RPC 167 disabled vehicle collisions (patch table row 82). Mirrors
/// samp.dll 0x100A5B00.
extern "C" fn camera_collision_vehicles_off() -> u32 {
    u32::from(crate::vehhooks::with_shared(|s| s.disable_collisions))
}

#[unsafe(naked)]
pub(super) unsafe extern "C" fn check_camera_collision_vehicles_stub() {
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

/// `CCollision::BuildCacheOfCameraCollision(CColSphere*, CColSphere*)` (cdecl, bool), the call patch table row 81
/// redirects.
pub(super) const BUILD_CACHE_OF_CAMERA_COLLISION: u32 = 0x41AC40;
/// `CCollision::SphereCastVsEntity(CColSphere*, CColSphere*, CEntity*)` (cdecl, bool).
const SPHERE_CAST_VS_ENTITY: u32 = 0x419F00;
/// `CWorld::pIgnoreEntity`.
const WORLD_IGNORE_ENTITY: u32 = 0xB7CD68;
/// `CPhysical::m_pMovingList` (samp's vehicle camera test requires it to be set).
const PHYSICAL_MOVING_LIST: u32 = 0xB4;
/// samp adds its own tests only while the camera sphere is outside this square (|x| or |y| > 3000, i.e. outside
/// GTA's sector grid, where GTA's own cache sees no objects); measured in docs/phase3/m6p-notes.md (mark `cam81`).
const CAMERA_TEST_LIMIT: f32 = 3000.0;
/// Camera-distance limits of samp's object (< 200) and vehicle (< 300) camera tests.
const CAMERA_OBJECT_RANGE: f32 = 200.0;
const CAMERA_VEHICLE_RANGE: f32 = 300.0;

fn sphere_cast(a: u32, b: u32, ent: u32) -> bool {
    // SAFETY: GTA's cdecl function on the two spheres of the running camera test and a live entity.
    unsafe {
        let f: extern "C" fn(u32, u32, u32) -> u8 = std::mem::transmute(SPHERE_CAST_VS_ENTITY as usize);
        f(a, b, ent) & 1 != 0
    }
}

/// samp's per-entity camera test: alive, not `pIgnoreEntity`, in range, a collision model with radius > 0, then the
/// sphere cast. Mirrors samp.dll B 0x1009FC30.
fn camera_entity_hit(a: u32, b: u32, ent: u32, range: f32) -> bool {
    if ent == 0 || rd::<u32>(ent) == g::VTBL_PLACEABLE || ent == rd::<u32>(WORLD_IGNORE_ENTITY) {
        return false;
    }
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    if m == 0 {
        return false;
    }
    let p = v3(m + 0x30);
    let c = v3(g::CAMERA_POS);
    let d = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
    if d >= range {
        return false;
    }
    let mi = model_info(i32::from(rd::<u16>(ent + g::ENTITY_MODEL)));
    let col: u32 = if mi != 0 { rd(mi + g::MI_COL_MODEL) } else { 0 };
    if col == 0 || rd::<f32>(col + 0x24) <= 0.0 {
        return false;
    }
    sphere_cast(a, b, ent)
}

/// Camera collision cache (patch table row 81): GTA's cache first; when it found nothing and the camera is outside
/// GTA's world square, samp's object pool (objects with the no-camera-collision flag skipped) and then the vehicles
/// (skipped while RPC 167 disabled collisions). samp also walks its debug-script object pool, which is empty in
/// network play. Mirrors samp.dll B 0x100A5960.
pub(super) extern "C" fn build_cache_of_camera_collision(a: u32, b: u32) -> u32 {
    // SAFETY: the original cdecl call this site made.
    let found = unsafe {
        let f: extern "C" fn(u32, u32) -> u8 = std::mem::transmute(BUILD_CACHE_OF_CAMERA_COLLISION as usize);
        f(a, b) & 1 != 0
    };
    if found || b == 0 {
        return u32::from(found);
    }
    let (x, y): (f32, f32) = (rd(b), rd(b + 4));
    if (-CAMERA_TEST_LIMIT..=CAMERA_TEST_LIMIT).contains(&x) && (-CAMERA_TEST_LIMIT..=CAMERA_TEST_LIMIT).contains(&y) {
        return 0;
    }
    let objects: Vec<u32> = try_shared(|s| {
        if !s.active {
            return Vec::new();
        }
        s.objects.values().filter(|o| o.pool_object && !o.no_camera_collision && o.ptr != 0).map(|o| o.ptr).collect()
    })
    .unwrap_or_default();
    if objects
        .into_iter()
        .any(|o| rd::<u32>(o + g::ENTITY_RW_OBJECT) != 0 && camera_entity_hit(a, b, o, CAMERA_OBJECT_RANGE))
    {
        return 1;
    }
    let Some((off, vehicles)) =
        crate::vehhooks::try_with_shared(|s| (s.disable_collisions, s.by_ptr.keys().copied().collect::<Vec<_>>()))
    else {
        return 0;
    };
    if off {
        return 0;
    }
    let hit = vehicles
        .into_iter()
        .any(|v| rd::<u32>(v + PHYSICAL_MOVING_LIST) != 0 && camera_entity_hit(a, b, v, CAMERA_VEHICLE_RANGE));
    u32::from(hit)
}

/// The camera ignores attached objects (`m_pAttachedTo` set; patch table row 83). Mirrors samp.dll 0x100A5B30.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn camera_ignore_object_stub() {
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

// ---- CWorld::ProcessVerticalLine (patch table row 96) ----------------------------------------------------------

/// Every caller except `BreakObject_c::Init` (return address 0x59E8F9) that tests
/// buildings also tests objects, so ground searches (e.g. FIND_GROUND_Z_FOR_COORD for SetPlayerPosFindZ) land on
/// client objects. Arguments (cdecl): +0x14 buildings, +0x20 objects. Then the overwritten `sub esp, 0x2C` and a
/// jump to 0x407C37, where the original's `jmp` after the entry leads (its bytes at 0x5674E3 are now part of the
/// `jmp dword [slot]`), like samp's wrapper. Mirrors samp.dll B 0x100A5FD0.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn process_vertical_line_stub() {
    core::arch::naked_asm!(
        "cmp dword ptr [esp], 0x59E8F9",
        "je 2f",
        "cmp byte ptr [esp+0x14], 0",
        "je 2f",
        "mov byte ptr [esp+0x20], 1",
        "2:",
        "sub esp, 0x2C",
        "push {cont}",
        "ret",
        cont = const 0x407C37u32,
    )
}
