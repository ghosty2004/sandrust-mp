//! MoveObject / StopObject motion.

use super::*;

/// MoveObject setup; a move in progress is finished first. Mirrors samp.dll 0x100A9C10.
pub(super) fn start_move(o: &mut Obj, target: [f32; 3], speed: f32, rot: [f32; 3], now_ms: u32) {
    if let Some(m) = o.motion.take() {
        // A move in progress stops and snaps to its old target (and target rotation).
        stop(o);
        let r = m.rotation.map(world::rotation_rows).or_else(|| rows(o.ptr).map(|(r, _)| r));
        if let Some(r) = r {
            write_matrix(o.ptr, &r, m.target);
        }
        if let Some(r) = m.rotation {
            o.rotation = r;
        }
    }
    let Some((r, pos)) = rows(o.ptr) else { return };
    let rotation = (rot.iter().all(|c| *c > world::NO_ROTATION)).then_some(rot);
    let from = quat_from_matrix(&r);
    let to = rotation.map(|t| quat_from_matrix(&world::rotation_rows(t))).unwrap_or(from);
    o.motion = Some(Motion { target, speed, start_ms: now_ms, total: world::dist3(target, pos), rotation, from, to });
}

/// StopObject: move and turn speed 0, moving flag off; the object stays where it is. Mirrors samp.dll 0x100A8070.
pub(super) fn stop(o: &mut Obj) {
    o.motion = None;
    for i in 0..3 {
        wr(o.ptr + g::MOVE_SPEED + 4 * i, 0.0f32);
        wr(o.ptr + g::TURN_SPEED + 4 * i, 0.0f32);
    }
}

pub(super) fn move_step(o: &mut Obj, m: &Motion, dt: f32, now_ms: u32) {
    let Some((r, pos)) = rows(o.ptr) else { return };
    match m.step(pos, dt, now_ms) {
        MotionStep::Arrive { pos, rotation } => {
            for i in 0..3 {
                wr(o.ptr + g::MOVE_SPEED + 4 * i, 0.0f32);
                wr(o.ptr + g::TURN_SPEED + 4 * i, 0.0f32);
            }
            let rr = rotation.unwrap_or(r);
            write_matrix(o.ptr, &rr, pos);
            if let Some(t) = m.rotation {
                o.rotation = t;
            }
            stop(o);
        }
        MotionStep::Move { move_speed, turn_z, progress } => {
            for (i, v) in move_speed.iter().enumerate() {
                wr(o.ptr + g::MOVE_SPEED + 4 * i as u32, *v);
            }
            // samp advances the matrix itself: pos += move speed × CTimer::ms_fTimeStep.
            let ts: f32 = rd(g::TIME_STEP);
            let next = [0, 1, 2].map(|i| pos[i] + move_speed[i] * ts);
            let rr = match turn_z {
                Some(tz) => {
                    wr(o.ptr + g::TURN_SPEED, 0.0f32);
                    wr(o.ptr + g::TURN_SPEED + 4, 0.0f32);
                    wr(o.ptr + g::TURN_SPEED + 8, tz);
                    rows_from_quat(normalise(slerp(m.from, m.to, progress.clamp(0.0, 1.0))))
                }
                None => r,
            };
            write_matrix(o.ptr, &rr, next);
        }
        MotionStep::Idle => {}
    }
}
