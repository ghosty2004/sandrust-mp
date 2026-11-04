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
const PEDSTATE_DRIVING: u32 = 50;
const PEDSTATE_DIE: u32 = 54;
const PEDSTATE_DEAD: u32 = 55;

/// Win32 virtual-key codes polled like the original client (player-state.md 128/129).
/// Animation blocks and models the original requests every frame (TRACE-1).
const PRELOAD_ANIM_BLOCKS: [&str; 9] =
    ["PARACHUTE", "BAR", "GANGS", "PAULNMAC", "VENDING", "FIGHT_B", "FIGHT_C", "FIGHT_D", "FIGHT_E"];
const PRELOAD_MODELS: [i32; 9] = [3131, 1485, 1486, 1543, 1546, 343, 342, 344, 330];

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
    /// Hour from InitGame (SET_TIME_OF_DAY every frame).
    time_of_day: u8,
    /// Remote players (M3, players.rs).
    players: crate::players::Players,
    /// Vehicles (M5, vehicles.rs).
    vehicles: crate::vehicles::Vehicles,
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
            time_of_day: 12,
            players: crate::players::Players::default(),
            vehicles: crate::vehicles::Vehicles::default(),
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
        for ev in self.net.tick(now) {
            self.on_event(ev, now);
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
            // CSV row 356: skills to 1000, then SetStatValue := RET (vehicles.md RE-10 "Driving skills").
            self.vehicles.first_frame();
        }
        // Every frame, like the original: the server's time of day and the
        // pre-loading of the animation blocks and models samp uses.
        self.runner.run(op::SET_TIME_OF_DAY, &[Arg::Int(i32::from(self.time_of_day)), Arg::Int(0)]);
        self.preload();
        // Remote players before the local player's tick and send (onfoot §6 step 5b).
        self.players.tick(now);
        self.vehicles.remote_tick(&mut self.runner, &mut self.players);
        self.vehicles.hook_events(&mut self.runner, &mut self.net);
        if self.class_sel.is_active() {
            self.class_selection_tick(now);
        } else if self.spawned {
            self.sync_tick(now);
        } else if !matches!(self.net.phase(), Phase::Connected) {
            self.connecting_tick();
        }
        // Chat input and connection lines of the in-game UI (M4).
        crate::ui::net_tick(&mut self.net);
        // The vehicle pool tick follows the local player's (remote-control-vehicles §6 step 3c).
        if matches!(self.net.phase(), Phase::Connected) {
            self.vehicles.pool_tick(&mut self.runner, &mut self.net, &self.players, now);
        }
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
                self.time_of_day = g.world_time;
                crate::log!(
                    "client: InitGame: server {:?}, our id {}, {} class(es), on-foot rate {} ms",
                    g.server_name,
                    g.player_id,
                    g.spawns_available,
                    g.on_foot_rate
                );
                self.players.set_init_game(g.player_id, g.death_drop_money);
                crate::ui::on_init_game(g.server_name.as_bytes(), g.player_id);
                self.vehicles.set_init_game(&g);
                self.enter_class_selection(now, g.spawns_available);
            }
            Event::Rpc(r) => self.on_rpc(r),
            Event::Sync(s) => self.on_sync(s, now),
            Event::Closed(reason) => {
                crate::ui::on_closed();
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
                let (streamed_in, quit) = match &r {
                    R::PlayerStreamIn(s) => (Some(s.player_id), None),
                    R::PlayerQuit(q) => (None, Some(q.player_id)),
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
                P::SetPlayerTime(t) => self.time_of_day = t.hour,
                P::SetPlayerWorldTime(t) => self.time_of_day = t.hour,
                P::DisableRemoteVehicleCollisions(d) => self.vehicles.set_disable_collisions(d.disable),
                P::SetPlayerArmedWeapon(w) => {
                    let ped = self.ped();
                    if let Some(h) = self.ped_handle(ped) {
                        self.runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(w.weapon as i32)]);
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
        // While no frontend menu is open the HUD stays hidden (the original also
        // locks keyboard→pad input here; we rely on the player being frozen).
        if ProcessMemory.read_u8(game::MENU_ACTIVE).unwrap_or(0) == 0 {
            set_hud(false);
        }
        let mem = ProcessMemory;
        let keys = classsel::Keys {
            left: async_key_down(VK_LEFT) || game::key_down(&mem, game::key::LEFT),
            right: async_key_down(VK_RIGHT) || game::key_down(&mem, game::key::RIGHT),
            shift: async_key_down(VK_SHIFT)
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
        // 2. leave class selection.
        if self.class_sel.is_active() {
            self.class_sel.leave();
        }
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
        self.spawned_once = true;
        self.vehicles.set_spawned();
        self.sender.reset();
        self.last_stats = None;
        self.net.send_rpc(&player::PlayerSpawn, Reliability::ReliableOrdered);
        crate::log!("client: spawned at {:?} heading {} skin {}", s.pos, s.rotation, s.skin);
        true
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
        // In-car packets (200/210/211/106, passenger poll); on-foot sync only in ped states 1 and 12, so nothing
        // is sent while entering a seat from the server's warp or during the exit animation (TRACE-4 §4).
        let in_car = self.vehicles.local_tick(&mut self.runner, &mut self.net, self.players.streamed(), now);
        if !in_car && state != PEDSTATE_DRIVING {
            match game::read_local_sample(&mem, ped) {
                Ok(sample) => {
                    let keys = footsync::key_word(&sample.pad, sample.key_ctx);
                    let interval = footsync::on_foot_interval(self.on_foot_rate, self.players.streamed());
                    if self.sender.due(now, interval, keys, sample.pad.left_stick_x, sample.pad.left_stick_y) {
                        let f = footsync::build_foot_sync(&sample, self.sender.last_rotation());
                        if let Some(f) = self.sender.offer(now, f) {
                            self.net.send_sync(&f);
                        }
                    }
                }
                Err(e) => crate::log!("client: cannot read local ped: {e}"),
            }
        }
        if self.last_stats.is_none_or(|t| now.duration_since(t) > footsync::STATS_INTERVAL) {
            self.last_stats = Some(now);
            let money = mem.read_i32(game::PLAYER_MONEY).unwrap_or(0);
            self.net.send_sync(&StatsSync { money, drunk_level: 0 });
        }
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

    /// `CPed::ClearWeapons`.
    fn clear_weapons(&mut self, ped: u32) {
        // SAFETY: thiscall on the live local ped, main thread.
        unsafe {
            let f: extern "thiscall" fn(u32) = std::mem::transmute(game::PED_CLEAR_WEAPONS as usize);
            f(ped);
        }
    }

    /// RPC 22 behaviour: load the weapon model, `CPed::GiveWeapon`, arm it.
    fn give_weapon(&mut self, ped: u32, weapon: u8, ammo: u32) {
        let Some(model) = game::weapon_model(u32::from(weapon)) else { return };
        if !self.load_model(model) {
            return;
        }
        // SAFETY: thiscall on the live local ped, main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32, u32, u32) -> u32 =
                std::mem::transmute(game::PED_GIVE_WEAPON as usize);
            f(ped, u32::from(weapon), ammo, 1);
        }
        if let Some(h) = self.ped_handle(ped) {
            self.runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(i32::from(weapon))]);
        }
    }
}

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
    if skin > 20000 {
        return;
    }
    let mut mem = ProcessMemory;
    let _ = mem.write(0x5A82C0, &[0xC3]);
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
