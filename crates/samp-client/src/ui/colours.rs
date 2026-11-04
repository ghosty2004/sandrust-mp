//! Player colour table used for chat names (UCA §101, player-state.md §72).
//!
//! samp keeps a u32 RGBA colour per player id (array A 0x101516a0, 4000 entries).
//! At start-up its first 1000 entries are copied from a static table in samp .data
//! (A 0x10113b48, 1000 × u32, copy routine A 0x100a72c0); that table repeats the
//! 100 values below with period 100 (verified by reading both R4 builds' .data).
//! Entries 1000..3999 start at 0. Setter A 0x100a7250 (`id < 4000`), getter
//! A 0x100a7270 (ids 0x3EC/0x3ED/0x3EE return fixed colours, not used by chat).

/// Number of entries of the table (A 0x101516a0).
pub const TABLE_LEN: usize = 4000;
/// Entries initialised from [`DEFAULT_COLOURS`] at start-up.
pub const INITIALISED: usize = 1000;

/// The 100 default colours (RGBA), samp .data A 0x10113b48.
pub const DEFAULT_COLOURS: [u32; 100] = [
    0xFF8C13FF, 0xC715FFFF, 0x20B2AAFF, 0xDC143CFF, 0x6495EDFF, 0xF0E68CFF, 0x778899FF, 0xFF1493FF, 0xF4A460FF,
    0xEE82EEFF, 0xFFD720FF, 0x8B4513FF, 0x4949A0FF, 0x148B8BFF, 0x14FF7FFF, 0x556B2FFF, 0x0FD9FAFF, 0x10DC29FF,
    0x534081FF, 0x0495CDFF, 0xEF6CE8FF, 0xBD34DAFF, 0x247C1BFF, 0x0C8E5DFF, 0x635B03FF, 0xCB7ED3FF, 0x65ADEBFF,
    0x5C1ACCFF, 0xF2F853FF, 0x11F891FF, 0x7B39AAFF, 0x53EB10FF, 0x54137DFF, 0x275222FF, 0xF09F5BFF, 0x3D0A4FFF,
    0x22F767FF, 0xD63034FF, 0x9A6980FF, 0xDFB935FF, 0x3793FAFF, 0x90239DFF, 0xE9AB2FFF, 0xAF2FF3FF, 0x057F94FF,
    0xB98519FF, 0x388EEAFF, 0x028151FF, 0xA55043FF, 0x0DE018FF, 0x93AB1CFF, 0x95BAF0FF, 0x369976FF, 0x18F71FFF,
    0x4B8987FF, 0x491B9EFF, 0x829DC7FF, 0xBCE635FF, 0xCEA6DFFF, 0x20D4ADFF, 0x2D74FDFF, 0x3C1C0DFF, 0x12D6D4FF,
    0x48C000FF, 0x2A51E2FF, 0xE3AC12FF, 0xFC42A8FF, 0x2FC827FF, 0x1A30BFFF, 0xB740C2FF, 0x42ACF5FF, 0x2FD9DEFF,
    0xFAFB71FF, 0x05D1CDFF, 0xC471BDFF, 0x94436EFF, 0xC1F7ECFF, 0xCE79EEFF, 0xBD1EF2FF, 0x93B7E4FF, 0x3214AAFF,
    0x184D3BFF, 0xAE4B99FF, 0x7E49D7FF, 0x4C436EFF, 0xFA24CCFF, 0xCE76BEFF, 0xA04E0AFF, 0x9F945CFF, 0xDCDE3DFF,
    0x10C9C5FF, 0x70524DFF, 0x0BE472FF, 0x8A2CD7FF, 0x6152C2FF, 0xCF72A9FF, 0xE59338FF, 0xEEDC2DFF, 0xD8C762FF,
    0x3FE65CFF,
];

/// The per-player colour table.
#[derive(Clone, Debug)]
pub struct PlayerColours {
    rgba: Vec<u32>,
}

impl Default for PlayerColours {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerColours {
    pub fn new() -> Self {
        let mut rgba = vec![0u32; TABLE_LEN];
        for (i, c) in rgba.iter_mut().take(INITIALISED).enumerate() {
            *c = DEFAULT_COLOURS[i % DEFAULT_COLOURS.len()];
        }
        Self { rgba }
    }

    /// Setter A 0x100a7250: ignored for `id >= 4000`.
    pub fn set(&mut self, id: u16, rgba: u32) {
        if let Some(c) = self.rgba.get_mut(usize::from(id)) {
            *c = rgba;
        }
    }

    /// RGBA colour of `id` (0 outside the table).
    pub fn rgba(&self, id: u16) -> u32 {
        self.rgba.get(usize::from(id)).copied().unwrap_or(0)
    }

    /// Chat name colour: `(c >> 8) | 0xFF000000` (UCA §101).
    pub fn chat_argb(&self, id: u16) -> u32 {
        samp_ui::colour::rgba_to_argb(self.rgba(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_repeat_every_100_and_stop_at_1000() {
        let c = PlayerColours::new();
        assert_eq!(c.rgba(0), 0xFF8C13FF);
        assert_eq!(c.rgba(100), 0xFF8C13FF);
        assert_eq!(c.rgba(999), DEFAULT_COLOURS[99]);
        assert_eq!(c.rgba(1000), 0);
        assert_eq!(c.chat_argb(0), 0xFFFF8C13);
    }

    #[test]
    fn setter_bounds() {
        let mut c = PlayerColours::new();
        c.set(3999, 0x11223344);
        assert_eq!(c.rgba(3999), 0x11223344);
        c.set(4000, 1);
        assert_eq!(c.rgba(4000), 0);
    }
}
