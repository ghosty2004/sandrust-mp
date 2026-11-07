//! Quantised encodings used by the compact server→client sync forms.
//!
//! The client only ever *decodes* these (it sends raw structs); the encoders here are the implied
//! inverses (stock RakNet behaviour), marked as such, so tests and server tools can build S→C
//! packets. The wire types keep the quantised values exactly as transmitted (a decoded packet
//! re-encodes to the same bits); conversion to floats uses the client's own constants.
//! Spec: docs/re/subsystems/remote-players-sync.md §0.2.

use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Quat, Vec3};

/// `1/65535` as the client's quaternion reader uses it. Mirrors samp.dll 0x10009680.
pub const QUAT_COMPONENT_SCALE: f32 = 1.525_902_2e-5;
/// `1/32767.5` as the client's compressed-vector reader uses it. Mirrors samp.dll 0x1000a3b0.
pub const VEC_COMPONENT_SCALE: f32 = 3.051_804_4e-5;
/// Magnitude gate of the compressed vector: at or below it no components follow.
pub const VEC_MAGNITUDE_EPSILON: f32 = 1e-5;

/// RakNet "normalised quaternion" (§0.2): four sign bits (w, x, y, z), then
/// `|x|`, `|y|`, `|z|` each quantised to u16; `w` is reconstructed.
///
/// Wire: `wNeg bit, xNeg bit, yNeg bit, zNeg bit, x u16, y u16, z u16` (52 bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormQuat {
    pub w_neg: bool,
    pub x_neg: bool,
    pub y_neg: bool,
    pub z_neg: bool,
    /// `trunc(|x| * 65535)`.
    pub x: u16,
    pub y: u16,
    pub z: u16,
}

impl NormQuat {
    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            w_neg: r.read_bit()?,
            x_neg: r.read_bit()?,
            y_neg: r.read_bit()?,
            z_neg: r.read_bit()?,
            x: r.read_u16()?,
            y: r.read_u16()?,
            z: r.read_u16()?,
        })
    }

    pub fn write(&self, w: &mut BitWriter) {
        w.write_bit(self.w_neg);
        w.write_bit(self.x_neg);
        w.write_bit(self.y_neg);
        w.write_bit(self.z_neg);
        w.write_u16(self.x);
        w.write_u16(self.y);
        w.write_u16(self.z);
    }

    /// Decode as the client does: `c = q * (1/65535)`, signs applied, and
    /// `w = ±sqrt(max(0, 1 − x² − y² − z²))`. Result in (w, x, y, z) order.
    pub fn to_quat(self) -> Quat {
        let comp = |q: u16, neg: bool| {
            let v = f32::from(q) * QUAT_COMPONENT_SCALE;
            if neg { -v } else { v }
        };
        let x = comp(self.x, self.x_neg);
        let y = comp(self.y, self.y_neg);
        let z = comp(self.z, self.z_neg);
        let w = ((1.0 - x * x) - y * y - z * z).max(0.0).sqrt();
        Quat { w: if self.w_neg { -w } else { w }, x, y, z }
    }

    /// Implied encoder (not observed in the client): sign bits from `< 0`,
    /// magnitudes `trunc(|c| * 65535)` (saturating). Assumes a unit quaternion.
    pub fn from_quat(q: Quat) -> Self {
        let mag = |c: f32| (c.abs() * 65535.0) as u16;
        Self {
            w_neg: q.w < 0.0,
            x_neg: q.x < 0.0,
            y_neg: q.y < 0.0,
            z_neg: q.z < 0.0,
            x: mag(q.x),
            y: mag(q.y),
            z: mag(q.z),
        }
    }
}

/// RakNet "compressed vector" (§0.2), used for velocities.
///
/// Wire: `magnitude f32`, then — only if `magnitude > 1e-5` — three u16
/// components `c` with `v_i = (c * (1/32767.5) − 1) * magnitude`.
///
/// The client tests `magnitude <= 1e-5` to skip the components; this codec
/// uses exactly that predicate on both sides (so a NaN magnitude carries
/// components). When the components are absent they read as 0 here and are
/// ignored by [`CompressedVec3::to_vec3`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CompressedVec3 {
    pub magnitude: f32,
    /// Quantised direction components; only on the wire when
    /// [`CompressedVec3::has_components`].
    pub components: [u16; 3],
}

impl CompressedVec3 {
    /// True when the components follow the magnitude on the wire.
    pub fn has_components(&self) -> bool {
        // Not `magnitude > EPSILON`: the client's predicate is `<=`, so NaN
        // counts as "components present".
        self.magnitude.partial_cmp(&VEC_MAGNITUDE_EPSILON).is_none_or(|o| o.is_gt())
    }

    pub fn read(r: &mut BitReader) -> Option<Self> {
        let mut v = Self { magnitude: r.read_f32()?, components: [0; 3] };
        if v.has_components() {
            for c in &mut v.components {
                *c = r.read_u16()?;
            }
        }
        Some(v)
    }

    pub fn write(&self, w: &mut BitWriter) {
        w.write_f32(self.magnitude);
        if self.has_components() {
            for &c in &self.components {
                w.write_u16(c);
            }
        }
    }

    /// Decode as the client does; `(0, 0, 0)` when the magnitude is gated.
    pub fn to_vec3(self) -> Vec3 {
        if !self.has_components() {
            return Vec3::default();
        }
        let c = |q: u16| (f32::from(q) * VEC_COMPONENT_SCALE - 1.0) * self.magnitude;
        Vec3 { x: c(self.components[0]), y: c(self.components[1]), z: c(self.components[2]) }
    }

    /// Implied encoder (not observed in the client): magnitude = |v|;
    /// `c = trunc((clamp(v_i/|v|, −1, 1) + 1) * 32767.5)` when above the gate.
    pub fn from_vec3(v: Vec3) -> Self {
        let magnitude = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        let mut out = Self { magnitude, components: [0; 3] };
        if out.has_components() {
            let q = |c: f32| (((c / magnitude).clamp(-1.0, 1.0) + 1.0) * 32767.5) as u16;
            out.components = [q(v.x), q(v.y), q(v.z)];
        }
        out
    }
}

/// Packed health/armour byte (§0.2): high nibble health, low nibble armour.
///
/// Nibble `n` decodes to `0` if `n == 0`, `100` if `n == 15`, else `7 * n`.
/// The client stores the decoded values as floats for name-tag bars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedHealthArmour(pub u8);

impl PackedHealthArmour {
    pub fn read(r: &mut BitReader) -> Option<Self> {
        r.read_u8().map(Self)
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.0);
    }

    pub fn health_nibble(self) -> u8 {
        self.0 >> 4
    }
    pub fn armour_nibble(self) -> u8 {
        self.0 & 0x0f
    }
    /// Decoded health (0, 7..98 or 100).
    pub fn health(self) -> f32 {
        Self::decode_nibble(self.health_nibble())
    }
    /// Decoded armour (0, 7..98 or 100).
    pub fn armour(self) -> f32 {
        Self::decode_nibble(self.armour_nibble())
    }

    pub fn decode_nibble(n: u8) -> f32 {
        match n & 0x0f {
            0 => 0.0,
            15 => 100.0,
            n => f32::from(7 * n),
        }
    }

    /// Implied encoder (not observed in the client): `15` if value ≥ 100,
    /// otherwise `trunc(value / 7)` (negative values give 0).
    pub fn encode_nibble(value: f32) -> u8 {
        if value >= 100.0 { 15 } else { ((value / 7.0) as u8).min(14) }
    }

    /// Implied encoder for a (health, armour) pair.
    pub fn from_values(health: f32, armour: f32) -> Self {
        Self((Self::encode_nibble(health) << 4) | Self::encode_nibble(armour))
    }
}

/// Weapon byte (§0.2): low 6 bits weapon id, high 2 bits "additional key"
/// (§0.3: 1 DPadRight, 2 DPadLeft, 3 DPadDown). The S→C readers of the client
/// keep only the weapon id; the full byte is preserved here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponByte(pub u8);

impl WeaponByte {
    /// Pack; `weapon` is masked to 6 bits and `additional_key` to 2 bits.
    pub fn new(weapon: u8, additional_key: u8) -> Self {
        Self((weapon & 0x3f) | ((additional_key & 0x03) << 6))
    }
    pub fn weapon(self) -> u8 {
        self.0 & 0x3f
    }
    pub fn additional_key(self) -> u8 {
        self.0 >> 6
    }
    pub fn read(r: &mut BitReader) -> Option<Self> {
        r.read_u8().map(Self)
    }
    pub fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_util::bits;

    #[test]
    fn norm_quat_bit_exact() {
        // wNeg=0 xNeg=1 yNeg=0 zNeg=1, x=0xffff, y=0x0001, z=0x8000 (LE bytes).
        let (bytes, n) = bits("0101 11111111 11111111  00000001 00000000  00000000 10000000");
        assert_eq!(n, 52);
        let q = NormQuat::read(&mut BitReader::with_bit_len(&bytes, n)).unwrap();
        assert_eq!(q, NormQuat { w_neg: false, x_neg: true, y_neg: false, z_neg: true, x: 0xffff, y: 1, z: 0x8000 });
        let mut w = BitWriter::new();
        q.write(&mut w);
        assert_eq!((w.as_bytes(), w.bit_len()), (&bytes[..], n));
    }

    #[test]
    fn norm_quat_decode_values() {
        // |x| = 1 → w clamps to 0.
        let q = NormQuat { x_neg: true, x: 0xffff, ..Default::default() }.to_quat();
        assert!((q.x + 1.0).abs() < 1e-6);
        assert_eq!(q.w, 0.0);
        // all zero → identity
        assert_eq!(NormQuat::default().to_quat(), Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 });
        // w sign
        let q = NormQuat { w_neg: true, ..Default::default() }.to_quat();
        assert_eq!(q.w, -1.0);
    }

    #[test]
    fn norm_quat_roundtrip_values() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        for q in [
            Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 },
            Quat { w: s, x: 0.0, y: 0.0, z: -s },
            Quat { w: -0.5, x: 0.5, y: -0.5, z: 0.5 },
        ] {
            let d = NormQuat::from_quat(q).to_quat();
            for (a, b) in [(q.w, d.w), (q.x, d.x), (q.y, d.y), (q.z, d.z)] {
                assert!((a - b).abs() < 1e-3, "{q:?} -> {d:?}");
            }
        }
    }

    #[test]
    fn compressed_vec_gate() {
        // magnitude exactly 1e-5: only the f32 is on the wire.
        let v = CompressedVec3 { magnitude: 1e-5, components: [0; 3] };
        let mut w = BitWriter::new();
        v.write(&mut w);
        assert_eq!(w.bit_len(), 32);
        assert_eq!(w.as_bytes(), 1e-5f32.to_le_bytes());
        assert_eq!(v.to_vec3(), Vec3::default());
        // trailing data after a gated magnitude is not consumed
        let mut bytes = 0.0f32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0xaa, 0xbb]);
        let mut r = BitReader::new(&bytes);
        assert_eq!(CompressedVec3::read(&mut r), Some(CompressedVec3::default()));
        assert_eq!(r.remaining(), 16);
        // just above the gate: three u16 follow
        let v = CompressedVec3 { magnitude: 2e-5, components: [1, 2, 3] };
        let mut w = BitWriter::new();
        v.write(&mut w);
        assert_eq!(w.bit_len(), 32 + 48);
        assert_eq!(&w.as_bytes()[4..], &[1, 0, 2, 0, 3, 0]);
        assert_eq!(CompressedVec3::read(&mut BitReader::new(w.as_bytes())), Some(v));
        // client predicate is `magnitude <= 1e-5`, so NaN carries components
        assert!(CompressedVec3 { magnitude: f32::NAN, components: [0; 3] }.has_components());
        assert!(!CompressedVec3 { magnitude: -1.0, components: [0; 3] }.has_components());
    }

    #[test]
    fn compressed_vec_decode_values() {
        // magnitude 2.0 (0x40000000), c = 0xffff, 0x8000, 0x0000
        let bytes = [0x00, 0x00, 0x00, 0x40, 0xff, 0xff, 0x00, 0x80, 0x00, 0x00];
        let v = CompressedVec3::read(&mut BitReader::new(&bytes)).unwrap();
        assert_eq!(v.components, [0xffff, 0x8000, 0]);
        let d = v.to_vec3();
        assert!((d.x - 2.0).abs() < 1e-3);
        assert!(d.y.abs() < 1e-3);
        assert!((d.z + 2.0).abs() < 1e-6);
    }

    #[test]
    fn compressed_vec_roundtrip_values() {
        for v in [Vec3 { x: 0.0, y: 0.0, z: 0.0 }, Vec3 { x: 0.3, y: -0.2, z: 0.05 }, Vec3 { x: -10.0, y: 4.0, z: 0.0 }]
        {
            let c = CompressedVec3::from_vec3(v);
            let mut w = BitWriter::new();
            c.write(&mut w);
            let back = CompressedVec3::read(&mut BitReader::new(w.as_bytes())).unwrap();
            assert_eq!(back, c);
            let d = back.to_vec3();
            for (a, b) in [(v.x, d.x), (v.y, d.y), (v.z, d.z)] {
                assert!((a - b).abs() < 1e-3 * c.magnitude.max(1.0), "{v:?} -> {d:?}");
            }
        }
    }

    #[test]
    fn packed_health_armour() {
        let p = PackedHealthArmour(0xf7);
        assert_eq!((p.health(), p.armour()), (100.0, 49.0));
        let p = PackedHealthArmour(0x0e);
        assert_eq!((p.health(), p.armour()), (0.0, 98.0));
        assert_eq!(PackedHealthArmour::from_values(100.0, 0.0), PackedHealthArmour(0xf0));
        assert_eq!(PackedHealthArmour::from_values(99.9, 7.0), PackedHealthArmour(0xe1));
        assert_eq!(PackedHealthArmour::from_values(-5.0, 250.0), PackedHealthArmour(0x0f));
    }

    #[test]
    fn weapon_byte() {
        let b = WeaponByte::new(31, 2);
        assert_eq!(b.0, 0b1001_1111);
        assert_eq!((b.weapon(), b.additional_key()), (31, 2));
    }
}
