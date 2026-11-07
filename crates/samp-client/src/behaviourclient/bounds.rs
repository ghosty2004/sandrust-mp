//! World bounds and the drunk effect.

use super::camera::area_visible;
use super::*;

impl Behaviour {
    // ---- world bounds --------------------------------------------------------------------------------------------

    /// On foot, every frame before the 207; only outside interiors (GET_AREA_VISIBLE = 0).
    /// Mirrors samp.dll B 0x10003F40.
    pub fn bounds_on_foot(&mut self, runner: &mut Runner, ped: u32) {
        if area_visible(runner) != 0 {
            return;
        }
        if push_back_entity(ped, &self.bounds) {
            if let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten()
                && !in_vehicle(ped)
            {
                runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
            }
            big_message(runner, bh::BOUNDS_TEXT, bh::BOUNDS_TEXT_MS, bh::BOUNDS_TEXT_STYLE);
        }
    }

    /// Driving: pushes back the pool vehicle the driver sits in. Mirrors samp.dll B 0x10003FB0.
    pub fn bounds_driver(&mut self, runner: &mut Runner, vehicles: &crate::vehicles::Vehicles, ped: u32) {
        let veh: u32 = rd(ped + PED_VEHICLE);
        if is_passenger(ped) || veh == 0 {
            return;
        }
        // samp reads the area first, then needs a pool vehicle.
        if area_visible(runner) != 0 || vehicles.id_of(veh).is_none() {
            return;
        }
        if push_back_entity(veh, &self.bounds) {
            big_message(runner, bh::BOUNDS_TEXT, bh::BOUNDS_TEXT_MS, bh::BOUNDS_TEXT_STYLE);
        }
    }

    // ---- drunk -------------------------------------------------------------------------------------------------

    /// Every alive frame of the local tick. Mirrors samp.dll B 0x100ADE00.
    pub fn drunk_tick(&mut self, runner: &mut Runner, ped: u32) {
        if self.drunk > bh::DRUNK_THRESHOLD {
            let v = bh::drunk_value(self.drunk);
            runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(v)]);
            if ped != 0 && in_vehicle(ped) && !is_passenger(ped) {
                let veh: u32 = rd(ped + PED_VEHICLE);
                if veh != 0 {
                    let tick = tick_count();
                    if self.wobble_tick == 0 || tick.wrapping_sub(self.wobble_tick) > bh::DRUNK_WOBBLE_MS {
                        let w = bh::drunk_wobble(self.rand.next_rand(), self.drunk);
                        if bh::drunk_moving(v3(veh + off::MOVE_SPEED)) {
                            let t: f32 = rd(veh + TURN_SPEED + 8);
                            poke(veh + TURN_SPEED + 8, t + w);
                        }
                        self.wobble_tick = tick_count();
                    }
                }
            }
            self.drunk -= 1;
        } else {
            if self.drunk > 0 {
                self.drunk -= 1;
            }
            runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(0)]);
        }
    }

    /// Not spawned: a positive level is reset to 0 and the routine runs once (drunkenness 0).
    pub fn drunk_not_spawned(&mut self, runner: &mut Runner) {
        if self.drunk > 0 {
            self.drunk = 0;
            self.drunk_tick(runner, 0);
        }
    }
}

/// The world bounds while the local player is spawned, published by the client tick for the
/// `CPlayerPed::ProcessControl` hook ([`bounds_jump`]).
static LOCAL_BOUNDS: std::sync::Mutex<Option<Bounds>> = std::sync::Mutex::new(None);

pub fn publish_bounds(b: Option<Bounds>) {
    *LOCAL_BOUNDS.lock().unwrap_or_else(|e| e.into_inner()) = b;
}

/// samp's local branch of the `CPlayerPed::ProcessControl` hook: while the local player is spawned, outside the
/// bounds (same strict compares as the push-back), in area 0, not in a vehicle and standing (ped flags: bit 8 clear,
/// bit 0 or 1 set), the pad gets a fresh jump press (NewState.Square = 0xFF, OldState.Square = 0) before GTA
/// processes the ped. Runs before the special-action masks, like samp. Measurement: docs/phase3/fixa-notes.md.
/// Mirrors samp.dll B 0x100A2AF9.
pub fn bounds_jump(ped: u32) {
    let Some(b) = *LOCAL_BOUNDS.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    let Some(p) = matrix_pos(ped) else { return };
    if !bh::outside(p, &b) {
        return;
    }
    thread_local! {
        static RUNNER: std::cell::RefCell<Runner> = std::cell::RefCell::new(Runner::new());
    }
    if RUNNER.with(|r| area_visible(&mut r.borrow_mut())) != 0 {
        return;
    }
    let flags: u32 = rd(ped + off::PED_FLAGS);
    if flags & 0x100 != 0 || flags & 0x3 == 0 {
        return;
    }
    use crate::combat::pad;
    poke(game::PAD0 + pad::SQUARE as u32, 0xFFi16);
    poke(game::PAD0 + (pad::OLD_STATE + pad::SQUARE) as u32, 0i16);
}

/// [`bh::push_back`] on a live entity: speed, then the matrix rows written back with z + 0.04.
fn push_back_entity(e: u32, b: &Bounds) -> bool {
    let m: u32 = rd(e + off::PLACEABLE_MATRIX);
    if !entity_live(e) || m == 0 {
        return false;
    }
    let Some(p) = bh::push_back(v3(m + off::MATRIX_POS), v3(e + off::MOVE_SPEED), b) else { return false };
    for (i, s) in p.speed.iter().enumerate() {
        poke(e + off::MOVE_SPEED + 4 * i as u32, *s);
    }
    poke(m + off::MATRIX_POS + 8, p.z);
    true
}
