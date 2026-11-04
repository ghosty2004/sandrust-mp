//! samp-rs in-game client: a DLL injected into gta_sa.exe 1.0 US.
//!
//! Host builds (Linux) compile only the platform-independent parts so the
//! workspace tests and lints run everywhere; the Windows entry point, the
//! patch application and the per-frame hook live in `dll` (cfg(windows)).

pub mod cdstream;
pub mod classsel;
pub mod cmdline;
pub mod control;
pub mod footsync;
pub mod game;
pub mod log;
pub mod netclient;
pub mod patch;
pub mod patches;
pub mod remote;
pub mod scm;
pub mod script;
pub mod sectors;
pub mod ui;

#[cfg(windows)]
mod client;
#[cfg(windows)]
mod diag;
#[cfg(windows)]
mod dll;
#[cfg(windows)]
mod players;
#[cfg(windows)]
mod swap;
