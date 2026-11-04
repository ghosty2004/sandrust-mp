//! What the bot knows about other players and streamed vehicles, kept up to
//! date from S→C RPCs and sync packets.

use std::collections::BTreeMap;
use std::time::Instant;

use samp_proto::rpcs::{Incoming, remote, vehicle};
use samp_proto::sync::IncomingSync;
use samp_proto::types::Vec3;

use crate::geom;

/// A remote player.
#[derive(Clone, Debug, Default)]
pub struct Player {
    pub name: String,
    /// Streamed in for us (between RPC 32 and 163).
    pub streamed: bool,
    pub pos: Option<Vec3>,
    /// Heading in radians.
    pub heading: f32,
    /// Vehicle from the last driver/passenger sync (cleared by foot sync).
    pub vehicle: Option<u16>,
}

/// A streamed-in vehicle.
#[derive(Clone, Debug)]
pub struct Vehicle {
    pub model: u32,
    pub pos: Vec3,
    /// Heading in radians.
    pub heading: f32,
    pub health: f32,
    /// SA-MP doors-locked flag (RPC 24 `doors` == 1).
    pub locked: bool,
    /// Remote player that last sent driver sync for it, and when.
    pub driver: Option<(u16, Instant)>,
}

#[derive(Debug, Default)]
pub struct World {
    pub players: BTreeMap<u16, Player>,
    pub vehicles: BTreeMap<u16, Vehicle>,
}

impl World {
    pub fn player_by_name(&self, name: &str) -> Option<u16> {
        self.players.iter().find(|(_, p)| p.name.eq_ignore_ascii_case(name)).map(|(&id, _)| id)
    }

    /// Number of streamed-in remote players (the "N" of the sync intervals).
    pub fn streamed_players(&self) -> u32 {
        self.players.values().filter(|p| p.streamed).count() as u32
    }

    /// Closest vehicle to `pos` that nobody drove in the last 2 s.
    pub fn nearest_free_vehicle(&self, pos: Vec3, now: Instant) -> Option<u16> {
        self.vehicles
            .iter()
            .filter(|(_, v)| v.driver.is_none_or(|(_, t)| now.duration_since(t).as_secs_f32() > 2.0))
            .min_by(|a, b| geom::dist(a.1.pos, pos).total_cmp(&geom::dist(b.1.pos, pos)))
            .map(|(&id, _)| id)
    }

    /// Apply an RPC. Returns nothing: callers read the updated state.
    pub fn apply_rpc(&mut self, rpc: &Incoming) {
        match rpc {
            Incoming::Remote(r) => self.apply_remote(r),
            Incoming::Vehicle(v) => self.apply_vehicle(v),
            _ => {}
        }
    }

    fn apply_remote(&mut self, r: &remote::Incoming) {
        use remote::Incoming as R;
        match r {
            R::PlayerJoin(j) => {
                self.players.insert(
                    j.player_id,
                    Player { name: String::from_utf8_lossy(&j.name).into_owned(), ..Player::default() },
                );
            }
            R::PlayerQuit(q) => {
                self.players.remove(&q.player_id);
            }
            R::PlayerStreamIn(s) => {
                let p = self.players.entry(s.player_id).or_default();
                p.streamed = true;
                p.pos = Some(s.position);
                p.heading = s.facing_angle.to_radians();
                p.vehicle = None;
            }
            R::PlayerStreamOut(s) => {
                if let Some(p) = self.players.get_mut(&s.player_id) {
                    p.streamed = false;
                }
            }
            R::SetPlayerName(n) if n.success != 0 => {
                if let Some(p) = self.players.get_mut(&n.player_id) {
                    p.name = String::from_utf8_lossy(&n.name).into_owned();
                }
            }
            _ => {}
        }
    }

    fn apply_vehicle(&mut self, v: &vehicle::Incoming) {
        use vehicle::Incoming as V;
        match v {
            V::StreamInVehicle(s) => {
                self.vehicles.insert(
                    s.vehicle_id,
                    Vehicle {
                        model: s.model,
                        pos: s.position,
                        heading: s.angle.to_radians(),
                        health: s.health,
                        locked: false,
                        driver: None,
                    },
                );
            }
            V::StreamOutVehicle(s) => {
                self.vehicles.remove(&s.vehicle_id);
            }
            V::SetVehiclePosition(s) => {
                if let Some(veh) = self.vehicles.get_mut(&s.vehicle_id) {
                    veh.pos = s.position;
                }
            }
            V::SetVehicleZAngle(s) => {
                if let Some(veh) = self.vehicles.get_mut(&s.vehicle_id) {
                    veh.heading = s.angle.to_radians();
                }
            }
            V::SetVehicleHealth(s) => {
                if let Some(veh) = self.vehicles.get_mut(&s.vehicle_id) {
                    veh.health = s.health;
                }
            }
            V::SetVehicleParams(s) => {
                if let Some(veh) = self.vehicles.get_mut(&s.vehicle_id)
                    && s.params.doors <= 1
                {
                    veh.locked = s.params.doors == 1;
                }
            }
            _ => {}
        }
    }

    /// Apply a S→C sync packet (positions of remote players and vehicles).
    pub fn apply_sync(&mut self, s: &IncomingSync, now: Instant) {
        match s {
            IncomingSync::Foot { sync, .. } => {
                let p = self.players.entry(sync.player_id).or_default();
                p.pos = Some(sync.position);
                p.heading = geom::quat_to_heading(sync.rotation.to_quat());
                p.vehicle = None;
            }
            IncomingSync::Vehicle { sync, .. } => {
                let p = self.players.entry(sync.player_id).or_default();
                p.pos = Some(sync.position);
                p.vehicle = Some(sync.vehicle_id);
                if let Some(v) = self.vehicles.get_mut(&sync.vehicle_id) {
                    v.pos = sync.position;
                    v.heading = geom::quat_to_heading(sync.rotation.to_quat());
                    v.health = sync.vehicle_health.into();
                    v.driver = Some((sync.player_id, now));
                }
            }
            IncomingSync::Passenger(p) => {
                let pl = self.players.entry(p.player_id).or_default();
                pl.vehicle = Some(p.sync.vehicle_id);
                pl.pos = self.vehicles.get(&p.sync.vehicle_id).map(|v| v.pos).or(Some(p.sync.position));
            }
            IncomingSync::Unoccupied(u) => {
                if let Some(v) = self.vehicles.get_mut(&u.sync.vehicle_id) {
                    v.pos = u.sync.position;
                    // direction = forward vector of the vehicle matrix.
                    v.heading = geom::heading_of(u.sync.direction);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samp_proto::rpcs::vehicle::StreamInVehicle;

    #[test]
    fn nearest_free_vehicle_skips_driven() {
        let now = Instant::now();
        let mut w = World::default();
        for (id, x) in [(1u16, 10.0f32), (2, 3.0), (3, 50.0)] {
            w.apply_rpc(&Incoming::Vehicle(vehicle::Incoming::StreamInVehicle(StreamInVehicle {
                vehicle_id: id,
                model: 560,
                position: geom::v3(x, 0.0, 0.0),
                health: 1000.0,
                ..StreamInVehicle::default()
            })));
        }
        assert_eq!(w.nearest_free_vehicle(Vec3::default(), now), Some(2));
        w.vehicles.get_mut(&2).unwrap().driver = Some((7, now));
        assert_eq!(w.nearest_free_vehicle(Vec3::default(), now), Some(1));
    }
}
