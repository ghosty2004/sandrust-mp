//! Objects inside the game (Windows only): the samp object pool (RPC 44/45/46/47/99/122/75/84/169),
//! player-attached objects (RPC 113), the select/edit RPC flows (27/28/116/117, state in `objedit`), the pool
//! ray test used for picking, and RemoveBuildingForPlayer (43). Entry points: [`Objects::on_rpc`] and the
//! per-frame [`Objects::process`]. Material text is drawn by `ui::mattext`.
//!
//! Spec: docs/re/subsystems/world-entities.md ("Objects", "Object streaming and the two render paths",
//! "Materials (RPC 44/84) at render time"); observed behaviour of the original in docs/phase3/trace-notes.md
//! ("World entities"). samp.dll addresses are build A unless marked B (full list: docs/re/addresses/objects.md).
//! "Patch table row N" is line N of docs/re/data/gta-patches-r4-annotated.csv (header = line 1).

use std::collections::BTreeMap;
use std::time::Instant;

use samp_proto::rpcs::OutgoingRpc;
use samp_proto::rpcs::world::{self as rw, ObjectMaterial};
use samp_proto::types::Vec3;

use crate::footsync::{Rotation, quat_from_matrix};
use crate::netclient::NetClient;
use crate::objedit;
use crate::players::Players;
use crate::remote::{normalise, rows_from_quat, slerp};
use crate::script::{Arg, Runner};
use crate::vehicles::Vehicles;
use crate::worldents::{self as world, MATERIAL_SLOTS, Motion, MotionStep, op};
use crate::worldhooks::{self as wh, MatSlot, ObjInfo, g, rd, wr};

/// Vtables of `CAtomicModelInfo` and its derived classes (samp's "is atomic model" test checks these).
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

/// Matrix write: Remove, rows + position, UpdateRW, Add. Mirrors samp.dll 0x1009FD60.
pub(crate) fn write_matrix(ent: u32, r: &Rotation, pos: [f32; 3]) {
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

/// A material slot of the object (samp wrapper +0x217 texture, +0x257 colour, text slots +0x110B/+0x114B).
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
    /// samp wrapper +0x49 / +0x4A: the RW object was created by samp's own streaming.
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
            pool_object: true,
            no_camera_collision: self.no_camera_collision,
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

/// What the object editor / selection mode is working on.
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
    /// The editor's matrix copy (samp editor object +0xC3): taken at the button press, its position follows the
    /// translate drag.
    edit_matrix: Option<(Rotation, [f32; 3])>,
    pub sent_edit: u32,
}

impl Objects {
    /// Objects in the pool (the "ObjectSlotsUsed" line of the F5 debug overlay).
    pub fn count(&self) -> u32 {
        self.pool.len() as u32
    }

    fn now_ms(&mut self) -> u32 {
        let e = *self.epoch.get_or_insert_with(Instant::now);
        e.elapsed().as_millis() as u32
    }

    /// GTA object of pool slot `id`, if the slot exists (camera attachment, RPC 81); 0 = no GTA object.
    pub fn gta_ptr(&self, id: u16) -> Option<u32> {
        self.pool.get(&id).map(|o| o.ptr)
    }

    /// Pool id of a GTA object, for the surf probe. Mirrors samp.dll 0x10012A10.
    pub fn id_of_entity(&self, ptr: u32) -> Option<u16> {
        if ptr == 0 {
            return None;
        }
        self.pool.values().find(|o| o.ptr == ptr).map(|o| o.id)
    }

    /// samp's "moving" test of a pool object: attached to an object → that object moves; attached to a
    /// vehicle → yes; otherwise its own move (MoveObject) in progress. Mirrors samp.dll 0x100A8500.
    pub fn moving(&self, id: u16) -> bool {
        let Some(o) = self.pool.get(&id) else { return false };
        if let Some((_, _, oid, _)) = o.attach_object {
            return oid <= 1000 && self.pool.get(&oid).is_some_and(|p| p.ptr != 0 && p.motion.is_some());
        }
        o.attach_vehicle.is_some() || o.motion.is_some()
    }

    pub fn set_local_id(&mut self, id: u16) {
        self.local_id = Some(id);
        wh::with_shared(|s| s.active = true);
    }

    // ---- RPCs --------------------------------------------------------------------------------------------

    /// Object RPCs; every handled RPC ends with the object table handed to the render hooks, except an ignored
    /// move of the edited object and an attach to a player without a ped.
    pub fn on_rpc(&mut self, runner: &mut Runner, net: &mut NetClient, players: &Players, r: rw::Incoming) {
        use rw::Incoming as W;
        match r {
            W::CreateObject(c) => self.create(runner, c),
            W::SetObjectPosition(p) => self.set_object_position(p),
            W::SetObjectRotation(p) => self.set_object_rotation(runner, p),
            W::DestroyObject(d) => self.destroy_object(runner, d),
            W::MoveObject(m) => {
                if self.is_edited(m.object_id) {
                    return;
                }
                self.move_object(m);
            }
            W::StopObject(s) => self.stop_object(s),
            W::AttachObjectToPlayer(a) => {
                if self.attach_to_player(runner, players, a).is_none() {
                    return;
                }
            }
            W::SetPlayerObjectMaterial(m) => self.set_object_material(runner, m),
            W::SetObjectNoCameraCollision(n) => self.set_no_camera_collision(n),
            W::SetPlayerAttachedObject(a) => self.attached_object(runner, players, a),
            W::PlayerBeginObjectSelect(_) => self.begin_select(net),
            W::PlayerCancelObjectEdit(_) => self.cancel_edit(net),
            W::PlayerBeginObjectEdit(e) => self.begin_edit(e),
            W::PlayerBeginAttachedObjectEdit(e) => self.begin_attached_edit(e),
            _ => {}
        }
        self.publish(players);
    }

    // ---- per frame (samp's step before CGame::Process) ---------------------------------------------------------

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
            // Material text render targets are prepared here, outside any scene, as samp does from its
            // CGame::Process wrapper.
            if o.materials.iter().any(|m| m.text.is_some()) {
                crate::ui::mattext::ensure(o.ptr);
            }
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

    /// Created objects as the pool ray test takes them: (id, entity, model).
    pub fn pick_list(&self) -> Vec<(u16, u32, i32)> {
        self.pool.values().filter(|o| o.ptr != 0).map(|o| (o.id, o.ptr, o.model)).collect()
    }

    /// Control-channel lines: one per pool object and one per attached slot.
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

    pub fn header_fields(&self) -> String {
        format!(
            "objects={} attached={} removals={} edit={:?} editor={} selected={} cam={:.2},{:.2},{:.2} samp_draws={} filtered={} zone_hook={}/{}/{}",
            self.pool.len(),
            self.attached.len(),
            self.removals,
            self.edit,
            objedit::with(|sh| format!("{}/{}/{}", u8::from(sh.editor.active), sh.editor.mode, sh.editor.hovered)),
            objedit::with(|sh| sh.selection.hovered),
            rd::<f32>(g::CAMERA_POS),
            rd::<f32>(g::CAMERA_POS + 4),
            rd::<f32>(g::CAMERA_POS + 8),
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

/// Entity teleport virtual (vtable +0x38), skipping a bare `CPlaceable`. Mirrors samp.dll 0x1009F0A0.
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

/// Wrapper destructor: DeleteRwObject, DELETE_OBJECT, release the model if unused, free textures. Mirrors
/// samp.dll 0x100A9470.
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
    wh::forget_material_rw(ptr);
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

mod editor;
mod materials;
mod motion;
mod pick;
mod rpc;
mod streaming;

use materials::release_texture;
use motion::move_step;
pub use pick::{bound_sphere_hit, pick, pick_within};
use streaming::stream;
