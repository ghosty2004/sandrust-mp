//! `d3dx9_25.dll`, loaded at run time like samp does: the D3DX entry points the UI uses.

use std::ffi::c_void;

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use super::{HResult, IDENTITY, Rect};

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
type CreateLine = extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
type LoadSurfaceFromSurface = extern "system" fn(
    *mut c_void,
    *const c_void,
    *const Rect,
    *mut c_void,
    *const c_void,
    *const Rect,
    u32,
    u32,
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
    pub create_line: CreateLine,
    pub load_surface_from_surface: LoadSurfaceFromSurface,
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
                create_line: std::mem::transmute::<usize, CreateLine>(get(b"D3DXCreateLine\0")?),
                load_surface_from_surface: std::mem::transmute::<usize, LoadSurfaceFromSurface>(get(
                    b"D3DXLoadSurfaceFromSurface\0",
                )?),
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
