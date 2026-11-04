//! Client send policy for sync packets, as constants only (no timers).
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §0.1 (send parameters) and
//! §0.4 (send-rate machinery, local player tick A 0x100074c0).

use super::ids;
use crate::raknet::Reliability;

/// Send parameters of one packet type: `Send(bs, priority, reliability, channel)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendParams {
    /// RakNet priority as passed by the client (1 = HIGH for every sync packet).
    pub priority: u8,
    pub reliability: Reliability,
    /// Ordering channel.
    pub channel: u8,
}

/// Priority used for every client-sent sync packet (HIGH).
pub const PRIORITY_HIGH: u8 = 1;

const fn params(reliability: Reliability, channel: u8) -> SendParams {
    SendParams { priority: PRIORITY_HIGH, reliability, channel }
}

/// 200 driver, 203 aim, 207 foot, 209 unoccupied, 210 trailer, 211 passenger:
/// unreliable-sequenced on channel 1.
pub const SEQUENCED_SYNC: SendParams = params(Reliability::UnreliableSequenced, 1);
/// 206 bullet: unreliable-sequenced on channel 0.
pub const BULLET: SendParams = params(Reliability::UnreliableSequenced, 0);
/// 204 weapons and 205 stats: unreliable on channel 0.
pub const WEAPONS_STATS: SendParams = params(Reliability::Unreliable, 0);
/// 212 spectator: unreliable on channel 1.
pub const SPECTATOR: SendParams = params(Reliability::Unreliable, 1);
/// 201 RCON command: reliable on channel 0.
pub const RCON: SendParams = params(Reliability::Reliable, 0);

/// Send parameters the client uses for an outgoing packet id (`None` for ids
/// the client never sends, e.g. 208).
pub const fn send_params(packet_id: u8) -> Option<SendParams> {
    Some(match packet_id {
        ids::VEHICLE_SYNC
        | ids::AIM_SYNC
        | ids::FOOT_SYNC
        | ids::UNOCCUPIED_SYNC
        | ids::TRAILER_SYNC
        | ids::PASSENGER_SYNC => SEQUENCED_SYNC,
        ids::BULLET_SYNC => BULLET,
        ids::WEAPONS_UPDATE | ids::STATS_SYNC => WEAPONS_STATS,
        ids::SPECTATOR_SYNC => SPECTATOR,
        ids::RCON_COMMAND => RCON,
        _ => return None,
    })
}

// --- Rates (InitGame fields 21–24, handshake.md §5) ---------------------------

/// Default on-foot rate (ms) before InitGame overrides it.
pub const DEFAULT_ON_FOOT_RATE_MS: u32 = 30;
/// Default in-car rate (ms).
pub const DEFAULT_IN_CAR_RATE_MS: u32 = 30;
/// Default weapon rate (ms); used for aim sync while firing when the
/// lag-comp selector is 2.
pub const DEFAULT_WEAPON_RATE_MS: u32 = 30;
/// Default send-rate multiplier; stored by the client but never read.
pub const DEFAULT_MULTIPLIER: u32 = 2;

/// On-foot / in-car interval when the local ped does not exist.
pub const NO_PED_INTERVAL_MS: u32 = 1000;
/// On-foot / in-car interval in LAN mode.
pub const LAN_INTERVAL_MS: u32 = 15;
// Otherwise interval = rate + N ms, N = number of allocated game player-ped
// slots (roughly the streamed-in remote players).

/// Duplicate suppression: a foot/aim/driver/passenger/trailer struct
/// byte-identical to the last one sent of its type is not re-sent while fewer
/// than this many ms have elapsed since the last such send. The timestamp is
/// shared by those five types, the "last struct" copy is per type.
pub const DUPLICATE_SUPPRESSION_MS: u32 = 501;

/// Rotation jitter filter: the last sent quaternion is reused if every
/// component of the new one differs from it by less than this (207, 200).
pub const ROTATION_JITTER_EPSILON: f32 = 1e-5;

/// Aim sync (203) interval when not aiming/firing (on foot, passenger, and the
/// extra driver tick).
pub const AIM_IDLE_INTERVAL_MS: u32 = 1000;
/// Aim sync interval on foot with FIRE or AIM held (lag-comp selector != 2),
/// and for passengers during drive-by.
pub const AIM_ACTIVE_INTERVAL_MS: u32 = 100;
/// While spectating, aim sync is sent when elapsed > this × on-foot interval.
pub const SPECTATOR_AIM_INTERVAL_FACTOR: u32 = 2;
/// Vehicle models for which a driver sends aim sync after every 200
/// (armed vehicles).
pub const ARMED_VEHICLE_MODELS: [u16; 4] = [432, 564, 407, 601];

/// Stats (205): sent when more than this many ms have elapsed.
pub const STATS_INTERVAL_MS: u32 = 1000;
/// Weapons update (204): with unchanged targets and armed weapon, skipped if
/// fewer than this many ms since the last send.
pub const WEAPONS_MIN_INTERVAL_MS: u32 = 100;
/// Weapon slots compared by 204 (0..12).
pub const WEAPON_SLOTS: u8 = 13;

/// Unoccupied sync (209): at most this many sends per vehicle-pool tick.
pub const UNOCCUPIED_MAX_PER_TICK: u32 = 3;
/// Unoccupied sync: minimum time between sends for one vehicle (exclusive).
pub const UNOCCUPIED_PER_VEHICLE_MS: u32 = 100;
/// Unoccupied sync: maximum distance from the local player to the vehicle.
pub const UNOCCUPIED_MAX_DISTANCE: f32 = 90.0;
/// Unoccupied sync is never sent for these models (trains/tram).
pub const UNOCCUPIED_EXCLUDED_MODELS: [u16; 5] = [537, 538, 569, 570, 449];

// --- Receive-side id bounds (documented; decoders do not apply them) --------

/// Incoming sync handlers drop packets unless `playerId <` this (most types).
pub const MAX_SYNC_PLAYER_ID_EXCLUSIVE: u16 = 1005;
/// Bound used by bullet (206) and markers (208).
pub const MAX_BULLET_MARKER_PLAYER_ID_EXCLUSIVE: u16 = 1004;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_params_table() {
        for id in [200u8, 203, 207, 209, 210, 211] {
            assert_eq!(
                send_params(id),
                Some(SendParams { priority: 1, reliability: Reliability::UnreliableSequenced, channel: 1 })
            );
        }
        assert_eq!(send_params(206).map(|p| (p.reliability as u8, p.channel)), Some((7, 0)));
        assert_eq!(send_params(204).map(|p| (p.reliability as u8, p.channel)), Some((6, 0)));
        assert_eq!(send_params(205).map(|p| (p.reliability as u8, p.channel)), Some((6, 0)));
        assert_eq!(send_params(212).map(|p| (p.reliability as u8, p.channel)), Some((6, 1)));
        assert_eq!(send_params(201).map(|p| (p.reliability as u8, p.channel)), Some((8, 0)));
        assert_eq!(send_params(208), None);
    }
}
