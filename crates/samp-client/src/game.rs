//! gta_sa.exe 1.0 US addresses and read-only views of game state.
//!
//! Addresses and field offsets are public facts about the executable; names
//! follow gta-reversed (`re/gta-reversed`) and plugin-sdk (`re/plugin-sdk`).

use crate::patch::{Memory, PatchError};

/// Byte probed to identify the executable: 0x89 on 1.0 US (first byte of
/// `mov [gGameState],esi` in `psInitialize`); gta-patches.md §2.
pub const VERSION_PROBE_ADDR: u32 = 0x747483;
pub const VERSION_PROBE_US10: u8 = 0x89;
/// Same probe on 1.0 EU (not supported).
pub const VERSION_PROBE_EU10: u8 = 0xC8;

/// `gGameState` (int32), named via gta-reversed `app/app.h`.
pub const G_GAME_STATE: u32 = 0xC8D4C0;
/// `RsGlobal` (RsGlobalType), named via gta-reversed `RenderWare.h`.
pub const RS_GLOBAL: u32 = 0xC17040;
/// `RsGlobal.frameLimit` (+0xC), named via gta-reversed `skeleton.h`.
pub const RS_GLOBAL_FRAME_LIMIT: u32 = RS_GLOBAL + 0xC;
/// `RsGlobal.quit` (+0x10, RwBool), named via gta-reversed `skeleton.h`.
pub const RS_GLOBAL_QUIT: u32 = RS_GLOBAL + 0x10;
/// `FrontEndMenuManager` (CMenuManager), named via gta-reversed `MenuManager.cpp`.
pub const FRONTEND_MENU_MANAGER: u32 = 0xBA6748;
/// `CWorld::Players[0]` (CPlayerInfo, 0x190 bytes each), named via gta-reversed `World.h`.
pub const CWORLD_PLAYERS: u32 = 0xB7CD98;
/// `Render2dStuff`, named via gta-reversed; called from `Idle` at 0x53EB12.
pub const RENDER_2D_STUFF: u32 = 0x53E230;
/// Call site of `Render2dStuff` inside `Idle` (0x53E920); gta-patches CSV row 6.
pub const IDLE_RENDER_2D_CALL: u32 = 0x53EB12;

/// `CFileMgr::OpenFile(path, mode)` (cdecl, a jump to the CRT `fopen`), gta-reversed `FileMgr.cpp`.
pub const CFILEMGR_OPEN_FILE: u32 = 0x538900;
/// The `call CFileMgr::OpenFile` that opens "main.scm" in `CTheScripts::Init` (0x468D50).
pub const MAIN_SCM_OPEN_CALL: u32 = 0x468EC9;
/// Where the local player ped is created before the server places it
/// (the stock script's own start point, Grove Street).
pub const INITIAL_PLAYER_POS: [f32; 3] = [2488.56, -1666.84, 13.38];

/// `CPad::Pads[0]` (CPad, NewState `CControllerState` at +0), gta-reversed `Pad.h`.
pub const PAD0: u32 = 0xB73458;
/// `CPad::NewKeyState` (CKeyboardState, 0x270 bytes), gta-reversed `Pad.h`.
pub const NEW_KEY_STATE: u32 = 0xB73190;
/// Offsets of i16 key flags in `CKeyboardState` (gta-reversed `KeyboardState.h`).
pub mod key {
    pub const LEFT: u32 = 0x22A;
    pub const RIGHT: u32 = 0x22C;
    pub const LSHIFT: u32 = 0x25C;
    pub const RSHIFT: u32 = 0x25E;
    pub const SHIFT: u32 = 0x260;
}
/// `CPools::ms_pPedPool` (CPool*: objects +0, slot bytes +4, size +8), gta-reversed `Pools.cpp`.
pub const PED_POOL: u32 = 0xB74490;
/// Ped pool element size (`CCopPed`, 0x7C4), gta-reversed `CopPed.h`.
pub const PED_POOL_ELEMENT: u32 = 0x7C4;
/// `CWorld::Players[0].m_nMoney` (player-state.md, ResetPlayerMoney: 0xB7CE50).
pub const PLAYER_MONEY: u32 = CWORLD_PLAYERS + 0xB8;
/// `FrontEndMenuManager.m_bHudOn` (+0x21), gta-reversed `MenuManager.h`.
pub const MENU_HUD_ON: u32 = FRONTEND_MENU_MANAGER + 0x21;
/// `FrontEndMenuManager.m_bMenuActive` (+0x5C).
pub const MENU_ACTIVE: u32 = FRONTEND_MENU_MANAGER + 0x5C;
/// `CHud::bScriptDontDisplayRadar`, gta-reversed `Hud.h`.
pub const HUD_SCRIPT_DONT_DISPLAY_RADAR: u32 = 0xBAA3FB;
/// `CCamera::Fade` entry; samp writes `ret 8` here at every spawn (CSV row 365).
pub const CAMERA_FADE: u32 = 0x50AC20;
/// `CPlayerPed::SetInitialState(bool)` (thiscall), gta-reversed `PlayerPed.cpp`.
pub const PLAYER_PED_SET_INITIAL_STATE: u32 = 0x60CD20;
/// `CPed::GiveWeapon(eWeaponType, uint32 ammo, bool)` (thiscall), gta-reversed `Ped.cpp`.
pub const PED_GIVE_WEAPON: u32 = 0x5E6080;
/// `CPed::ClearWeapons()` (thiscall), gta-reversed `Ped.cpp`.
pub const PED_CLEAR_WEAPONS: u32 = 0x5E6320;
/// `CAEPedSpeechAudioEntity::Initialise(CEntity*)` (thiscall), gta-reversed `AEPedSpeechAudioEntity.cpp`.
pub const PED_SPEECH_INITIALISE: u32 = 0x4E68D0;
/// `CTaskSimpleJetPack` vtable (remote-control-onfoot.md §2.1).
pub const VTBL_TASK_SIMPLE_JETPACK: u32 = 0x8705C4;

/// Field offsets (named via plugin-sdk VALIDATE_OFFSET / gta-reversed headers).
pub mod off {
    /// `CEntity` vtable pointer.
    pub const VTABLE: u32 = 0x0;
    /// `CEntity::Teleport(CVector, bool)` vtable slot (player-state.md RPC 12).
    pub const VT_TELEPORT: u32 = 0x38;
    /// `CEntity::SetModelIndex(uint32)` vtable slot (samp's skin change).
    pub const VT_SET_MODEL_INDEX: u32 = 0x14;
    /// `CEntity::DeleteRwObject()` vtable slot.
    pub const VT_DELETE_RW_OBJECT: u32 = 0x20;
    /// `CPed::m_pedSpeech` (CAEPedSpeechAudioEntity).
    pub const PED_SPEECH: u32 = 0x294;
    /// `CEntity::m_nModelIndex` (u16).
    pub const ENTITY_MODEL: u32 = 0x22;
    /// `CPhysical::m_vecMoveSpeed`.
    pub const MOVE_SPEED: u32 = 0x44;
    /// `CPed` flag words (first: bit 8 bInVehicle, bit 26 bIsDucking).
    pub const PED_FLAGS: u32 = 0x46C;
    /// `CPed::m_pIntelligence`; its primary task slot is at +0x10 (remote-control-onfoot.md §2.1).
    pub const PED_INTELLIGENCE: u32 = 0x47C;
    pub const INTELLIGENCE_PRIMARY_TASK: u32 = 0x10;
    /// `CPed::m_ePedState`.
    pub const PED_STATE: u32 = 0x530;
    /// `CPed::m_fHeadingGoal`.
    pub const PED_HEADING_GOAL: u32 = 0x55C;
    /// `CPed::m_aWeapons` (13 × CWeapon, 0x1C each: type +0, total ammo +0xC).
    pub const PED_WEAPONS: u32 = 0x5A0;
    pub const WEAPON_SIZE: u32 = 0x1C;
    pub const WEAPON_AMMO_TOTAL: u32 = 0xC;
    /// `CPed::m_nSelectedWepSlot` (u8).
    pub const PED_SELECTED_WEAPON_SLOT: u32 = 0x718;
    /// Last damage weapon / entity (player-state.md 53; spawn clears them).
    pub const PED_LAST_DAMAGE_WEAPON: u32 = 0x760;
    pub const PED_LAST_DAMAGE_ENTITY: u32 = 0x764;
    /// `CPlayerInfo::m_pPed`.
    pub const PLAYER_INFO_PED: u32 = 0x0;
    /// `CPlaceable::m_placement` (CSimpleTransform: CVector pos, float heading).
    pub const PLACEABLE_PLACEMENT: u32 = 0x4;
    /// `CPlaceable::m_matrix` (CMatrixLink*).
    pub const PLACEABLE_MATRIX: u32 = 0x14;
    /// `CMatrix::pos`.
    pub const MATRIX_POS: u32 = 0x30;
    /// `CPed::m_fHealth`.
    pub const PED_HEALTH: u32 = 0x540;
    /// `CPed::m_fArmour`.
    pub const PED_ARMOUR: u32 = 0x548;
    /// `CPed::m_fCurrentRotation` (plugin-sdk: m_fHeadingCurrent), radians.
    pub const PED_HEADING: u32 = 0x558;
}

/// `eGameState` values (gta-reversed `Enums/eGameState.h`).
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

/// Weapon id → weapon model id (GTA SA `weapon.dat` / `default.ide`).
pub fn weapon_model(weapon: u32) -> Option<i32> {
    Some(match weapon {
        1 => 331,
        2 => 333,
        3 => 334,
        4 => 335,
        5 => 336,
        6 => 337,
        7 => 338,
        8 => 339,
        9 => 341,
        10 => 321,
        11 => 322,
        12 => 323,
        13 => 324,
        14 => 325,
        15 => 326,
        16 => 342,
        17 => 343,
        18 => 344,
        22 => 346,
        23 => 347,
        24 => 348,
        25 => 349,
        26 => 350,
        27 => 351,
        28 => 352,
        29 => 353,
        30 => 355,
        31 => 356,
        32 => 372,
        33 => 357,
        34 => 358,
        35 => 359,
        36 => 360,
        37 => 361,
        38 => 362,
        39 => 363,
        40 => 364,
        41 => 365,
        42 => 366,
        43 => 367,
        44 => 368,
        45 => 369,
        46 => 371,
        _ => return None,
    })
}

/// Degrees → radians in (−π, π].
pub fn heading_rad(deg: f32) -> f32 {
    let mut r = deg.to_radians() % std::f32::consts::TAU;
    if r > std::f32::consts::PI {
        r -= std::f32::consts::TAU;
    } else if r <= -std::f32::consts::PI {
        r += std::f32::consts::TAU;
    }
    r
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

/// Reads the local player ped (`CWorld::Players[0].m_pPed`), if any.
pub fn read_player<M: Memory>(mem: &M) -> Result<Option<PedState>, PatchError> {
    let ped = mem.read_u32(CWORLD_PLAYERS + off::PLAYER_INFO_PED)?;
    if ped == 0 {
        return Ok(None);
    }
    let matrix = mem.read_u32(ped + off::PLACEABLE_MATRIX)?;
    let pos_addr = if matrix != 0 { matrix + off::MATRIX_POS } else { ped + off::PLACEABLE_PLACEMENT };
    let pos = [mem.read_f32(pos_addr)?, mem.read_f32(pos_addr + 4)?, mem.read_f32(pos_addr + 8)?];
    Ok(Some(PedState {
        ptr: ped,
        pos,
        heading: mem.read_f32(ped + off::PED_HEADING)?,
        health: mem.read_f32(ped + off::PED_HEALTH)?,
        armour: mem.read_f32(ped + off::PED_ARMOUR)?,
    }))
}

/// Script handle of a ped (`CPool::GetRef`: index << 8 | slot byte).
pub fn ped_handle<M: Memory>(mem: &M, ped: u32) -> Result<Option<i32>, PatchError> {
    let pool = mem.read_u32(PED_POOL)?;
    if pool == 0 || ped == 0 {
        return Ok(None);
    }
    let objects = mem.read_u32(pool)?;
    let slots = mem.read_u32(pool + 4)?;
    let size = mem.read_i32(pool + 8)?;
    if ped < objects || !(ped - objects).is_multiple_of(PED_POOL_ELEMENT) {
        return Ok(None);
    }
    let index = (ped - objects) / PED_POOL_ELEMENT;
    if index as i32 >= size {
        return Ok(None);
    }
    let slot = mem.read_u8(slots + index)?;
    Ok(Some(((index as i32) << 8) | i32::from(slot)))
}

/// Reads one i16 key of `CPad::NewKeyState`.
pub fn key_down<M: Memory>(mem: &M, key_offset: u32) -> bool {
    mem.read_u16(NEW_KEY_STATE + key_offset).map(|v| v != 0).unwrap_or(false)
}

/// Builds the on-foot sync sample from the local ped and pad 0.
pub fn read_local_sample<M: Memory>(mem: &M, ped: u32) -> Result<crate::footsync::LocalSample, PatchError> {
    use crate::footsync::{KeyContext, LocalSample, PadState, Rotation};
    let mut pad = [0u8; PadState::SIZE];
    mem.read(PAD0, &mut pad)?;
    let pad = PadState::from_bytes(&pad);
    let v3 =
        |a: u32| -> Result<[f32; 3], PatchError> { Ok([mem.read_f32(a)?, mem.read_f32(a + 4)?, mem.read_f32(a + 8)?]) };
    let matrix = mem.read_u32(ped + off::PLACEABLE_MATRIX)?;
    let (position, rotation) = if matrix != 0 {
        (
            v3(matrix + off::MATRIX_POS)?,
            Rotation { right: v3(matrix)?, forward: v3(matrix + 0x10)?, up: v3(matrix + 0x20)? },
        )
    } else {
        let h = mem.read_f32(ped + off::PLACEABLE_PLACEMENT + 12)?;
        (
            v3(ped + off::PLACEABLE_PLACEMENT)?,
            Rotation { right: [h.cos(), h.sin(), 0.0], forward: [-h.sin(), h.cos(), 0.0], up: [0.0, 0.0, 1.0] },
        )
    };
    let flags = mem.read_u32(ped + off::PED_FLAGS)?;
    let in_vehicle = flags & (1 << 8) != 0;
    let ducking = flags & (1 << 26) != 0;
    let slot = u32::from(mem.read_u8(ped + off::PED_SELECTED_WEAPON_SLOT)?.min(12));
    let wbase = ped + off::PED_WEAPONS + slot * off::WEAPON_SIZE;
    let weapon = mem.read_u32(wbase)?;
    let ammo = mem.read_u32(wbase + off::WEAPON_AMMO_TOTAL)?;
    let intel = mem.read_u32(ped + off::PED_INTELLIGENCE)?;
    let jetpack = if intel != 0 {
        let task = mem.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK)?;
        task != 0 && mem.read_u32(task)? == VTBL_TASK_SIMPLE_JETPACK
    } else {
        false
    };
    let can_fire = weapon < 16 || weapon == 46 || ammo > 0;
    Ok(LocalSample {
        pad,
        key_ctx: KeyContext { in_vehicle, can_fire, jetpack },
        position,
        rotation,
        health: mem.read_f32(ped + off::PED_HEALTH)?,
        armour: mem.read_f32(ped + off::PED_ARMOUR)?,
        weapon: weapon.min(63) as u8,
        // Special actions other than ducking (jetpack, phone, carry…) come with later milestones.
        special_action: if jetpack {
            2
        } else if ducking && !in_vehicle {
            1
        } else {
            0
        },
        move_speed: v3(ped + off::MOVE_SPEED)?,
        animation: 0,
    })
}

pub fn snapshot<M: Memory>(mem: &M) -> Result<Snapshot, PatchError> {
    let game_state = mem.read_i32(G_GAME_STATE)?;
    // The player array is only meaningful once the world has been loaded.
    let player = if game_state >= 9 { read_player(mem)? } else { None };
    Ok(Snapshot { game_state, player })
}

/// One-line `state` reply for the control channel.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::fake::FakeMemory;

    fn put_f32(m: &mut FakeMemory, a: u32, v: f32) {
        m.set(a, &v.to_le_bytes());
    }

    #[test]
    fn snapshot_reads_matrix_position() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0100_0000);
        m.set(G_GAME_STATE, &9i32.to_le_bytes());
        let ped = 0x0100_0000u32;
        let mat = 0x0100_1000u32;
        m.set(CWORLD_PLAYERS, &ped.to_le_bytes());
        m.set(ped + off::PLACEABLE_MATRIX, &mat.to_le_bytes());
        put_f32(&mut m, mat + 0x30, 2488.5);
        put_f32(&mut m, mat + 0x34, -1666.25);
        put_f32(&mut m, mat + 0x38, 13.5);
        put_f32(&mut m, ped + off::PED_HEADING, 1.5);
        put_f32(&mut m, ped + off::PED_HEALTH, 100.0);
        put_f32(&mut m, ped + off::PED_ARMOUR, 25.0);
        let s = snapshot(&m).unwrap();
        let p = s.player.unwrap();
        assert_eq!(p.pos, [2488.5, -1666.25, 13.5]);
        assert_eq!(
            format_state(&s, 7),
            "state game_state=9 (IDLE) ticks=7 ped=0x1000000 pos=2488.500,-1666.250,13.500 \
             heading=1.5000 health=100.0 armour=25.0"
        );
    }

    #[test]
    fn heading_normalisation() {
        assert!((heading_rad(90.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((heading_rad(270.0) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(heading_rad(355.0) < 0.0);
    }

    #[test]
    fn weapon_models() {
        assert_eq!(weapon_model(24), Some(348));
        assert_eq!(weapon_model(0), None);
        assert_eq!(weapon_model(19), None);
    }

    #[test]
    fn ped_handle_from_pool() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0100_0000);
        let pool = 0x0100_0000u32;
        let objects = 0x0100_1000u32;
        let slots = 0x0100_0800u32;
        m.set(PED_POOL, &pool.to_le_bytes());
        m.set(pool, &objects.to_le_bytes());
        m.set(pool + 4, &slots.to_le_bytes());
        m.set(pool + 8, &140i32.to_le_bytes());
        m.set(slots + 2, &[0x05]);
        let ped = objects + 2 * PED_POOL_ELEMENT;
        assert_eq!(ped_handle(&m, ped).unwrap(), Some(0x205));
        assert_eq!(ped_handle(&m, ped + 4).unwrap(), None);
    }

    #[test]
    fn local_sample_reads_pad_matrix_and_weapon() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0100_0000);
        let ped = 0x0100_0000u32;
        let mat = 0x0100_1000u32;
        m.set(ped + off::PLACEABLE_MATRIX, &mat.to_le_bytes());
        for (i, v) in [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0].iter().enumerate() {
            put_f32(&mut m, mat + 4 * i as u32, *v);
        }
        put_f32(&mut m, mat + 0x30, 10.0);
        put_f32(&mut m, ped + off::PED_HEALTH, 100.0);
        put_f32(&mut m, ped + off::MOVE_SPEED, 0.25);
        m.set(ped + off::PED_FLAGS, &(1u32 << 26).to_le_bytes());
        m.set(ped + off::PED_SELECTED_WEAPON_SLOT, &[2]);
        m.set(ped + off::PED_WEAPONS + 2 * off::WEAPON_SIZE, &24u32.to_le_bytes());
        m.set(PAD0 + 2, &(-128i16).to_le_bytes());
        m.set(PAD0 + 0x22, &255i16.to_le_bytes()); // circle
        let s = read_local_sample(&m, ped).unwrap();
        assert_eq!(s.position, [10.0, 0.0, 0.0]);
        assert_eq!(s.pad.left_stick_y, -128);
        assert_eq!((s.weapon, s.special_action, s.health), (24, 1, 100.0));
        assert!(!s.key_ctx.can_fire, "deagle without ammo");
        assert_eq!(s.move_speed, [0.25, 0.0, 0.0]);
    }

    #[test]
    fn snapshot_without_ped() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0100_0000);
        m.set(G_GAME_STATE, &7i32.to_le_bytes());
        let s = snapshot(&m).unwrap();
        assert_eq!(format_state(&s, 0), "state game_state=7 (FRONTEND_IDLE) ticks=0 ped=none");
    }
}
