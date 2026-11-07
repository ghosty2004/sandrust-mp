//! Camera RPCs and spectating.

use super::*;

impl Behaviour {
    // ---- camera ------------------------------------------------------------------------------------------------

    /// RPC 82 (InterpolateCamera): a time ≤ 0 is ignored; the camera counts as "applied" for the spectator tick.
    /// Mirrors samp.dll B 0x1001A0A0.
    pub fn interpolate(&mut self, runner: &mut Runner, position: bool, from: [f32; 3], to: [f32; 3], ms: i32, cut: u8) {
        if ms <= 0 {
            return;
        }
        self.spectate.applied = true;
        let cut = i32::from(bh::camera_cut(cut));
        if position {
            vector_move(runner, from, to, ms, cut);
        } else {
            runner.run(op::CAMERA_PERSIST_TRACK, &[Arg::Int(1)]);
            let mut a: Vec<Arg> = from.iter().chain(to.iter()).map(|&v| Arg::Float(v)).collect();
            a.extend([Arg::Int(ms), Arg::Int(cut)]);
            runner.run(op::CAMERA_SET_VECTOR_TRACK, &a);
        }
    }

    /// RPC 81 (AttachCameraToObject): ids ≤ 1000 of existing objects; snaps with a 100 ms move to itself.
    /// Mirrors samp.dll B 0x10019FA0.
    pub fn attach_camera(&mut self, runner: &mut Runner, objects: &crate::objects::Objects, id: u16) {
        if id > 1000 {
            return;
        }
        let Some(ptr) = objects.gta_ptr(id) else { return };
        self.spectate.applied = true;
        self.attached = Some(id);
        self.follow_entity(runner, ptr);
    }

    /// Every frame before `CCamera::Process`: the camera follows the attached object. Mirrors samp.dll B 0x1009D6E0.
    pub fn camera_follow(&mut self, runner: &mut Runner, objects: &crate::objects::Objects) {
        let Some(id) = self.attached else { return };
        if let Some(ptr) = objects.gta_ptr(id) {
            self.follow_entity(runner, ptr);
        }
    }

    fn follow_entity(&mut self, runner: &mut Runner, ptr: u32) {
        if ptr == 0 {
            return;
        }
        if let Some(p) = matrix_pos(ptr)
            && bh::valid_position(p)
        {
            vector_move(runner, p, p, 100, 1);
        }
    }

    // ---- spectating ----------------------------------------------------------------------------------------------

    /// RPC 124. Returns true when the spawn routine has to run (leaving spectator mode while spectating).
    /// Mirrors samp.dll B 0x100041A0.
    pub fn toggle_spectating(&mut self, enable: bool) -> bool {
        let spawn = self.spectate.on && !enable;
        self.spectate = Spectate { on: enable, target: SpecTarget::None, applied: false, mode: self.spectate.mode };
        spawn
    }

    /// RPC 126: id < 1004, slot in use, state neither 0 nor 32. Mirrors samp.dll B 0x10004260.
    pub fn spectate_player(&mut self, players: &crate::players::Players, id: u16, mode: u8) {
        self.spectate.mode = mode;
        if id >= 1004 {
            return;
        }
        if players.get(id).is_some_and(|p| p.state != 0 && p.state != crate::remote::state::WASTED) {
            self.spectate.target = SpecTarget::Player(id);
            self.spectate.applied = false;
        }
    }

    /// RPC 127: id < 2000 and the vehicle exists. Mirrors samp.dll B 0x10004210.
    pub fn spectate_vehicle(&mut self, vehicles: &crate::vehicles::Vehicles, id: u16, mode: u8) {
        self.spectate.mode = mode;
        if id < 2000 && vehicles.script_handle(id).is_some() {
            self.spectate.target = SpecTarget::Vehicle(id);
            self.spectate.applied = false;
        }
    }

    /// One spectator tick: a 212 at the on-foot interval (then `after_212`, the 203 attempt), HUD off, the ped
    /// parked 20 m above the camera with 100 health, then the view (once per target). Mirrors samp.dll B 0x10006530.
    #[allow(clippy::too_many_arguments)]
    pub fn spectator_tick(
        &mut self,
        runner: &mut Runner,
        net: &mut NetClient,
        players: &crate::players::Players,
        vehicles: &crate::vehicles::Vehicles,
        interval: Duration,
        now: Instant,
        after_212: impl FnOnce(&mut NetClient),
    ) {
        let ped = local_ped();
        if ped == 0 {
            return;
        }
        let cam = v3(CAMERA_MATRIX_POS);
        if let Ok(s) = game::read_local_sample(&ProcessMemory, ped) {
            let keys = crate::footsync::key_word(&s.pad, s.key_ctx);
            if self.last_212.is_none_or(|t| now.duration_since(t) > interval) {
                self.last_212 = Some(now);
                net.send_sync(&SpectatorSync {
                    left_right: s.pad.left_stick_x,
                    up_down: s.pad.left_stick_y,
                    keys,
                    position: Vec3 { x: cam[0], y: cam[1], z: cam[2] },
                });
                // Like samp, the spectator 203 is tried only right after a 212.
                after_212(net);
            }
        }
        crate::client::set_hud(false);
        poke(ped + off::PED_HEALTH, bh::SPEC_PED_HEALTH);
        teleport_entity(ped, [cam[0], cam[1], cam[2] + bh::SPEC_PED_ABOVE]);
        if let SpecTarget::Player(id) = self.spectate.target {
            let ok =
                players.get(id).is_some_and(|p| p.ped != 0 && p.state != 0 && p.state != crate::remote::state::WASTED);
            if !ok {
                crate::log!("behaviour: spectated player {id} gone (not streamed in, state 0 or wasted)");
                self.spectate.target = SpecTarget::None;
                self.spectate.applied = false;
            }
        }
        if self.spectate.applied {
            return;
        }
        match self.spectate.target {
            SpecTarget::None => {
                if in_vehicle(ped)
                    && let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten()
                {
                    let w = bh::SPEC_WARP_OUT;
                    runner.run(
                        op::WARP_CHAR_FROM_CAR_TO_COORD,
                        &[Arg::Int(h), Arg::Float(w[0]), Arg::Float(w[1]), Arg::Float(w[2])],
                    );
                }
                self.camera_position(runner, bh::SPEC_NO_TARGET_POS);
                self.camera_look_at(runner, bh::SPEC_NO_TARGET_LOOK, 2);
                self.spectate.applied = true;
            }
            SpecTarget::Player(id) => {
                let Some(p) = players.get(id).filter(|p| p.ped != 0) else { return };
                let Some(h) = game::ped_handle(&ProcessMemory, p.ped).ok().flatten() else { return };
                runner.run(
                    op::POINT_CAMERA_AT_CHAR,
                    &[Arg::Int(h), Arg::Int(i32::from(self.spectate.mode)), Arg::Int(2)],
                );
                self.spectate.applied = true;
                crate::log!("behaviour: spectating player {id} (mode {})", self.spectate.mode);
            }
            SpecTarget::Vehicle(id) => {
                let Some(h) = vehicles.script_handle(id) else { return };
                runner
                    .run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(h), Arg::Int(i32::from(self.spectate.mode)), Arg::Int(2)]);
                self.spectate.applied = true;
                crate::log!("behaviour: spectating vehicle {id} (mode {})", self.spectate.mode);
            }
        }
    }

    /// RPC 157: opcode 0x0925, attachment off, then 0x015F. Mirrors samp.dll B 0x1009D7B0.
    pub fn camera_position(&mut self, runner: &mut Runner, p: [f32; 3]) {
        runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.attached = None;
        runner.run(op::SET_FIXED_CAMERA_POSITION, &[p[0], p[1], p[2], 0.0, 0.0, 0.0].map(Arg::Float));
    }

    /// RPC 158: opcode 0x0925, attachment off, then 0x0160 with the cut. Mirrors samp.dll B 0x1009D810.
    pub fn camera_look_at(&mut self, runner: &mut Runner, p: [f32; 3], cut: u8) {
        runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.attached = None;
        runner.run(
            op::POINT_CAMERA_AT_POINT,
            &[Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Int(i32::from(bh::camera_cut(cut)))],
        );
    }
}

/// `CAMERA_RESET_NEW_SCRIPTABLES`, `CAMERA_PERSIST_POS(1)`, `CAMERA_SET_VECTOR_MOVE`. Mirrors samp.dll B 0x1009D5C0.
fn vector_move(runner: &mut Runner, from: [f32; 3], to: [f32; 3], ms: i32, cut: i32) {
    runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
    runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(1)]);
    let mut a: Vec<Arg> = from.iter().chain(to.iter()).map(|&v| Arg::Float(v)).collect();
    a.extend([Arg::Int(ms), Arg::Int(cut)]);
    runner.run(op::CAMERA_SET_VECTOR_MOVE, &a);
}

pub(super) fn area_visible(runner: &mut Runner) -> i32 {
    runner.run(op::GET_AREA_VISIBLE, &[Arg::Local(0)]);
    runner.local_i32(0) & 0xFF
}
