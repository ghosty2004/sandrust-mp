//! Pickups, actors, checkpoints, gang zones, map icons and 3D text labels (Windows only, milestone M6).
//!
//! Specs: world-entities.md (RPC layouts and client behaviour) with its RE-10 additions, and the original client
//! measured by TRACE-6 (docs/phase3/trace-notes.md "World entities"):
//! * pickups: CREATE_PICKUP(model, type, x, y, z); RPC 131 rel 9 from the collision path (row 44), type-14
//!   pickups polled with HAS_PICKUP_BEEN_COLLECTED every 7 frames (rel 10);
//! * actors: busy model load, CREATE_CHAR(5, skin, x, y, z − 1), heading, 0x0446(h, 0), decision maker 65542,
//!   health (≤ 0 → EXPLODE_CHAR_HEAD), proofs; RPC 177 from the damage hook (worldhooks row 53);
//! * checkpoints: LOCATE_CHAR_ANY_MEANS_3D every frame, blip 0x04CE + colour 1005 + scale 3 once; race checkpoint
//!   blip / DELETE_CHECKPOINT / CREATE_CHECKPOINT order;
//! * gang zones: drawn by the radar hooks (worldhooks rows 41/42) from the records kept here;
//! * map icons: blip opcode by style, colour + scale for type 0 (scale rule: unsigned colour < 1004 → 2);
//! * 3D labels: small font, 4-way black outline, rect [x, y, x+1, y+1], format 0x101, colour RGBA → ARGB, draw
//!   distance, LOS test IS_LINE_OF_SIGHT_CLEAR(label → camera, 1, 0, 0, 1, 0).

use std::collections::BTreeMap;
use std::sync::Mutex;

use samp_proto::rpcs::OutgoingRpc;
use samp_proto::rpcs::world::{self as rw, Incoming as W};
use samp_proto::types::Vec3;

use crate::netclient::NetClient;
use crate::players::Players;
use crate::script::{Arg, Runner, op as sop};
use crate::vehicles::Vehicles;
use crate::worldents::{self as world, GangZone, op};
use crate::worldhooks::{self as wh, Event, g, rd, wr};

/// `IS_LINE_OF_SIGHT_CLEAR` (gta-reversed eScriptCommands.h); samp descriptor A 0x100ea618 "ffffffiiiii".
const IS_LINE_OF_SIGHT_CLEAR: u16 = 0x06BD;
/// Camera source position samp's label LOS test uses (pointer A 0x101039C8 = 0xB6F32C, vector at +0xC).
const LOS_CAMERA_POS: u32 = 0xB6F32C + 0xC;
/// `CPedIntelligence::FlushImmediately(bool)` (RPC 174).
const FLUSH_IMMEDIATELY: u32 = 0x601640;

fn vec(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn send<P: OutgoingRpc>(net: &mut NetClient, p: &P) {
    net.send_rpc(p, P::RELIABILITY);
}

fn ped_handle(ped: u32) -> Option<i32> {
    crate::game::ped_handle(&crate::patch::ProcessMemory, ped).ok().flatten()
}

fn local_ped() -> u32 {
    rd(g::CWORLD_PLAYERS)
}

/// Busy model load like the actor/pickup creators (HAS_MODEL_LOADED, REQUEST_MODEL, LOAD_ALL_MODELS_NOW, poll).
fn busy_load(runner: &mut Runner, model: i32) -> bool {
    if runner.run(sop::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        return true;
    }
    runner.run(sop::REQUEST_MODEL, &[Arg::Int(model)]);
    runner.run(sop::LOAD_ALL_MODELS_NOW, &[]);
    let t0 = std::time::Instant::now();
    while !runner.run(sop::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        if t0.elapsed() > std::time::Duration::from_secs(10) {
            crate::log!("entities: model {model} not loaded after 10 s");
            return false;
        }
        std::thread::sleep(std::time::Duration::ZERO);
    }
    true
}

/// Animation names for SCM string parameters (`Arg::Text` takes `&'static str`; names are interned once).
fn intern(b: &[u8]) -> &'static str {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let s = String::from_utf8_lossy(b).into_owned();
    let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = g.iter().find(|n| **n == s) {
        return n;
    }
    let n: &'static str = Box::leak(s.into_boxed_str());
    g.push(n);
    n
}

#[derive(Clone, Copy, Debug)]
struct Pickup {
    model: i32,
    kind: i32,
    pos: [f32; 3],
    handle: i32,
}

#[derive(Clone, Copy, Debug)]
struct Actor {
    skin: u32,
    handle: i32,
    ped: u32,
    invulnerable: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Checkpoint {
    active: bool,
    pos: [f32; 3],
    size: f32,
    blip: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct RaceCheckpoint {
    active: bool,
    kind: u8,
    pos: [f32; 3],
    next: [f32; 3],
    size: f32,
    blip: i32,
    handle: i32,
}

#[derive(Clone, Debug)]
struct Label {
    colour: u32,
    pos: [f32; 3],
    draw_distance: f32,
    los: bool,
    player: u16,
    vehicle: u16,
    text: Vec<u8>,
}

/// One label to draw this frame (screen position from the client tick, drawn in the Present pass).
#[derive(Clone, Debug)]
pub struct LabelDraw {
    /// World position (projected in the Present pass like samp's 3D-text painter).
    pub pos: [f32; 3],
    pub text: Vec<u8>,
    pub argb: u32,
}

static LABEL_DRAWS: Mutex<Vec<LabelDraw>> = Mutex::new(Vec::new());

/// The labels of the current frame (UI Present pass).
pub fn label_draws() -> Vec<LabelDraw> {
    LABEL_DRAWS.try_lock().map(|g| g.clone()).unwrap_or_default()
}

#[derive(Default)]
pub struct Entities {
    pickups: BTreeMap<i32, Pickup>,
    actors: BTreeMap<u16, Actor>,
    cp: Checkpoint,
    rcp: RaceCheckpoint,
    icons: BTreeMap<u8, i32>,
    labels: BTreeMap<u16, Label>,
    frame: u64,
    pub sent_131: u32,
    pub sent_177: u32,
    local_id: Option<u16>,
}

impl Entities {
    pub fn set_local_id(&mut self, id: u16) {
        self.local_id = Some(id);
    }

    pub fn on_rpc(&mut self, runner: &mut Runner, r: W) {
        match r {
            // ---- pickups (A 0x10013270 / 0x10013320)
            W::PlayerCreatePickup(p) => {
                if !(0..world::MAX_PICKUPS).contains(&p.pickup_id) || self.pickups.len() >= world::MAX_PICKUPS as usize
                {
                    return;
                }
                if let Some(old) = self.pickups.remove(&p.pickup_id) {
                    runner.run(op::REMOVE_PICKUP, &[Arg::Int(old.handle)]);
                }
                let mut model = p.model;
                if wh::model_info(model) == 0 {
                    model = world::MODEL_UNKNOWN;
                }
                if (0..=world::MAX_MODEL).contains(&model) {
                    busy_load(runner, model);
                }
                let pos = vec(p.position);
                runner.run(
                    op::CREATE_PICKUP,
                    &[
                        Arg::Int(model),
                        Arg::Int(p.pickup_type),
                        Arg::Float(pos[0]),
                        Arg::Float(pos[1]),
                        Arg::Float(pos[2]),
                        Arg::Local(0),
                    ],
                );
                let handle = runner.local_i32(0);
                self.pickups.insert(p.pickup_id, Pickup { model, kind: p.pickup_type, pos, handle });
                let id = p.pickup_id;
                wh::with_shared(|s| {
                    s.pickups.insert((handle & 0xFFFF) as u32, id);
                    s.cooldown.insert(id, 0);
                });
            }
            W::PlayerDestroyPickup(p) => {
                if let Some(old) = self.pickups.remove(&p.pickup_id) {
                    runner.run(op::REMOVE_PICKUP, &[Arg::Int(old.handle)]);
                    wh::with_shared(|s| {
                        s.pickups.retain(|_, v| *v != p.pickup_id);
                        s.cooldown.remove(&p.pickup_id);
                    });
                }
            }
            // ---- actors (A 0x10001900, 0x1009C310 …)
            W::ShowActorForPlayer(a) => {
                if a.actor_id >= world::MAX_ACTORS || wh::model_info(a.skin as i32) == 0 {
                    return;
                }
                if self.actors.contains_key(&a.actor_id) {
                    crate::log!("Warning: actor {} was not deleted", a.actor_id);
                    self.delete_actor(runner, a.actor_id);
                }
                if !busy_load(runner, a.skin as i32) {
                    return;
                }
                let p = vec(a.position);
                runner.run(
                    op::CREATE_CHAR,
                    &[
                        Arg::Int(world::ACTOR_PED_TYPE),
                        Arg::Int(a.skin as i32),
                        Arg::Float(p[0]),
                        Arg::Float(p[1]),
                        Arg::Float(p[2] - 1.0),
                        Arg::Local(0),
                    ],
                );
                let handle = runner.local_i32(0);
                let ped = ped_at(handle);
                runner.run(sop::SET_CHAR_HEADING, &[Arg::Int(handle), Arg::Float(a.angle)]);
                runner.run(sop::SET_CHAR_SUFFERS_CRITICAL_HITS, &[Arg::Int(handle), Arg::Int(0)]);
                runner.run(op::SET_CHAR_DECISION_MAKER, &[Arg::Int(handle), Arg::Int(world::ACTOR_DECISION_MAKER)]);
                set_actor_health(runner, handle, ped, a.health);
                let pr = world::actor_proofs(a.invulnerable != 0);
                runner.run(
                    sop::SET_CHAR_PROOFS,
                    &[
                        Arg::Int(handle),
                        Arg::Int(pr[0]),
                        Arg::Int(pr[1]),
                        Arg::Int(pr[2]),
                        Arg::Int(pr[3]),
                        Arg::Int(pr[4]),
                    ],
                );
                self.actors.insert(a.actor_id, Actor { skin: a.skin, handle, ped, invulnerable: a.invulnerable != 0 });
                let id = a.actor_id;
                wh::with_shared(|s| {
                    s.actor_peds.insert(ped, id);
                });
            }
            W::HideActorForPlayer(h) => self.delete_actor(runner, h.actor_id),
            W::ApplyActorAnimationForPlayer(a) => {
                let Some(ac) = self.actors.get(&a.actor_id).copied() else { return };
                if ac.ped == 0 || world::actor_anim_refused(&a.library) {
                    return;
                }
                let lib = intern(&a.library);
                let name = intern(&a.name);
                if !runner.run(sop::HAS_ANIMATION_LOADED, &[Arg::Text(lib)]) {
                    runner.run(sop::REQUEST_ANIMATION, &[Arg::Text(lib)]);
                    let t0 = std::time::Instant::now();
                    while !runner.run(sop::HAS_ANIMATION_LOADED, &[Arg::Text(lib)]) {
                        if t0.elapsed() > std::time::Duration::from_millis(15) {
                            // Gives up without playing (DIFF-2: the original runs no 0x0812 then).
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
                runner.run(
                    op::TASK_PLAY_ANIM_NON_INTERRUPTABLE,
                    &[
                        Arg::Int(ac.handle),
                        Arg::Text(name),
                        Arg::Text(lib),
                        Arg::Float(a.delta),
                        Arg::Int(i32::from(a.loop_anim)),
                        Arg::Int(i32::from(a.lock_x)),
                        Arg::Int(i32::from(a.lock_y)),
                        Arg::Int(i32::from(a.freeze)),
                        Arg::Int(a.time as i32),
                    ],
                );
            }
            W::ClearActorAnimationsForPlayer(c) => {
                if let Some(ac) = self.actors.get(&c.actor_id).filter(|a| a.ped != 0) {
                    let intel: u32 = rd(ac.ped + crate::game::off::PED_INTELLIGENCE);
                    if intel != 0 {
                        // SAFETY: CPedIntelligence::FlushImmediately(bool) (thiscall) on the actor's intelligence.
                        unsafe {
                            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(FLUSH_IMMEDIATELY as usize);
                            f(intel, 1);
                        }
                    }
                }
            }
            W::SetActorFacingAngleForPlayer(f) => {
                if let Some(ac) = self.actors.get(&f.actor_id).filter(|a| a.ped != 0) {
                    wr(ac.ped + crate::game::off::PED_HEADING_GOAL, world::samp_actor_heading(f.angle));
                }
            }
            W::SetActorPosForPlayer(p) => {
                if let Some(ac) = self.actors.get(&p.actor_id).filter(|a| a.ped != 0) {
                    teleport(ac.ped, vec(p.position));
                }
            }
            W::SetActorHealthForPlayer(h) => {
                if let Some(ac) = self.actors.get(&h.actor_id).copied().filter(|a| a.ped != 0) {
                    set_actor_health(runner, ac.handle, ac.ped, h.health);
                }
            }
            // ---- checkpoints (A 0x100A1E20 / 0x100A2140)
            W::SetCheckpoint(c) => {
                self.cp.pos = vec(c.position);
                self.cp.size = c.size;
                if self.cp.blip != 0 {
                    runner.run(sop::REMOVE_BLIP, &[Arg::Int(self.cp.blip)]);
                    self.cp.blip = add_checkpoint_blip(runner, self.cp.pos);
                }
                self.cp.active = true;
            }
            W::DisableCheckpoint(_) => self.cp.active = false,
            W::SetRaceCheckpoint(c) => {
                let r = &mut self.rcp;
                r.kind = c.checkpoint_type;
                r.pos = vec(c.position);
                r.next = vec(c.next_position);
                r.size = c.size;
                if r.blip != 0 {
                    runner.run(sop::REMOVE_BLIP, &[Arg::Int(r.blip)]);
                    r.blip = add_checkpoint_blip(runner, r.pos);
                }
                if r.handle != 0 {
                    runner.run(op::DELETE_CHECKPOINT, &[Arg::Int(r.handle)]);
                    r.handle = 0;
                }
                runner.run(
                    op::CREATE_CHECKPOINT,
                    &[
                        Arg::Int(i32::from(r.kind)),
                        Arg::Float(r.pos[0]),
                        Arg::Float(r.pos[1]),
                        Arg::Float(r.pos[2]),
                        Arg::Float(r.next[0]),
                        Arg::Float(r.next[1]),
                        Arg::Float(r.next[2]),
                        Arg::Float(r.size),
                        Arg::Local(0),
                    ],
                );
                r.handle = runner.local_i32(0);
                r.active = true;
            }
            W::DisableRaceCheckpoint(_) => self.rcp.active = false,
            // ---- gang zones (A 0x10002180 …; the wire id is bounds-checked here)
            W::ShowGangZone(z) => {
                if z.zone_id < world::MAX_GANG_ZONES {
                    wh::with_shared(|s| {
                        s.zones.insert(z.zone_id, GangZone::new(z.min.x, z.min.y, z.max.x, z.max.y, z.colour));
                    });
                }
            }
            W::HideGangZone(z) => wh::with_shared(|s| {
                s.zones.remove(&z.zone_id);
            }),
            W::FlashGangZone(z) => wh::with_shared(|s| {
                if let Some(r) = s.zones.get_mut(&z.zone_id) {
                    r.flash = z.colour;
                }
            }),
            W::StopFlashGangZone(z) => wh::with_shared(|s| {
                if let Some(r) = s.zones.get_mut(&z.zone_id) {
                    r.flash = r.colour;
                }
            }),
            // ---- map icons (A 0x1000a300 → 0x100a0dd0)
            W::SetPlayerMapIcon(m) => {
                if m.icon_id >= world::MAX_MAP_ICONS {
                    return;
                }
                if let Some(h) = self.icons.remove(&m.icon_id) {
                    runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
                }
                let p = vec(m.position);
                runner.run(
                    world::map_icon_opcode(m.style),
                    &[
                        Arg::Float(p[0]),
                        Arg::Float(p[1]),
                        Arg::Float(p[2]),
                        Arg::Int(i32::from(m.icon_type)),
                        Arg::Local(0),
                    ],
                );
                let h = runner.local_i32(0);
                if m.icon_type == 0 {
                    runner.run(sop::CHANGE_BLIP_COLOUR, &[Arg::Int(h), Arg::Int(m.colour as i32)]);
                    runner.run(sop::CHANGE_BLIP_SCALE, &[Arg::Int(h), Arg::Int(world::map_icon_scale(m.colour))]);
                }
                self.icons.insert(m.icon_id, h);
            }
            W::RemovePlayerMapIcon(m) => {
                if m.icon_id < world::MAX_MAP_ICONS {
                    let h = self.icons.remove(&m.icon_id).unwrap_or(0);
                    runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
                }
            }
            // ---- 3D labels (A 0x100011d0 / 0x100012e0)
            W::PlayerShowTextLabel(l) => {
                if l.label_id < world::MAX_LABELS {
                    self.labels.insert(
                        l.label_id,
                        Label {
                            colour: l.colour,
                            pos: vec(l.position),
                            draw_distance: l.draw_distance,
                            los: l.test_los != 0,
                            player: l.attached_player_id,
                            vehicle: l.attached_vehicle_id,
                            text: l.text,
                        },
                    );
                }
            }
            W::PlayerHideTextLabel(l) => {
                self.labels.remove(&l.label_id);
            }
            _ => {}
        }
    }

    fn delete_actor(&mut self, runner: &mut Runner, id: u16) {
        if let Some(a) = self.actors.remove(&id) {
            wh::with_shared(|s| {
                s.actor_peds.remove(&a.ped);
            });
            // The wrapper's destructor (A 0x1009C460) deletes the ped through its own virtual destructor
            // (scalar deleting, flag 1) when the handle is still valid — no script command.
            let _ = runner;
            if a.ped != 0 && ped_at(a.handle) == a.ped && rd::<u32>(a.ped) != g::VTBL_PLACEABLE {
                let vt: u32 = rd(a.ped);
                // SAFETY: the ped's own scalar deleting destructor (thiscall, flag).
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(rd::<u32>(vt) as usize);
                    f(a.ped, 1);
                }
            }
        }
    }

    /// Hook events (pickup collisions, actor damage) → RPCs, in the order they happened.
    pub fn hook_events(&mut self, net: &mut NetClient) {
        let events = wh::with_shared(|s| std::mem::take(&mut s.events));
        for e in events {
            match e {
                Event::PickupHit(id) => {
                    send(net, &rw::OnPlayerPickUpPickup { pickup_id: id });
                    self.sent_131 += 1;
                }
                Event::ActorDamage { actor, damage, weapon, part } => {
                    send(
                        net,
                        &rw::OnPlayerDamageActor { unknown: false, actor_id: actor, damage, weapon, body_part: part },
                    );
                    self.sent_177 += 1;
                }
            }
        }
    }

    /// Per frame, from the netgame tick (checkpoints A 0x100A1F60, pickup poll A 0x10013520 every 7 frames).
    pub fn tick(&mut self, runner: &mut Runner, net: &mut NetClient, players: &Players, vehicles: &Vehicles) {
        self.frame += 1;
        let ped = local_ped();
        let ph = ped_handle(ped);
        // Normal checkpoint.
        if self.cp.active {
            if let Some(h) = ph {
                let (p, s) = (self.cp.pos, self.cp.size);
                runner.run(
                    op::LOCATE_CHAR_ANY_MEANS_3D,
                    &[
                        Arg::Int(h),
                        Arg::Float(p[0]),
                        Arg::Float(p[1]),
                        Arg::Float(p[2]),
                        Arg::Float(s),
                        Arg::Float(s),
                        Arg::Float(s),
                        Arg::Int(1),
                    ],
                );
            }
            if self.cp.blip == 0 {
                self.cp.blip = add_checkpoint_blip(runner, self.cp.pos);
            }
        } else if self.cp.blip != 0 {
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(self.cp.blip)]);
            self.cp.blip = 0;
        }
        // Race checkpoint.
        if self.rcp.active {
            if self.rcp.blip == 0 {
                self.rcp.blip = add_checkpoint_blip(runner, self.rcp.pos);
            }
        } else {
            if self.rcp.blip != 0 {
                runner.run(sop::REMOVE_BLIP, &[Arg::Int(self.rcp.blip)]);
                self.rcp.blip = 0;
            }
            if self.rcp.handle != 0 {
                runner.run(op::DELETE_CHECKPOINT, &[Arg::Int(self.rcp.handle)]);
                self.rcp.handle = 0;
            }
        }
        // Pickup poll.
        if self.frame.is_multiple_of(world::PICKUP_POLL_FRAMES) {
            let list: Vec<(i32, Pickup)> = self.pickups.iter().map(|(k, v)| (*k, *v)).collect();
            for (id, p) in list {
                if p.kind == world::PICKUP_TYPE_POLLED {
                    if runner.run(op::HAS_PICKUP_BEEN_COLLECTED, &[Arg::Int(p.handle)]) {
                        net.send_rpc(
                            &rw::OnPlayerPickUpPickup { pickup_id: id },
                            rw::OnPlayerPickUpPickup::POLL_SEND_PARAMS.reliability,
                        );
                        self.sent_131 += 1;
                    }
                } else {
                    wh::with_shared(|s| {
                        if let Some(c) = s.cooldown.get_mut(&id) {
                            *c = c.saturating_sub(1);
                        }
                    });
                }
            }
        }
        self.labels_tick(runner, players, vehicles);
    }

    /// 3D label placement (A 0x10001350) and screen projection for the Present pass.
    fn labels_tick(&mut self, runner: &mut Runner, players: &Players, vehicles: &Vehicles) {
        let cam = rd_v3(g::CAMERA_POS);
        let mut out = Vec::new();
        for l in self.labels.values() {
            let pos = if l.player != rw::INVALID_ID {
                // TRACE-6: a label attached to the local player is not drawn for that player.
                if Some(l.player) == self.local_id {
                    continue;
                }
                let ped = match players.get(l.player) {
                    Some(p) if l.player < 1004 => p.ped,
                    _ => continue,
                };
                if ped == 0 {
                    continue;
                }
                let Some(head) = head_position(ped) else { continue };
                let mut p = [head[0] + l.pos[0], head[1] + l.pos[1], head[2] + l.pos[2]];
                p[2] += 0.05 * world::dist3(p, cam);
                p
            } else if l.vehicle != rw::INVALID_ID {
                let Some(v) = vehicles.gta_ptr(l.vehicle) else { continue };
                let m: u32 = rd(v + g::ENTITY_MATRIX);
                if m == 0 {
                    continue;
                }
                let (r, f, u, o) = (rd_v3(m), rd_v3(m + 0x10), rd_v3(m + 0x20), rd_v3(m + 0x30));
                [0, 1, 2].map(|i| o[i] + r[i] * l.pos[0] + f[i] * l.pos[1] + u[i] * l.pos[2])
            } else {
                l.pos
            };
            if world::dist3(pos, cam) > l.draw_distance {
                continue;
            }
            if l.los {
                let c = rd_v3(LOS_CAMERA_POS);
                // One test per label and frame (parity run orig-20261104-233854: 189 frames × 1 call).
                let args = [
                    Arg::Float(pos[0]),
                    Arg::Float(pos[1]),
                    Arg::Float(pos[2]),
                    Arg::Float(c[0]),
                    Arg::Float(c[1]),
                    Arg::Float(c[2]),
                    Arg::Int(1),
                    Arg::Int(0),
                    Arg::Int(0),
                    Arg::Int(1),
                    Arg::Int(0),
                ];
                let clear = runner.run(IS_LINE_OF_SIGHT_CLEAR, &args);
                if !clear {
                    continue;
                }
            }
            out.push(LabelDraw { pos, text: l.text.clone(), argb: world::label_argb(l.colour) });
        }
        if let Ok(mut g) = LABEL_DRAWS.lock() {
            *g = out;
        }
    }

    pub fn clear(&mut self, runner: &mut Runner) {
        for (_, p) in std::mem::take(&mut self.pickups) {
            runner.run(op::REMOVE_PICKUP, &[Arg::Int(p.handle)]);
        }
        let ids: Vec<u16> = self.actors.keys().copied().collect();
        for id in ids {
            self.delete_actor(runner, id);
        }
        for (_, h) in std::mem::take(&mut self.icons) {
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
        }
        self.cp.active = false;
        self.rcp.active = false;
        self.labels.clear();
        wh::with_shared(|s| {
            s.pickups.clear();
            s.cooldown.clear();
            s.actor_peds.clear();
            s.zones.clear();
        });
        if let Ok(mut g) = LABEL_DRAWS.lock() {
            g.clear();
        }
    }

    pub fn header_fields(&self) -> String {
        let zones = wh::with_shared(|s| s.zones.len());
        format!(
            "pickups={} actors={} checkpoint={} race_checkpoint={} zones={zones} icons={} labels={} sent_131={} sent_177={}",
            self.pickups.len(),
            self.actors.len(),
            u8::from(self.cp.active),
            u8::from(self.rcp.active),
            self.icons.len(),
            self.labels.len(),
            self.sent_131,
            self.sent_177
        )
    }

    pub fn status_lines(&self, out: &mut Vec<String>) {
        for (id, p) in &self.pickups {
            out.push(format!(
                "pickup id={id} model={} type={} handle={} pos={:.3},{:.3},{:.3}",
                p.model, p.kind, p.handle, p.pos[0], p.pos[1], p.pos[2]
            ));
        }
        for (id, a) in &self.actors {
            let (pos, hp) = if a.ped != 0 {
                let m: u32 = rd(a.ped + g::ENTITY_MATRIX);
                let p = if m != 0 { rd_v3(m + 0x30) } else { [0.0; 3] };
                (format!("{:.3},{:.3},{:.3}", p[0], p[1], p[2]), rd::<f32>(a.ped + crate::game::off::PED_HEALTH))
            } else {
                ("-".into(), 0.0)
            };
            out.push(format!(
                "actor id={id} skin={} handle={} ped={:#x} pos={pos} health={hp:.1} invulnerable={}",
                a.skin,
                a.handle,
                a.ped,
                u8::from(a.invulnerable)
            ));
        }
        if self.cp.active || self.cp.blip != 0 {
            let c = &self.cp;
            out.push(format!(
                "checkpoint active={} pos={:.3},{:.3},{:.3} size={:.2} blip={}",
                u8::from(c.active),
                c.pos[0],
                c.pos[1],
                c.pos[2],
                c.size,
                c.blip
            ));
        }
        if self.rcp.active || self.rcp.handle != 0 {
            let c = &self.rcp;
            out.push(format!(
                "race_checkpoint active={} type={} pos={:.3},{:.3},{:.3} next={:.3},{:.3},{:.3} size={:.2} blip={} handle={}",
                u8::from(c.active),
                c.kind,
                c.pos[0],
                c.pos[1],
                c.pos[2],
                c.next[0],
                c.next[1],
                c.next[2],
                c.size,
                c.blip,
                c.handle
            ));
        }
        let zones: Vec<(u16, GangZone)> = wh::with_shared(|s| s.zones.iter().map(|(k, v)| (*k, *v)).collect());
        for (id, z) in zones {
            out.push(format!(
                "zone id={id} rect={:.1},{:.1},{:.1},{:.1} colour={:08x} flash={:08x}",
                z.rect[0], z.rect[1], z.rect[2], z.rect[3], z.colour, z.flash
            ));
        }
        for (id, h) in &self.icons {
            out.push(format!("icon id={id} blip={h}"));
        }
        for (id, l) in &self.labels {
            out.push(format!(
                "label id={id} colour={:08x} pos={:.3},{:.3},{:.3} dd={:.1} los={} player={} vehicle={} text={:?}",
                l.colour,
                l.pos[0],
                l.pos[1],
                l.pos[2],
                l.draw_distance,
                u8::from(l.los),
                l.player,
                l.vehicle,
                String::from_utf8_lossy(&l.text)
            ));
        }
        let drawn = label_draws().len();
        out.push(format!("labels_drawn count={drawn}"));
    }
}

fn rd_v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

/// `CPool<CPed>::GetAt(handle)` (index = handle >> 8, slot byte must match).
fn ped_at(handle: i32) -> u32 {
    let pool: u32 = rd(crate::game::PED_POOL);
    if pool == 0 || handle < 0 {
        return 0;
    }
    let objects: u32 = rd(pool);
    let flags: u32 = rd(pool + 4);
    let size: i32 = rd(pool + 8);
    let index = (handle >> 8) as u32;
    if index as i32 >= size || rd::<u8>(flags + index) != (handle & 0xFF) as u8 {
        return 0;
    }
    objects + index * crate::game::PED_POOL_ELEMENT
}

/// RPC 178 / actor creation (A 0x1009C630): CPed health; ≤ 0 → EXPLODE_CHAR_HEAD.
fn set_actor_health(runner: &mut Runner, handle: i32, ped: u32, health: f32) {
    if ped == 0 {
        return;
    }
    wr(ped + crate::game::off::PED_HEALTH, health);
    if health <= 0.0 {
        runner.run(op::EXPLODE_CHAR_HEAD, &[Arg::Int(handle)]);
    }
}

fn teleport(ent: u32, p: [f32; 3]) {
    let vt: u32 = rd(ent);
    if vt == g::VTBL_PLACEABLE {
        return;
    }
    let target: u32 = rd(vt + 0x38);
    // SAFETY: the entity's own virtual (thiscall, CVector by value, bool).
    unsafe {
        let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
        f(ent, p[0], p[1], p[2], 0);
    }
}

/// Checkpoint blip (A 0x100A1F60): 0x04CE (sprite 0), colour 0x3ED (1005), scale 3.
fn add_checkpoint_blip(runner: &mut Runner, p: [f32; 3]) -> i32 {
    runner.run(
        op::ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD,
        &[Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Int(0), Arg::Local(0)],
    );
    let h = runner.local_i32(0);
    runner.run(sop::CHANGE_BLIP_COLOUR, &[Arg::Int(h), Arg::Int(world::BLIP_COLOUR_CHECKPOINT)]);
    runner.run(sop::CHANGE_BLIP_SCALE, &[Arg::Int(h), Arg::Int(3)]);
    h
}

/// Head bone (8) position of a ped, from its animation hierarchy.
fn head_position(ped: u32) -> Option<[f32; 3]> {
    let frame_data: u32 = rd(ped + g::PED_BONES + 4 * 8);
    if frame_data == 0 {
        return None;
    }
    let node_id: i32 = rd(frame_data + 0x14);
    let clump: u32 = rd(ped + g::ENTITY_RW_OBJECT);
    if clump == 0 {
        return None;
    }
    // SAFETY: GetAnimHierarchyFromSkinClump / RpHAnimIDGetIndex (cdecl) on the ped's live clump.
    let hier = unsafe {
        let f: extern "C" fn(u32) -> u32 = std::mem::transmute(g::GET_ANIM_HIERARCHY_FROM_SKIN_CLUMP as usize);
        f(clump)
    };
    if hier == 0 {
        return None;
    }
    let idx = unsafe {
        let f: extern "C" fn(u32, i32) -> i32 = std::mem::transmute(g::RP_HANIM_ID_GET_INDEX as usize);
        f(hier, node_id)
    };
    let mats: u32 = rd(hier + 8);
    (idx >= 0 && mats != 0).then(|| rd_v3(mats + 0x40 * idx as u32 + 0x30))
}
