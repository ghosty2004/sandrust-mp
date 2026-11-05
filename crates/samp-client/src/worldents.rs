//! World entities (milestone M6): the platform-independent rules, host-tested.
//!
//! The GTA-side work lives in `objects.rs` (objects), `entities.rs` (pickups, actors, checkpoints, gang zones,
//! map icons, 3D labels, building removal) and `worldhooks.rs` (the CSV hook rows); this file holds the
//! constants and the arithmetic they share.
//!
//! Specs: docs/re/subsystems/world-entities.md (RPC layouts, RE-10 additions §1–§10) and the measurements of
//! the original client in docs/phase3/trace-notes.md "World entities (TRACE-6)". Constants read from samp.dll
//! build A (2021) are cited with their address.

use samp_proto::types::Quat;

use crate::footsync::Rotation;

/// Object ids the client accepts (the original tests `id < 1001` against 1000-entry arrays; world-entities.md
/// "Object-ID bound": a Rust client accepts 0…999).
pub const MAX_OBJECTS: u16 = 1000;
/// The "?" model used for models without model info (A 0x100a923b: 0x48C7).
pub const MODEL_UNKNOWN: i32 = 18631;
/// Model ids above this are not world models (A 0x100a09d0 compares with 0x4E20).
pub const MAX_MODEL: i32 = 20000;
/// Draw distance above which samp streams and draws the object itself (constant A 0x100E6170).
pub const SAMP_PASS_DRAW_DISTANCE: f32 = 300.0;
/// Ordinary objects request their model when the camera is closer than draw distance − 30 (A 0x100E5808).
pub const REQUEST_MARGIN: f32 = 30.0;
/// Camera distance below which samp's own pass renders through GTA's renderer (A 0x100ECD88).
pub const DIRECT_RENDER_DISTANCE: f32 = 600.0;
/// Consecutive out-of-range frames before a samp-streamed object loses its RW object (world-entities.md RE-10
/// §2; TRACE-6: exactly 1000 frames).
pub const RW_RELEASE_FRAMES: u32 = 1000;
/// Material slots per object (the client checks `slot < 17` against 16-entry arrays; accept 0…15).
pub const MATERIAL_SLOTS: usize = 16;
/// Material name limit: names of 32 or more characters make the client skip the record.
pub const MATERIAL_NAME_MAX: usize = 31;

/// MoveObject: GTA speed units per second → per 1/50 s (A 0x100E6130).
pub const MOVE_SPEED_SCALE: f32 = 0.02;
/// MoveObject: drift threshold and gain (A 0x100E611C, used for both).
pub const MOVE_DRIFT: f32 = 0.1;
/// MoveObject: rotation turn-speed gain and clamp (A 0x100E6120, A 0x100E60A4 / 0x100ECD80).
pub const TURN_GAIN: f32 = 0.01;
pub const TURN_CLAMP: f32 = 0.001;
/// MoveObject: a target rotation component ≤ this means "do not rotate" (open.mp sends −1000).
pub const NO_ROTATION: f32 = -999.0;

/// Pickup ids (`0 ≤ id < 0x1000`, world-entities.md 95).
pub const MAX_PICKUPS: i32 = 4096;
/// GTA's pickup array (`CPickups::aPickUps`, 32-byte entries; world-entities.md 131 stub at A 0x100a3f80).
pub const GTA_PICKUPS: u32 = 0x9788C0;
pub const GTA_PICKUP_SIZE: u32 = 0x20;
/// Collision-path cooldown, counted down by one per poll (A 0x100134e5).
pub const PICKUP_COOLDOWN: u32 = 15;
/// The poll runs every 7 frames (TRACE-6: HAS_PICKUP_BEEN_COLLECTED for type-14 pickups every 7 frames exactly).
pub const PICKUP_POLL_FRAMES: u64 = 7;
/// Pickup type polled with HAS_PICKUP_BEEN_COLLECTED (A 0x10013573).
pub const PICKUP_TYPE_POLLED: i32 = 14;

/// Actor ids (`id < 1000`).
pub const MAX_ACTORS: u16 = 1000;
/// Ped type of actors (CREATE_CHAR, TRACE-6).
pub const ACTOR_PED_TYPE: i32 = 5;
/// Decision maker given to actors (SET_CHAR_DECISION_MAKER, TRACE-6: 65542).
pub const ACTOR_DECISION_MAKER: i32 = 65542;

/// Gang zone slots (1024; the client does not bounds-check the wire id, we do).
pub const MAX_GANG_ZONES: u16 = 1024;
/// Flash phase period (A 0x10002284: 500 ms by the system tick count).
pub const GANG_ZONE_FLASH_MS: u32 = 500;
/// Map icon slots (`id < 100`).
pub const MAX_MAP_ICONS: u8 = 100;
/// 3D label slots (`id < 0x800`).
pub const MAX_LABELS: u16 = 2048;

/// Blip colour index of checkpoints (`CRadar::GetRadarTraceColour` replacement A 0x100a7270).
pub const BLIP_COLOUR_CHECKPOINT: i32 = 0x3ED;

/// SCM opcodes (named via gta-reversed `eScriptCommands.h`; world-entities.md RE-10 §1).
pub mod op {
    pub const CREATE_OBJECT: u16 = 0x0107;
    pub const DELETE_OBJECT: u16 = 0x0108;
    pub const SET_OBJECT_ROTATION: u16 = 0x0453;
    pub const ATTACH_OBJECT_TO_CAR: u16 = 0x0681;
    pub const ATTACH_OBJECT_TO_OBJECT: u16 = 0x069A;
    pub const ATTACH_OBJECT_TO_CHAR: u16 = 0x069B;
    pub const IS_OBJECT_ATTACHED: u16 = 0x0685;
    pub const CREATE_PICKUP: u16 = 0x0213;
    pub const HAS_PICKUP_BEEN_COLLECTED: u16 = 0x0214;
    pub const REMOVE_PICKUP: u16 = 0x0215;
    pub const CREATE_CHAR: u16 = 0x009A;
    pub const SET_CHAR_DECISION_MAKER: u16 = 0x060B;
    pub const EXPLODE_CHAR_HEAD: u16 = 0x0321;
    pub const TASK_PLAY_ANIM_NON_INTERRUPTABLE: u16 = 0x0812;
    pub const LOCATE_CHAR_ANY_MEANS_3D: u16 = 0x00FE;
    pub const CREATE_CHECKPOINT: u16 = 0x06D5;
    pub const DELETE_CHECKPOINT: u16 = 0x06D6;
    pub const ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD: u16 = 0x04CE;
    pub const ADD_SPRITE_BLIP_FOR_COORD: u16 = 0x02A8;
    pub const ADD_SHORT_RANGE_SPRITE_BLIP_FOR_CONTACT_POINT: u16 = 0x0570;
    pub const ADD_SPRITE_BLIP_FOR_CONTACT_POINT: u16 = 0x02A7;
}

/// Blip opcode of a map icon style (world-entities.md 56; TRACE-6 confirmed 0–3).
pub fn map_icon_opcode(style: u8) -> u16 {
    match style {
        1 => op::ADD_SPRITE_BLIP_FOR_COORD,
        2 => op::ADD_SHORT_RANGE_SPRITE_BLIP_FOR_CONTACT_POINT,
        3 => op::ADD_SPRITE_BLIP_FOR_CONTACT_POINT,
        _ => op::ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD,
    }
}

/// CHANGE_BLIP_SCALE of a type-0 map icon: 2 for colours below 1004, else 3. The comparison is unsigned:
/// TRACE-6 saw scale 3 for 0xFF0000FF (negative as an int) and 2 for colour 5.
pub fn map_icon_scale(colour: u32) -> i32 {
    if colour < 1004 { 2 } else { 3 }
}

/// `CRadar::GetRadarTraceColour` replacement (CSV row 51, A 0x100a7270): fixed colours for the vehicle and
/// checkpoint blips, the player colour table for ids below 4000, RGBA values otherwise.
pub fn radar_trace_colour(index: u32, player_rgba: impl Fn(u16) -> u32) -> u32 {
    match index {
        0x3EC => 0x89A4_C440,
        0x3ED => 0xAA00_00FF,
        0x3EE => 0xE2C0_63FF,
        i if i < 4000 => player_rgba(i as u16),
        i => i,
    }
}

/// RPC 36: the client rotates the wire colour right by 8 bits (RGBA → ARGB).
pub fn label_argb(rgba: u32) -> u32 {
    rgba.rotate_right(8)
}

/// Material texture model of a kind-1 record: 0xFFFF → −1 (colour only); RPC 44 also maps > 20000 to −1.
pub fn material_model(model: u16, from_create: bool) -> i32 {
    if model == 0xFFFF || (from_create && i32::from(model) > MAX_MODEL) { -1 } else { i32::from(model) }
}

/// A material name the client accepts (shorter than 32 characters).
pub fn material_name_ok(name: &[u8]) -> bool {
    name.len() <= MATERIAL_NAME_MAX
}

/// The object pass a samp-marked object belongs to (render-list filter, CSV row 78, A 0x100A5150).
pub fn drawn_by_samp(has_materials: bool, draw_distance: f32, timed: bool) -> bool {
    has_materials || draw_distance > SAMP_PASS_DRAW_DISTANCE || timed
}

/// Degrees (SA:MP object rotation) → the rotation rows SET_OBJECT_ROTATION gives a CObject
/// (`CPlaceable::SetOrientation(x, y, z)` in radians; gta-reversed `Matrix.cpp` `SetRotate`):
/// TRACE-6 shows rotation (0, 0, 90) as right = (0, 1, 0), forward = (−1, 0, 0).
pub fn rotation_rows(deg: [f32; 3]) -> Rotation {
    let [x, y, z] = deg.map(f32::to_radians);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    Rotation {
        right: [cz * cy - sz * sx * sy, cz * sx * sy + sz * cy, -sy * cx],
        forward: [-sz * cx, cz * cx, sx],
        up: [sz * sx * cy + cz * sy, sz * sy - cz * sx * cy, cx * cy],
    }
}

pub fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A MoveObject in progress (world-entities.md 99, RE-10 §3; samp wrapper +0x10B target, +0x15B total
/// distance, +0x15F start time, +0x1B0 speed, +0x1B5 rotation flag, quaternions +0x167/+0x177/+0x187).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub target: [f32; 3],
    pub speed: f32,
    pub start_ms: u32,
    pub total: f32,
    /// Target rotation in degrees, when all three components were > −999.
    pub rotation: Option<[f32; 3]>,
    /// Rotation at the start of the move and the target rotation.
    pub from: Quat,
    pub to: Quat,
}

/// What one frame of motion does to the object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MotionStep {
    /// Arrived: speeds 0, position = target, rotation = target rotation (if any), move ends.
    Arrive { pos: [f32; 3], rotation: Option<Rotation> },
    /// Moving: set the move speed, advance the matrix by move speed × time step; with a rotation also the
    /// turn speed z and the slerped rotation.
    Move { move_speed: [f32; 3], turn_z: Option<f32>, progress: f32 },
    /// dt ≤ 0: nothing this frame.
    Idle,
}

impl Motion {
    /// `now_ms`: samp's millisecond clock; `dt`: seconds since the previous object pass.
    pub fn step(&self, current: [f32; 3], dt: f32, now_ms: u32) -> MotionStep {
        let step = dt * self.speed;
        let remaining = dist3(self.target, current);
        let expected = now_ms.wrapping_sub(self.start_ms) as i32 as f32 * 0.001 * self.speed;
        let travelled = self.total - remaining;
        if remaining <= step {
            let rotation = self.rotation.map(rotation_rows);
            return MotionStep::Arrive { pos: self.target, rotation };
        }
        if dt <= 0.0 {
            return MotionStep::Idle;
        }
        let f = step / remaining;
        let next = [0, 1, 2].map(|i| current[i] + (self.target[i] - current[i]) * f);
        let inv_dt = 1.0 / dt;
        let mut v = [0, 1, 2].map(|i| (next[i] - current[i]) * inv_dt * MOVE_SPEED_SCALE);
        let drift = expected - travelled;
        if drift.abs() > MOVE_DRIFT {
            let k = 1.0 + drift * MOVE_DRIFT;
            v = v.map(|c| c * k);
        }
        let progress = if self.total > 0.0 { travelled / self.total } else { 1.0 };
        let turn_z = self.rotation.map(|r| {
            let diff = r[2] - current_heading_hint(self);
            (TURN_GAIN * diff).clamp(-TURN_CLAMP, TURN_CLAMP)
        });
        MotionStep::Move { move_speed: v, turn_z, progress }
    }
}

/// The turn speed only drives GTA's own (unused) rotation integration; samp clamps it to ±0.001 almost
/// always (TRACE-6: 0.001 every frame of a 90° turn). The z of the start rotation is the reference.
fn current_heading_hint(m: &Motion) -> f32 {
    let q = m.from;
    (2.0 * (q.w * q.z + q.x * q.y)).atan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z)).to_degrees()
}

/// Gang zone flash phase (A 0x10002270): toggles when 500 ms passed since the last toggle (GetTickCount).
#[derive(Clone, Copy, Debug, Default)]
pub struct FlashPhase {
    pub on: bool,
    pub last_ms: u32,
}

impl FlashPhase {
    pub fn update(&mut self, now_ms: u32) -> bool {
        if now_ms.wrapping_sub(self.last_ms) as i32 >= GANG_ZONE_FLASH_MS as i32 {
            self.on = !self.on;
            self.last_ms = now_ms;
        }
        self.on
    }
}

/// One gang zone record (A 0x10002180: `[minX, maxY, maxX, minY, colour, flash colour]`, the CRect order
/// `CRadar::DrawAreaOnRadar` takes; TRACE-6 saw exactly that rect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GangZone {
    pub rect: [f32; 4],
    pub colour: u32,
    pub flash: u32,
}

impl GangZone {
    pub fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32, colour: u32) -> Self {
        Self { rect: [min_x, max_y, max_x, min_y], colour, flash: colour }
    }

    /// The colour drawn this frame.
    pub fn colour_now(&self, flash_phase: bool) -> u32 {
        if flash_phase { self.flash } else { self.colour }
    }
}

/// The CRGBA bytes `DrawAreaOnRadar` gets: the wire colour's bytes as they are in memory (TRACE-6:
/// 0xFF000080 → R ff, G 00, B 00, A 80, i.e. the server RGBA value read big-endian).
pub fn crgba_bytes(wire: u32) -> [u8; 4] {
    wire.to_le_bytes()
}

/// RemoveBuildingForPlayer match rule (world-entities.md RE-10 §7): model equal (or −1) and within radius (≤).
pub fn removal_matches(removal_model: i32, centre: [f32; 3], radius: f32, model: i32, pos: [f32; 3]) -> bool {
    (removal_model == -1 || removal_model == model) && dist3(centre, pos) <= radius
}

/// Model a matching IPL instance is replaced with (row 65, A 0x100a4b03: 0x4B64).
pub const REMOVED_BUILDING_MODEL: i32 = 19300;
/// Removal: occluders within this distance of the point are zeroed; matching entities sink by 2000 units.
pub const REMOVAL_OCCLUDER_RANGE: f32 = 500.0;
pub const REMOVAL_SINK: f32 = 2000.0;
/// `COcclusion::Occluders` (0xC73FA0), 0x12-byte records, count at `NumOccludersOnMap` (0xC73F98).
pub const OCCLUDERS: u32 = 0xC73FA0;
pub const OCCLUDER_COUNT: u32 = 0xC73F98;
pub const OCCLUDER_SIZE: u32 = 0x12;
/// The part of an occluder record a removal clears (A 0x1009D0F0): the six i16 of centre and size (+0..+0xC). The
/// rotation bytes and the 15-bit next-occluder link + far-away bit at +0x10 stay: GTA walks its nearby / far-away
/// occluder lists through that link every frame (0x7201C0), so a zeroed link points at occluder 0 and can close
/// a cycle (the main thread then spins in the list walk forever).
pub const OCCLUDER_CLEARED: std::ops::Range<u32> = 0..0xC;

/// samp's degrees → radians for actor facing (A 0x100B59B0): outside 0…360 → 0; above 180 → negative.
pub fn samp_actor_heading(deg: f32) -> f32 {
    if !(0.0..=360.0).contains(&deg) {
        0.0
    } else if deg > 180.0 {
        (deg - 180.0) * std::f32::consts::PI / 180.0 - std::f32::consts::PI
    } else {
        deg * std::f32::consts::PI / 180.0
    }
}

/// RPC 173: the library samp refuses (case-insensitive "SEX").
pub fn actor_anim_refused(library: &[u8]) -> bool {
    library.eq_ignore_ascii_case(b"SEX")
}

/// SET_CHAR_PROOFS arguments of an actor (world-entities.md 171; TRACE-6).
pub fn actor_proofs(invulnerable: bool) -> [i32; 5] {
    if invulnerable { [1, 1, 1, 1, 1] } else { [0, 0, 0, 1, 0] }
}

/// The physical flags samp tests for an invulnerable actor (row 20, A 0x100a2cba): bullet, explosion and
/// melee proof (`CPhysical` +0x40 bits 0x40000, 0x800000, 0x200000).
pub fn invulnerable_flags(flags: u32) -> bool {
    flags & 0x40000 != 0 && flags & 0x80_0000 != 0 && flags & 0x20_0000 != 0
}

/// Material pre-pass gate of one samp object (samp +0x1D3 = RW object of the last record, +0x118F = drawn since
/// that record; A 0x100A8C40 / 0x100AA082 / 0x100AA124 / 0x100A9E9D). Nothing is applied before the first draw;
/// the first pre-pass after the RW object changed only records; later ones apply. A new RW object is therefore
/// drawn twice with the geometry's own flags before the material colour state (PRELIT off) is ever applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialPrePass {
    rw: u32,
    drawn: bool,
}

impl MaterialPrePass {
    /// Pre-pass for RW object `rw`: whether the materials are applied in this draw.
    pub fn begin(&mut self, rw: u32) -> bool {
        if !self.drawn {
            return false;
        }
        if self.rw != rw {
            self.rw = rw;
            self.drawn = false;
            return false;
        }
        true
    }

    /// End of a draw (+0x118F := 1).
    pub fn drawn(&mut self) {
        self.drawn = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_pre_pass_applies_from_the_third_draw_of_an_rw_object() {
        let mut p = MaterialPrePass::default();
        let mut applied = Vec::new();
        for rw in [0xA0, 0xA0, 0xA0, 0xA0, 0xB0, 0xB0, 0xB0] {
            applied.push(p.begin(rw));
            p.drawn();
        }
        assert_eq!(applied, [false, false, true, true, false, true, true]);
        // An undrawn (culled) object never gets past the first gate.
        let mut q = MaterialPrePass::default();
        assert!(!q.begin(0xA0) && !q.begin(0xA0));
    }

    #[test]
    fn map_icons() {
        assert_eq!(map_icon_opcode(0), 0x04CE);
        assert_eq!(map_icon_opcode(1), 0x02A8);
        assert_eq!(map_icon_opcode(2), 0x0570);
        assert_eq!(map_icon_opcode(3), 0x02A7);
        assert_eq!(map_icon_opcode(9), 0x04CE);
        assert_eq!(map_icon_scale(5), 2);
        assert_eq!(map_icon_scale(0xFF00_00FF), 3);
        assert_eq!(map_icon_scale(1004), 3);
    }

    #[test]
    fn radar_colours() {
        let pc = |id: u16| 0x1000 + u32::from(id);
        assert_eq!(radar_trace_colour(0x3ED, pc), 0xAA00_00FF);
        assert_eq!(radar_trace_colour(0x3EC, pc), 0x89A4_C440);
        assert_eq!(radar_trace_colour(7, pc), 0x1007);
        assert_eq!(radar_trace_colour(3999, pc), 0x1000 + 3999);
        assert_eq!(radar_trace_colour(0xFF00_00FF, pc), 0xFF00_00FF);
    }

    #[test]
    fn colours_and_materials() {
        assert_eq!(label_argb(0x11223344), 0x44112233);
        assert_eq!(material_model(0xFFFF, false), -1);
        assert_eq!(material_model(20001, true), -1);
        assert_eq!(material_model(20001, false), 20001);
        assert_eq!(material_model(0, true), 0);
        assert!(material_name_ok(&[b'a'; 31]) && !material_name_ok(&[b'a'; 32]));
        assert_eq!(crgba_bytes(0xFF00_0080), [0x80, 0, 0, 0xFF]);
    }

    #[test]
    fn render_pass_choice() {
        assert!(!drawn_by_samp(false, 0.0, false));
        assert!(!drawn_by_samp(false, 300.0, false));
        assert!(drawn_by_samp(false, 300.5, false));
        assert!(drawn_by_samp(true, 0.0, false));
        assert!(drawn_by_samp(false, 0.0, true));
    }

    #[test]
    fn rotation_matches_trace() {
        let r = rotation_rows([0.0, 0.0, 90.0]);
        for (a, b) in r.right.iter().zip([0.0, 1.0, 0.0]) {
            assert!((a - b).abs() < 1e-6);
        }
        for (a, b) in r.forward.iter().zip([-1.0, 0.0, 0.0]) {
            assert!((a - b).abs() < 1e-6);
        }
        let i = rotation_rows([0.0; 3]);
        assert_eq!(i.up, [0.0, 0.0, 1.0]);
    }

    fn motion(target: [f32; 3], speed: f32, total: f32) -> Motion {
        let q = Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };
        Motion { target, speed, start_ms: 0, total, rotation: None, from: q, to: q }
    }

    #[test]
    fn motion_speed_is_speed_times_0_02() {
        // 2 m/s straight up, 10 m: on schedule the move speed is exactly 2 × 0.02 = 0.04 per 1/50 s.
        let m = motion([0.0, 0.0, 10.0], 2.0, 10.0);
        match m.step([0.0, 0.0, 1.0], 0.02, 500) {
            MotionStep::Move { move_speed, progress, turn_z } => {
                assert!((move_speed[2] - 0.04).abs() < 1e-6, "{move_speed:?}");
                assert!((progress - 0.1).abs() < 1e-6);
                assert_eq!(turn_z, None);
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn motion_catches_up_and_arrives() {
        let m = motion([0.0, 0.0, 10.0], 2.0, 10.0);
        // 1 s in but only 1 m travelled: 1 m behind → velocity × (1 + 1 × 0.1).
        match m.step([0.0, 0.0, 1.0], 0.02, 1000) {
            MotionStep::Move { move_speed, .. } => assert!((move_speed[2] - 0.044).abs() < 1e-5),
            s => panic!("{s:?}"),
        }
        assert_eq!(m.step([0.0, 0.0, 9.97], 0.02, 4985), MotionStep::Arrive { pos: [0.0, 0.0, 10.0], rotation: None });
        assert_eq!(m.step([0.0, 0.0, 5.0], 0.0, 2500), MotionStep::Idle);
    }

    #[test]
    fn turn_speed_is_clamped() {
        let mut m = motion([0.0, 0.0, 10.0], 2.0, 10.0);
        m.rotation = Some([0.0, 0.0, 90.0]);
        match m.step([0.0, 0.0, 1.0], 0.02, 500) {
            MotionStep::Move { turn_z, .. } => assert_eq!(turn_z, Some(TURN_CLAMP)),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn flash_phase_toggles_every_500_ms() {
        let mut p = FlashPhase::default();
        assert!(p.update(1000));
        assert!(p.update(1499));
        assert!(!p.update(1500));
        let z = GangZone::new(-10.0, -20.0, 10.0, 20.0, 0xFF0000AA);
        assert_eq!(z.rect, [-10.0, 20.0, 10.0, -20.0]);
        assert_eq!(z.colour_now(true), 0xFF0000AA);
    }

    #[test]
    fn actor_heading() {
        assert_eq!(samp_actor_heading(-1.0), 0.0);
        assert_eq!(samp_actor_heading(361.0), 0.0);
        assert!((samp_actor_heading(180.0) - std::f32::consts::PI).abs() < 1e-6);
        assert!((samp_actor_heading(270.0) + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn removals_and_actors() {
        assert!(removal_matches(-1, [0.0; 3], 5.0, 1234, [3.0, 4.0, 0.0]));
        assert!(!removal_matches(10, [0.0; 3], 5.0, 1234, [3.0, 4.0, 0.0]));
        assert!(!removal_matches(1234, [0.0; 3], 4.9, 1234, [3.0, 4.0, 0.0]));
        assert!(actor_anim_refused(b"sex") && !actor_anim_refused(b"PED"));
        assert_eq!(actor_proofs(false), [0, 0, 0, 1, 0]);
        assert!(invulnerable_flags(0x40000 | 0x80_0000 | 0x20_0000));
        assert!(!invulnerable_flags(0x40000 | 0x80_0000));
    }

    #[test]
    fn removal_keeps_the_occluder_list_link() {
        // Centre (3 × i16) and size (3 × i16) only; rotation (+0xC..+0xE) and the list link (+0x10) are kept.
        let mut rec = [0xAAu8; OCCLUDER_SIZE as usize];
        rec[0x10..0x12].copy_from_slice(&0x0007u16.to_le_bytes());
        for b in OCCLUDER_CLEARED {
            rec[b as usize] = 0;
        }
        assert!(rec[..12].iter().all(|&b| b == 0));
        assert!(rec[12..16].iter().all(|&b| b == 0xAA));
        assert_eq!(u16::from_le_bytes([rec[0x10], rec[0x11]]), 7, "next-occluder link untouched");
    }
}
