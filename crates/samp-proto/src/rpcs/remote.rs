//! Remote-player RPCs: join/quit, stream in/out, name, chat bubble, name-tag
//! visibility and spectating.
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §1 (RPC sections only; the
//! sync packets 200–212 live in `crate::sync`). Every RPC here is S→C. The spec
//! found no wire-layout disagreement between open.mp and the 0.3.7-R4 client
//! for any of them (§3).
//!
//! Player-id bounds used by the client handlers (not enforced by the decoders):
//! most handlers accept `playerId < 1004`; PlayerJoin, PlayerStreamOut and
//! SetPlayerName use `< 1005` (`≤ 1004`).

use super::RpcPayload;
use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Vec3, read_str8, write_str8};

/// Longest player name the client accepts (PlayerJoin, SetPlayerName).
pub const MAX_PLAYER_NAME: usize = 24;
/// Longest chat-bubble text the client accepts (SetPlayerChatBubble).
pub const MAX_CHAT_BUBBLE_TEXT: usize = 144;
/// Number of weapon skill levels carried by PlayerStreamIn.
pub const NUM_SKILL_LEVELS: usize = 11;
/// Fighting style value the client treats as "leave the default".
pub const FIGHTING_STYLE_DEFAULT: u8 = 4;

/// Spectate camera modes as sent by the server (RPC 126 / 127 `mode`).
pub mod spectate_mode {
    /// Follow camera (also the fallback for any unknown value).
    pub const NORMAL: u8 = 1;
    /// Fixed camera.
    pub const FIXED: u8 = 2;
    /// Side camera.
    pub const SIDE: u8 = 3;
}

/// RPC 137 PlayerJoin (S→C): a remote player connected.
///
/// Spec: remote-players-sync.md §1 "137 PlayerJoin". Layout: `u16 playerId,
/// u32 colour (RGBA), u8 isNPC, u8 nameLen, raw[nameLen] name`.
///
/// Client behaviour: the whole RPC is ignored if the name is longer than
/// [`MAX_PLAYER_NAME`]; the pool entry is only created for ids < 1005; a
/// non-zero `is_npc` sets the remote NPC flag; a non-zero colour (id < 1004)
/// sets the player's colour. No chat message is printed. Agrees with open.mp.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerJoin {
    pub player_id: u16,
    /// RGBA.
    pub colour: u32,
    /// Non-zero for NPCs.
    pub is_npc: u8,
    pub name: Vec<u8>,
}

impl RpcPayload for PlayerJoin {
    const ID: u8 = 137;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, colour: r.read_u32()?, is_npc: r.read_u8()?, name: read_str8(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u32(self.colour);
        w.write_u8(self.is_npc);
        write_str8(w, &self.name);
    }
}

/// RPC 138 PlayerQuit (S→C): a remote player left.
///
/// Spec: remote-players-sync.md §1 "138 PlayerQuit". Layout: `u16 playerId,
/// u8 reason`.
///
/// Client behaviour: for ids < 1004 in use, stops spectating that player if the
/// local player was, then destroys the entry. `reason` is read but **unused**
/// by the client (no message is printed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerQuit {
    pub player_id: u16,
    pub reason: u8,
}

impl RpcPayload for PlayerQuit {
    const ID: u8 = 138;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, reason: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u8(self.reason);
    }
}

/// RPC 32 PlayerStreamIn (S→C): create a remote player's ped.
///
/// Spec: remote-players-sync.md §1 "32 PlayerStreamIn". Layout: `u16 playerId,
/// u8 team, u32 skin, 3 × f32 position, f32 facingAngle (degrees), u32 colour
/// (RGBA), u8 fightingStyle, 11 × u16 skillLevels` (the skills are a raw
/// 22-byte copy, equal to eleven unaligned little-endian u16).
///
/// Client behaviour: requires id < 1004 with a remote object. Spawns the ped
/// (prints "Warning: Couldn't spawn player(%u)" on an invalid skin), applies a
/// non-zero colour, a fighting style other than [`FIGHTING_STYLE_DEFAULT`],
/// the skill levels, and the team unless it is 255.
///
/// The 0.3.7 client tolerates a truncated payload (fields it fails to read
/// keep defaults; the fighting style defaults to 4). This decoder is strict
/// and requires every field, which is what open.mp sends. open.mp adds a
/// `u32 CustomSkin` after `skin` only for 0.3.DL clients; a 0.3.7 client must
/// receive this (non-DL) layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerStreamIn {
    pub player_id: u16,
    /// 255 = no team (left unchanged by the client).
    pub team: u8,
    pub skin: u32,
    pub position: Vec3,
    /// Degrees.
    pub facing_angle: f32,
    /// RGBA; 0 leaves the colour unchanged.
    pub colour: u32,
    pub fighting_style: u8,
    pub skill_levels: [u16; NUM_SKILL_LEVELS],
}

impl RpcPayload for PlayerStreamIn {
    const ID: u8 = 32;
    fn read(r: &mut BitReader) -> Option<Self> {
        let player_id = r.read_u16()?;
        let team = r.read_u8()?;
        let skin = r.read_u32()?;
        let position = Vec3::read(r)?;
        let facing_angle = r.read_f32()?;
        let colour = r.read_u32()?;
        let fighting_style = r.read_u8()?;
        let mut skill_levels = [0u16; NUM_SKILL_LEVELS];
        for s in &mut skill_levels {
            *s = r.read_u16()?;
        }
        Some(Self { player_id, team, skin, position, facing_angle, colour, fighting_style, skill_levels })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u8(self.team);
        w.write_u32(self.skin);
        self.position.write(w);
        w.write_f32(self.facing_angle);
        w.write_u32(self.colour);
        w.write_u8(self.fighting_style);
        for &s in &self.skill_levels {
            w.write_u16(s);
        }
    }
}

/// RPC 163 PlayerStreamOut (S→C): delete a remote player's ped.
///
/// Spec: remote-players-sync.md §1 "163 PlayerStreamOut". Layout: `u16 playerId`.
///
/// Client behaviour: for ids < 1005 that are present, deletes the ped, sets the
/// remote state to 0 and **resets name-tag visibility to shown** (undoing a
/// previous RPC 80 with `show = 0`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerStreamOut {
    pub player_id: u16,
}

impl RpcPayload for PlayerStreamOut {
    const ID: u8 = 163;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
    }
}

/// RPC 11 SetPlayerName (S→C): rename a player.
///
/// Spec: remote-players-sync.md §1 "11 SetPlayerName" (also player-state.md).
/// Layout: `u16 playerId, u8 nameLen, raw[nameLen] name, u8 success`; the name
/// is not NUL-terminated on the wire.
///
/// Client behaviour: the whole RPC is ignored if `nameLen` exceeds
/// [`MAX_PLAYER_NAME`]. If `success == 1` and the id is a known remote player
/// (≤ 1004) its name is replaced; if the id is the local player the local name
/// is replaced **regardless of `success`**. Agrees with open.mp.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetPlayerName {
    pub player_id: u16,
    pub name: Vec<u8>,
    pub success: u8,
}

impl RpcPayload for SetPlayerName {
    const ID: u8 = 11;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, name: read_str8(r)?, success: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        write_str8(w, &self.name);
        w.write_u8(self.success);
    }
}

/// RPC 59 SetPlayerChatBubble (S→C): text bubble above a player.
///
/// Spec: remote-players-sync.md §1 "59 SetPlayerChatBubble" and
/// ui-chat-audio.md "59 SetPlayerChatBubble". Layout: `u16 playerId, u32 colour
/// (RGBA), f32 drawDistance, u32 expireTime (ms), u8 textLen, raw[textLen] text`.
///
/// Client behaviour: ignored if `textLen` exceeds [`MAX_CHAT_BUBBLE_TEXT`];
/// accepted for ids < 1004 in use. A new bubble replaces the old one and
/// expires `expire_time` ms after receipt. Agrees with open.mp.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetPlayerChatBubble {
    pub player_id: u16,
    /// RGBA.
    pub colour: u32,
    pub draw_distance: f32,
    /// Lifetime in milliseconds.
    pub expire_time: u32,
    pub text: Vec<u8>,
}

impl RpcPayload for SetPlayerChatBubble {
    const ID: u8 = 59;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            player_id: r.read_u16()?,
            colour: r.read_u32()?,
            draw_distance: r.read_f32()?,
            expire_time: r.read_u32()?,
            text: read_str8(r)?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u32(self.colour);
        w.write_f32(self.draw_distance);
        w.write_u32(self.expire_time);
        write_str8(w, &self.text);
    }
}

/// RPC 80 ShowPlayerNameTagForPlayer (S→C): show/hide one player's name tag.
///
/// Spec: remote-players-sync.md §1 "80 ShowPlayerNameTagForPlayer". Layout:
/// `u16 playerId, u8 show`.
///
/// Client behaviour: for ids < 1004 in use, visibility := `show != 0`. The flag
/// is reset to shown when the player streams out (RPC 163).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShowPlayerNameTagForPlayer {
    pub player_id: u16,
    /// Non-zero = show.
    pub show: u8,
}

impl RpcPayload for ShowPlayerNameTagForPlayer {
    const ID: u8 = 80;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, show: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u8(self.show);
    }
}

/// RPC 124 TogglePlayerSpectating (S→C): enter/leave spectator mode.
///
/// Spec: remote-players-sync.md §1 "124 TogglePlayerSpectating". Layout:
/// `u32 enable`.
///
/// Client behaviour: when leaving spectator mode (`enable == 0` while
/// spectating) the client returns to class selection if that is pending, or
/// otherwise immediately requests a spawn (sends RPC 52). The spectate target
/// is cleared either way. While spectating, the client sends only packet 212
/// (plus 203).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TogglePlayerSpectating {
    /// Stored as-is as the spectating flag; non-zero = spectating.
    pub enable: u32,
}

impl RpcPayload for TogglePlayerSpectating {
    const ID: u8 = 124;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { enable: r.read_u32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.enable);
    }
}

/// RPC 126 PlayerSpectatePlayer (S→C): spectate a player.
///
/// Spec: remote-players-sync.md §1 "126 PlayerSpectatePlayer". Layout:
/// `u16 playerId, u8 mode` ([`spectate_mode`]).
///
/// Client behaviour: the target is accepted only if id < 1004, in use, and the
/// remote state is neither none (0) nor wasted (32); the camera is attached on
/// the next spectator tick. See [`PlayerSpectatePlayer::gta_camera_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSpectatePlayer {
    pub player_id: u16,
    pub mode: u8,
}

impl PlayerSpectatePlayer {
    /// GTA camera mode the client derives from `mode`: FIXED → 15, SIDE → 14,
    /// anything else → 4 (follow ped).
    pub fn gta_camera_mode(&self) -> u8 {
        match self.mode {
            spectate_mode::FIXED => 15,
            spectate_mode::SIDE => 14,
            _ => 4,
        }
    }
}

impl RpcPayload for PlayerSpectatePlayer {
    const ID: u8 = 126;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, mode: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u8(self.mode);
    }
}

/// RPC 127 PlayerSpectateVehicle (S→C): spectate a vehicle.
///
/// Spec: remote-players-sync.md §1 "127 PlayerSpectateVehicle". Layout:
/// `u16 vehicleId, u8 mode` ([`spectate_mode`]).
///
/// Client behaviour: accepted if `vehicleId < 2000` and the vehicle exists.
/// See [`PlayerSpectateVehicle::gta_camera_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSpectateVehicle {
    pub vehicle_id: u16,
    pub mode: u8,
}

impl PlayerSpectateVehicle {
    /// GTA camera mode the client derives from `mode`: FIXED → 15, SIDE → 14,
    /// anything else → 3 (behind car).
    pub fn gta_camera_mode(&self) -> u8 {
        match self.mode {
            spectate_mode::FIXED => 15,
            spectate_mode::SIDE => 14,
            _ => 3,
        }
    }
}

impl RpcPayload for PlayerSpectateVehicle {
    const ID: u8 = 127;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, mode: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.mode);
    }
}

/// S→C payloads of this module.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    PlayerJoin(PlayerJoin),
    PlayerQuit(PlayerQuit),
    PlayerStreamIn(PlayerStreamIn),
    PlayerStreamOut(PlayerStreamOut),
    SetPlayerName(SetPlayerName),
    SetPlayerChatBubble(SetPlayerChatBubble),
    ShowPlayerNameTagForPlayer(ShowPlayerNameTagForPlayer),
    TogglePlayerSpectating(TogglePlayerSpectating),
    PlayerSpectatePlayer(PlayerSpectatePlayer),
    PlayerSpectateVehicle(PlayerSpectateVehicle),
}

/// RPC ids decoded by [`decode`].
pub const INCOMING_IDS: &[u8] = &[
    SetPlayerName::ID,
    PlayerStreamIn::ID,
    SetPlayerChatBubble::ID,
    ShowPlayerNameTagForPlayer::ID,
    TogglePlayerSpectating::ID,
    PlayerSpectatePlayer::ID,
    PlayerSpectateVehicle::ID,
    PlayerJoin::ID,
    PlayerQuit::ID,
    PlayerStreamOut::ID,
];

/// Decode an S→C RPC payload of this module; `None` for other ids or
/// malformed (truncated) data. Trailing bits are ignored, as in the client.
pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
    Some(match id {
        PlayerJoin::ID => Incoming::PlayerJoin(PlayerJoin::read(r)?),
        PlayerQuit::ID => Incoming::PlayerQuit(PlayerQuit::read(r)?),
        PlayerStreamIn::ID => Incoming::PlayerStreamIn(PlayerStreamIn::read(r)?),
        PlayerStreamOut::ID => Incoming::PlayerStreamOut(PlayerStreamOut::read(r)?),
        SetPlayerName::ID => Incoming::SetPlayerName(SetPlayerName::read(r)?),
        SetPlayerChatBubble::ID => Incoming::SetPlayerChatBubble(SetPlayerChatBubble::read(r)?),
        ShowPlayerNameTagForPlayer::ID => Incoming::ShowPlayerNameTagForPlayer(ShowPlayerNameTagForPlayer::read(r)?),
        TogglePlayerSpectating::ID => Incoming::TogglePlayerSpectating(TogglePlayerSpectating::read(r)?),
        PlayerSpectatePlayer::ID => Incoming::PlayerSpectatePlayer(PlayerSpectatePlayer::read(r)?),
        PlayerSpectateVehicle::ID => Incoming::PlayerSpectateVehicle(PlayerSpectateVehicle::read(r)?),
        _ => return None,
    })
}

impl Incoming {
    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::PlayerJoin(_) => PlayerJoin::ID,
            Self::PlayerQuit(_) => PlayerQuit::ID,
            Self::PlayerStreamIn(_) => PlayerStreamIn::ID,
            Self::PlayerStreamOut(_) => PlayerStreamOut::ID,
            Self::SetPlayerName(_) => SetPlayerName::ID,
            Self::SetPlayerChatBubble(_) => SetPlayerChatBubble::ID,
            Self::ShowPlayerNameTagForPlayer(_) => ShowPlayerNameTagForPlayer::ID,
            Self::TogglePlayerSpectating(_) => TogglePlayerSpectating::ID,
            Self::PlayerSpectatePlayer(_) => PlayerSpectatePlayer::ID,
            Self::PlayerSpectateVehicle(_) => PlayerSpectateVehicle::ID,
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::PlayerJoin(p) => p.write(w),
            Self::PlayerQuit(p) => p.write(w),
            Self::PlayerStreamIn(p) => p.write(w),
            Self::PlayerStreamOut(p) => p.write(w),
            Self::SetPlayerName(p) => p.write(w),
            Self::SetPlayerChatBubble(p) => p.write(w),
            Self::ShowPlayerNameTagForPlayer(p) => p.write(w),
            Self::TogglePlayerSpectating(p) => p.write(w),
            Self::PlayerSpectatePlayer(p) => p.write(w),
            Self::PlayerSpectateVehicle(p) => p.write(w),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes<P: RpcPayload>(p: &P) -> Vec<u8> {
        let mut w = BitWriter::new();
        p.write(&mut w);
        assert_eq!(w.bit_len() % 8, 0);
        w.into_bytes()
    }

    /// Write, read back (aligned and at a 3-bit offset), and check full consumption.
    fn roundtrip<P: RpcPayload + PartialEq + std::fmt::Debug>(p: &P) {
        let mut w = BitWriter::new();
        p.write(&mut w);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        assert_eq!(P::read(&mut r).as_ref(), Some(p));
        assert_eq!(r.remaining(), 0);

        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        p.write(&mut w);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        r.skip(3).unwrap();
        assert_eq!(P::read(&mut r).as_ref(), Some(p));
        assert_eq!(r.remaining(), 0);

        // Every strict prefix is rejected.
        for n in 0..w.bit_len() - 3 {
            let mut r = BitReader::with_bit_len(w.as_bytes(), 3 + n);
            r.skip(3).unwrap();
            assert_eq!(P::read(&mut r), None, "prefix of {n} bits");
        }
    }

    fn samples() -> Vec<Incoming> {
        vec![
            Incoming::PlayerJoin(PlayerJoin {
                player_id: 7,
                colour: 0xFF00_00FF,
                is_npc: 1,
                name: b"Bot_[1]".to_vec(),
            }),
            Incoming::PlayerQuit(PlayerQuit { player_id: 1003, reason: 2 }),
            Incoming::PlayerStreamIn(PlayerStreamIn {
                player_id: 12,
                team: 255,
                skin: 294,
                position: Vec3 { x: 1958.33, y: 1343.12, z: 15.36 },
                facing_angle: 270.5,
                colour: 0x00FF_00AA,
                fighting_style: 6,
                skill_levels: [999, 1, 2, 3, 4, 5, 6, 7, 8, 9, 1000],
            }),
            Incoming::PlayerStreamOut(PlayerStreamOut { player_id: 1004 }),
            Incoming::SetPlayerName(SetPlayerName { player_id: 3, name: b"New_Name".to_vec(), success: 1 }),
            Incoming::SetPlayerChatBubble(SetPlayerChatBubble {
                player_id: 9,
                colour: 0xFFFF_FFFF,
                draw_distance: 30.0,
                expire_time: 5000,
                text: b"hello \xe9".to_vec(),
            }),
            Incoming::ShowPlayerNameTagForPlayer(ShowPlayerNameTagForPlayer { player_id: 4, show: 0 }),
            Incoming::TogglePlayerSpectating(TogglePlayerSpectating { enable: 1 }),
            Incoming::PlayerSpectatePlayer(PlayerSpectatePlayer { player_id: 5, mode: spectate_mode::SIDE }),
            Incoming::PlayerSpectateVehicle(PlayerSpectateVehicle { vehicle_id: 1999, mode: spectate_mode::FIXED }),
        ]
    }

    fn write_incoming(m: &Incoming, w: &mut BitWriter) -> u8 {
        match m {
            Incoming::PlayerJoin(p) => (p.write(w), PlayerJoin::ID).1,
            Incoming::PlayerQuit(p) => (p.write(w), PlayerQuit::ID).1,
            Incoming::PlayerStreamIn(p) => (p.write(w), PlayerStreamIn::ID).1,
            Incoming::PlayerStreamOut(p) => (p.write(w), PlayerStreamOut::ID).1,
            Incoming::SetPlayerName(p) => (p.write(w), SetPlayerName::ID).1,
            Incoming::SetPlayerChatBubble(p) => (p.write(w), SetPlayerChatBubble::ID).1,
            Incoming::ShowPlayerNameTagForPlayer(p) => (p.write(w), ShowPlayerNameTagForPlayer::ID).1,
            Incoming::TogglePlayerSpectating(p) => (p.write(w), TogglePlayerSpectating::ID).1,
            Incoming::PlayerSpectatePlayer(p) => (p.write(w), PlayerSpectatePlayer::ID).1,
            Incoming::PlayerSpectateVehicle(p) => (p.write(w), PlayerSpectateVehicle::ID).1,
        }
    }

    #[test]
    fn all_payloads_roundtrip() {
        for m in samples() {
            match &m {
                Incoming::PlayerJoin(p) => roundtrip(p),
                Incoming::PlayerQuit(p) => roundtrip(p),
                Incoming::PlayerStreamIn(p) => roundtrip(p),
                Incoming::PlayerStreamOut(p) => roundtrip(p),
                Incoming::SetPlayerName(p) => roundtrip(p),
                Incoming::SetPlayerChatBubble(p) => roundtrip(p),
                Incoming::ShowPlayerNameTagForPlayer(p) => roundtrip(p),
                Incoming::TogglePlayerSpectating(p) => roundtrip(p),
                Incoming::PlayerSpectatePlayer(p) => roundtrip(p),
                Incoming::PlayerSpectateVehicle(p) => roundtrip(p),
            }
        }
    }

    #[test]
    fn decode_dispatches_every_incoming_id() {
        let mut seen = Vec::new();
        for m in samples() {
            let mut w = BitWriter::new();
            let id = write_incoming(&m, &mut w);
            assert!(INCOMING_IDS.contains(&id), "id {id} missing from INCOMING_IDS");
            let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
            assert_eq!(decode(id, &mut r), Some(m));
            seen.push(id);
        }
        seen.sort_unstable();
        let mut ids = INCOMING_IDS.to_vec();
        ids.sort_unstable();
        assert_eq!(seen, ids);
        assert_eq!(ids, [11, 32, 59, 80, 124, 126, 127, 137, 138, 163]);
    }

    #[test]
    fn decode_rejects_foreign_ids_and_truncation() {
        let data = [0u8; 128];
        for id in 0..=u8::MAX {
            if !INCOMING_IDS.contains(&id) {
                assert_eq!(decode(id, &mut BitReader::new(&data)), None, "id {id}");
            }
            assert_eq!(decode(id, &mut BitReader::new(&[])), None, "id {id} on empty data");
        }
    }

    #[test]
    fn decode_through_rpc_framing() {
        let p = SetPlayerName { player_id: 0x0102, name: b"abc".to_vec(), success: 1 };
        let w = p.to_rpc();
        let rpc = crate::rpc::decode(w.as_bytes(), w.bit_len()).unwrap();
        assert_eq!(rpc.id, 11);
        assert_eq!(decode(rpc.id, &mut rpc.reader()), Some(Incoming::SetPlayerName(p)));
    }

    // ---- byte-exact layouts, built by hand from the spec tables ----

    #[test]
    fn player_join_bytes() {
        let wire = [
            0x2a, 0x00, // playerId 42
            0x11, 0x22, 0x33, 0x44, // colour 0x44332211
            0x01, // isNPC
            0x03, b'B', b'o', b'b', // nameLen + name
        ];
        let p = PlayerJoin { player_id: 42, colour: 0x4433_2211, is_npc: 1, name: b"Bob".to_vec() };
        assert_eq!(bytes(&p), wire);
        assert_eq!(PlayerJoin::read(&mut BitReader::new(&wire)), Some(p));
    }

    #[test]
    fn player_quit_bytes() {
        let wire = [0xeb, 0x03, 0x01];
        let p = PlayerQuit { player_id: 1003, reason: 1 };
        assert_eq!(bytes(&p), wire);
        assert_eq!(PlayerQuit::read(&mut BitReader::new(&wire)), Some(p));
    }

    #[test]
    fn player_stream_in_bytes() {
        let mut wire = vec![
            0x05, 0x00, // playerId 5
            0xff, // team 255
            0x26, 0x01, 0x00, 0x00, // skin 294
        ];
        wire.extend_from_slice(&1.0f32.to_le_bytes()); // x
        wire.extend_from_slice(&(-2.5f32).to_le_bytes()); // y
        wire.extend_from_slice(&10.0f32.to_le_bytes()); // z
        wire.extend_from_slice(&90.0f32.to_le_bytes()); // facing angle
        wire.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]); // colour
        wire.push(0x04); // fighting style
        for i in 0..11u16 {
            wire.extend_from_slice(&(i * 100 + 1).to_le_bytes()); // skills
        }
        assert_eq!(wire.len(), 2 + 1 + 4 + 12 + 4 + 4 + 1 + 22);
        let p = PlayerStreamIn {
            player_id: 5,
            team: 255,
            skin: 294,
            position: Vec3 { x: 1.0, y: -2.5, z: 10.0 },
            facing_angle: 90.0,
            colour: 0xddcc_bbaa,
            fighting_style: FIGHTING_STYLE_DEFAULT,
            skill_levels: core::array::from_fn(|i| i as u16 * 100 + 1),
        };
        assert_eq!(bytes(&p), wire);
        assert_eq!(PlayerStreamIn::read(&mut BitReader::new(&wire)), Some(p));
    }

    #[test]
    fn player_stream_out_bytes() {
        let wire = [0x34, 0x12];
        assert_eq!(bytes(&PlayerStreamOut { player_id: 0x1234 }), wire);
        assert_eq!(PlayerStreamOut::read(&mut BitReader::new(&wire)), Some(PlayerStreamOut { player_id: 0x1234 }));
    }

    #[test]
    fn set_player_name_bytes() {
        let wire = [0x00, 0x01, 0x02, b'h', b'i', 0x01];
        let p = SetPlayerName { player_id: 0x0100, name: b"hi".to_vec(), success: 1 };
        assert_eq!(bytes(&p), wire);
        assert_eq!(SetPlayerName::read(&mut BitReader::new(&wire)), Some(p));
    }

    #[test]
    fn chat_bubble_bytes() {
        let mut wire = vec![0x09, 0x00, 0xff, 0x00, 0x00, 0xff];
        wire.extend_from_slice(&[0x00, 0x00, 0xa0, 0x41]); // 20.0f32
        wire.extend_from_slice(&[0x88, 0x13, 0x00, 0x00]); // 5000 ms
        wire.extend_from_slice(&[0x02, b'o', b'k']);
        let p = SetPlayerChatBubble {
            player_id: 9,
            colour: 0xff00_00ff,
            draw_distance: 20.0,
            expire_time: 5000,
            text: b"ok".to_vec(),
        };
        assert_eq!(bytes(&p), wire);
        assert_eq!(SetPlayerChatBubble::read(&mut BitReader::new(&wire)), Some(p));
    }

    #[test]
    fn small_layouts_bytes() {
        let tag = ShowPlayerNameTagForPlayer { player_id: 2, show: 1 };
        assert_eq!(bytes(&tag), [0x02, 0x00, 0x01]);
        let toggle = TogglePlayerSpectating { enable: 1 };
        assert_eq!(bytes(&toggle), [0x01, 0x00, 0x00, 0x00]);
        let sp = PlayerSpectatePlayer { player_id: 0x0203, mode: 2 };
        assert_eq!(bytes(&sp), [0x03, 0x02, 0x02]);
        let sv = PlayerSpectateVehicle { vehicle_id: 0x07cf, mode: 3 };
        assert_eq!(bytes(&sv), [0xcf, 0x07, 0x03]);
    }

    /// Payloads are bit-packed and not aligned: the same fields shifted by one bit.
    #[test]
    fn unaligned_bits_exact() {
        // 1 | 0x2a 0x00 | 0x01  →  1001_0101 0000_0000 0000_0000 1000_0000
        let wire = [0x95, 0x00, 0x00, 0x80];
        let mut r = BitReader::with_bit_len(&wire, 1 + 24);
        assert_eq!(r.read_bit(), Some(true));
        assert_eq!(PlayerQuit::read(&mut r), Some(PlayerQuit { player_id: 42, reason: 1 }));
        let mut w = BitWriter::new();
        w.write_bit(true);
        PlayerQuit { player_id: 42, reason: 1 }.write(&mut w);
        assert_eq!(w.as_bytes(), wire);
    }

    #[test]
    fn trailing_data_is_ignored() {
        let wire = [0x02, 0x00, 0x01, 0xde, 0xad];
        let mut r = BitReader::new(&wire);
        assert_eq!(
            decode(80, &mut r),
            Some(Incoming::ShowPlayerNameTagForPlayer(ShowPlayerNameTagForPlayer { player_id: 2, show: 1 }))
        );
    }

    #[test]
    fn spectate_camera_modes() {
        let p = |mode| PlayerSpectatePlayer { player_id: 0, mode }.gta_camera_mode();
        let v = |mode| PlayerSpectateVehicle { vehicle_id: 0, mode }.gta_camera_mode();
        assert_eq!([p(1), p(2), p(3), p(0), p(200)], [4, 15, 14, 4, 4]);
        assert_eq!([v(1), v(2), v(3), v(0), v(200)], [3, 15, 14, 3, 3]);
    }
}
