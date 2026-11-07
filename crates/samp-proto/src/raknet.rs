//! RakNet 2.x datagram and message framing as used by SA:MP 0.3.7.
//!
//! Spec: docs/re/handshake.md §3. Every connected-mode datagram is:
//! `hasAcks` bit, optional ack ranges, then zero or more messages until fewer
//! than 16 bits remain.

use crate::bitstream::{BitReader, BitWriter};

/// Message IDs carried inside reliable/unreliable messages (SA:MP numbering).
pub mod id {
    pub const INTERNAL_PING: u8 = 0x06;
    pub const CONNECTED_PONG: u8 = 0x09;
    pub const CONNECTION_REQUEST: u8 = 0x0b;
    pub const AUTH_KEY: u8 = 0x0c;
    pub const RPC: u8 = 0x14;
    pub const KEEP_ALIVE: u8 = 0x17;
    pub const NEW_INCOMING_CONNECTION: u8 = 0x1e;
    pub const DISCONNECTION_NOTIFICATION: u8 = 0x20;
    pub const CONNECTION_LOST: u8 = 0x21;
    pub const CONNECTION_REQUEST_ACCEPTED: u8 = 0x22;
    /// RakNet static data; the 0.3.7 client sends it empty right after connecting.
    pub const RECEIVED_STATIC_DATA: u8 = 0x29;
    pub const CONNECTION_BANNED: u8 = 0x24;
    pub const INVALID_PASSWORD: u8 = 0x25;
    pub const TIMESTAMP: u8 = 0x28;
}

/// Reliability values as they appear on the wire (stock RakNet enum + 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reliability {
    Unreliable = 6,
    UnreliableSequenced = 7,
    Reliable = 8,
    ReliableOrdered = 9,
    ReliableSequenced = 10,
}

impl Reliability {
    pub fn from_wire(v: u32) -> Option<Self> {
        Some(match v {
            6 => Self::Unreliable,
            7 => Self::UnreliableSequenced,
            8 => Self::Reliable,
            9 => Self::ReliableOrdered,
            10 => Self::ReliableSequenced,
            _ => return None,
        })
    }

    /// Carries an ordering channel + index.
    pub fn is_ordered_or_sequenced(self) -> bool {
        matches!(self, Self::UnreliableSequenced | Self::ReliableOrdered | Self::ReliableSequenced)
    }

    /// Must be acknowledged by the receiver.
    pub fn is_reliable(self) -> bool {
        matches!(self, Self::Reliable | Self::ReliableOrdered | Self::ReliableSequenced)
    }

    pub fn is_sequenced(self) -> bool {
        matches!(self, Self::UnreliableSequenced | Self::ReliableSequenced)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ordering {
    /// 5-bit channel (0..32).
    pub channel: u8,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub id: u16,
    pub index: u32,
    pub count: u32,
}

/// One message inside a datagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub number: u16,
    pub reliability: Reliability,
    pub ordering: Option<Ordering>,
    pub split: Option<Split>,
    /// Payload length in bits (payload may end mid-byte).
    pub bit_len: u16,
    pub data: Vec<u8>,
}

/// Inclusive range of acknowledged message numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AckRange {
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Datagram {
    pub acks: Vec<AckRange>,
    pub messages: Vec<Message>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    Truncated,
    BadReliability(u32),
    BadAckRange,
    MissingOrdering,
}

/// Maximum message payload the 0.3.7 client accepts (bytes).
pub const MAX_MESSAGE_BYTES: usize = 0x5d4;

impl Message {
    fn encode(&self, w: &mut BitWriter) {
        w.write_u16(self.number);
        w.write_bits(self.reliability as u32, 4);
        if self.reliability.is_ordered_or_sequenced() {
            let o = self.ordering.expect("ordered/sequenced message without ordering info");
            w.write_bits(o.channel.into(), 5);
            w.write_u16(o.index);
        }
        w.write_bit(self.split.is_some());
        if let Some(s) = self.split {
            w.write_u16(s.id);
            w.write_compressed_u32(s.index);
            w.write_compressed_u32(s.count);
        }
        w.write_compressed_u16(self.bit_len);
        w.write_aligned_bytes(&self.data[..usize::from(self.bit_len).div_ceil(8)]);
    }

    fn decode(r: &mut BitReader) -> Result<Self, DecodeError> {
        let t = DecodeError::Truncated;
        let number = r.read_u16().ok_or(t)?;
        let rel = r.read_bits(4).ok_or(DecodeError::Truncated)?;
        let reliability = Reliability::from_wire(rel).ok_or(DecodeError::BadReliability(rel))?;
        let ordering = if reliability.is_ordered_or_sequenced() {
            let channel = r.read_bits(5).ok_or(DecodeError::Truncated)? as u8;
            let index = r.read_u16().ok_or(DecodeError::Truncated)?;
            Some(Ordering { channel, index })
        } else {
            None
        };
        let split = if r.read_bit().ok_or(DecodeError::Truncated)? {
            Some(Split {
                id: r.read_u16().ok_or(DecodeError::Truncated)?,
                index: r.read_compressed_u32().ok_or(DecodeError::Truncated)?,
                count: r.read_compressed_u32().ok_or(DecodeError::Truncated)?,
            })
        } else {
            None
        };
        let bit_len = r.read_compressed_u16().ok_or(DecodeError::Truncated)?;
        let bytes = usize::from(bit_len).div_ceil(8);
        if bytes >= MAX_MESSAGE_BYTES {
            return Err(DecodeError::Truncated);
        }
        let data = r.read_aligned_bytes(bytes).ok_or(DecodeError::Truncated)?;
        Ok(Self { number, reliability, ordering, split, bit_len, data })
    }
}

impl Datagram {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bit(!self.acks.is_empty());
        if !self.acks.is_empty() {
            w.write_compressed_u16(self.acks.len() as u16);
            for a in &self.acks {
                w.write_bit(a.min == a.max);
                w.write_u16(a.min);
                if a.min != a.max {
                    w.write_u16(a.max);
                }
            }
        }
        for m in &self.messages {
            m.encode(&mut w);
        }
        w.into_bytes()
    }

    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 2 {
            return Err(DecodeError::TooShort);
        }
        let mut r = BitReader::new(data);
        let mut dg = Datagram::default();
        if r.read_bit().ok_or(DecodeError::Truncated)? {
            let count = r.read_compressed_u16().ok_or(DecodeError::Truncated)?;
            for _ in 0..count {
                let single = r.read_bit().ok_or(DecodeError::Truncated)?;
                let min = r.read_u16().ok_or(DecodeError::Truncated)?;
                let max = if single { min } else { r.read_u16().ok_or(DecodeError::Truncated)? };
                if min > max {
                    return Err(DecodeError::BadAckRange);
                }
                dg.acks.push(AckRange { min, max });
            }
        }
        while r.remaining() >= 16 {
            dg.messages.push(Message::decode(&mut r)?);
        }
        Ok(dg)
    }
}

/// Collapse a set of message numbers into sorted inclusive ranges.
pub fn ack_ranges(numbers: &mut Vec<u16>) -> Vec<AckRange> {
    numbers.sort_unstable();
    numbers.dedup();
    let mut out: Vec<AckRange> = Vec::new();
    for &n in numbers.iter() {
        match out.last_mut() {
            Some(r) if r.max.checked_add(1) == Some(n) => r.max = n,
            _ => out.push(AckRange { min: n, max: n }),
        }
    }
    out
}

/// Most password bytes a connection request carries (terminator included); RakPeer::Connect cuts longer
/// passwords. Mirrors samp.dll 0x10040581.
pub const MAX_PASSWORD_BYTES: usize = 255;

/// The connection request (`0x0b`) sent after the open-connection reply. RakClient keeps a non-empty password
/// as its bytes plus the terminating NUL and RakPeer::Connect cuts that to [`MAX_PASSWORD_BYTES`]; an empty
/// password sends the id alone (docs/re/handshake.md, "Server password"). Mirrors samp.dll 0x10034220.
pub fn connection_request(password: &[u8]) -> Vec<u8> {
    let mut msg = vec![id::CONNECTION_REQUEST];
    if !password.is_empty() {
        msg.extend_from_slice(password);
        msg.push(0);
        msg.truncate(1 + MAX_PASSWORD_BYTES);
    }
    msg
}

/// Strip an optional `0x28 + u32` timestamp prefix; returns (timestamp, rest).
pub fn strip_timestamp(data: &[u8]) -> (Option<u32>, &[u8]) {
    match data {
        [id::TIMESTAMP, a, b, c, d, rest @ ..] => (Some(u32::from_le_bytes([*a, *b, *c, *d])), rest),
        _ => (None, data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_request_carries_the_password_with_its_terminator() {
        assert_eq!(connection_request(b""), vec![0x0b]);
        assert_eq!(connection_request(b"pw"), vec![0x0b, b'p', b'w', 0]);
        // 254 characters still fit with the NUL; longer ones lose it and are cut to 255 bytes.
        let p = vec![b'a'; 254];
        let m = connection_request(&p);
        assert_eq!((m.len(), m[254], m[255]), (256, b'a', 0));
        let m = connection_request(&[b'a'; 300]);
        assert_eq!(m.len(), 1 + MAX_PASSWORD_BYTES);
        assert!(m[1..].iter().all(|&b| b == b'a'));
    }

    fn msg(number: u16, reliability: Reliability, data: &[u8]) -> Message {
        Message {
            number,
            reliability,
            ordering: reliability.is_ordered_or_sequenced().then_some(Ordering { channel: 3, index: 77 }),
            split: None,
            bit_len: (data.len() * 8) as u16,
            data: data.to_vec(),
        }
    }

    #[test]
    fn roundtrip_with_acks_and_messages() {
        let dg = Datagram {
            acks: vec![AckRange { min: 4, max: 4 }, AckRange { min: 7, max: 9 }],
            messages: vec![
                msg(1, Reliability::Reliable, &[0x0b]),
                msg(2, Reliability::ReliableOrdered, &[0x14, 1, 2, 3]),
                Message {
                    split: Some(Split { id: 5, index: 1, count: 3 }),
                    ..msg(3, Reliability::Unreliable, &[9; 40])
                },
            ],
        };
        assert_eq!(Datagram::decode(&dg.encode()), Ok(dg));
    }

    #[test]
    fn partial_last_byte_payload() {
        let mut m = msg(10, Reliability::Reliable, &[0b1010_0000]);
        m.bit_len = 3;
        let dg = Datagram { acks: vec![], messages: vec![m] };
        assert_eq!(Datagram::decode(&dg.encode()), Ok(dg));
    }

    #[test]
    fn acks_only() {
        let dg = Datagram { acks: vec![AckRange { min: 0, max: 0 }], messages: vec![] };
        assert_eq!(Datagram::decode(&dg.encode()), Ok(dg));
    }

    #[test]
    fn rejects_low_reliability() {
        let mut w = BitWriter::new();
        w.write_bit(false);
        w.write_u16(1);
        w.write_bits(2, 4);
        w.write_bits(0, 16);
        assert_eq!(Datagram::decode(w.as_bytes()), Err(DecodeError::BadReliability(2)));
    }

    #[test]
    fn range_collapse() {
        let mut v = vec![5, 3, 4, 9, 4, 10];
        assert_eq!(ack_ranges(&mut v), vec![AckRange { min: 3, max: 5 }, AckRange { min: 9, max: 10 }]);
    }

    #[test]
    fn timestamp_prefix() {
        assert_eq!(strip_timestamp(&[0x28, 1, 0, 0, 0, 0xc8, 9]), (Some(1), &[0xc8, 9][..]));
        assert_eq!(strip_timestamp(&[0xc8]), (None, &[0xc8][..]));
    }
}
