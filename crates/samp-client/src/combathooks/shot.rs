//! The shot: firing call sites, the bullet line-of-sight hook and the local capture of bullet records.

use super::*;

/// Shooter and record published for the LOS hook while a shot is fired.
static SHOOTER: AtomicU32 = AtomicU32::new(0);
static RECORD_VALID: AtomicBool = AtomicBool::new(false);
static RECORD_ENTITY: AtomicU32 = AtomicU32::new(0);
static RECORD_OFFSET: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
/// Lag-comp selector cached for the hooks (InitGame).
pub(super) static SELECTOR: AtomicU32 = AtomicU32::new(2);

pub fn selector() -> u8 {
    SELECTOR.load(Ordering::Relaxed) as u8
}

/// The camera context of a remote shot (no pad, no PlayerInFocus).
struct ShotContext {
    cam_mode: u8,
    weapon_mode: u16,
    fov: f32,
    aspect: f32,
    vectors: [f32; AIM_FLOATS],
    skills: [f32; SKILLS],
}

fn enter_shot_context(slot: u8) -> Option<ShotContext> {
    let shadow = crate::swap::with_table(|t| t.shadows.get(usize::from(slot)).copied())?;
    let saved = ShotContext {
        cam_mode: rd(sw::CAM0_MODE),
        weapon_mode: rd(sw::PLAYER_WEAPON_MODE),
        fov: rd(sw::CAM0_FOV),
        aspect: rd(sw::ASPECT_RATIO),
        vectors: read_floats(sw::CAM0_VECTORS),
        skills: read_floats(sw::WEAPON_SKILLS),
    };
    wr(sw::CAM0_MODE, shadow.cam_mode);
    wr(sw::PLAYER_WEAPON_MODE, shadow.weapon_mode());
    wr(sw::CAM0_FOV, shadow.fov());
    wr(sw::ASPECT_RATIO, shadow.aspect + 1.0);
    write_floats(sw::CAM0_VECTORS, &shadow.aim);
    write_floats(sw::WEAPON_SKILLS, &shadow.skills);
    Some(saved)
}

fn leave_shot_context(s: &ShotContext) {
    wr(sw::CAM0_MODE, s.cam_mode);
    wr(sw::PLAYER_WEAPON_MODE, s.weapon_mode);
    wr(sw::CAM0_FOV, s.fov);
    wr(sw::ASPECT_RATIO, s.aspect);
    write_floats(sw::CAM0_VECTORS, &s.vectors);
    write_floats(sw::WEAPON_SKILLS, &s.skills);
}

/// Origin and muzzle of a shot: the right-hand bone, z + fire offset z + 0.15; the muzzle is the weapon's fire
/// offset transformed by the same bone. Mirrors samp.dll 0x100AFA00.
fn origin_and_muzzle(ped: u32, weapon: u8) -> (V3, V3) {
    // SAFETY: GTA's weapon-info lookup and bone queries (cdecl / thiscall) on a live ped, main thread.
    unsafe {
        let info: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(g::GET_WEAPON_INFO as usize);
        let wi = info(u32::from(weapon), 1);
        let fire_offset = if wi != 0 { v3(wi + g::WEAPON_INFO_FIRE_OFFSET) } else { [0.0; 3] };
        let mut origin = [0f32; 3];
        let bone: extern "thiscall" fn(u32, *mut f32, u32, u32) = std::mem::transmute(g::GET_BONE_POSITION as usize);
        bone(ped, origin.as_mut_ptr(), combat::BONE_RIGHT_HAND, 0);
        origin[2] += fire_offset[2] + combat::ORIGIN_Z_EXTRA;
        let mut muzzle = fire_offset;
        let tbone: extern "thiscall" fn(u32, *mut f32, u32, u32) =
            std::mem::transmute(g::GET_TRANSFORMED_BONE_POSITION as usize);
        tbone(ped, muzzle.as_mut_ptr(), combat::BONE_RIGHT_HAND, 0);
        (origin, muzzle)
    }
}

/// Fires one shot for `ped`; a local shot ignores GTA's origin arguments and comes through here too. `slot` 0 =
/// the local ped (no swap); a remote shot runs with the shooter's camera swapped in and the saved values restored.
/// `record` = the remote 206's target entity and offset (the line-of-sight hook's redirect / suppression input).
/// Mirrors samp.dll 0x100AFAB0.
pub fn fire_shot(ped: u32, slot: u8, record: Option<(u32, V3)>) {
    if ped == 0 {
        return;
    }
    let ctx = if slot != 0 { enter_shot_context(slot) } else { None };
    SHOOTER.store(ped, Ordering::SeqCst);
    match record {
        Some((e, off)) => {
            RECORD_ENTITY.store(e, Ordering::SeqCst);
            for (a, v) in RECORD_OFFSET.iter().zip(off) {
                a.store(v.to_bits(), Ordering::SeqCst);
            }
            RECORD_VALID.store(true, Ordering::SeqCst);
        }
        None => RECORD_VALID.store(false, Ordering::SeqCst),
    }
    let weapon = current_weapon(ped);
    let wptr = current_weapon_ptr(ped);
    // SAFETY: GTA's own fire functions (thiscall on the ped's CWeapon), main thread, outside any ped context.
    unsafe {
        if weapon == combat::weapon::SNIPER {
            let f: extern "thiscall" fn(u32, u32, u32, u32) -> u32 = std::mem::transmute(g::FIRE_SNIPER as usize);
            f(wptr, ped, 0, 0);
        } else {
            let (mut origin, mut muzzle) = origin_and_muzzle(ped, weapon);
            let f: extern "thiscall" fn(u32, u32, *mut f32, *mut f32, u32, u32, u32, u32, u32) -> u32 =
                std::mem::transmute(g::FIRE_INSTANT_HIT as usize);
            f(wptr, ped, origin.as_mut_ptr(), muzzle.as_mut_ptr(), 0, 0, 0, 0, 1);
        }
    }
    SHOOTER.store(0, Ordering::SeqCst);
    RECORD_VALID.store(false, Ordering::SeqCst);
    if let Some(c) = ctx {
        leave_shot_context(&c);
    }
}

/// Before a local instant-hit shot every streamed on-foot remote ped gets collision; the saved state is put back
/// afterwards. Mirrors samp.dll 0x10013C90.
fn force_remote_collisions() -> Vec<(u32, bool)> {
    let peds = with_shared(|s| s.remotes.iter().map(|r| r.ped).collect::<Vec<_>>()).unwrap_or_default();
    peds.into_iter()
        .filter(|&p| p != 0 && rd::<u32>(p + g::PED_FLAGS) & (1 << 8) == 0)
        .map(|p| {
            let flags: u32 = rd(p + g::ENTITY_FLAGS);
            wr(p + g::ENTITY_FLAGS, crate::remote::collision_flags(flags, true));
            (p, flags & 1 != 0)
        })
        .collect()
}

fn restore_remote_collisions(saved: Vec<(u32, bool)>) {
    for (p, on) in saved {
        let flags: u32 = rd(p + g::ENTITY_FLAGS);
        wr(p + g::ENTITY_FLAGS, crate::remote::collision_flags(flags, on));
    }
}

/// H3: replaces `call CWeapon::FireInstantHit` in `CWeapon::Fire`; only the local ped fires. Returns the 8th
/// argument ("fired"). Mirrors samp.dll 0x100A5820.
pub(super) extern "thiscall" fn fire_instant_hit_site(
    _weapon: u32,
    firing: u32,
    _a2: u32,
    _a3: u32,
    _a4: u32,
    _a5: u32,
    _a6: u32,
    _a7: u32,
    a8: u32,
) -> u32 {
    let local = local_ped();
    if firing != local || local == 0 {
        N_DROPPED_REMOTE.fetch_add(1, Ordering::Relaxed);
        return a8;
    }
    N_LOCAL_SHOTS.fetch_add(1, Ordering::Relaxed);
    let saved = force_remote_collisions();
    fire_shot(local, 0, None);
    restore_remote_collisions(saved);
    a8
}

/// H3: replaces `call CWeapon::FireSniper` in `CWeapon::Fire`; only the local ped fires. Returns the shooter.
/// Mirrors samp.dll 0x100A58E0.
pub(super) extern "thiscall" fn fire_sniper_site(_weapon: u32, shooter: u32, _victim: u32, _target: u32) -> u32 {
    let local = local_ped();
    if shooter == local && local != 0 {
        N_LOCAL_SHOTS.fetch_add(1, Ordering::Relaxed);
        fire_shot(local, 0, None);
    } else {
        N_DROPPED_REMOTE.fetch_add(1, Ordering::Relaxed);
    }
    shooter
}

/// H5: replaces `call CBulletInfo::AddBullet` in `FireSniper`: velocity × 50, then `CBulletInfo::Update` at once,
/// while the shooter globals are still set. Mirrors samp.dll 0x100A5930.
pub(super) extern "C" fn add_bullet_site(
    creator: u32,
    weapon: u32,
    px: f32,
    py: f32,
    pz: f32,
    vx: f32,
    vy: f32,
    vz: f32,
) {
    let k = combat::SNIPER_VELOCITY_SCALE;
    // SAFETY: GTA's static bullet functions (cdecl), main thread.
    unsafe {
        let add: extern "C" fn(u32, u32, f32, f32, f32, f32, f32, f32) = std::mem::transmute(g::ADD_BULLET as usize);
        add(creator, weapon, px, py, pz, vx * k, vy * k, vz * k);
        let update: extern "C" fn() = std::mem::transmute(g::BULLET_UPDATE as usize);
        update();
    }
}

type Los = extern "C" fn(*const f32, *mut f32, *mut f32, *mut u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32;

/// H4: replaces `call CWorld::ProcessLineOfSight` at the three sites; returns 0 when it suppresses a hit on the
/// local ped. Mirrors samp.dll 0x100A5640.
#[allow(clippy::too_many_arguments)]
pub(super) extern "C" fn los_site(
    origin: *const f32,
    target: *mut f32,
    colpoint: *mut f32,
    entity: *mut u32,
    b1: u32,
    b2: u32,
    b3: u32,
    b4: u32,
    b5: u32,
    b6: u32,
    b7: u32,
    b8: u32,
) -> u32 {
    let s = selector();
    let shooter = SHOOTER.load(Ordering::SeqCst);
    let local = local_ped();
    let remote_shot = s != 2 && shooter != 0 && shooter != local;
    // SAFETY: GTA passes valid pointers to its own vectors / col point / entity slot.
    let start: V3 = unsafe { [*origin, *origin.add(1), *origin.add(2)] };
    let record_entity =
        if RECORD_VALID.load(Ordering::SeqCst) { Some(RECORD_ENTITY.load(Ordering::SeqCst)) } else { None };
    if remote_shot
        && let Some(e) = record_entity
        && let Some(m) = matrix_of(e)
    {
        let off: V3 = std::array::from_fn(|i| f32::from_bits(RECORD_OFFSET[i].load(Ordering::SeqCst)));
        if let Some(end) = combat::redirect_end(s, start, &m, off) {
            N_REDIRECTED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: as above.
            unsafe { std::ptr::copy_nonoverlapping(end.as_ptr(), target, 3) };
        }
    }
    // SAFETY: the original cdecl function with the caller's arguments.
    let r = unsafe {
        let f: Los = std::mem::transmute(g::PROCESS_LINE_OF_SIGHT as usize);
        f(origin, target, colpoint, entity, b1, b2, b3, b4, b5, b6, b7, b8)
    };
    if remote_shot && N_REMOTE_SHOTS.load(Ordering::Relaxed) <= LOG_LIMIT {
        // SAFETY: as above.
        let (end, hit): (V3, u32) = unsafe { ([*target, *target.add(1), *target.add(2)], *entity) };
        crate::log!(
            "combat: remote LOS start {start:?} end {end:?} record {record_entity:?} -> {r} entity {hit:#x} (local {local:#x})"
        );
    }
    if remote_shot && record_entity == Some(0) {
        // SAFETY: as above.
        let hit: u32 = unsafe { *entity };
        let in_car = local != 0 && rd::<u32>(local + g::PED_FLAGS) & (1 << 8) != 0;
        if hit != 0 && (hit == local || (in_car && hit == rd::<u32>(local + g::PED_VEHICLE))) {
            N_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: as above.
            unsafe {
                *entity = 0;
                std::ptr::write_bytes(colpoint, 0, 3);
            }
            return 0;
        }
    }
    if shooter != 0 && shooter == local {
        // SAFETY: as above.
        let (hit_pos, hit): (V3, u32) = unsafe { ([*colpoint, *colpoint.add(1), *colpoint.add(2)], *entity) };
        capture_local(s, start, hit_pos, hit);
    }
    r
}

/// Local capture: build the record and queue the 206.
fn capture_local(s: u8, origin: V3, hit_pos: V3, hit: u32) {
    let m = if hit != 0 { matrix_of(hit) } else { None };
    let rec = combat::capture(s, origin, hit_pos, m.as_ref());
    let entity = if m.is_some() { hit } else { 0 };
    let target = if entity == 0 {
        HitTarget::None
    } else {
        with_shared(|sh| {
            if let Some(r) = sh.remote_by_ped(entity) {
                HitTarget::Player(r.id)
            } else if let Some(id) = sh.vehicle_id(entity) {
                HitTarget::Vehicle(id)
            } else if let Some(id) = combat::object_of_entity(&sh.objects, entity) {
                HitTarget::Object(id)
            } else {
                HitTarget::Other
            }
        })
        .unwrap_or(HitTarget::Other)
    };
    let other = (target == HitTarget::Other).then(|| match m {
        Some(m) => Placement::Matrix(m),
        None => Placement::Position(v3(entity + 4)),
    });
    let weapon = current_weapon(local_ped());
    if let Some(b) = combat::bullet_sync(&rec, target, other, weapon) {
        queue(Outgoing::Bullet(b));
    }
}
