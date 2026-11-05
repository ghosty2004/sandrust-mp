//! Held items (special actions 20–23), pissing (68) and the remote parachute object (FEAT-B).
//!
//! Static reading of samp.dll build B (behaviour in our own words; constants with their addresses):
//!
//! * **Hold item** (ped method B 0x100B02E0; RPC 88 local B 0x100030F0, remote pass B 0x10014A78): a held object
//!   replaces any previous one; `CREATE_OBJECT` with the item model at the ped's position, then (when the object
//!   exists) `TASK_PICK_UP_OBJECT(ped, obj, offset, 6, 16, "NULL", "NULL", -1)`. Item kinds 1 beer (1543),
//!   2 wine (1486), 3 sprunk (1546), 4 cigar (1485), from special actions 20 / 22 / 23 / 21.
//! * **Drop** (B 0x100ADC10): `TASK_PICK_UP_OBJECT(ped, obj, 0, 0, 0, 6, 16, "NULL", "NULL", 0)` (the object stays
//!   in the world), the ped re-set to its own position through the `Teleport` virtual, kind 0.
//! * **Use** (B 0x100ADCB0, every frame while an item is held): the ped is armed with fists (set-armed helper), then —
//!   local player only — when no primary task runs and the fire button (pad Circle, +0x22) is down: drinks play
//!   `BAR:dnk_stndM_loop` (or `dnk_stndF_loop`, see [`female_drink`]) delta 4 for the task's own length and raise the
//!   drunk level by 1250 (not for sprunk; capped at 50000); the cigar plays `GANGS:smkcig_prtl` delta 700, 2750 ms.
//! * **Local tick** (B 0x1000772C…0x100077AB): while pissing, the enter/exit key (Triangle) or sitting in a vehicle
//!   ends it; while holding an item, Triangle or a vehicle drops it, otherwise the item is used.
//! * **Piss** (B 0x100AE110): local only `PAULNMAC:PISS_LOOP` delta 4 looped; then
//!   `CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION("PETROLCAN", ped, 0, 0.58, −0.08, 0, 0.01, 0, 1)` and `PLAY_FX_SYSTEM`.
//!   Stop (B 0x100AE1F0): `KILL_FX_SYSTEM`, position self-set.
//! * **Remote parachute** (B 0x100AD630 after a remote PARACHUTE animation, B 0x100AE7E0): only while the ped's current
//!   weapon is 46. The helper object (B 0x100AD4E0) is created once: `CREATE_OBJECT(3131)` at the ped,
//!   `ATTACH_OBJECT_TO_CHAR` with zero offsets, hidden, scale 0, `PLAY_OBJECT_ANIM("PARA_OPEN_O", 1000, 0, 1)`, shown,
//!   scale 1. PARA_OPEN then plays sound 1039 at local + (remote − local) × 0.1; PARA_DECEL / FLOAT / STEERR / STEERL
//!   play their `_O` object anim (speed 1, loop); PARA_LAND_WATER its `_O` anim not looped. Every remote pass ends
//!   with B 0x100AD450: a held parachute object is detached and removed once the ped's weapon is no longer 46; the
//!   ped's destruction (B 0x100B0FB0) does the same and drops a held item (a piss effect is left alone).

/// Item kind for a special action (B 0x100030F0 cases 0x14…0x17).
pub fn item_for_action(action: u8) -> Option<u8> {
    match action {
        20 => Some(1),
        22 => Some(2),
        23 => Some(3),
        21 => Some(4),
        _ => None,
    }
}

/// The special action a held item reports in 207 (B 0x10003410).
pub fn action_for_item(kind: u8) -> u8 {
    match kind {
        1 => 20,
        2 => 22,
        3 => 23,
        4 => 21,
        _ => 0,
    }
}

/// Object model and hand offset per item kind (B 0x100B02E0; doubles B 0x100ED298…0x100ED2C0).
pub fn item_object(kind: u8) -> Option<(i32, [f32; 3])> {
    match kind {
        1 => Some((1543, [0.05, 0.03, -0.3])),
        2 => Some((1486, [0.05, 0.03, -0.05])),
        3 => Some((1546, [0.04, 0.1, -0.02])),
        4 => Some((1485, [0.0, 0.0, 0.0])),
        _ => None,
    }
}

/// Drunk-level raise per drink and its cap (B 0x100ADD8F).
pub const DRINK_DRUNK: i32 = 1250;
pub const DRUNK_CAP: i32 = 50000;

/// New drunk level after a drink of `kind` (sprunk does not count).
pub fn after_drink(level: i32, kind: u8) -> i32 {
    if kind == 3 { level } else { (level + DRINK_DRUNK).min(DRUNK_CAP) }
}

/// samp picks the female drinking anim when its "ped type" value is 5 or 22 (B 0x100ADD71). That value is read
/// by B 0x100B4110 as the 11th entry of the ped model info's *vtable* (`[[info] + 0x28]`, a code address), so it is
/// never 5 or 22 for a ped model and every ped drinks with `dnk_stndM_loop`. Kept literally.
pub fn female_drink(value: u32) -> bool {
    value == 5 || value == 22
}

/// The 207 special action of the local player with these states added (B 0x10003410 order: jetpack 2, dance,
/// hands up 10, phone 11, piss 68, duck 1, enter 3, exit 4, items, cuffs, carry). `base` is what the game state
/// gives (2, 10, 1 or 0).
pub fn local_report(base: u8, phone: bool, piss: bool, item: u8) -> u8 {
    match base {
        2 | 10 => base,
        _ if phone => 11,
        _ if piss => 68,
        1 => 1,
        _ if item != 0 => action_for_item(item),
        b => b,
    }
}

/// Parachute anims with an object anim (B 0x100AD630): the `_O` name and whether it loops.
pub fn para_object_anim(name: &str) -> Option<(&'static str, bool)> {
    Some(match name {
        "PARA_DECEL" => ("PARA_DECEL_O", true),
        "PARA_FLOAT" => ("PARA_FLOAT_O", true),
        "PARA_STEERR" => ("PARA_STEERR_O", true),
        "PARA_STEERL" => ("PARA_STEERL_O", true),
        "PARA_LAND_WATER" => ("PARA_LAND_WATER_O", false),
        _ => return None,
    })
}

/// Sound position for PARA_OPEN: a tenth of the way from the local player to the remote one (B 0x100AD6C5).
pub fn para_sound_pos(local: [f32; 3], remote: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| (remote[i] - local[i]) * 0.1 + local[i])
}

/// Per-ped state samp keeps in its ped object (+0x2B9 parachute object, +0x2BD piss effect, +0x2C1 item kind,
/// +0x2C5 item object, +0x2F2 pissing).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PedExtras {
    pub item: u8,
    pub item_obj: i32,
    pub piss: bool,
    pub piss_fx: i32,
    pub para_obj: i32,
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::game::{self, off};
    use crate::patch::{Memory, ProcessMemory};
    use crate::script::{Arg, Runner, op};
    use std::collections::HashMap;
    use std::sync::Mutex;

    const PARACHUTE_MODEL: i32 = 3131;
    const PARA_OPEN_SOUND: i32 = 1039;
    const WEAPON_PARACHUTE: u8 = 46;
    /// `CPad::NewState.ButtonCircle` (fire on foot).
    const PAD_CIRCLE: u32 = 0x22;
    const PED_IN_VEHICLE: u32 = 1 << 8;

    fn handle(ped: u32) -> Option<i32> {
        game::ped_handle(&ProcessMemory, ped).ok().flatten()
    }

    fn pos(ped: u32) -> Option<[f32; 3]> {
        let m = ProcessMemory;
        let matrix = m.read_u32(ped + off::PLACEABLE_MATRIX).ok().filter(|&x| x != 0)?;
        let p = matrix + off::MATRIX_POS;
        Some([m.read_f32(p).ok()?, m.read_f32(p + 4).ok()?, m.read_f32(p + 8).ok()?])
    }

    fn in_vehicle(ped: u32) -> bool {
        ProcessMemory.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & PED_IN_VEHICLE != 0
    }

    fn pick_up(runner: &mut Runner, h: i32, obj: i32, o: [f32; 3], time: i32) {
        runner.run(
            op::TASK_PICK_UP_OBJECT,
            &[
                Arg::Int(h),
                Arg::Int(obj),
                Arg::Float(o[0]),
                Arg::Float(o[1]),
                Arg::Float(o[2]),
                Arg::Int(6),
                Arg::Int(16),
                Arg::Text("NULL"),
                Arg::Text("NULL"),
                Arg::Int(time),
            ],
        );
    }

    /// Hold item `kind` (B 0x100B02E0).
    pub fn start_item(runner: &mut Runner, ped: u32, ex: &mut PedExtras, kind: u8) {
        if ex.item_obj != 0 {
            drop_item(runner, ped, ex);
        }
        let Some((model, o)) = item_object(kind) else { return };
        let (Some(h), Some(p)) = (handle(ped), pos(ped)) else { return };
        runner.run(op::CREATE_OBJECT, &[Arg::Int(model), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Local(0)]);
        let obj = runner.local_i32(0);
        if crate::objects::object_at(obj) == 0 {
            ex.item_obj = 0;
            return;
        }
        ex.item_obj = obj;
        pick_up(runner, h, obj, o, -1);
        ex.item = kind;
        crate::log!("specials: ped {ped:#x} holds item {kind} (object {obj})");
    }

    /// Drop the held item (B 0x100ADC10).
    pub fn drop_item(runner: &mut Runner, ped: u32, ex: &mut PedExtras) {
        if ex.item_obj != 0 {
            if let Some(h) = handle(ped) {
                pick_up(runner, h, ex.item_obj, [0.0; 3], 0);
            }
            ex.item_obj = 0;
        }
        crate::players::teleport_in_place(ped);
        ex.item = 0;
    }

    /// Fists for the ped (samp's set-armed helper B 0x100AC070 with weapon 0): the local context or the slot's.
    fn arm_fists(runner: &mut Runner, ped: u32, slot: Option<u8>) {
        match slot {
            None => crate::combatclient::set_armed_local(runner, ped, 0),
            Some(s) if !in_vehicle(ped) => crate::players::set_weapon(runner, ped, s, 0),
            Some(_) => {
                // SAFETY: CPed::SetCurrentWeapon(eWeaponType) (thiscall) on a live remote ped, main thread.
                unsafe {
                    let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(0x5E6280usize);
                    f(ped, 0);
                }
            }
        }
    }

    /// The value samp compares for the drink anim (see [`female_drink`]).
    fn drink_type_value(ped: u32) -> u32 {
        let m = ProcessMemory;
        let model = u32::from(m.read_u16(ped + off::ENTITY_MODEL).unwrap_or(0));
        if model >= game::MODEL_INFO_COUNT {
            return 0;
        }
        let info = m.read_u32(game::MODEL_INFO_PTRS + 4 * model).unwrap_or(0);
        let vt = if info != 0 { m.read_u32(info).unwrap_or(0) } else { 0 };
        if vt != game::VTBL_PED_MODEL_INFO {
            return 0;
        }
        m.read_u32(vt + 0x28).unwrap_or(0)
    }

    /// Use the held item (B 0x100ADCB0). `drunk` is the local drunk level (`None` for a remote ped: samp only arms
    /// the fists there). Returns whether an animation started.
    pub fn use_item(runner: &mut Runner, ped: u32, ex: &PedExtras, slot: Option<u8>, drunk: Option<&mut i32>) -> bool {
        if ex.item == 0 || ped == 0 || !crate::client::ped_in_world(ped) {
            return false;
        }
        arm_fists(runner, ped, slot);
        let Some(drunk) = drunk else { return false };
        let m = ProcessMemory;
        let intel = m.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
        if intel != 0 && m.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) != 0 {
            return false;
        }
        if m.read_u16(game::PAD0 + PAD_CIRCLE).unwrap_or(0) == 0 {
            return false;
        }
        match ex.item {
            1..=3 => {
                let name: &[u8] = if female_drink(drink_type_value(ped)) { b"dnk_stndF_loop" } else { b"dnk_stndM_loop" };
                crate::behaviourclient::play(runner, ped, name, b"BAR", 4.0, [false; 4], -1);
                *drunk = after_drink(*drunk, ex.item);
                true
            }
            4 => {
                crate::behaviourclient::play(runner, ped, b"smkcig_prtl", b"GANGS", 700.0, [false; 4], 2750);
                true
            }
            _ => false,
        }
    }

    /// Start pissing (B 0x100AE110); the animation only for the local player.
    pub fn start_piss(runner: &mut Runner, ped: u32, ex: &mut PedExtras, local: bool) {
        let Some(h) = handle(ped) else { return };
        if ex.piss {
            return;
        }
        if local {
            crate::behaviourclient::play(runner, ped, b"PISS_LOOP", b"PAULNMAC", 4.0, [true, false, false, false], -1);
        }
        let f = Arg::Float;
        runner.run(
            op::CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION,
            &[Arg::Text("PETROLCAN"), Arg::Int(h), f(0.0), f(0.58), f(-0.08), f(0.0), f(0.01), f(0.0), Arg::Int(1), Arg::Local(0)],
        );
        ex.piss_fx = runner.local_i32(0);
        runner.run(op::PLAY_FX_SYSTEM, &[Arg::Int(ex.piss_fx)]);
        ex.piss = true;
    }

    /// Stop pissing (B 0x100AE1F0).
    pub fn stop_piss(runner: &mut Runner, ped: u32, ex: &mut PedExtras) {
        if handle(ped).is_none() || !ex.piss {
            return;
        }
        if ex.piss_fx != 0 {
            runner.run(op::KILL_FX_SYSTEM, &[Arg::Int(ex.piss_fx)]);
            ex.piss_fx = 0;
        }
        crate::players::teleport_in_place(ped);
        ex.piss = false;
    }

    /// The local tick's part (B 0x1000772C…0x100077AB).
    pub fn local_tick(runner: &mut Runner, ped: u32, ex: &mut PedExtras, triangle: bool, drunk: &mut i32) {
        let veh = in_vehicle(ped);
        if ex.piss && (triangle || veh) {
            stop_piss(runner, ped, ex);
        }
        if ex.item != 0 {
            if !triangle && !veh {
                use_item(runner, ped, ex, None, Some(drunk));
            } else {
                drop_item(runner, ped, ex);
            }
        }
    }

    /// RPC 88 for the local player, the cases this module owns (B 0x100030F0): 0 stops pissing and drops the
    /// item; 20–23 / 68 start them unless the ped sits in a vehicle.
    pub fn local_action(runner: &mut Runner, ped: u32, ex: &mut PedExtras, action: u8) {
        if ped == 0 {
            return;
        }
        match action {
            0 => {
                if ex.piss {
                    stop_piss(runner, ped, ex);
                }
                if ex.item != 0 {
                    drop_item(runner, ped, ex);
                }
            }
            68 if !in_vehicle(ped) => start_piss(runner, ped, ex, true),
            a => {
                if let Some(kind) = item_for_action(a)
                    && !in_vehicle(ped)
                {
                    start_item(runner, ped, ex, kind);
                }
            }
        }
    }

    // ---- remote players --------------------------------------------------------------------------------------

    static REMOTE: Mutex<Option<HashMap<u16, PedExtras>>> = Mutex::new(None);

    fn with_remote<R>(id: u16, f: impl FnOnce(&mut PedExtras) -> R) -> R {
        let mut g = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(HashMap::new).entry(id).or_default())
    }

    /// The remote special-action pass after hands up (B 0x10014A38…0x10014B39), then the parachute check that ends
    /// the pass (B 0x100AD450).
    pub fn remote_pass(runner: &mut Runner, id: u16, ped: u32, slot: u8, action: u8) {
        let mut ex = with_remote(id, |e| *e);
        if !ex.piss && action == 68 {
            start_piss(runner, ped, &mut ex, false);
        }
        if ex.piss && action != 68 {
            stop_piss(runner, ped, &mut ex);
        }
        for a in [20u8, 22, 23, 21] {
            if let Some(kind) = item_for_action(a)
                && action == a
                && ex.item != kind
            {
                start_item(runner, ped, &mut ex, kind);
            }
        }
        if ex.item != 0 {
            if (20..=23).contains(&action) {
                use_item(runner, ped, &ex, Some(slot), None);
            } else {
                drop_item(runner, ped, &mut ex);
            }
        }
        para_check(runner, ped, &mut ex);
        with_remote(id, |e| *e = ex);
    }

    /// The remote PARACHUTE animation hook (B 0x100AD630), after the animation itself was played.
    pub fn remote_para_anim(runner: &mut Runner, id: u16, ped: u32, name: &str) {
        let mut ex = with_remote(id, |e| *e);
        para_anim(runner, ped, &mut ex, name);
        with_remote(id, |e| *e = ex);
    }

    /// Stream-out / quit of a remote ped (B 0x100B0FB0): parachute object off, item dropped; the state is gone.
    pub fn remote_destroyed(runner: &mut Runner, id: u16, ped: u32) {
        let mut ex = {
            let mut g = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
            g.get_or_insert_with(HashMap::new).remove(&id).unwrap_or_default()
        };
        if ex.para_obj != 0 {
            remove_para(runner, &mut ex);
        }
        if ex.item != 0 {
            drop_item(runner, ped, &mut ex);
        }
    }

    fn remove_para(runner: &mut Runner, ex: &mut PedExtras) {
        let f = Arg::Float(0.0);
        runner.run(op::DETACH_OBJECT, &[Arg::Int(ex.para_obj), f, f, f, Arg::Int(0)]);
        runner.run(op::REMOVE_OBJECT_ELEGANTLY, &[Arg::Int(ex.para_obj)]);
        ex.para_obj = 0;
    }

    fn para_check(runner: &mut Runner, ped: u32, ex: &mut PedExtras) {
        if ex.para_obj == 0 || handle(ped).is_none() || !crate::client::ped_in_world(ped) {
            return;
        }
        if crate::players::current_weapon(runner, ped) != WEAPON_PARACHUTE {
            remove_para(runner, ex);
        }
    }

    /// The parachute helper object (B 0x100AD4E0); `false` when it could not be created.
    fn para_object(runner: &mut Runner, ped: u32, ex: &mut PedExtras) -> bool {
        if ex.para_obj != 0 {
            return true;
        }
        let (Some(h), Some(p)) = (handle(ped), pos(ped)) else { return false };
        let a = [Arg::Int(PARACHUTE_MODEL), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Local(0)];
        runner.run(op::CREATE_OBJECT, &a);
        let obj = runner.local_i32(0);
        if crate::objects::object_at(obj) == 0 {
            ex.para_obj = 0;
            return false;
        }
        ex.para_obj = obj;
        let z = Arg::Float(0.0);
        runner.run(op::ATTACH_OBJECT_TO_CHAR, &[Arg::Int(obj), Arg::Int(h), z, z, z, z, z, z]);
        runner.run(op::SET_OBJECT_VISIBLE, &[Arg::Int(obj), Arg::Int(0)]);
        runner.run(op::SET_OBJECT_SCALE, &[Arg::Int(obj), Arg::Float(0.0)]);
        runner.run(
            op::PLAY_OBJECT_ANIM,
            &[Arg::Int(obj), Arg::Text("PARA_OPEN_O"), Arg::Text("PARACHUTE"), Arg::Float(1000.0), Arg::Int(0), Arg::Int(1)],
        );
        runner.run(op::SET_OBJECT_VISIBLE, &[Arg::Int(obj), Arg::Int(1)]);
        runner.run(op::SET_OBJECT_SCALE, &[Arg::Int(obj), Arg::Float(1.0)]);
        true
    }

    fn para_anim(runner: &mut Runner, ped: u32, ex: &mut PedExtras, name: &str) {
        if ped == 0 || handle(ped).is_none() || crate::players::current_weapon(runner, ped) != WEAPON_PARACHUTE {
            return;
        }
        if name == "PARA_OPEN" {
            if para_object(runner, ped, ex) {
                let local = ProcessMemory.read_u32(game::CWORLD_PLAYERS).unwrap_or(0);
                if local != 0
                    && let (Some(l), Some(r)) = (pos(local), pos(ped))
                {
                    let s = para_sound_pos(l, r);
                    let a = [Arg::Float(s[0]), Arg::Float(s[1]), Arg::Float(s[2]), Arg::Int(PARA_OPEN_SOUND)];
                    runner.run(op::ADD_ONE_OFF_SOUND, &a);
                }
            }
        } else if let Some((anim, looped)) = para_object_anim(name)
            && para_object(runner, ped, ex)
        {
            runner.run(
                op::PLAY_OBJECT_ANIM,
                &[
                    Arg::Int(ex.para_obj),
                    Arg::Text(anim),
                    Arg::Text("PARACHUTE"),
                    Arg::Float(1.0),
                    Arg::Int(i32::from(looped)),
                    Arg::Int(1),
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_and_reports() {
        assert_eq!([20, 21, 22, 23].map(item_for_action), [Some(1), Some(4), Some(2), Some(3)]);
        assert_eq!(item_for_action(24), None);
        for a in 20..=23 {
            assert_eq!(action_for_item(item_for_action(a).unwrap()), a);
        }
        assert_eq!(after_drink(0, 1), 1250);
        assert_eq!(after_drink(49_000, 2), 50_000);
        assert_eq!(after_drink(100, 3), 100);
        assert_eq!(after_drink(100, 4), 1350, "the cigar never reaches after_drink in samp; the rule itself is plain");
        assert_eq!(local_report(0, false, false, 1), 20);
        assert_eq!(local_report(1, false, false, 4), 1, "duck before items");
        assert_eq!(local_report(0, false, true, 2), 68);
        assert_eq!(local_report(10, true, true, 2), 10);
        assert_eq!(local_report(0, true, true, 0), 11);
        assert!(!female_drink(0x5E_1234));
        assert_eq!(para_object_anim("PARA_LAND_WATER"), Some(("PARA_LAND_WATER_O", false)));
        assert_eq!(para_object_anim("PARA_OPEN"), None);
        assert_eq!(para_object_anim("PARA_LAND"), None);
        assert_eq!(para_sound_pos([0.0, 0.0, 0.0], [10.0, -20.0, 5.0]), [1.0, -2.0, 0.5]);
    }
}
