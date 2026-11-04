//! Typed RPC payloads, one module per subsystem (see docs/re/subsystems/).
//!
//! Conventions every module follows:
//! - One struct per RPC payload (unit struct if empty), implementing
//!   [`RpcPayload`]. Fields use `crate::types` (Vec3, Quat, byte strings).
//!   Struct names follow open.mp's NetCode names where one exists.
//! - S→C payloads get `read` + `write` (write lets tests and tools build them);
//!   C→S payloads get `write` + `read` likewise. Both directions round-trip.
//! - Each module exposes `pub enum Incoming` (S→C payloads of the module) with
//!   `pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming>` (None for
//!   ids outside the module or malformed data) and `pub const INCOMING_IDS: &[u8]`.
//! - [`Incoming`] / [`decode`] here dispatch an S→C RPC to the owning module
//!   (plus the handshake RPCs 130 and 139, which live in `crate::session`).
//! - Behavioural notes (client-side limits, clamps) live in doc comments with
//!   a pointer to the spec section; the decoders themselves are faithful to the
//!   wire and do not apply client behaviour.
//! - C→S payloads also implement [`OutgoingRpc`] (the reliability, ordering
//!   channel and priority the original client sends them with). Each module
//!   with C→S payloads exposes `pub enum Outgoing`, `pub const OUTGOING_IDS`,
//!   `pub fn decode_outgoing` and `pub fn outgoing_send_params`; [`Outgoing`]
//!   here wraps them per module (see `docs/phase3/net7-notes.md`).

use crate::bitstream::{BitReader, BitWriter};
use crate::raknet::Reliability;

pub use crate::sync::policy::SendParams;

/// A typed RPC payload with a fixed RPC id.
pub trait RpcPayload: Sized {
    const ID: u8;
    fn read(r: &mut BitReader) -> Option<Self>;
    fn write(&self, w: &mut BitWriter);

    /// Encode as a complete RPC message body (`0x14`, id, length, payload).
    fn to_rpc(&self) -> BitWriter {
        let mut p = BitWriter::new();
        self.write(&mut p);
        crate::rpc::encode(Self::ID, &p)
    }
}

/// Priority the original client passes at every RPC send site (rpc-ids.md
/// "Client→server RPCs": "Priority argument is 1 at every site").
pub const RPC_PRIORITY: u8 = 1;
/// Ordering channel the original client passes at every RPC send site
/// (rpc-ids.md, and the per-subsystem send notes).
pub const RPC_CHANNEL: u8 = 0;

/// A client→server RPC payload together with the send parameters the original
/// 0.3.7-R4 client uses for it (RakClient `RPC(id, bs, priority, reliability,
/// channel, shiftTimestamp = 0)`).
///
/// Every C→S send site found so far passes priority [`RPC_PRIORITY`] and
/// channel [`RPC_CHANNEL`], so only [`OutgoingRpc::RELIABILITY`] has no
/// default. Each impl's doc comment cites its spec and says whether the value
/// was also confirmed in a capture of the original client (see
/// `docs/phase3/net7-notes.md`).
///
/// Note: the priority never appears on the wire, and the channel is only
/// encoded for ordered/sequenced reliabilities.
pub trait OutgoingRpc: RpcPayload {
    const RELIABILITY: Reliability;
    const CHANNEL: u8 = RPC_CHANNEL;
    const PRIORITY: u8 = RPC_PRIORITY;
    /// The three values above as one [`SendParams`].
    const SEND_PARAMS: SendParams =
        SendParams { priority: Self::PRIORITY, reliability: Self::RELIABILITY, channel: Self::CHANNEL };

    /// Encode as a ready RakNet message (RPC body + send parameters).
    fn to_message(&self) -> OutgoingMessage {
        OutgoingMessage::new(self.to_rpc(), Self::SEND_PARAMS)
    }
}

/// A complete outgoing RPC message plus the reliability layer parameters it is
/// sent with. Hand it to the connection with
/// `conn.send_bits(&m.bits, m.reliability, m.channel)`.
#[derive(Clone, Debug)]
pub struct OutgoingMessage {
    /// RPC message body: `0x14`, id, compressed bit length, payload.
    pub bits: BitWriter,
    pub reliability: Reliability,
    /// Ordering channel (only on the wire for ordered/sequenced reliabilities).
    pub channel: u8,
    /// RakNet send priority (local scheduling only, never on the wire).
    pub priority: u8,
}

impl OutgoingMessage {
    pub fn new(bits: BitWriter, params: SendParams) -> Self {
        Self { bits, reliability: params.reliability, channel: params.channel, priority: params.priority }
    }

    pub fn send_params(&self) -> SendParams {
        SendParams { priority: self.priority, reliability: self.reliability, channel: self.channel }
    }

    /// Replace the send parameters (e.g. [`world::OnPlayerPickUpPickup::POLL_SEND_PARAMS`]).
    pub fn with_params(self, params: SendParams) -> Self {
        Self::new(self.bits, params)
    }
}

/// Declares the C→S payloads of a module (used inside each module): an
/// [`OutgoingRpc`] impl per type (doc comments go on the impl), and the
/// module's `Outgoing` enum, `OUTGOING_IDS`, `decode_outgoing`,
/// `outgoing_send_params`, plus `From` conversions into it and into the
/// top-level [`Outgoing`] variant named first.
macro_rules! outgoing {
    ($top:ident; $( $(#[$m:meta])* $ty:ident => $rel:ident; )+) => {
        $(
            $(#[$m])*
            impl $crate::rpcs::OutgoingRpc for $ty {
                const RELIABILITY: $crate::raknet::Reliability = $crate::raknet::Reliability::$rel;
            }

            impl From<$ty> for Outgoing {
                fn from(p: $ty) -> Self {
                    Self::$ty(p)
                }
            }

            impl From<$ty> for $crate::rpcs::Outgoing {
                fn from(p: $ty) -> Self {
                    Self::$top(Outgoing::$ty(p))
                }
            }
        )+

        /// C→S payloads of this module (see [`crate::rpcs::OutgoingRpc`]).
        #[derive(Clone, Debug, PartialEq)]
        pub enum Outgoing {
            $( $ty($ty), )+
        }

        /// C→S RPC ids of this module, decoded by [`decode_outgoing`].
        pub const OUTGOING_IDS: &[u8] = &[$( <$ty as $crate::rpcs::RpcPayload>::ID, )+];

        /// Decode a C→S payload of this module (tools, tests, differential
        /// checks); `None` for other ids or malformed data.
        pub fn decode_outgoing(id: u8, r: &mut $crate::bitstream::BitReader) -> Option<Outgoing> {
            match id {
                $( <$ty as $crate::rpcs::RpcPayload>::ID => {
                    <$ty as $crate::rpcs::RpcPayload>::read(r).map(Outgoing::$ty)
                } )+
                _ => None,
            }
        }

        /// Send parameters of a C→S id of this module; `None` for other ids.
        pub const fn outgoing_send_params(id: u8) -> Option<$crate::rpcs::SendParams> {
            $(
                if id == <$ty as $crate::rpcs::RpcPayload>::ID {
                    return Some(<$ty as $crate::rpcs::OutgoingRpc>::SEND_PARAMS);
                }
            )+
            None
        }

        impl Outgoing {
            /// RPC id of the payload.
            pub fn id(&self) -> u8 {
                match self {
                    $( Self::$ty(_) => <$ty as $crate::rpcs::RpcPayload>::ID, )+
                }
            }

            /// Encode the payload body (without RPC framing).
            pub fn write(&self, w: &mut $crate::bitstream::BitWriter) {
                match self {
                    $( Self::$ty(p) => $crate::rpcs::RpcPayload::write(p, w), )+
                }
            }

            /// Send parameters the original client uses for this payload.
            pub fn send_params(&self) -> $crate::rpcs::SendParams {
                match self {
                    $( Self::$ty(_) => <$ty as $crate::rpcs::OutgoingRpc>::SEND_PARAMS, )+
                }
            }

            /// Encode as a complete RPC message body.
            pub fn to_rpc(&self) -> $crate::bitstream::BitWriter {
                let mut p = $crate::bitstream::BitWriter::new();
                self.write(&mut p);
                $crate::rpc::encode(self.id(), &p)
            }

            /// Encode as a ready RakNet message.
            pub fn to_message(&self) -> $crate::rpcs::OutgoingMessage {
                $crate::rpcs::OutgoingMessage::new(self.to_rpc(), self.send_params())
            }
        }
    };
}

mod outgoing;
pub use outgoing::{OUTGOING_IDS_BY_MODULE, Outgoing, decode_outgoing, outgoing_ids, send_params};

pub mod player;
pub mod remote;
pub mod session;
pub mod ui;
pub mod vehicle;
mod wire;
pub mod world;

/// Any S→C RPC payload the client registers (plus the dormant custom-model
/// RPCs, see [`world::DORMANT_IN_R4`]), wrapped per owning module.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Player(player::Incoming),
    Remote(remote::Incoming),
    Session(session::Incoming),
    Ui(ui::Incoming),
    Vehicle(vehicle::Incoming),
    World(world::Incoming),
    /// RPC 139 PlayerInit / InitGame (handshake.md §5), see [`crate::session`].
    InitGame(Box<crate::session::InitGame>),
    /// RPC 130 connection rejected: `u8 reason` (session-misc.md §130); see
    /// [`crate::session::rejection_reason`].
    ConnectionRejected {
        reason: u8,
    },
}

impl Incoming {
    /// Same as [`decode`].
    pub fn decode(id: u8, r: &mut BitReader) -> Option<Self> {
        decode(id, r)
    }

    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::Player(m) => m.id(),
            Self::Remote(m) => m.id(),
            Self::Session(m) => m.id(),
            Self::Ui(m) => m.id(),
            Self::Vehicle(m) => m.id(),
            Self::World(m) => m.id(),
            Self::InitGame(_) => crate::rpc::ids::PLAYER_INIT,
            Self::ConnectionRejected { .. } => crate::rpc::ids::CONNECTION_REJECTED,
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::Player(m) => m.write(w),
            Self::Remote(m) => m.write(w),
            Self::Session(m) => m.write(w),
            Self::Ui(m) => m.write(w),
            Self::Vehicle(m) => m.write(w),
            Self::World(m) => m.write(w),
            Self::InitGame(g) => g.write(w),
            Self::ConnectionRejected { reason } => w.write_u8(*reason),
        }
    }

    /// Encode as a complete RPC message body.
    pub fn to_rpc(&self) -> BitWriter {
        let mut p = BitWriter::new();
        self.write(&mut p);
        crate::rpc::encode(self.id(), &p)
    }
}

/// Decode an S→C RPC payload by id, dispatching on each module's
/// `INCOMING_IDS`. `None` for unknown/C→S-only ids or malformed data.
pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
    if player::INCOMING_IDS.contains(&id) {
        player::decode(id, r).map(Incoming::Player)
    } else if remote::INCOMING_IDS.contains(&id) {
        remote::decode(id, r).map(Incoming::Remote)
    } else if session::INCOMING_IDS.contains(&id) {
        session::decode(id, r).map(Incoming::Session)
    } else if ui::INCOMING_IDS.contains(&id) {
        ui::decode(id, r).map(Incoming::Ui)
    } else if vehicle::INCOMING_IDS.contains(&id) {
        vehicle::decode(id, r).map(Incoming::Vehicle)
    } else if world::INCOMING_IDS.contains(&id) {
        world::decode(id, r).map(Incoming::World)
    } else if id == crate::rpc::ids::PLAYER_INIT {
        crate::session::parse_init_game(r).map(|g| Incoming::InitGame(Box::new(g)))
    } else if id == crate::rpc::ids::CONNECTION_REJECTED {
        Some(Incoming::ConnectionRejected { reason: r.read_u8()? })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 141 S→C RPC ids the 0.3.7-R4 client registers (docs/re/rpc-ids.md,
    /// "Server→client RPCs registered by the client"), hardcoded on purpose.
    const REGISTERED_S2C: [u8; 141] = [
        11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24, 26, 27, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
        42, 43, 44, 45, 46, 47, 48, 55, 56, 57, 58, 59, 60, 61, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76,
        77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98, 99, 101, 102, 103, 104,
        105, 106, 107, 108, 111, 112, 113, 116, 117, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 133, 134,
        135, 137, 138, 139, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161,
        162, 163, 164, 165, 166, 167, 169, 170, 171, 172, 173, 174, 175, 176, 178,
    ];

    /// Ids owned by each decoder (the handshake RPCs count as one owner).
    fn owners() -> [(&'static str, &'static [u8]); 7] {
        [
            ("player", player::INCOMING_IDS),
            ("remote", remote::INCOMING_IDS),
            ("session", session::INCOMING_IDS),
            ("ui", ui::INCOMING_IDS),
            ("vehicle", vehicle::INCOMING_IDS),
            ("world", world::INCOMING_IDS),
            ("handshake", &[crate::rpc::ids::CONNECTION_REJECTED, crate::rpc::ids::PLAYER_INIT]),
        ]
    }

    #[test]
    fn every_registered_id_has_exactly_one_owner() {
        for id in REGISTERED_S2C {
            let found: Vec<&str> =
                owners().iter().filter(|(_, ids)| ids.contains(&id)).map(|(name, _)| *name).collect();
            assert_eq!(found.len(), 1, "id {id} owned by {found:?}");
        }
    }

    #[test]
    fn no_unregistered_incoming_ids() {
        // Everything a module decodes is either registered or documented dormant.
        for (name, ids) in owners() {
            for id in ids {
                assert!(
                    REGISTERED_S2C.contains(id) || world::DORMANT_IN_R4.contains(id),
                    "{name} decodes unregistered id {id}"
                );
            }
        }
    }

    #[test]
    fn every_registered_id_decodes_to_its_owner() {
        let zeros = [0u8; 512];
        for id in REGISTERED_S2C.iter().chain(world::DORMANT_IN_R4) {
            let got = decode(*id, &mut BitReader::new(&zeros)).unwrap_or_else(|| panic!("id {id}"));
            let owner = match got {
                Incoming::Player(_) => player::INCOMING_IDS,
                Incoming::Remote(_) => remote::INCOMING_IDS,
                Incoming::Session(_) => session::INCOMING_IDS,
                Incoming::Ui(_) => ui::INCOMING_IDS,
                Incoming::Vehicle(_) => vehicle::INCOMING_IDS,
                Incoming::World(_) => world::INCOMING_IDS,
                Incoming::InitGame(_) => &[crate::rpc::ids::PLAYER_INIT],
                Incoming::ConnectionRejected { .. } => &[crate::rpc::ids::CONNECTION_REJECTED],
            };
            assert!(owner.contains(id), "id {id}");
        }
    }

    #[test]
    fn client_to_server_only_ids_do_not_decode() {
        let zeros = [0u8; 512];
        for id in [25u8, 50, 52, 53, 62, 97, 115, 118, 119, 131, 132, 136, 140, 168, 177, 181, 182, 184] {
            assert_eq!(decode(id, &mut BitReader::new(&zeros)), None, "id {id}");
        }
    }

    #[test]
    fn decode_round_trips_through_rpc_framing() {
        let msg = world::DestroyObject { object_id: 7 }.to_rpc();
        let rpc = crate::rpc::decode(msg.as_bytes(), msg.bit_len()).unwrap();
        assert_eq!(
            decode(rpc.id, &mut rpc.reader()),
            Some(Incoming::World(world::Incoming::DestroyObject(world::DestroyObject { object_id: 7 })))
        );
        let mut r = BitReader::new(&[2]);
        assert_eq!(decode(130, &mut r), Some(Incoming::ConnectionRejected { reason: 2 }));
    }

    #[test]
    fn incoming_id_and_write_round_trip() {
        // Samples: decode zeros and a non-zero byte pattern (where it parses).
        let zeros = [0u8; 2048];
        let ones = [0x01u8; 2048];
        let mut checked = 0;
        for id in REGISTERED_S2C.iter().chain(world::DORMANT_IN_R4) {
            for buf in [&zeros, &ones] {
                let Some(m) = decode(*id, &mut BitReader::new(buf)) else { continue };
                assert_eq!(m.id(), *id, "id {id}");
                let msg = m.to_rpc();
                let rpc = crate::rpc::decode(msg.as_bytes(), msg.bit_len()).unwrap();
                assert_eq!(rpc.id, *id);
                assert_eq!(decode(rpc.id, &mut rpc.reader()).as_ref(), Some(&m), "id {id}");
                checked += 1;
            }
        }
        assert!(checked >= REGISTERED_S2C.len() + world::DORMANT_IN_R4.len());
    }
}
