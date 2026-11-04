//! Reader for `samp.saa`, the encrypted data archive shipped with the SA:MP
//! 0.3.7 client. samp.dll serves some game data files (main.scm, script.img,
//! gta.dat, …) from this archive instead of the game folder. This crate reads the
//! archive from the user's own SA:MP install; nothing from it is redistributed.
//!
//! Format, algorithms and the lookup rule are specified in `docs/re/samp-saa.md`.
//!
//! ```no_run
//! let saa = samp_saa::SaaArchive::open("C:/Games/GTA San Andreas/samp.saa")?;
//! let scm = saa.read("data\\script\\main.scm").expect("main.scm is in the archive");
//! # Ok::<(), samp_saa::SaaError>(())
//! ```

pub mod crypto;
pub mod verify;

use std::fmt;
use std::path::Path;

use crypto::{ChainedXtea, name_hash, unmask_word};

/// Length of the header block read at open (0x78 * 2 + 8 bytes).
pub const HEADER_LEN: usize = 248;
/// Offset of the 8-byte check pair that follows the header.
pub const CHECK_OFFSET: usize = HEADER_LEN;
/// Start of the file data area (0x78 * 2 + 0x10).
pub const DATA_START: u64 = 256;
/// Number of directory slots.
pub const DIR_ENTRIES: usize = 256;
/// Size of one directory slot.
pub const DIR_ENTRY_LEN: usize = 8;
/// Size of the encrypted directory.
pub const DIR_LEN: usize = DIR_ENTRIES * DIR_ENTRY_LEN;
/// Size of the trailing RSA signature.
pub const SIG_LEN: usize = 128;
/// File data is stored and decrypted in blocks of this size.
pub const BLOCK: u32 = 2048;
/// Smallest possible archive (header + check pair, directory, signature).
pub const MIN_LEN: usize = DATA_START as usize + DIR_LEN + SIG_LEN;

/// Directory key. Source: 16 bytes at 0x100E9A2C in samp.dll 0.3.7-R4 (both
/// builds), stored XORed with 0xAA; this is the de-obfuscated value.
pub const DIRECTORY_KEY: [u8; 16] =
    [0x13, 0x40, 0xea, 0xa0, 0x09, 0xb5, 0xab, 0x89, 0x1a, 0x40, 0xec, 0xce, 0xd2, 0x05, 0xfa, 0x2a];

/// Required value of `(w0 ^ w1) & 0xFFFFF` for the check pair at [`CHECK_OFFSET`].
pub const CHECK_LOW20: u32 = 0x8_3433;
/// Required value of `(w0 ^ w1) & 0x700000`.
pub const CHECK_BITS_20_22: u32 = 0x20_0000;

/// A file samp.dll is known to serve from the archive, identified by the lower-case
/// basename it hashes, plus the path the game normally opens it under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownEntry {
    /// Lower-case basename: exactly the string samp hashes for the lookup.
    pub name: &'static str,
    /// Where the stock game keeps this file (relative to the game folder, as GTA
    /// opens it). `None` for entries only samp itself uses.
    pub game_path: Option<&'static str>,
}

/// Every named entry of the 0.3.7-R4 archive (27 of its 28 live entries; the
/// remaining one is only ever looked up by hash, see [`INTEGRITY_TABLE_HASH`]).
/// Names were recovered by hashing candidate file names and matching the
/// directory keys; contents were confirmed against what the game read under the
/// original client.
pub const KNOWN_ENTRIES: &[KnownEntry] = &[
    KnownEntry { name: "main.scm", game_path: Some("data\\script\\main.scm") },
    KnownEntry { name: "script.img", game_path: Some("data\\script\\script.img") },
    KnownEntry { name: "gta.dat", game_path: Some("data\\gta.dat") },
    KnownEntry { name: "default.dat", game_path: Some("data\\default.dat") },
    KnownEntry { name: "default.ide", game_path: Some("data\\default.ide") },
    KnownEntry { name: "peds.ide", game_path: Some("data\\peds.ide") },
    KnownEntry { name: "vehicles.ide", game_path: Some("data\\vehicles.ide") },
    KnownEntry { name: "ar_stats.dat", game_path: Some("data\\ar_stats.dat") },
    KnownEntry { name: "lan2.ide", game_path: Some("data\\maps\\LA\\LAn2.IDE") },
    KnownEntry { name: "law2.ide", game_path: Some("data\\maps\\LA\\LAw2.IDE") },
    KnownEntry { name: "laxref.ide", game_path: Some("data\\maps\\LA\\LAxref.IDE") },
    KnownEntry { name: "props.ide", game_path: Some("data\\maps\\interior\\props.IDE") },
    KnownEntry { name: "stream.ini", game_path: Some("stream.ini") },
    KnownEntry { name: "handling.cfg", game_path: Some("data\\handling.cfg") },
    KnownEntry { name: "weapon.dat", game_path: Some("data\\weapon.dat") },
    KnownEntry { name: "melee.dat", game_path: Some("data\\melee.dat") },
    KnownEntry { name: "object.dat", game_path: Some("data\\object.dat") },
    KnownEntry { name: "ped.dat", game_path: Some("data\\ped.dat") },
    KnownEntry { name: "carmods.dat", game_path: Some("data\\carmods.dat") },
    KnownEntry { name: "shopping.dat", game_path: Some("data\\shopping.dat") },
    KnownEntry { name: "surface.dat", game_path: Some("data\\surface.dat") },
    KnownEntry { name: "timecyc.dat", game_path: Some("data\\timecyc.dat") },
    KnownEntry { name: "tracks2.dat", game_path: Some("data\\Paths\\tracks2.dat") },
    KnownEntry { name: "tracks4.dat", game_path: Some("data\\Paths\\tracks4.dat") },
    KnownEntry { name: "vehicle.txd", game_path: Some("models\\generic\\vehicle.txd") },
    KnownEntry { name: "loadscs.txd", game_path: Some("models\\txd\\LOADSCS.txd") },
    // Read by samp itself through the hash (0x100576A0, 0x10070C20), not a game file.
    KnownEntry { name: "logo.png", game_path: None },
];

/// Directory key of the one unnamed entry: a binary table that samp only ever
/// requests by this hash (0x10062390). See the spec, §6.
pub const INTEGRITY_TABLE_HASH: u32 = 0xA848_B69A;

/// Errors from opening an archive.
#[derive(Debug)]
pub enum SaaError {
    Io(std::io::Error),
    /// Shorter than header + directory + signature.
    TooShort(usize),
    /// The check pair after the header does not have the required bit pattern
    /// (samp refuses such a file at open).
    BadCheckWord(u32),
}

impl fmt::Display for SaaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaaError::Io(e) => write!(f, "cannot read archive: {e}"),
            SaaError::TooShort(n) => write!(f, "archive too short ({n} bytes, need at least {MIN_LEN})"),
            SaaError::BadCheckWord(w) => write!(f, "not a samp.saa archive (check word {w:#010x})"),
        }
    }
}

impl std::error::Error for SaaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaaError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SaaError {
    fn from(e: std::io::Error) -> Self {
        SaaError::Io(e)
    }
}

/// How to treat the RSA signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignaturePolicy {
    /// Like samp.dll: verify; a valid signature enables the hash term of the
    /// directory decoding, an invalid one disables it (the archive still opens,
    /// but sizes/links decode differently, i.e. garbage for a genuine archive).
    #[default]
    Verify,
    /// Skip verification and decode as if the signature were valid.
    AssumeValid,
}

/// One live directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryInfo {
    /// Directory slot (0..=255).
    pub index: u8,
    /// Name hash ([`name_hash`] of the lower-case basename).
    pub hash: u32,
    /// Exact file size in bytes.
    pub size: u32,
    /// Byte offset of the (encrypted, block-aligned) data in the archive, or
    /// `None` if the entry's link chain is broken.
    pub offset: Option<u64>,
    /// The basename, if the hash is one of [`KNOWN_ENTRIES`].
    pub name: Option<&'static str>,
}

/// An opened archive (whole file kept in memory; the 0.3.7 archive is ~2.6 MB).
pub struct SaaArchive {
    data: Vec<u8>,
    /// Decrypted directory (needed whole: file keys are cut out of it).
    dir: Vec<u8>,
    /// AND-mask applied to the hash word when decoding (all ones if the
    /// signature is valid, zero otherwise).
    hash_mask: u32,
    signature_valid: bool,
    /// Link value that terminates every offset chain.
    terminator: u8,
}

impl fmt::Debug for SaaArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaaArchive")
            .field("len", &self.data.len())
            .field("signature_valid", &self.signature_valid)
            .field("entries", &self.list().len())
            .finish()
    }
}

impl SaaArchive {
    /// Read and open an archive file (verifying the signature like samp does).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SaaError> {
        Self::from_bytes(std::fs::read(path)?)
    }

    /// Open an archive held in memory (verifying the signature like samp does).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, SaaError> {
        Self::from_bytes_with(data, SignaturePolicy::Verify)
    }

    /// Open with an explicit signature policy.
    pub fn from_bytes_with(data: Vec<u8>, policy: SignaturePolicy) -> Result<Self, SaaError> {
        if data.len() < MIN_LEN {
            return Err(SaaError::TooShort(data.len()));
        }
        let w0 = le32(&data[CHECK_OFFSET..]);
        let w1 = le32(&data[CHECK_OFFSET + 4..]);
        let check = w0 ^ w1;
        if check & 0xF_FFFF != CHECK_LOW20 || check & 0x70_0000 != CHECK_BITS_20_22 {
            return Err(SaaError::BadCheckWord(check));
        }
        let terminator = (w1 >> 5) as u8;

        let sig_start = data.len() - SIG_LEN;
        let signature_valid = match policy {
            SignaturePolicy::Verify => verify::verify(&data[DATA_START as usize..sig_start], &data[sig_start..]),
            SignaturePolicy::AssumeValid => true,
        };
        let hash_mask = if signature_valid { u32::MAX } else { 0 };

        let dir_start = sig_start - DIR_LEN;
        let mut dir = data[dir_start..sig_start].to_vec();
        ChainedXtea::new(&DIRECTORY_KEY).decrypt(&mut dir);

        Ok(Self { data, dir, hash_mask, signature_valid, terminator })
    }

    /// Whether the RSA signature checked out (always `true` under
    /// [`SignaturePolicy::AssumeValid`]).
    pub fn signature_valid(&self) -> bool {
        self.signature_valid
    }

    fn slot(&self, i: usize) -> (u32, u32) {
        let o = i * DIR_ENTRY_LEN;
        (le32(&self.dir[o..]), le32(&self.dir[o + 4..]))
    }

    /// Decoded value of a slot: `size << 8 | link`.
    fn decode(&self, i: usize) -> u32 {
        let (hash, packed) = self.slot(i);
        unmask_word(packed) ^ (hash & self.hash_mask)
    }

    /// A slot whose link points at itself is unused.
    fn is_live(&self, i: usize) -> bool {
        self.decode(i) as u8 as usize != i
    }

    /// Data offset of slot `i`: start of the data area plus the block-rounded
    /// sizes of every entry along the link chain that starts at `i`'s link.
    fn offset_of(&self, i: usize) -> Option<u64> {
        let mut off = DATA_START;
        let mut link = self.decode(i) as u8;
        let mut steps = 0;
        while link != self.terminator {
            steps += 1;
            if steps > DIR_ENTRIES {
                return None; // cycle; samp would spin forever here
            }
            let v = self.decode(usize::from(link));
            off += u64::from(round_block(v >> 8));
            link = v as u8;
        }
        Some(off)
    }

    fn info(&self, i: usize) -> EntryInfo {
        let (hash, _) = self.slot(i);
        EntryInfo {
            index: i as u8,
            hash,
            size: self.decode(i) >> 8,
            offset: self.offset_of(i),
            name: KNOWN_ENTRIES.iter().find(|k| name_hash(k.name.as_bytes()) == hash).map(|k| k.name),
        }
    }

    /// All live entries, in directory order.
    pub fn list(&self) -> Vec<EntryInfo> {
        (0..DIR_ENTRIES).filter(|&i| self.is_live(i)).map(|i| self.info(i)).collect()
    }

    /// samp's lookup by hash: the first slot (in directory order) carrying this
    /// hash; if that slot is unused the lookup fails, even if a later slot has
    /// the same hash.
    pub fn find_hash(&self, hash: u32) -> Option<EntryInfo> {
        let i = (0..DIR_ENTRIES).find(|&i| self.slot(i).0 == hash)?;
        self.is_live(i).then(|| self.info(i))
    }

    /// samp's lookup by path: see [`lookup_key`].
    pub fn find(&self, path: &str) -> Option<EntryInfo> {
        self.find_bytes(path.as_bytes())
    }

    /// Like [`SaaArchive::find`] for a raw (ANSI) path.
    pub fn find_bytes(&self, path: &[u8]) -> Option<EntryInfo> {
        self.find_hash(name_hash(&lookup_key(path)))
    }

    /// Whether samp would serve this path from the archive.
    pub fn is_redirected(&self, path: &str) -> bool {
        self.find(path).is_some()
    }

    /// Contents of the file samp would serve for `path` (any directory; only the
    /// basename matters).
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.read_entry(&self.find(path)?)
    }

    /// Contents of the entry with this name hash.
    pub fn read_hash(&self, hash: u32) -> Option<Vec<u8>> {
        self.read_entry(&self.find_hash(hash)?)
    }

    /// Decrypt one entry. The file key is 16 bytes of the decrypted directory,
    /// taken at `hash % 2032`; data is decrypted block by block with one running
    /// key state and cut to the exact size.
    pub fn read_entry(&self, e: &EntryInfo) -> Option<Vec<u8>> {
        let off = usize::try_from(e.offset?).ok()?;
        let len = round_block(e.size) as usize;
        let end = off.checked_add(len)?;
        if end > self.data.len() {
            return None;
        }
        let key_at = (e.hash % (DIR_LEN as u32 - 16)) as usize;
        let key: [u8; 16] = self.dir[key_at..key_at + 16].try_into().ok()?;
        let mut buf = self.data[off..end].to_vec();
        let mut cipher = ChainedXtea::new(&key);
        for block in buf.chunks_mut(BLOCK as usize) {
            cipher.decrypt(block);
        }
        buf.truncate(e.size as usize);
        Some(buf)
    }

    /// The paths samp redirects for this archive: the basenames of
    /// [`KNOWN_ENTRIES`] that have a live entry (lower case; any directory
    /// matches). Entries whose name is unknown are not included (see
    /// [`SaaArchive::list`]).
    pub fn redirected_paths(&self) -> Vec<&'static str> {
        KNOWN_ENTRIES.iter().filter(|k| self.find(k.name).is_some()).map(|k| k.name).collect()
    }
}

/// The string samp hashes for a path passed to `CreateFileA`:
/// 1. if the path ends in a space, drop that one character;
/// 2. keep what follows the last `\` or `/` (a separator in the very first
///    position is not recognised, so `"\\x"` stays `"\\x"`);
/// 3. ASCII lower-case.
pub fn lookup_key(path: &[u8]) -> Vec<u8> {
    let path = match path.last() {
        Some(b' ') => &path[..path.len() - 1],
        _ => path,
    };
    let base = match path.iter().rposition(|&c| c == b'\\' || c == b'/') {
        Some(p) if p > 0 => &path[p + 1..],
        _ => path,
    };
    base.to_ascii_lowercase()
}

/// Name hash of a path as samp computes it ([`lookup_key`] then [`name_hash`]).
pub fn path_hash(path: &str) -> u32 {
    name_hash(&lookup_key(path.as_bytes()))
}

fn round_block(n: u32) -> u32 {
    n.wrapping_add(BLOCK - 1) & !(BLOCK - 1)
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[cfg(test)]
mod tests;
