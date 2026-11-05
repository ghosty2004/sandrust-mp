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
    /// Local head look (A 0x10002fa0): offset refreshed every 1000 ms, LookAt every 200 ms.
    look_offset: [f32; 3],
    look_refresh: Option<Instant>,
    look_last: Option<Instant>,
    /// Remote players (M3, players.rs).
    players: crate::players::Players,
    /// Vehicles (M5, vehicles.rs).
    vehicles: crate::vehicles::Vehicles,
    /// The connect tick has fired (networking runs from then on).
    net_started: bool,
    /// Combat (M7, combatclient.rs).
    combat: crate::combatclient::Combat,
    /// Local player "dead" flag (local player +0xF4): set with RPC 53, cleared by the respawn.
    dead: bool,
    /// InitGame class count (class selection after a forced reselection).
    class_count: u32,
    /// Objects (M6, objects.rs).
    objects: crate::objects::Objects,
    /// Pickups, actors, checkpoints, gang zones, map icons, 3D labels (M6, entities.rs).
    entities: crate::entities::Entities,
}

/// Raw write into game data (heap objects, .data/.bss); no protection change.
fn poke<T: Copy>(addr: u32, v: T) {
    if addr == 0 {
        return;
    }
    // SAFETY: callers pass addresses of live game objects / game globals.
    unsafe { std::ptr::write_unaligned(addr as usize as *mut T, v) };
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
        // "Started"; the UI's connect tick starts the session 3.0 s later (A 0x10008940:
        // 3000 ms after the netgame constructor; TRACE-3 U-CONNECT-TIMING; parity run: join
        // in frame 251 in both clients).
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
            look_offset: [0.0; 3],
            look_refresh: None,
            look_last: None,
            players: crate::players::Players::default(),
            vehicles: crate::vehicles::Vehicles::default(),
            net_started: false,
            combat: crate::combatclient::Combat::new(),
            dead: false,
            class_count: 0,
            objects: crate::objects::Objects::default(),
            entities: crate::entities::Entities::default(),
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
            format!("world count={} {} {}", lines.len(), self.objects.header_fields(), self.entities.header_fields());
        for l in lines {
            out.push('\n');
            out.push_str(&l);
        }
        out
    }

    /// CGame::Process pre-step (dll.rs): the per-frame object process (world-entities.md RE-10 §3).
    pub fn pre_process(&mut self) {
        if matches!(self.net.phase(), Phase::Connected) {
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

    fn ped(&self) -> u32 {
        ProcessMemory.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0)
    }

    fn ped_handle(&self, ped: u32) -> Option<i32> {
        game::ped_handle(&ProcessMemory, ped).ok().flatten()
    }

    /// One frame (main thread).
    pub fn tick(&mut self) {
        let now = Instant::now();
        // The connect tick starts connecting 3.0 s after "Started" (TRACE-3
        // U-CONNECT-TIMING); the UI prints the connect / retry lines.
        let answered = !matches!(self.net.phase(), Phase::Opening);
        if crate::ui::connect_tick(answered) {
            self.net_started = true;
        }
        // What the combat hooks queued during this frame's CGame::Process (206 / 115), first.
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        if self.net_started {
            self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
            for ev in self.net.tick(now) {
                self.on_event(ev, now);
            }
            // Remote 206 shots fired while the packets were handled (115 taking 1).
            self.entities.sent_177 += self.combat.drain(&mut self.net);
        }
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
        if self.class_sel.is_active() {
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
            } else if !matches!(self.net.phase(), Phase::Connected) {
                self.connecting_tick();
            }
        }
        // Remote peds whose ProcessControl faulted (swap.rs fault containment).
        self.players.handle_faults(&mut self.runner);
        // Chat input and connection lines of the in-game UI (M4).
        crate::ui::net_tick(&mut self.net);
        // The vehicle pool tick follows the local player's (remote-control-vehicles §6 step 3c).
        if matches!(self.net.phase(), Phase::Connected) {
            self.vehicles.pool_tick(&mut self.runner, &mut self.net, &self.players, now);
            // World entities (M6): hook events (RPC 131 / 177), checkpoints, pickup poll, labels.
            self.entities.hook_events(&mut self.net);
            self.objects.poll_escape(&mut self.net);
            self.entities.tick(&mut self.runner, &mut self.net, &self.players, &self.vehicles);
        }
        // The hooks' view for the next CGame::Process.
        self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
        self.entities.sent_177 += self.combat.drain(&mut self.net);
        // Push out what this frame queued (sync packets, RPCs) right away.
        if self.net_started {
            self.net.flush(now);
        }
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
                crate::log!(
                    "client: InitGame: server {:?}, our id {}, {} class(es), on-foot rate {} ms",
                    g.server_name,
                    g.player_id,
                    g.spawns_available,
                    g.on_foot_rate
                );
                self.players.set_init_game(g.player_id, g.death_drop_money);
                self.combat.on_init_game(&g);
                self.class_count = g.spawns_available;
                crate::ui::on_init_game(&g);
                self.vehicles.set_init_game(&g);
                self.objects.set_local_id(g.player_id);
                self.entities.set_local_id(g.player_id);
                self.enter_class_selection(now, g.spawns_available);
            }
            Event::Rpc(r) => self.on_rpc(r),
            Event::Sync(s) => self.on_sync(s, now),
            Event::Closed(reason) => {
                crate::ui::on_closed();
                self.objects.clear(&mut self.runner);
                self.entities.clear(&mut self.runner);
                self.vehicles.clear(&mut self.runner);
                self.players.clear(&mut self.runner);
                crate::log!("client: session closed: {reason}");
                self.class_sel.leave();
                self.spawned = false;
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
            // Bullet sync fires its shot at once (A 0x10009B40 → 0x10016370; combat §8 P10).
            S::Bullet(b) => {
                self.combat.update_shared(&self.players, &self.vehicles, self.entities.hook_actors());
                self.combat.on_bullet(&mut self.runner, &mut self.players, &self.vehicles, &b);
            }
            s => self.players.on_sync(s, now),
        }
    }

    fn on_rpc(&mut self, r: Incoming) {
        use player::Incoming as P;
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        crate::ui::on_rpc(&r);
        match r {
            Incoming::Ui(ui::Incoming::SendClientMessage(m)) => {
                crate::log!("[chat {:08x}] {}", m.colour, text(&m.message));
            }
            Incoming::Ui(ui::Incoming::SendGameText(g)) => crate::log!("[gametext] {}", text(&g.text)),
            Incoming::Remote(r) => {
                use samp_proto::rpcs::remote::Incoming as R;
                if let R::PlayerJoin(j) = &r {
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
                P::ForcePlayerClassSelection(_) => {
                    crate::log!("client: server forces class selection on next death");
                    self.force_class_selection = true;
                }
                P::SetPlayerPosition(s) => self.teleport(s.pos.x, s.pos.y, s.pos.z),
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
                    self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
                    let v = [c.pos.x, c.pos.y, c.pos.z, 0.0, 0.0, 0.0].map(Arg::Float);
                    self.runner.run(op::SET_FIXED_CAMERA_POSITION, &v);
                }
                P::SetPlayerCameraLookAt(c) => {
                    let cut = if c.cut == 0 || c.cut > 2 { 2 } else { c.cut };
                    self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
                    self.runner.run(
                        op::POINT_CAMERA_AT_POINT,
                        &[Arg::Float(c.pos.x), Arg::Float(c.pos.y), Arg::Float(c.pos.z), Arg::Int(i32::from(cut))],
                    );
                }
                P::SetPlayerCameraBehindPlayer(_) => self.camera_behind_player(),
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
                P::SetPlayerColor(c) => crate::worldhooks::with_shared(|s| {
                    s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(c.player_id, c.colour)
                }),
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
        let connected = matches!(self.net.phase(), Phase::Connected);
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
        // On foot: the surf probe (A 0x10003730), before the head look.
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER) {
            surf_probe(ped);
        }
        // Head look in the on-foot, driver and passenger branches of the local tick.
        if matches!(state, PEDSTATE_IDLE | PEDSTATE_SNIPER | PEDSTATE_DRIVING) {
            self.head_look(ped, now);
        }
        // 205 (> 1000 ms) and 204 (every tick) come before the per-state packets (A 0x100078A8 / 0x100078C9).
        if self.last_stats.is_none_or(|t| now.duration_since(t) > footsync::STATS_INTERVAL) {
            self.last_stats = Some(now);
            let money = mem.read_i32(game::PLAYER_MONEY).unwrap_or(0);
            self.net.send_sync(&StatsSync { money, drunk_level: 0 });
        }
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
        // In-car packets (200/210/211/106, passenger poll); on-foot sync only in ped states 1 and 12, so nothing
        // is sent while entering a seat from the server's warp or during the exit animation (TRACE-4 §4).
        self.vehicles.local_drive_by = self.combat.drive_by;
        self.vehicles.local_cuffed = self.combat.cuffs.cuffed;
        let in_car = self.vehicles.local_tick(&mut self.runner, &mut self.net, self.players.streamed(), now);
        if in_car && in_vehicle_flag && veh != 0 {
            self.combat.aim_in_car(&mut self.net, &mut self.vehicles, ped, is_driver, now);
        }
        if !in_car && state != PEDSTATE_DRIVING {
            // On foot (A 0x10003f50): GET_AREA_VISIBLE for the world-bounds check (bounds: M8b).
            self.runner.run(op::GET_AREA_VISIBLE, &[Arg::Local(0)]);
            match game::read_local_sample(&mem, ped) {
                Ok(sample) => {
                    let keys = footsync::key_word(&sample.pad, sample.key_ctx);
                    let interval = footsync::on_foot_interval(self.on_foot_rate, self.players.streamed());
                    if self.sender.due(now, interval, keys, sample.pad.left_stick_x, sample.pad.left_stick_y) {
                        // The 207 builder (A 0x10004f00) reads the weapon with GET_CURRENT_CHAR_WEAPON.
                        self.current_weapon(ped);
                        let mut sample = sample;
                        // samp's order (A 0x10003410): jetpack, dance, hands up, phone, piss, duck.
                        if self.phone && matches!(sample.special_action, 0 | 1) {
                            sample.special_action = crate::remote::special::PHONE;
                        }
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

    // ---- local per-frame commands (local tick A 0x100074c0) -----------------

    /// The script commands samp's local tick issues every frame while spawned,
    /// before the sync sends (TRACE-1 "Walking": 052C, 09BA, 0470, 077E).
    fn local_frame(&mut self, now: Instant) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        // Hands up + Triangle (enter/exit key) → control on again, which re-sets the ped
        // to its own position and so ends the task (A 0x100074c0 → 0x100ac7d0(1)).
        let triangle = ProcessMemory.read_u16(game::PAD0 + PAD_TRIANGLE).unwrap_or(0);
        if triangle != 0 && self.hands_up(ped) {
            self.set_controllable(true);
        }
        // Drunk handling (A 0x100ade30): no drunk level is ever set here (RPC 35 is M8b),
        // so it always resets the drunkenness to 0.
        self.runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(0)]);
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
    }

    /// The local head follows the camera (A 0x10002fa0; "disableheadmove" in sa-mp.cfg is not
    /// read yet, so it is always on, samp's default): every 1000 ms the offset := 20 × camera
    /// front, every 200 ms `LookAt` at position + offset for 2000 ms.
    fn head_look(&mut self, ped: u32, now: Instant) {
        let m = ProcessMemory;
        let matrix = m.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
        if matrix == 0 {
            return;
        }
        let v3 = |a: u32| [m.read_f32(a).unwrap_or(0.0), m.read_f32(a + 4).unwrap_or(0.0), m.read_f32(a + 8).unwrap_or(0.0)];
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

    /// RPC 88 (LocalPlayer method A 0x100030f0), for the actions this client has.
    /// 0 stops the states samp stops (jetpack, phone, items, dance, cuffs, carry,
    /// piss) — none of which exist here — and, as in samp, not hands up.
    fn on_special_action(&mut self, action: u8) {
        use crate::remote::special;
        crate::log!("client: special action {action}");
        self.combat.on_special_action(action);
        let ped = self.ped();
        let Some(h) = self.ped_handle(ped) else { return };
        let in_vehicle = ProcessMemory.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        match action {
            special::NONE | special::PHONE_STOP if self.phone => self.set_phone(h, false),
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

    /// RPC 162 behaviour (Camera method 0x1009d7a0).
    fn camera_behind_player(&mut self) {
        self.runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(0)]);
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
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
fn set_hud(on: bool) {
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
