//! Start-up hooks that belong to no feature module (Windows only):
//! * the HUD scale fix around `CHud::DrawRadar` ([`install`]) and `CHud::DrawCrossHairs` ([`crosshairs`], called
//!   from combathooks.rs), switchable through sa-mp.cfg `nohudscalefix` and `/hudscalefix` ([`set_enabled`]);
//! * `Sleep(1)` before the first time read of Idle's 14 ms busy-wait;
//! * typed single-player cheats do nothing (`CCheat::AddToCheatString`).
//!
//! Spec: docs/re/subsystems/ui-rendering.md, "Other hooks that matter for UI"; docs/re/subsystems/gta-patches.md.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::patch::{PatchError, Patcher, ProcessMemory};

/// Screen-scale float, `1/640` in the file.
const SCALE_X: u32 = 0x859520;
/// Screen-scale float, `1/448` in the file.
const SCALE_Y: u32 = 0x859524;
/// First of two HUD floats chosen by the aspect ratio (76.0 in the file).
const HUD_A: u32 = 0x866B74;
/// Second HUD float (94.0 in the file).
const HUD_B: u32 = 0x866B78;
/// `RsGlobal.maximumWidth`.
const SCREEN_W: u32 = 0xC17044;
/// `RsGlobal.maximumHeight`.
const SCREEN_H: u32 = 0xC17048;
/// Bits samp writes into `SCALE_Y` on a wide screen.
const WIDE_SCALE_Y: u32 = 0x3B1E_98DD;
/// On a wide screen `SCALE_X` := this / aspect.
const WIDE_NUMERATOR: f32 = 0.00222;
/// Aspect ratio from which the screen counts as wide.
const WIDE_MIN_ASPECT: f32 = 1.6;
const HUD_NARROW: (f32, f32) = (76.0, 94.0);
const HUD_WIDE: (f32, f32) = (82.0, 96.0);

/// `CHud::Draw` → `CHud::DrawRadar` call.
pub const RADAR_CALL: u32 = 0x58FC53;
/// `CHud::DrawRadar`.
const DRAW_RADAR: u32 = 0x58A330;
/// Idle's busy-wait `call CTimer::GetCurrentTimeInCycles`.
pub const IDLE_WAIT_CALL: u32 = 0x53E930;
/// `CTimer::GetCurrentTimeInCycles`.
const GET_CURRENT_TIME_IN_CYCLES: u32 = 0x561A80;
/// `CCheat::AddToCheatString`: the 7-byte `mov byte [m_bHasPlayerCheated],1` right before the cheat function is
/// called.
pub const CHEAT_SITE: u32 = 0x438576;
const CHEAT_SITE_ORIGINAL: [u8; 7] = [0xC6, 0x05, 0x8C, 0x91, 0x96, 0x00, 0x01];
/// Pointer slot in `AddToCheatString`'s alignment padding (target of our `jmp [slot]`).
const CHEAT_SLOT: u32 = 0x4385AA;
const CHEAT_SLOT_ORIGINAL: [u8; 4] = [0x90; 4];
/// `CCheat::m_bHasPlayerCheated`.
const HAS_PLAYER_CHEATED: u32 = 0x96918C;
/// `AddToCheatString` right after the `call eax` that runs the cheat (the toggle branch is not taken from here).
const CHEAT_RESUME: u32 = 0x438581;
/// `TheCamera.m_aCams[0].m_nMode` and the mode the crosshair correction applies to.
const CAM0_MODE: u32 = crate::swap::addr::CAM0_MODE;
const CROSSHAIR_FIX_MODE: u16 = 0x35;

/// Whether the HUD scale fix runs (on unless sa-mp.cfg `nohudscalefix` is exactly 1). Mirrors samp.dll 0x10117470.
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

/// Saves both screen-scale floats, then by aspect ratio: below 1.6 the HUD floats get 76.0 / 94.0 and nothing is
/// marked; otherwise HUD floats := 82.0 / 96.0, the scale floats are set for the wide screen and the correction is
/// marked active, so [`restore`] undoes it after the draw. Nothing happens with a non-positive screen size.
/// Mirrors samp.dll 0x100A2440.
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

/// `CHud::DrawRadar` with the correction around it. Mirrors samp.dll 0x100A2540.
extern "C" fn radar_site() {
    correct();
    // SAFETY: the original cdecl function.
    unsafe { std::mem::transmute::<usize, extern "C" fn()>(DRAW_RADAR as usize)() };
    restore();
}

/// Runs `draw` (`CHud::DrawCrossHairs`) with the correction while the local camera mode is 0x35.
/// Mirrors samp.dll 0x100A2590.
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

/// `Sleep(1)`, then the original time read. Mirrors samp.dll 0x100A2420.
extern "C" fn idle_wait_time() -> u32 {
    // SAFETY: Win32 Sleep; then the original cdecl `CTimer::GetCurrentTimeInCycles`.
    unsafe {
        windows_sys::Win32::System::Threading::Sleep(1);
        std::mem::transmute::<usize, extern "C" fn() -> u32>(GET_CURRENT_TIME_IN_CYCLES as usize)()
    }
}

/// Keeps the "has cheated" write and resumes after `call eax`, so neither the cheat function nor the toggle branch
/// runs. (samp also stores the cheat function pointer into a global nothing reads; not replicated.)
/// Mirrors samp.dll 0x100A3AA0.
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

/// Makes the two `.rdata` float ranges writable (as samp does once at start-up), then installs the radar hook, the
/// idle-wait hook and the cheat hook. Returns one result per hook, keyed by its row in
/// docs/re/data/gta-patches-r4-annotated.csv.
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
