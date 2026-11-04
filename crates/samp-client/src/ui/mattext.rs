//! Object material text (RPC 44 material kind 2, milestone M6), Windows only.
//!
//! TRACE-6 batch 3 (docs/phase3/trace-notes.md "World entities"): in the frame after RPC 44, from samp's object
//! pass, once per text slot: `D3DXCreateTexture(2w, 2h, 1, RENDERTARGET, X8R8G8B8, DEFAULT)` and
//! `D3DXCreateRenderToSurface(2w, 2h, X8R8G8B8, depth, D16)` (the render target is twice the material size),
//! `D3DXCreateFontA(face, height = font size, width 0, weight 700 when bold, mip 1, charset 1, quality 0)`;
//! `BeginScene`, `Clear(background ARGB)`, `sprite.Begin(0x10)`, `DrawTextA(stripped text, [0, 0, 2w, 2h],
//! 0x105 for centre alignment, font colour, colour codes per glyph)`, flush, `EndScene`. Never redrawn.
//! While the object renders, samp's device proxy `SetTexture` (A 0x10074BB0) replaces the slot's original texture
//! with the render target (world-entities.md RE-10 §4).
//!
//! Inferred (not measured): left / right alignment as DT_LEFT / DT_RIGHT with the same DT_VCENTER | DT_NOCLIP;
//! the render targets are not re-created after a device reset.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use samp_ui::colour;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use super::d3d::{self, D3dx, Font, FontArgs, Rect, dt};

/// One text slot as RPC 44 gave it.
#[derive(Clone, Debug)]
pub struct TextSpec {
    pub slot: usize,
    /// Material size code (10 = 32×32 … 140 = 512×512).
    pub size: u8,
    pub font: Vec<u8>,
    pub font_size: u8,
    pub bold: bool,
    pub font_colour: u32,
    pub background: u32,
    pub alignment: u8,
    pub text: Vec<u8>,
}

/// Material size code → (width, height) (samp A 0x100A8CC0).
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
        let old = s.objects.remove(&object);
        if !specs.is_empty() {
            s.objects.insert(object, specs.into_iter().map(|spec| Entry { spec, texture: 0, tried: false }).collect());
        }
        old
    })
    .flatten();
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

/// From samp's object pass: creates the missing render targets of `object` (once) and returns (slot, texture).
pub fn ensure(object: u32) -> Vec<(usize, u32)> {
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
    textures(object)
}

/// The render targets of `object` (no creation).
pub fn textures(object: u32) -> Vec<(usize, u32)> {
    with(|s| {
        s.objects
            .get(&object)
            .map(|v| v.iter().filter(|e| e.texture != 0).map(|e| (e.spec.slot, e.texture)).collect())
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

type CreateTextureFn = extern "system" fn(*mut c_void, u32, u32, u32, u32, u32, u32, *mut *mut c_void) -> d3d::HResult;
type CreateRtsFn = extern "system" fn(*mut c_void, u32, u32, u32, i32, u32, *mut *mut c_void) -> d3d::HResult;

fn d3dx_fn(name: &[u8]) -> Result<usize, String> {
    // SAFETY: LoadLibraryA / GetProcAddress with NUL-terminated names.
    unsafe {
        let h = LoadLibraryA(d3d::D3DX_DLL.as_ptr());
        if h.is_null() {
            return Err("d3dx9_25.dll not loaded".into());
        }
        GetProcAddress(h, name.as_ptr()).map(|f| f as usize).ok_or_else(|| "missing D3DX export".to_string())
    }
}

const D3DUSAGE_RENDERTARGET: u32 = 1;
const D3DFMT_X8R8G8B8: u32 = 22;
const D3DFMT_D16: u32 = 80;
const D3DPOOL_DEFAULT: u32 = 0;
const D3DCLEAR_TARGET: u32 = 1;

/// Creates and fills one render target (see the module header).
fn render(spec: &TextSpec) -> Result<u32, String> {
    let (w, h) = material_size(spec.size).ok_or("unknown material size")?;
    let (w, h) = (w * 2, h * 2);
    let dev = d3d::real_device();
    if dev.is_null() {
        return Err("no device".into());
    }
    let d3dx = D3dx::load()?;
    // SAFETY: documented D3DX signatures (D3DXCreateTexture, D3DXCreateRenderToSurface).
    let (create_tex, create_rts): (CreateTextureFn, CreateRtsFn) = unsafe {
        (
            std::mem::transmute::<usize, CreateTextureFn>(d3dx_fn(b"D3DXCreateTexture\0")?),
            std::mem::transmute::<usize, CreateRtsFn>(d3dx_fn(b"D3DXCreateRenderToSurface\0")?),
        )
    };
    let mut tex = std::ptr::null_mut();
    let hr = create_tex(dev, w, h, 1, D3DUSAGE_RENDERTARGET, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, &mut tex);
    if hr < 0 || tex.is_null() {
        return Err(format!("D3DXCreateTexture failed {hr:#x}"));
    }
    let mut rts = std::ptr::null_mut();
    let hr = create_rts(dev, w, h, D3DFMT_X8R8G8B8, 1, D3DFMT_D16, &mut rts);
    if hr < 0 || rts.is_null() {
        // SAFETY: we own `tex`.
        unsafe { d3d::release(tex) };
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
    )?;
    let mut sprite = std::ptr::null_mut();
    if (d3dx.create_sprite)(dev, &mut sprite) < 0 || sprite.is_null() {
        // SAFETY: we own these references.
        unsafe {
            d3d::release(rts);
            d3d::release(tex);
        }
        return Err("D3DXCreateSprite failed".into());
    }
    // SAFETY: COM calls on objects created above: IDirect3DTexture9::GetSurfaceLevel (slot 18),
    // ID3DXRenderToSurface::BeginScene (slot 5) / EndScene (slot 6), IDirect3DDevice9::Clear, ID3DXSprite.
    unsafe {
        let get_level: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> d3d::HResult = d3d::vfn(tex, 18);
        let mut surf = std::ptr::null_mut();
        if get_level(tex, 0, &mut surf) >= 0 && !surf.is_null() {
            let begin: extern "system" fn(*mut c_void, *mut c_void, *const c_void) -> d3d::HResult = d3d::vfn(rts, 5);
            let end: extern "system" fn(*mut c_void, u32) -> d3d::HResult = d3d::vfn(rts, 6);
            // Called from inside the 3D scene (samp's object pass): the device scene is closed around the
            // render-to-surface scene and reopened afterwards (D3D9 allows no nested BeginScene).
            let mut reopened = false;
            let mut hb = begin(rts, surf, std::ptr::null());
            if hb < 0 && d3d::end_scene() >= 0 {
                reopened = true;
                hb = begin(rts, surf, std::ptr::null());
            }
            if hb < 0 {
                crate::log!("mattext: {}x{} slot {}: render-to-surface BeginScene failed {hb:#x}", w, h, spec.slot);
            }
            if hb >= 0 {
                let clear: extern "system" fn(*mut c_void, u32, *const c_void, u32, u32, f32, u32) -> d3d::HResult =
                    d3d::vfn(dev, d3d::dev::CLEAR);
                clear(dev, 0, std::ptr::null(), D3DCLEAR_TARGET, spec.background, 1.0, 0);
                let sb: extern "system" fn(*mut c_void, u32) -> d3d::HResult = d3d::vfn(sprite, d3d::sprite::BEGIN);
                let se: extern "system" fn(*mut c_void) -> d3d::HResult = d3d::vfn(sprite, d3d::sprite::END);
                sb(sprite, d3d::sprite::ALPHABLEND);
                let stripped = colour::strip_codes(&spec.text);
                let glyphs: Vec<u32> =
                    colour::glyph_colours(&spec.text, spec.font_colour).iter().map(|g| g.colour).collect();
                let align = match spec.alignment {
                    1 => dt::CENTER,
                    2 => 0x2,
                    _ => 0,
                };
                let mut r = Rect { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                d3d::draw_glyph_coloured_with(
                    &font,
                    sprite,
                    &stripped,
                    &mut r,
                    align | dt::VCENTER | dt::NOCLIP,
                    spec.font_colour,
                    glyphs,
                );
                se(sprite);
                end(rts, 0);
            }
            if reopened {
                d3d::begin_scene();
            }
            d3d::release(surf);
        }
        d3d::release(sprite);
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
        assert_eq!(material_size(5), None);
    }
}
