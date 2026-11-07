//! World-anchored overlays the Present pass draws (nametags, chat bubbles) and the pause-menu map click.

use super::State;
use super::adapters::{PED_VEHICLE, bone_pos, camera_pos, dist, ped_pos, ped_valid};
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner};
use crate::ui::render::{BubbleDraw, TagDraw};

/// `CPhysical::m_pMovingListEntry`: non-zero while the entity is in the moving list.
const PHYSICAL_MOVING_LIST_ENTRY: u32 = 0xB4;
/// `IS_LINE_OF_SIGHT_CLEAR`, the name-tag line-of-sight test.
const OP_IS_LINE_OF_SIGHT_CLEAR: u16 = 0x06BD;

/// Nametags to draw this frame (docs/re/subsystems/ui-rendering.md, "16.1 Who gets one").
pub(super) fn nametags(st: &mut State) -> Vec<TagDraw> {
    let core = &st.core;
    if !core.show_name_tags {
        return Vec::new();
    }
    let cam = camera_pos();
    let mut out = Vec::new();
    for r in &st.remotes {
        if r.ped == 0 || r.state == 0 || r.npc || core.hidden_tags.contains(&r.id) || !ped_valid(r.ped) {
            continue;
        }
        let Some(p) = ped_pos(r.ped) else { continue };
        let d = dist(p, cam);
        if d >= core.name_tag_distance {
            continue;
        }
        // Only the driver of an RC vehicle (models 441/464/465/501/564/594) is anchored at the vehicle's
        // matrix position; every other player, drivers of ordinary cars included, must be in the moving
        // list (`CPhysical` +0xB4 ≠ 0) and is anchored at bone 8 (head).
        let vehicle = if r.state == crate::vehicle::state::DRIVER {
            let v = ProcessMemory.read_u32(r.ped + PED_VEHICLE).unwrap_or(0);
            let model =
                ped_valid(v).then(|| ProcessMemory.read_u16(v + off::ENTITY_MODEL).ok()).flatten().map(u32::from);
            if crate::vehicle::tag_at_vehicle(r.state, model) { v } else { 0 }
        } else {
            0
        };
        let anchor = if vehicle != 0 {
            let Some(v) = ped_pos(vehicle) else { continue };
            v
        } else {
            if ProcessMemory.read_u32(r.ped + PHYSICAL_MOVING_LIST_ENTRY).unwrap_or(0) == 0 {
                continue;
            }
            bone_pos(r.ped, 8)
        };
        if core.name_tag_los {
            let los = |run: &mut Runner| {
                let a = [anchor[0], anchor[1], anchor[2], cam[0], cam[1], cam[2]].map(Arg::Float);
                let flags = [1, 0, 0, 1, 0].map(Arg::Int);
                let args: Vec<Arg> = a.into_iter().chain(flags).collect();
                run.run(OP_IS_LINE_OF_SIGHT_CLEAR, &args)
            };
            // Tested twice, once per samp pass (bars, then names).
            let a = los(&mut st.runner);
            let b = los(&mut st.runner);
            if !(a && b) {
                continue;
            }
        }
        let Some(name) = core.name(r.id) else { continue };
        let mut text = name.to_vec();
        text.extend_from_slice(format!(" ({})", r.id).as_bytes());
        out.push(TagDraw {
            anchor,
            distance: d,
            name: text,
            colour: core.colours.chat_argb(r.id) | 0xFF00_0000,
            health: r.health,
            armour: r.armour,
            afk: r.ms_since_sync >= samp_ui::nametags::AFK_AFTER_MS,
        });
    }
    out
}

/// Chat bubbles to draw (docs/re/subsystems/ui-rendering.md, "11. Chat bubbles").
pub(super) fn bubbles(st: &State) -> Vec<BubbleDraw> {
    let local = game::read_player(&ProcessMemory).ok().flatten();
    let cam = camera_pos();
    let mut out = Vec::new();
    for r in &st.remotes {
        let Some(b) = st.core.bubbles.get(r.id) else { continue };
        if r.ped == 0 || r.state == 0 || !ped_valid(r.ped) {
            continue;
        }
        let Some(p) = ped_pos(r.ped) else { continue };
        let d_local = local.map_or(f32::MAX, |l| dist(p, l.pos));
        if d_local >= b.distance {
            continue;
        }
        let head = bone_pos(r.ped, samp_ui::bubbles::ANCHOR_BONE);
        let raise = b.anchor_raise(dist(p, cam));
        out.push(BubbleDraw { anchor: [head[0], head[1], head[2] + raise], text: b.text.clone(), colour: b.colour });
    }
    out
}

/// Pause-menu map right-click (docs/re/subsystems/session-misc.md, "119 OnPlayerClickMap"): with the map page
/// shown, the right button's "pressed since the last call" bit set and the map not blocked, the radar-trace
/// entries are scanned for the waypoint (the last match wins); its x/y go out with
/// z = `CWorld::FindGroundZForCoord(x, y)` + 2. samp sends uninitialised x/y when there is no waypoint; we send
/// nothing then (docs/phase3/feata-notes.md). Mirrors samp.dll B 0x10075B7B.
pub(super) fn map_click() -> Option<[f32; 3]> {
    use crate::worldhooks::rd;
    if rd::<u32>(MENU_MAP_PAGE) == 0 {
        return None;
    }
    // SAFETY: plain Win32 query.
    let pressed = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x02) } & 1 != 0;
    if !pressed || rd::<u8>(MENU_MAP_BLOCK) != 0 {
        return None;
    }
    let mut found = None;
    for i in 1..=RADAR_TRACE_SCAN as u32 {
        let e = RADAR_TRACE + i * RADAR_TRACE_STRIDE;
        if rd::<u16>(e + 0x25) == WAYPOINT_TAG {
            found = Some((rd::<f32>(e + 8), rd::<f32>(e + 0xC)));
        }
    }
    let (x, y) = found?;
    // SAFETY: CWorld::FindGroundZForCoord(float, float) (cdecl, returns float), main thread.
    let z = unsafe {
        let f: extern "C" fn(f32, f32) -> f32 = std::mem::transmute(FIND_GROUND_Z_FOR_COORD as usize);
        f(x, y)
    };
    Some([x, y, z + 2.0])
}

/// Non-zero while the pause menu shows the map page.
const MENU_MAP_PAGE: u32 = 0xBA6774;
/// Non-zero when the map click must be ignored.
const MENU_MAP_BLOCK: u32 = 0xBA8298;
/// GTA's radar-trace (blip) array.
const RADAR_TRACE: u32 = 0xBA86F0;
/// Size of one radar-trace entry.
const RADAR_TRACE_STRIDE: u32 = 0x28;
/// Last radar-trace entry samp scans (from entry 1).
const RADAR_TRACE_SCAN: usize = 175;
/// The u16 at +0x25 that marks the waypoint entry.
const WAYPOINT_TAG: u16 = 0x1203;
/// `CWorld::FindGroundZForCoord(x, y)`.
const FIND_GROUND_Z_FOR_COORD: u32 = 0x569660;
