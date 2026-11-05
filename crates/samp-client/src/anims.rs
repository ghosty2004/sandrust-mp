//! The 207 "animation" field and samp's animation-name table (platform independent).
//!
//! Spec: remote-control-onfoot.md §4.1 (remote apply), docs/phase3/trace-notes.md "Animations (86/87) and the 207
//! animation table" (TRACE-8, measured values) and the static reading of samp.dll build B noted per function:
//!
//! * the table ([`crate::animtable`]) holds 1812 upper-case `"LIB:NAME"` records, 1-based (accessor B 0x1009CF00
//!   accepts 1..=0x714);
//! * a script animation (`CTaskSimpleRunNamedAnim`) is encoded from the task's own fields (B 0x100AE600): index of
//!   `"LIB:NAME"` (string search B 0x1009CE20), delta, the task's flag word and its time;
//! * any other animation is encoded from the last `AddAnimation` / `BlendAnimation` call on the local clump
//!   (B 0x100AE6B0): index of the first record whose NAME hash equals the hierarchy's hash, the blend delta, bit 31;
//! * the local tick keeps the value until a 207 carried it (B 0x100077EF, read by the 207 builder B 0x10005131);
//! * remotes apply a changed value once per stored 207 (B 0x100167E4 → B 0x100AE7E0).

pub use crate::animtable::{COUNT, NAMES};

/// Index value samp uses for "not found" (`or ax, 0xFFFF`), sign-extended into the 32-bit field.
pub const NOT_FOUND: i16 = -1;
/// `VENDING:VEND_USE` (1660), delta 4, freeze, time code 1: after it, a 0 plays `VENDING:VEND_EAT_P` (B 0x1001680A).
pub const VEND_USE_VALUE: u32 = 0x1804_067C;

/// Record `index` (1-based) as `"LIB:NAME"` (B 0x1009CF00: only 1..=0x714).
pub fn record(index: i32) -> Option<&'static str> {
    if (1..=COUNT as i32).contains(&index) { Some(NAMES[index as usize - 1]) } else { None }
}

/// Splits a record at its first ':' into (library, name); `None` without a ':' (samp then plays nothing).
pub fn split(rec: &str) -> Option<(&str, &str)> {
    rec.split_once(':')
}

/// The NAME part of record `index` (what samp hashes at start-up, B 0x1009CF30).
pub fn name_part(index: usize) -> Option<&'static str> {
    NAMES.get(index).and_then(|r| split(r)).map(|(_, n)| n)
}

/// samp's `"%s:%s"` search (B 0x1009CE20): 1-based index of the upper-cased `lib:name`, or [`NOT_FOUND`] when
/// `len(lib) + len(name) > 36` or no record matches.
pub fn index_of(lib: &[u8], name: &[u8]) -> i16 {
    if lib.len() + name.len() > 0x24 {
        return NOT_FOUND;
    }
    let mut key = Vec::with_capacity(lib.len() + name.len() + 1);
    key.extend(lib.iter().map(u8::to_ascii_uppercase));
    key.push(b':');
    key.extend(name.iter().map(u8::to_ascii_uppercase));
    NAMES.iter().position(|r| r.as_bytes() == key.as_slice()).map(|i| i as i16 + 1).unwrap_or(NOT_FOUND)
}

/// First 1-based index whose NAME hash equals `hash` (B 0x1009D000), or [`NOT_FOUND`].
pub fn index_of_hash(hashes: &[u32], hash: u32) -> i16 {
    hashes.iter().take(COUNT).position(|&h| h == hash).map(|i| i as i16 + 1).unwrap_or(NOT_FOUND)
}

/// MSVC `_ftol2` (B 0x100C7464): truncation toward zero; out-of-range and NaN give 0x80000000.
pub fn ftol(v: f32) -> i32 {
    if v.is_finite() && v > -2_147_483_648.0 && v < 2_147_483_648.0 { v as i32 } else { i32::MIN }
}

/// Encoding of a running script animation (B 0x100AE600). `flags` is the task's flag word (+0x5C), `time` its
/// time (+0x40): bit 24 = flag 0x2 (loop), 25 = 0x40 (lock x), 26 = 0x80 (lock y), 27 = flag 0x8 clear (freeze),
/// bits 28–29 = time > 0 → 2, time < 0 → 1, 0 → 0; bits 16–23 = delta; low 16 bits = index (sign-extended).
pub fn encode_task(index: i16, delta: f32, flags: u32, time: i32) -> u32 {
    let time_code: u32 = match time {
        t if t > 0 => 2,
        0 => 0,
        _ => 1,
    };
    let mut v = time_code;
    v = (v << 1) | u32::from(flags & 0x8 == 0);
    v = (v << 1) | u32::from(flags & 0x80 != 0);
    v = (v << 1) | u32::from(flags & 0x40 != 0);
    v = (v << 1) | u32::from(flags & 0x2 != 0);
    v = (v << 8) | (ftol(delta) as u32 & 0xFF);
    (v << 16) | (i32::from(index) as u32)
}

/// Encoding of the last plain (blend) animation (B 0x100AE6B0): bit 31 set, bits 16–23 = blend delta, low 16 bits =
/// index (sign-extended, so "not found" makes the whole value 0xFFFFFFFF).
pub fn encode_plain(index: i16, delta: f32) -> u32 {
    let hi = ((ftol(delta) as u32 & 0xFF) | 0xFFFF_8000) << 16;
    hi | (i32::from(index) as u32)
}

/// A decoded animation field (remote side, B 0x100AE7E0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoded {
    pub index: u16,
    pub delta: f32,
    pub looping: bool,
    pub lock_x: bool,
    pub lock_y: bool,
    pub freeze: bool,
    /// 2 → 1, 1 → −1, otherwise 0.
    pub time: i32,
}

pub fn decode(v: u32) -> Decoded {
    let b = |n: u32| (v >> n) & 1 != 0;
    Decoded {
        index: (v & 0xFFFF) as u16,
        delta: ((v >> 16) & 0xFF) as f32,
        looping: b(24),
        lock_x: b(25),
        lock_y: b(26),
        freeze: b(27),
        time: match (v >> 28) & 3 {
            2 => 1,
            1 => -1,
            _ => 0,
        },
    }
}

/// The local player's animation value between frames and 207 sends (LocalPlayer +0xFC / +0x100).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latch {
    pub value: u32,
    pub sent: bool,
}

impl Latch {
    /// Every alive frame (B 0x100077F5): a non-zero value replaces the stored one (not yet sent); a zero clears
    /// the stored value only after a 207 carried it.
    pub fn update(&mut self, anim: u32) {
        if anim != 0 {
            self.value = anim;
            self.sent = false;
        } else if self.value != 0 && self.sent {
            self.value = 0;
        }
    }

    /// The 207 builder reads the value and marks it sent (B 0x10005131).
    pub fn take(&mut self) -> u32 {
        self.sent = true;
        self.value
    }
}

/// Per remote player (remote +0x105 / +0x1B4 / +0x1C1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteState {
    /// Last applied value (+0x1B4).
    pub last: u32,
    /// An RPC 86 played an animation on this ped (+0x1C1); cleared by a changed value, RPC 87, stream-in.
    pub by_rpc: bool,
}

/// What the one-shot apply does with one stored animation value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RemoteAction {
    /// Play the decoded record.
    Play(Decoded),
    /// `VENDING:VEND_EAT_P`, delta 4, flags 0, time −1.
    VendEat,
    /// `CARRY:crry_prtial`, delta 4, flags 0, time 0 (ends a running script animation).
    Stop,
}

impl RemoteState {
    /// B 0x100167E4. `named_anim_running` is asked only when needed (task type 401 on the ped).
    pub fn step(&mut self, anim: u32, named_anim_running: impl FnOnce() -> bool) -> Vec<RemoteAction> {
        let mut out = Vec::new();
        let mut cur = anim;
        if self.last != anim {
            if anim != 0 {
                out.push(RemoteAction::Play(decode(anim)));
            }
            if anim == 0 && self.last == VEND_USE_VALUE {
                out.push(RemoteAction::VendEat);
                cur = u32::MAX;
            }
            self.last = cur;
            self.by_rpc = false;
        }
        if !self.by_rpc && cur == 0 && named_anim_running() {
            out.push(RemoteAction::Stop);
        }
        out
    }
}

/// samp's "SEX" refusal (B 0x100AD270: case-insensitive compare of the library).
pub fn refused_library(lib: &[u8]) -> bool {
    lib.eq_ignore_ascii_case(b"SEX")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_the_measured_records() {
        assert_eq!(record(1), Some("AIRPORT:THRW_BARL_THRW"));
        assert_eq!(record(1660), Some("VENDING:VEND_USE"));
        assert_eq!(record(1812), Some("SAMP:FISHINGIDLE"));
        assert_eq!(record(0), None);
        assert_eq!(record(1813), None);
        assert_eq!(name_part(1188), Some("IDLE_STANCE"));
    }

    #[test]
    fn string_search_is_case_insensitive_and_bounded() {
        // TRACE-8 measured indexes.
        assert_eq!(index_of(b"PED", b"WALK_DRUNK"), 1260);
        assert_eq!(index_of(b"ped", b"KO_skid_front"), 1209);
        assert_eq!(index_of(b"PED", b"IDLE_CHAT"), 1184);
        assert_eq!(index_of(b"DANCING", b"DNCE_M_A"), 411);
        assert_eq!(index_of(b"NOSUCHLIB", b"X"), NOT_FOUND);
        assert_eq!(index_of(&[b'A'; 30], &[b'B'; 7]), NOT_FOUND);
    }

    #[test]
    fn task_encoding_matches_the_measured_values() {
        // WALK_DRUNK 4.1 loop lockX lockY, not frozen, time 0 (task time −1) → 0x1704.
        assert_eq!(encode_task(1260, 4.1, 0x2 | 0x40 | 0x80 | 0x8, -1) >> 16, 0x1704);
        assert_eq!(encode_task(1260, 4.1, 0x2 | 0x40 | 0x80 | 0x8, -1) & 0xFFFF, 1260);
        // KO_skid_front lockX lockY freeze → 0x1E04; IDLE_CHAT freeze, time > 0 → 0x2804; DNCE_M_A loop → 0x1104.
        assert_eq!(encode_task(1209, 4.1, 0x40 | 0x80, -1) >> 16, 0x1E04);
        assert_eq!(encode_task(1184, 4.1, 0, 1500) >> 16, 0x2804);
        assert_eq!(encode_task(411, 4.1, 0x2 | 0x8, -1) >> 16, 0x1104);
        assert_eq!(encode_task(NOT_FOUND, 4.0, 0, 0), u32::MAX);
    }

    #[test]
    fn plain_encoding_matches_the_measured_values() {
        assert_eq!(encode_plain(1189, 1000.0), 0x80E8_0000 | 1189);
        assert_eq!(encode_plain(1133, 100.0) >> 16, 0x8064);
        assert_eq!(encode_plain(NOT_FOUND, 4.0), u32::MAX);
    }

    #[test]
    fn decode_round_trips_the_flags() {
        let d = decode(encode_task(1260, 4.0, 0x2 | 0x40 | 0x80 | 0x8, -1));
        assert_eq!(
            (d.index, d.delta, d.looping, d.lock_x, d.lock_y, d.freeze, d.time),
            (1260, 4.0, true, true, true, false, -1)
        );
        let d = decode(encode_task(1184, 4.0, 0, 1500));
        assert_eq!((d.freeze, d.time), (true, 1));
        let d = decode(VEND_USE_VALUE);
        assert_eq!((d.index, d.delta, d.freeze, d.time), (1660, 4.0, true, -1));
    }

    #[test]
    fn latch_keeps_a_value_until_sent() {
        let mut l = Latch::default();
        l.update(5);
        l.update(0);
        assert_eq!(l.value, 5, "zero does not clear an unsent value");
        assert_eq!(l.take(), 5);
        l.update(0);
        assert_eq!(l.value, 0);
        l.update(7);
        assert!(!l.sent);
    }

    #[test]
    fn remote_steps() {
        let mut s = RemoteState::default();
        assert!(s.step(0, || false).is_empty());
        assert!(matches!(s.step(0x1104_019B, || false)[..], [RemoteAction::Play(_)]));
        assert!(s.step(0x1104_019B, || true).is_empty(), "unchanged value: nothing");
        assert_eq!(s.step(0, || true), vec![RemoteAction::Stop]);
        s.by_rpc = true;
        assert!(s.step(0, || true).is_empty(), "an RPC 86 animation is not stopped");
        let mut s = RemoteState { last: VEND_USE_VALUE, by_rpc: false };
        assert_eq!(s.step(0, || true), vec![RemoteAction::VendEat]);
        assert_eq!(s.last, u32::MAX);
    }

    #[test]
    fn sex_is_refused() {
        assert!(refused_library(b"sex"));
        assert!(!refused_library(b"SEXY"));
    }
}
