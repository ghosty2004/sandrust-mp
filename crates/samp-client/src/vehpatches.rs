//! samp.dll's plain byte patches to gta_sa.exe's vehicle code, as data: [`INIT`] (applied with the main patch
//! block), [`MANUAL_LIGHTS`] (the `CVehicle::DoVehicleLights` rewrite, once when InitGame enables manual
//! engine/lights), [`STATS`] (`CStats::SetStatValue` becomes RET on the first frame, right after GTA's own
//! driving- and weapon-skill cheats) and the 256-colour vehicle palette ([`apply_colour_table`]).
//! Hooks (vtable redirects, call/jmp hooks) live in `vehhooks.rs`; vehicle patches that belong to other systems
//! (pool sizes, single-player removal, plates, radar) live with those systems.
//! Each entry's `csv_row` is its line in `docs/re/data/gta-patches-r4-annotated.csv` (header = line 1).
//! Spec: docs/re/subsystems/gta-patches.md, "Vehicles"; docs/phase3/m5-notes.md lists the rows left out.

use crate::patch::{Memory, PatchError, Patcher};

/// What a patch writes.
#[derive(Debug, Clone, Copy)]
pub enum Write {
    Bytes(&'static [u8]),
    Nop(usize),
    /// `n` copies of one byte.
    Fill(u8, usize),
}

impl Write {
    /// The bytes written.
    pub fn bytes(&self) -> Vec<u8> {
        match *self {
            Write::Bytes(b) => b.to_vec(),
            Write::Nop(n) => vec![crate::patch::OP_NOP; n],
            Write::Fill(b, n) => vec![b; n],
        }
    }
}

/// One byte patch at a gta_sa.exe address.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub addr: u32,
    pub write: Write,
    /// Bytes of the unmodified gta_sa.exe 1.0 US.
    pub original: &'static [u8],
    /// Line in `docs/re/data/gta-patches-r4-annotated.csv`.
    pub csv_row: u32,
    /// GTA function the patch lands in (used as the patch label).
    pub what: &'static str,
}

impl Entry {
    /// Number of bytes written.
    pub fn size(&self) -> usize {
        self.write.bytes().len()
    }
}

/// Init-main byte patches (applied with the main block at gGameState 7).
pub const INIT: &[Entry] = &[
    // 'VehicleStruct' pool capacity 50 → 127.
    Entry {
        addr: 0x5b8fe4,
        write: Write::Bytes(&[0x7f]),
        original: &[0x32],
        csv_row: 156,
        what: "0x5b8f00 CVehicleModelInfo::SetupCommonData",
    },
    // FindPlayerVehicle: drop the branch that returns the player's remote-controlled vehicle.
    Entry {
        addr: 0x56e0fa,
        write: Write::Nop(18),
        original: &[
            0x8a, 0x54, 0x24, 0x08, 0x84, 0xd2, 0x74, 0x0a, 0x8b, 0x80, 0x48, 0xce, 0xb7, 0x00, 0x85, 0xc0, 0x75, 0x09,
        ],
        csv_row: 168,
        what: "0x56e0d0 FindPlayerVehicle",
    },
    // CBike::ProcessAI: no comparison of the driver with `CWorld::Players[0]` (remote players drive bikes too).
    Entry {
        addr: 0x6bc9eb,
        write: Write::Nop(2),
        original: &[0x75, 0x0b],
        csv_row: 169,
        what: "0x6bc930 CBike::ProcessAI (virtual)",
    },
    // CGarage::Update: `CGarages::IsCarSprayable` replaced by `mov al,1` — every vehicle can be resprayed.
    Entry {
        addr: 0x44ac75,
        write: Write::Bytes(&[0xb0, 0x01, 0x90, 0x90, 0x90]),
        original: &[0xe8, 0x26, 0xcd, 0xff, 0xff],
        csv_row: 171,
        what: "0x44aa50 CGarage::Update",
    },
    // CTrain::ProcessControl: jump over the train passenger block.
    Entry {
        addr: 0x6f8cf8,
        write: Write::Bytes(&[0x90, 0x90, 0x90, 0x90, 0x90, 0xb8, 0x89, 0x8f, 0x6f, 0x00, 0xff, 0xe0]),
        original: &[0x0f, 0xb7, 0x9e, 0xa0, 0x05, 0x00, 0x00, 0x8d, 0x43, 0xff, 0x3b, 0xc5],
        csv_row: 188,
        what: "0x6f86a0 CTrain::ProcessControl (virtual)",
    },
    // CPad::GetHandBrake: ignores DisablePlayerControls.
    Entry {
        addr: 0x540040,
        write: Write::Bytes(&[0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x85, 0xc9, 0x74]),
        original: &[0x66, 0x83, 0xb9, 0x0e, 0x01, 0x00, 0x00, 0x00, 0x75],
        csv_row: 190,
        what: "0x540040 CPad::GetHandBrake",
    },
    // The `CAEVehicleAudioEntity::Service` call in the ProcessControl of automobiles, bikes, boats and trains
    // (samp drives vehicle audio itself).
    Entry {
        addr: 0x6b18f1,
        write: Write::Nop(5),
        original: &[0xe8, 0x8a, 0x09, 0xe5, 0xff],
        csv_row: 194,
        what: "0x6b1880 CAutomobile::ProcessControl (virtual)",
    },
    Entry {
        addr: 0x6b9298,
        write: Write::Nop(5),
        original: &[0xe8, 0xe3, 0x8f, 0xe4, 0xff],
        csv_row: 195,
        what: "0x6b9250 CBike::ProcessControl (virtual)",
    },
    Entry {
        addr: 0x6f1793,
        write: Write::Nop(5),
        original: &[0xe8, 0xe8, 0x0a, 0xe1, 0xff],
        csv_row: 196,
        what: "0x6f1770 CBoat::ProcessControl (virtual)",
    },
    Entry {
        addr: 0x6f86b6,
        write: Write::Nop(5),
        original: &[0xe8, 0xc5, 0x9b, 0xe0, 0xff],
        csv_row: 197,
        what: "0x6f86a0 CTrain::ProcessControl (virtual)",
    },
    // CTaskComplexEnterCarAsDriverTimed constructor: no `CEntity::RegisterReference` on the vehicle.
    Entry {
        addr: 0x63adc8,
        write: Write::Nop(6),
        original: &[0x50, 0xe8, 0xa2, 0x6d, 0xf3, 0xff],
        csv_row: 204,
        what: "0x63ad70 CTaskComplexEnterCarAsDriverTimed::Constructor",
    },
    // CAutomobile::ProcessFlyingCarStuff filled with RET: no flying-cars cheat physics.
    Entry {
        addr: 0x6a8500,
        write: Write::Fill(0xc3, 119),
        original: &[
            0x8a, 0x41, 0x36, 0xc0, 0xe8, 0x03, 0x84, 0xc0, 0x74, 0x08, 0x3c, 0x08, 0x74, 0x04, 0x3c, 0x03, 0x75, 0x65,
            0xa0, 0x60, 0x91, 0x96, 0x00, 0x84, 0xc0, 0x74, 0x5c, 0xd9, 0x41, 0x4c, 0xd9, 0x41, 0x48, 0xd9, 0x41, 0x44,
            0xd9, 0xc0, 0xd8, 0xc9, 0xd9, 0xc2, 0xd8, 0xcb, 0xde, 0xc1, 0xd9, 0xc3, 0xd8, 0xcc, 0xde, 0xc1, 0xd9, 0xfa,
            0xdd, 0xdb, 0xdd, 0xd8, 0xdd, 0xd8, 0xd8, 0x1d, 0x50, 0x8b, 0x85, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x41, 0x75,
            0x2e, 0xd9, 0x05, 0x5c, 0xcb, 0xb7, 0x00, 0xd8, 0x1d, 0x50, 0x8b, 0x85, 0x00, 0xdf, 0xe0, 0xf6, 0xc4, 0x41,
            0x75, 0x1b, 0x68, 0xf6, 0x3f, 0x1c, 0xc6, 0x68, 0xf6, 0x3f, 0x1c, 0xc6, 0x68, 0xf6, 0x3f, 0x1c, 0xc6, 0x68,
            0xf6, 0x3f, 0x1c, 0xc6, 0x6a, 0x03, 0xe8, 0x79, 0x00, 0x03, 0x00,
        ],
        csv_row: 207,
        what: "0x6a8500 CAutomobile::ProcessFlyingCarStuff (virtual)",
    },
    // CVehicle::SetDriver: six model-switch cases point at the default case — no "freebies" (ambulance health,
    // taxi cash, Enforcer armour, Caddy club, Hotdog calories, cop-car shotgun).
    Entry {
        addr: 0x6d1874,
        write: Write::Bytes(&[
            0xd5, 0x17, 0x6d, 0x00, 0xd5, 0x17, 0x6d, 0x00, 0xd5, 0x17, 0x6d, 0x00, 0xd5, 0x17, 0x6d, 0x00, 0xd5, 0x17,
            0x6d, 0x00, 0xd5, 0x17, 0x6d, 0x00,
        ]),
        original: &[
            0x61, 0x17, 0x6d, 0x00, 0x41, 0x17, 0x6d, 0x00, 0x6f, 0x17, 0x6d, 0x00, 0x9b, 0x17, 0x6d, 0x00, 0xc3, 0x17,
            0x6d, 0x00, 0x1f, 0x17, 0x6d, 0x00,
        ],
        csv_row: 218,
        what: "0x6d16a0 CVehicle::SetDriver",
    },
    // Car-entry tasks keep the ped's weapon: `CPed::RemoveWeaponWhenEnteringVehicle` calls removed here and in
    // the next two entries.
    Entry {
        addr: 0x64db49,
        write: Write::Nop(9),
        original: &[0x6a, 0x00, 0x8b, 0xcf, 0xe8, 0x1e, 0x88, 0xf9, 0xff],
        csv_row: 219,
        what: "0x64db30 CTaskSimpleCarGetIn::ProcessPed (virtual)",
    },
    Entry {
        addr: 0x64bc9f,
        write: Write::Nop(9),
        original: &[0x6a, 0x00, 0x8b, 0xcf, 0xe8, 0xc8, 0xa6, 0xf9, 0xff],
        csv_row: 220,
        what: "0x64b950 CTaskSimpleCarSetPedInAsDriver::ProcessPed",
    },
    Entry {
        addr: 0x64b872,
        write: Write::Nop(9),
        original: &[0x6a, 0x00, 0x8b, 0xcf, 0xe8, 0xf5, 0xaa, 0xf9, 0xff],
        csv_row: 221,
        what: "0x64b5d0 CTaskSimpleCarSetPedInAsPassenger::ProcessPed",
    },
    // CPed::AttachPedToEntity returns before it gives the weapon, sets the camera mode or changes the ped state.
    Entry {
        addr: 0x5e7d62,
        write: Write::Bytes(&[0x5e, 0x5d, 0xc2, 0x1c, 0x00]),
        original: &[0x85, 0xc0, 0x74, 0x09, 0x83],
        csv_row: 222,
        what: "0x5e7cb0 CPed::AttachPedToEntity",
    },
    // CPostEffects::Render: the `CPostEffects::SpeedFX` call is NOPed (no speed blur).
    Entry {
        addr: 0x704e8a,
        write: Write::Nop(5),
        original: &[0xe8, 0x11, 0xe2, 0xff, 0xff],
        csv_row: 177,
        what: "0x7046e0 CPostEffects::Render",
    },
];

/// Manual engine/lights: the `CVehicle::DoVehicleLights` rewrite (InitGame, once). The game's own on/off logic
/// is removed and the lights follow bit 0x40 of the vehicle's byte at +0x428. Order matters: the last entry
/// writes inside the NOPs of the first.
pub const MANUAL_LIGHTS: &[Entry] = &[
    Entry {
        addr: 0x6e1be3,
        write: Write::Nop(37),
        original: &[
            0x66, 0x8b, 0x86, 0x5c, 0x04, 0x00, 0x00, 0x66, 0x85, 0xc0, 0x74, 0x28, 0x66, 0x3d, 0xff, 0xff, 0x74, 0x22,
            0x8a, 0x46, 0x36, 0x24, 0xf8, 0x3c, 0x28, 0x74, 0x19, 0xa1, 0x84, 0xcb, 0xb7, 0x00, 0xf6, 0xc4, 0x01, 0x74,
            0x04,
        ],
        csv_row: 140,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1c38,
        write: Write::Nop(8),
        original: &[0xa8, 0x10, 0x0f, 0x84, 0x8e, 0x00, 0x00, 0x00],
        csv_row: 141,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1d98,
        write: Write::Nop(15),
        original: &[0xf6, 0xc3, 0x10, 0x74, 0x0a, 0xc6, 0x44, 0x24, 0x18, 0x00, 0xc6, 0x44, 0x24, 0x14, 0x00],
        csv_row: 142,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1dbc,
        write: Write::Nop(8),
        original: &[0xa8, 0x10, 0x0f, 0x84, 0x2b, 0x0b, 0x00, 0x00],
        csv_row: 143,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1ba0,
        write: Write::Nop(6),
        original: &[0x88, 0x9e, 0x28, 0x04, 0x00, 0x00],
        csv_row: 144,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1bb1,
        write: Write::Nop(6),
        original: &[0x88, 0x86, 0x28, 0x04, 0x00, 0x00],
        csv_row: 145,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    Entry {
        addr: 0x6e1bd2,
        write: Write::Nop(7),
        original: &[0x80, 0xa6, 0x28, 0x04, 0x00, 0x00, 0xbf],
        csv_row: 146,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
    // Rewritten inside the NOPed range: `mov al,[esi+0x428]; test al,0x40; jz +0x28`.
    Entry {
        addr: 0x6e1be5,
        write: Write::Bytes(&[0x8a, 0x86, 0x28, 0x04, 0x00, 0x00, 0xa8, 0x40, 0x74, 0x28]),
        original: &[0x86, 0x5c, 0x04, 0x00, 0x00, 0x66, 0x85, 0xc0, 0x74, 0x28],
        csv_row: 147,
        what: "0x6e1a60 CVehicle::DoVehicleLights",
    },
];

/// First frame, after `CCheat::VehicleSkillsCheat` / `WeaponSkillsCheat`: `CStats::SetStatValue` becomes RET, so
/// the game cannot change stat values afterwards (samp sets them directly).
pub const STATS: &[Entry] = &[
    Entry {
        addr: 0x55a070,
        write: Write::Bytes(&[0xc3]),
        original: &[0xd9],
        csv_row: 356,
        what: "0x55a070 CStats::SetStatValue",
    },
];

/// `CCheat::VehicleSkillsCheat` (cdecl): maxes the driving skills.
pub const VEHICLE_SKILLS_CHEAT: u32 = 0x4399D0;
/// `CCheat::WeaponSkillsCheat` (cdecl): maxes the weapon skills.
pub const WEAPON_SKILLS_CHEAT: u32 = 0x439940;

/// samp's 256-entry vehicle colour palette (RGBA, one byte per channel in memory order R, G, B, A), copied
/// from samp.dll .data 0x101169C8 (byte-identical in 0.3.7-R4 builds 2019-11-21 and 2021-05-07; 1024 bytes).
/// The first 128 entries are GTA's own carcols palette; 128–255 are samp's extension.
pub const VEHICLE_COLOURS: [u32; 256] = [
    0x000000ff, 0xf5f5f5ff, 0x2a77a1ff, 0x840410ff, 0x263739ff, 0x86446eff, 0xd78e10ff, 0x4c75b7ff, 0xbdbec6ff,
    0x5e7072ff, 0x46597aff, 0x656a79ff, 0x5d7e8dff, 0x58595aff, 0xd6dad6ff, 0x9ca1a3ff, 0x335f3fff, 0x730e1aff,
    0x7b0a2aff, 0x9f9d94ff, 0x3b4e78ff, 0x732e3eff, 0x691e3bff, 0x96918cff, 0x515459ff, 0x3f3e45ff, 0xa5a9a7ff,
    0x635c5aff, 0x3d4a68ff, 0x979592ff, 0x421f21ff, 0x5f272bff, 0x8494abff, 0x767b7cff, 0x646464ff, 0x5a5752ff,
    0x252527ff, 0x2d3a35ff, 0x93a396ff, 0x6d7a88ff, 0x221918ff, 0x6f675fff, 0x7c1c2aff, 0x5f0a15ff, 0x193826ff,
    0x5d1b20ff, 0x9d9872ff, 0x7a7560ff, 0x989586ff, 0xadb0b0ff, 0x848988ff, 0x304f45ff, 0x4d6268ff, 0x162248ff,
    0x272f4bff, 0x7d6256ff, 0x9ea4abff, 0x9c8d71ff, 0x6d1822ff, 0x4e6881ff, 0x9c9c98ff, 0x917347ff, 0x661c26ff,
    0x949d9fff, 0xa4a7a5ff, 0x8e8c46ff, 0x341a1eff, 0x6a7a8cff, 0xaaad8eff, 0xab988fff, 0x851f2eff, 0x6f8297ff,
    0x585853ff, 0x9aa790ff, 0x601a23ff, 0x20202cff, 0xa4a096ff, 0xaa9d84ff, 0x78222bff, 0x0e316dff, 0x722a3fff,
    0x7b715eff, 0x741d28ff, 0x1e2e32ff, 0x4d322fff, 0x7c1b44ff, 0x2e5b20ff, 0x395a83ff, 0x6d2837ff, 0xa7a28fff,
    0xafb1b1ff, 0x364155ff, 0x6d6c6eff, 0x0f6a89ff, 0x204b6bff, 0x2b3e57ff, 0x9b9f9dff, 0x6c8495ff, 0x4d8495ff,
    0xae9b7fff, 0x406c8fff, 0x1f253bff, 0xab9276ff, 0x134573ff, 0x96816cff, 0x64686aff, 0x105082ff, 0xa19983ff,
    0x385694ff, 0x525661ff, 0x7f6956ff, 0x8c929aff, 0x596e87ff, 0x473532ff, 0x44624fff, 0x730a27ff, 0x223457ff,
    0x640d1bff, 0xa3adc6ff, 0x695853ff, 0x9b8b80ff, 0x620b1cff, 0x5b5d5eff, 0x624428ff, 0x731827ff, 0x1b376dff,
    0xec6aaeff, 0x000000ff, 0x177517ff, 0x210606ff, 0x125478ff, 0x452a0dff, 0x571e1eff, 0x010701ff, 0x25225aff,
    0x2c89aaff, 0x8a4dbdff, 0x35963aff, 0xb7b7b7ff, 0x464c8dff, 0x84888cff, 0x817867ff, 0x817a26ff, 0x6a506fff,
    0x583e6fff, 0x8cb972ff, 0x824f78ff, 0x6d276aff, 0x1e1d13ff, 0x1e1306ff, 0x1f2518ff, 0x2c4531ff, 0x1e4c99ff,
    0x2e5f43ff, 0x1e9948ff, 0x1e9999ff, 0x999976ff, 0x7c8499ff, 0x992e1eff, 0x2c1e08ff, 0x142407ff, 0x993e4dff,
    0x1e4c99ff, 0x198181ff, 0x1a292aff, 0x16616fff, 0x1b6687ff, 0x6c3f99ff, 0x481a0eff, 0x7a7399ff, 0x746d99ff,
    0x53387eff, 0x222407ff, 0x3e190cff, 0x46210eff, 0x991e1eff, 0x8d4c8dff, 0x805b80ff, 0x7b3e7eff, 0x3c1737ff,
    0x733517ff, 0x781818ff, 0x83341aff, 0x8e2f1cff, 0x7e3e53ff, 0x7c6d7cff, 0x020c02ff, 0x072407ff, 0x163012ff,
    0x16301bff, 0x642b4fff, 0x368452ff, 0x999590ff, 0x818d96ff, 0x99991eff, 0x7f994cff, 0x839292ff, 0x788222ff,
    0x2b3c99ff, 0x3a3a0bff, 0x8a794eff, 0x0e1f49ff, 0x15371cff, 0x15273aff, 0x375775ff, 0x060820ff, 0x071326ff,
    0x20394bff, 0x2c5089ff, 0x15426cff, 0x103250ff, 0x241663ff, 0x692015ff, 0x8c8d94ff, 0x516013ff, 0x090f02ff,
    0x8c573aff, 0x52888eff, 0x995c52ff, 0x99581eff, 0x993a63ff, 0x998f4eff, 0x99311eff, 0x0d1842ff, 0x521e1eff,
    0x42420dff, 0x4c991eff, 0x082a1dff, 0x96821dff, 0x197f19ff, 0x3b141fff, 0x745217ff, 0x893f8dff, 0x7e1a6cff,
    0x0b370bff, 0x27450dff, 0x071f24ff, 0x784573ff, 0x8a653aff, 0x732617ff, 0x319490ff, 0x56941dff, 0x59163dff,
    0x1b8a2fff, 0x38160bff, 0x041804ff, 0x355d8eff, 0x2e3f5bff, 0x561a28ff, 0x4e0e27ff, 0x706c67ff, 0x3b3e42ff,
    0x2e2d33ff, 0x7b7e7dff, 0x4a4442ff, 0x28344eff,
];

const fn colour_bytes() -> [u8; 1024] {
    let mut out = [0u8; 1024];
    let mut i = 0;
    while i < 256 {
        let b = VEHICLE_COLOURS[i].to_be_bytes();
        out[4 * i] = b[0];
        out[4 * i + 1] = b[1];
        out[4 * i + 2] = b[2];
        out[4 * i + 3] = b[3];
        i += 1;
    }
    out
}

/// The palette as the game reads it (`ms_vehicleColourTable` layout: 4 bytes per colour).
pub static COLOUR_TABLE: [u8; 1024] = colour_bytes();

/// `CVehicleModelInfo::ms_vehicleColourTable`.
pub const GTA_COLOUR_TABLE: u32 = 0xB4E480;

/// The seven imm32 operands in gta_sa.exe that point into the colour table: (`csv_row`, operand address, byte
/// offset into the table).
pub const COLOUR_REFS: [(u32, u32, u32); 7] = [
    (148, 0x44B1C1, 0),
    (149, 0x4C8390, 0),
    (150, 0x4C8399, 1),
    (151, 0x4C83A3, 2),
    (152, 0x5817CC, 0),
    (153, 0x582176, 0),
    (154, 0x6A6FFA, 0),
];

/// Outcome of applying a group.
#[derive(Debug, Default)]
pub struct Report {
    /// (`csv_row`, address) of each patch written.
    pub applied: Vec<(u32, u32)>,
    /// (`csv_row`, address, error) of each patch that failed.
    pub failed: Vec<(u32, u32, PatchError)>,
}

/// Applies `entries` in order. An entry that overlaps an earlier one of the same group expects the bytes that
/// entry left there instead of the original bytes.
pub fn apply<M: Memory>(patcher: &mut Patcher, mem: &mut M, entries: &[Entry]) -> Report {
    let mut r = Report::default();
    let mut written: Vec<(u32, usize)> = Vec::new();
    for e in entries {
        let bytes = e.write.bytes();
        let overlaps = written.iter().any(|&(a, n)| e.addr < a + n as u32 && a < e.addr + bytes.len() as u32);
        let expect: Vec<u8> = if overlaps {
            // Expect what the earlier rows of this group left there.
            let mut cur = vec![0u8; bytes.len()];
            match mem.read(e.addr, &mut cur) {
                Ok(()) => cur,
                Err(err) => {
                    r.failed.push((e.csv_row, e.addr, err));
                    continue;
                }
            }
        } else {
            e.original.to_vec()
        };
        match patcher.write(mem, e.what, e.addr, &bytes, Some(&expect)) {
            Ok(()) => {
                written.push((e.addr, bytes.len()));
                r.applied.push((e.csv_row, e.addr));
            }
            Err(err) => r.failed.push((e.csv_row, e.addr, err)),
        }
    }
    r
}

/// Points the colour-table operands at [`COLOUR_TABLE`] (each must still hold the GTA address).
pub fn apply_colour_table<M: Memory>(patcher: &mut Patcher, mem: &mut M) -> Report {
    let base = COLOUR_TABLE.as_ptr() as usize as u32;
    let mut r = Report::default();
    for (row, addr, off) in COLOUR_REFS {
        let expect = (GTA_COLOUR_TABLE + off).to_le_bytes();
        match patcher.write(mem, "vehicle colour table operand", addr, &(base + off).to_le_bytes(), Some(&expect)) {
            Ok(()) => r.applied.push((row, addr)),
            Err(err) => r.failed.push((row, addr, err)),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patches::testutil::{self, Exe, unhex};

    fn all() -> impl Iterator<Item = &'static Entry> {
        INIT.iter().chain(MANUAL_LIGHTS).chain(STATS)
    }

    #[test]
    fn rows_match_annotated_csv() {
        let rows = testutil::csv_rows();
        for e in all() {
            let row = &rows[&e.csv_row];
            assert_eq!(testutil::field_u32(row, "gta_addr"), e.addr, "row {} address", e.csv_row);
            assert_eq!(row["size"].parse::<usize>().unwrap(), e.size(), "row {} size", e.csv_row);
            assert_eq!(e.original.len(), e.size(), "row {} original length", e.csv_row);
            let orig = unhex(&row["original_bytes_hex"]);
            assert!(!orig.is_empty() && e.original.starts_with(&orig), "row {} original bytes", e.csv_row);
            let new = row["new_bytes_hex"].trim();
            match e.write {
                Write::Bytes(b) => assert_eq!(unhex(new), b, "row {} new bytes", e.csv_row),
                Write::Nop(n) => assert!(new == "90".repeat(n) || new == format!("90 x{n}"), "row {}", e.csv_row),
                Write::Fill(b, n) => assert_eq!(new, format!("{b:02x} x{n}"), "row {}", e.csv_row),
            }
        }
        for (row, addr, off) in COLOUR_REFS {
            let r = &rows[&row];
            assert_eq!(testutil::field_u32(r, "gta_addr"), addr, "row {row}");
            assert_eq!(unhex(&r["original_bytes_hex"]), (GTA_COLOUR_TABLE + off).to_le_bytes(), "row {row}");
        }
    }

    #[test]
    fn originals_match_reference_exe() {
        let Some(exe) = Exe::load() else { return };
        for e in all() {
            assert_eq!(exe.read_va(e.addr, e.original.len()), e.original, "row {} {:#x}", e.csv_row, e.addr);
        }
        for (row, addr, off) in COLOUR_REFS {
            assert_eq!(exe.read_va(addr, 4), (GTA_COLOUR_TABLE + off).to_le_bytes(), "row {row}");
        }
    }

    #[test]
    fn no_overlap_with_other_tables() {
        let others: Vec<(u32, usize)> = crate::patches::TABLE.iter().map(|e| (e.addr, e.size())).collect();
        for e in all() {
            for &(a, n) in &others {
                let disjoint = e.addr + e.size() as u32 <= a || a + n as u32 <= e.addr;
                assert!(disjoint, "row {} overlaps the main table at {a:#x}", e.csv_row);
            }
        }
        for (i, a) in INIT.iter().enumerate() {
            for b in &INIT[i + 1..] {
                let disjoint = a.addr + a.size() as u32 <= b.addr || b.addr + b.size() as u32 <= a.addr;
                assert!(disjoint, "rows {} / {} overlap", a.csv_row, b.csv_row);
            }
        }
    }

    #[test]
    fn colour_table_layout() {
        assert_eq!(&COLOUR_TABLE[0..8], &[0, 0, 0, 0xFF, 0xF5, 0xF5, 0xF5, 0xFF]);
        // Entry 128 (first samp extension).
        assert_eq!(&COLOUR_TABLE[512..516], &0x177517FFu32.to_be_bytes());
    }

    #[test]
    fn colour_table_matches_samp_dll() {
        // samp.dll is gitignored; the test passes silently without it.
        let root = testutil::root().join("re/samp-dll");
        let Ok(dirs) = std::fs::read_dir(&root) else { return };
        for d in dirs.flatten() {
            let Ok(dll) = std::fs::read(d.path().join("samp.dll")) else { continue };
            let rd = |o: usize| u32::from_le_bytes(dll[o..o + 4].try_into().unwrap());
            let pe = rd(0x3C) as usize;
            let nsec = u16::from_le_bytes([dll[pe + 6], dll[pe + 7]]) as usize;
            let opt = u16::from_le_bytes([dll[pe + 20], dll[pe + 21]]) as usize;
            let rva = 0x101169C8 - rd(pe + 24 + 28);
            let mut found = false;
            for i in 0..nsec {
                let s = pe + 24 + opt + 40 * i;
                let (vs, va, rs, ro) = (rd(s + 8), rd(s + 12), rd(s + 16), rd(s + 20));
                if rva >= va && rva < va + vs.max(rs) {
                    let o = (ro + rva - va) as usize;
                    assert_eq!(&dll[o..o + 1024], &COLOUR_TABLE[..], "{}", d.path().display());
                    found = true;
                }
            }
            assert!(found);
        }
    }

    #[test]
    fn apply_handles_overlap_in_order() {
        use crate::patch::fake::FakeMemory;
        let mut m = FakeMemory::new(0x0060_0000, 0x0020_0000);
        for e in MANUAL_LIGHTS {
            if e.csv_row != 147 {
                m.set(e.addr, e.original);
            }
        }
        let mut p = Patcher::new();
        let r = apply(&mut p, &mut m, MANUAL_LIGHTS);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.applied.len(), MANUAL_LIGHTS.len());
        assert_eq!(m.get(0x6E1BE3, 2), &[0x90, 0x90]);
        assert_eq!(m.get(0x6E1BE5, 2), &[0x8A, 0x86]);
    }
}
