//! samp-tracer: a DLL loaded into gta_sa.exe 1.0 US next to a SA:MP client
//! (the original samp.dll or our samp_client.dll). It inline-hooks a curated
//! list of GTA functions and writes every relevant call — frame number,
//! function, caller module+offset, key arguments — to `<GTA_DIR>/trace.jsonl`,
//! and serves the samp-client control-channel protocol so tests can query the
//! game state of either client (TRACE-1 / DIFF-1).
//!
//! Black-box observation only: the tracer never reads or patches the client
//! module's code; it watches what reaches the game.
//!
//! Host builds compile the platform-independent parts (decoder, hook table,
//! record formatting, control protocol) for tests; the Windows side lives in
//! `win` (cfg(windows)).

pub mod bass;
pub mod control;
pub mod cov;
pub mod d3d;
pub mod efx;
pub mod gta;
pub mod hooks;
pub mod json;
pub mod mem;
pub mod scm;
pub mod watch;
pub mod x86;

#[cfg(windows)]
mod win;
