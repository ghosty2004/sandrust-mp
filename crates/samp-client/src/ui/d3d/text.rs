//! D3DX fonts, lines and sprites, and the colour-code wrapper: text drawn through a proxy `ID3DXSprite` whose
//! `Draw` recolours each glyph.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

use super::proxy::{forwarding_vtable, proxy_object};
use super::{D3dx, HResult, Rect, dt, font, real_device, release, sprite, vfn};

/// `D3DXCreateFontA` arguments (ui-rendering.md, "Size and face settings").
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
    pub fn draw_n(
        &self,
        sprite: *mut c_void,
        text: &[u8],
        count: i32,
        rect: &mut Rect,
        flags: u32,
        colour: u32,
    ) -> i32 {
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
    pub fn draw_w(
        &self,
        sprite: *mut c_void,
        text: &[u16],
        count: i32,
        rect: &mut Rect,
        flags: u32,
        colour: u32,
    ) -> i32 {
        let mut t = Vec::with_capacity(text.len() + 1);
        t.extend_from_slice(text);
        t.push(0);
        // SAFETY: live ID3DXFont, slot 15 = DrawTextW; NUL-terminated text.
        let f: DrawTextW = unsafe { vfn(self.ptr, font::DRAW_TEXT_W) };
        f(self.ptr, sprite, t.as_ptr(), count, rect, flags, colour)
    }

    /// `DT_CALCRECT` size as (w, h) with a NUL-terminated copy and count −1 (how samp measures its fixed
    /// strings at start-up).
    pub fn measure_z(&self, text: &[u8], flags: u32) -> (i32, i32) {
        let mut r = Rect::default();
        self.draw_n(std::ptr::null_mut(), text, -1, &mut r, flags | dt::CALCRECT, 0xFF00_0000);
        (r.right - r.left, r.bottom - r.top)
    }

    /// `DT_CALCRECT` size of `text` as (w, h), with count = length; (0, 0) for empty text.
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

/// An `ID3DXLine` (the object editor's; samp creates it just before the editor's two fonts).
pub struct Line {
    pub ptr: *mut c_void,
}

/// `ID3DXLine` vtable slots.
mod line {
    pub const BEGIN: usize = 4;
    pub const DRAW: usize = 5;
    pub const SET_ANTIALIAS: usize = 13;
    pub const END: usize = 17;
    pub const ON_LOST_DEVICE: usize = 18;
    pub const ON_RESET_DEVICE: usize = 19;
}

impl Line {
    pub fn create(d3dx: &D3dx) -> Result<Self, String> {
        let mut out = std::ptr::null_mut();
        let hr = (d3dx.create_line)(real_device(), &mut out);
        if hr < 0 || out.is_null() {
            return Err(format!("D3DXCreateLine failed: {hr:#x}"));
        }
        Ok(Self { ptr: out })
    }

    fn call0(&self, slot: usize) {
        // SAFETY: live ID3DXLine; an argument-less method returning HRESULT.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(self.ptr, slot) };
        f(self.ptr);
    }

    pub fn set_antialias(&self, on: bool) {
        // SAFETY: live ID3DXLine; SetAntialias(BOOL).
        let f: extern "system" fn(*mut c_void, i32) -> HResult = unsafe { vfn(self.ptr, line::SET_ANTIALIAS) };
        f(self.ptr, i32::from(on));
    }

    /// `Begin`, `Draw(points, colour)`, `End` (how samp draws every editor line).
    pub fn segment(&self, a: [f32; 2], b: [f32; 2], colour: u32) {
        let pts = [a, b];
        self.call0(line::BEGIN);
        // SAFETY: live ID3DXLine; Draw(const D3DXVECTOR2*, DWORD count, D3DCOLOR).
        let f: extern "system" fn(*mut c_void, *const [f32; 2], u32, u32) -> HResult =
            unsafe { vfn(self.ptr, line::DRAW) };
        f(self.ptr, pts.as_ptr(), 2, colour);
        self.call0(line::END);
    }

    pub fn on_lost(&self) {
        self.call0(line::ON_LOST_DEVICE);
    }

    pub fn on_reset(&self) {
        self.call0(line::ON_RESET_DEVICE);
    }
}

impl Drop for Line {
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

/// The colour of the next glyph, walking the per-glyph colour list; cdecl, called by the proxy's `Draw` thunk
/// before it jumps to the real `Draw`.
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
    pub fn draw(
        &self,
        tex: *mut c_void,
        src: Option<&Rect>,
        center: Option<&[f32; 3]>,
        pos: Option<&[f32; 3]>,
        colour: u32,
    ) {
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

/// The colour-code wrapper's `DrawTextA` (ui-rendering.md, "Embedded colour codes"): draws `stripped` with
/// `font` through the proxy in front of `sprite`, giving the visible glyphs `colours` in order. Returns how many
/// glyph draws D3DX made (for verification).
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
