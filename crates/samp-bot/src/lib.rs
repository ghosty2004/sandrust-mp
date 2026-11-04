//! Headless SA:MP test client as a library: transport and join
//! ([`client`]), the game-side state machine with scripted modes ([`bot`]),
//! and the pieces they share. The `samp-bot` binary is a thin CLI on top.

pub mod bot;
pub mod client;
pub mod gate;
pub mod geom;
pub mod net;
pub mod path;
pub mod weapons;
pub mod world;

use samp_proto::bitstream::BitWriter;
use samp_proto::rpcs::{OutgoingRpc, SendParams};
use samp_proto::sync::{SyncPacket, policy};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Where game messages go: the live [`client::Client`], or a recorder in tests.
pub trait Outbox {
    fn send(&mut self, bits: &BitWriter, params: SendParams);
}

/// Send a C→S RPC with the original client's send parameters.
pub fn send_rpc<P: OutgoingRpc>(out: &mut (impl Outbox + ?Sized), p: &P) {
    out.send(&p.to_rpc(), P::SEND_PARAMS);
}

/// Send a sync packet with its send parameters (no duplicate suppression).
pub fn send_sync<S: SyncPacket>(out: &mut (impl Outbox + ?Sized), s: &S) {
    out.send(&s.to_bits(), policy::send_params(S::PACKET_ID).expect("sync packet has send params"));
}
