//! Remote players inside the game (Windows only): pool, GTA ped lifecycle and
//! on-foot sync application. Runs on the game's main thread from the client
//! tick (samp: netgame tick → player pool → A 0x100166B0 per remote).
//!
//! Specs: remote-players-sync.md §1 (137/138/32/163) and §2.1;
//! remote-control-onfoot.md §1.2 (creation), §1.5 (destruction), §3 (movement);
//! TRACE-2 (original client: CREATE_PLAYER(slot) → GET_PLAYER_CHAR → 087E,
//! 02AB, 0446, 03FE ×2 → direct SetModelIndex → heading/matrix/pad by memory
//! writes, all in one frame).
//!
//! Parity pass (docs/phase3/parity-notes.md): special actions duck / hands up
//! (onfoot §4.2), the one-shot weapon check with GET_CURRENT_CHAR_WEAPON and
//! the sniper ped state (§4.3), RPC 153 on remote peds, and the streaming-out
//! of a ped whose ProcessControl faulted (swap.rs).
//!
//! Player markers (docs/phase3/fixb-notes.md): the ped blip at spawn and the 208
//! coordinate blips; their colour index is the player id (radar colour hook, row 51).
//!
//! Deviations (docs/phase3/m3-notes.md): no fighting style, no animation sync, the other special actions,
//! no surfing positions, no vehicles;
//! `CWorld::Players[slot].m_pPed` is cleared after deletion (samp leaves it
//! dangling; main-session decision after TRACE-2).

use std::collections::BTreeMap;
use std::time::Instant;

use samp_proto::rpcs::remote::Incoming as R;
use samp_proto::sync::IncomingSync;

use crate::footsync::{Rotation, quat_from_matrix};
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::remote::{
    self, Correction, PadInput, RemotePlayer, SKILLS, SlotAllocator, SpecialStep, correct_position,
    heading_from_forward, normalise, rows_from_quat, samp_deg_to_rad, slerp, state,
};
use crate::script::{Arg, Runner, op};
use crate::swap;

/// `sizeof(CPlayerInfo)`.
const PLAYER_INFO_SIZE: u32 = 0x190;
/// `CPlaceable` vtable: what a deleted entity's vtable reads as (onfoot §1.5).
const VTBL_CPLACEABLE: u32 = 0x863C40;
/// `CPhysical::m_pMovingList` (onfoot §3.1: "live entity in the moving list").
const PHYSICAL_MOVING_LIST: u32 = 0xB4;
/// `CPlayerPed::m_pPlayerData` (CPlayerPedData*), offset read from `ProcessAnimGroups` (0x609BFB).
const PLAYER_PED_DATA: u32 = 0x480;
/// Anim-block field of `CPlayerPedData` cleared before deletion (onfoot §1.5).
const PLAYER_DATA_ANIM_BLOCK: u32 = 0x4C;
/// `CEntity::m_nFlags`.
const ENTITY_FLAGS: u32 = 0x1C;
/// `IKChainManager_c::LookAt` (thiscall) and `g_ikChainMan` (gta-reversed `IKChainManager.cpp`).
const IK_LOOK_AT: u32 = 0x618970;
const IK_CHAIN_MANAGER: u32 = 0xC15448;

/// `g_ikChainMan.LookAt("FollowPedSA", ped, nullptr, time_ms, bone −1, &pos, false, 0.1, 500, 3, false)`
/// (onfoot §5.2: remotes 1000 ms; the local player 2000 ms, A 0x10002fa0).
pub(crate) fn look_at(ped: u32, time_ms: u32, pos: &mut [f32; 3]) {
    type LookAt = extern "thiscall" fn(u32, *const u8, u32, u32, u32, i32, *mut f32, u8, f32, i32, i32, u8);
    // SAFETY: GTA's IK chain manager on a live ped, main thread; the strings and vector outlive the call.
    unsafe {
        let f: LookAt = std::mem::transmute(IK_LOOK_AT as usize);
        f(IK_CHAIN_MANAGER, c"FollowPedSA".as_ptr().cast(), ped, 0, time_ms, -1, pos.as_mut_ptr(), 0, 0.1, 500, 3, 0);
    }
}

/// PlayerJoin ids are accepted below this (sync §137).
const MAX_PLAYER_ID: u16 = 1004;

fn poke<T: Copy>(addr: u32, v: T) {
    if addr == 0 {
        return;
    }
    // SAFETY: callers pass fields of live game objects.
    unsafe { std::ptr::write_unaligned(addr as usize as *mut T, v) };
}

fn v3(mem: &ProcessMemory, a: u32) -> Option<[f32; 3]> {
    Some([mem.read_f32(a).ok()?, mem.read_f32(a + 4).ok()?, mem.read_f32(a + 8).ok()?])
}

fn put_v3(a: u32, v: [f32; 3]) {
    poke(a, v[0]);
    poke(a + 4, v[1]);
    poke(a + 8, v[2]);
}

#[derive(Default)]
pub struct Players {
    players: BTreeMap<u16, RemotePlayer>,
    slots: SlotAllocator,
    death_drop_money: i32,
    local_id: Option<u16>,
    /// InitGame player-marker mode (netgame settings +0x30): 0 off, otherwise a blip on every spawned ped.
    marker_mode: u32,
}

impl Players {
    pub fn set_init_game(&mut self, local_id: u16, death_drop_money: u32, marker_mode: u32) {
        self.local_id = Some(local_id);
        self.death_drop_money = death_drop_money.min(i32::MAX as u32) as i32;
        self.marker_mode = marker_mode;
    }

    // ---- player markers (radar blips) -----------------------------------------------------------------------

    /// 208 PlayerMarkersSync (B 0x10009F00): ids below 1004 that are in the pool. A record without a position
    /// removes the marker blip (B 0x100144B0(0)); a position replaces it, and a new coordinate blip is created
    /// only while the player is not streamed in (B 0x100144F0).
    pub fn on_markers(&mut self, runner: &mut Runner, m: &samp_proto::sync::player::MarkersSync) {
        for rec in &m.markers {
            if rec.player_id >= MAX_PLAYER_ID {
                continue;
            }
            let Some(p) = self.players.get_mut(&rec.player_id) else { continue };
            match rec.position {
                None => {
                    remove_marker_blip(runner, p);
                    p.marker_active = false;
                }
                Some([x, y, z]) => {
                    p.marker_active = true;
                    remove_marker_blip(runner, p);
                    if p.ped == 0 {
                        let pos = [f32::from(x), f32::from(y), f32::from(z)];
                        p.marker_blip = add_coord_blip(runner, pos, u32::from(p.id));
                    }
                }
            }
        }
    }

    /// Spawn part of the stream-in (B 0x10017164…0x100171A1): the 208 marker blip goes, and with markers on the
    /// ped gets a blip of colour index = player id, display 2 (B 0x100AF740).
    fn spawn_blips(&mut self, runner: &mut Runner, id: u16, handle: i32) {
        let mode = self.marker_mode;
        let Some(p) = self.players.get_mut(&id) else { return };
        remove_marker_blip(runner, p);
        if mode != 0 {
            if p.ped_blip != 0 {
                runner.run(op::REMOVE_BLIP, &[Arg::Int(p.ped_blip)]);
                p.ped_blip = 0;
            }
            runner.run(op::ADD_BLIP_FOR_CHAR, &[Arg::Int(handle), Arg::Local(0)]);
            p.ped_blip = runner.local_i32(0);
            runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(p.ped_blip), Arg::Int(i32::from(id))]);
            runner.run(op::CHANGE_BLIP_DISPLAY, &[Arg::Int(p.ped_blip), Arg::Int(2)]);
        }
    }

    /// Remote reset (B 0x10015F50: stream-out of a spawned ped, game-mode restart) and destructor
    /// (B 0x10016610): the marker blip goes and the marker flag is cleared.
    pub fn reset_marker(&mut self, runner: &mut Runner, id: u16) {
        if let Some(p) = self.players.get_mut(&id) {
            remove_marker_blip(runner, p);
            p.marker_active = false;
        }
    }

    pub fn get(&self, id: u16) -> Option<&RemotePlayer> {
        self.players.get(&id)
    }

    pub fn get_mut(&mut self, id: u16) -> Option<&mut RemotePlayer> {
        self.players.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RemotePlayer> {
        self.players.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut RemotePlayer> {
        self.players.values_mut()
    }

    /// What the UI needs of each remote player (nametags, bubbles, scoreboard).
    #[cfg(windows)]
    pub fn ui_snapshot(&self, now: Instant) -> Vec<crate::ui::core::RemoteUi> {
        self.players
            .values()
            .map(|p| crate::ui::core::RemoteUi {
                id: p.id,
                ped: p.ped,
                state: p.state,
                health: p.health,
                armour: p.armour,
                npc: p.npc,
                ms_since_sync: p.last_sync.map_or(u64::MAX, |t| now.duration_since(t).as_millis() as u64),
            })
            .collect()
    }

    /// Streamed-in remote peds (N of the on-foot send interval).
    pub fn streamed(&self) -> u32 {
        self.slots.count()
    }

    pub fn on_rpc(&mut self, runner: &mut Runner, r: R) {
        match r {
            R::PlayerJoin(j) => {
                let name = String::from_utf8_lossy(&j.name).into_owned();
                if j.name.len() > 24 || j.player_id > MAX_PLAYER_ID {
                    return;
                }
                crate::log!("players: join id={} name={name:?} npc={} colour={:08x}", j.player_id, j.is_npc, j.colour);
                if let Some(old) = self.players.get(&j.player_id)
                    && old.ped != 0
                {
                    self.destroy_ped(runner, j.player_id);
                }
                self.players.insert(j.player_id, RemotePlayer::new(j.player_id, name, j.colour, j.is_npc != 0));
            }
            R::PlayerQuit(q) => {
                crate::log!("players: quit id={} reason={}", q.player_id, q.reason);
                self.reset_marker(runner, q.player_id);
                self.destroy_ped(runner, q.player_id);
                self.players.remove(&q.player_id);
            }
            R::PlayerStreamIn(s) => {
                let Some(p) = self.players.get_mut(&s.player_id) else {
                    crate::log!("players: stream-in for unknown id {}", s.player_id);
                    return;
                };
                p.skin = s.skin;
                if s.team != 255 {
                    p.team = s.team;
                }
                if s.colour != 0 {
                    p.colour = s.colour;
                }
                let skills = s.skill_levels.map(f32::from);
                let pos = [s.position.x, s.position.y, s.position.z];
                crate::log!(
                    "players: stream-in id={} skin={} pos={:.2},{:.2},{:.2} angle={:.1}",
                    s.player_id,
                    s.skin,
                    pos[0],
                    pos[1],
                    pos[2],
                    s.facing_angle
                );
                self.create_ped(runner, s.player_id, pos, s.facing_angle, skills);
                // RPC 32 tail (B 0x10010CC9…0x10010CEC): an NPC's ped becomes fire- and explosion-proof
                // (B 0x100ABD10 → SET_CHAR_PROOFS(0, 1, 1, 0, 0)).
                if let Some(p) = self.players.get(&s.player_id)
                    && p.npc
                    && p.ped != 0
                    && let Some(h) = game::ped_handle(&ProcessMemory, p.ped).ok().flatten()
                {
                    let a = [Arg::Int(h), Arg::Int(0), Arg::Int(1), Arg::Int(1), Arg::Int(0), Arg::Int(0)];
                    runner.run(op::SET_CHAR_PROOFS, &a);
                }
                // Spawn (B 0x1001714E): a non-zero colour goes into the colour table once the ped exists.
                if s.colour != 0 && self.players.get(&s.player_id).is_some_and(|p| p.ped != 0) {
                    crate::worldhooks::with_shared(|t| {
                        t.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(s.player_id, s.colour)
                    });
                }
            }
            R::PlayerStreamOut(s) => {
                crate::log!("players: stream-out id={}", s.player_id);
                if self.players.get(&s.player_id).is_some_and(|p| p.ped != 0) {
                    self.reset_marker(runner, s.player_id);
                }
                self.destroy_ped(runner, s.player_id);
                if let Some(p) = self.players.get_mut(&s.player_id) {
                    p.state = state::NONE;
                }
            }
            R::SetPlayerName(n) => {
                if let Some(p) = self.players.get_mut(&n.player_id) {
                    p.name = String::from_utf8_lossy(&n.name).into_owned();
                }
            }
            _ => {}
        }
    }

    pub fn on_sync(&mut self, s: IncomingSync, now: Instant) {
        match s {
            IncomingSync::Foot { timestamp, sync } => {
                if Some(sync.player_id) == self.local_id {
                    return;
                }
                if let Some(p) = self.players.get_mut(&sync.player_id) {
                    p.store_foot(timestamp, &sync, now);
                }
            }
            IncomingSync::Aim(a) => {
                if let Some(slot) = self.players.get(&a.player_id).and_then(|p| p.slot) {
                    let s = a.sync;
                    swap::with_table(|t| {
                        if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                            sh.apply_aim(
                                s.cam_mode,
                                [s.cam_front.x, s.cam_front.y, s.cam_front.z],
                                [s.cam_pos.x, s.cam_pos.y, s.cam_pos.z],
                                s.zoom,
                                s.aspect,
                            );
                        }
                    });
                }
            }
            _ => {}
        }
    }

    /// Session ended: every remote ped goes.
    pub fn clear(&mut self, runner: &mut Runner) {
        let ids: Vec<u16> = self.players.keys().copied().collect();
        for id in ids {
            self.destroy_ped(runner, id);
        }
        self.players.clear();
    }

    // ---- ped lifecycle ------------------------------------------------------

    fn create_ped(&mut self, runner: &mut Runner, id: u16, pos: [f32; 3], angle: f32, skills: [f32; SKILLS]) {
        if !crate::dll::REMOTE_PEDS_READY.load(std::sync::atomic::Ordering::SeqCst) {
            crate::log!("players: sector relocation or swap hooks missing; not creating a ped for id {id}");
            return;
        }
        self.destroy_ped(runner, id);
        let Some(slot) = self.slots.alloc() else {
            crate::log!("players: no free ped slot for id {id}");
            return;
        };
        let info = game::CWORLD_PLAYERS + u32::from(slot) * PLAYER_INFO_SIZE;
        // Players[slot] lies in the former (zeroed) sector grid; like samp, it is not
        // cleared again on reuse — GTA's SetupPlayerPed / CPlayerPed ctor fill it.
        let s = i32::from(slot);
        runner.run(
            op::CREATE_PLAYER,
            &[Arg::Int(s), Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(pos[2]), Arg::Local(0)],
        );
        runner.run(op::GET_PLAYER_CHAR, &[Arg::Int(s), Arg::Local(1)]);
        let handle = runner.local_i32(1);
        let mem = ProcessMemory;
        let ped = mem.read_u32(info + off::PLAYER_INFO_PED).unwrap_or(0);
        if ped == 0 || game::ped_handle(&mem, ped).ok().flatten() != Some(handle) {
            crate::log!("players: CREATE_PLAYER({slot}) gave no ped (ped {ped:#x}, handle {handle:#x})");
            self.slots.free(slot);
            return;
        }
        let h = Arg::Int(handle);
        runner.run(op::SET_CHAR_DROPS_WEAPONS_WHEN_DEAD, &[h, Arg::Int(1)]);
        runner.run(op::SET_CHAR_PROOFS, &[h, Arg::Int(0), Arg::Int(0), Arg::Int(1), Arg::Int(0), Arg::Int(0)]);
        runner.run(op::SET_CHAR_SUFFERS_CRITICAL_HITS, &[h, Arg::Int(0)]);
        runner.run(op::SET_CHAR_MONEY, &[h, Arg::Int(0)]);
        runner.run(op::SET_CHAR_MONEY, &[h, Arg::Int(self.death_drop_money)]);
        let Some(p) = self.players.get_mut(&id) else { return };
        crate::client::set_ped_skin(runner, ped, p.skin);
        let r = samp_deg_to_rad(angle);
        poke(ped + off::PED_HEADING, r);
        poke(ped + off::PED_HEADING_GOAL, r);
        let matrix = mem.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
        if matrix != 0 {
            put_v3(matrix + off::MATRIX_POS, [pos[0], pos[1], pos[2] + remote::CREATE_Z_OFFSET]);
        }
        swap::with_table(|t| t.bind(slot, ped, skills));
        p.slot = Some(slot);
        p.ped = ped;
        p.state = state::SPAWNED;
        p.health = 100.0;
        p.foot_new = p.foot.is_some();
        crate::log!("players: id={id} {} → slot {slot}, ped {ped:#x}, handle {handle:#x}", p.name);
        self.spawn_blips(runner, id, handle);
    }

    fn destroy_ped(&mut self, runner: &mut Runner, id: u16) {
        let Some(p) = self.players.get_mut(&id) else { return };
        let (ped, slot) = (p.ped, p.slot);
        p.ped = 0;
        p.slot = None;
        // GTA's `CPed` destructor clears the ped's blip (gta-reversed `CPed::~CPed`).
        p.ped_blip = 0;
        let Some(slot) = slot else { return };
        swap::with_table(|t| t.unbind(slot));
        let mem = ProcessMemory;
        let vtbl = mem.read_u32(ped + off::VTABLE).unwrap_or(0);
        if ped != 0 && vtbl != 0 && vtbl != VTBL_CPLACEABLE {
            // A ped still in a vehicle is warped out to (100, 100, 10) first (onfoot §1.5), so the vehicle keeps
            // no pointer to the deleted ped.
            if mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0
                && let Some(h) = game::ped_handle(&mem, ped).ok().flatten()
            {
                let a = [Arg::Int(h), Arg::Float(100.0), Arg::Float(100.0), Arg::Float(10.0)];
                runner.run(op::WARP_CHAR_FROM_CAR_TO_COORD, &a);
            }
            // m_pPlayerData->+0x4C := 0 (the anim-block field ProcessAnimGroups no longer
            // maintains, CSV row 167), through the ped's own pointer (TRACE-2 finding 3a).
            let data = mem.read_u32(ped + PLAYER_PED_DATA).unwrap_or(0);
            if data != 0 {
                poke(data + PLAYER_DATA_ANIM_BLOCK, 0u32);
            }
            // Scalar deleting destructor (vtable slot 0); CPed::~CPed removes
            // the ped from the world itself.
            if let Ok(dtor) = mem.read_u32(vtbl) {
                // SAFETY: the ped's own virtual destructor, main thread.
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) -> u32 = std::mem::transmute(dtor as usize);
                    f(ped, 1);
                }
            }
        }
        poke(game::CWORLD_PLAYERS + u32::from(slot) * PLAYER_INFO_SIZE + off::PLAYER_INFO_PED, 0u32);
        self.slots.free(slot);
        crate::log!("players: id={id} ped {ped:#x} (slot {slot}) deleted");
    }

    // ---- per frame (A 0x100166B0, on foot) ----------------------------------

    pub fn tick(&mut self, runner: &mut Runner, now: Instant) {
        let mem = ProcessMemory;
        for p in self.players.values_mut() {
            if p.ped == 0 {
                continue;
            }
            let ped = p.ped;
            let Some(slot) = p.slot else { continue };
            let matrix = mem.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
            let flags = mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0);
            let in_vehicle = flags & (1 << 8) != 0;
            if p.state == state::ON_FOOT
                && !in_vehicle
                && matrix != 0
                && let Some(f) = p.foot
            {
                let surfing = f.surf_id != 0 && f.surf_id != 0xFFFF && f.surf_id < 3000;
                if p.foot_new {
                    p.foot_new = false;
                    // One-shot apply (onfoot §3.1): velocity, then the position correction.
                    put_v3(ped + off::MOVE_SPEED, f.velocity);
                    if !surfing && let Some(cur) = v3(&mem, matrix + off::MATRIX_POS) {
                        let vt = mem.read_u32(ped + off::VTABLE).unwrap_or(0);
                        let moving =
                            vt != VTBL_CPLACEABLE && mem.read_u32(ped + PHYSICAL_MOVING_LIST).unwrap_or(0) != 0;
                        match correct_position(cur, f.position, f.velocity, moving) {
                            Correction::None => {}
                            Correction::Nudge(s) => {
                                put_v3(ped + off::MOVE_SPEED, s);
                            }
                            Correction::Teleport => {
                                p.teleports += 1;
                                put_v3(matrix + off::MATRIX_POS, f.position);
                            }
                        }
                    }
                    // One-shot weapon and ped state (onfoot §4.3, A 0x100166B0).
                    if crate::client::ped_in_world(ped) {
                        apply_weapon(runner, ped, slot, f.weapon);
                    }
                    sniper_state(runner, ped, f.keys);
                    // The synced animation (onfoot §4.1, B 0x100167E4; M8b).
                    crate::behaviourclient::remote_anim(runner, p.id, ped, f.animation);
                }
                if let Some(v) = v3(&mem, ped + off::MOVE_SPEED) {
                    p.last_speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                }
                // Every frame: rotation 25 % toward the synced quaternion, mirrored into the headings.
                if let (Some(right), Some(forward), Some(up)) =
                    (v3(&mem, matrix), v3(&mem, matrix + 0x10), v3(&mem, matrix + 0x20))
                {
                    let cur = quat_from_matrix(&Rotation { right, forward, up });
                    let q = normalise(slerp(f.rotation, cur, remote::SLERP_KEEP));
                    let rows = rows_from_quat(q);
                    put_v3(matrix, rows.right);
                    put_v3(matrix + 0x10, rows.forward);
                    put_v3(matrix + 0x20, rows.up);
                    let h = heading_from_forward(rows.forward);
                    poke(ped + off::PED_HEADING, h);
                    poke(ped + off::PED_HEADING_GOAL, h);
                }
                // Head look (onfoot §5.2; not for NPCs).
                if !p.npc {
                    let (refresh, call) = remote::head_look_due(p, now);
                    if refresh {
                        p.look_refresh = Some(now);
                        let front = swap::with_table(|t| {
                            t.shadows.get(usize::from(slot)).map(|s| [s.aim[0], s.aim[1], s.aim[2]]).unwrap_or_default()
                        });
                        p.look_offset = front.map(|c| c * remote::LOOK_DISTANCE);
                    }
                    if call && let Some(pos) = v3(&mem, matrix + off::MATRIX_POS) {
                        p.look_last = Some(now);
                        let mut target = [0, 1, 2].map(|i| pos[i] + p.look_offset[i]);
                        look_at(ped, 1000, &mut target);
                    }
                }
                // Special actions (onfoot §4.2, A 0x10014950).
                special_actions(runner, ped, f.special_action, in_vehicle, &mut p.phone);
                let stale = p.is_stale(now);
                let intel = mem.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
                let task =
                    if intel != 0 { mem.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
                let jetpack = task != 0 && mem.read_u32(task).unwrap_or(0) == game::VTBL_TASK_SIMPLE_JETPACK;
                let input = PadInput {
                    keys: f.keys,
                    left_right: if stale { 0 } else { f.left_right },
                    up_down: if stale { 0 } else { f.up_down },
                    in_vehicle,
                    jetpack,
                };
                swap::with_table(|t| {
                    if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                        remote::apply_keys(&mut sh.pad, input);
                    }
                });
                // Collision gating (onfoot §3.6).
                if let Some(on) = remote::collision_wanted(&f) {
                    let flags = mem.read_u32(ped + ENTITY_FLAGS).unwrap_or(0);
                    poke(ped + ENTITY_FLAGS, remote::collision_flags(flags, on));
                }
                if f.velocity == [0.0; 3] {
                    put_v3(ped + off::MOVE_SPEED, [0.0; 3]);
                }
                if stale {
                    put_v3(ped + off::MOVE_SPEED, [0.0; 3]);
                    put_v3(matrix + off::MATRIX_POS, f.position);
                }
            }
            if p.state != state::WASTED {
                poke(ped + off::PED_HEALTH, remote::REMOTE_PED_HEALTH);
            }
        }
    }

    /// RPC 69 for a remote player.
    pub fn set_team(&mut self, id: u16, team: u8) {
        if let Some(p) = self.players.get_mut(&id) {
            p.team = team;
        }
    }

    /// RPC 34 for a remote player (A 0x1000F5E0): stored in the slot's skill table while streamed in.
    pub fn set_skill(&mut self, id: u16, skill: u32, level: u16) {
        let Some(slot) = self.players.get(&id).and_then(|p| p.slot) else { return };
        if skill as usize >= SKILLS {
            return;
        }
        swap::with_table(|t| {
            if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                sh.skills[skill as usize] = f32::from(level);
            }
        });
    }

    /// 206 apply, weapon step (A 0x10016370 step 3): arm the shooter with the bullet's weapon, giving it with
    /// 9999 rounds if missing. Returns the shooter's (ped, slot).
    pub fn arm_shooter(&mut self, runner: &mut Runner, id: u16, weapon: u8) -> Option<(u32, u8)> {
        let p = self.players.get(&id)?;
        let (ped, slot) = (p.ped, p.slot?);
        if ped == 0 || !crate::client::ped_in_world(ped) {
            return None;
        }
        apply_weapon(runner, ped, slot, weapon);
        Some((ped, slot))
    }

    /// RPC 166 (A 0x10017570 → ped helper A 0x100B0010): pad keys cleared, jetpack removed, fires
    /// extinguished, teleported to its own position, health 0, EXPLODE_CHAR_HEAD under PlayerInFocus = slot;
    /// state := wasted and the sync caches reset. Measured (TRACE-6): the head stays on.
    pub fn kill(&mut self, runner: &mut Runner, id: u16) {
        let Some(p) = self.players.get_mut(&id) else { return };
        if let (Some(slot), ped) = (p.slot, p.ped)
            && ped != 0
        {
            swap::with_table(|t| {
                if let Some(sh) = t.shadows.get_mut(usize::from(slot)) {
                    let input = PadInput { keys: 0, left_right: 0, up_down: 0, in_vehicle: false, jetpack: false };
                    remote::apply_keys(&mut sh.pad, input);
                }
            });
            let mem = ProcessMemory;
            let in_vehicle = mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
            let intel = mem.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
            let task =
                if intel != 0 { mem.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
            if !in_vehicle && task != 0 && mem.read_u32(task).ok() == Some(game::VTBL_TASK_SIMPLE_JETPACK) {
                // SAFETY: the jetpack task's deleting destructor (thiscall), main thread.
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(JETPACK_DELETING_DTOR as usize);
                    f(task, 1);
                }
                poke(intel + off::INTELLIGENCE_PRIMARY_TASK, 0u32);
            }
            crate::combathooks::extinguish(ped);
            teleport_in_place(ped);
            poke(ped + off::PED_HEALTH, 0.0f32);
            if let Some(h) = game::ped_handle(&mem, ped).ok().flatten() {
                let focus = mem.read_u8(game::PLAYER_IN_FOCUS).unwrap_or(0);
                poke(game::PLAYER_IN_FOCUS, slot);
                runner.run(op::EXPLODE_CHAR_HEAD, &[Arg::Int(h)]);
                poke(game::PLAYER_IN_FOCUS, focus);
            }
        }
        p.state = state::WASTED;
        p.foot_new = false;
        p.last_ts = None;
        crate::log!("players: id={id} died (RPC 166)");
    }

    /// RPC 153 for a remote player: the shared skin path on its ped, if streamed in.
    pub fn set_skin(&mut self, runner: &mut Runner, id: u16, skin: u32) {
        let Some(p) = self.players.get_mut(&id) else { return };
        p.skin = skin;
        if p.ped != 0 && game::ped_handle(&ProcessMemory, p.ped).ok().flatten().is_some() {
            crate::client::set_ped_skin(runner, p.ped, skin);
        }
    }

    /// Streams out every player whose ped faulted inside ProcessControl
    /// (samp's handler in A 0x100A2800: "Removed player %u due to error.").
    pub fn handle_faults(&mut self, runner: &mut Runner) {
        for fault in swap::take_faults() {
            crate::ui::print_debug(&format!(
                "Warning(playerped): Exception 0x{:X} at 0x{:X}.",
                fault.code, fault.address
            ));
            let id = self.players.values().find(|p| p.slot == Some(fault.slot)).map(|p| p.id);
            let Some(id) = id else { continue };
            self.destroy_ped(runner, id);
            if let Some(p) = self.players.get_mut(&id) {
                p.state = state::NONE;
            }
            crate::ui::print_debug(&format!("Removed player {id} due to error."));
        }
    }

    /// `remote` control-channel reply: a header line, then one line per remote player.
    pub fn status(&self, now: Instant, extra: impl Fn(&RemotePlayer) -> String) -> String {
        let mem = ProcessMemory;
        let (calls_remote, calls_local) = swap::call_counts();
        let mut out = format!(
            "remote count={} streamed={} process_control_calls={calls_remote}/{calls_local}",
            self.players.len(),
            self.slots.count()
        );
        for p in self.players.values() {
            let (pos, heading, health) = if p.ped != 0 {
                let matrix = mem.read_u32(p.ped + off::PLACEABLE_MATRIX).unwrap_or(0);
                (
                    if matrix != 0 { v3(&mem, matrix + off::MATRIX_POS) } else { None },
                    mem.read_f32(p.ped + off::PED_HEADING).ok(),
                    mem.read_f32(p.ped + off::PED_HEALTH).ok(),
                )
            } else {
                (None, None, None)
            };
            out.push('\n');
            out.push_str(&remote::format_remote(p, pos, heading, health, now));
            out.push_str(&extra(p));
        }
        out
    }
}

fn remove_marker_blip(runner: &mut Runner, p: &mut RemotePlayer) {
    if p.marker_blip != 0 {
        runner.run(op::REMOVE_BLIP, &[Arg::Int(p.marker_blip)]);
        p.marker_blip = 0;
    }
}

/// samp's coordinate blip (B 0x100A0DA0, style 0 = `ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD`) with sprite 0:
/// colour index, then scale 2 for player ids (3 from 1004 on).
fn add_coord_blip(runner: &mut Runner, pos: [f32; 3], colour: u32) -> i32 {
    let a = [Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(pos[2]), Arg::Int(0), Arg::Local(0)];
    runner.run(op::ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD, &a);
    let blip = runner.local_i32(0);
    runner.run(op::CHANGE_BLIP_COLOUR, &[Arg::Int(blip), Arg::Int(colour as i32)]);
    runner.run(op::CHANGE_BLIP_SCALE, &[Arg::Int(blip), Arg::Int(if colour < 0x3EC { 2 } else { 3 })]);
    blip
}

/// `GET_CURRENT_CHAR_WEAPON` on a ped (samp A 0x100abc90; 0 without a valid handle).
fn current_weapon(runner: &mut Runner, ped: u32) -> u8 {
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return 0 };
    runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Local(0)]);
    runner.local_i32(0) as u8
}

/// Runs `f` with `PlayerInFocus` = slot and the slot's weapon skills swapped in
/// (samp A 0x100ac0a0 / 0x100af7d0); everything is restored afterwards.
fn with_focus<R>(slot: u8, f: impl FnOnce() -> R) -> R {
    let skills = swap::with_table(|t| t.shadows.get(usize::from(slot)).map(|s| s.skills)).unwrap_or([999.0; SKILLS]);
    let mem = ProcessMemory;
    let mut saved = [0f32; SKILLS];
    for (i, v) in saved.iter_mut().enumerate() {
        *v = mem.read_f32(game::WEAPON_SKILL_STATS + 4 * i as u32).unwrap_or(0.0);
    }
    let focus = mem.read_u8(game::PLAYER_IN_FOCUS).unwrap_or(0);
    poke(game::PLAYER_IN_FOCUS, slot);
    for (i, v) in skills.iter().enumerate() {
        poke(game::WEAPON_SKILL_STATS + 4 * i as u32, *v);
    }
    let r = f();
    for (i, v) in saved.iter().enumerate() {
        poke(game::WEAPON_SKILL_STATS + 4 * i as u32, *v);
    }
    poke(game::PLAYER_IN_FOCUS, focus);
    r
}

/// SET_CURRENT_CHAR_WEAPON under the slot's context (A 0x100ac0a0, on foot).
pub(crate) fn set_weapon(runner: &mut Runner, ped: u32, slot: u8, weapon: u8) {
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
    with_focus(slot, || runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(i32::from(weapon))]));
}

/// The one-shot weapon check: arm the synced weapon; if the ped does not have it,
/// give it with 9999 rounds and arm it again (A 0x100166B0 → 0x100af7d0).
pub(crate) fn apply_weapon(runner: &mut Runner, ped: u32, slot: u8, synced: u8) {
    let synced = synced & 0x3F;
    if current_weapon(runner, ped) == synced {
        return;
    }
    set_weapon(runner, ped, slot, synced);
    if current_weapon(runner, ped) == synced {
        return;
    }
    if let Some(model) = game::weapon_model(u32::from(synced))
        && crate::client::load_model(runner, model)
    {
        with_focus(slot, || {
            // SAFETY: CPed::GiveWeapon (thiscall) on a live remote ped, main thread.
            unsafe {
                let give: extern "thiscall" fn(u32, u32, u32, u32) -> u32 =
                    std::mem::transmute(game::PED_GIVE_WEAPON as usize);
                give(ped, u32::from(synced), 9999, 1);
            }
        });
        set_weapon(runner, ped, slot, synced);
    }
    set_weapon(runner, ped, slot, synced);
}

/// Aiming a sniper / rocket weapon puts the ped in state 12 (sniper mode), otherwise a
/// ped in state 12 goes back to 1 (onfoot §4.3).
fn sniper_state(runner: &mut Runner, ped: u32, keys: u16) {
    const PEDSTATE_IDLE: u32 = 1;
    const PEDSTATE_SNIPER_MODE: u32 = 12;
    if keys & remote::KEY_AIM != 0 {
        if remote::SNIPER_STATE_WEAPONS.iter().any(|&w| current_weapon(runner, ped) == w) {
            poke(ped + off::PED_STATE, PEDSTATE_SNIPER_MODE);
        }
    } else if ProcessMemory.read_u32(ped + off::PED_STATE).unwrap_or(0) == PEDSTATE_SNIPER_MODE {
        poke(ped + off::PED_STATE, PEDSTATE_IDLE);
    }
}

/// Duck and hands-up edges for a remote ped (onfoot §4.2).
fn special_actions(runner: &mut Runner, ped: u32, action: u8, in_vehicle: bool, phone: &mut bool) {
    let mem = ProcessMemory;
    if !crate::client::ped_in_world(ped) {
        return;
    }
    let Some(h) = game::ped_handle(&mem, ped).ok().flatten() else { return };
    let flags = mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0);
    let ducking = flags & PED_FLAG_DUCKING != 0;
    let intel = mem.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
    let task = if intel != 0 { mem.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
    let hands_up = !in_vehicle && task != 0 && mem.read_u32(task).ok() == Some(game::VTBL_TASK_HANDS_UP);
    for step in remote::special_action_steps(action, ducking, *phone, hands_up) {
        match step {
            SpecialStep::StartDuck => {
                if !in_vehicle && intel != 0 {
                    // SAFETY: CPedIntelligence::SetTaskDuckSecondary (thiscall), main thread.
                    unsafe {
                        let f: extern "thiscall" fn(u32, u16) =
                            std::mem::transmute(game::INTELLIGENCE_SET_TASK_DUCK_SECONDARY as usize);
                        f(intel, 0);
                    }
                }
            }
            SpecialStep::StopDuck => {
                let flags = mem.read_u32(ped + off::PED_FLAGS).unwrap_or(0);
                poke(ped + off::PED_FLAGS, flags & !PED_FLAG_DUCKING);
            }
            SpecialStep::StartHandsUp => {
                if !in_vehicle {
                    runner.run(op::TASK_HANDS_UP, &[Arg::Int(h), Arg::Int(-2)]);
                }
            }
            SpecialStep::StopHandsUp => teleport_in_place(ped),
            SpecialStep::StartPhone | SpecialStep::StopPhone => {
                // A 0x100ae0b0: flag, then TASK_USE_MOBILE_PHONE(ped, flag).
                *phone = step == SpecialStep::StartPhone;
                runner.run(op::TASK_USE_MOBILE_PHONE, &[Arg::Int(h), Arg::Int(i32::from(*phone))]);
            }
        }
    }
}

/// `CTaskSimpleJetPack` scalar deleting destructor (A 0x100ACDA0).
const JETPACK_DELETING_DTOR: u32 = 0x6801D0;

/// `CPed::bIsDucking` (m_nPedFlags bit 26).
const PED_FLAG_DUCKING: u32 = 1 << 26;

/// Re-sets a ped to its own matrix position through its `Teleport` virtual (A 0x100adb60).
fn teleport_in_place(ped: u32) {
    let mem = ProcessMemory;
    let matrix = mem.read_u32(ped + off::PLACEABLE_MATRIX).unwrap_or(0);
    let Some(pos) = (matrix != 0).then(|| v3(&mem, matrix + off::MATRIX_POS)).flatten() else { return };
    let vtbl = mem.read_u32(ped + off::VTABLE).unwrap_or(0);
    let Ok(target) = mem.read_u32(vtbl + off::VT_TELEPORT) else { return };
    // SAFETY: the ped's own Teleport virtual (thiscall: CVector by value, bool).
    unsafe {
        let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
        f(ped, pos[0], pos[1], pos[2], 0);
    }
}
