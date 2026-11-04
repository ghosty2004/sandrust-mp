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
    pub const CREATE_TEXTURE: usize = 23;
    pub const CREATE_OFFSCREEN_PLAIN_SURFACE: usize = 36;
    pub const BEGIN_SCENE: usize = 41;
    pub const END_SCENE: usize = 42;
}

/// `ID3DXSprite` vtable slots (d3dx9_24 and later).
pub mod sprite {
    pub const SLOTS: usize = 14;
    pub const SET_TRANSFORM: usize = 5;
    pub const BEGIN: usize = 8;
    pub const DRAW: usize = 9;
    pub const END: usize = 11;
    pub const ON_LOST_DEVICE: usize = 12;
    pub const ON_RESET_DEVICE: usize = 13;
    pub const ALPHABLEND: u32 = 0x10;
}

/// `ID3DXFont` vtable slots.
pub mod font {
    pub const DRAW_TEXT_A: usize = 14;
    pub const ON_LOST_DEVICE: usize = 16;
    pub const ON_RESET_DEVICE: usize = 17;
}

/// `DrawText` format flags.
pub mod dt {
    pub const VCENTER: u32 = 0x4;
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
    let vt = forwarding_vtable(&REAL_DEVICE, dev::SLOTS, &[(dev::PRESENT, present), (dev::RESET, reset)])
        .ok_or("VirtualAlloc failed")?;
    let proxy = proxy_object(vt);
    // SAFETY: as above.
    unsafe { std::ptr::write_volatile(RW_D3D_DEVICE as usize as *mut u32, proxy as u32) };
    crate::log!("ui: device proxy {:#x} installed over the real device {cur:#x}", proxy as u32);
    Ok(())
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

pub fn begin_scene() -> HResult {
    let dev = real_device();
    // SAFETY: real device, BeginScene().
    let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(dev, dev::BEGIN_SCENE) };
    f(dev)
}

pub fn end_scene() -> HResult {
    let dev = real_device();
    // SAFETY: real device, EndScene().
    let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(dev, dev::END_SCENE) };
    f(dev)
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

/// The D3DX entry points samp uses, from `d3dx9_25.dll`.
#[derive(Clone, Copy)]
pub struct D3dx {
    pub create_font: CreateFontA,
    pub create_sprite: CreateSprite,
    pub create_texture_from_file_ex: CreateTextureFromFileExA,
    pub load_surface_from_file: LoadSurfaceFromFileA,
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
            })
        }
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

    /// `DrawTextA`; returns the height drawn.
    pub fn draw(&self, sprite: *mut c_void, text: &[u8], rect: &mut Rect, flags: u32, colour: u32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        // SAFETY: live ID3DXFont, slot 14 = DrawTextA.
        let f: DrawTextA = unsafe { vfn(self.ptr, font::DRAW_TEXT_A) };
        f(self.ptr, sprite, text.as_ptr(), text.len() as i32, rect, flags, colour)
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

/// An `ID3DXSprite` plus the colour proxy in front of it.
pub struct Sprite {
    pub ptr: *mut c_void,
    /// Proxy forwarding to `ptr`, with `Draw` recolouring glyphs.
    pub proxy: *mut c_void,
}

/// The sprite behind the proxy (the proxy thunks' cell).
static REAL_SPRITE: AtomicU32 = AtomicU32::new(0);
static SPRITE_PROXY: AtomicU32 = AtomicU32::new(0);

type SpriteDraw = extern "system" fn(*mut c_void, *mut c_void, *const Rect, *const f32, *const f32, u32) -> HResult;

/// Per-glyph colours of the text being drawn through the proxy.
struct GlyphState {
    colours: Vec<u32>,
    next: usize,
    active: bool,
}

thread_local! {
    static GLYPHS: RefCell<GlyphState> = const { RefCell::new(GlyphState { colours: Vec::new(), next: 0, active: false }) };
}

/// Proxy `ID3DXSprite::Draw`: the i-th glyph D3DX draws gets the i-th colour from
/// `samp_ui::colour::glyph_colours` (UR §3.3 cursor walk).
extern "system" fn proxy_draw(
    _this: *mut c_void,
    tex: *mut c_void,
    src: *const Rect,
    center: *const f32,
    pos: *const f32,
    colour: u32,
) -> HResult {
    let c = GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if !g.active {
            return colour;
        }
        let c = g.colours.get(g.next).copied().or(g.colours.last().copied()).unwrap_or(colour);
        g.next += 1;
        c
    });
    let real = REAL_SPRITE.load(Ordering::Relaxed) as usize as *mut c_void;
    // SAFETY: the real sprite's Draw.
    let f: SpriteDraw = unsafe { vfn(real, sprite::DRAW) };
    f(real, tex, src, center, pos, c)
}

impl Sprite {
    pub fn create(d3dx: &D3dx) -> Result<Self, String> {
        let mut out = std::ptr::null_mut();
        let hr = (d3dx.create_sprite)(real_device(), &mut out);
        if hr < 0 || out.is_null() {
            return Err(format!("D3DXCreateSprite failed: {hr:#x}"));
        }
        REAL_SPRITE.store(out as u32, Ordering::SeqCst);
        let mut proxy = SPRITE_PROXY.load(Ordering::SeqCst) as usize as *mut c_void;
        if proxy.is_null() {
            let vt =
                forwarding_vtable(&REAL_SPRITE, sprite::SLOTS, &[(sprite::DRAW, proxy_draw as *const () as usize)])
                    .ok_or("VirtualAlloc failed")?;
            proxy = proxy_object(vt);
            SPRITE_PROXY.store(proxy as u32, Ordering::SeqCst);
        }
        Ok(Self { ptr: out, proxy })
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
    pub fn on_lost(&self) {
        self.call0(sprite::ON_LOST_DEVICE);
    }
    pub fn on_reset(&self) {
        self.call0(sprite::ON_RESET_DEVICE);
    }

    /// Draws `src` of `tex` scaled into the screen rect `dst` (sprite transform).
    pub fn draw_scaled(&self, tex: *mut c_void, src: Rect, dst: Rect, colour: u32) {
        let sw = (src.right - src.left).max(1) as f32;
        let sh = (src.bottom - src.top).max(1) as f32;
        let sx = (dst.right - dst.left) as f32 / sw;
        let sy = (dst.bottom - dst.top) as f32 / sh;
        let m: [f32; 16] =
            [sx, 0.0, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, dst.left as f32, dst.top as f32, 0.0, 1.0];
        let id: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        // SAFETY: live ID3DXSprite; SetTransform(const D3DXMATRIX*), Draw(...).
        unsafe {
            let set: extern "system" fn(*mut c_void, *const f32) -> HResult = vfn(self.ptr, sprite::SET_TRANSFORM);
            let draw: SpriteDraw = vfn(self.ptr, sprite::DRAW);
            set(self.ptr, m.as_ptr());
            draw(self.ptr, tex, &src, std::ptr::null(), std::ptr::null(), colour);
            set(self.ptr, id.as_ptr());
        }
    }
}

impl Drop for Sprite {
    fn drop(&mut self) {
        // SAFETY: we own one reference.
        unsafe { release(self.ptr) };
    }
}

/// Draws `stripped` with `font` through the proxy sprite, giving the visible glyphs
/// `colours` in order. Returns how many glyph draws D3DX made (for verification).
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
    font.draw(sprite.proxy, stripped, rect, flags, base);
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

/// A 1×1 opaque white texture (managed pool) for solid fills drawn with the
/// sprite (caret and selection rectangles, which DXUT draws untextured).
pub fn white_texture() -> Result<*mut c_void, String> {
    let dev = real_device();
    let mut tex = std::ptr::null_mut();
    // SAFETY: real device; CreateTexture(w, h, levels, usage, fmt, pool, out, shared).
    let f: extern "system" fn(*mut c_void, u32, u32, u32, u32, u32, u32, *mut *mut c_void, *mut c_void) -> HResult =
        unsafe { vfn(dev, dev::CREATE_TEXTURE) };
    let hr = f(dev, 1, 1, 1, 0, 21, 1, &mut tex, std::ptr::null_mut());
    if hr < 0 || tex.is_null() {
        return Err(format!("CreateTexture(1x1) failed: {hr:#x}"));
    }
    #[repr(C)]
    struct Locked {
        pitch: i32,
        bits: *mut u32,
    }
    let mut l = Locked { pitch: 0, bits: std::ptr::null_mut() };
    // SAFETY: IDirect3DTexture9::LockRect (19) / UnlockRect (20) on level 0.
    unsafe {
        let lock: extern "system" fn(*mut c_void, u32, *mut Locked, *const Rect, u32) -> HResult = vfn(tex, 19);
        let unlock: extern "system" fn(*mut c_void, u32) -> HResult = vfn(tex, 20);
        if lock(tex, 0, &mut l, std::ptr::null(), 0) >= 0 && !l.bits.is_null() {
            *l.bits = 0xFFFF_FFFF;
            unlock(tex, 0);
        }
    }
    Ok(tex)
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
