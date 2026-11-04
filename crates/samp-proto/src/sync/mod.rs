//! Sync packets 200–212 (RakNet user packets, not RPCs).
//!
//! Spec: docs/re/subsystems/remote-players-sync.md (§0 conventions, §2 packets).
//!
//! Shape of the protocol:
//! - **Client→server** sync is `u8 packetId` followed by a fixed-size raw copy
//!   of a client struct (68/63/24/31/40/67/54/18 bytes for 207/200/211/203/206/
//!   209/210/212). Nothing is aligned: after the id byte the stream happens to
//!   be byte-aligned, but the raw blocks would be copied bit-wise otherwise.
//! - **Server→client** foot (207) and driver (200) sync use compact encodings
//!   (conditional fields, packed health/armour nibbles, normalised quaternion,
//!   compressed velocity; see [`encoding`]). The other S→C sync packets are
//!   `u8 id, u16 playerId` + the same raw struct as C→S. Markers (208) have
//!   their own layout and no player id.
//! - Only 207 and 200 honour an optional leading `0x28 + u32` RakNet timestamp
//!   on receive (§0.1); [`decode`] rejects a timestamp prefix on any other id.
//!
//! Conventions in this module:
//! - Each packet type implements [`SyncPacket`]: `PACKET_ID`, `read` (starting
//!   after the id byte) and `write` (including the id byte).
//! - C→S raw types are named after open.mp (`FootSync`, `VehicleSync`, ...)
//!   and also expose `SIZE` plus `read_body`/`write_body` for the raw block.
//!   S→C forms are `Remote*` (player id + data) and [`MarkersSync`].
//! - Decoders are faithful to the wire: quantised values are kept as
//!   transmitted, and client-side validation (player-id bounds, range checks)
//!   is documented, never applied.
//! - Send policy (rates, duplicate suppression, reliabilities, channels) is in
//!   [`policy`] as constants.

use crate::bitstream::{BitReader, BitWriter};
use crate::raknet::id::TIMESTAMP;

/// A sync packet with a fixed packet id.
pub trait SyncPacket: Sized {
    const PACKET_ID: u8;

    /// Read the packet body, starting right after the id byte.
    fn read(r: &mut BitReader) -> Option<Self>;

    /// Write the complete packet, id byte included.
    fn write(&self, w: &mut BitWriter);

    /// Encode as a fresh packet (id byte included).
    fn to_bits(&self) -> BitWriter {
        let mut w = BitWriter::new();
        self.write(&mut w);
        w
    }
}

/// Implements [`SyncPacket`] for a raw fixed-size C→S struct that has inherent
/// `read_body` / `write_body`.
macro_rules! raw_sync_packet {
    ($ty:ty, $id:expr) => {
        impl $crate::sync::SyncPacket for $ty {
            const PACKET_ID: u8 = $id;
            fn read(r: &mut $crate::bitstream::BitReader) -> Option<Self> {
                Self::read_body(r)
            }
            fn write(&self, w: &mut $crate::bitstream::BitWriter) {
                w.write_u8($id);
                self.write_body(w);
            }
        }
    };
}

/// Declares an S→C wrapper `u8 id, u16 playerId, raw body` around a raw struct.
macro_rules! remote_raw_sync {
    ($(#[$m:meta])* $name:ident, $inner:ty) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            /// Player the data belongs to.
            pub player_id: u16,
            /// Same raw layout as the C→S packet.
            pub sync: $inner,
        }

        impl $crate::sync::SyncPacket for $name {
            const PACKET_ID: u8 = <$inner as $crate::sync::SyncPacket>::PACKET_ID;
            fn read(r: &mut $crate::bitstream::BitReader) -> Option<Self> {
                Some(Self { player_id: r.read_u16()?, sync: <$inner>::read_body(r)? })
            }
            fn write(&self, w: &mut $crate::bitstream::BitWriter) {
                w.write_u8(<Self as $crate::sync::SyncPacket>::PACKET_ID);
                w.write_u16(self.player_id);
                self.sync.write_body(w);
            }
        }
    };
}

pub mod encoding;
pub mod player;
pub mod policy;
pub mod rcon;
#[cfg(test)]
mod test_util;
pub mod vehicle;

pub use encoding::{CompressedVec3, NormQuat, PackedHealthArmour, WeaponByte};
pub use player::{
    AimSync, Animation, BulletSync, FootSync, Marker, MarkersSync, RemoteAimSync, RemoteBulletSync, RemoteFootSync,
    SpectatorSync, StatsSync, Surf, WeaponSlot, WeaponsUpdate,
};
pub use rcon::RconCommand;
pub use vehicle::{
    PassengerSync, RemotePassengerSync, RemoteTrailerSync, RemoteUnoccupiedSync, RemoteVehicleSync, TrailerSync,
    UnoccupiedSync, VehicleSync,
};

/// Packet ids (SA:MP numbering).
pub mod ids {
    pub const VEHICLE_SYNC: u8 = 200;
    pub const RCON_COMMAND: u8 = 201;
    pub const AIM_SYNC: u8 = 203;
    pub const WEAPONS_UPDATE: u8 = 204;
    pub const STATS_SYNC: u8 = 205;
    pub const BULLET_SYNC: u8 = 206;
    pub const FOOT_SYNC: u8 = 207;
    pub const MARKERS_SYNC: u8 = 208;
    pub const UNOCCUPIED_SYNC: u8 = 209;
    pub const TRAILER_SYNC: u8 = 210;
    pub const PASSENGER_SYNC: u8 = 211;
    pub const SPECTATOR_SYNC: u8 = 212;
}

/// Packet ids the client handles on receive.
pub const INCOMING_IDS: &[u8] = &[
    ids::VEHICLE_SYNC,
    ids::AIM_SYNC,
    ids::BULLET_SYNC,
    ids::FOOT_SYNC,
    ids::MARKERS_SYNC,
    ids::UNOCCUPIED_SYNC,
    ids::TRAILER_SYNC,
    ids::PASSENGER_SYNC,
];

/// A decoded server→client sync packet.
#[derive(Clone, Debug, PartialEq)]
pub enum IncomingSync {
    /// 207; `timestamp` is the optional `0x28` prefix (used by the client to
    /// drop late packets, §2.1).
    Foot {
        timestamp: Option<u32>,
        sync: RemoteFootSync,
    },
    /// 200; `timestamp` as for [`IncomingSync::Foot`].
    Vehicle {
        timestamp: Option<u32>,
        sync: RemoteVehicleSync,
    },
    Passenger(RemotePassengerSync),
    Aim(RemoteAimSync),
    Bullet(RemoteBulletSync),
    Markers(MarkersSync),
    Unoccupied(RemoteUnoccupiedSync),
    Trailer(RemoteTrailerSync),
}

impl IncomingSync {
    pub fn packet_id(&self) -> u8 {
        match self {
            Self::Foot { .. } => ids::FOOT_SYNC,
            Self::Vehicle { .. } => ids::VEHICLE_SYNC,
            Self::Passenger(_) => ids::PASSENGER_SYNC,
            Self::Aim(_) => ids::AIM_SYNC,
            Self::Bullet(_) => ids::BULLET_SYNC,
            Self::Markers(_) => ids::MARKERS_SYNC,
            Self::Unoccupied(_) => ids::UNOCCUPIED_SYNC,
            Self::Trailer(_) => ids::TRAILER_SYNC,
        }
    }

    /// Encode the packet as a server would send it, including the timestamp
    /// prefix when one is set (only representable for 207/200).
    pub fn write(&self, w: &mut BitWriter) {
        let ts = |w: &mut BitWriter, t: &Option<u32>| {
            if let Some(t) = t {
                w.write_u8(TIMESTAMP);
                w.write_u32(*t);
            }
        };
        match self {
            Self::Foot { timestamp, sync } => {
                ts(w, timestamp);
                sync.write(w);
            }
            Self::Vehicle { timestamp, sync } => {
                ts(w, timestamp);
                sync.write(w);
            }
            Self::Passenger(p) => p.write(w),
            Self::Aim(p) => p.write(w),
            Self::Bullet(p) => p.write(w),
            Self::Markers(p) => p.write(w),
            Self::Unoccupied(p) => p.write(w),
            Self::Trailer(p) => p.write(w),
        }
    }
}

/// Decode a server→client sync packet from a message's bytes (`bit_len` bits).
///
/// Honours the optional `0x28 + u32` timestamp prefix only for 207 and 200,
/// as the client does (§0.1). Returns `None` for other ids, a timestamp prefix
/// on any other packet, or truncated data. Trailing bits are ignored (the
/// client does not check them). Player-id validity (`< 1005`, or `< 1004` for
/// 206/208) is left to the caller.
pub fn decode(packet: &[u8], bit_len: usize) -> Option<IncomingSync> {
    let mut r = BitReader::with_bit_len(packet, bit_len);
    let mut id = r.read_u8()?;
    let mut timestamp = None;
    if id == TIMESTAMP {
        timestamp = Some(r.read_u32()?);
        id = r.read_u8()?;
        if id != ids::FOOT_SYNC && id != ids::VEHICLE_SYNC {
            return None;
        }
    }
    let r = &mut r;
    Some(match id {
        ids::FOOT_SYNC => IncomingSync::Foot { timestamp, sync: RemoteFootSync::read(r)? },
        ids::VEHICLE_SYNC => IncomingSync::Vehicle { timestamp, sync: RemoteVehicleSync::read(r)? },
        ids::PASSENGER_SYNC => IncomingSync::Passenger(RemotePassengerSync::read(r)?),
        ids::AIM_SYNC => IncomingSync::Aim(RemoteAimSync::read(r)?),
        ids::BULLET_SYNC => IncomingSync::Bullet(RemoteBulletSync::read(r)?),
        ids::MARKERS_SYNC => IncomingSync::Markers(MarkersSync::read(r)?),
        ids::UNOCCUPIED_SYNC => IncomingSync::Unoccupied(RemoteUnoccupiedSync::read(r)?),
        ids::TRAILER_SYNC => IncomingSync::Trailer(RemoteTrailerSync::read(r)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Vec3;

    fn roundtrip(p: IncomingSync) {
        let mut w = BitWriter::new();
        p.write(&mut w);
        assert_eq!(decode(w.as_bytes(), w.bit_len()).as_ref(), Some(&p), "{p:?}");
        assert!(INCOMING_IDS.contains(&p.packet_id()));
    }

    fn foot() -> RemoteFootSync {
        RemoteFootSync {
            player_id: 12,
            left_right: Some(-128),
            keys: 4,
            position: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            velocity: CompressedVec3::from_vec3(Vec3 { x: 0.1, y: 0.0, z: 0.0 }),
            ..Default::default()
        }
    }

    #[test]
    fn decode_all_incoming() {
        roundtrip(IncomingSync::Foot { timestamp: None, sync: foot() });
        roundtrip(IncomingSync::Foot { timestamp: Some(0xdead_beef), sync: foot() });
        let veh = RemoteVehicleSync { player_id: 3, vehicle_id: 400, extra: Some(5), ..Default::default() };
        roundtrip(IncomingSync::Vehicle { timestamp: None, sync: veh });
        roundtrip(IncomingSync::Vehicle { timestamp: Some(77), sync: veh });
        roundtrip(IncomingSync::Passenger(RemotePassengerSync { player_id: 1, ..Default::default() }));
        roundtrip(IncomingSync::Aim(RemoteAimSync { player_id: 2, ..Default::default() }));
        roundtrip(IncomingSync::Bullet(RemoteBulletSync { player_id: 3, ..Default::default() }));
        roundtrip(IncomingSync::Unoccupied(RemoteUnoccupiedSync { player_id: 4, ..Default::default() }));
        roundtrip(IncomingSync::Trailer(RemoteTrailerSync { player_id: 5, ..Default::default() }));
        roundtrip(IncomingSync::Markers(MarkersSync {
            markers: vec![Marker { player_id: 9, position: Some([1, -2, 3]) }],
        }));
    }

    #[test]
    fn timestamp_prefix_only_on_foot_and_driver() {
        let mut w = BitWriter::new();
        RemoteAimSync { player_id: 2, ..Default::default() }.write(&mut w);
        let mut msg = vec![TIMESTAMP, 1, 0, 0, 0];
        msg.extend_from_slice(w.as_bytes());
        assert_eq!(decode(&msg, 40 + w.bit_len()), None);
        // Same prefix on 207 is accepted and the time is little-endian.
        let mut w = BitWriter::new();
        foot().write(&mut w);
        let mut msg = vec![TIMESTAMP, 0x78, 0x56, 0x34, 0x12];
        msg.extend_from_slice(w.as_bytes());
        assert_eq!(
            decode(&msg, 40 + w.bit_len()),
            Some(IncomingSync::Foot { timestamp: Some(0x1234_5678), sync: foot() })
        );
    }

    #[test]
    fn rejects_outgoing_only_unknown_and_truncated() {
        for id in [201u8, 204, 205, 212, 0, 0x14] {
            assert_eq!(decode(&[id, 0, 0, 0, 0], 40), None, "id {id}");
        }
        let w = foot().to_bits();
        assert_eq!(decode(w.as_bytes(), w.bit_len() - 1), None);
        assert_eq!(decode(&[], 0), None);
        assert_eq!(decode(&[TIMESTAMP, 1, 2], 24), None);
    }
}
