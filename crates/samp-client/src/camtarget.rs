//! Camera target reporting: RPC 170 turns it on, then the client sends RPC 168 with the object, vehicle, player
//! and actor ids under the camera ray whenever that set changes; the `/ctd` command labels the targets on screen.
//! `CamTarget::tick` holds the enable flag, the rate limit and the change detection; `ray` and `nearest` are the
//! geometry helpers. The line-of-sight tests and pool searches (each category's ray is first shortened by a world
//! line-of-sight test, then the nearest entity whose bounding sphere it hits wins) run in game code.
//! Spec: docs/re/subsystems/player-state.md, "170" and "168"; docs/phase3/feata-notes.md.

/// "No target" id.
pub const NONE: u16 = 0xFFFF;
/// Minimum time between two computations; a computation needs strictly more than this.
pub const INTERVAL_MS: u32 = 100;
/// Ray length along the camera front.
pub const RAY_LENGTH: f32 = 70.0;
/// Search radii (objects: start distance of the pool test; others: distance to the camera position).
pub const OBJECT_RADIUS: f32 = 40.0;
pub const VEHICLE_RADIUS: f32 = 40.0;
pub const PED_RADIUS: f32 = 20.0;
/// Bounding-sphere scales passed to samp's sphere/segment line test.
pub const VEHICLE_SCALE: f32 = 1.0;
pub const PED_SCALE: f32 = 1.5;
/// `/ctd` label colour (ARGB).
pub const LABEL_COLOUR: u32 = 0xFFD4_7B75;

/// Label text per category (0 object, 1 vehicle, 2 player, 3 actor), as samp formats it.
pub fn label(kind: usize, id: u16) -> String {
    let name = ["OBJECT", "VEHICLE", "PLAYER", "ACTOR"][kind.min(3)];
    format!("CAMERA_TARGET_TYPE_{name}: ID({id})")
}

/// The sender's state: enable flag, last ids sent and the time of the last computation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CamTarget {
    pub enabled: bool,
    /// Last ids sent, in the order object, vehicle, player, actor.
    pub cached: [u16; 4],
    pub last_ms: u32,
}

impl Default for CamTarget {
    fn default() -> Self {
        Self { enabled: false, cached: [NONE; 4], last_ms: 0 }
    }
}

impl CamTarget {
    /// One local tick. While disabled the cache is reset to [`NONE`]. Otherwise, when more than [`INTERVAL_MS`]
    /// has passed, `compute` is called and its result returned (and cached) if it differs from the cache.
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

/// Nearest candidate to `start` among the hits; only a strictly nearer hit replaces the current best, and hits
/// 100000 units or farther away are ignored, as in samp.
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
