//! Vehicle sync packets: 200 driver, 211 passenger, 209 unoccupied, 210 trailer.
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §2.

use super::encoding::{CompressedVec3, NormQuat, PackedHealthArmour, WeaponByte};
use super::{SyncPacket, ids};
use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Quat, Vec3};

// ---------------------------------------------------------------------------
// 200 PlayerVehicleSync (driver)

/// 200 PlayerVehicleSync (driver), client→server (spec §2 "200"):
/// `u8 200` + raw[63]; unreliable-sequenced, channel 1. Mirrors samp.dll 0x10007080.
///
/// Client behaviour: sent every in-car interval only while the local vehicle
/// id is valid (< 2000); one forced send on death while driving; identical
/// structs within 501 ms suppressed. Rotation uses the same 1e-5 jitter filter
/// as foot sync. `siren`/`landing_gear` are 0/1. `trailer_id` is 0 without an
/// attached trailer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleSync {
    pub vehicle_id: u16,
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    /// (w, x, y, z).
    pub rotation: Quat,
    pub position: Vec3,
    pub velocity: Vec3,
    pub vehicle_health: f32,
    pub player_health: u8,
    pub player_armour: u8,
    pub weapon: WeaponByte,
    pub siren: u8,
    pub landing_gear: u8,
    pub trailer_id: u16,
    /// Model-dependent 32 bits: Hydra (520) raw u32 thrust-nozzle value;
    /// trains/tram (538/537/449) f32 train speed; bikes/BMX f32 lean value;
    /// otherwise 0.0. Use `f32::from_bits` where it is a float.
    pub extra: u32,
}

impl VehicleSync {
    pub const SIZE: usize = 63;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            vehicle_id: r.read_u16()?,
            left_right: r.read_u16()? as i16,
            up_down: r.read_u16()? as i16,
            keys: r.read_u16()?,
            rotation: Quat::read(r)?,
            position: Vec3::read(r)?,
            velocity: Vec3::read(r)?,
            vehicle_health: r.read_f32()?,
            player_health: r.read_u8()?,
            player_armour: r.read_u8()?,
            weapon: WeaponByte::read(r)?,
            siren: r.read_u8()?,
            landing_gear: r.read_u8()?,
            trailer_id: r.read_u16()?,
            extra: r.read_u32()?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u16(self.left_right as u16);
        w.write_u16(self.up_down as u16);
        w.write_u16(self.keys);
        self.rotation.write(w);
        self.position.write(w);
        self.velocity.write(w);
        w.write_f32(self.vehicle_health);
        w.write_u8(self.player_health);
        w.write_u8(self.player_armour);
        self.weapon.write(w);
        w.write_u8(self.siren);
        w.write_u8(self.landing_gear);
        w.write_u16(self.trailer_id);
        w.write_u32(self.extra);
    }
}

raw_sync_packet!(VehicleSync, ids::VEHICLE_SYNC);

/// 200 PlayerVehicleSync (driver), server→client compact form (spec §2 "200",
/// S→C table). May carry a `0x28` timestamp prefix. Mirrors samp.dll 0x1000aa10.
///
/// Wire: `u16 playerId`, `u16 vehicleId, leftRight, upDown, keys`, normalised
/// quaternion, position raw[12], compressed velocity, `u16 vehicleHealth`,
/// packed health/armour, weapon byte, `siren bit`, `landingGear bit`,
/// `hasExtra bit [+ u32]`, `hasTrailer bit [+ u16]`.
///
/// Client behaviour: vehicle health is converted to float; absent extra and
/// trailer read as 0; weapon additional key discarded. Agrees with open.mp
/// (order siren, gear, extra, trailer; open.mp sets `hasExtra` only for a
/// non-zero Hydra thrust value).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteVehicleSync {
    pub player_id: u16,
    pub vehicle_id: u16,
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    pub rotation: NormQuat,
    pub position: Vec3,
    pub velocity: CompressedVec3,
    /// Integer vehicle health.
    pub vehicle_health: u16,
    pub health_armour: PackedHealthArmour,
    pub weapon: WeaponByte,
    pub siren: bool,
    pub landing_gear: bool,
    /// See [`VehicleSync::extra`].
    pub extra: Option<u32>,
    pub trailer_id: Option<u16>,
}

impl SyncPacket for RemoteVehicleSync {
    const PACKET_ID: u8 = ids::VEHICLE_SYNC;

    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            player_id: r.read_u16()?,
            vehicle_id: r.read_u16()?,
            left_right: r.read_u16()? as i16,
            up_down: r.read_u16()? as i16,
            keys: r.read_u16()?,
            rotation: NormQuat::read(r)?,
            position: Vec3::read(r)?,
            velocity: CompressedVec3::read(r)?,
            vehicle_health: r.read_u16()?,
            health_armour: PackedHealthArmour::read(r)?,
            weapon: WeaponByte::read(r)?,
            siren: r.read_bit()?,
            landing_gear: r.read_bit()?,
            extra: if r.read_bit()? { Some(r.read_u32()?) } else { None },
            trailer_id: if r.read_bit()? { Some(r.read_u16()?) } else { None },
        })
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        w.write_u16(self.player_id);
        w.write_u16(self.vehicle_id);
        w.write_u16(self.left_right as u16);
        w.write_u16(self.up_down as u16);
        w.write_u16(self.keys);
        self.rotation.write(w);
        self.position.write(w);
        self.velocity.write(w);
        w.write_u16(self.vehicle_health);
        self.health_armour.write(w);
        self.weapon.write(w);
        w.write_bit(self.siren);
        w.write_bit(self.landing_gear);
        w.write_bit(self.extra.is_some());
        if let Some(e) = self.extra {
            w.write_u32(e);
        }
        w.write_bit(self.trailer_id.is_some());
        if let Some(t) = self.trailer_id {
            w.write_u16(t);
        }
    }
}

impl RemoteVehicleSync {
    /// Build the compact form from a raw driver struct, the way a server would
    /// (presence bits for non-zero extra/trailer; vehicle health truncated and
    /// saturated to u16; implied encoders, since the client never builds this form).
    pub fn from_raw(player_id: u16, s: &VehicleSync) -> Self {
        Self {
            player_id,
            vehicle_id: s.vehicle_id,
            left_right: s.left_right,
            up_down: s.up_down,
            keys: s.keys,
            rotation: NormQuat::from_quat(s.rotation),
            position: s.position,
            velocity: CompressedVec3::from_vec3(s.velocity),
            vehicle_health: s.vehicle_health as u16,
            health_armour: PackedHealthArmour::from_values(s.player_health.into(), s.player_armour.into()),
            weapon: s.weapon,
            siren: s.siren != 0,
            landing_gear: s.landing_gear != 0,
            extra: (s.extra != 0).then_some(s.extra),
            trailer_id: (s.trailer_id != 0).then_some(s.trailer_id),
        }
    }
}

// ---------------------------------------------------------------------------
// 211 PlayerPassengerSync

/// 211 PlayerPassengerSync (spec §2 "211"): `u8 211` + raw[24];
/// unreliable-sequenced, channel 1, every in-car interval. Mirrors samp.dll 0x10005590.
///
/// Byte 2 packs `seat` (low 6 bits), `drive_by` (bit 6) and `cuffed` (bit 7).
/// open.mp reads bytes 2–3 as one u16 and bytes 4–5 as its "percent pair",
/// but writes plain u8 health and armour — byte sizes agree.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassengerSync {
    pub vehicle_id: u16,
    /// Seat index (6 bits).
    pub seat: u8,
    /// Drive-by active.
    pub drive_by: bool,
    /// Cuffed special action active.
    pub cuffed: bool,
    pub weapon: WeaponByte,
    pub health: u8,
    pub armour: u8,
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    /// Ped matrix translation (stored but unused by receivers while seated).
    pub position: Vec3,
}

impl PassengerSync {
    pub const SIZE: usize = 24;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        let vehicle_id = r.read_u16()?;
        let b = r.read_u8()?;
        Some(Self {
            vehicle_id,
            seat: b & 0x3f,
            drive_by: b & 0x40 != 0,
            cuffed: b & 0x80 != 0,
            weapon: WeaponByte::read(r)?,
            health: r.read_u8()?,
            armour: r.read_u8()?,
            left_right: r.read_u16()? as i16,
            up_down: r.read_u16()? as i16,
            keys: r.read_u16()?,
            position: Vec3::read(r)?,
        })
    }

    /// `seat` is masked to 6 bits.
    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8((self.seat & 0x3f) | (u8::from(self.drive_by) << 6) | (u8::from(self.cuffed) << 7));
        self.weapon.write(w);
        w.write_u8(self.health);
        w.write_u8(self.armour);
        w.write_u16(self.left_right as u16);
        w.write_u16(self.up_down as u16);
        w.write_u16(self.keys);
        self.position.write(w);
    }
}

raw_sync_packet!(PassengerSync, ids::PASSENGER_SYNC);

remote_raw_sync!(
    /// 211 PlayerPassengerSync, server→client: `u16 playerId` + raw[24] as
    /// [`PassengerSync`]; no timestamp handling. The client takes the seat from
    /// `byte2 & 0x3f` and applies drive-by and cuff state. Mirrors samp.dll 0x10009d30.
    RemotePassengerSync,
    PassengerSync
);

// ---------------------------------------------------------------------------
// 209 PlayerUnoccupiedSync

/// 209 PlayerUnoccupiedSync (spec §2 "209"): `u8 209` + raw[67];
/// unreliable-sequenced, channel 1. Mirrors samp.dll 0x10004d30.
///
/// `seat` is 0 when nobody is in the car, `k + 1` when the local player sits
/// in passenger seat `k`. `roll` is the vehicle matrix right vector and
/// `direction` its forward vector (open.mp calls them Roll/Rotation).
/// Client behaviour: at most 3 sends per vehicle-pool tick, > 100 ms per
/// vehicle, only for moving driverless vehicles within 90 units (see
/// [`super::policy`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnoccupiedSync {
    pub vehicle_id: u16,
    pub seat: u8,
    pub roll: Vec3,
    pub direction: Vec3,
    pub position: Vec3,
    pub velocity: Vec3,
    pub turn_velocity: Vec3,
    pub health: f32,
}

impl UnoccupiedSync {
    pub const SIZE: usize = 67;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            vehicle_id: r.read_u16()?,
            seat: r.read_u8()?,
            roll: Vec3::read(r)?,
            direction: Vec3::read(r)?,
            position: Vec3::read(r)?,
            velocity: Vec3::read(r)?,
            turn_velocity: Vec3::read(r)?,
            health: r.read_f32()?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.seat);
        self.roll.write(w);
        self.direction.write(w);
        self.position.write(w);
        self.velocity.write(w);
        self.turn_velocity.write(w);
        w.write_f32(self.health);
    }
}

raw_sync_packet!(UnoccupiedSync, ids::UNOCCUPIED_SYNC);

remote_raw_sync!(
    /// 209 PlayerUnoccupiedSync, server→client (mirrors samp.dll 0x10009a40): `u16
    /// playerId` + raw[67] as [`UnoccupiedSync`]. Client validation (not
    /// applied here): vehicle 1..1999, exists, no driver, not towed; roll and
    /// direction components in [−1, 1]; position x, y in (−20000, 20000), z in
    /// (−10000, 100000); velocity and turn velocity components in (−100, 100).
    /// Agrees with open.mp.
    RemoteUnoccupiedSync,
    UnoccupiedSync
);

// ---------------------------------------------------------------------------
// 210 PlayerTrailerSync

/// 210 PlayerTrailerSync (spec §2 "210"): `u8 210` + raw[54];
/// unreliable-sequenced, channel 1, sent right after each 200 while a trailer
/// is attached; duplicate suppression as for 200. Mirrors samp.dll 0x100053d0.
///
/// The quaternion is (w, x, y, z), normalised; the client expects w first
/// (open.mp's component order for this field is unverified).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrailerSync {
    pub trailer_id: u16,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub turn_velocity: Vec3,
}

impl TrailerSync {
    pub const SIZE: usize = 54;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            trailer_id: r.read_u16()?,
            position: Vec3::read(r)?,
            rotation: Quat::read(r)?,
            velocity: Vec3::read(r)?,
            turn_velocity: Vec3::read(r)?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.trailer_id);
        self.position.write(w);
        self.rotation.write(w);
        self.velocity.write(w);
        self.turn_velocity.write(w);
    }
}

raw_sync_packet!(TrailerSync, ids::TRAILER_SYNC);

remote_raw_sync!(
    /// 210 PlayerTrailerSync, server→client (mirrors samp.dll 0x10009e20): `u16
    /// playerId` + raw[54] as [`TrailerSync`]. Applied only if the remote
    /// player has a vehicle and the trailer id (1..1999) is a towable model.
    RemoteTrailerSync,
    TrailerSync
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_util::{Bytes, b, bits};

    fn roundtrip<T: SyncPacket + PartialEq + std::fmt::Debug>(p: &T) -> BitWriter {
        let w = p.to_bits();
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        assert_eq!(r.read_u8(), Some(T::PACKET_ID));
        assert_eq!(T::read(&mut r).as_ref(), Some(p));
        assert_eq!(r.remaining(), 0);
        w
    }

    fn sample_driver() -> VehicleSync {
        VehicleSync {
            vehicle_id: 42,
            left_right: -128,
            up_down: 0,
            keys: 0x0001,
            rotation: Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 },
            position: Vec3 { x: 100.0, y: -50.0, z: 12.5 },
            velocity: Vec3 { x: 0.5, y: 0.0, z: 0.0 },
            vehicle_health: 1000.0,
            player_health: 100,
            player_armour: 0,
            weapon: WeaponByte::new(0, 0),
            siren: 1,
            landing_gear: 0,
            trailer_id: 7,
            extra: 0.25f32.to_bits(),
        }
    }

    #[test]
    fn driver_raw_layout() {
        let s = sample_driver();
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 63) * 8);
        assert_eq!(VehicleSync::SIZE, 63);
        let expected = Bytes::default()
            .u8(200)
            .u16(42) // 0 vehicleId
            .i16(-128) // 2
            .i16(0) // 4
            .u16(1) // 6 keys
            .f32s(&[1.0, 0.0, 0.0, 0.0]) // 8 rotation
            .f32s(&[100.0, -50.0, 12.5]) // 24 position
            .f32s(&[0.5, 0.0, 0.0]) // 36 velocity
            .f32(1000.0) // 48 vehicle health
            .u8(100) // 52
            .u8(0) // 53
            .u8(0) // 54 weapon
            .u8(1) // 55 siren
            .u8(0) // 56 gear
            .u16(7) // 57 trailer
            .f32(0.25) // 59 extra
            .0;
        assert_eq!(w.as_bytes(), expected);
    }

    #[test]
    fn remote_driver_bit_exact() {
        let f = |v: f32| b(&v.to_le_bytes());
        let spec = [
            b(&[200]),
            b(&[1, 0]),             // playerId 1
            b(&[42, 0]),            // vehicleId
            b(&[0x80, 0xff]),       // leftRight -128
            b(&[0, 0]),             // upDown
            b(&[1, 0]),             // keys
            "0000".into(),          // quat signs
            b(&[0, 0, 0, 0, 0, 0]), // identity
            f(100.0) + &f(-50.0) + &f(12.5),
            f(1.0),                                   // velocity magnitude 1
            b(&[0xff, 0xff, 0xff, 0x7f, 0xff, 0x7f]), // (1, ~0, ~0)
            b(&[0xe8, 0x03]),                         // vehicle health 1000
            b(&[0xf0]),                               // health 100, armour 0
            b(&[0x1f]),                               // weapon 31
            "1".into(),                               // siren
            "0".into(),                               // gear
            "1".into(),
            b(&[0x78, 0x56, 0x34, 0x12]), // extra
            "0".into(),                   // no trailer
        ]
        .concat();
        let (bytes, n) = bits(&spec);
        let mut r = BitReader::with_bit_len(&bytes, n);
        assert_eq!(r.read_u8(), Some(200));
        let p = RemoteVehicleSync::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!((p.player_id, p.vehicle_id, p.left_right, p.up_down, p.keys), (1, 42, -128, 0, 1));
        assert_eq!(p.rotation.to_quat(), Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 });
        assert_eq!(p.position, Vec3 { x: 100.0, y: -50.0, z: 12.5 });
        let v = p.velocity.to_vec3();
        assert!((v.x - 1.0).abs() < 1e-6 && v.y.abs() < 1e-4 && v.z.abs() < 1e-4);
        assert_eq!(p.vehicle_health, 1000);
        assert_eq!(p.health_armour.health(), 100.0);
        assert_eq!(p.weapon.weapon(), 31);
        assert!(p.siren && !p.landing_gear);
        assert_eq!((p.extra, p.trailer_id), (Some(0x1234_5678), None));
        let w = p.to_bits();
        assert_eq!((w.as_bytes(), w.bit_len()), (&bytes[..], n));
    }

    #[test]
    fn remote_driver_from_raw() {
        let c = RemoteVehicleSync::from_raw(9, &sample_driver());
        roundtrip(&c);
        assert_eq!(c.vehicle_health, 1000);
        assert_eq!((c.extra, c.trailer_id), (Some(0.25f32.to_bits()), Some(7)));
        assert!(c.siren && !c.landing_gear);
        let mut s = sample_driver();
        s.extra = 0;
        s.trailer_id = 0;
        let c = RemoteVehicleSync::from_raw(9, &s);
        assert_eq!((c.extra, c.trailer_id), (None, None));
        // minimal compact size: 9 + 4*16 + 52 + 96 + 32 (gated vel) + 16 + 8 + 8 + 4 bits
        let w = roundtrip(&RemoteVehicleSync::default());
        assert_eq!(w.bit_len(), 8 + 16 + 64 + 52 + 96 + 32 + 16 + 8 + 8 + 4);
    }

    #[test]
    fn passenger_layout() {
        let s = PassengerSync {
            vehicle_id: 0x0203,
            seat: 2,
            drive_by: true,
            cuffed: false,
            weapon: WeaponByte::new(29, 0),
            health: 75,
            armour: 10,
            left_right: 0,
            up_down: -128,
            keys: 4,
            position: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
        };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 24) * 8);
        assert_eq!(PassengerSync::SIZE, 24);
        let expected = Bytes::default()
            .u8(211)
            .u16(0x0203)
            .u8(0b0100_0010)
            .u8(29)
            .u8(75)
            .u8(10)
            .i16(0)
            .i16(-128)
            .u16(4)
            .f32s(&[1.0, 2.0, 3.0])
            .0;
        assert_eq!(w.as_bytes(), expected);
        let mut bytes = expected.clone();
        bytes[3] = 0b1000_0101;
        let back = PassengerSync::read_body(&mut BitReader::new(&bytes[1..])).unwrap();
        assert_eq!((back.seat, back.drive_by, back.cuffed), (5, false, true));
        let w = roundtrip(&RemotePassengerSync { player_id: 4, sync: s });
        assert_eq!(&w.as_bytes()[..3], &[211, 4, 0]);
        assert_eq!(&w.as_bytes()[3..], &expected[1..]);
    }

    #[test]
    fn unoccupied_layout() {
        let s = UnoccupiedSync {
            vehicle_id: 500,
            seat: 1,
            roll: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            direction: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            position: Vec3 { x: 10.0, y: 20.0, z: 30.0 },
            velocity: Vec3 { x: 0.1, y: 0.0, z: 0.0 },
            turn_velocity: Vec3 { x: 0.0, y: 0.0, z: 0.01 },
            health: 650.0,
        };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 67) * 8);
        assert_eq!(UnoccupiedSync::SIZE, 67);
        let expected = Bytes::default()
            .u8(209)
            .u16(500)
            .u8(1)
            .f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.01])
            .f32(650.0)
            .0;
        assert_eq!(w.as_bytes(), expected);
        let w = roundtrip(&RemoteUnoccupiedSync { player_id: 2, sync: s });
        assert_eq!(w.bit_len(), (3 + 67) * 8);
    }

    #[test]
    fn trailer_layout() {
        let s = TrailerSync {
            trailer_id: 1999,
            position: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            rotation: Quat { w: 0.5, x: 0.5, y: -0.5, z: 0.5 },
            velocity: Vec3 { x: 0.0, y: 0.25, z: 0.0 },
            turn_velocity: Vec3 { x: 0.0, y: 0.0, z: -0.125 },
        };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 54) * 8);
        assert_eq!(TrailerSync::SIZE, 54);
        let expected = Bytes::default()
            .u8(210)
            .u16(1999)
            .f32s(&[1.0, 2.0, 3.0])
            .f32s(&[0.5, 0.5, -0.5, 0.5]) // w first
            .f32s(&[0.0, 0.25, 0.0, 0.0, 0.0, -0.125])
            .0;
        assert_eq!(w.as_bytes(), expected);
        let w = roundtrip(&RemoteTrailerSync { player_id: 1004, sync: s });
        assert_eq!(&w.as_bytes()[1..3], &1004u16.to_le_bytes());
    }
}
