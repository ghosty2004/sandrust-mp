//! gta_sa.exe 1.0 US globals and field offsets the tracer reads, plus the
//! `state` line of the control channel (same format as samp-client).
//!
//! Names are given "named via" gta-reversed (`re/gta-reversed`) or plugin-sdk
//! (`re/plugin-sdk`), which also give the addresses/offsets.

use crate::json::Val;
use crate::mem::Mem;

/// Byte probed to identify 1.0 US (gta-patches.md §2).
pub const VERSION_PROBE_ADDR: u32 = 0x747483;
pub const VERSION_PROBE_US10: u8 = 0x89;

/// `gGameState` (named via gta-reversed app.h).
pub const G_GAME_STATE: u32 = 0xC8D4C0;
/// `RsGlobal.quit` (named via gta-reversed skeleton.h).
pub const RS_GLOBAL_QUIT: u32 = 0xC17040 + 0x10;
/// `CTimer::m_FrameCounter` (named via plugin-sdk CTimer.cpp).
pub const FRAME_COUNTER: u32 = 0xB7CB4C;
/// `CTimer::m_CodePause` / `m_UserPause` (plugin-sdk CTimer.cpp).
pub const TIMER_CODE_PAUSE: u32 = 0xB7CB48;
pub const TIMER_USER_PAUSE: u32 = 0xB7CB49;
/// `CWorld::Players` (CPlayerInfo[], 0x190 bytes each; gta-reversed World.h).
pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
pub const PLAYER_INFO_SIZE: u32 = 0x190;
/// `CWorld::PlayerInFocus` (plugin-sdk CWorld.cpp).
pub const PLAYER_IN_FOCUS: u32 = 0xB7CD74;
/// `CGame::currArea` (plugin-sdk CGame.cpp).
pub const CUR_AREA: u32 = 0xB72914;
/// `FrontEndMenuManager` (gta-reversed MenuManager.cpp); `m_bMenuActive` at +0x5C.
pub const MENU_MANAGER: u32 = 0xBA6748;
/// `CPad::Pads[0]` (gta-reversed Pad.h); `DisablePlayerControls` (u16) at +0x10E (plugin-sdk).
pub const PAD0: u32 = 0xB73458;
pub const PAD_DISABLE_CONTROLS: u32 = 0x10E;
/// `TheCamera` (plugin-sdk CCamera.cpp) and fields (plugin-sdk VALIDATE_OFFSET).
pub const THE_CAMERA: u32 = 0xB6F028;
pub const CAM_WIDESCREEN: u32 = 0x3D;
pub const CAM_FADING: u32 = 0x51;
pub const CAM_ACTIVE: u32 = 0x59;
pub const CAM_CAMS: u32 = 0x174;
pub const CAM_CAM_SIZE: u32 = 0x238;
pub const CAM_MODE: u32 = 0xC;
/// `CHud::m_Wants_To_Draw_Hud`, `CHud::bScriptDontDisplayRadar` (plugin-sdk CHud.cpp),
/// `CTheScripts::bDisplayHud` (plugin-sdk CTheScripts.cpp).
pub const HUD_WANTS_TO_DRAW: u32 = 0xBAA45D;
pub const HUD_SCRIPT_NO_RADAR: u32 = 0xBAA3FB;
pub const SCRIPTS_DISPLAY_HUD: u32 = 0xA444A0;
/// `TheCamera.m_aCams[0].m_nMode` (u16), `TheCamera.m_PlayerWeaponMode.m_nMode` (u16),
/// `m_aCams[0].m_fFOV`, `m_aCams[0].m_vecFront` (plugin-sdk CCamera/CCam VALIDATE_OFFSET;
/// the same addresses samp's remote-ped context swap writes, remote-control-combat.md §1.2).
pub const CAM0_MODE: u32 = THE_CAMERA + CAM_CAMS + CAM_MODE;
pub const CAM_WEAPON_MODE: u32 = 0xB6F858;
pub const CAM0_FOV: u32 = 0xB6F250;
pub const CAM0_FRONT: u32 = 0xB6F32C;
/// `CDraw::ms_fAspectRatio` (gta-reversed Draw.h).
pub const ASPECT_RATIO: u32 = 0xC3EFA4;
/// `CStats::StatTypesFloat[69]` (pistol skill, first of the 11 weapon-skill stats; gta-reversed Stats.h).
pub const STAT_PISTOL_SKILL: u32 = 0xB79380 + 0x114;
/// Number of `CWorld::Players` entries samp 0.3.7 indexes (slot 0 local, 2..209 remote;
/// remote-control-onfoot.md §1.3). GTA itself has 2.
pub const SAMP_PLAYER_SLOTS: u32 = 210;
/// `CWorld::ms_aSectors` (gta-reversed World.h): directly after `Players[1]`.
pub const CWORLD_SECTORS: u32 = 0xB7D0B8;
/// `CTimer::ms_fTimeStep` (plugin-sdk CTimer.cpp).
pub const TIME_STEP: u32 = 0xB7CB5C;
/// `CTheScripts::ScriptSpace`, `pActiveScripts` (plugin-sdk CTheScripts.cpp).
pub const SCRIPT_SPACE: u32 = 0xA49960;
pub const ACTIVE_SCRIPTS: u32 = 0xA8B42C;

/// `CRunningScript` fields (plugin-sdk VALIDATE_OFFSET).
pub mod script {
    pub const NEXT: u32 = 0x0;
    pub const NAME: u32 = 0x8;
    pub const BASE_IP: u32 = 0x10;
    pub const CURRENT_IP: u32 = 0x14;
    pub const LOCAL_VARS: u32 = 0x3C;
    pub const IS_MISSION: u32 = 0xDC;
}

/// Entity/ped/player-info fields (plugin-sdk VALIDATE_OFFSET unless noted).
pub mod off {
    pub const PLAYER_INFO_PED: u32 = 0x0;
    pub const PLAYER_INFO_STATE: u32 = 0xDC;
    pub const PLACEABLE_PLACEMENT: u32 = 0x4;
    pub const PLACEABLE_MATRIX: u32 = 0x14;
    pub const MATRIX_POS: u32 = 0x30;
    pub const ENTITY_MODEL: u32 = 0x22;
    pub const ENTITY_AREA: u32 = 0x2F;
    /// `m_nType : 3` in the low bits of the byte at +0x36 (plugin-sdk CEntity.h layout).
    pub const ENTITY_TYPE: u32 = 0x36;
    /// `CPhysical::m_vecMoveSpeed` (plugin-sdk VALIDATE_OFFSET).
    pub const PHYSICAL_MOVE_SPEED: u32 = 0x44;
    pub const PED_STATE: u32 = 0x530;
    pub const PED_MOVE_STATE: u32 = 0x534;
    pub const PED_HEALTH: u32 = 0x540;
    pub const PED_ARMOUR: u32 = 0x548;
    pub const PED_HEADING: u32 = 0x558;
    /// `m_fAimingRotation` / plugin-sdk `m_fHeadingGoal`.
    pub const PED_HEADING_GOAL: u32 = 0x55C;
    pub const PED_VEHICLE: u32 = 0x58C;
    pub const PED_TYPE: u32 = 0x598;
    pub const PED_WEAPON_SLOT: u32 = 0x718;
}

pub fn game_state_name(v: i32) -> &'static str {
    match v {
        0 => "INITIAL",
        1 => "LOGO",
        2 => "PLAYING_LOGO",
        3 => "TITLE",
        4 => "PLAYING_INTRO",
        5 => "FRONTEND_LOADING",
        6 => "FRONTEND_LOADED",
        7 => "FRONTEND_IDLE",
        8 => "LOADING_STARTED",
        9 => "IDLE",
        _ => "?",
    }
}

pub fn player_ped<M: Mem>(m: &M) -> u32 {
    m.u32(CWORLD_PLAYERS + off::PLAYER_INFO_PED).unwrap_or(0)
}

/// Address of `CWorld::Players[slot]` (also for slots ≥ 2, which samp uses).
pub fn player_info(slot: u32) -> u32 {
    CWORLD_PLAYERS + slot * PLAYER_INFO_SIZE
}

/// `(slot, CPed*)` for every non-empty `Players[slot].m_pPed`, slot < 210.
/// Slots ≥ 2 lie in the former sector array; without samp's relocation they
/// hold sector list heads, which never equal a ped pointer we look up.
pub fn scan_player_peds<M: Mem>(m: &M) -> Vec<(u32, u32)> {
    (0..SAMP_PLAYER_SLOTS)
        .filter_map(|i| m.u32(player_info(i) + off::PLAYER_INFO_PED).filter(|&p| p != 0).map(|p| (i, p)))
        .collect()
}

/// Remote player peds (slot ≥ 1 → ped), refreshed by the Windows side at
/// every frame boundary (the Players area may be guard-protected mid-frame).
static REMOTE_PEDS: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());

pub fn set_remote_peds(v: Vec<(u32, u32)>) {
    *REMOTE_PEDS.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

/// Snapshot of the remote-ped cache.
pub fn scan_cached_remote() -> Vec<(u32, u32)> {
    REMOTE_PEDS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The `CWorld::Players` slot of `ped`: 0 for the local ped, ≥ 1 from the cache.
pub fn player_slot_of<M: Mem>(m: &M, ped: u32) -> Option<u32> {
    if ped == 0 {
        return None;
    }
    if ped == player_ped(m) {
        return Some(0);
    }
    REMOTE_PEDS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|&&(_, p)| p == ped).map(|&(s, _)| s)
}

pub fn entity_pos<M: Mem>(m: &M, e: u32) -> Option<[f32; 3]> {
    let matrix = m.u32(e + off::PLACEABLE_MATRIX)?;
    let p = if matrix != 0 { matrix + off::MATRIX_POS } else { e + off::PLACEABLE_PLACEMENT };
    Some([m.f32(p)?, m.f32(p + 4)?, m.f32(p + 8)?])
}

pub fn entity_type_name(t: u8) -> &'static str {
    match t & 7 {
        0 => "nothing",
        1 => "building",
        2 => "vehicle",
        3 => "ped",
        4 => "object",
        5 => "dummy",
        _ => "?",
    }
}

/// `{"type":…,"model":…,"player":…,"ptr":…}` for an entity pointer.
pub fn describe_entity<M: Mem>(m: &M, e: u32) -> Val {
    if e == 0 {
        return Val::Null;
    }
    let mut v = Val::obj();
    if let Some(t) = m.u8(e + off::ENTITY_TYPE) {
        v.put("type", entity_type_name(t));
        v.put("model", m.i16(e + off::ENTITY_MODEL).map(i32::from));
        if t & 7 == 3 {
            v.put("player", e == player_ped(m));
            if let Some(s) = player_slot_of(m, e).filter(|&s| s != 0) {
                v.put("slot", s);
            }
        }
    }
    v.put("ptr", Val::Hex(e));
    v
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PedState {
    pub ptr: u32,
    pub pos: [f32; 3],
    pub heading: f32,
    pub health: f32,
    pub armour: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub game_state: i32,
    pub player: Option<PedState>,
}

pub fn snapshot<M: Mem>(m: &M) -> Option<Snapshot> {
    let game_state = m.i32(G_GAME_STATE)?;
    let ped = if game_state >= 9 { player_ped(m) } else { 0 };
    let player = if ped == 0 {
        None
    } else {
        Some(PedState {
            ptr: ped,
            pos: entity_pos(m, ped)?,
            heading: m.f32(ped + off::PED_HEADING)?,
            health: m.f32(ped + off::PED_HEALTH)?,
            armour: m.f32(ped + off::PED_ARMOUR)?,
        })
    };
    Some(Snapshot { game_state, player })
}

/// Same text as samp-client's `game::format_state` (DIFF-1: identical `state`
/// output on both clients); the caller appends ` frame_hook=…`.
pub fn format_state(s: &Snapshot, ticks: u64) -> String {
    let mut out = format!("state game_state={} ({}) ticks={ticks}", s.game_state, game_state_name(s.game_state));
    match &s.player {
        None => out.push_str(" ped=none"),
        Some(p) => out.push_str(&format!(
            " ped={:#x} pos={:.3},{:.3},{:.3} heading={:.4} health={:.1} armour={:.1}",
            p.ptr, p.pos[0], p.pos[1], p.pos[2], p.heading, p.health, p.armour
        )),
    }
    out
}

/// `remote` reply of the control channel (DIFF-3 cross-client): a header line
/// `remote count=N`, then one line per remote player ped `(slot, CPed*)` from
/// the frame-boundary cache. Like samp-client's `remote` (header + one line per
/// player), but the tracer only sees GTA: no player id or name, just the slot
/// and what the ped holds. Unreadable fields print `-`.
pub fn format_remote<M: Mem>(m: &M, peds: &[(u32, u32)]) -> String {
    let mut out = format!("remote count={}", peds.len());
    for &(slot, ped) in peds {
        let model = m.i16(ped + off::ENTITY_MODEL).map(|v| v.to_string());
        let pos = entity_pos(m, ped).map(|p| format!("{:.3},{:.3},{:.3}", p[0], p[1], p[2]));
        let heading = m.f32(ped + off::PED_HEADING).map(|v| format!("{v:.4}"));
        let health = m.f32(ped + off::PED_HEALTH).map(|v| format!("{v:.1}"));
        let armour = m.f32(ped + off::PED_ARMOUR).map(|v| format!("{v:.1}"));
        let dash = || "-".to_string();
        out.push_str(&format!(
            "\nremote slot={slot} ped={ped:#x} model={} pos={} heading={} health={} armour={}",
            model.unwrap_or_else(dash),
            pos.unwrap_or_else(dash),
            heading.unwrap_or_else(dash),
            health.unwrap_or_else(dash),
            armour.unwrap_or_else(dash),
        ));
    }
    out
}

/// Names of the scripts in `CTheScripts::pActiveScripts`, in list order.
pub fn active_scripts<M: Mem>(m: &M) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = m.u32(ACTIVE_SCRIPTS).unwrap_or(0);
    while s != 0 && out.len() < 128 {
        let name = m.cstr(s + script::NAME, 8).unwrap_or_default();
        let mission = m.u8(s + script::IS_MISSION).unwrap_or(0) != 0;
        out.push(if mission { format!("{name}*") } else { name });
        s = m.u32(s + script::NEXT).unwrap_or(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    #[test]
    fn state_line_matches_samp_client_format() {
        // Same fixture and expected text as samp-client game.rs tests.
        let mut m = FakeMem::default();
        m.set_u32(G_GAME_STATE, 9);
        let ped = 0x0100_0000u32;
        let mat = 0x0100_1000u32;
        m.set_u32(CWORLD_PLAYERS, ped);
        m.set_u32(ped + off::PLACEABLE_MATRIX, mat);
        m.set_f32(mat + 0x30, 2488.5);
        m.set_f32(mat + 0x34, -1666.25);
        m.set_f32(mat + 0x38, 13.5);
        m.set_f32(ped + off::PED_HEADING, 1.5);
        m.set_f32(ped + off::PED_HEALTH, 100.0);
        m.set_f32(ped + off::PED_ARMOUR, 25.0);
        let s = snapshot(&m).unwrap();
        assert_eq!(
            format_state(&s, 7),
            "state game_state=9 (IDLE) ticks=7 ped=0x1000000 pos=2488.500,-1666.250,13.500 \
             heading=1.5000 health=100.0 armour=25.0"
        );
        let mut m2 = FakeMem::default();
        m2.set_u32(G_GAME_STATE, 7);
        assert_eq!(format_state(&snapshot(&m2).unwrap(), 0), "state game_state=7 (FRONTEND_IDLE) ticks=0 ped=none");
    }

    #[test]
    fn player_slots_overlap_sector_array() {
        // Players[2] starts exactly where CWorld::ms_aSectors starts (remote-control-onfoot.md §1.3).
        assert_eq!(player_info(2), CWORLD_SECTORS);
        assert_eq!(player_info(SAMP_PLAYER_SLOTS), 0xB915B8);
        let mut m = FakeMem::default();
        for i in 0..SAMP_PLAYER_SLOTS {
            m.set_u32(player_info(i), 0);
        }
        m.set_u32(player_info(0), 0x100);
        m.set_u32(player_info(2), 0x200);
        m.set_u32(player_info(209), 0x300);
        assert_eq!(scan_player_peds(&m), vec![(0, 0x100), (2, 0x200), (209, 0x300)]);
        assert_eq!(player_slot_of(&m, 0x100), Some(0));
        assert_eq!(player_slot_of(&m, 0), None);
    }

    #[test]
    fn entity_and_scripts() {
        let mut m = FakeMem::default();
        let ped = 0x200_0000;
        m.set_u32(CWORLD_PLAYERS, ped);
        m.set(ped + off::ENTITY_TYPE, &[3 | (1 << 3)]);
        m.set(ped + off::ENTITY_MODEL, &7i16.to_le_bytes());
        assert_eq!(describe_entity(&m, ped).to_json(), r#"{"type":"ped","model":7,"player":true,"ptr":"0x2000000"}"#);
        let (a, b) = (0x300_0000u32, 0x300_1000u32);
        m.set_u32(ACTIVE_SCRIPTS, a);
        m.set_u32(a + script::NEXT, b);
        m.set(a + script::NAME, b"main\0\0\0\0");
        m.set(a + script::IS_MISSION, &[0]);
        m.set_u32(b + script::NEXT, 0);
        m.set(b + script::NAME, b"intro\0\0\0");
        m.set(b + script::IS_MISSION, &[1]);
        assert_eq!(active_scripts(&m), vec!["main".to_string(), "intro*".to_string()]);
    }

    #[test]
    fn remote_reply_lists_cached_peds() {
        let mut m = FakeMem::default();
        let (ped, mat) = (0x400_0000u32, 0x400_1000u32);
        m.set(ped + off::ENTITY_MODEL, &60i16.to_le_bytes());
        m.set_u32(ped + off::PLACEABLE_MATRIX, mat);
        m.set_f32(mat + 0x30, 1958.5);
        m.set_f32(mat + 0x34, 1343.25);
        m.set_f32(mat + 0x38, 15.375);
        m.set_f32(ped + off::PED_HEADING, -1.5);
        m.set_f32(ped + off::PED_HEALTH, 64.0);
        m.set_f32(ped + off::PED_ARMOUR, 37.0);
        assert_eq!(format_remote(&m, &[]), "remote count=0");
        assert_eq!(
            format_remote(&m, &[(2, ped), (3, 0x500_0000)]),
            "remote count=2\n\
             remote slot=2 ped=0x4000000 model=60 pos=1958.500,1343.250,15.375 heading=-1.5000 health=64.0 armour=37.0\n\
             remote slot=3 ped=0x5000000 model=- pos=- heading=- health=- armour=-"
        );
    }
}
