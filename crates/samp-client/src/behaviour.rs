//! Remaining local behaviour (M8b), platform-independent rules: world bounds, drunk handling, camera RPC
//! arguments, spectating targets, ClientCheck replies.
//!
//! Spec: player-state.md (17, 35, 82, 158, 81, 33, 90), remote-players-sync.md §1 (124/126/127) and §2 "212",
//! session-misc.md "103", and the measurements in docs/phase3/trace-notes.md "Remaining behaviour (TRACE-8)".
//! Static details are cited by samp.dll build B address where they were read.

/// World bounds (Settings +0x05/+0x09/+0x0D/+0x11, RPC 17); ±20000 = no bounds (InitGame, B 0x100088E0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub max_x: f32,
    pub min_x: f32,
    pub max_y: f32,
    pub min_y: f32,
}

impl Default for Bounds {
    fn default() -> Self {
        Self { max_x: 20000.0, min_x: -20000.0, max_y: 20000.0, min_y: -20000.0 }
    }
}

/// Text, time and style of the out-of-bounds message (B 0x10003F9D: `CMessages::AddBigMessage`).
pub const BOUNDS_TEXT: &str = "Stay within the ~r~world boundries";
pub const BOUNDS_TEXT_MS: u32 = 1000;
pub const BOUNDS_TEXT_STYLE: u32 = 5;

/// Push-back of an entity outside the bounds (B 0x1009F450): the new move speed and the new matrix z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushBack {
    pub speed: [f32; 3],
    pub z: f32,
}

/// B 0x1009F450: strict compares in the order x > maxX, x < minX, y > maxY, y < minY. A non-zero speed on that
/// axis becomes ∓0.2 towards the inside and speed z 0.1; the matrix is re-set with z + 0.04 in every case.
pub fn push_back(pos: [f32; 3], speed: [f32; 3], b: &Bounds) -> Option<PushBack> {
    let mut s = speed;
    let (axis, sign) = if pos[0] > b.max_x {
        (0, -1.0)
    } else if pos[0] < b.min_x {
        (0, 1.0)
    } else if pos[1] > b.max_y {
        (1, -1.0)
    } else if pos[1] < b.min_y {
        (1, 1.0)
    } else {
        return None;
    };
    if s[axis] != 0.0 {
        s[axis] = 0.2 * sign;
        s[2] = 0.1;
    }
    Some(PushBack { speed: s, z: pos[2] + 0.04 })
}

/// Drunk level above which the effects run (B 0x100ADE06).
pub const DRUNK_THRESHOLD: i32 = 2000;
/// The wobble is applied at most every this many ms (B 0x100ADEC9: elapsed > 200).
pub const DRUNK_WOBBLE_MS: u32 = 200;

/// `SET_PLAYER_DRUNKENNESS` value for a level above the threshold (B 0x100ADE31): `ftol(level · 0.02)` in single
/// precision (GTA's D3D device leaves the x87 at 24-bit precision: 5000 → 100, 2100 → 42, both measured), > 250 →
/// 250, < 5 → 0.
pub fn drunk_value(level: i32) -> i32 {
    let v = crate::anims::ftol(level as f32 * 0.02f32);
    if v > 250 {
        250
    } else if v < 5 {
        0
    } else {
        v
    }
}

/// Steering wobble added to the vehicle's turn speed z (B 0x100ADED4): `rand() % 40` ≥ 20 → > 30 positive, else
/// negative; 0.012, or 0.015 from level 5000 on; below 20 nothing.
pub fn drunk_wobble(rand: i32, level: i32) -> f32 {
    let r = rand % 40;
    if r < 20 {
        return 0.0;
    }
    let m = if level >= 5000 { 0.015 } else { 0.012 };
    if r > 30 { m } else { -m }
}

/// The wobble applies only while the vehicle moves faster than 0.05 on x or y (B 0x100ADF23).
pub fn drunk_moving(speed: [f32; 3]) -> bool {
    speed[0].abs() > 0.05 || speed[1].abs() > 0.05
}

/// MSVC CRT `rand()` (samp's static CRT, B 0x100CB096): per-thread seed, initially 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsvcRand(pub u32);

impl Default for MsvcRand {
    fn default() -> Self {
        Self(1)
    }
}

impl MsvcRand {
    pub fn next_rand(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(0x343FD).wrapping_add(0x269EC3);
        ((self.0 >> 16) & 0x7FFF) as i32
    }
}

/// Cut argument of RPC 158 / 82: 0 or > 2 becomes 2 (B 0x1001A18C).
pub fn camera_cut(cut: u8) -> u8 {
    if cut == 0 || cut > 2 { 2 } else { cut }
}

/// samp's coordinate validator (B 0x100B4B60): −20000 < x, y < 20000 and −10000 < z < 100000.
pub fn valid_position(p: [f32; 3]) -> bool {
    p[0] > -20000.0 && p[0] < 20000.0 && p[1] > -20000.0 && p[1] < 20000.0 && p[2] > -10000.0 && p[2] < 100000.0
}

/// The spectated target (LocalPlayer +0x30F type, +0x310 id, +0x314 applied, +0x30E camera mode).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpecTarget {
    #[default]
    None,
    Player(u16),
    Vehicle(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spectate {
    pub on: bool,
    pub target: SpecTarget,
    pub applied: bool,
    /// GTA camera mode for the target (4/14/15 for players, 3/14/15 for vehicles).
    pub mode: u8,
}

/// Camera of the "no target" spectator view (B 0x1000674E): fixed at (50, 50, 50) looking at (60, 60, 50), jump cut.
pub const SPEC_NO_TARGET_POS: [f32; 3] = [50.0, 50.0, 50.0];
pub const SPEC_NO_TARGET_LOOK: [f32; 3] = [60.0, 60.0, 50.0];
/// The local ped is parked this far above the camera every spectator tick (B 0x1000669C).
pub const SPEC_PED_ABOVE: f32 = 20.0;
/// Its health is forced to this every spectator tick (B 0x10006688).
pub const SPEC_PED_HEALTH: f32 = 100.0;
/// Out of a vehicle first (B 0x1000674E → 0x100AC580): WARP_CHAR_FROM_CAR_TO_COORD to (0, 0, 10).
pub const SPEC_WARP_OUT: [f32; 3] = [0.0, 0.0, 10.0];

/// ClientCheck (RPC 103, session-misc.md): requests outside these limits get no reply (B 0x10011AE3).
pub fn client_check_in_limits(offset: u16, count: u16) -> bool {
    offset <= 0x100 && (2..=0x100).contains(&count)
}

/// Type 0x48 reply argument (B 0x10011DC9): the QueryPerformanceCounter clock in ms (B 0x1002E7A0), low 28 bits,
/// with 0x30000000 set.
pub fn client_check_time_arg(ms: u64) -> u32 {
    (ms as u32 & 0x0FFF_FFFF) | 0x3000_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_back_cases() {
        let b = Bounds { max_x: 10.0, min_x: -10.0, max_y: 10.0, min_y: -10.0 };
        assert_eq!(push_back([0.0, 0.0, 5.0], [1.0, 1.0, 0.0], &b), None);
        assert_eq!(push_back([10.0, 0.0, 5.0], [1.0, 1.0, 0.0], &b), None, "strict compare");
        let p = push_back([11.0, 20.0, 5.0], [0.3, 0.5, -0.1], &b).unwrap();
        assert_eq!(p.speed, [-0.2, 0.5, 0.1], "x is tested first");
        assert_eq!(p.z, 5.04);
        let p = push_back([-11.0, 0.0, 5.0], [0.0, 0.5, -0.1], &b).unwrap();
        assert_eq!(p.speed, [0.0, 0.5, -0.1], "zero speed on the axis: unchanged");
        let p = push_back([0.0, -11.0, 1.0], [0.0, -0.5, 0.0], &b).unwrap();
        assert_eq!(p.speed, [0.0, 0.2, 0.1]);
        let p = push_back([0.0, 11.0, 1.0], [0.0, 0.5, 0.0], &b).unwrap();
        assert_eq!(p.speed, [0.0, -0.2, 0.1]);
    }

    #[test]
    fn drunk_values_match_the_measurements() {
        assert_eq!(drunk_value(5000), 100);
        assert_eq!(drunk_value(2100), 42);
        assert_eq!(drunk_value(4999), 99);
        assert_eq!(drunk_value(50000), 250);
        assert_eq!(drunk_wobble(19, 3000), 0.0);
        assert_eq!(drunk_wobble(31, 3000), 0.012);
        assert_eq!(drunk_wobble(30, 3000), -0.012);
        assert_eq!(drunk_wobble(71, 5000), 0.015);
        assert!(drunk_moving([0.06, 0.0, 0.0]));
        assert!(!drunk_moving([0.05, -0.05, 1.0]));
    }

    #[test]
    fn msvc_rand_sequence() {
        let mut r = MsvcRand::default();
        assert_eq!([r.next_rand(), r.next_rand(), r.next_rand()], [41, 18467, 6334]);
    }

    #[test]
    fn camera_rules() {
        assert_eq!([0, 1, 2, 3].map(camera_cut), [2, 1, 2, 2]);
        assert!(valid_position([0.0, 0.0, 0.0]));
        assert!(!valid_position([20000.0, 0.0, 0.0]));
        assert!(!valid_position([0.0, 0.0, -10000.0]));
    }

    #[test]
    fn client_check_rules() {
        assert!(client_check_in_limits(0, 4));
        assert!(!client_check_in_limits(257, 4));
        assert!(!client_check_in_limits(0, 300));
        assert!(!client_check_in_limits(0, 1));
        assert_eq!(client_check_time_arg(0x1234_5678_9ABC), 0x3678_9ABC);
    }
}
