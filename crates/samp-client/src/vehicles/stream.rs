//! Streaming vehicles in and out (RPC 164 / 165), samp's vehicle constructor and destructor, and the
//! standalone create / delete helpers other modules use (crime report, model preview).

use super::*;

impl Vehicles {
    /// RPC 164 StreamInVehicle: busy model load, CREATE_CAR (or CREATE_MISSION_TRAIN plus its carriages), then the
    /// creation fix-ups, colours, damage, components and paintjob in the original's opcode order.
    pub(super) fn stream_in(&mut self, runner: &mut Runner, s: &rv::StreamInVehicle) {
        if s.vehicle_id >= vm::MAX_VEHICLES || !(vm::MODEL_MIN..=vm::MODEL_MAX).contains(&s.model) {
            return;
        }
        if self.pool.contains_key(&s.vehicle_id) {
            crate::ui::print_debug(&format!("Warning: vehicle {} was not deleted", s.vehicle_id));
            self.delete(runner, s.vehicle_id);
        }
        let (x, y, z) = (s.position.x, s.position.y, s.position.z);
        let Some(handle) = create_for_stream_in(runner, s) else { return };
        let ptr = vehicle_at(handle);
        if ptr == 0 {
            crate::ui::print_debug(&format!("Warning: couldn't create vehicle type: {}", s.model));
            return;
        }
        let mut v = Veh::new(s.vehicle_id, s.model, handle, ptr);
        v.spawn_pos = [x, y, z];
        apply_stream_in_state(runner, &mut v, s);
        crate::log!(
            "vehicles: stream-in id={} model={} handle={handle:#x} ptr={ptr:#x} pos={x:.2},{y:.2},{z:.2} angle={:.1}",
            s.vehicle_id,
            s.model,
            s.angle
        );
        let kind = v.kind;
        self.pool.insert(s.vehicle_id, v);
        self.add_carriages(s, ptr);
        if kind == Kind::Automobile {
            apply_stream_in_mods(runner, handle, s);
        }
        if s.body_colour1 != rv::StreamInVehicle::BODY_COLOUR_UNSET
            || s.body_colour2 != rv::StreamInVehicle::BODY_COLOUR_UNSET
        {
            self.set_colours(s.vehicle_id, s.body_colour1 as u8, s.body_colour2 as u8);
        }
    }

    /// Locomotive carriages: GTA created them with the train; samp objects take m_pNextCarriage in turn.
    fn add_carriages(&mut self, s: &rv::StreamInVehicle, ptr: u32) {
        let Some(cm) = vm::carriage_of(s.model) else { return };
        let mut prev = ptr;
        for i in 1..=3u16 {
            let next: u32 = rd(prev + g::TRAIN_NEXT_CARRIAGE);
            if next == 0 {
                break;
            }
            let h = pool_ref(next);
            let mut c = Veh::new(s.vehicle_id + i, cm, h, next);
            c.carriage = true;
            self.pool.insert(s.vehicle_id + i, c);
            prev = next;
        }
    }

    /// RPC 165 StreamOutVehicle: occupants are warped out first, trailers detached, then the vehicle is deleted.
    pub(super) fn stream_out(&mut self, runner: &mut Runner, players: &mut Players, id: u16) {
        let Some(v) = self.get(id).cloned() else { return };
        crate::log!("vehicles: stream-out id={id}");
        let local = local_ped();
        if local != 0 && in_vehicle(local) && ped_vehicle(local) == v.ptr {
            // The local ped goes to the vehicle position (samp's helper; we use the warp opcode).
            if let (Some(p), Some(h)) = (pos_of(v.ptr), ped_handle(local)) {
                runner.run(
                    op::WARP_CHAR_FROM_CAR_TO_COORD,
                    &[Arg::Int(h), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2])],
                );
            }
        }
        for p in players.iter_mut() {
            if p.ped != 0 && in_vehicle(p.ped) && ped_vehicle(p.ped) == v.ptr {
                warp_out_own_position(runner, p.ped);
                if let Some(r) = self.remote.get_mut(&p.id) {
                    r.warps_out += 1;
                }
            }
        }
        let cabs: Vec<u16> = self.pool.values().filter(|c| c.trailer == Some(id)).map(|c| c.id).collect();
        for c in cabs {
            self.detach_trailer(runner, c);
        }
        self.delete(runner, id);
        if matches!(v.model, 537 | 538) {
            for i in 1..=3 {
                self.pool.remove(&(id + i));
            }
        }
    }

    /// The samp object's destructor: blips, occupants warped out 2 m above the car, trailer detached, then
    /// DELETE_CAR / DELETE_MISSION_TRAIN.
    pub(super) fn delete(&mut self, runner: &mut Runner, id: u16) {
        let Some(v) = self.pool.remove(&id) else { return };
        if v.carriage || v.ptr == 0 {
            return;
        }
        for b in [v.blip, v.objective_blip].into_iter().flatten() {
            runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
        }
        let pos = pos_of(v.ptr).unwrap_or_default();
        let mut occupants = vec![driver_of(v.ptr)];
        occupants.extend((0..8).map(|i| passenger_at(v.ptr, i)));
        for ped in occupants.into_iter().filter(|&p| p != 0) {
            if let Some(h) = ped_handle(ped) {
                let z = pos[2] + vm::EJECT_Z_OFFSET;
                runner.run(
                    op::WARP_CHAR_FROM_CAR_TO_COORD,
                    &[Arg::Int(h), Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(z)],
                );
            }
        }
        if let Some(t) = v.trailer.and_then(|t| self.get(t)) {
            detach_trailer_from_cab(runner, t.handle, v.handle);
        }
        if matches!(v.model, 537 | 538) {
            runner.run(op::DELETE_MISSION_TRAIN, &[Arg::Int(v.handle)]);
        } else {
            runner.run(op::DELETE_CAR, &[Arg::Int(v.handle)]);
        }
        // TODO: samp also releases the model on stream-out when more than 80 vehicle models are loaded; not
        // reproduced yet.
        crate::log!("vehicles: id={id} handle={:#x} deleted", v.handle);
    }

    /// Session ended: every vehicle goes.
    pub fn clear(&mut self, runner: &mut Runner) {
        let ids: Vec<u16> = self.pool.keys().copied().collect();
        for id in ids {
            self.delete(runner, id);
        }
        self.remote.clear();
        vehhooks::with_shared(|s| s.by_ptr.clear());
    }
}

/// The GTA vehicle of a stream-in: CREATE_MISSION_TRAIN for a train (every rail model loaded first), else
/// CREATE_CAR with heading, petrol-tank, hydraulics and tyre settings. `None` (after the samp warning) when a
/// model never loads.
fn create_for_stream_in(runner: &mut Runner, s: &rv::StreamInVehicle) -> Option<i32> {
    let (x, y, z) = (s.position.x, s.position.y, s.position.z);
    let model = s.model as i32;
    if let Some(train) = vm::train_type(s.model) {
        // Every rail model is waited for (samp: five loops); one that never loads means no train.
        let loaded = vm::RAIL_MODELS.iter().all(|&m| busy_load(runner, m as i32));
        if !loaded {
            crate::ui::print_debug(&format!("Warning: couldn't create vehicle type: {}", s.model));
            return None;
        }
        let dir = i32::from(s.angle > 180.0);
        runner.run(
            op::CREATE_MISSION_TRAIN,
            &[Arg::Int(train), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Int(dir), Arg::Local(0)],
        );
        Some(runner.local_i32(0))
    } else {
        if !busy_load(runner, model) {
            crate::ui::print_debug(&format!("Warning: couldn't create vehicle type: {}", s.model));
            return None;
        }
        runner.run(op::CREATE_CAR, &[Arg::Int(model), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Local(0)]);
        let handle = runner.local_i32(0);
        runner.run(op::SET_CAR_HEADING, &[Arg::Int(handle), Arg::Float(s.angle)]);
        runner.run(op::SET_PETROL_TANK_WEAKPOINT, &[Arg::Int(handle), Arg::Int(0)]);
        runner.run(op::SET_CAR_HYDRAULICS, &[Arg::Int(handle), Arg::Int(0)]);
        runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(handle), Arg::Int(0)]);
        Some(handle)
    }
}

/// The creation fix-ups of a stream-in (not for trains: door lock, dirt, z offset), then siren, colours,
/// health, interior, damage and tyres.
fn apply_stream_in_state(runner: &mut Runner, v: &mut Veh, s: &rv::StreamInVehicle) {
    let (ptr, handle) = (v.ptr, v.handle);
    let (x, y, z) = (s.position.x, s.position.y, s.position.z);
    if vm::train_type(s.model).is_none() {
        poke(ptr + g::VEH_DOOR_LOCK, 0u32);
        poke(ptr + g::VEH_DIRT, 0.0f32);
        let m = matrix(ptr);
        if m != 0 {
            let dz = if matches!(v.kind, Kind::Bike | Kind::Bmx) { 0.0 } else { vm::CREATE_Z_OFFSET };
            put_v3(m + off::MATRIX_POS, [x, y, z + dz]);
        }
    }
    v.siren = s.siren != 0;
    if s.colour1 != rv::StreamInVehicle::COLOUR_DEFAULT || s.colour2 != rv::StreamInVehicle::COLOUR_DEFAULT {
        poke(ptr + g::VEH_PRIMARY_COLOUR, s.colour1);
        poke(ptr + g::VEH_SECONDARY_COLOUR, s.colour2);
        v.colours = Some((s.colour1, s.colour2));
    }
    poke(ptr + g::VEH_HEALTH, s.health);
    if s.interior != 0 {
        runner.run(op::SET_VEHICLE_AREA_VISIBLE, &[Arg::Int(handle), Arg::Int(i32::from(s.interior))]);
    }
    if s.panel_damage != 0 || s.door_damage != 0 || s.light_damage != 0 {
        apply_damage(ptr, v.kind, s.panel_damage, s.door_damage, s.light_damage);
    }
    apply_tyres(ptr, v.kind, s.tyre_damage);
}

/// Stream-in components and paintjob of an automobile.
fn apply_stream_in_mods(runner: &mut Runner, handle: i32, s: &rv::StreamInVehicle) {
    for slot in 0..s.mods.len() {
        if let Some(c) = s.component(slot) {
            add_component(runner, handle, i32::from(c));
        }
    }
    if let Some(p) = s.paintjob()
        && p < 4
    {
        runner.run(op::GIVE_VEHICLE_PAINTJOB, &[Arg::Int(handle), Arg::Int(i32::from(p))]);
    }
}

/// PlayCrimeReport's temporary vehicle: samp's vehicle constructor at (0, 0, 0) with heading 0, then colours
/// (colour, 0xFF) and `CWorld::Remove` — the stream-in creation sequence without a pool entry. Returns
/// (script handle, CVehicle*).
pub fn create_detached(runner: &mut Runner, model: u16, colour: u8) -> Option<(i32, u32)> {
    let (handle, ptr) = create_at(runner, model, [0.0, 0.0, 0.0])?;
    poke(ptr + g::VEH_PRIMARY_COLOUR, colour);
    poke(ptr + g::VEH_SECONDARY_COLOUR, 0xFFu8);
    if crate::client::ped_in_world(ptr) {
        cworld_remove(ptr);
    }
    Some((handle, ptr))
}

/// samp's vehicle constructor for a non-train model at `pos`, heading 0: busy model load, CREATE_CAR, 0175
/// heading, 09C4/07FF/053F, door lock and dirt 0, matrix z + 0.25 (not bikes / BMX). Returns
/// (script handle, CVehicle*). Mirrors samp.dll 0x100B8410.
pub fn create_at(runner: &mut Runner, model: u16, pos: [f32; 3]) -> Option<(i32, u32)> {
    let [x, y, z] = pos;
    if !busy_load(runner, i32::from(model)) {
        crate::log!("Warning: couldn't create vehicle type: {model}");
        return None;
    }
    runner
        .run(op::CREATE_CAR, &[Arg::Int(i32::from(model)), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Local(0)]);
    let handle = runner.local_i32(0);
    runner.run(op::SET_CAR_HEADING, &[Arg::Int(handle), Arg::Float(0.0)]);
    runner.run(op::SET_PETROL_TANK_WEAKPOINT, &[Arg::Int(handle), Arg::Int(0)]);
    runner.run(op::SET_CAR_HYDRAULICS, &[Arg::Int(handle), Arg::Int(0)]);
    runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(handle), Arg::Int(0)]);
    let ptr = vehicle_at(handle);
    if ptr == 0 {
        crate::log!("Warning: couldn't create vehicle type: {model}");
        return None;
    }
    poke(ptr + g::VEH_DOOR_LOCK, 0u32);
    poke(ptr + g::VEH_DIRT, 0.0f32);
    let m = matrix(ptr);
    if m != 0 {
        let dz = if matches!(kind_of(ptr), Kind::Bike | Kind::Bmx) { 0.0 } else { vm::CREATE_Z_OFFSET };
        put_v3(m + off::MATRIX_POS, [x, y, z + dz]);
    }
    Some((handle, ptr))
}

/// samp's vehicle destructor for a vehicle made by [`create_at`]: DELETE_CAR (the model release over 80 loaded
/// vehicle models is not reproduced). Mirrors samp.dll 0x100B8930.
pub fn delete_handle(runner: &mut Runner, handle: i32) {
    runner.run(op::DELETE_CAR, &[Arg::Int(handle)]);
}

/// samp's vehicle-type code from the CVehicle vtable (same table as `vm::Kind`, 0 when unknown).
/// Mirrors samp.dll 0x100B6220.
pub fn samp_type(ptr: u32) -> u8 {
    match rd::<u32>(ptr) {
        vm::vtbl::AUTOMOBILE => 1,
        vm::vtbl::BOAT => 4,
        vm::vtbl::BIKE => 2,
        vm::vtbl::PLANE => 5,
        vm::vtbl::HELI => 3,
        vm::vtbl::BMX => 6,
        vm::vtbl::TRAIN => 7,
        _ => 0,
    }
}

/// Model load before CREATE_CAR: HAS_MODEL_LOADED; if missing REQUEST_MODEL + LOAD_ALL_MODELS_NOW and busy-wait on
/// HAS_MODEL_LOADED with Sleep(0). samp waits without limit (a hang if the model never loads); we stop after 10 s,
/// log, and return false: the caller then creates nothing (CREATE_CAR on a model that is not loaded faults in GTA
/// and leaves a dead handle for the next opcodes; recorded in docs/ROADMAP.md "Decisions made autonomously").
fn busy_load(runner: &mut Runner, model: i32) -> bool {
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        return true;
    }
    runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
    runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
    let t0 = Instant::now();
    while !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        if t0.elapsed() > Duration::from_secs(10) {
            crate::log!("vehicles: model {model} still not loaded after 10 s");
            return false;
        }
        std::thread::sleep(Duration::ZERO);
    }
    true
}
