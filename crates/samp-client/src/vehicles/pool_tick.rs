//! The vehicle pool tick (every 4th frame): damage authority, wasted / sunk vehicles, unoccupied sync (209),
//! engine / lights, colours and blips, and the state the GTA hooks read.

use super::*;

/// What every vehicle of one pool tick needs to know about the players.
struct TickPeds {
    local: u32,
    local_pos: Option<[f32; 3]>,
    remote: Vec<(u16, u32)>,
}

impl Vehicles {
    /// The vehicle pool tick (every 4th frame): damage authority, wasted / sunk vehicles, unoccupied sync,
    /// engine/lights, colours and blips; publishes the hook state every frame. Mirrors samp.dll 0x1001F3C0.
    pub fn pool_tick(&mut self, runner: &mut Runner, net: &mut NetClient, players: &Players, now: Instant) {
        self.frame += 1;
        let local = local_ped();
        self.end_local_driving(local);
        let local_pos = if local != 0 { pos_of(local) } else { None };
        let remote_peds: Vec<(u16, u32)> = players.iter().filter(|p| p.ped != 0).map(|p| (p.id, p.ped)).collect();
        // The original runs this pool tick on every 4th frame only; a car created by RPC 164 is first processed on
        // the next such frame.
        if !self.frame.is_multiple_of(vm::POOL_TICK_FRAMES) {
            self.publish_hook_state(players);
            return;
        }
        let peds = TickPeds { local, local_pos, remote: remote_peds };
        let mut sends = 0;
        let ids: Vec<u16> = self.pool.keys().copied().collect();
        for id in ids {
            self.tick_vehicle(runner, net, id, &peds, &mut sends, now);
        }
        self.publish_hook_state(players);
    }

    /// "Last driven": also when the local driver dies in the car (the sync tick does not run while dead).
    fn end_local_driving(&mut self, local: u32) {
        if let Some(id) = self.driving
            && self.get(id).is_none_or(|v| local == 0 || driver_of(v.ptr) != local || dead(local))
        {
            self.driving = None;
            self.last_driven = Some(id);
        }
    }

    /// One vehicle of the pool tick.
    fn tick_vehicle(
        &mut self,
        runner: &mut Runner,
        net: &mut NetClient,
        id: u16,
        peds: &TickPeds,
        sends: &mut u32,
        now: Instant,
    ) {
        let Some(v) = self.pool.get(&id) else { return };
        if v.carriage || v.ptr == 0 {
            return;
        }
        let ptr = vehicle_at(v.handle);
        if ptr == 0 || is_deleted(ptr) {
            return;
        }
        if let Some(t) = v.wasted_at
            && now.duration_since(t) >= vm::WASTED_DELETE
        {
            self.delete(runner, id);
            return;
        }
        let h = v.handle;
        let local = peds.local;
        let driver = driver_of(ptr);
        let local_drives = local != 0 && driver == local;
        let local_passenger = local != 0 && !local_drives && is_passenger(ptr, local);
        damage_authority(runner, h, local_drives, local_passenger);
        let pos = pos_of(ptr);
        let dist = match (pos, peds.local_pos) {
            (Some(a), Some(b)) => vm::dist3d(a, b),
            _ => f32::MAX,
        };
        let kind = kind_of(ptr);
        if rd::<f32>(ptr + g::VEH_HEALTH) == 0.0 {
            if v.wasted_at.is_none() {
                self.vehicle_death(net, id);
                if let Some(v) = self.pool.get_mut(&id) {
                    v.wasted_at = Some(now);
                }
            }
            return;
        }
        // Sunk check, not for boats, within 200 units.
        if kind != Kind::Boat && dist <= vm::NEAR_RANGE && runner.run(op::IS_CAR_IN_WATER, &[Arg::Int(h)]) {
            self.vehicle_death(net, id);
            return;
        }
        // Unoccupied sync.
        if *sends < vm::UNOCC_SENDS_PER_TICK
            && let Some(p) = pos
            && self.unoccupied_send(net, id, ptr, p, dist, &peds.remote, now)
        {
            *sends += 1;
        }
        self.engine_lights(id, ptr, driver);
        let Some(v) = self.pool.get(&id) else { return };
        if let Some((c1, c2)) = v.colours
            && !v.in_mod_shop
        {
            poke(ptr + g::VEH_PRIMARY_COLOUR, c1);
            poke(ptr + g::VEH_SECONDARY_COLOUR, c2);
        }
        self.blips(runner, id, driver != 0 || local_passenger, dist);
    }

    fn vehicle_death(&mut self, net: &mut NetClient, id: u16) {
        if self.last_driven == Some(id) {
            self.last_driven = None;
            send(net, &rv::VehicleDeath { vehicle_id: id });
            self.n.sent_136 += 1;
            crate::log!("vehicles: vehicle {id} death reported");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn unoccupied_send(
        &mut self,
        net: &mut NetClient,
        id: u16,
        ptr: u32,
        pos: [f32; 3],
        local_dist: f32,
        remote_peds: &[(u16, u32)],
        now: Instant,
    ) -> bool {
        let Some(v) = self.pool.get(&id) else { return false };
        if vm::is_rail(v.model) || v.last_unocc_send.is_some_and(|t| now.duration_since(t) <= vm::UNOCC_SEND_INTERVAL) {
            return false;
        }
        let tractor: u32 = rd(ptr + g::VEH_TRACTOR);
        let towed = tractor != 0 && driver_of(tractor) != 0;
        let driver = driver_of(ptr);
        let local = local_ped();
        let speed = v3(ptr + off::MOVE_SPEED);
        // Not "stopped": stopped = in the moving list with a zero move speed.
        let moving = rd::<u32>(ptr + g::MOVING_LIST) == 0 || speed != [0.0; 3];
        let first = first_player_passenger(ptr, local);
        let has_driver = driver != 0 && in_vehicle(driver);
        let closest = first.is_none() && self.local_is_closest(pos, local, remote_peds);
        let Some(seat) = vm::unoccupied_seat(has_driver, towed, moving, local_dist, first, closest) else {
            return false;
        };
        let m = matrix(ptr);
        if m == 0 {
            return false;
        }
        let u = vm::build_unoccupied_sync(
            id,
            seat,
            &rows_of(m),
            pos,
            speed,
            v3(ptr + g::TURN_SPEED),
            rd(ptr + g::VEH_HEALTH),
        );
        net.send_sync(&u);
        self.n.sent_209 += 1;
        if let Some(v) = self.pool.get_mut(&id) {
            v.last_unocc_send = Some(now);
            v.last_unocc_sender = self.local_id;
        }
        true
    }

    /// The local player is the closest player to `pos` among all players whose peds are in the world.
    fn local_is_closest(&self, pos: [f32; 3], local: u32, remote_peds: &[(u16, u32)]) -> bool {
        // Peds count while in the world: not a deleted placeable, in the moving list.
        let dist_of = |ped: u32| {
            (ped != 0 && !is_deleted(ped) && rd::<u32>(ped + g::MOVING_LIST) != 0)
                .then(|| pos_of(ped).map(|p| vm::dist3d(p, pos)))
                .flatten()
        };
        let local_id = self.local_id.unwrap_or(0);
        let mut all: Vec<(u16, Option<f32>)> =
            remote_peds.iter().filter(|(pid, _)| *pid != local_id).map(|&(pid, ped)| (pid, dist_of(ped))).collect();
        all.push((local_id, dist_of(local)));
        all.sort_by_key(|e| e.0);
        vm::local_is_closest(local_id, all)
    }

    fn engine_lights(&mut self, id: u16, ptr: u32, driver: u32) {
        let Some(v) = self.pool.get(&id) else { return };
        let flags: u8 = rd(ptr + g::VEH_FLAGS);
        let engine = vm::engine_wanted(self.manual, v.engine, driver != 0 && in_vehicle(driver));
        let mut f = if engine { flags | g::VEH_FLAG_ENGINE_ON } else { flags & !g::VEH_FLAG_ENGINE_ON };
        if let Some(on) = vm::lights_wanted(self.manual, v.lights) {
            f = if on { f | g::VEH_FLAG_LIGHTS_ON } else { f & !g::VEH_FLAG_LIGHTS_ON };
            set_bits_u8(ptr + g::VEH_LIGHTS_FLAGS, g::VEH_LIGHTS_FORCED_OFF, !on);
        }
        if f != flags {
            poke(ptr + g::VEH_FLAGS, f);
        }
    }

    /// Objective / unoccupied radar blips. "Occupied" = a driver ped is set or the local ped sits in it as a
    /// passenger. Remote passengers in a driverless car have not been measured. Mirrors samp.dll 0x100B7720.
    fn blips(&mut self, runner: &mut Runner, id: u16, occupied: bool, dist: f32) {
        let Some(v) = self.pool.get_mut(&id) else { return };
        let h = v.handle;
        if v.objective {
            if let Some(b) = v.blip.take() {
                runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
            }
            if v.objective_blip.is_none() {
                runner.run(op::ADD_BLIP_FOR_CAR_OLD, &[Arg::Int(h), Arg::Int(1), Arg::Int(3), Arg::Local(0)]);
                let b = runner.local_i32(0);
                runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(b), Arg::Int(vm::BLIP_COLOUR_OBJECTIVE)]);
                runner.run(op::CHANGE_BLIP_SCALE, &[Arg::Int(b), Arg::Int(3)]);
                v.objective_blip = Some(b);
            }
            return;
        }
        if let Some(b) = v.objective_blip.take() {
            runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
        }
        let want = !occupied && dist <= vm::NEAR_RANGE;
        match (want, v.blip) {
            (true, None) => {
                runner.run(op::ADD_BLIP_FOR_CAR_OLD, &[Arg::Int(h), Arg::Int(1), Arg::Int(2), Arg::Local(0)]);
                let b = runner.local_i32(0);
                runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(b), Arg::Int(vm::BLIP_COLOUR_UNOCCUPIED)]);
                v.blip = Some(b);
            }
            (false, Some(b)) => {
                runner.run(op::REMOVE_BLIP, &[Arg::Int(b)]);
                v.blip = None;
            }
            _ => {}
        }
    }

    /// Mirrors what the GTA hooks need (ptr → id, lock, siren, trailer, NPC peds).
    fn publish_hook_state(&self, players: &Players) {
        let by_ptr: HashMap<u32, VehInfo> = self
            .pool
            .values()
            .filter(|v| v.ptr != 0)
            .map(|v| {
                (
                    v.ptr,
                    VehInfo {
                        id: v.id,
                        model: v.model,
                        locked: v.locked,
                        siren: v.siren,
                        has_trailer: v.trailer.is_some(),
                    },
                )
            })
            .collect();
        let npc: Vec<u32> = players.iter().filter(|p| p.npc && p.ped != 0).map(|p| p.ped).collect();
        vehhooks::with_shared(|s| {
            s.by_ptr = by_ptr;
            s.npc_peds = npc;
        });
    }
}

/// Damage authority: proofs + tyres pair on every pool tick (only the local driver's car is vulnerable); none
/// at all for the car the local ped sits in as a passenger.
fn damage_authority(runner: &mut Runner, h: i32, local_drives: bool, local_passenger: bool) {
    if local_passenger {
        // no pair
    } else if local_drives {
        runner.run(op::SET_CAR_PROOFS, &[Arg::Int(h), Arg::Int(0), Arg::Int(0), Arg::Int(0), Arg::Int(0), Arg::Int(0)]);
        runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(h), Arg::Int(1)]);
    } else {
        runner.run(op::SET_CAR_PROOFS, &[Arg::Int(h), Arg::Int(1), Arg::Int(1), Arg::Int(1), Arg::Int(1), Arg::Int(1)]);
        runner.run(op::SET_CAN_BURST_CAR_TYRES, &[Arg::Int(h), Arg::Int(0)]);
    }
}

/// The first passenger seat held by a player ped (`m_nPedType` == 0, local or remote; AI peds do not count):
/// (is the local ped, seat index).
fn first_player_passenger(veh: u32, local: u32) -> Option<(bool, u8)> {
    (0..8u32).find_map(|i| {
        let p = passenger_at(veh, i);
        (p != 0 && rd::<u32>(p + g::PED_TYPE) == 0).then_some((p == local, i as u8))
    })
}
