//! Server-driven world state: the game clock and the weather, kept the way samp's Game object and its
//! netgame settings keep them (docs/re/subsystems/player-state.md, "30 TogglePlayerClock",
//! "152 SetPlayerWeather"; samp.dll addresses in docs/re/addresses/world.md).
//!
//! On its first frame samp stops GTA's clock and blanks the HUD clock; while the server holds the time the
//! hour and minute are re-applied every frame. [`World`] holds that state and returns the memory writes
//! ([`Write`]) each change needs; `apply` performs them on the game.

/// First byte of `CClock::Update` (patch table row 354, i.e. line 354 of
/// docs/re/data/gta-patches-r4-annotated.csv): `push esi` = running, `ret` = stopped.
pub const CLOCK_UPDATE: u32 = 0x52CF10;
pub const CLOCK_UPDATE_ON: u8 = 0x56;
pub const CLOCK_UPDATE_OFF: u8 = 0xC3;
/// HUD clock format string (patch table row 355); samp writes all 12 bytes (three dwords).
pub const HUD_CLOCK_FORMAT: u32 = 0x859A6C;
pub const HUD_CLOCK_FORMAT_ON: [u8; 12] = *b"%02d:%02d\0\0\0";
/// `CTimer::m_snTimeInMilliseconds`; RPC 60 (SendGameTimeUpdate) writes it while the HUD clock is off.
pub const TIMER_MS: u32 = 0xB7CB84;
/// `CClock::ms_nGameClockMinutes` / `ms_nGameClockHours` (u8).
pub const CLOCK_MINUTES: u32 = 0xB70152;
pub const CLOCK_HOURS: u32 = 0xB70153;
/// `CWeather::ForcedWeatherType`, `NewWeatherType`, `OldWeatherType` (i16; samp writes a dword
/// to the first, which also covers the 2-byte padding; we write the same 4 bytes).
pub const WEATHER_FORCED: u32 = 0xC81318;
pub const WEATHER_NEW: u32 = 0xC8131C;
pub const WEATHER_OLD: u32 = 0xC81320;

/// Weather samp forces every frame while connecting; also the netgame constructor's default.
pub const CONNECTING_WEATHER: u8 = 1;

/// The client-side copy of samp's clock/weather state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct World {
    /// samp Game +0x69: the HUD clock (and GTA's running clock) is on.
    pub clock_enabled: bool,
    /// samp settings +0x1f: the server holds the time (re-applied every frame with SET_TIME_OF_DAY).
    pub server_time: bool,
    pub hour: u8,
    pub minute: u8,
    pub weather: u8,
}

impl Default for World {
    /// State after samp's first frame and netgame constructor.
    fn default() -> Self {
        Self { clock_enabled: false, server_time: true, hour: 12, minute: 0, weather: CONNECTING_WEATHER }
    }
}

/// One memory write the world state asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    /// Code/rdata bytes (needs the protection change).
    Code(u32, Vec<u8>),
    /// Plain data bytes.
    Data(u32, Vec<u8>),
}

impl World {
    /// Clock on/off: the `CClock::Update` byte, the HUD string and the flag. The first frame calls it
    /// with `false`. Mirrors samp.dll 0x100A14A0.
    pub fn toggle_clock(&mut self, enable: bool) -> Vec<Write> {
        self.clock_enabled = enable;
        if enable {
            vec![
                Write::Code(CLOCK_UPDATE, vec![CLOCK_UPDATE_ON]),
                Write::Code(HUD_CLOCK_FORMAT, HUD_CLOCK_FORMAT_ON.to_vec()),
            ]
        } else {
            vec![Write::Code(CLOCK_UPDATE, vec![CLOCK_UPDATE_OFF]), Write::Code(HUD_CLOCK_FORMAT, vec![0; 12])]
        }
    }

    /// RPC 30 (TogglePlayerClock): the toggle, then "server holds the time" := !enable; switching off
    /// copies GTA's current (hour, minute) into the settings. Mirrors samp.dll 0x1000F500.
    pub fn on_toggle_clock_rpc(&mut self, enable: bool, current: (u8, u8)) -> Vec<Write> {
        let w = self.toggle_clock(enable);
        self.server_time = !enable;
        if !enable {
            self.hour = current.0;
            self.minute = current.1;
        }
        w
    }

    /// Weather writes: the forced weather always, the old and new weather too while the clock is off (so
    /// the change is immediate). Mirrors samp.dll 0x100A0BA0.
    pub fn weather_writes(&self, weather: u8) -> Vec<Write> {
        let v = u32::from(weather).to_le_bytes().to_vec();
        let mut w = vec![Write::Data(WEATHER_FORCED, v.clone())];
        if !self.clock_enabled {
            w.push(Write::Data(WEATHER_NEW, v.clone()));
            w.push(Write::Data(WEATHER_OLD, v));
        }
        w
    }

    /// Per frame: (hour, minute) for SET_TIME_OF_DAY, if the server holds the time.
    pub fn time_of_day(&self) -> Option<(u8, u8)> {
        self.server_time.then_some((self.hour, self.minute))
    }
}

#[cfg(windows)]
pub use apply::apply;

#[cfg(windows)]
mod apply {
    use super::Write;
    use crate::patch::{Memory, ProcessMemory};

    /// Performs the writes (main thread).
    pub fn apply(writes: &[Write]) {
        let mut mem = ProcessMemory;
        for w in writes {
            match w {
                Write::Code(a, b) => {
                    if let Err(e) = mem.write(*a, b) {
                        crate::log!("world: write {a:#x} failed: {e}");
                    }
                }
                Write::Data(a, b) => {
                    // SAFETY: GTA .data/.bss globals of at least b.len() bytes, main thread.
                    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), *a as usize as *mut u8, b.len()) };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patches::testutil;

    #[test]
    fn defaults_match_the_netgame_constructor() {
        let w = World::default();
        assert!(!w.clock_enabled && w.server_time);
        assert_eq!((w.hour, w.minute, w.weather), (12, 0, 1));
        assert_eq!(w.time_of_day(), Some((12, 0)));
    }

    #[test]
    fn weather_is_immediate_only_with_the_clock_off() {
        let mut w = World::default();
        assert_eq!(w.weather_writes(10).len(), 3);
        w.toggle_clock(true);
        assert_eq!(w.weather_writes(10), vec![Write::Data(WEATHER_FORCED, vec![10, 0, 0, 0])]);
    }

    #[test]
    fn clock_toggle() {
        let mut w = World::default();
        assert_eq!(w.toggle_clock(false)[0], Write::Code(CLOCK_UPDATE, vec![0xC3]));
        assert!(w.server_time, "the first-frame toggle keeps the settings");
        let on = w.on_toggle_clock_rpc(true, (3, 4));
        assert!(w.clock_enabled && !w.server_time);
        assert_eq!(w.time_of_day(), None);
        assert_eq!(on[0], Write::Code(CLOCK_UPDATE, vec![0x56]));
        let off = w.on_toggle_clock_rpc(false, (7, 30));
        assert_eq!(w.time_of_day(), Some((7, 30)));
        assert_eq!(off, vec![Write::Code(CLOCK_UPDATE, vec![0xC3]), Write::Code(HUD_CLOCK_FORMAT, vec![0; 12])]);
    }

    #[test]
    fn csv_rows_354_355() {
        let csv = testutil::csv_rows();
        assert_eq!(testutil::field_u32(&csv[&354], "gta_addr"), CLOCK_UPDATE);
        assert_eq!(testutil::field_u32(&csv[&355], "gta_addr"), HUD_CLOCK_FORMAT);
        assert_eq!(testutil::unhex(&csv[&354]["original_bytes_hex"]), [CLOCK_UPDATE_ON]);
        if let Some(exe) = testutil::Exe::load() {
            assert_eq!(exe.read_va(CLOCK_UPDATE, 1), [CLOCK_UPDATE_ON]);
            assert_eq!(exe.read_va(HUD_CLOCK_FORMAT, 12), HUD_CLOCK_FORMAT_ON);
        }
    }
}
