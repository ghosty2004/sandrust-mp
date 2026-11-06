//! Combat (milestone M7): the platform-independent part.
//!
//! Specs: remote-control-combat.md (§3.3 bullet apply, §3.4 the line-of-sight hook and the lag-comp selector,
//! §6.1 the damage decision table, §7 death), its RE-10 additions, remote-players-sync.md §0.4 (send rates),
//! §203/§204/§206 and player-state.md 53/115/34/145. Measured on the original client: docs/phase3/trace-notes.md
//! "Combat (TRACE-6)" and "TRACE-7 additions".
//!
//! Static details confirmed in samp.dll A for this module: the 203 builder (A 0x10005210: zoom
//! `trunc((FOV − 35) / 35 × 63)`, aspect `trunc((ratio − 1) × 255)`, weapon state 3 while the weapon state is 2
//! (reloading), else `min(clip, 2)`), the 206 record (A 0x100A5450 / 0x10006BC0), the killer derivation
//! (A 0x100AC890, weapon read as a byte) and the damage decision (A 0x100A3BF0).
//!
//! Everything here is plain data and arithmetic; the GTA side lives in `combathooks.rs` and `client.rs`.

use std::time::{Duration, Instant};

use samp_proto::sync::player::{AimSync, BulletSync, WeaponSlot, WeaponsUpdate, hit_type};
use samp_proto::types::Vec3;

/// 115 reports GTA's damage × 0.33 for player victims (A 0x100068A0 / 0x100069B0, constant A 0x100E5900).
pub const DAMAGE_SCALE: f32 = 0.33;
/// No player / actor / target.
pub const NONE_ID: u16 = 0xFFFF;
/// Offsets of a remote 206 with a hit entity must lie in ±300 (A 0x10016370).
pub const MAX_OFFSET: f32 = 300.0;
/// The AddBullet hook multiplies the sniper bullet's velocity by this (A 0x100A5930, constant A 0x100EA63C).
pub const SNIPER_VELOCITY_SCALE: f32 = 50.0;
/// Origin z offset added to the hand bone besides the weapon's fire offset z (A 0x100AFA00, constant 0.15).
pub const ORIGIN_Z_EXTRA: f32 = 0.15;
/// Bones used for the shot origin (A 0x100AFA00: 24 normally, 34 for its other caller).
pub const BONE_RIGHT_HAND: u32 = 24;

/// Weapon ids with a fixed meaning here.
pub mod weapon {
    pub const FIST: u8 = 0;
    pub const SNIPER: u8 = 34;
    pub const PARACHUTE: u8 = 46;
    pub const RAMMED: u8 = 49;
    pub const HELI_BLADES: u8 = 50;
    pub const EXPLOSION: u8 = 51;
    pub const DROWNED: u8 = 53;
    pub const COLLISION: u8 = 54;
}

/// Lag-comp selector S (A 0x10118990): InitGame lag-comp 1 → 0, 2 → 1, anything else → 2 (off).
pub fn lag_comp_selector(init_game_lag_comp: u32) -> u8 {
    match init_game_lag_comp {
        1 => 0,
        2 => 1,
        _ => 2,
    }
}

/// With S = 2 samp installs none of its bullet hooks (InitGame A 0x10010909: the installer A 0x100A7050 and the
/// lock-on RETs A 0x100A1C10 run only for lag-comp 1 and 2). Measured (TRACE-6 mode 0): no LOS call from samp.
pub fn bullet_hooks_active(selector: u8) -> bool {
    selector != 2
}

// ---- 3-D helpers ------------------------------------------------------------------------------------------

pub type V3 = [f32; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn to_vec3(v: V3) -> Vec3 {
    Vec3 { x: v[0], y: v[1], z: v[2] }
}

/// A GTA `CMatrix` as samp uses it: right, forward, up (columns of the rotation) and the position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub right: V3,
    pub forward: V3,
    pub up: V3,
    pub pos: V3,
}

impl Matrix {
    /// `M × p` including the translation (samp's A 0x100B4D50).
    pub fn transform(&self, p: V3) -> V3 {
        let m = self;
        [
            m.right[0] * p[0] + m.forward[0] * p[1] + m.up[0] * p[2] + m.pos[0],
            m.right[1] * p[0] + m.forward[1] * p[1] + m.up[1] * p[2] + m.pos[1],
            m.right[2] * p[0] + m.forward[2] * p[1] + m.up[2] * p[2] + m.pos[2],
        ]
    }

    /// The inverse of a rigid matrix applied to `p` (samp: A 0x100B5780 inverse, then A 0x100B4D50).
    pub fn inverse_transform(&self, p: V3) -> V3 {
        let d = sub(p, self.pos);
        let dot = |a: V3| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        [dot(self.right), dot(self.forward), dot(self.up)]
    }
}

// ---- 206: local capture -----------------------------------------------------------------------------------

/// What the line-of-sight hook captured for a local shot (the 44-byte record of A 0x100A5450).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotRecord {
    /// Ray start (the LOS origin).
    pub origin: V3,
    /// The col point's position after the LOS call. On a miss GTA did not write it, so this is whatever its
    /// stack held (samp sends that too; see docs/phase3/m7-notes.md "garbage colpoint").
    pub hit_pos: V3,
    /// Hit point relative to the entity (S 0: entity frame; S 1: minus the entity position), else 0.
    pub offset: V3,
    /// The hit entity had a matrix (entity pointer ≠ 0).
    pub entity: bool,
}

/// Builds the record (A 0x100A5450): the offset is only computed when the hit entity has a matrix.
pub fn capture(selector: u8, origin: V3, hit_pos: V3, entity_matrix: Option<&Matrix>) -> ShotRecord {
    let offset = match entity_matrix {
        Some(m) if selector == 0 => m.inverse_transform(hit_pos),
        Some(m) => sub(hit_pos, m.pos),
        None => [0.0; 3],
    };
    ShotRecord { origin, hit_pos, offset, entity: entity_matrix.is_some() }
}

/// What the hit entity is to the network (A 0x10006BC0: remote player, pool vehicle, object, else nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    None,
    Player(u16),
    Vehicle(u16),
    Object(u16),
    /// An entity that is none of the above (building, actor, …): its matrix (or placement) turns the
    /// offset back into world coordinates.
    Other,
}

/// How an unknown entity's placement is read: a matrix, or the simple transform's position (`CPlaceable` +4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    Matrix(Matrix),
    Position(V3),
}

/// Builds the 206 for a captured record (A 0x10006BC0). `None` = not sent (sniper without a hit entity).
pub fn bullet_sync(rec: &ShotRecord, target: HitTarget, other: Option<Placement>, weapon: u8) -> Option<BulletSync> {
    if weapon == weapon::SNIPER && !rec.entity {
        return None;
    }
    let (ht, id) = match target {
        HitTarget::Player(id) => (hit_type::PLAYER, id),
        HitTarget::Vehicle(id) => (hit_type::VEHICLE, id),
        HitTarget::Object(id) => (hit_type::OBJECT, id),
        HitTarget::None | HitTarget::Other => (hit_type::NONE, NONE_ID),
    };
    let mut offset = rec.offset;
    if target == HitTarget::Other {
        offset = match other {
            Some(Placement::Matrix(m)) => m.transform(offset),
            Some(Placement::Position(p)) => add(p, offset),
            None => offset,
        };
    }
    Some(BulletSync {
        hit_type: ht,
        hit_id: id,
        origin: to_vec3(rec.origin),
        hit_pos: to_vec3(rec.hit_pos),
        offset: to_vec3(offset),
        weapon_id: weapon,
    })
}

// ---- 206: remote apply ------------------------------------------------------------------------------------

/// Receive-side check (A 0x10016370): with a hit type every offset component must be within ±300.
pub fn remote_bullet_valid(b: &BulletSync) -> bool {
    b.hit_type == hit_type::NONE || [b.offset.x, b.offset.y, b.offset.z].iter().all(|c| c.abs() <= MAX_OFFSET)
}

/// The redirect of the LOS hook (A 0x100A5640): P = entity matrix × offset (S 0) or entity position + offset
/// (S 1); the ray end becomes P + (P − start). `None` for S 2.
pub fn redirect_end(selector: u8, start: V3, entity: &Matrix, offset: V3) -> Option<V3> {
    let p = match selector {
        0 => entity.transform(offset),
        1 => add(entity.pos, offset),
        _ => return None,
    };
    Some(add(p, sub(p, start)))
}

// ---- 115 / damage decision --------------------------------------------------------------------------------

/// A remote player as the damage hook sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteRef {
    pub id: u16,
    pub npc: bool,
    pub team: u8,
}

/// Inputs of the damage decision (A 0x100A3BF0), resolved by the hook.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageCtx {
    pub victim_local: bool,
    pub damager_local: bool,
    /// The victim is a `CCivilianPed` (samp actors).
    pub victim_civilian: bool,
    /// The surfed entity (local surf state +0x2DA, a pool vehicle or object) is the damager.
    pub surface_is_damager: bool,
    /// The local surf state has a surface (+0x2DA set).
    pub surfing: bool,
    pub weapon: u32,
    /// Local victim: the attacker as a remote player (ped, or the first remote in the damaging vehicle).
    pub attacker: Option<RemoteRef>,
    /// Local attacker: the victim as a remote player.
    pub victim_remote: Option<RemoteRef>,
    /// Local attacker: the victim as an actor.
    pub victim_actor: Option<u16>,
    /// Local team (255 = none).
    pub local_team: u8,
}

/// What the hook reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Report {
    /// RPC 115 taking 1.
    Take { attacker: u16 },
    /// RPC 115 taking 0.
    Give { victim: u16 },
    /// RPC 177 (unscaled damage).
    Actor { actor: u16 },
}

/// The decision: an optional report and whether GTA's damage response is cancelled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub report: Option<Report>,
    pub cancel: bool,
}

/// The same-team rule (§6.3; Settings +0x22 is always 1, RE-10 §2).
fn same_team(local_team: u8, other: Option<RemoteRef>) -> bool {
    local_team != 255 && other.is_some_and(|r| r.team == local_team)
}

/// The surf inputs of the decision (A 0x100A3BF0): `surface` = the GTA entity of the surfed pool vehicle or
/// object while the local surf state has one (`Some(0)` when that pool entry has no GTA entity).
pub fn surf_inputs(surface: Option<u32>, damager: u32) -> (bool, bool) {
    match surface {
        Some(e) => (true, e == damager),
        None => (false, false),
    }
}

/// Object pool lookup by GTA entity (A 0x10012A10): it scans ids 1..=highest in use, so id 0 is never found.
pub fn object_of_entity(objects: &[(u32, u16)], entity: u32) -> Option<u16> {
    if entity == 0 {
        return None;
    }
    objects.iter().find(|o| o.1 != 0 && o.0 == entity).map(|o| o.1)
}

/// The decision table of remote-control-combat.md §6.1 in samp's order.
pub fn decide(c: &DamageCtx) -> Decision {
    let cancel = |report| Decision { report, cancel: true };
    let original = |report| Decision { report, cancel: false };
    if !c.damager_local && c.victim_civilian {
        return cancel(None);
    }
    if c.victim_local {
        if c.surfing && (c.surface_is_damager || c.weapon == u32::from(weapon::COLLISION)) {
            return cancel(None);
        }
        let report = Some(Report::Take { attacker: c.attacker.map_or(NONE_ID, |r| r.id) });
        return if same_team(c.local_team, c.attacker) { cancel(report) } else { original(report) };
    }
    if !c.damager_local {
        return original(None);
    }
    if let Some(v) = c.victim_remote {
        let report = Some(Report::Give { victim: v.id });
        return if v.npc || same_team(c.local_team, c.victim_remote) { cancel(report) } else { original(report) };
    }
    if let Some(actor) = c.victim_actor {
        return cancel(Some(Report::Actor { actor }));
    }
    original(None)
}

// ---- 53: killer and reason --------------------------------------------------------------------------------

/// The last damaging entity, resolved against the pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damager {
    None,
    /// A remote player's ped.
    Player(u16),
    /// A vehicle of the pool; its driver as a remote player, if any.
    Vehicle {
        driver: Option<u16>,
    },
    /// Anything else (the local ped, buildings, peds that are no player, vehicles outside the pool).
    Other,
}

/// Reason and killer of RPC 53 (A 0x100AC890; the weapon is read as a byte).
pub fn death_reason(last_weapon: u32, damager: Damager) -> (u8, u16) {
    const NOBODY: (u8, u16) = (255, NONE_ID);
    let w = last_weapon as u8;
    let driver = |d: Damager| match d {
        Damager::Vehicle { driver: Some(id) } => Some(id),
        _ => None,
    };
    match w {
        _ if w < 43 || w == weapon::HELI_BLADES || w == weapon::EXPLOSION => match damager {
            Damager::Player(id) => (w, id),
            d => driver(d).map_or(NOBODY, |id| (w, id)),
        },
        weapon::DROWNED => (w, NONE_ID),
        weapon::RAMMED => driver(damager).map_or(NOBODY, |id| (w, id)),
        weapon::COLLISION => match damager {
            Damager::None => NOBODY,
            Damager::Vehicle { driver: Some(id) } => (w, id),
            Damager::Vehicle { driver: None } => NOBODY,
            Damager::Player(_) | Damager::Other => (w, NONE_ID),
        },
        _ => NOBODY,
    }
}

// ---- 203 -------------------------------------------------------------------------------------------------

/// The local camera and weapon as the 203 builder reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimSample {
    /// `TheCamera.m_aCams[0].m_nMode`.
    pub cam_mode: u8,
    pub front: V3,
    pub source: V3,
    /// `m_pPlayerData->m_fLookPitch` (CPlayerPed +0x480 → +0x54).
    pub aim_z: f32,
    pub fov: f32,
    pub aspect_ratio: f32,
    /// Current `CWeapon::m_nState` and `m_nAmmoInClip`.
    pub weapon_state: u32,
    pub clip: u32,
}

/// `CWeapon::m_nState` value of a reloading weapon.
pub const WEAPONSTATE_RELOADING: u32 = 2;

/// x87 `_ftol` (truncation toward zero), stored as a byte.
fn ftol_u8(v: f32) -> u8 {
    v as i32 as u8
}

/// Builds the 203 (A 0x10005210).
pub fn build_aim(s: &AimSample) -> AimSync {
    let state = if s.weapon_state == WEAPONSTATE_RELOADING { 3 } else { s.clip.min(2) as u8 };
    AimSync {
        cam_mode: s.cam_mode,
        cam_front: to_vec3(s.front),
        cam_pos: to_vec3(s.source),
        aim_z: s.aim_z,
        zoom: ftol_u8((s.fov - 35.0) * (1.0 / 35.0) * 63.0) & 0x3F,
        weapon_state: state,
        aspect: ftol_u8((s.aspect_ratio - 1.0) * 255.0),
    }
}

/// 203 intervals (remote-players-sync §0.4; local tick A 0x10007CB3, passenger A 0x100057B0, spectating
/// A 0x10006665).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AimContext {
    /// On foot with the key word of this tick.
    OnFoot {
        keys: u16,
    },
    Passenger {
        drive_by: bool,
    },
    /// The driver's own 1000 ms tick (armed vehicles also send after every 200).
    Driver,
    Spectating {
        on_foot_interval_ms: u32,
    },
}

/// FIRE | AIM in the key word.
pub const KEYS_FIRE_AIM: u16 = 0x84;

pub fn aim_interval(ctx: AimContext, selector: u8, weapon_rate_ms: u32) -> Duration {
    let ms = match ctx {
        AimContext::OnFoot { keys } if keys & KEYS_FIRE_AIM != 0 => {
            if selector == 2 {
                weapon_rate_ms
            } else {
                100
            }
        }
        AimContext::OnFoot { .. } | AimContext::Driver | AimContext::Passenger { drive_by: false } => 1000,
        AimContext::Passenger { drive_by: true } => 100,
        AimContext::Spectating { on_foot_interval_ms } => 2 * on_foot_interval_ms,
    };
    Duration::from_millis(u64::from(ms))
}

/// The 203 timer: due when more than the interval has passed since the last attempt.
#[derive(Debug, Default, Clone, Copy)]
pub struct AimTimer {
    last: Option<Instant>,
}

impl AimTimer {
    pub fn due(&mut self, now: Instant, interval: Duration) -> bool {
        if self.last.is_some_and(|t| now.duration_since(t) <= interval) {
            return false;
        }
        self.last = Some(now);
        true
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

// ---- 204 -------------------------------------------------------------------------------------------------

/// One `CWeapon` as the 204 builder compares it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotState {
    pub weapon: u8,
    pub ammo: u32,
}

pub const WEAPON_SLOTS: usize = 13;

/// 204 delta encoder (A 0x10006290, remote-players-sync RE-10 "204 target fields").
#[derive(Debug, Default, Clone)]
pub struct WeaponsTracker {
    slots: [SlotState; WEAPON_SLOTS],
    armed: u8,
    targets: (u16, u16),
    last_send: Option<Instant>,
    started: bool,
}

impl WeaponsTracker {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Evaluated every tick. `None` = nothing to send.
    pub fn evaluate(
        &mut self,
        now: Instant,
        slots: &[SlotState; WEAPON_SLOTS],
        armed: u8,
        target_player: u16,
        target_actor: u16,
    ) -> Option<WeaponsUpdate> {
        if !self.started {
            // samp's copies start as zero (constructor), like ours; an unarmed spawn sends nothing.
            self.started = true;
            self.targets = (NONE_ID, NONE_ID);
        }
        let targets = (target_player, target_actor);
        if targets == self.targets
            && armed == self.armed
            && self.last_send.is_some_and(|t| now.duration_since(t) < Duration::from_millis(100))
        {
            return None;
        }
        self.armed = armed;
        let mut records = Vec::new();
        for (i, s) in slots.iter().enumerate() {
            if *s != self.slots[i] {
                self.slots[i] = *s;
                records.push(WeaponSlot { slot: i as u8, weapon_id: s.weapon, ammo: s.ammo.min(0xFFFF) as u16 });
            }
        }
        let targets_changed = targets != self.targets;
        self.targets = targets;
        if !targets_changed && records.is_empty() {
            return None;
        }
        self.last_send = Some(now);
        Some(WeaponsUpdate { target_player, target_actor, slots: records })
    }
}

// ---- local pad masks (carry / cuffs) ----------------------------------------------------------------------

/// The local ped object's three pad masks (RE-10 §1): +0x2F6 clears Square (jump), +0x2F7 ShockL (crouch),
/// +0x2F8 Cross (sprint), in both NewState and OldState of `Pads[0]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadMasks {
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
}

/// Special actions with masks (player-state.md 88).
pub mod special {
    pub const CUFFED: u8 = 24;
    pub const CARRY: u8 = 25;
}

/// Masks after a local special action (A 0x100ACE90 carry: all three; A 0x100ACF40 cuffs: sprint).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalHandcuffs {
    pub carry: bool,
    pub cuffed: bool,
}

impl LocalHandcuffs {
    pub fn masks(&self) -> PadMasks {
        PadMasks { jump: self.carry, crouch: self.carry, sprint: self.carry || self.cuffed }
    }
}

/// Word offsets in `CControllerState` (gta-reversed `ControllerState.h`), NewState at +0, OldState at +0x30.
pub mod pad {
    pub const SQUARE: usize = 0x1C;
    pub const CROSS: usize = 0x20;
    pub const SHOCK_L: usize = 0x24;
    pub const OLD_STATE: usize = 0x30;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(pos: V3) -> Matrix {
        Matrix { right: [1.0, 0.0, 0.0], forward: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], pos }
    }

    /// A ped facing −y (heading 180°): right = −x, forward = −y.
    fn facing_south(pos: V3) -> Matrix {
        Matrix { right: [-1.0, 0.0, 0.0], forward: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], pos }
    }

    #[test]
    fn selector() {
        assert_eq!([0, 1, 2, 3].map(lag_comp_selector), [2, 0, 1, 2]);
        assert!(bullet_hooks_active(0) && bullet_hooks_active(1) && !bullet_hooks_active(2));
    }

    #[test]
    fn capture_offsets_per_mode() {
        // TRACE-6: the same hit on a bot facing 180° gives (−0.096, 0.296, 0.469) in mode 1 (S 0) and
        // (0.097, −0.293, 0.469) in mode 2 (S 1).
        let m = facing_south([1700.0, -2590.0, 13.55]);
        let hit = [1700.097, -2590.293, 14.019];
        let r0 = capture(0, [0.0; 3], hit, Some(&m));
        let r1 = capture(1, [0.0; 3], hit, Some(&m));
        assert!((r0.offset[0] + 0.097).abs() < 1e-3 && (r0.offset[1] - 0.293).abs() < 1e-3);
        assert!((r1.offset[0] - 0.097).abs() < 1e-3 && (r1.offset[1] + 0.293).abs() < 1e-3);
        assert!((r0.offset[2] - 0.469).abs() < 1e-3);
        let miss = capture(0, [1.0; 3], [9.0; 3], None);
        assert_eq!((miss.offset, miss.entity), ([0.0; 3], false));
    }

    #[test]
    fn bullet_records() {
        let m = ident([10.0, 20.0, 3.0]);
        let rec = capture(0, [0.0; 3], [11.0, 20.0, 4.0], Some(&m));
        let p = bullet_sync(&rec, HitTarget::Player(7), None, 24).unwrap();
        assert_eq!((p.hit_type, p.hit_id, p.weapon_id), (1, 7, 24));
        // A building: offset back to world coordinates = hitPos.
        let b = bullet_sync(&rec, HitTarget::Other, Some(Placement::Matrix(m)), 24).unwrap();
        assert_eq!((b.hit_type, b.hit_id), (0, NONE_ID));
        assert_eq!(b.offset, to_vec3([11.0, 20.0, 4.0]));
        // A miss: offset 0, hitPos as captured; the sniper sends nothing.
        let miss = capture(0, [1.0; 3], [5.0; 3], None);
        let s = bullet_sync(&miss, HitTarget::None, None, 28).unwrap();
        assert_eq!((s.offset, s.hit_pos), (to_vec3([0.0; 3]), to_vec3([5.0; 3])));
        assert!(bullet_sync(&miss, HitTarget::None, None, weapon::SNIPER).is_none());
        assert!(bullet_sync(&rec, HitTarget::Other, Some(Placement::Matrix(m)), weapon::SNIPER).is_some());
    }

    #[test]
    fn redirect() {
        let m = ident([1700.0, -2600.0, 13.547]);
        // Mode 2 (S 1), offset (0, 0, 0.3): P = position + offset (TRACE-7: P = (1700, −2600, 13.847)).
        let e = redirect_end(1, [1700.0, -2590.0, 14.0], &m, [0.0, 0.0, 0.3]).unwrap();
        assert!((e[1] + 2610.0).abs() < 1e-3 && (e[2] - 13.694).abs() < 1e-3);
        assert!(redirect_end(2, [0.0; 3], &m, [0.0; 3]).is_none());
        let rot = facing_south([0.0; 3]);
        assert_eq!(redirect_end(0, [0.0; 3], &rot, [1.0, 0.0, 0.0]), Some([-2.0, 0.0, 0.0]));
    }

    #[test]
    fn remote_offset_bounds() {
        let mut b = BulletSync { hit_type: 1, offset: to_vec3([0.0, 300.0, -300.0]), ..Default::default() };
        assert!(remote_bullet_valid(&b));
        b.offset.y = 300.5;
        assert!(!remote_bullet_valid(&b));
        b.hit_type = 0;
        assert!(remote_bullet_valid(&b));
    }

    fn ctx() -> DamageCtx {
        DamageCtx {
            victim_local: false,
            damager_local: false,
            victim_civilian: false,
            surface_is_damager: false,
            surfing: false,
            weapon: 24,
            attacker: None,
            victim_remote: None,
            victim_actor: None,
            local_team: 255,
        }
    }

    #[test]
    fn damage_table() {
        let bot = RemoteRef { id: 3, npc: false, team: 1 };
        // Local victim: report, original; same team: report, cancel; no attacker: 0xFFFF.
        let c = DamageCtx { victim_local: true, attacker: Some(bot), ..ctx() };
        assert_eq!(decide(&c), Decision { report: Some(Report::Take { attacker: 3 }), cancel: false });
        assert!(decide(&DamageCtx { local_team: 1, ..c }).cancel);
        let fall = DamageCtx { victim_local: true, weapon: 54, ..ctx() };
        assert_eq!(decide(&fall).report, Some(Report::Take { attacker: NONE_ID }));
        // Surfing cases cancel without a report.
        let surf = DamageCtx { surfing: true, ..fall };
        assert_eq!(decide(&surf), Decision { report: None, cancel: true });
        // Surfing: the surfed entity as damager cancels for any weapon; another damager is reported as usual.
        let (surfing, by_surface) = surf_inputs(Some(0x1234), 0x1234);
        let rammed = DamageCtx { victim_local: true, weapon: 49, surfing, surface_is_damager: by_surface, ..ctx() };
        assert_eq!(decide(&rammed), Decision { report: None, cancel: true });
        let (surfing, by_surface) = surf_inputs(Some(0x1234), 0x9999);
        let shot = DamageCtx { victim_local: true, attacker: Some(bot), surfing, surface_is_damager: by_surface, ..ctx() };
        assert_eq!(decide(&shot), Decision { report: Some(Report::Take { attacker: 3 }), cancel: false });
        assert_eq!(surf_inputs(None, 0), (false, false));
        assert_eq!(surf_inputs(Some(0), 0), (true, true));
        // Local attacker.
        let g = DamageCtx { damager_local: true, victim_remote: Some(bot), ..ctx() };
        assert_eq!(decide(&g), Decision { report: Some(Report::Give { victim: 3 }), cancel: false });
        assert!(decide(&DamageCtx { local_team: 1, ..g }).cancel);
        assert!(decide(&DamageCtx { victim_remote: Some(RemoteRef { npc: true, ..bot }), ..g }).cancel);
        let a = DamageCtx { damager_local: true, victim_actor: Some(9), victim_civilian: true, ..ctx() };
        assert_eq!(decide(&a), Decision { report: Some(Report::Actor { actor: 9 }), cancel: true });
        // Others hitting an actor: cancelled silently; neither side local: original.
        assert_eq!(decide(&DamageCtx { victim_civilian: true, ..ctx() }), Decision { report: None, cancel: true });
        assert_eq!(decide(&ctx()), Decision { report: None, cancel: false });
    }

    #[test]
    fn object_lookup_skips_id_zero() {
        let objs = [(0x100, 0u16), (0x200, 1), (0x300, 999)];
        assert_eq!(object_of_entity(&objs, 0x100), None);
        assert_eq!(object_of_entity(&objs, 0x200), Some(1));
        assert_eq!(object_of_entity(&objs, 0x300), Some(999));
        assert_eq!(object_of_entity(&objs, 0), None);
        assert_eq!(object_of_entity(&objs, 0x400), None);
    }

    #[test]
    fn bullet_object_hit() {
        // An object hit keeps the record's local offset (like a vehicle) and carries the object id.
        let m = ident([10.0, 20.0, 30.0]);
        let rec = capture(0, [0.0; 3], [11.0, 22.0, 33.0], Some(&m));
        let b = bullet_sync(&rec, HitTarget::Object(7), None, 24).unwrap();
        assert_eq!((b.hit_type, b.hit_id), (hit_type::OBJECT, 7));
        assert_eq!([b.offset.x, b.offset.y, b.offset.z], [1.0, 2.0, 3.0]);
        let rec = capture(1, [0.0; 3], [11.0, 22.0, 33.0], Some(&m));
        let b = bullet_sync(&rec, HitTarget::Object(7), None, 24).unwrap();
        assert_eq!([b.offset.x, b.offset.y, b.offset.z], [1.0, 2.0, 3.0]);
        // The same entity as an unknown one: hit type 0 with the offset back in world coordinates.
        let b = bullet_sync(&rec, HitTarget::Other, Some(Placement::Matrix(m)), 24).unwrap();
        assert_eq!((b.hit_type, b.hit_id), (hit_type::NONE, NONE_ID));
        assert_eq!([b.offset.x, b.offset.y, b.offset.z], [11.0, 22.0, 33.0]);
    }

    #[test]
    fn killer_reason() {
        assert_eq!(death_reason(24, Damager::Player(5)), (24, 5));
        assert_eq!(death_reason(24, Damager::Vehicle { driver: Some(6) }), (24, 6));
        assert_eq!(death_reason(24, Damager::Other), (255, NONE_ID));
        assert_eq!(death_reason(51, Damager::Player(5)), (51, 5));
        assert_eq!(death_reason(53, Damager::Player(5)), (53, NONE_ID));
        assert_eq!(death_reason(49, Damager::Vehicle { driver: Some(2) }), (49, 2));
        assert_eq!(death_reason(49, Damager::Player(2)), (255, NONE_ID));
        assert_eq!(death_reason(54, Damager::None), (255, NONE_ID));
        assert_eq!(death_reason(54, Damager::Other), (54, NONE_ID));
        assert_eq!(death_reason(54, Damager::Vehicle { driver: Some(4) }), (54, 4));
        assert_eq!(death_reason(54, Damager::Vehicle { driver: None }), (255, NONE_ID));
        // The spawn routine stores 0xFF: health 0 by the server gives 255/0xFFFF (TRACE-6).
        assert_eq!(death_reason(0xFF, Damager::None), (255, NONE_ID));
    }

    #[test]
    fn aim_packet() {
        // TRACE-7 drive-by: cam mode 55, zoom 63 (FOV 70), weapon state 2.
        let s = AimSample {
            cam_mode: 55,
            fov: 70.0,
            aspect_ratio: 1.7777,
            weapon_state: 0,
            clip: 30,
            ..Default::default()
        };
        let a = build_aim(&s);
        assert_eq!((a.cam_mode, a.zoom, a.weapon_state, a.aspect), (55, 63, 2, 198));
        assert_eq!(build_aim(&AimSample { weapon_state: 2, ..s }).weapon_state, 3);
        assert_eq!(build_aim(&AimSample { clip: 1, ..s }).weapon_state, 1);
    }

    #[test]
    fn aim_rates() {
        let ms = |c, s| aim_interval(c, s, 40).as_millis();
        assert_eq!(ms(AimContext::OnFoot { keys: 0x80 }, 0), 100);
        assert_eq!(ms(AimContext::OnFoot { keys: 0x04 }, 2), 40);
        assert_eq!(ms(AimContext::OnFoot { keys: 0x20 }, 0), 1000);
        assert_eq!(ms(AimContext::Passenger { drive_by: true }, 0), 100);
        assert_eq!(ms(AimContext::Spectating { on_foot_interval_ms: 31 }, 0), 62);
        let t0 = Instant::now();
        let mut t = AimTimer::default();
        assert!(t.due(t0, Duration::from_millis(100)));
        assert!(!t.due(t0 + Duration::from_millis(100), Duration::from_millis(100)));
        assert!(t.due(t0 + Duration::from_millis(101), Duration::from_millis(100)));
    }

    #[test]
    fn weapons_delta() {
        let t0 = Instant::now();
        let mut w = WeaponsTracker::default();
        let mut slots = [SlotState::default(); WEAPON_SLOTS];
        assert_eq!(w.evaluate(t0, &slots, 0, NONE_ID, NONE_ID), None);
        slots[2] = SlotState { weapon: 24, ammo: 50 };
        let p = w.evaluate(t0, &slots, 24, NONE_ID, NONE_ID).unwrap();
        assert_eq!(p.slots, vec![WeaponSlot { slot: 2, weapon_id: 24, ammo: 50 }]);
        // Within 100 ms with the same armed weapon and targets: not even compared.
        slots[2].ammo = 49;
        assert_eq!(w.evaluate(t0 + Duration::from_millis(50), &slots, 24, NONE_ID, NONE_ID), None);
        let p = w.evaluate(t0 + Duration::from_millis(100), &slots, 24, NONE_ID, NONE_ID).unwrap();
        assert_eq!(p.slots[0].ammo, 49);
        // Aiming at player 3: a target-only packet; release: targets back to 0xFFFF.
        let p = w.evaluate(t0 + Duration::from_millis(110), &slots, 24, 3, NONE_ID).unwrap();
        assert_eq!((p.target_player, p.slots.len()), (3, 0));
        let p = w.evaluate(t0 + Duration::from_millis(300), &slots, 24, NONE_ID, NONE_ID).unwrap();
        assert_eq!((p.target_player, p.slots.len()), (NONE_ID, 0));
        // An armed-weapon change alone bypasses the throttle but sends nothing.
        assert_eq!(w.evaluate(t0 + Duration::from_millis(301), &slots, 0, NONE_ID, NONE_ID), None);
    }

    #[test]
    fn masks() {
        let c = LocalHandcuffs { carry: true, cuffed: false };
        assert_eq!(c.masks(), PadMasks { jump: true, crouch: true, sprint: true });
        let c = LocalHandcuffs { carry: false, cuffed: true };
        assert_eq!(c.masks(), PadMasks { jump: false, crouch: false, sprint: true });
    }
}
