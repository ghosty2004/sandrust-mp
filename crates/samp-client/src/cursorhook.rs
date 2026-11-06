//! `user32!ShowCursor` neutralised for the game (UR §5, CURSOR-1).
//!
//! samp replaces `ShowCursor` at start-up (A 0x10062970, B 0x10062930, called from
//! B 0x100C5201) with a stub that leaves the display counter alone and answers
//! 0 for "show" and −1 for "hide". GTA calls `ShowCursor(FALSE)` from its window
//! procedure on every activation (0x747FE9) and every `WM_SETCURSOR` it sees
//! (0x7481CD, also `SetCursor(NULL)`) and once at shutdown (0x748EAF). Without
//! the stub the counter drops below zero before samp's window procedure is in
//! place, and Windows (Wine included) then hides every cursor: the D3D hardware
//! cursor that DXVK turns into a Win32 cursor (`CreateIconIndirect` +
//! `SetCursor`) for chat input, dialogs and the scoreboard never shows.
//! samp's own object editor reaches the real function through the saved
//! original (B 0x10073610 / 0x10073684), as ours does (`ui::glue`).
//!
//! Deviation from samp: samp inline-hooks the user32 export (a 5-byte jump),
//! which also catches other modules; we patch gta_sa.exe's import slot, the only
//! caller besides the client itself (d3d9/DXVK and d3dx do not import
//! `ShowCursor`). Measured under Wine 11 + DXVK 3.1.1 with `WINEDEBUG=+cursor`:
//! the original makes no `NtUserShowCursor` call at all during boot and play.

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::patch::{Memory, Patcher, ProcessMemory};

/// samp's replacement (B 0x10062920): the counter is never changed.
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
    patcher.write(mem, "user32 ShowCursor import", slot, &stub.to_le_bytes(), Some(&orig.to_le_bytes()))
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
