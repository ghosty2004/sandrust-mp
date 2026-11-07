//! Trailers: attach / detach (RPC 148 / 149), the trailer sync apply (210 received) and the 210 the local driver
//! sends while towing.

use super::*;

/// DETACH_TRAILER_FROM_CAB with the "samp is detaching" flag set, so the `BreakTowLink` hook lets it through.
pub(super) fn detach_trailer_from_cab(runner: &mut Runner, trailer_handle: i32, cab_handle: i32) {
    vehhooks::with_shared(|s| s.detaching = true);
    runner.run(op::DETACH_TRAILER_FROM_CAB, &[Arg::Int(trailer_handle), Arg::Int(cab_handle)]);
    vehhooks::with_shared(|s| s.detaching = false);
}

impl Vehicles {
    /// RPC 148 AttachTrailerToVehicle.
    pub(super) fn attach_trailer(&mut self, runner: &mut Runner, trailer_id: u16, cab_id: u16) {
        let (Some(t), Some(c)) = (self.get(trailer_id), self.get(cab_id)) else { return };
        let (th, ch) = (t.handle, c.handle);
        if let Some(c) = self.pool.get_mut(&cab_id) {
            c.trailer = Some(trailer_id);
        }
        runner.run(op::ATTACH_TRAILER_TO_CAB, &[Arg::Int(th), Arg::Int(ch)]);
    }

    pub(super) fn detach_trailer(&mut self, runner: &mut Runner, cab: u16) {
        let Some(c) = self.get(cab) else { return };
        let ch = c.handle;
        if let Some(t) = c.trailer.and_then(|t| self.get(t)) {
            detach_trailer_from_cab(runner, t.handle, ch);
        }
        if let Some(c) = self.pool.get_mut(&cab) {
            c.trailer = None;
        }
    }

    /// Trailer sync (210) apply, immediately on receive: attach to the sender's cab, then nudge or teleport.
    /// Mirrors samp.dll 0x10015C90.
    pub(super) fn apply_trailer(&mut self, runner: &mut Runner, pid: u16, s: &TrailerSync) {
        let Some(cab_id) = self.remote.get(&pid).and_then(|r| r.driver).map(|d| d.vehicle_id) else { return };
        let (Some(cab), Some(t)) = (self.get(cab_id).cloned(), self.get(s.trailer_id).cloned()) else { return };
        if s.trailer_id == 0 || !vm::is_towable_list(t.model) {
            // TODO: samp has a second towable test that is not specified yet; only the model list is checked.
            return;
        }
        if rd::<u32>(t.ptr + g::VEH_TRACTOR) != cab.ptr {
            runner.run(op::ATTACH_TRAILER_TO_CAB, &[Arg::Int(t.handle), Arg::Int(cab.handle)]);
        }
        if let Some(c) = self.pool.get_mut(&cab_id) {
            c.trailer = Some(s.trailer_id);
        }
        let m = matrix(t.ptr);
        if m == 0 {
            return;
        }
        let cur = v3(m + off::MATRIX_POS);
        let target = vm::vec3(s.position);
        let d = [0, 1, 2].map(|i| (target[i] - cur[i]).abs());
        let rows = rows_from_quat(normalise(s.rotation));
        if d.iter().all(|&x| x <= 0.5) {
            return;
        }
        if d[0] <= 6.0 && d[1] <= 6.0 && d[2] <= 3.0 {
            write_rows(m, &rows);
            put_v3(t.ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            let mut vel = v3(t.ptr + off::MOVE_SPEED);
            for i in 0..3 {
                if d[i] > 0.05 {
                    vel[i] += (target[i] - cur[i]) * 0.025;
                }
            }
            put_v3(t.ptr + off::MOVE_SPEED, vel);
        } else {
            write_rows(m, &rows);
            put_v3(m + off::MATRIX_POS, target);
            put_v3(t.ptr + off::MOVE_SPEED, vm::vec3(s.velocity));
            put_v3(t.ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
        }
    }

    /// 210 while towing: sent on every in-car tick, even when the 200 was suppressed as a duplicate, and it
    /// refreshes the shared send timestamp.
    pub(super) fn send_trailer(&mut self, net: &mut NetClient, trailer_id: u16, now: Instant) {
        if trailer_id == 0 {
            return;
        }
        let Some(t) = self.get(trailer_id) else { return };
        let tm = matrix(t.ptr);
        if tm == 0 {
            return;
        }
        let ts = TrailerSync {
            trailer_id,
            position: vm::to_vec3(v3(tm + off::MATRIX_POS)),
            rotation: quat_from_matrix(&rows_of(tm)),
            velocity: vm::to_vec3(v3(t.ptr + off::MOVE_SPEED)),
            turn_velocity: vm::to_vec3(v3(t.ptr + g::TURN_SPEED)),
        };
        self.dup.touch(now);
        net.send_sync(&ts);
        self.n.sent_210 += 1;
    }
}
