//! RW object create/delete wrappers, escalator constants, attached-entity rotation and entity matrix writes.

use super::*;

// ---- Create/DeleteRwObject (patch table rows 67/68/69 and 372/373) -------------------------------------------

/// `call [edx+0x20]` in `CStreaming::AddEntity` (patch table row 372).
pub(super) const ADD_ENTITY_CALL: u32 = 0x4096AA;
pub(super) const ADD_ENTITY_CALL_BYTES: [u8; 3] = [0xFF, 0x52, 0x20];
/// `call CEscalator::SwitchOff` in `CEntity::RemoveEscalatorsForEntity` (patch table row 373).
pub(super) const ESCALATOR_SWITCH_OFF_CALL: u32 = 0x71791F;
pub(super) const ESCALATOR_SWITCH_OFF_BYTES: [u8; 5] = [0xE8, 0x3C, 0xFF, 0xFF, 0xFF];
/// Escalator models whose RW-object deletion keeps the escalator running. Mirrors samp.dll 0x100A4BB8.
const ESCALATOR_MODELS: [u16; 4] = [3586, 3743, 8980, 8979];

fn raw_patch(addr: u32, bytes: &[u8]) {
    // SAFETY: the range was made writable at install (unprotect) and is code no other thread executes now.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as usize as *mut u8, bytes.len()) }
}

/// `CEntity::CreateRwObject` with the AddEntity `call [edx+0x20]` NOPed for the call, so creating one RW object
/// does not make the streamer drop another (patch table row 67). Mirrors samp.dll 0x100A4B30.
pub(super) extern "thiscall" fn object_create_rw(this: u32) {
    raw_patch(ADD_ENTITY_CALL, &[0x90; 3]);
    thiscall0(g::ENTITY_CREATE_RW, this);
    raw_patch(ADD_ENTITY_CALL, &ADD_ENTITY_CALL_BYTES);
}

/// Escalator models keep running when their RW object goes (patch table rows 68/69). Mirrors samp.dll 0x100A4BA0.
pub(super) extern "thiscall" fn entity_delete_rw(this: u32) {
    let model: u16 = rd(this + g::ENTITY_MODEL);
    if ESCALATOR_MODELS.contains(&model) {
        raw_patch(ESCALATOR_SWITCH_OFF_CALL, &[0x90; 5]);
        thiscall0(g::ENTITY_DELETE_RW, this);
        raw_patch(ESCALATOR_SWITCH_OFF_CALL, &ESCALATOR_SWITCH_OFF_BYTES);
    } else {
        thiscall0(g::ENTITY_DELETE_RW, this);
    }
}

// ---- escalator speed constants (patch table rows 66 and 371) ---------------------------------------------------

pub(super) const ESCALATOR_FLOATS: [(u32, f32); 2] = [(0x858BA4, 40.0), (0x858F84, 50.0)];

/// `CEscalator::Update` with the two constants raised to 40.0 / 50.0 for the call (patch table row 66).
/// Mirrors samp.dll 0x100A4C40.
pub(super) extern "thiscall" fn escalator_update(this: u32) {
    let saved = ESCALATOR_FLOATS.map(|(a, _)| rd::<u32>(a));
    for (a, v) in ESCALATOR_FLOATS {
        wr(a, v);
    }
    thiscall0(g::ESCALATOR_UPDATE, this);
    for ((a, _), v) in ESCALATOR_FLOATS.iter().zip(saved) {
        wr(*a, v);
    }
}

// ---- attached entity rotation, sync rotation 0 (patch table rows 24 and 75) -----------------------------------

/// `CWorld::ProcessAttachedEntities()` (cdecl).
pub(super) const PROCESS_ATTACHED_ENTITIES: u32 = 0x5647F0;

/// The game's `ProcessAttachedEntities`, then the surf glues (surf.rs; patch table row 24).
/// Mirrors samp.dll 0x100A4990.
pub(super) extern "C" fn process_attached_entities() {
    // SAFETY: GTA's own argument-less cdecl function, called from its only call site's hook.
    unsafe {
        let f: extern "C" fn() = std::mem::transmute(PROCESS_ATTACHED_ENTITIES as usize);
        f();
    }
    crate::surf::after_process_attached_entities(rd::<u32>(g::CWORLD_PLAYERS));
}

/// After GTA positions an attached entity, an object attached to an object with sync rotation 0 gets its own
/// stored rotation back (patch table row 75). Mirrors samp.dll 0x100A4F10.
pub(super) extern "thiscall" fn position_attached_entity(this: u32) -> u32 {
    let r = thiscall0(g::POSITION_ATTACHED_ENTITY, this);
    if rd::<u32>(this + g::ENTITY_FLAGS) & ENTITY_FLAG_CLIENT_OBJECT != 0
        && let Some(Some(rot)) = try_shared(|s| s.objects.get(&this).map(|o| o.keep_rotation)).flatten()
    {
        let m: u32 = rd(this + g::ENTITY_MATRIX);
        if m != 0 {
            let rows = world::rotation_rows(rot);
            for (i, row) in [rows.right, rows.forward, rows.up].iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    wr(m + 0x10 * i as u32 + 4 * j as u32, *v);
                }
            }
            update_rw(this);
        }
    }
    r
}

/// `CMatrix::UpdateRwMatrix` into the RW frame + `CEntity::UpdateRwFrame`. Mirrors samp.dll 0x1009ECE0.
pub fn update_rw(ent: u32) {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    let rw: u32 = rd(ent + g::ENTITY_RW_OBJECT);
    if m == 0 || rw == 0 {
        return;
    }
    let frame: u32 = rd(rw + 4);
    if frame == 0 {
        return;
    }
    // SAFETY: CMatrix::UpdateRwMatrix(RwMatrix*) (thiscall on the entity matrix) and CEntity::UpdateRwFrame.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(g::UPDATE_RW_MATRIX as usize);
        f(m, frame + 0x10);
    }
    thiscall0(g::UPDATE_RW_FRAME, ent);
}

/// Entity matrix position (Remove, write, UpdateRW, Add). Mirrors samp.dll 0x1009FD60.
pub fn set_position(ent: u32, p: [f32; 3]) {
    let m: u32 = rd(ent + g::ENTITY_MATRIX);
    if m == 0 {
        return;
    }
    vcall0(ent, 0x0C);
    wr(m + 0x30, p[0]);
    wr(m + 0x34, p[1]);
    wr(m + 0x38, p[2]);
    update_rw(ent);
    vcall0(ent, 0x08);
}
