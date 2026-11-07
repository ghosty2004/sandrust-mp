//! Installs the combat hooks: the start-up rows, the vehicle friendly-fire jmp and the InitGame rows.

use super::damage::{affects_ped_stub, damage_stub, inflict_stub};
use super::shot::{SELECTOR, add_bullet_site, fire_instant_hit_site, fire_sniper_site, los_site};
use super::weapons::{
    BOUND_MAX, BOUND_MIN, TWO, crosshairs_site, detonator_stub, no_cam_shake, render_weapon_skill, satchel_update_stub,
    take_photo_stub,
};
use super::*;

fn fn_addr(f: usize) -> u32 {
    f as u32
}

/// `jmp [slot]` + pointer at `slot` (samp's pattern for entry hooks).
fn jmp_indirect(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    what: &'static str,
    at: u32,
    slot: u32,
    original: &[u8; 6],
    target: u32,
) -> Result<(), PatchError> {
    patcher.write(mem, what, slot, &target.to_le_bytes(), Some(&SLOT_PADDING))?;
    let mut b = vec![0xFF, 0x25];
    b.extend_from_slice(&slot.to_le_bytes());
    patcher.write(mem, what, at, &b, Some(original))
}

/// (patch table row, result) per row, for the log.
type Report = Vec<(u32, Result<(), PatchError>)>;

/// The start-up patches of this module (applied once). Returns one result per patch-table row.
pub fn install_main(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Report {
    let mut out = Vec::new();
    install_entry_hooks(patcher, mem, &mut out);
    install_call_sites(patcher, mem, &mut out);
    install_byte_patches(patcher, mem, &mut out);
    out
}

/// The jmp-indirect entry hooks: damage response, AffectsPed, satchel update, detonator.
fn install_entry_hooks(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    let hooks = [
        (
            53,
            "ComputeDamageResponse hook",
            site::COMPUTE_DAMAGE_RESPONSE,
            site::COMPUTE_DAMAGE_RESPONSE_SLOT,
            &site::COMPUTE_DAMAGE_RESPONSE_ORIGINAL,
            fn_addr(damage_stub as *const () as usize),
        ),
        (
            80,
            "CEventDamage::AffectsPed hook",
            site::AFFECTS_PED,
            site::AFFECTS_PED_SLOT,
            &site::AFFECTS_PED_ORIGINAL,
            fn_addr(affects_ped_stub as *const () as usize),
        ),
        (
            56,
            "CProjectileInfo::Update satchel",
            site::SATCHEL_UPDATE,
            site::SATCHEL_UPDATE_SLOT,
            &site::SATCHEL_UPDATE_ORIGINAL,
            fn_addr(satchel_update_stub as *const () as usize),
        ),
        (
            57,
            "RemoveDetonatorProjectiles",
            site::DETONATOR,
            site::DETONATOR_SLOT,
            &site::DETONATOR_ORIGINAL,
            fn_addr(detonator_stub as *const () as usize),
        ),
    ];
    for (row, what, at, slot, original, target) in hooks {
        out.push((row, jmp_indirect(patcher, mem, what, at, slot, original, target)));
    }
}

/// Retargeted calls (weapon skill while rendering, crosshairs, sniper camera shake), then the TakePhotograph jmp.
fn install_call_sites(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    let calls = [
        (
            22,
            "RenderWeaponPedsForPC GetWeaponSkill",
            site::RENDER_WEAPON_SKILL,
            fn_addr(render_weapon_skill as *const () as usize),
            g::GET_WEAPON_SKILL,
        ),
        (
            8,
            "CHud::DrawCrossHairs",
            site::CROSSHAIRS,
            fn_addr(crosshairs_site as *const () as usize),
            g::DRAW_CROSSHAIRS,
        ),
        (
            46,
            "FireSniper CamShakeNoPos",
            site::SNIPER_CAM_SHAKE,
            fn_addr(no_cam_shake as *const () as usize),
            site::CAM_SHAKE_NO_POS,
        ),
    ];
    for (row, what, at, target, original) in calls {
        let r = patcher.retarget_branch(mem, what, at, target, Some(original)).map(|_| ());
        out.push((row, r));
    }
    let mut jmp =
        encode_branch(crate::patch::OP_JMP, site::TAKE_PHOTO, fn_addr(take_photo_stub as *const () as usize)).to_vec();
    jmp.extend_from_slice(&[0x90, 0x90]);
    out.push((
        45,
        patcher.write(mem, "TakePhotograph flags", site::TAKE_PHOTO, &jmp[..5], Some(&site::TAKE_PHOTO_ORIGINAL[..5])),
    ));
}

/// Plain byte patches: NOPed calls, then float pointers redirected to our constants.
fn install_byte_patches(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    let nops: [(u32, u32, usize, &[u8], &'static str); 4] = [
        (191, 0x56E5AD, 5, &[0xE8, 0xFE, 0x0C, 0x13, 0x00], "KillPlayer: no Wasted message"),
        (200, 0x60F289, 8, &[0x68, 0x00, 0x00, 0x80, 0x3F, 0x50, 0x6A, 0x29], "sniper fire-failed sound"),
        (
            201,
            0x60F29D,
            19,
            &[
                0x68, 0x00, 0x00, 0x80, 0x3F, 0x6A, 0x00, 0x6A, 0x2A, 0xB9, 0x90, 0xBC, 0xB6, 0x00, 0xE8, 0xF0, 0x7B,
                0xEF, 0xFF,
            ],
            "rocket fire-failed sound",
        ),
        (247, 0x4C3A5B, 10, &[0x55, 0x53, 0x57, 0x8B, 0xCE, 0xE8, 0xDB, 0xED, 0xFF, 0xFF], "ComputeGunAimedAtResponse"),
    ];
    for (row, addr, len, orig, what) in nops {
        out.push((row, patcher.nop(mem, what, addr, len, Some(orig))));
    }
    let ptrs: [(u32, u32, u32, u32, &'static str); 5] = [
        (210, 0x61E0C2, 0xB7CB5C, &TWO as *const f32 as u32, "ControlGunMove time step := 2.0"),
        (240, 0x7361B2, 0x859A90, &BOUND_MIN as *const f32 as u32, "bullet bound -20000"),
        (241, 0x7361CB, 0x859A94, &BOUND_MAX as *const f32 as u32, "bullet bound 20000"),
        (242, 0x7361E0, 0x859A90, &BOUND_MIN as *const f32 as u32, "bullet bound -20000"),
        (243, 0x7361F5, 0x859A94, &BOUND_MAX as *const f32 as u32, "bullet bound 20000"),
    ];
    for (row, addr, orig, new, what) in ptrs {
        out.push((row, patcher.write(mem, what, addr, &new.to_le_bytes(), Some(&orig.to_le_bytes()))));
    }
}

static INIT_GAME_DONE: AtomicBool = AtomicBool::new(false);
static FRIENDLY_FIRE_DONE: AtomicBool = AtomicBool::new(false);

/// InitGame installs the `CVehicle::InflictDamage` jmp only when the server enables vehicle friendly fire
/// (InitGame's last field), independent of the lag-compensation mode. Installed once (samp rewrites the same bytes
/// on every such InitGame). Mirrors samp.dll 0x100A7200.
pub fn install_vehicle_friendly_fire(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
) -> Vec<(u32, Result<(), PatchError>)> {
    if FRIENDLY_FIRE_DONE.swap(true, Ordering::SeqCst) {
        return Vec::new();
    }
    let jmp = encode_branch(crate::patch::OP_JMP, site::INFLICT_DAMAGE, fn_addr(inflict_stub as *const () as usize));
    vec![(
        106,
        patcher.write(
            mem,
            "CVehicle::InflictDamage hook",
            site::INFLICT_DAMAGE,
            &jmp,
            Some(&site::INFLICT_DAMAGE_ORIGINAL[..5]),
        ),
    )]
}

/// The InitGame patches: the selector, then for lag-comp 1/2 the bullet hooks and the lock-on RETs. Installed once
/// per process (samp never removes them either). Mirrors samp.dll 0x10010909.
pub fn install_init_game(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    selector: u8,
) -> Vec<(u32, Result<(), PatchError>)> {
    SELECTOR.store(u32::from(selector), Ordering::SeqCst);
    let mut out = Vec::new();
    if !combat::bullet_hooks_active(selector) || INIT_GAME_DONE.swap(true, Ordering::SeqCst) {
        return out;
    }
    for (row, at) in site::FIRE_INSTANT_HIT {
        let r = patcher
            .retarget_branch(
                mem,
                "CWeapon::Fire FireInstantHit",
                at,
                fn_addr(fire_instant_hit_site as *const () as usize),
                Some(g::FIRE_INSTANT_HIT),
            )
            .map(|_| ());
        out.push((row, r));
    }
    for (row, at) in site::LOS {
        let r = patcher
            .retarget_branch(
                mem,
                "bullet ProcessLineOfSight",
                at,
                fn_addr(los_site as *const () as usize),
                Some(g::PROCESS_LINE_OF_SIGHT),
            )
            .map(|_| ());
        out.push((row, r));
    }
    let r = patcher
        .retarget_branch(
            mem,
            "CWeapon::Fire FireSniper",
            site::FIRE_SNIPER,
            fn_addr(fire_sniper_site as *const () as usize),
            Some(g::FIRE_SNIPER),
        )
        .map(|_| ());
    out.push((104, r));
    let r = patcher
        .retarget_branch(
            mem,
            "FireSniper AddBullet",
            site::ADD_BULLET,
            fn_addr(add_bullet_site as *const () as usize),
            Some(g::ADD_BULLET),
        )
        .map(|_| ());
    out.push((105, r));
    out.push((4, patcher.write(mem, "FindWeaponLockOnTarget := ret", site::FIND_LOCK_ON, &[0xC3], Some(&[0x81]))));
    out.push((
        5,
        patcher.write(
            mem,
            "FindNextWeaponLockOnTarget := ret 8",
            site::FIND_NEXT_LOCK_ON,
            &[0xC2, 0x08, 0x00],
            Some(&[0x6A, 0xFF, 0xE9]),
        ),
    ));
    out
}
