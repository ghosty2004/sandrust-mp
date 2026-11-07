//! The vehicle RPC dispatcher and the handlers that only set vehicle state (health, damage, params, velocity,
//! position, heading, interior, plate).

use super::*;

impl Vehicles {
    /// Dispatches one vehicle RPC.
    pub fn on_rpc(&mut self, runner: &mut Runner, players: &mut Players, r: V) {
        match r {
            V::StreamInVehicle(s) => self.stream_in(runner, &s),
            V::StreamOutVehicle(s) => self.stream_out(runner, players, s.vehicle_id),
            V::SetVehicleParams(p) => self.set_params(p.vehicle_id, &p.params),
            V::SetVehicleHealth(h) => self.set_health(h.vehicle_id, h.health),
            V::SetVehicleDamageStatus(d) => self.set_damage_status(&d),
            V::RemoveVehicleComponent(c) => self.remove_component(runner, c.vehicle_id, c.component),
            V::LinkVehicleToInterior(l) => self.link_to_interior(runner, l.vehicle_id, l.interior),
            V::SetVehicleVelocity(s) => self.set_local_velocity(&s),
            V::ScmEvent(e) => self.scm_event(runner, players, &e),
            V::SetVehiclePlate(p) => self.set_plate(p.vehicle_id, &p.plate),
            V::AttachTrailer(a) => self.attach_trailer(runner, a.trailer_id, a.vehicle_id),
            V::DetachTrailer(d) => self.detach_trailer(runner, d.vehicle_id),
            V::SetVehiclePosition(p) => {
                self.set_position(runner, p.vehicle_id, [p.position.x, p.position.y, p.position.z])
            }
            V::SetVehicleZAngle(a) => self.set_z_angle(runner, a.vehicle_id, a.angle),
            V::PutPlayerInVehicle(p) => self.put_local_in(runner, p.vehicle_id, p.seat),
            V::RemovePlayerFromVehicle(_) => self.remove_local_from_vehicle(runner),
            V::EnterVehicle(e) => self.remote_enter(runner, players, e.player_id, e.vehicle_id, e.passenger != 0),
            V::ExitVehicle(e) => self.remote_exit(runner, players, e.player_id),
        }
    }

    /// RPC 147 SetVehicleHealth.
    fn set_health(&mut self, id: u16, health: f32) {
        if let Some(v) = self.get(id) {
            poke(v.ptr + g::VEH_HEALTH, health);
        }
    }

    /// RPC 106 UpdateVehicleDamageStatus.
    fn set_damage_status(&mut self, d: &rv::SetVehicleDamageStatus) {
        if let Some(v) = self.get(d.vehicle_id) {
            apply_damage(v.ptr, v.kind, d.panels, d.doors, d.lights);
            apply_tyres(v.ptr, v.kind, d.tyres);
        }
    }

    /// RPC 57 RemoveVehicleComponent.
    fn remove_component(&mut self, runner: &mut Runner, id: u16, component: u16) {
        if let Some(v) = self.get(id) {
            runner.run(op::REMOVE_VEHICLE_MOD, &[Arg::Int(v.handle), Arg::Int(i32::from(component))]);
        }
    }

    /// RPC 65 LinkVehicleToInterior.
    fn link_to_interior(&mut self, runner: &mut Runner, id: u16, interior: u8) {
        if let Some(v) = self.get(id) {
            runner.run(op::SET_VEHICLE_AREA_VISIBLE, &[Arg::Int(v.handle), Arg::Int(i32::from(interior))]);
        }
    }

    /// RPC 91 SetVehicleVelocity: the move (kind 0) or turn (kind 1) speed of the pool vehicle the local ped is in.
    fn set_local_velocity(&mut self, s: &rv::SetVehicleVelocity) {
        let ped = local_ped();
        if ped != 0 && in_vehicle(ped) {
            let veh = ped_vehicle(ped);
            if self.id_of_ptr(veh).is_some() {
                let v = vm::vec3(s.velocity);
                match s.kind {
                    0 => put_v3(veh + off::MOVE_SPEED, v),
                    1 => put_v3(veh + g::TURN_SPEED, v),
                    _ => {}
                }
            }
        }
    }

    /// RPC 123 SetVehiclePlate: text only; the texture is created once by the plate pass (`plates.rs`).
    fn set_plate(&mut self, id: u16, plate: &[u8]) {
        if plate.len() <= 32
            && let Some(v) = self.pool.get_mut(&id)
        {
            v.plate = plate.to_vec();
        }
    }

    /// RPC 159 SetVehiclePos: trains through SET_MISSION_TRAIN_COORDINATES, other vehicles by the entity teleport.
    fn set_position(&mut self, runner: &mut Runner, id: u16, pos: [f32; 3]) {
        if let Some(v) = self.get(id) {
            let [x, y, z] = pos;
            if vm::TRAIN_SYNC_MODELS.contains(&v.model) {
                runner.run(
                    op::SET_MISSION_TRAIN_COORDINATES,
                    &[Arg::Int(v.handle), Arg::Float(x), Arg::Float(y), Arg::Float(z)],
                );
            } else {
                teleport(v.ptr, [x, y, z]);
            }
        }
    }

    /// RPC 160 SetVehicleZAngle.
    fn set_z_angle(&mut self, runner: &mut Runner, id: u16, angle: f32) {
        if let Some(v) = self.get(id) {
            runner.run(op::SET_CAR_HEADING, &[Arg::Int(v.handle), Arg::Float(angle)]);
        }
    }

    /// RPC 98: tyre damage (docs/re/subsystems/session-misc.md).
    pub fn on_tyres(&mut self, id: u16, tyres: u8) {
        if let Some(v) = self.get(id) {
            apply_tyres(v.ptr, v.kind, tyres);
        }
    }

    /// RPC 161: old-style vehicle params (objective, doors locked).
    pub fn on_old_params(&mut self, id: u16, objective: u8, doors: u8) {
        if let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) {
            if objective != 0 {
                v.objective = true;
            }
            v.locked = doors != 0;
            poke(v.ptr + g::VEH_DOOR_LOCK, if doors != 0 { 2u32 } else { 0 });
        }
    }

    /// RPC 24 SetVehicleParams: engine, lights, alarm, doors, objective, siren, door and window states.
    fn set_params(&mut self, id: u16, p: &rv::VehicleParams) {
        let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) else { return };
        let ptr = v.ptr;
        if let Some(on) = param_bit(p.engine) {
            v.engine = i32::from(on);
        }
        if let Some(on) = param_bit(p.lights) {
            v.lights = i32::from(on);
        }
        if let Some(on) = param_bit(p.alarm) {
            set_bits_u8(ptr + g::VEH_SIREN_BYTE, g::VEH_SIREN_ON, on);
            poke(ptr + g::VEH_ALARM, if on { 20000u16 } else { 0 });
        }
        if let Some(on) = param_bit(p.doors) {
            poke(ptr + g::VEH_DOOR_LOCK, if on { 2u32 } else { 0 });
            v.locked = on;
        }
        if let Some(on) = param_bit(p.objective) {
            v.objective = on;
        }
        if let Some(on) = param_bit(p.siren) {
            set_bits_u8(ptr + g::VEH_SIREN_BYTE, g::VEH_SIREN_ON, on);
        }
        if v.kind == Kind::Automobile {
            open_doors(ptr, p);
        }
        set_windows(ptr, p);
    }
}

/// A SetVehicleParams byte: 0 / 1 set the state, anything else leaves it.
fn param_bit(b: u8) -> Option<bool> {
    matches!(b, 0 | 1).then_some(b == 1)
}

/// The bonnet, boot and door states of SetVehicleParams (automobiles).
fn open_doors(ptr: u32, p: &rv::VehicleParams) {
    // (wire byte, node, door index)
    let doors = [
        (p.bonnet, 0x10, 0),
        (p.boot, 0x11, 1),
        (p.door_driver, 10, 2),
        (p.door_passenger, 8, 3),
        (p.door_back_left, 11, 4),
        (p.door_back_right, 9, 5),
    ];
    for (b, node, door) in doors {
        if let Some(open) = param_bit(b) {
            // SAFETY: CAutomobile::OpenDoor(CPed*, node, door, ratio, playSound) on a live automobile.
            // The ped / sound arguments samp passes are not documented: null / false here.
            unsafe {
                let f: extern "thiscall" fn(u32, u32, i32, i32, f32, u8) =
                    std::mem::transmute(g::AUTOMOBILE_OPEN_DOOR as usize);
                f(ptr, 0, node, door, if open { 1.0 } else { 0.0 }, 0);
            }
        }
    }
}

/// The window states of SetVehicleParams.
fn set_windows(ptr: u32, p: &rv::VehicleParams) {
    for (b, node) in
        [(p.window_driver, 10u32), (p.window_passenger, 8), (p.window_back_left, 11), (p.window_back_right, 9)]
    {
        if let Some(closed) = param_bit(b) {
            let f = if closed { g::CLEAR_WINDOW_OPEN } else { g::SET_WINDOW_OPEN };
            // SAFETY: CVehicle::Set/ClearWindowOpenFlag(uint8 node) on a live vehicle.
            unsafe {
                let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(f as usize);
                f(ptr, node);
            }
        }
    }
}
