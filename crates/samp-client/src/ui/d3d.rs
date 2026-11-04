//! Direct3D 9 plumbing for the UI pass (Windows only).
//!
//! - **Device proxy** (UR §2.1): on the first frame the RenderWare device pointer
//!   `_RwD3DDevice` (0xC97C28) is saved and replaced by a pointer to a 4-byte object
//!   whose 119-slot vtable forwards every `IDirect3DDevice9` method to the real
//!   device, except Present (17) and Reset (16), which run our UI pass / lost-reset
//!   handling around the real call. Our own drawing always uses the real device.
//! - **Forwarding thunks** are generated machine code: each slot replaces the `this`
//!   argument on the stack with the real object and jumps to the real method.
//! - **D3DX** comes from `d3dx9_25.dll`, loaded dynamically like samp does.
//! - **Proxy sprite** (UR §3.3): the colour-code wrapper draws the stripped text
//!   through a proxy `ID3DXSprite` whose `Draw` recolours each glyph.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

pub type HResult = i32;

/// `_RwD3DDevice` in gta_sa.exe 1.0 US.
pub const RW_D3D_DEVICE: u32 = 0xC97C28;

/// `IDirect3DDevice9` vtable slots.
pub mod dev {
    pub const SLOTS: usize = 119;
    pub const SET_CURSOR_PROPERTIES: usize = 10;
    pub const SHOW_CURSOR: usize = 12;
    pub const RESET: usize = 16;
    pub const PRESENT: usize = 17;
    pub const CREATE_OFFSCREEN_PLAIN_SURFACE: usize = 36;
    pub const END_SCENE: usize = 42;
    pub const CLEAR: usize = 43;
    pub const SET_TRANSFORM: usize = 44;
    pub const GET_VIEWPORT: usize = 48;
    pub const SET_RENDER_STATE: usize = 57;
    pub const SET_TEXTURE: usize = 65;
    pub const SET_TEXTURE_STAGE_STATE: usize = 67;
    pub const SET_SAMPLER_STATE: usize = 69;
    pub const DRAW_PRIMITIVE_UP: usize = 83;
    pub const SET_VERTEX_DECLARATION: usize = 87;
    pub const GET_VERTEX_DECLARATION: usize = 88;
    pub const SET_FVF: usize = 89;
    pub const SET_VERTEX_SHADER: usize = 92;
    pub const SET_PIXEL_SHADER: usize = 107;
}

/// `ID3DXSprite` vtable slots (d3dx9_24 and later).
pub mod sprite {
    pub const SLOTS: usize = 14;
    pub const SET_TRANSFORM: usize = 5;
    pub const BEGIN: usize = 8;
    pub const DRAW: usize = 9;
    pub const FLUSH: usize = 10;
    pub const END: usize = 11;
    pub const DONOTSAVESTATE: u32 = 0x1;
    pub const ON_LOST_DEVICE: usize = 12;
    pub const ON_RESET_DEVICE: usize = 13;
    pub const ALPHABLEND: u32 = 0x10;
}

/// `ID3DXFont` vtable slots.
pub mod font {
    pub const DRAW_TEXT_A: usize = 14;
    pub const DRAW_TEXT_W: usize = 15;
    pub const ON_LOST_DEVICE: usize = 16;
    pub const ON_RESET_DEVICE: usize = 17;
}

/// `DrawText` format flags.
pub mod dt {
    pub const CENTER: u32 = 0x1;
    pub const VCENTER: u32 = 0x4;
    pub const EXPANDTABS: u32 = 0x40;
    pub const SINGLELINE: u32 = 0x20;
    pub const NOCLIP: u32 = 0x100;
    pub const CALCRECT: u32 = 0x400;
}

/// COM `Release` (slot 2).
pub unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        // SAFETY: caller passes a live COM object.
        let f: extern "system" fn(*mut c_void) -> u32 = unsafe { vfn(obj, 2) };
        f(obj);
    }
}

/// The function pointer in slot `slot` of a COM object's vtable.
///
/// # Safety
/// `obj` must be a live COM object with at least `slot + 1` vtable entries and
/// `F` must be the exact `extern "system"` signature of that method.
pub unsafe fn vfn<F: Copy>(obj: *mut c_void, slot: usize) -> F {
    unsafe {
        let vtbl = *(obj as *const *const usize);
        let p = *vtbl.add(slot);
        std::mem::transmute_copy::<usize, F>(&p)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

// ---- forwarding thunks ---------------------------------------------------------------

/// Machine code for `slots` forwarders: slot `s` does
/// `mov eax,[cell]; mov [esp+4],eax; mov eax,[eax]; jmp [eax+4*s]`.
pub fn thunk_code(cell: u32, base: u32, slots: usize) -> (Vec<u8>, Vec<u32>) {
    const LEN: usize = 17;
    let mut code = Vec::with_capacity(slots * LEN);
    let mut entries = Vec::with_capacity(slots);
    for s in 0..slots {
        entries.push(base + code.len() as u32);
        code.push(0xA1);
        code.extend_from_slice(&cell.to_le_bytes());
        code.extend_from_slice(&[0x89, 0x44, 0x24, 0x04]);
        code.extend_from_slice(&[0x8B, 0x00]);
        code.extend_from_slice(&[0xFF, 0xA0]);
        code.extend_from_slice(&((s * 4) as u32).to_le_bytes());
    }
    (code, entries)
}

/// Builds a leaked vtable of `slots` forwarders to the object stored in `cell`,
/// with `overrides` (slot, function address) replacing some entries.
fn forwarding_vtable(cell: &'static AtomicU32, slots: usize, overrides: &[(usize, usize)]) -> Option<&'static [u32]> {
    let size = slots * 17;
    // SAFETY: fresh RWX allocation for generated code.
    let mem = unsafe { VirtualAlloc(std::ptr::null(), size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    if mem.is_null() {
        return None;
    }
    let (code, mut entries) = thunk_code(cell.as_ptr() as u32, mem as u32, slots);
    // SAFETY: `mem` has `size` writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), mem as *mut u8, code.len()) };
    for &(slot, f) in overrides {
        entries[slot] = f as u32;
    }
    Some(Box::leak(entries.into_boxed_slice()))
}

/// A leaked proxy COM object (one word: the vtable pointer).
fn proxy_object(vtable: &'static [u32]) -> *mut c_void {
    Box::leak(Box::new([vtable.as_ptr() as u32])) as *mut [u32; 1] as *mut c_void
}

// ---- device proxy --------------------------------------------------------------------

/// The real `IDirect3DDevice9` (also the thunks' cell).
static REAL_DEVICE: AtomicU32 = AtomicU32::new(0);

pub fn real_device() -> *mut c_void {
    REAL_DEVICE.load(Ordering::Relaxed) as usize as *mut c_void
}

type PresentFn = extern "system" fn(*mut c_void, *const Rect, *const Rect, isize, *const c_void) -> HResult;
type ResetFn = extern "system" fn(*mut c_void, *mut c_void) -> HResult;

/// Installs the device proxy (first frame, UR §2.1). Returns false while the game
/// has no device yet.
pub fn install_device_proxy(present: PresentFn, reset: ResetFn) -> Result<(), String> {
    let (present, reset) = (present as *const () as usize, reset as *const () as usize);
    // SAFETY: reading/writing a gta_sa.exe .bss pointer on the main thread.
    let cur = unsafe { std::ptr::read_volatile(RW_D3D_DEVICE as usize as *const u32) };
    if cur == 0 {
        return Err("no device yet".into());
    }
    REAL_DEVICE.store(cur, Ordering::SeqCst);
    let vt = forwarding_vtable(
        &REAL_DEVICE,
        dev::SLOTS,
        &[
            (dev::PRESENT, present),
            (dev::RESET, reset),
            (dev::SET_TRANSFORM, proxy_set_transform as *const () as usize),
            (dev::END_SCENE, proxy_end_scene as *const () as usize),
        ],
    )
    .ok_or("VirtualAlloc failed")?;
    let proxy = proxy_object(vt);
    // SAFETY: as above.
    unsafe { std::ptr::write_volatile(RW_D3D_DEVICE as usize as *mut u32, proxy as u32) };
    crate::log!("ui: device proxy {:#x} installed over the real device {cur:#x}", proxy as u32);
    Ok(())
}

/// samp's copies of the last VIEW / PROJECTION / WORLD matrices set through the proxy
/// (A 0x10140be8 / 0x10140c28 / 0x10140c68, proxy slot 44 A 0x100748e0); the nametag,
/// bubble and label projections use them.
pub static VIEW: std::sync::Mutex<[f32; 16]> = std::sync::Mutex::new(IDENTITY);
pub static PROJECTION: std::sync::Mutex<[f32; 16]> = std::sync::Mutex::new(IDENTITY);
pub const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

extern "system" fn proxy_set_transform(_this: *mut c_void, state: u32, m: *const [f32; 16]) -> HResult {
    if !m.is_null() {
        // SAFETY: D3DMATRIX from the caller.
        let v = unsafe { *m };
        match state {
            2 => *VIEW.lock().unwrap_or_else(|e| e.into_inner()) = v,
            3 => *PROJECTION.lock().unwrap_or_else(|e| e.into_inner()) = v,
            _ => {}
        }
    }
    let d = real_device();
    // SAFETY: real device, SetTransform(state, matrix).
    let f: extern "system" fn(*mut c_void, u32, *const [f32; 16]) -> HResult = unsafe { vfn(d, dev::SET_TRANSFORM) };
    f(d, state, m)
}

/// Proxy `EndScene` (slot 42): samp wraps the real call (between two integrity checks
/// without visible effect, UR §2.1); the real call is made from the client.
extern "system" fn proxy_end_scene(_this: *mut c_void) -> HResult {
    let d = real_device();
    // SAFETY: real device, EndScene().
    let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(d, dev::END_SCENE) };
    f(d)
}

/// Real `Present`.
pub fn call_present(a: *const Rect, b: *const Rect, c: isize, d: *const c_void) -> HResult {
    let dev = real_device();
    // SAFETY: the real device and its Present signature.
    let f: PresentFn = unsafe { vfn(dev, dev::PRESENT) };
    f(dev, a, b, c, d)
}

/// Real `Reset`.
pub fn call_reset(params: *mut c_void) -> HResult {
    let dev = real_device();
    // SAFETY: the real device and its Reset signature.
    let f: ResetFn = unsafe { vfn(dev, dev::RESET) };
    f(dev, params)
}

/// `IDirect3DDevice9::ShowCursor` on the real device.
pub fn show_cursor(show: bool) {
    let dev = real_device();
    if dev.is_null() {
        return;
    }
    // SAFETY: real device, ShowCursor(BOOL).
    let f: extern "system" fn(*mut c_void, i32) -> i32 = unsafe { vfn(dev, dev::SHOW_CURSOR) };
    f(dev, i32::from(show));
}

// ---- device helpers (real device) ------------------------------------------------------

/// Thin wrappers over the real device's methods (samp draws on the real device).
pub mod devcall {
    use super::*;

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
        let f: extern "system" fn(*mut c_void, *mut c_void) -> HResult =
            unsafe { vfn(d(), dev::SET_VERTEX_DECLARATION) };
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
}

// ---- D3DX ----------------------------------------------------------------------------

type CreateFontA = extern "system" fn(
    *mut c_void,
    i32,
    u32,
    u32,
    u32,
    i32,
    u32,
    u32,
    u32,
    u32,
    *const u8,
    *mut *mut c_void,
) -> HResult;
type CreateSprite = extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
#[allow(clippy::type_complexity)]
type CreateTextureFromFileExA = extern "system" fn(
    *mut c_void,
    *const u8,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
) -> HResult;
type LoadSurfaceFromFileA = extern "system" fn(
    *mut c_void,
    *const c_void,
    *const Rect,
    *const u8,
    *const Rect,
    u32,
    u32,
    *mut c_void,
) -> HResult;
type CreateTexture = extern "system" fn(*mut c_void, u32, u32, u32, u32, u32, u32, *mut *mut c_void) -> HResult;
type CreateRenderToSurface = extern "system" fn(*mut c_void, u32, u32, u32, i32, u32, *mut *mut c_void) -> HResult;
type Vec3Project = extern "system" fn(
    *mut [f32; 3],
    *const [f32; 3],
    *const [u32; 6],
    *const [f32; 16],
    *const [f32; 16],
    *const [f32; 16],
) -> *mut [f32; 3];

/// The D3DX entry points samp uses, from `d3dx9_25.dll`.
#[derive(Clone, Copy)]
pub struct D3dx {
    pub create_font: CreateFontA,
    pub create_sprite: CreateSprite,
    pub create_texture_from_file_ex: CreateTextureFromFileExA,
    pub load_surface_from_file: LoadSurfaceFromFileA,
    pub create_texture: CreateTexture,
    pub create_render_to_surface: CreateRenderToSurface,
    pub vec3_project: Vec3Project,
}

/// The D3DX build samp links against.
pub const D3DX_DLL: &[u8] = b"d3dx9_25.dll\0";

impl D3dx {
    pub fn load() -> Result<Self, String> {
        // SAFETY: LoadLibraryA/GetProcAddress with NUL-terminated names; the
        // transmutes match the documented D3DX signatures.
        unsafe {
            let h = LoadLibraryA(D3DX_DLL.as_ptr());
            if h.is_null() {
                return Err("LoadLibrary(d3dx9_25.dll) failed".into());
            }
            let get = |name: &[u8]| -> Result<usize, String> {
                GetProcAddress(h, name.as_ptr())
                    .map(|f| f as usize)
                    .ok_or_else(|| format!("d3dx9_25.dll has no {}", String::from_utf8_lossy(&name[..name.len() - 1])))
            };
            Ok(Self {
                create_font: std::mem::transmute::<usize, CreateFontA>(get(b"D3DXCreateFontA\0")?),
                create_sprite: std::mem::transmute::<usize, CreateSprite>(get(b"D3DXCreateSprite\0")?),
                create_texture_from_file_ex: std::mem::transmute::<usize, CreateTextureFromFileExA>(get(
                    b"D3DXCreateTextureFromFileExA\0",
                )?),
                load_surface_from_file: std::mem::transmute::<usize, LoadSurfaceFromFileA>(get(
                    b"D3DXLoadSurfaceFromFileA\0",
                )?),
                create_texture: std::mem::transmute::<usize, CreateTexture>(get(b"D3DXCreateTexture\0")?),
                create_render_to_surface: std::mem::transmute::<usize, CreateRenderToSurface>(get(
                    b"D3DXCreateRenderToSurface\0",
                )?),
                vec3_project: std::mem::transmute::<usize, Vec3Project>(get(b"D3DXVec3Project\0")?),
            })
        }
    }

    /// `D3DXVec3Project(out, v, viewport, proj, view, world)`.
    pub fn project(&self, v: [f32; 3], vp: &[u32; 6], proj: &[f32; 16], view: &[f32; 16]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        (self.vec3_project)(&mut out, &v, vp, proj, view, &IDENTITY);
        out
    }
}

// ---- fonts and sprites -----------------------------------------------------------------

/// `D3DXCreateFontA` arguments (UR §3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontArgs {
    pub height: i32,
    pub width: u32,
    pub weight: u32,
    pub charset: u32,
    pub quality: u32,
    pub face: Vec<u8>,
}

/// An `ID3DXFont`.
pub struct Font {
    pub ptr: *mut c_void,
}

type DrawTextA = extern "system" fn(*mut c_void, *mut c_void, *const u8, i32, *mut Rect, u32, u32) -> i32;
type DrawTextW = extern "system" fn(*mut c_void, *mut c_void, *const u16, i32, *mut Rect, u32, u32) -> i32;

impl Font {
    /// `D3DXCreateFontA(device, Height, Width, Weight, MipLevels=1, Italic=0, CharSet,
    /// OutputPrecision=0, Quality, PitchAndFamily=0, Face)`.
    pub fn create(d3dx: &D3dx, args: FontArgs) -> Result<Self, String> {
        let mut face = args.face.clone();
        face.push(0);
        let mut out = std::ptr::null_mut();
        let hr = (d3dx.create_font)(
            real_device(),
            args.height,
            args.width,
            args.weight,
            1,
            0,
            args.charset,
            0,
            args.quality,
            0,
            face.as_ptr(),
            &mut out,
        );
        if hr < 0 || out.is_null() {
            return Err(format!("D3DXCreateFontA({args:?}) failed: {hr:#x}"));
        }
        Ok(Self { ptr: out })
    }

    /// `DrawTextA` with an explicit count (−1 = NUL-terminated; `text` gets a NUL).
    pub fn draw_n(&self, sprite: *mut c_void, text: &[u8], count: i32, rect: &mut Rect, flags: u32, colour: u32) -> i32 {
        let mut t = Vec::with_capacity(text.len() + 1);
        t.extend_from_slice(text);
        t.push(0);
        // SAFETY: live ID3DXFont, slot 14 = DrawTextA; NUL-terminated text.
        let f: DrawTextA = unsafe { vfn(self.ptr, font::DRAW_TEXT_A) };
        f(self.ptr, sprite, t.as_ptr(), count, rect, flags, colour)
    }

    /// `DrawTextA` of the whole text (count = length).
    pub fn draw(&self, sprite: *mut c_void, text: &[u8], rect: &mut Rect, flags: u32, colour: u32) -> i32 {
        self.draw_n(sprite, text, text.len() as i32, rect, flags, colour)
    }

    /// `DrawTextW` (`count` −1 = NUL-terminated).
    pub fn draw_w(&self, sprite: *mut c_void, text: &[u16], count: i32, rect: &mut Rect, flags: u32, colour: u32) -> i32 {
        let mut t = Vec::with_capacity(text.len() + 1);
        t.extend_from_slice(text);
        t.push(0);
        // SAFETY: live ID3DXFont, slot 15 = DrawTextW; NUL-terminated text.
        let f: DrawTextW = unsafe { vfn(self.ptr, font::DRAW_TEXT_W) };
        f(self.ptr, sprite, t.as_ptr(), count, rect, flags, colour)
    }

    /// `DT_CALCRECT` size of `text` (single line) as (w, h).
    pub fn measure(&self, text: &[u8], flags: u32) -> (i32, i32) {
        if text.is_empty() {
            return (0, 0);
        }
        let mut r = Rect::default();
        self.draw(std::ptr::null_mut(), text, &mut r, flags | dt::CALCRECT, 0xFF00_0000);
        (r.right - r.left, r.bottom - r.top)
    }

    pub fn on_lost(&self) {
        // SAFETY: live ID3DXFont.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(self.ptr, font::ON_LOST_DEVICE) };
        f(self.ptr);
    }

    pub fn on_reset(&self) {
        // SAFETY: live ID3DXFont.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(self.ptr, font::ON_RESET_DEVICE) };
        f(self.ptr);
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: we own one reference.
        unsafe { release(self.ptr) };
    }
}

/// An `ID3DXSprite`.
pub struct Sprite {
    pub ptr: *mut c_void,
}

type SpriteDraw = extern "system" fn(*mut c_void, *mut c_void, *const Rect, *const f32, *const f32, u32) -> HResult;

/// The sprite the colour proxy forwards to (the proxy thunks' cell).
static PROXY_TARGET: AtomicU32 = AtomicU32::new(0);
static SPRITE_PROXY: AtomicU32 = AtomicU32::new(0);

/// Per-glyph colours of the text being drawn through the proxy.
struct GlyphState {
    colours: Vec<u32>,
    next: usize,
    active: bool,
}

thread_local! {
    static GLYPHS: RefCell<GlyphState> = const { RefCell::new(GlyphState { colours: Vec::new(), next: 0, active: false }) };
}

/// The colour of the next glyph (UR §3.3 cursor walk); cdecl, called by the proxy's
/// `Draw` thunk before it jumps to the real `Draw`.
extern "C" fn next_glyph_colour(colour: u32) -> u32 {
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if !g.active {
            return colour;
        }
        let c = g.colours.get(g.next).copied().or(g.colours.last().copied()).unwrap_or(colour);
        g.next += 1;
        c
    })
}

/// Machine code of the proxy `Draw`: replaces the colour argument with
/// `next_glyph_colour(colour)`, swaps `this` for the target sprite and tail-jumps into
/// its `Draw`, so the real call keeps D3DX as its caller (like samp's wrapper).
fn proxy_draw_code(colour_fn: u32, cell: u32) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&[0xFF, 0x74, 0x24, 0x18]); // push dword [esp+0x18]
    c.push(0xB8); // mov eax, imm32
    c.extend_from_slice(&colour_fn.to_le_bytes());
    c.extend_from_slice(&[0xFF, 0xD0]); // call eax
    c.extend_from_slice(&[0x83, 0xC4, 0x04]); // add esp, 4
    c.extend_from_slice(&[0x89, 0x44, 0x24, 0x18]); // mov [esp+0x18], eax
    c.push(0xA1); // mov eax, [cell]
    c.extend_from_slice(&cell.to_le_bytes());
    c.extend_from_slice(&[0x89, 0x44, 0x24, 0x04]); // mov [esp+4], eax
    c.extend_from_slice(&[0x8B, 0x00]); // mov eax, [eax]
    c.extend_from_slice(&[0xFF, 0x60, (sprite::DRAW * 4) as u8]); // jmp [eax+0x24]
    c
}

/// The colour-proxy sprite object (created once).
fn sprite_proxy() -> Result<*mut c_void, String> {
    let p = SPRITE_PROXY.load(Ordering::SeqCst) as usize as *mut c_void;
    if !p.is_null() {
        return Ok(p);
    }
    let code = proxy_draw_code(next_glyph_colour as *const () as usize as u32, PROXY_TARGET.as_ptr() as u32);
    // SAFETY: fresh RWX allocation for the generated Draw thunk.
    let mem = unsafe { VirtualAlloc(std::ptr::null(), code.len(), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    if mem.is_null() {
        return Err("VirtualAlloc failed".into());
    }
    // SAFETY: `mem` has room for the code.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), mem as *mut u8, code.len()) };
    let vt = forwarding_vtable(&PROXY_TARGET, sprite::SLOTS, &[(sprite::DRAW, mem as usize)])
        .ok_or("VirtualAlloc failed")?;
    let p = proxy_object(vt);
    SPRITE_PROXY.store(p as u32, Ordering::SeqCst);
    Ok(p)
}

impl Sprite {
    pub fn create(d3dx: &D3dx) -> Result<Self, String> {
        sprite_proxy()?;
        let mut out = std::ptr::null_mut();
        let hr = (d3dx.create_sprite)(real_device(), &mut out);
        if hr < 0 || out.is_null() {
            return Err(format!("D3DXCreateSprite failed: {hr:#x}"));
        }
        Ok(Self { ptr: out })
    }

    fn call0(&self, slot: usize) -> HResult {
        // SAFETY: live ID3DXSprite; slots used here take no arguments.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(self.ptr, slot) };
        f(self.ptr)
    }

    pub fn begin(&self, flags: u32) -> HResult {
        // SAFETY: live ID3DXSprite, Begin(DWORD).
        let f: extern "system" fn(*mut c_void, u32) -> HResult = unsafe { vfn(self.ptr, sprite::BEGIN) };
        f(self.ptr, flags)
    }
    pub fn end(&self) -> HResult {
        self.call0(sprite::END)
    }
    pub fn flush(&self) -> HResult {
        self.call0(sprite::FLUSH)
    }
    pub fn on_lost(&self) {
        self.call0(sprite::ON_LOST_DEVICE);
    }
    pub fn on_reset(&self) {
        self.call0(sprite::ON_RESET_DEVICE);
    }

    pub fn set_transform(&self, m: &[f32; 16]) {
        // SAFETY: live ID3DXSprite; SetTransform(const D3DXMATRIX*).
        let f: extern "system" fn(*mut c_void, *const f32) -> HResult = unsafe { vfn(self.ptr, sprite::SET_TRANSFORM) };
        f(self.ptr, m.as_ptr());
    }

    /// `Draw(tex, src, center, pos, colour)`; `None` passes NULL.
    pub fn draw(&self, tex: *mut c_void, src: Option<&Rect>, center: Option<&[f32; 3]>, pos: Option<&[f32; 3]>, colour: u32) {
        // SAFETY: live ID3DXSprite, Draw.
        let f: SpriteDraw = unsafe { vfn(self.ptr, sprite::DRAW) };
        f(
            self.ptr,
            tex,
            src.map_or(std::ptr::null(), |r| r as *const Rect),
            center.map_or(std::ptr::null(), |c| c.as_ptr()),
            pos.map_or(std::ptr::null(), |p| p.as_ptr()),
            colour,
        );
    }

    /// The colour proxy, forwarding to this sprite.
    pub fn colour_proxy(&self) -> *mut c_void {
        PROXY_TARGET.store(self.ptr as u32, Ordering::SeqCst);
        SPRITE_PROXY.load(Ordering::SeqCst) as usize as *mut c_void
    }
}

impl Drop for Sprite {
    fn drop(&mut self) {
        // SAFETY: we own one reference.
        unsafe { release(self.ptr) };
    }
}

/// The colour-code wrapper's `DrawTextA` (UR §3.3): draws `stripped` with `font`
/// through the proxy in front of `sprite`, giving the visible glyphs `colours` in
/// order. Returns how many glyph draws D3DX made (for verification).
pub fn draw_glyph_coloured(
    font: &Font,
    sprite: &Sprite,
    stripped: &[u8],
    rect: &mut Rect,
    flags: u32,
    base: u32,
    colours: Vec<u32>,
) -> usize {
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        g.colours = colours;
        g.next = 0;
        g.active = true;
    });
    let proxy = sprite.colour_proxy();
    font.draw(proxy, stripped, rect, flags, base);
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        g.active = false;
        g.next
    })
}

/// Loads a texture file (DXUT-style: one mip level, managed pool).
pub fn load_texture(d3dx: &D3dx, path: &[u8]) -> Result<*mut c_void, String> {
    let mut p = path.to_vec();
    p.push(0);
    let mut out = std::ptr::null_mut();
    const DEFAULT: u32 = 0xFFFF_FFFF;
    let hr = (d3dx.create_texture_from_file_ex)(
        real_device(),
        p.as_ptr(),
        DEFAULT,
        DEFAULT,
        1,
        0,
        0, // D3DFMT_UNKNOWN
        1, // D3DPOOL_MANAGED
        DEFAULT,
        DEFAULT,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut out,
    );
    if hr < 0 || out.is_null() {
        return Err(format!("D3DXCreateTextureFromFileExA({}) failed: {hr:#x}", String::from_utf8_lossy(path)));
    }
    Ok(out)
}

/// The chat render target (UR §7.6, TRACE-3): `D3DXCreateTexture(w, h, 1,
/// RENDERTARGET, A8R8G8B8, DEFAULT)` + `D3DXCreateRenderToSurface(w, h, A8R8G8B8,
/// depth, D16)`.
pub struct RenderTarget {
    pub tex: *mut c_void,
    pub rts: *mut c_void,
    pub surface: *mut c_void,
}

impl RenderTarget {
    pub fn create(d3dx: &D3dx, w: u32, h: u32) -> Result<Self, String> {
        let mut tex = std::ptr::null_mut();
        let hr = (d3dx.create_texture)(real_device(), w, h, 1, 1, 21, 0, &mut tex);
        if hr < 0 || tex.is_null() {
            return Err(format!("D3DXCreateTexture({w}x{h} RT) failed: {hr:#x}"));
        }
        let mut rts = std::ptr::null_mut();
        let hr = (d3dx.create_render_to_surface)(real_device(), w, h, 21, 1, 80, &mut rts);
        if hr < 0 || rts.is_null() {
            // SAFETY: we own the texture.
            unsafe { release(tex) };
            return Err(format!("D3DXCreateRenderToSurface failed: {hr:#x}"));
        }
        let mut surface = std::ptr::null_mut();
        // SAFETY: IDirect3DTexture9::GetSurfaceLevel (18).
        let f: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HResult = unsafe { vfn(tex, 18) };
        f(tex, 0, &mut surface);
        Ok(Self { tex, rts, surface })
    }

    /// `ID3DXRenderToSurface::BeginScene(surface, NULL)`.
    pub fn begin(&self) -> HResult {
        // SAFETY: live ID3DXRenderToSurface, BeginScene (5).
        let f: extern "system" fn(*mut c_void, *mut c_void, *const c_void) -> HResult = unsafe { vfn(self.rts, 5) };
        f(self.rts, self.surface, std::ptr::null())
    }

    /// `EndScene(D3DX_FILTER_NONE… 0)`.
    pub fn end(&self) -> HResult {
        // SAFETY: live ID3DXRenderToSurface, EndScene (6).
        let f: extern "system" fn(*mut c_void, u32) -> HResult = unsafe { vfn(self.rts, 6) };
        f(self.rts, 0)
    }
}

impl Drop for RenderTarget {
    fn drop(&mut self) {
        // SAFETY: we own these references.
        unsafe {
            release(self.surface);
            release(self.rts);
            release(self.tex);
        }
    }
}

/// The D3D hardware cursor from `mouse.png` (UR §5): a 32×32 A8R8G8B8
/// system-memory surface set with `SetCursorProperties(0, 0, surface)`.
pub fn create_cursor(d3dx: &D3dx, path: &[u8]) -> Result<*mut c_void, String> {
    let dev = real_device();
    let mut surf = std::ptr::null_mut();
    // SAFETY: real device; CreateOffscreenPlainSurface(w, h, fmt, pool, out, shared).
    let f: extern "system" fn(*mut c_void, u32, u32, u32, u32, *mut *mut c_void, *mut c_void) -> HResult =
        unsafe { vfn(dev, dev::CREATE_OFFSCREEN_PLAIN_SURFACE) };
    const A8R8G8B8: u32 = 21;
    const SYSTEMMEM: u32 = 2;
    let hr = f(dev, 32, 32, A8R8G8B8, SYSTEMMEM, &mut surf, std::ptr::null_mut());
    if hr < 0 || surf.is_null() {
        return Err(format!("CreateOffscreenPlainSurface failed: {hr:#x}"));
    }
    let mut p = path.to_vec();
    p.push(0);
    let hr = (d3dx.load_surface_from_file)(
        surf,
        std::ptr::null(),
        std::ptr::null(),
        p.as_ptr(),
        std::ptr::null(),
        1, // D3DX_FILTER_NONE (TRACE-3)
        0,
        std::ptr::null_mut(),
    );
    if hr < 0 {
        // SAFETY: we own the surface.
        unsafe { release(surf) };
        return Err(format!("D3DXLoadSurfaceFromFileA(mouse.png) failed: {hr:#x}"));
    }
    // SAFETY: real device; SetCursorProperties(x, y, surface).
    let set: extern "system" fn(*mut c_void, u32, u32, *mut c_void) -> HResult =
        unsafe { vfn(dev, dev::SET_CURSOR_PROPERTIES) };
    let hr = set(dev, 0, 0, surf);
    if hr < 0 {
        crate::log!("ui: SetCursorProperties failed: {hr:#x}");
    }
    Ok(surf)
}

/// Registers the SA:MP font files from the game directory (UR §3.1, A 0x100c5100):
/// `AddFontResourceA("gtaweap3.ttf")`, `AddFontResourceA("sampaux3.ttf")`.
pub fn add_font_resources() {
    use windows_sys::Win32::Graphics::Gdi::AddFontResourceA;
    for f in [&b"gtaweap3.ttf\0"[..], b"sampaux3.ttf\0"] {
        // SAFETY: NUL-terminated relative path (the process directory is the GTA folder).
        let n = unsafe { AddFontResourceA(f.as_ptr()) };
        crate::log!("ui: AddFontResourceA({}) = {n}", String::from_utf8_lossy(&f[..f.len() - 1]));
    }
}
