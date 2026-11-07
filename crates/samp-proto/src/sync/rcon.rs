//! 201 RCON command (client→server packet). Layout only.
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §2 "201 RCON command".

use super::{SyncPacket, ids};
use crate::bitstream::{BitReader, BitWriter};
use crate::types::{read_str32, write_str32};

/// 201 RCON command, client→server only (the "/rcon" chat command):
/// `u8 201, u32 length, raw[length]` with no NUL terminator; sent RELIABLE on
/// channel 0, never without an argument. The client has no receive path for it;
/// open.mp reads it as a u32-prefixed string. Mirrors samp.dll 0x10069030.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RconCommand {
    /// Command text (Windows code page bytes), e.g. `b"login secret"`.
    pub command: Vec<u8>,
}

impl SyncPacket for RconCommand {
    const PACKET_ID: u8 = ids::RCON_COMMAND;

    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { command: read_str32(r)? })
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        write_str32(w, &self.command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_roundtrip() {
        let p = RconCommand { command: b"cmdlist".to_vec() };
        let w = p.to_bits();
        assert_eq!(w.as_bytes(), [&[201u8, 7, 0, 0, 0][..], b"cmdlist"].concat());
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(r.read_u8(), Some(201));
        assert_eq!(RconCommand::read(&mut r), Some(p));
        // length larger than the data
        assert_eq!(RconCommand::read(&mut BitReader::new(&[9, 0, 0, 0, b'a'])), None);
    }
}
