//! Building removals replayed on IPL loads.

use super::*;

// ---- building removal replay on IPL loads (patch table row 65) -------------------------------------------------

/// `CFileObjectInstance`: position +0, model id +0x1C (gta-reversed FileObjectInstance.h).
extern "C" fn load_object_instance_filter(inst: u32) {
    if inst == 0 {
        return;
    }
    let model: i32 = rd(inst + 0x1C);
    let pos = v3(inst);
    let hit =
        try_shared(|s| s.removals.iter().any(|r| world::removal_matches(r.model, r.centre, r.radius, model, pos)))
            .unwrap_or(false);
    if hit {
        wr(inst + 0x1C, world::REMOVED_BUILDING_MODEL);
    }
}

/// `CFileLoader::LoadObjectInstance(CFileObjectInstance*, const char*)` entry: replays the removals, then runs the
/// overwritten `push -1; push 0x83C931` and continues at 0x538097. Mirrors samp.dll 0x100A4AB0.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn load_object_instance_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x24]",
        "call {f}",
        "add esp, 4",
        "popad",
        "push -1",
        "push 0x83C931",
        "push {cont}",
        "ret",
        f = sym load_object_instance_filter,
        cont = const 0x538097u32,
    )
}
