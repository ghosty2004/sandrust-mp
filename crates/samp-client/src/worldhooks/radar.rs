//! Radar trace colours and gang zones.

use super::*;

// ---- radar trace colour (patch table row 51) -------------------------------------------------------------------

pub(super) extern "C" fn radar_trace_colour(index: u32) -> u32 {
    try_shared(|s| {
        let c = s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new);
        world::radar_trace_colour(index, |id| c.rgba(id))
    })
    .unwrap_or_else(|| world::radar_trace_colour(index, |_| 0))
}

// ---- gang zones (patch table rows 41/42) -------------------------------------------------------------------------

/// Replaces the `CRadar::DrawRadarGangOverlay` calls: every shown zone, in id order, through
/// `CRadar::DrawAreaOnRadar(rect, colour, in_menu)` with the 500 ms flash phase. Mirrors samp.dll 0x100A3F40.
pub(super) extern "C" fn draw_gang_zones() {
    let now = tick_ms();
    ZONE_CALLS[0].fetch_add(1, Ordering::Relaxed);
    let list: Vec<([f32; 4], u32)> = try_shared(|s| {
        if !s.active {
            return Vec::new();
        }
        let phase = s.flash.update(now);
        s.zones.values().map(|z| (z.rect, z.colour_now(phase))).collect()
    })
    .unwrap_or_else(|| {
        ZONE_CALLS[1].fetch_add(1, Ordering::Relaxed);
        Vec::new()
    });
    ZONE_CALLS[2].fetch_add(list.len() as u32, Ordering::Relaxed);
    let in_menu = u32::from(rd::<u8>(g::RADAR_IN_MENU));
    for (rect, colour) in list {
        let crgba = world::crgba_bytes(colour);
        // SAFETY: CRadar::DrawAreaOnRadar(const CRect&, const CRGBA&, bool) (cdecl) with locals.
        unsafe {
            let f: extern "C" fn(*const f32, *const u8, u32) = std::mem::transmute(g::DRAW_AREA_ON_RADAR as usize);
            f(rect.as_ptr(), crgba.as_ptr(), in_menu);
        }
    }
}

pub fn tick_ms() -> u32 {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}
