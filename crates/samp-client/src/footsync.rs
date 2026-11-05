//! Local on-foot sync (packet 207) and stats (205): building the packet from
//! the local ped and deciding when to send it.
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §0.3 (key word), §0.4
//! (send-rate machinery and the 501 ms duplicate suppression), §2 "207
//! PlayerFootSync" (field sources and the quaternion jitter filter), and
//! remote-control-onfoot.md §3.3 (matrix → quaternion convention).

use std::time::{Duration, Instant};

use samp_proto::bitstream::BitWriter;
use samp_proto::sync::SyncPacket;
use samp_proto::sync::encoding::WeaponByte;
use samp_proto::sync::player::{Animation, FootSync};
use samp_proto::types::{Quat, Vec3};

/// Identical structs are not re-sent within this window (§0.4).
pub const DUPLICATE_WINDOW: Duration = Duration::from_millis(501);
/// Stats (205) interval: "every > 1000 ms".
pub const STATS_INTERVAL: Duration = Duration::from_millis(1000);
/// Rotation components closer than this to the last sent ones reuse the old quaternion.
pub const JITTER_EPSILON: f32 = 1e-5;

/// The game pad's `CControllerState` (gta-reversed `ControllerState.h`):
/// 24 signed 16-bit words in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub left_stick_x: i16,
    pub left_stick_y: i16,
    pub right_stick_x: i16,
    pub right_stick_y: i16,
    pub left_shoulder1: i16,
    pub left_shoulder2: i16,
    pub right_shoulder1: i16,
    pub right_shoulder2: i16,
    pub dpad_up: i16,
    pub dpad_down: i16,
    pub dpad_left: i16,
    pub dpad_right: i16,
    pub start: i16,
    pub select: i16,
    pub square: i16,
    pub triangle: i16,
    pub cross: i16,
    pub circle: i16,
    pub shock_left: i16,
    pub shock_right: i16,
    pub chat_indicated: i16,
    pub ped_walk: i16,
    pub vehicle_mouse_look: i16,
    pub radio_track_skip: i16,
}

impl PadState {
    pub const SIZE: usize = 0x30;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Self {
        let w = |i: usize| i16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
        Self {
            left_stick_x: w(0),
            left_stick_y: w(1),
            right_stick_x: w(2),
            right_stick_y: w(3),
            left_shoulder1: w(4),
            left_shoulder2: w(5),
            right_shoulder1: w(6),
            right_shoulder2: w(7),
            dpad_up: w(8),
            dpad_down: w(9),
            dpad_left: w(10),
            dpad_right: w(11),
            start: w(12),
            select: w(13),
            square: w(14),
            triangle: w(15),
            cross: w(16),
            circle: w(17),
            shock_left: w(18),
            shock_right: w(19),
            chat_indicated: w(20),
            ped_walk: w(21),
            vehicle_mouse_look: w(22),
            radio_track_skip: w(23),
        }
    }
}

/// Context flags that gate some key bits (§0.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyContext {
    pub in_vehicle: bool,
    /// Current weapon is melee (< 16), the parachute (46) or has ammo.
    pub can_fire: bool,
    /// The ped's primary task is the jetpack (Triangle is then suppressed on foot).
    pub jetpack: bool,
}

/// The u16 key word of the sync packets (§0.3).
pub fn key_word(p: &PadState, ctx: KeyContext) -> u16 {
    let on = |v: i16| v != 0;
    let mut k = 0u16;
    if on(p.left_shoulder1) {
        k |= 1;
    }
    if on(p.shock_left) {
        k |= 2;
    }
    if on(p.circle) && ctx.can_fire {
        k |= 4;
    }
    if on(p.cross) {
        k |= 8;
    }
    if on(p.triangle) && (ctx.in_vehicle || !ctx.jetpack) {
        k |= 16;
    }
    if on(p.square) {
        k |= 32;
    }
    if on(p.right_shoulder2) && ctx.in_vehicle {
        k |= 64;
    }
    if on(p.right_shoulder1) {
        k |= 128;
    }
    if on(p.left_shoulder2) && ctx.in_vehicle {
        k |= 256;
    }
    if on(p.shock_right) {
        k |= 512;
    }
    if on(p.ped_walk) {
        k |= 1024;
    }
    if p.right_stick_y < 0 {
        k |= 2048;
    }
    if p.right_stick_y > 0 {
        k |= 4096;
    }
    if p.right_stick_x < 0 {
        k |= 8192;
    }
    if p.right_stick_x > 0 {
        k |= 16384;
    }
    k
}

/// "Additional key" of the weapon byte (§0.3): DPadRight 1, DPadLeft 2, DPadDown 3.
pub fn additional_key(p: &PadState) -> u8 {
    if p.dpad_right != 0 {
        1
    } else if p.dpad_left != 0 {
        2
    } else if p.dpad_down != 0 {
        3
    } else {
        0
    }
}

/// Rotation part of a GTA `CMatrix`: rows right (+0x00), forward (+0x10), up (+0x20).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rotation {
    pub right: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

/// Matrix → (w, x, y, z), the inverse of the receiver's formula
/// (remote-control-onfoot.md §3.3), then normalised.
pub fn quat_from_matrix(m: &Rotation) -> Quat {
    let (m00, m01, m02) = (m.right[0], m.right[1], m.right[2]);
    let (m10, m11, m12) = (m.forward[0], m.forward[1], m.forward[2]);
    let (m20, m21, m22) = (m.up[0], m.up[1], m.up[2]);
    let half_sqrt = |v: f32| 0.5 * v.max(0.0).sqrt();
    let w = half_sqrt(1.0 + m00 + m11 + m22);
    let x = half_sqrt(1.0 + m00 - m11 - m22).copysign(m21 - m12);
    let y = half_sqrt(1.0 - m00 + m11 - m22).copysign(m02 - m20);
    let z = half_sqrt(1.0 - m00 - m11 + m22).copysign(m10 - m01);
    let n = (w * w + x * x + y * y + z * z).sqrt();
    if n > 0.0 { Quat { w: w / n, x: x / n, y: y / n, z: z / n } } else { Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 } }
}

/// Everything read from the game for one 207 packet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalSample {
    pub pad: PadState,
    pub key_ctx: KeyContext,
    pub position: [f32; 3],
    pub rotation: Rotation,
    pub health: f32,
    pub armour: f32,
    pub weapon: u8,
    pub special_action: u8,
    pub move_speed: [f32; 3],
    pub animation: u32,
    /// Surf offset and id (A 0x100050E8; zeros when not surfing, surf.rs).
    pub surf_offset: [f32; 3],
    pub surf_id: u16,
}

/// Float → u8 as the client does (truncate to int, keep the low byte).
fn trunc_u8(v: f32) -> u8 {
    (v as i32) as u8
}

/// Builds a 207 struct; `last_rotation` is the quaternion of the last sent
/// packet for the jitter filter.
pub fn build_foot_sync(s: &LocalSample, last_rotation: Option<Quat>) -> FootSync {
    let mut rotation = quat_from_matrix(&s.rotation);
    if let Some(old) = last_rotation {
        let close = |a: f32, b: f32| (a - b).abs() < JITTER_EPSILON;
        if close(rotation.w, old.w) && close(rotation.x, old.x) && close(rotation.y, old.y) && close(rotation.z, old.z)
        {
            rotation = old;
        }
    }
    let v3 = |a: [f32; 3]| Vec3 { x: a[0], y: a[1], z: a[2] };
    FootSync {
        left_right: s.pad.left_stick_x,
        up_down: s.pad.left_stick_y,
        keys: key_word(&s.pad, s.key_ctx),
        position: v3(s.position),
        rotation,
        health: trunc_u8(s.health),
        armour: trunc_u8(s.armour),
        weapon: WeaponByte::new(s.weapon, additional_key(&s.pad)),
        special_action: s.special_action,
        velocity: v3(s.move_speed),
        surf_offset: v3(s.surf_offset),
        surf_id: s.surf_id,
        animation: Animation::from_u32(s.animation),
    }
}

fn body_bytes(f: &FootSync) -> Vec<u8> {
    let mut w = BitWriter::new();
    f.write(&mut w);
    w.as_bytes().to_vec()
}

/// Send-rate state for 207 (§0.4).
#[derive(Debug, Default)]
pub struct FootSender {
    last_sent: Option<FootSync>,
    last_bytes: Vec<u8>,
    /// Time of the last packet actually sent (duplicate suppression).
    last_send: Option<Instant>,
    /// Time of the last tick that called the sender (interval timer).
    last_attempt: Option<Instant>,
}

impl FootSender {
    pub fn last_rotation(&self) -> Option<Quat> {
        self.last_sent.map(|f| f.rotation)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the tick should build a packet now: keys/sticks changed since
    /// the last sent struct, or more than `interval` since the last attempt.
    pub fn due(&self, now: Instant, interval: Duration, keys: u16, lr: i16, ud: i16) -> bool {
        let changed = self.last_sent.is_none_or(|l| l.keys != keys || l.left_right != lr || l.up_down != ud);
        changed || self.last_attempt.is_none_or(|t| now.duration_since(t) > interval)
    }

    /// A foot / aim packet was sent less than [`DUPLICATE_WINDOW`] ago (the shared timestamp of §0.4).
    pub fn recent_send(&self, now: Instant) -> bool {
        self.last_send.is_some_and(|t| now.duration_since(t) < DUPLICATE_WINDOW)
    }

    /// Another packet type sharing the timestamp (203) was sent.
    pub fn touch(&mut self, now: Instant) {
        self.last_send = Some(now);
    }

    /// Returns the packet to send, or None when it duplicates the last one
    /// within [`DUPLICATE_WINDOW`].
    pub fn offer(&mut self, now: Instant, f: FootSync) -> Option<FootSync> {
        self.last_attempt = Some(now);
        let bytes = body_bytes(&f);
        if self.last_send.is_some_and(|t| now.duration_since(t) < DUPLICATE_WINDOW) && bytes == self.last_bytes {
            return None;
        }
        self.last_send = Some(now);
        self.last_bytes = bytes;
        self.last_sent = Some(f);
        Some(f)
    }
}

/// On-foot interval: `onFootRate + N` ms, N = streamed-in remote player peds (§0.4).
pub fn on_foot_interval(on_foot_rate_ms: u32, streamed_peds: u32) -> Duration {
    Duration::from_millis(u64::from(on_foot_rate_ms) + u64::from(streamed_peds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading_rot(h: f32) -> Rotation {
        Rotation { right: [h.cos(), h.sin(), 0.0], forward: [-h.sin(), h.cos(), 0.0], up: [0.0, 0.0, 1.0] }
    }

    /// Receiver formula from remote-control-onfoot.md §3.3.
    fn matrix_from_quat(q: Quat) -> Rotation {
        let Quat { w, x, y, z } = q;
        Rotation {
            right: [w * w + x * x - y * y - z * z, 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
            forward: [2.0 * (x * y + w * z), w * w - x * x + y * y - z * z, 2.0 * (y * z - w * x)],
            up: [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), w * w - x * x - y * y + z * z],
        }
    }

    #[test]
    fn heading_quaternion_convention() {
        for h in [0.0f32, 0.5, 1.5, 3.0, -2.0] {
            let q = quat_from_matrix(&heading_rot(h));
            assert!((q.w - (h / 2.0).cos().abs()).abs() < 1e-5, "w for {h}");
            // q and -q are the same rotation; compare with w >= 0.
            let s = if (h / 2.0).cos() < 0.0 { -1.0 } else { 1.0 };
            assert!((q.z - s * -(h / 2.0).sin()).abs() < 1e-5, "z for {h}: {q:?}");
            assert!(q.x.abs() < 1e-6 && q.y.abs() < 1e-6);
        }
    }

    #[test]
    fn quaternion_round_trips_through_receiver_formula() {
        let q0 = Quat { w: 0.7, x: 0.1, y: -0.3, z: 0.2 };
        let n = (q0.w * q0.w + q0.x * q0.x + q0.y * q0.y + q0.z * q0.z).sqrt();
        let q0 = Quat { w: q0.w / n, x: q0.x / n, y: q0.y / n, z: q0.z / n };
        let q = quat_from_matrix(&matrix_from_quat(q0));
        for (a, b) in [(q.w, q0.w), (q.x, q0.x), (q.y, q0.y), (q.z, q0.z)] {
            assert!((a - b).abs() < 1e-5, "{q:?} vs {q0:?}");
        }
    }

    #[test]
    fn key_word_bits() {
        let p = PadState {
            left_shoulder1: 255,
            circle: 255,
            cross: 255,
            square: 255,
            right_shoulder1: 255,
            right_shoulder2: 255,
            left_shoulder2: 255,
            ped_walk: 255,
            right_stick_x: 128,
            right_stick_y: -128,
            ..PadState::default()
        };
        let on_foot = KeyContext { in_vehicle: false, can_fire: true, jetpack: false };
        assert_eq!(key_word(&p, on_foot), 1 | 4 | 8 | 32 | 128 | 1024 | 2048 | 16384);
        let no_ammo = KeyContext { can_fire: false, ..on_foot };
        assert_eq!(key_word(&p, no_ammo) & 4, 0);
        let car = KeyContext { in_vehicle: true, ..on_foot };
        assert_eq!(key_word(&p, car) & (64 | 256), 64 | 256);
        let tri = PadState { triangle: 255, ..PadState::default() };
        assert_eq!(key_word(&tri, on_foot), 16);
        assert_eq!(key_word(&tri, KeyContext { jetpack: true, ..on_foot }), 0);
    }

    #[test]
    fn pad_state_parsing_and_additional_key() {
        let mut b = [0u8; PadState::SIZE];
        b[2..4].copy_from_slice(&(-128i16).to_le_bytes()); // left stick Y
        b[0x2A..0x2C].copy_from_slice(&255i16.to_le_bytes()); // ped walk
        b[0x14..0x16].copy_from_slice(&255i16.to_le_bytes()); // dpad left
        let p = PadState::from_bytes(&b);
        assert_eq!(p.left_stick_y, -128);
        assert_eq!(p.ped_walk, 255);
        assert_eq!(additional_key(&p), 2);
    }

    #[test]
    fn foot_sync_fields_and_jitter_filter() {
        let s = LocalSample {
            pad: PadState { left_stick_y: -128, ..PadState::default() },
            key_ctx: KeyContext::default(),
            position: [1.0, 2.0, 3.0],
            rotation: heading_rot(1.0),
            health: 99.9,
            armour: 300.0,
            weapon: 24,
            move_speed: [0.1, 0.0, 0.0],
            ..LocalSample::default()
        };
        let f = build_foot_sync(&s, None);
        assert_eq!((f.up_down, f.health, f.armour, f.weapon.weapon()), (-128, 99, 44, 24));
        let old = Quat { w: f.rotation.w + 5e-6, ..f.rotation };
        assert_eq!(build_foot_sync(&s, Some(old)).rotation, old, "within jitter epsilon");
        let far = Quat { w: f.rotation.w + 1e-3, ..f.rotation };
        assert_eq!(build_foot_sync(&s, Some(far)).rotation, f.rotation);
    }

    #[test]
    fn sender_interval_and_duplicates() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let iv = on_foot_interval(30, 0);
        let mut s = FootSender::default();
        let f = FootSync::default();
        assert!(s.due(t0, iv, 0, 0, 0));
        assert!(s.offer(t0, f).is_some());
        assert!(!s.due(at(20), iv, 0, 0, 0));
        assert!(s.due(at(20), iv, 8, 0, 0), "key change");
        assert!(s.due(at(31), iv, 0, 0, 0), "interval");
        assert!(s.offer(at(31), f).is_none(), "duplicate within 501 ms");
        let moved = FootSync { position: Vec3 { x: 1.0, ..Vec3::default() }, ..f };
        assert!(s.offer(at(62), moved).is_some());
        assert!(s.offer(at(500), moved).is_none());
        assert!(s.offer(at(563), moved).is_some(), "501 ms after the last send");
    }
}
