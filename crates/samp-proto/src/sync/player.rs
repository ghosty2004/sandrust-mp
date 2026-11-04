//! On-foot / player sync packets: 207 foot, 203 aim, 206 bullet, 212 spectator,
//! 204 weapons, 205 stats, 208 markers.
//!
//! Spec: docs/re/subsystems/remote-players-sync.md §2.

use super::encoding::{CompressedVec3, NormQuat, PackedHealthArmour, WeaponByte};
use super::{SyncPacket, ids};
use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Quat, Vec3};

/// Animation word: on the wire one u32 (C→S raw) = `index u16` then `flags u16`
/// (little-endian), which is bit-identical to open.mp's two u16 writes.
///
/// Client behaviour (§2.1): a value whose u32 is negative (flags bit 15) is
/// treated as "no animation".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Animation {
    pub index: u16,
    pub flags: u16,
}

impl Animation {
    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { index: r.read_u16()?, flags: r.read_u16()? })
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.index);
        w.write_u16(self.flags);
    }
    /// The client's u32 view (`index | flags << 16`).
    pub fn as_u32(self) -> u32 {
        u32::from(self.index) | (u32::from(self.flags) << 16)
    }
    pub fn from_u32(v: u32) -> Self {
        Self { index: v as u16, flags: (v >> 16) as u16 }
    }
}

// ---------------------------------------------------------------------------
// 207 PlayerFootSync

/// 207 PlayerFootSync, client→server (spec §2 "207 PlayerFootSync"):
/// `u8 207` + raw[68], sent unreliable-sequenced on channel 1 (see
/// [`super::policy`]).
///
/// Client behaviour: sent when keys/lr/ud change or the on-foot interval
/// elapses; identical structs within 501 ms are suppressed. The rotation is
/// the normalised ped matrix quaternion, reused from the last send if every
/// component moved by < 1e-5. Health/armour are truncated to integers.
/// `surf_id` is 0 when not surfing, a vehicle id 1..1999, or 2000 + object id.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FootSync {
    /// Left stick X.
    pub left_right: i16,
    /// Left stick Y.
    pub up_down: i16,
    /// Key word (§0.3).
    pub keys: u16,
    pub position: Vec3,
    /// (w, x, y, z).
    pub rotation: Quat,
    pub health: u8,
    pub armour: u8,
    pub weapon: WeaponByte,
    pub special_action: u8,
    pub velocity: Vec3,
    /// Position in the surfed entity's local frame; zero when not surfing.
    pub surf_offset: Vec3,
    pub surf_id: u16,
    pub animation: Animation,
}

impl FootSync {
    /// Raw body size in bytes (after the id byte).
    pub const SIZE: usize = 68;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            left_right: r.read_u16()? as i16,
            up_down: r.read_u16()? as i16,
            keys: r.read_u16()?,
            position: Vec3::read(r)?,
            rotation: Quat::read(r)?,
            health: r.read_u8()?,
            armour: r.read_u8()?,
            weapon: WeaponByte::read(r)?,
            special_action: r.read_u8()?,
            velocity: Vec3::read(r)?,
            surf_offset: Vec3::read(r)?,
            surf_id: r.read_u16()?,
            animation: Animation::read(r)?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.left_right as u16);
        w.write_u16(self.up_down as u16);
        w.write_u16(self.keys);
        self.position.write(w);
        self.rotation.write(w);
        w.write_u8(self.health);
        w.write_u8(self.armour);
        self.weapon.write(w);
        w.write_u8(self.special_action);
        self.velocity.write(w);
        self.surf_offset.write(w);
        w.write_u16(self.surf_id);
        self.animation.write(w);
    }
}

raw_sync_packet!(FootSync, ids::FOOT_SYNC);

/// Surfing data of a remote foot sync: the surfed entity id (vehicle 1..1999,
/// or 2000 + object id) and the offset in its local frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Surf {
    pub id: u16,
    pub offset: Vec3,
}

/// 207 PlayerFootSync, server→client compact form (spec §2 "207", S→C table;
/// reader A 0x1000a740). May carry a `0x28` timestamp prefix (see
/// [`super::decode`]).
///
/// Wire: `u16 playerId`, `hasLR bit [+ u16]`, `hasUD bit [+ u16]`, `u16 keys`,
/// position raw[12], normalised quaternion, packed health/armour, weapon byte,
/// `u8 specialAction`, compressed velocity, `hasSurf bit [+ u16 surfId,
/// raw[12] offset]`, `hasAnim bit [+ u32 animation]`.
///
/// Client behaviour: absent lr/ud/animation read as 0; absent surf data sets
/// the stored surf id to 0xffff; the weapon's additional-key bits are
/// discarded; health/armour nibbles become floats. Agrees with open.mp (which
/// sets each presence bit when the value is non-zero).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteFootSync {
    pub player_id: u16,
    /// `None` when the presence bit is 0 (client uses 0).
    pub left_right: Option<i16>,
    pub up_down: Option<i16>,
    pub keys: u16,
    pub position: Vec3,
    pub rotation: NormQuat,
    pub health_armour: PackedHealthArmour,
    pub weapon: WeaponByte,
    pub special_action: u8,
    pub velocity: CompressedVec3,
    pub surf: Option<Surf>,
    pub animation: Option<Animation>,
}

impl SyncPacket for RemoteFootSync {
    const PACKET_ID: u8 = ids::FOOT_SYNC;

    fn read(r: &mut BitReader) -> Option<Self> {
        let player_id = r.read_u16()?;
        let left_right = if r.read_bit()? { Some(r.read_u16()? as i16) } else { None };
        let up_down = if r.read_bit()? { Some(r.read_u16()? as i16) } else { None };
        Some(Self {
            player_id,
            left_right,
            up_down,
            keys: r.read_u16()?,
            position: Vec3::read(r)?,
            rotation: NormQuat::read(r)?,
            health_armour: PackedHealthArmour::read(r)?,
            weapon: WeaponByte::read(r)?,
            special_action: r.read_u8()?,
            velocity: CompressedVec3::read(r)?,
            surf: if r.read_bit()? { Some(Surf { id: r.read_u16()?, offset: Vec3::read(r)? }) } else { None },
            animation: if r.read_bit()? { Some(Animation::read(r)?) } else { None },
        })
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        w.write_u16(self.player_id);
        for v in [self.left_right, self.up_down] {
            w.write_bit(v.is_some());
            if let Some(v) = v {
                w.write_u16(v as u16);
            }
        }
        w.write_u16(self.keys);
        self.position.write(w);
        self.rotation.write(w);
        self.health_armour.write(w);
        self.weapon.write(w);
        w.write_u8(self.special_action);
        self.velocity.write(w);
        w.write_bit(self.surf.is_some());
        if let Some(s) = &self.surf {
            w.write_u16(s.id);
            s.offset.write(w);
        }
        w.write_bit(self.animation.is_some());
        if let Some(a) = &self.animation {
            a.write(w);
        }
    }
}

impl RemoteFootSync {
    /// Build the compact form from a raw foot struct, the way a server would
    /// (presence bits set for non-zero values; implied encoders for the
    /// quaternion, velocity and health/armour). Not observed in the client.
    pub fn from_raw(player_id: u16, s: &FootSync) -> Self {
        let nz = |v: i16| (v != 0).then_some(v);
        Self {
            player_id,
            left_right: nz(s.left_right),
            up_down: nz(s.up_down),
            keys: s.keys,
            position: s.position,
            rotation: NormQuat::from_quat(s.rotation),
            health_armour: PackedHealthArmour::from_values(s.health.into(), s.armour.into()),
            weapon: s.weapon,
            special_action: s.special_action,
            velocity: CompressedVec3::from_vec3(s.velocity),
            surf: (s.surf_id != 0).then_some(Surf { id: s.surf_id, offset: s.surf_offset }),
            animation: (s.animation != Animation::default()).then_some(s.animation),
        }
    }
}

// ---------------------------------------------------------------------------
// 203 PlayerAimSync

/// 203 PlayerAimSync, client→server (spec §2 "203 PlayerAimSync"): `u8 203` +
/// raw[31]; unreliable-sequenced, channel 1.
///
/// Client behaviour: `zoom = trunc(((FOV − 35) / 35) × 63)`; `weapon_state` =
/// 3 while reloading, else `min(ammoInClip, 2)`; `aspect =
/// trunc((aspectRatio − 1) × 255)`. Rates in [`super::policy`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimSync {
    /// Active GTA camera mode.
    pub cam_mode: u8,
    pub cam_front: Vec3,
    pub cam_pos: Vec3,
    /// Ped aim pitch.
    pub aim_z: f32,
    /// Low 6 bits of byte 29.
    pub zoom: u8,
    /// High 2 bits of byte 29.
    pub weapon_state: u8,
    pub aspect: u8,
}

impl AimSync {
    pub const SIZE: usize = 31;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        let cam_mode = r.read_u8()?;
        let cam_front = Vec3::read(r)?;
        let cam_pos = Vec3::read(r)?;
        let aim_z = r.read_f32()?;
        let zs = r.read_u8()?;
        Some(Self { cam_mode, cam_front, cam_pos, aim_z, zoom: zs & 0x3f, weapon_state: zs >> 6, aspect: r.read_u8()? })
    }

    /// `zoom` is masked to 6 bits and `weapon_state` to 2 bits.
    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u8(self.cam_mode);
        self.cam_front.write(w);
        self.cam_pos.write(w);
        w.write_f32(self.aim_z);
        w.write_u8((self.zoom & 0x3f) | ((self.weapon_state & 0x03) << 6));
        w.write_u8(self.aspect);
    }
}

raw_sync_packet!(AimSync, ids::AIM_SYNC);

remote_raw_sync!(
    /// 203 PlayerAimSync, server→client (reader A 0x10009c40): `u16 playerId`
    /// + raw[31] as [`AimSync`]; no timestamp handling. The client stores zoom
    /// as `zoom/63`, aspect as `aspect/255`, and maps weapon state 3 to
    /// RELOADING. Agrees with open.mp.
    RemoteAimSync,
    AimSync
);

// ---------------------------------------------------------------------------
// 206 PlayerBulletSync

/// Bullet hit types (byte 0 of [`BulletSync`]).
pub mod hit_type {
    pub const NONE: u8 = 0;
    pub const PLAYER: u8 = 1;
    pub const VEHICLE: u8 = 2;
    pub const OBJECT: u8 = 3;
}

/// 206 PlayerBulletSync, client→server (spec §2 "206 PlayerBulletSync"):
/// `u8 206` + raw[40]; unreliable-sequenced, channel 0; one per hitscan shot,
/// no throttle.
///
/// Client behaviour: `hit_id` is 0xffff when nothing was hit; `offset` is the
/// hit point in the entity's local frame (lag-comp mode 0) or hit position
/// minus entity position, or world coordinates when the entity is unknown
/// (hit type then stays 0). Not sent for the sniper rifle (34) when nothing
/// was hit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BulletSync {
    /// See [`hit_type`].
    pub hit_type: u8,
    pub hit_id: u16,
    pub origin: Vec3,
    pub hit_pos: Vec3,
    pub offset: Vec3,
    pub weapon_id: u8,
}

impl BulletSync {
    pub const SIZE: usize = 40;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            hit_type: r.read_u8()?,
            hit_id: r.read_u16()?,
            origin: Vec3::read(r)?,
            hit_pos: Vec3::read(r)?,
            offset: Vec3::read(r)?,
            weapon_id: r.read_u8()?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u8(self.hit_type);
        w.write_u16(self.hit_id);
        self.origin.write(w);
        self.hit_pos.write(w);
        self.offset.write(w);
        w.write_u8(self.weapon_id);
    }
}

raw_sync_packet!(BulletSync, ids::BULLET_SYNC);

remote_raw_sync!(
    /// 206 PlayerBulletSync, server→client (reader A 0x10009b40): `u16
    /// playerId` + raw[40] as [`BulletSync`]. Client checks: shooter id
    /// `< 1004`; when `hit_type != 0` every offset component must lie in
    /// [−300, 300] or the bullet is dropped. Agrees with open.mp.
    RemoteBulletSync,
    BulletSync
);

// ---------------------------------------------------------------------------
// 212 PlayerSpectatorSync

/// 212 PlayerSpectatorSync, client→server only (spec §2 "212"): `u8 212` +
/// raw[18]; unreliable, channel 1, every on-foot interval while spectating.
/// `position` is the camera matrix translation. Agrees with open.mp's 18-byte
/// read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpectatorSync {
    pub left_right: i16,
    pub up_down: i16,
    pub keys: u16,
    pub position: Vec3,
}

impl SpectatorSync {
    pub const SIZE: usize = 18;

    pub fn read_body(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            left_right: r.read_u16()? as i16,
            up_down: r.read_u16()? as i16,
            keys: r.read_u16()?,
            position: Vec3::read(r)?,
        })
    }

    pub fn write_body(&self, w: &mut BitWriter) {
        w.write_u16(self.left_right as u16);
        w.write_u16(self.up_down as u16);
        w.write_u16(self.keys);
        self.position.write(w);
    }
}

raw_sync_packet!(SpectatorSync, ids::SPECTATOR_SYNC);

// ---------------------------------------------------------------------------
// 204 PlayerWeaponsUpdate

/// One changed weapon slot in [`WeaponsUpdate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponSlot {
    /// Slot 0..12.
    pub slot: u8,
    pub weapon_id: u8,
    pub ammo: u16,
}

/// 204 PlayerWeaponsUpdate, client→server only (spec §2 "204"; send A
/// 0x10006290): `u8 204, u16 targetPlayer, u16 targetActor`, then
/// `slot u8, weaponId u8, ammo u16` per changed slot (0–13 records).
///
/// Client behaviour: evaluated every tick, delta-encoded against the last
/// copy, sent UNRELIABLE only if the targets changed or a slot record was
/// appended (so a lost packet is never repeated). Targets are 0xffff for none.
/// The reader consumes records until fewer than 8 bits remain (as open.mp
/// does); a partial record is malformed. No record cap is applied here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponsUpdate {
    pub target_player: u16,
    pub target_actor: u16,
    pub slots: Vec<WeaponSlot>,
}

impl SyncPacket for WeaponsUpdate {
    const PACKET_ID: u8 = ids::WEAPONS_UPDATE;

    fn read(r: &mut BitReader) -> Option<Self> {
        let mut p = Self { target_player: r.read_u16()?, target_actor: r.read_u16()?, slots: Vec::new() };
        while r.remaining() >= 8 {
            p.slots.push(WeaponSlot { slot: r.read_u8()?, weapon_id: r.read_u8()?, ammo: r.read_u16()? });
        }
        Some(p)
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        w.write_u16(self.target_player);
        w.write_u16(self.target_actor);
        for s in &self.slots {
            w.write_u8(s.slot);
            w.write_u8(s.weapon_id);
            w.write_u16(s.ammo);
        }
    }
}

// ---------------------------------------------------------------------------
// 205 PlayerStatsSync

/// 205 PlayerStatsSync, client→server only (spec §2 "205"; send A
/// 0x10005d00): `u8 205, i32 money, i32 drunkLevel`; unreliable, channel 0,
/// every > 1000 ms. Agrees with open.mp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatsSync {
    pub money: i32,
    pub drunk_level: i32,
}

impl SyncPacket for StatsSync {
    const PACKET_ID: u8 = ids::STATS_SYNC;

    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { money: r.read_u32()? as i32, drunk_level: r.read_u32()? as i32 })
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        w.write_u32(self.money as u32);
        w.write_u32(self.drunk_level as u32);
    }
}

// ---------------------------------------------------------------------------
// 208 PlayerMarkersSync

/// One record of [`MarkersSync`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marker {
    pub player_id: u16,
    /// `None` (hasPos 0): remove that player's marker blip. `Some`: integer
    /// world position (i16 each).
    pub position: Option<[i16; 3]>,
}

/// 208 PlayerMarkersSync, server→client only (spec §2 "208"; reader A
/// 0x10009f10). No player id after the packet id.
///
/// Wire: `u32 count`, then per record `u16 playerId, hasPos bit [+ i16 x, y,
/// z]`.
///
/// Client behaviour: records with ids `>= 1004` or unused slots are skipped;
/// a position creates a radar blip only if that player is not streamed in.
/// Agrees with open.mp.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkersSync {
    pub markers: Vec<Marker>,
}

impl SyncPacket for MarkersSync {
    const PACKET_ID: u8 = ids::MARKERS_SYNC;

    fn read(r: &mut BitReader) -> Option<Self> {
        let count = r.read_u32()?;
        let mut markers = Vec::new();
        for _ in 0..count {
            let player_id = r.read_u16()?;
            let position = if r.read_bit()? {
                Some([r.read_u16()? as i16, r.read_u16()? as i16, r.read_u16()? as i16])
            } else {
                None
            };
            markers.push(Marker { player_id, position });
        }
        Some(Self { markers })
    }

    fn write(&self, w: &mut BitWriter) {
        w.write_u8(Self::PACKET_ID);
        w.write_u32(self.markers.len() as u32);
        for m in &self.markers {
            w.write_u16(m.player_id);
            w.write_bit(m.position.is_some());
            if let Some(p) = m.position {
                for c in p {
                    w.write_u16(c as u16);
                }
            }
        }
    }
}

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

    fn sample_foot() -> FootSync {
        FootSync {
            left_right: -128,
            up_down: 128,
            keys: 0x0408,
            position: Vec3 { x: 1.5, y: -2.25, z: 10.0 },
            rotation: Quat { w: 0.5, x: -0.5, y: 0.5, z: -0.5 },
            health: 100,
            armour: 50,
            weapon: WeaponByte::new(24, 1),
            special_action: 2,
            velocity: Vec3 { x: 0.1, y: 0.2, z: -0.3 },
            surf_offset: Vec3 { x: 0.5, y: 0.0, z: 1.0 },
            surf_id: 2001,
            animation: Animation { index: 1189, flags: 0x8004 },
        }
    }

    #[test]
    fn foot_raw_size_and_layout() {
        let s = sample_foot();
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + FootSync::SIZE) * 8);
        assert_eq!(FootSync::SIZE, 68);
        let expected = Bytes::default()
            .u8(207)
            .i16(-128) // 0 leftRight
            .i16(128) // 2 upDown
            .u16(0x0408) // 4 keys
            .f32s(&[1.5, -2.25, 10.0]) // 6 position
            .f32s(&[0.5, -0.5, 0.5, -0.5]) // 18 rotation w,x,y,z
            .u8(100) // 34 health
            .u8(50) // 35 armour
            .u8(24 | (1 << 6)) // 36 weapon | addKey<<6
            .u8(2) // 37 special action
            .f32s(&[0.1, 0.2, -0.3]) // 38 velocity
            .f32s(&[0.5, 0.0, 1.0]) // 50 surf offset
            .u16(2001) // 62 surf id
            .u32(1189 | (0x8004 << 16)) // 64 animation
            .0;
        assert_eq!(w.as_bytes(), expected);
        assert_eq!(s.animation.as_u32(), 1189 | (0x8004 << 16));
        assert_eq!(Animation::from_u32(s.animation.as_u32()), s.animation);
    }

    #[test]
    fn remote_foot_bit_exact() {
        let f = |v: f32| b(&v.to_le_bytes());
        let spec = [
            b(&[207]),
            b(&[3, 0]),                   // playerId 3
            "1".into(),                   // hasLR
            b(&[0x02, 0x01]),             // leftRight 0x0102
            "0".into(),                   // hasUD
            b(&[0x04, 0x00]),             // keys 4
            f(1.0) + &f(0.0) + &f(0.0),   // position
            "0100".into(),                // quat signs w,x,y,z
            b(&[0xff, 0xff, 0, 0, 0, 0]), // qx=0xffff qy=0 qz=0
            b(&[0xf7]),                   // health 15 → 100, armour 7 → 49
            b(&[0x45]),                   // weapon 5, addKey 1
            b(&[0]),                      // special action
            f(0.0),                       // velocity magnitude 0 → no components
            "1".into(),                   // hasSurf
            b(&[0xd1, 0x07]),             // surfId 2001
            f(0.0) + &f(0.0) + &f(0.0),   // offset
            "1".into(),                   // hasAnim
            b(&[0x01, 0x00, 0x04, 0x00]), // index 1, flags 4
        ]
        .concat();
        let (bytes, n) = bits(&spec);
        let mut r = BitReader::with_bit_len(&bytes, n);
        assert_eq!(r.read_u8(), Some(207));
        let p = RemoteFootSync::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!(p.player_id, 3);
        assert_eq!(p.left_right, Some(0x0102));
        assert_eq!(p.up_down, None);
        assert_eq!(p.keys, 4);
        assert_eq!(p.position, Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        let q = p.rotation.to_quat();
        assert!((q.x + 1.0).abs() < 1e-6 && q.w == 0.0);
        assert_eq!((p.health_armour.health(), p.health_armour.armour()), (100.0, 49.0));
        assert_eq!((p.weapon.weapon(), p.weapon.additional_key()), (5, 1));
        assert_eq!(p.velocity.to_vec3(), Vec3::default());
        assert_eq!(p.surf, Some(Surf { id: 2001, offset: Vec3::default() }));
        assert_eq!(p.animation, Some(Animation { index: 1, flags: 4 }));
        let w = p.to_bits();
        assert_eq!((w.as_bytes(), w.bit_len()), (&bytes[..], n));
    }

    #[test]
    fn remote_foot_minimal_and_from_raw() {
        let p = RemoteFootSync::default();
        let w = roundtrip(&p);
        // id + pid + 2 bits + keys + pos + 52-bit quat + 3 bytes + f32 + 2 bits
        assert_eq!(w.bit_len(), 8 + 16 + 2 + 16 + 96 + 52 + 24 + 32 + 2);

        let raw = sample_foot();
        let c = RemoteFootSync::from_raw(7, &raw);
        roundtrip(&c);
        assert_eq!(c.left_right, Some(-128));
        assert_eq!(c.surf, Some(Surf { id: 2001, offset: raw.surf_offset }));
        assert_eq!(c.health_armour.health(), 100.0);
        assert_eq!(c.health_armour.armour(), 49.0);
        let v = c.velocity.to_vec3();
        assert!((v.z + 0.3).abs() < 1e-3);
        let q = c.rotation.to_quat();
        assert!((q.w - 0.5).abs() < 1e-3 && (q.x + 0.5).abs() < 1e-3);
        let none = RemoteFootSync::from_raw(7, &FootSync::default());
        assert_eq!((none.left_right, none.surf, none.animation), (None, None, None));
    }

    #[test]
    fn aim_layout() {
        let s = AimSync {
            cam_mode: 53,
            cam_front: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            cam_pos: Vec3 { x: 10.0, y: 20.0, z: 30.0 },
            aim_z: -0.25,
            zoom: 63,
            weapon_state: 3,
            aspect: 0x55,
        };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 31) * 8);
        assert_eq!(AimSync::SIZE, 31);
        let expected = Bytes::default()
            .u8(203)
            .u8(53)
            .f32s(&[0.0, 1.0, 0.0])
            .f32s(&[10.0, 20.0, 30.0])
            .f32(-0.25)
            .u8(0xff)
            .u8(0x55)
            .0;
        assert_eq!(w.as_bytes(), expected);
        // zoom low 6 bits, weapon state high 2
        let mut r = BitReader::new(&expected[1..]);
        let back = AimSync::read_body(&mut r).unwrap();
        assert_eq!((back.zoom, back.weapon_state), (63, 3));
        let mut bytes = expected.clone();
        bytes[30] = 0b1000_0101;
        let back = AimSync::read_body(&mut BitReader::new(&bytes[1..])).unwrap();
        assert_eq!((back.zoom, back.weapon_state), (5, 2));

        let remote = RemoteAimSync { player_id: 0x0102, sync: s };
        let w = roundtrip(&remote);
        let mut exp = vec![203, 0x02, 0x01];
        exp.extend_from_slice(&expected[1..]);
        assert_eq!(w.as_bytes(), exp);
    }

    #[test]
    fn bullet_layout() {
        let s = BulletSync {
            hit_type: hit_type::PLAYER,
            hit_id: 0xffff,
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            hit_pos: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            offset: Vec3 { x: 0.1, y: 0.2, z: 0.3 },
            weapon_id: 31,
        };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 40) * 8);
        assert_eq!(BulletSync::SIZE, 40);
        let expected =
            Bytes::default().u8(206).u8(1).u16(0xffff).f32s(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.1, 0.2, 0.3]).u8(31).0;
        assert_eq!(w.as_bytes(), expected);
        let remote = RemoteBulletSync { player_id: 1003, sync: s };
        let w = roundtrip(&remote);
        assert_eq!(w.bit_len(), (3 + 40) * 8);
        assert_eq!(&w.as_bytes()[1..3], &1003u16.to_le_bytes());
    }

    #[test]
    fn spectator_layout() {
        let s = SpectatorSync { left_right: -1, up_down: 2, keys: 8, position: Vec3 { x: 50.0, y: 50.0, z: 50.0 } };
        let w = roundtrip(&s);
        assert_eq!(w.bit_len(), (1 + 18) * 8);
        assert_eq!(SpectatorSync::SIZE, 18);
        let expected = Bytes::default().u8(212).i16(-1).i16(2).u16(8).f32s(&[50.0; 3]).0;
        assert_eq!(w.as_bytes(), expected);
    }

    #[test]
    fn weapons_update_layout() {
        let p = WeaponsUpdate {
            target_player: 0xffff,
            target_actor: 7,
            slots: vec![
                WeaponSlot { slot: 2, weapon_id: 24, ammo: 500 },
                WeaponSlot { slot: 0, weapon_id: 0, ammo: 1 },
            ],
        };
        let w = roundtrip(&p);
        let expected = Bytes::default().u8(204).u16(0xffff).u16(7).u8(2).u8(24).u16(500).u8(0).u8(0).u16(1).0;
        assert_eq!(w.as_bytes(), expected);
        let empty = WeaponsUpdate { target_player: 1, target_actor: 0xffff, slots: vec![] };
        assert_eq!(roundtrip(&empty).bit_len(), 5 * 8);
        // partial record is malformed
        let bad = Bytes::default().u16(1).u16(2).u8(3).u8(4).0;
        assert_eq!(WeaponsUpdate::read(&mut BitReader::new(&bad)), None);
    }

    #[test]
    fn stats_layout() {
        let p = StatsSync { money: -5, drunk_level: 2000 };
        let w = roundtrip(&p);
        assert_eq!(w.as_bytes(), Bytes::default().u8(205).u32(-5i32 as u32).u32(2000).0);
    }

    #[test]
    fn markers_bit_exact() {
        let spec = [
            b(&[208]),
            b(&[2, 0, 0, 0]), // count 2
            b(&[5, 0]),       // player 5
            "1".into(),
            b(&[0xff, 0xff, 0x02, 0x00, 0x00, 0x80]), // x -1, y 2, z -32768
            b(&[6, 0]),                               // player 6
            "0".into(),
        ]
        .concat();
        let (bytes, n) = bits(&spec);
        let mut r = BitReader::with_bit_len(&bytes, n);
        assert_eq!(r.read_u8(), Some(208));
        let p = MarkersSync::read(&mut r).unwrap();
        assert_eq!(
            p.markers,
            vec![Marker { player_id: 5, position: Some([-1, 2, i16::MIN]) }, Marker { player_id: 6, position: None }]
        );
        let w = p.to_bits();
        assert_eq!((w.as_bytes(), w.bit_len()), (&bytes[..], n));
        // count larger than the data → None
        let (bytes, n) = bits(&(b(&[9, 0, 0, 0]) + &b(&[1, 0]) + "0"));
        assert_eq!(MarkersSync::read(&mut BitReader::with_bit_len(&bytes, n)), None);
    }
}
