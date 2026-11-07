//! The spawned part of the local tick: the per-frame script commands, the sync senders (205 / 204, the in-car
//! packets, 207 / 203), local surfing, the head look and the camera-target reporting.

use std::time::Instant;

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::player;
use samp_proto::sync::player::StatsSync;

use super::gta::{self, dist3, entity_matrix, entity_pos, read_vec3, surf_probe, tick_count};
use super::{CAMERA_FRONT, Client, PEDSTATE_DEAD, PEDSTATE_DIE, PEDSTATE_DRIVING, PEDSTATE_IDLE, PEDSTATE_SNIPER};
use crate::footsync;
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, op};

/// `CPed::RemoveWeaponWhenEnteringVehicle(int)` (thiscall; gta-reversed `Ped.cpp`).
const PED_REMOVE_WEAPON_ENTERING_VEHICLE: usize = 0x5E6370;
/// `CControllerState::ButtonTriangle` in `CPad::Pads[0].NewState` (samp's pad +0x1E).
const PAD_TRIANGLE: u32 = 0x1E;

/// Where the local ped sits: the in-vehicle flag, its vehicle and whether it drives it.
#[derive(Clone, Copy)]
struct Seat {
    in_vehicle: bool,
    vehicle: u32,
    driver: bool,
}

impl Seat {
    fn read(ped: u32) -> Self {
        let mem = ProcessMemory;
        let in_vehicle = gta::ped_in_vehicle(ped);
        let vehicle = mem.read_u32(ped + off::PED_VEHICLE).unwrap_or(0);
        let driver = in_vehicle && vehicle != 0 && mem.read_u32(vehicle + off::VEHICLE_DRIVER).unwrap_or(0) == ped;
        Self { in_vehicle, vehicle, driver }
    }

    /// In a vehicle (flag set and a vehicle pointer).
    fn seated(self) -> bool {
        self.in_vehicle && self.vehicle != 0
    }
}

impl Client {
    /// The sync part of the local tick, in samp's order: spectating, or the surf probe, head look, 205 / 204,
    /// passenger drive-by, the driving branch, the in-car packets, and on foot the 207 and then the 203.
    pub(super) fn sync_tick(&mut self, now: Instant) {
        crate::behaviourclient::publish_bounds(self.spawned.then_some(self.behaviour.bounds));
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let mem = ProcessMemory;
        let state = mem.read_u32(ped + off::PED_STATE).unwrap_or(0);
        if state == PEDSTATE_DIE || state == PEDSTATE_DEAD || mem.read_f32(ped + off::PED_HEALTH).unwrap_or(0.0) <= 0.0
        {
            return;
        }
        // Spectating: the spectator tick replaces the on-foot / driver branches.
        if self.behaviour.spectate.on {
            self.spectator_tick(now);
            self.stats_tick(now);
            return;
        }
        // On foot: the surf probe, before the head look.
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER) {
            self.surf_step(ped);
        }
        // Head look in the on-foot, driver and passenger branches of the local tick. `disableheadmove` clears
        // samp's Game+0x59 flag, which the head look tests first (set at boot and by `/headmove`).
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER | PEDSTATE_DRIVING) && !crate::ui::head_move_disabled() {
            self.head_look(ped, now);
        }
        // 205 (> 1000 ms) and 204 (every tick) come before the per-state packets.
        self.stats_tick(now);
        self.combat.weapons_tick(&mut self.net, &self.players, ped, now);
        self.combat.cuffed_tick(&mut self.runner, ped);
        // Local passenger drive-by, before the 211.
        let seat = Seat::read(ped);
        if seat.seated() && !seat.driver {
            self.combat.passenger_drive_by(&mut self.runner, ped);
        } else if self.combat.drive_by {
            self.combat.drive_by = false;
        }
        // Driving: the world bounds on the vehicle, before the driver packets.
        if state == PEDSTATE_DRIVING {
            self.driving_frame(ped, seat);
        }
        // In-car packets (200/210/211/106, passenger poll); on-foot sync only in ped states 1 and 12, so nothing
        // is sent while entering a seat from the server's warp or during the exit animation.
        self.vehicles.local_drive_by = self.combat.drive_by;
        self.vehicles.local_cuffed = self.combat.cuffs.cuffed;
        let in_car = self.vehicles.local_tick(&mut self.runner, &mut self.net, self.players.streamed(), now);
        if in_car && seat.seated() {
            self.combat.aim_in_car(&mut self.net, &mut self.vehicles, ped, seat.driver, now);
        }
        if !in_car && state != PEDSTATE_DRIVING {
            self.on_foot_sync(ped, now);
        }
    }

    /// The driving branch: unless the ped is a passenger (or sits in a 569/570 train carriage),
    /// `CPed::RemoveWeaponWhenEnteringVehicle(0)` every driving frame; then the world bounds on the vehicle.
    fn driving_frame(&mut self, ped: u32, seat: Seat) {
        let model =
            if seat.vehicle != 0 { ProcessMemory.read_u16(seat.vehicle + off::ENTITY_MODEL).unwrap_or(0) } else { 0 };
        if !(seat.seated() && (!seat.driver || matches!(model, 569 | 570))) {
            // SAFETY: GTA's thiscall method on the live local ped, main thread.
            unsafe {
                let f: extern "thiscall" fn(u32, i32) = std::mem::transmute(PED_REMOVE_WEAPON_ENTERING_VEHICLE);
                f(ped, 0);
            }
        }
        self.behaviour.bounds_driver(&mut self.runner, &self.vehicles, ped);
    }

    /// On foot: the world-bounds check, the 207 when due, then the 203.
    fn on_foot_sync(&mut self, ped: u32, now: Instant) {
        self.behaviour.bounds_on_foot(&mut self.runner, ped);
        match game::read_local_sample(&ProcessMemory, ped) {
            Ok(sample) => {
                let keys = footsync::key_word(&sample.pad, sample.key_ctx);
                let interval = footsync::on_foot_interval(self.on_foot_rate, self.players.streamed());
                if self.sender.due(now, interval, keys, sample.pad.left_stick_x, sample.pad.left_stick_y) {
                    self.send_foot_sync(ped, sample, now);
                }
                // 203 after the 207.
                self.combat.aim_on_foot(&mut self.net, &mut self.sender, ped, keys, now);
            }
            Err(e) => crate::log!("client: cannot read local ped: {e}"),
        }
    }

    /// The 207 builder: weapon, special action, animation, surf fields, then the send.
    fn send_foot_sync(&mut self, ped: u32, sample: footsync::LocalSample, now: Instant) {
        // The 207 builder reads the weapon with GET_CURRENT_CHAR_WEAPON.
        self.current_weapon(ped);
        let mut sample = sample;
        // samp's order: jetpack, dance, hands up, phone, piss, duck.
        sample.special_action =
            crate::specials::local_report(sample.special_action, self.phone, self.specials.piss, self.specials.item);
        sample.animation = self.behaviour.anim.take();
        if let Some(o) = crate::surf::take_measured() {
            self.surf.offset = o;
        }
        (sample.surf_offset, sample.surf_id) = self.surf.sync_fields();
        let f = footsync::build_foot_sync(&sample, self.sender.last_rotation());
        if let Some(f) = self.sender.offer(now, f) {
            self.net.send_sync(&f);
        }
    }

    /// 205 every > 1000 ms: money and the drunk level.
    pub(super) fn stats_tick(&mut self, now: Instant) {
        if self.last_stats.is_none_or(|t| now.duration_since(t) > footsync::STATS_INTERVAL) {
            self.last_stats = Some(now);
            let money = ProcessMemory.read_i32(game::PLAYER_MONEY).unwrap_or(0);
            self.net.send_sync(&StatsSync { money, drunk_level: self.behaviour.drunk });
        }
    }

    /// The script commands samp's local tick issues every frame while spawned, before the sync sends
    /// (052C, 09BA, 0470, 077E; docs/phase3/trace-notes.md). Mirrors samp.dll 0x100074C0.
    pub(super) fn local_frame(&mut self, now: Instant) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        // `/ctd` labels, early in the spawned branch of the local tick.
        if crate::ui::camera_target_labels_on() {
            self.draw_camera_target_labels();
        }
        // Hands up + Triangle (enter/exit key) → control on again, which re-sets the ped to its own position and so
        // ends the task.
        let triangle = ProcessMemory.read_u16(game::PAD0 + PAD_TRIANGLE).unwrap_or(0);
        if triangle != 0 && self.hands_up(ped) {
            self.set_controllable(true);
        }
        // Piss / held item: Triangle or a vehicle ends them, else the item is used.
        crate::specials::local_tick(
            &mut self.runner,
            ped,
            &mut self.specials,
            triangle != 0,
            &mut self.behaviour.drunk,
        );
        // Drunk handling, then the 207 animation encoder.
        self.behaviour.drunk_tick(&mut self.runner, ped);
        self.behaviour.anim_tick(ped);
        if self.zone_names_from.is_some_and(|t| now >= t) {
            self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(i32::from(self.zone_names))]);
        }
        // Weapons-update check (the 204 packet itself goes out from `sync_tick`).
        self.current_weapon(ped);
        // Interior change → RPC 118.
        self.runner.run(op::GET_AREA_VISIBLE, &[Arg::Local(0)]);
        let area = self.runner.local_i32(0) as u8;
        if area != self.last_interior {
            self.last_interior = area;
            crate::log!("client: interior {area} -> RPC 118");
            self.net.send_rpc(&player::OnPlayerInteriorChange { interior: area }, Reliability::Reliable);
        }
        // Camera target, right after the interior check.
        self.camera_target_tick();
    }

    /// The surf probe and state step, on foot. Mirrors samp.dll 0x10003730.
    fn surf_step(&mut self, ped: u32) {
        use crate::surf::{ProbeAction, Target};
        if let Some(o) = crate::surf::take_measured() {
            self.surf.offset = o;
        }
        let hit = surf_probe(ped);
        let contact = self.surf_contact(ped, hit);
        let cancel = crate::surf::cancel_tasks(ped);
        let now_ms = tick_count();
        let (vehicles, objects) = (&self.vehicles, &self.objects);
        let exists = |t: Target| match t {
            Target::Vehicle(id) => vehicles.gta_ptr(id).is_some(),
            Target::Object(id) => objects.gta_ptr(id).is_some_and(|p| p != 0),
        };
        if self.surf.probe(contact, cancel, now_ms, exists) == ProbeAction::MeasureOffset {
            if let Some(surface) = self.surf_entity(self.surf.target)
                && let Some(o) = crate::surf::local_glue(ped, surface, self.surf.offset, false)
            {
                self.surf.offset = o;
            }
            self.surf.latch_after_measure();
        }
    }

    /// The surf probe's hit as a contact: a player-driven (or 569/570) pool vehicle close enough, or a moving
    /// pool object.
    fn surf_contact(&self, ped: u32, hit: u32) -> crate::surf::Contact {
        use crate::surf::Contact;
        let mut contact = Contact::None;
        if hit != 0 {
            match self.vehicles.id_of(hit) {
                Some(id) if id != 0 && id != 0xFFFF => {
                    if id < 2000 {
                        let model = self.vehicles.model_of(id).unwrap_or(0);
                        let ok = crate::surf::player_driven(hit) || matches!(model, 569 | 570);
                        if ok
                            && let (Some(a), Some(b)) = (entity_pos(hit), entity_pos(ped))
                            && dist3(a, b) < crate::surf::MAX_VEHICLE_DISTANCE
                        {
                            contact = Contact::Vehicle(id);
                        }
                    }
                }
                _ => {
                    if let Some(id) = self.objects.id_of_entity(hit)
                        && self.objects.moving(id)
                    {
                        contact = Contact::Object(id);
                    }
                }
            }
        }
        contact
    }

    fn surf_entity(&self, t: Option<crate::surf::Target>) -> Option<u32> {
        use crate::surf::Target;
        match t? {
            Target::Vehicle(id) => self.vehicles.gta_ptr(id),
            Target::Object(id) => self.objects.gta_ptr(id).filter(|&p| p != 0),
        }
    }

    /// The combat hooks' view of the pools and the local surf state (written before GTA runs and before a remote
    /// shot is fired).
    pub(super) fn update_combat_shared(&self) {
        let objects = self.objects.pick_list().into_iter().map(|(id, ptr, _)| (ptr, id)).collect();
        // The surf state's surface as a GTA entity (pool id → wrapper → entity), also when that entity is missing.
        let surface = self.surf.target.map(|t| match t {
            crate::surf::Target::Vehicle(id) => self.vehicles.gta_ptr(id).unwrap_or(0),
            crate::surf::Target::Object(id) => self.objects.gta_ptr(id).unwrap_or(0),
        });
        self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors(), objects, surface);
    }

    /// Hands the surfing hook this frame's surfaces (local and remote). Mirrors samp.dll 0x100A4990.
    pub(super) fn surf_publish(&self) {
        let local = if self.spawned {
            self.surf_entity(self.surf.target).map(|e| (e, self.surf.offset, self.surf.glue))
        } else {
            None
        };
        let mut remotes = Vec::new();
        for p in self.players.iter() {
            if p.ped == 0 || p.state != crate::remote::state::ON_FOOT {
                continue;
            }
            let Some(f) = p.foot else { continue };
            if let Some(t) = crate::surf::remote_target(f.surf_id, f.surf_offset)
                && let Some(e) = self.surf_entity(Some(t))
            {
                remotes.push((p.ped, e, f.surf_offset));
            }
        }
        crate::surf::publish(local, remotes);
    }

    /// RPC 168 sender (camtarget.rs).
    fn camera_target_tick(&mut self) {
        let now_ms = tick_count();
        let send = if self.cam_target.enabled {
            let objects = self.objects.pick_list();
            let vehicles = self.vehicles.hook_vehicles();
            let peds: Vec<(u32, u16)> = self.players.iter().filter(|p| p.ped != 0).map(|p| (p.ped, p.id)).collect();
            let actors = self.entities.hook_actors();
            self.cam_target.tick(now_ms, || gta::camera_targets(&objects, &vehicles, &peds, &actors))
        } else {
            self.cam_target.tick(now_ms, || [crate::camtarget::NONE; 4])
        };
        if let Some([object_id, vehicle_id, player_id, actor_id]) = send {
            crate::log!("client: RPC 168 object {object_id} vehicle {vehicle_id} player {player_id} actor {actor_id}");
            self.net.send_rpc(
                &samp_proto::rpcs::session::OnPlayerCameraTarget { object_id, vehicle_id, player_id, actor_id },
                Reliability::ReliableOrdered,
            );
        }
    }

    /// `/ctd`: the cached targets whose entities exist, with their matrices.
    fn draw_camera_target_labels(&self) {
        let [o, v, p, a] = self.cam_target.cached;
        let none = crate::camtarget::NONE;
        let objects = self.objects.pick_list();
        let actors = self.entities.hook_actors();
        let ents = [
            (o != none).then(|| objects.iter().find(|x| x.0 == o).map(|x| x.1)).flatten(),
            (v != none).then(|| self.vehicles.ptr_of(v)).flatten(),
            (p != none).then(|| self.players.get(p).map(|r| r.ped).filter(|&e| e != 0)).flatten(),
            (a != none).then(|| actors.iter().find(|x| x.1 == a).map(|x| x.0)).flatten(),
        ];
        let items: Vec<([f32; 16], Vec<u8>)> = ents
            .iter()
            .zip(self.cam_target.cached)
            .enumerate()
            .filter_map(|(k, (e, id))| {
                let m = entity_matrix((*e)?)?;
                Some((m, crate::camtarget::label(k, id).into_bytes()))
            })
            .collect();
        crate::ui::draw_camera_target_labels(&items);
    }

    /// The local head follows the camera (unless `disableheadmove`): every 1000 ms the offset := 20 × camera front,
    /// every 200 ms `LookAt` at position + offset for 2000 ms. Mirrors samp.dll 0x10002FA0.
    fn head_look(&mut self, ped: u32, now: Instant) {
        let matrix = ProcessMemory.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
        if matrix == 0 {
            return;
        }
        if self.look_refresh.is_none_or(|t| now.duration_since(t) > crate::remote::LOOK_REFRESH) {
            self.look_refresh = Some(now);
            self.look_offset = read_vec3(CAMERA_FRONT).map(|c| c * crate::remote::LOOK_DISTANCE);
        }
        if self.look_last.is_none_or(|t| now.duration_since(t) > crate::remote::LOOK_INTERVAL) {
            self.look_last = Some(now);
            let pos = read_vec3(matrix + off::MATRIX_POS);
            let mut target = [0, 1, 2].map(|i| pos[i] + self.look_offset[i]);
            crate::players::look_at(ped, 2000, &mut target);
        }
    }
}
