//! Init-main hooks that belong to no feature module (Windows only; from the parity audit, docs/phase3/par-audit.md).
//!
//! * HUD scale fix (CSV rows 7, 368, 369; also used by the row 8 crosshair wrapper in combathooks.rs). Static
//!   reading of samp.dll A 0x100A2440 (correction), 0x100A2540 (radar wrapper) and 0x100A2590 (crosshair wrapper):
//!   - only with samp's flag A 0x10117470 (initial value 1; start-up clears it when sa-mp.cfg `nohudscalefix` is
//!     exactly 1, A 0x100C49DD; `/hudscalefix` toggles it);
//!   - the two screen-scale floats 0x859520/0x859524 are saved, then the screen size `RsGlobal` 0xC17044/0xC17048 is
//!     read; with a non-positive width or height nothing else happens;
//!   - aspect = width / height (x87). Below 1.6 the HUD floats 0x866B74/0x866B78 := 76.0/94.0 and nothing is
//!     marked; otherwise HUD floats := 82.0/96.0, 0x859524 := 0x3B1E98DD and 0x859520 := 0.00222 / aspect, and the
//!     correction is marked active;
//!   - after the wrapped draw an active correction is undone: both scale floats back to the saved values, HUD floats
//!     := 76.0/94.0, mark cleared;
//!   - the radar wrapper always corrects around `CHud::DrawRadar`; the crosshair wrapper only while the local
//!     camera mode (`TheCamera.m_aCams[0].m_nMode`) is 0x35.
//!
//!   samp makes both .rdata ranges writable once at start-up (A 0x100AAEF0); so do we.
//! * Row 16: the first `CTimer::GetCurrentTimeInCycles` call of Idle's 14 ms busy-wait goes through samp
//!   (A 0x100A2420), which calls `Sleep(1)` first and then the original.
//! * Row 50: `CCheat::AddToCheatString`, at `mov [CCheat::m_bHasPlayerCheated],1` right before the cheat function
//!   would be called: samp (A 0x100A3AA0) keeps the flag write and resumes at 0x438581, i.e. neither the cheat
//!   function (`call eax`) nor the toggle branch (0x43858E) runs — typed cheats do nothing. samp also stores EAX (the
//!   cheat function) into a global that nothing reads (only reference: A 0x100A3AA0); not replicated.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::patch::{PatchError, Patcher, ProcessMemory};

/// Screen-scale floats (`1/640`, `1/448` in the file).
const SCALE_X: u32 = 0x859520;
const SCALE_Y: u32 = 0x859524;
/// Two HUD floats chosen by the aspect ratio (76.0 / 94.0 in the file).
const HUD_A: u32 = 0x866B74;
const HUD_B: u32 = 0x866B78;
/// `RsGlobal.maximumWidth` / `maximumHeight`.
const SCREEN_W: u32 = 0xC17044;
const SCREEN_H: u32 = 0xC17048;
const WIDE_SCALE_Y: u32 = 0x3B1E_98DD;
const WIDE_NUMERATOR: f32 = 0.00222;
const WIDE_MIN_ASPECT: f32 = 1.6;
const HUD_NARROW: (f32, f32) = (76.0, 94.0);
const HUD_WIDE: (f32, f32) = (82.0, 96.0);

/// `CHud::Draw` → `CHud::DrawRadar` call (row 7).
pub const RADAR_CALL: u32 = 0x58FC53;
const DRAW_RADAR: u32 = 0x58A330;
/// Idle's busy-wait `call CTimer::GetCurrentTimeInCycles` (row 16).
pub const IDLE_WAIT_CALL: u32 = 0x53E930;
const GET_CURRENT_TIME_IN_CYCLES: u32 = 0x561A80;
/// `CCheat::AddToCheatString`: the 7-byte `mov byte [m_bHasPlayerCheated],1` (row 50), the pointer slot in the
/// function's alignment padding and the resume address after `call eax`.
pub const CHEAT_SITE: u32 = 0x438576;
const CHEAT_SITE_ORIGINAL: [u8; 7] = [0xC6, 0x05, 0x8C, 0x91, 0x96, 0x00, 0x01];
const CHEAT_SLOT: u32 = 0x4385AA;
const CHEAT_SLOT_ORIGINAL: [u8; 4] = [0x90; 4];
const HAS_PLAYER_CHEATED: u32 = 0x96918C;
const CHEAT_RESUME: u32 = 0x438581;
/// `TheCamera.m_aCams[0].m_nMode` and the mode the crosshair correction applies to.
const CAM0_MODE: u32 = crate::swap::addr::CAM0_MODE;
const CROSSHAIR_FIX_MODE: u16 = 0x35;

/// samp's flag A 0x10117470 (`nohudscalefix` ≠ 1).
static ENABLED: AtomicBool = AtomicBool::new(true);
/// Set once the .rdata ranges are writable.
static WRITABLE: AtomicBool = AtomicBool::new(false);
static SAVED_X: AtomicU32 = AtomicU32::new(0);
static SAVED_Y: AtomicU32 = AtomicU32::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// sa-mp.cfg `nohudscalefix` / `/hudscalefix` (flag on = correction runs).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

fn rd(a: u32) -> u32 {
    // SAFETY: fixed addresses inside gta_sa.exe's image.
    unsafe { std::ptr::read_volatile(a as usize as *const u32) }
}

fn wr(a: u32, v: u32) {
    // SAFETY: the two .rdata ranges were made writable at install (WRITABLE is checked by the callers).
    unsafe { std::ptr::write_volatile(a as usize as *mut u32, v) }
}

fn set_hud(v: (f32, f32)) {
    wr(HUD_A, v.0.to_bits());
    wr(HUD_B, v.1.to_bits());
}

/// `w / h >= 1.6` and `0.00222 / (w / h)`, both on the x87 unit like samp, so the result follows the game's FPU
/// precision control (16:10 sits exactly on the threshold).
fn wide_scale(w: i32, h: i32) -> Option<f32> {
    let lt: u32;
    let mut out = 0f32;
    // SAFETY: x87 arithmetic on locals; the stack is balanced (fild ×2, fdivp, fld, fcomp → 1 entry, fdivr, fstp).
    unsafe {
        std::arch::asm!(
            "fild dword ptr [{w}]",
            "fild dword ptr [{h}]",
            "fdivp st(1), st",
            "fld st(0)",
            "fcomp dword ptr [{min}]",
            "fnstsw ax",
            "and eax, 0x100",
            "fdivr dword ptr [{num}]",
            "fstp dword ptr [{out}]",
            w = in(reg) &w,
            h = in(reg) &h,
            min = in(reg) &WIDE_MIN_ASPECT,
            num = in(reg) &WIDE_NUMERATOR,
            out = in(reg) &mut out,
            out("eax") lt,
        );
    }
    (lt == 0).then_some(out)
}

/// A 0x100A2440.
fn correct() {
    if !ENABLED.load(Ordering::Relaxed) || !WRITABLE.load(Ordering::Relaxed) {
        return;
    }
    SAVED_X.store(rd(SCALE_X), Ordering::Relaxed);
    SAVED_Y.store(rd(SCALE_Y), Ordering::Relaxed);
    let (w, h) = (rd(SCREEN_W) as i32, rd(SCREEN_H) as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    match wide_scale(w, h) {
        Some(x) => {
            set_hud(HUD_WIDE);
            wr(SCALE_Y, WIDE_SCALE_Y);
            wr(SCALE_X, x.to_bits());
            ACTIVE.store(true, Ordering::Relaxed);
        }
        None => set_hud(HUD_NARROW),
    }
}

fn restore() {
    if ACTIVE.swap(false, Ordering::Relaxed) {
        wr(SCALE_X, SAVED_X.load(Ordering::Relaxed));
        wr(SCALE_Y, SAVED_Y.load(Ordering::Relaxed));
        set_hud(HUD_NARROW);
    }
}

/// Row 7 (A 0x100A2540): `CHud::DrawRadar` with the correction around it.
extern "C" fn radar_site() {
    correct();
    // SAFETY: the original cdecl function.
    unsafe { std::mem::transmute::<usize, extern "C" fn()>(DRAW_RADAR as usize)() };
    restore();
}

/// Row 8 (A 0x100A2590): runs `draw` (`CHud::DrawCrossHairs`) with the correction while the local camera mode is
/// 0x35.
pub fn crosshairs(draw: impl FnOnce()) {
    // SAFETY: TheCamera is a static of the executable.
    let mode = unsafe { std::ptr::read_volatile(CAM0_MODE as usize as *const u16) };
    let fixed = mode == CROSSHAIR_FIX_MODE;
    if fixed {
        correct();
    }
    draw();
    if fixed {
        restore();
    }
}

/// Row 16 (A 0x100A2420).
extern "C" fn idle_wait_time() -> u32 {
    // SAFETY: Win32 Sleep; then the original cdecl `CTimer::GetCurrentTimeInCycles`.
    unsafe {
        windows_sys::Win32::System::Threading::Sleep(1);
        std::mem::transmute::<usize, extern "C" fn() -> u32>(GET_CURRENT_TIME_IN_CYCLES as usize)()
    }
}

/// Row 50 (A 0x100A3AA0): keep the "has cheated" write, skip the cheat.
#[unsafe(naked)]
unsafe extern "C" fn cheat_stub() {
    std::arch::naked_asm!(
        "mov byte ptr [{flag}], 1",
        "mov edx, {resume}",
        "jmp edx",
        flag = const HAS_PLAYER_CHEATED,
        resume = const CHEAT_RESUME,
    )
}

fn unprotect(a: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
    let mut old = 0u32;
    // SAFETY: plain Win32 call on our own address space.
    unsafe { VirtualProtect(a as usize as *const core::ffi::c_void, len, PAGE_READWRITE, &mut old) != 0 }
}

/// Installs rows 7 (with 368/369), 16 and 50. Returns one result per row.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    let skip = crate::patches::skipped_rows();
    let writable = unprotect(SCALE_X, 8) && unprotect(HUD_A, 8);
    WRITABLE.store(writable, Ordering::SeqCst);
    let r = if writable {
        patcher
            .retarget_branch(
                mem,
                "CHud::DrawRadar (HUD scale fix)",
                RADAR_CALL,
                radar_site as *const () as usize as u32,
                Some(DRAW_RADAR),
            )
            .map(|_| ())
    } else {
        Err(PatchError::Access { addr: SCALE_X, len: 8 })
    };
    out.push((7, r));
    // Diagnostics: SAMP_RS_SKIP_CSV_ROWS also leaves these hooks out.
    if !skip.contains(&16) {
        let r = patcher
            .retarget_branch(
                mem,
                "Idle busy-wait Sleep(1)",
                IDLE_WAIT_CALL,
                idle_wait_time as *const () as usize as u32,
                Some(GET_CURRENT_TIME_IN_CYCLES),
            )
            .map(|_| ());
        out.push((16, r));
    }
    let r = patcher
        .write(
            mem,
            "AddToCheatString slot",
            CHEAT_SLOT,
            &(cheat_stub as *const () as usize as u32).to_le_bytes(),
            Some(&CHEAT_SLOT_ORIGINAL),
        )
        .and_then(|()| {
            let mut b = vec![0xFF, 0x25];
            b.extend_from_slice(&CHEAT_SLOT.to_le_bytes());
            b.push(crate::patch::OP_NOP);
            patcher.write(mem, "AddToCheatString: no cheats", CHEAT_SITE, &b, Some(&CHEAT_SITE_ORIGINAL))
        });
    out.push((50, r));
    out
}
