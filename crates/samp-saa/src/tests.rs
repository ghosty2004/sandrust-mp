//! Unit tests on synthetic archives built by an independent writer (the inverse of
//! the documented format), so no third-party data is needed.

use super::*;
use crate::crypto::mask_word;

/// Check-pair XOR value with the required bits (any value with them works).
const CHECK: u32 = 0x4028_3433;

struct Built {
    bytes: Vec<u8>,
}

/// Lay out `files` (hash, contents) in the given slots. `chain` lists slot
/// indices in link order; the last links to `terminator`. `valid_mask` chooses
/// which decoding the archive is written for (signature valid → hash term on).
fn build(files: &[(usize, u32, Vec<u8>)], terminator: u8, valid_mask: bool, extra_dead: &[(usize, u32)]) -> Built {
    let mask = if valid_mask { u32::MAX } else { 0 };
    let mut dir = vec![0u8; DIR_LEN];
    let mut seed = 0x1234_5678u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        seed
    };
    let put = |dir: &mut Vec<u8>, i: usize, hash: u32, value: u32| {
        let packed = mask_word(value ^ (hash & mask));
        dir[i * 8..i * 8 + 4].copy_from_slice(&hash.to_le_bytes());
        dir[i * 8 + 4..i * 8 + 8].copy_from_slice(&packed.to_le_bytes());
    };
    // Unused slots: random hash, link to self.
    for i in 0..DIR_ENTRIES {
        let h = rnd();
        put(&mut dir, i, h, (rnd() & 0xFF_FF00) | i as u32);
    }
    for &(i, h) in extra_dead {
        put(&mut dir, i, h, i as u32);
    }
    // Live slots: link to the next in chain order.
    for (k, (slot, hash, data)) in files.iter().enumerate() {
        let link = files.get(k + 1).map_or(u32::from(terminator), |n| n.0 as u32);
        put(&mut dir, *slot, *hash, ((data.len() as u32) << 8) | link);
    }
    // Data: the chain's last entry sits first at DATA_START.
    let mut area = Vec::new();
    for (_, hash, data) in files.iter().rev() {
        let at = (hash % (DIR_LEN as u32 - 16)) as usize;
        let key: [u8; 16] = dir[at..at + 16].try_into().unwrap();
        let mut buf = data.clone();
        buf.resize(round_block(data.len() as u32) as usize, 0);
        crypto::ChainedXtea::new(&key).encrypt(&mut buf);
        area.extend_from_slice(&buf);
    }
    let mut out = vec![0u8; HEADER_LEN];
    out[..4].copy_from_slice(b"SAA2");
    let w1 = (rnd() & !(0xFF << 5)) | (u32::from(terminator) << 5);
    out.extend_from_slice(&(w1 ^ CHECK).to_le_bytes());
    out.extend_from_slice(&w1.to_le_bytes());
    out.extend_from_slice(&area);
    let mut edir = dir.clone();
    crypto::ChainedXtea::new(&DIRECTORY_KEY).encrypt(&mut edir);
    out.extend_from_slice(&edir);
    out.extend_from_slice(&[0u8; SIG_LEN]);
    Built { bytes: out }
}

fn sample() -> Vec<(usize, u32, Vec<u8>)> {
    let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    vec![
        (3, path_hash("main.scm"), b"SCM!".to_vec()),
        (200, path_hash("gta.dat"), b"# gta.dat\r\nIDE DATA\\X.IDE\r\n".to_vec()),
        (17, path_hash("tracks2.dat"), b"0".to_vec()),
        (90, path_hash("script.img"), big),
        (91, path_hash("empty.bin"), Vec::new()),
        (150, path_hash("exact.blk"), vec![0xAB; 2048]),
    ]
}

#[test]
fn roundtrip_without_signature_like_samp() {
    // No valid signature: samp decodes with the hash term off; the writer used
    // the same convention, so everything reads back.
    let files = sample();
    let a = SaaArchive::from_bytes(build(&files, 77, false, &[]).bytes).unwrap();
    assert!(!a.signature_valid());
    for (_, hash, data) in &files {
        assert_eq!(a.read_hash(*hash).as_deref(), Some(&data[..]));
    }
    assert_eq!(a.list().len(), files.len());
}

#[test]
fn roundtrip_assume_valid() {
    let files = sample();
    let b = build(&files, 5, true, &[]).bytes;
    let a = SaaArchive::from_bytes_with(b.clone(), SignaturePolicy::AssumeValid).unwrap();
    for (_, hash, data) in &files {
        assert_eq!(a.read_hash(*hash).as_deref(), Some(&data[..]));
    }
    // Same bytes, signature check fails -> hash term off -> decoding differs.
    let v = SaaArchive::from_bytes(b).unwrap();
    assert!(!v.signature_valid());
    let differs = files.iter().any(|(_, h, d)| v.read_hash(*h).as_deref() != Some(&d[..]));
    assert!(differs);
}

#[test]
fn path_lookup_uses_lowercased_basename() {
    let files = sample();
    let a = SaaArchive::from_bytes(build(&files, 77, false, &[]).bytes).unwrap();
    for p in ["main.scm", "MAIN.SCM", "data\\script\\main.scm", "DATA/SCRIPT/Main.Scm", "x\\y/main.scm "] {
        assert_eq!(a.read(p).as_deref(), Some(&b"SCM!"[..]), "{p}");
    }
    assert!(a.read("main.scm2").is_none());
    assert!(a.read("data\\main.scm\\").is_none());
    assert!(!a.is_redirected("\\main.scm"), "separator at position 0 is not stripped");
    assert!(a.is_redirected("data\\Paths\\TRACKS2.DAT"));
    assert_eq!(a.read("x/empty.bin").unwrap().len(), 0);
}

#[test]
fn lookup_key_rules() {
    assert_eq!(lookup_key(b"DATA\\GTA.DAT"), b"gta.dat");
    assert_eq!(lookup_key(b"a/b\\C.d"), b"c.d");
    assert_eq!(lookup_key(b"stream.ini "), b"stream.ini");
    assert_eq!(lookup_key(b"stream.ini  "), b"stream.ini ");
    assert_eq!(lookup_key(b"\\x.dat"), b"\\x.dat");
    assert_eq!(lookup_key(b"/x.dat"), b"/x.dat");
    assert_eq!(lookup_key(b"a\\"), b"");
    assert_eq!(lookup_key(b""), b"");
    assert_eq!(lookup_key(b" "), b"");
    // Only ASCII is folded.
    assert_eq!(lookup_key(b"\xC4.DAT"), b"\xC4.dat");
}

#[test]
fn first_slot_with_hash_wins_even_if_dead() {
    let h = path_hash("dup.dat");
    let files = vec![(40, h, b"live".to_vec())];
    // Dead slot 10 carries the same hash and comes first.
    let a = SaaArchive::from_bytes(build(&files, 9, false, &[(10, h)]).bytes).unwrap();
    assert!(a.find("dup.dat").is_none());
    // A dead slot after the live one does not matter.
    let a = SaaArchive::from_bytes(build(&files, 9, false, &[(60, h)]).bytes).unwrap();
    assert_eq!(a.read("dup.dat").as_deref(), Some(&b"live"[..]));
}

#[test]
fn offsets_follow_chain_order() {
    let files = sample();
    let a = SaaArchive::from_bytes(build(&files, 77, false, &[]).bytes).unwrap();
    let list = a.list();
    let off = |name: &str| list.iter().find(|e| e.hash == path_hash(name)).unwrap().offset.unwrap();
    // Last in chain is first in the data area.
    assert_eq!(off("exact.blk"), DATA_START);
    assert_eq!(off("empty.bin"), DATA_START + 2048);
    assert_eq!(off("script.img"), DATA_START + 2048);
    assert_eq!(off("tracks2.dat"), DATA_START + 2048 + 6144);
    let names: Vec<_> = list.iter().filter_map(|e| e.name).collect();
    assert_eq!(names, ["main.scm", "tracks2.dat", "script.img", "gta.dat"]);
    assert_eq!(a.redirected_paths(), ["main.scm", "script.img", "gta.dat", "tracks2.dat"]);
}

#[test]
fn rejects_bad_files() {
    assert!(matches!(SaaArchive::from_bytes(vec![0; 100]), Err(SaaError::TooShort(100))));
    let mut b = build(&sample(), 77, false, &[]).bytes;
    b[CHECK_OFFSET] ^= 1;
    assert!(matches!(SaaArchive::from_bytes(b), Err(SaaError::BadCheckWord(_))));
}

#[test]
fn truncated_data_reads_none_not_panic() {
    let files = sample();
    let mut b = build(&files, 77, false, &[]).bytes;
    // Drop most of the data area but keep directory + signature at the end.
    let tail = b.split_off(b.len() - DIR_LEN - SIG_LEN);
    b.truncate(DATA_START as usize + 2048);
    b.extend_from_slice(&tail);
    let a = SaaArchive::from_bytes(b).unwrap();
    assert!(a.read("main.scm").is_none());
}
