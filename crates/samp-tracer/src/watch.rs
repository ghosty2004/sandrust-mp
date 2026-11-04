//! Per-frame snapshot of game globals; changes are written as `watch`
//! records. Covers state that clients change by writing memory instead of
//! calling functions (HUD flags, pad lock, camera mode, active scripts).

use crate::gta::{self, off};
use crate::json::Val;
use crate::mem::Mem;

pub type Snap = Vec<(&'static str, String)>;

/// Collects the watched values. `extra` holds per-frame flags computed by
/// the hooks (e.g. whether any stock script command ran this frame).
pub fn collect<M: Mem>(m: &M, extra: &[(&'static str, String)]) -> Snap {
    let mut s: Snap = Vec::new();
    let u8s = |a: u32| m.u8(a).map(|v| v.to_string()).unwrap_or_else(|| "?".into());
    let gs = m.i32(gta::G_GAME_STATE).unwrap_or(-1);
    s.push(("game_state", format!("{gs} {}", gta::game_state_name(gs))));
    s.push(("code_pause", u8s(gta::TIMER_CODE_PAUSE)));
    s.push(("user_pause", u8s(gta::TIMER_USER_PAUSE)));
    s.push(("menu_active", u8s(gta::MENU_MANAGER + 0x5C)));
    s.push(("cur_area", m.i32(gta::CUR_AREA).map(|v| v.to_string()).unwrap_or_default()));
    s.push(("player_in_focus", u8s(gta::PLAYER_IN_FOCUS)));
    s.push((
        "player_state",
        m.u32(gta::CWORLD_PLAYERS + off::PLAYER_INFO_STATE).map(|v| v.to_string()).unwrap_or_default(),
    ));
    let ped = gta::player_ped(m);
    if ped == 0 {
        s.push(("player_ped", "none".into()));
    } else {
        s.push(("player_ped", "present".into()));
        s.push(("player_model", m.i16(ped + off::ENTITY_MODEL).map(|v| v.to_string()).unwrap_or_default()));
        s.push(("player_ped_state", m.u32(ped + off::PED_STATE).map(|v| v.to_string()).unwrap_or_default()));
        s.push(("player_area", u8s(ped + off::ENTITY_AREA)));
        s.push(("player_in_vehicle", (m.u32(ped + off::PED_VEHICLE).unwrap_or(0) != 0).to_string()));
        s.push(("player_weapon_slot", u8s(ped + off::PED_WEAPON_SLOT)));
    }
    s.push((
        "controls_disabled",
        m.u16(gta::PAD0 + gta::PAD_DISABLE_CONTROLS).map(|v| v.to_string()).unwrap_or_default(),
    ));
    let cam = gta::THE_CAMERA;
    s.push(("widescreen", u8s(cam + gta::CAM_WIDESCREEN)));
    s.push(("fading", u8s(cam + gta::CAM_FADING)));
    let active = m.u8(cam + gta::CAM_ACTIVE).unwrap_or(0) as u32;
    s.push((
        "cam_mode",
        m.u16(cam + gta::CAM_CAMS + active.min(2) * gta::CAM_CAM_SIZE + gta::CAM_MODE)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    ));
    s.push(("hud_wants_to_draw", u8s(gta::HUD_WANTS_TO_DRAW)));
    s.push(("scripts_display_hud", u8s(gta::SCRIPTS_DISPLAY_HUD)));
    s.push(("script_no_radar", u8s(gta::HUD_SCRIPT_NO_RADAR)));
    population(m, &mut s);
    s.push(("active_scripts", gta::active_scripts(m).join(",")));
    s.push(("script_ips", script_ips(m).join(",")));
    s.extend(extra.iter().cloned());
    s
}

/// Single-player population / police state (TRACE-4, SP-OFF baseline). Addresses from
/// gta-reversed: `CPools::ms_pPedPool` / `ms_pVehiclePool`, `CPopulation::ms_nTotalPeds` /
/// `ms_nNumCop`, `CCarCtrl::NumRandomCars` / `NumLawEnforcerCars` / `NumParkedCars` /
/// `NumMissionCars`, `CWanted::MaximumWantedLevel`, the density multipliers, and the local
/// player's `CWanted` (`CPlayerPed::m_pPlayerData` +0x480 → `m_pWanted` +0x0; chaos +0x0,
/// wanted level +0x2C).
pub mod pop {
    pub const PED_POOL: u32 = 0xB74490;
    pub const VEHICLE_POOL: u32 = 0xB74494;
    pub const TOTAL_PEDS: u32 = 0xC0EC28;
    pub const NUM_COPS: u32 = 0xC0EC68;
    pub const RANDOM_CARS: u32 = 0x969094;
    pub const LAW_CARS: u32 = 0x969098;
    pub const PARKED_CARS: u32 = 0x9690A0;
    pub const MISSION_CARS: u32 = 0x96909C;
    pub const MAX_WANTED: u32 = 0x8CDEE4;
    pub const PED_DENSITY: u32 = 0x8D2530;
    pub const CAR_DENSITY: u32 = 0x8A5B20;
    pub const PLAYER_DATA: u32 = 0x480;
    pub const WANTED_CHAOS: u32 = 0x0;
    pub const WANTED_LEVEL: u32 = 0x2C;
}

/// Used slots of a `CPool` (`m_byteMap` +0x4, `m_nSize` +0x8; a slot is free when bit 7 is set).
pub fn pool_used<M: Mem>(m: &M, pool_ptr: u32) -> Option<u32> {
    let pool = m.u32(pool_ptr).filter(|&p| p != 0)?;
    let map = m.u32(pool + 4)?;
    let size = m.i32(pool + 8).filter(|&n| (1..=0x4000).contains(&n))? as u32;
    let mut buf = vec![0u8; size as usize];
    if !m.read(map, &mut buf) {
        return None;
    }
    Some(buf.iter().filter(|&&b| b & 0x80 == 0).count() as u32)
}

fn population<M: Mem>(m: &M, s: &mut Snap) {
    let num = |v: Option<i32>| v.map(|v| v.to_string()).unwrap_or_default();
    let flt = |v: Option<f32>| v.map(|v| format!("{v:.2}")).unwrap_or_default();
    s.push(("ped_pool_used", pool_used(m, pop::PED_POOL).map(|v| v.to_string()).unwrap_or_default()));
    s.push(("vehicle_pool_used", pool_used(m, pop::VEHICLE_POOL).map(|v| v.to_string()).unwrap_or_default()));
    s.push(("pop_total_peds", num(m.i32(pop::TOTAL_PEDS))));
    s.push(("pop_cops", num(m.i32(pop::NUM_COPS))));
    s.push(("cars_random", num(m.i32(pop::RANDOM_CARS))));
    s.push(("cars_law", num(m.i32(pop::LAW_CARS))));
    s.push(("cars_parked", num(m.i32(pop::PARKED_CARS))));
    s.push(("cars_mission", num(m.i32(pop::MISSION_CARS))));
    s.push(("max_wanted", num(m.i32(pop::MAX_WANTED))));
    s.push(("ped_density", flt(m.f32(pop::PED_DENSITY))));
    s.push(("car_density", flt(m.f32(pop::CAR_DENSITY))));
    let ped = gta::player_ped(m);
    let wanted = if ped == 0 {
        0
    } else {
        m.u32(ped + pop::PLAYER_DATA).and_then(|d| if d == 0 { None } else { m.u32(d) }).unwrap_or(0)
    };
    if wanted != 0 {
        s.push(("wanted_level", num(m.i32(wanted + pop::WANTED_LEVEL))));
        s.push(("wanted_chaos", num(m.i32(wanted + pop::WANTED_CHAOS))));
    }
}

/// `name@offset` of every active script's current IP (offset into ScriptSpace).
/// A script waiting in a loop keeps its IP, so this changes only when a script
/// moves on to other code.
pub fn script_ips<M: Mem>(m: &M) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = m.u32(gta::ACTIVE_SCRIPTS).unwrap_or(0);
    while s != 0 && out.len() < 128 {
        let name = m.cstr(s + gta::script::NAME, 8).unwrap_or_default();
        let ip = m.u32(s + gta::script::CURRENT_IP).unwrap_or(0).wrapping_sub(gta::SCRIPT_SPACE);
        out.push(format!("{name}@{ip}"));
        s = m.u32(s + gta::script::NEXT).unwrap_or(0);
    }
    out
}

/// Byte range of main.scm's global-variable block: main.scm starts with
/// `GOTO <first code>` (opcode 0x0002, type 0x01, int32 target); the globals
/// lie between that instruction (8 bytes) and the target. Capped at 256 KiB.
pub fn globals_range<M: Mem>(m: &M) -> Option<(u32, u32)> {
    let sp = gta::SCRIPT_SPACE;
    if m.u16(sp)? != 0x0002 || m.u8(sp + 2)? != 0x01 {
        return None;
    }
    let target = m.u32(sp + 3)?;
    (8..=0x40000).contains(&target).then_some((8, target & !3))
}

/// Changed 32-bit globals (`g<byte offset>`: "old->new"), at most `max`.
pub fn diff_words(prev: &[u32], cur: &[u32], first_off: u32, max: usize) -> Option<Val> {
    let mut v = Val::obj();
    let mut n = 0;
    for (i, (a, b)) in prev.iter().zip(cur).enumerate() {
        if a != b {
            n += 1;
            if n <= max {
                v.put(&format!("g{}", first_off + 4 * i as u32), format!("{}->{}", *a as i32, *b as i32));
            }
        }
    }
    if n > max {
        v.put("more", (n - max) as u32);
    }
    (n > 0).then_some(v)
}

/// Changed (and new) keys between two snapshots, as a JSON object.
pub fn diff(prev: &Snap, cur: &Snap) -> Option<Val> {
    let mut v = Val::obj();
    let mut any = false;
    for (k, val) in cur {
        let old = prev.iter().find(|(pk, _)| pk == k).map(|(_, pv)| pv);
        if old != Some(val) {
            v.put(k, val.as_str());
            any = true;
        }
    }
    for (k, _) in prev {
        if !cur.iter().any(|(ck, _)| ck == k) {
            v.put(k, Val::Null);
            any = true;
        }
    }
    any.then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    #[test]
    fn reports_changes_only() {
        let mut m = FakeMem::default();
        m.set_u32(gta::G_GAME_STATE, 9);
        let a = collect(&m, &[("stock_scripts", "off".into())]);
        assert!(a.iter().any(|(k, v)| *k == "game_state" && v == "9 IDLE"));
        assert!(a.iter().any(|(k, v)| *k == "player_ped" && v == "none"));
        assert_eq!(diff(&a, &a), None);
        let b = collect(&m, &[("stock_scripts", "on".into())]);
        assert_eq!(diff(&a, &b).unwrap().to_json(), r#"{"stock_scripts":"on"}"#);
        assert_eq!(diff(&b, &a[..a.len() - 1].to_vec()).unwrap().to_json(), r#"{"stock_scripts":null}"#);
    }

    #[test]
    fn pool_counts() {
        let mut m = FakeMem::default();
        assert_eq!(pool_used(&m, pop::PED_POOL), None);
        m.set_u32(pop::PED_POOL, 0x30_0000);
        m.set_u32(0x30_0004, 0x31_0000);
        m.set_u32(0x30_0008, 4);
        m.set(0x31_0000, &[0x80, 0x01, 0x81, 0x7F]);
        assert_eq!(pool_used(&m, pop::PED_POOL), Some(2));
    }

    #[test]
    fn globals() {
        let mut m = FakeMem::default();
        m.set(gta::SCRIPT_SPACE, &[0x02, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00]);
        assert_eq!(globals_range(&m), Some((8, 16)));
        assert_eq!(diff_words(&[1, 2, 3], &[1, 5, 3], 8, 10).unwrap().to_json(), r#"{"g12":"2->5"}"#);
        assert_eq!(diff_words(&[1, 2], &[3, 4], 8, 1).unwrap().to_json(), r#"{"g8":"1->3","more":1}"#);
        assert_eq!(diff_words(&[1], &[1], 8, 1), None);
    }
}
