//! The in-game client session (Windows only): network session, class
//! selection, local spawn, on-foot sync and the S→C player RPCs of M2.
//!
//! Everything here runs on the game's main thread from the per-frame hook,
//! except [`Client::status_line`] / [`Client::request_debug_spawn`], which
//! only touch plain data.
//!
//! Specs: player-state.md (class selection, 128/129/52, "Local Spawn routine",
//! 12/14/19/66/68/74, camera RPCs 157/158/162), remote-players-sync.md §0.3/§0.4
//! and §2 (207, 205), gta-patches.md row 365 (fade) and row 385 (restart
//! override).

use std::time::{Duration, Instant};

use samp_proto::auth::AuthTable;
use samp_proto::raknet::Reliability;
use samp_proto::rpcs::{Incoming, player, ui};
use samp_proto::sync::player::StatsSync;
use samp_proto::sync::policy;

use crate::classsel::{self, ClassSelection};
use crate::footsync::{self, FootSender};
use crate::game::{self, off};
use crate::lifecycle::{self, Action, NetGame};
use crate::netclient::{Event, NetClient, Phase};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner, op};

/// Environment variable with the (Windows) path of `auth_table.txt`.
pub const AUTH_TABLE_ENV: &str = "SAMP_RS_AUTH_TABLE";

/// Ped states (gta-reversed `Enums/ePedState.h`).
const PEDSTATE_IDLE: u32 = 1;
const PEDSTATE_SNIPER: u32 = 12;
const PEDSTATE_DRIVING: u32 = 50;
/// `TheCamera.m_aCams[0].Front` (samp's camera-front pointer at A 0x101039c8 holds 0xB6F32C).
const CAMERA_FRONT: u32 = 0xB6F32C;
const PEDSTATE_DIE: u32 = 54;
/// `CPed::RemoveWeaponWhenEnteringVehicle(int)` (thiscall; gta-reversed `Ped.cpp`).
const PED_REMOVE_WEAPON_ENTERING_VEHICLE: usize = 0x5E6370;
const PEDSTATE_DEAD: u32 = 55;

/// Win32 virtual-key codes polled like the original client (player-state.md 128/129).
/// Animation blocks and models the original requests every frame (TRACE-1).
const PRELOAD_ANIM_BLOCKS: [&str; 9] =
    ["PARACHUTE", "BAR", "GANGS", "PAULNMAC", "VENDING", "FIGHT_B", "FIGHT_C", "FIGHT_D", "FIGHT_E"];
const PRELOAD_MODELS: [i32; 9] = [3131, 1485, 1486, 1543, 1546, 343, 342, 344, 330];

/// DISPLAY_ZONE_NAMES starts this long after a spawn (A 0x10003c20 sets +0x30a := now + 1000).
const ZONE_NAMES_DELAY: Duration = Duration::from_millis(1000);
/// `CControllerState::ButtonTriangle` in `CPad::Pads[0].NewState` (samp's pad +0x1E).
const PAD_TRIANGLE: u32 = 0x1E;

/// Script commands of the RPC 40 reset (B 0x100ad350 / 0x100affe0).
const OP_SELECT_INTERIOR: u16 = 0x04BB;
const OP_LINK_CHAR_TO_INTERIOR: u16 = 0x0860;
const OP_EXPLODE_CHAR_HEAD: u16 = 0x0321;
/// `CPed::m_pFire`, `CVehicle::m_pFire`, `CPed` flags dword with the in-vehicle bit 0x100
/// (B 0x100ad100 / 0x100affff).
const PED_FIRE: u32 = 0x730;
const VEHICLE_FIRE: u32 = 0x490;
/// `CPed::m_pVehicle`.
const PED_VEHICLE: u32 = 0x58C;
const PED_FLAGS_IN_VEHICLE: u32 = 0x46C;
/// `CFire::Extinguish` (thiscall).
const GTA_FIRE_EXTINGUISH: u32 = 0x5393F0;
/// `CWorld::PlayerInFocus` (B 0x10113978 → 0xB7CD74).
const GTA_PLAYER_IN_FOCUS: u32 = 0xB7CD74;
/// GTA gravity and samp's default (+0x3d5 +0x16 = 0x3c03126f = 0.008).
const GTA_GRAVITY: u32 = 0x863984;
const DEFAULT_GRAVITY: f32 = 0.008;

const VK_SHIFT: i32 = 0x10;
const VK_LEFT: i32 = 0x25;
const VK_RIGHT: i32 = 0x27;

pub struct Client {
    net: NetClient,
    runner: Runner,
    class_sel: ClassSelection,
    sender: FootSender,
    spawn_info: Option<player::SpawnInfo>,
    spawned: bool,
    /// RPC 74 flag: re-enter class selection on the next death (no death handling yet).
    force_class_selection: bool,
    on_foot_rate: u32,
    last_stats: Option<Instant>,
    debug_spawn: bool,
    fade_patched: bool,
    last_net_log: Option<Instant>,
    spawned_once: bool,
    first_frame_done: bool,
    /// Clock and weather (world.rs): settings hour/minute/weather, clock state.
    world: crate::world::World,
    /// InitGame "zone names" (settings +0x21): DISPLAY_ZONE_NAMES every frame
    /// from 1000 ms after the spawn (local tick A 0x100074c0, spawn A 0x10003c20).
    zone_names: bool,
    zone_names_from: Option<Instant>,
    /// Last interior reported with RPC 118 (local player +0x1cf, starts at 0).
    last_interior: u8,
    /// Phone special action active (local ped object +0x48).
    phone: bool,
    /// Held item / piss state of the local ped (FEAT-B, `specials.rs`).
    specials: crate::specials::PedExtras,
    /// Local head look (A 0x10002fa0): offset refreshed every 1000 ms, LookAt every 200 ms.
    look_offset: [f32; 3],
    look_refresh: Option<Instant>,
    look_last: Option<Instant>,
    /// Remote players (M3, players.rs).
    players: crate::players::Players,
    /// Vehicles (M5, vehicles.rs).
    vehicles: crate::vehicles::Vehicles,
    /// Combat (M7, combatclient.rs).
    combat: crate::combatclient::Combat,
    /// Local player "dead" flag (local player +0xF4): set with RPC 53, cleared by the respawn.
    dead: bool,
    /// InitGame class count (class selection after a forced reselection).
    class_count: u32,
    /// samp's netgame state and connect timer (lifecycle.rs, M10).
    netgame: NetGame,
    /// Start of the netgame clock (GetTickCount stand-in).
    clock: Instant,
    /// `host`, `port` for the `Connecting to` line.
    connect_target: (String, u16),
    /// Objects (M6, objects.rs).
    objects: crate::objects::Objects,
    /// Pickups, actors, checkpoints, gang zones, map icons, 3D labels (M6, entities.rs).
    entities: crate::entities::Entities,
    /// Camera, spectating, bounds, drunk, animations, ClientCheck (M8b, behaviourclient.rs).
    behaviour: crate::behaviourclient::Behaviour,
    /// Camera target reporting (RPC 170 → 168, camtarget.rs).
    cam_target: crate::camtarget::CamTarget,
}

/// Raw write into game data (heap objects, .data/.bss); no protection change.
fn poke<T: Copy>(addr: u32, v: T) {
    if addr == 0 {
        return;
    }
    // SAFETY: callers pass addresses of live game objects / game globals.
    unsafe { std::ptr::write_unaligned(addr as usize as *mut T, v) };
}

/// F4 (B 0x100074B0 `GetAsyncKeyState(0x73)`) and its client line (B string 0x100E5910, type-4 helper 0x10068030).
const VK_F4: i32 = 0x73;
pub const F4_LINE: &str = "Returning to class selection after next death";

/// `GetAsyncKeyState(vk) != 0` (either bit), like samp's F4 test.
fn async_key_any(vk: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // SAFETY: plain Win32 query.
    (unsafe { GetAsyncKeyState(vk) }) != 0
}

fn async_key_down(vk: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // SAFETY: plain Win32 query.
    // Only the "held now" bit, like the original (a stale "pressed since the
    // last call" bit must not trigger a spawn request after the guard).
    (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0
}

impl Client {
    /// Creates the session from the command line (`-n/-h/-p`) and the auth
    /// table named by [`AUTH_TABLE_ENV`].
    pub fn start(args: &crate::cmdline::ConnectArgs) -> Result<Self, String> {
        let host = args.host.clone().unwrap_or_else(|| "127.0.0.1".into());
        let port = args.port.unwrap_or(7777);
        let name = args.name.clone().unwrap_or_else(|| "RustClient".into());
        let path = std::env::var(AUTH_TABLE_ENV)
            .map_err(|_| format!("{AUTH_TABLE_ENV} is not set (path of data/auth_table.txt)"))?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read auth table {path}: {e}"))?;
        let auth = AuthTable::parse(&text).map_err(|e| format!("auth table {path}: parse error at line {}", e.line))?;
        let server = crate::netclient::resolve(&host, port).map_err(|e| format!("resolve {host}:{port}: {e}"))?;
        crate::log!("client: connecting to {server} as {name:?} (auth table {path}, {} entries)", auth.len());
        let net = NetClient::new(server, &name, auth).map_err(|e| format!("udp socket: {e}"))?;
        // "Started"; the connect tick starts the session 3.0 s later (lifecycle.rs: more than
        // 3000 ms after the netgame constructor; TRACE-3 U-CONNECT-TIMING; parity run: join in
        // frame 251 in both clients).
        crate::ui::on_netgame_start(&host, port, &name);
        let now = Instant::now();
        Ok(Self {
            net,
            runner: Runner::new(),
            class_sel: ClassSelection::new(now),
            sender: FootSender::default(),
            spawn_info: None,
            spawned: false,
            force_class_selection: false,
            on_foot_rate: policy::DEFAULT_ON_FOOT_RATE_MS,
            last_stats: None,
            debug_spawn: false,
            fade_patched: false,
            last_net_log: None,
            spawned_once: false,
            first_frame_done: false,
            world: crate::world::World::default(),
            zone_names: false,
            zone_names_from: None,
            last_interior: 0,
            phone: false,
            specials: Default::default(),
            look_offset: [0.0; 3],
            look_refresh: None,
            look_last: None,
            players: crate::players::Players::default(),
            vehicles: crate::vehicles::Vehicles::default(),
            combat: crate::combatclient::Combat::new(),
            dead: false,
            class_count: 0,
            netgame: NetGame::new(0),
            clock: now,
            connect_target: (host.clone(), port),
            objects: crate::objects::Objects::default(),
            entities: crate::entities::Entities::default(),
            behaviour: crate::behaviourclient::Behaviour::new(),
            cam_target: crate::camtarget::CamTarget::default(),
        })
    }

    pub fn status_line(&self) -> String {
        format!(
            "{} class_selection={} class={} spawned={}",
            self.net.status_line(),
            self.class_sel.is_active(),
            self.class_sel.index(),
            self.spawned
        )
    }

    /// Control-channel `remote`: one header line plus one line per remote player.
    pub fn remote_lines(&self) -> String {
        self.players.status(Instant::now(), |p| self.vehicles.remote_fields(p))
    }

    /// Control-channel `vehicles`: one header line plus one line per streamed vehicle.
    pub fn vehicle_lines(&self) -> String {
        self.vehicles.status(&self.players)
    }

    /// Control-channel `world`: one header line plus one line per world entity.
    pub fn world_lines(&self) -> String {
        let mut lines = Vec::new();
        self.objects.status_lines(&mut lines);
        self.entities.status_lines(&mut lines);
        let mut out =
            format!(
                "world count={} {} {} {}",
                lines.len(),
                self.objects.header_fields(),
                self.entities.header_fields(),
                crate::plates::status()
            );
        for l in lines {
            out.push('\n');
            out.push_str(&l);
        }
        out
    }

    /// Control-channel `behaviour`: one line of M8b state.
    pub fn behaviour_line(&self) -> String {
        self.behaviour.status_line(self.spawned)
    }

    /// The call of `CCamera::Process` in `CGame::Process` (CSV row 9, dll.rs): the camera follows an attached object.
    pub fn camera_process(&mut self) {
        if matches!(self.net.phase(), Phase::Connected) {
            self.behaviour.camera_follow(&mut self.runner, &self.objects);
        }
    }

    /// CGame::Process pre-step (dll.rs): the per-frame object process (world-entities.md RE-10 §3).
    pub fn pre_process(&mut self) {
        if self.netgame.connected() {
            // The vehicle pool's plate pass (B 0x100A2741 → 0x1001ED20) before the object process.
            crate::plates::tick(&self.vehicles.plate_list(), &mut |t| crate::ui::paint_plate(t));
            self.objects.process(&mut self.runner, &self.vehicles, &self.players);
        }
    }

    /// Control-channel `spawn`: run the spawn routine on the next tick.
    pub fn request_debug_spawn(&mut self) -> String {
        if self.spawn_info.is_none() {
            return "error no spawn info yet".into();
        }
        self.debug_spawn = true;
        "spawn queued".into()
    }

    pub fn shutdown(&mut self) {
        self.net.disconnect(Instant::now());
    }

    /// `/quit` (A 0x100c3ec0): RakClient Disconnect(500) when the netgame is in state 5 (the
    /// notification goes out); the game keeps running until [`Self::quit_teardown`].
    pub fn begin_quit(&mut self) {
        crate::log!("client: /quit in netgame state {}", self.netgame.state);
        if self.netgame.connected() {
            self.net.disconnect(Instant::now());
        }
    }

    /// The netgame's destruction before samp's `ExitProcess` (A 0x10009880): RakClient
    /// Disconnect(0) (no notification), then the pools in samp's order: players (remote peds),
    /// vehicles (occupants warped out first), actors, textdraws, gang zones, 3D labels, pickups,
    /// objects, menus. Textdraws, zones, labels and menus have no game-side objects.
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

    fn ped(&self) -> u32 {
        ProcessMemory.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0)
    }

    fn ped_handle(&self, ped: u32) -> Option<i32> {
        game::ped_handle(&ProcessMemory, ped).ok().flatten()
    }

    /// One frame (main thread).
    pub fn tick(&mut self) {
        let now = Instant::now();
        // What the combat hooks queued during this frame's CGame::Process (206 / 115), first.
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
        // samp's main loop (B 0x1000b5a0): the packet handler first.
        for ev in self.net.tick(now) {
            self.on_event(ev, now);
        }
        // Remote 206 shots fired while the packets were handled (115 taking 1).
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        if self.debug_spawn {
            self.debug_spawn = false;
            crate::log!("client: debug spawn requested over the control channel");
            self.spawn();
        }
        if !self.first_frame_done {
            // The original's first frame: two ped flags on the local player
            // (normally already set from the CGame::Process hook in dll.rs).
            self.first_frame_done = true;
            if !crate::dll::PED_FLAGS_DONE.load(std::sync::atomic::Ordering::SeqCst)
                && let Some(h) = self.ped_handle(self.ped())
            {
                self.runner.run(op::SET_CHAR_DROPS_WEAPONS_WHEN_DEAD, &[Arg::Int(h), Arg::Int(1)]);
                self.runner.run(op::SET_CHAR_SUFFERS_CRITICAL_HITS, &[Arg::Int(h), Arg::Int(0)]);
            }
            self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(0)]);
            // samp's first frame (A 0x100c47d0): the GTA clock and the HUD clock string off
            // (CSV rows 354/355).
            let w = self.world.toggle_clock(false);
            crate::world::apply(&w);
            // CSV row 356: skills to 1000, then SetStatValue := RET (vehicles.md RE-10 "Driving skills";
            // also TRACE-5 "Stats at state 9").
            self.vehicles.first_frame();
        }
        // Every frame, like the original (netgame tick A 0x1000b5b0): the server's time
        // of day while the server holds the time, then the pre-loading of the animation
        // blocks and models samp uses.
        if let Some((h, m)) = self.world.time_of_day() {
            poke(crate::world::CLOCK_HOURS, h);
            poke(crate::world::CLOCK_MINUTES, m);
            self.runner.run(op::SET_TIME_OF_DAY, &[Arg::Int(i32::from(h)), Arg::Int(i32::from(m))]);
        }
        self.preload();
        // Remote players before the local player's tick and send (onfoot §6 step 5b).
        self.players.tick(&mut self.runner, now);
        self.vehicles.remote_tick(&mut self.runner, &mut self.players);
        self.vehicles.hook_events(&mut self.runner, &mut self.net);
        crate::ui::set_remotes(self.players.ui_snapshot(now));
        crate::ui::set_class_selection(self.class_sel.is_active());
        if self.behaviour.spectate.on && !self.spawned && matches!(self.net.phase(), Phase::Connected) {
            // Spectating without being spawned (B 0x10007A9A): class selection is left, then the spectator tick.
            self.behaviour.drunk_not_spawned(&mut self.runner);
            if self.class_sel.is_active() {
                self.class_sel.leave();
                crate::ui::class_selection_release();
            }
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
        } else if self.class_sel.is_active() {
            self.class_selection_tick(now);
        } else {
            // Death (RPC 53) and the respawn after the wasted sequence (local tick A 0x100074C0).
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
        // F4 at the end of the local tick (B 0x100074B0), except while spectating unspawned: once per pending
        // flag, "class selection after the next death" (the flag RPC 74 sets) and its client line.
        if !(self.behaviour.spectate.on && !self.spawned) && !self.force_class_selection && async_key_any(VK_F4) {
            self.force_class_selection = true;
            crate::log!("client: F4 -> class selection after the next death");
            crate::ui::print_info(F4_LINE.as_bytes());
        }
        // The connect tick (A 0x10008940), in state 1 only; GetTickCount at this point (the
        // packet handler may have blocked in RakNet's Disconnect).
        let now_ms = Instant::now().duration_since(self.clock).as_millis() as u64;
        if self.netgame.connect_due(now_ms) {
            let (host, port) = self.connect_target.clone();
            crate::ui::print_debug(&lifecycle::msg::connecting(&host, port));
            if let Err(e) = self.net.connect() {
                crate::log!("client: connect failed: {e}");
            }
            // samp reads GetTickCount again after RakClient::Connect returned (A 0x10008985).
            let after = Instant::now();
            self.netgame.attempt_started(after.duration_since(self.clock).as_millis() as u64);
            for ev in self.net.tick(after) {
                self.on_event(ev, after);
            }
        }
        // Remote peds whose ProcessControl faulted (swap.rs fault containment).
        self.players.handle_faults(&mut self.runner);
        // `/dl` labels (B 0x10075760), rebuilt every tick while the flag is on.
        let spectating = self.behaviour.spectate.on;
        crate::vehlabels::tick(self.vehicles.debug_rows(), self.ped(), spectating);
        for line in crate::swap::take_fault_lines() {
            crate::ui::print_debug(&line);
        }
        // Chat input and connection lines of the in-game UI (M4).
        crate::ui::net_tick(&mut self.net);
        // The vehicle pool tick follows the local player's (remote-control-vehicles §6 step 3c).
        if self.netgame.connected() {
            self.vehicles.pool_tick(&mut self.runner, &mut self.net, &self.players, now);
            // World entities (M6): hook events (RPC 131 / 177), checkpoints, pickup poll, labels.
            self.entities.hook_events(&mut self.net);
            self.objects.poll_editor(&mut self.runner, &mut self.net);
            self.entities.tick(&mut self.runner, &mut self.net, &self.players, &self.vehicles);
        }
        // The hooks' view for the next CGame::Process.
        self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        // Push out what this frame queued (sync packets, RPCs) right away.
        self.net.flush(now);
        if self.last_net_log.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(5)) {
            self.last_net_log = Some(now);
            crate::log!("client: {}", self.status_line());
        }
    }

    fn on_event(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::InitGame(g) => {
                self.on_foot_rate = g.on_foot_rate.clamp(10, 1000);
                // InitGame (A 0x10010660): hour, weather (applied), zone names, CJ walk.
                self.world.hour = g.world_time;
                self.world.weather = g.weather;
                crate::world::apply(&self.world.weather_writes(g.weather));
                self.zone_names = g.zone_names;
                crate::pedhooks::set_cj_walk(g.cj_walk);
                self.behaviour.on_init_game(g.gravity);
                self.behaviour.hash_table();
                crate::log!(
                    "client: InitGame: server {:?}, our id {}, {} class(es), on-foot rate {} ms",
                    g.server_name,
                    g.player_id,
                    g.spawns_available,
                    g.on_foot_rate
                );
                self.players.set_init_game(g.player_id, g.death_drop_money, g.player_marker_mode);
                self.combat.on_init_game(&g);
                self.class_count = g.spawns_available;
                crate::ui::on_init_game(&g);
                self.netgame.on_init_game();
                self.vehicles.set_init_game(&g);
                // The InitGame vehicle-model preload (B 0x100A1620; `/dl` reports its flags).
                crate::vehlabels::preload(&mut self.runner, &g.vehicle_models);
                self.objects.set_local_id(g.player_id);
                self.entities.set_local_id(g.player_id);
                self.enter_class_selection(now, g.spawns_available);
            }
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

    /// Lifecycle actions (lifecycle.rs), in order.
    fn apply_actions(&mut self, actions: Vec<Action>) {
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

    /// RPC 40 reset (B 0x1000a530; session-misc.md §40). Measured effects (TRACE-8 gmx):
    /// `0x04BB(0)`, `0x0860(char, 0)`, `0x04E4` at the ped, `0x0321(char)` in that frame, then
    /// the not-connected scene every frame until InitGame.
    fn reset_routine(&mut self) {
        crate::log!("client: reset routine (server restart / lost connection)");
        // Remote players lose their peds but stay in the pool.
        self.players.despawn_all(&mut self.runner);
        crate::log!("client: reset: remote peds removed");
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
        // Game side: interior 0 (B 0x100ad350(0, 1)), then the local ped is killed (B 0x100affe0).
        let ped = self.ped();
        crate::log!("client: reset: settings restored, local ped {ped:#x}");
        if let Some(h) = self.ped_handle(ped) {
            self.runner.run(OP_SELECT_INTERIOR, &[Arg::Int(0)]);
            self.runner.run(OP_LINK_CHAR_TO_INTERIOR, &[Arg::Int(h), Arg::Int(0)]);
            if let Some(p) = game::snapshot(&ProcessMemory).ok().and_then(|s| s.player) {
                self.runner.run(op::REQUEST_COLLISION, &[Arg::Float(p.pos[0]), Arg::Float(p.pos[1])]);
            }
            crate::log!("client: reset: interior 0");
            self.kill_local_ped(ped, h);
            crate::log!("client: reset: local ped killed");
        }
        // GTA gravity back to the default (B 0x100a1410 → 0x863984).
        // The gravity constant lives in a read-only page; samp unprotects it first (B 0x100aa4d0).
        if let Err(e) = ProcessMemory.write(GTA_GRAVITY, &DEFAULT_GRAVITY.to_le_bytes()) {
            crate::log!("client: reset: gravity write failed: {e}");
        }
        crate::ui::print_info(lifecycle::msg::RESTARTING.as_bytes());
    }

    /// B 0x100affe0: fires out, teleport to its own position, health 0, `EXPLODE_CHAR_HEAD`
    /// with `CWorld::PlayerInFocus` set to the ped's player number around it.
    fn kill_local_ped(&mut self, ped: u32, handle: i32) {
        let m = ProcessMemory;
        let extinguish = |fire: u32| {
            if fire != 0 {
                // SAFETY: CFire::Extinguish (thiscall) on the game's fire object.
                let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(GTA_FIRE_EXTINGUISH as usize) };
                f(fire);
            }
        };
        extinguish(m.read_u32(ped + PED_FIRE).unwrap_or(0));
        if m.read_u32(ped + PED_FLAGS_IN_VEHICLE).unwrap_or(0) & 0x100 != 0 {
            let veh = m.read_u32(ped + PED_VEHICLE).unwrap_or(0);
            if veh != 0 {
                extinguish(m.read_u32(veh + VEHICLE_FIRE).unwrap_or(0));
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
            // Bullet sync fires its shot at once (A 0x10009B40 → 0x10016370; combat §8 P10).
            S::Bullet(b) => {
                self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
                self.combat.on_bullet(&mut self.runner, &mut self.players, &self.vehicles, &b);
            }
            S::Markers(m) => self.players.on_markers(&mut self.runner, &m),
            s => self.players.on_sync(s, now),
        }
    }

    fn on_rpc(&mut self, r: Incoming) {
        use player::Incoming as P;
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        crate::ui::on_rpc(&r);
        match r {
            // RPC 40 (gmx): the reset routine, state 11 until the server sends InitGame again.
            Incoming::Session(samp_proto::rpcs::session::Incoming::PlayerClose(_)) => {
                let actions = self.netgame.on_server_restart();
                self.apply_actions(actions);
            }
            Incoming::Ui(ui::Incoming::SendClientMessage(m)) => {
                crate::log!("[chat {:08x}] {}", m.colour, text(&m.message));
            }
            Incoming::Ui(ui::Incoming::SendGameText(g)) => crate::log!("[gametext] {}", text(&g.text)),
            // M9 audio (audio.rs): RPC 41 / 42.
            Incoming::Ui(ui::Incoming::PlayAudioStreamForPlayer(a)) => crate::audio::on_play_rpc(&a),
            Incoming::Ui(ui::Incoming::StopAudioStreamForPlayer(_)) => {
                crate::audio::stop(false);
            }
            Incoming::Remote(r) => {
                use samp_proto::rpcs::remote::Incoming as R;
                // Colour table (B 0x100A7220): PlayerJoin (B 0x1000FDE2) sets it only for a non-zero colour and an
                // id below 1004 (the spawn of PlayerStreamIn: players.rs).
                if let R::PlayerJoin(j) = &r
                    && j.colour != 0
                    && j.player_id < 1004
                {
                    let (id, colour) = (j.player_id, j.colour);
                    crate::worldhooks::with_shared(|s| {
                        s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(id, colour)
                    });
                }
                let (streamed_in, quit) = match &r {
                    R::PlayerStreamIn(s) => (Some(s.player_id), None),
                    R::PlayerQuit(q) => (None, Some(q.player_id)),
                    R::PlayerStreamOut(s) => {
                        self.vehicles.reset_remote_vehicle(s.player_id);
                        (None, None)
                    }
                    _ => (None, None),
                };
                match &r {
                    R::PlayerJoin(j) => self.behaviour.forget_remote(j.player_id),
                    R::PlayerQuit(q) => self.behaviour.forget_remote(q.player_id),
                    R::TogglePlayerSpectating(t) => {
                        if self.behaviour.toggle_spectating(t.enable != 0) {
                            self.spawn();
                        }
                    }
                    R::PlayerSpectatePlayer(p) => {
                        self.behaviour.spectate_player(&self.players, p.player_id, p.gta_camera_mode())
                    }
                    R::PlayerSpectateVehicle(v) => {
                        self.behaviour.spectate_vehicle(&self.vehicles, v.vehicle_id, v.gta_camera_mode())
                    }
                    _ => {}
                }
                self.players.on_rpc(&mut self.runner, r);
                if let Some(id) = streamed_in {
                    self.vehicles.after_stream_in(&mut self.runner, &mut self.players, id);
                }
                if let Some(id) = quit {
                    self.vehicles.forget_player(id);
                }
            }
            Incoming::Vehicle(v) => self.vehicles.on_rpc(&mut self.runner, &mut self.players, v),
            Incoming::World(w) => {
                use samp_proto::rpcs::world::Incoming as W;
                match w {
                    W::CreateObject(_)
                    | W::SetObjectPosition(_)
                    | W::SetObjectRotation(_)
                    | W::DestroyObject(_)
                    | W::MoveObject(_)
                    | W::StopObject(_)
                    | W::AttachObjectToPlayer(_)
                    | W::SetPlayerObjectMaterial(_)
                    | W::SetObjectNoCameraCollision(_)
                    | W::SetPlayerAttachedObject(_)
                    | W::PlayerBeginObjectSelect(_)
                    | W::PlayerCancelObjectEdit(_)
                    | W::PlayerBeginObjectEdit(_)
                    | W::PlayerBeginAttachedObjectEdit(_) => {
                        self.objects.on_rpc(&mut self.runner, &mut self.net, &self.players, w)
                    }
                    w => self.entities.on_rpc(&mut self.runner, w),
                }
            }
            Incoming::Session(samp_proto::rpcs::session::Incoming::ClientCheckRequest(c)) => {
                self.behaviour.client_check(&mut self.net, &mut self.runner, &c)
            }
            Incoming::Session(samp_proto::rpcs::session::Incoming::SetVehicleTyreStatus(t)) => {
                self.vehicles.on_tyres(t.vehicle_id, t.tyres)
            }
            Incoming::Session(samp_proto::rpcs::session::Incoming::SetVehicleParamsForPlayerOld(p)) => {
                self.vehicles.on_old_params(p.vehicle_id, p.objective, p.doors_locked)
            }
            Incoming::Player(p) => match p {
                P::PlayerRequestClassResponse(c) => {
                    crate::log!(
                        "client: class response selectable={} skin={} pos={:?} rot={}",
                        c.selectable,
                        c.spawn.skin,
                        c.spawn.pos,
                        c.spawn.rotation
                    );
                    if c.selectable != 0 {
                        self.store_spawn_info(c.spawn);
                        // Spec: clear weapons and show the class skin on the ped.
                        let ped = self.ped();
                        if ped != 0 {
                            self.clear_weapons(ped);
                            self.set_skin(ped, c.spawn.skin);
                        }
                    }
                    self.class_sel.on_class_response(c.selectable != 0);
                }
                P::SetSpawnInfo(s) => self.store_spawn_info(s.spawn),
                P::PlayerRequestSpawnResponse(s) => {
                    crate::log!("client: spawn response {}", s.response);
                    if self.class_sel.on_spawn_response(s.response) {
                        self.spawn();
                    }
                }
                P::SetPlayerCameraTargeting(t) => self.cam_target.enabled = t.enabled,
                // RPC 60 (B 0x1000FF10 → 0x100A1760): with the HUD clock off, GTA's millisecond timer := time & 0x3FFFFFFF.
                P::SendGameTimeUpdate(t) => {
                    if !self.world.clock_enabled {
                        poke(crate::world::TIMER_MS, (t.time as u32) & 0x3FFF_FFFF);
                    }
                }
                // RPC 48 (B 0x1001DC70): stored in the custom-model manager, which R4 never creates (session-misc.md
                // "179"), so nothing happens; the value is only logged.
                P::SetPlayerVirtualWorld(w) => crate::log!("client: RPC 48 virtual world {} (no effect in R4)", w.world_id),
                P::PlayCrimeReport(c) => self.crime_report(&c),
                P::ForcePlayerClassSelection(_) => {
                    crate::log!("client: server forces class selection on next death");
                    self.force_class_selection = true;
                }
                P::SetPlayerPosition(s) => self.teleport(s.pos.x, s.pos.y, s.pos.z),
                // RPC 13 (player-state.md): the ground height at (x, y, z) from 0x02CE, plus 1.5, then as RPC 12.
                P::SetPlayerPositionFindZ(s) => {
                    if self.ped() != 0 {
                        self.runner.run(
                            op::GET_GROUND_Z_FOR_3D_COORD,
                            &[Arg::Float(s.pos.x), Arg::Float(s.pos.y), Arg::Float(s.pos.z), Arg::Local(0)],
                        );
                        let z = self.runner.local_f32(0) + 1.5;
                        self.teleport(s.pos.x, s.pos.y, z);
                    }
                }
                P::SetPlayerFacingAngle(a) => self.set_facing(a.angle),
                P::SetPlayerHealth(h) => {
                    let ped = self.ped();
                    if ped != 0 {
                        poke(ped + off::PED_HEALTH, h.health);
                    }
                }
                P::SetPlayerArmour(a) => {
                    let ped = self.ped();
                    if ped != 0 {
                        poke(ped + off::PED_ARMOUR, a.armour);
                    }
                }
                P::TogglePlayerControllable(t) => self.set_controllable(t.enable != 0),
                P::SetPlayerCameraPosition(c) => {
                    self.behaviour.camera_position(&mut self.runner, [c.pos.x, c.pos.y, c.pos.z])
                }
                P::SetPlayerCameraLookAt(c) => {
                    self.behaviour.camera_look_at(&mut self.runner, [c.pos.x, c.pos.y, c.pos.z], c.cut)
                }
                P::SetPlayerCameraBehindPlayer(_) => self.camera_behind_player(),
                P::InterpolateCamera(c) => self.behaviour.interpolate(
                    &mut self.runner,
                    c.is_position,
                    [c.from.x, c.from.y, c.from.z],
                    [c.to.x, c.to.y, c.to.z],
                    c.time,
                    c.cut,
                ),
                P::AttachCameraToObject(a) => {
                    self.behaviour.attach_camera(&mut self.runner, &self.objects, a.object_id)
                }
                P::ToggleWidescreen(w) => crate::behaviourclient::widescreen(&mut self.runner, w.enable),
                P::SetWorldBounds(b) => {
                    self.behaviour.bounds =
                        crate::behaviour::Bounds { max_x: b.max_x, min_x: b.min_x, max_y: b.max_y, min_y: b.min_y }
                }
                P::SetPlayerGravity(g) => crate::behaviourclient::set_gravity(g.gravity),
                P::SetPlayerInterior(i) => crate::behaviourclient::set_interior(&mut self.runner, i.interior),
                P::ApplyPlayerAnimation(a) => {
                    let local = self.net.player_id();
                    self.behaviour.apply_animation(&mut self.runner, &self.players, local, &a)
                }
                P::ClearPlayerTasks(c) => {
                    let local = self.net.player_id();
                    self.behaviour.clear_animations(&self.players, local, c.player_id)
                }
                P::SetPlayerDrunkLevel(d) => self.behaviour.drunk = d.level,
                P::SetPlayerFightingStyle(f) => {
                    let ped = if Some(f.player_id) == self.net.player_id() {
                        self.ped()
                    } else {
                        self.players.get(f.player_id).map(|p| p.ped).unwrap_or(0)
                    };
                    crate::behaviourclient::fighting_style(&mut self.runner, ped, f.style)
                }
                P::SetPlayerShopName(n) => crate::behaviourclient::shop_name(&n.name),
                P::SetPlayerWantedLevel(w) => crate::behaviourclient::set_wanted(w.level),
                P::EnableStuntBonusForPlayer(e) => crate::behaviourclient::set_stunt_bonus(e.enable),
                P::GivePlayerMoney(m) => crate::behaviourclient::give_money(&mut self.runner, m.amount),
                P::ResetPlayerMoney(_) => crate::behaviourclient::reset_money(&mut self.runner),
                P::SetPlayerVelocity(v) => {
                    crate::behaviourclient::set_velocity([v.velocity.x, v.velocity.y, v.velocity.z])
                }
                P::GivePlayerWeapon(g) => {
                    let ped = self.ped();
                    if ped != 0 {
                        self.give_weapon(ped, g.weapon.min(255) as u8, g.ammo);
                    }
                }
                P::SetPlayerAmmo(a) => {
                    crate::log!("client: RPC 145 weapon {} ammo {}", a.weapon, a.ammo);
                    crate::combatclient::set_ammo(self.ped(), a.weapon, a.ammo)
                }
                P::SetPlayerSkillLevel(k) => {
                    if Some(k.player_id) == self.net.player_id() {
                        crate::combatclient::set_local_skill(k.skill_type, k.level);
                    } else if k.player_id < 1004 {
                        self.players.set_skill(k.player_id, k.skill_type, k.level);
                    }
                }
                P::SetPlayerTeam(t) => {
                    if Some(t.player_id) == self.net.player_id() {
                        self.combat.local_team = t.team;
                    } else {
                        self.players.set_team(t.player_id, t.team);
                    }
                }
                P::PlayerDeath(d) => {
                    // The local id is not special-cased by samp, but it has no remote entry (ignored).
                    if d.player_id < 1005 && Some(d.player_id) != self.net.player_id() {
                        self.players.kill(&mut self.runner, d.player_id);
                        self.vehicles.reset_remote_vehicle(d.player_id);
                    }
                }
                P::CreateExplosion(e) => crate::combatclient::create_explosion(&mut self.runner, &e),
                P::SetPlayerTime(t) => {
                    // A 0x1000f420: GTA clock, SET_TIME_OF_DAY, settings.
                    poke(crate::world::CLOCK_HOURS, t.hour);
                    poke(crate::world::CLOCK_MINUTES, t.minute);
                    self.runner.run(op::SET_TIME_OF_DAY, &[Arg::Int(i32::from(t.hour)), Arg::Int(i32::from(t.minute))]);
                    self.world.hour = t.hour;
                    self.world.minute = t.minute;
                }
                P::SetPlayerWorldTime(t) => self.world.hour = t.hour,
                P::TogglePlayerClock(t) => {
                    let mem = ProcessMemory;
                    let cur = (
                        mem.read_u8(crate::world::CLOCK_HOURS).unwrap_or(0),
                        mem.read_u8(crate::world::CLOCK_MINUTES).unwrap_or(0),
                    );
                    let w = self.world.on_toggle_clock_rpc(t.enable != 0, cur);
                    crate::world::apply(&w);
                }
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
                // B 0x100197B0: the local id, or a remote id up to 1004 that is in the pool.
                P::SetPlayerColor(c)
                    if Some(c.player_id) == self.net.player_id()
                        || (c.player_id <= 1004 && self.players.get(c.player_id).is_some()) =>
                {
                    crate::worldhooks::with_shared(|s| {
                        s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(c.player_id, c.colour)
                    })
                }
                P::SetPlayerColor(_) => {}
                P::SetPlayerArmedWeapon(w) => {
                    // A 0x10018E50: 0…46 only; set-armed-weapon (A 0x100AC0A0, not forced).
                    if w.weapon <= 46 {
                        let ped = self.ped();
                        crate::combatclient::set_armed_local(&mut self.runner, ped, w.weapon as u8);
                    }
                }
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
            },
            _ => {}
        }
    }

    /// RPC 68 / 128 spawn block, with the client's range check (player-state.md 68).
    fn store_spawn_info(&mut self, s: player::SpawnInfo) {
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

    // ---- connecting -------------------------------------------------------

    /// Every frame until InitGame: the original parks the ped and shows a fixed
    /// view of Los Santos (TRACE-1: teleport to (1133.05, -2038.40, 69.1),
    /// SET_FIXED_CAMERA_POSITION (1093, -2036, 90), POINT_CAMERA_AT_POINT (384, -1557, 20)).
    fn connecting_tick(&mut self) {
        // A 0x100089b0 also forces weather 1 every frame while not connected.
        crate::world::apply(&self.world.weather_writes(crate::world::CONNECTING_WEATHER));
        set_hud(false);
        self.teleport(1133.05, -2038.40, 69.1);
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.runner.run(op::SET_FIXED_CAMERA_POSITION, &[1093.0, -2036.0, 90.0, 0.0, 0.0, 0.0].map(Arg::Float));
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.runner
            .run(op::POINT_CAMERA_AT_POINT, &[Arg::Float(384.0), Arg::Float(-1557.0), Arg::Float(20.0), Arg::Int(2)]);
    }

    // ---- class selection --------------------------------------------------

    fn enter_class_selection(&mut self, now: Instant, count: u32) {
        crate::log!("client: entering class selection ({count} class(es))");
        self.runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(596)]);
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

    fn class_selection_tick(&mut self, now: Instant) {
        // While no frontend menu is open (samp reads the menu flag as a dword at
        // FrontEndMenuManager+0x5C, A 0x100a0980): HUD off, every frame (A 0x10006100).
        // Its input-lock mode 2 comes from the UI (set_class_selection → the class panel
        // requests mode 2 every frame; measured: 0x541DF5 NOPed from the join frame on).
        if ProcessMemory.read_u32(game::MENU_ACTIVE).unwrap_or(0) == 0 {
            set_hud(false);
        }
        let mem = ProcessMemory;
        // Clicks on the class-selection panel (<<, >>, Spawn) act like the keys.
        let click = crate::ui::take_class_click();
        let keys = classsel::Keys {
            left: click == 1 || async_key_down(VK_LEFT) || game::key_down(&mem, game::key::LEFT),
            right: click == 2 || async_key_down(VK_RIGHT) || game::key_down(&mem, game::key::RIGHT),
            shift: click == 3
                || async_key_down(VK_SHIFT)
                || game::key_down(&mem, game::key::SHIFT)
                || game::key_down(&mem, game::key::LSHIFT)
                || game::key_down(&mem, game::key::RSHIFT),
        };
        match self.class_sel.tick(now, keys) {
            Some(classsel::Action::RequestClass(i)) => {
                crate::log!("client: requesting class {i}");
                self.net.send_rpc(&player::PlayerRequestClass { class_index: i }, Reliability::Reliable);
            }
            Some(classsel::Action::RequestSpawn) => {
                crate::log!("client: requesting spawn");
                self.net.send_rpc(&player::PlayerRequestSpawn, Reliability::Reliable);
            }
            None => {}
        }
    }

    // ---- local spawn routine (player-state.md "Local Spawn routine") -------

    fn spawn(&mut self) -> bool {
        let Some(s) = self.spawn_info else {
            crate::log!("client: spawn: no spawn info");
            return false;
        };
        let ped = self.ped();
        if ped == 0 {
            crate::log!("client: spawn: no local ped");
            return false;
        }
        // 2. leave class selection; its input lock ends 10 frames later (A 0x100a0750(0, 0)).
        if self.class_sel.is_active() {
            self.class_sel.leave();
        }
        crate::ui::class_selection_release();
        // 3. camera, HUD, control (TRACE-1 order: 02EB, 0930/0925/0373/02EB, 01B4, 04D7, teleport).
        self.behaviour.attached = None;
        self.runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
        self.camera_behind_player();
        set_hud(true);
        self.set_controllable(true);
        // 4. ped reset (the original skips it on the first spawn: one-shot flag), area,
        //    restart point, skin, weapons, damage record.
        if self.spawned_once {
            self.reset_ped_state(ped);
        }
        self.runner.run(op::REQUEST_COLLISION, &[Arg::Float(s.pos.x), Arg::Float(s.pos.y)]);
        // Restart override at the spawn point (CSV row 385; the traced build uses
        // ADD_HOSPITAL_RESTART instead of writing the restart table directly).
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
        // 6. no random trains; CCamera::Fade := ret 8 (CSV row 365).
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
        // (The original also issues two REMOVE_BLIP commands here with handles
        // of its own that are never valid blips; nothing to reproduce.)
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

    // ---- death and respawn (local tick A 0x100074C0) -------------------------

    fn death_tick(&mut self, now: Instant) {
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
        // Cuffs off (A 0x100ACF40(0)); the jetpack does not exist here.
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
    /// RPC 53 with the killer / reason of A 0x100AC890, then dead := 1, spawned := 0, zone names off.
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
        crate::log!("client: local death: weapon {weapon} damager {damager:?} -> RPC 53 reason {reason} killer {killer}");
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
            let driver = ProcessMemory.read_u32(entity + 0x460).unwrap_or(0);
            let id = if driver != 0 { self.players.iter().find(|p| p.ped == driver).map(|p| p.id) } else { None };
            return Damager::Vehicle { driver: id };
        }
        Damager::Other
    }

    // ---- sync ---------------------------------------------------------------

    fn sync_tick(&mut self, now: Instant) {
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
        // Spectating (B 0x100078E9): the spectator tick replaces the on-foot / driver branches.
        if self.behaviour.spectate.on {
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
            self.stats_tick(now);
            return;
        }
        // On foot: the surf probe (A 0x10003730), before the head look.
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER) {
            surf_probe(ped);
        }
        // Head look in the on-foot, driver and passenger branches of the local tick.
        // `disableheadmove` clears samp's Game+0x59 flag, which the head look tests first
        // (B 0x10002fbd; set at boot B 0x100c4ef7 and by `/headmove`).
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER | PEDSTATE_DRIVING) && !crate::ui::head_move_disabled() {
            self.head_look(ped, now);
        }
        // 205 (> 1000 ms) and 204 (every tick) come before the per-state packets (A 0x100078A8 / 0x100078C9).
        self.stats_tick(now);
        self.combat.weapons_tick(&mut self.net, &self.players, ped, now);
        self.combat.cuffed_tick(&mut self.runner, ped);
        // Local passenger drive-by (A 0x10007B68), before the 211.
        let in_vehicle_flag = mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        let veh = mem.read_u32(ped + 0x58C).unwrap_or(0);
        let is_driver = in_vehicle_flag && veh != 0 && mem.read_u32(veh + 0x460).unwrap_or(0) == ped;
        if in_vehicle_flag && veh != 0 && !is_driver {
            self.combat.passenger_drive_by(&mut self.runner, ped);
        } else if self.combat.drive_by {
            self.combat.drive_by = false;
        }
        // Driving: the world bounds on the vehicle (B 0x1000793D), before the driver packets.
        if state == PEDSTATE_DRIVING {
            // B 0x10007903…0x1000792F: unless the ped is a passenger (or sits in a 569/570 carriage, B 0x100ABFD0),
            // `CPed::RemoveWeaponWhenEnteringVehicle(0)` every driving frame.
            let model = if veh != 0 { mem.read_u16(veh + 0x22).unwrap_or(0) } else { 0 };
            if !(in_vehicle_flag && veh != 0 && (!is_driver || matches!(model, 569 | 570))) {
                // SAFETY: GTA's thiscall method on the live local ped, main thread.
                unsafe {
                    let f: extern "thiscall" fn(u32, i32) = std::mem::transmute(PED_REMOVE_WEAPON_ENTERING_VEHICLE);
                    f(ped, 0);
                }
            }
            self.behaviour.bounds_driver(&mut self.runner, &self.vehicles, ped);
        }
        // In-car packets (200/210/211/106, passenger poll); on-foot sync only in ped states 1 and 12, so nothing
        // is sent while entering a seat from the server's warp or during the exit animation (TRACE-4 §4).
        self.vehicles.local_drive_by = self.combat.drive_by;
        self.vehicles.local_cuffed = self.combat.cuffs.cuffed;
        let in_car = self.vehicles.local_tick(&mut self.runner, &mut self.net, self.players.streamed(), now);
        if in_car && in_vehicle_flag && veh != 0 {
            self.combat.aim_in_car(&mut self.net, &mut self.vehicles, ped, is_driver, now);
        }
        if !in_car && state != PEDSTATE_DRIVING {
            // On foot (B 0x10003F40): the world-bounds check.
            self.behaviour.bounds_on_foot(&mut self.runner, ped);
            match game::read_local_sample(&mem, ped) {
                Ok(sample) => {
                    let keys = footsync::key_word(&sample.pad, sample.key_ctx);
                    let interval = footsync::on_foot_interval(self.on_foot_rate, self.players.streamed());
                    if self.sender.due(now, interval, keys, sample.pad.left_stick_x, sample.pad.left_stick_y) {
                        // The 207 builder (A 0x10004f00) reads the weapon with GET_CURRENT_CHAR_WEAPON.
                        self.current_weapon(ped);
                        let mut sample = sample;
                        // samp's order (A 0x10003410): jetpack, dance, hands up, phone, piss, duck.
                        sample.special_action = crate::specials::local_report(
                            sample.special_action,
                            self.phone,
                            self.specials.piss,
                            self.specials.item,
                        );
                        sample.animation = self.behaviour.anim.take();
                        let f = footsync::build_foot_sync(&sample, self.sender.last_rotation());
                        if let Some(f) = self.sender.offer(now, f) {
                            self.net.send_sync(&f);
                        }
                    }
                    // 203 after the 207 (A 0x10007C9A).
                    self.combat.aim_on_foot(&mut self.net, &mut self.sender, ped, keys, now);
                }
                Err(e) => crate::log!("client: cannot read local ped: {e}"),
            }
        }
    }

    /// 205 every > 1000 ms (B 0x100078A3): money and the drunk level (B 0x100ADFB0).
    fn stats_tick(&mut self, now: Instant) {
        if self.last_stats.is_none_or(|t| now.duration_since(t) > footsync::STATS_INTERVAL) {
            self.last_stats = Some(now);
            let money = ProcessMemory.read_i32(game::PLAYER_MONEY).unwrap_or(0);
            self.net.send_sync(&StatsSync { money, drunk_level: self.behaviour.drunk });
        }
    }

    // ---- local per-frame commands (local tick A 0x100074c0) -----------------

    /// The script commands samp's local tick issues every frame while spawned,
    /// before the sync sends (TRACE-1 "Walking": 052C, 09BA, 0470, 077E).
    fn local_frame(&mut self, now: Instant) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        // `/ctd` labels (B 0x10004830, early in the spawned branch of the local tick).
        if crate::ui::camera_target_labels_on() {
            self.draw_camera_target_labels();
        }
        // Hands up + Triangle (enter/exit key) → control on again, which re-sets the ped
        // to its own position and so ends the task (A 0x100074c0 → 0x100ac7d0(1)).
        let triangle = ProcessMemory.read_u16(game::PAD0 + PAD_TRIANGLE).unwrap_or(0);
        if triangle != 0 && self.hands_up(ped) {
            self.set_controllable(true);
        }
        // Piss / held item (B 0x1000772C…0x100077AB): Triangle or a vehicle ends them, else the item is used.
        crate::specials::local_tick(&mut self.runner, ped, &mut self.specials, triangle != 0, &mut self.behaviour.drunk);
        // Drunk handling (B 0x100ADE00), then the 207 animation encoder (B 0x100AE7A0).
        self.behaviour.drunk_tick(&mut self.runner, ped);
        self.behaviour.anim_tick(ped);
        if self.zone_names_from.is_some_and(|t| now >= t) {
            self.runner.run(op::DISPLAY_ZONE_NAMES, &[Arg::Int(i32::from(self.zone_names))]);
        }
        // Weapons-update check (A 0x10006290; the 204 packet itself is not sent yet).
        self.current_weapon(ped);
        // Interior change → RPC 118 (A 0x100a0f20 / 0x10005970).
        self.runner.run(op::GET_AREA_VISIBLE, &[Arg::Local(0)]);
        let area = self.runner.local_i32(0) as u8;
        if area != self.last_interior {
            self.last_interior = area;
            crate::log!("client: interior {area} -> RPC 118");
            self.net.send_rpc(&player::OnPlayerInteriorChange { interior: area }, Reliability::Reliable);
        }
        // Camera target (B 0x100046E0, right after the interior check).
        self.camera_target_tick();
    }

    /// RPC 112 (B 0x10019000 → 0x100A17A0): the police scanner's stand-in ped (CSV row 55) is cleared, the
    /// on-foot description in `CAEPoliceScannerAudioEntity::AddAudioEvent` is NOPed (CSV row 361), a reported
    /// vehicle becomes a temporary vehicle the stand-in sits in, then the local player's scanner gets
    /// `AddAudioEvent(0xA4, crime, pos)`; the temporary vehicle is deleted afterwards. `suspect_id` is unused.
    fn crime_report(&mut self, c: &player::PlayCrimeReport) {
        let dummy = crate::dll::scanner_dummy_ped();
        if dummy != 0 {
            // SAFETY: our own leaked 0x7A0-byte buffer.
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
                poke(dummy + 0x58C, ptr);
                poke(dummy + off::PED_FLAGS, ProcessMemory.read_u32(dummy + off::PED_FLAGS).unwrap_or(0) | 0x100);
            }
        }
        if ped != 0 {
            let player_data = ProcessMemory.read_u32(ped + 0x480).unwrap_or(0);
            let wanted = if player_data != 0 { ProcessMemory.read_u32(player_data).unwrap_or(0) } else { 0 };
            if wanted != 0 {
                let pos = [c.pos.x, c.pos.y, c.pos.z];
                // SAFETY: CAEPoliceScannerAudioEntity::AddAudioEvent (thiscall) on the local player's CWanted scanner.
                unsafe {
                    let f: extern "thiscall" fn(u32, u32, u32, *const [f32; 3]) =
                        std::mem::transmute(SCANNER_ADD_AUDIO_EVENT as usize);
                    f(wanted + 0x21C, 0xA4, c.crime_id, &pos);
                }
            }
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

    /// RPC 119 (reliability 8), flushed at once (called from the Present pass while the pause menu is up).
    pub fn send_click_map(&mut self, pos: [f32; 3]) {
        crate::log!("client: RPC 119 {pos:?}");
        let p = samp_proto::rpcs::session::OnPlayerClickMap {
            pos: samp_proto::types::Vec3 { x: pos[0], y: pos[1], z: pos[2] },
        };
        self.net.send_rpc(&p, Reliability::Reliable);
        self.net.flush(Instant::now());
    }

    /// RPC 168 sender (camtarget.rs).
    fn camera_target_tick(&mut self) {
        let now_ms = tick_count();
        let send = if self.cam_target.enabled {
            let objects = self.objects.pick_list();
            let vehicles = self.vehicles.hook_vehicles();
            let peds: Vec<(u32, u16)> = self.players.iter().filter(|p| p.ped != 0).map(|p| (p.ped, p.id)).collect();
            let actors = self.entities.hook_actors();
            self.cam_target.tick(now_ms, || camera_targets(&objects, &vehicles, &peds, &actors))
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

    /// The local head follows the camera (A 0x10002fa0, unless `disableheadmove`): every 1000 ms the offset := 20 × camera
    /// front, every 200 ms `LookAt` at position + offset for 2000 ms.
    fn head_look(&mut self, ped: u32, now: Instant) {
        let m = ProcessMemory;
        let matrix = m.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
        if matrix == 0 {
            return;
        }
        let v3 =
            |a: u32| [m.read_f32(a).unwrap_or(0.0), m.read_f32(a + 4).unwrap_or(0.0), m.read_f32(a + 8).unwrap_or(0.0)];
        if self.look_refresh.is_none_or(|t| now.duration_since(t) > crate::remote::LOOK_REFRESH) {
            self.look_refresh = Some(now);
            self.look_offset = v3(CAMERA_FRONT).map(|c| c * crate::remote::LOOK_DISTANCE);
        }
        if self.look_last.is_none_or(|t| now.duration_since(t) > crate::remote::LOOK_INTERVAL) {
            self.look_last = Some(now);
            let pos = v3(matrix + off::MATRIX_POS);
            let mut target = [0, 1, 2].map(|i| pos[i] + self.look_offset[i]);
            crate::players::look_at(ped, 2000, &mut target);
        }
    }

    /// `GET_CURRENT_CHAR_WEAPON` on the local ped (samp A 0x100abc90).
    fn current_weapon(&mut self, ped: u32) -> u8 {
        let Some(h) = self.ped_handle(ped) else { return 0 };
        self.runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Local(0)]);
        self.runner.local_i32(0) as u8
    }

    /// The local ped's primary task is the hands-up task (samp A 0x100acba0).
    fn hands_up(&self, ped: u32) -> bool {
        let m = ProcessMemory;
        if m.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0 || !ped_in_world(ped) {
            return false;
        }
        let intel = m.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
        let task = if intel != 0 { m.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
        task != 0 && m.read_u32(task).ok() == Some(game::VTBL_TASK_HANDS_UP)
    }

    /// RPC 153 (A 0x10019190): valid ped models only; the local ped, or a streamed-in remote ped.
    fn on_set_player_skin(&mut self, player_id: u32, skin: u32) {
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

    /// RPC 88 (LocalPlayer method A 0x100030f0), for the actions this client has:
    /// 0 stops the phone, pissing and a held item (not hands up, as in samp); 10, 11,
    /// 13, 20–23 and 68 start their state. Jetpack, dances, cuffs and carry are not here.
    fn on_special_action(&mut self, action: u8) {
        use crate::remote::special;
        crate::log!("client: special action {action}");
        self.combat.on_special_action(action);
        let ped = self.ped();
        let Some(h) = self.ped_handle(ped) else { return };
        let in_vehicle = ProcessMemory.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        // samp's case 0 order: phone, piss, item (B 0x100030F0); 20–23 / 68 start an item / pissing.
        if matches!(action, special::NONE | special::PHONE_STOP) && self.phone {
            self.set_phone(h, false);
        }
        crate::specials::local_action(&mut self.runner, ped, &mut self.specials, action);
        match action {
            special::PHONE if !in_vehicle => self.set_phone(h, true),
            special::HANDS_UP if !in_vehicle && ped_in_world(ped) => {
                self.runner.run(op::TASK_HANDS_UP, &[Arg::Int(h), Arg::Int(-2)]);
            }
            _ => {}
        }
    }

    /// Phone flag + `TASK_USE_MOBILE_PHONE(ped, flag)` (ped method A 0x100ae0b0).
    fn set_phone(&mut self, handle: i32, on: bool) {
        self.phone = on;
        self.runner.run(op::TASK_USE_MOBILE_PHONE, &[Arg::Int(handle), Arg::Int(i32::from(on))]);
    }

    // ---- game operations ----------------------------------------------------

    /// `CPlayerPed::SetInitialState(false)`.
    fn reset_ped_state(&mut self, ped: u32) {
        // SAFETY: thiscall on the live local CPlayerPed, main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::PLAYER_PED_SET_INITIAL_STATE as usize);
            f(ped, 0);
        }
    }

    /// Entity `Teleport` virtual (player-state.md RPC 12).
    fn teleport(&mut self, x: f32, y: f32, z: f32) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let vtbl = ProcessMemory.read_u32(ped + off::VTABLE).unwrap_or(0);
        let Ok(target) = ProcessMemory.read_u32(vtbl + off::VT_TELEPORT) else { return };
        // SAFETY: the ped's own virtual, thiscall (CVector by value, bool).
        unsafe {
            let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
            f(ped, x, y, z, 0);
        }
    }

    /// RPC 19 behaviour: current and goal heading, then SET_CHAR_HEADING.
    fn set_facing(&mut self, deg: f32) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let r = game::heading_rad(deg);
        poke(ped + off::PED_HEADING_GOAL, r);
        poke(ped + off::PED_HEADING, r);
        if let Some(h) = self.ped_handle(ped) {
            self.runner.run(op::SET_CHAR_HEADING, &[Arg::Int(h), Arg::Float(deg)]);
        }
    }

    /// RPC 15 behaviour (ped method 0x100ac7d0 in the spec).
    fn set_controllable(&mut self, enable: bool) {
        let ped = self.ped();
        let Some(h) = self.ped_handle(ped) else { return };
        self.runner.run(op::SET_PLAYER_CONTROL, &[Arg::Int(0), Arg::Int(i32::from(enable))]);
        self.runner.run(op::FREEZE_CHAR_POSITION, &[Arg::Int(h), Arg::Int(i32::from(!enable))]);
        let in_vehicle = ProcessMemory.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        if enable
            && !in_vehicle
            && let Ok(Some(p)) = game::read_player(&ProcessMemory)
        {
            self.teleport(p.pos[0], p.pos[1], p.pos[2]);
        }
    }

    /// RPC 162 behaviour (Camera method 0x1009d7a0): the attachment ends too.
    fn camera_behind_player(&mut self) {
        self.runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(0)]);
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.behaviour.attached = None;
        self.runner.run(op::SET_CAMERA_BEHIND_PLAYER, &[]);
        self.runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
    }

    fn load_model(&mut self, model: i32) -> bool {
        load_model(&mut self.runner, model)
    }

    fn set_skin(&mut self, ped: u32, skin: u32) {
        set_ped_skin(&mut self.runner, ped, skin);
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

    /// `CPed::ClearWeapons` inside the local slot context (A 0x100AC050: PlayerInFocus only).
    fn clear_weapons(&mut self, ped: u32) {
        crate::combatclient::local_context(false, || {
            // SAFETY: thiscall on the live local ped, main thread.
            unsafe {
                let f: extern "thiscall" fn(u32) = std::mem::transmute(game::PED_CLEAR_WEAPONS as usize);
                f(ped);
            }
        });
    }

    /// RPC 22 behaviour: load the weapon model, `CPed::GiveWeapon`, arm it.
    fn give_weapon(&mut self, ped: u32, weapon: u8, ammo: u32) {
        let Some(model) = game::weapon_model(u32::from(weapon)) else { return };
        if !self.load_model(model) {
            return;
        }
        // GiveWeapon inside the slot context, then set-armed-weapon, not forced (A 0x100AF7D0 → 0x100AC0A0(weapon, 0)).
        crate::combatclient::local_context(true, || {
            // SAFETY: thiscall on the live local ped, main thread.
            unsafe {
                let f: extern "thiscall" fn(u32, u32, u32, u32) -> u32 =
                    std::mem::transmute(game::PED_GIVE_WEAPON as usize);
                f(ped, u32::from(weapon), ammo, 1);
            }
        });
        crate::combatclient::set_armed_local(&mut self.runner, ped, weapon);
    }
}

/// samp's local surf probe (A 0x10003730 → 0x100aca50): on foot, a vertical
/// `CWorld::ProcessLineOfSight` (0x56BA00) from 0.25 m to 2 m below the ped's
/// position, vehicles and objects only. What samp does with a hit (surfing a vehicle
/// of its pool or one of its objects, remote-players-sync.md "Local surfing") needs
/// the vehicle/object pools (M5/M6); here only the probe itself runs, as in the original.
fn surf_probe(ped: u32) -> u32 {
    const PROCESS_LINE_OF_SIGHT: usize = 0x56BA00;
    type Los = extern "C" fn(*const f32, *const f32, *mut u8, *mut u32, u8, u8, u8, u8, u8, u8, u8, u8) -> u8;
    let m = ProcessMemory;
    if m.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0 {
        return 0;
    }
    let matrix = m.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
    if matrix == 0 {
        return 0;
    }
    let p = [0, 4, 8].map(|o| m.read_f32(matrix + off::MATRIX_POS + o).unwrap_or(0.0));
    let from = [p[0], p[1], p[2] - 0.25];
    let to = [p[0], p[1], p[2] - 0.25 - 1.75];
    let mut colpoint = [0u8; 64];
    let mut entity = 0u32;
    // SAFETY: GTA's cdecl line-of-sight query with valid in/out buffers, main thread.
    unsafe {
        let f: Los = std::mem::transmute(PROCESS_LINE_OF_SIGHT);
        f(from.as_ptr(), to.as_ptr(), colpoint.as_mut_ptr(), &mut entity, 0, 1, 0, 1, 0, 0, 0, 0);
    }
    entity
}

/// samp's "ped is live" test (A 0x1009f430): not a deleted entity, in the moving list.
pub(crate) fn ped_in_world(ped: u32) -> bool {
    let m = ProcessMemory;
    let vt = m.read_u32(ped + off::VTABLE).unwrap_or(0);
    vt != 0 && vt != VTBL_CPLACEABLE && m.read_u32(ped + PHYSICAL_MOVING_LIST).unwrap_or(0) != 0
}

/// `CPlaceable` vtable: what a deleted entity's vtable reads as.
const VTBL_CPLACEABLE: u32 = 0x863C40;
/// `CPhysical::m_pMovingList`.
const PHYSICAL_MOVING_LIST: u32 = 0xB4;

/// HUD drawing on/off (samp's 0x100a1df0: menu "HUD on" flag and the script radar flag).
pub(crate) fn set_hud(on: bool) {
    poke(game::MENU_HUD_ON, u8::from(on));
    poke(game::HUD_SCRIPT_DONT_DISPLAY_RADAR, u8::from(!on));
}

/// Requests a model and waits (briefly) until it is loaded.
pub(crate) fn load_model(runner: &mut Runner, model: i32) -> bool {
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        return true;
    }
    runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
    runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
    for _ in 0..200 {
        if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    crate::log!("client: model {model} did not load in time");
    false
}

/// Skin change as the original does it, always (even for the current
/// model): load the model, `DeleteRwObject` + `SetModelIndex` through the
/// ped's vtable, then re-initialise its speech audio entity. CSV row 363
/// (`CClothes::RebuildPlayer := ret`) first, so CJ's clothes never override it.
pub(crate) fn set_ped_skin(runner: &mut Runner, ped: u32, skin: u32) {
    // Ped method A 0x100aff90: an invalid model becomes 0.
    let skin = if game::is_ped_model(&ProcessMemory, skin) { skin } else { 0 };
    let mut mem = ProcessMemory;
    let _ = mem.write(0x5A82C0, &[0xC3]);
    // A 0x100ae060: the IK secondary task (head look) is deleted first.
    let intel = ProcessMemory.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
    let ik = if intel != 0 { ProcessMemory.read_u32(intel + game::INTELLIGENCE_SECONDARY_IK).unwrap_or(0) } else { 0 };
    if ik != 0 {
        // SAFETY: GTA's scalar deleting destructor of that task (thiscall), main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::TASK_IK_DELETING_DTOR as usize);
            f(ik, 1);
        }
        poke(intel + game::INTELLIGENCE_SECONDARY_IK, 0u32);
    }
    if !load_model(runner, skin as i32) {
        // B 0x1009EF80: a debug chat line (FEAT-B, A17).
        crate::ui::print_debug(&format!("Warning: Model {skin} wouldn't load in time!"));
        return;
    }
    let vtbl = ProcessMemory.read_u32(ped + off::VTABLE).unwrap_or(0);
    let (Ok(delete_rw), Ok(set_model)) = (
        ProcessMemory.read_u32(vtbl + off::VT_DELETE_RW_OBJECT),
        ProcessMemory.read_u32(vtbl + off::VT_SET_MODEL_INDEX),
    ) else {
        return;
    };
    // SAFETY: the ped's own virtuals and the game's speech-entity init (thiscall), main thread.
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(delete_rw as usize);
        f(ped);
        poke(ped + off::ENTITY_MODEL, skin as u16);
        let g: extern "thiscall" fn(u32, u32) = std::mem::transmute(set_model as usize);
        g(ped, skin);
        let h: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::PED_SPEECH_INITIALISE as usize);
        h(ped + off::PED_SPEECH, ped);
    }
}

/// `GetTickCount` (samp's clock for the camera-target rate).
fn tick_count() -> u32 {
    // SAFETY: plain Win32 call.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

/// An entity's matrix as 16 floats (CMatrix rows right / forward / up / position, padded).
fn entity_matrix(ent: u32) -> Option<[f32; 16]> {
    let m = ProcessMemory;
    let mat = m.read_u32(ent + off::PLACEABLE_MATRIX).unwrap_or(0);
    (mat != 0).then(|| std::array::from_fn(|i| m.read_f32(mat + 4 * i as u32).unwrap_or(0.0)))
}

fn entity_pos(ent: u32) -> Option<[f32; 3]> {
    entity_matrix(ent).map(|m| [m[12], m[13], m[14]])
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// CSV row 361: 39 bytes NOPed by RPC 112.
const CRIME_NOP_ADDR: u32 = 0x4E7529;
const CRIME_NOP_LEN: usize = 39;
/// `CAEPoliceScannerAudioEntity::AddAudioEvent(eAudioEvents, eCrimeType, const CVector&)`.
const SCANNER_ADD_AUDIO_EVENT: u32 = 0x4E71E0;

/// `TheCamera` matrix position (samp's camera distance, B 0x1009F1C0).
const CAMERA_POS: u32 = 0xB6F9CC;

/// The camera-target search (B 0x10004420; camtarget.rs): object, vehicle, player, actor.
fn camera_targets(objects: &[(u16, u32, i32)], vehicles: &[(u32, u16)], peds: &[(u32, u16)], actors: &[(u32, u16)]) -> [u16; 4] {
    use crate::camtarget as ct;
    let m = ProcessMemory;
    let v3 = |a: u32| [m.read_f32(a).unwrap_or(0.0), m.read_f32(a + 4).unwrap_or(0.0), m.read_f32(a + 8).unwrap_or(0.0)];
    let (start, end) = ct::ray(v3(CAMERA_FRONT), v3(CAMERA_FRONT + 0xC));
    let cam = v3(CAMERA_POS);
    // CWorld::ProcessLineOfSight(start, end, …, buildings, vehicles, peds, objects, 0, 0, 0, 0): the hit point, if any.
    let los = |vehicles: u8, objects: u8| -> [f32; 3] {
        type Los = extern "C" fn(*const f32, *const f32, *mut f32, *mut u32, u8, u8, u8, u8, u8, u8, u8, u8) -> u8;
        let mut colpoint = [0f32; 16];
        let mut entity = 0u32;
        // SAFETY: GTA's cdecl line-of-sight query with valid in/out buffers, main thread.
        unsafe {
            let f: Los = std::mem::transmute(0x56BA00usize);
            f(start.as_ptr(), end.as_ptr(), colpoint.as_mut_ptr(), &mut entity, 1, vehicles, 0, objects, 0, 0, 0, 0);
        }
        if entity != 0 { [colpoint[0], colpoint[1], colpoint[2]] } else { end }
    };
    let mut out = [ct::NONE; 4];
    let e = los(0, 0);
    if let Some((id, _, _)) = crate::objects::pick_within(start, e, objects, ct::OBJECT_RADIUS) {
        out[0] = id;
    }
    // The local ped's own vehicle is skipped (B 0x100B7430).
    let ped = m.read_u32(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED).unwrap_or(0);
    let own = if ped != 0 && m.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & 0x100 != 0 {
        m.read_u32(ped + 0x58C).unwrap_or(0)
    } else {
        0
    };
    let search = |list: &[(u32, u16)], radius: f32, scale: f32, e: [f32; 3], skip: u32| -> u16 {
        ct::nearest(
            start,
            list.iter().filter_map(|&(ent, id)| {
                if !ped_in_world(ent) || ent == skip {
                    return None;
                }
                let p = entity_pos(ent)?;
                (dist3(p, cam) < radius && crate::objects::bound_sphere_hit(start, e, ent, scale)).then_some((id, p))
            }),
        )
    };
    let e = los(0, 1);
    out[1] = search(vehicles, ct::VEHICLE_RADIUS, ct::VEHICLE_SCALE, e, own);
    let e = los(1, 1);
    out[2] = search(peds, ct::PED_RADIUS, ct::PED_SCALE, e, 0);
    out[3] = search(actors, ct::PED_RADIUS, ct::PED_SCALE, e, 0);
    out
}
