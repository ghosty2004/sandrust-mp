//! Game text (RPC 73): acceptance rules and a model of GTA's big-message styles
//! (docs/re/subsystems/ui-chat-audio.md, "73 SendGameText").
//!
//! samp only clears prints (opcode 0x00BE) and hands the text to GTA's
//! `CMessages::AddBigMessage`; position, font, colour and animation are GTA's (`CHud`). The
//! style table lets a headless or non-GTA renderer approximate them.

use samp_proto::rpcs::ui::SendGameText;

use crate::text::c_str;

/// Accepted text length: 1..=200 (else the whole RPC is ignored).
pub const MAX_TEXT_LEN: usize = 200;
/// Styles (as signed int) at or above this are ignored by the client.
pub const STYLE_LIMIT: i32 = 201;
/// Size of the static text buffer the text is copied into.
pub const BUFFER_LEN: usize = 512;
/// The client passes any style below 201 to GTA, which only has 7 slots. A hardened
/// client ignores styles outside 0..=6.
pub const HARDEN_STYLE_RANGE: bool = true;

/// GTA big-message styles (`eMessageStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Style {
    /// "In the middle" (mission passed style).
    Middle = 0,
    /// Bottom right (mission title): slides in, then fades out by itself.
    BottomRight = 1,
    /// White text in the middle (wasted/busted style), fades in.
    WhiteMiddle = 2,
    /// Smaller text in the middle.
    MiddleSmaller = 3,
    /// Smaller, a bit higher on the screen.
    MiddleSmallerHigher = 4,
    /// Small white text in the middle: slides in, holds, slides out.
    WhiteMiddleSmaller = 5,
    /// Light blue text at the top.
    LightBlueTop = 6,
}

/// Horizontal anchor of a style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Centre,
    Right,
}

/// How long a style stays (GTA behaviour; not confirmed visually).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duration {
    /// Shown until the message time runs out.
    Time,
    /// HUD animation (slide in, then fade) that can end before the time runs out.
    AnimatedFade,
    /// HUD slide-in / hold (~1500 ms) / slide-out cycle while the message lasts.
    AnimatedSlide,
}

/// Style description (GTA `CHud` draw functions; y values are on GTA's 448-high canvas).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleInfo {
    pub align: Align,
    pub duration: Duration,
}

impl Style {
    pub fn from_index(i: i32) -> Option<Self> {
        Some(match i {
            0 => Self::Middle,
            1 => Self::BottomRight,
            2 => Self::WhiteMiddle,
            3 => Self::MiddleSmaller,
            4 => Self::MiddleSmallerHigher,
            5 => Self::WhiteMiddleSmaller,
            6 => Self::LightBlueTop,
            _ => return None,
        })
    }

    pub fn info(self) -> StyleInfo {
        let (align, duration) = match self {
            Self::BottomRight => (Align::Right, Duration::AnimatedFade),
            Self::WhiteMiddleSmaller => (Align::Centre, Duration::AnimatedSlide),
            _ => (Align::Centre, Duration::Time),
        };
        StyleInfo { align, duration }
    }
}

/// What to do with an RPC 73.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameText {
    /// Length outside 1..=200, or style ≥ 201: nothing happens.
    Ignored,
    /// Style accepted by the original but outside GTA's 0..=6 (only when hardening).
    IgnoredInvalidStyle(i32),
    /// Clear current prints (opcode 0x00BE), then `AddBigMessage(text, time, style)`.
    Show { style: Style, time_ms: u32, text: Vec<u8> },
}

impl GameText {
    /// Applies the client's acceptance rules.
    pub fn from_rpc(rpc: &SendGameText) -> Self {
        if rpc.text.is_empty() || rpc.text.len() > MAX_TEXT_LEN || rpc.style >= STYLE_LIMIT {
            return Self::Ignored;
        }
        match Style::from_index(rpc.style) {
            Some(style) => Self::Show { style, time_ms: rpc.time_ms, text: c_str(&rpc.text).to_vec() },
            None if HARDEN_STYLE_RANGE => Self::IgnoredInvalidStyle(rpc.style),
            None => Self::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc(style: i32, len: usize) -> SendGameText {
        SendGameText { style, time_ms: 3000, text: vec![b'a'; len] }
    }

    #[test]
    fn uca_73_length_1_to_200() {
        assert_eq!(GameText::from_rpc(&rpc(3, 0)), GameText::Ignored);
        assert_eq!(GameText::from_rpc(&rpc(3, 201)), GameText::Ignored);
        assert!(matches!(GameText::from_rpc(&rpc(3, 200)), GameText::Show { style: Style::MiddleSmaller, .. }));
        assert!(matches!(GameText::from_rpc(&rpc(0, 1)), GameText::Show { time_ms: 3000, .. }));
    }

    #[test]
    fn uca_73_style_signed_below_201_hardened_to_0_6() {
        assert_eq!(GameText::from_rpc(&rpc(201, 5)), GameText::Ignored);
        assert_eq!(GameText::from_rpc(&rpc(7, 5)), GameText::IgnoredInvalidStyle(7));
        assert_eq!(GameText::from_rpc(&rpc(-1, 5)), GameText::IgnoredInvalidStyle(-1));
        assert!(matches!(GameText::from_rpc(&rpc(6, 5)), GameText::Show { style: Style::LightBlueTop, .. }));
    }

    #[test]
    fn style_info_table() {
        assert_eq!(Style::BottomRight.info(), StyleInfo { align: Align::Right, duration: Duration::AnimatedFade });
        assert_eq!(Style::WhiteMiddleSmaller.info().duration, Duration::AnimatedSlide);
        for i in [0, 2, 3, 4, 6] {
            assert_eq!(
                Style::from_index(i).unwrap().info(),
                StyleInfo { align: Align::Centre, duration: Duration::Time }
            );
        }
    }
}
