//! Game adapters for the UI: F5 values from GTA's tables and the process, key polling, the HUD flags,
//! GTA's `~k~` expansion, the local-command game queries, and ped / camera positions.

use samp_ui::commands::{GameInfo, PlayerPosition};
use samp_ui::text::KeyNameExpander;

use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner};

/// `FrontEndMenuManager.m_bMenuActive`: while set, samp draws no UI.
pub(super) const GTA_MENU_ACTIVE: u32 = 0xBA67A4;
/// `CMessages::InsertPlayerControlKeysInString` (expands `~k~` key names).
const INSERT_PLAYER_CONTROL_KEYS: u32 = 0x69E160;
/// `CStreaming::ms_memoryAvailable`: the memory counter shown by the `/mem` local command and F5.
const MEMORY_COUNTER: u32 = 0x8A5A80;
/// `CStreaming::ms_memoryUsed`.
const STREAMING_MEMORY_USED: u32 = 0x8E4CB4;
/// Script opcodes (gta-reversed `eScriptCommands.h`).
const OP_GET_AREA_VISIBLE: u16 = 0x077E;
/// `GET_CHAR_HEADING` / `GET_CAR_HEADING`, used by `/save` and `/rs`.
const OP_GET_CHAR_HEADING: u16 = 0x0172;
const OP_GET_CAR_HEADING: u16 = 0x0174;
/// `CPed::m_pVehicle`, vehicle colours (gta-reversed `Ped.h` / `Vehicle.h`).
pub(super) const PED_VEHICLE: u32 = 0x58C;
const VEHICLE_COLOUR1: u32 = 0x434;
const VEHICLE_COLOUR2: u32 = 0x435;

/// `CStreaming::ms_aInfoForModel` (20-byte entries, load state at +0x10).
const STREAMING_INFO: u32 = 0x8E4CC0;
/// `CModelInfo::ms_modelInfoPtrs` (20000 entries).
const MODEL_INFO_PTRS: u32 = 0xA9B0C8;
/// The RenderWare object counter F5 shows as RWObjects.
const RW_OBJECT_COUNT: u32 = 0xB71804;

/// F5 values: memory from the process, GTA's streaming / model tables (vehicle models 400…610
/// with a non-zero load state, null model-info slots among ids 1000…19999, the RW object
/// counter), the rest from what our client counts (pools from the session tick, the network
/// block from our reliability layer).
pub(super) fn net_stats_values(base: &crate::ui::core::NetStats) -> crate::ui::core::NetStats {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut s = base.clone();
    let m = ProcessMemory;
    s.streaming_mb = m.read_u32(STREAMING_MEMORY_USED).unwrap_or(0) / (1024 * 1024);
    s.streaming_total_mb = m.read_u32(MEMORY_COUNTER).unwrap_or(0) / (1024 * 1024);
    let mut info = vec![0u8; 211 * 20];
    if m.read(STREAMING_INFO + 400 * 20, &mut info).is_ok() {
        s.vehicle_models_loaded = info.as_chunks::<20>().0.iter().filter(|e| e[0x10..0x14] != [0; 4]).count() as u32;
    }
    s.rw_objects = m.read_u32(RW_OBJECT_COUNT).unwrap_or(0);
    let mut ptrs = vec![0u8; 19000 * 4];
    if m.read(MODEL_INFO_PTRS + 1000 * 4, &mut ptrs).is_ok() {
        s.free_ides = ptrs.as_chunks::<4>().0.iter().filter(|p| **p == [0; 4]).count() as u32;
    }
    // SAFETY: plain Win32 queries into correctly sized structs.
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            s.process_kb = (pmc.PagefileUsage / 1024) as u32;
            s.working_set_kb = (pmc.WorkingSetSize / 1024) as u32;
        }
        let mut ms: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of_val(&ms) as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut ms) != 0 {
            s.system_mb = (ms.ullTotalPhys / (1024 * 1024)) as u32;
        }
    }
    s
}

/// `GetAsyncKeyState(vk) != 0` as samp's Present pass tests it (either bit).
pub(super) fn key_polled(vk: i32) -> bool {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) != 0 }
}

/// HUD on/off as samp's Present pass does it: the menu HUD flag and the script radar flag.
pub(super) fn set_hud(on: bool) {
    let mut m = ProcessMemory;
    let _ = m.write(game::MENU_HUD_ON, &[u8::from(on)]);
    let _ = m.write(game::HUD_SCRIPT_DONT_DISPLAY_RADAR, &[u8::from(!on)]);
}

pub(super) fn gta_menu_active() -> bool {
    ProcessMemory.read_u8(GTA_MENU_ACTIVE).unwrap_or(0) != 0
}

/// GTA's `~k~` substitution on a private buffer.
pub(crate) struct GameExpander;

impl KeyNameExpander for GameExpander {
    fn expand(&self, text: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 2048];
        let n = text.len().min(1000);
        buf[..n].copy_from_slice(&text[..n]);
        // SAFETY: cdecl `void(char*)` editing a NUL-terminated buffer in place; at most 1000 input bytes in a 2048-byte buffer.
        unsafe {
            let f: extern "C" fn(*mut u8) = std::mem::transmute(INSERT_PLAYER_CONTROL_KEYS as usize);
            f(buf.as_mut_ptr());
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(end);
        buf
    }
}

/// Game queries for local commands (read on the main thread).
pub(super) struct LiveGame {
    pub(super) connected: bool,
}

impl GameInfo for LiveGame {
    fn connected(&self) -> bool {
        self.connected
    }
    fn memory_counter(&self) -> u32 {
        ProcessMemory.read_u32(MEMORY_COUNTER).unwrap_or(0)
    }
    fn interior(&self) -> u32 {
        let mut r = Runner::new();
        r.run(OP_GET_AREA_VISIBLE, &[Arg::Local(0)]);
        r.local_i32(0) as u32
    }
    /// `/save` and `/rs`: in a vehicle (ped flags bit 8) the vehicle's matrix position, `GET_CAR_HEADING`,
    /// model and colours; on foot the ped's matrix position and `GET_CHAR_HEADING`. Both commands read the
    /// same values.
    fn position(&self) -> Option<PlayerPosition> {
        let mem = ProcessMemory;
        let p = game::read_player(&mem).ok().flatten()?;
        let in_vehicle = mem.read_u32(p.ptr + off::PED_FLAGS).unwrap_or(0) & (1 << 8) != 0;
        let veh = mem.read_u32(p.ptr + PED_VEHICLE).unwrap_or(0);
        let mut r = Runner::new();
        if in_vehicle && veh != 0 {
            let m = mem.read_u32(veh + off::PLACEABLE_MATRIX).unwrap_or(0);
            let rd = |a: u32| mem.read_f32(a).unwrap_or(0.0);
            let pos = if m != 0 { [rd(m + 0x30), rd(m + 0x34), rd(m + 0x38)] } else { p.pos };
            r.run(OP_GET_CAR_HEADING, &[Arg::Int(crate::vehicles::pool_ref(veh)), Arg::Local(0)]);
            return Some(PlayerPosition::InCar {
                model: u32::from(mem.read_u16(veh + off::ENTITY_MODEL).unwrap_or(0)),
                x: pos[0],
                y: pos[1],
                z: pos[2],
                heading: r.local_f32(0),
                colour1: u32::from(mem.read_u8(veh + VEHICLE_COLOUR1).unwrap_or(0)),
                colour2: u32::from(mem.read_u8(veh + VEHICLE_COLOUR2).unwrap_or(0)),
            });
        }
        let handle = game::ped_handle(&mem, p.ptr).ok().flatten().unwrap_or(-1);
        r.run(OP_GET_CHAR_HEADING, &[Arg::Int(handle), Arg::Local(0)]);
        Some(PlayerPosition::OnFoot {
            skin: u32::from(mem.read_u16(p.ptr + off::ENTITY_MODEL).unwrap_or(0)),
            x: p.pos[0],
            y: p.pos[1],
            z: p.pos[2],
            heading: r.local_f32(0),
        })
    }
}

/// Camera position (`TheCamera`), as samp's distance helper reads it.
const CAMERA_POS: u32 = 0xB6F9CC;
/// `CPed::GetBonePosition(RwV3d&, uint32 bone, bool)` (thiscall).
const PED_GET_BONE_POSITION: u32 = 0x5E4280;

pub(super) fn camera_pos() -> [f32; 3] {
    let m = ProcessMemory;
    [0, 4, 8].map(|o| m.read_f32(CAMERA_POS + o).unwrap_or(0.0))
}

pub(super) fn ped_pos(ped: u32) -> Option<[f32; 3]> {
    let m = ProcessMemory;
    let mat = m.read_u32(ped + off::PLACEABLE_MATRIX).ok()?;
    let a = if mat != 0 { mat + off::MATRIX_POS } else { ped + off::PLACEABLE_PLACEMENT };
    Some([m.read_f32(a).ok()?, m.read_f32(a + 4).ok()?, m.read_f32(a + 8).ok()?])
}

pub(super) fn bone_pos(ped: u32, bone: u32) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    // SAFETY: GTA's thiscall CPed::GetBonePosition on a live ped, main thread.
    unsafe {
        let f: extern "thiscall" fn(u32, *mut [f32; 3], u32, u8) = std::mem::transmute(PED_GET_BONE_POSITION as usize);
        f(ped, &mut out, bone, 0);
    }
    out
}

pub(super) fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// `CPlaceable` vtable: a deleted entity (samp's validity test).
const VTBL_CPLACEABLE: u32 = 0x863C40;

pub(super) fn ped_valid(ped: u32) -> bool {
    ped != 0 && ProcessMemory.read_u32(ped).is_ok_and(|v| v != VTBL_CPLACEABLE)
}
