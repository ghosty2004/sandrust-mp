//! Direct3D 9 plumbing for the UI pass (Windows only): the device proxy, D3DX loading and thin wrappers for
//! fonts, sprites, lines, render targets and state blocks.
//! On the first frame GTA's `_RwD3DDevice` pointer is replaced by a one-word object whose generated thunks
//! forward every `IDirect3DDevice9` method to the real device (swapping `this`), except the few we intercept
//! (Present, Reset, SetTransform, SetTexture, EndScene, ...). Our own drawing always uses the real device.
//! The colour-code wrapper draws text through a proxy `ID3DXSprite` whose `Draw` recolours each glyph.
//! D3DX comes from `d3dx9_25.dll`, loaded at run time like samp does.
//! Spec: docs/re/subsystems/ui-rendering.md ("Device proxy", "Embedded colour codes").

use std::ffi::c_void;

mod d3dx;
pub mod devcall;
mod proxy;
mod resources;
mod text;

pub use d3dx::*;
pub use proxy::*;
pub use resources::*;
pub use text::*;

/// A COM `HRESULT` (negative = failure).
pub type HResult = i32;

/// `_RwD3DDevice` in gta_sa.exe 1.0 US.
pub const RW_D3D_DEVICE: u32 = 0xC97C28;

/// `IDirect3DDevice9` vtable slots.
pub mod dev {
    pub const SLOTS: usize = 119;
    pub const SET_CURSOR_PROPERTIES: usize = 10;
    pub const SHOW_CURSOR: usize = 12;
    pub const TEST_COOPERATIVE_LEVEL: usize = 3;
    pub const RESET: usize = 16;
    pub const CREATE_VERTEX_BUFFER: usize = 26;
    pub const PRESENT: usize = 17;
    pub const GET_FRONT_BUFFER_DATA: usize = 33;
    pub const CREATE_OFFSCREEN_PLAIN_SURFACE: usize = 36;
    pub const END_SCENE: usize = 42;
    pub const CLEAR: usize = 43;
    pub const SET_TRANSFORM: usize = 44;
    pub const CREATE_STATE_BLOCK: usize = 59;
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
    // SAFETY: the caller guarantees a live COM object with that slot and the method's exact signature.
    unsafe {
        let vtbl = *(obj as *const *const usize);
        let p = *vtbl.add(slot);
        std::mem::transmute_copy::<usize, F>(&p)
    }
}

/// Win32 `RECT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
