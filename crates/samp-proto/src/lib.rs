//! Clean-room, version-independent implementation of the SA:MP 0.3.7 network protocol.
//!
//! Layers, bottom up: [`cipher`] and [`unconnected`] (pre-connection handshake), [`raknet`] framing
//! and the sans-IO reliability layer in [`connection`], [`auth`] and [`session`] (join, InitGame,
//! client serial), [`rpc`] framing with typed payloads for every RPC in [`rpcs`], and the sync
//! packets 200–212 in [`sync`]. [`bitstream`], [`strcomp`] and [`types`] are the shared encodings.
//! Implemented from the specs in `docs/re/` (start at docs/re/README.md); never from decompiled code.

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
