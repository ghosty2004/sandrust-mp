//! Camera target reporting (RPC 170 → RPC 168) and the `/ctd` labels, samp B 0x100046E0 / 0x10004420 /
//! 0x10004830 (player-state.md "170", "168"; docs/phase3/feata-notes.md "FEAT-2").
//!
//! Behaviour, in our words:
//! * RPC 170 stores an "enabled" flag (netgame +0x233).
//! * Every local tick while disabled, the four cached ids are reset to "none" and nothing is sent.
//! * While enabled, at most once per >100 ms (GetTickCount), the targets are computed; RPC 168 (four u16 in one
//!   8-byte block, reliability 9) goes out only when one of them differs from the cached set, which then becomes
//!   the cache. The timer restarts after every computation.
//! * The ray: camera front F and source S (`TheCamera.m_aCams[0]`, 0xB6F32C / 0xB6F338), from S + F to S + 70·F.
//!   For each category the end is first shortened to the hit point of a `CWorld::ProcessLineOfSight` with that
//!   category's flags (objects: buildings; vehicles: buildings + objects; players / actors: buildings + vehicles +
//!   objects), then the category's pool is searched:
//!   - objects (B 0x10012E40): the pool ray test with a 40 m start distance;
//!   - vehicles (B 0x1001EEE0): live vehicles within 40 m of the camera position, not the one the local ped sits
//!     in, whose bounding sphere (scale 1.0) the segment hits; nearest to the ray start wins;
//!   - players (B 0x10013D50) and actors (B 0x10001770): live peds within 20 m of the camera, bounding sphere scale
//!     1.5, nearest to the ray start (actors use the players' shortened ray).
//! * `/ctd` (B 0x10004830, every local tick while the flag is on): for each cached id whose entity exists, the
//!   object editor's axis cross on the entity matrix and the label `CAMERA_TARGET_TYPE_<KIND>: ID(<id>)` in
//!   0xFFD47B75 at the matrix position (3D-text painter, outline on, no LOS test), inside one sprite begin/end.

/// "No target" id.
pub const NONE: u16 = 0xFFFF;
/// Minimum time between two computations (strictly more than this, B 0x10004702 `cmp 100`).
pub const INTERVAL_MS: u32 = 100;
/// Ray length along the camera front.
pub const RAY_LENGTH: f32 = 70.0;
/// Search radii (objects: start distance of the pool test; others: distance to the camera position).
pub const OBJECT_RADIUS: f32 = 40.0;
pub const VEHICLE_RADIUS: f32 = 40.0;
pub const PED_RADIUS: f32 = 20.0;
/// Bounding-sphere scales of the line test (B 0x100B65E0 argument).
pub const VEHICLE_SCALE: f32 = 1.0;
pub const PED_SCALE: f32 = 1.5;
/// `/ctd` label colour (ARGB).
pub const LABEL_COLOUR: u32 = 0xFFD4_7B75;

/// Label text per category (B strings "CAMERA_TARGET_TYPE_*").
pub fn label(kind: usize, id: u16) -> String {
    let name = ["OBJECT", "VEHICLE", "PLAYER", "ACTOR"][kind.min(3)];
    format!("CAMERA_TARGET_TYPE_{name}: ID({id})")
}

/// The sender's state (local player +0x11F..+0x12A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CamTarget {
    pub enabled: bool,
    /// object, vehicle, player, actor.
    pub cached: [u16; 4],
    pub last_ms: u32,
}

impl Default for CamTarget {
    fn default() -> Self {
        Self { enabled: false, cached: [NONE; 4], last_ms: 0 }
    }
}

impl CamTarget {
    /// One local tick. `compute` is called when a computation is due; returns the set to send, if any.
    pub fn tick(&mut self, now_ms: u32, compute: impl FnOnce() -> [u16; 4]) -> Option<[u16; 4]> {
        if !self.enabled {
            self.cached = [NONE; 4];
            return None;
        }
        if now_ms.wrapping_sub(self.last_ms) <= INTERVAL_MS {
            return None;
        }
        let t = compute();
        let send = (t != self.cached).then(|| {
            self.cached = t;
            t
        });
        self.last_ms = now_ms;
        send
    }
}

/// Ray start and end from the camera front `f` and source `s`.
pub fn ray(f: [f32; 3], s: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let start = [s[0] + f[0], s[1] + f[1], s[2] + f[2]];
    let end = [s[0] + RAY_LENGTH * f[0], s[1] + RAY_LENGTH * f[1], s[2] + RAY_LENGTH * f[2]];
    (start, end)
}

/// Nearest candidate by distance to `start` among the hits (strictly nearer replaces; samp starts at 100000).
pub fn nearest(start: [f32; 3], hits: impl IntoIterator<Item = (u16, [f32; 3])>) -> u16 {
    let mut best = (100_000.0f32, NONE);
    for (id, p) in hits {
        let d = ((p[0] - start[0]).powi(2) + (p[1] - start[1]).powi(2) + (p[2] - start[2]).powi(2)).sqrt();
        if d < best.0 {
            best = (d, id);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_resets_and_never_sends() {
        let mut c = CamTarget { enabled: false, cached: [1, 2, 3, 4], last_ms: 0 };
        assert_eq!(c.tick(1000, || panic!("no computation while disabled")), None);
        assert_eq!(c.cached, [NONE; 4]);
    }

    #[test]
    fn sends_on_change_only_and_rate_limits() {
        let mut c = CamTarget { enabled: true, ..CamTarget::default() };
        assert_eq!(c.tick(50, || [NONE; 4]), None, "≤ 100 ms since 0");
        assert_eq!(c.tick(101, || [NONE; 4]), None, "same as the cache: nothing sent");
        assert_eq!(c.last_ms, 101);
        assert_eq!(c.tick(201, || [NONE, 5, NONE, NONE]), None, "exactly 100 ms: not due");
        assert_eq!(c.tick(202, || [NONE, 5, NONE, NONE]), Some([NONE, 5, NONE, NONE]));
        assert_eq!(c.tick(400, || [NONE, 5, NONE, NONE]), None);
        assert_eq!(c.tick(600, || [NONE; 4]), Some([NONE; 4]));
    }

    #[test]
    fn ray_and_nearest() {
        let (a, b) = ray([0.0, 1.0, 0.0], [10.0, 20.0, 5.0]);
        assert_eq!(a, [10.0, 21.0, 5.0]);
        assert_eq!(b, [10.0, 90.0, 5.0]);
        assert_eq!(nearest(a, [(3, [10.0, 30.0, 5.0]), (7, [10.0, 25.0, 5.0])]), 7);
        assert_eq!(nearest(a, []), NONE);
        assert_eq!(label(1, 5), "CAMERA_TARGET_TYPE_VEHICLE: ID(5)");
        assert_eq!(label(3, 0), "CAMERA_TARGET_TYPE_ACTOR: ID(0)");
    }
}
