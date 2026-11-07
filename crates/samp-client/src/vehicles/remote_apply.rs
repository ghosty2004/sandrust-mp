//! Remote players in vehicles: storing received driver / passenger sync (200 / 211), applying unoccupied sync
//! (209), seating remote peds, "Car Jacked", and the per-frame apply of every remote driver and passenger.

use samp_proto::sync::{RemotePassengerSync, RemoteVehicleSync};

use super::*;

impl Vehicles {
    /// Received in-car sync: 200 / 211 are stored for the remote tick, 209 / 210 applied immediately.
    pub fn on_sync(&mut self, runner: &mut Runner, players: &mut Players, s: IncomingSync, now: Instant) {
        match s {
            IncomingSync::Vehicle { timestamp, sync } => self.store_driver(runner, players, timestamp, &sync, now),
            IncomingSync::Passenger(ps) => self.store_passenger(runner, players, &ps, now),
            IncomingSync::Unoccupied(u) => self.apply_unoccupied(u.player_id, &u.sync),
            IncomingSync::Trailer(t) => self.apply_trailer(runner, t.player_id, &t.sync),
            _ => {}
        }
    }

    /// Stores a remote driver sync (200), seats the ped when it is not in a vehicle yet, then the state change.
    fn store_driver(
        &mut self,
        runner: &mut Runner,
        players: &mut Players,
        timestamp: Option<u32>,
        sync: &RemoteVehicleSync,
        now: Instant,
    ) {
        if Some(sync.player_id) == self.local_id {
            return;
        }
        let Some(p) = players.get_mut(sync.player_id) else { return };
        if !accept_timestamp(p.last_ts, timestamp) {
            return;
        }
        if timestamp.is_some() {
            p.last_ts = timestamp;
        }
        let d = DriverData::from_sync(sync);
        p.health = d.health;
        p.armour = d.armour;
        let old_state = p.state;
        p.state = state::DRIVER;
        p.last_sync = Some(now);
        let r = self.remote.entry(sync.player_id).or_default();
        r.driver = Some(d);
        r.driver_new = true;
        self.last_vehicle.insert(sync.player_id, d.vehicle_id);
        r.passenger = None;
        let ped = p.ped;
        if ped != 0 && !in_vehicle(ped) && self.seat_remote(runner, ped, d.vehicle_id, 0) {
            self.remote.entry(sync.player_id).or_default().warps_in += 1;
        }
        // The state change to driver comes last.
        if old_state != state::DRIVER {
            self.car_jacked(runner, old_state, d.vehicle_id);
        }
    }

    /// Stores a remote passenger sync (211) and seats the ped when it is not in a vehicle yet.
    fn store_passenger(&mut self, runner: &mut Runner, players: &mut Players, ps: &RemotePassengerSync, now: Instant) {
        if Some(ps.player_id) == self.local_id {
            return;
        }
        let Some(p) = players.get_mut(ps.player_id) else { return };
        let d = PassengerData::from_sync(&ps.sync);
        p.health = d.health;
        p.armour = d.armour;
        p.state = state::PASSENGER;
        p.last_sync = Some(now);
        let r = self.remote.entry(ps.player_id).or_default();
        r.passenger = Some(d);
        r.passenger_new = true;
        self.last_vehicle.insert(ps.player_id, d.vehicle_id);
        r.driver = None;
        let ped = p.ped;
        if ped != 0 && !in_vehicle(ped) && d.seat > 0 && self.seat_remote(runner, ped, d.vehicle_id, d.seat) {
            self.remote.entry(ps.player_id).or_default().warps_in += 1;
        }
    }

    /// "Car Jacked": a remote player goes from on foot to driving the vehicle the local ped is driving → the local
    /// ped is warped out of it to its own position + 1 m up, game text "~r~Car Jacked~w~!" (1000 ms, style 5).
    /// Mirrors samp.dll 0x10016220 (build B).
    fn car_jacked(&mut self, runner: &mut Runner, old_state: u8, vehicle_id: u16) {
        let ped = local_ped();
        if old_state != remote::state::ON_FOOT || ped == 0 || !in_vehicle(ped) {
            return;
        }
        let veh = ped_vehicle(ped);
        if veh == 0 || driver_of(veh) != ped || self.id_of_ptr(veh) != Some(vehicle_id) {
            return;
        }
        let (Some(p), Some(h)) = (pos_of(ped), ped_handle(ped)) else { return };
        runner.run(
            op::WARP_CHAR_FROM_CAR_TO_COORD,
            &[Arg::Int(h), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2] + 1.0)],
        );
        crate::behaviourclient::big_message(runner, CAR_JACKED_TEXT, CAR_JACKED_MS, CAR_JACKED_STYLE);
        self.n.jacked += 1;
        crate::log!("vehicles: car jacked (vehicle {vehicle_id}): local ped warped out");
    }

    /// Driver/passenger seat warp of a remote ped (the weapon is applied by the remote tick).
    /// Mirrors samp.dll 0x10014800.
    fn seat_remote(&mut self, runner: &mut Runner, ped: u32, id: u16, seat: u8) -> bool {
        let Some(v) = self.get(id).cloned() else { return false };
        if !seat_allowed(&v, seat) {
            return false;
        }
        let Some(h) = ped_handle(ped) else { return false };
        warp_into(runner, h, &v, seat);
        crate::log!("vehicles: remote ped {ped:#x} warped into vehicle {id} seat {seat}");
        true
    }

    /// After a 207 store: a ped still in a vehicle is pulled out unless entering/exiting (special action 3/4).
    pub fn after_foot_store(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get_mut(pid) else { return };
        let sa = p.foot.map(|f| f.special_action).unwrap_or(0);
        if let Some(r) = self.remote.get_mut(&pid) {
            r.driver = None;
            r.passenger = None;
        }
        // Not while the ped runs its leave-car task (the RPC 154 exit animation).
        if p.ped != 0 && in_vehicle(p.ped) && sa != 3 && sa != 4 && !leaving_car(p.ped) {
            warp_out_own_position(runner, p.ped);
            if let Some(r) = self.remote.get_mut(&pid) {
                r.warps_out += 1;
            }
        }
    }

    /// Right after a remote ped was (re)created: seat it from the stored driver/passenger sync.
    pub fn after_stream_in(&mut self, runner: &mut Runner, players: &mut Players, pid: u16) {
        let Some(p) = players.get(pid) else { return };
        let ped = p.ped;
        if ped == 0 || in_vehicle(ped) {
            return;
        }
        let Some(r) = self.remote.get(&pid).copied() else { return };
        let ok = match (p.state, r.driver, r.passenger) {
            (state::DRIVER, Some(d), _) => self.seat_remote(runner, ped, d.vehicle_id, 0),
            (state::PASSENGER, _, Some(d)) if d.seat > 0 => self.seat_remote(runner, ped, d.vehicle_id, d.seat),
            _ => false,
        };
        if ok && let Some(r) = self.remote.get_mut(&pid) {
            r.warps_in += 1;
            r.driver_new = r.driver.is_some();
        }
    }

    /// A remote player left: drop its in-car state.
    pub fn forget_player(&mut self, pid: u16) {
        self.remote.remove(&pid);
        self.last_vehicle.remove(&pid);
    }

    /// samp's cache reset of a remote (RPC 166, stream-out) clears its last synced vehicle.
    /// Mirrors samp.dll 0x10015FA0.
    pub fn reset_remote_vehicle(&mut self, pid: u16) {
        self.last_vehicle.remove(&pid);
    }

    /// Unoccupied sync (209) apply, immediately on receive; skipped for player-driven or towed vehicles.
    fn apply_unoccupied(&mut self, pid: u16, s: &samp_proto::sync::vehicle::UnoccupiedSync) {
        if !vm::unoccupied_valid(s) {
            return;
        }
        let Some(v) = self.pool.get_mut(&s.vehicle_id).filter(|v| v.ptr != 0) else { return };
        let ptr = v.ptr;
        let driver = driver_of(ptr);
        if driver != 0 && rd::<u32>(driver + g::PED_TYPE) == 0 {
            return;
        }
        let tractor: u32 = rd(ptr + g::VEH_TRACTOR);
        if tractor != 0 && driver_of(tractor) != 0 {
            return;
        }
        v.last_unocc_sender = Some(pid);
        let m = matrix(ptr);
        if m == 0 {
            return;
        }
        let cur = v3(m + off::MATRIX_POS);
        let target = vm::vec3(s.position);
        let up = v3(m + g::MATRIX_UP);
        let rows = Rotation { right: vm::vec3(s.roll), forward: vm::vec3(s.direction), up };
        match vm::correct_unoccupied(cur, target, vm::vec3(s.velocity)) {
            VehCorrection::None => {}
            VehCorrection::Speed(speed) => {
                write_rows(m, &rows);
                put_v3(ptr + off::MOVE_SPEED, speed);
                put_v3(ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            }
            VehCorrection::Teleport => {
                write_rows(m, &rows);
                put_v3(m + off::MATRIX_POS, target);
                put_v3(ptr + off::MOVE_SPEED, vm::vec3(s.velocity));
                put_v3(ptr + g::TURN_SPEED, vm::vec3(s.turn_velocity));
            }
        }
        self.n.applied_209 += 1;
    }

    /// Per frame, for every remote driver / passenger: the one-shot apply of a new 200 (corrections, extra,
    /// siren, gear, health, weapon, trailer) or 211 (weapon, drive-by), then every frame the turn clamp, rotation
    /// slerp and pad keys. Mirrors the driver / passenger part of samp.dll 0x100166B0.
    pub fn remote_tick(&mut self, runner: &mut Runner, players: &mut Players) {
        for p in players.iter_mut() {
            let Some(slot) = p.slot else { continue };
            if p.ped == 0 || !(p.state == state::DRIVER || p.state == state::PASSENGER) {
                continue;
            }
            let ped = p.ped;
            let seated = in_vehicle(ped);
            let Some(r) = self.remote.get_mut(&p.id) else { continue };
            if let (state::DRIVER, Some(d)) = (p.state, r.driver) {
                self.remote_driver_frame(runner, p.id, ped, slot, p.npc, seated, d);
            } else if let (state::PASSENGER, Some(d)) = (p.state, r.passenger) {
                if r.passenger_new && seated {
                    // One-shot apply, passenger: weapon and drive-by task.
                    crate::players::apply_weapon(runner, ped, slot, d.weapon);
                    remote_drive_by(runner, ped, slot, d.drive_by);
                }
                r.passenger_new = false;
                let input = PadInput { keys: d.keys, left_right: 0, up_down: 0, in_vehicle: true, jetpack: false };
                apply_remote_keys(slot, input);
            }
        }
    }

    /// One remote driver in one frame: the one-shot apply of a new 200 while seated, then the per-frame part.
    #[allow(clippy::too_many_arguments)]
    fn remote_driver_frame(
        &mut self,
        runner: &mut Runner,
        pid: u16,
        ped: u32,
        slot: u8,
        npc: bool,
        seated: bool,
        d: DriverData,
    ) {
        let Some(v) = self.pool.get(&d.vehicle_id).filter(|v| v.ptr != 0) else { return };
        if !seated {
            return;
        }
        let m = matrix(v.ptr);
        let Some(r) = self.remote.get_mut(&pid) else { return };
        if r.driver_new && m != 0 {
            r.driver_new = false;
            apply_driver_motion(runner, r, v, m, &d);
            apply_driver_state(runner, v, ped, slot, &d);
            if d.trailer_id == 0 || d.trailer_id >= vm::MAX_VEHICLES {
                let tr: u32 = rd(v.ptr + g::VEH_TRAILER);
                if tr != 0 && self.pool.values().any(|t| t.ptr == tr && t.id != 0) {
                    let id = v.id;
                    self.detach_trailer(runner, id);
                }
            }
        }
        self.remote_driver_every_frame(ped, slot, npc, &d);
    }

    /// Every frame for a seated remote driver: turn clamp and rotation slerp (not trains), the RC driver kept out
    /// of the world, an NPC's zero speed, pad keys and horn.
    fn remote_driver_every_frame(&self, ped: u32, slot: u8, npc: bool, d: &DriverData) {
        let Some(v) = self.pool.get(&d.vehicle_id) else { return };
        let ptr = v.ptr;
        let m = matrix(ptr);
        if m != 0 && !vm::TRAIN_SYNC_MODELS.contains(&v.model) {
            put_v3(ptr + g::TURN_SPEED, vm::clamp_turn(v3(ptr + g::TURN_SPEED)));
            let cur = quat_from_matrix(&rows_of(m));
            let q = normalise(slerp(d.rotation, cur, remote::SLERP_KEEP));
            write_rows(m, &rows_from_quat(q));
        }
        // RC models: the seated "driver" is taken out of the world (`CWorld::Remove`) whenever GTA has put
        // it in the moving list again.
        if vm::is_rc(v.model) {
            remove_from_world(ped);
        }
        if npc && d.velocity == [0.0; 3] {
            put_v3(ptr + off::MOVE_SPEED, [0.0; 3]);
        }
        let input =
            PadInput { keys: d.keys, left_right: d.left_right, up_down: d.up_down, in_vehicle: true, jetpack: false };
        apply_remote_keys(slot, input);
        poke(ptr + g::VEH_HORN, u32::from(d.keys & 2 != 0));
    }
}

/// The one-shot position apply of a new 200: trains through SET_MISSION_TRAIN_COORDINATES and the train speed,
/// other vehicles through the driver correction (nudge or teleport).
fn apply_driver_motion(runner: &mut Runner, r: &mut RemoteVeh, v: &Veh, m: u32, d: &DriverData) {
    let ptr = v.ptr;
    if vm::TRAIN_SYNC_MODELS.contains(&v.model) {
        let cur = v3(m + off::MATRIX_POS);
        if vm::train_needs_teleport(cur, d.position) {
            let a =
                [Arg::Int(v.handle), Arg::Float(d.position[0]), Arg::Float(d.position[1]), Arg::Float(d.position[2])];
            runner.run(op::SET_MISSION_TRAIN_COORDINATES, &a);
        }
        put_v3(ptr + off::MOVE_SPEED, d.velocity);
        let sp = d.extra_f32();
        if sp.abs() <= 100.0 {
            poke(ptr + g::TRAIN_SPEED, sp);
        }
    } else {
        put_v3(ptr + off::MOVE_SPEED, d.velocity);
        let cur = v3(m + off::MATRIX_POS);
        let moving = rd::<u32>(ptr + g::MOVING_LIST) != 0;
        match vm::correct_driver(cur, d.position, d.velocity, moving, vm::z_threshold(v.kind)) {
            VehCorrection::None => {}
            VehCorrection::Speed(s) => {
                r.corrections += 1;
                put_v3(ptr + off::MOVE_SPEED, s);
            }
            VehCorrection::Teleport => {
                r.teleports += 1;
                put_v3(m + off::MATRIX_POS, d.position);
                put_v3(ptr + off::MOVE_SPEED, d.velocity);
            }
        }
    }
}

/// The one-shot state apply of a new 200: Hydra nozzle or bike lean, siren, plane gear, health and weapon.
fn apply_driver_state(runner: &mut Runner, v: &Veh, ped: u32, slot: u8, d: &DriverData) {
    let ptr = v.ptr;
    if v.model == vm::MODEL_HYDRA {
        poke(ptr + g::HYDRA_NOZZLE, d.extra as u16);
        poke(ptr + g::HYDRA_NOZZLE + 2, d.extra as u16);
    } else if matches!(v.kind, Kind::Bike | Kind::Bmx) {
        poke(ptr + g::BIKE_DESIRED_LEAN, d.extra_f32().clamp(-1.0, 1.0));
    }
    set_bits_u8(ptr + g::VEH_SIREN_BYTE, g::VEH_SIREN_ON, d.siren);
    if v.kind == Kind::Plane {
        let up = rd::<f32>(ptr + g::PLANE_GEAR) != 0.0;
        if up != d.landing_gear {
            thiscall0(if d.landing_gear { g::PLANE_GEAR_UP } else { g::PLANE_GEAR_DOWN }, ptr);
        }
    }
    poke(ptr + g::VEH_HEALTH, d.vehicle_health);
    // Weapon, as on foot.
    crate::players::apply_weapon(runner, ped, slot, d.weapon);
}

/// Writes a remote player's keys into its shadow pad.
pub(super) fn apply_remote_keys(slot: u8, input: PadInput) {
    crate::swap::with_table(|t| {
        if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
            remote::apply_keys(&mut sh.pad, input);
        }
    });
}
