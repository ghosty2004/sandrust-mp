//! Client-side send filters shared by the sync senders
//! (docs/re/subsystems/remote-players-sync.md §0.4, §207, §200).

use std::time::{Duration, Instant};

use samp_proto::sync::{ids, policy};
use samp_proto::types::Quat;

/// Packet types covered by duplicate suppression.
const GATED: [u8; 5] = [ids::FOOT_SYNC, ids::AIM_SYNC, ids::VEHICLE_SYNC, ids::PASSENGER_SYNC, ids::TRAILER_SYNC];

/// Duplicate suppression: one timestamp shared by foot/aim/driver/passenger/
/// trailer sync, one "last sent" copy per type. A struct byte-identical to the
/// last one of its type is not sent while fewer than 501 ms have passed since
/// the last send of *any* of the five types.
#[derive(Debug, Default)]
pub struct DuplicateGate {
    last_send: Option<Instant>,
    last: [Vec<u8>; 5],
}

impl DuplicateGate {
    /// Decide whether `packet` (complete, id byte first) may be sent now; if
    /// so, record it. Types outside the gated set always pass.
    pub fn admit(&mut self, packet: &[u8], now: Instant) -> bool {
        let Some(slot) = packet.first().and_then(|id| GATED.iter().position(|g| g == id)) else { return true };
        let window = Duration::from_millis(policy::DUPLICATE_SUPPRESSION_MS.into());
        if self.last_send.is_some_and(|t| now.duration_since(t) < window) && self.last[slot] == packet {
            return false;
        }
        self.last_send = Some(now);
        self.last[slot] = packet.to_vec();
        true
    }
}

/// Rotation jitter filter of 207/200: the previously sent quaternion is
/// reused if every component of the new one is within 1e-5 of it.
#[derive(Debug, Default)]
pub struct RotationFilter {
    last: Option<Quat>,
}

impl RotationFilter {
    pub fn filter(&mut self, q: Quat) -> Quat {
        let eps = policy::ROTATION_JITTER_EPSILON;
        if let Some(l) = self.last
            && (l.w - q.w).abs() < eps
            && (l.x - q.x).abs() < eps
            && (l.y - q.y).abs() < eps
            && (l.z - q.z).abs() < eps
        {
            return l;
        }
        self.last = Some(q);
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_struct_suppressed_for_501_ms() {
        let t0 = Instant::now();
        let mut g = DuplicateGate::default();
        let p = [207u8, 1, 2, 3];
        assert!(g.admit(&p, t0));
        assert!(!g.admit(&p, t0 + Duration::from_millis(500)));
        assert!(g.admit(&p, t0 + Duration::from_millis(501)));
        // A different struct passes at once.
        assert!(g.admit(&[207, 9], t0 + Duration::from_millis(502)));
    }

    #[test]
    fn timestamp_is_shared_between_types() {
        let t0 = Instant::now();
        let mut g = DuplicateGate::default();
        let foot = [207u8, 1];
        assert!(g.admit(&foot, t0));
        // An aim send at +400 renews the shared timestamp ...
        assert!(g.admit(&[203u8, 5], t0 + Duration::from_millis(400)));
        // ... so the identical foot struct is still suppressed at +600.
        assert!(!g.admit(&foot, t0 + Duration::from_millis(600)));
        assert!(g.admit(&foot, t0 + Duration::from_millis(902)));
    }

    #[test]
    fn ungated_types_always_pass() {
        let t0 = Instant::now();
        let mut g = DuplicateGate::default();
        assert!(g.admit(&[205u8, 0], t0));
        assert!(g.admit(&[205u8, 0], t0));
        assert!(g.admit(&[206u8, 0], t0));
    }

    #[test]
    fn rotation_jitter_reuses_last() {
        let mut f = RotationFilter::default();
        let a = Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };
        assert_eq!(f.filter(a), a);
        let b = Quat { w: 1.0 - 5e-6, z: 5e-6, ..a };
        assert_eq!(f.filter(b), a);
        let c = Quat { w: 0.9, z: -0.1, ..a };
        assert_eq!(f.filter(c), c);
    }
}
