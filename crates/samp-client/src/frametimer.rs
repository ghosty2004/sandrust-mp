//! samp's step before every `CGame::Process` (A 0x100A1C50, called from its
//! `CGame::Process` wrapper A 0x100A26A0, CSV row 10): a time-step estimate
//! written every 250 ms, a 30 ms counter written into the pad, and the frame
//! limiter. Static analysis only (docs/phase3/parity-notes.md "Speed
//! magnitude"); the effect it was looked at for is the frame rate: the
//! original runs at ~52 fps in the trace lane, ours ran at ~59 fps.
//!
//! Per call, with `now` in milliseconds from the performance counter:
//! * first call: time step and non-clipped time step := 1.0, timers start,
//!   nothing else;
//! * counter: every > 30 ms a counter 0…4 advances (wrapping at 5); it is
//!   written to 0xB7356E on every call once the timer has started;
//! * every > 250 ms: `n = 4 × frames − 4` (frames counted since the last
//!   estimate); time step := 3.0 if n < 17, else 50 / n;
//! * limiter, `elapsed` = now − end of the previous call:
//!   * the local ped's primary event task (intelligence +0xC) has type 0x10C
//!     and it is on foot: with the menu frame limiter on and a limit < 46,
//!     sleep to the limit; otherwise sleep `21 − elapsed` when 0 < elapsed < 22;
//!   * otherwise: menu frame limiter off → sleep `10 − elapsed` when
//!     0 < elapsed < 11; on → sleep to the `fpslimit` (sleep `1000/limit −
//!     elapsed − 1` when 0 < elapsed < 1000/limit);
//!   * the end time is taken after the sleep.
//!
//! The sleep is kernel32 `Sleep` as in samp (NET-6 §4: with std's sleep our frames were ~19.2 ms
//! instead of the original's 19.0, which changed GTA's physics around vehicles and samp's send races).

/// `CTimer::ms_fTimeStep` and `ms_fTimeStepNonClipped`.
pub const TIME_STEP: u32 = 0xB7CB5C;
pub const TIME_STEP_NON_CLIPPED: u32 = 0xB7CB58;
/// Byte samp's 30 ms counter is written to (inside `CPad::Pads[0]`, meaning not identified).
pub const PAD_COUNTER: u32 = 0xB7356E;
/// Menu frame-limiter flag samp tests (`FrontEndMenuManager` + 0x4C).
pub const MENU_FRAME_LIMITER: u32 = 0xBA6794;
/// Task type samp tests in the local ped's intelligence +0xC slot.
pub const LIMITER_TASK_TYPE: u32 = 0x10C;

/// What one call asks the game side to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub time_step: Option<f32>,
    pub pad_counter: Option<u8>,
    pub sleep_ms: u32,
}

#[derive(Debug, Default, Clone)]
pub struct FrameTimer {
    rate_start: Option<u64>,
    frame_end: u64,
    frames: u32,
    counter_tick: Option<u64>,
    counter: u8,
}

/// Inputs of the limiter decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limiter {
    pub menu_limiter_on: bool,
    /// `fpslimit` (0 = none).
    pub fps_limit: u32,
    /// Local ped on foot with the 0x10C task in its intelligence +0xC slot.
    pub task_gate: bool,
}

impl FrameTimer {
    pub fn step(&mut self, now: u64, l: Limiter) -> Step {
        let Some(rate_start) = self.rate_start else {
            self.rate_start = Some(now);
            self.frame_end = now;
            return Step { time_step: Some(1.0), pad_counter: None, sleep_ms: 0 };
        };
        let pad_counter = match self.counter_tick {
            None => {
                self.counter_tick = Some(now);
                None
            }
            Some(t) => {
                if now.saturating_sub(t) > 30 {
                    self.counter = (self.counter + 1) % 5;
                    self.counter_tick = Some(now);
                }
                Some(self.counter)
            }
        };
        self.frames += 1;
        let mut time_step = None;
        if now.saturating_sub(rate_start) > 250 {
            let n = self.frames as i64 * 4 - 4;
            time_step = Some(if n < 17 { 3.0 } else { 50.0 / n as f32 });
            self.rate_start = Some(now);
            self.frames = 0;
        }
        let elapsed = now.saturating_sub(self.frame_end) as u32;
        let to_limit = |fps: u32| {
            if elapsed != 0 && fps != 0 && elapsed < 1000 / fps { 1000 / fps - elapsed - 1 } else { 0 }
        };
        let sleep_ms = if l.task_gate {
            if l.menu_limiter_on && l.fps_limit < 46 {
                to_limit(l.fps_limit)
            } else if elapsed != 0 && elapsed < 22 {
                21 - elapsed
            } else {
                0
            }
        } else if !l.menu_limiter_on {
            if elapsed != 0 && elapsed < 11 { 10 - elapsed } else { 0 }
        } else {
            to_limit(l.fps_limit)
        };
        Step { time_step, pad_counter, sleep_ms }
    }

    /// End of the call (after the sleep).
    pub fn finish(&mut self, now: u64) {
        self.frame_end = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON48: Limiter = Limiter { menu_limiter_on: true, fps_limit: 48, task_gate: false };

    #[test]
    fn first_call_resets_the_time_step() {
        let mut t = FrameTimer::default();
        assert_eq!(t.step(1000, ON48), Step { time_step: Some(1.0), pad_counter: None, sleep_ms: 0 });
    }

    #[test]
    fn limiter_sleeps_to_the_fps_limit() {
        let mut t = FrameTimer::default();
        t.step(1000, ON48);
        t.finish(1000);
        // 12 ms of work: 1000/48 = 20 → sleep 20 − 12 − 1.
        assert_eq!(t.step(1012, ON48).sleep_ms, 7);
        t.finish(1020);
        assert_eq!(t.step(1045, ON48).sleep_ms, 0);
        let off = Limiter { menu_limiter_on: false, ..ON48 };
        t.finish(1050);
        assert_eq!(t.step(1054, off).sleep_ms, 6);
        let gate = Limiter { task_gate: true, ..ON48 };
        t.finish(1060);
        assert_eq!(t.step(1065, gate).sleep_ms, 16);
    }

    #[test]
    fn time_step_estimate_every_250_ms() {
        let mut t = FrameTimer::default();
        t.step(0, ON48);
        let mut now = 0;
        let mut last = None;
        for _ in 0..13 {
            now += 20;
            let s = t.step(now, ON48);
            t.finish(now);
            if s.time_step.is_some() {
                last = s.time_step;
            }
        }
        // 13 frames in 260 ms → n = 48 → 50/48.
        assert_eq!(last, Some(50.0 / 48.0));
        // Slow: 4 frames → n = 12 < 17 → 3.0.
        for _ in 0..4 {
            now += 70;
            if let Some(ts) = t.step(now, ON48).time_step {
                assert_eq!(ts, 3.0);
            }
        }
    }

    #[test]
    fn pad_counter_wraps_at_five() {
        let mut t = FrameTimer::default();
        t.step(0, ON48);
        assert_eq!(t.step(10, ON48).pad_counter, None);
        let seen: Vec<_> = (1..=6).map(|i| t.step(10 + 31 * i, ON48).pad_counter.unwrap()).collect();
        assert_eq!(seen, [1, 2, 3, 4, 0, 1]);
    }
}
