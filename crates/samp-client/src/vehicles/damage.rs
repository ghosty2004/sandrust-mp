//! Vehicle damage: the panel / door / light / tyre setters used by stream-in and the damage RPCs, and the
//! damage status the local driver reports.

use super::*;

/// Damage setter (panels, doors, lights; all zero = fix): automobiles and planes only. Mirrors samp.dll 0x100B7B00.
pub(super) fn apply_damage(ptr: u32, kind: Kind, panels: u32, doors: u32, lights: u8) {
    if !matches!(kind, Kind::Automobile | Kind::Plane) {
        return;
    }
    if panels == 0 && doors == 0 && lights == 0 {
        let has = rd::<u32>(ptr + g::DMG_PANELS) != 0
            || rd::<u32>(ptr + g::DMG_DOORS) != 0
            || rd::<u8>(ptr + g::DMG_LIGHTS) != 0;
        if has {
            thiscall0(g::AUTOMOBILE_FIX, ptr);
        }
        return;
    }
    poke(ptr + g::DMG_PANELS, panels);
    poke(ptr + g::DMG_DOORS, doors);
    poke(ptr + g::DMG_LIGHTS, u32::from(lights));
    thiscall0(g::AUTOMOBILE_SETUP_DAMAGE, ptr);
}

/// Tyre setter: four bits for automobiles, two for bikes. Mirrors samp.dll 0x100B7980.
pub(super) fn apply_tyres(ptr: u32, kind: Kind, tyres: u8) {
    match kind {
        Kind::Automobile => {
            // Bit 0 is the last wheel-status byte, bit 3 the first.
            let [w0, w1, w2, w3] = g::DMG_WHEEL_STATUS;
            for (bit, off) in [(0, w3), (1, w2), (2, w1), (3, w0)] {
                poke(ptr + off, u8::from(tyres & (1 << bit) != 0));
            }
        }
        Kind::Bike => {
            poke(ptr + g::BIKE_TYRE_REAR, u8::from(tyres & 1 != 0));
            poke(ptr + g::BIKE_TYRE_FRONT, u8::from(tyres & 2 != 0));
        }
        _ => {}
    }
}

pub(super) fn damage_of(ptr: u32, kind: Kind) -> vm::DamageStatus {
    let auto = matches!(kind, Kind::Automobile | Kind::Plane);
    vm::DamageStatus {
        panels: if auto { rd(ptr + g::DMG_PANELS) } else { 0 },
        doors: if auto { rd(ptr + g::DMG_DOORS) } else { 0 },
        lights: if kind == Kind::Automobile { rd(ptr + g::DMG_LIGHTS) } else { 0 },
        tyres: match kind {
            Kind::Automobile => vm::car_tyre_bits(g::DMG_WHEEL_STATUS.map(|o| rd(ptr + o))),
            Kind::Bike => {
                (u8::from(rd::<u8>(ptr + g::BIKE_TYRE_FRONT) != 0) << 1)
                    | u8::from(rd::<u8>(ptr + g::BIKE_TYRE_REAR) != 0)
            }
            _ => 0,
        },
    }
}
