//! The curated list of gta_sa.exe 1.0 US functions the tracer hooks.
//!
//! Each entry has the exact prologue bytes that are moved into the
//! trampoline (whole instructions, ≥ 5 bytes). Tests check them against
//! `re/gta/1.0-US/gta_sa.exe`, against the x86 decoder, and against every
//! site samp.dll 0.3.7-R4 patches or reads (gta-patches-r4-annotated.csv and
//! the 707-entry model-pointer table), so the hooks never collide with the
//! original client. At runtime the bytes are compared again before patching.
//!
//! Names and signatures are named via gta-reversed (`RH_ScopedInstall` lines)
//! or plugin-sdk (`Converted from …` comments).

use crate::gta;
use crate::json::Val;
use crate::mem::Mem;

/// When a call is written to the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Every call (rate-capped per frame).
    Always,
    /// Only if the caller is a client module (samp.dll / samp_client.dll), the
    /// call happens inside a script command, or the local player ped is involved.
    Relevant,
    /// Never written; counted per frame and reported as on/off in `watch`.
    Count,
}

/// Extra behaviour beyond logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    None,
    /// Per-frame boundary (flush, watch diff, frame tick count).
    Frame,
    /// `CRunningScript::ProcessOneCommand`: script-command context + decoding.
    /// On this exe only samp-style callers use it: the stock interpreter loop
    /// in `CRunningScript::Process` has it inlined.
    Scm,
    /// `CRunningScript::Process`: one stock script's turn (context for nested calls).
    StockScript,
    /// `CFileMgr` file access, wrapped so file contents can be captured
    /// (`SAMP_TRACER_CAPTURE_DIR`); otherwise a plain pass-through.
    FileOpen,
    FileRead,
    FileReadLine,
    FileSeek,
    FileClose,
    FileLoad,
    /// `CPlayerPed::ProcessControl`: wrapped so the swapped game context
    /// (PlayerInFocus, pad, camera mode, …) and the ped's position/speed can be
    /// sampled at entry and exit (remote players, TRACE-2).
    PedProcessControl,
    /// `CWorld::Process`: wrapped so remote player peds can be sampled before
    /// and after the world (tasks, physics) runs.
    WorldProcess,
    /// `CAutomobile::ProcessControl`: wrapped so the context the client swaps in for a
    /// remote driver (PlayerInFocus, pad, camera) and the car's state can be sampled (TRACE-4).
    VehProcessControl,
    /// GTA's window procedure (`__MainWndProc`): only keyboard messages (0x100–0x109) are
    /// written, to see which keys a client lets through to the game (TRACE-3).
    WndProc,
}

/// How to render one argument. Stack slots count from the first argument
/// (`[esp+4]` at entry); `this` is ECX for thiscall methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    ThisEntity,
    ThisScript,
    I32(usize, &'static str),
    Bool(usize, &'static str),
    F32(usize, &'static str),
    CStr(usize, &'static str),
    /// Pointer to 3 floats.
    VecPtr(usize, &'static str),
    /// 3 floats passed by value (3 slots).
    VecVal(usize, &'static str),
    Entity(usize, &'static str),
    /// Pointer into `CTheScripts::ScriptSpace`, logged as an offset.
    ScriptIp(usize, &'static str),
    /// `this` is a `CTaskManager`: its owner ped (`m_pPed`, +0x2C).
    TaskOwner,
    /// Pointer to a C++ object: logged as its vtable address (identifies the class).
    Vtable(usize, &'static str),
    /// A pointer value kept in diffs (e.g. to see whether a buffer is static).
    Addr(usize, &'static str),
}

/// `CTaskManager::m_pPed` (5 primary + 6 secondary task pointers precede it; gta-reversed TaskManager.h).
pub const TASK_MANAGER_PED: u32 = 0x2C;

pub struct HookSpec {
    pub name: &'static str,
    pub addr: u32,
    pub prologue: &'static [u8],
    pub policy: Policy,
    pub special: Special,
    pub args: &'static [Arg],
}

const fn h(name: &'static str, addr: u32, prologue: &'static [u8], policy: Policy, args: &'static [Arg]) -> HookSpec {
    HookSpec { name, addr, prologue, policy, special: Special::None, args }
}

use Arg::*;
use Policy::*;

pub static HOOKS: &[HookSpec] = &[
    // --- frame / scripts --------------------------------------------------------------
    HookSpec {
        name: "CGame::Process",
        addr: 0x53BEE0,
        prologue: &[0x83, 0xEC, 0x0C, 0x53, 0x56],
        policy: Count,
        special: Special::Frame,
        args: &[],
    },
    HookSpec {
        name: "CRunningScript::ProcessOneCommand",
        addr: 0x469EB0,
        prologue: &[0x66, 0xFF, 0x05, 0xF4, 0x47, 0xA4, 0x00],
        policy: Count,
        special: Special::Scm,
        args: &[],
    },
    HookSpec {
        name: "CRunningScript::Process",
        addr: 0x469F00,
        prologue: &[0x53, 0x56, 0x8B, 0xF1, 0x8B, 0x86, 0xD8, 0x00, 0x00, 0x00],
        policy: Count,
        special: Special::StockScript,
        args: &[],
    },
    h("CTheScripts::Process", 0x46A000, &[0xA0, 0x88, 0x30, 0xA4, 0x00], Count, &[]),
    h("CTheScripts::Init", 0x468D50, &[0x83, 0xEC, 0x18, 0x53, 0x56], Always, &[]),
    h("CTheScripts::StartNewScript", 0x464C20, &[0xE9, 0x7B, 0x56, 0x10, 0x01], Always, &[ScriptIp(0, "ip")]),
    h(
        "CTheScripts::StartNewScript(indexed)",
        0x464C90,
        &[0xE9, 0x7B, 0x95, 0x10, 0x01],
        Always,
        &[ScriptIp(0, "ip"), I32(1, "index")],
    ),
    h("CTheScripts::StartTestScript", 0x464D40, &[0xE9, 0xCB, 0x08, 0x10, 0x01], Always, &[]),
    h(
        "CRunningScript::ShutdownThisScript",
        0x465AA0,
        &[0x6A, 0xFF, 0x68, 0xFB, 0x7C, 0x83, 0x00],
        Always,
        &[ThisScript],
    ),
    // --- game (re)initialisation ----------------------------------------------------------
    h(
        "CGame::InitialiseWhenRestarting",
        0x53C680,
        &[0x83, 0xEC, 0x08, 0x53, 0x68, 0xFF, 0x00, 0x00, 0x00],
        Always,
        &[],
    ),
    h("CGame::ReInitGameObjectVariables", 0x53BCF0, &[0x53, 0x56, 0x57, 0xB9, 0x28, 0xF0, 0xB6, 0x00], Always, &[]),
    h("CGame::ShutDownForRestart", 0x53C550, &[0x56, 0x57, 0x6A, 0x00, 0xE8, 0x67, 0x45, 0x1F, 0x00], Always, &[]),
    HookSpec {
        name: "CFileMgr::OpenFile",
        addr: 0x538900,
        prologue: &[0xE9, 0xD3, 0xA9, 0x2E, 0x00],
        policy: Always,
        special: Special::FileOpen,
        args: &[CStr(0, "path"), CStr(1, "mode")],
    },
    HookSpec {
        name: "CFileMgr::Read",
        addr: 0x538950,
        prologue: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x0C],
        policy: Count,
        special: Special::FileRead,
        args: &[],
    },
    HookSpec {
        name: "CFileMgr::ReadLine",
        addr: 0x5389B0,
        prologue: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x0C],
        policy: Count,
        special: Special::FileReadLine,
        args: &[],
    },
    HookSpec {
        name: "CFileMgr::Seek",
        addr: 0x538990,
        prologue: &[0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x4C, 0x24, 0x08],
        policy: Count,
        special: Special::FileSeek,
        args: &[],
    },
    HookSpec {
        name: "CFileMgr::CloseFile",
        addr: 0x5389D0,
        prologue: &[0xE9, 0xB6, 0xA7, 0x2E, 0x00],
        policy: Count,
        special: Special::FileClose,
        args: &[],
    },
    HookSpec {
        name: "CFileMgr::LoadFile",
        addr: 0x538890,
        prologue: &[0x8B, 0x44, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x04],
        policy: Always,
        special: Special::FileLoad,
        args: &[CStr(0, "path"), I32(2, "size"), CStr(3, "mode")],
    },
    h(
        "CRestart::OverrideNextRestart",
        0x4607D0,
        &[0xE9, 0x1B, 0xB1, 0x10, 0x01],
        Always,
        &[VecPtr(0, "pos"), F32(1, "heading")],
    ),
    // --- player / peds ----------------------------------------------------------------------
    h(
        "CPlayerPed::CPlayerPed",
        0x60D5B0,
        &[0x6A, 0xFF, 0x68, 0x58, 0xED, 0x83, 0x00],
        Always,
        &[I32(0, "player"), Bool(1, "group_created")],
    ),
    HookSpec {
        name: "CPlayerPed::ProcessControl",
        addr: 0x60EA90,
        prologue: &[0x83, 0xEC, 0x34, 0x53, 0x55],
        policy: Count,
        special: Special::PedProcessControl,
        args: &[],
    },
    HookSpec {
        name: "CWorld::Process",
        addr: 0x5684A0,
        prologue: &[0x56, 0x8B, 0x35, 0xC8, 0xAC, 0xB9, 0x00],
        policy: Count,
        special: Special::WorldProcess,
        args: &[],
    },
    HookSpec {
        name: "CAutomobile::ProcessControl",
        addr: 0x6B1880,
        prologue: &[0x81, 0xEC, 0xD8, 0x00, 0x00, 0x00],
        policy: Count,
        special: Special::VehProcessControl,
        args: &[],
    },
    h(
        "CTaskManager::SetTask",
        0x681AF0,
        &[0x53, 0x8B, 0x5C, 0x24, 0x08],
        Relevant,
        &[TaskOwner, Vtable(0, "task_vt"), I32(1, "index")],
    ),
    h(
        "CTaskManager::SetTaskSecondary",
        0x681B60,
        &[0x53, 0x8B, 0x5C, 0x24, 0x08],
        Relevant,
        &[TaskOwner, Vtable(0, "task_vt"), I32(1, "index")],
    ),
    h("CPlayerPed::SetupPlayerPed", 0x60D790, &[0x6A, 0xFF, 0x68, 0x7B, 0xED, 0x83, 0x00], Always, &[I32(0, "player")]),
    h("CPlayerPed::RemovePlayerPed", 0x6094A0, &[0x56, 0x8B, 0x74, 0x24, 0x08], Always, &[I32(0, "player")]),
    h(
        "CPlayerPed::DeactivatePlayerPed",
        0x609520,
        &[0x8B, 0x44, 0x24, 0x04, 0x69, 0xC0, 0x90, 0x01, 0x00, 0x00],
        Always,
        &[I32(0, "player")],
    ),
    h(
        "CPlayerPed::ReactivatePlayerPed",
        0x609540,
        &[0x8B, 0x44, 0x24, 0x04, 0x69, 0xC0, 0x90, 0x01, 0x00, 0x00],
        Always,
        &[I32(0, "player")],
    ),
    h("CPed::CPed", 0x5E8030, &[0x6A, 0xFF, 0x68, 0x03, 0xDA, 0x83, 0x00], Always, &[I32(0, "ped_type")]),
    h("CPed::~CPed", 0x5E8620, &[0x6A, 0xFF, 0xE9, 0x45, 0x8D, 0xE1, 0xFF], Always, &[ThisEntity]),
    h(
        "CPed::SetModelIndex",
        0x5E4880,
        &[0x56, 0x8B, 0xF1, 0x81, 0x4E, 0x1C, 0x80, 0x00, 0x00, 0x00],
        Always,
        &[ThisEntity, I32(0, "model")],
    ),
    h("CPed::SetPedState", 0x5E4500, &[0x53, 0x8B, 0x5C, 0x24, 0x08], Relevant, &[ThisEntity, I32(0, "state")]),
    h("CPed::ClearAll", 0x5E5320, &[0x8B, 0x81, 0x6C, 0x04, 0x00, 0x00], Relevant, &[ThisEntity]),
    h(
        "CPed::Teleport",
        0x5E4110,
        &[0x56, 0x8B, 0xF1, 0x8B, 0x86, 0x98, 0x05, 0x00, 0x00],
        Always,
        &[ThisEntity, VecVal(0, "pos"), Bool(3, "reset_rotation")],
    ),
    h(
        "CPed::GiveWeapon",
        0x5E6080,
        &[0x51, 0x53, 0x8B, 0x5C, 0x24, 0x0C],
        Always,
        &[ThisEntity, I32(0, "weapon"), I32(1, "ammo"), Bool(2, "flag")],
    ),
    h("CPed::ClearWeapons", 0x5E6320, &[0x53, 0x56, 0x57, 0x6A, 0xFF], Always, &[ThisEntity]),
    h("CPed::SetCurrentWeapon(slot)", 0x5E61F0, &[0x53, 0x8B, 0x5C, 0x24, 0x08], Always, &[ThisEntity, I32(0, "slot")]),
    h(
        "CPed::SetCurrentWeapon(type)",
        0x5E6280,
        &[0x8B, 0x44, 0x24, 0x04, 0x56],
        Always,
        &[ThisEntity, I32(0, "weapon")],
    ),
    h(
        "CPlayerInfo::MakePlayerSafe",
        0x56E870,
        &[0x83, 0xEC, 0x0C, 0x56, 0x8B, 0xF1],
        Always,
        &[Bool(0, "enable"), F32(1, "radius")],
    ),
    h("CPlayerInfo::SetPlayerSkin", 0x5717F0, &[0x8B, 0x44, 0x24, 0x04, 0x56], Always, &[CStr(0, "skin")]),
    h("CPlayerInfo::LoadPlayerSkin", 0x56F7D0, &[0x56, 0x8B, 0xF1, 0x8B, 0x86, 0x84, 0x01, 0x00, 0x00], Always, &[]),
    // --- world / placement --------------------------------------------------------------------
    h("CWorld::Add", 0x563220, &[0x56, 0x8B, 0x74, 0x24, 0x08], Relevant, &[Entity(0, "entity")]),
    h("CWorld::Remove", 0x563280, &[0x56, 0x8B, 0x74, 0x24, 0x08], Relevant, &[Entity(0, "entity")]),
    h(
        "CPlaceable::SetPosn(xyz)",
        0x420B80,
        &[0xE9, 0xAB, 0x62, 0x14, 0x01],
        Relevant,
        &[ThisEntity, F32(0, "x"), F32(1, "y"), F32(2, "z")],
    ),
    h(
        "CPlaceable::SetPosn(vec)",
        0x4241C0,
        &[0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08],
        Relevant,
        &[ThisEntity, VecPtr(0, "pos")],
    ),
    h("CPlaceable::SetHeading", 0x43E0C0, &[0x8B, 0xC1, 0x8B, 0x48, 0x14], Relevant, &[ThisEntity, F32(0, "heading")]),
    // --- streaming ------------------------------------------------------------------------------
    h(
        "CStreaming::RequestModel",
        0x4087E0,
        &[0x53, 0x8B, 0x5C, 0x24, 0x0C],
        Relevant,
        &[I32(0, "model"), I32(1, "flags")],
    ),
    h("CStreaming::RemoveModel", 0x4089A0, &[0x56, 0x8B, 0x74, 0x24, 0x08], Relevant, &[I32(0, "model")]),
    h(
        "CStreaming::LoadAllRequestedModels",
        0x40EA10,
        &[0xE9, 0x49, 0xBA, 0xFF, 0xFF],
        Relevant,
        &[Bool(0, "only_priority")],
    ),
    h(
        "CStreaming::LoadScene",
        0x40EB70,
        &[0x83, 0xEC, 0x1C, 0x55, 0x8B, 0x6C, 0x24, 0x24],
        Always,
        &[VecPtr(0, "pos")],
    ),
    // --- camera ---------------------------------------------------------------------------------
    h("CCamera::Restore", 0x50B930, &[0x53, 0x33, 0xDB, 0x56, 0x8B, 0xF1], Always, &[]),
    h("CCamera::RestoreWithJumpCut", 0x50BAB0, &[0x53, 0x33, 0xDB, 0x56, 0x8B, 0xF1], Always, &[]),
    h(
        "CCamera::SetCameraDirectlyBehindForFollowPed_CamOnAString",
        0x50BD40,
        &[0x56, 0x8B, 0xF1, 0x6A, 0xFF],
        Always,
        &[],
    ),
    h(
        "CCamera::TakeControl",
        0x50C7C0,
        &[0x56, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x14],
        Always,
        &[Entity(0, "target"), I32(1, "mode"), I32(2, "switch"), I32(3, "who")],
    ),
    h("CCamera::SetWideScreenOn", 0x50C140, &[0xC6, 0x41, 0x3D, 0x01, 0xC6, 0x41, 0x3C, 0x00], Always, &[]),
    h("CCamera::SetWideScreenOff", 0x50C150, &[0x80, 0x79, 0x3D, 0x01, 0x0F, 0x94, 0xC0], Always, &[]),
    // --- single-player systems: ambient population, traffic, police (TRACE-4) -------------------
    // Per-frame managers are `Count` (on/off in `watch`); anything that creates a ped/vehicle,
    // changes the wanted level or plays a police-scanner report is logged on every call.
    // CCarCtrl::GenerateOneRandomCar (0x430050) and GenerateOneEmergencyServicesCar (0x42B7D0)
    // are not hooked: the exe's obfuscated stubs read their entry bytes as data (0x403796 reads
    // 0x42B7D3; a hook there crashed SET_PLAYER_CONTROL); vehicle creation is covered by the
    // CVehicle constructor and the per-frame generator flags.
    // CPopulation::AddPed (0x612710) itself cannot be hooked: samp rewrites its entry (CSV row 185);
    // ped creation is covered by the CPed / CCopPed constructors (CCivilianPed's ctor can't be
    // hooked: a rel32 at 0x5D1A03 points into its first 7 bytes).
    h("CCopPed::CCopPed", 0x5DDC60, &[0x6A, 0xFF, 0x68, 0x88, 0xD7, 0x83, 0x00], Always, &[I32(0, "cop_type")]),
    h("CVehicle::CVehicle", 0x6D5F10, &[0x6A, 0xFF, 0x68, 0x06, 0x87, 0x84, 0x00], Always, &[I32(0, "created_by")]),
    h("CPopulation::AddToPopulation", 0x614720, &[0x6A, 0xFF, 0x68, 0x8E, 0xEF, 0x83, 0x00], Count, &[]),
    h("CPopulation::AddPedInCar", 0x613A00, &[0x83, 0xEC, 0x18, 0x53, 0x56], Always, &[
        Entity(0, "vehicle"),
        Bool(1, "driver"),
    ]),
    h("CPopulation::AddPedAtAttractor", 0x614210, &[0x64, 0xA1, 0x00, 0x00, 0x00, 0x00], Always, &[I32(0, "model")]),
    h("CPopulation::CreateWaitingCoppers", 0x6133F0, &[0x64, 0xA1, 0x00, 0x00, 0x00, 0x00], Always, &[VecVal(
        0, "pos",
    )]),
    h("CPopulation::PopulateInterior", 0x616470, &[0xD9, 0x44, 0x24, 0x10, 0x83, 0xEC, 0x24], Always, &[
        I32(0, "num_peds"),
        VecVal(1, "pos"),
    ]),
    h("CPopulation::Update", 0x616650, &[0x0F, 0xBF, 0x05, 0x14, 0x13, 0xC8, 0x00], Count, &[]),
    h("CPopulation::GeneratePedsAtAttractors", 0x615970, &[0x81, 0xEC, 0x2C, 0x08, 0x00, 0x00], Count, &[]),
    h("CPopulation::ManagePopulation", 0x616190, &[0x83, 0xEC, 0x1C, 0xA1, 0x90, 0x44, 0xB7, 0x00], Count, &[]),
    h(
        "CPopulation::ManageAllPopulation",
        0x6160A0,
        &[0x83, 0xEC, 0x0C, 0x0F, 0xB6, 0x05, 0x74, 0xCD, 0xB7, 0x00],
        Count,
        &[],
    ),
    h("CEntryExit::GenerateAmbientPeds", 0x43E8B0, &[0xA1, 0x14, 0x29, 0xB7, 0x00], Always, &[VecPtr(0, "pos")]),
    h("CCarCtrl::GenerateRandomCars", 0x4341C0, &[0xA0, 0x51, 0xF8, 0xB5, 0x00], Count, &[]),
    h("CCarCtrl::GenerateEmergencyServicesCar", 0x42F9C0, &[0xA0, 0x28, 0x5B, 0x8A, 0x00], Count, &[]),
    h("CTheCarGenerators::Process", 0x6F3F40, &[0x6A, 0xFF, 0xE8, 0x19, 0xA2, 0xE7, 0xFF], Count, &[]),
    h("CPlane::DoPlaneGenerationAndRemoval", 0x6CD2F0, &[0x64, 0xA1, 0x00, 0x00, 0x00, 0x00], Count, &[]),
    h("CTrain::DoTrainGenerationAndRemoval", 0x6F7900, &[0xB8, 0xF1, 0x19, 0x76, 0x05], Count, &[]),
    h("CRoadBlocks::GenerateRoadBlocks", 0x4629E0, &[0x6A, 0xFF, 0x68, 0x73, 0x7C, 0x83, 0x00], Count, &[]),
    h("CWanted::Update", 0x562C90, &[0x83, 0xEC, 0x14, 0x53, 0x55], Count, &[]),
    h("CWanted::SetWantedLevel", 0x562470, &[0xA0, 0x71, 0x91, 0x96, 0x00], Always, &[I32(0, "level")]),
    h("CWanted::SetWantedLevelNoDrop", 0x562570, &[0x56, 0x8B, 0xF1, 0x8B, 0x46, 0x30], Always, &[I32(0, "level")]),
    h("CWanted::RegisterCrime", 0x562410, &[0x8B, 0x44, 0x24, 0x10, 0x8B, 0x54, 0x24, 0x08], Always, &[
        I32(0, "crime"),
        VecPtr(1, "pos"),
    ]),
    h("CWanted::RegisterCrime_Immediately", 0x562430, &[0x8B, 0x44, 0x24, 0x0C, 0x53], Always, &[
        I32(0, "crime"),
        VecPtr(1, "pos"),
    ]),
    h("CWanted::UpdateWantedLevel", 0x561C90, &[0x51, 0xA1, 0xE8, 0xDE, 0x8C, 0x00], Always, &[]),
    h("CWanted::SetMaximumWantedLevel", 0x561E70, &[0x8B, 0x44, 0x24, 0x04, 0x83, 0xF8, 0x06], Always, &[I32(
        0, "level",
    )]),
    h("CAEPoliceScannerAudioEntity::AddAudioEvent", 0x4E71E0, &[0x83, 0xEC, 0x30, 0x8B, 0x44, 0x24, 0x34], Always, &[
        I32(0, "event"),
        I32(1, "crime"),
        VecPtr(2, "pos"),
    ]),
    // --- UI (TRACE-3) ------------------------------------------------------------------
    h("CMessages::AddBigMessage", 0x69F2B0, &[0x81, 0xEC, 0x20, 0x03, 0x00, 0x00], Always, &[
        CStr(0, "text"),
        Addr(0, "text_addr"),
        I32(1, "time"),
        I32(2, "style"),
    ]),
    HookSpec {
        name: "__MainWndProc",
        addr: 0x747EB0,
        prologue: &[0x83, 0xEC, 0x3C, 0x53, 0x55],
        policy: Always,
        special: Special::WndProc,
        args: &[I32(1, "msg"), I32(2, "wparam"), Addr(3, "lparam")],
    },
    h(
        "CAEPoliceScannerAudioEntity::PlayPoliceScannerDialogue",
        0x4E6ED0,
        &[0x66, 0x83, 0x3D, 0x4C, 0x1D, 0xB6, 0x00, 0x00],
        Always,
        &[],
    ),
];

/// Register/stack view of a hooked call at function entry.
#[derive(Debug, Clone, Copy, Default)]
pub struct Call {
    pub ecx: u32,
    /// ESP at entry: points at the return address.
    pub esp: u32,
}

impl Call {
    pub fn slot_addr(&self, i: usize) -> u32 {
        self.esp + 4 + 4 * i as u32
    }
}

fn vec3<M: Mem>(m: &M, a: u32) -> Val {
    match (m.f32(a), m.f32(a + 4), m.f32(a + 8)) {
        (Some(x), Some(y), Some(z)) => Val::List(vec![x.into(), y.into(), z.into()]),
        _ => Val::Null,
    }
}

/// Renders the arguments of `spec` for one call into a JSON object.
pub fn render_args<M: Mem>(m: &M, spec: &HookSpec, c: &Call) -> Val {
    let mut v = Val::obj();
    for a in spec.args {
        match *a {
            ThisEntity => v.put("this", gta::describe_entity(m, c.ecx)),
            ThisScript => {
                v.put("script", m.cstr(c.ecx + gta::script::NAME, 8));
                v.put("this_ptr", Val::Hex(c.ecx));
            }
            I32(i, k) => v.put(k, m.i32(c.slot_addr(i))),
            Bool(i, k) => v.put(k, m.u8(c.slot_addr(i)).map(|b| b != 0)),
            F32(i, k) => v.put(k, m.f32(c.slot_addr(i))),
            CStr(i, k) => {
                let p = m.u32(c.slot_addr(i)).unwrap_or(0);
                v.put(k, if p == 0 { None } else { m.cstr(p, 260) });
            }
            VecPtr(i, k) => {
                let p = m.u32(c.slot_addr(i)).unwrap_or(0);
                v.put(k, if p == 0 { Val::Null } else { vec3(m, p) });
            }
            VecVal(i, k) => v.put(k, vec3(m, c.slot_addr(i))),
            Entity(i, k) => v.put(k, gta::describe_entity(m, m.u32(c.slot_addr(i)).unwrap_or(0))),
            TaskOwner => v.put("ped", gta::describe_entity(m, m.u32(c.ecx + TASK_MANAGER_PED).unwrap_or(0))),
            Vtable(i, k) => {
                let p = m.u32(c.slot_addr(i)).unwrap_or(0);
                v.put(k, if p == 0 { Val::Null } else { m.u32(p).map(Val::Hex).unwrap_or(Val::Null) });
            }
            Addr(i, k) => v.put(k, m.u32(c.slot_addr(i)).map(Val::Hex).unwrap_or(Val::Null)),
            ScriptIp(i, k) => {
                let p = m.u32(c.slot_addr(i)).unwrap_or(0);
                let off = p.wrapping_sub(gta::SCRIPT_SPACE);
                // main.scm + mission space are < 0x40000 bytes; anything else is outside ScriptSpace.
                if off < 0x0010_0000 {
                    v.put(&format!("{k}_offset"), off);
                } else {
                    v.put(&format!("{k}_ptr"), Val::Hex(p));
                }
            }
        }
    }
    v
}

/// True if a player ped (the local one or a remote player's, i.e. any
/// `CWorld::Players[slot].m_pPed`) is `this`, an entity argument or the task owner.
pub fn involves_player<M: Mem>(m: &M, spec: &HookSpec, c: &Call) -> bool {
    let is_player = |p: Option<u32>| p.is_some_and(|p| gta::player_slot_of(m, p).is_some());
    spec.args.iter().any(|a| match *a {
        ThisEntity => is_player(Some(c.ecx)),
        Entity(i, _) => is_player(m.u32(c.slot_addr(i))),
        TaskOwner => is_player(m.u32(c.ecx + TASK_MANAGER_PED)),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;
    use crate::x86;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Minimal PE reader: VA → bytes.
    struct Pe {
        data: Vec<u8>,
        secs: Vec<(u32, u32, u32)>, // va, size, file offset
    }
    impl Pe {
        fn open(p: &Path) -> Option<Pe> {
            let data = std::fs::read(p).ok()?;
            let rd = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
            let rd16 = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap()) as usize;
            let e = rd(0x3C) as usize;
            let n = rd16(e + 6);
            let opt = rd16(e + 20);
            let base = rd(e + 24 + 28);
            let mut secs = Vec::new();
            for i in 0..n {
                let o = e + 24 + opt + 40 * i;
                let (va, raw_size, raw_off) = (rd(o + 12), rd(o + 16), rd(o + 20));
                secs.push((base + va, raw_size, raw_off));
            }
            Some(Pe { data, secs })
        }
        fn bytes(&self, va: u32, n: usize) -> Option<&[u8]> {
            let &(sva, _, off) = self.secs.iter().find(|&&(sva, size, _)| va >= sva && va < sva + size)?;
            let start = (off + va - sva) as usize;
            self.data.get(start..start + n)
        }
    }

    #[test]
    fn table_is_consistent() {
        let mut seen = HashSet::new();
        for s in HOOKS {
            assert!(seen.insert(s.addr), "{} hooked twice", s.name);
            let n = x86::steal_len(s.prologue, 5).unwrap_or_else(|e| panic!("{}: {e}", s.name));
            assert_eq!(n, s.prologue.len(), "{}: prologue must be exactly the stolen instructions", s.name);
        }
        // Hooks must not overlap each other.
        let mut r: Vec<_> = HOOKS.iter().map(|s| (s.addr, s.addr + s.prologue.len() as u32, s.name)).collect();
        r.sort();
        for w in r.windows(2) {
            assert!(w[0].1 <= w[1].0, "{} overlaps {}", w[0].2, w[1].2);
        }
    }

    #[test]
    fn prologues_match_reference_exe() {
        let Some(pe) = Pe::open(&root().join("re/gta/1.0-US/gta_sa.exe")) else {
            eprintln!("re/gta/1.0-US/gta_sa.exe not present; skipping");
            return;
        };
        for s in HOOKS {
            let got = pe.bytes(s.addr, 16).unwrap_or_else(|| panic!("{} outside the image", s.name));
            assert_eq!(&got[..s.prologue.len()], s.prologue, "{} at {:#x}", s.name, s.addr);
            // The decoder agrees on the instruction boundary in the real code.
            assert_eq!(x86::steal_len(got, 5).unwrap(), s.prologue.len(), "{}", s.name);
        }
    }

    /// Sites samp.dll writes or reads in gta_sa.exe: (addr, len).
    fn samp_sites() -> Vec<(u32, u32, String)> {
        let mut out = Vec::new();
        let csv = std::fs::read_to_string(root().join("docs/re/data/gta-patches-r4-annotated.csv")).unwrap();
        for line in csv.lines().skip(1) {
            // gta_addr is the first field, possibly quoted with several addresses.
            let first = if let Some(rest) = line.strip_prefix('"') {
                rest.split('"').next().unwrap_or("")
            } else {
                line.split(',').next().unwrap_or("")
            };
            let size: u32 = if line.starts_with('"') {
                line.split('"').nth(2).and_then(|r| r.split(',').nth(1)).and_then(|s| s.parse().ok()).unwrap_or(4)
            } else {
                line.split(',').nth(1).and_then(|s| s.parse().ok()).unwrap_or(4)
            };
            for tok in first.split(|c: char| !c.is_ascii_hexdigit() && c != 'x') {
                if let Some(h) = tok.strip_prefix("0x")
                    && let Ok(a) = u32::from_str_radix(h, 16)
                {
                    out.push((a, size.max(4), line.chars().take(60).collect()));
                }
            }
        }
        // Auth handler reads 20 bytes at 0x4D3AA0 (gta-patches.md §4).
        out.push((0x4D3AA0, 20, "auth read".into()));
        // samp-client's extra CdStreamSync patch (not in the CSV).
        out.push((0x156CDAE, 6, "samp-client cdstream".into()));
        out
    }

    #[test]
    fn no_overlap_with_samp_patch_sites() {
        let sites = samp_sites();
        assert!(sites.len() > 300, "CSV parse produced {} sites", sites.len());
        for s in HOOKS {
            let (a, b) = (s.addr, s.addr + s.prologue.len() as u32);
            for (sa, sl, what) in &sites {
                assert!(
                    *sa + sl <= a || *sa >= b,
                    "{} [{a:#x},{b:#x}) overlaps samp site {sa:#x}+{sl}: {what}",
                    s.name
                );
            }
        }
    }

    #[test]
    fn no_overlap_with_samp_model_pointer_table() {
        // 707 records {u32 code address, u8 opcode} at samp.dll A .data 0x10114B10 (gta-patches.md §4.4).
        let p = root().join("re/samp-dll/0.3.7-R4_fv0.3.7.4_2021-05-07/samp.dll");
        let Some(pe) = Pe::open(&p) else {
            eprintln!("{} not present; skipping", p.display());
            return;
        };
        let t = pe.bytes(0x1011_4B10, 707 * 5).unwrap();
        for r in t.chunks(5) {
            let a = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
            let imm = a + if matches!(r[4], 0x8B | 0x89 | 0x39) { 3 } else { 1 };
            for s in HOOKS {
                let (ha, hb) = (s.addr, s.addr + s.prologue.len() as u32);
                assert!(imm + 4 <= ha || imm >= hb, "{} overlaps model-pointer site {a:#x}", s.name);
            }
        }
    }

    #[test]
    fn no_branch_into_stolen_bytes() {
        // Any rel32 call/jmp in the image that lands strictly inside a stolen range would break.
        let Some(pe) = Pe::open(&root().join("re/gta/1.0-US/gta_sa.exe")) else { return };
        let ranges: Vec<_> = HOOKS.iter().map(|s| (s.addr + 1, s.addr + s.prologue.len() as u32, s.name)).collect();
        for &(sva, size, off) in &pe.secs {
            if !(0x401000..0x1600000).contains(&sva) {
                continue;
            }
            let code = &pe.data[off as usize..(off + size) as usize];
            for i in 0..code.len().saturating_sub(5) {
                if code[i] == 0xE8 || code[i] == 0xE9 {
                    let rel = i32::from_le_bytes(code[i + 1..i + 5].try_into().unwrap());
                    let t = x86::rel32_target(sva + i as u32, 5, rel);
                    for &(a, b, name) in &ranges {
                        assert!(!(a..b).contains(&t), "rel32 at {:#x} targets {t:#x} inside {name}", sva + i as u32);
                    }
                }
            }
        }
    }

    #[test]
    fn no_absolute_reference_into_stolen_bytes() {
        // The exe's protection stubs read code bytes as data (e.g. 0x403796 reads 0x42B7D3), so
        // no 32-bit value anywhere in the image may point strictly inside a stolen range
        // (pointers to the function start, e.g. vtable entries, are fine).
        let Ok(data) = std::fs::read(root().join("re/gta/1.0-US/gta_sa.exe")) else { return };
        let mut r: Vec<_> = HOOKS.iter().map(|s| (s.addr + 1, s.addr + s.prologue.len() as u32, s.name)).collect();
        r.sort();
        for w in data.windows(4) {
            let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            let i = r.partition_point(|&(a, _, _)| a <= v);
            if i > 0 {
                let (a, b, name) = r[i - 1];
                assert!(!(a..b).contains(&v), "a 32-bit value {v:#x} in the image points inside {name}");
            }
        }
    }

    #[test]
    fn task_and_remote_ped_args() {
        let mut m = FakeMem::default();
        let (esp, mgr, task, vt) = (0x10_0000u32, 0x30_0000u32, 0x31_0000u32, 0x0086_DD2Cu32);
        // A remote ped in slot 7 (unique pointer: the slot cache is process-wide).
        let ped = 0x7700_0000u32;
        m.set_u32(gta::player_info(7), ped);
        m.set(ped + gta::off::ENTITY_TYPE, &[3]);
        m.set(ped + gta::off::ENTITY_MODEL, &0i16.to_le_bytes());
        m.set_u32(mgr + TASK_MANAGER_PED, ped);
        m.set_u32(task, vt);
        m.set_u32(esp + 4, task);
        m.set_u32(esp + 8, 4);
        let spec = HOOKS.iter().find(|s| s.name == "CTaskManager::SetTask").unwrap();
        let c = Call { ecx: mgr, esp };
        assert!(!involves_player(&m, spec, &c), "not a player ped until the cache knows the slot");
        gta::set_remote_peds(gta::scan_player_peds(&m).into_iter().filter(|&(s, _)| s != 0).collect());
        assert!(involves_player(&m, spec, &c));
        assert_eq!(
            render_args(&m, spec, &c).to_json(),
            r#"{"ped":{"type":"ped","model":0,"player":false,"slot":7,"ptr":"0x77000000"},"task_vt":"0x86dd2c","index":4}"#
        );
    }

    #[test]
    fn renders_args() {
        let mut m = FakeMem::default();
        let esp = 0x10_0000;
        let ped = 0x20_0000;
        m.set_u32(gta::CWORLD_PLAYERS, ped);
        m.set(ped + gta::off::ENTITY_TYPE, &[3]);
        m.set(ped + gta::off::ENTITY_MODEL, &0i16.to_le_bytes());
        m.set_f32(esp + 4, 1.0);
        m.set_f32(esp + 8, 2.0);
        m.set_f32(esp + 12, 3.5);
        let spec = HOOKS.iter().find(|s| s.name == "CPlaceable::SetPosn(xyz)").unwrap();
        let c = Call { ecx: ped, esp };
        assert_eq!(
            render_args(&m, spec, &c).to_json(),
            r#"{"this":{"type":"ped","model":0,"player":true,"ptr":"0x200000"},"x":1.000,"y":2.000,"z":3.500}"#
        );
        assert!(involves_player(&m, spec, &c));
        let spec = HOOKS.iter().find(|s| s.name == "CTheScripts::StartNewScript").unwrap();
        m.set_u32(esp + 4, gta::SCRIPT_SPACE + 0x1234);
        assert_eq!(render_args(&m, spec, &c).to_json(), r#"{"ip_offset":4660}"#);
    }
}
