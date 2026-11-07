//! Object material text (CreateObject / SetPlayerObjectMaterial, material kind 2), Windows only.
//!
//! Each text slot gets a fresh 2x2 RenderWare texture as its material texture. Once per slot, from the
//! CGame::Process pre-step, [`ensure`] renders the text into a D3D render target of exactly the material
//! size; while the object renders, the device proxy's `SetTexture` swaps the 2x2 texture for that target.
//! Targets are never redrawn; on device loss they are released and re-created by the next object pass
//! ([`on_lost`]). Spec: docs/re/subsystems/world-entities.md, "84 SetPlayerObjectMaterial"; measurements in
//! docs/phase3/m6-notes.md; addresses in docs/re/addresses/ui-mattext.md.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use samp_ui::colour;

use super::d3d::{self, D3dx, Font, FontArgs, Rect, Sprite, dt};

/// One text slot as RPC 44 gave it.
#[derive(Clone, Debug)]
pub struct TextSpec {
    pub slot: usize,
    /// D3D texture of the slot's 2x2 RenderWare texture, which the render target stands in for (samp keeps it at
    /// object +0x110B).
    pub key: u32,
    /// Material size code (10 = 32x32 … 140 = 512x512); the render target has exactly this size.
    pub size: u8,
    pub font: Vec<u8>,
    pub font_size: u8,
    pub bold: bool,
    pub font_colour: u32,
    pub background: u32,
    pub alignment: u8,
    pub text: Vec<u8>,
}

/// Material size code → render-target (width, height). Mirrors samp.dll 0x100A8CC0.
pub fn material_size(code: u8) -> Option<(u32, u32)> {
    Some(match code {
        10 => (32, 32),
        20 => (64, 32),
        30 => (64, 64),
        40 => (128, 32),
        50 => (128, 64),
        60 => (128, 128),
        70 => (256, 32),
        80 => (256, 64),
        90 => (256, 128),
        100 => (256, 256),
        110 => (512, 64),
        120 => (512, 128),
        130 => (512, 256),
        140 => (512, 512),
        _ => return None,
    })
}

struct Entry {
    spec: TextSpec,
    texture: u32,
    tried: bool,
}

#[derive(Default)]
struct State {
    objects: HashMap<u32, Vec<Entry>>,
    /// While an object renders: (original D3D texture, render target).
    active: Vec<(u32, u32)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = STATE.try_lock().ok()?;
    Some(f(g.get_or_insert_with(State::default)))
}

/// Registers the text slots of an object (replacing earlier ones, whose textures are released).
pub fn register(object: u32, specs: Vec<TextSpec>) {
    let old = with(|s| {
        let mut old = s.objects.remove(&object).unwrap_or_default();
        // Slots whose record did not change (same 2x2 key) keep their render target, like samp's per-slot state.
        let entries: Vec<Entry> = specs
            .into_iter()
            .map(|spec| match old.iter().position(|e| e.spec.key == spec.key && e.spec.slot == spec.slot) {
                Some(i) => old.swap_remove(i),
                None => Entry { spec, texture: 0, tried: false },
            })
            .collect();
        if !entries.is_empty() {
            s.objects.insert(object, entries);
        }
        old
    });
    release(old);
}

pub fn unregister(object: u32) {
    let old = with(|s| s.objects.remove(&object)).flatten();
    release(old);
}

fn release(old: Option<Vec<Entry>>) {
    for e in old.into_iter().flatten() {
        if e.texture != 0 {
            // SAFETY: we own the render-target texture reference.
            unsafe { d3d::release(e.texture as usize as *mut c_void) };
        }
    }
}

/// Device lost: every created render target (D3DPOOL_DEFAULT) is released and marked "not created", so `Reset`
/// can succeed and the next object pass re-creates it ([`ensure`]). Slots whose creation failed stay failed,
/// as in samp (its "created" flag is cleared only together with a texture). Mirrors samp.dll B 0x100A8EE0.
pub fn on_lost() {
    let old: Vec<u32> = with(|s| {
        let mut out = Vec::new();
        for e in s.objects.values_mut().flatten() {
            if e.texture != 0 {
                out.push(e.texture);
                e.texture = 0;
                e.tried = false;
            }
        }
        s.active.clear();
        out
    })
    .unwrap_or_default();
    let sp = SPRITE.load(Ordering::SeqCst);
    if sp != 0 && !SPRITE_LOST.swap(true, Ordering::SeqCst) {
        std::mem::ManuallyDrop::new(Sprite { ptr: sp as usize as *mut c_void }).on_lost();
    }
    for t in &old {
        // SAFETY: we own these render-target texture references.
        unsafe { d3d::release(*t as usize as *mut c_void) };
    }
    if !old.is_empty() {
        crate::log!("mattext: device lost: {} render targets released", old.len());
    }
}

/// From the CGame::Process pre-step (object pool pass): creates the missing render targets of `object` (once each).
pub fn ensure(object: u32) {
    let todo: Vec<TextSpec> =
        with(|s| s.objects.get(&object).map(|v| v.iter().filter(|e| !e.tried).map(|e| e.spec.clone()).collect()))
            .flatten()
            .unwrap_or_default();
    for spec in todo {
        let tex = match render(&spec) {
            Ok(t) => t,
            Err(e) => {
                crate::log!("mattext: slot {} of object {object:#x}: {e}", spec.slot);
                0
            }
        };
        with(|s| {
            if let Some(e) = s.objects.get_mut(&object).and_then(|v| v.iter_mut().find(|e| e.spec.slot == spec.slot)) {
                e.tried = true;
                e.texture = tex;
            }
        });
    }
}

/// The substitutions of `object`: (2x2 slot D3D texture, render target), no creation.
pub fn textures(object: u32) -> Vec<(u32, u32)> {
    with(|s| {
        s.objects
            .get(&object)
            .map(|v| v.iter().filter(|e| e.texture != 0 && e.spec.key != 0).map(|e| (e.spec.key, e.texture)).collect())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Starts / ends the substitution for one object render: (original D3D texture, render target) pairs.
pub fn set_active(pairs: Vec<(u32, u32)>) {
    let any = !pairs.is_empty();
    with(|s| s.active = pairs);
    let f = if any { substitute as fn(*mut c_void) -> *mut c_void as usize as u32 } else { 0 };
    d3d::SET_TEXTURE_FILTER.store(f, std::sync::atomic::Ordering::SeqCst);
}

fn substitute(tex: *mut c_void) -> *mut c_void {
    let t = tex as u32;
    with(|s| s.active.iter().find(|(orig, _)| *orig == t && t != 0).map(|(_, rt)| *rt as usize as *mut c_void))
        .flatten()
        .unwrap_or(tex)
}

const D3DUSAGE_RENDERTARGET: u32 = 1;
const D3DFMT_A8R8G8B8: u32 = 21;
const D3DFMT_X8R8G8B8: u32 = 22;
const D3DFMT_D16: u32 = 80;
const D3DPOOL_DEFAULT: u32 = 0;
/// `D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER`; the target is cleared to the background colour with depth 0.0.
const CLEAR_TARGET_ZBUFFER: u32 = 3;
/// `D3DRS_DESTBLEND` / `D3DBLEND_BLENDFACTOR`: set after `sprite.Begin` when the background is 0.
const D3DRS_DESTBLEND: u32 = 20;
const D3DBLEND_BLENDFACTOR: u32 = 14;

/// Render-target format: with a zero background the target keeps an alpha channel.
pub fn target_format(background: u32) -> u32 {
    if background == 0 { D3DFMT_A8R8G8B8 } else { D3DFMT_X8R8G8B8 }
}

/// DrawText format per alignment: 1 centre (DT_CENTER | DT_VCENTER), 2 right (DT_RIGHT), anything else
/// left / top; always DT_NOCLIP.
pub fn draw_format(alignment: u8) -> u32 {
    let a = match alignment {
        1 => dt::CENTER | dt::VCENTER,
        2 => 0x2,
        _ => 0,
    };
    a | dt::NOCLIP
}

/// samp's sign-text renderer keeps one sprite for every fill; ours is created before the first fill and kept
/// (released only with the process), reset around device loss.
static SPRITE: AtomicU32 = AtomicU32::new(0);
static SPRITE_LOST: AtomicBool = AtomicBool::new(false);

fn shared_sprite(d3dx: &D3dx) -> Result<u32, String> {
    let p = SPRITE.load(Ordering::SeqCst);
    if p != 0 {
        if SPRITE_LOST.swap(false, Ordering::SeqCst) {
            std::mem::ManuallyDrop::new(Sprite { ptr: p as usize as *mut c_void }).on_reset();
        }
        return Ok(p);
    }
    let sprite = std::mem::ManuallyDrop::new(Sprite::create(d3dx)?);
    let p = sprite.ptr as u32;
    SPRITE.store(p, Ordering::SeqCst);
    Ok(p)
}

/// Creates and fills one render target: D3DX texture + render-to-surface (D16 depth), D3DX font, clear, then
/// the text with per-glyph colour codes. Mirrors samp.dll B 0x100708E0.
fn render(spec: &TextSpec) -> Result<u32, String> {
    let (w, h) = material_size(spec.size).ok_or("unknown material size")?;
    let dev = d3d::real_device();
    if dev.is_null() {
        return Err("no device".into());
    }
    let d3dx = D3dx::load()?;
    let sprite_ptr = shared_sprite(&d3dx)?;
    // SAFETY: IDirect3DDevice9::GetDisplayMode (slot 8) into a local D3DDISPLAYMODE (samp's first call; unused).
    unsafe {
        let get_mode: extern "system" fn(*mut c_void, u32, *mut [u32; 4]) -> d3d::HResult = d3d::vfn(dev, 8);
        let mut mode = [0u32; 4];
        get_mode(dev, 0, &mut mode);
    }
    let mut tex = std::ptr::null_mut();
    let fmt = target_format(spec.background);
    let hr = (d3dx.create_texture)(dev, w, h, 1, D3DUSAGE_RENDERTARGET, fmt, D3DPOOL_DEFAULT, &mut tex);
    if hr < 0 || tex.is_null() {
        return Err(format!("D3DXCreateTexture failed {hr:#x}"));
    }
    // SAFETY: IDirect3DTexture9::GetSurfaceLevel (slot 18), IDirect3DSurface9::GetDesc (slot 12) on our texture.
    let (surf, desc) = unsafe {
        let get_level: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> d3d::HResult = d3d::vfn(tex, 18);
        let mut surf = std::ptr::null_mut();
        let mut desc = [fmt, 0, 0, 0, 0, 0, w, h];
        if get_level(tex, 0, &mut surf) >= 0 && !surf.is_null() {
            let get_desc: extern "system" fn(*mut c_void, *mut [u32; 8]) -> d3d::HResult = d3d::vfn(surf, 12);
            get_desc(surf, &mut desc);
        }
        (surf, desc)
    };
    let mut rts = std::ptr::null_mut();
    // D3DSURFACE_DESC: Format +0, Width +0x18, Height +0x1C.
    let hr = (d3dx.create_render_to_surface)(dev, desc[6], desc[7], desc[0], 1, D3DFMT_D16, &mut rts);
    if hr < 0 || rts.is_null() {
        // SAFETY: we own `tex` and `surf`.
        unsafe {
            if !surf.is_null() {
                d3d::release(surf);
            }
            d3d::release(tex);
        }
        return Err(format!("D3DXCreateRenderToSurface failed {hr:#x}"));
    }
    let font = Font::create(
        &d3dx,
        FontArgs {
            height: i32::from(spec.font_size),
            width: 0,
            weight: if spec.bold { 700 } else { 400 },
            charset: 1,
            quality: 0,
            face: spec.font.clone(),
        },
    )
    .inspect_err(|_| {
        // Like samp: a debug chat line naming the face that failed.
        super::glue::print_debug_later(format!("SignText: Can't create font {}", String::from_utf8_lossy(&spec.font)));
    })?;
    let sprite = std::mem::ManuallyDrop::new(Sprite { ptr: sprite_ptr as usize as *mut c_void });
    // SAFETY: COM calls on objects created above: ID3DXRenderToSurface::BeginScene (slot 5) / EndScene (slot 6),
    // IDirect3DDevice9::Clear / SetRenderState.
    unsafe {
        if !surf.is_null() {
            let begin: extern "system" fn(*mut c_void, *mut c_void, *const c_void) -> d3d::HResult = d3d::vfn(rts, 5);
            let end: extern "system" fn(*mut c_void, u32) -> d3d::HResult = d3d::vfn(rts, 6);
            // Runs from the CGame::Process pre-step, between the previous Present and RenderWare's BeginScene,
            // so the render-to-surface scene is never nested.
            let hb = begin(rts, surf, std::ptr::null());
            if hb < 0 {
                crate::log!("mattext: {}x{} slot {}: render-to-surface BeginScene failed {hb:#x}", w, h, spec.slot);
            } else {
                d3d::devcall::clear(CLEAR_TARGET_ZBUFFER, spec.background, 0.0, 0);
                sprite.begin(d3d::sprite::ALPHABLEND);
                if spec.background == 0 {
                    d3d::devcall::set_render_state(D3DRS_DESTBLEND, D3DBLEND_BLENDFACTOR);
                }
                let stripped = colour::strip_codes(&spec.text);
                let glyphs: Vec<u32> =
                    colour::glyph_colours(&spec.text, spec.font_colour).iter().map(|g| g.colour).collect();
                let mut r = Rect { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                d3d::draw_glyph_coloured(
                    &font,
                    &sprite,
                    &stripped,
                    &mut r,
                    draw_format(spec.alignment),
                    spec.font_colour,
                    glyphs,
                );
                sprite.end();
                end(rts, 0);
            }
            d3d::release(surf);
        }
        d3d::release(rts);
    }
    drop(font);
    Ok(tex as u32)
}

/// The D3D texture behind an RW texture (`RwTexture.raster` → D3D9 raster extension at `_RwD3D9RasterExtOffset`
/// 0xB4E9E0 → `IDirect3DTexture9*` first field; gta-reversed `WindowedMode.cpp`).
pub fn d3d_texture_of(rw_texture: u32) -> u32 {
    if rw_texture == 0 {
        return 0;
    }
    // SAFETY: reads of live RenderWare objects and a gta_sa.exe global.
    unsafe {
        let raster = std::ptr::read_unaligned(rw_texture as usize as *const u32);
        if raster == 0 {
            return 0;
        }
        let off = std::ptr::read_unaligned(0xB4E9E0usize as *const i32);
        std::ptr::read_unaligned((raster as i32 + off) as usize as *const u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(material_size(90), Some((256, 128)));
        assert_eq!(material_size(140), Some((512, 512)));
        assert_eq!(material_size(130), Some((512, 256)));
        assert_eq!(material_size(5), None);
    }

    #[test]
    fn format_and_alignment() {
        assert_eq!(target_format(0), D3DFMT_A8R8G8B8);
        assert_eq!(target_format(0xFF00_0000), D3DFMT_X8R8G8B8);
        assert_eq!(target_format(0x00FF_FFFF), D3DFMT_X8R8G8B8);
        assert_eq!(draw_format(0), 0x100);
        assert_eq!(draw_format(1), 0x105);
        assert_eq!(draw_format(2), 0x102);
        assert_eq!(draw_format(3), 0x100);
    }
}
