//! Every client→server RPC payload in one enum, wrapped per owning module
//! (the C→S counterpart of [`super::Incoming`]), plus send-parameter lookup.
//!
//! Send parameters come from the specs' send-site notes (rpc-ids.md
//! "Client→server RPCs", docs/re/subsystems/*.md); which of them were also
//! observed in a capture of the original client is listed in
//! docs/phase3/net7-notes.md and pinned by the tests below.

use super::{OutgoingMessage, SendParams, player, session, ui, vehicle, world};
use crate::bitstream::{BitReader, BitWriter};

/// Any C→S RPC payload the original client sends (incl. the dormant
/// custom-model requests 181/182/184), wrapped per owning module. The remote
/// module has no C→S RPCs (155 lives in `session`).
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Player(player::Outgoing),
    Session(session::Outgoing),
    Ui(ui::Outgoing),
    Vehicle(vehicle::Outgoing),
    World(world::Outgoing),
}

/// C→S ids per module, in the order of the [`Outgoing`] variants.
pub const OUTGOING_IDS_BY_MODULE: [(&str, &[u8]); 5] = [
    ("player", player::OUTGOING_IDS),
    ("session", session::OUTGOING_IDS),
    ("ui", ui::OUTGOING_IDS),
    ("vehicle", vehicle::OUTGOING_IDS),
    ("world", world::OUTGOING_IDS),
];

/// Every C→S RPC id covered by [`Outgoing`].
pub fn outgoing_ids() -> impl Iterator<Item = u8> {
    OUTGOING_IDS_BY_MODULE.into_iter().flat_map(|(_, ids)| ids.iter().copied())
}

impl Outgoing {
    /// Same as [`decode_outgoing`].
    pub fn decode(id: u8, r: &mut BitReader) -> Option<Self> {
        decode_outgoing(id, r)
    }

    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::Player(m) => m.id(),
            Self::Session(m) => m.id(),
            Self::Ui(m) => m.id(),
            Self::Vehicle(m) => m.id(),
            Self::World(m) => m.id(),
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::Player(m) => m.write(w),
            Self::Session(m) => m.write(w),
            Self::Ui(m) => m.write(w),
            Self::Vehicle(m) => m.write(w),
            Self::World(m) => m.write(w),
        }
    }

    /// Send parameters the original client uses for this payload. For RPC 131
    /// this is the collision-hook path; the type-14 poll path uses
    /// [`world::OnPlayerPickUpPickup::POLL_SEND_PARAMS`].
    pub fn send_params(&self) -> SendParams {
        match self {
            Self::Player(m) => m.send_params(),
            Self::Session(m) => m.send_params(),
            Self::Ui(m) => m.send_params(),
            Self::Vehicle(m) => m.send_params(),
            Self::World(m) => m.send_params(),
        }
    }

    /// Encode as a complete RPC message body (`0x14`, id, length, payload).
    pub fn to_rpc(&self) -> BitWriter {
        let mut p = BitWriter::new();
        self.write(&mut p);
        crate::rpc::encode(self.id(), &p)
    }

    /// Encode as a ready RakNet message: pass it to the connection with
    /// `conn.send_bits(&m.bits, m.reliability, m.channel)`.
    pub fn to_message(&self) -> OutgoingMessage {
        OutgoingMessage::new(self.to_rpc(), self.send_params())
    }
}

impl From<player::Outgoing> for Outgoing {
    fn from(m: player::Outgoing) -> Self {
        Self::Player(m)
    }
}
impl From<session::Outgoing> for Outgoing {
    fn from(m: session::Outgoing) -> Self {
        Self::Session(m)
    }
}
impl From<ui::Outgoing> for Outgoing {
    fn from(m: ui::Outgoing) -> Self {
        Self::Ui(m)
    }
}
impl From<vehicle::Outgoing> for Outgoing {
    fn from(m: vehicle::Outgoing) -> Self {
        Self::Vehicle(m)
    }
}
impl From<world::Outgoing> for Outgoing {
    fn from(m: world::Outgoing) -> Self {
        Self::World(m)
    }
}

/// Decode a C→S RPC payload by id (for tools and differential checks).
/// `None` for ids the client never sends or malformed data.
pub fn decode_outgoing(id: u8, r: &mut BitReader) -> Option<Outgoing> {
    if player::OUTGOING_IDS.contains(&id) {
        player::decode_outgoing(id, r).map(Outgoing::Player)
    } else if session::OUTGOING_IDS.contains(&id) {
        session::decode_outgoing(id, r).map(Outgoing::Session)
    } else if ui::OUTGOING_IDS.contains(&id) {
        ui::decode_outgoing(id, r).map(Outgoing::Ui)
    } else if vehicle::OUTGOING_IDS.contains(&id) {
        vehicle::decode_outgoing(id, r).map(Outgoing::Vehicle)
    } else if world::OUTGOING_IDS.contains(&id) {
        world::decode_outgoing(id, r).map(Outgoing::World)
    } else {
        None
    }
}

/// Send parameters the original client uses for C→S RPC `id` (`None` for ids
/// it never sends). The RPC counterpart of [`crate::sync::policy::send_params`].
pub const fn send_params(id: u8) -> Option<SendParams> {
    if let Some(p) = player::outgoing_send_params(id) {
        Some(p)
    } else if let Some(p) = session::outgoing_send_params(id) {
        Some(p)
    } else if let Some(p) = ui::outgoing_send_params(id) {
        Some(p)
    } else if let Some(p) = vehicle::outgoing_send_params(id) {
        Some(p)
    } else {
        world::outgoing_send_params(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raknet::{Datagram, Reliability};
    use crate::rpcs::{OutgoingRpc, RpcPayload};
    use crate::types::Vec3;

    /// docs/re/rpc-ids.md "Client→server RPCs sent by the client", one row per
    /// id with its reliability (shifted enum), hardcoded on purpose. 131 has
    /// two send sites (10/9); the default is the collision hook (9).
    const SEND_TABLE: [(u8, u32); 29] = [
        (23, 9),
        (25, 8),
        (26, 10),
        (50, 8),
        (52, 9),
        (53, 9),
        (62, 9),
        (83, 9),
        (96, 10),
        (97, 10),
        (101, 8),
        (103, 9),
        (106, 8),
        (115, 9),
        (116, 9),
        (117, 9),
        (118, 8),
        (119, 8),
        (128, 8),
        (129, 8),
        (131, 9),
        (132, 8),
        (136, 10),
        (140, 8),
        (154, 10),
        (155, 8),
        (168, 9),
        (177, 9),
        (184, 9),
    ];

    /// Sends the rpc-ids.md table lacks (docs/re/README.md, "Gaps"): 27
    /// (world-entities.md §27) and 181/182 from the dormant variable-ID send
    /// site (session-misc.md). Send sites: docs/re/addresses/samp-proto-rpcs.md.
    const MISSING_FROM_TABLE: [(u8, u32); 3] = [(27, 9), (181, 9), (182, 9)];

    /// C→S RPCs seen in a capture of the original R4 client
    /// (captures/selftest.log, gitignored; summarised in
    /// docs/differential-testing.md): id, reliability, ordering channel (only
    /// on the wire for ordered/sequenced), payload bits.
    const CAPTURED: [(u8, Reliability, Option<u8>, usize); 4] = [
        (25, Reliability::Reliable, None, 616),
        (128, Reliability::Reliable, None, 32),
        (129, Reliability::Reliable, None, 0),
        (52, Reliability::ReliableOrdered, Some(0), 0),
    ];

    /// The PlayerConnect payload of that capture (nick "OrigClient").
    const CAPTURED_PLAYER_CONNECT: &str = "d90f0000010a4f726967436c69656e742e44c1222b3235344646383639443944313439433644353935394346433942363432434237313532463446394241383708302e332e372d52342e44c122";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn v3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// One non-trivial sample per [`Outgoing`] variant.
    fn samples() -> Vec<Outgoing> {
        vec![
            player::PlayerRequestClass { class_index: 3 }.into(),
            player::PlayerRequestSpawn.into(),
            player::PlayerSpawn.into(),
            player::OnPlayerDeath { reason: 255, killer_id: player::INVALID_PLAYER_ID }.into(),
            player::OnPlayerInteriorChange { interior: 3 }.into(),
            player::OnPlayerGiveTakeDamage { taking: true, player_id: 7, damage: 13.2, weapon_id: 24, bodypart: 3 }
                .into(),
            session::PlayerConnect::new(&crate::session::R4, "Bot", 0x1234_5678, "ABCD").into(),
            session::ClientCheckResponse { check_type: 0x48, arg: 0x00BA_6774, result: 0x10 }.into(),
            session::OnPlayerRequestScoresAndPings.into(),
            session::OnPlayerClickPlayer { player_id: 5, source: session::OnPlayerClickPlayer::SOURCE_SCOREBOARD }
                .into(),
            session::OnPlayerClickMap { pos: v3(1.0, -2.5, 3.0) }.into(),
            session::OnPlayerCameraTarget {
                object_id: 1,
                vehicle_id: 2,
                player_id: session::OnPlayerCameraTarget::NONE,
                actor_id: 4,
            }
            .into(),
            session::PickedUpDroppedWeapon { owner: 9 }.into(),
            ui::PlayerRequestChatMessage { message: b"hello".to_vec() }.into(),
            ui::PlayerRequestCommandMessage { command: b"/help me".to_vec() }.into(),
            ui::OnPlayerSelectTextDraw { textdraw_id: 12 }.into(),
            ui::OnPlayerDialogResponse { dialog_id: 3, response: 1, list_item: 2, text: b"row".to_vec() }.into(),
            ui::OnPlayerSelectedMenuRow { row: 4 }.into(),
            ui::OnPlayerExitedMenu.into(),
            vehicle::OnPlayerEnterVehicle { vehicle_id: 42, passenger: 1 }.into(),
            vehicle::OnPlayerExitVehicle { vehicle_id: 42 }.into(),
            vehicle::SetVehicleDamageStatus { vehicle_id: 42, panels: 0x1111, doors: 0x2222, lights: 5, tyres: 0b1010 }
                .into(),
            vehicle::VehicleDeath { vehicle_id: 42 }.into(),
            vehicle::OnPlayerScmEvent { vehicle_id: 42, arg1: 1010, arg2: 0, event_type: 2 }.into(),
            world::OnPlayerSelectObject { select_type: 0, object_id: 9, model: 1337, position: v3(1.0, 2.0, 3.0) }
                .into(),
            world::OnPlayerEditObject {
                player_object: true,
                object_id: 9,
                response: world::OnPlayerEditObject::RESPONSE_UPDATE,
                position: v3(1.0, 2.0, 3.0),
                rotation: v3(0.0, 90.0, 180.0),
            }
            .into(),
            world::OnPlayerEditAttachedObject {
                response: world::OnPlayerEditAttachedObject::RESPONSE_FINAL,
                index: 2,
                data: Some(world::AttachedObjectData { model: 19_000, bone: 2, ..Default::default() }),
            }
            .into(),
            world::OnPlayerPickUpPickup { pickup_id: 17 }.into(),
            world::OnPlayerDamageActor { unknown: false, actor_id: 3, damage: 4.5, weapon: 24, body_part: 9 }.into(),
            world::RequestDff { checksum: 0xDEAD_BEEF }.into(),
            world::RequestTxd { checksum: 0xCAFE_BABE }.into(),
            world::FinishDownload.into(),
        ]
    }

    fn sorted(mut v: Vec<u8>) -> Vec<u8> {
        v.sort_unstable();
        v
    }

    #[test]
    fn every_sent_id_is_covered_exactly_once() {
        for (id, _) in SEND_TABLE.iter().chain(&MISSING_FROM_TABLE) {
            let owners: Vec<&str> =
                OUTGOING_IDS_BY_MODULE.iter().filter(|(_, ids)| ids.contains(id)).map(|(m, _)| *m).collect();
            assert_eq!(owners.len(), 1, "id {id} owned by {owners:?}");
            let n = outgoing_ids().filter(|i| i == id).count();
            assert_eq!(n, 1, "id {id} appears {n} times");
        }
        // Nothing beyond the documented sends.
        let documented = sorted(SEND_TABLE.iter().chain(&MISSING_FROM_TABLE).map(|(id, _)| *id).collect());
        assert_eq!(sorted(outgoing_ids().collect()), documented);
        assert_eq!(documented.len(), 32);
    }

    #[test]
    fn send_params_match_the_spec_table() {
        for (id, rel) in SEND_TABLE.iter().chain(&MISSING_FROM_TABLE) {
            let p = send_params(*id).unwrap_or_else(|| panic!("id {id}"));
            assert_eq!(p.reliability as u32, *rel, "id {id}");
            assert_eq!((p.priority, p.channel), (1, 0), "id {id}");
        }
        for id in [0u8, 24, 93, 139, 200, 207, 255] {
            assert_eq!(send_params(id), None, "id {id}");
        }
        // 131's second send site (type-14 pickup poll) is RELIABLE_SEQUENCED.
        let poll = world::OnPlayerPickUpPickup::POLL_SEND_PARAMS;
        assert_eq!((poll.reliability, poll.channel, poll.priority), (Reliability::ReliableSequenced, 0, 1));
        assert_eq!(world::OnPlayerPickUpPickup::RELIABILITY, Reliability::ReliableOrdered);
    }

    #[test]
    fn samples_cover_every_variant_once() {
        let ids = samples().iter().map(Outgoing::id).collect();
        assert_eq!(sorted(ids), sorted(outgoing_ids().collect()));
    }

    #[test]
    fn outgoing_round_trips_through_rpc_framing() {
        for m in samples() {
            let id = m.id();
            let msg = m.to_message();
            assert_eq!(msg.send_params(), m.send_params(), "id {id}");
            assert_eq!(Some(m.send_params()), send_params(id), "id {id}");
            let rpc = crate::rpc::decode(msg.bits.as_bytes(), msg.bits.bit_len()).unwrap();
            assert_eq!(rpc.id, id);
            // Body bits = exactly what `write` produced.
            let mut body = BitWriter::new();
            m.write(&mut body);
            assert_eq!(rpc.bit_len as usize, body.bit_len(), "id {id}");
            let mut r = rpc.reader();
            assert_eq!(decode_outgoing(id, &mut r).as_ref(), Some(&m), "id {id}");
            assert_eq!(r.remaining(), 0, "id {id}: reader left bits");
            assert_eq!(Outgoing::decode(id, &mut rpc.reader()).as_ref(), Some(&m));
        }
    }

    #[test]
    fn s2c_only_ids_do_not_decode_as_outgoing() {
        let zeros = [0u8; 512];
        for id in [11u8, 12, 24, 44, 61, 93, 137, 138, 139, 164] {
            assert_eq!(decode_outgoing(id, &mut BitReader::new(&zeros)), None, "id {id}");
        }
    }

    #[test]
    fn typed_send_params_match_the_capture() {
        assert_eq!(<session::PlayerConnect as OutgoingRpc>::RELIABILITY, Reliability::Reliable);
        assert_eq!(<player::PlayerRequestClass as OutgoingRpc>::RELIABILITY, Reliability::Reliable);
        assert_eq!(<player::PlayerRequestSpawn as OutgoingRpc>::RELIABILITY, Reliability::Reliable);
        assert_eq!(<player::PlayerSpawn as OutgoingRpc>::RELIABILITY, Reliability::ReliableOrdered);
        assert_eq!(<player::PlayerSpawn as OutgoingRpc>::CHANNEL, 0);
        assert_eq!(<player::PlayerSpawn as OutgoingRpc>::PRIORITY, 1);
    }

    /// Queue each captured RPC on a [`crate::connection::Connection`] via
    /// `send_bits(bits, reliability, channel)` and check what lands on the wire
    /// against the capture of the original client.
    #[test]
    fn captured_sends_match_on_the_wire() {
        let by_id = |id: u8| samples().into_iter().find(|m| m.id() == id).unwrap();
        let now = std::time::Instant::now();
        for (id, rel, channel, bits) in CAPTURED {
            let m = if id == 25 {
                let wire = hex(CAPTURED_PLAYER_CONNECT);
                Outgoing::from(session::PlayerConnect::read(&mut BitReader::new(&wire)).unwrap())
            } else {
                by_id(id)
            };
            let msg = m.to_message();
            assert_eq!(msg.reliability, rel, "id {id}");
            let mut conn = crate::connection::Connection::new(now);
            conn.send_bits(&msg.bits, msg.reliability, msg.channel);
            let out = conn.poll(now);
            assert_eq!(out.len(), 1);
            let dg = Datagram::decode(&out[0]).unwrap();
            assert_eq!(dg.messages.len(), 1);
            let wire = &dg.messages[0];
            assert_eq!(wire.reliability, rel, "id {id}");
            assert_eq!(wire.ordering.map(|o| o.channel), channel, "id {id}");
            let rpc = crate::rpc::decode(&wire.data, wire.bit_len.into()).unwrap();
            assert_eq!((rpc.id, rpc.bit_len as usize), (id, bits), "id {id}");
        }
    }

    #[test]
    fn player_connect_matches_capture_and_builder() {
        let wire = hex(CAPTURED_PLAYER_CONNECT);
        let p = session::PlayerConnect::read(&mut BitReader::new(&wire)).unwrap();
        assert_eq!(p.netgame_version, 4057);
        assert_eq!(p.mod_flag, 1);
        assert_eq!(p.nickname, b"OrigClient");
        assert_eq!(p.version_string, b"0.3.7-R4");
        assert_eq!(p.serial.len(), 43);
        assert_eq!(p.trailing_response, p.challenge_response);
        let mut w = BitWriter::new();
        p.write(&mut w);
        assert_eq!((w.as_bytes(), w.bit_len()), (&wire[..], 616));

        // Same bits as the untyped builder the clients use today.
        let typed = session::PlayerConnect::new(&crate::session::R4, "Bot", 0x1234_5678, "ABCD");
        let mut w = BitWriter::new();
        typed.write(&mut w);
        let built = crate::session::build_player_connect(&crate::session::R4, "Bot", 0x1234_5678, "ABCD");
        assert_eq!((w.as_bytes(), w.bit_len()), (built.as_bytes(), built.bit_len()));
    }

    #[test]
    fn typed_to_message_equals_enum_to_message() {
        let typed = player::PlayerRequestClass { class_index: 3 }.to_message();
        let wrapped = Outgoing::from(player::PlayerRequestClass { class_index: 3 }).to_message();
        assert_eq!(typed.bits.as_bytes(), wrapped.bits.as_bytes());
        assert_eq!(typed.send_params(), wrapped.send_params());
        let polled = world::OnPlayerPickUpPickup { pickup_id: 1 }
            .to_message()
            .with_params(world::OnPlayerPickUpPickup::POLL_SEND_PARAMS);
        assert_eq!(polled.reliability, Reliability::ReliableSequenced);
    }
}
