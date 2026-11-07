//! Nametags: health/armour bars ([`bars`]), the name rect and the AFK icon. `x`, `y` are the
//! truncated (`_ftol2`) screen coordinates of the projected anchor `(px, py, pz + 0.0475·d + 0.2)`;
//! a tag is drawn only when the projected z ≤ 1. Spec: docs/re/subsystems/ui-rendering.md
//! ("Nametags and health/armour bars"); measured pixels in docs/phase3/trace-notes.md.

/// Anchor raise: `0.0475·d + 0.2` above the ped (d = camera distance).
pub const RAISE_PER_METRE: f32 = 0.0475;
pub const RAISE_BASE: f32 = 0.2;
/// Bar width per health point (100 → 38.46 px).
pub const BAR_SCALE: f32 = 0.384_615_4;
pub const BORDER: u32 = 0xFF00_0000;
pub const HEALTH_BG: u32 = 0xFF4B_0B14;
pub const HEALTH_FILL: u32 = 0xFFB9_2228;
pub const ARMOUR_BG: u32 = 0xFF28_2828;
pub const ARMOUR_FILL: u32 = 0xFFC8_C8C8;
/// AFK glyph colours (SAMPAUX3 22 `C` backdrop, SAMPAUX3 20 `E`).
pub const AFK_BACKDROP: u32 = 0xFF00_0000;
pub const AFK_ICON: u32 = 0xFFE3_E1E3;
/// No update for this long → paused (AFK icon); the flag is remote-player field +0x1c5 in samp.
pub const AFK_AFTER_MS: u64 = 3000;

/// A filled quad (screen pixels, pre-transformed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub colour: u32,
}

/// One bar set: border, background, fill.
fn bar(x: f32, y: f32, value: f32, bg: u32, fill: u32) -> [Quad; 3] {
    [
        Quad { x0: x - 20.0, y0: y + 18.0, x1: x + 21.0, y1: y + 24.0, colour: BORDER },
        Quad { x0: x - 19.0, y0: y + 19.0, x1: x + 19.0, y1: y + 23.0, colour: bg },
        Quad { x0: x - 19.0, y0: y + 19.0, x1: x - 19.0 + value * BAR_SCALE, y1: y + 23.0, colour: fill },
    ]
}

/// Bar groups in draw order (one `SetFVF` + three fans per group). Health moves down
/// 8 px when armour is shown; armour is drawn after it at the original height.
/// Nothing is drawn for `hp < 0`. Mirrors samp.dll 0x1006d0a0.
pub fn bars(x: i32, y: i32, hp: f32, armour: f32) -> Vec<[Quad; 3]> {
    if hp < 0.0 {
        return Vec::new();
    }
    let (fx, fy) = (x as f32, y as f32);
    let hp = hp.min(100.0);
    let ar = armour.min(100.0);
    if ar > 0.0 {
        vec![bar(fx, fy + 8.0, hp, HEALTH_BG, HEALTH_FILL), bar(fx, fy, ar, ARMOUR_BG, ARMOUR_FILL)]
    } else {
        vec![bar(fx, fy, hp, HEALTH_BG, HEALTH_FILL)]
    }
}

/// Name rect: centred on `x` with the measured width, top `y + 17 − h`, bottom `y`.
pub fn name_rect(x: i32, y: i32, w: i32, h: i32) -> [i32; 4] {
    [x - w / 2, y + 17 - h, x + w / 2, y]
}

/// AFK glyph rects from the `C` measure (`DT_CALCRECT|DT_CENTER|DT_VCENTER` of an
/// empty rect, e.g. [−7, −11, 8, 11]): backdrop `C` at [nameLeft + l, y + 17 + b, +1, +2],
/// icon `E` one px left and two up (measured once, at width 1280).
pub fn afk_rects(name_left: i32, y: i32, measure: [i32; 4]) -> ([i32; 4], [i32; 4]) {
    let l = name_left + measure[0];
    let t = y + 17 + measure[3];
    ([l, t, l + 1, t + 2], [l - 1, t - 2, l + 1, t + 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace3_bars_with_and_without_armour() {
        // Values measured on the original: X 826, Y 278, hp 56, armour 100.
        let g = bars(826, 278, 56.0, 100.0);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0][0].x0, g[0][0].y0, g[0][0].x1, g[0][0].y1), (806.0, 304.0, 847.0, 310.0));
        assert!((g[0][2].x1 - 828.538).abs() < 0.01);
        assert_eq!((g[1][0].y0, g[1][1].colour), (296.0, ARMOUR_BG));
        assert!((g[1][2].x1 - 845.462).abs() < 0.01);
        let g = bars(502, 284, 100.0, 0.0);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0][1].x0, g[0][1].y0, g[0][1].x1), (483.0, 303.0, 521.0));
        assert!(bars(0, 0, -1.0, 0.0).is_empty());
    }

    #[test]
    fn trace3_name_and_afk() {
        assert_eq!(name_rect(826, 278, 45, 14), [804, 281, 848, 278]);
        let (c, e) = afk_rects(804, 278, [-7, -11, 8, 11]);
        assert_eq!(c, [797, 306, 798, 308]);
        assert_eq!(e, [796, 304, 798, 308]);
    }
}
