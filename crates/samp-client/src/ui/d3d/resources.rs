//! Textures, render targets, the cursor surface, the SA:MP font resources, the F8 screenshot and state blocks.

use std::ffi::c_void;

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use super::{D3DX_DLL, D3dx, HResult, dev, real_device, release, vfn};

/// Loads a texture file (DXUT-style: full mip chain, managed pool).
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
        DEFAULT, // mip levels: D3DX_DEFAULT (full chain), as samp loads sampgui.png
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

/// A texture render target as samp makes the chat and plate targets: `D3DXCreateTexture(w, h, 1, RENDERTARGET,
/// format, DEFAULT)`, `D3DXCreateRenderToSurface(w, h, format, depth, D16)` and the texture's level-0 surface
/// (ui-rendering.md, "Render-to-texture cache").
pub struct RenderTarget {
    pub tex: *mut c_void,
    pub rts: *mut c_void,
    pub surface: *mut c_void,
}

impl RenderTarget {
    pub fn create(d3dx: &D3dx, w: u32, h: u32, format: u32) -> Result<Self, String> {
        let mut tex = std::ptr::null_mut();
        let hr = (d3dx.create_texture)(real_device(), w, h, 1, 1, format, 0, &mut tex);
        if hr < 0 || tex.is_null() {
            return Err(format!("D3DXCreateTexture({w}x{h} RT) failed: {hr:#x}"));
        }
        let mut rts = std::ptr::null_mut();
        let hr = (d3dx.create_render_to_surface)(real_device(), w, h, format, 1, 80, &mut rts);
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

    /// `ID3DXRenderToSurface::EndScene(0)` (no mip filter).
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

/// The D3D hardware cursor from `mouse.png`: a 32×32 A8R8G8B8 system-memory surface set with
/// `SetCursorProperties(0, 0, surface)` (ui-rendering.md, "Cursor and input-lock modes").
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
        1, // D3DX_FILTER_NONE, as samp
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

/// Registers the SA:MP font files from the game directory (ui-rendering.md, "Font assets"):
/// `AddFontResourceA("gtaweap3.ttf")`, `AddFontResourceA("sampaux3.ttf")`. Mirrors samp.dll 0x100C5100.
pub fn add_font_resources() {
    use windows_sys::Win32::Graphics::Gdi::AddFontResourceA;
    for f in [&b"gtaweap3.ttf\0"[..], b"sampaux3.ttf\0"] {
        // SAFETY: NUL-terminated relative path (the process directory is the GTA folder).
        let n = unsafe { AddFontResourceA(f.as_ptr()) };
        crate::log!("ui: AddFontResourceA({}) = {n}", String::from_utf8_lossy(&f[..f.len() - 1]));
    }
}

// ---- F8 screenshot -----------------------------------------------------------------------

/// F8 screenshot: a desktop-sized `A8R8G8B8` scratch surface, `GetFrontBufferData` (the previous frame with its
/// UI), then the window's client rect in screen coordinates saved as PNG with `D3DXSaveSurfaceToFileA`. Returns
/// false when the front buffer could not be read; the save result is not checked. Like samp, the surface is
/// never released. Mirrors samp.dll 0x10075620.
pub fn screenshot(path: &[u8], hwnd: *mut c_void) -> bool {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    type CreateOffscreen =
        extern "system" fn(*mut c_void, u32, u32, u32, u32, *mut *mut c_void, *mut c_void) -> HResult;
    type FrontBuffer = extern "system" fn(*mut c_void, u32, *mut c_void) -> HResult;
    type SaveSurface = extern "system" fn(*const u8, u32, *mut c_void, *const c_void, *const RECT) -> HResult;
    let dev = real_device();
    if dev.is_null() {
        return false;
    }
    let mut surf: *mut c_void = std::ptr::null_mut();
    // SAFETY: real device methods with their documented signatures; Win32 calls on the game's
    // own window; D3DX export from the module samp uses.
    unsafe {
        let (h, w) = (GetSystemMetrics(SM_CYSCREEN), GetSystemMetrics(SM_CXSCREEN));
        let create: CreateOffscreen = vfn(dev, dev::CREATE_OFFSCREEN_PLAIN_SURFACE);
        let _ = create(dev, w as u32, h as u32, 0x15, 3, &mut surf, std::ptr::null_mut());
        let fb: FrontBuffer = vfn(dev, dev::GET_FRONT_BUFFER_DATA);
        if fb(dev, 0, surf) < 0 {
            return false;
        }
        let mut pt = POINT { x: 0, y: 0 };
        ClientToScreen(hwnd, &mut pt);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        rc.left += pt.x;
        rc.right += pt.x;
        rc.top += pt.y;
        rc.bottom += pt.y;
        let m = LoadLibraryA(D3DX_DLL.as_ptr());
        if let Some(save) = GetProcAddress(m, c"D3DXSaveSurfaceToFileA".as_ptr().cast()) {
            let save: SaveSurface = std::mem::transmute(save);
            let mut p = path.to_vec();
            p.push(0);
            let _ = save(p.as_ptr(), 3, surf, std::ptr::null(), &rc);
        }
    }
    true
}

// ---- state blocks -----------------------------------------------------------------------

/// `IDirect3DStateBlock9::Capture` (slot 4); no-op for null.
pub fn state_block_capture(sb: *mut c_void) {
    if !sb.is_null() {
        // SAFETY: live state block.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(sb, 4) };
        f(sb);
    }
}

/// `IDirect3DStateBlock9::Apply` (slot 5); no-op for null.
pub fn state_block_apply(sb: *mut c_void) {
    if !sb.is_null() {
        // SAFETY: live state block.
        let f: extern "system" fn(*mut c_void) -> HResult = unsafe { vfn(sb, 5) };
        f(sb);
    }
}

/// `D3DXCreateTextureFromFileInMemory` from the D3DX build samp uses.
pub fn texture_from_memory(_d3dx: &D3dx, bytes: &[u8]) -> Option<*mut c_void> {
    type F = extern "system" fn(*mut c_void, *const u8, u32, *mut *mut c_void) -> HResult;
    // SAFETY: the module is loaded at start-up; documented D3DX signature.
    unsafe {
        let m = LoadLibraryA(D3DX_DLL.as_ptr());
        let f: F = std::mem::transmute(GetProcAddress(m, c"D3DXCreateTextureFromFileInMemory".as_ptr().cast())?);
        let mut tex = std::ptr::null_mut();
        (f(real_device(), bytes.as_ptr(), bytes.len() as u32, &mut tex) >= 0 && !tex.is_null()).then_some(tex)
    }
}
