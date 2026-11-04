//! Small vector helpers and the GTA heading conventions.
//!
//! Heading `h` (radians, counter-clockwise from +y) faces `(-sin h, cos h)`.
//! Heading ↔ sync quaternion: `h ↔ (cos h/2, 0, 0, −sin h/2)` (verified live,
//! docs/re/subsystems/remote-control-onfoot.md).

use samp_proto::types::{Quat, Vec3};

pub fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    v3(a.x + b.x, a.y + b.y, a.z + b.z)
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    v3(a.x - b.x, a.y - b.y, a.z - b.z)
}

pub fn scale(a: Vec3, k: f32) -> Vec3 {
    v3(a.x * k, a.y * k, a.z * k)
}

pub fn len(a: Vec3) -> f32 {
    (a.x * a.x + a.y * a.y + a.z * a.z).sqrt()
}

pub fn dist(a: Vec3, b: Vec3) -> f32 {
    len(sub(a, b))
}

pub fn dist2d(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// `a / |a|`, or zero for a (near) zero vector.
pub fn normalize(a: Vec3) -> Vec3 {
    let l = len(a);
    if l < 1e-6 { Vec3::default() } else { scale(a, 1.0 / l) }
}

/// Unit forward vector (horizontal) of heading `h`.
pub fn forward(h: f32) -> Vec3 {
    v3(-h.sin(), h.cos(), 0.0)
}

/// Unit right vector (horizontal) of heading `h`.
pub fn right(h: f32) -> Vec3 {
    v3(h.cos(), h.sin(), 0.0)
}

/// Heading that faces along `d` (horizontal part).
pub fn heading_of(d: Vec3) -> f32 {
    (-d.x).atan2(d.y)
}

pub fn heading_to_quat(h: f32) -> Quat {
    Quat { w: (h / 2.0).cos(), x: 0.0, y: 0.0, z: -(h / 2.0).sin() }
}

/// Inverse of [`heading_to_quat`] (ignores pitch/roll).
pub fn quat_to_heading(q: Quat) -> f32 {
    -2.0 * q.z.atan2(q.w)
}

/// `p` expressed in the frame of an entity at `origin` with heading `h`
/// (x = right, y = forward, z = up).
pub fn to_local(p: Vec3, origin: Vec3, h: f32) -> Vec3 {
    let d = sub(p, origin);
    let (r, f) = (right(h), forward(h));
    v3(d.x * r.x + d.y * r.y, d.x * f.x + d.y * f.y, d.z)
}

/// GTA move speed (units per 1/50 s) for a world velocity in m/s.
pub fn move_speed(v: Vec3) -> Vec3 {
    scale(v, 1.0 / 50.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn heading_roundtrips() {
        for h in [0.0f32, 0.5, FRAC_PI_2, 2.5, -1.0] {
            let q = heading_to_quat(h);
            let back = quat_to_heading(q);
            assert!((back.sin() - h.sin()).abs() < 1e-5 && (back.cos() - h.cos()).abs() < 1e-5);
            assert!((heading_of(forward(h)) - back).abs() < 1e-4 || (heading_of(forward(h)) - h).abs() < 1e-4);
        }
    }

    #[test]
    fn local_frame() {
        // Facing -x (h = 90°): a point 1 m further along -x is 1 m forward.
        let l = to_local(v3(-1.0, 0.0, 0.5), Vec3::default(), FRAC_PI_2);
        assert!((l.y - 1.0).abs() < 1e-5 && l.x.abs() < 1e-5 && (l.z - 0.5).abs() < 1e-6);
    }
}
