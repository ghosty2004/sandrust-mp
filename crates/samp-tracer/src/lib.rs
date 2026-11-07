//! samp-tracer: a DLL loaded into gta_sa.exe 1.0 US next to a SA:MP client (the original samp.dll or
//! our samp_client.dll). It inline-hooks a curated list of GTA functions, writes every relevant call
//! (frame, function, caller module+offset, key arguments) to `<GTA_DIR>/trace.jsonl`, and serves the
//! samp-client control-channel protocol so tests can query either client's game state.
//! Black-box observation: it never patches the client module's code, it watches what reaches the game.
//! Host builds compile the platform-independent parts for tests; the Windows side is `win`.
//! Background and findings: docs/phase3/trace-notes.md.

pub mod bass;
pub mod control;
pub mod cov;
pub mod d3d;
pub mod efx;
pub mod fileio;
pub mod gta;
pub mod hooks;
pub mod json;
pub mod mem;
pub mod sampstate;
pub mod scm;
pub mod watch;
pub mod x86;

#[cfg(windows)]
mod win;
