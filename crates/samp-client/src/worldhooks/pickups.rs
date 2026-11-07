//! Pickup collision reports.

use super::*;

// ---- pickup collision (patch table row 44) ---------------------------------------------------------------------

/// GTA pickup index → samp id; report when live, cooldown 0 and not a dropped weapon. Mirrors samp.dll 0x100A3F80.
extern "C" fn pickup_touched(pickup: u32) {
    let index = pickup.wrapping_sub(world::GTA_PICKUPS) / world::GTA_PICKUP_SIZE;
    try_shared(|s| {
        if !s.active {
            return;
        }
        if let Some(&id) = s.pickups.get(&index) {
            let cd = s.cooldown.entry(id).or_insert(0);
            if *cd == 0 {
                *cd = world::PICKUP_COOLDOWN;
                s.events.push(Event::PickupHit(id));
            }
        }
    });
}

#[unsafe(naked)]
pub(super) unsafe extern "C" fn pickup_update_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push esi",
        "call {f}",
        "add esp, 4",
        "popad",
        "mov al, [esi+0x1C]",
        "cmp al, 6",
        "push {cont}",
        "ret",
        f = sym pickup_touched,
        cont = const 0x4579CBu32,
    )
}
