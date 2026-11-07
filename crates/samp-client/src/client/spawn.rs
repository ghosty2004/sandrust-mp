//! Class selection, the local spawn routine, and death / respawn (docs/re/subsystems/player-state.md).

use std::time::{Duration, Instant};

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::player;

use super::gta::{async_key_down, entity_pos, poke, set_hud};
use super::{Client, PEDSTATE_DEAD, PEDSTATE_DIE};
use crate::classsel;
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, op};

/// DISPLAY_ZONE_NAMES starts this long after a spawn.
const ZONE_NAMES_DELAY: Duration = Duration::from_millis(1000);

/// Win32 virtual-key codes polled like the original client (docs/re/subsystems/player-state.md, class selection).
const VK_SHIFT: i32 = 0x10;
const VK_LEFT: i32 = 0x25;
const VK_RIGHT: i32 = 0x27;

impl Client {
    /// RPC 68 / 128 spawn block, with the client's range check (docs/re/subsystems/player-state.md,
    /// "68 SetSpawnInfo").
    pub(super) fn store_spawn_info(&mut self, s: player::SpawnInfo) {
        let ok = s.pos.x > -20000.0
            && s.pos.x < 20000.0
            && s.pos.y > -20000.0
            && s.pos.y < 20000.0
            && s.pos.z > -10000.0
            && s.pos.z < 100000.0;
        if ok {
            self.spawn_info = Some(s);
        } else {
            crate::log!("client: spawn info out of range ignored: {:?}", s.pos);
        }
    }

    // ---- class selection --------------------------------------------------

    pub(super) fn enter_class_selection(&mut self, now: Instant, count: u32) {
        crate::log!("client: entering class selection ({count} class(es))");
        self.spawned = false;
        self.sender.reset();
        let ped = self.ped();
        if ped != 0 {
            self.reset_ped_state(ped);
            poke(ped + off::PED_HEALTH, 100.0f32);
            self.set_controllable(false);
        }
        self.class_sel.enter(now, count);
    }

    pub(super) fn class_selection_tick(&mut self, now: Instant) {
        // While no frontend menu is open (samp reads the menu flag as a dword): HUD off, every frame. Its
        // input-lock mode 2 comes from the UI (set_class_selection → the class panel requests mode 2 every frame).
        if ProcessMemory.read_u32(game::MENU_ACTIVE).unwrap_or(0) == 0 {
            set_hud(false);
        }
        // Clicks on the class-selection panel (<<, >>, Spawn) act like the keys.
        let click = crate::ui::take_class_click();
        // The rest runs only while no dialog is open: a dialog the server shows on connect holds back the first
        // RPC 128 until it is answered; it goes out in the first local tick after the close (the close clears the
        // flag at once, not after the input-lock restore).
        if crate::ui::dialog_open() {
            return;
        }
        let keys = class_selection_keys(click);
        match self.class_sel.tick(now, keys) {
            Some(classsel::Action::RequestClass(i)) => {
                crate::log!("client: requesting class {i}");
                self.net.send_rpc(&player::PlayerRequestClass { class_index: i }, Reliability::Reliable);
            }
            Some(classsel::Action::ChangeClass { index, sound }) => {
                // The sound at the ped's matrix position through the game audio object (the RPC 16 path), then
                // RPC 128, same tick.
                let pos = entity_pos(self.ped()).unwrap_or([0.0; 3]);
                crate::audio::play_sound(&mut self.runner, sound, pos);
                crate::log!("client: requesting class {index} (sound {sound})");
                self.net.send_rpc(&player::PlayerRequestClass { class_index: index }, Reliability::Reliable);
            }
            Some(classsel::Action::RequestSpawn) => {
                crate::log!("client: requesting spawn");
                self.net.send_rpc(&player::PlayerRequestSpawn, Reliability::Reliable);
            }
            None => {}
        }
    }

    // ---- local spawn routine (docs/re/subsystems/player-state.md, "Local Spawn routine") ----

    /// The local spawn routine, in the original's order; the numbered steps follow the spec.
    pub(super) fn spawn(&mut self) -> bool {
        let Some(s) = self.spawn_info else {
            crate::log!("client: spawn: no spawn info");
            return false;
        };
        let ped = self.ped();
        if ped == 0 {
            crate::log!("client: spawn: no local ped");
            return false;
        }
        // Surf state cleared.
        self.surf.reset();
        // 2. leave class selection; its input lock ends 10 frames later.
        if self.class_sel.is_active() {
            self.class_sel.leave();
        }
        crate::ui::class_selection_release();
        // 3. camera, HUD, control (the original's order: 02EB, 0930/0925/0373/02EB, 01B4, 04D7, teleport).
        self.behaviour.attached = None;
        self.runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
        self.camera_behind_player();
        set_hud(true);
        self.set_controllable(true);
        self.spawn_reset_ped(ped, &s);
        // 6. no random trains; CCamera::Fade := ret 8.
        self.runner.run(op::SWITCH_RANDOM_TRAINS, &[Arg::Int(0)]);
        if !self.fade_patched {
            let mut mem = ProcessMemory;
            match mem.write(game::CAMERA_FADE, &[0xC2, 0x08, 0x00]) {
                Ok(()) => self.fade_patched = true,
                Err(e) => crate::log!("client: fade patch failed: {e}"),
            }
        }
        // 7. teleport slightly above the spawn point, face the spawn heading.
        self.teleport(s.pos.x, s.pos.y, s.pos.z + 0.5);
        self.set_facing(s.rotation);
        // (The original also issues two REMOVE_BLIP commands here with handles of its own that are never valid
        // blips; nothing to reproduce.)
        // 8./9. state and RPC 52.
        self.spawned = true;
        self.dead = false;
        self.combat.reset_local();
        self.spawned_once = true;
        self.zone_names_from = Some(Instant::now() + ZONE_NAMES_DELAY);
        self.vehicles.set_spawned();
        self.sender.reset();
        self.last_stats = None;
        self.net.send_rpc(&player::PlayerSpawn, Reliability::ReliableOrdered);
        crate::log!("client: spawned at {:?} heading {} skin {}", s.pos, s.rotation, s.skin);
        true
    }

    /// Spawn steps 4 and 5: ped reset (the original skips it on the first spawn: one-shot flag), area, restart
    /// point, skin, weapons, damage record, then the spawn weapons.
    fn spawn_reset_ped(&mut self, ped: u32, s: &player::SpawnInfo) {
        if self.spawned_once {
            self.reset_ped_state(ped);
        }
        self.runner.run(op::REQUEST_COLLISION, &[Arg::Float(s.pos.x), Arg::Float(s.pos.y)]);
        // Restart override at the spawn point (the traced build uses ADD_HOSPITAL_RESTART instead of writing the
        // restart table directly).
        self.runner.run(
            op::ADD_HOSPITAL_RESTART,
            &[Arg::Float(s.pos.x), Arg::Float(s.pos.y), Arg::Float(s.pos.z), Arg::Float(s.rotation), Arg::Int(0)],
        );
        self.set_skin(ped, s.skin);
        self.clear_weapons(ped);
        // Spawn routine: special action 0 (stops the phone etc.).
        self.on_special_action(0);
        poke(ped + off::PED_LAST_DAMAGE_ENTITY, 0u32);
        poke(ped + off::PED_LAST_DAMAGE_WEAPON, 0xFFu32);
        // 5. weapons: slot 3, then 2, then 1.
        for i in (0..3).rev() {
            if s.weapons[i] != player::SpawnInfo::NO_WEAPON {
                self.give_weapon(ped, s.weapons[i] as u8, s.ammo[i]);
            }
        }
    }

    // ---- death and respawn (local tick) -------------------------------------------

    /// Death detection while spawned, then the respawn (or class selection) once GTA's wasted sequence has ended.
    pub(super) fn death_tick(&mut self, now: Instant) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let mem = ProcessMemory;
        let st = mem.read_u32(ped + off::PED_STATE).unwrap_or(0);
        let health = mem.read_f32(ped + off::PED_HEALTH).unwrap_or(0.0);
        if self.spawned {
            if (!self.dead && st == PEDSTATE_DIE) || health <= 0.0 {
                self.on_local_death(ped, now);
            }
            return;
        }
        // Dead: wait until GTA's wasted sequence has ended (ped state no longer 54/55).
        if st == PEDSTATE_DIE || st == PEDSTATE_DEAD {
            return;
        }
        // Cuffs off; the jetpack does not exist here.
        self.combat.cuffs.cuffed = false;
        self.combat.publish_masks();
        let connected = self.netgame.connected();
        if self.spawn_info.is_some() && !self.force_class_selection && connected {
            if health > 0.0 {
                self.dead = false;
                crate::log!("client: respawn after death");
                self.spawn();
            }
            return;
        }
        self.dead = false;
        let count = self.class_count;
        self.enter_class_selection(now, count);
        self.force_class_selection = false;
    }

    /// The death branch of the local tick: special states off, controllable, a driver's last 200, fires out,
    /// RPC 53 with the killer / reason, then dead := 1, spawned := 0, zone names off.
    fn on_local_death(&mut self, ped: u32, now: Instant) {
        if self.hands_up(ped) {
            self.set_controllable(true);
        }
        if self.phone
            && let Some(h) = self.ped_handle(ped)
        {
            self.set_phone(h, false);
        }
        self.runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(0)]);
        self.set_controllable(true);
        if self.combat.drive_by {
            crate::vehicles::stop_drive_by(ped);
            self.combat.drive_by = false;
        }
        self.vehicles.force_driver_send(&mut self.net, now);
        crate::combathooks::extinguish(ped);
        let mem = ProcessMemory;
        let weapon = mem.read_u8(ped + off::PED_LAST_DAMAGE_WEAPON).unwrap_or(0xFF);
        let entity = mem.read_u32(ped + off::PED_LAST_DAMAGE_ENTITY).unwrap_or(0);
        let damager = self.damager_of(entity);
        let (reason, killer) = crate::combat::death_reason(u32::from(weapon), damager);
        crate::log!(
            "client: local death: weapon {weapon} damager {damager:?} -> RPC 53 reason {reason} killer {killer}"
        );
        self.net.send_rpc(&player::OnPlayerDeath { reason, killer_id: killer }, Reliability::ReliableOrdered);
        self.spawned = false;
        self.dead = true;
        self.zone_names_from = None;
        self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(0)]);
    }

    /// The last damaging entity against the pools (remote ped, pool vehicle with its remote driver).
    fn damager_of(&self, entity: u32) -> crate::combat::Damager {
        use crate::combat::Damager;
        if entity == 0 {
            return Damager::None;
        }
        if let Some(p) = self.players.iter().find(|p| p.ped == entity) {
            return Damager::Player(p.id);
        }
        if self.vehicles.hook_vehicles().iter().any(|v| v.0 == entity) {
            let driver = ProcessMemory.read_u32(entity + off::VEHICLE_DRIVER).unwrap_or(0);
            let id = if driver != 0 { self.players.iter().find(|p| p.ped == driver).map(|p| p.id) } else { None };
            return Damager::Vehicle { driver: id };
        }
        Damager::Other
    }
}

/// The class-selection keys: panel clicks, the arrow keys and Shift (Win32 state or GTA's keyboard state).
fn class_selection_keys(click: u8) -> classsel::Keys {
    let mem = ProcessMemory;
    classsel::Keys {
        left: click == 1 || async_key_down(VK_LEFT) || game::key_down(&mem, game::key::LEFT),
        right: click == 2 || async_key_down(VK_RIGHT) || game::key_down(&mem, game::key::RIGHT),
        // Shift does nothing while the chat input is open.
        shift: click == 3
            || (!crate::ui::chat_input_open()
                && (async_key_down(VK_SHIFT)
                    || game::key_down(&mem, game::key::SHIFT)
                    || game::key_down(&mem, game::key::LSHIFT)
                    || game::key_down(&mem, game::key::RSHIFT))),
    }
}
