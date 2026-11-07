//! Object RPC handlers: create, move, rotate, attach, materials, attached slots and building removal.

use super::materials::apply_material;
use super::motion::{start_move, stop};
use super::*;

impl Objects {
    pub(super) fn set_object_position(&mut self, p: rw::SetObjectPosition) {
        if let Some(o) = self.pool.get(&p.object_id).filter(|o| o.ptr != 0) {
            teleport(o.ptr, vec(p.position));
        }
    }

    pub(super) fn set_object_rotation(&mut self, runner: &mut Runner, p: rw::SetObjectRotation) {
        if let Some(o) = self.pool.get_mut(&p.object_id).filter(|o| o.ptr != 0) {
            let r = vec(p.rotation);
            runner.run(
                op::SET_OBJECT_ROTATION,
                &[Arg::Int(o.handle), Arg::Float(r[0]), Arg::Float(r[1]), Arg::Float(r[2])],
            );
            o.rotation = r;
        }
    }

    pub(super) fn destroy_object(&mut self, runner: &mut Runner, d: rw::DestroyObject) {
        if let Some(o) = self.pool.remove(&d.object_id) {
            crate::crash::note(crate::crash::crumb::OBJ_DESTROY, o.id, o.model as u32);
            destroy(runner, &o);
        }
    }

    /// The object (either kind) is open in the editor; its MoveObject is ignored.
    pub(super) fn is_edited(&self, object_id: u16) -> bool {
        self.edit == (Edit::Object { player_object: false, object_id })
            || self.edit == (Edit::Object { player_object: true, object_id })
    }

    pub(super) fn move_object(&mut self, m: rw::MoveObject) {
        let now = self.now_ms();
        crate::crash::note(crate::crash::crumb::OBJ_MOVE, m.object_id, 0);
        if let Some(o) = self.pool.get_mut(&m.object_id).filter(|o| o.ptr != 0) {
            start_move(o, vec(m.target_position), m.speed, vec(m.target_rotation), now);
        }
    }

    pub(super) fn stop_object(&mut self, s: rw::StopObject) {
        if let Some(o) = self.pool.get_mut(&s.object_id).filter(|o| o.ptr != 0) {
            stop(o);
        }
    }

    /// AttachObjectToPlayer; `None` when the object or the player's ped is missing.
    pub(super) fn attach_to_player(
        &mut self,
        runner: &mut Runner,
        players: &Players,
        a: rw::AttachObjectToPlayer,
    ) -> Option<()> {
        let o = self.pool.get(&a.object_id).filter(|o| o.ptr != 0)?;
        let ped = if Some(a.player_id) == self.local_id {
            rd(g::CWORLD_PLAYERS)
        } else {
            players.get(a.player_id).map(|p| p.ped).unwrap_or(0)
        };
        let h = crate::game::ped_handle(&crate::patch::ProcessMemory, ped).ok().flatten()?;
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
        Some(())
    }

    pub(super) fn set_object_material(&mut self, runner: &mut Runner, m: rw::SetPlayerObjectMaterial) {
        if let Some(o) = self.pool.get_mut(&m.object_id).filter(|o| o.ptr != 0) {
            apply_material(runner, o, &m.material, false);
        }
    }

    pub(super) fn set_no_camera_collision(&mut self, n: rw::SetObjectNoCameraCollision) {
        if let Some(o) = self.pool.get_mut(&n.object_id) {
            o.no_camera_collision = true;
        }
    }

    /// RPC 44 CreateObject: a re-sent id replaces the old object; unknown models fall back to
    /// `world::MODEL_UNKNOWN`. Mirrors samp.dll 0x100A9150 (the object wrapper constructor).
    pub(super) fn create(&mut self, runner: &mut Runner, c: rw::CreateObject) {
        if c.object_id >= world::MAX_OBJECTS {
            return;
        }
        crate::crash::note(crate::crash::crumb::OBJ_CREATE, c.object_id, c.model as u32);
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
            // Exact position into the matrix, then the rotation (SCM opcode 0x0453, SET_OBJECT_ROTATION).
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
            // Bounding radius of the collision model (CColModel +0x24).
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

    /// RPC 113 SetPlayerAttachedObject: slots 0–9, bones 1–18; the old object of the slot is deleted first.
    /// Mirrors samp.dll 0x10018F00.
    pub(super) fn attached_object(&mut self, runner: &mut Runner, players: &Players, a: rw::SetPlayerAttachedObject) {
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

    /// RPC 43 RemoveBuildingForPlayer: occluders within 500 units lose centre and size; matching buildings,
    /// dummies and objects get area 1 and sink 2000 units; later IPL instances are replaced by a hook (patch table
    /// row 65). Mirrors samp.dll 0x1009D1E0.
    pub fn remove_building(&mut self, model: i32, centre: [f32; 3], radius: f32) {
        self.removals += 1;
        wh::with_shared(|s| s.removals.push(wh::Removal { model, centre, radius }));
        // Occluders: 0x12-byte records, centre = 3 × i16 / 4 (gta-reversed Occluder.h). Only centre and size are
        // cleared; the list link at +0x10 must survive (world::OCCLUDER_CLEARED).
        let n: i32 = rd(world::OCCLUDER_COUNT);
        for i in 0..n.clamp(0, 1000) as u32 {
            let o = world::OCCLUDERS + world::OCCLUDER_SIZE * i;
            let c = [rd::<i16>(o), rd::<i16>(o + 2), rd::<i16>(o + 4)].map(|v| f32::from(v) / 4.0);
            if world::dist3(c, centre) < world::REMOVAL_OCCLUDER_RANGE {
                for b in world::OCCLUDER_CLEARED {
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
}
