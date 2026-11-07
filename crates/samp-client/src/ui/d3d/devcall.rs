//! Thin wrappers over the real device's methods (samp draws on the real device).

use std::ffi::c_void;

use super::{HResult, dev, real_device, vfn};

fn d() -> *mut c_void {
    real_device()
}

pub fn set_render_state(rs: u32, v: u32) {
    // SAFETY: real device, SetRenderState(state, value).
    let f: extern "system" fn(*mut c_void, u32, u32) -> HResult = unsafe { vfn(d(), dev::SET_RENDER_STATE) };
    f(d(), rs, v);
}
pub fn set_tss(stage: u32, t: u32, v: u32) {
    // SAFETY: real device, SetTextureStageState.
    let f: extern "system" fn(*mut c_void, u32, u32, u32) -> HResult =
        unsafe { vfn(d(), dev::SET_TEXTURE_STAGE_STATE) };
    f(d(), stage, t, v);
}
pub fn set_sampler_state(sampler: u32, t: u32, v: u32) {
    // SAFETY: real device, SetSamplerState.
    let f: extern "system" fn(*mut c_void, u32, u32, u32) -> HResult = unsafe { vfn(d(), dev::SET_SAMPLER_STATE) };
    f(d(), sampler, t, v);
}
pub fn set_texture(stage: u32, tex: *mut c_void) {
    // SAFETY: real device, SetTexture.
    let f: extern "system" fn(*mut c_void, u32, *mut c_void) -> HResult = unsafe { vfn(d(), dev::SET_TEXTURE) };
    f(d(), stage, tex);
}
pub fn set_fvf(fvf: u32) {
    // SAFETY: real device, SetFVF.
    let f: extern "system" fn(*mut c_void, u32) -> HResult = unsafe { vfn(d(), dev::SET_FVF) };
    f(d(), fvf);
}
/// `BeginStateBlock`, the calls of `record`, `EndStateBlock`; null on failure.
pub fn record_state_block(record: impl FnOnce()) -> *mut c_void {
    // SAFETY: real device, BeginStateBlock (60) / EndStateBlock (61).
    let begin: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(d(), 60) };
    if begin(d()) < 0 {
        return std::ptr::null_mut();
    }
    record();
    let mut sb: *mut c_void = std::ptr::null_mut();
    // SAFETY: as above.
    let end: extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult = unsafe { vfn(d(), 61) };
    if end(d(), &mut sb) < 0 {
        return std::ptr::null_mut();
    }
    sb
}
/// `CreateStateBlock(type)` (1 = `D3DSBT_ALL`); null on failure.
pub fn create_state_block(kind: u32) -> *mut c_void {
    let mut sb: *mut c_void = std::ptr::null_mut();
    // SAFETY: real device, CreateStateBlock(type, IDirect3DStateBlock9**).
    let f: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HResult =
        unsafe { vfn(d(), dev::CREATE_STATE_BLOCK) };
    if f(d(), kind, &mut sb) < 0 {
        return std::ptr::null_mut();
    }
    sb
}
pub fn set_vertex_shader_null() {
    // SAFETY: real device, SetVertexShader(NULL).
    let f: extern "system" fn(*mut c_void, *mut c_void) -> HResult = unsafe { vfn(d(), dev::SET_VERTEX_SHADER) };
    f(d(), std::ptr::null_mut());
}
pub fn set_pixel_shader_null() {
    // SAFETY: real device, SetPixelShader(NULL).
    let f: extern "system" fn(*mut c_void, *mut c_void) -> HResult = unsafe { vfn(d(), dev::SET_PIXEL_SHADER) };
    f(d(), std::ptr::null_mut());
}
pub fn get_vertex_declaration() -> *mut c_void {
    let mut out = std::ptr::null_mut();
    // SAFETY: real device, GetVertexDeclaration(out) (AddRef'd).
    let f: extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult =
        unsafe { vfn(d(), dev::GET_VERTEX_DECLARATION) };
    f(d(), &mut out);
    out
}
pub fn set_vertex_declaration(decl: *mut c_void) {
    // SAFETY: real device, SetVertexDeclaration.
    let f: extern "system" fn(*mut c_void, *mut c_void) -> HResult = unsafe { vfn(d(), dev::SET_VERTEX_DECLARATION) };
    f(d(), decl);
}
/// `DrawPrimitiveUP(type, prims, data, stride)`.
pub fn draw_primitive_up(kind: u32, prims: u32, data: *const c_void, stride: u32) {
    // SAFETY: real device; `data` holds prims + 2 vertices of `stride` bytes.
    let f: extern "system" fn(*mut c_void, u32, u32, *const c_void, u32) -> HResult =
        unsafe { vfn(d(), dev::DRAW_PRIMITIVE_UP) };
    f(d(), kind, prims, data, stride);
}
/// `Clear(0, NULL, flags, colour, z, stencil)`.
pub fn clear(flags: u32, colour: u32, z: f32, stencil: u32) {
    // SAFETY: real device, Clear.
    let f: extern "system" fn(*mut c_void, u32, *const c_void, u32, u32, f32, u32) -> HResult =
        unsafe { vfn(d(), dev::CLEAR) };
    f(d(), 0, std::ptr::null(), flags, colour, z, stencil);
}
/// `GetViewport` → (x, y, w, h, minz, maxz).
pub fn viewport() -> [u32; 6] {
    let mut vp = [0u32; 6];
    // SAFETY: real device, GetViewport(D3DVIEWPORT9*).
    let f: extern "system" fn(*mut c_void, *mut [u32; 6]) -> HResult = unsafe { vfn(d(), dev::GET_VIEWPORT) };
    f(d(), &mut vp);
    vp
}
