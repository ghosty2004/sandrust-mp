//! Remote players on foot: the platform-independent part (milestone M3).
//!
//! Specs: remote-control-onfoot.md (§1 ped slots, §2 the per-player context
//! switch, §3 movement application), remote-players-sync.md (§1 lifecycle RPCs
//! 137/138/32/163, §207, §2.1 store/apply rules) and remote-control-combat.md
//! §1.2 (per-slot tables and their defaults).
//!
//! Everything here is pure data and arithmetic so it can be unit-tested on the
//! host; the GTA side (ped creation, hooks, memory writes) lives in
//! `players.rs` / `swap.rs` (Windows only).

use std::time::{Duration, Instant};

use samp_proto::sync::player::RemoteFootSync;
use samp_proto::types::Quat;

use crate::footsync::Rotation;

/// Ped slots = GTA player indices. 0 is the local player, 1 (GTA's second
/// player) is never used, remote players get 2…209 (onfoot §1.1).
pub const PED_SLOTS: usize = 210;
pub const FIRST_REMOTE_SLOT: usize = 2;
/// `sizeof(CPad)` (gta-reversed `Pad.h`).
pub const PAD_SIZE: usize = 0x134;
/// `CControllerState` OldState offset inside `CPad`.
pub const PAD_OLD_STATE: usize = 0x30;
/// Weapon-skill stats swapped per slot (`CStats::StatTypesFloat[69..79]`).
pub const SKILLS: usize = 11;
/// Camera vectors Front/Source/SourceBeforeLookBehind/Up (12 floats).
pub const AIM_FLOATS: usize = 12;

/// Position nudge gain per received 207 (A 0x100FE4B0).
pub const NUDGE_GAIN: f32 = 0.1;
/// Teleport thresholds (per axis) and dead zone (onfoot §3.4).
pub const TELEPORT_XY: f32 = 2.0;
pub const TELEPORT_Z: f32 = 1.0;
pub const DEAD_ZONE: f32 = 1e-5;
/// `slerp(synced, current, 0.75)`: 25 % toward the synced rotation per frame.
pub const SLERP_KEEP: f32 = 0.75;
/// No sync for longer than this → stop and snap to the last synced position.
pub const STALE: Duration = Duration::from_millis(3000);
/// Health forced on every remote ped each frame (state ≠ wasted).
pub const REMOTE_PED_HEALTH: f32 = 1000.0;
/// Head look (onfoot §5.2): refresh and call intervals, look distance.
pub const LOOK_REFRESH: Duration = Duration::from_millis(1000);
pub const LOOK_INTERVAL: Duration = Duration::from_millis(200);
pub const LOOK_DISTANCE: f32 = 20.0;

/// Due timers of the head look: (refresh the offset, call LookAt).
pub fn head_look_due(p: &RemotePlayer, now: Instant) -> (bool, bool) {
    let due = |t: Option<Instant>, d: Duration| t.is_none_or(|t| now.duration_since(t) >= d);
    (due(p.look_refresh, LOOK_REFRESH), due(p.look_last, LOOK_INTERVAL))
}

/// Collision helpers per frame on foot (onfoot §3.6): `Some(true)` on,
/// `Some(false)` off, `None` unchanged.
pub fn collision_wanted(f: &FootData) -> Option<bool> {
    let surfing = f.surf_id != 0 && f.surf_id != 0xFFFF && f.surf_id < 3000;
    if !surfing {
        Some(f.special_action != 3)
    } else if f.surf_id < 2000 {
        Some(false)
    } else {
        None
    }
}

/// `CEntity::m_nFlags` after samp's two collision helpers (A 0x1009F6E0 and
/// 0x1009F690): on clears bit 31 (samp's "skip collision" marker) and bit 1
/// (`m_bCollisionProcessed`) and sets bit 0 (`m_bUsesCollision`); off does the reverse.
pub fn collision_flags(flags: u32, on: bool) -> u32 {
    if on { (flags & 0x7FFF_FFFD) | 1 } else { (flags | 0x8000_0002) & !1 }
}

/// z offset of the matrix translation right after creation (onfoot §1.2).
pub const CREATE_Z_OFFSET: f32 = 0.15;

/// Remote state byte (remote-players-sync §2.1).
pub mod state {
    pub const NONE: u8 = 0;
    pub const ON_FOOT: u8 = 17;
    pub const WASTED: u8 = 32;
    pub const SPAWNED: u8 = 33;
}

// ---- ped slots -------------------------------------------------------------

/// Allocation of ped slots 2…209 (first free, like A 0x100A0170).
#[derive(Clone)]
pub struct SlotAllocator {
    used: [bool; PED_SLOTS],
}

impl Default for SlotAllocator {
    fn default() -> Self {
        Self { used: [false; PED_SLOTS] }
    }
}

impl SlotAllocator {
    pub fn alloc(&mut self) -> Option<u8> {
        let i = (FIRST_REMOTE_SLOT..PED_SLOTS).find(|&i| !self.used[i])?;
        self.used[i] = true;
        Some(i as u8)
    }

    pub fn free(&mut self, slot: u8) {
        if let Some(u) = self.used.get_mut(usize::from(slot)) {
            *u = false;
        }
    }

    /// Slots 2…209 in use: the N of the on-foot send interval (sync §0.4).
    pub fn count(&self) -> u32 {
        self.used[FIRST_REMOTE_SLOT..].iter().filter(|&&u| u).count() as u32
    }
}

// ---- per-slot shadow state (shared with the ProcessControl hook) ----------

/// What the context switch writes into GTA for one slot
/// (remote-control-combat.md §1.2, defaults from the same table).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SlotShadow {
    pub pad: [u8; PAD_SIZE],
    pub cam_mode: u8,
    pub aim: [f32; AIM_FLOATS],
    /// 0…1; FOV := zoom × 35 + 35.
    pub zoom: f32,
    /// aspect − 1.0.
    pub aspect: f32,
    pub skills: [f32; SKILLS],
}

impl Default for SlotShadow {
    fn default() -> Self {
        Self {
            pad: [0; PAD_SIZE],
            cam_mode: 4,
            aim: [0.0; AIM_FLOATS],
            zoom: 1.0,
            aspect: 0.3333,
            skills: [999.0; SKILLS],
        }
    }
}

impl SlotShadow {
    /// `TheCamera.m_PlayerWeaponMode.m_nMode`: the camera mode, 4 written as 0.
    pub fn weapon_mode(&self) -> u16 {
        if self.cam_mode == 4 { 0 } else { u16::from(self.cam_mode) }
    }

    pub fn fov(&self) -> f32 {
        self.zoom * 35.0 + 35.0
    }

    /// `Cams[0].m_bLookingLeft/Right` from NewState.LeftShoulder2 / RightShoulder2.
    pub fn looking_left_right(&self) -> (u8, u8) {
        (u8::from(pad_word(&self.pad, 5) != 0), u8::from(pad_word(&self.pad, 7) != 0))
    }

    /// Aim sync 203 → camera tables (remote-players-sync §203 store, combat §1.2).
    pub fn apply_aim(&mut self, cam_mode: u8, front: [f32; 3], pos: [f32; 3], zoom_byte: u8, aspect_byte: u8) {
        self.cam_mode = cam_mode;
        let up = up_from_front(front);
        let v = [front, pos, pos, up];
        for (i, a) in v.iter().enumerate() {
            self.aim[3 * i..3 * i + 3].copy_from_slice(a);
        }
        self.zoom = f32::from(zoom_byte & 0x3F) / 63.0;
        self.aspect = f32::from(aspect_byte) / 255.0;
    }
}

/// An up vector perpendicular to `front` (camera roll 0).
pub fn up_from_front(f: [f32; 3]) -> [f32; 3] {
    let cross =
        |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let right = cross(f, [0.0, 0.0, 1.0]);
    let up = cross(right, f);
    let n = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt();
    if n > 1e-6 { [up[0] / n, up[1] / n, up[2] / n] } else { [0.0, 0.0, 1.0] }
}

/// Slot → ped table plus the shadow state of every slot.
pub struct SwapTable {
    pub peds: [u32; PED_SLOTS],
    pub shadows: Vec<SlotShadow>,
}

impl Default for SwapTable {
    fn default() -> Self {
        Self { peds: [0; PED_SLOTS], shadows: vec![SlotShadow::default(); PED_SLOTS] }
    }
}

impl SwapTable {
    /// Reverse lookup ped → remote slot (A 0x100B43A0); `None` for the local
    /// player and unknown peds.
    pub fn slot_of(&self, ped: u32) -> Option<usize> {
        if ped == 0 {
            return None;
        }
        (FIRST_REMOTE_SLOT..PED_SLOTS).find(|&i| self.peds[i] == ped)
    }

    /// Binds `ped` to `slot` with fresh (default) shadow state.
    pub fn bind(&mut self, slot: u8, ped: u32, skills: [f32; SKILLS]) {
        let i = usize::from(slot);
        if i < PED_SLOTS {
            self.peds[i] = ped;
            self.shadows[i] = SlotShadow { skills, ..SlotShadow::default() };
        }
    }

    pub fn unbind(&mut self, slot: u8) {
        let i = usize::from(slot);
        if i < PED_SLOTS {
            self.peds[i] = 0;
            self.shadows[i] = SlotShadow::default();
        }
    }
}

// ---- keys → shadow pad (onfoot §2.1) ---------------------------------------

pub fn pad_word(pad: &[u8; PAD_SIZE], word: usize) -> i16 {
    i16::from_le_bytes([pad[2 * word], pad[2 * word + 1]])
}

fn set_pad_word(pad: &mut [u8; PAD_SIZE], base: usize, word: usize, v: i16) {
    pad[base + 2 * word..base + 2 * word + 2].copy_from_slice(&v.to_le_bytes());
}

/// Synced input for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadInput {
    pub keys: u16,
    pub left_right: i16,
    pub up_down: i16,
    pub in_vehicle: bool,
    /// Primary task is `CTaskSimpleJetPack`: SECONDARY_ATTACK is not injected on foot.
    pub jetpack: bool,
}

/// Writes NewState/OldState of the shadow pad. A newly active input gets
/// OldState 0 for one frame (a "just pressed" edge); a held input has
/// OldState == NewState; an inactive one is 0 in both.
pub fn apply_keys(pad: &mut [u8; PAD_SIZE], i: PadInput) {
    let k = |bit: u16| i.keys & (1 << bit) != 0;
    let button = |on: bool| if on { 0xFF } else { 0 };
    let axis = |neg: bool, pos: bool| {
        if neg {
            -128
        } else if pos {
            128
        } else {
            0
        }
    };
    let words: [(usize, i16); 16] = [
        (0, i.left_right),
        (1, i.up_down),
        (2, axis(k(13), k(14))),
        (3, axis(k(11), k(12))),
        (4, button(k(0))),
        (5, button(k(8))),
        (6, button(k(7))),
        (7, button(k(6))),
        (0x0E, button(k(5))),
        (0x0F, button(k(4) && (i.in_vehicle || !i.jetpack))),
        (0x10, button(k(3))),
        (0x11, button(k(2))),
        (0x12, button(k(1) && i.in_vehicle)),
        (0x13, button(k(9))),
        (0x15, button(k(10))),
        // DPad and the other words stay zero.
        (0x08, 0),
    ];
    for (w, v) in words {
        let was = pad_word(pad, w) != 0;
        let old = if v != 0 && was { v } else { 0 };
        set_pad_word(pad, 0, w, v);
        set_pad_word(pad, PAD_OLD_STATE, w, old);
    }
}

// ---- rotation and movement (onfoot §3) -----------------------------------

/// Receiver formula (onfoot §3.3): quaternion (w, x, y, z) → matrix rows.
pub fn rows_from_quat(q: Quat) -> Rotation {
    let Quat { w, x, y, z } = q;
    Rotation {
        right: [w * w + x * x - y * y - z * z, 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        forward: [2.0 * (x * y + w * z), w * w - x * x + y * y - z * z, 2.0 * (y * z - w * x)],
        up: [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), w * w - x * x - y * y + z * z],
    }
}

/// `D3DXQuaternionSlerp(a, b, t)`: `a` at t = 0, `b` at t = 1, shortest arc.
pub fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let mut cos = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
    let mut b = b;
    if cos < 0.0 {
        cos = -cos;
        b = Quat { w: -b.w, x: -b.x, y: -b.y, z: -b.z };
    }
    let (s0, s1) = if 1.0 - cos > 1e-6 {
        let theta = cos.min(1.0).acos();
        let sin = theta.sin();
        (((1.0 - t) * theta).sin() / sin, (t * theta).sin() / sin)
    } else {
        (1.0 - t, t)
    };
    Quat { w: s0 * a.w + s1 * b.w, x: s0 * a.x + s1 * b.x, y: s0 * a.y + s1 * b.y, z: s0 * a.z + s1 * b.z }
}

pub fn normalise(q: Quat) -> Quat {
    let n = (q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z).sqrt();
    if n > 0.0 && n.is_finite() {
        Quat { w: q.w / n, x: q.x / n, y: q.y / n, z: q.z / n }
    } else {
        Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 }
    }
}

/// samp's degrees → radians (A 0x100B59B0): 0…180° → 0…π, (180°, 360°] →
/// (−π, 0], anything else → 0.
pub fn samp_deg_to_rad(deg: f32) -> f32 {
    if (0.0..=180.0).contains(&deg) {
        deg.to_radians()
    } else if deg > 180.0 && deg <= 360.0 {
        (deg - 360.0).to_radians()
    } else {
        0.0
    }
}

/// Heading written to both ped heading fields after the slerp (onfoot §3.2).
pub fn heading_from_forward(fwd: [f32; 3]) -> f32 {
    let mut deg = (-fwd[0]).atan2(fwd[1]).to_degrees();
    if deg < 0.0 {
        deg += 360.0;
    }
    if deg >= 360.0 {
        deg -= 360.0;
    }
    samp_deg_to_rad(deg)
}

/// Result of the per-packet position correction (A 0x10014C40).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction {
    /// Within the dead zone on every axis.
    None,
    /// New move speed (synced velocity plus 0.1 × error on each axis outside the dead zone).
    Nudge([f32; 3]),
    /// Write the matrix translation.
    Teleport,
}

pub fn correct_position(current: [f32; 3], target: [f32; 3], speed: [f32; 3], in_moving_list: bool) -> Correction {
    if !in_moving_list {
        return Correction::Teleport;
    }
    let d = [0, 1, 2].map(|i| (target[i] - current[i]).abs());
    if d.iter().all(|&v| v < DEAD_ZONE) {
        return Correction::None;
    }
    if d[0] <= TELEPORT_XY && d[1] <= TELEPORT_XY && d[2] <= TELEPORT_Z {
        let mut s = speed;
        for i in 0..3 {
            if d[i] > DEAD_ZONE {
                s[i] += (target[i] - current[i]) * NUDGE_GAIN;
            }
        }
        Correction::Nudge(s)
    } else {
        Correction::Teleport
    }
}

// ---- stored sync -------------------------------------------------------------

/// The decoded fields of the last accepted 207 (remote-players-sync §207).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootData {
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    pub position: [f32; 3],
    pub rotation: Quat,
    pub health: f32,
    pub armour: f32,
    pub weapon: u8,
    pub special_action: u8,
    pub velocity: [f32; 3],
    /// 0xFFFF when the packet carried no surf block.
    pub surf_id: u16,
    pub animation: u32,
}

impl FootData {
    pub fn from_sync(s: &RemoteFootSync) -> Self {
        let v = s.velocity.to_vec3();
        let anim = s.animation.map(|a| a.as_u32()).unwrap_or(0);
        Self {
            left_right: s.left_right.unwrap_or(0),
            up_down: s.up_down.unwrap_or(0),
            keys: s.keys,
            position: [s.position.x, s.position.y, s.position.z],
            rotation: normalise(s.rotation.to_quat()),
            health: s.health_armour.health(),
            armour: s.health_armour.armour(),
            weapon: s.weapon.weapon(),
            special_action: s.special_action,
            velocity: [v.x, v.y, v.z],
            surf_id: s.surf.map(|x| x.id).unwrap_or(0xFFFF),
            // Negative values (flag bit 15 of the flags half) count as "no animation".
            animation: if (anim as i32) < 0 { 0 } else { anim },
        }
    }
}

/// Late-packet rule of the foot/driver store: with a timestamp, a packet
/// older than the last accepted one (signed 32-bit difference) is dropped.
pub fn accept_timestamp(last: Option<u32>, new: Option<u32>) -> bool {
    match (last, new) {
        (Some(l), Some(n)) => (n.wrapping_sub(l) as i32) >= 0,
        _ => true,
    }
}

/// One remote player (pool entry).
#[derive(Clone, Debug)]
pub struct RemotePlayer {
    pub id: u16,
    pub name: String,
    pub colour: u32,
    pub npc: bool,
    pub team: u8,
    pub skin: u32,
    pub state: u8,
    pub slot: Option<u8>,
    /// `CPed*` (0 = not streamed in).
    pub ped: u32,
    pub foot: Option<FootData>,
    /// A 207 was stored and not yet applied (update type 16).
    pub foot_new: bool,
    pub last_ts: Option<u32>,
    pub last_sync: Option<Instant>,
    /// Displayed health/armour (synced; the GTA ped keeps 1000).
    pub health: f32,
    pub armour: f32,
    /// Position corrections applied as a teleport (diagnostics).
    pub teleports: u32,
    /// |move speed| written by the last one-shot apply (diagnostics).
    pub last_speed: f32,
    /// Head look (onfoot §5.2): offset refreshed every 1000 ms, LookAt every 200 ms.
    pub look_offset: [f32; 3],
    pub look_refresh: Option<Instant>,
    pub look_last: Option<Instant>,
}

impl RemotePlayer {
    pub fn new(id: u16, name: String, colour: u32, npc: bool) -> Self {
        Self {
            id,
            name,
            colour,
            npc,
            team: 255,
            skin: 0,
            state: state::NONE,
            slot: None,
            ped: 0,
            foot: None,
            foot_new: false,
            last_ts: None,
            last_sync: None,
            health: 0.0,
            armour: 0.0,
            teleports: 0,
            last_speed: 0.0,
            look_offset: [0.0; 3],
            look_refresh: None,
            look_last: None,
        }
    }

    /// Store rule for 207 (A 0x10017260). Returns false for a late packet.
    pub fn store_foot(&mut self, ts: Option<u32>, s: &RemoteFootSync, now: Instant) -> bool {
        if !accept_timestamp(self.last_ts, ts) {
            return false;
        }
        if ts.is_some() {
            self.last_ts = ts;
        }
        let f = FootData::from_sync(s);
        self.health = f.health;
        self.armour = f.armour;
        self.foot = Some(f);
        self.foot_new = true;
        self.state = state::ON_FOOT;
        self.last_sync = Some(now);
        true
    }

    pub fn is_stale(&self, now: Instant) -> bool {
        self.last_sync.is_some_and(|t| now.duration_since(t) > STALE)
    }
}

/// One `remote` control-channel line.
pub fn format_remote(
    p: &RemotePlayer,
    pos: Option<[f32; 3]>,
    heading: Option<f32>,
    ped_health: Option<f32>,
    now: Instant,
) -> String {
    let slot = p.slot.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
    let pos = pos.map(|v| format!("{:.2},{:.2},{:.2}", v[0], v[1], v[2])).unwrap_or_else(|| "-".into());
    let heading = heading.map(|h| format!("{h:.3}")).unwrap_or_else(|| "-".into());
    let age = p.last_sync.map(|t| now.duration_since(t).as_millis().to_string()).unwrap_or_else(|| "-".into());
    let sync_pos = p
        .foot
        .map(|f| format!("{:.2},{:.2},{:.2}", f.position[0], f.position[1], f.position[2]))
        .unwrap_or_else(|| "-".into());
    let ped_health = ped_health.map(|h| format!("{h:.0}")).unwrap_or_else(|| "-".into());
    format!(
        "remote id={} name={} slot={slot} ped={:#x} pos={pos} heading={heading} health={:.0} ped_health={ped_health} \
         state={} sync_age_ms={age} sync_pos={sync_pos} teleports={} speed={:.4}",
        p.id, p.name, p.ped, p.health, p.state, p.teleports, p.last_speed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footsync::quat_from_matrix;

    fn heading_quat(h: f32) -> Quat {
        Quat { w: (h / 2.0).cos(), x: 0.0, y: 0.0, z: -(h / 2.0).sin() }
    }

    #[test]
    fn slot_allocation_first_free_from_2() {
        let mut a = SlotAllocator::default();
        assert_eq!(a.alloc(), Some(2));
        assert_eq!(a.alloc(), Some(3));
        a.free(2);
        assert_eq!(a.count(), 1);
        assert_eq!(a.alloc(), Some(2));
        for _ in 0..206 {
            assert!(a.alloc().is_some());
        }
        assert_eq!(a.count(), 208);
        assert_eq!(a.alloc(), None);
        a.free(0); // never allocated; harmless
        a.free(250);
    }

    #[test]
    fn swap_table_lookup() {
        let mut t = SwapTable::default();
        t.bind(5, 0x1234, [500.0; SKILLS]);
        assert_eq!(t.slot_of(0x1234), Some(5));
        assert_eq!(t.slot_of(0), None);
        assert_eq!(t.shadows[5].skills[0], 500.0);
        assert_eq!(t.shadows[5].cam_mode, 4);
        assert_eq!(t.shadows[5].weapon_mode(), 0);
        t.unbind(5);
        assert_eq!(t.slot_of(0x1234), None);
    }

    #[test]
    fn keys_edges_and_mapping() {
        let mut pad = [0u8; PAD_SIZE];
        let i = PadInput { keys: 8 | 4 | 128, left_right: 0, up_down: -128, ..Default::default() };
        apply_keys(&mut pad, i);
        // Just pressed: NewState set, OldState 0.
        assert_eq!(pad_word(&pad, 0x10), 0xFF); // sprint → Cross
        assert_eq!(pad_word(&pad, 0x11), 0xFF); // fire → Circle
        assert_eq!(pad_word(&pad, 6), 0xFF); // aim → RightShoulder1
        assert_eq!(pad_word(&pad, 1), -128);
        let old = |p: &[u8; PAD_SIZE], w: usize| i16::from_le_bytes([p[0x30 + 2 * w], p[0x31 + 2 * w]]);
        assert_eq!(old(&pad, 0x10), 0);
        assert_eq!(old(&pad, 1), 0);
        // Held: OldState == NewState.
        apply_keys(&mut pad, i);
        assert_eq!(old(&pad, 0x10), 0xFF);
        assert_eq!(old(&pad, 1), -128);
        // Released: both zero, no "just released" edge.
        apply_keys(&mut pad, PadInput::default());
        assert_eq!((pad_word(&pad, 0x10), old(&pad, 0x10)), (0, 0));
    }

    #[test]
    fn keys_gates() {
        let mut pad = [0u8; PAD_SIZE];
        // Crouch (2) only in a vehicle; secondary attack (16) masked by the jetpack on foot.
        apply_keys(&mut pad, PadInput { keys: 2 | 16, jetpack: true, ..Default::default() });
        assert_eq!((pad_word(&pad, 0x12), pad_word(&pad, 0x0F)), (0, 0));
        apply_keys(&mut pad, PadInput { keys: 2 | 16, jetpack: true, in_vehicle: true, ..Default::default() });
        assert_eq!((pad_word(&pad, 0x12), pad_word(&pad, 0x0F)), (0xFF, 0xFF));
        // Analog keys and walk.
        apply_keys(&mut pad, PadInput { keys: (1 << 13) | (1 << 12) | 1024, ..Default::default() });
        assert_eq!((pad_word(&pad, 2), pad_word(&pad, 3), pad_word(&pad, 0x15)), (-128, 128, 0xFF));
    }

    #[test]
    fn quat_matrix_round_trip_and_heading() {
        for h in [0.0f32, 0.5, 1.5, 3.0, -2.0] {
            let rows = rows_from_quat(heading_quat(h));
            assert!((rows.forward[0] + h.sin()).abs() < 1e-5 && (rows.forward[1] - h.cos()).abs() < 1e-5);
            let back = quat_from_matrix(&rows);
            let q = heading_quat(h);
            let dot = back.w * q.w + back.x * q.x + back.y * q.y + back.z * q.z;
            assert!(dot.abs() > 0.99999, "h={h}");
            let hd = heading_from_forward(rows.forward);
            let diff = (hd - h).rem_euclid(std::f32::consts::TAU);
            assert!(diff < 1e-4 || diff > std::f32::consts::TAU - 1e-4, "h={h} got {hd}");
        }
    }

    #[test]
    fn slerp_quarter_step() {
        let cur = heading_quat(0.0);
        let target = heading_quat(1.0);
        let q = slerp(target, cur, SLERP_KEEP);
        let h = heading_from_forward(rows_from_quat(normalise(q)).forward);
        assert!((h - 0.25).abs() < 1e-4, "{h}");
        // Shortest arc through the sign flip.
        let neg = Quat { w: -target.w, x: -target.x, y: -target.y, z: -target.z };
        let q2 = slerp(neg, cur, SLERP_KEEP);
        let h2 = heading_from_forward(rows_from_quat(normalise(q2)).forward);
        assert!((h2 - 0.25).abs() < 1e-4, "{h2}");
        // Identical inputs.
        assert_eq!(normalise(slerp(cur, cur, SLERP_KEEP)), cur);
    }

    #[test]
    fn degrees_rule() {
        assert_eq!(samp_deg_to_rad(0.0), 0.0);
        assert!((samp_deg_to_rad(180.0) - std::f32::consts::PI).abs() < 1e-6);
        assert!((samp_deg_to_rad(270.0) + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(samp_deg_to_rad(-5.0), 0.0);
        assert_eq!(samp_deg_to_rad(400.0), 0.0);
    }

    #[test]
    fn position_correction() {
        let c = [0.0, 0.0, 0.0];
        let v = [0.03, 0.0, 0.0];
        assert_eq!(correct_position(c, c, v, true), Correction::None);
        assert_eq!(correct_position(c, [1.0, 0.0, 0.0], v, false), Correction::Teleport);
        match correct_position(c, [1.0, -2.0, 0.0], v, true) {
            Correction::Nudge(s) => {
                assert!((s[0] - 0.13).abs() < 1e-6 && (s[1] + 0.2).abs() < 1e-6 && s[2] == 0.0);
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(correct_position(c, [2.5, 0.0, 0.0], v, true), Correction::Teleport);
        assert_eq!(correct_position(c, [0.0, 0.0, 1.5], v, true), Correction::Teleport);
    }

    #[test]
    fn timestamps() {
        assert!(accept_timestamp(None, Some(5)));
        assert!(accept_timestamp(Some(5), None));
        assert!(accept_timestamp(Some(5), Some(5)));
        assert!(!accept_timestamp(Some(5), Some(4)));
        assert!(accept_timestamp(Some(u32::MAX - 1), Some(3)), "wrap-around");
    }

    #[test]
    fn foot_store_and_format() {
        let now = Instant::now();
        let mut p = RemotePlayer::new(3, "RemoteBot".into(), 0, false);
        let s = RemoteFootSync {
            player_id: 3,
            position: samp_proto::types::Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            health_armour: samp_proto::sync::encoding::PackedHealthArmour::from_values(100.0, 0.0),
            ..Default::default()
        };
        assert!(p.store_foot(Some(10), &s, now));
        assert!(!p.store_foot(Some(9), &s, now));
        assert_eq!(p.state, state::ON_FOOT);
        assert!(p.foot_new);
        assert_eq!(p.foot.unwrap().surf_id, 0xFFFF);
        assert_eq!(p.health, 100.0);
        p.slot = Some(2);
        p.ped = 0x1000;
        let line = format_remote(&p, Some([1.0, 2.0, 3.0]), Some(0.5), Some(1000.0), now);
        assert!(
            line.starts_with("remote id=3 name=RemoteBot slot=2 ped=0x1000 pos=1.00,2.00,3.00 heading=0.500"),
            "{line}"
        );
        assert!(line.contains("sync_age_ms=0"));
        assert!(!p.is_stale(now));
        assert!(p.is_stale(now + Duration::from_millis(3001)));
    }

    #[test]
    fn collision_and_look_rules() {
        let mut f = FootData::from_sync(&RemoteFootSync::default());
        assert_eq!(collision_wanted(&f), Some(true));
        f.special_action = 3;
        assert_eq!(collision_wanted(&f), Some(false));
        f.special_action = 0;
        f.surf_id = 5;
        assert_eq!(collision_wanted(&f), Some(false));
        f.surf_id = 2005;
        assert_eq!(collision_wanted(&f), None);
        assert_eq!(collision_flags(0x8000_0002, true), 1);
        assert_eq!(collision_flags(0x0000_0101, false), 0x8000_0102);
        let now = Instant::now();
        let mut p = RemotePlayer::new(1, "a".into(), 0, false);
        assert_eq!(head_look_due(&p, now), (true, true));
        p.look_refresh = Some(now);
        p.look_last = Some(now);
        assert_eq!(head_look_due(&p, now + Duration::from_millis(250)), (false, true));
        assert_eq!(head_look_due(&p, now + Duration::from_millis(1000)), (true, true));
    }

    #[test]
    fn aim_store() {
        let mut s = SlotShadow::default();
        s.apply_aim(53, [0.0, 1.0, 0.0], [1.0, 2.0, 3.0], 63, 255);
        assert_eq!(s.cam_mode, 53);
        assert_eq!(s.weapon_mode(), 53);
        assert_eq!(&s.aim[0..3], &[0.0, 1.0, 0.0]);
        assert_eq!(&s.aim[3..9], &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
        assert_eq!(&s.aim[9..12], &[0.0, 0.0, 1.0]);
        assert_eq!((s.fov(), s.aspect), (70.0, 1.0));
    }
}
