//! Archive signature check: RSA-1024 / PKCS#1 v1.5 over a SHA-1 digest, with the
//! digest placed in the padding block without an ASN.1 DigestInfo prefix (the
//! CryptoAPI "no hash OID" form). Spec: `docs/re/samp-saa.md` §4.

/// RSA public exponent of the archive key.
pub const RSA_EXPONENT: u32 = 65_537;

/// RSA-1024 modulus of the archive key, little-endian (CryptoAPI blob order).
/// Source: PUBLICKEYBLOB at 0x100E9998 in samp.dll 0.3.7-R4 (both builds), stored
/// XORed with 0xAA; this is the de-obfuscated modulus field (blob offset 20..148).
pub const RSA_MODULUS_LE: [u8; 128] = [
    0xef, 0xd5, 0x47, 0x67, 0xa9, 0xa0, 0x80, 0x93, 0x52, 0x6b, 0x64, 0x1b, 0x36, 0x6f, 0x2c, 0xf0, 0x08, 0xb7, 0xa1,
    0x14, 0x4e, 0x5f, 0x5e, 0x0c, 0x48, 0xee, 0xb8, 0xac, 0x44, 0xe6, 0x6d, 0xe0, 0xdc, 0x13, 0x46, 0x46, 0xda, 0x6c,
    0x88, 0xc0, 0x96, 0x35, 0x9a, 0xe8, 0xbf, 0x61, 0x3d, 0x22, 0x03, 0xa3, 0xb0, 0x08, 0x48, 0xa0, 0xda, 0xf7, 0x75,
    0xdb, 0x84, 0xb4, 0x55, 0xd9, 0xb9, 0x1b, 0x42, 0x4f, 0x39, 0x35, 0xde, 0x59, 0xe6, 0x43, 0x32, 0x0e, 0x92, 0x7d,
    0xa9, 0x6f, 0x22, 0xbe, 0x20, 0xf3, 0x9a, 0xff, 0x41, 0x8b, 0xe5, 0x71, 0x1f, 0x54, 0xbb, 0x40, 0xd0, 0x11, 0x86,
    0x3e, 0x9a, 0x36, 0xd7, 0x72, 0xea, 0x83, 0xc7, 0xe4, 0xb4, 0xb7, 0x9f, 0x14, 0x0a, 0xf6, 0x99, 0xd3, 0x46, 0x06,
    0xe6, 0x87, 0x26, 0xd6, 0xa9, 0xa8, 0x00, 0x85, 0xb8, 0x99, 0xa9, 0xf4, 0x6c, 0xbd,
];

/// Check `sig_le` (128 bytes, little-endian, as stored at the end of the archive)
/// against `message` with the archive's public key.
pub fn verify(message: &[u8], sig_le: &[u8]) -> bool {
    if sig_le.len() != 128 {
        return false;
    }
    let n = Big::from_le(&RSA_MODULUS_LE);
    let s = Big::from_le(sig_le);
    if !s.lt(&n) {
        return false;
    }
    let m = s.modpow(RSA_EXPONENT, &n).to_be_128();
    let digest = sha1(message);
    // 00 01 FF..FF 00 <20-byte digest>
    let pad_end = 128 - 20 - 1;
    m[0] == 0x00
        && m[1] == 0x01
        && m[2..pad_end].iter().all(|&b| b == 0xFF)
        && m[pad_end] == 0x00
        && m[pad_end + 1..] == digest
}

/// SHA-1 (FIPS 180-4).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = Vec::with_capacity(128);
    let full = data.len() / 64 * 64;
    tail.extend_from_slice(&data[full..]);
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data[..full].as_chunks::<64>().0.iter().chain(tail.as_chunks::<64>().0) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (hv, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hv = hv.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Minimal unsigned big integer (little-endian u32 limbs), just enough for one
/// 1024-bit modular exponentiation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_le(bytes: &[u8]) -> Self {
        let mut limbs: Vec<u32> = bytes
            .chunks(4)
            .map(|c| {
                let mut w = [0u8; 4];
                w[..c.len()].copy_from_slice(c);
                u32::from_le_bytes(w)
            })
            .collect();
        trim(&mut limbs);
        Big(limbs)
    }

    fn to_be_128(&self) -> [u8; 128] {
        let mut out = [0u8; 128];
        for (i, limb) in self.0.iter().enumerate().take(32) {
            let le = limb.to_le_bytes();
            for (j, b) in le.iter().enumerate() {
                out[127 - (4 * i + j)] = *b;
            }
        }
        out
    }

    fn lt(&self, other: &Big) -> bool {
        cmp(&self.0, &other.0) == core::cmp::Ordering::Less
    }

    fn mulmod(&self, other: &Big, m: &Big) -> Big {
        let mut prod = vec![0u32; self.0.len() + other.0.len() + 1];
        for (i, &a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.0.iter().enumerate() {
                let t = u64::from(a) * u64::from(b) + u64::from(prod[i + j]) + carry;
                prod[i + j] = t as u32;
                carry = t >> 32;
            }
            let mut k = i + other.0.len();
            while carry != 0 {
                let t = u64::from(prod[k]) + carry;
                prod[k] = t as u32;
                carry = t >> 32;
                k += 1;
            }
        }
        trim(&mut prod);
        Big(rem(&prod, &m.0))
    }

    fn modpow(&self, mut e: u32, m: &Big) -> Big {
        let mut base = self.clone();
        let mut acc = Big(vec![1]);
        while e != 0 {
            if e & 1 == 1 {
                acc = acc.mulmod(&base, m);
            }
            base = base.mulmod(&base, m);
            e >>= 1;
        }
        acc
    }
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp(a: &[u32], b: &[u32]) -> core::cmp::Ordering {
    let la = a.iter().rposition(|&x| x != 0).map_or(0, |p| p + 1);
    let lb = b.iter().rposition(|&x| x != 0).map_or(0, |p| p + 1);
    if la != lb {
        return la.cmp(&lb);
    }
    for i in (0..la).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    core::cmp::Ordering::Equal
}

/// `a mod m` by binary long division (shift in one bit, subtract when >= m).
fn rem(a: &[u32], m: &[u32]) -> Vec<u32> {
    let mut r: Vec<u32> = vec![0; m.len() + 1];
    for i in (0..a.len() * 32).rev() {
        // r = (r << 1) | bit
        let bit = (a[i / 32] >> (i % 32)) & 1;
        let mut carry = bit;
        for limb in r.iter_mut() {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if cmp(&r, m) != core::cmp::Ordering::Less {
            let mut borrow = 0i64;
            for (k, limb) in r.iter_mut().enumerate() {
                let sub = i64::from(m.get(k).copied().unwrap_or(0)) + borrow;
                let mut t = i64::from(*limb) - sub;
                if t < 0 {
                    t += 1 << 32;
                    borrow = 1;
                } else {
                    borrow = 0;
                }
                *limb = t as u32;
            }
        }
    }
    trim(&mut r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha1(&million)), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    #[test]
    fn small_modpow() {
        // 4^13 mod 497 = 445
        let r = Big(vec![4]).modpow(13, &Big(vec![497]));
        assert_eq!(r, Big(vec![445]));
        // Multi-limb: (2^64 + 3)^65537 mod (2^61 - 1) checked against u128 arithmetic.
        let m: u128 = (1 << 61) - 1;
        let b: u128 = ((1u128 << 64) + 3) % m;
        let mut acc: u128 = 1;
        let mut base = b;
        let mut e = 65_537u32;
        while e != 0 {
            if e & 1 == 1 {
                acc = acc * base % m;
            }
            base = base * base % m;
            e >>= 1;
        }
        let got = Big(vec![3, 0, 1]).modpow(65_537, &Big(vec![m as u32, (m >> 32) as u32]));
        let want = Big::from_le(&(acc as u64).to_le_bytes());
        assert_eq!(got, want);
    }

    #[test]
    fn bad_signature_rejected() {
        assert!(!verify(b"hello", &[0u8; 128]));
        assert!(!verify(b"hello", &[1u8; 127]));
        let mut s = [0u8; 128];
        s[0] = 2;
        assert!(!verify(b"hello", &s));
    }
}
