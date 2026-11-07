//! Typed access to GTA vehicles and peds for the vehicle pool: field reads and writes, matrices, seats, the
//! vehicle pool lookups and the entity calls the pool needs.

use super::*;

pub(super) fn rd<T: Copy + Default>(a: u32) -> T {
    if a < 0x10000 {
        return T::default();
    }
    // SAFETY: callers pass fields of live game objects / GTA globals.
    unsafe { std::ptr::read_unaligned(a as usize as *const T) }
}

pub(super) fn poke<T: Copy>(a: u32, v: T) {
    if a < 0x10000 {
        return;
    }
    // SAFETY: callers pass fields of live game objects.
    unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) };
}

pub(super) fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

pub(super) fn put_v3(a: u32, v: [f32; 3]) {
    poke(a, v[0]);
    poke(a + 4, v[1]);
    poke(a + 8, v[2]);
}

/// Sets or clears `bits` in the byte at `a`.
pub(super) fn set_bits_u8(a: u32, bits: u8, on: bool) {
    let b: u8 = rd(a);
    poke(a, if on { b | bits } else { b & !bits });
}

pub(super) fn matrix(ent: u32) -> u32 {
    rd(ent + off::PLACEABLE_MATRIX)
}

pub(super) fn pos_of(ent: u32) -> Option<[f32; 3]> {
    let m = matrix(ent);
    if m == 0 { None } else { Some(v3(m + off::MATRIX_POS)) }
}

pub(super) fn rows_of(m: u32) -> Rotation {
    Rotation { right: v3(m + g::MATRIX_RIGHT), forward: v3(m + g::MATRIX_FORWARD), up: v3(m + g::MATRIX_UP) }
}

pub(super) fn write_rows(m: u32, r: &Rotation) {
    put_v3(m + g::MATRIX_RIGHT, r.right);
    put_v3(m + g::MATRIX_FORWARD, r.forward);
    put_v3(m + g::MATRIX_UP, r.up);
}

pub(super) fn in_vehicle(ped: u32) -> bool {
    rd::<u32>(ped + off::PED_FLAGS) & g::PED_IN_VEHICLE != 0
}

/// The `CVehicle*` a ped is in (`m_pVehicle`; also set while entering or leaving).
pub(super) fn ped_vehicle(ped: u32) -> u32 {
    rd(ped + g::PED_VEHICLE)
}

/// The driver ped of a vehicle (0 without one).
pub(super) fn driver_of(veh: u32) -> u32 {
    rd(veh + g::VEH_DRIVER)
}

/// Passenger seat `i` (0..8) of a vehicle (0 when empty).
pub(super) fn passenger_at(veh: u32, i: u32) -> u32 {
    rd(veh + g::VEH_PASSENGERS + 4 * i)
}

/// The entity is a deleted `CPlaceable` (its vtable was reset by the destructor).
pub(super) fn is_deleted(ent: u32) -> bool {
    rd::<u32>(ent) == g::VTBL_CPLACEABLE
}

/// samp's entity "remove from world": a live entity (not the deleted-entity `CPlaceable` vtable) that is in the
/// moving list goes through `CWorld::Remove`. Mirrors samp.dll 0x1009F460.
pub(super) fn remove_from_world(ent: u32) {
    if ent == 0 || is_deleted(ent) || rd::<u32>(ent + g::MOVING_LIST) == 0 {
        return;
    }
    cworld_remove(ent);
}

/// `CWorld::Remove(CEntity*)`.
pub(super) fn cworld_remove(ent: u32) {
    // SAFETY: `CWorld::Remove(CEntity*)` (cdecl) on a live entity, main thread.
    unsafe {
        let f: extern "C" fn(u32) = std::mem::transmute(g::CWORLD_REMOVE as usize);
        f(ent);
    }
}

/// A ped's seat for samp's ped helpers (in-vehicle flag, `m_pVehicle`, its driver and model).
pub(super) fn seat_of(ped: u32) -> Option<vm::Seat> {
    if ped == 0 || !in_vehicle(ped) {
        return None;
    }
    let veh = ped_vehicle(ped);
    if veh == 0 {
        return None;
    }
    Some(vm::Seat { vehicle: veh, is_driver: driver_of(veh) == ped, model: model_of_entity(veh) })
}

/// `CEntity::m_nModelIndex`.
pub(super) fn model_of_entity(ent: u32) -> u32 {
    u32::from(rd::<u16>(ent + off::ENTITY_MODEL))
}

pub(super) fn local_ped() -> u32 {
    rd(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED)
}

/// The script handle of a ped (`None` when it has none).
pub(super) fn ped_handle(ped: u32) -> Option<i32> {
    game::ped_handle(&ProcessMemory, ped).ok().flatten()
}

/// Current weapon type of a ped (`m_aWeapons[m_nActiveWeaponSlot].m_eWeaponType`).
pub fn current_weapon(ped: u32) -> u32 {
    let slot = u32::from(rd::<u8>(ped + off::PED_SELECTED_WEAPON_SLOT).min(12));
    rd(ped + off::PED_WEAPONS + slot * off::WEAPON_SIZE)
}

pub(super) fn vehicle_at(handle: i32) -> u32 {
    let pool: u32 = rd(g::VEHICLE_POOL);
    if pool == 0 || handle < 0 {
        return 0;
    }
    // SAFETY: GTA's own pool lookup (thiscall) on the live vehicle pool.
    unsafe {
        let f: extern "thiscall" fn(u32, i32) -> u32 = std::mem::transmute(g::POOL_GET_AT as usize);
        f(pool, handle)
    }
}

/// `CPool<CVehicle>::GetRef` for a vehicle pointer: its script handle, or −1 without a pool.
pub(crate) fn pool_ref(ptr: u32) -> i32 {
    let pool: u32 = rd(g::VEHICLE_POOL);
    if pool == 0 {
        return -1;
    }
    // SAFETY: GTA's own pool function (thiscall) on the live vehicle pool.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(g::POOL_GET_REF as usize);
        f(pool, ptr)
    }
}

pub(super) fn thiscall0(f: u32, this: u32) {
    // SAFETY: GTA thiscall without arguments on a live object.
    unsafe {
        let f: extern "thiscall" fn(u32) = std::mem::transmute(f as usize);
        f(this);
    }
}

pub(super) fn kind_of(veh: u32) -> Kind {
    vm::kind_from_vtable(rd(veh))
}

/// Entity teleport virtual (vtable +0x38: x, y, z, false).
pub(super) fn teleport(ent: u32, p: [f32; 3]) {
    let vt: u32 = rd(ent);
    if vt == g::VTBL_CPLACEABLE {
        return;
    }
    let target: u32 = rd(vt + off::VT_TELEPORT);
    // SAFETY: the entity's own virtual (thiscall, CVector by value, bool).
    unsafe {
        let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
        f(ent, p[0], p[1], p[2], 0);
    }
}

pub(super) fn is_passenger(veh: u32, ped: u32) -> bool {
    (0..8u32).any(|i| passenger_at(veh, i) == ped)
}

/// Ped state DIE / DEAD or no health.
pub(super) fn dead(ped: u32) -> bool {
    let st: u32 = rd(ped + off::PED_STATE);
    st == PEDSTATE_DIE || st == PEDSTATE_DEAD || rd::<f32>(ped + off::PED_HEALTH) <= 0.0
}
