//! In-game UI: chat window and input, local commands, game text, client messages, dialogs, textdraws,
//! menus, scoreboard, nametags, death window and chat bubbles.
//!
//! [`core`] holds the platform-independent state on top of `samp-ui` (tested on the host); [`colours`] and
//! [`inputlock`] are host-testable too. The Windows-only modules hook Direct3D (`d3d`, `dxut`), draw
//! (`render`, `textdraws`, `menus`, `mattext`, `preview`), subclass the game window (`wndproc`) and expose
//! the entry points used by `dll` and `client` (`glue`).
//! Spec: docs/re/subsystems/ui-rendering.md and ui-chat-audio.md; notes in docs/phase3/m4-notes.md.

pub mod colours;
pub mod core;
pub mod inputlock;

#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod dxut;
#[cfg(windows)]
mod menus;
#[cfg(windows)]
mod textdraws;
#[cfg(windows)]
mod glue;
#[cfg(windows)]
pub mod mattext;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod wndproc;

#[cfg(windows)]
pub use glue::*;

/// Test hook for the control-channel command `devreset`: the device reports "lost, reset possible" for
/// the next `checks` `TestCooperativeLevel` calls.
#[cfg(windows)]
pub fn force_device_lost(checks: u32) {
    d3d::FORCE_LOST.store(checks, std::sync::atomic::Ordering::SeqCst);
}
