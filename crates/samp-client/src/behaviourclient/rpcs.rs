//! Small RPCs: gravity, wanted level, stunt bonus, money, widescreen, fighting style, shop name, velocity, interior.

use super::*;

// ---- small RPCs ------------------------------------------------------------------------------------------------

/// RPC 146 and InitGame: the global gravity float. Mirrors samp.dll B 0x100A1410.
pub fn set_gravity(g: f32) {
    let _ = ProcessMemory.write(GRAVITY, &g.to_le_bytes());
}

/// RPC 133: the HUD's wanted-level immediate.
pub fn set_wanted(level: u8) {
    let _ = ProcessMemory.write(WANTED_HUD, &[level]);
}

/// RPC 104 and InitGame: GTA's stunt-bonus dword. Mirrors samp.dll 0x100A1580.
pub fn set_stunt_bonus(on: bool) {
    let _ = ProcessMemory.write(STUNT_BONUS, &u32::from(on).to_le_bytes());
}

/// RPC 18: `ADD_SCORE(0, amount)`; RPC 20: the negated current money (nothing when it is 0).
pub fn give_money(runner: &mut Runner, amount: i32) {
    runner.run(op::ADD_SCORE, &[Arg::Int(0), Arg::Int(amount)]);
}

pub fn reset_money(runner: &mut Runner) {
    let m: i32 = rd(game::PLAYER_MONEY);
    if m != 0 {
        give_money(runner, m.wrapping_neg());
    }
}

/// RPC 111: the debug chat line, then `SWITCH_WIDESCREEN`. Mirrors samp.dll B 0x10018A70.
pub fn widescreen(runner: &mut Runner, on: u8) {
    crate::ui::print_debug(&format!("Widescreen = {on}"));
    runner.run(op::SWITCH_WIDESCREEN, &[Arg::Int(i32::from(on))]);
}

/// RPC 89: `GIVE_MELEE_ATTACK_TO_CHAR(ped, style, 6)`. Mirrors samp.dll B 0x100AE0E0.
pub fn fighting_style(runner: &mut Runner, ped: u32, style: u8) {
    if let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() {
        runner.run(op::GIVE_MELEE_ATTACK_TO_CHAR, &[Arg::Int(h), Arg::Int(i32::from(style)), Arg::Int(6)]);
    }
}

/// RPC 33: the ped's shop-name pointer (an empty name clears only its low byte, like samp), then
/// `CShopping::LoadShop` for names of ≤ 8 characters (the empty one included). Mirrors samp.dll B 0x10017E00.
pub fn shop_name(name: &[u8]) {
    let ped = local_ped();
    if ped == 0 || game::ped_handle(&ProcessMemory, ped).ok().flatten().is_none() {
        return;
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let name = &name[..end];
    if name.is_empty() {
        poke(ped + PED_SHOP_NAME, 0u8);
    } else if name.len() <= 31 {
        poke(ped + PED_SHOP_NAME, SHOP_NAME.set(name));
    }
    if name.len() <= 8 {
        let c = CString::new(name).unwrap_or_default();
        // SAFETY: GTA's cdecl shop loader with a NUL-terminated name, main thread.
        unsafe {
            let f: extern "C" fn(*const u8) = std::mem::transmute(LOAD_SHOP as usize);
            f(c.as_ptr() as *const u8);
        }
    }
}

/// RPC 90: the ground bits (low two of the ped flags) are flipped when set, then the move speed is written.
/// Mirrors samp.dll B 0x10018800.
pub fn set_velocity(v: [f32; 3]) {
    let ped = local_ped();
    if ped == 0 {
        return;
    }
    let flags: u32 = rd(ped + off::PED_FLAGS);
    if flags & 3 != 0 {
        poke(ped + off::PED_FLAGS, flags ^ 3);
    }
    for (i, s) in v.iter().enumerate() {
        poke(ped + off::MOVE_SPEED + 4 * i as u32, *s);
    }
}

/// RPC 156: `SET_AREA_VISIBLE`, `SET_CHAR_AREA_VISIBLE`, then `REQUEST_COLLISION` at the ped's x, y.
/// Mirrors samp.dll B 0x100199B0.
pub fn set_interior(runner: &mut Runner, interior: u8) {
    let ped = local_ped();
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
    let i = i32::from(interior);
    runner.run(op::SET_AREA_VISIBLE, &[Arg::Int(i)]);
    runner.run(op::SET_CHAR_AREA_VISIBLE, &[Arg::Int(h), Arg::Int(i)]);
    if let Some(p) = matrix_pos(ped) {
        runner.run(op::REQUEST_COLLISION, &[Arg::Float(p[0]), Arg::Float(p[1])]);
    }
}
