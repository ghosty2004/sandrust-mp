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
//! Deviations (docs/phase3/m3-notes.md): no name tags / markers / colours
//! applied, no fighting style, no weapon or animation sync, no special
//! actions, no surfing positions, no collision gating, no vehicles;
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
    self, Correction, PadInput, RemotePlayer, SKILLS, SlotAllocator, correct_position, heading_from_forward,
    normalise, rows_from_quat, samp_deg_to_rad, slerp, state,
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

/// `g_ikChainMan.LookAt("FollowPedSA", ped, nullptr, 1000, bone −1, &pos, false, 0.1, 500, 3, false)`
/// (onfoot §5.2).
fn look_at(ped: u32, pos: &mut [f32; 3]) {
    type LookAt = extern "thiscall" fn(u32, *const u8, u32, u32, i32, i32, *mut f32, u8, f32, i32, i32, u8);
    // SAFETY: GTA's IK chain manager on a live ped, main thread; the strings and vector outlive the call.
    unsafe {
        let f: LookAt = std::mem::transmute(IK_LOOK_AT as usize);
        f(IK_CHAIN_MANAGER, c"FollowPedSA".as_ptr().cast(), ped, 0, 1000, -1, pos.as_mut_ptr(), 0, 0.1, 500, 3, 0);
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
}

impl Players {
    pub fn set_init_game(&mut self, local_id: u16, death_drop_money: u32) {
        self.local_id = Some(local_id);
        self.death_drop_money = death_drop_money.min(i32::MAX as u32) as i32;
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
                    self.destroy_ped(j.player_id);
                }
                self.players.insert(j.player_id, RemotePlayer::new(j.player_id, name, j.colour, j.is_npc != 0));
            }
            R::PlayerQuit(q) => {
                crate::log!("players: quit id={} reason={}", q.player_id, q.reason);
                self.destroy_ped(q.player_id);
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
            }
            R::PlayerStreamOut(s) => {
                crate::log!("players: stream-out id={}", s.player_id);
                self.destroy_ped(s.player_id);
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
    pub fn clear(&mut self) {
        let ids: Vec<u16> = self.players.keys().copied().collect();
        for id in ids {
            self.destroy_ped(id);
        }
        self.players.clear();
    }

    // ---- ped lifecycle ------------------------------------------------------

    fn create_ped(&mut self, runner: &mut Runner, id: u16, pos: [f32; 3], angle: f32, skills: [f32; SKILLS]) {
        if !crate::dll::REMOTE_PEDS_READY.load(std::sync::atomic::Ordering::SeqCst) {
            crate::log!("players: sector relocation or swap hooks missing; not creating a ped for id {id}");
            return;
        }
        self.destroy_ped(id);
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
    }

    fn destroy_ped(&mut self, id: u16) {
        let Some(p) = self.players.get_mut(&id) else { return };
        let (ped, slot) = (p.ped, p.slot);
        p.ped = 0;
        p.slot = None;
        let Some(slot) = slot else { return };
        swap::with_table(|t| t.unbind(slot));
        let mem = ProcessMemory;
        let vtbl = mem.read_u32(ped + off::VTABLE).unwrap_or(0);
        if ped != 0 && vtbl != 0 && vtbl != VTBL_CPLACEABLE {
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

    pub fn tick(&mut self, now: Instant) {
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
                        let moving = vt != VTBL_CPLACEABLE && mem.read_u32(ped + PHYSICAL_MOVING_LIST).unwrap_or(0) != 0;
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
                        look_at(ped, &mut target);
                    }
                }
                let stale = p.is_stale(now);
                let intel = mem.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
                let task = if intel != 0 { mem.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
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

    /// `remote` control-channel reply: a header line, then one line per remote player.
    pub fn status(&self, now: Instant) -> String {
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
        }
        out
    }
}
