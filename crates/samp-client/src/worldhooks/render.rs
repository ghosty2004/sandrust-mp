//! The render-list filter and samp's own object passes, plus the roadsign guard.

use super::fog::{rw_render_state, set_fog};
use super::materials::{apply_materials, mark_drawn, restore_materials};
use super::*;

// ---- render-list filter and samp's own passes (patch table rows 76/77/78) ------------------------------------

/// Render-list filter decision (patch table row 78): a samp object with materials, a draw distance > 300 or timed
/// hours is not added to GTA's render list. Mirrors samp.dll 0x100A5150.
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
pub(super) unsafe extern "C" fn add_entity_to_render_list_stub() {
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

/// The entity our render hooks are drawing (samp pass or CObject::Render), for diagnostics (0 = none).
pub static RENDERING: AtomicU32 = AtomicU32::new(0);

/// One manual draw of a samp-drawn object. Mirrors samp.dll 0x100A9FD0.
fn draw_object(o: &ObjInfo) {
    let ent = o.ptr;
    RENDERING.store(ent, Ordering::Relaxed);
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
    // Material text render targets were created in the CGame::Process pre-step (objects.rs).
    let texts = if o.has_text { crate::ui::mattext::textures(ent) } else { Vec::new() };
    let saved = o.has_materials.then(|| apply_materials(ent, &o.materials, &texts));
    set_fog(true);
    let coloured = saved.as_ref().is_some_and(|s| s.coloured);
    if coloured && o.cam_dist < world::DIRECT_RENDER_DISTANCE {
        // samp's lighting wrapper: entity bit 28, SetupLighting; RW render; RemoveLighting(1).
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
        // PreRender + CRenderer::RenderOneNonRoad, as samp does for uncoloured near objects.
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
        mark_drawn(ent);
    }
    RENDERING.store(0, Ordering::Relaxed);
    PASS_DRAWS.fetch_add(1, Ordering::Relaxed);
}

/// RW object render: atomic → its render callback, clump → RpClumpRender. Mirrors samp.dll 0x100B6370.
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
        // Draw-last objects far → near, like samp's sort.
        let mut l = list;
        l.sort_by(|a, b| b.cam_dist.partial_cmp(&a.cam_dist).unwrap_or(std::cmp::Ordering::Equal));
        for o in &l {
            draw_object(o);
        }
    }
}

/// Opaque samp objects after `RenderEverythingBarRoads` (patch table row 76). Mirrors samp.dll 0x100A50D0.
pub(super) extern "C" fn after_render_everything_bar_roads() {
    // SAFETY: CRenderer::RenderEverythingBarRoads (cdecl, no arguments).
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::RENDER_EVERYTHING_BAR_ROADS as usize);
        f();
    }
    samp_pass(false);
}

/// Draw-last samp objects after `RenderFadingInEntities` (patch table row 77). Mirrors samp.dll 0x100A5110.
pub(super) extern "C" fn after_render_fading_in() {
    // SAFETY: CRenderer::RenderFadingInEntities (cdecl, no arguments).
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(g::RENDER_FADING_IN as usize);
        f();
    }
    samp_pass(true);
}

// ---- roadsign guard: no RenderRoadsignAtomic with a null atomic (patch table row 79) ---------------------------

#[unsafe(naked)]
pub(super) unsafe extern "C" fn roadsign_guard_stub() {
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
