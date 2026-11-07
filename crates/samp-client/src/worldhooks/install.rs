//! Installs every world-entity hook.

use super::buildings::load_object_instance_stub;
use super::collision::{
    BUILD_CACHE_OF_CAMERA_COLLISION, build_cache_of_camera_collision, camera_ignore_object_stub,
    check_camera_collision_vehicles_stub, process_vertical_line_stub,
};
use super::entity::{
    ADD_ENTITY_CALL, ADD_ENTITY_CALL_BYTES, ESCALATOR_FLOATS, ESCALATOR_SWITCH_OFF_BYTES, ESCALATOR_SWITCH_OFF_CALL,
    PROCESS_ATTACHED_ENTITIES, entity_delete_rw, escalator_update, object_create_rw, position_attached_entity,
    process_attached_entities,
};
use super::materials::object_render;
use super::peds::{civilian_process_control, ped_render, process_peds_after_pre_render};
use super::pickups::pickup_update_stub;
use super::radar::{draw_gang_zones, radar_trace_colour};
use super::render::{
    add_entity_to_render_list_stub, after_render_everything_bar_roads, after_render_fading_in, roadsign_guard_stub,
};
use super::*;

fn expect_ptr(slot: u32, want: u32) -> Result<(), PatchError> {
    let found = ProcessMemory.read_u32(slot)?;
    if found == want {
        Ok(())
    } else {
        Err(PatchError::Mismatch {
            addr: slot,
            expected: want.to_le_bytes().to_vec(),
            found: found.to_le_bytes().to_vec(),
        })
    }
}

/// samp's hook-jmp-indirect form: the handler address in `slot`, `jmp dword [slot]` (+ NOP padding) at `site`.
pub(crate) fn jmp_indirect(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    label: &'static str,
    site: u32,
    slot: u32,
    handler: u32,
    original: &[u8],
) -> Result<(), PatchError> {
    let mut b = vec![0xFF, 0x25];
    b.extend_from_slice(&slot.to_le_bytes());
    b.resize(original.len(), 0x90);
    let mut cur = vec![0u8; original.len()];
    mem.read(site, &mut cur)?;
    if cur != original {
        return Err(PatchError::Mismatch { addr: site, expected: original.to_vec(), found: cur });
    }
    patcher.write(mem, label, slot, &handler.to_le_bytes(), None)?;
    patcher.write(mem, label, site, &b, Some(original))
}

fn unprotect(addr: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    let mut old = 0u32;
    // SAFETY: changes the protection of a range of gta_sa.exe's image.
    unsafe { VirtualProtect(addr as usize as *const _, len, PAGE_EXECUTE_READWRITE, &mut old) != 0 }
}

/// Pickup visibility distance that `CPickup::IsVisible`'s `fcomp` is pointed at (patch table row 211). Mirrors
/// samp.dll 0x1011671C.
static PICKUP_VISIBLE_DISTANCE: f32 = 10000.0;

/// Byte rows, all applied at init-main: (row, address, new bytes, original bytes, label).
type ByteRow = (u32, u32, &'static [u8], &'static [u8], &'static str);
/// (row, site, slot, handler, original bytes, label).
type IndirectRow = (u32, u32, u32, u32, &'static [u8], &'static str);

const BYTE_ROWS: &[ByteRow] = &[(
    113,
    0x551097,
    &[0xB8, 0x0B, 0x00, 0x00],
    &[0x5E, 0x01, 0x00, 0x00],
    "CPools::Initialise object pool 350 -> 3000",
)];

/// (patch table row, result) per row, for the log.
type Report = Vec<(u32, Result<(), PatchError>)>;
/// (row, call site, new target, original target, label).
type CallRow = (u32, u32, u32, u32, &'static str);

/// Installs every world-entity row. Returns (patch table row, result) per row for the log.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Report {
    let mut out = Vec::new();
    install_byte_rows(patcher, mem, &mut out);
    install_vtable_slots(patcher, mem, &mut out);
    for (row, site, target, orig, label) in call_rows() {
        out.push((row, patcher.retarget_branch(mem, label, site, target, Some(orig)).map(|_| ())));
    }
    make_temporary_patch_sites_writable(&mut out);
    install_indirect_rows(patcher, mem, &mut out);
    with_shared(|_| ());
    out
}

/// The byte rows and the pickup visibility distance pointer (patch table row 211).
fn install_byte_rows(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    for &(row, addr, new, orig, label) in BYTE_ROWS {
        out.push((row, patcher.write(mem, label, addr, new, Some(orig))));
    }
    let vis = &PICKUP_VISIBLE_DISTANCE as *const f32 as u32;
    out.push((
        211,
        patcher.write(
            mem,
            "CPickup::IsVisible distance 10000",
            0x454CC9,
            &vis.to_le_bytes(),
            Some(&0x858628u32.to_le_bytes()),
        ),
    ));
}

/// Checks that a vtable slot still holds GTA's function, then points it at our handler.
fn swap_slot(
    patcher: &mut Patcher,
    mem: &mut ProcessMemory,
    label: &'static str,
    slot: u32,
    want: u32,
    handler: u32,
) -> Result<(), PatchError> {
    expect_ptr(slot, want).and_then(|()| patcher.swap_pointer(mem, label, slot, handler).map(|_| ()))
}

/// Vtable slots: object render and RW create/delete, ped render, civilian ProcessControl.
fn install_vtable_slots(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    let render = object_render as extern "thiscall" fn(u32) as usize as u32;
    out.push((63, swap_slot(patcher, mem, "CObject::Render", 0x866FA8, g::OBJECT_RENDER, render)));
    let create = object_create_rw as extern "thiscall" fn(u32) as usize as u32;
    out.push((67, swap_slot(patcher, mem, "CObject::CreateRwObject", 0x866F7C, 0x59F110, create)));
    let delete = entity_delete_rw as extern "thiscall" fn(u32) as usize as u32;
    for (row, slot) in [(68, 0x866F80), (69, 0x8585E8)] {
        out.push((row, swap_slot(patcher, mem, "DeleteRwObject", slot, g::ENTITY_DELETE_RW, delete)));
    }
    let prc = ped_render as extern "thiscall" fn(u32) as usize as u32;
    for (row, slot) in [(11, 0x86D1B0), (12, 0x86C0F0), (13, 0x86C168), (14, 0x86C248), (15, 0x86C3A0)] {
        out.push((row, swap_slot(patcher, mem, "ped Render", slot, g::PED_RENDER, prc)));
    }
    let cpc = civilian_process_control as extern "thiscall" fn(u32) as usize as u32;
    out.push((20, swap_slot(patcher, mem, "CCivilianPed::ProcessControl", 0x86C0D0, g::CIVILIAN_PROCESS_CONTROL, cpc)));
}

/// Call sites retargeted to our handlers, in install order.
fn call_rows() -> [CallRow; 12] {
    [
        (
            24,
            0x5689FD,
            process_attached_entities as extern "C" fn() as usize as u32,
            PROCESS_ATTACHED_ENTITIES,
            "CWorld::Process ProcessAttachedEntities (surfing)",
        ),
        (
            25,
            0x53EA03,
            process_peds_after_pre_render as extern "C" fn() as usize as u32,
            g::PROCESS_PEDS_AFTER_PRE_RENDER,
            "Idle ProcessPedsAfterPreRender",
        ),
        (
            41,
            0x5869BF,
            draw_gang_zones as extern "C" fn() as usize as u32,
            g::DRAW_RADAR_GANG_OVERLAY,
            "DrawRadarMap gang overlay",
        ),
        (
            42,
            0x5759E4,
            draw_gang_zones as extern "C" fn() as usize as u32,
            g::DRAW_RADAR_GANG_OVERLAY,
            "PrintMap gang overlay",
        ),
        (
            66,
            0x718599,
            escalator_update as extern "thiscall" fn(u32) as usize as u32,
            g::ESCALATOR_UPDATE,
            "CEscalators::Update",
        ),
        (
            75,
            0x5648D3,
            position_attached_entity as extern "thiscall" fn(u32) -> u32 as usize as u32,
            g::POSITION_ATTACHED_ENTITY,
            "ProcessAttachedEntities",
        ),
        (
            76,
            0x53DFDD,
            after_render_everything_bar_roads as extern "C" fn() as usize as u32,
            g::RENDER_EVERYTHING_BAR_ROADS,
            "RenderScene pass 1",
        ),
        (
            77,
            0x53E019,
            after_render_fading_in as extern "C" fn() as usize as u32,
            g::RENDER_FADING_IN,
            "RenderScene pass 2",
        ),
        (
            79,
            0x5342F9,
            roadsign_guard_stub as unsafe extern "C" fn() as usize as u32,
            g::ROADSIGN_RENDER_ATOMIC,
            "RenderEffects roadsign guard",
        ),
        (
            81,
            0x41B02E,
            build_cache_of_camera_collision as extern "C" fn(u32, u32) -> u32 as usize as u32,
            BUILD_CACHE_OF_CAMERA_COLLISION,
            "camera collision cache",
        ),
        (
            82,
            0x41AF80,
            check_camera_collision_vehicles_stub as unsafe extern "C" fn() as usize as u32,
            g::CHECK_CAMERA_COLLISION_VEHICLES,
            "camera collision vehicles",
        ),
        (
            83,
            0x41AB78,
            camera_ignore_object_stub as unsafe extern "C" fn() as usize as u32,
            g::CAMERA_TO_IGNORE_THIS_OBJECT,
            "camera ignores attached objects",
        ),
    ]
}

/// Patch table rows 371/372/373: the escalator floats and the two call sites that get temporary NOPs are written
/// around each call, so they are made writable once here.
fn make_temporary_patch_sites_writable(out: &mut Report) {
    out.push((
        371,
        if ESCALATOR_FLOATS.iter().all(|(a, _)| unprotect(*a, 4)) {
            Ok(())
        } else {
            Err(PatchError::Mismatch { addr: 0x858BA4, expected: vec![], found: vec![] })
        },
    ));
    let ok372 = unprotect(ADD_ENTITY_CALL, 3) && ProcessMemory.read_u8(ADD_ENTITY_CALL).ok() == Some(0xFF);
    out.push((
        372,
        if ok372 {
            Ok(())
        } else {
            Err(PatchError::Mismatch { addr: ADD_ENTITY_CALL, expected: ADD_ENTITY_CALL_BYTES.to_vec(), found: vec![] })
        },
    ));
    let ok373 =
        unprotect(ESCALATOR_SWITCH_OFF_CALL, 5) && ProcessMemory.read_u8(ESCALATOR_SWITCH_OFF_CALL).ok() == Some(0xE8);
    out.push((
        373,
        if ok373 {
            Ok(())
        } else {
            Err(PatchError::Mismatch {
                addr: ESCALATOR_SWITCH_OFF_CALL,
                expected: ESCALATOR_SWITCH_OFF_BYTES.to_vec(),
                found: vec![],
            })
        },
    ));
}

/// The jmp-indirect entries, then the pickup-collision jmp over `mov al,[esi+0x1C]; cmp al,6` (patch table
/// row 44).
fn install_indirect_rows(patcher: &mut Patcher, mem: &mut ProcessMemory, out: &mut Report) {
    let ind: [IndirectRow; 4] = [
        (
            96,
            0x5674E0,
            0x5674DB,
            process_vertical_line_stub as unsafe extern "C" fn() as usize as u32,
            &[0x83, 0xEC, 0x2C, 0x90, 0x90, 0x90],
            "CWorld::ProcessVerticalLine",
        ),
        (
            51,
            0x584770,
            0x584A79,
            radar_trace_colour as extern "C" fn(u32) -> u32 as usize as u32,
            &[0x8B, 0x44, 0x24, 0x04, 0x83, 0xF8, 0x08],
            "CRadar::GetRadarTraceColour",
        ),
        (
            65,
            0x538090,
            0x538084,
            load_object_instance_stub as unsafe extern "C" fn() as usize as u32,
            &[0x6A, 0xFF, 0x68, 0x31, 0xC9, 0x83, 0x00],
            "CFileLoader::LoadObjectInstance",
        ),
        (
            78,
            0x5534B0,
            0x5534A6,
            add_entity_to_render_list_stub as unsafe extern "C" fn() as usize as u32,
            &[0x56, 0x8B, 0x74, 0x24, 0x08, 0x0F, 0xBF, 0x46, 0x22],
            "CRenderer::AddEntityToRenderList",
        ),
    ];
    for (row, site, slot, handler, orig, label) in ind {
        out.push((row, jmp_indirect(patcher, mem, label, site, slot, handler, orig)));
    }
    let pk = pickup_update_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        44,
        patcher.hook_jmp(mem, "CPickup::Update collision", 0x4579C6, pk, Some(&[0x8A, 0x46, 0x1C, 0x3C, 0x06])),
    ));
}
