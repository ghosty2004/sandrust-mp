//! Colours, components, paintjobs and the mod shop: RPC 96 SCMEvent in both directions and the passenger who
//! follows its driver into a mod shop.

use super::*;

impl Vehicles {
    pub(super) fn set_colours(&mut self, id: u16, c1: u8, c2: u8) {
        if let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) {
            poke(v.ptr + g::VEH_PRIMARY_COLOUR, c1);
            poke(v.ptr + g::VEH_SECONDARY_COLOUR, c2);
            v.colours = Some((c1, c2));
        }
    }

    /// RPC 96 SCMEvent (S→C): paintjob, component, colours or the mod-shop follow.
    pub(super) fn scm_event(&mut self, runner: &mut Runner, players: &Players, e: &rv::ScmEvent) {
        if !self.spawned_once || e.vehicle_id >= u32::from(vm::MAX_VEHICLES) {
            return;
        }
        let id = e.vehicle_id as u16;
        let Some(v) = self.get(id) else { return };
        let (h, kind) = (v.handle, v.kind);
        match e.event_type {
            1 if kind == Kind::Automobile && e.arg1 < 4 => {
                runner.run(op::GIVE_VEHICLE_PAINTJOB, &[Arg::Int(h), Arg::Int(e.arg1 as i32)]);
            }
            2 if kind == Kind::Automobile => add_component(runner, h, e.arg1 as i32),
            3 => self.set_colours(id, e.arg1 as u8, e.arg2 as u8),
            4 => self.mod_shop_follow(runner, players, e),
            _ => {}
        }
    }

    /// RPC 96 type 4: the driver `player_id` entered (arg1 ≠ 0) or left a mod shop; the local player riding along
    /// as a passenger in the same vehicle follows into interior arg2 and is frozen, or back to interior 0 and
    /// controllable. The packet's vehicle id only has to exist.
    fn mod_shop_follow(&mut self, runner: &mut Runner, players: &Players, e: &rv::ScmEvent) {
        if e.player_id > vm::MAX_REMOTE_PLAYER_ID {
            return;
        }
        let Some(rp) = players.get(e.player_id).filter(|p| p.ped != 0) else { return };
        let local = local_ped();
        if local == 0 || !vm::mod_shop_follow_applies(seat_of(rp.ped), seat_of(local)) {
            return;
        }
        let Some(h) = ped_handle(local) else { return };
        let (veh_interior, interior, controllable) = vm::mod_shop_follow(e.arg1, e.arg2);
        // The remote's vehicle follows first (LINK_VEHICLE_TO_INTERIOR when the vehicle exists in GTA).
        if let Some(v) = self.remote_vehicle(e.player_id).and_then(|id| self.get(id)).filter(|v| v.ptr != 0) {
            runner.run(op::SET_VEHICLE_AREA_VISIBLE, &[Arg::Int(v.handle), Arg::Int(veh_interior)]);
        }
        // Local interior without the scene refresh.
        runner.run(op::SET_AREA_VISIBLE, &[Arg::Int(i32::from(interior))]);
        runner.run(op::SET_CHAR_AREA_VISIBLE, &[Arg::Int(h), Arg::Int(i32::from(interior))]);
        // Controllable: the ped is seated, so samp's "teleport to its own position" step never runs.
        runner.run(op::SET_PLAYER_CONTROL, &[Arg::Int(0), Arg::Int(i32::from(controllable))]);
        runner.run(op::FREEZE_CHAR_POSITION, &[Arg::Int(h), Arg::Int(i32::from(!controllable))]);
        crate::log!(
            "vehicles: mod-shop follow (driver {}, arg1 {}): interior {interior}, controllable {controllable}",
            e.player_id,
            e.arg1
        );
    }

    /// RPC 96 SCMEvent (C→S): only for a vehicle of the pool; type 3 then applies the colours locally, type 4
    /// sets / clears the "in mod shop" flag (on leaving, the vehicle's current GTA colours become its stored
    /// colours).
    pub(super) fn scm_event_out(&mut self, net: &mut NetClient, handle: i32, arg1: i32, arg2: i32, kind: i32) {
        let ptr = vehicle_at(handle);
        let Some(id) = self.id_of_ptr(ptr) else {
            crate::log!("vehicles: SCM event type {kind} for handle {handle}: not a pool vehicle, not sent");
            return;
        };
        crate::log!("vehicles: RPC 96 vehicle {id} arg1 {arg1} arg2 {arg2} type {kind}");
        send(
            net,
            &rv::OnPlayerScmEvent {
                vehicle_id: u32::from(id),
                arg1: arg1 as u32,
                arg2: arg2 as u32,
                event_type: kind as u32,
            },
        );
        self.n.sent_96 += 1;
        match kind {
            3 => self.set_colours(id, arg1 as u8, arg2 as u8),
            4 => {
                let Some(v) = self.pool.get_mut(&id).filter(|v| v.ptr != 0) else { return };
                if arg1 == 0 {
                    let (c1, c2) = (rd::<u8>(v.ptr + g::VEH_PRIMARY_COLOUR), rd::<u8>(v.ptr + g::VEH_SECONDARY_COLOUR));
                    v.in_mod_shop = false;
                    self.set_colours(id, c1, c2);
                } else {
                    v.in_mod_shop = true;
                }
            }
            _ => {}
        }
    }
}

/// Add-component (samp's vehicle method, used by stream-in and SCM event 2): not loaded → `REQUEST_MODEL`,
/// `LOAD_ALL_MODELS_NOW`, `REQUEST_VEHICLE_MOD`, then up to 20 polls with `Sleep(1)`; if loaded, `ADD_VEHICLE_MOD`.
pub(super) fn add_component(runner: &mut Runner, handle: i32, component: i32) {
    if !runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
        runner.run(op::REQUEST_MODEL, &[Arg::Int(component)]);
        runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
        runner.run(op::REQUEST_VEHICLE_MOD, &[Arg::Int(component)]);
        for _ in 0..20 {
            if runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    if runner.run(op::HAS_VEHICLE_MOD_LOADED, &[Arg::Int(component)]) {
        runner.run(op::ADD_VEHICLE_MOD, &[Arg::Int(handle), Arg::Int(component), Arg::Local(0)]);
    }
}
