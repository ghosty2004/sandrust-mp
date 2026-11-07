//! Session and connection: network events (InitGame, RPCs, sync, connection packets), the lifecycle actions, the
//! RPC 40 reset routine, the connecting scene and the quit / teardown path.

use std::time::Instant;

use samp_proto::session::InitGame;

use super::Client;
use super::gta::{poke, set_hud};
use crate::game::{self, off};
use crate::lifecycle::{self, Action};
use crate::netclient::Event;
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, op};

/// Script commands of the RPC 40 reset: select interior, link char to interior, explode char head.
const OP_SELECT_INTERIOR: u16 = 0x04BB;
const OP_LINK_CHAR_TO_INTERIOR: u16 = 0x0860;
const OP_EXPLODE_CHAR_HEAD: u16 = 0x0321;
/// `CFire::Extinguish` (thiscall).
const GTA_FIRE_EXTINGUISH: u32 = 0x5393F0;
/// `CWorld::PlayerInFocus`.
const GTA_PLAYER_IN_FOCUS: u32 = 0xB7CD74;
/// GTA's gravity constant.
const GTA_GRAVITY: u32 = 0x863984;
/// samp's default gravity (server settings default, bits 0x3C03126F).
const DEFAULT_GRAVITY: f32 = 0.008;

impl Client {
    pub fn shutdown(&mut self) {
        self.net.disconnect(Instant::now());
    }

    /// `/quit`: RakClient Disconnect(500) when the netgame is in state 5 (the notification goes out); the game
    /// keeps running until [`Self::quit_teardown`].
    pub fn begin_quit(&mut self) {
        crate::log!("client: /quit in netgame state {}", self.netgame.state);
        if self.netgame.connected() {
            self.net.disconnect(Instant::now());
        }
    }

    /// The netgame's destruction before samp's `ExitProcess`: RakClient Disconnect(0) (no notification), then the
    /// pools in samp's order: players (remote peds), vehicles (occupants warped out first), actors, textdraws,
    /// gang zones, 3D labels, pickups, objects, menus. Textdraws, zones, labels and menus have no game-side
    /// objects. Mirrors samp.dll 0x10009880.
    pub fn quit_teardown(&mut self) {
        self.net.disconnect_silent();
        self.players.clear(&mut self.runner);
        crate::log!("client: teardown: players");
        self.vehicles.clear(&mut self.runner);
        crate::log!("client: teardown: vehicles");
        self.entities.clear_actors(&mut self.runner);
        self.entities.clear_pickups(&mut self.runner);
        self.objects.clear(&mut self.runner);
        crate::log!("client: teardown: actors, pickups, objects");
    }

    pub(super) fn on_event(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::InitGame(g) => self.on_init_game(&g, now),
            Event::Rpc(r) => self.on_rpc(r),
            Event::Sync(s) => self.on_sync(s, now),
            Event::Packet(p) => {
                crate::log!("client: packet {p:?} in netgame state {}", self.netgame.state);
                let actions = self.netgame.on_packet(p);
                self.apply_actions(actions);
            }
            Event::Rejected(reason) => {
                let actions = self.netgame.on_rejected(reason);
                self.apply_actions(actions);
            }
        }
    }

    /// InitGame: rates, hour, weather (applied), zone names, CJ walk, the pools' settings, then class selection.
    fn on_init_game(&mut self, g: &InitGame, now: Instant) {
        self.on_foot_rate = g.on_foot_rate.clamp(10, 1000);
        self.world.hour = g.world_time;
        self.world.weather = g.weather;
        crate::world::apply(&self.world.weather_writes(g.weather));
        self.zone_names = g.zone_names;
        crate::pedhooks::set_cj_walk(g.cj_walk);
        self.behaviour.on_init_game(g.gravity, g.stunt_bonus);
        self.behaviour.hash_table();
        crate::log!(
            "client: InitGame: server {:?}, our id {}, {} class(es), on-foot rate {} ms",
            g.server_name,
            g.player_id,
            g.spawns_available,
            g.on_foot_rate
        );
        self.players.set_init_game(g.player_id, g.death_drop_money, g.player_marker_mode);
        self.combat.on_init_game(g);
        self.class_count = g.spawns_available;
        crate::ui::on_init_game(g);
        self.netgame.on_init_game();
        self.vehicles.set_init_game(g);
        // The InitGame vehicle-model preload (`/dl` reports its flags).
        crate::vehlabels::preload(&mut self.runner, &g.vehicle_models);
        self.objects.set_local_id(g.player_id);
        self.entities.set_local_id(g.player_id);
        // Join: MARK_MODEL_AS_NO_LONGER_NEEDED(596) first, only on InitGame (the death → class-selection
        // path does not release it, as in the original).
        self.runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(596)]);
        self.enter_class_selection(now, g.spawns_available);
    }

    /// Lifecycle actions (lifecycle.rs), in order.
    pub(super) fn apply_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Debug(t) => crate::ui::print_debug(t),
                Action::Info(t) => crate::ui::print_info(t.as_bytes()),
                Action::Disconnect(_) => self.net.disconnect_silent(),
                Action::Reset => self.reset_routine(),
                Action::DeletePlayers => {
                    self.players.clear(&mut self.runner);
                    crate::ui::forget_players();
                }
                Action::StopAudio { wait } => {
                    crate::audio::stop(wait);
                }
            }
        }
    }

    /// Sync packets: 207/203 go to the player pool; 200/211 are stored and 209/210 applied by the vehicle pool.
    fn on_sync(&mut self, s: samp_proto::sync::IncomingSync, now: Instant) {
        use samp_proto::sync::IncomingSync as S;
        match s {
            S::Foot { ref sync, .. } if Some(sync.player_id) != self.net.player_id() => {
                let id = sync.player_id;
                self.players.on_sync(s, now);
                self.vehicles.after_foot_store(&mut self.runner, &mut self.players, id);
            }
            S::Vehicle { .. } | S::Passenger(_) | S::Unoccupied(_) | S::Trailer(_) => {
                self.vehicles.on_sync(&mut self.runner, &mut self.players, s, now)
            }
            // Bullet sync fires its shot at once (docs/re/subsystems/remote-control-combat.md, "Per-frame order").
            S::Bullet(b) => {
                self.update_combat_shared();
                self.combat.on_bullet(&mut self.runner, &mut self.players, &self.vehicles, &b);
            }
            S::Markers(m) => self.players.on_markers(&mut self.runner, &m),
            s => self.players.on_sync(s, now),
        }
    }

    /// RPC 40 reset (docs/re/subsystems/session-misc.md, "40 PlayerClose"): remote peds removed, pools cleared,
    /// server settings back to their defaults, interior 0, the local ped killed, then the not-connected scene every
    /// frame until InitGame (docs/phase3/trace-notes.md, "Connection lifecycle"). Mirrors samp.dll B 0x1000A530.
    fn reset_routine(&mut self) {
        crate::log!("client: reset routine (server restart / lost connection)");
        self.reset_pools_and_settings();
        // Game side: interior 0, then the local ped is killed.
        let ped = self.ped();
        crate::log!("client: reset: settings restored, local ped {ped:#x}");
        if let Some(h) = self.ped_handle(ped) {
            self.runner.run(OP_SELECT_INTERIOR, &[Arg::Int(0)]);
            self.runner.run(OP_LINK_CHAR_TO_INTERIOR, &[Arg::Int(h), Arg::Int(0)]);
            if let Some(p) = game::snapshot(&ProcessMemory).ok().and_then(|s| s.player) {
                self.runner.run(op::REQUEST_COLLISION, &[Arg::Float(p.pos[0]), Arg::Float(p.pos[1])]);
            }
            crate::log!("client: reset: interior 0");
        }
        // The money back to 0 (ADD_SCORE with the negated amount).
        crate::behaviourclient::reset_money(&mut self.runner);
        if let Some(h) = self.ped_handle(ped) {
            self.kill_local_ped(ped, h);
            crate::log!("client: reset: local ped killed");
            // Armour 0.
            poke(ped + off::PED_ARMOUR, 0.0f32);
        }
        self.reset_game_defaults();
        crate::ui::print_info(lifecycle::msg::RESTARTING.as_bytes());
    }

    /// The reset's first part: remote peds, weapon skills, the local state, the entity pools, the server-settings
    /// defaults.
    fn reset_pools_and_settings(&mut self) {
        // Remote players lose their peds but stay in the pool.
        self.players.despawn_all(&mut self.runner);
        crate::log!("client: reset: remote peds removed");
        // The 11 weapon-skill stats := 999.0, written directly.
        for skill in 0..11 {
            crate::combatclient::set_local_skill(skill, 999);
        }
        // Local player: back to the not-spawned state.
        self.class_sel.leave();
        self.spawned = false;
        self.dead = false;
        // The entity pools are deleted and recreated.
        self.objects.clear(&mut self.runner);
        self.entities.clear(&mut self.runner);
        self.vehicles.clear(&mut self.runner);
        crate::log!("client: reset: pools cleared");
        crate::ui::on_reset();
        // Server-settings defaults (+0x3d5): hour 12:00, weather 1, zone names off, CJ walk off.
        self.world.hour = 12;
        self.world.minute = 0;
        self.world.weather = 1;
        self.zone_names = false;
        self.zone_names_from = None;
        crate::pedhooks::set_cj_walk(false);
    }

    /// The reset's last part: zone names off, default player colours, gravity, wanted level 0, the HUD clock off.
    fn reset_game_defaults(&mut self) {
        self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(0)]);
        crate::worldhooks::with_shared(|s| s.colours = Some(crate::ui::colours::PlayerColours::new()));
        crate::ui::reset_colours();
        // GTA gravity back to the default. The constant lives in a read-only page; samp unprotects it first.
        if let Err(e) = ProcessMemory.write(GTA_GRAVITY, &DEFAULT_GRAVITY.to_le_bytes()) {
            crate::log!("client: reset: gravity write failed: {e}");
        }
        // The HUD wanted level 0, the HUD clock off.
        crate::behaviourclient::set_wanted(0);
        let w = self.world.toggle_clock(false);
        crate::world::apply(&w);
    }

    /// Kills the local ped like samp: fires out, teleport to its own position, health 0, `EXPLODE_CHAR_HEAD` with
    /// `CWorld::PlayerInFocus` set to the ped's player number around it. Mirrors samp.dll B 0x100AFFE0.
    fn kill_local_ped(&mut self, ped: u32, handle: i32) {
        let m = ProcessMemory;
        extinguish(m.read_u32(ped + off::PED_FIRE).unwrap_or(0));
        if super::gta::ped_in_vehicle(ped) {
            let veh = m.read_u32(ped + off::PED_VEHICLE).unwrap_or(0);
            if veh != 0 {
                extinguish(m.read_u32(veh + off::VEHICLE_FIRE).unwrap_or(0));
            }
        }
        if let Some(p) = game::snapshot(&m).ok().and_then(|s| s.player) {
            self.teleport(p.pos[0], p.pos[1], p.pos[2]);
        }
        poke(ped + off::PED_HEALTH, 0.0f32);
        poke(GTA_PLAYER_IN_FOCUS, 0u8);
        self.runner.run(OP_EXPLODE_CHAR_HEAD, &[Arg::Int(handle)]);
        poke(GTA_PLAYER_IN_FOCUS, 0u8);
    }

    /// Every frame until InitGame: the original forces weather 1, parks the ped and shows a fixed view of Los Santos
    /// (docs/phase3/m2-notes.md). Mirrors samp.dll 0x100089B0.
    pub(super) fn connecting_tick(&mut self) {
        // Weather 1 every frame while not connected.
        crate::world::apply(&self.world.weather_writes(crate::world::CONNECTING_WEATHER));
        set_hud(false);
        self.teleport(1133.05, -2038.40, 69.1);
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.runner.run(op::SET_FIXED_CAMERA_POSITION, &[1093.0, -2036.0, 90.0, 0.0, 0.0, 0.0].map(Arg::Float));
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.runner
            .run(op::POINT_CAMERA_AT_POINT, &[Arg::Float(384.0), Arg::Float(-1557.0), Arg::Float(20.0), Arg::Int(2)]);
    }
}

/// `CFire::Extinguish` on a ped's or vehicle's fire, when there is one.
fn extinguish(fire: u32) {
    if fire != 0 {
        // SAFETY: CFire::Extinguish (thiscall) on the game's fire object.
        let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(GTA_FIRE_EXTINGUISH as usize) };
        f(fire);
    }
}
