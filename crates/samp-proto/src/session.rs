//! Session messages: connection accepted (`0x22`), join (RPC 25), InitGame
//! (RPC 139), rejection (RPC 130) and the client serial.
//!
//! Spec: docs/re/handshake.md §1 and §5; docs/re/subsystems/session-misc.md.

use crate::bitstream::{BitReader, BitWriter};
use crate::raknet::id;

/// Per-version join parameters. One profile per supported client build.
#[derive(Clone, Copy, Debug)]
pub struct VersionProfile {
    pub netgame_version: u32,
    pub mod_flag: u8,
    pub version_string: &'static str,
}

/// Both known 0.3.7-R4 builds (2019 and 2021) behave identically on the wire.
pub const R4: VersionProfile = VersionProfile { netgame_version: 4057, mod_flag: 1, version_string: "0.3.7-R4" };

/// Body of `0x22` connection-request-accepted (exactly 13 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub external_ip: u32,
    pub external_port: u16,
    pub player_id: u16,
    pub challenge: u32,
}

pub fn parse_accepted(msg: &[u8]) -> Option<Accepted> {
    if msg.len() != 13 || msg[0] != id::CONNECTION_REQUEST_ACCEPTED {
        return None;
    }
    let mut r = BitReader::new(&msg[1..]);
    Some(Accepted {
        external_ip: r.read_u32()?,
        external_port: r.read_u16()?,
        player_id: r.read_u16()?,
        challenge: r.read_u32()?,
    })
}

/// RPC 25 payload. Note the challenge response is written twice (field 5 and
/// a trailing copy after the version string), exactly like the original client.
pub fn build_player_connect(v: &VersionProfile, nickname: &str, challenge: u32, serial: &str) -> BitWriter {
    let response = challenge ^ v.netgame_version;
    let mut w = BitWriter::new();
    w.write_u32(v.netgame_version);
    w.write_u8(v.mod_flag);
    w.write_u8(nickname.len() as u8);
    w.write_bytes(nickname.as_bytes());
    w.write_u32(response);
    w.write_u8(serial.len() as u8);
    w.write_bytes(serial.as_bytes());
    w.write_u8(v.version_string.len() as u8);
    w.write_bytes(v.version_string.as_bytes());
    w.write_u32(response);
    w
}

/// Client serial: a 40-hex-digit value multiplied by 1001, rendered in
/// uppercase hex. Servers require the parsed value to be divisible by 1001.
/// `seed_hex` should be 40 hex digits (e.g. a SHA-1 rendered as hex).
pub fn make_serial(seed_hex: &str) -> String {
    let mut digits: Vec<u32> = seed_hex.chars().filter_map(|c| c.to_digit(16)).collect();
    // digits are most-significant first; multiply in place by 1001.
    let mut carry = 0u32;
    for d in digits.iter_mut().rev() {
        let v = *d * 1001 + carry;
        *d = v % 16;
        carry = v / 16;
    }
    let mut head = Vec::new();
    while carry > 0 {
        head.push(carry % 16);
        carry /= 16;
    }
    head.reverse();
    head.extend(digits);
    let s: String = head.iter().map(|d| char::from_digit(*d, 16).unwrap().to_ascii_uppercase()).collect();
    let s = s.trim_start_matches('0');
    if s.is_empty() { "0".into() } else { s.into() }
}

/// RPC 139 InitGame, fields in wire order (handshake.md §5).
#[derive(Clone, Debug, PartialEq)]
pub struct InitGame {
    pub zone_names: bool,
    pub cj_walk: bool,
    pub allow_weapons: bool,
    pub limit_chat_radius: bool,
    pub chat_radius: f32,
    pub stunt_bonus: bool,
    pub name_tag_distance: f32,
    pub disable_enter_exits: bool,
    pub name_tag_los: bool,
    pub manual_engine_and_lights: bool,
    pub spawns_available: u32,
    pub player_id: u16,
    pub show_name_tags: bool,
    pub player_marker_mode: u32,
    pub world_time: u8,
    pub weather: u8,
    pub gravity: f32,
    pub lan_mode: bool,
    pub death_drop_money: u32,
    pub instagib: bool,
    pub on_foot_rate: u32,
    pub in_car_rate: u32,
    pub weapon_rate: u32,
    pub multiplier: u32,
    pub lag_compensation: u32,
    pub server_name: String,
    pub vehicle_models: Vec<u8>,
    pub vehicle_friendly_fire: u32,
}

pub fn parse_init_game(r: &mut BitReader) -> Option<InitGame> {
    let bit = |r: &mut BitReader| r.read_bit();
    Some(InitGame {
        zone_names: bit(r)?,
        cj_walk: bit(r)?,
        allow_weapons: bit(r)?,
        limit_chat_radius: bit(r)?,
        chat_radius: r.read_f32()?,
        stunt_bonus: bit(r)?,
        name_tag_distance: r.read_f32()?,
        disable_enter_exits: bit(r)?,
        name_tag_los: bit(r)?,
        manual_engine_and_lights: bit(r)?,
        spawns_available: r.read_u32()?,
        player_id: r.read_u16()?,
        show_name_tags: bit(r)?,
        player_marker_mode: r.read_u32()?,
        world_time: r.read_u8()?,
        weather: r.read_u8()?,
        gravity: r.read_f32()?,
        lan_mode: bit(r)?,
        death_drop_money: r.read_u32()?,
        instagib: bit(r)?,
        on_foot_rate: r.read_u32()?,
        in_car_rate: r.read_u32()?,
        weapon_rate: r.read_u32()?,
        multiplier: r.read_u32()?,
        lag_compensation: r.read_u32()?,
        server_name: {
            let n = r.read_u8()?;
            String::from_utf8_lossy(&r.read_bytes(n.into())?).into_owned()
        },
        vehicle_models: r.read_bytes(212)?,
        vehicle_friendly_fire: r.read_u32()?,
    })
}

/// Number of vehicle-model bytes in [`InitGame`] (fixed on the wire).
pub const INIT_GAME_VEHICLE_MODELS: usize = 212;

impl InitGame {
    /// Encode in wire order (the inverse of [`parse_init_game`], for tests and
    /// tools). The server name is truncated to 255 bytes; `vehicle_models` is
    /// truncated / zero-padded to [`INIT_GAME_VEHICLE_MODELS`] bytes.
    pub fn write(&self, w: &mut BitWriter) {
        w.write_bit(self.zone_names);
        w.write_bit(self.cj_walk);
        w.write_bit(self.allow_weapons);
        w.write_bit(self.limit_chat_radius);
        w.write_f32(self.chat_radius);
        w.write_bit(self.stunt_bonus);
        w.write_f32(self.name_tag_distance);
        w.write_bit(self.disable_enter_exits);
        w.write_bit(self.name_tag_los);
        w.write_bit(self.manual_engine_and_lights);
        w.write_u32(self.spawns_available);
        w.write_u16(self.player_id);
        w.write_bit(self.show_name_tags);
        w.write_u32(self.player_marker_mode);
        w.write_u8(self.world_time);
        w.write_u8(self.weather);
        w.write_f32(self.gravity);
        w.write_bit(self.lan_mode);
        w.write_u32(self.death_drop_money);
        w.write_bit(self.instagib);
        w.write_u32(self.on_foot_rate);
        w.write_u32(self.in_car_rate);
        w.write_u32(self.weapon_rate);
        w.write_u32(self.multiplier);
        w.write_u32(self.lag_compensation);
        let name = &self.server_name.as_bytes()[..self.server_name.len().min(255)];
        w.write_u8(name.len() as u8);
        w.write_bytes(name);
        let mut models = [0u8; INIT_GAME_VEHICLE_MODELS];
        let n = self.vehicle_models.len().min(INIT_GAME_VEHICLE_MODELS);
        models[..n].copy_from_slice(&self.vehicle_models[..n]);
        w.write_bytes(&models);
        w.write_u32(self.vehicle_friendly_fire);
    }
}

/// RPC 130 rejection reason text, as the original client words it.
pub fn rejection_reason(code: u8) -> &'static str {
    match code {
        1 => "Incorrect Version",
        2 => "Unacceptable NickName",
        3 => "Bad mod version",
        4 => "Unable to allocate a player slot",
        _ => "unknown reason",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_layout() {
        let mut m = vec![0x22];
        m.extend_from_slice(&0x0100007fu32.to_le_bytes());
        m.extend_from_slice(&5000u16.to_le_bytes());
        m.extend_from_slice(&3u16.to_le_bytes());
        m.extend_from_slice(&0xAABBCCDDu32.to_le_bytes());
        assert_eq!(
            parse_accepted(&m),
            Some(Accepted { external_ip: 0x0100007f, external_port: 5000, player_id: 3, challenge: 0xAABBCCDD })
        );
        assert_eq!(parse_accepted(&m[..12]), None);
    }

    #[test]
    fn player_connect_layout() {
        let w = build_player_connect(&R4, "Bot", 0x1000, "AB");
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(r.read_u32(), Some(4057));
        assert_eq!(r.read_u8(), Some(1));
        assert_eq!(r.read_u8(), Some(3));
        assert_eq!(r.read_bytes(3).as_deref(), Some(&b"Bot"[..]));
        let resp = 0x1000 ^ 4057;
        assert_eq!(r.read_u32(), Some(resp));
        assert_eq!(r.read_u8(), Some(2));
        assert_eq!(r.read_bytes(2).as_deref(), Some(&b"AB"[..]));
        assert_eq!(r.read_u8(), Some(8));
        assert_eq!(r.read_bytes(8).as_deref(), Some(&b"0.3.7-R4"[..]));
        assert_eq!(r.read_u32(), Some(resp));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn serial_is_multiple_of_1001() {
        // 0x10 * 1001 = 16016 = 0x3E90
        assert_eq!(make_serial("10"), "3E90");
        let s = make_serial("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        // verify divisibility by long division in base 16
        let rem = s.chars().fold(0u64, |acc, c| (acc * 16 + c.to_digit(16).unwrap() as u64) % 1001);
        assert_eq!(rem, 0);
        assert!(s.len() < 50);
    }
}
