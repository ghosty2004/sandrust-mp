//! Render-state and fog setters the way samp drives them, and the vehicle-preview fog flag.

use super::*;

/// `RwRenderStateSet(state, value)` through `RwEngineInstance` (+0x20). Mirrors samp.dll 0x100C3E60.
pub(super) fn rw_render_state(state: u32, value: u32) {
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

/// `IDirect3DDevice9::SetRenderState` on `_RwD3DDevice` (vtable +0xE4), as samp's fog setter calls it.
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

/// samp's vehicle-preview flag: set around the vehicle draw of a model-preview render; while it is set the fog
/// setter always writes FOGENABLE 0. Mirrors samp.dll 0x1026EA58.
static PREVIEW_VEHICLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One `RwRenderStateSet(FOGENABLE, on)` the way samp's RenderWare render-state hook turns it into D3D calls: samp's
/// fog setter with the value, then RenderWare itself gets FALSE. Mirrors samp.dll 0x100C3DC0.
pub(crate) fn samp_fog_render_state(on: bool) {
    set_fog(on);
    rw_render_state(14, 0);
}

pub(crate) fn set_preview_vehicle(on: bool) {
    PREVIEW_VEHICLE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// samp's fog setter: FOGENABLE := on (0 while a preview vehicle is drawn), FOGTABLEMODE := NONE,
/// FOGVERTEXMODE := LINEAR. Mirrors samp.dll 0x100C3D70.
pub(super) fn set_fog(on: bool) {
    let on = on && !PREVIEW_VEHICLE.load(std::sync::atomic::Ordering::Relaxed);
    d3d_render_state(28, u32::from(on));
    d3d_render_state(35, 0);
    d3d_render_state(140, 3);
}

/// RenderWare's current fog state (`rwRENDERSTATEFOGENABLE` = 14 via the `RwEngineInstance` +0x24 getter). samp
/// keeps the same value in a flag that its render-state hook updates.
pub(super) fn rw_fog_enabled() -> bool {
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
