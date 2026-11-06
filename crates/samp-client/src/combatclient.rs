//! The client side of combat (Windows only, milestone M7): InitGame selector and hook install, the hooks'
//! shared view, sending what the hooks queued (206 / 115 / 177), the remote 206 apply, 203 / 204 senders,
//! local drive-by and the carry / cuffs pad masks.
//!
//! Specs: see combat.rs. The local death / respawn sequence lives in client.rs (it drives the spawn routine).

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::{player, world};
use samp_proto::sync::player::{RemoteBulletSync, hit_type};
use samp_proto::sync::{SyncPacket, ids};

use crate::combat::{self, AimContext, AimSample, AimTimer, LocalHandcuffs, SlotState, WeaponsTracker};
use crate::combathooks::{self as hooks, Outgoing, RemoteHook, Shared};
use crate::game::off;
use crate::netclient::NetClient;
use crate::patch::ProcessMemory;
use crate::players::Players;
use crate::script::{Arg, Runner, op};
use crate::swap::addr as sw;
use crate::vehicles::Vehicles;

/// `CPlayerPed` +0x79C: the ped the player targets (remote-players-sync RE-10 "204 target fields").
const PLAYER_PED_TARGET: u32 = 0x79C;
/// `CEntity::m_pRwObject`.
const ENTITY_RW_OBJECT: u32 = 0x18;
/// `CPlayerPed::m_pPlayerData` and its `m_fLookPitch` (the 203 aimZ).
const PLAYER_PED_DATA: u32 = 0x480;
const PLAYER_DATA_LOOK_PITCH: u32 = 0x54;
/// `CWeapon::m_nState` / `m_nAmmoInClip`.
const WEAPON_STATE: u32 = 4;
const WEAPON_CLIP: u32 = 8;
/// `CPad::Pads[0].NewState.ShockButtonL` (in a car: the horn key).
const PAD_SHOCK_L: u32 = crate::game::PAD0 + combat::pad::SHOCK_L as u32;

/// Local pad masks read by the local branch of the ProcessControl hook (swap.rs): bit 0 jump, 1 crouch, 2 sprint.
pub static LOCAL_MASKS: AtomicU8 = AtomicU8::new(0);

fn rd<T: Copy>(a: u32) -> T {
    // SAFETY: callers pass addresses of live game objects / globals.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

#[derive(Default)]
pub struct Combat {
    pub selector: u8,
    weapon_rate: u32,
    pub local_team: u8,
    weapons: WeaponsTracker,
    aim_timer: AimTimer,
    /// Last 203 struct sent (duplicate gate).
    aim_last: Vec<u8>,
    /// Local passenger drive-by active (local player +0x1CB).
    pub drive_by: bool,
    pub cuffs: LocalHandcuffs,
    /// Our player id.
    local_id: Option<u16>,
}

impl Combat {
    pub fn new() -> Self {
        Self { selector: 2, weapon_rate: 30, local_team: 255, ..Default::default() }
    }

    /// InitGame (A 0x10010660): the selector, then the InitGame rows (lag-comp 1 and 2 only).
    pub fn on_init_game(&mut self, g: &samp_proto::session::InitGame) {
        self.selector = combat::lag_comp_selector(g.lag_compensation);
        self.weapon_rate = g.weapon_rate.clamp(10, 1000);
        self.local_id = Some(g.player_id);
        let selector = self.selector;
        let friendly_fire = g.vehicle_friendly_fire != 0;
        let report = crate::dll::with_patcher(|p, mem| {
            let mut r = hooks::install_init_game(p, mem, selector);
            if friendly_fire {
                r.extend(hooks::install_vehicle_friendly_fire(p, mem));
            }
            r
        });
        let failed: Vec<String> =
            report.iter().filter_map(|(row, r)| r.as_ref().err().map(|e| format!("row {row}: {e}"))).collect();
        crate::log!(
            "combat: InitGame lag-comp {} → selector {selector}; {} bullet-hook rows installed, failed: {failed:?}",
            g.lag_compensation,
            report.len() - failed.len()
        );
    }

    /// Publishes the remote players / vehicles / actors for the hooks (before GTA runs and before remote shots).
    /// `actors` = (ped, actor id) of the world-entity code (entities.rs), for RPC 177 and 204's target actor;
    /// `objects` = pool objects (GTA pointer, id) for the 206 hit type; `surface` = the GTA entity of the local
    /// surf state's surface (the damage decision's surf rows).
    pub fn update_shared(
        &self,
        players: &Players,
        vehicles: &Vehicles,
        actors: Vec<(u32, u16)>,
        objects: Vec<(u32, u16)>,
        surface: Option<u32>,
    ) {
        let remotes = players
            .iter()
            .filter(|p| p.ped != 0)
            .map(|p| RemoteHook {
                ped: p.ped,
                id: p.id,
                npc: p.npc,
                team: p.team,
                vehicle: vehicles.remote_vehicle(p.id),
            })
            .collect();
        hooks::set_shared(Shared {
            local_team: self.local_team,
            remotes,
            vehicles: vehicles.hook_vehicles(),
            actors,
            objects,
            surface,
        });
    }

    /// Sends what the hooks queued, in order (206 rel 7 ch 0; 115 / 177 rel 9 ch 0). Returns how many 177s
    /// were sent (counted by the world-entity code).
    pub fn drain(&mut self, net: &mut NetClient) -> u32 {
        let mut sent_177 = 0;
        for o in hooks::take_outgoing() {
            match o {
                Outgoing::Bullet(b) => net.send_sync(&b),
                Outgoing::Damage { taking, player, damage, weapon, body_part } => net.send_rpc(
                    &player::OnPlayerGiveTakeDamage {
                        taking,
                        player_id: player,
                        damage,
                        weapon_id: weapon,
                        bodypart: body_part,
                    },
                    Reliability::ReliableOrdered,
                ),
                Outgoing::ActorDamage { actor, damage, weapon, body_part } => {
                    net.send_rpc(
                        &world::OnPlayerDamageActor { unknown: false, actor_id: actor, damage, weapon, body_part },
                        Reliability::ReliableOrdered,
                    );
                    sent_177 += 1;
                }
            }
        }
        sent_177
    }

    /// 206 from the server (A 0x10009B40 → 0x10016370): checks, target entity, weapon, then the shot.
    pub fn on_bullet(&mut self, runner: &mut Runner, players: &mut Players, vehicles: &Vehicles, b: &RemoteBulletSync) {
        if b.player_id >= samp_proto::sync::policy::MAX_BULLET_MARKER_PLAYER_ID_EXCLUSIVE
            || Some(b.player_id) == self.local_id
        {
            return;
        }
        let s = &b.sync;
        if !combat::remote_bullet_valid(s) {
            return;
        }
        let entity = match s.hit_type {
            hit_type::PLAYER if Some(s.hit_id) == self.local_id => hooks::local_ped(),
            hit_type::PLAYER if s.hit_id == b.player_id => 0,
            hit_type::PLAYER => players.get(s.hit_id).map_or(0, |p| p.ped),
            hit_type::VEHICLE => vehicles.ptr_of(s.hit_id).unwrap_or(0),
            _ => 0,
        };
        let Some((ped, slot)) = players.arm_shooter(runner, b.player_id, s.weapon_id) else { return };
        hooks::N_REMOTE_SHOTS.fetch_add(1, Ordering::Relaxed);
        hooks::fire_shot(ped, slot, Some((entity, [s.offset.x, s.offset.y, s.offset.z])));
    }

    // ---- local senders -----------------------------------------------------------------------------------

    /// 204 (A 0x10006290), evaluated every tick while spawned.
    pub fn weapons_tick(&mut self, net: &mut NetClient, players: &Players, ped: u32, now: Instant) {
        let slots: [SlotState; combat::WEAPON_SLOTS] = std::array::from_fn(|i| {
            let w = ped + off::PED_WEAPONS + i as u32 * off::WEAPON_SIZE;
            SlotState { weapon: rd::<u32>(w) as u8, ammo: rd(w + off::WEAPON_AMMO_TOTAL) }
        });
        let armed = hooks::current_weapon(ped);
        let target: u32 = if rd::<u32>(ped + ENTITY_RW_OBJECT) != 0 { rd(ped + PLAYER_PED_TARGET) } else { 0 };
        let (tp, ta) = if target == 0 {
            (combat::NONE_ID, combat::NONE_ID)
        } else {
            let tp = players.iter().find(|p| p.ped == target).map_or(combat::NONE_ID, |p| p.id);
            let ta = hooks::SHARED
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|s| s.actors.iter().find(|a| a.0 == target).map(|a| a.1)))
                .unwrap_or(combat::NONE_ID);
            (tp, ta)
        };
        if let Some(p) = self.weapons.evaluate(now, &slots, armed, tp, ta) {
            net.send_sync(&p);
        }
    }

    fn aim_sample(ped: u32) -> AimSample {
        let data: u32 = rd(ped + PLAYER_PED_DATA);
        let w = hooks::current_weapon_ptr(ped);
        AimSample {
            cam_mode: rd(sw::CAM0_MODE),
            front: v3(sw::CAM0_VECTORS),
            source: v3(sw::CAM0_VECTORS + 12),
            aim_z: if data != 0 { rd(data + PLAYER_DATA_LOOK_PITCH) } else { 0.0 },
            fov: rd(sw::CAM0_FOV),
            aspect_ratio: rd(sw::ASPECT_RATIO),
            weapon_state: rd(w + WEAPON_STATE),
            clip: rd(w + WEAPON_CLIP),
        }
    }

    /// 203 on foot, after the 207 (A 0x10007C9A); the duplicate gate shares the 207 timestamp.
    pub fn aim_on_foot(
        &mut self,
        net: &mut NetClient,
        foot: &mut crate::footsync::FootSender,
        ped: u32,
        keys: u16,
        now: Instant,
    ) {
        let interval = combat::aim_interval(AimContext::OnFoot { keys }, self.selector, self.weapon_rate);
        if !self.aim_timer.due(now, interval) {
            return;
        }
        self.send_aim_gated(net, foot, ped, now);
    }

    /// 203 while spectating, right after a 212 (A 0x10006620): more than 2 × the on-foot interval since the last
    /// attempt (the aim timer shared with the on-foot 203); the duplicate gate shares the 207 timestamp.
    pub fn aim_spectating(
        &mut self,
        net: &mut NetClient,
        foot: &mut crate::footsync::FootSender,
        on_foot_interval: std::time::Duration,
        now: Instant,
    ) {
        let ped = hooks::local_ped();
        if ped == 0 {
            return;
        }
        let ms = u32::try_from(on_foot_interval.as_millis()).unwrap_or(u32::MAX);
        let interval =
            combat::aim_interval(AimContext::Spectating { on_foot_interval_ms: ms }, self.selector, self.weapon_rate);
        if !self.aim_timer.due(now, interval) {
            return;
        }
        self.send_aim_gated(net, foot, ped, now);
    }

    /// The 203 builder and its duplicate gate (A 0x10005210): identical to the last 203 within 501 ms of the shared
    /// foot / aim timestamp → not sent.
    fn send_aim_gated(&mut self, net: &mut NetClient, foot: &mut crate::footsync::FootSender, ped: u32, now: Instant) {
        let a = combat::build_aim(&Self::aim_sample(ped));
        let bytes = a.to_bits().as_bytes().to_vec();
        if foot.recent_send(now) && bytes == self.aim_last {
            return;
        }
        foot.touch(now);
        self.aim_last = bytes;
        net.send_sync(&a);
    }

    /// 203 in a car: passenger (100 ms with drive-by, else 1000 ms; A 0x100057B0) or driver (after each 200 of an
    /// armed vehicle, and from the 1000 ms tick; A 0x10007452 / 0x10007951). The in-car duplicate gate is shared
    /// with 200/211.
    pub fn aim_in_car(&mut self, net: &mut NetClient, vehicles: &mut Vehicles, ped: u32, driver: bool, now: Instant) {
        let after_200 = vehicles.take_driver_sent();
        let armed = after_200
            .and_then(|id| vehicles.model_of(id))
            .is_some_and(|m| samp_proto::sync::policy::ARMED_VEHICLE_MODELS.contains(&(m as u16)));
        let ctx = if driver { AimContext::Driver } else { AimContext::Passenger { drive_by: self.drive_by } };
        let due = if driver && armed {
            self.aim_timer.due(now, std::time::Duration::ZERO)
        } else {
            self.aim_timer.due(now, combat::aim_interval(ctx, self.selector, self.weapon_rate))
        };
        if !due {
            return;
        }
        let a = combat::build_aim(&Self::aim_sample(ped));
        let bytes = a.to_bits().as_bytes().to_vec();
        if vehicles.dup_offer(now, ids::AIM_SYNC, bytes) {
            net.send_sync(&a);
        }
    }

    /// Local passenger drive-by (A 0x10007B68): the horn key starts it; it stops when the weapon is fists or a
    /// gun (> 15, not the parachute) has no ammo left.
    pub fn passenger_drive_by(&mut self, runner: &mut Runner, ped: u32) {
        if !self.drive_by {
            if rd::<i16>(PAD_SHOCK_L) != 0
                && crate::vehicles::start_drive_by(runner, ped, |r, w| set_armed_local(r, ped, w))
            {
                self.drive_by = true;
            }
            return;
        }
        let w = hooks::current_weapon_ptr(ped);
        let kind: u32 = rd(w);
        let empty =
            kind > 15 && kind != u32::from(combat::weapon::PARACHUTE) && rd::<u32>(w + off::WEAPON_AMMO_TOTAL) == 0;
        if kind == 0 || empty {
            crate::vehicles::stop_drive_by(ped);
            self.drive_by = false;
        }
    }

    /// RPC 88 for the local player: carry / cuffs masks (RE-10 §1); 0 clears them.
    pub fn on_special_action(&mut self, action: u8) {
        match action {
            0 => self.cuffs = LocalHandcuffs::default(),
            combat::special::CARRY => self.cuffs.carry = true,
            combat::special::CUFFED => self.cuffs.cuffed = true,
            _ => {}
        }
        self.publish_masks();
    }

    pub fn publish_masks(&self) {
        let m = self.cuffs.masks();
        LOCAL_MASKS.store(u8::from(m.jump) | u8::from(m.crouch) << 1 | u8::from(m.sprint) << 2, Ordering::SeqCst);
    }

    /// While cuffed, every frame (A 0x100ACF60): an armed weapon goes back to fists.
    pub fn cuffed_tick(&mut self, runner: &mut Runner, ped: u32) {
        if self.cuffs.cuffed && hooks::current_weapon(ped) != 0 {
            set_armed_local(runner, ped, 0);
        }
    }

    pub fn reset_local(&mut self) {
        self.aim_timer.reset();
        self.drive_by = false;
    }
}

/// The local player's slot context around samp's weapon helpers (A 0x100AC0A0 / 0x100AF7D0 / 0x100AC050):
/// `PlayerInFocus` := slot 0, and with `skills` the 11 weapon-skill stats saved, written from the slot's table and
/// restored afterwards (EFX: two writes per stat and two `PlayerInFocus` writes per helper call). The local slot's
/// table holds the current stats (RPC 34 updates both), so the values never change.
pub fn local_context<R>(skills: bool, f: impl FnOnce() -> R) -> R {
    let wr32 = |a: u32, v: u32| {
        // SAFETY: GTA globals (.data/.bss), main thread.
        unsafe { std::ptr::write_volatile(a as usize as *mut u32, v) }
    };
    // SAFETY: byte global, main thread.
    unsafe { std::ptr::write_volatile(sw::PLAYER_IN_FOCUS as usize as *mut u8, 0) };
    let saved: [u32; crate::remote::SKILLS] = std::array::from_fn(|i| rd(sw::WEAPON_SKILLS + 4 * i as u32));
    if skills {
        for (i, v) in saved.iter().enumerate() {
            wr32(sw::WEAPON_SKILLS + 4 * i as u32, *v);
        }
    }
    let r = f();
    if skills {
        for (i, v) in saved.iter().enumerate() {
            wr32(sw::WEAPON_SKILLS + 4 * i as u32, *v);
        }
    }
    // SAFETY: as above.
    unsafe { std::ptr::write_volatile(sw::PLAYER_IN_FOCUS as usize as *mut u8, 0) };
    r
}

/// Set-armed-weapon (A 0x100AC0A0, not forced) on the local ped: on foot SET_CURRENT_CHAR_WEAPON, in a vehicle
/// `CPed::SetCurrentWeapon` directly, inside the local slot context.
pub fn set_armed_local(runner: &mut Runner, ped: u32, weapon: u8) {
    if ped == 0 {
        return;
    }
    local_context(true, || set_armed_raw(runner, ped, weapon));
}

fn set_armed_raw(runner: &mut Runner, ped: u32, weapon: u8) {
    if rd::<u32>(ped + off::PED_FLAGS) & (1 << 8) != 0 {
        // SAFETY: CPed::SetCurrentWeapon (thiscall) on the live local ped, main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(PED_SET_CURRENT_WEAPON as usize);
            f(ped, u32::from(weapon));
        }
    } else if let Some(h) = crate::game::ped_handle(&ProcessMemory, ped).ok().flatten() {
        runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(i32::from(weapon))]);
    }
}

/// `CPed::SetCurrentWeapon(eWeaponType)` (thiscall).
const PED_SET_CURRENT_WEAPON: u32 = 0x5E6280;

/// RPC 145 (A 0x1001AC10 → 0x100B00C0): the slot holding `weapon` gets `ammo` as total ammo.
pub fn set_ammo(ped: u32, weapon: u8, ammo: u16) {
    if ped == 0 {
        return;
    }
    for i in 0..13u32 {
        let w = ped + off::PED_WEAPONS + i * off::WEAPON_SIZE;
        if rd::<u32>(w) == u32::from(weapon) {
            // SAFETY: the ped's own CWeapon, main thread.
            unsafe { std::ptr::write_unaligned((w + off::WEAPON_AMMO_TOTAL) as usize as *mut u32, u32::from(ammo)) };
            return;
        }
    }
}

/// RPC 34 for the local player (A 0x1000F5E0): the GTA stat (and samp's local copy, not kept here).
pub fn set_local_skill(skill: u32, level: u16) {
    if skill < 11 {
        // SAFETY: CStats::StatTypesFloat entry (.data), main thread.
        unsafe {
            std::ptr::write_unaligned(
                (crate::game::WEAPON_SKILL_STATS + 4 * skill) as usize as *mut f32,
                f32::from(level),
            )
        };
    }
}

/// RPC 79 (A 0x1001BD10): script command 0x0948 with (x, y, z, type, radius); the radius float is passed through
/// as the integer parameter (descriptor "fffii").
pub fn create_explosion(runner: &mut Runner, e: &player::CreateExplosion) {
    runner.run(
        op::ADD_EXPLOSION,
        &[
            Arg::Float(e.pos.x),
            Arg::Float(e.pos.y),
            Arg::Float(e.pos.z),
            Arg::Int(e.kind as i32),
            // samp (A 0x1001BDB3) pushes the radius as a double, but the descriptor "fffii" reads one dword:
            // the low half of the double (0 for every radius with few mantissa bits, e.g. 2.0).
            Arg::Int(f64::from(e.radius).to_bits() as u32 as i32),
        ],
    );
}
