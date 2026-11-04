//! The per-player context switch around GTA's own player code (Windows only).
//!
//! Spec: remote-control-onfoot.md §2.2/§2.3 and remote-control-combat.md §1.2.
//! A remote player is a real `CPlayerPed` (player index = ped slot 2…209).
//! GTA's player code reads `CPad::Pads[0]`, `CWorld::PlayerInFocus`, the
//! camera and the weapon-skill stats, so around a remote ped's
//! `CPlayerPed::ProcessControl` (vtable slot 0x86D190, CSV row 19) and
//! `CTaskSimpleUseGun::SetPedPosition` (0x86D744, row 21) those globals are
//! saved, replaced with the slot's shadow state, and restored afterwards.
//!
//! Differences to samp on purpose (documented samp bugs, combat spec §9.2):
//! * every swapped global is restored to its *saved* value, including
//!   `PlayerInFocus` (samp writes 0);
//! * the slot data is copied out of the shared table before the original is
//!   called, so the swap never holds a lock while GTA runs, and a nested call
//!   (focus already ≠ 0) runs the original untouched (samp's guard).
//!
//! Not reproduced: the SEH fault containment around the original
//! (A 0x100A2800, which streams the player out on a fault) — Rust cannot
//! catch SEH; a fault inside GTA's player code ends the game as it would
//! without samp. The saved remote health global (no reader) is skipped.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::remote::{AIM_FLOATS, PAD_SIZE, SKILLS, SlotShadow, SwapTable};

/// GTA globals touched by the switch (onfoot §2.2; names via gta-reversed / plugin-sdk).
pub mod addr {
    /// `CPad::Pads[0]`.
    pub const PAD0: u32 = crate::game::PAD0;
    /// `TheCamera.m_aCams[0].m_bLookingLeft` / `m_bLookingRight`.
    pub const CAM0_LOOKING_LEFT: u32 = 0xB6F1A4;
    pub const CAM0_LOOKING_RIGHT: u32 = 0xB6F1A5;
    /// `TheCamera.m_aCams[0].m_nMode` (written as a byte).
    pub const CAM0_MODE: u32 = 0xB6F1A8;
    /// `TheCamera.m_PlayerWeaponMode.m_nMode` (u16).
    pub const PLAYER_WEAPON_MODE: u32 = 0xB6F858;
    /// `TheCamera.m_aCams[0].m_fFOV`.
    pub const CAM0_FOV: u32 = 0xB6F250;
    /// `CDraw::ms_fAspectRatio`.
    pub const ASPECT_RATIO: u32 = 0xC3EFA4;
    /// `m_aCams[0]` Front, Source, SourceBeforeLookBehind, Up (12 floats).
    pub const CAM0_VECTORS: u32 = 0xB6F32C;
    /// `CStats::StatTypesFloat[69..79]` (weapon skills).
    pub const WEAPON_SKILLS: u32 = 0xB79380 + 0x114;
    /// `CWorld::PlayerInFocus` (u8).
    pub const PLAYER_IN_FOCUS: u32 = 0xB7CD74;
    /// `CPlayerPed` vtable slot +0x28 (`ProcessControl`), CSV row 19.
    pub const VT_PLAYER_PED_PROCESS_CONTROL: u32 = 0x86D190;
    /// `CPlayerPed::ProcessControl`.
    pub const PLAYER_PED_PROCESS_CONTROL: u32 = 0x60EA90;
    /// `CTaskSimpleUseGun` vtable slot +0x20 (`SetPedPosition`), CSV row 21.
    pub const VT_USE_GUN_SET_PED_POSITION: u32 = 0x86D744;
    /// `CTaskSimpleUseGun::SetPedPosition`.
    pub const USE_GUN_SET_PED_POSITION: u32 = 0x624ED0;
    /// `CPlayerPed` vtable slot +0x2C (`ProcessCollision`), CSV row 23.
    pub const VT_PLAYER_PED_PROCESS_COLLISION: u32 = 0x86D194;
    /// `CPhysical::ProcessCollision`.
    pub const PHYSICAL_PROCESS_COLLISION: u32 = 0x54DFB0;
    /// `call CPed::UpdatePosition` inside `CPed::ProcessControl` (CSV row 366).
    pub const UPDATE_POSITION_CALL: u32 = 0x5E92F4;
    /// `fst [esi+0x55C]` in PlayerControlZelda / PlayerControlDucked (CSV rows 213/214, 367).
    pub const AIM_ROT_STORE_ZELDA: u32 = 0x6884C4;
    pub const AIM_ROT_STORE_DUCKED: u32 = 0x688200;
}

pub const UPDATE_POSITION_CALL_ORIGINAL: [u8; 5] = [0xE8, 0x17, 0x88, 0xFF, 0xFF];
pub const AIM_ROT_STORE_ORIGINAL: [u8; 6] = [0xD9, 0x96, 0x5C, 0x05, 0x00, 0x00];

/// Slot → ped table and per-slot shadow state, shared by the client tick
/// (writer) and the hooks (readers). Main thread only in practice.
pub static TABLE: Mutex<Option<SwapTable>> = Mutex::new(None);

/// Runs `f` on the table (created on first use).
pub fn with_table<R>(f: impl FnOnce(&mut SwapTable) -> R) -> R {
    let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(SwapTable::default))
}

static CALLS_REMOTE: AtomicU32 = AtomicU32::new(0);
static CALLS_LOCAL: AtomicU32 = AtomicU32::new(0);

/// (remote, local) ProcessControl calls seen so far (for the log).
pub fn call_counts() -> (u32, u32) {
    (CALLS_REMOTE.load(Ordering::Relaxed), CALLS_LOCAL.load(Ordering::Relaxed))
}

// ---- raw access (pages made writable once at install) ----------------------

unsafe fn rd<T: Copy>(a: u32) -> T {
    // SAFETY: caller passes a GTA global of type T.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

unsafe fn wr<T: Copy>(a: u32, v: T) {
    // SAFETY: caller passes a writable GTA global of type T.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) }
}

unsafe fn copy_in(a: u32, src: &[u8]) {
    // SAFETY: caller passes a writable range of src.len() bytes.
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), a as usize as *mut u8, src.len()) }
}

unsafe fn copy_out(a: u32, dst: &mut [u8]) {
    // SAFETY: caller passes a readable range of dst.len() bytes.
    unsafe { std::ptr::copy_nonoverlapping(a as usize as *const u8, dst.as_mut_ptr(), dst.len()) }
}

fn floats_to_bytes<const N: usize>(f: &[f32; N]) -> Vec<u8> {
    f.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Everything the switch replaces, as found before the call.
struct Saved {
    pad: [u8; PAD_SIZE],
    looking: (u8, u8),
    cam_mode: u8,
    weapon_mode: u16,
    fov: f32,
    aspect: f32,
    vectors: [u8; 4 * AIM_FLOATS],
    skills: [u8; 4 * SKILLS],
    focus: u8,
}

/// Steps 1–7 of onfoot §2.2: save the local values, write the slot's.
unsafe fn enter(slot: u8, s: &SlotShadow) -> Saved {
    // SAFETY: fixed GTA globals (see `addr`), main thread.
    unsafe {
        let mut saved = Saved {
            pad: [0; PAD_SIZE],
            looking: (rd(addr::CAM0_LOOKING_LEFT), rd(addr::CAM0_LOOKING_RIGHT)),
            cam_mode: rd(addr::CAM0_MODE),
            weapon_mode: rd(addr::PLAYER_WEAPON_MODE),
            fov: rd(addr::CAM0_FOV),
            aspect: rd(addr::ASPECT_RATIO),
            vectors: [0; 4 * AIM_FLOATS],
            skills: [0; 4 * SKILLS],
            focus: rd(addr::PLAYER_IN_FOCUS),
        };
        copy_out(addr::PAD0, &mut saved.pad);
        copy_out(addr::CAM0_VECTORS, &mut saved.vectors);
        copy_out(addr::WEAPON_SKILLS, &mut saved.skills);

        copy_in(addr::PAD0, &s.pad);
        let (l, r) = s.looking_left_right();
        wr(addr::CAM0_LOOKING_LEFT, l);
        wr(addr::CAM0_LOOKING_RIGHT, r);
        wr(addr::CAM0_MODE, s.cam_mode);
        wr(addr::PLAYER_WEAPON_MODE, s.weapon_mode());
        wr(addr::CAM0_FOV, s.fov());
        wr(addr::ASPECT_RATIO, s.aspect + 1.0);
        copy_in(addr::CAM0_VECTORS, &floats_to_bytes(&s.aim));
        copy_in(addr::WEAPON_SKILLS, &floats_to_bytes(&s.skills));
        wr(addr::PLAYER_IN_FOCUS, slot);
        saved
    }
}

/// Restores every value saved by [`enter`].
unsafe fn leave(saved: &Saved) {
    // SAFETY: fixed GTA globals, main thread.
    unsafe {
        copy_in(addr::WEAPON_SKILLS, &saved.skills);
        wr(addr::CAM0_MODE, saved.cam_mode);
        wr(addr::PLAYER_WEAPON_MODE, saved.weapon_mode);
        wr(addr::CAM0_FOV, saved.fov);
        wr(addr::ASPECT_RATIO, saved.aspect);
        wr(addr::PLAYER_IN_FOCUS, saved.focus);
        copy_in(addr::PAD0, &saved.pad);
        wr(addr::CAM0_LOOKING_LEFT, saved.looking.0);
        wr(addr::CAM0_LOOKING_RIGHT, saved.looking.1);
        copy_in(addr::CAM0_VECTORS, &saved.vectors);
    }
}

/// The slot and a copy of its shadow if `ped` is a remote ped and no switch
/// is active (samp: slot ≠ 0 and PlayerInFocus == 0).
fn remote_context(ped: u32) -> Option<(u8, SlotShadow)> {
    // SAFETY: byte global.
    if unsafe { rd::<u8>(addr::PLAYER_IN_FOCUS) } != 0 {
        return None;
    }
    let g = TABLE.try_lock().ok()?;
    let t = g.as_ref()?;
    let slot = t.slot_of(ped)?;
    Some((slot as u8, t.shadows[slot]))
}

/// Replacement for `CPlayerPed::ProcessControl` (thiscall, vtable 0x86D190).
pub extern "thiscall" fn player_ped_process_control(ped: u32) {
    // SAFETY: GTA's own function with the same signature.
    let original: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(addr::PLAYER_PED_PROCESS_CONTROL as usize) };
    match remote_context(ped) {
        Some((slot, shadow)) => {
            CALLS_REMOTE.fetch_add(1, Ordering::Relaxed);
            // SAFETY: main thread; code sites made writable at install.
            unsafe {
                let saved = enter(slot, &shadow);
                copy_in(addr::UPDATE_POSITION_CALL, &[0x90; 5]);
                original(ped);
                copy_in(addr::UPDATE_POSITION_CALL, &UPDATE_POSITION_CALL_ORIGINAL);
                leave(&saved);
            }
        }
        None => {
            CALLS_LOCAL.fetch_add(1, Ordering::Relaxed);
            // Local player: the heading-goal stores run, then are NOPed again
            // so remote peds never get them.
            // SAFETY: as above.
            unsafe {
                copy_in(addr::AIM_ROT_STORE_ZELDA, &AIM_ROT_STORE_ORIGINAL);
                copy_in(addr::AIM_ROT_STORE_DUCKED, &AIM_ROT_STORE_ORIGINAL);
                original(ped);
                copy_in(addr::AIM_ROT_STORE_ZELDA, &[0x90; 6]);
                copy_in(addr::AIM_ROT_STORE_DUCKED, &[0x90; 6]);
            }
        }
    }
}

/// Replacement for `CTaskSimpleUseGun::SetPedPosition(CPed*)` (thiscall, vtable 0x86D744).
pub extern "thiscall" fn use_gun_set_ped_position(task: u32, ped: u32) -> u32 {
    // SAFETY: GTA's own function with the same signature.
    let original: extern "thiscall" fn(u32, u32) -> u32 =
        unsafe { std::mem::transmute(addr::USE_GUN_SET_PED_POSITION as usize) };
    match remote_context(ped) {
        Some((slot, shadow)) => {
            // SAFETY: main thread.
            unsafe {
                let saved = enter(slot, &shadow);
                let r = original(task, ped);
                leave(&saved);
                r
            }
        }
        None => original(task, ped),
    }
}

/// Replacement for `CPlayerPed::ProcessCollision` (thiscall, vtable 0x86D194):
/// skipped while entity flag bit 31 (samp's "no collision" marker, onfoot §3.6) is set.
pub extern "thiscall" fn player_ped_process_collision(ent: u32) {
    if ent == 0 {
        return;
    }
    // SAFETY: `m_nFlags` of a live entity.
    if unsafe { rd::<u32>(ent + 0x1C) } & 0x8000_0000 != 0 {
        return;
    }
    // SAFETY: GTA's own function with the same signature.
    let original: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(addr::PHYSICAL_PROCESS_COLLISION as usize) };
    original(ent);
}

/// Makes `len` bytes at `a` writable (code pages stay RWX afterwards).
fn unprotect(a: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    let mut old = 0u32;
    // SAFETY: plain Win32 call on our own address space.
    unsafe { VirtualProtect(a as usize as *const core::ffi::c_void, len, PAGE_EXECUTE_READWRITE, &mut old) != 0 }
}

/// Installs the three vtable redirects (CSV rows 19, 21 and 23). The 0x6884C4 /
/// 0x688200 NOPs (rows 213/214) come from the patch table.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Result<(), PatchError> {
    let mut b = [0u8; 5];
    mem.read(addr::UPDATE_POSITION_CALL, &mut b)?;
    if b != UPDATE_POSITION_CALL_ORIGINAL {
        return Err(PatchError::Mismatch {
            addr: addr::UPDATE_POSITION_CALL,
            expected: UPDATE_POSITION_CALL_ORIGINAL.to_vec(),
            found: b.to_vec(),
        });
    }
    for (a, n) in [(addr::UPDATE_POSITION_CALL, 5), (addr::AIM_ROT_STORE_ZELDA, 6), (addr::AIM_ROT_STORE_DUCKED, 6)] {
        if !unprotect(a, n) {
            return Err(PatchError::Access { addr: a, len: n });
        }
    }
    let pc = player_ped_process_control as extern "thiscall" fn(u32) as usize as u32;
    let ug = use_gun_set_ped_position as extern "thiscall" fn(u32, u32) -> u32 as usize as u32;
    let expect = |slot: u32, want: u32| -> Result<(), PatchError> {
        let found = ProcessMemory.read_u32(slot)?;
        if found == want {
            Ok(())
        } else {
            Err(PatchError::Mismatch { addr: slot, expected: want.to_le_bytes().to_vec(), found: found.to_le_bytes().to_vec() })
        }
    };
    expect(addr::VT_PLAYER_PED_PROCESS_CONTROL, addr::PLAYER_PED_PROCESS_CONTROL)?;
    expect(addr::VT_USE_GUN_SET_PED_POSITION, addr::USE_GUN_SET_PED_POSITION)?;
    expect(addr::VT_PLAYER_PED_PROCESS_COLLISION, addr::PHYSICAL_PROCESS_COLLISION)?;
    patcher.swap_pointer(mem, "CPlayerPed::ProcessControl vtable", addr::VT_PLAYER_PED_PROCESS_CONTROL, pc)?;
    patcher.swap_pointer(mem, "CTaskSimpleUseGun::SetPedPosition vtable", addr::VT_USE_GUN_SET_PED_POSITION, ug)?;
    let pcol = player_ped_process_collision as extern "thiscall" fn(u32) as usize as u32;
    patcher.swap_pointer(mem, "CPlayerPed::ProcessCollision vtable", addr::VT_PLAYER_PED_PROCESS_COLLISION, pcol)?;
    with_table(|_| ());
    Ok(())
}
