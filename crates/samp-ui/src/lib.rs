//! Renderer-independent logic of the SA:MP 0.3.7-R4 client UI (milestone M4-core).
//!
//! This crate decides *what* the UI shows and *what* user input does; the in-game
//! renderer (samp-client, milestones M4/M8a) only draws the result. No Windows or
//! Direct3D dependency: everything here is host-testable.
//!
//! Specs (clean room, our own words):
//! - `docs/re/subsystems/ui-rendering.md` (RE-5, primary; cited as "UR §n"),
//! - `docs/re/subsystems/ui-chat-audio.md` (RPC payloads; cited as "UCA §n" or by
//!   section name). Where RE-5 corrects UCA, RE-5 wins.
//!
//! Pixel metrics are pluggable: every layout function takes a [`fonts::TextMeasure`]
//! so the renderer supplies real D3DX text sizes. Behaviour the specs mark unknown
//! is implemented behind named constants listed in `docs/phase3/m4-core-notes.md`.
//!
//! Text is handled as raw bytes in the Windows ANSI code page, like the original.

pub mod bubbles;
pub mod chat;
pub mod colour;
pub mod commands;
pub mod config;
pub mod deathwindow;
pub mod dialog;
pub mod edit;
pub mod fonts;
pub mod gametext;
pub mod input;
pub mod keys;
pub mod menu;
pub mod nametags;
pub mod scoreboard;
pub mod text;
pub mod textdraw;

/// Axis-aligned rectangle in pixels (`right`/`bottom` exclusive, D3D `RECT` style).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }
    /// Rectangle from position and size.
    pub const fn xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { left: x, top: y, right: x + w, bottom: y + h }
    }
    pub const fn width(&self) -> i32 {
        self.right - self.left
    }
    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }
    /// Point-in-rectangle test (left/top inclusive, right/bottom exclusive).
    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// C `round()` for the layout formulas (half away from zero).
///
/// Unknown (notes U-ROUND): the original may use a different float→int conversion.
pub fn round_i(v: f64) -> i32 {
    v.round() as i32
}
