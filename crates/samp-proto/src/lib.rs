//! Clean-room implementation of the SA:MP 0.3.7 network protocol.
//!
//! Every behaviour here is implemented from the specs in `docs/re/`, which
//! describe the protocol in our own words. Do not paste decompiled code.

pub mod auth;
pub mod bitstream;
pub mod cipher;
pub mod connection;
pub mod raknet;
pub mod rpc;
pub mod rpcs;
mod sbox;
pub mod session;
pub mod strcomp;
pub mod sync;
pub mod types;
pub mod unconnected;
