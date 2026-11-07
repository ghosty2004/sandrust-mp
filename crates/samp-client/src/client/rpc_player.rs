//! The S→C player RPCs (docs/re/subsystems/player-state.md): a dispatch `match` with one handler per RPC.

use samp_proto::rpcs::player::{self, Incoming as P};

use super::gta::poke;
use super::{Client, PED_FLAG_IN_VEHICLE};
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, op};

/// Start of the on-foot suspect description in `AddAudioEvent`, NOPed by RPC 112 …
const CRIME_NOP_ADDR: u32 = 0x4E7529;
/// … for this many bytes.
const CRIME_NOP_LEN: usize = 39;
/// `CAEPoliceScannerAudioEntity::AddAudioEvent(eAudioEvents, eCrimeType, const CVector&)`.
const SCANNER_ADD_AUDIO_EVENT: u32 = 0x4E71E0;
/// `CWanted`'s police-scanner audio entity.
const WANTED_SCANNER: u32 = 0x21C;

impl Client {
    pub(super) fn on_player_rpc(&mut self, p: P) {
        use crate::behaviourclient as bc;
        match p {
            P::PlayerRequestClassResponse(c) => self.on_class_response(c),
            P::SetSpawnInfo(s) => self.store_spawn_info(s.spawn),
            P::PlayerRequestSpawnResponse(s) => self.on_spawn_response(s.response),
            P::SetPlayerCameraTargeting(t) => self.cam_target.enabled = t.enabled,
            P::SendGameTimeUpdate(t) => self.on_game_time(t.time),
            // RPC 48: stored in the custom-model manager, which R4 never creates
            // (docs/re/subsystems/session-misc.md, "Custom-model download machinery"), so nothing happens; the
            // value is only logged.
            P::SetPlayerVirtualWorld(w) => crate::log!("client: RPC 48 virtual world {} (no effect in R4)", w.world_id),
            P::PlayCrimeReport(c) => self.crime_report(&c),
            P::ForcePlayerClassSelection(_) => {
                crate::log!("client: server forces class selection on next death");
                self.force_class_selection = true;
            }
            P::SetPlayerPosition(s) => self.on_set_position(s.pos),
            P::SetPlayerPositionFindZ(s) => self.on_set_position_find_z(s.pos),
            P::SetPlayerFacingAngle(a) => self.set_facing(a.angle),
            P::SetPlayerHealth(h) => self.poke_local_ped(off::PED_HEALTH, h.health),
            P::SetPlayerArmour(a) => self.poke_local_ped(off::PED_ARMOUR, a.armour),
            P::TogglePlayerControllable(t) => self.set_controllable(t.enable != 0),
            P::SetPlayerCameraPosition(c) => {
                self.behaviour.camera_position(&mut self.runner, [c.pos.x, c.pos.y, c.pos.z])
            }
            P::SetPlayerCameraLookAt(c) => {
                self.behaviour.camera_look_at(&mut self.runner, [c.pos.x, c.pos.y, c.pos.z], c.cut)
            }
            P::SetPlayerCameraBehindPlayer(_) => self.camera_behind_player(),
            P::InterpolateCamera(c) => self.on_interpolate_camera(&c),
            P::AttachCameraToObject(a) => self.behaviour.attach_camera(&mut self.runner, &self.objects, a.object_id),
            P::ToggleWidescreen(w) => bc::widescreen(&mut self.runner, w.enable),
            P::SetWorldBounds(b) => {
                self.behaviour.bounds =
                    crate::behaviour::Bounds { max_x: b.max_x, min_x: b.min_x, max_y: b.max_y, min_y: b.min_y }
            }
            P::SetPlayerGravity(g) => bc::set_gravity(g.gravity),
            P::SetPlayerInterior(i) => bc::set_interior(&mut self.runner, i.interior),
            P::ApplyPlayerAnimation(a) => {
                let local = self.net.player_id();
                self.behaviour.apply_animation(&mut self.runner, &self.players, local, &a)
            }
            P::ClearPlayerTasks(c) => {
                let local = self.net.player_id();
                self.behaviour.clear_animations(&self.players, local, c.player_id)
            }
            P::SetPlayerDrunkLevel(d) => self.behaviour.drunk = d.level,
            P::SetPlayerFightingStyle(f) => self.on_fighting_style(f.player_id, f.style),
            P::SetPlayerShopName(n) => bc::shop_name(&n.name),
            P::SetPlayerWantedLevel(w) => bc::set_wanted(w.level),
            P::EnableStuntBonusForPlayer(e) => bc::set_stunt_bonus(e.enable),
            P::GivePlayerMoney(m) => bc::give_money(&mut self.runner, m.amount),
            P::ResetPlayerMoney(_) => bc::reset_money(&mut self.runner),
            P::SetPlayerVelocity(v) => bc::set_velocity([v.velocity.x, v.velocity.y, v.velocity.z]),
            P::GivePlayerWeapon(g) => self.on_give_weapon(g.weapon, g.ammo),
            P::SetPlayerAmmo(a) => {
                crate::log!("client: RPC 145 weapon {} ammo {}", a.weapon, a.ammo);
                crate::combatclient::set_ammo(self.ped(), a.weapon, a.ammo)
            }
            P::SetPlayerSkillLevel(k) => self.on_skill_level(k.player_id, k.skill_type, k.level),
            P::SetPlayerTeam(t) => self.on_set_team(t.player_id, t.team),
            P::PlayerDeath(d) => self.on_player_death(d.player_id),
            P::CreateExplosion(e) => crate::combatclient::create_explosion(&mut self.runner, &e),
            P::SetPlayerTime(t) => self.on_set_time(t.hour, t.minute),
            P::SetPlayerWorldTime(t) => self.world.hour = t.hour,
            P::TogglePlayerClock(t) => self.on_toggle_clock(t.enable != 0),
            P::SetPlayerWeather(w) => {
                self.world.weather = w.weather;
                crate::world::apply(&self.world.weather_writes(w.weather));
            }
            P::SetPlayerSkin(k) => self.on_set_player_skin(k.player_id, k.skin),
            P::SetPlayerSpecialAction(a) => self.on_special_action(a.action),
            P::DisableRemoteVehicleCollisions(d) => self.vehicles.set_disable_collisions(d.disable),
            P::RemoveBuildingForPlayer(b) => {
                self.objects.remove_building(b.model_id as i32, [b.pos.x, b.pos.y, b.pos.z], b.radius)
            }
            P::SetPlayerColor(c) => self.on_set_color(c.player_id, c.colour),
            P::SetPlayerArmedWeapon(w) => self.on_set_armed_weapon(w.weapon),
            P::ResetPlayerWeapons(_) => {
                let ped = self.ped();
                if ped != 0 {
                    self.clear_weapons(ped);
                }
            }
            P::PlayerPlaySound(s) => {
                crate::audio::play_sound(&mut self.runner, s.sound_id as i32, [s.pos.x, s.pos.y, s.pos.z])
            }
            _ => {}
        }
    }

    /// RPC 128 answer: a selectable class is stored, the ped loses its weapons and shows the class skin.
    fn on_class_response(&mut self, c: player::PlayerRequestClassResponse) {
        crate::log!(
            "client: class response selectable={} skin={} pos={:?} rot={}",
            c.selectable,
            c.spawn.skin,
            c.spawn.pos,
            c.spawn.rotation
        );
        if c.selectable != 0 {
            self.store_spawn_info(c.spawn);
            // As in the spec: clear weapons and show the class skin on the ped.
            let ped = self.ped();
            if ped != 0 {
                self.clear_weapons(ped);
                self.set_skin(ped, c.spawn.skin);
            }
        }
        self.class_sel.on_class_response(c.selectable != 0);
    }

    fn on_spawn_response(&mut self, response: u8) {
        crate::log!("client: spawn response {}", response);
        if self.class_sel.on_spawn_response(response) {
            self.spawn();
        }
    }

    /// RPC 60: with the HUD clock off, GTA's millisecond timer := time & 0x3FFFFFFF.
    fn on_game_time(&mut self, time: i32) {
        if !self.world.clock_enabled {
            poke(crate::world::TIMER_MS, (time as u32) & 0x3FFF_FFFF);
        }
    }

    /// RPC 12: surfing is detached before the teleport.
    fn on_set_position(&mut self, pos: samp_proto::types::Vec3) {
        self.surf.detach();
        self.teleport(pos.x, pos.y, pos.z)
    }

    /// RPC 13: the ground height at (x, y, z) from 0x02CE, plus 1.5, then as RPC 12
    /// (docs/re/subsystems/player-state.md, "13 SetPlayerPositionFindZ").
    fn on_set_position_find_z(&mut self, pos: samp_proto::types::Vec3) {
        self.surf.detach();
        if self.ped() != 0 {
            self.runner.run(
                op::GET_GROUND_Z_FOR_3D_COORD,
                &[Arg::Float(pos.x), Arg::Float(pos.y), Arg::Float(pos.z), Arg::Local(0)],
            );
            let z = self.runner.local_f32(0) + 1.5;
            self.teleport(pos.x, pos.y, z);
        }
    }

    /// RPC 14 / 66: a float field of the local ped, when there is one.
    fn poke_local_ped(&mut self, field: u32, v: f32) {
        let ped = self.ped();
        if ped != 0 {
            poke(ped + field, v);
        }
    }

    fn on_interpolate_camera(&mut self, c: &player::InterpolateCamera) {
        self.behaviour.interpolate(
            &mut self.runner,
            c.is_position,
            [c.from.x, c.from.y, c.from.z],
            [c.to.x, c.to.y, c.to.z],
            c.time,
            c.cut,
        )
    }

    /// RPC 89: the local ped, or a remote ped (0 when absent).
    fn on_fighting_style(&mut self, player_id: u16, style: u8) {
        let ped = if Some(player_id) == self.net.player_id() {
            self.ped()
        } else {
            self.players.get(player_id).map(|p| p.ped).unwrap_or(0)
        };
        crate::behaviourclient::fighting_style(&mut self.runner, ped, style)
    }

    fn on_give_weapon(&mut self, weapon: u32, ammo: u32) {
        let ped = self.ped();
        if ped != 0 {
            self.give_weapon(ped, weapon.min(255) as u8, ammo);
        }
    }

    fn on_skill_level(&mut self, player_id: u16, skill_type: u32, level: u16) {
        if Some(player_id) == self.net.player_id() {
            crate::combatclient::set_local_skill(skill_type, level);
        } else if player_id < 1004 {
            self.players.set_skill(player_id, skill_type, level);
        }
    }

    fn on_set_team(&mut self, player_id: u16, team: u8) {
        if Some(player_id) == self.net.player_id() {
            self.combat.local_team = team;
        } else {
            self.players.set_team(player_id, team);
        }
    }

    /// RPC 166: the local id is not special-cased by samp, but it has no remote entry (ignored).
    fn on_player_death(&mut self, player_id: u16) {
        if player_id < 1005 && Some(player_id) != self.net.player_id() {
            self.players.kill(&mut self.runner, player_id);
            self.vehicles.reset_remote_vehicle(player_id);
        }
    }

    /// RPC 29: GTA clock, SET_TIME_OF_DAY, settings.
    fn on_set_time(&mut self, hour: u8, minute: u8) {
        poke(crate::world::CLOCK_HOURS, hour);
        poke(crate::world::CLOCK_MINUTES, minute);
        self.runner.run(op::SET_TIME_OF_DAY, &[Arg::Int(i32::from(hour)), Arg::Int(i32::from(minute))]);
        self.world.hour = hour;
        self.world.minute = minute;
    }

    fn on_toggle_clock(&mut self, enable: bool) {
        let mem = ProcessMemory;
        let cur = (
            mem.read_u8(crate::world::CLOCK_HOURS).unwrap_or(0),
            mem.read_u8(crate::world::CLOCK_MINUTES).unwrap_or(0),
        );
        let w = self.world.on_toggle_clock_rpc(enable, cur);
        crate::world::apply(&w);
    }

    /// RPC 72: the local id, or a remote id up to 1004 that is in the pool.
    fn on_set_color(&mut self, player_id: u16, colour: u32) {
        if Some(player_id) == self.net.player_id() || (player_id <= 1004 && self.players.get(player_id).is_some()) {
            crate::worldhooks::with_shared(|s| {
                s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(player_id, colour)
            })
        }
    }

    /// RPC 67: 0…46 only; set-armed-weapon, not forced.
    fn on_set_armed_weapon(&mut self, weapon: u32) {
        if weapon <= 46 {
            let ped = self.ped();
            crate::combatclient::set_armed_local(&mut self.runner, ped, weapon as u8);
        }
    }

    /// RPC 112: the police scanner's stand-in ped is cleared, the on-foot description in
    /// `CAEPoliceScannerAudioEntity::AddAudioEvent` is NOPed, a reported vehicle becomes a temporary vehicle the
    /// stand-in sits in, then the local player's scanner gets `AddAudioEvent(0xA4, crime, pos)`; the temporary
    /// vehicle is deleted afterwards. `suspect_id` is unused. Mirrors samp.dll B 0x100A17A0.
    fn crime_report(&mut self, c: &player::PlayCrimeReport) {
        let dummy = crate::dll::scanner_dummy_ped();
        if dummy != 0 {
            // SAFETY: our own leaked buffer of `SCANNER_DUMMY_PED_BYTES`.
            unsafe { std::ptr::write_bytes(dummy as usize as *mut u8, 0, game::SCANNER_DUMMY_PED_BYTES) };
        }
        let mut mem = ProcessMemory;
        if let Err(e) = crate::patch::Memory::write(&mut mem, CRIME_NOP_ADDR, &[0x90; CRIME_NOP_LEN]) {
            crate::log!("client: RPC 112 NOP at {CRIME_NOP_ADDR:#x} failed: {e}");
        }
        let ped = self.ped();
        let mut temp = None;
        if c.in_vehicle != 0 && (400..612).contains(&c.vehicle_model) {
            temp = crate::vehicles::create_detached(&mut self.runner, c.vehicle_model as u16, c.vehicle_colour as u8);
            if let Some((_, ptr)) = temp
                && dummy != 0
            {
                poke(dummy + off::PED_VEHICLE, ptr);
                let flags = ProcessMemory.read_u32(dummy + off::PED_FLAGS).unwrap_or(0);
                poke(dummy + off::PED_FLAGS, flags | PED_FLAG_IN_VEHICLE);
            }
        }
        if ped != 0 {
            scanner_add_crime(ped, c);
        }
        if let Some((handle, _)) = temp {
            self.runner.run(op::DELETE_CAR, &[Arg::Int(handle)]);
        }
        crate::log!(
            "client: RPC 112 crime {} in_vehicle {} model {} colour {}",
            c.crime_id,
            c.in_vehicle,
            c.vehicle_model,
            c.vehicle_colour
        );
    }

    /// RPC 153: valid ped models only; the local ped, or a streamed-in remote ped. Mirrors samp.dll 0x10019190.
    pub(super) fn on_set_player_skin(&mut self, player_id: u32, skin: u32) {
        if !game::is_ped_model(&ProcessMemory, skin) {
            let text = format!("Warning: SetPlayerSkin {skin} isn't a valid ped model.");
            crate::log!("client: {text}");
            crate::ui::print_debug(&text);
            return;
        }
        if Some(player_id) == self.net.player_id().map(u32::from) {
            let ped = self.ped();
            if ped != 0 {
                self.set_skin(ped, skin);
            }
        } else if player_id < 1004 {
            self.players.set_skin(&mut self.runner, player_id as u16, skin);
        }
    }

    /// RPC 88, for the actions this client has: 0 stops the phone, pissing and a held item (not hands up, as in
    /// samp); 10, 11, 13, 20–23 and 68 start their state. Jetpack, dances, cuffs and carry are not here.
    /// Mirrors samp.dll 0x100030F0.
    pub(super) fn on_special_action(&mut self, action: u8) {
        use crate::remote::special;
        crate::log!("client: special action {action}");
        self.combat.on_special_action(action);
        let ped = self.ped();
        let Some(h) = self.ped_handle(ped) else { return };
        let in_vehicle = super::gta::ped_in_vehicle(ped);
        // samp's case 0 order: phone, piss, item; 20–23 / 68 start an item / pissing.
        if matches!(action, special::NONE | special::PHONE_STOP) && self.phone {
            self.set_phone(h, false);
        }
        crate::specials::local_action(&mut self.runner, ped, &mut self.specials, action);
        match action {
            special::PHONE if !in_vehicle => self.set_phone(h, true),
            special::HANDS_UP if !in_vehicle && super::gta::ped_in_world(ped) => {
                self.runner.run(op::TASK_HANDS_UP, &[Arg::Int(h), Arg::Int(-2)]);
            }
            _ => {}
        }
    }
}

/// RPC 112's report: `AddAudioEvent(0xA4, crime, pos)` on the local player's `CWanted` scanner, when it has one.
fn scanner_add_crime(ped: u32, c: &player::PlayCrimeReport) {
    let player_data = ProcessMemory.read_u32(ped + off::PED_PLAYER_DATA).unwrap_or(0);
    let wanted = if player_data != 0 { ProcessMemory.read_u32(player_data).unwrap_or(0) } else { 0 };
    if wanted != 0 {
        let pos = [c.pos.x, c.pos.y, c.pos.z];
        // SAFETY: CAEPoliceScannerAudioEntity::AddAudioEvent (thiscall) on the local player's CWanted scanner.
        unsafe {
            let f: extern "thiscall" fn(u32, u32, u32, *const [f32; 3]) =
                std::mem::transmute(SCANNER_ADD_AUDIO_EVENT as usize);
            f(wanted + WANTED_SCANNER, 0xA4, c.crime_id, &pos);
        }
    }
}
