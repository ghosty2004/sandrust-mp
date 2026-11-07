//! The table of gta_sa.exe byte patches samp.dll applies at start-up, minus the ones done elsewhere.
//!
//! Each [`Entry`] carries `csv_row`, its "patch table row": the 1-based line number in
//! docs/re/data/gta-patches-r4-annotated.csv (header = line 1), where its purpose and the samp.dll site are
//! recorded; docs/re/subsystems/gta-patches.md explains them. `original` holds the bytes of the unmodified
//! 1.0 US executable, and a patch is only written while memory still holds them ([`apply_phase`]).
//! Table-driven elsewhere: the sector relocation (`sectors.rs`), the model-info stores (`stores.rs`), the
//! vehicle patches and colour table (`vehpatches.rs`) and the world-entity hooks (`worldhooks.rs`).

use crate::patch::{Memory, PatchError, Patcher};

/// When an entry is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// At DLL attach, while the main thread is still suspended (gta-patches.md, "2. When patches are applied",
    /// step 2).
    Early,
    /// Once `gGameState == 7` (FRONTEND_IDLE), before switching to state 8 (same section, step 3).
    Main,
    /// At attach, only when files are served from samp.saa: samp's default.dat registers the image
    /// archives itself.
    EarlyArchive,
}

/// What an entry writes.
#[derive(Debug, Clone, Copy)]
pub enum Write {
    Bytes(&'static [u8]),
    Nop(usize),
    /// imm32 chosen from physical RAM ([`streaming_budget`]; gta-patches.md, "4.6 Streaming memory").
    StreamingBudget,
}

/// One patch: site, phase, new bytes, the bytes expected before patching and its patch table row.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub addr: u32,
    pub phase: Phase,
    pub write: Write,
    /// Bytes expected before patching (from gta_sa.exe 1.0 US).
    pub original: &'static [u8],
    /// Patch table row: line number in gta-patches-r4-annotated.csv (header = line 1).
    pub csv_row: u32,
    pub what: &'static str,
}

impl Entry {
    pub fn size(&self) -> usize {
        match self.write {
            Write::Bytes(b) => b.len(),
            Write::Nop(n) => n,
            Write::StreamingBudget => 4,
        }
    }
}

const TIMER_QPC_BYPASS: [u8; 29] = {
    let mut b = [0x90u8; 29];
    b[0] = 0x33; // xor eax,eax
    b[1] = 0xC0;
    b
};

pub const TABLE: &[Entry] = &[
    // psInitialize: NOP `mov [gGameState],esi` so the state set at
    // attach (5, written separately) survives (gta-patches.md, "Startup / intro").
    Entry {
        addr: 0x747483,
        phase: Phase::Early,
        write: Write::Nop(6),
        original: &[0x89, 0x35, 0xC0, 0xD4, 0xC8, 0x00],
        csv_row: 121,
        what: "psInitialize: keep gGameState",
    },
    // GetVideoModeList: drop the aspect-ratio filter (68 bytes).
    Entry {
        addr: 0x745B87,
        phase: Phase::Early,
        write: Write::Nop(68),
        original: &[
            0xd9, 0x44, 0x24, 0x08, 0xd8, 0x1d, 0x98, 0x8c, 0x85, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x44, 0x7b, 0x33, 0xd9,
            0x44, 0x24, 0x08, 0xd8, 0x1d, 0x34, 0x8f, 0x85, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x44, 0x7b, 0x22, 0xd9, 0x44,
            0x24, 0x08, 0xd8, 0x1d, 0xc8, 0x8c, 0x85, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x44, 0x7b, 0x11, 0xd9, 0x44, 0x24,
            0x08, 0xd8, 0x1d, 0xdc, 0x52, 0x86, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x44, 0x7a, 0x6d,
        ],
        csv_row: 124,
        what: "GetVideoModeList: no aspect filter",
    },
    // FillAvailableVMs: NOP one `jp` of the aspect test.
    Entry {
        addr: 0x7459E1,
        phase: Phase::Early,
        write: Write::Nop(2),
        original: &[0x7A, 0x71],
        csv_row: 125,
        what: "FillAvailableVMs: no aspect filter",
    },
    // CTimer::Initialise: always the millisecond timer path.
    Entry {
        addr: 0x561872,
        phase: Phase::Early,
        write: Write::Bytes(&TIMER_QPC_BYPASS),
        original: &[
            0x85, 0xc0, 0x74, 0x19, 0xb8, 0xd3, 0x4d, 0x62, 0x10, 0xf7, 0x64, 0x24, 0x00, 0xc1, 0xea, 0x06, 0xb9, 0xa0,
            0x17, 0x56, 0x00, 0x89, 0x15, 0x2c, 0xcb, 0xb7, 0x00, 0xeb, 0x0f,
        ],
        csv_row: 126,
        what: "CTimer::Initialise: no QPC",
    },
    // CStreaming::Init2: ms_memoryAvailable budget from physical RAM (see `streaming_budget`).
    Entry {
        addr: 0x5B8E6A,
        phase: Phase::Early,
        write: Write::StreamingBudget,
        original: &[0x00, 0x00, 0x20, 0x03],
        csv_row: 127,
        what: "CStreaming::Init2: streaming memory budget",
    },
    // CLoadingScreen::RenderSplash: NOP a CSprite2d::DrawRect call.
    Entry {
        addr: 0x590099,
        phase: Phase::Early,
        write: Write::Nop(5),
        original: &[0xE8, 0xC2, 0x7A, 0x19, 0x00],
        csv_row: 129,
        what: "RenderSplash: no DrawRect",
    },
    // Idle: frame-limiter busy-wait 14 ms -> 2 ms.
    Entry {
        addr: 0x53E94C,
        phase: Phase::Early,
        write: Write::Bytes(&[0x02]),
        original: &[0x0E],
        csv_row: 130,
        what: "Idle: frame limiter 2 ms",
    },
    // __MainWndProc: NOP the CGame::InitAfterLostFocus call.
    Entry {
        addr: 0x748063,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0xF8, 0x3B, 0xDF, 0xFF],
        csv_row: 160,
        what: "__MainWndProc: no InitAfterLostFocus",
    },
    // CGame::Process: NOP the CCutsceneMgr::Update call. Besides
    // removing cutscenes this keeps the stock main.scm from taking over the
    // player: its intro mission waits for a cutscene that never finishes
    // loading (see docs/phase3/m2-notes.md).
    Entry {
        addr: 0x53BF28,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0xD3, 0x9D, 0xF9, 0xFF],
        csv_row: 246,
        what: "CGame::Process: no CCutsceneMgr::Update",
    },
    // CUserDisplay::Process: NOP CPlaceName::Process (no zone names).
    Entry {
        addr: 0x5720A5,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0x76, 0xFE, 0xFF, 0xFF],
        csv_row: 157,
        what: "CUserDisplay::Process: no CPlaceName::Process",
    },
    // CHud::Draw: NOP CHud::DrawVehicleName.
    Entry {
        addr: 0x58FBE9,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0xB2, 0xB2, 0xFF, 0xFF],
        csv_row: 161,
        what: "CHud::Draw: no DrawVehicleName",
    },
    // CHud::SetHelpMessage := ret (no single-player help boxes).
    Entry {
        addr: 0x588BE0,
        phase: Phase::Main,
        write: Write::Bytes(&[0xC3]),
        original: &[0xA0],
        csv_row: 180,
        what: "CHud::SetHelpMessage: ret",
    },
    // CHud::DrawWanted: the shown wanted level is an immediate
    // (low byte at 0x58DB60) that SetPlayerWantedLevel (RPC 133) rewrites; 0 here.
    Entry {
        addr: 0x58DB5F,
        phase: Phase::Main,
        write: Write::Bytes(&[0xBD, 0x00, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90, 0x90]),
        original: &[0x83, 0xFE, 0x05, 0x0F, 0x84, 0xDA, 0x00, 0x00, 0x00],
        csv_row: 202,
        what: "CHud::DrawWanted: level from samp",
    },
    // CHud::Draw: NOP the vital-stats block.
    Entry {
        addr: 0x58FC2E,
        phase: Phase::Main,
        write: Write::Nop(30),
        original: &[
            0x6a, 0x00, 0x6a, 0xff, 0xe8, 0x99, 0xe4, 0xfd, 0xff, 0x83, 0xc4, 0x08, 0x85, 0xc0, 0x75, 0x0e, 0xc6, 0x05,
            0xde, 0xb1, 0xba, 0x00, 0x01, 0xe8, 0x06, 0x9a, 0xff, 0xff, 0xeb, 0x0c,
        ],
        csv_row: 205,
        what: "CHud::Draw: no vital stats",
    },
    // diMouseInit: cooperative level always non-exclusive|foreground.
    Entry {
        addr: 0x7469AF,
        phase: Phase::Main,
        write: Write::Bytes(&[0x33, 0xC0]),
        original: &[0x1B, 0xC0],
        csv_row: 209,
        what: "diMouseInit: non-exclusive mouse",
    },
    // ---- Remote players (remote-control-onfoot.md, "1.6 GTA patches that exist for remote `CPlayerPed`s"
    // and "7.1 Minimal hook and patch set for remote on-foot players") ----
    // CStreaming::DeleteAllRwObjects := RET (goes with the sector relocation:
    // samp's table would otherwise corrupt an instruction inside it, see sectors.rs).
    Entry {
        addr: 0x4090A0,
        phase: Phase::Main,
        write: Write::Bytes(&[0xC3]),
        original: &[0xa1],
        csv_row: 178,
        what: "CStreaming::DeleteAllRwObjects: ret",
    },
    // CPools::Initialise / CPlaceable::InitMatrixArray capacities that grow
    // with the number of peds (gta-patches.md, "Pools & limits"). CPools::Initialise runs at gGameState 8,
    // after this phase.
    Entry {
        addr: 0x550F46,
        phase: Phase::Main,
        write: Write::Bytes(&[0xa0, 0x86, 0x01, 0x00]),
        original: &[0x70, 0x11, 0x01, 0x00],
        csv_row: 110,
        what: "CPools::Initialise: PtrNode Single 100000",
    },
    Entry {
        addr: 0x550F82,
        phase: Phase::Main,
        write: Write::Bytes(&[0x40, 0x1f, 0x00, 0x00]),
        original: &[0x80, 0x0c, 0x00, 0x00],
        csv_row: 111,
        what: "CPools::Initialise: PtrNode Double 8000",
    },
    Entry {
        addr: 0x550FBA,
        phase: Phase::Main,
        write: Write::Bytes(&[0x88, 0x13, 0x00, 0x00]),
        original: &[0xf4, 0x01, 0x00, 0x00],
        csv_row: 112,
        what: "CPools::Initialise: EntryInfoNode 5000",
    },
    Entry {
        addr: 0x550FF2,
        phase: Phase::Main,
        write: Write::Bytes(&[0xF0]),
        original: &[0x8c],
        csv_row: 114,
        what: "CPools::Initialise: Peds 240",
    },
    Entry {
        addr: 0x551283,
        phase: Phase::Main,
        write: Write::Bytes(&[0xF0]),
        original: &[0x8c],
        csv_row: 115,
        what: "CPools::Initialise: PedIntelligence 240",
    },
    Entry {
        addr: 0x551140,
        phase: Phase::Main,
        write: Write::Bytes(&[0x05]),
        original: &[0x01],
        csv_row: 116,
        what: "CPools::Initialise: Task 1524",
    },
    Entry {
        addr: 0x551178,
        phase: Phase::Main,
        write: Write::Bytes(&[0x01]),
        original: &[0x00],
        csv_row: 117,
        what: "CPools::Initialise: Event 456",
    },
    Entry {
        addr: 0x54F3A1,
        phase: Phase::Main,
        write: Write::Bytes(&[0x70, 0x17, 0x00, 0x00]),
        original: &[0x84, 0x03, 0x00, 0x00],
        csv_row: 118,
        what: "CPlaceable::InitMatrixArray: 6000 matrices",
    },
    // CPlayerPed::CPlayerPed: inverted jcc, SetupPlayerPed(false) creates no ped group
    // (CPedGroups has only 8 groups).
    Entry {
        addr: 0x60D64E,
        phase: Phase::Main,
        write: Write::Bytes(&[0x84]),
        original: &[0x85],
        csv_row: 183,
        what: "CPlayerPed ctor: no ped group",
    },
    // CPlayerPed::ProcessControl: drop FindPlayerSlotWithPedPointer + Players[slot]
    // .m_pRemoteVehicle (−1 for remote peds → Players[-1]).
    Entry {
        addr: 0x60F2C4,
        phase: Phase::Main,
        write: Write::Nop(25),
        original: &[
            0x56, 0xe8, 0xd6, 0x4c, 0xf5, 0xff, 0x69, 0xc0, 0x90, 0x01, 0x00, 0x00, 0x8b, 0x88, 0x48, 0xce, 0xb7, 0x00,
            0x83, 0xc4, 0x04, 0x85, 0xc9, 0x75, 0x08,
        ],
        csv_row: 193,
        what: "CPlayerPed::ProcessControl: no FindPlayerSlotWithPedPointer",
    },
    // GetPlayerInfoForThisPlayerPed()->+0x153 uses removed (NULL for slots ≥ 2).
    Entry {
        addr: 0x5E63A6,
        phase: Phase::Main,
        write: Write::Nop(19),
        original: &[
            0xe8, 0x45, 0x3c, 0x02, 0x00, 0x8a, 0x88, 0x53, 0x01, 0x00, 0x00, 0x84, 0xc9, 0x0f, 0x84, 0xa8, 0x00, 0x00,
            0x00,
        ],
        csv_row: 173,
        what: "CPed::RemoveWeaponWhenEnteringVehicle: no player-info lookup",
    },
    Entry {
        addr: 0x621AEA,
        phase: Phase::Main,
        write: Write::Nop(12),
        original: &[0xe8, 0x01, 0x85, 0xfe, 0xff, 0xc6, 0x80, 0x53, 0x01, 0x00, 0x00, 0x00],
        csv_row: 174,
        what: "0x621960: no player-info lookup",
    },
    Entry {
        addr: 0x62D331,
        phase: Phase::Main,
        write: Write::Nop(11),
        original: &[0xe8, 0xba, 0xcc, 0xfd, 0xff, 0x88, 0x98, 0x53, 0x01, 0x00, 0x00],
        csv_row: 175,
        what: "CTaskComplexKillPedOnFootArmed::ControlSubTask: no player-info lookup",
    },
    Entry {
        addr: 0x741FFF,
        phase: Phase::Main,
        write: Write::Nop(27),
        original: &[
            0xe8, 0xec, 0x7f, 0xec, 0xff, 0x85, 0xc0, 0x0f, 0x84, 0xec, 0x02, 0x00, 0x00, 0x8a, 0x88, 0x53, 0x01, 0x00,
            0x00, 0x84, 0xc9, 0x0f, 0x84, 0xde, 0x02, 0x00, 0x00,
        ],
        csv_row: 176,
        what: "CVehicle::DoDriveByShootings: no player-info lookup",
    },
    // CPed::UpdatePosition: standing-on-entity block always skipped.
    Entry {
        addr: 0x5E1E70,
        phase: Phase::Main,
        write: Write::Bytes(&[0x33, 0xC0]),
        original: &[0x85, 0xc0],
        csv_row: 206,
        what: "CPed::UpdatePosition: ignore m_standingOnEntity",
    },
    // The m_fAimingRotation stores of CTaskSimplePlayerOnFoot; restored around the
    // local player's ProcessControl only (the remote.rs hook, patch table row 367).
    Entry {
        addr: 0x6884C4,
        phase: Phase::Main,
        write: Write::Nop(6),
        original: &[0xd9, 0x96, 0x5c, 0x05, 0x00, 0x00],
        csv_row: 213,
        what: "PlayerControlZelda: no m_fAimingRotation store",
    },
    Entry {
        addr: 0x688200,
        phase: Phase::Main,
        write: Write::Nop(6),
        original: &[0xd9, 0x96, 0x5c, 0x05, 0x00, 0x00],
        csv_row: 214,
        what: "PlayerControlDucked: no m_fAimingRotation store",
    },
    // CPlayerPed::ProcessAnimGroups: no release of the previous anim block and no
    // store to playerData+0x4C (players.rs zeroes that field before deleting a remote ped, as samp).
    Entry {
        addr: 0x609C08,
        phase: Phase::Main,
        write: Write::Nop(39),
        original: &[
            0x8b, 0x0d, 0x34, 0xea, 0xb4, 0x00, 0x8d, 0x04, 0x80, 0x8b, 0x14, 0x81, 0x81, 0xea, 0xa0, 0xd4, 0xb5, 0x00,
            0xc1, 0xfa, 0x05, 0x52, 0xe8, 0xad, 0xa3, 0xec, 0xff, 0x8b, 0x86, 0x80, 0x04, 0x00, 0x00, 0x83, 0xc4, 0x04,
            0x89, 0x68, 0x4c,
        ],
        csv_row: 167,
        what: "CPlayerPed::ProcessAnimGroups: no anim block switch",
    },
    // Pool sizes samp sets in CPools::Initialise (gta-patches.md, "4.1 Pool capacities"). samp.saa's data
    // (SAMP.ide, SAMP.ipl, SAMPCOL.img) needs more buildings, dummies and collision models.
    // Vehicle pool: `push "Vehicles"; push 110` -> `push 0; push 710`.
    Entry {
        addr: 0x551024,
        phase: Phase::Main,
        write: Write::Bytes(&[0x6a, 0x00, 0x68, 0xc6, 0x02, 0x00, 0x00]),
        original: &[0x68, 0xdc, 0x3c, 0x86, 0x00, 0x6a, 0x6e],
        csv_row: 107,
        what: "CPools::Initialise: vehicle pool 710",
    },
    // Building pool 13000 -> 20000.
    Entry {
        addr: 0x55105F,
        phase: Phase::Main,
        write: Write::Bytes(&[0x20, 0x4e, 0x00, 0x00]),
        original: &[0xc8, 0x32, 0x00, 0x00],
        csv_row: 108,
        what: "CPools::Initialise: building pool 20000",
    },
    // Dummy pool 2500 -> 4000.
    Entry {
        addr: 0x5510CF,
        phase: Phase::Main,
        write: Write::Bytes(&[0xa0, 0x0f, 0x00, 0x00]),
        original: &[0xc4, 0x09, 0x00, 0x00],
        csv_row: 109,
        what: "CPools::Initialise: dummy pool 4000",
    },
    // Collision-model pool 10150 -> 32511.
    Entry {
        addr: 0x551106,
        phase: Phase::Main,
        write: Write::Bytes(&[0x68, 0xff, 0x7e, 0x00, 0x00]),
        original: &[0x68, 0xa6, 0x27, 0x00, 0x00],
        csv_row: 119,
        what: "CPools::Initialise: col-model pool 32511",
    },
    // CTxdStore::Initialise: texture-dictionary pool 5000 -> 20000.
    Entry {
        addr: 0x731F60,
        phase: Phase::Early,
        write: Write::Bytes(&[0x20, 0x4e, 0x00, 0x00]),
        original: &[0x88, 0x13, 0x00, 0x00],
        csv_row: 131,
        what: "CTxdStore::Initialise: txd pool 20000",
    },
    // CStreaming::InitImageList := RET. samp.saa's default.dat lists
    // SAMP\CUSTOM.IMG, SAMP\SAMP.IMG, MODELS\GTA3.IMG and MODELS\GTA_INT.IMG itself;
    // without this the two stock archives would be registered twice (9 of 8 image slots).
    Entry {
        addr: 0x4083C0,
        phase: Phase::EarlyArchive,
        write: Write::Bytes(&[0xC3]),
        original: &[0xB8],
        csv_row: 128,
        what: "CStreaming::InitImageList := RET",
    },
    // Splash texture names "title_pc_US" and "loadsc%d" -> "title":
    // samp.saa's loadscs.txd holds a single texture called `title` (loading screen).
    Entry {
        addr: 0x866CD8,
        phase: Phase::EarlyArchive,
        write: Write::Bytes(b"title\0"),
        original: b"title_",
        csv_row: 122,
        what: "splash name title_pc_US -> title",
    },
    Entry {
        addr: 0x866CCC,
        phase: Phase::EarlyArchive,
        write: Write::Bytes(b"title\0"),
        original: b"loadsc",
        csv_row: 123,
        what: "splash name loadsc%d -> title",
    },
    // Single-player population, traffic and replay off (docs/phase3/trace-notes.md, "SP-OFF baseline").
    // Patch table row 55 (police scanner) is a call hook installed in dll.rs.
    // CGame::Process: no CCarCtrl::GenerateRandomCars.
    Entry {
        addr: 0x53C1C1,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0xfa, 0x7f, 0xef, 0xff],
        csv_row: 189,
        what: "CGame::Process: no GenerateRandomCars",
    },
    // CGame::Process: no CTheCarGenerators::Process (parked cars).
    Entry {
        addr: 0x53C06A,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0xd1, 0x7e, 0x1b, 0x00],
        csv_row: 181,
        what: "CGame::Process: no CTheCarGenerators::Process",
    },
    // CGame::Process: no CReplay::Update.
    Entry {
        addr: 0x53C090,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x6b, 0x44, 0xf2, 0xff],
        csv_row: 163,
        what: "CGame::Process: no CReplay::Update",
    },
    // GenerateRandomCars: no CPlane::DoPlaneGenerationAndRemoval.
    Entry {
        addr: 0x434272,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x79, 0x90, 0x29, 0x00],
        csv_row: 182,
        what: "GenerateRandomCars: no plane generation",
    },
    // CPopulation::AddPed := xor eax,eax; ret.
    Entry {
        addr: 0x612710,
        phase: Phase::Main,
        write: Write::Bytes(&[0x33, 0xc0, 0xc3]),
        original: &[0x90, 0xe9, 0xad],
        csv_row: 185,
        what: "CPopulation::AddPed := return NULL",
    },
    // CPopulation::AddPedInCar: no CCarEnterExit::SetPedInCarDirect.
    Entry {
        addr: 0x613BA7,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0xd4, 0xc6, 0x03, 0x00],
        csv_row: 186,
        what: "AddPedInCar: no SetPedInCarDirect",
    },
    // CEntryExit::TransitionFinished: no GenerateAmbientPeds (push + call).
    Entry {
        addr: 0x440833,
        phase: Phase::Main,
        write: Write::Nop(8),
        original: &[0x52, 0x8b, 0xcd, 0xe8, 0x75, 0xe0, 0xff, 0xff],
        csv_row: 164,
        what: "CEntryExit::TransitionFinished: no GenerateAmbientPeds",
    },
    // "playidles" blanked: no idle anims on player peds.
    Entry {
        addr: 0x86D1EC,
        phase: Phase::Main,
        write: Write::Bytes(&[0x00]),
        original: &[0x70],
        csv_row: 162,
        what: "playidles anim block name blanked",
    },
    // CPlayerPed::ProcessAnimGroups: NOP the move-anim-group store; samp then
    // writes its gated call over the first 5 bytes (patch table row 43, pedhooks.rs), so a player ped keeps
    // its skin's own walk style unless InitGame "CJ walk" is set.
    Entry {
        addr: 0x609A4E,
        phase: Phase::Main,
        write: Write::Nop(6),
        original: &[0x89, 0x86, 0xd4, 0x04, 0x00, 0x00],
        csv_row: 187,
        what: "ProcessAnimGroups: no move-anim-group store",
    },
    // CFileLoader::LoadPickup := RET: no pickups from the IPL files (the stock
    // weapon pickups such as the sniper rifle and fire extinguishers).
    Entry {
        addr: 0x5B47B0,
        phase: Phase::Main,
        write: Write::Bytes(&[0xc3]),
        original: &[0x83],
        csv_row: 172,
        what: "CFileLoader::LoadPickup := RET",
    },
    // Interior_c::Init: NOP Interior_c::AddPickups (no interior pickups).
    Entry {
        addr: 0x593D7C,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x0f, 0xe2, 0xff, 0xff],
        csv_row: 208,
        what: "Interior_c::Init: no AddPickups",
    },
    // GTA systems that samp switches off and that a coverage comparison showed running only with our client.
    // CGame::Process: no CPlantMgr::Update (grass/plants).
    Entry {
        addr: 0x53C159,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x42, 0x0e, 0x0a, 0x00],
        csv_row: 158,
        what: "CGame::Process: no CPlantMgr::Update",
    },
    // CGame::Process: no CCoronas::DoSunAndMoon.
    Entry {
        addr: 0x53C136,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x65, 0x04, 0x1c, 0x00],
        csv_row: 159,
        what: "CGame::Process: no CCoronas::DoSunAndMoon",
    },
    // CAEGlobalWeaponAudioEntity::ServiceAmbientGunFire: state always reset
    // (no ambient gunfire sounds).
    Entry {
        addr: 0x4DF7E2,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0x7a, 0x15, 0xa1, 0x84, 0xcb],
        csv_row: 155,
        what: "ServiceAmbientGunFire: no ambient gunfire",
    },
    // CPlayerPed::ClearWeaponTarget: no CCamera::ClearPlayerWeaponMode.
    Entry {
        addr: 0x609CB4,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0x57, 0x0e, 0xf0, 0xff],
        csv_row: 192,
        what: "ClearWeaponTarget: no ClearPlayerWeaponMode",
    },
    // CCam::Process_FollowPed_SA: no CIdleCam::Process (idle camera).
    Entry {
        addr: 0x524582,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xe8, 0xf9, 0xe6, 0xff, 0xff],
        csv_row: 244,
        what: "Process_FollowPed_SA: no idle camera",
    },
    // Further rows samp applies at init-main (docs/phase3/par-audit.md). The next two turn real-time shadows
    // off (docs/phase3/par-audit.md, "CSV row 165 re-test").
    // Idle: no CRealTimeShadowManager::Update.
    Entry {
        addr: 0x53EA08,
        phase: Phase::Main,
        write: Write::Nop(10),
        original: &[0xb9, 0x50, 0x03, 0xc4, 0x00, 0xe8, 0x9e, 0x80, 0x1c, 0x00],
        csv_row: 165,
        what: "Idle: no CRealTimeShadowManager::Update",
    },
    // CPhysical dtor: no ReturnRealTimeShadow.
    Entry {
        addr: 0x542485,
        phase: Phase::Main,
        write: Write::Nop(11),
        original: &[0x50, 0xb9, 0x50, 0x03, 0xc4, 0x00, 0xe8, 0xa0, 0x36, 0x1c, 0x00],
        csv_row: 184,
        what: "CPhysical dtor: no ReturnRealTimeShadow",
    },
    // RenderStencilShadows: overlay alpha 80.
    Entry {
        addr: 0x71162C,
        phase: Phase::Main,
        write: Write::Bytes(&[0x50]),
        original: &[0x32],
        csv_row: 166,
        what: "RenderStencilShadows: overlay alpha 80",
    },
    // SetEventResponseTask: no editable response (facial expressions).
    Entry {
        addr: 0x4BC6C1,
        phase: Phase::Main,
        write: Write::Bytes(&[0xb0, 0x00, 0x90]),
        original: &[0xff, 0x50, 0x24],
        csv_row: 170,
        what: "SetEventResponseTask: no editable response (facial expressions)",
    },
    // CPlayerInfo::Process: no hold/fight test.
    Entry {
        addr: 0x570535,
        phase: Phase::Main,
        write: Write::Nop(15),
        original: &[0x85, 0xdb, 0x74, 0x0b, 0x8b, 0x43, 0x08, 0x85, 0xc0, 0x0f, 0x85, 0x61, 0x05, 0x00, 0x00],
        csv_row: 215,
        what: "CPlayerInfo::Process: no hold/fight test",
    },
    // CPed::Render: weapon also while holding.
    Entry {
        addr: 0x5E7847,
        phase: Phase::Main,
        write: Write::Nop(17),
        original: &[
            0x8b, 0x8d, 0x7c, 0x04, 0x00, 0x00, 0x6a, 0x00, 0xe8, 0x9c, 0x97, 0x01, 0x00, 0x85, 0xc0, 0x75, 0x24,
        ],
        csv_row: 216,
        what: "CPed::Render: weapon also while holding",
    },
    // CPlayerInfo::Process: no script-command task.
    Entry {
        addr: 0x570546,
        phase: Phase::Main,
        write: Write::Nop(175),
        original: &[
            0x0f, 0x84, 0xa9, 0x00, 0x00, 0x00, 0x8b, 0x0e, 0x8b, 0x81, 0x7c, 0x04, 0x00, 0x00, 0x8d, 0x48, 0x04, 0xe8,
            0xc4, 0x11, 0x11, 0x00, 0x85, 0xc0, 0x74, 0x16, 0x8b, 0x0e, 0x8b, 0x10, 0x6a, 0x00, 0x6a, 0x01, 0x51, 0x8b,
            0xc8, 0xff, 0x52, 0x18, 0x84, 0xc0, 0x0f, 0x84, 0x2f, 0x05, 0x00, 0x00, 0x6a, 0x34, 0xe8, 0x23, 0xa0, 0x0a,
            0x00, 0x83, 0xc4, 0x04, 0x89, 0x44, 0x24, 0x30, 0x85, 0xc0, 0xc7, 0x44, 0x24, 0x60, 0x02, 0x00, 0x00, 0x00,
            0x74, 0x0c, 0x6a, 0x51, 0x55, 0x8b, 0xc8, 0xe8, 0x26, 0x14, 0x12, 0x00, 0xeb, 0x02, 0x33, 0xc0, 0x6a, 0x00,
            0x50, 0x83, 0xcf, 0xff, 0x6a, 0x03, 0x8d, 0x4c, 0x24, 0x40, 0x89, 0x7c, 0x24, 0x6c, 0xe8, 0x4d, 0x04, 0xf4,
            0xff, 0x0f, 0xb6, 0x05, 0x74, 0xcd, 0xb7, 0x00, 0x69, 0xc0, 0x90, 0x01, 0x00, 0x00, 0x8b, 0x80, 0x98, 0xcd,
            0xb7, 0x00, 0x8b, 0x80, 0x7c, 0x04, 0x00, 0x00, 0x6a, 0x00, 0x8d, 0x54, 0x24, 0x38, 0x52, 0x8d, 0x48, 0x68,
            0xc7, 0x44, 0x24, 0x68, 0x03, 0x00, 0x00, 0x00, 0xe8, 0x3d, 0xae, 0xf3, 0xff, 0x8d, 0x4c, 0x24, 0x34, 0x89,
            0x7c, 0x24, 0x60, 0xe8, 0x60, 0x04, 0xf4, 0xff, 0xe9, 0xb0, 0x04, 0x00, 0x00,
        ],
        csv_row: 217,
        what: "CPlayerInfo::Process: no script-command task",
    },
    // Unnamed GTA function: the cell-phone model pushed for CPed::RemoveWeaponModel becomes -1.
    Entry {
        addr: 0x6348F6,
        phase: Phase::Main,
        write: Write::Bytes(&[0xff, 0xff, 0xff, 0xff]),
        original: &[0x4a, 0x01, 0x00, 0x00],
        csv_row: 223,
        what: "0x6348F0: phone model -1",
    },
    // Same in a second unnamed function.
    Entry {
        addr: 0x634DE2,
        phase: Phase::Main,
        write: Write::Bytes(&[0xff, 0xff, 0xff, 0xff]),
        original: &[0x4a, 0x01, 0x00, 0x00],
        csv_row: 224,
        what: "0x634A40: phone model -1",
    },
    // CFont::ProcessCurrentString: 15 stack-offset high bytes 0x01 -> 0x03 together grow the stack frame
    // from 0x12C to 0x32C bytes (a bigger local line buffer for longer text lines).
    Entry {
        addr: 0x71A223,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 225,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A5D5,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 226,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A247,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 227,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A282,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 228,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A26A,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 229,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A2E5,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 230,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A3AC,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 231,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A4B9,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 232,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A53C,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 233,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A37F,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 234,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A518,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 235,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A525,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 236,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A54B,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 237,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A5B6,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 238,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    Entry {
        addr: 0x71A3BB,
        phase: Phase::Main,
        write: Write::Bytes(&[0x03]),
        original: &[0x01],
        csv_row: 239,
        what: "CFont::ProcessCurrentString: frame 0x32C",
    },
    // CGameLogic::SortOutStreamingAndMemory: NOP the
    // CStreaming::DeleteRwObjectsAfterDeath call.
    Entry {
        addr: 0x441482,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0x89, 0x7D, 0xFC, 0xFF],
        csv_row: 179,
        what: "SortOutStreamingAndMemory: keep RW objects after death",
    },
    // CFire::ProcessFire: `jmp +0x77` over the BMX branch (a BMX
    // near a fire sets the local player on fire / bursts a tyre).
    Entry {
        addr: 0x53A984,
        phase: Phase::Main,
        write: Write::Bytes(&[0xEB, 0x77]),
        original: &[0x6A, 0xFF],
        csv_row: 199,
        what: "CFire::ProcessFire: skip the BMX branch",
    },
    // CMenuManager::PrintMap: NOP one CSprite2d::DrawRect.
    Entry {
        addr: 0x575B0E,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0x4D, 0x20, 0x1B, 0x00],
        csv_row: 203,
        what: "PrintMap: no DrawRect",
    },
    // InteriorGroup_c::Exit: NOP the Interior_c::Exit call.
    Entry {
        addr: 0x5952A6,
        phase: Phase::Main,
        write: Write::Nop(5),
        original: &[0xE8, 0x85, 0xCF, 0xFF, 0xFF],
        csv_row: 212,
        what: "InteriorGroup_c::Exit: no Interior_c::Exit",
    },
    // CBirds::CreateNumberOfBirds: the line-of-sight test also
    // checks objects (`push 0` → `push 1`).
    Entry {
        addr: 0x712025,
        phase: Phase::Main,
        write: Write::Bytes(&[0x01]),
        original: &[0x00],
        csv_row: 245,
        what: "CreateNumberOfBirds: LOS checks objects",
    },
];

/// Streaming budget for `CStreaming::ms_memoryAvailable` from physical RAM in MB (patch table row 127;
/// gta-patches.md, "4.6 Streaming memory").
pub fn streaming_budget(ram_mb: u64) -> u32 {
    match ram_mb {
        m if m > 4000 => 0x4000_0000,
        m if m > 2000 => 0x2000_0000,
        m if m > 1000 => 0x1000_0000,
        m if m > 500 => 0x0800_0000,
        _ => 0x0600_0000,
    }
}

/// Outcome of applying one phase.
#[derive(Debug, Default)]
pub struct Report {
    pub applied: Vec<(u32, &'static str)>,
    pub failed: Vec<(u32, &'static str, PatchError)>,
}

/// Diagnostics only: `SAMP_RS_SKIP_CSV_ROWS=158,159` leaves those patch table rows unapplied (for bisecting).
pub fn skipped_rows() -> Vec<u32> {
    std::env::var("SAMP_RS_SKIP_CSV_ROWS")
        .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Applies every entry of `phase`. Entries whose original bytes do not match
/// are skipped and reported (never partially written).
pub fn apply_phase<M: Memory>(patcher: &mut Patcher, mem: &mut M, phase: Phase, ram_mb: u64) -> Report {
    let mut report = Report::default();
    let skip = skipped_rows();
    for e in TABLE.iter().filter(|e| e.phase == phase && !skip.contains(&e.csv_row)) {
        let budget;
        let bytes: Vec<u8> = match e.write {
            Write::Bytes(b) => b.to_vec(),
            Write::Nop(n) => vec![crate::patch::OP_NOP; n],
            Write::StreamingBudget => {
                budget = streaming_budget(ram_mb);
                budget.to_le_bytes().to_vec()
            }
        };
        match patcher.write(mem, e.what, e.addr, &bytes, Some(e.original)) {
            Ok(()) => report.applied.push((e.addr, e.what)),
            Err(err) => report.failed.push((e.addr, e.what, err)),
        }
    }
    report
}

/// Shared test helpers: the annotated CSV and the reference executable.
#[cfg(test)]
pub(crate) mod testutil {
    use std::collections::HashMap;
    use std::path::PathBuf;

    pub fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub fn unhex(s: &str) -> Vec<u8> {
        let s = s.split("...").next().unwrap_or("").trim();
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// Minimal CSV field split (quoted fields with commas, doubled quotes).
    pub fn split_csv(line: &str) -> Vec<String> {
        let mut out = vec![String::new()];
        let mut q = false;
        let mut it = line.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '"' if q && it.peek() == Some(&'"') => {
                    it.next();
                    out.last_mut().unwrap().push('"');
                }
                '"' => q = !q,
                ',' if !q => out.push(String::new()),
                _ => out.last_mut().unwrap().push(c),
            }
        }
        out
    }

    pub type Row = HashMap<String, String>;

    /// Every row of gta-patches-r4-annotated.csv by line number (header = 1).
    pub fn csv_rows() -> HashMap<u32, Row> {
        let csv = std::fs::read_to_string(root().join("docs/re/data/gta-patches-r4-annotated.csv")).unwrap();
        let mut lines = csv.lines();
        let header = split_csv(lines.next().unwrap());
        lines
            .enumerate()
            .map(|(i, l)| (i as u32 + 2, header.iter().cloned().zip(split_csv(l)).collect::<Row>()))
            .collect()
    }

    pub fn field_u32(row: &Row, name: &str) -> u32 {
        u32::from_str_radix(row[name].trim_start_matches("0x"), 16).unwrap()
    }

    /// `re/gta/1.0-US/gta_sa.exe`, if present (it is gitignored).
    pub struct Exe(Vec<u8>);

    impl Exe {
        pub fn load() -> Option<Self> {
            std::fs::read(root().join("re/gta/1.0-US/gta_sa.exe")).ok().map(Exe)
        }

        pub fn read_va(&self, va: u32, len: usize) -> Vec<u8> {
            let exe = &self.0;
            let rd = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap());
            let pe = rd(0x3C) as usize;
            let nsec = u16::from_le_bytes([exe[pe + 6], exe[pe + 7]]) as usize;
            let opt = u16::from_le_bytes([exe[pe + 20], exe[pe + 21]]) as usize;
            let rva = va - rd(pe + 24 + 28);
            for i in 0..nsec {
                let s = pe + 24 + opt + 40 * i;
                let (vsize, vaddr, rsize, roff) = (rd(s + 8), rd(s + 12), rd(s + 16), rd(s + 20));
                if rva >= vaddr && rva < vaddr + vsize.max(rsize) {
                    let o = (roff + rva - vaddr) as usize;
                    return exe[o..o + len].to_vec();
                }
            }
            panic!("{va:#x} not mapped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{self, Exe, unhex};
    use super::*;
    use crate::patch::fake::FakeMemory;

    #[test]
    fn table_matches_annotated_csv() {
        let rows = testutil::csv_rows();
        for e in TABLE {
            let row = &rows[&e.csv_row];
            assert_eq!(testutil::field_u32(row, "gta_addr"), e.addr, "row {} address", e.csv_row);
            assert_eq!(row["size"].parse::<usize>().unwrap(), e.size(), "row {} size", e.csv_row);
            let orig = unhex(&row["original_bytes_hex"]);
            assert!(!orig.is_empty() && e.original.starts_with(&orig), "row {} original bytes", e.csv_row);
            assert_eq!(e.original.len(), e.size(), "row {} original length", e.csv_row);
            let new = &row["new_bytes_hex"];
            match e.write {
                Write::Bytes(b) => {
                    if !new.contains('x') && !new.contains('…') {
                        assert_eq!(unhex(new), b, "row {} new bytes", e.csv_row);
                    } else {
                        assert!(b.starts_with(&unhex(new.split('…').next().unwrap())));
                    }
                }
                Write::Nop(n) => assert!(new == &"90".repeat(n) || new == &format!("90 x{n}"), "row {}", e.csv_row),
                Write::StreamingBudget => assert_eq!(new, "<RAM-dependent>"),
            }
        }
    }

    /// Checks `original` against the reference executable when it is present
    /// (it is gitignored, so the test passes silently without it).
    #[test]
    fn originals_match_reference_exe() {
        let Some(exe) = Exe::load() else { return };
        assert_eq!(exe.read_va(crate::game::VERSION_PROBE_ADDR, 1), [crate::game::VERSION_PROBE_US10]);
        for e in TABLE {
            assert_eq!(exe.read_va(e.addr, e.original.len()), e.original, "{:#x} {}", e.addr, e.what);
        }
        assert_eq!(exe.read_va(crate::cdstream::SYNC_WAIT_CALL, 6), crate::cdstream::SYNC_WAIT_CALL_ORIGINAL);
        // CGame::Shutdown entry (patch table row 52).
        assert_eq!(exe.read_va(crate::game::CGAME_SHUTDOWN, 7), crate::game::CGAME_SHUTDOWN_ORIGINAL);
        // The police scanner's FindPlayerPed call (patch table row 55).
        assert_eq!(
            exe.read_va(crate::game::POLICE_SCANNER_FIND_PLAYER_CALL, 5),
            crate::patch::encode_branch(
                0xE8,
                crate::game::POLICE_SCANNER_FIND_PLAYER_CALL,
                crate::game::FIND_PLAYER_PED
            )
        );
        // The CGame::Process call site in Idle (patch table row 10).
        assert_eq!(
            exe.read_va(crate::game::IDLE_GAME_PROCESS_CALL, 5),
            crate::patch::encode_branch(0xE8, crate::game::IDLE_GAME_PROCESS_CALL, crate::game::CGAME_PROCESS)
        );
        // The per-frame call site: call Render2dStuff.
        assert_eq!(
            exe.read_va(crate::game::IDLE_RENDER_2D_CALL, 5),
            crate::patch::encode_branch(0xE8, crate::game::IDLE_RENDER_2D_CALL, crate::game::RENDER_2D_STUFF)
        );
    }

    #[test]
    fn no_overlapping_entries() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                let disjoint = a.addr + a.size() as u32 <= b.addr || b.addr + b.size() as u32 <= a.addr;
                assert!(disjoint, "{:#x} overlaps {:#x}", a.addr, b.addr);
            }
        }
    }

    #[test]
    fn budget_thresholds() {
        assert_eq!(streaming_budget(256), 0x0600_0000);
        assert_eq!(streaming_budget(501), 0x0800_0000);
        assert_eq!(streaming_budget(2000), 0x1000_0000);
        assert_eq!(streaming_budget(4000), 0x2000_0000);
        assert_eq!(streaming_budget(32768), 0x4000_0000);
    }

    #[test]
    fn apply_skips_mismatched_sites() {
        let mut m = FakeMemory::new(0x0050_0000, 0x0030_0000);
        // Only provide the original bytes for two of the early entries.
        m.set(0x747483, &[0x89, 0x35, 0xC0, 0xD4, 0xC8, 0x00]);
        m.set(0x5B8E6A, &[0x00, 0x00, 0x20, 0x03]);
        let mut p = Patcher::new();
        let r = apply_phase(&mut p, &mut m, Phase::Early, 16384);
        assert_eq!(r.applied.len(), 2);
        assert_eq!(r.failed.len(), TABLE.iter().filter(|e| e.phase == Phase::Early).count() - 2);
        assert_eq!(m.get(0x747483, 6), &[0x90; 6]);
        assert_eq!(m.read_u32(0x5B8E6A).unwrap(), 0x4000_0000);
        p.restore_all(&mut m).unwrap();
        assert_eq!(m.get(0x747483, 1), &[0x89]);
    }
}
