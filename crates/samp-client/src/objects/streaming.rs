//! samp's own RW-object streaming for the objects it draws itself.

use super::*;

/// samp's own streaming for objects it draws in its own pass (RW object created within the draw distance,
/// released after `world::RW_RELEASE_FRAMES` frames outside it), or the model request of ordinary objects.
/// Mirrors samp.dll 0x100A95A0.
pub(super) fn stream(runner: &mut Runner, o: &mut Obj, cam: [f32; 3]) {
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
                // DeleteRwObject, model index, SetModelIndex (creates the RW object through the hook of patch
                // table row 67).
                crate::crash::note(crate::crash::crumb::OBJ_RW, o.id, o.model as u32);
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
                // Clear the model's "pre-rendered" flag.
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
        // Only while the local ped is in a vehicle (ped flags +0x46C bit 8).
        runner.run(crate::script::op::REQUEST_MODEL, &[Arg::Int(o.model)]);
    }
}

fn local_in_vehicle() -> bool {
    let ped: u32 = rd(g::CWORLD_PLAYERS);
    ped != 0 && rd::<u32>(ped + crate::game::off::PED_FLAGS) & 0x100 != 0
}
