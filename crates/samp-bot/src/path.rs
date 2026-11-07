//! Scripted drive paths for the driver mode: a speed profile (accelerate to a
//! cruise speed, brake to a stop) integrated along a path shape.

use std::f32::consts::FRAC_PI_2;

use samp_proto::types::Vec3;

use crate::geom::{self, add, forward, right, scale};

/// Path shape, starting at the vehicle's position and heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Endless left-hand circle of this radius.
    Circle { radius: f32 },
    /// `straight` metres ahead, a 90° left turn of radius `turn_radius`, then
    /// straight on.
    LineTurn { straight: f32, turn_radius: f32 },
}

/// Pose on a path at arc length `s`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: Vec3,
    pub heading: f32,
    /// Signed curvature (1/m, positive = turning left).
    pub curvature: f32,
}

impl Shape {
    pub fn pose(&self, start: Vec3, h0: f32, s: f32) -> Pose {
        match *self {
            Shape::Circle { radius } => arc(start, h0, radius, s),
            Shape::LineTurn { straight, turn_radius } => {
                if s <= straight {
                    return Pose { pos: add(start, scale(forward(h0), s)), heading: h0, curvature: 0.0 };
                }
                let p1 = add(start, scale(forward(h0), straight));
                let turn_len = turn_radius * FRAC_PI_2;
                if s <= straight + turn_len {
                    return arc(p1, h0, turn_radius, s - straight);
                }
                let end = arc(p1, h0, turn_radius, turn_len);
                Pose {
                    pos: add(end.pos, scale(forward(end.heading), s - straight - turn_len)),
                    heading: end.heading,
                    curvature: 0.0,
                }
            }
        }
    }
}

/// Left-hand arc from `start` (heading `h0`) of radius `r`, arc length `s`.
fn arc(start: Vec3, h0: f32, r: f32, s: f32) -> Pose {
    let centre = add(start, scale(right(h0), -r));
    let h = h0 + s / r;
    Pose { pos: add(centre, scale(right(h), r)), heading: h, curvature: 1.0 / r }
}

/// Speed profile + integrated arc length.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub shape: Shape,
    pub start: Vec3,
    pub h0: f32,
    /// Cruise speed (m/s).
    pub cruise: f32,
    pub accel: f32,
    pub brake: f32,
    pub s: f32,
    pub v: f32,
    pub braking: bool,
}

/// What the driver's pad does at the current moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pedal {
    Accelerate,
    Brake,
    None,
}

impl Motion {
    pub fn new(shape: Shape, start: Vec3, h0: f32, cruise: f32) -> Self {
        Self { shape, start, h0, cruise, accel: 5.0, brake: 8.0, s: 0.0, v: 0.0, braking: false }
    }

    /// Advance by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let v0 = self.v;
        self.v = if self.braking { (v0 - self.brake * dt).max(0.0) } else { (v0 + self.accel * dt).min(self.cruise) };
        self.s += (v0 + self.v) * 0.5 * dt;
    }

    pub fn pose(&self) -> Pose {
        self.shape.pose(self.start, self.h0, self.s)
    }

    /// World velocity (m/s).
    pub fn velocity(&self) -> Vec3 {
        scale(forward(self.pose().heading), self.v)
    }

    pub fn stopped(&self) -> bool {
        self.v <= 0.0
    }

    pub fn pedal(&self) -> Pedal {
        if self.braking {
            if self.stopped() { Pedal::None } else { Pedal::Brake }
        } else {
            Pedal::Accelerate
        }
    }

    /// Steering stick: full left lock (−128) for radius ≤ 5 m, proportional above.
    pub fn steer(&self) -> i16 {
        let k = self.pose().curvature;
        if k == 0.0 || self.stopped() { 0 } else { (-128.0 * (k * 5.0).min(1.0)).round() as i16 }
    }

    pub fn distance_from_start(&self) -> f32 {
        geom::dist(self.pose().pos, self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::v3;

    #[test]
    fn circle_starts_at_start_and_turns_left() {
        let start = v3(10.0, 20.0, 5.0);
        let s = Shape::Circle { radius: 10.0 };
        let p0 = s.pose(start, 0.0, 0.0);
        assert!(geom::dist(p0.pos, start) < 1e-4);
        // Quarter circle: heading +90° (facing -x), displaced (-10, +10).
        let q = s.pose(start, 0.0, 10.0 * FRAC_PI_2);
        assert!((q.heading - FRAC_PI_2).abs() < 1e-5);
        assert!(geom::dist(q.pos, v3(0.0, 30.0, 5.0)) < 1e-3, "{:?}", q.pos);
    }

    #[test]
    fn line_turn_is_continuous() {
        let s = Shape::LineTurn { straight: 50.0, turn_radius: 20.0 };
        let a = s.pose(Vec3::default(), 0.0, 50.0);
        assert!(geom::dist(a.pos, v3(0.0, 50.0, 0.0)) < 1e-4);
        let end = 50.0 + 20.0 * FRAC_PI_2;
        let b = s.pose(Vec3::default(), 0.0, end);
        assert!(geom::dist(b.pos, v3(-20.0, 70.0, 0.0)) < 1e-3, "{:?}", b.pos);
        let c = s.pose(Vec3::default(), 0.0, end + 10.0);
        assert!(geom::dist(c.pos, v3(-30.0, 70.0, 0.0)) < 1e-3, "{:?}", c.pos);
    }

    #[test]
    fn accelerate_cruise_brake() {
        let mut m = Motion::new(Shape::Circle { radius: 10.0 }, Vec3::default(), 0.0, 10.0);
        for _ in 0..100 {
            m.step(0.04);
        }
        assert_eq!(m.v, 10.0);
        assert_eq!(m.pedal(), Pedal::Accelerate);
        m.braking = true;
        m.step(0.04);
        assert_eq!(m.pedal(), Pedal::Brake);
        for _ in 0..50 {
            m.step(0.04);
        }
        assert!(m.stopped());
        assert_eq!(m.pedal(), Pedal::None);
        assert_eq!(m.steer(), 0);
    }
}
