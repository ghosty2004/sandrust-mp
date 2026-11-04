//! In-game UI (milestone M4): chat window, chat input, local commands, game
//! text and client messages.
//!
//! - [`core`]: platform-independent state on top of `samp-ui` (host-tested).
//! - [`colours`]: the player colour table used for chat names.
//! - Windows only: `d3d` (device proxy, Present/Reset wrappers, D3DX loading),
//!   `render` (fonts and drawing), `wndproc` (window-procedure subclass),
//!   `glue` (global state and the entry points called by `dll` and `client`).
//!
//! Notes and deviations: `docs/phase3/m4-notes.md`.

pub mod colours;
pub mod core;
pub mod inputlock;

#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod glue;
#[cfg(windows)]
pub mod mattext;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod wndproc;

#[cfg(windows)]
pub use glue::*;
