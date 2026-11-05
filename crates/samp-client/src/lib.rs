//! samp-rs in-game client: a DLL injected into gta_sa.exe 1.0 US.
//!
//! Host builds (Linux) compile only the platform-independent parts so the
//! workspace tests and lints run everywhere; the Windows entry point, the
//! patch application and the per-frame hook live in `dll` (cfg(windows)).

pub mod anims;
pub mod animtable;
pub mod behaviour;
pub mod cdstream;
pub mod classsel;
pub mod combat;
pub mod cmdline;
pub mod control;
pub mod footsync;
pub mod frametimer;
pub mod game;
pub mod log;
pub mod netclient;
pub mod patch;
pub mod patches;
pub mod pe;
pub mod pedhooks;
pub mod remote;
pub mod saafs;
pub mod scm;
pub mod scmops;
pub mod script;
pub mod sectors;
pub mod stores;
pub mod ui;
pub mod vehicle;
pub mod vehpatches;
pub mod world;
pub mod worldents;

#[cfg(windows)]
mod behaviourclient;
#[cfg(windows)]
mod client;
#[cfg(windows)]
mod combatclient;
#[cfg(windows)]
mod combathooks;
#[cfg(windows)]
mod diag;
#[cfg(windows)]
mod dll;
#[cfg(windows)]
mod entities;
#[cfg(windows)]
mod filehook;
#[cfg(windows)]
mod objects;
#[cfg(windows)]
mod players;
#[cfg(windows)]
mod swap;
#[cfg(windows)]
mod vehhooks;
#[cfg(windows)]
mod vehicles;
#[cfg(windows)]
mod worldhooks;
