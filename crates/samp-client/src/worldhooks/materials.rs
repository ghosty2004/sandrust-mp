//! Material slots applied around an object draw, and the `CObject::Render` hook.

use super::*;

// ---- materials (world-entities.md, "Materials (RPC 44/84) at render time") -----------------------------------

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

static PRE_PASS: Mutex<Option<HashMap<u32, world::MaterialPrePass>>> = Mutex::new(None);

fn with_pre_pass<R>(entity: u32, f: impl FnOnce(&mut world::MaterialPrePass) -> R) -> R {
    let mut g = PRE_PASS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new).entry(entity).or_default())
}

/// Forgets the pre-pass state of an entity whose samp object goes away (a new object starts afresh).
pub fn forget_material_rw(entity: u32) {
    let mut g = PRE_PASS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = g.as_mut() {
        m.remove(&entity);
    }
}

/// End of a draw of a material-carrying object: samp sets its "drawn" flag (+0x118F). Mirrors samp.dll 0x100AA124.
pub fn mark_drawn(entity: u32) {
    with_pre_pass(entity, |p| p.drawn());
}

/// Material pre-pass: slot textures and colours onto the RW materials in geometry order, gated by samp's "drawn"
/// flag. Nothing happens before the object's first draw; the first pre-pass after the RW object changed only
/// records and changes nothing; later ones apply. A new RW object is therefore drawn twice with its own geometry
/// flags first (`world::MaterialPrePass`). Applying from the first draw crashes RenderWare: it re-instances a
/// geometry whose prelight is still locked while PRELIT is cleared (docs/phase3/m6-notes.md, "Moved from
/// worldhooks.rs comments"). The originals are saved on every applying render; the post-pass restores them.
/// Mirrors samp.dll 0x100A8C40.
///
/// `texts`: (D3D texture of a text slot's 2x2 RW texture, render target) — samp's device proxy swaps them in
/// `SetTexture` while the object renders (the 2x2 texture is that slot's material texture).
pub fn apply_materials(entity: u32, slots: &[MatSlot; MATERIAL_SLOTS], texts: &[(u32, u32)]) -> SavedMaterials {
    let mut saved = SavedMaterials::default();
    let rw: u32 = rd(entity + g::ENTITY_RW_OBJECT);
    if rw == 0 || (texts.is_empty() && slots.iter().all(|s| s.texture == 0 && s.colour == 0)) {
        return saved;
    }
    let apply = with_pre_pass(entity, |p| p.begin(rw));
    if !apply {
        return saved;
    }
    let mut ctx = ApplyCtx { slots, saved: &mut saved };
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
    if !texts.is_empty() {
        saved.text = true;
        crate::ui::mattext::set_active(texts.to_vec());
    }
    saved
}

/// Material post-pass: restores what the pre-pass changed. Mirrors samp.dll 0x100A9EB0.
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

// ---- CObject::Render (patch table row 63) ---------------------------------------------------------------------

pub(super) extern "thiscall" fn object_render(this: u32) {
    let info = if rd::<u32>(this + g::ENTITY_FLAGS) & ENTITY_FLAG_CLIENT_OBJECT != 0 {
        try_shared(|s| s.objects.get(&this).copied()).flatten()
    } else {
        None
    };
    RENDERING.store(this, Ordering::Relaxed);
    let saved = info.map(|i| {
        let texts = if i.has_text { crate::ui::mattext::textures(this) } else { Vec::new() };
        apply_materials(this, &i.materials, &texts)
    });
    if saved.is_some() {
        mark_drawn(this);
    }
    thiscall0(g::OBJECT_RENDER, this);
    if let Some(s) = saved {
        restore_materials(&s);
    }
    RENDERING.store(0, Ordering::Relaxed);
}
