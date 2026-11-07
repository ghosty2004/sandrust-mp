//! Pickups, actors, checkpoints, gang zones, map icons and 3D text labels (Windows only).
//!
//! [`Entities`] keeps the client's records for these RPCs and turns them into GTA script commands the way
//! samp does; the per-frame part (`tick`) runs checkpoints, the pickup poll and the label placement. Gang zones
//! are drawn by the radar hooks in `worldhooks.rs` from the records kept here. Spec:
//! docs/re/subsystems/world-entities.md; the original's measured behaviour is in docs/phase3/trace-notes.md
//! ("World entities"). samp.dll addresses: docs/re/addresses/entities.md.

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

/// `IS_LINE_OF_SIGHT_CLEAR` (named via gta-reversed eScriptCommands.h; samp's descriptor has the parameter
/// types "ffffffiiiii").
const IS_LINE_OF_SIGHT_CLEAR: u16 = 0x06BD;
/// Camera position samp's label line-of-sight test starts from (a GTA camera object, vector at +0xC).
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

/// The client's pickups, actors, checkpoints, gang zones, map icons and 3D labels, keyed by wire id.
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
            // ---- pickups (create / destroy)
            W::PlayerCreatePickup(p) => self.create_pickup(runner, p),
            W::PlayerDestroyPickup(p) => self.destroy_pickup(runner, p),
            // ---- actors
            W::ShowActorForPlayer(a) => self.show_actor(runner, a),
            W::HideActorForPlayer(h) => self.delete_actor(runner, h.actor_id),
            W::ApplyActorAnimationForPlayer(a) => self.apply_actor_animation(runner, a),
            W::ClearActorAnimationsForPlayer(c) => self.clear_actor_animations(c),
            W::SetActorFacingAngleForPlayer(f) => self.set_actor_facing_angle(f),
            W::SetActorPosForPlayer(p) => self.set_actor_pos(p),
            W::SetActorHealthForPlayer(h) => self.update_actor_health(runner, h),
            // ---- checkpoints (normal / race)
            W::SetCheckpoint(c) => self.set_checkpoint(runner, c),
            W::DisableCheckpoint(_) => self.cp.active = false,
            W::SetRaceCheckpoint(c) => self.set_race_checkpoint(runner, c),
            W::DisableRaceCheckpoint(_) => self.rcp.active = false,
            // ---- gang zones (the wire id is bounds-checked here)
            W::ShowGangZone(z) => Self::show_gang_zone(z),
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
            // ---- map icons
            W::SetPlayerMapIcon(m) => self.set_map_icon(runner, m),
            W::RemovePlayerMapIcon(m) => self.remove_map_icon(runner, m),
            // ---- 3D labels
            W::PlayerShowTextLabel(l) => self.show_label(l),
            W::PlayerHideTextLabel(l) => {
                self.labels.remove(&l.label_id);
            }
            _ => {}
        }
    }

    fn create_pickup(&mut self, runner: &mut Runner, p: rw::PlayerCreatePickup) {
        if !(0..world::MAX_PICKUPS).contains(&p.pickup_id) || self.pickups.len() >= world::MAX_PICKUPS as usize {
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

    fn destroy_pickup(&mut self, runner: &mut Runner, p: rw::PlayerDestroyPickup) {
        if let Some(old) = self.pickups.remove(&p.pickup_id) {
            runner.run(op::REMOVE_PICKUP, &[Arg::Int(old.handle)]);
            wh::with_shared(|s| {
                s.pickups.retain(|_, v| *v != p.pickup_id);
                s.cooldown.remove(&p.pickup_id);
            });
        }
    }

    fn show_actor(&mut self, runner: &mut Runner, a: rw::ShowActorForPlayer) {
        if a.actor_id >= world::MAX_ACTORS || wh::model_info(a.skin as i32) == 0 {
            return;
        }
        if self.actors.contains_key(&a.actor_id) {
            crate::ui::print_debug(&format!("Warning: actor {} was not deleted", a.actor_id));
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
            &[Arg::Int(handle), Arg::Int(pr[0]), Arg::Int(pr[1]), Arg::Int(pr[2]), Arg::Int(pr[3]), Arg::Int(pr[4])],
        );
        self.actors.insert(a.actor_id, Actor { skin: a.skin, handle, ped, invulnerable: a.invulnerable != 0 });
    }

    fn apply_actor_animation(&mut self, runner: &mut Runner, a: rw::ApplyActorAnimationForPlayer) {
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
                    // Gives up without playing, as the original does when the block is not loaded yet.
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

    fn clear_actor_animations(&mut self, c: rw::ClearActorAnimationsForPlayer) {
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

    fn set_actor_facing_angle(&mut self, f: rw::SetActorFacingAngleForPlayer) {
        if let Some(ac) = self.actors.get(&f.actor_id).filter(|a| a.ped != 0) {
            wr(ac.ped + crate::game::off::PED_HEADING_GOAL, world::samp_actor_heading(f.angle));
        }
    }

    fn set_actor_pos(&mut self, p: rw::SetActorPosForPlayer) {
        if let Some(ac) = self.actors.get(&p.actor_id).filter(|a| a.ped != 0) {
            teleport(ac.ped, vec(p.position));
        }
    }

    fn update_actor_health(&mut self, runner: &mut Runner, h: rw::SetActorHealthForPlayer) {
        if let Some(ac) = self.actors.get(&h.actor_id).copied().filter(|a| a.ped != 0) {
            set_actor_health(runner, ac.handle, ac.ped, h.health);
        }
    }

    fn set_checkpoint(&mut self, runner: &mut Runner, c: rw::SetCheckpoint) {
        self.cp.pos = vec(c.position);
        self.cp.size = c.size;
        if self.cp.blip != 0 {
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(self.cp.blip)]);
            self.cp.blip = add_checkpoint_blip(runner, self.cp.pos);
        }
        self.cp.active = true;
    }

    fn set_race_checkpoint(&mut self, runner: &mut Runner, c: rw::SetRaceCheckpoint) {
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

    fn show_gang_zone(z: rw::ShowGangZone) {
        if z.zone_id < world::MAX_GANG_ZONES {
            wh::with_shared(|s| {
                s.zones.insert(z.zone_id, GangZone::new(z.min.x, z.min.y, z.max.x, z.max.y, z.colour));
            });
        }
    }

    fn set_map_icon(&mut self, runner: &mut Runner, m: rw::SetPlayerMapIcon) {
        if m.icon_id >= world::MAX_MAP_ICONS {
            return;
        }
        if let Some(h) = self.icons.remove(&m.icon_id) {
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
        }
        let p = vec(m.position);
        runner.run(
            world::map_icon_opcode(m.style),
            &[Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Int(i32::from(m.icon_type)), Arg::Local(0)],
        );
        let h = runner.local_i32(0);
        if m.icon_type == 0 {
            runner.run(sop::CHANGE_BLIP_COLOUR, &[Arg::Int(h), Arg::Int(m.colour as i32)]);
            runner.run(sop::CHANGE_BLIP_SCALE, &[Arg::Int(h), Arg::Int(world::map_icon_scale(m.colour))]);
        }
        self.icons.insert(m.icon_id, h);
    }

    fn remove_map_icon(&mut self, runner: &mut Runner, m: rw::RemovePlayerMapIcon) {
        if m.icon_id < world::MAX_MAP_ICONS {
            let h = self.icons.remove(&m.icon_id).unwrap_or(0);
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
        }
    }

    fn show_label(&mut self, l: rw::PlayerShowTextLabel) {
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
                    // Stored as received (colour codes are kept for the painter's colour wrapper); a text shorter
                    // than 400 chars with `~k~` gets GTA's key-name expansion, kept if it fits in 399 chars.
                    text: samp_ui::text::expand_key_names_limit(
                        &l.text,
                        &crate::ui::GameExpander,
                        samp_ui::text::LABEL_KEY_EXPANSION_MAX_OUTPUT,
                    ),
                },
            );
        }
    }

    fn delete_actor(&mut self, runner: &mut Runner, id: u16) {
        if let Some(a) = self.actors.remove(&id) {
            // samp's actor wrapper destructor deletes the ped through its own virtual destructor (scalar
            // deleting, flag 1) when the handle is still valid; no script command is used.
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

    /// The streamed actors as (ped, actor id), published every tick to the combat hooks: their damage hook
    /// (combathooks.rs) sends RPC 177 for them, and the 204 packet carries them as the target actor.
    pub fn hook_actors(&self) -> Vec<(u32, u16)> {
        self.actors.iter().filter(|(_, a)| a.ped != 0).map(|(&id, a)| (a.ped, id)).collect()
    }

    /// Hook events (pickup collisions) → RPCs, in the order they happened.
    pub fn hook_events(&mut self, net: &mut NetClient) {
        let events = wh::with_shared(|s| std::mem::take(&mut s.events));
        for e in events {
            match e {
                Event::PickupHit(id) => {
                    send(net, &rw::OnPlayerPickUpPickup { pickup_id: id });
                    self.sent_131 += 1;
                }
            }
        }
    }

    /// Per frame, from the netgame tick: checkpoint tests, the pickup poll (every 7 frames) and the labels.
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

    /// 3D label placement and screen projection for the Present pass. Mirrors samp.dll 0x10001350.
    fn labels_tick(&mut self, runner: &mut Runner, players: &Players, vehicles: &Vehicles) {
        let cam = rd_v3(g::CAMERA_POS);
        let mut out = Vec::new();
        for l in self.labels.values() {
            let pos = if l.player != rw::INVALID_ID {
                // A label attached to the local player is not drawn for that player.
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
                // One test per label and frame, as in the original.
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
        self.clear_pickups(runner);
        self.clear_actors(runner);
        for (_, h) in std::mem::take(&mut self.icons) {
            runner.run(sop::REMOVE_BLIP, &[Arg::Int(h)]);
        }
        self.cp.active = false;
        self.rcp.active = false;
        self.labels.clear();
        wh::with_shared(|s| {
            s.pickups.clear();
            s.cooldown.clear();
            s.zones.clear();
        });
        if let Ok(mut g) = LABEL_DRAWS.lock() {
            g.clear();
        }
    }

    /// The pickup pool's deletion: `REMOVE_PICKUP` for every slot in id order. Mirrors samp.dll 0x10013410.
    pub fn clear_pickups(&mut self, runner: &mut Runner) {
        for (_, p) in std::mem::take(&mut self.pickups) {
            runner.run(op::REMOVE_PICKUP, &[Arg::Int(p.handle)]);
        }
    }

    /// The actor pool's deletion: every actor in id order. Mirrors samp.dll 0x100018E0.
    pub fn clear_actors(&mut self, runner: &mut Runner) {
        let ids: Vec<u16> = self.actors.keys().copied().collect();
        for id in ids {
            self.delete_actor(runner, id);
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

/// RPC 178 and actor creation: sets the CPed health; ≤ 0 → EXPLODE_CHAR_HEAD. Mirrors samp.dll 0x1009C630.
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

/// Checkpoint blip: sprite 0 at the position, colour 1005, scale 3. Mirrors samp.dll 0x100A1F60.
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
    // SAFETY: RpHAnimIDGetIndex (cdecl) on the hierarchy just returned for the live clump.
    let idx = unsafe {
        let f: extern "C" fn(u32, i32) -> i32 = std::mem::transmute(g::RP_HANIM_ID_GET_INDEX as usize);
        f(hier, node_id)
    };
    let mats: u32 = rd(hier + 8);
    (idx >= 0 && mats != 0).then(|| rd_v3(mats + 0x40 * idx as u32 + 0x30))
}
