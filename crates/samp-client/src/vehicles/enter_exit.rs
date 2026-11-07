//! Entering and leaving vehicles: the server's put-in / remove-from RPCs (70 / 71), the enter / exit relays of
//! remote players (26 / 154), the events of the enter / leave hooks and the local passenger-enter poll.

use super::*;

impl Vehicles {
    /// RPC 70 PutPlayerInVehicle for the local ped. Mirrors samp.dll 0x100AC2D0.
    pub(super) fn put_local_in(&mut self, runner: &mut Runner, id: u16, seat: u8) {
        let Some(v) = self.get(id).cloned() else { return };
        let ped = local_ped();
        let Some(h) = ped_handle(ped) else { return };
        if !seat_allowed(&v, seat) {
            return;
        }
        drop_parachute(runner, h);
        warp_into(runner, h, &v, seat);
        if in_vehicle(ped) {
            camera_behind(runner);
            if vm::is_rail(v.model) {
                runner.run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(v.handle), Arg::Int(3), Arg::Int(2)]);
            }
        }
    }

    /// RPC 71 RemovePlayerFromVehicle: the local ped leaves its pool vehicle (not the carriages 538 / 570).
    pub(super) fn remove_local_from_vehicle(&mut self, runner: &mut Runner) {
        let ped = local_ped();
        if ped != 0 && in_vehicle(ped) {
            let veh = ped_vehicle(ped);
            if let Some(v) = self.id_of_ptr(veh).and_then(|id| self.get(id))
                && !matches!(v.model, 538 | 570)
                && let Some(h) = ped_handle(ped)
            {
                runner.run(op::TASK_LEAVE_CAR, &[Arg::Int(h), Arg::Int(v.handle)]);
            }
        }
    }

    /// RPC 26 EnterVehicle relay: a nearby remote ped starts the enter task (driver or passenger).
    pub(super) fn remote_enter(
        &mut self,
        runner: &mut Runner,
        players: &mut Players,
        pid: u16,
        id: u16,
        passenger: bool,
    ) {
        let Some(v) = self.get(id).cloned() else { return };
        let Some(p) = players.get_mut(pid) else { return };
        if p.ped == 0 || !within_relay_range(p.ped) || in_vehicle(p.ped) {
            return;
        }
        clear_keys(p);
        let Some(h) = ped_handle(p.ped) else { return };
        if passenger {
            runner.run(
                op::TASK_ENTER_CAR_AS_PASSENGER,
                &[Arg::Int(h), Arg::Int(v.handle), Arg::Int(vm::ENTER_TASK_MS), Arg::Int(-1)],
            );
        } else {
            runner.run(op::TASK_ENTER_CAR_AS_DRIVER, &[Arg::Int(h), Arg::Int(v.handle), Arg::Int(vm::ENTER_TASK_MS)]);
        }
    }

    /// RPC 154 S→C relay: the vehicle id is ignored; the ped leaves whatever it is in.
    pub(super) fn remote_exit(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get_mut(pid) else { return };
        if p.ped == 0 || !within_relay_range(p.ped) || !in_vehicle(p.ped) {
            return;
        }
        let veh = ped_vehicle(p.ped);
        let model = if veh != 0 { model_of_entity(veh) } else { 0 };
        if matches!(model, 570 | 538) {
            return;
        }
        clear_keys(p);
        let vh = self.id_of_ptr(veh).and_then(|id| self.get(id)).map(|v| v.handle);
        if let (Some(h), Some(vh)) = (ped_handle(p.ped), vh) {
            runner.run(op::TASK_LEAVE_CAR, &[Arg::Int(h), Arg::Int(vh)]);
        }
    }

    /// Events queued by the enter/leave hooks since the last tick.
    pub fn hook_events(&mut self, runner: &mut Runner, net: &mut NetClient) {
        let events = vehhooks::with_shared(|s| std::mem::take(&mut s.events));
        let ped = local_ped();
        let h = ped_handle(ped);
        for ev in events {
            match ev {
                HookEvent::Enter { vehicle_id, parachute } => {
                    if parachute && let Some(h) = h {
                        runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
                    }
                    crate::log!("vehicles: enter request vehicle {vehicle_id} (driver)");
                    send(net, &rv::OnPlayerEnterVehicle { vehicle_id, passenger: 0 });
                    self.n.sent_26 += 1;
                    self.train_camera(runner, vehicle_id);
                }
                HookEvent::Exit { vehicle_id, driver, rc, train } => {
                    if driver {
                        self.last_driven = Some(vehicle_id);
                    }
                    if train {
                        // Warped inside the constructor hook (`vehhooks::leave_filter`); nothing else.
                    } else if !rc {
                        crate::log!("vehicles: exit vehicle {vehicle_id}");
                        send(net, &rv::OnPlayerExitVehicle { vehicle_id });
                        self.n.sent_154 += 1;
                    }
                }
                HookEvent::Scm { handle, arg1, arg2, kind } => self.scm_event_out(net, handle, arg1, arg2, kind),
            }
        }
    }

    fn train_camera(&mut self, runner: &mut Runner, id: u16) {
        if let Some(v) = self.get(id)
            && vm::is_rail(v.model)
        {
            runner.run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(v.handle), Arg::Int(3), Arg::Int(2)]);
            self.train_camera_since = Some(Instant::now());
        }
    }

    /// Passenger-enter poll: on the GROUP_CONTROL_FWD (DPadUp) press edge, enter the closest vehicle within 8 m as
    /// a passenger and send RPC 26. Mirrors samp.dll 0x10006FE0.
    pub(super) fn passenger_poll(&mut self, runner: &mut Runner, net: &mut NetClient, ped: u32) {
        let new: i16 = rd(g::PAD_DPAD_UP_NEW);
        let old: i16 = rd(g::PAD_DPAD_UP_OLD);
        if new == 0 || old != 0 {
            return;
        }
        let Some(me) = pos_of(ped) else { return };
        let closest = self
            .pool
            .values()
            .filter(|v| v.ptr != 0 && v.id != 0)
            .filter_map(|v| pos_of(v.ptr).map(|p| (vm::dist3d(p, me), v.id, v.handle, v.model)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((d, id, vh, model)) = closest else { return };
        if d >= vm::PASSENGER_ENTER_RANGE {
            return;
        }
        let Some(h) = ped_handle(ped) else { return };
        drop_parachute(runner, h);
        // Model 570 warps (seat −1 = GTA's first free passenger seat), others animate.
        if vm::passenger_enter_warps(model) {
            runner.run(op::WARP_CHAR_INTO_CAR_AS_PASSENGER, &[Arg::Int(h), Arg::Int(vh), Arg::Int(-1)]);
        } else {
            runner.run(
                op::TASK_ENTER_CAR_AS_PASSENGER,
                &[Arg::Int(h), Arg::Int(vh), Arg::Int(vm::ENTER_TASK_MS), Arg::Int(-1)],
            );
        }
        crate::log!("vehicles: enter request vehicle {id} (passenger)");
        send(net, &rv::OnPlayerEnterVehicle { vehicle_id: id, passenger: 1 });
        self.n.sent_26 += 1;
        self.train_camera(runner, id);
    }
}

/// samp reads the weapon with GET_CURRENT_CHAR_WEAPON; a parachute is swapped for fists.
fn drop_parachute(runner: &mut Runner, ped_handle: i32) {
    runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(ped_handle), Arg::Local(0)]);
    if runner.local_i32(0) as u32 == u32::from(vm::WEAPON_PARACHUTE) {
        runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(ped_handle), Arg::Int(0)]);
    }
}

fn within_relay_range(ped: u32) -> bool {
    match (pos_of(ped), pos_of(local_ped())) {
        (Some(a), Some(b)) => vm::dist3d(a, b) < vm::RELAY_RANGE,
        _ => false,
    }
}

fn clear_keys(p: &mut remote::RemotePlayer) {
    if let Some(f) = p.foot.as_mut() {
        f.keys = 0;
        f.left_right = 0;
        f.up_down = 0;
    }
    if let Some(slot) = p.slot {
        super::remote_apply::apply_remote_keys(slot, PadInput::default());
    }
}

/// Seat checks of samp's seat warp: refused for vehicle health 0, a seat beyond the passenger count
/// (automobile/bike kinds), a deleted entity, or an occupied driver seat.
pub(super) fn seat_allowed(v: &Veh, seat: u8) -> bool {
    let ptr = v.ptr;
    if is_deleted(ptr) || rd::<f32>(ptr + g::VEH_HEALTH) == 0.0 {
        return false;
    }
    if matches!(v.kind, Kind::Automobile | Kind::Bike)
        && u32::from(seat) > u32::from(rd::<u8>(ptr + g::VEH_MAX_PASSENGERS))
    {
        return false;
    }
    !(seat == 0 && driver_of(ptr) != 0)
}

pub(super) fn warp_into(runner: &mut Runner, ped_handle: i32, v: &Veh, seat: u8) {
    if seat == 0 {
        runner.run(op::WARP_CHAR_INTO_CAR, &[Arg::Int(ped_handle), Arg::Int(v.handle)]);
    } else {
        runner.run(
            op::WARP_CHAR_INTO_CAR_AS_PASSENGER,
            &[Arg::Int(ped_handle), Arg::Int(v.handle), Arg::Int(i32::from(seat) - 1)],
        );
    }
}

/// WARP_CHAR_FROM_CAR_TO_COORD to the ped's own current position.
pub(super) fn warp_out_own_position(runner: &mut Runner, ped: u32) {
    let (Some(p), Some(h)) = (pos_of(ped), ped_handle(ped)) else { return };
    runner.run(op::WARP_CHAR_FROM_CAR_TO_COORD, &[Arg::Int(h), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2])]);
    crate::log!("vehicles: ped {ped:#x} warped out to its own position");
}

/// samp's camera reset: 0930(0), 0925, 0373, 02EB. Mirrors samp.dll 0x1009D7A0.
pub(super) fn camera_behind(runner: &mut Runner) {
    runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(0)]);
    runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
    runner.run(op::SET_CAMERA_BEHIND_PLAYER, &[]);
    runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
}
