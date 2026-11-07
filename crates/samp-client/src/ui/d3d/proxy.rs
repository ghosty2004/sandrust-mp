//! The device proxy: generated forwarding thunks, the proxy device installed in `_RwD3DDevice`, and the
//! methods it intercepts (Present, Reset, SetTexture, TestCooperativeLevel, CreateVertexBuffer, SetTransform,
//! EndScene), plus the calls that go to the real device.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

use super::{HResult, RW_D3D_DEVICE, Rect, dev, vfn};
use crate::patch::{Memory, ProcessMemory};

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
pub(super) fn forwarding_vtable(
    cell: &'static AtomicU32,
    slots: usize,
    overrides: &[(usize, usize)],
) -> Option<&'static [u32]> {
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
pub(super) fn proxy_object(vtable: &'static [u32]) -> *mut c_void {
    Box::leak(Box::new([vtable.as_ptr() as u32])) as *mut [u32; 1] as *mut c_void
}

// ---- device proxy --------------------------------------------------------------------

/// The real `IDirect3DDevice9` (also the thunks' cell).
static REAL_DEVICE: AtomicU32 = AtomicU32::new(0);

/// The real device (null before the proxy is installed).
pub fn real_device() -> *mut c_void {
    REAL_DEVICE.load(Ordering::Relaxed) as usize as *mut c_void
}

type PresentFn = extern "system" fn(*mut c_void, *const Rect, *const Rect, isize, *const c_void) -> HResult;
type ResetFn = extern "system" fn(*mut c_void, *mut c_void) -> HResult;

/// Installs the device proxy on the first frame (ui-rendering.md, "Device proxy"). Fails while the game has no
/// device yet.
pub fn install_device_proxy(present: PresentFn, reset: ResetFn) -> Result<(), String> {
    let (present, reset) = (present as *const () as usize, reset as *const () as usize);
    let cur = ProcessMemory.read_u32(RW_D3D_DEVICE).unwrap_or(0);
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
            (dev::SET_TEXTURE, set_texture_stub()),
            (dev::CREATE_VERTEX_BUFFER, proxy_create_vertex_buffer as *const () as usize),
            (dev::TEST_COOPERATIVE_LEVEL, proxy_test_cooperative_level as *const () as usize),
        ],
    )
    .ok_or("VirtualAlloc failed")?;
    let proxy = proxy_object(vt);
    // SAFETY: `_RwD3DDevice` is a writable gta_sa.exe .bss pointer; main thread.
    unsafe { std::ptr::write_volatile(RW_D3D_DEVICE as usize as *mut u32, proxy as u32) };
    crate::log!("ui: device proxy {:#x} installed over the real device {cur:#x}", proxy as u32);
    Ok(())
}

/// Texture substitution for `SetTexture`, used for object material text: while an object renders, a material
/// slot's original texture is swapped for the slot's text render target. The filter returns the texture to set
/// instead, or the argument unchanged. Mirrors samp.dll 0x10074BB0 (its proxy `SetTexture`).
pub static SET_TEXTURE_FILTER: AtomicU32 = AtomicU32::new(0);

type SetTextureFn = extern "system" fn(*mut c_void, u32, *mut c_void) -> HResult;

/// The number-plate step of the proxy `SetTexture`: maps the shared plate raster's texture to the rendered
/// vehicle's plate (or the default plate); 0 = no substitution. cdecl, called by [`set_texture_stub_code`].
extern "C" fn plate_texture(tex: u32) -> u32 {
    if SET_TEXTURE_FILTER.load(Ordering::Relaxed) != 0 {
        return 0;
    }
    std::panic::catch_unwind(|| crate::plates::substitute(tex)).ok().flatten().unwrap_or(0)
}

/// Machine code of the proxy `SetTexture` entry: a plate substitution replaces the texture argument and `this`
/// and tail-jumps into the real `SetTexture`, so the real call keeps GTA as its caller like in samp; everything
/// else jumps to [`proxy_set_texture`] with the stack unchanged.
fn set_texture_stub_code(plate_fn: u32, real_cell: u32, rust_fn: u32) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&[0xFF, 0x74, 0x24, 0x0C]); // push dword [esp+0xC] (texture)
    c.push(0xB8); // mov eax, imm32
    c.extend_from_slice(&plate_fn.to_le_bytes());
    c.extend_from_slice(&[0xFF, 0xD0]); // call eax
    c.extend_from_slice(&[0x83, 0xC4, 0x04]); // add esp, 4
    c.extend_from_slice(&[0x85, 0xC0]); // test eax, eax
    c.extend_from_slice(&[0x74, 0x15]); // jz normal (+21)
    c.extend_from_slice(&[0x89, 0x44, 0x24, 0x0C]); // mov [esp+0xC], eax
    c.push(0xA1); // mov eax, [real_cell]
    c.extend_from_slice(&real_cell.to_le_bytes());
    c.extend_from_slice(&[0x89, 0x44, 0x24, 0x04]); // mov [esp+4], eax
    c.extend_from_slice(&[0x8B, 0x00]); // mov eax, [eax]
    c.extend_from_slice(&[0xFF, 0xA0]); // jmp [eax+disp32]
    c.extend_from_slice(&((dev::SET_TEXTURE * 4) as u32).to_le_bytes());
    // normal:
    c.push(0xB8); // mov eax, imm32
    c.extend_from_slice(&rust_fn.to_le_bytes());
    c.extend_from_slice(&[0xFF, 0xE0]); // jmp eax
    c
}

/// The generated `SetTexture` entry (created once).
fn set_texture_stub() -> usize {
    static STUB: AtomicU32 = AtomicU32::new(0);
    let p = STUB.load(Ordering::SeqCst);
    if p != 0 {
        return p as usize;
    }
    let code = set_texture_stub_code(
        plate_texture as *const () as usize as u32,
        REAL_DEVICE.as_ptr() as u32,
        proxy_set_texture as *const () as usize as u32,
    );
    // SAFETY: fresh RWX allocation for the generated thunk.
    let mem = unsafe { VirtualAlloc(std::ptr::null(), code.len(), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    if mem.is_null() {
        return proxy_set_texture as *const () as usize;
    }
    // SAFETY: `mem` has room for the code.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), mem as *mut u8, code.len()) };
    STUB.store(mem as u32, Ordering::SeqCst);
    mem as usize
}

/// Proxy `IDirect3DDevice9::SetTexture`: runs [`SET_TEXTURE_FILTER`] (if set) on the texture, then the real call.
extern "system" fn proxy_set_texture(_this: *mut c_void, stage: u32, tex: *mut c_void) -> HResult {
    let filter = SET_TEXTURE_FILTER.load(Ordering::Relaxed);
    let tex = if filter != 0 {
        // SAFETY: the filter is a `fn(*mut c_void) -> *mut c_void` stored by its owner.
        let f: fn(*mut c_void) -> *mut c_void = unsafe { std::mem::transmute(filter as usize) };
        f(tex)
    } else {
        tex
    };
    // Number plates while a material filter is active (the stub handles the other plate draws).
    let tex = if filter != 0 {
        crate::plates::substitute(tex as u32).map(|t| t as usize as *mut c_void).unwrap_or(tex)
    } else {
        tex
    };
    let dev = real_device();
    // SAFETY: the real device's SetTexture.
    let f: SetTextureFn = unsafe { vfn(dev, dev::SET_TEXTURE) };
    f(dev, stage, tex)
}

/// Test hook (control channel `devreset`): while non-zero, `TestCooperativeLevel` through the proxy reports
/// D3DERR_DEVICENOTRESET (and counts down), so RenderWare releases its video memory and resets the device.
pub static FORCE_LOST: AtomicU32 = AtomicU32::new(0);
const D3DERR_DEVICENOTRESET: HResult = 0x8876_0869_u32 as HResult;

/// Proxy `TestCooperativeLevel` (slot 3): the real call, unless the `devreset` test hook is armed.
extern "system" fn proxy_test_cooperative_level(_this: *mut c_void) -> HResult {
    if FORCE_LOST.load(Ordering::Relaxed) > 0 {
        FORCE_LOST.fetch_sub(1, Ordering::Relaxed);
        return D3DERR_DEVICENOTRESET;
    }
    let d = real_device();
    // SAFETY: the real device's TestCooperativeLevel().
    let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(d, dev::TEST_COOPERATIVE_LEVEL) };
    let hr = f(d);
    // Breadcrumb on every change of the device state (lost / not reset / ok).
    if LAST_COOPERATIVE_LEVEL.swap(hr as u32, Ordering::Relaxed) != hr as u32 {
        crate::crash::note(crate::crash::crumb::DEVICE_LOST, 0, hr as u32);
    }
    hr
}

static LAST_COOPERATIVE_LEVEL: AtomicU32 = AtomicU32::new(0);

/// Failed `CreateVertexBuffer` calls seen through the proxy (diagnostics: after a failed create RenderWare keeps
/// a dynamic vertex buffer record without a buffer, which can crash later).
pub static VB_CREATE_FAILURES: AtomicU32 = AtomicU32::new(0);

type CreateVertexBufferFn =
    extern "system" fn(*mut c_void, u32, u32, u32, u32, *mut *mut c_void, *mut c_void) -> HResult;

/// Proxy `CreateVertexBuffer` (slot 26): the real call; failures are logged (the first 20) with their size.
extern "system" fn proxy_create_vertex_buffer(
    _this: *mut c_void,
    length: u32,
    usage: u32,
    fvf: u32,
    pool: u32,
    out: *mut *mut c_void,
    shared: *mut c_void,
) -> HResult {
    let d = real_device();
    // SAFETY: the real device's CreateVertexBuffer with the caller's arguments.
    let f: CreateVertexBufferFn = unsafe { vfn(d, dev::CREATE_VERTEX_BUFFER) };
    let hr = f(d, length, usage, fvf, pool, out, shared);
    if hr < 0 {
        let n = VB_CREATE_FAILURES.fetch_add(1, Ordering::Relaxed);
        crate::crash::note(crate::crash::crumb::VB_CREATE_FAILED, 0, length);
        if n < 20 {
            let ent = crate::worldhooks::RENDERING.load(Ordering::Relaxed);
            let model: u16 = if ent != 0 { crate::worldhooks::rd(ent + 0x22) } else { 0 };
            let flags: u32 = if ent != 0 { crate::worldhooks::rd(ent + 0x1C) } else { 0 };
            let rw: u32 = if ent != 0 { crate::worldhooks::rd(ent + 0x18) } else { 0 };
            // Return-address candidates of gta_sa.exe on the stack (diagnostics).
            let probe = 0u32;
            let sp = &probe as *const u32 as u32;
            let mut calls = Vec::new();
            for i in 0..0x300u32 {
                let a = sp + 4 * i;
                if !crate::dll::readable(a, 4) {
                    break;
                }
                let w: u32 = crate::worldhooks::rd(a);
                if (0x40_1000..0x85_6000).contains(&w) && calls.len() < 24 {
                    calls.push(format!("{w:#x}"));
                }
            }
            let rd = |base: u32, off: u32| -> u32 { if base != 0 { crate::worldhooks::rd(base + off) } else { 0 } };
            let kind = rd(rw, 0) & 0xFF;
            let geom = if kind == 1 { rd(rw, 0x18) } else { 0 };
            crate::log!(
                "d3d: CreateVertexBuffer(length {length}, usage {usage:#x}, fvf {fvf:#x}, pool {pool}) failed: {hr:#x} \
                 (rendering entity {ent:#x} model {model} flags {flags:#x} rw {rw:#x} type {kind} geometry {geom:#x} \
                 flags {:#x} locked {:#x} verts {} morphs {}; stack {})",
                rd(geom, 8),
                rd(geom, 0xC) & 0xFFFF,
                rd(geom, 0x14),
                rd(geom, 0x18),
                calls.join(" ")
            );
        }
    }
    hr
}

/// The last VIEW matrix set through the proxy; the nametag, bubble and label projections use it with
/// [`PROJECTION`]. Mirrors samp.dll 0x10140BE8.
pub static VIEW: std::sync::Mutex<[f32; 16]> = std::sync::Mutex::new(IDENTITY);
/// The last PROJECTION matrix set through the proxy. Mirrors samp.dll 0x10140C28.
pub static PROJECTION: std::sync::Mutex<[f32; 16]> = std::sync::Mutex::new(IDENTITY);
/// The last WORLD matrix (transform state 256); the selection pick ray uses world × view.
/// Mirrors samp.dll 0x10140C68.
pub static WORLD: std::sync::Mutex<[f32; 16]> = std::sync::Mutex::new(IDENTITY);
/// The 4×4 identity matrix.
pub const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Proxy `SetTransform` (slot 44): keeps copies of VIEW, PROJECTION and WORLD, then the real call.
/// Mirrors samp.dll 0x100748E0.
extern "system" fn proxy_set_transform(_this: *mut c_void, state: u32, m: *const [f32; 16]) -> HResult {
    if !m.is_null() {
        // SAFETY: D3DMATRIX from the caller.
        let v = unsafe { *m };
        match state {
            2 => *VIEW.lock().unwrap_or_else(|e| e.into_inner()) = v,
            3 => *PROJECTION.lock().unwrap_or_else(|e| e.into_inner()) = v,
            256 => *WORLD.lock().unwrap_or_else(|e| e.into_inner()) = v,
            _ => {}
        }
    }
    let d = real_device();
    // SAFETY: real device, SetTransform(state, matrix).
    let f: extern "system" fn(*mut c_void, u32, *const [f32; 16]) -> HResult = unsafe { vfn(d, dev::SET_TRANSFORM) };
    f(d, state, m)
}

/// Proxy `EndScene` (slot 42): forwards to the real call. samp also runs two integrity checks around it, which
/// have no visible effect and are not reproduced.
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
