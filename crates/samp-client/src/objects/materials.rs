//! Material slots: blank and text-slot textures, TXD lookups and the material records of RPC 44/84.

use super::*;

/// The blank material texture (material records with model 0): the loose file `SAMP\\blanktex.txd` of the SA:MP
/// install, loaded into a TXD slot "blanktex" on first use; the texture pointer is cached for the session and never
/// released.
static BLANK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn blank_texture() -> u32 {
    use std::sync::atomic::Ordering;
    let cached = BLANK.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    // SAFETY: CTxdStore::FindTxdSlot / AddTxdSlot / LoadTxd (cdecl) with NUL-terminated names.
    unsafe {
        let find: extern "C" fn(*const u8) -> i32 = std::mem::transmute(0x731850usize);
        let add: extern "C" fn(*const u8) -> i32 = std::mem::transmute(0x731C80usize);
        let load: extern "C" fn(i32, *const u8) -> u8 = std::mem::transmute(0x7320B0usize);
        if find(c"blanktex".as_ptr().cast()) < 0 {
            let slot = add(c"blanktex".as_ptr().cast());
            if slot < 0 || load(slot, c"SAMP\\blanktex.txd".as_ptr().cast()) == 0 {
                crate::log!("objects: SAMP\\blanktex.txd could not be loaded");
                return 0;
            }
        }
    }
    let tex = find_texture(b"blanktex", b"blanktex");
    BLANK.store(tex, Ordering::Relaxed);
    tex
}

pub(super) fn release_texture(tex: u32) {
    if tex != 0 && tex != BLANK.load(std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: RwTextureDestroy (cdecl) on a texture we hold a reference to.
        unsafe {
            let f: extern "C" fn(u32) -> u32 = std::mem::transmute(0x7F3820usize);
            f(tex);
        }
    }
}

/// Applies a material record (kind 1 texture, kind 2 text). `from_create`: the RPC 44 model rule. Mirrors
/// samp.dll 0x100A8570.
pub(super) fn apply_material(runner: &mut Runner, o: &mut Obj, m: &ObjectMaterial, from_create: bool) {
    let slot = match m {
        ObjectMaterial::Texture { slot, .. } | ObjectMaterial::Text { slot, .. } => u32::from(*slot),
        ObjectMaterial::Unknown { .. } => u32::MAX,
    };
    crate::crash::note(crate::crash::crumb::OBJ_MATERIAL, o.id, slot);
    crate::log!("{}", world::material_log_line(if from_create { 44 } else { 84 }, o.id, m));
    match m {
        ObjectMaterial::Texture { slot, model, txd, texture, colour } => {
            let slot = usize::from(*slot);
            if slot >= MATERIAL_SLOTS || !world::material_name_ok(txd) || !world::material_name_ok(texture) {
                return;
            }
            let model = world::material_model(*model, from_create);
            release_texture(o.materials[slot].texture);
            o.materials[slot].texture = 0;
            // The slot's text texture is gone: samp clears +0x110B, so its render target is never shown again.
            if o.materials[slot].text.take().is_some() {
                let specs: Vec<_> = o.materials.iter().filter_map(|m| m.text.clone()).collect();
                crate::ui::mattext::register(o.ptr, specs);
            }
            if model > 0 {
                if crate::client::load_model(runner, model) {
                    o.materials[slot].texture = find_texture(txd, texture);
                }
                // The model is released again when nothing else uses it.
                let mi = wh::model_info(model);
                if mi != 0
                    && rd::<u16>(mi + g::MI_REF_COUNT) == 0
                    && runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(model)])
                {
                    runner.run(crate::script::op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
                }
            } else if model == 0 {
                o.materials[slot].texture = blank_texture();
            }
            o.materials[slot].colour = *colour;
        }
        ObjectMaterial::Text {
            slot,
            material_size,
            font,
            font_size,
            bold,
            font_colour,
            background_colour,
            alignment,
            text,
        } => {
            let slot = usize::from(*slot);
            if slot >= MATERIAL_SLOTS || !world::material_name_ok(font) || text.is_empty() {
                return;
            }
            // The slot's texture (a kind-1 texture or an earlier text texture) and render target are released;
            // a fresh 2x2 RenderWare texture becomes the slot texture (the render target replaces it at D3D
            // level) and the slot colour is cleared.
            release_texture(o.materials[slot].texture);
            o.materials[slot].texture = 0;
            o.materials[slot].text = None;
            let tex = text_slot_texture();
            if tex == 0 {
                let specs: Vec<_> = o.materials.iter().filter_map(|m| m.text.clone()).collect();
                crate::ui::mattext::register(o.ptr, specs);
                return;
            }
            o.materials[slot].texture = tex;
            o.materials[slot].colour = 0;
            o.materials[slot].text = Some(crate::ui::mattext::TextSpec {
                slot,
                key: crate::ui::mattext::d3d_texture_of(tex),
                size: *material_size,
                font: font.clone(),
                font_size: *font_size,
                bold: *bold != 0,
                font_colour: *font_colour,
                background: *background_colour,
                alignment: *alignment,
                text: text.clone(),
            });
            let specs: Vec<_> = o.materials.iter().filter_map(|m| m.text.clone()).collect();
            crate::ui::mattext::register(o.ptr, specs);
        }
        ObjectMaterial::Unknown { .. } => {}
    }
}

/// The 2x2 texture of a material-text slot: `RwRasterCreate(2, 2, 0, 0)` + `RwTextureCreate`, filter mode + 1.
fn text_slot_texture() -> u32 {
    // SAFETY: RenderWare's cdecl RwRasterCreate (0x7FB230), RwTextureCreate (0x7F37C0), RwRasterDestroy (0x7FB020).
    unsafe {
        let raster_create: extern "C" fn(i32, i32, i32, i32) -> u32 = std::mem::transmute(0x7FB230usize);
        let texture_create: extern "C" fn(u32) -> u32 = std::mem::transmute(0x7F37C0usize);
        let raster = raster_create(2, 2, 0, 0);
        if raster == 0 {
            return 0;
        }
        let tex = texture_create(raster);
        if tex == 0 {
            let destroy: extern "C" fn(u32) -> u32 = std::mem::transmute(0x7FB020usize);
            destroy(raster);
            return 0;
        }
        // samp adds 1 to `RwTexture.filterAddressing` (+0x50): filter mode NEAREST (1) → LINEAR (2), so the text
        // is sampled bilinearly like the original's text draws.
        wr(tex + 0x50, rd::<u32>(tex + 0x50) + 1);
        tex
    }
}

/// `txd/texture` lookup: FindTxdSlot(txd), push/set the current TXD, read the texture, pop. Mirrors samp.dll
/// 0x100B2CA0.
fn find_texture(txd: &[u8], texture: &[u8]) -> u32 {
    let cstr = |b: &[u8]| {
        let mut v = b.to_vec();
        v.push(0);
        v
    };
    let (t, n) = (cstr(txd), cstr(texture));
    // SAFETY: CTxdStore::FindTxdSlot / PushCurrentTxd / SetCurrentTxd / PopCurrentTxd and RwTextureRead (cdecl)
    // with NUL-terminated names that outlive the calls.
    unsafe {
        let find: extern "C" fn(*const u8) -> i32 = std::mem::transmute(0x731850usize);
        let slot = find(t.as_ptr());
        if slot < 0 {
            return 0;
        }
        let push: extern "C" fn() = std::mem::transmute(0x7316A0usize);
        let set: extern "C" fn(i32) = std::mem::transmute(0x7319C0usize);
        let pop: extern "C" fn() = std::mem::transmute(0x7316B0usize);
        let read: extern "C" fn(*const u8, *const u8) -> u32 = std::mem::transmute(0x7F3AC0usize);
        push();
        set(slot);
        let tex = read(n.as_ptr(), std::ptr::null());
        pop();
        tex
    }
}
