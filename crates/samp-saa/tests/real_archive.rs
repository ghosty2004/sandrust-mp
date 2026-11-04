//! Tests against the user's local samp.saa and the tracer's captures of what the
//! game read under the original client. Both are third-party data that only
//! exist on the developer machine (never in the repo); every test here prints a
//! message and passes when they are absent.
//!
//! Overrides: `SAMP_SAA=<path to samp.saa>`, `SAMP_SAA_CAPTURES=<dir>`.

use std::path::{Path, PathBuf};

use samp_saa::{INTEGRITY_TABLE_HASH, KNOWN_ENTRIES, SaaArchive};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn archive() -> Option<SaaArchive> {
    let path = std::env::var_os("SAMP_SAA")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("re/samp-installer/extracted/samp.saa"));
    if !path.exists() {
        eprintln!("skipping: no samp.saa at {} (set SAMP_SAA)", path.display());
        return None;
    }
    Some(SaaArchive::open(&path).expect("open samp.saa"))
}

fn captures_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("SAMP_SAA_CAPTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/trace/saa-files/orig"));
    if !dir.is_dir() {
        eprintln!("skipping: no tracer captures at {} (set SAMP_SAA_CAPTURES)", dir.display());
        return None;
    }
    Some(dir)
}

#[test]
fn real_archive_structure() {
    let Some(a) = archive() else { return };
    assert!(a.signature_valid(), "RSA signature of the genuine archive must verify");
    let list = a.list();
    assert_eq!(list.len(), 28, "live entries");
    assert_eq!(list.iter().filter(|e| e.name.is_some()).count(), KNOWN_ENTRIES.len());
    for e in &list {
        let data = a.read_entry(e).unwrap_or_else(|| panic!("entry {} unreadable", e.index));
        assert_eq!(data.len(), e.size as usize);
    }
    for k in KNOWN_ENTRIES {
        assert!(a.find(k.name).is_some(), "{} missing", k.name);
        if let Some(p) = k.game_path {
            assert!(a.is_redirected(p), "{p} not redirected");
        }
    }
    assert_eq!(a.redirected_paths().len(), KNOWN_ENTRIES.len());
    assert!(a.find_hash(INTEGRITY_TABLE_HASH).is_some());
    assert!(a.read("logo.png").unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(a.read("data\\script\\script.img").unwrap().starts_with(b"VER2"));
    // Files samp does not ship stay with the game.
    for p in ["data\\maps\\LA\\LAn.IDE", "data\\Paths\\tracks.dat", "models\\gta3.img", "data\\water.dat"] {
        assert!(!a.is_redirected(p), "{p}");
    }
}

/// How a tracer capture relates to the archive plaintext. The tracer rebuilds a
/// file from the game's `CFileMgr` calls, so the capture is:
/// * `Exact` — whole file read with `Read`;
/// * `Prefix` — the game stopped reading early (e.g. an `end` marker, or only the
///   script.img header goes through `CFileMgr`);
/// * `Sparse` — read at several offsets with seeks (unread bytes are zero);
/// * `Text`/`TextPrefix` — read with `ReadLine` from a file the game opened in
///   text mode (the C runtime turns CRLF into LF) and stored by the tracer as a
///   lossy UTF-8 string per line; compared after applying the same two transforms.
#[derive(Debug, PartialEq)]
enum Match {
    Exact,
    Prefix,
    Sparse,
    Text,
    TextPrefix,
}

fn text_model(plain: &[u8], crlf_to_lf: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(plain.len());
    for line in plain.split_inclusive(|&b| b == b'\n') {
        let line = if crlf_to_lf && line.ends_with(b"\r\n") {
            let mut l = line[..line.len() - 2].to_vec();
            l.push(b'\n');
            l
        } else {
            line.to_vec()
        };
        out.extend_from_slice(String::from_utf8_lossy(&line).as_bytes());
    }
    out
}

fn classify(capture: &[u8], plain: &[u8]) -> Option<Match> {
    if capture == plain {
        return Some(Match::Exact);
    }
    if !capture.is_empty() && plain.starts_with(capture) {
        return Some(Match::Prefix);
    }
    let nonzero = capture.iter().filter(|&&b| b != 0).count();
    if capture.len() == plain.len() && nonzero > 0 && capture.iter().zip(plain).all(|(&c, &p)| c == 0 || c == p) {
        return Some(Match::Sparse);
    }
    for crlf in [false, true] {
        let t = text_model(plain, crlf);
        if capture == t {
            return Some(Match::Text);
        }
        if !capture.is_empty() && t.starts_with(capture) {
            return Some(Match::TextPrefix);
        }
    }
    None
}

#[test]
fn reproduces_tracer_captures() {
    let Some(a) = archive() else { return };
    let Some(dir) = captures_dir() else { return };
    let mut checked = 0;
    let mut exact = 0;
    let mut covered = std::collections::BTreeSet::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for path in files {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        // "<NNN>_<game path with separators as '_'>"
        let Some((_, rest)) = fname.split_once('_') else { continue };
        let rest = rest.to_ascii_lowercase();
        let Some(k) = KNOWN_ENTRIES.iter().find(|k| rest == k.name || rest.ends_with(&format!("_{}", k.name))) else {
            continue;
        };
        let capture = std::fs::read(&path).unwrap();
        let plain = a.read(k.name).unwrap();
        let m = classify(&capture, &plain);
        eprintln!("{fname:40} {:>7} B vs {:>7} B  {m:?}", capture.len(), plain.len());
        assert!(m.is_some(), "{fname}: capture does not match archive entry {}", k.name);
        if m == Some(Match::Exact) {
            exact += 1;
        }
        checked += 1;
        covered.insert(k.name);
    }
    eprintln!("{checked} captures checked ({exact} byte-identical), {} distinct entries", covered.len());
    assert!(checked > 0, "capture dir has no archive files");
}

#[test]
fn captured_text_files_are_lf_or_raw() {
    // Sanity check of the comparison model itself on a synthetic file.
    let plain = b"a\r\nb\xA3\r\nend\r\nzzz";
    assert_eq!(classify(b"a\nb\xEF\xBF\xBD\nend\n", plain), Some(Match::TextPrefix));
    assert_eq!(classify(b"a\r\nb", plain), Some(Match::Prefix));
    assert_eq!(classify(b"a\r\x00\x00", &plain[..4]), Some(Match::Sparse));
    assert_eq!(classify(b"x", plain), None);
}
