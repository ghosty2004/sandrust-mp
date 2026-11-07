//! Keeps GTA from hiding the mouse cursor for good by neutralising `user32!ShowCursor`.
//!
//! GTA calls `ShowCursor(FALSE)` on every window activation and `WM_SETCURSOR`, which drives the display counter
//! below zero before our window procedure is installed; Windows (and Wine) then hide every cursor, including the
//! D3D hardware cursor used for chat input, dialogs and the scoreboard. samp swaps in a stub that never touches
//! the counter. We do the same, but patch gta_sa.exe's import slot instead of inline-hooking the user32 export
//! (gta_sa.exe is its only other caller; d3d9 and d3dx do not import it). The object editor still calls the
//! real function (`ui::glue`). Spec: docs/re/subsystems/ui-rendering.md, "5. Cursor and input-lock modes".

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::patch::{Memory, Patcher, ProcessMemory};

/// Replacement for `ShowCursor`: leaves the display counter alone and answers 0 for "show", −1 for "hide".
/// Mirrors samp.dll 0x10062960.
pub extern "system" fn show_cursor_stub(show: i32) -> i32 {
    if show != 0 { 0 } else { -1 }
}

/// Redirects gta_sa.exe's `ShowCursor` import to [`show_cursor_stub`] (attach time,
/// main thread still suspended).
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Result<(), String> {
    // SAFETY: GetModuleHandleW(NULL) returns the base of the main executable.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as u32;
    let slot = crate::pe::iat_slots(&ImageReader, base, "user32.dll", &["ShowCursor"])[0]
        .ok_or("gta_sa.exe does not import ShowCursor from user32")?;
    let orig = mem.read_u32(slot).map_err(|e| e.to_string())?;
    let stub = show_cursor_stub as extern "system" fn(i32) -> i32 as usize as u32;
    patcher
        .write(mem, "user32 ShowCursor import", slot, &stub.to_le_bytes(), Some(&orig.to_le_bytes()))
        .map_err(|e| e.to_string())?;
    crate::log!("early: ShowCursor import at {slot:#x}: {orig:#x} -> {stub:#x} (counter left alone)");
    Ok(())
}

struct ImageReader;

impl crate::pe::Reader for ImageReader {
    fn bytes(&self, va: u32, len: usize) -> Option<Vec<u8>> {
        let mut b = vec![0u8; len];
        ProcessMemory.read(va, &mut b).ok().map(|_| b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_answers_like_samp() {
        assert_eq!(show_cursor_stub(1), 0);
        assert_eq!(show_cursor_stub(0), -1);
        assert_eq!(show_cursor_stub(5), 0);
    }
}
