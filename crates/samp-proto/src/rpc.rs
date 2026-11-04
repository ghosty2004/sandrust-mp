//! RPC message framing (`0x14`).
//!
//! Spec: docs/re/handshake.md §6:
//! `[0x28 u32 time]? 0x14, u8 id, compressed-u32 payload bit length, payload bits (unaligned)`.

use crate::bitstream::{BitReader, BitWriter};
use crate::raknet::{id, strip_timestamp};

/// RPC IDs used so far (names follow open.mp's NetCode where one exists).
pub mod ids {
    pub const PLAYER_CONNECT: u8 = 25;
    pub const CONNECTION_REJECTED: u8 = 130;
    pub const PLAYER_INIT: u8 = 139;
    pub const SEND_CLIENT_MESSAGE: u8 = 93;
    pub const PLAYER_JOIN: u8 = 137;
    pub const PLAYER_QUIT: u8 = 138;
}

/// A decoded incoming RPC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rpc {
    pub id: u8,
    pub bit_len: u32,
    pub payload: Vec<u8>,
}

impl Rpc {
    pub fn reader(&self) -> BitReader<'_> {
        BitReader::with_bit_len(&self.payload, self.bit_len as usize)
    }
}

/// Encode an RPC message body from a finished payload writer.
pub fn encode(rpc_id: u8, payload: &BitWriter) -> BitWriter {
    let mut w = BitWriter::new();
    w.write_u8(id::RPC);
    w.write_u8(rpc_id);
    w.write_compressed_u32(payload.bit_len() as u32);
    w.write_bits_from(payload.as_bytes(), payload.bit_len());
    if payload.bit_len() == 0 {
        // Observed quirk of the original sender: an empty payload is followed
        // by a second compressed zero.
        w.write_compressed_u32(0);
    }
    w
}

/// Decode an RPC from a message's bytes; returns `None` if it is not an RPC
/// or is truncated.
pub fn decode(msg: &[u8], msg_bits: usize) -> Option<Rpc> {
    let (ts, body) = strip_timestamp(msg);
    let skipped = if ts.is_some() { 40 } else { 0 };
    let mut r = BitReader::with_bit_len(body, msg_bits.checked_sub(skipped)?);
    if r.read_u8()? != id::RPC {
        return None;
    }
    let rpc_id = r.read_u8()?;
    let bit_len = r.read_compressed_u32()?;
    let payload = r.read_bits_to_vec(bit_len as usize)?;
    Some(Rpc { id: rpc_id, bit_len, payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_unaligned_payload() {
        let mut p = BitWriter::new();
        p.write_bits(0b101, 3);
        p.write_u32(0xdeadbeef);
        let w = encode(42, &p);
        let rpc = decode(w.as_bytes(), w.bit_len()).unwrap();
        assert_eq!(rpc.id, 42);
        assert_eq!(rpc.bit_len, 35);
        let mut r = rpc.reader();
        assert_eq!(r.read_bits(3), Some(0b101));
        assert_eq!(r.read_u32(), Some(0xdeadbeef));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn timestamp_prefixed() {
        let p = BitWriter::new();
        let w = encode(7, &p);
        let mut msg = vec![0x28, 1, 2, 3, 4];
        msg.extend_from_slice(w.as_bytes());
        let rpc = decode(&msg, 40 + w.bit_len()).unwrap();
        assert_eq!((rpc.id, rpc.bit_len), (7, 0));
    }
}
