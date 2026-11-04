//! TRACE-7: projectiles, pad masks, mid-frame ped health.
//!
//! * `proj` record when a `gaProjectileInfo` entry becomes active (weapon type, creator, victim,
//!   the projectile object with its position and move speed) and `proj_end` when it becomes
//!   inactive again (last sampled position). `CProjectileInfo::AddProjectile` itself cannot be
//!   hooked (the exe's protection reads its entry bytes), so creation is seen by sampling at
//!   the frame boundary; `CWeapon::FireProjectile` (hooked) shows who asked for it.
//! * `pad` record when the local pad's jump / sprint / crouch / fire / aim buttons (new and old
//!   state of `CPad::Pads[0]`), the local ped's move state or its ducking / in-air flags change.
//!   Sampled at `CGame::Process` entry, i.e. after the previous frame's ped processing and
//!   before `CPad::UpdatePads`, so the buttons are what the previous frame's processing left.
//! * `hp_mid` record: health and ped state of every remote player's ped, sampled at the entry of
//!   `CCamera::Process` (after `CWorld::Process`, before the client's end-of-frame tick), written
//!   when health ≤ 0 or the state is a dying/dead state; the one-frame gap of the client's
//!   per-frame health reset (MR-7.10) shows up here.

use std::collections::BTreeMap;

use super::*;

/// `gaProjectileInfo` (32 × 0x24) and `CProjectileInfo::ms_apProjectile` (32 pointers)
/// (gta-reversed ProjectileInfo.h).
const PROJECTILE_INFO: u32 = 0xC891A8;
const PROJECTILE_INFO_SIZE: u32 = 0x24;
const PROJECTILE_OBJECTS: u32 = 0xC89110;
const MAX_PROJECTILES: u32 = 32;

mod poff {
    pub const WEAPON: u32 = 0x0;
    pub const CREATOR: u32 = 0x4;
    pub const VICTIM: u32 = 0x8;
    pub const DESTROY_TIME: u32 = 0xC;
    pub const ACTIVE: u32 = 0x10;
    pub const LAST_POS: u32 = 0x14;
}

/// `CPed::m_nPedFlags` (first 32 bits; gta-reversed Ped.h): bit 9 in the air, bit 26 ducking.
const PED_FLAGS: u32 = 0x46C;

fn vec(m: &ProcMem, a: u32, scale: f32) -> Val {
    Val::List((0..3).map(|i| m.f32(a + 4 * i).map(|f| (f * scale).round() / scale).into()).collect())
}

static PROJ_PREV: Mutex<BTreeMap<u32, (u32, u32)>> = Mutex::new(BTreeMap::new());

fn sample_projectiles(m: &ProcMem) {
    let mut prev = PROJ_PREV.lock().unwrap_or_else(|e| e.into_inner());
    for i in 0..MAX_PROJECTILES {
        let info = PROJECTILE_INFO + i * PROJECTILE_INFO_SIZE;
        let active = m.u8(info + poff::ACTIVE).unwrap_or(0) != 0;
        let obj = m.u32(PROJECTILE_OBJECTS + 4 * i).unwrap_or(0);
        let wt = m.u32(info + poff::WEAPON).unwrap_or(0);
        let was = prev.get(&i).copied();
        if active && was != Some((obj, wt)) {
            let mut v = Val::obj()
                .with("index", i)
                .with("weapon", wt)
                .with("creator", gta::describe_entity(m, m.u32(info + poff::CREATOR).unwrap_or(0)))
                .with("victim", gta::describe_entity(m, m.u32(info + poff::VICTIM).unwrap_or(0)))
                .with("destroy_time", m.i32(info + poff::DESTROY_TIME))
                .with("object", Val::Hex(obj));
            if obj != 0 {
                if let Some(p) = gta::entity_pos(m, obj) {
                    v.put("pos", Val::List(p.iter().map(|&f| ((f * 100.0).round() / 100.0).into()).collect()));
                }
                v.put("model", m.i16(obj + gta::off::ENTITY_MODEL).map(i32::from));
                v.put("move", vec(m, obj + gta::off::PHYSICAL_MOVE_SPEED, 10_000.0));
            }
            emit(record("proj", None, None, None, v));
            prev.insert(i, (obj, wt));
        } else if !active && was.is_some() {
            emit(record(
                "proj_end",
                None,
                None,
                None,
                Val::obj().with("index", i).with("weapon", wt).with("last_pos", vec(m, info + poff::LAST_POS, 100.0)),
            ));
            prev.remove(&i);
        }
    }
}

static PAD_PREV: Mutex<String> = Mutex::new(String::new());

fn sample_pad(m: &ProcMem) {
    let ped = gta::player_ped(m);
    if ped == 0 {
        return;
    }
    // CControllerState (gta-reversed ControllerState.h): RightShoulder1 +0xC (aim), Square +0x1C
    // (jump), Cross +0x20 (sprint), Circle +0x22 (fire), ShockButtonL +0x24 (crouch).
    let btn = |base: u32| -> [i16; 5] {
        [0xC, 0x1C, 0x20, 0x22, 0x24].map(|o| m.i16(base + o).unwrap_or(-1))
    };
    let (new, old) = (btn(gta::PAD0), btn(gta::PAD0 + 0x30));
    let flags = m.u32(ped + PED_FLAGS).unwrap_or(0);
    let mv = m.u32(ped + gta::off::PED_MOVE_STATE).unwrap_or(0);
    let st = m.u32(ped + gta::off::PED_STATE).unwrap_or(0);
    let (air, duck) = (flags >> 9 & 1 != 0, flags >> 26 & 1 != 0);
    let key = format!("{new:?}|{old:?}|{mv}|{st}|{air}|{duck}");
    let mut prev = PAD_PREV.lock().unwrap_or_else(|e| e.into_inner());
    if *prev == key {
        return;
    }
    *prev = key;
    let names = ["aim", "jump", "sprint", "fire", "crouch"];
    let mut n = Val::obj();
    let mut o = Val::obj();
    for (i, name) in names.iter().enumerate() {
        n.put(name, i32::from(new[i]));
        o.put(name, i32::from(old[i]));
    }
    emit(record(
        "pad",
        None,
        None,
        None,
        Val::obj()
            .with("new", n)
            .with("old", o)
            .with("move_state", mv)
            .with("ped_state", st)
            .with("in_air", air)
            .with("ducking", duck),
    ));
}

/// Remote player peds whose health is ≤ 0 or whose state is dying (54) / dead (55) after
/// `CWorld::Process`; one record per ped and frame.
pub fn on_camera_process() {
    if !client_loaded() {
        return;
    }
    let m = ProcMem;
    for (slot, p) in gta::scan_cached_remote() {
        if p == 0 {
            continue;
        }
        let hp = m.f32(p + gta::off::PED_HEALTH).unwrap_or(f32::NAN);
        let st = m.u32(p + gta::off::PED_STATE).unwrap_or(0);
        if hp <= 0.0 || st == 54 || st == 55 {
            emit(record(
                "hp_mid",
                None,
                None,
                None,
                Val::obj()
                    .with("slot", slot)
                    .with("ped", gta::describe_entity(&m, p))
                    .with("health", (hp * 1000.0).round() / 1000.0)
                    .with("state", st),
            ));
        }
    }
}

/// Called from `on_frame` (main thread, CGame::Process entry).
pub fn frame() {
    if !client_loaded() {
        return;
    }
    let m = ProcMem;
    sample_projectiles(&m);
    sample_pad(&m);
}

fn client_loaded() -> bool {
    MODULES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|m| m.client)
}
