//! Objects inside the game (Windows only, milestone M6): the samp object pool (RPC 44/45/46/47/99/122/75/84/169),
//! player-attached objects (RPC 113), the select/edit RPC flows (27/28/116/117) and RemoveBuildingForPlayer (43).
//!
//! Specs: world-entities.md (layouts; RE-10 §2 streaming and render passes, §3 per-frame process and MoveObject,
//! §4 materials, §5 attached objects, §7 building removal, §10 sync rotation), the disassembly of samp A cited per
//! function, and TRACE-6 (trace-notes "World entities"): CREATE_OBJECT + SET_OBJECT_ROTATION in the RPC frame, RW
//! object 1–3 frames later, flags bit 31, RPC 44 re-send → DELETE_OBJECT + CREATE_OBJECT, 1000-frame RW release,
//! MoveObject speed × 0.02 with arrival clamp and slerp, StopObject in place, attached objects re-placed every frame.
//!
//! Material text: `ui::mattext` (render target per slot, SetTexture substitution while the object renders).
//! Not reproduced (docs/phase3/m6-notes.md): the "blanktex"
//! texture of model-0 material records, the object editor gizmo and the mouse picking of selection mode (MR-6.11),
//! the camera-collision row 81 (MR-6.12).

use std::collections::BTreeMap;
use std::time::Instant;

use samp_proto::rpcs::OutgoingRpc;
use samp_proto::rpcs::world::{self as rw, ObjectMaterial};
use samp_proto::types::Vec3;

use crate::footsync::{Rotation, quat_from_matrix};
use crate::netclient::NetClient;
use crate::players::Players;
use crate::remote::{normalise, rows_from_quat, slerp};
use crate::script::{Arg, Runner};
use crate::vehicles::Vehicles;
use crate::world::{self, MATERIAL_SLOTS, Motion, MotionStep, op};
use crate::worldhooks::{self as wh, MatSlot, ObjInfo, g, rd, wr};

/// Atomic-type model-info vtables (A 0x100B4520: CAtomicModelInfo and its derived classes).
const ATOMIC_MI_VTABLES: [u32; 5] = [0x85BBF0, 0x85BC30, 0x85BC70, 0x85BCB0, 0x85BCF0];

fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

fn vec(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn send<P: OutgoingRpc>(net: &mut NetClient, p: &P) {
    net.send_rpc(p, P::RELIABILITY);
}

/// `CPool<CObject>::GetAt(handle)` without the GTA call: index = handle >> 8, slot byte must match.
pub fn object_at(handle: i32) -> u32 {
    let pool: u32 = rd(g::OBJECT_POOL);
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
    objects + index * g::OBJECT_SIZE
}

fn is_atomic_mi(mi: u32) -> bool {
    mi != 0 && ATOMIC_MI_VTABLES.contains(&rd::<u32>(mi))
}

fn model_flags(model: i32) -> u16 {
    let mi = wh::model_info(model);
    if mi != 0 { rd(mi + g::MI_FLAGS) } else { 0 }
}

/// Matrix rows of an entity.
fn rows(ent: u32) -> Option<(Rotation, [f32; 3])> {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    (m != 0).then(|| (Rotation { right: v3(m), forward: v3(m + 0x10), up: v3(m + 0x20) }, v3(m + 0x30)))
}

/// Matrix write: Remove, rows + position, UpdateRW, Add (samp A 0x1009FD60).
fn write_matrix(ent: u32, r: &Rotation, pos: [f32; 3]) {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    if m == 0 {
        return;
    }
    vcall(ent, 0x0C);
    for (i, row) in [r.right, r.forward, r.up].iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            wr(m + 0x10 * i as u32 + 4 * j as u32, *v);
        }
    }
    for (j, v) in pos.iter().enumerate() {
        wr(m + 0x30 + 4 * j as u32, *v);
    }
    wh::update_rw(ent);
    vcall(ent, 0x08);
}

fn vcall(ent: u32, slot: u32) {
    let vt: u32 = rd(ent);
    let f: u32 = rd(vt + slot);
    if f == 0 {
        return;
    }
    // SAFETY: the entity's own argument-less virtual (thiscall).
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(f as usize);
        f(ent);
    }
}

/// A material slot of the object (+0x217 texture, +0x257 colour, text slots +0x110B/+0x114B).
#[derive(Clone, Debug, Default)]
struct Material {
    texture: u32,
    colour: u32,
    /// Material text (kind 2), rendered by `ui::mattext`.
    text: Option<crate::ui::mattext::TextSpec>,
}

#[derive(Clone, Debug)]
struct Obj {
    id: u16,
    model: i32,
    handle: i32,
    ptr: u32,
    /// samp wrapper +0x54.
    draw_distance: f32,
    rotation: [f32; 3],
    attach_vehicle: Option<([f32; 3], [f32; 3], u16)>,
    attach_object: Option<([f32; 3], [f32; 3], u16, u8)>,
    no_camera_collision: bool,
    materials: Vec<Material>,
    motion: Option<Motion>,
    /// +0x49 / +0x4A.
    drawn_by_samp: bool,
    out_frames: u32,
    radius: f32,
    cam_dist: f32,
}

impl Obj {
    fn has_materials(&self) -> bool {
        self.materials.iter().any(|m| m.texture != 0 || m.colour != 0 || m.text.is_some())
    }

    fn info(&self) -> ObjInfo {
        let centre = rows(self.ptr).map(|(_, p)| p).unwrap_or_default();
        let mut materials = [MatSlot::default(); MATERIAL_SLOTS];
        for (i, m) in self.materials.iter().enumerate().take(MATERIAL_SLOTS) {
            materials[i] = MatSlot { texture: m.texture, colour: m.colour };
        }
        ObjInfo {
            ptr: self.ptr,
            model: self.model,
            draw_distance: self.draw_distance,
            has_materials: self.has_materials(),
            has_text: self.materials.iter().any(|m| m.text.is_some()),
            drawn_by_samp: self.drawn_by_samp,
            draw_last: model_flags(self.model) & 0x2 != 0,
            centre,
            radius: self.radius,
            cam_dist: self.cam_dist,
            materials,
            keep_rotation: match self.attach_object {
                Some((_, _, _, 0)) => Some(self.rotation),
                _ => None,
            },
        }
    }
}

/// One player-attached object slot (RPC 113).
#[derive(Clone, Debug)]
struct AttachedSlot {
    data: rw::AttachedObjectData,
    handle: i32,
    ptr: u32,
}

/// The object editor / selection state (A globals 0x1026e9d8 / 0x1026e9dc).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Edit {
    #[default]
    None,
    Selecting,
    Object {
        player_object: bool,
        object_id: u16,
    },
    Attached {
        index: u32,
    },
}

#[derive(Default)]
pub struct Objects {
    pool: BTreeMap<u16, Obj>,
    /// (player id or local, slot) → attached object.
    attached: BTreeMap<(u16, u32), AttachedSlot>,
    removals: u32,
    edit: Edit,
    last_pass: Option<Instant>,
    epoch: Option<Instant>,
    local_id: Option<u16>,
    esc_down: bool,
    pub sent_edit: u32,
}

impl Objects {
    fn now_ms(&mut self) -> u32 {
        let e = *self.epoch.get_or_insert_with(Instant::now);
        e.elapsed().as_millis() as u32
    }

    pub fn set_local_id(&mut self, id: u16) {
        self.local_id = Some(id);
        wh::with_shared(|s| s.active = true);
    }

    // ---- RPCs --------------------------------------------------------------------------------------------

    pub fn on_rpc(&mut self, runner: &mut Runner, net: &mut NetClient, players: &Players, r: rw::Incoming) {
        use rw::Incoming as W;
        match r {
            W::CreateObject(c) => self.create(runner, c),
            W::SetObjectPosition(p) => {
                if let Some(o) = self.pool.get(&p.object_id).filter(|o| o.ptr != 0) {
                    teleport(o.ptr, vec(p.position));
                }
            }
            W::SetObjectRotation(p) => {
                if let Some(o) = self.pool.get_mut(&p.object_id).filter(|o| o.ptr != 0) {
                    let r = vec(p.rotation);
                    runner.run(
                        op::SET_OBJECT_ROTATION,
                        &[Arg::Int(o.handle), Arg::Float(r[0]), Arg::Float(r[1]), Arg::Float(r[2])],
                    );
                    o.rotation = r;
                }
            }
            W::DestroyObject(d) => {
                if let Some(o) = self.pool.remove(&d.object_id) {
                    destroy(runner, &o);
                }
            }
            W::MoveObject(m) => {
                if self.edit == (Edit::Object { player_object: false, object_id: m.object_id })
                    || self.edit == (Edit::Object { player_object: true, object_id: m.object_id })
                {
                    return;
                }
                let now = self.now_ms();
                if let Some(o) = self.pool.get_mut(&m.object_id).filter(|o| o.ptr != 0) {
                    start_move(o, vec(m.target_position), m.speed, vec(m.target_rotation), now);
                }
            }
            W::StopObject(s) => {
                if let Some(o) = self.pool.get_mut(&s.object_id).filter(|o| o.ptr != 0) {
                    stop(o);
                }
            }
            W::AttachObjectToPlayer(a) => {
                let Some(o) = self.pool.get(&a.object_id).filter(|o| o.ptr != 0) else { return };
                let ped = if Some(a.player_id) == self.local_id {
                    rd(g::CWORLD_PLAYERS)
                } else {
                    players.get(a.player_id).map(|p| p.ped).unwrap_or(0)
                };
                let Some(h) = crate::game::ped_handle(&crate::patch::ProcessMemory, ped).ok().flatten() else { return };
                let (off, rot) = (vec(a.offset), vec(a.rotation));
                runner.run(
                    op::ATTACH_OBJECT_TO_CHAR,
                    &[
                        Arg::Int(o.handle),
                        Arg::Int(h),
                        Arg::Float(off[0]),
                        Arg::Float(off[1]),
                        Arg::Float(off[2]),
                        Arg::Float(rot[0]),
                        Arg::Float(rot[1]),
                        Arg::Float(rot[2]),
                    ],
                );
            }
            W::SetPlayerObjectMaterial(m) => {
                if let Some(o) = self.pool.get_mut(&m.object_id).filter(|o| o.ptr != 0) {
                    apply_material(runner, o, &m.material, false);
                }
            }
            W::SetObjectNoCameraCollision(n) => {
                if let Some(o) = self.pool.get_mut(&n.object_id) {
                    o.no_camera_collision = true;
                }
            }
            W::SetPlayerAttachedObject(a) => self.attached_object(runner, players, a),
            W::PlayerBeginObjectSelect(_) => {
                self.stop_editing(net, false);
                self.edit = Edit::Selecting;
                // TODO(input-lock): samp switches the cursor mode here (A 0x1006DB80 → 0x100A0750); wire
                // `ui::inputlock::set_mode(mode, flag)` (UI worktree ui2) once merged.
                crate::log!("objects: selection mode on (mouse picking not implemented, MR-6.11)");
            }
            W::PlayerCancelObjectEdit(_) => match self.edit {
                Edit::Object { .. } | Edit::Attached { .. } => self.stop_editing(net, false),
                Edit::Selecting => self.edit = Edit::None,
                Edit::None => {}
            },
            W::PlayerBeginObjectEdit(e) => {
                // TODO(input-lock): editor start = cursor mode 4 (A 0x10072480 → 0x100A0750); wire
                // `ui::inputlock::set_mode(4, …)` (UI worktree ui2) once merged.
                self.edit = Edit::Object { player_object: e.player_object, object_id: e.object_id };
                crate::log!("objects: editing object {} (gizmo not implemented, MR-6.11)", e.object_id);
            }
            W::PlayerBeginAttachedObjectEdit(e) => {
                self.edit = Edit::Attached { index: e.index };
                crate::log!("objects: editing attached object slot {} (gizmo not implemented, MR-6.11)", e.index);
            }
            _ => {}
        }
        self.publish(players);
    }

    /// 0x10072c20(response): ends editing and sends the final/cancel 117 or 116.
    fn stop_editing(&mut self, net: &mut NetClient, save: bool) {
        // TODO(input-lock): samp restores cursor mode 0 here first (A 0x10072C20 → 0x100A0750(0, 1); the GTA
        // reaction includes SetTaskSecondary(IK) on the local ped). Wire `ui::inputlock::set_mode(0, true)`
        // (UI worktree ui2) once merged; only when an editor/selection was active.
        let response = u32::from(save);
        match self.edit {
            Edit::Object { player_object, object_id } => {
                if let Some(o) = self.pool.get(&object_id).filter(|o| o.ptr != 0)
                    && let Some((_, p)) = rows(o.ptr)
                {
                    let v = |a: [f32; 3]| Vec3 { x: a[0], y: a[1], z: a[2] };
                    send(
                        net,
                        &rw::OnPlayerEditObject {
                            player_object,
                            object_id,
                            response,
                            position: v(p),
                            rotation: v(o.rotation),
                        },
                    );
                    self.sent_edit += 1;
                }
            }
            Edit::Attached { index } => {
                let local = self.local_id.unwrap_or(u16::MAX);
                let data = self.attached.get(&(local, index)).map(|s| s.data);
                send(net, &rw::OnPlayerEditAttachedObject { response, index, data });
                self.sent_edit += 1;
            }
            _ => {}
        }
        self.edit = Edit::None;
    }

    /// RPC 44 (A 0x1001b340 → pool create 0x100128d0 → wrapper 0x100a9150).
    fn create(&mut self, runner: &mut Runner, c: rw::CreateObject) {
        if c.object_id >= world::MAX_OBJECTS {
            return;
        }
        if let Some(old) = self.pool.remove(&c.object_id) {
            destroy(runner, &old);
        }
        let pos = vec(c.position);
        let rot = vec(c.rotation);
        let mut model = c.model;
        if wh::model_info(model) == 0 {
            model = world::MODEL_UNKNOWN;
        }
        let mi = wh::model_info(model);
        if c.draw_distance != 0.0 && is_atomic_mi(mi) {
            wr(mi + g::MI_DRAW_DISTANCE, c.draw_distance);
        }
        runner.run(
            op::CREATE_OBJECT,
            &[Arg::Int(model), Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(pos[2]), Arg::Local(0)],
        );
        let handle = runner.local_i32(0);
        let ptr = object_at(handle);
        let mut o = Obj {
            id: c.object_id,
            model,
            handle,
            ptr,
            draw_distance: c.draw_distance,
            rotation: rot,
            attach_vehicle: None,
            attach_object: None,
            no_camera_collision: false,
            materials: vec![Material::default(); MATERIAL_SLOTS],
            motion: None,
            drawn_by_samp: false,
            out_frames: 0,
            radius: 0.0,
            cam_dist: 0.0,
        };
        if ptr != 0 {
            o.draw_distance = if is_atomic_mi(mi) { rd(mi + g::MI_DRAW_DISTANCE) } else { c.draw_distance };
            // Exact position into the matrix (A 0x1009EC20), then the rotation (A 0x100A80E0 → SCM 0x0453).
            let m: u32 = rd(ptr + g::ENTITY_MATRIX);
            if m != 0 {
                wr(m + 0x30, pos[0]);
                wr(m + 0x34, pos[1]);
                wr(m + 0x38, pos[2]);
            }
            runner.run(
                op::SET_OBJECT_ROTATION,
                &[Arg::Int(handle), Arg::Float(rot[0]), Arg::Float(rot[1]), Arg::Float(rot[2])],
            );
            wr(ptr + g::ENTITY_FLAGS, rd::<u32>(ptr + g::ENTITY_FLAGS) | wh::ENTITY_FLAG_CLIENT_OBJECT);
            // Bounding radius of the collision model (A 0x100B63A0: colmodel +0x24).
            let col: u32 = rd(mi + g::MI_COL_MODEL);
            o.radius = if col != 0 { rd(col + 0x24) } else { 0.0 };
            if c.attached_vehicle_id != rw::INVALID_ID {
                o.attach_vehicle = Some((vec(c.attach_offset), vec(c.attach_rotation), c.attached_vehicle_id));
            } else if c.attached_object_id != rw::INVALID_ID {
                o.attach_object =
                    Some((vec(c.attach_offset), vec(c.attach_rotation), c.attached_object_id, c.sync_rotation));
            }
            for m in &c.materials {
                apply_material(runner, &mut o, m, true);
            }
            if c.no_camera_collision != 0 {
                o.no_camera_collision = true;
            }
        } else {
            crate::log!("objects: CREATE_OBJECT for id {} model {model} gave no object (handle {handle})", c.object_id);
        }
        self.pool.insert(c.object_id, o);
    }

    /// RPC 113 (A 0x10018f00 → 0x100aee90 / 0x100b0b50).
    fn attached_object(&mut self, runner: &mut Runner, players: &Players, a: rw::SetPlayerAttachedObject) {
        if a.index > 9 {
            return;
        }
        let key = (a.player_id, a.index);
        let ped = if Some(a.player_id) == self.local_id {
            rd(g::CWORLD_PLAYERS)
        } else {
            match players.get(a.player_id) {
                Some(p) if a.player_id <= 1003 => p.ped,
                _ => return,
            }
        };
        if let Some(old) = self.attached.remove(&key) {
            delete_object(runner, old.handle, old.ptr, old.data.model);
        }
        let Some(d) = a.data else { return };
        if ped == 0 || rd::<u32>(ped) == g::VTBL_PLACEABLE || !(1..=18).contains(&d.bone) {
            return;
        }
        let mut model = d.model;
        if wh::model_info(model) == 0 {
            model = world::MODEL_UNKNOWN;
        }
        let pos = rows(ped).map(|(_, p)| p).unwrap_or_default();
        let mi = wh::model_info(model);
        if is_atomic_mi(mi) {
            wr(mi + g::MI_DRAW_DISTANCE, 200.0f32);
        }
        runner.run(
            op::CREATE_OBJECT,
            &[Arg::Int(model), Arg::Float(pos[0]), Arg::Float(pos[1]), Arg::Float(pos[2]), Arg::Local(0)],
        );
        let handle = runner.local_i32(0);
        let ptr = object_at(handle);
        if ptr != 0 {
            let r = vec(d.rotation);
            runner.run(
                op::SET_OBJECT_ROTATION,
                &[Arg::Int(handle), Arg::Float(r[0]), Arg::Float(r[1]), Arg::Float(r[2])],
            );
            wr(ptr + g::ENTITY_FLAGS, rd::<u32>(ptr + g::ENTITY_FLAGS) | wh::ENTITY_FLAG_CLIENT_OBJECT);
        }
        self.attached.insert(key, AttachedSlot { data: d, handle, ptr });
    }

    /// RPC 43 (A 0x1009D1E0): occluders within 500 units zeroed; matching buildings, dummies and objects get
    /// area 1 and sink 2000 units; later IPL instances are replaced (row 65).
    pub fn remove_building(&mut self, model: i32, centre: [f32; 3], radius: f32) {
        self.removals += 1;
        wh::with_shared(|s| s.removals.push(wh::Removal { model, centre, radius }));
        // Occluders: 0x12-byte records, centre = 3 × i16 / 4 (gta-reversed Occluder.h).
        let n: u32 = rd(0xC73F98u32);
        for i in 0..n.min(1000) {
            let o = 0xC73FA0 + 0x12 * i;
            let c = [rd::<i16>(o), rd::<i16>(o + 2), rd::<i16>(o + 4)].map(|v| f32::from(v) / 4.0);
            if world::dist3(c, centre) < world::REMOVAL_OCCLUDER_RANGE {
                for b in 0..0x12 {
                    wr(o + b, 0u8);
                }
            }
        }
        for (pool_ptr, size) in [(0xB74498u32, 0x38u32), (0xB744A0, 0x38), (g::OBJECT_POOL, g::OBJECT_SIZE)] {
            let pool: u32 = rd(pool_ptr);
            if pool == 0 {
                continue;
            }
            let objects: u32 = rd(pool);
            let flags: u32 = rd(pool + 4);
            let count: i32 = rd(pool + 8);
            for i in 0..count.max(0) as u32 {
                if rd::<u8>(flags + i) & 0x80 != 0 {
                    continue;
                }
                let e = objects + i * size;
                let em = i32::from(rd::<u16>(e + g::ENTITY_MODEL));
                let m: u32 = rd(e + g::ENTITY_MATRIX);
                let pos = if m != 0 { v3(m + 0x30) } else { v3(e + 4) };
                if !world::removal_matches(model, centre, radius, em, pos) {
                    continue;
                }
                wr(e + 0x2F, 1u8);
                wr(e + 0xC, rd::<f32>(e + 0xC) - world::REMOVAL_SINK);
                if m != 0 {
                    wr(m + 0x38, rd::<f32>(m + 0x38) - world::REMOVAL_SINK);
                }
            }
        }
    }

    // ---- per frame (CGame::Process pre-step, A 0x10012A50 → 0x100A9690) ---------------------------------------

    pub fn process(&mut self, runner: &mut Runner, vehicles: &Vehicles, players: &Players) {
        let now = Instant::now();
        let dt = self.last_pass.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0);
        self.last_pass = Some(now);
        let now_ms = self.now_ms();
        let cam = v3(g::CAMERA_POS);
        let handles: BTreeMap<u16, i32> =
            self.pool.iter().filter(|(_, o)| o.ptr != 0).map(|(k, o)| (*k, o.handle)).collect();
        for o in self.pool.values_mut() {
            if o.ptr == 0 {
                continue;
            }
            stream(runner, o, cam);
            if let Some((off, rot, vid)) = o.attach_vehicle {
                if let Some(vh) = vehicles.script_handle(vid)
                    && !runner.run(op::IS_OBJECT_ATTACHED, &[Arg::Int(o.handle)])
                {
                    attach(runner, op::ATTACH_OBJECT_TO_CAR, o.handle, vh, off, rot);
                }
                continue;
            }
            if let Some((off, rot, oid, _)) = o.attach_object {
                if let Some(&oh) = handles.get(&oid)
                    && !runner.run(op::IS_OBJECT_ATTACHED, &[Arg::Int(o.handle)])
                {
                    attach(runner, op::ATTACH_OBJECT_TO_OBJECT, o.handle, oh, off, rot);
                }
                continue;
            }
            if let Some(m) = o.motion {
                move_step(o, &m, dt, now_ms);
            }
        }
        self.publish(players);
    }

    /// Hands the render hooks the current object table (pool objects in id order, attached objects with their
    /// colours, and the ped of every attached slot).
    pub fn publish(&self, players: &Players) {
        let local = rd::<u32>(g::CWORLD_PLAYERS);
        let infos: Vec<ObjInfo> = self.pool.values().filter(|o| o.ptr != 0).map(Obj::info).collect();
        let mut attached_infos = Vec::new();
        let mut slots = Vec::new();
        for (&(pid, _), s) in self.attached.iter().filter(|(_, s)| s.ptr != 0) {
            let mut materials = [MatSlot::default(); MATERIAL_SLOTS];
            materials[0].colour = s.data.colour1;
            materials[1].colour = s.data.colour2;
            attached_infos.push(ObjInfo {
                ptr: s.ptr,
                model: s.data.model,
                draw_distance: 200.0,
                materials,
                ..Default::default()
            });
            let ped = if Some(pid) == self.local_id { local } else { players.get(pid).map(|p| p.ped).unwrap_or(0) };
            slots.push(wh::Attached {
                ped,
                object: s.ptr,
                bone: s.data.bone,
                offset: vec(s.data.offset),
                rotation: vec(s.data.rotation),
                scale: vec(s.data.scale),
            });
        }
        wh::with_shared(|s| {
            s.objects.clear();
            for i in infos.iter().chain(attached_infos.iter()) {
                s.objects.insert(i.ptr, *i);
            }
            s.draw_order = infos.iter().map(|i| i.ptr).collect();
            s.attached = slots;
        });
    }

    pub fn clear(&mut self, runner: &mut Runner) {
        for (_, o) in std::mem::take(&mut self.pool) {
            destroy(runner, &o);
        }
        for (_, s) in std::mem::take(&mut self.attached) {
            delete_object(runner, s.handle, s.ptr, s.data.model);
        }
        self.edit = Edit::None;
        wh::with_shared(|s| {
            s.objects.clear();
            s.draw_order.clear();
            s.attached.clear();
        });
    }

    /// Control-channel lines.
    pub fn status_lines(&self, out: &mut Vec<String>) {
        for o in self.pool.values() {
            let (pos, rot) = match rows(o.ptr).filter(|_| o.ptr != 0) {
                Some((r, p)) => (
                    format!("{:.3},{:.3},{:.3}", p[0], p[1], p[2]),
                    format!("{:.4},{:.4},{:.4}", r.right[0], r.right[1], r.right[2]),
                ),
                None => ("-".into(), "-".into()),
            };
            let flags: u32 = if o.ptr != 0 { rd(o.ptr + g::ENTITY_FLAGS) } else { 0 };
            let rwobj = o.ptr != 0 && rd::<u32>(o.ptr + g::ENTITY_RW_OBJECT) != 0;
            let mats = o.materials.iter().filter(|m| m.texture != 0 || m.colour != 0 || m.text.is_some()).count();
            out.push(format!(
                "object id={} model={} handle={} ptr={:#x} pos={pos} right={rot} dd={:.1} samp_pass={} rw={} flags={flags:#x} \
                 moving={} materials={mats} attached={}",
                o.id,
                o.model,
                o.handle,
                o.ptr,
                o.draw_distance,
                o.drawn_by_samp,
                rwobj,
                o.motion.is_some(),
                match (o.attach_vehicle, o.attach_object) {
                    (Some((_, _, v)), _) => format!("veh:{v}"),
                    (_, Some((_, _, ob, _))) => format!("obj:{ob}"),
                    _ => "-".into(),
                },
            ));
        }
        for (&(pid, idx), s) in &self.attached {
            let pos = rows(s.ptr).filter(|_| s.ptr != 0).map(|(_, p)| format!("{:.3},{:.3},{:.3}", p[0], p[1], p[2]));
            out.push(format!(
                "attached player={pid} slot={idx} model={} bone={} ptr={:#x} pos={}",
                s.data.model,
                s.data.bone,
                s.ptr,
                pos.unwrap_or_else(|| "-".into())
            ));
        }
    }

    /// Per frame: ESC while editing ends the editor with response 0 (TRACE-6: 117 / 116 response 0 with the
    /// current values). The gizmo itself is not implemented (MR-6.11).
    pub fn poll_escape(&mut self, net: &mut NetClient) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        // SAFETY: plain Win32 query.
        let down = (unsafe { GetAsyncKeyState(0x1B) } as u16) & 0x8000 != 0;
        let pressed = down && !self.esc_down;
        self.esc_down = down;
        if pressed && matches!(self.edit, Edit::Object { .. } | Edit::Attached { .. }) {
            self.stop_editing(net, false);
        }
    }

    pub fn header_fields(&self) -> String {
        format!(
            "objects={} attached={} removals={} edit={:?} samp_draws={} filtered={} zone_hook={}/{}/{}",
            self.pool.len(),
            self.attached.len(),
            self.removals,
            self.edit,
            wh::PASS_DRAWS.load(std::sync::atomic::Ordering::Relaxed),
            wh::FILTERED.load(std::sync::atomic::Ordering::Relaxed),
            wh::ZONE_CALLS[0].load(std::sync::atomic::Ordering::Relaxed),
            wh::ZONE_CALLS[1].load(std::sync::atomic::Ordering::Relaxed),
            wh::ZONE_CALLS[2].load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

fn attach(runner: &mut Runner, opcode: u16, obj: i32, target: i32, off: [f32; 3], rot: [f32; 3]) {
    runner.run(
        opcode,
        &[
            Arg::Int(obj),
            Arg::Int(target),
            Arg::Float(off[0]),
            Arg::Float(off[1]),
            Arg::Float(off[2]),
            Arg::Float(rot[0]),
            Arg::Float(rot[1]),
            Arg::Float(rot[2]),
        ],
    );
}

/// Entity teleport virtual (vtable +0x38), skipping the GTA placeholder (A 0x1009F0A0).
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

/// Wrapper destructor (A 0x100A9470): DeleteRwObject, DELETE_OBJECT, release the model if unused, free textures.
fn destroy(runner: &mut Runner, o: &Obj) {
    crate::ui::mattext::unregister(o.ptr);
    wh::with_shared(|s| {
        s.objects.remove(&o.ptr);
        s.draw_order.retain(|p| *p != o.ptr);
    });
    delete_object(runner, o.handle, o.ptr, o.model);
    for m in &o.materials {
        release_texture(m.texture);
    }
}

fn delete_object(runner: &mut Runner, handle: i32, ptr: u32, model: i32) {
    if ptr != 0 && rd::<u32>(ptr) != g::VTBL_PLACEABLE {
        if rd::<u32>(ptr + g::ENTITY_RW_OBJECT) != 0 {
            vcall(ptr, 0x20);
        }
        runner.run(op::DELETE_OBJECT, &[Arg::Int(handle)]);
        if model >= 0 {
            let mi = wh::model_info(model);
            if mi != 0
                && rd::<u16>(mi + g::MI_REF_COUNT) == 0
                && runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(model)])
            {
                runner.run(crate::script::op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
            }
        }
        wh::with_shared(|s| {
            s.objects.remove(&ptr);
        });
    }
}

fn release_texture(tex: u32) {
    if tex != 0 {
        // SAFETY: RwTextureDestroy (cdecl) on a texture we hold a reference to.
        unsafe {
            let f: extern "C" fn(u32) -> u32 = std::mem::transmute(0x7F3820usize);
            f(tex);
        }
    }
}

/// Material record (A 0x100A8570 kind 1, 0x100A86F0 kind 2). `from_create`: the RPC 44 model rule.
fn apply_material(runner: &mut Runner, o: &mut Obj, m: &ObjectMaterial, from_create: bool) {
    match m {
        ObjectMaterial::Texture { slot, model, txd, texture, colour } => {
            let slot = usize::from(*slot);
            if slot >= MATERIAL_SLOTS || !world::material_name_ok(txd) || !world::material_name_ok(texture) {
                return;
            }
            let model = world::material_model(*model, from_create);
            release_texture(o.materials[slot].texture);
            o.materials[slot].texture = 0;
            if model > 0 {
                if crate::client::load_model(runner, model) {
                    o.materials[slot].texture = find_texture(txd, texture);
                }
                // The model is released again when nothing else uses it (A 0x100A864B → 0x100A0A00).
                let mi = wh::model_info(model);
                if mi != 0
                    && rd::<u16>(mi + g::MI_REF_COUNT) == 0
                    && runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(model)])
                {
                    runner.run(crate::script::op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
                }
            } else if model == 0 {
                crate::log!("objects: material slot {slot} of object {} wants the blank texture (not available)", o.id);
            }
            o.materials[slot].colour = *colour;
        }
        ObjectMaterial::Text {
            slot,
            material_size,
            font,
            font_size,
            bold,
            font_colour,
            background_colour,
            alignment,
            text,
        } => {
            let slot = usize::from(*slot);
            if slot >= MATERIAL_SLOTS || !world::material_name_ok(font) || text.is_empty() {
                return;
            }
            o.materials[slot].text = Some(crate::ui::mattext::TextSpec {
                slot,
                size: *material_size,
                font: font.clone(),
                font_size: *font_size,
                bold: *bold != 0,
                font_colour: *font_colour,
                background: *background_colour,
                alignment: *alignment,
                text: text.clone(),
            });
            let specs: Vec<_> = o.materials.iter().filter_map(|m| m.text.clone()).collect();
            crate::ui::mattext::register(o.ptr, specs);
        }
        ObjectMaterial::Unknown { .. } => {}
    }
}

/// `txd/texture` lookup (A 0x100B2CA0): FindTxdSlot(txd), push/set the current TXD, read the texture, pop.
fn find_texture(txd: &[u8], texture: &[u8]) -> u32 {
    let cstr = |b: &[u8]| {
        let mut v = b.to_vec();
        v.push(0);
        v
    };
    let (t, n) = (cstr(txd), cstr(texture));
    // SAFETY: CTxdStore::FindTxdSlot / PushCurrentTxd / SetCurrentTxd / PopCurrentTxd and RwTextureRead (cdecl)
    // with NUL-terminated names that outlive the calls.
    unsafe {
        let find: extern "C" fn(*const u8) -> i32 = std::mem::transmute(0x731850usize);
        let slot = find(t.as_ptr());
        if slot < 0 {
            return 0;
        }
        let push: extern "C" fn() = std::mem::transmute(0x7316A0usize);
        let set: extern "C" fn(i32) = std::mem::transmute(0x7319C0usize);
        let pop: extern "C" fn() = std::mem::transmute(0x7316B0usize);
        let read: extern "C" fn(*const u8, *const u8) -> u32 = std::mem::transmute(0x7F3AC0usize);
        push();
        set(slot);
        let tex = read(n.as_ptr(), std::ptr::null());
        pop();
        tex
    }
}

/// samp's own streaming for its pass (A 0x100A95A0) or the model request of ordinary objects (A 0x100A96C0).
fn stream(runner: &mut Runner, o: &mut Obj, cam: [f32; 3]) {
    let has_rw = rd::<u32>(o.ptr + g::ENTITY_RW_OBJECT) != 0;
    let pos = rows(o.ptr).map(|(_, p)| p).unwrap_or_default();
    let dist = world::dist3(pos, cam);
    if world::drawn_by_samp(o.has_materials(), o.draw_distance, false) {
        o.drawn_by_samp = false;
        o.cam_dist = dist;
        if dist <= o.draw_distance {
            if !has_rw {
                if !runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(o.model)]) {
                    runner.run(crate::script::op::REQUEST_MODEL, &[Arg::Int(o.model)]);
                    return;
                }
                // DeleteRwObject, model index, SetModelIndex (creates the RW object through row 67).
                vcall(o.ptr, 0x20);
                wr(o.ptr + g::ENTITY_MODEL, o.model as u16);
                let vt: u32 = rd(o.ptr);
                // SAFETY: the entity's SetModelIndex virtual (thiscall, model).
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(rd::<u32>(vt + 0x14) as usize);
                    f(o.ptr, o.model as u32);
                }
            }
            if rd::<u32>(o.ptr + g::ENTITY_RW_OBJECT) != 0 {
                o.drawn_by_samp = true;
                o.out_frames = 0;
                // Clear the model's "pre-rendered" flag (A 0x100A8FE0).
                let mi = wh::model_info(o.model);
                if mi != 0 {
                    wr(mi + g::MI_FLAGS, rd::<u16>(mi + g::MI_FLAGS) & !1);
                }
            }
        } else if has_rw {
            o.out_frames += 1;
            if o.out_frames == world::RW_RELEASE_FRAMES {
                vcall(o.ptr, 0x20);
                o.out_frames = 0;
                let mi = wh::model_info(o.model);
                if o.model > 0
                    && mi != 0
                    && rd::<u16>(mi + g::MI_REF_COUNT) == 0
                    && runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(o.model)])
                {
                    runner.run(crate::script::op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(o.model)]);
                }
            }
        }
    } else if !has_rw
        && local_in_vehicle()
        && dist < o.draw_distance - world::REQUEST_MARGIN
        && !runner.run(crate::script::op::HAS_MODEL_LOADED, &[Arg::Int(o.model)])
    {
        // Only while the local ped is in a vehicle (A 0x100A96C9: local ped flags +0x46C bit 8).
        runner.run(crate::script::op::REQUEST_MODEL, &[Arg::Int(o.model)]);
    }
}

fn local_in_vehicle() -> bool {
    let ped: u32 = rd(g::CWORLD_PLAYERS);
    ped != 0 && rd::<u32>(ped + crate::game::off::PED_FLAGS) & 0x100 != 0
}

/// MoveObject setup (A 0x100A9C10).
fn start_move(o: &mut Obj, target: [f32; 3], speed: f32, rot: [f32; 3], now_ms: u32) {
    if let Some(m) = o.motion.take() {
        // A move in progress stops and snaps to its old target (and target rotation).
        stop(o);
        let r = m.rotation.map(world::rotation_rows).or_else(|| rows(o.ptr).map(|(r, _)| r));
        if let Some(r) = r {
            write_matrix(o.ptr, &r, m.target);
        }
        if let Some(r) = m.rotation {
            o.rotation = r;
        }
    }
    let Some((r, pos)) = rows(o.ptr) else { return };
    let rotation = (rot.iter().all(|c| *c > world::NO_ROTATION)).then_some(rot);
    let from = quat_from_matrix(&r);
    let to = rotation.map(|t| quat_from_matrix(&world::rotation_rows(t))).unwrap_or(from);
    o.motion = Some(Motion { target, speed, start_ms: now_ms, total: world::dist3(target, pos), rotation, from, to });
}

/// StopObject (A 0x100A8070): move and turn speed 0, moving flag off; the object stays where it is.
fn stop(o: &mut Obj) {
    o.motion = None;
    for i in 0..3 {
        wr(o.ptr + g::MOVE_SPEED + 4 * i, 0.0f32);
        wr(o.ptr + g::TURN_SPEED + 4 * i, 0.0f32);
    }
}

fn move_step(o: &mut Obj, m: &Motion, dt: f32, now_ms: u32) {
    let Some((r, pos)) = rows(o.ptr) else { return };
    match m.step(pos, dt, now_ms) {
        MotionStep::Arrive { pos, rotation } => {
            for i in 0..3 {
                wr(o.ptr + g::MOVE_SPEED + 4 * i, 0.0f32);
                wr(o.ptr + g::TURN_SPEED + 4 * i, 0.0f32);
            }
            let rr = rotation.unwrap_or(r);
            write_matrix(o.ptr, &rr, pos);
            if let Some(t) = m.rotation {
                o.rotation = t;
            }
            stop(o);
        }
        MotionStep::Move { move_speed, turn_z, progress } => {
            for (i, v) in move_speed.iter().enumerate() {
                wr(o.ptr + g::MOVE_SPEED + 4 * i as u32, *v);
            }
            // samp advances the matrix itself: pos += move speed × CTimer::ms_fTimeStep (A 0x1009FDC0).
            let ts: f32 = rd(g::TIME_STEP);
            let next = [0, 1, 2].map(|i| pos[i] + move_speed[i] * ts);
            let rr = match turn_z {
                Some(tz) => {
                    wr(o.ptr + g::TURN_SPEED, 0.0f32);
                    wr(o.ptr + g::TURN_SPEED + 4, 0.0f32);
                    wr(o.ptr + g::TURN_SPEED + 8, tz);
                    rows_from_quat(normalise(slerp(m.from, m.to, progress.clamp(0.0, 1.0))))
                }
                None => r,
            };
            write_matrix(o.ptr, &rr, next);
        }
        MotionStep::Idle => {}
    }
}
