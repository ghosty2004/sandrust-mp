//! The object pool ray test used for picking (selection mode and the camera target).

use super::*;

/// `CCollision::SphereCastVsEntity` (cdecl), `CCollision::TestLineSphere` (cdecl), `CEntity::GetBoundCentre`
/// (thiscall) and `gpColCache`, used by the pick ray test.
const SPHERE_CAST_VS_ENTITY: u32 = 0x419F00;
const TEST_LINE_SPHERE: u32 = 0x417470;
const GET_BOUND_CENTRE: u32 = 0x534250;
const COL_CACHE_PTR: u32 = 0x9655CC;

/// Our copy of samp's own collision cache for the pick (zero-initialised, like samp's .bss buffer); swapped into
/// `gpColCache` around the sphere cast.
static mut PICK_COL_CACHE: [u32; 0x420] = [0; 0x420];

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The object pool ray test, used by selection mode (`max` 150) and the camera target (`max` 40): of the created
/// objects other than model 19300 that are closer to `start` than the best so far (initially `max`), the ones the
/// segment hits — a sphere cast (radius 0.25 spheres) against the collision model when the entity uses collision
/// and has collision data, else [`bound_sphere_hit`] — and the nearest of those wins. Mirrors samp.dll
/// B 0x10012E40.
pub fn pick_within(start: [f32; 3], end: [f32; 3], list: &[(u16, u32, i32)], max: f32) -> Option<(u16, u32, i32)> {
    let mut best_d = max;
    let mut best = None;
    for &(id, ptr, model) in list {
        // Only entities with an RW object, and not the removed-building model.
        if rd::<u32>(ptr + 0x18) == 0 || model == crate::worldents::REMOVED_BUILDING_MODEL {
            continue;
        }
        let Some((_, p)) = rows(ptr) else { continue };
        let d = dist3(p, start);
        if d >= best_d {
            continue;
        }
        let mi = wh::model_info(model);
        let col: u32 = if mi != 0 { rd(mi + g::MI_COL_MODEL) } else { 0 };
        if col == 0 {
            continue;
        }
        let uses_collision = rd::<u8>(ptr + g::ENTITY_FLAGS) & 1 != 0;
        let hit = if uses_collision && rd::<u32>(col + 0x2C) != 0 {
            let r = objedit::PICK_SPHERE_RADIUS;
            let a: [f32; 5] = [start[0], start[1], start[2], r, 0.0];
            let b: [f32; 5] = [end[0], end[1], end[2], r, 0.0];
            // SAFETY: GTA's cdecl sphere cast on a live object; gpColCache points at our buffer during the call.
            unsafe {
                let old: u32 = rd(COL_CACHE_PTR);
                wr(COL_CACHE_PTR, std::ptr::addr_of_mut!(PICK_COL_CACHE) as u32);
                let f: extern "C" fn(*const [f32; 5], *const [f32; 5], u32) -> u8 =
                    std::mem::transmute(SPHERE_CAST_VS_ENTITY as usize);
                let h = f(&a, &b, ptr) & 1 != 0;
                wr(COL_CACHE_PTR, old);
                h
            }
        } else {
            bound_sphere_hit(start, end, ptr, 1.0)
        };
        if hit {
            best_d = d;
            best = Some((id, ptr, model));
        }
    }
    best
}

/// [`pick_within`] with selection mode's 150 m.
pub fn pick(start: [f32; 3], end: [f32; 3], list: &[(u16, u32, i32)]) -> Option<(u16, u32, i32)> {
    pick_within(start, end, list, objedit::PICK_DISTANCE)
}

/// samp's bounding-sphere line test (with its frustum check on): the entity's model needs a collision
/// model; the bounding sphere (GetBoundCentre, collision-model radius) must not be outside the camera frustum; then
/// the segment is tested against a sphere of [`objedit::pick_radius`] × `scale` around the centre. Mirrors
/// samp.dll B 0x100B65E0.
pub fn bound_sphere_hit(start: [f32; 3], end: [f32; 3], ptr: u32, scale: f32) -> bool {
    let model = i32::from(rd::<i16>(ptr + 0x22));
    let mi = wh::model_info(model);
    let col: u32 = if mi != 0 { rd(mi + g::MI_COL_MODEL) } else { 0 };
    if col == 0 {
        return false;
    }
    let mut c = [0.0f32; 3];
    // SAFETY: CEntity::GetBoundCentre(CVector&) on a live entity.
    unsafe {
        let f: extern "thiscall" fn(u32, *mut [f32; 3]) = std::mem::transmute(GET_BOUND_CENTRE as usize);
        f(ptr, &mut c);
    }
    let col_r: f32 = rd(col + 0x24);
    let sphere: [f32; 4] = [c[0], c[1], c[2], col_r];
    let cam: u32 = rd(g::SCENE_CAMERA);
    // SAFETY: RwCameraFrustumTestSphere(camera, RwSphere*) (cdecl).
    let visible = cam != 0
        && unsafe {
            let f: extern "C" fn(u32, *const [f32; 4]) -> i32 =
                std::mem::transmute(g::RW_CAMERA_FRUSTUM_TEST_SPHERE as usize);
            f(cam, &sphere) != 0
        };
    visible && {
        let line: [f32; 8] =
            [start[0] - c[0], start[1] - c[1], start[2] - c[2], 0.0, end[0] - c[0], end[1] - c[1], end[2] - c[2], 0.0];
        let sph: [f32; 5] = [0.0, 0.0, 0.0, objedit::pick_radius(col_r) * scale, 0.0];
        // SAFETY: CCollision::TestLineSphere(const CColLine&, const CColSphere&) (cdecl).
        unsafe {
            let f: extern "C" fn(*const [f32; 8], *const [f32; 5]) -> u8 =
                std::mem::transmute(TEST_LINE_SPHERE as usize);
            f(&line, &sph) & 1 != 0
        }
    }
}
