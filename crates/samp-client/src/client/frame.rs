//! The per-frame tick in samp's main-loop order, and the session's steps inside `CGame::Process` (camera follow,
//! plate pass and object process).

use std::time::{Duration, Instant};

use super::Client;
use super::gta::{async_key_any, poke};
use crate::footsync;
use crate::lifecycle;
use crate::netclient::Phase;
use crate::script::{Arg, op};

/// Animation blocks the original requests every frame.
const PRELOAD_ANIM_BLOCKS: [&str; 9] =
    ["PARACHUTE", "BAR", "GANGS", "PAULNMAC", "VENDING", "FIGHT_B", "FIGHT_C", "FIGHT_D", "FIGHT_E"];
/// Models the original requests every frame.
const PRELOAD_MODELS: [i32; 9] = [3131, 1485, 1486, 1543, 1546, 343, 342, 344, 330];

impl Client {
    /// One frame (main thread), in samp's main-loop order: the packet handler first, then the first-frame setup,
    /// the per-frame world work, remote players, the local tick (class selection / spawned / connecting), the
    /// connect tick, the pools, and finally the queued sends. Mirrors samp.dll B 0x1000B5A0.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.packet_handler(now);
        if self.debug_spawn {
            self.debug_spawn = false;
            crate::log!("client: debug spawn requested over the control channel");
            self.spawn();
        }
        if !self.first_frame_done {
            self.first_frame();
        }
        self.world_frame();
        self.remote_frame(now);
        // The UI's messages first: the original sends them from its window procedure, before the local tick
        // (a dialog closed by Enter sends RPC 62, then this frame's class tick its first RPC 128).
        crate::ui::send_outgoing(&mut self.net);
        crate::ui::set_class_selection(self.class_sel.is_active());
        self.local_tick(now);
        self.f4_tick();
        self.connect_tick();
        self.after_local_tick();
        self.pools_tick(now);
        self.end_frame(now);
    }

    /// The call of `CCamera::Process` in `CGame::Process` (hooked in dll.rs): the camera follows an attached object.
    pub fn camera_process(&mut self) {
        if matches!(self.net.phase(), Phase::Connected) {
            self.behaviour.camera_follow(&mut self.runner, &self.objects);
        }
    }

    /// CGame::Process pre-step (dll.rs): the vehicle pool's plate pass, then the per-frame object process
    /// (docs/re/subsystems/world-entities.md, "Per-frame object process").
    pub fn pre_process(&mut self) {
        if self.netgame.connected() {
            // The vehicle pool's plate pass before the object process, as in samp.
            crate::plates::tick(&self.vehicles.plate_list(), &mut |t| crate::ui::paint_plate(t));
            self.objects.process(&mut self.runner, &self.vehicles, &self.players);
        }
    }

    /// What the combat hooks queued, then the packet handler, then the remote shots it fired.
    fn packet_handler(&mut self, now: Instant) {
        // What the combat hooks queued during this frame's CGame::Process (206 / 115), first.
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        self.update_combat_shared();
        // The packet handler first.
        for ev in self.net.tick(now) {
            self.on_event(ev, now);
        }
        // Remote 206 shots fired while the packets were handled (115 taking 1).
        self.entities.sent_177 += self.combat.drain(&mut self.net);
    }

    /// The original's first frame: two ped flags on the local player (normally already set from the
    /// CGame::Process hook in dll.rs), zone names off, the clock off, the driving skills.
    fn first_frame(&mut self) {
        self.first_frame_done = true;
        if !crate::dll::PED_FLAGS_DONE.load(std::sync::atomic::Ordering::SeqCst)
            && let Some(h) = self.ped_handle(self.ped())
        {
            self.runner.run(op::SET_CHAR_DROPS_WEAPONS_WHEN_DEAD, &[Arg::Int(h), Arg::Int(1)]);
            self.runner.run(op::SET_CHAR_SUFFERS_CRITICAL_HITS, &[Arg::Int(h), Arg::Int(0)]);
        }
        self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(0)]);
        // samp's first frame: the GTA clock and the HUD clock string off.
        let w = self.world.toggle_clock(false);
        crate::world::apply(&w);
        // Driving skills to 1000, then SetStatValue := RET (docs/re/subsystems/vehicles.md, "Driving skills").
        self.vehicles.first_frame();
    }

    /// Every frame, like the original's netgame tick: the server's time of day while the server holds the time,
    /// then the pre-loading of the animation blocks and models samp uses.
    fn world_frame(&mut self) {
        if let Some((h, m)) = self.world.time_of_day() {
            poke(crate::world::CLOCK_HOURS, h);
            poke(crate::world::CLOCK_MINUTES, m);
            self.runner.run(op::SET_TIME_OF_DAY, &[Arg::Int(i32::from(h)), Arg::Int(i32::from(m))]);
        }
        self.preload();
    }

    /// Remote players before the local player's tick and send (docs/re/subsystems/remote-control-onfoot.md,
    /// "Per-frame order for a remote on-foot player").
    fn remote_frame(&mut self, now: Instant) {
        self.players.tick(&mut self.runner, now);
        self.vehicles.remote_tick(&mut self.runner, &mut self.players);
        self.vehicles.hook_events(&mut self.runner, &mut self.net);
        crate::ui::set_remotes(self.players.ui_snapshot(now));
    }

    /// The local tick: spectating unspawned, class selection, or death / respawn, the spawned frame and its sync,
    /// or the connecting scene.
    fn local_tick(&mut self, now: Instant) {
        if self.behaviour.spectate.on && !self.spawned && matches!(self.net.phase(), Phase::Connected) {
            // Spectating without being spawned: class selection is left, then the spectator tick.
            self.behaviour.drunk_not_spawned(&mut self.runner);
            if self.class_sel.is_active() {
                self.class_sel.leave();
                crate::ui::class_selection_release();
            }
            self.spectator_tick(now);
        } else if self.class_sel.is_active() {
            self.class_selection_tick(now);
        } else {
            // Death (RPC 53) and the respawn after the wasted sequence.
            if self.spawned || self.dead {
                self.death_tick(now);
            }
            if self.class_sel.is_active() || self.dead {
            } else if self.spawned {
                self.local_frame(now);
                self.sync_tick(now);
            } else if !self.netgame.connected() {
                self.connecting_tick();
            }
        }
    }

    /// The spectator tick (unspawned, or replacing the on-foot / driver branches while spawned).
    pub(super) fn spectator_tick(&mut self, now: Instant) {
        let interval = footsync::on_foot_interval(self.on_foot_rate, self.players.streamed());
        self.behaviour.spectator_tick(
            &mut self.runner,
            &mut self.net,
            &self.players,
            &self.vehicles,
            interval,
            now,
            |net| self.combat.aim_spectating(net, &mut self.sender, interval, now),
        );
    }

    /// F4 at the end of the local tick, except while spectating unspawned: once per pending flag, "class selection
    /// after the next death" (the flag RPC 74 sets) and its client line.
    fn f4_tick(&mut self) {
        if !(self.behaviour.spectate.on && !self.spawned) && !self.force_class_selection && async_key_any(super::VK_F4)
        {
            self.force_class_selection = true;
            crate::log!("client: F4 -> class selection after the next death");
            crate::ui::print_info(super::F4_LINE.as_bytes());
        }
    }

    /// The connect tick, in state 1 only; the tick count is read here (the packet handler may have blocked in
    /// RakNet's Disconnect).
    fn connect_tick(&mut self) {
        let now_ms = Instant::now().duration_since(self.clock).as_millis() as u64;
        if self.netgame.connect_due(now_ms) {
            let (host, port) = self.connect_target.clone();
            crate::ui::print_debug(&lifecycle::msg::connecting(&host, port));
            if let Err(e) = self.net.connect() {
                crate::log!("client: connect failed: {e}");
            }
            // samp reads the tick count again after RakClient::Connect returned.
            let after = Instant::now();
            self.netgame.attempt_started(after.duration_since(self.clock).as_millis() as u64);
            for ev in self.net.tick(after) {
                self.on_event(ev, after);
            }
        }
    }

    /// Fault handling, the `/dl` labels and the UI's network tick after the local tick.
    fn after_local_tick(&mut self) {
        // Remote peds whose ProcessControl faulted (swap.rs fault containment).
        self.players.handle_faults(&mut self.runner);
        // `/dl` labels, rebuilt every tick while the flag is on.
        let spectating = self.behaviour.spectate.on;
        crate::vehlabels::tick(self.vehicles.debug_rows(), self.ped(), spectating);
        for line in crate::swap::take_fault_lines() {
            crate::ui::print_debug(&line);
        }
        // Chat input and connection lines of the in-game UI.
        let (veh_count, veh_models) = self.vehicles.pool_stats();
        crate::ui::set_pool_stats(veh_count, veh_models, self.players.streamed(), self.objects.count());
        crate::ui::net_tick(&mut self.net);
    }

    /// The vehicle pool tick follows the local player's (docs/re/subsystems/remote-control-vehicles.md,
    /// "Per-frame order"), then the world entities.
    fn pools_tick(&mut self, now: Instant) {
        if self.netgame.connected() {
            self.vehicles.pool_tick(&mut self.runner, &mut self.net, &self.players, now);
            // World entities: hook events (RPC 131 / 177), checkpoints, pickup poll, labels.
            self.entities.hook_events(&mut self.net);
            self.objects.poll_editor(&mut self.runner, &mut self.net);
            self.entities.tick(&mut self.runner, &mut self.net, &self.players, &self.vehicles);
        }
    }

    /// The hooks' view for the next CGame::Process, then the queued sends go out.
    fn end_frame(&mut self, now: Instant) {
        self.update_combat_shared();
        self.surf_publish();
        // The driver-enter gate refuses while the local ped is cuffed or carrying.
        let cuffs = self.combat.cuffs;
        crate::vehhooks::with_shared(|s| (s.local_cuffed, s.local_carrying) = (cuffs.cuffed, cuffs.carry));
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        // Push out what this frame queued (sync packets, RPCs) right away.
        self.net.flush(now);
        if self.last_net_log.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(5)) {
            self.last_net_log = Some(now);
            crate::log!("client: {}", self.status_line());
        }
    }

    /// Every frame: request the animation blocks and models the original keeps loaded.
    fn preload(&mut self) {
        for block in PRELOAD_ANIM_BLOCKS {
            if !self.runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Text(block)]) {
                self.runner.run(op::REQUEST_ANIMATION, &[Arg::Text(block)]);
            }
        }
        for model in PRELOAD_MODELS {
            if !self.runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
                self.runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
            }
        }
    }
}
