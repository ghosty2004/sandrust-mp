//! Primitive algorithms used by the archive: the word mask applied to directory
//! fields, the name hash, and the chained XTEA variant. Spec: `docs/re/samp-saa.md`.

/// XOR constant of the directory word mask (§3.2 of the spec). samp.dll stores it
/// split over two immediates (0x3D3E7 for the low 19 bits, 0xC107FFFF for the high 13
/// bits); combined they are this single 32-bit value.
pub const WORD_MASK_XOR: u32 = 0xC103_D3E7;

/// Rotation of the directory word mask.
pub const WORD_MASK_ROT: u32 = 13;

/// Undo the per-word obfuscation of a directory field: `rotl(x ^ K, 13)`.
pub fn unmask_word(x: u32) -> u32 {
    (x ^ WORD_MASK_XOR).rotate_left(WORD_MASK_ROT)
}

/// Inverse of [`unmask_word`] (used to build synthetic archives in tests).
pub fn mask_word(y: u32) -> u32 {
    y.rotate_right(WORD_MASK_ROT) ^ WORD_MASK_XOR
}

/// XTEA delta. samp.dll keeps it masked (0x0CCF225C at 0x101027B4, unmasked with
/// [`unmask_word`] on first use); the result is the usual golden-ratio constant.
pub const XTEA_DELTA: u32 = 0x9E37_79B9;

/// Number of XTEA cycles (0x101027B0 = 32).
pub const XTEA_ROUNDS: u32 = 32;

/// Name hash: Bob Jenkins' 1996 "lookup2" hash with initial value 0x12345678.
/// The golden-ratio seed is stored masked at 0x10102790 (same value as the XTEA delta).
pub fn name_hash(key: &[u8]) -> u32 {
    const INITVAL: u32 = 0x1234_5678;
    let mut a: u32 = XTEA_DELTA;
    let mut b: u32 = XTEA_DELTA;
    let mut c: u32 = INITVAL;

    fn word(b: &[u8]) -> u32 {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    let mut rest = key;
    while rest.len() >= 12 {
        a = a.wrapping_add(word(&rest[0..]));
        b = b.wrapping_add(word(&rest[4..]));
        c = c.wrapping_add(word(&rest[8..]));
        mix(&mut a, &mut b, &mut c);
        rest = &rest[12..];
    }
    // Low byte of `c` is reserved for the length; tail bytes 8..10 go above it.
    c = c.wrapping_add(key.len() as u32);
    for (i, &byte) in rest.iter().enumerate() {
        let v = u32::from(byte);
        match i {
            0..=3 => a = a.wrapping_add(v << (8 * i)),
            4..=7 => b = b.wrapping_add(v << (8 * (i - 4))),
            _ => c = c.wrapping_add(v << (8 * (i - 7))),
        }
    }
    mix(&mut a, &mut b, &mut c);
    c
}

fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    macro_rules! step {
        ($x:ident, $y:ident, $z:ident, $op:tt, $s:expr) => {
            *$x = $x.wrapping_sub(*$y).wrapping_sub(*$z) ^ (*$z $op $s);
        };
    }
    step!(a, b, c, >>, 13);
    step!(b, c, a, <<, 8);
    step!(c, a, b, >>, 13);
    step!(a, b, c, >>, 12);
    step!(b, c, a, <<, 16);
    step!(c, a, b, >>, 5);
    step!(a, b, c, >>, 3);
    step!(b, c, a, <<, 10);
    step!(c, a, b, >>, 15);
}

/// XTEA (32 cycles, little-endian words) with key feedback: after every 8-byte
/// block the key is XORed with that block's ciphertext words (c0, c1, c0, c1).
/// The key state carries over between calls, so one instance decrypts a whole
/// stream in consecutive pieces.
#[derive(Clone)]
pub struct ChainedXtea {
    key: [u32; 4],
}

impl ChainedXtea {
    pub fn new(key: &[u8; 16]) -> Self {
        let mut k = [0u32; 4];
        for (i, w) in k.iter_mut().enumerate() {
            *w = u32::from_le_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
        }
        Self { key: k }
    }

    fn feedback(&mut self, c0: u32, c1: u32) {
        self.key[0] ^= c0;
        self.key[1] ^= c1;
        self.key[2] ^= c0;
        self.key[3] ^= c1;
    }

    /// Decrypt `buf` in place. Only whole 8-byte blocks are processed; callers pass
    /// multiples of the 2048-byte archive block.
    pub fn decrypt(&mut self, buf: &mut [u8]) {
        for blk in buf.as_chunks_mut::<8>().0 {
            let c0 = u32::from_le_bytes([blk[0], blk[1], blk[2], blk[3]]);
            let c1 = u32::from_le_bytes([blk[4], blk[5], blk[6], blk[7]]);
            let (mut v0, mut v1) = (c0, c1);
            let mut sum = XTEA_DELTA.wrapping_mul(XTEA_ROUNDS);
            for _ in 0..XTEA_ROUNDS {
                v1 = v1.wrapping_sub(
                    (((v0 << 4) ^ (v0 >> 5)).wrapping_add(v0)) ^ sum.wrapping_add(self.key[((sum >> 11) & 3) as usize]),
                );
                sum = sum.wrapping_sub(XTEA_DELTA);
                v0 = v0.wrapping_sub(
                    (((v1 << 4) ^ (v1 >> 5)).wrapping_add(v1)) ^ sum.wrapping_add(self.key[(sum & 3) as usize]),
                );
            }
            blk[..4].copy_from_slice(&v0.to_le_bytes());
            blk[4..].copy_from_slice(&v1.to_le_bytes());
            self.feedback(c0, c1);
        }
    }

    /// Inverse of [`ChainedXtea::decrypt`] (for building synthetic archives).
    pub fn encrypt(&mut self, buf: &mut [u8]) {
        for blk in buf.as_chunks_mut::<8>().0 {
            let mut v0 = u32::from_le_bytes([blk[0], blk[1], blk[2], blk[3]]);
            let mut v1 = u32::from_le_bytes([blk[4], blk[5], blk[6], blk[7]]);
            let mut sum: u32 = 0;
            for _ in 0..XTEA_ROUNDS {
                v0 = v0.wrapping_add(
                    (((v1 << 4) ^ (v1 >> 5)).wrapping_add(v1)) ^ sum.wrapping_add(self.key[(sum & 3) as usize]),
                );
                sum = sum.wrapping_add(XTEA_DELTA);
                v1 = v1.wrapping_add(
                    (((v0 << 4) ^ (v0 >> 5)).wrapping_add(v0)) ^ sum.wrapping_add(self.key[((sum >> 11) & 3) as usize]),
                );
            }
            blk[..4].copy_from_slice(&v0.to_le_bytes());
            blk[4..].copy_from_slice(&v1.to_le_bytes());
            self.feedback(v0, v1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_constants_unmask_to_golden_ratio() {
        // Values as stored in samp.dll (0x10102790 / 0x101027B4).
        assert_eq!(unmask_word(0x0CCF_225C), XTEA_DELTA);
        // The split-immediate form used by samp is the same function.
        let x = 0xDEAD_BEEFu32;
        let split = ((x ^ 0x3D3E7) << 13) | ((x ^ 0xC107_FFFF) >> 19);
        assert_eq!(unmask_word(x), split);
        assert_eq!(mask_word(unmask_word(x)), x);
    }

    #[test]
    fn chained_roundtrip_multi_call() {
        let key = [7u8; 16];
        let plain: Vec<u8> = (0..4096u32).map(|i| (i * 31 + 7) as u8).collect();
        let mut buf = plain.clone();
        ChainedXtea::new(&key).encrypt(&mut buf);
        // Identical plaintext blocks must not give identical ciphertext (feedback).
        let mut d = ChainedXtea::new(&key);
        let (a, b) = buf.split_at_mut(2048);
        d.decrypt(a);
        d.decrypt(b);
        assert_eq!(buf, plain);
    }

    #[test]
    fn lookup2_known_values() {
        // The hash is case-sensitive; samp lowercases before hashing.
        assert_ne!(name_hash(b"main.scm"), name_hash(b"MAIN.SCM"));
        // Values observed as directory keys in a genuine 0.3.7-R4 samp.saa.
        assert_eq!(name_hash(b"main.scm"), 0xD83F_24DD);
        assert_eq!(name_hash(b"stream.ini"), 0x4633_BCEB);
        assert_eq!(name_hash(b"logo.png"), 0xBADD_EA6B);
        assert_eq!(name_hash(b"handling.cfg"), 0x2CC7_CE25);
        // Exercises the 12-byte block path (>= 12 bytes).
        assert_eq!(name_hash(b"vehicles.ide"), 0xEE6D_FCB7);
        assert_eq!(name_hash(b"ar_stats.dat"), 0x5440_792E);
    }
}
