//! Session-level and miscellaneous RPCs: server restart (40), ClientCheck
//! (103, layout only), scoreboard (155, 23), map click (119), camera target
//! (168) and the registered RPCs that open.mp has no struct for (64, 92, 97,
//! 98, 102, 125, 150, 151, 161).
//!
//! Spec: docs/re/subsystems/session-misc.md. RPCs 139 and 130 live in
//! [`crate::session`]; 25 has a typed form here ([`PlayerConnect`]) next to
//! [`crate::session::build_player_connect`]; 54 (NPCConnect) is never used by
//! a player client.
//!
//! All client→server RPCs here are sent with priority 1 on ordering channel 0
//! without a timestamp; the reliability of each is noted on its struct (shifted
//! enum: 8 RELIABLE, 9 RELIABLE_ORDERED, 10 RELIABLE_SEQUENCED).

use super::RpcPayload;
use crate::bitstream::{BitReader, BitWriter};
use crate::types::Vec3;

// ---------------------------------------------------------------------------
// Connection / session control
// ---------------------------------------------------------------------------

/// RPC 40 PlayerClose, S→C: "server restarting". Empty payload.
///
/// Spec: session-misc.md "40 PlayerClose". The client tears down every remote
/// player and recreates all entity pools, restores the server-settings
/// defaults (name-tag distance 60.0, time 12:00, weather 1, gravity 0.008,
/// name-tag LOS on, enter/exits enabled, zone names off, ...), prints
/// "The server is restarting.." and enters netgame state 11 *without*
/// disconnecting: it waits for the server to re-send RPC 139 (InitGame),
/// which brings it back to the connected state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerClose;

impl RpcPayload for PlayerClose {
    const ID: u8 = 40;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

/// RPC 103 ClientCheck request, S→C: `u8 type, u32 arg, u16 offset, u16 count`.
///
/// Spec: session-misc.md "103 ClientCheck" (anti-cheat; layout only — how the
/// client computes answers is deliberately not implemented here). Matches
/// open.mp's write order. Client limits: the request is dropped
/// without reply when `offset > 256`, `count > 256` or `count < 2`, and only
/// type codes 0x02, 0x05, 0x45, 0x46, 0x47 and 0x48 are answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientCheckRequest {
    /// Check kind.
    pub check_type: u8,
    /// Address / model id / parameter; meaning depends on `check_type`.
    pub arg: u32,
    pub offset: u16,
    pub count: u16,
}

impl RpcPayload for ClientCheckRequest {
    const ID: u8 = 103;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { check_type: r.read_u8()?, arg: r.read_u32()?, offset: r.read_u16()?, count: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.check_type);
        w.write_u32(self.arg);
        w.write_u16(self.offset);
        w.write_u16(self.count);
    }
}

/// RPC 103 ClientCheck reply, C→S: `u8 type, u32 arg, u8 result`.
///
/// Spec: session-misc.md "103 ClientCheck" (layout only). Reliability 9.
/// `check_type` echoes the request; `arg` usually echoes the request's arg.
/// Matches open.mp's read order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientCheckResponse {
    pub check_type: u8,
    pub arg: u32,
    pub result: u8,
}

impl RpcPayload for ClientCheckResponse {
    const ID: u8 = 103;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { check_type: r.read_u8()?, arg: r.read_u32()?, result: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.check_type);
        w.write_u32(self.arg);
        w.write_u8(self.result);
    }
}

// ---------------------------------------------------------------------------
// Scoreboard, map click, camera target
// ---------------------------------------------------------------------------

/// RPC 155 OnPlayerRequestScoresAndPings, C→S. Empty payload.
///
/// Spec: session-misc.md "155". Reliability 8. The client sends it while the
/// scoreboard is open, at most once every 3000 ms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerRequestScoresAndPings;

impl RpcPayload for OnPlayerRequestScoresAndPings {
    const ID: u8 = 155;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

/// One record of [`SendPlayerScoresAndPings`] (10 bytes on the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerScoreAndPing {
    pub player_id: u16,
    pub score: i32,
    pub ping: u32,
}

/// RPC 155 SendPlayerScoresAndPings, S→C: repeated `u16 id, i32 score, u32 ping`.
///
/// Spec: session-misc.md "155" (also remote-players-sync.md). There is no
/// count field: the client processes `floor(floor(payload_bits / 8) / 10)`
/// records and ignores any trailing bits. Records with `player_id >= 1004`
/// are ignored by the client; the local player's id updates the local
/// score/ping. Matches open.mp's write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendPlayerScoresAndPings {
    pub players: Vec<PlayerScoreAndPing>,
}

impl SendPlayerScoresAndPings {
    /// Wire size of one record in bytes.
    pub const RECORD_BYTES: usize = 10;
}

impl RpcPayload for SendPlayerScoresAndPings {
    const ID: u8 = 155;
    fn read(r: &mut BitReader) -> Option<Self> {
        let count = r.remaining() / 8 / Self::RECORD_BYTES;
        let mut players = Vec::with_capacity(count);
        for _ in 0..count {
            players.push(PlayerScoreAndPing {
                player_id: r.read_u16()?,
                score: r.read_u32()? as i32,
                ping: r.read_u32()?,
            });
        }
        Some(Self { players })
    }
    fn write(&self, w: &mut BitWriter) {
        for p in &self.players {
            w.write_u16(p.player_id);
            w.write_u32(p.score as u32);
            w.write_u32(p.ping);
        }
    }
}

/// RPC 23 OnPlayerClickPlayer, C→S: `u16 player id, u8 source`.
///
/// Spec: session-misc.md "23" (also player-state.md). Reliability 9. The
/// original client only sends it from the scoreboard, so `source` is always
/// [`Self::SOURCE_SCOREBOARD`]; it closes the scoreboard after sending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerClickPlayer {
    pub player_id: u16,
    pub source: u8,
}

impl OnPlayerClickPlayer {
    pub const SOURCE_SCOREBOARD: u8 = 0;
}

impl RpcPayload for OnPlayerClickPlayer {
    const ID: u8 = 23;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, source: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u8(self.source);
    }
}

/// RPC 119 OnPlayerClickMap, C→S: `f32 x, f32 y, f32 z`.
///
/// Spec: session-misc.md "119" (also player-state.md). Reliability 8. Sent on
/// a right-click on the pause-menu map; x/y come from the player-placed
/// waypoint and z is GTA's ground height at (x, y) plus 2.0. If no waypoint
/// exists the original client sends uninitialised x/y; a client should simply
/// not send in that case.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OnPlayerClickMap {
    pub pos: Vec3,
}

impl RpcPayload for OnPlayerClickMap {
    const ID: u8 = 119;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { pos: Vec3::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        self.pos.write(w);
    }
}

/// RPC 168 OnPlayerCameraTarget, C→S: four u16 ids (object, vehicle, player,
/// actor), each [`Self::NONE`] when nothing is targeted.
///
/// Spec: session-misc.md "168" (also player-state.md). Reliability 9. Only
/// active once the server enabled it (RPC 170); the client checks at most
/// every 100 ms (object/vehicle within 40.0, player/actor within 20.0 along
/// the camera direction) and sends only when one of the ids changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnPlayerCameraTarget {
    pub object_id: u16,
    pub vehicle_id: u16,
    pub player_id: u16,
    pub actor_id: u16,
}

impl OnPlayerCameraTarget {
    pub const NONE: u16 = 0xFFFF;
}

impl Default for OnPlayerCameraTarget {
    /// Nothing targeted.
    fn default() -> Self {
        Self { object_id: Self::NONE, vehicle_id: Self::NONE, player_id: Self::NONE, actor_id: Self::NONE }
    }
}

impl RpcPayload for OnPlayerCameraTarget {
    const ID: u8 = 168;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            object_id: r.read_u16()?,
            vehicle_id: r.read_u16()?,
            player_id: r.read_u16()?,
            actor_id: r.read_u16()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.object_id);
        w.write_u16(self.vehicle_id);
        w.write_u16(self.player_id);
        w.write_u16(self.actor_id);
    }
}

// ---------------------------------------------------------------------------
// Registered RPCs without an open.mp struct (names are ours)
// ---------------------------------------------------------------------------

/// RPC 64, S→C: one u16 that the client reads and discards (no-op).
///
/// Spec: session-misc.md "64 (S→C) — no-op". Accept and ignore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unused64 {
    pub value: u16,
}

impl RpcPayload for Unused64 {
    const ID: u8 = 64;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { value: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.value);
    }
}

/// RPC 125, S→C: no payload; the client handler does nothing.
///
/// Spec: session-misc.md "125 (S→C) — no-op".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unused125;

impl RpcPayload for Unused125 {
    const ID: u8 = 125;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

/// RPC 92, S→C: `u32 value`, applied as GTA script opcode 0x052C
/// (local player, value).
///
/// Spec: session-misc.md "92". Ignored if the local player object does not
/// exist. Commonly called "set player drunk visuals"; that meaning is
/// unverified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetPlayerDrunkVisuals {
    pub value: u32,
}

impl RpcPayload for SetPlayerDrunkVisuals {
    const ID: u8 = 92;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { value: r.read_u32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.value);
    }
}

/// RPC 150, S→C: `u32 value`, applied as GTA script opcode 0x03FD
/// (local player, value).
///
/// Spec: session-misc.md "150". Ignored if the local player object does not
/// exist. Commonly called "set player drunk handling"; that meaning is
/// unverified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetPlayerDrunkHandling {
    pub value: u32,
}

impl RpcPayload for SetPlayerDrunkHandling {
    const ID: u8 = 150;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { value: r.read_u32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.value);
    }
}

/// RPC 98, S→C: vehicle tyre status, `u16 vehicle id, u8 tyre bits`.
///
/// Spec: session-misc.md "98". One bit per tyre, 1 = burst. The client
/// ignores ids ≥ 2000 or vehicles that do not exist. For cars bits 0..3 map
/// to the four wheels (inferred rear-right, front-right, rear-left,
/// front-left — the same order as the tyre field of vehicle damage status);
/// for bikes bit 0 is the rear and bit 1 the front wheel (inferred); other
/// vehicle classes ignore it. Bits 4..7 are unused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetVehicleTyreStatus {
    pub vehicle_id: u16,
    pub tyres: u8,
}

impl RpcPayload for SetVehicleTyreStatus {
    const ID: u8 = 98;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, tyres: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.tyres);
    }
}

/// RPC 102, S→C: a 296-byte opaque blob (inferred: the server's 32-bit
/// RakNet statistics structure).
///
/// Spec: session-misc.md "102". The client copies the bytes into a global
/// buffer and no reader of that buffer was found; keep or ignore it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerNetStats {
    pub data: Box<[u8; ServerNetStats::LEN]>,
}

impl ServerNetStats {
    /// Payload size in bytes.
    pub const LEN: usize = 296;
}

impl Default for ServerNetStats {
    fn default() -> Self {
        Self { data: Box::new([0; Self::LEN]) }
    }
}

impl RpcPayload for ServerNetStats {
    const ID: u8 = 102;
    fn read(r: &mut BitReader) -> Option<Self> {
        let mut data = Box::new([0; Self::LEN]);
        r.read_bytes_into(&mut data[..])?;
        Some(Self { data })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_bytes(&self.data[..]);
    }
}

/// RPC 151, S→C: destroy the dropped-weapon pickups owned by `owner` (u8).
///
/// Spec: session-misc.md "151". The client compares the zero-extended 8-bit
/// owner with the 16-bit owner field of each dropped-weapon pickup slot, but
/// in 0.3.7-R4 nothing ever creates such slots, so this is effectively a
/// no-op. Note the owner is only 8 bits wide here, while RPC 97 uses 16.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DestroyDroppedWeaponPickups {
    pub owner: u8,
}

impl RpcPayload for DestroyDroppedWeaponPickups {
    const ID: u8 = 151;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { owner: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.owner);
    }
}

/// RPC 161, S→C: old 3-field vehicle params for player,
/// `u16 vehicle id, u8 objective, u8 doors locked`.
///
/// Spec: session-misc.md "161". open.mp never sends it (it uses RPC 24).
/// Client behaviour: ignored unless the id is < 2000 and the vehicle exists;
/// a non-zero `objective` shows the objective marker, but zero does *not*
/// clear an existing one; `doors_locked` non-zero locks, zero unlocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetVehicleParamsForPlayerOld {
    pub vehicle_id: u16,
    pub objective: u8,
    pub doors_locked: u8,
}

impl RpcPayload for SetVehicleParamsForPlayerOld {
    const ID: u8 = 161;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, objective: r.read_u8()?, doors_locked: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.objective);
        w.write_u8(self.doors_locked);
    }
}

/// RPC 97, C→S: picked up a dropped weapon, `u16 owner`.
///
/// Spec: session-misc.md "97". Reliability 10. `owner` is the owner field of
/// the collected dropped-weapon pickup. Since 0.3.7-R4 never creates
/// dropped-weapon pickups (see [`DestroyDroppedWeaponPickups`]), the original
/// client never actually sends this; a client may omit it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickedUpDroppedWeapon {
    pub owner: u16,
}

impl RpcPayload for PickedUpDroppedWeapon {
    const ID: u8 = 97;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { owner: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.owner);
    }
}

// ---------------------------------------------------------------------------
// Join request (typed form of crate::session::build_player_connect)
// ---------------------------------------------------------------------------

/// RPC 25 PlayerConnect (C→S): the join request.
///
/// Spec: handshake.md §5, session-misc.md "25 PlayerConnect (C→S)". Wire:
/// `u32 netgame version, u8 mod flag, u8-len nickname, u32 challenge response,
/// u8-len serial, u8-len client version string, u32 challenge response again`.
/// The R4 client always sends the trailing copy (open.mp treats it as
/// optional and uses it to classify the peer as an official client); this
/// type always writes and requires it. Same bits as
/// [`crate::session::build_player_connect`]; [`PlayerConnect::new`] builds the
/// identical payload from a [`crate::session::VersionProfile`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerConnect {
    /// 4057 for 0.3.7-R4.
    pub netgame_version: u32,
    pub mod_flag: u8,
    pub nickname: Vec<u8>,
    /// `challenge ^ netgame_version` (challenge from `0x22`).
    pub challenge_response: u32,
    pub serial: Vec<u8>,
    /// e.g. `0.3.7-R4`.
    pub version_string: Vec<u8>,
    /// Second copy of the challenge response.
    pub trailing_response: u32,
}

impl PlayerConnect {
    /// The payload the original client sends for `profile`.
    pub fn new(profile: &crate::session::VersionProfile, nickname: &str, challenge: u32, serial: &str) -> Self {
        let response = challenge ^ profile.netgame_version;
        Self {
            netgame_version: profile.netgame_version,
            mod_flag: profile.mod_flag,
            nickname: nickname.as_bytes().to_vec(),
            challenge_response: response,
            serial: serial.as_bytes().to_vec(),
            version_string: profile.version_string.as_bytes().to_vec(),
            trailing_response: response,
        }
    }
}

impl RpcPayload for PlayerConnect {
    const ID: u8 = crate::rpc::ids::PLAYER_CONNECT;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            netgame_version: r.read_u32()?,
            mod_flag: r.read_u8()?,
            nickname: crate::types::read_str8(r)?,
            challenge_response: r.read_u32()?,
            serial: crate::types::read_str8(r)?,
            version_string: crate::types::read_str8(r)?,
            trailing_response: r.read_u32()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.netgame_version);
        w.write_u8(self.mod_flag);
        crate::types::write_str8(w, &self.nickname);
        w.write_u32(self.challenge_response);
        crate::types::write_str8(w, &self.serial);
        crate::types::write_str8(w, &self.version_string);
        w.write_u32(self.trailing_response);
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// S→C payloads of this module.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    PlayerClose(PlayerClose),
    ClientCheckRequest(ClientCheckRequest),
    SendPlayerScoresAndPings(SendPlayerScoresAndPings),
    Unused64(Unused64),
    Unused125(Unused125),
    SetPlayerDrunkVisuals(SetPlayerDrunkVisuals),
    SetPlayerDrunkHandling(SetPlayerDrunkHandling),
    SetVehicleTyreStatus(SetVehicleTyreStatus),
    ServerNetStats(ServerNetStats),
    DestroyDroppedWeaponPickups(DestroyDroppedWeaponPickups),
    SetVehicleParamsForPlayerOld(SetVehicleParamsForPlayerOld),
}

/// RPC ids decoded by [`decode`].
pub const INCOMING_IDS: &[u8] = &[
    PlayerClose::ID,
    Unused64::ID,
    SetPlayerDrunkVisuals::ID,
    SetVehicleTyreStatus::ID,
    ServerNetStats::ID,
    ClientCheckRequest::ID,
    Unused125::ID,
    SetPlayerDrunkHandling::ID,
    DestroyDroppedWeaponPickups::ID,
    SendPlayerScoresAndPings::ID,
    SetVehicleParamsForPlayerOld::ID,
];

/// Decode a server→client RPC payload of this module; `None` for ids outside
/// the module or truncated data.
pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
    Some(match id {
        PlayerClose::ID => Incoming::PlayerClose(PlayerClose::read(r)?),
        Unused64::ID => Incoming::Unused64(Unused64::read(r)?),
        SetPlayerDrunkVisuals::ID => Incoming::SetPlayerDrunkVisuals(SetPlayerDrunkVisuals::read(r)?),
        SetVehicleTyreStatus::ID => Incoming::SetVehicleTyreStatus(SetVehicleTyreStatus::read(r)?),
        ServerNetStats::ID => Incoming::ServerNetStats(ServerNetStats::read(r)?),
        ClientCheckRequest::ID => Incoming::ClientCheckRequest(ClientCheckRequest::read(r)?),
        Unused125::ID => Incoming::Unused125(Unused125::read(r)?),
        SetPlayerDrunkHandling::ID => Incoming::SetPlayerDrunkHandling(SetPlayerDrunkHandling::read(r)?),
        DestroyDroppedWeaponPickups::ID => Incoming::DestroyDroppedWeaponPickups(DestroyDroppedWeaponPickups::read(r)?),
        SendPlayerScoresAndPings::ID => Incoming::SendPlayerScoresAndPings(SendPlayerScoresAndPings::read(r)?),
        SetVehicleParamsForPlayerOld::ID => {
            Incoming::SetVehicleParamsForPlayerOld(SetVehicleParamsForPlayerOld::read(r)?)
        }
        _ => return None,
    })
}

impl Incoming {
    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::PlayerClose(_) => PlayerClose::ID,
            Self::ClientCheckRequest(_) => ClientCheckRequest::ID,
            Self::SendPlayerScoresAndPings(_) => SendPlayerScoresAndPings::ID,
            Self::Unused64(_) => Unused64::ID,
            Self::Unused125(_) => Unused125::ID,
            Self::SetPlayerDrunkVisuals(_) => SetPlayerDrunkVisuals::ID,
            Self::SetPlayerDrunkHandling(_) => SetPlayerDrunkHandling::ID,
            Self::SetVehicleTyreStatus(_) => SetVehicleTyreStatus::ID,
            Self::ServerNetStats(_) => ServerNetStats::ID,
            Self::DestroyDroppedWeaponPickups(_) => DestroyDroppedWeaponPickups::ID,
            Self::SetVehicleParamsForPlayerOld(_) => SetVehicleParamsForPlayerOld::ID,
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::PlayerClose(p) => p.write(w),
            Self::ClientCheckRequest(p) => p.write(w),
            Self::SendPlayerScoresAndPings(p) => p.write(w),
            Self::Unused64(p) => p.write(w),
            Self::Unused125(p) => p.write(w),
            Self::SetPlayerDrunkVisuals(p) => p.write(w),
            Self::SetPlayerDrunkHandling(p) => p.write(w),
            Self::SetVehicleTyreStatus(p) => p.write(w),
            Self::ServerNetStats(p) => p.write(w),
            Self::DestroyDroppedWeaponPickups(p) => p.write(w),
            Self::SetVehicleParamsForPlayerOld(p) => p.write(w),
        }
    }
}

// ---------------------------------------------------------------------------
// Outgoing (C→S) send parameters
// ---------------------------------------------------------------------------

outgoing! {
    Session;
    /// RPC 25: reliability 8 (RELIABLE), priority 1, channel 0
    /// (session-misc.md "25 PlayerConnect (C→S)"). Mirrors samp.dll 0x1000ad80.
    PlayerConnect => Reliable;
    /// RPC 103 reply: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (session-misc.md "103 ClientCheck"). Mirrors samp.dll 0x10011990.
    ClientCheckResponse => ReliableOrdered;
    /// RPC 155: reliability 8 (RELIABLE), priority 1, channel 0
    /// (session-misc.md "155", remote-players-sync.md). Mirrors samp.dll
    /// 0x10008f10.
    OnPlayerRequestScoresAndPings => Reliable;
    /// RPC 23: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (session-misc.md "23", player-state.md). Mirrors samp.dll 0x1006ec10.
    OnPlayerClickPlayer => ReliableOrdered;
    /// RPC 119: reliability 8 (RELIABLE), priority 1, channel 0
    /// (session-misc.md "119", player-state.md). The send site lies inside
    /// samp.dll 0x10075790.
    OnPlayerClickMap => Reliable;
    /// RPC 168: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (session-misc.md "168", player-state.md). Mirrors samp.dll 0x10004700.
    OnPlayerCameraTarget => ReliableOrdered;
    /// RPC 97: reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0
    /// (session-misc.md "97"). The original client never reaches this send in
    /// practice (see [`PickedUpDroppedWeapon`]). Mirrors samp.dll 0x10013520.
    PickedUpDroppedWeapon => ReliableSequenced;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;

    /// Write `v`, then read it back both aligned and after a 1-bit offset.
    fn roundtrip<T: RpcPayload + PartialEq + Debug>(v: &T) -> Vec<u8> {
        let mut w = BitWriter::new();
        v.write(&mut w);
        let bits = w.bit_len();
        let mut r = BitReader::with_bit_len(w.as_bytes(), bits);
        assert_eq!(T::read(&mut r).as_ref(), Some(v));
        assert_eq!(r.remaining(), 0, "reader left bits");

        let mut w2 = BitWriter::new();
        w2.write_bit(true);
        v.write(&mut w2);
        let mut r2 = BitReader::with_bit_len(w2.as_bytes(), w2.bit_len());
        assert_eq!(r2.read_bit(), Some(true));
        assert_eq!(T::read(&mut r2).as_ref(), Some(v));
        assert_eq!(r2.remaining(), 0);
        w.into_bytes()
    }

    /// Encode as a full RPC message, frame-decode it and run the module decoder.
    fn via_rpc<T: RpcPayload>(v: &T) -> Option<Incoming> {
        let msg = v.to_rpc();
        let rpc = crate::rpc::decode(msg.as_bytes(), msg.bit_len()).unwrap();
        assert_eq!(rpc.id, T::ID);
        decode(rpc.id, &mut rpc.reader())
    }

    #[test]
    fn player_close_empty() {
        assert!(roundtrip(&PlayerClose).is_empty());
        assert_eq!(via_rpc(&PlayerClose), Some(Incoming::PlayerClose(PlayerClose)));
        // Trailing data is not read.
        let mut r = BitReader::new(&[0xAA]);
        assert_eq!(decode(40, &mut r), Some(Incoming::PlayerClose(PlayerClose)));
        assert_eq!(r.remaining(), 8);
    }

    #[test]
    fn client_check_request_bytes() {
        // type 0x48, arg 0x00BA6774, offset 0x0010, count 0x0004
        let wire = [0x48, 0x74, 0x67, 0xBA, 0x00, 0x10, 0x00, 0x04, 0x00];
        let v = ClientCheckRequest { check_type: 0x48, arg: 0x00BA_6774, offset: 0x10, count: 4 };
        let mut r = BitReader::new(&wire);
        assert_eq!(decode(103, &mut r), Some(Incoming::ClientCheckRequest(v)));
        assert_eq!(r.remaining(), 0);
        assert_eq!(roundtrip(&v), wire);
        assert_eq!(via_rpc(&v), Some(Incoming::ClientCheckRequest(v)));
        assert_eq!(decode(103, &mut BitReader::new(&wire[..8])), None);
    }

    #[test]
    fn client_check_response_bytes() {
        let v = ClientCheckResponse { check_type: 0x05, arg: 0x1234_5678, result: 0xC3 };
        assert_eq!(roundtrip(&v), [0x05, 0x78, 0x56, 0x34, 0x12, 0xC3]);
        assert_eq!(ClientCheckResponse::ID, 103);
    }

    #[test]
    fn request_scores_empty() {
        assert!(roundtrip(&OnPlayerRequestScoresAndPings).is_empty());
        assert_eq!(OnPlayerRequestScoresAndPings::ID, 155);
    }

    #[test]
    fn scores_and_pings_bytes() {
        #[rustfmt::skip]
        let wire = [
            0x03, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x2A, 0x00, 0x00, 0x00, // id 3, score -1, ping 42
            0xEB, 0x03, 0x10, 0x27, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, // id 1003, score 10000, ping 256
        ];
        let v = SendPlayerScoresAndPings {
            players: vec![
                PlayerScoreAndPing { player_id: 3, score: -1, ping: 42 },
                PlayerScoreAndPing { player_id: 1003, score: 10000, ping: 256 },
            ],
        };
        assert_eq!(SendPlayerScoresAndPings::read(&mut BitReader::new(&wire)).as_ref(), Some(&v));
        assert_eq!(roundtrip(&v), wire);
        assert_eq!(via_rpc(&v), Some(Incoming::SendPlayerScoresAndPings(v.clone())));

        // Record count = floor(bytes / 10): a partial trailing record is ignored, not an error.
        let mut long = wire.to_vec();
        long.extend_from_slice(&[0x11; 9]);
        let mut r = BitReader::new(&long);
        assert_eq!(SendPlayerScoresAndPings::read(&mut r), Some(v));
        assert_eq!(r.remaining(), 72);

        // Fewer than 10 bytes → no records.
        let empty = SendPlayerScoresAndPings::read(&mut BitReader::new(&[0; 9])).unwrap();
        assert!(empty.players.is_empty());
    }

    #[test]
    fn click_player_bytes() {
        let v = OnPlayerClickPlayer { player_id: 0x0102, source: OnPlayerClickPlayer::SOURCE_SCOREBOARD };
        assert_eq!(roundtrip(&v), [0x02, 0x01, 0x00]);
    }

    #[test]
    fn click_map_bytes() {
        let v = OnPlayerClickMap { pos: Vec3 { x: 1.0, y: -2.0, z: 0.5 } };
        // 1.0 = 0x3F800000, -2.0 = 0xC0000000, 0.5 = 0x3F000000 (little-endian)
        assert_eq!(roundtrip(&v), [0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x3F]);
    }

    #[test]
    fn camera_target_bytes() {
        let v = OnPlayerCameraTarget { object_id: 0x0001, vehicle_id: 0xFFFF, player_id: 0x0203, actor_id: 0xFFFF };
        assert_eq!(roundtrip(&v), [0x01, 0x00, 0xFF, 0xFF, 0x03, 0x02, 0xFF, 0xFF]);
        assert_eq!(roundtrip(&OnPlayerCameraTarget::default()), [0xFF; 8]);
    }

    #[test]
    fn unused_64_and_125() {
        let mut r = BitReader::new(&[0x34, 0x12]);
        assert_eq!(decode(64, &mut r), Some(Incoming::Unused64(Unused64 { value: 0x1234 })));
        assert_eq!(roundtrip(&Unused64 { value: 0x1234 }), [0x34, 0x12]);
        assert_eq!(decode(64, &mut BitReader::new(&[0x34])), None);

        assert!(roundtrip(&Unused125).is_empty());
        assert_eq!(via_rpc(&Unused125), Some(Incoming::Unused125(Unused125)));
    }

    #[test]
    fn drunk_opcodes_bytes() {
        let wire = [0xD0, 0x07, 0x00, 0x00]; // 2000
        let mut r = BitReader::new(&wire);
        assert_eq!(decode(92, &mut r), Some(Incoming::SetPlayerDrunkVisuals(SetPlayerDrunkVisuals { value: 2000 })));
        let mut r = BitReader::new(&wire);
        assert_eq!(decode(150, &mut r), Some(Incoming::SetPlayerDrunkHandling(SetPlayerDrunkHandling { value: 2000 })));
        assert_eq!(roundtrip(&SetPlayerDrunkVisuals { value: 2000 }), wire);
        assert_eq!(roundtrip(&SetPlayerDrunkHandling { value: 2000 }), wire);
    }

    #[test]
    fn tyre_status_bytes() {
        let wire = [0xCF, 0x07, 0b0000_0101]; // vehicle 1999, tyres 0 and 2 burst
        let v = SetVehicleTyreStatus { vehicle_id: 1999, tyres: 0b101 };
        assert_eq!(decode(98, &mut BitReader::new(&wire)), Some(Incoming::SetVehicleTyreStatus(v)));
        assert_eq!(roundtrip(&v), wire);
    }

    #[test]
    fn server_net_stats() {
        let mut wire = [0u8; 296];
        for (i, b) in wire.iter_mut().enumerate() {
            *b = i as u8;
        }
        let v = ServerNetStats { data: Box::new(wire) };
        let mut r = BitReader::new(&wire);
        assert_eq!(decode(102, &mut r), Some(Incoming::ServerNetStats(v.clone())));
        assert_eq!(r.remaining(), 0);
        assert_eq!(roundtrip(&v), wire);
        assert_eq!(decode(102, &mut BitReader::new(&wire[..295])), None);
        assert_eq!(via_rpc(&ServerNetStats::default()), Some(Incoming::ServerNetStats(ServerNetStats::default())));
    }

    #[test]
    fn destroy_dropped_weapon_pickups_is_8_bit() {
        let v = DestroyDroppedWeaponPickups { owner: 0xAB };
        assert_eq!(roundtrip(&v), [0xAB]);
        let mut r = BitReader::new(&[0xAB, 0xCD]);
        assert_eq!(decode(151, &mut r), Some(Incoming::DestroyDroppedWeaponPickups(v)));
        assert_eq!(r.remaining(), 8);
    }

    #[test]
    fn vehicle_params_old_bytes() {
        let wire = [0x2C, 0x01, 0x01, 0x00]; // vehicle 300, objective on, unlocked
        let v = SetVehicleParamsForPlayerOld { vehicle_id: 300, objective: 1, doors_locked: 0 };
        assert_eq!(decode(161, &mut BitReader::new(&wire)), Some(Incoming::SetVehicleParamsForPlayerOld(v)));
        assert_eq!(roundtrip(&v), wire);
        assert_eq!(decode(161, &mut BitReader::new(&wire[..3])), None);
    }

    #[test]
    fn picked_up_dropped_weapon_bytes() {
        assert_eq!(roundtrip(&PickedUpDroppedWeapon { owner: 0xBEEF }), [0xEF, 0xBE]);
    }

    #[test]
    fn unaligned_bit_exact() {
        // A 1-bit offset shifts every byte: 1 | 0x05 | 0x78 0x56 0x34 0x12 | 0xC3
        let mut w = BitWriter::new();
        w.write_bit(true);
        ClientCheckResponse { check_type: 0x05, arg: 0x1234_5678, result: 0xC3 }.write(&mut w);
        assert_eq!(w.as_bytes(), [0x82, 0xBC, 0x2B, 0x1A, 0x09, 0x61, 0x80]);
    }

    #[test]
    fn incoming_ids_match_decode() {
        let mut ids = INCOMING_IDS.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids, [40, 64, 92, 98, 102, 103, 125, 150, 151, 155, 161]);
        let zeros = [0u8; 400];
        for id in 0..=u8::MAX {
            let decoded = decode(id, &mut BitReader::new(&zeros)).is_some();
            assert_eq!(decoded, INCOMING_IDS.contains(&id), "id {id}");
        }
        // C→S-only ids are not decoded as incoming.
        for id in [23, 97, 119, 168] {
            assert!(!INCOMING_IDS.contains(&id));
        }
    }
}
