//! Chat bubbles above players: one slot per player id, set by RPC 59, expired per frame.
//! Specs: docs/re/subsystems/ui-rendering.md ("Chat bubbles") and ui-chat-audio.md
//! ("59 SetPlayerChatBubble").

use crate::colour::rgba_to_argb;
use crate::text::{c_str, wrap_lines};

/// Bubble slots (1004 slots of 0x118 bytes in samp); RPC 59 accepts ids below this.
pub const SLOTS: usize = 1004;
/// RPC 59 texts longer than this are ignored.
pub const MAX_TEXT_LEN: usize = 144;
/// First argument of samp's wrap helper: the line width. Mirrors samp.dll 0x10017690.
pub const WRAP_WIDTH: usize = 36;
/// Second argument of the wrap helper, used as the look-back distance for a space (the same
/// rule as the 72/12 player-chat wrap). ui-chat-audio.md reads it as a maximum line count, but
/// with ≤ 144 chars in 36-char lines such a limit could never apply.
pub const WRAP_BACKTRACK: usize = 12;
/// Draw flags `DT_CENTER | DT_NOCLIP`.
pub const DRAW_FLAGS: u32 = 0x101;
/// Head bone used as the anchor.
pub const ANCHOR_BONE: u32 = 8;

/// One active bubble.
#[derive(Clone, Debug, PartialEq)]
pub struct Bubble {
    /// Text wrapped with `\n`.
    pub text: Vec<u8>,
    pub lines: u32,
    pub start_ms: u32,
    pub duration_ms: u32,
    /// ARGB with alpha FF.
    pub colour: u32,
    pub distance: f32,
}

impl Bubble {
    /// `now > start + duration` (tick arithmetic wraps like `GetTickCount`).
    pub fn expired(&self, now_ms: u32) -> bool {
        now_ms > self.start_ms.wrapping_add(self.duration_ms)
    }

    /// Height above the head bone: `0.2 + 0.02·(lines − 1) + (0.065 + 0.0125·(lines − 1))·d`
    /// with `d` the camera distance.
    pub fn anchor_raise(&self, camera_distance: f32) -> f32 {
        anchor_raise(self.lines, camera_distance)
    }
}

/// See [`Bubble::anchor_raise`].
pub fn anchor_raise(lines: u32, d: f32) -> f32 {
    let extra = lines.saturating_sub(1) as f32;
    0.2 + 0.02 * extra + (0.065 + 0.0125 * extra) * d
}

/// Wraps bubble text with the chat wrap rule at [`WRAP_WIDTH`].
pub fn wrap(text: &[u8]) -> (Vec<u8>, u32) {
    let lines = wrap_lines(text, WRAP_WIDTH, WRAP_BACKTRACK);
    (lines.join(&b'\n'), lines.len() as u32)
}

/// Remote player facts the visibility test needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleTarget {
    pub streamed_in_with_ped: bool,
    /// Player state ≠ 0.
    pub active_state: bool,
    /// Distance from the local player. Mirrors samp.dll 0x1009f120.
    pub distance: f32,
}

/// The bubble pool. Mirrors the samp.dll global at 0x1026e9f0.
#[derive(Clone, Debug)]
pub struct ChatBubbles {
    slots: Vec<Option<Bubble>>,
}

impl Default for ChatBubbles {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatBubbles {
    pub fn new() -> Self {
        Self { slots: vec![None; SLOTS] }
    }

    pub fn get(&self, player: u16) -> Option<&Bubble> {
        self.slots.get(usize::from(player)).and_then(Option::as_ref)
    }

    /// RPC 59: accepted if `player < 1004` (the caller checks that the
    /// player exists) and the text is ≤ 144 chars. Replaces any previous bubble.
    pub fn set(
        &mut self,
        player: u16,
        colour_rgba: u32,
        distance: f32,
        expire_ms: u32,
        text: &[u8],
        now_ms: u32,
    ) -> bool {
        let text = c_str(text);
        if usize::from(player) >= SLOTS || text.len() > MAX_TEXT_LEN {
            return false;
        }
        let (text, lines) = wrap(text);
        self.slots[usize::from(player)] = Some(Bubble {
            text,
            lines,
            start_ms: now_ms,
            duration_ms: expire_ms,
            colour: rgba_to_argb(colour_rgba),
            distance,
        });
        true
    }

    pub fn clear(&mut self, player: u16) {
        if let Some(s) = self.slots.get_mut(usize::from(player)) {
            *s = None;
        }
    }

    /// Per-frame expiry: deactivates bubbles with `now > start + duration`.
    pub fn expire(&mut self, now_ms: u32) {
        for s in &mut self.slots {
            if s.as_ref().is_some_and(|b| b.expired(now_ms)) {
                *s = None;
            }
        }
    }

    /// The bubble to draw for `player`, if any: streamed in with a ped, state ≠ 0, and
    /// distance strictly below the bubble's draw distance. The renderer then projects
    /// the anchor and draws only if projected z ≤ 1.
    pub fn drawable(&self, player: u16, target: &BubbleTarget) -> Option<&Bubble> {
        let b = self.get(player)?;
        (target.streamed_in_with_ped && target.active_state && target.distance < b.distance).then_some(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEAR: BubbleTarget = BubbleTarget { streamed_in_with_ped: true, active_state: true, distance: 5.0 };

    #[test]
    fn uca_59_accept_limits_and_colour() {
        let mut b = ChatBubbles::new();
        assert!(!b.set(1004, 0xFF0000FF, 10.0, 1000, b"x", 0));
        assert!(!b.set(1, 0xFF0000FF, 10.0, 1000, &[b'a'; 145], 0));
        assert!(b.set(1003, 0xFF0000FF, 10.0, 1000, &[b'a'; 144], 0));
        assert_eq!(b.get(1003).unwrap().colour, 0xFFFF0000);
    }

    #[test]
    fn ur_11_expiry_now_greater_than_start_plus_duration() {
        let mut b = ChatBubbles::new();
        b.set(7, 0xFFFFFFFF, 10.0, 1000, b"hi", 5000);
        b.expire(6000);
        assert!(b.get(7).is_some());
        b.expire(6001);
        assert!(b.get(7).is_none());
    }

    #[test]
    fn ur_11_visibility_distance_strict() {
        let mut b = ChatBubbles::new();
        b.set(7, 0xFFFFFFFF, 5.0, 1000, b"hi", 0);
        assert!(b.drawable(7, &NEAR).is_none());
        assert!(b.drawable(7, &BubbleTarget { distance: 4.9, ..NEAR }).is_some());
        assert!(b.drawable(7, &BubbleTarget { distance: 1.0, active_state: false, ..NEAR }).is_none());
        assert!(b.drawable(7, &BubbleTarget { distance: 1.0, streamed_in_with_ped: false, ..NEAR }).is_none());
    }

    #[test]
    fn ur_11_anchor_formula() {
        assert!((anchor_raise(1, 10.0) - (0.2 + 0.65)).abs() < 1e-6);
        assert!((anchor_raise(3, 10.0) - (0.2 + 0.04 + (0.065 + 0.025) * 10.0)).abs() < 1e-6);
    }

    #[test]
    fn uca_59_wrap_lines() {
        let (t, n) = wrap(b"the quick brown fox jumps over the lazy dog and keeps running");
        assert_eq!(n, 2);
        assert_eq!(t, b"the quick brown fox jumps over the\nlazy dog and keeps running");
        let (_, n) = wrap(&[b'x'; 144]);
        assert_eq!(n, 4);
    }
}
