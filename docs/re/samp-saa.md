# samp.saa — SA:MP's encrypted data archive (RE-9)

Clean-room description of the archive samp.dll 0.3.7-R4 opens at start-up and of the
file-open redirection that serves game data files out of it. Written from our reading of
samp.dll build B (2019, `0.3.7-R4_fv0.3.7.3_2019-11-21`); build A (2021) has the same code
40 (0x40) bytes later in this region (e.g. the CreateFileA hook at 0x10062B50 instead of
0x10062B10) and the same data constants at the same addresses. Reader implementation:
`crates/samp-saa`.

Labels: **[O]** observed (read in the code / checked against data), **[I]** inferred,
**[U]** unknown.

The archive is third-party data. It is read at runtime from the user's own SA:MP install and
is never copied into the repository. Its contents are only described here in summary.

## 1. Overview

* [O] samp.saa is a single file: a 248-byte header, an 8-byte check pair, the file data area,
  a 2048-byte encrypted directory of 256 slots, and a 128-byte RSA signature.
* [O] Files are identified only by a 32-bit hash of their lower-case basename. **No names are
  stored.** We recovered the names by hashing candidate file names (§9).
* [O] Directory and file data are encrypted with XTEA (32 cycles) in a chained mode where the key
  is modified by every ciphertext block. The directory key is a constant in samp.dll; each file's
  key is 16 bytes taken from the decrypted directory.
* [O] The signature (RSA-1024, SHA-1) is checked, but a failure does not stop loading: it
  silently switches the directory decoding to a different formula, which turns a genuine
  archive's sizes and links into garbage (§6).
* [O] samp.dll hooks `CreateFileA`. When the **basename** of the opened path (any directory)
  matches an archive entry, it returns a fake handle and serves reads from the decrypted entry.

## 2. Life cycle in samp.dll

* [O] `DllMain` (process attach, 0x100C50E0) creates the archive object and opens the file under
  the literal name `samp.saa`, i.e. relative to the current directory (the game folder). It builds
  a full path next to samp.dll first but does not use it for the open. If the open fails, it
  executes a breakpoint (the process dies).
* [O] Open (0x10065460): `fopen(…, "rb")`; read the 248-byte header and the 8-byte check pair;
  check it (§3). The file stays open for the life of the process.
* [O] The directory is loaded lazily on the first lookup (0x10064A90): signature check, directory
  read and decryption, and a binary search tree over the 256 slots keyed by their hash word.
* [O] Right after the archive opens, `DllMain` installs the file hooks (0x10062C60): CreateFileA,
  ReadFile, GetFileSize, SetFilePointer, CloseHandle, GetFileType (kernel32).
* [O] An entry is decrypted on its first open and the buffer is kept for the life of the process
  (0x10064F20); later opens reuse it.

## 3. File layout

All integers are little-endian u32 unless stated.

| offset | size | contents |
|---|---|---|
| 0 | 248 | header block (0x78 × 2 + 8 bytes; 0x78 is a field set by the constructor) |
| 248 | 8 | check pair `w0`, `w1` |
| 256 | … | data area (0x78 × 2 + 0x10): files, each padded to a multiple of 2048 bytes |
| len − 2176 | 2048 | directory: 256 slots × 8 bytes, encrypted |
| len − 128 | 128 | RSA signature, little-endian integer |

* [O] Check pair: `c = w0 ^ w1` must satisfy `c & 0xFFFFF == 0x83433` and `c & 0x700000 == 0x200000`,
  otherwise open fails. (The R4 archive has `c = 0x40283433`.)
* [O] `w1` also supplies the chain **terminator**: `t = (w1 >> 5) & 0xFF` (§4.3).
* [O] The header block itself is not interpreted by the R4 reader. [O] It begins with `SAA2`, a
  u32 22 and a u16; these match the layout an older reader in samp.dll expects (base class,
  0x10060BE0: `SAA2` magic, entry count, 16-bit key, 38-byte records with the name at +8,
  decrypted by a byte-wise stream cipher). [O] Decoding the R4 header with that older scheme
  gives random bytes. [I] The header is filler/decoy kept for the old format's magic.
* [O] In the R4 archive the data area ends exactly where the directory starts.

## 4. Directory

### 4.1 Decryption

* [O] The 2048 bytes are decrypted in one pass with the chained XTEA of §5, starting from the
  fixed directory key `13 40 EA A0 09 B5 AB 89 1A 40 EC CE D2 05 FA 2A` (16 bytes at 0x100E9A2C,
  stored XORed with 0xAA).

### 4.2 Slot format

Each slot is two words, `h` (the name hash, plain) and `p` (packed, masked):

* [O] Unmask: `u = rotl32(p ^ 0xC103D3E7, 13)`. samp computes this with two separate immediates
  (0x3D3E7 for the low 19 bits, 0xC107FFFF for the high 13); they combine to the one XOR value
  above.
* [O] Decoded value: `v = u ^ (h & M)`, where `M` is `0xFFFFFFFF` if the signature verified and
  `0` otherwise (§6).
* [O] `size = v >> 8` (exact byte size), `link = v & 0xFF` (slot index).
* [O] A slot whose `link` equals its own index is **unused**. In the R4 archive 28 slots are live;
  the other 228 hold random-looking hash words and self-links.

### 4.3 Data offset (link chain)

* [O] Offset of slot `i` = 256 + the sum of the block-rounded sizes (`ceil(size / 2048) * 2048`)
  of the slots visited by following links from `link(i)` until the link equals the terminator
  `t`. The terminator itself is not counted. A slot that links straight to `t` sits at 256.
* [I] So the live slots form one list and the files are stored in reverse list order (the end of
  the list is first in the data area). In the R4 archive the 28 live slots form one chain that
  covers the data area without gaps.
* [O] samp does not guard against cycles (it would loop forever). Our reader gives up after
  256 steps.

## 5. Chained XTEA

* [O] Block cipher: XTEA decryption, 32 cycles, delta 0x9E3779B9, little-endian words
  `v0 = bytes 0..3`, `v1 = bytes 4..7`, key words `k0..k3` little-endian, with the usual key
  schedule (`k[sum & 3]` for the `v0` half and `k[(sum >> 11) & 3]` for the `v1` half).
  * Delta source: 0x101027B4 holds 0x0CCF225C, unmasked once with the §4.2 word unmask.
  * Cycle count: 0x101027B0 = 32.
* [O] Chaining: after each 8-byte block, with `c0`, `c1` the block's **ciphertext** words:
  `k0 ^= c0; k1 ^= c1; k2 ^= c0; k3 ^= c1`. The key state continues across consecutive calls,
  so a file decrypted in 2048-byte pieces is one continuous stream.
* [O] File key: the 16 bytes of the **decrypted** directory at byte offset `h % 2032`
  (2032 = 256 × 8 − 16), where `h` is the slot's raw hash word (not masked).
* [O] File data: `ceil(size / 2048)` blocks of 2048 bytes at the slot's offset, decrypted in
  order with one running key; the result is cut to `size`.

## 6. Signature

* [O] Message: archive bytes from offset 256 up to the signature (data area + encrypted
  directory). Hash: SHA-1 (`CALG_SHA1` 0x8004 at 0x101027A8).
* [O] Key: RSA-1024, exponent 65537, PUBLICKEYBLOB at 0x100E9998 (148 bytes, stored XORed with
  0xAA). The modulus is in `crates/samp-saa/src/verify.rs`.
* [O] Verification goes through CryptoAPI (function names stored XORed with 0x55 and resolved at
  run time from advapi32) with the "no hash OID" flag: the decrypted signature block is
  `00 01 FF…FF 00` followed by the raw 20-byte digest, without a DigestInfo prefix. The R4
  archive verifies.
* [O] The result only feeds the mask `M` of §4.2 (`M = -result`). The archive is used either way.
  With a bad signature, every slot decodes to `rotl(p ^ K, 13)` alone, so sizes and links of a
  genuine archive become garbage. [I] Intent: tampered archives break instead of failing loudly.
  Our reader does the same by default (`SignaturePolicy::Verify`). `AssumeValid` skips the check.

## 7. Lookup

### 7.1 Name hash

* [O] Bob Jenkins' 1996 "lookup2" hash: golden-ratio seeds (`a = b = 0x9E3779B9`, stored masked
  as 0x0CCF225C at 0x10102790) and initial value `c = 0x12345678`. Blocks of 12 bytes, the
  standard 9-step mix, the length added to `c`, tail bytes 8..10 placed above `c`'s low byte.
  The result is `c` (0x10064840).

### 7.2 By path (what the CreateFileA hook uses)

1. [O] If the path's last character is a space, drop that one character.
2. [O] Take the part after the last `\` or `/`. The scan stops before position 0, so a
   separator in the first character is never found: `\x.dat` stays `\x.dat`.
3. [O] Lower-case (CRT `_strlwr`; ASCII in the C locale) and hash (§7.1).

The directory is only used through this basename hash, so **any** path with a matching file name
is redirected, whatever its directory.

### 7.3 Choosing the slot

* [O] The 256 slots are inserted in index order into a binary search tree keyed by `h`
  (equal keys go right). A lookup returns the first node met with an equal key. [I] That is
  always the lowest-index slot carrying that hash.
* [O] If that slot is unused (self-link, computed with the current `M`), the lookup fails, even
  if a later slot has the same hash.
* [O] Lookup by raw hash (vtable slot 7, 0x10064E00) is also used directly by samp itself (§9).

Archive object vtable (build B 0x100E9AA8): 0 destructor, 1 open, 2 close, 3 find by name,
4 size of entry, 5 entry data, 6 open from memory, 7 find by hash.

## 8. Redirection layer

* [O] Only `CreateFileA` is redirected (hook 0x10062B10). If the lookup of §7.2 succeeds, the hook
  returns a fake handle and never calls the real function. Access mode and creation flags are
  ignored. Otherwise it calls the original. [O] After a hit it also hashes the lower-case
  basename to 20 bytes (0x10062FA0; [I] SHA-1, not confirmed) and compares that with three stored
  digests (0x10062A40), bumping a counter in another samp object. [U] Purpose of that counter.
* [O] Fake handles: they start at 0xFF000001 and step by 2 for every open. The other hooks only
  treat values in `0xFF000001..=0xFF000100` as fake. At most 50 fake files can be open at once.
  [I] After 128 opens in one process, new handles fall outside the recognised range and reads
  on them go to the real kernel32 (and fail).
* [O] Each fake file holds `{size, position, data pointer}`. `ReadFile` copies
  `min(n, size − pos)`, advances, and reports the count (0 at end of file). It returns TRUE and
  then sets the last error to 38 (`ERROR_HANDLE_EOF`), even on a full read. An `OVERLAPPED`
  argument sets the position from its `Offset` field and signals its event.
* [O] `SetFilePointer`: FILE_BEGIN sets the position, FILE_CURRENT adds to it, FILE_END sets it
  to `size` (the distance is ignored). It returns the new position. `GetFileSize` returns the
  size. `CloseHandle` frees the slot (the decrypted buffer stays cached). `GetFileType` returns 1
  (disk).
* [O] The data reaches the game through the C runtime, so files the game opens in text mode see
  CRLF→LF translation as usual. [O] Seen in the tracer captures: stream.ini, gta.dat,
  default.dat, ped.dat, carmods.dat and shopping.dat are read as text; the others are read in
  binary mode.
* [O] A second CreateFile-style wrapper exists at 0x10062BF0. It hashes the contents of files
  read from disk (by extension) and compares them with a digest table that is an archive entry
  looked up by hash 0xA848B69A (0x10062390). In build B no reference to 0x10062BF0 was found,
  either as a pointer or as a call. [I] It is dormant. [U] Whether another build or path
  installs it.

## 9. Contents of the 0.3.7-R4 archive

28 live slots. 27 names were recovered by hashing candidate names. All 28 entries decrypt
cleanly and match the tracer captures (§10).

| basename (hashed) | stock location | vs. stock game file |
|---|---|---|
| main.scm | data\script\main.scm | different (SA:MP's own small script) |
| script.img | data\script\script.img | different (18 of 79 streamed scripts) |
| gta.dat, default.dat, default.ide, peds.ide, ar_stats.dat | data\ | different (see `docs/phase3/trace-notes.md`, "samp.saa data overrides") |
| lan2.ide, law2.ide, laxref.ide | data\maps\LA\ | different |
| props.ide | data\maps\interior\ | different |
| stream.ini | game root | different |
| tracks2.dat, tracks4.dat | data\Paths\ | different: both are the single byte `0` [I] (no nodes on those two train tracks) |
| vehicle.txd | models\generic\ | different (same size, other content) |
| loadscs.txd | models\txd\ | different (≈0.5 MB vs ≈3.3 MB: SA:MP loading screens [I]) |
| handling.cfg, weapon.dat, melee.dat, object.dat, ped.dat, vehicles.ide, carmods.dat, shopping.dat, surface.dat, timecyc.dat | data\ (stream.ini in root) | byte-identical to the stock files |
| logo.png | — | not a game file. samp loads it by hash 0xBADDEA6B as a D3D texture (0x10070C20) and also feeds its bytes into a checksum (0x100576A0) |
| (unnamed, hash 0xA848B69A) | — | digest table for the dormant wrapper of §8. One count byte (104), then 104 tree nodes of 8 bytes (u32 key, child indices, record index) and 104 records of 16 bytes [I] |

* [O] The tracer note "23 redirected files" counted the files the game opens through
  `CFileMgr`. The archive also covers tracks2/4.dat (which explains their 1-byte reads), and
  vehicle.txd / LOADSCS.txd, which the game opens through RenderWare streams, not `CFileMgr`.
  [O] The 1.0-US data the game would otherwise read is the Steam 3.0 copy in `wine/gta-sa`.
  [U] Whether vehicle.txd also differs from a retail 1.0 vehicle.txd.

## 10. Verification

* `cargo test -p samp-saa`: synthetic archives from an independent writer (inverse of this
  spec) test the round trip, decoding with and without the hash term, the lookup rules, the
  first-slot-wins rule, the chain offsets, and rejection of bad files. SHA-1 is checked against
  FIPS vectors and modexp against u128 arithmetic.
* `tests/real_archive.rs` (runs when `re/samp-installer/extracted/samp.saa` and
  `target/trace/saa-files/orig` exist, or `SAMP_SAA` / `SAMP_SAA_CAPTURES` point at them):
  * the signature verifies;
  * 28 live entries, and every named entry is found under its game path;
  * all 26 captures of archive files (24 distinct entries) match. 14 are byte-identical to
    the decrypted entry, among them main.scm, the four LA/interior IDEs, peds.ide, default.ide,
    ar_stats.dat and tracks2/4.dat. The other 12 differ only in how the tracer reassembles
    reads:
    * 3 prefixes: the game stopped early (timecyc.dat, object.dat, and the script.img header);
    * 2 sparse images: mission loads of main.scm seek around, and unread bytes are 0;
    * 7 text reads, CRLF→LF where the game opened the file in text mode, plus lossy UTF-8
      per line (the tracer stores lines as strings, e.g. the `£` in weapon.dat): stream.ini,
      default.dat, carmods.dat, gta.dat, ped.dat, weapon.dat, and shopping.dat (a prefix).
  * Every comparison model is exact: no byte is allowed to differ except where the model
    explains it.

## 11. Notes for SAA-2 (client integration)

* Serve the same files at the same layer: a `CreateFileA` redirect keyed on the lower-case
  basename hash, with the same 1-to-1 read, seek and size behaviour. Hooking `CFileMgr` instead
  would miss the RenderWare loads (vehicle.txd, LOADSCS.txd) and [I] the streaming reads of
  script.img (CdStream reads its handle through `ReadFile`, which is why samp's ReadFile hook
  handles `OVERLAPPED`), and would change text-mode behaviour.
* Look the archive up as samp does: `samp.saa` in the game directory. The reader verifies the
  signature like samp. A user's archive that fails verification also gives garbage in samp, so
  report it rather than "fixing" it.
* Which files to redirect is decided by the archive itself (every live entry). The crate's
  `KNOWN_ENTRIES` only names them for logs and tooling.
* logo.png and the digest table are samp-internal. Our client needs them only if it reproduces
  the same UI or checks.
* Fake-handle quirks (128-open limit, FILE_END ignoring the distance, EOF last-error) are
  samp's. Reproduce them only if a trace diff shows they matter.

## 12. Open questions

* [U] Meaning of the header fields (`SAA2`, 22, 0xEBB1) for the R4 reader: none found; only the
  check pair is read.
* [U] Purpose of the counter bumped when a redirected basename matches one of the three stored
  SHA-1 digests (0x10062A40).
* [U] Exact record layout of the 0xA848B69A digest table, and whether anything installs
  0x10062BF0.
* [U] Other SA:MP versions (R1/R2/R3/R5, 0.3.DL): archive keys and constants are not checked yet.

## Constants (build B addresses; same in build A)

| what | value | address |
|---|---|---|
| header length / data start | 248 / 256 (field 0x78) | ctor 0x10065310 |
| slot count | 256 | ctor 0x10065310 |
| check pair rule | `(w0^w1) & 0xFFFFF = 0x83433`, `& 0x700000 = 0x200000` | 0x10065460 |
| word unmask | `rotl(x ^ 0xC103D3E7, 13)` | 0x10064EE0, 0x10064F20 |
| XTEA delta (masked) | 0x0CCF225C → 0x9E3779B9 | 0x101027B4 |
| XTEA cycles | 32 | 0x101027B0 |
| hash seed (masked) | 0x0CCF225C → 0x9E3779B9; initval 0x12345678 | 0x10102790, 0x10064840 |
| directory key | `13 40 EA A0 09 B5 AB 89 1A 40 EC CE D2 05 FA 2A` (stored ^0xAA) | 0x100E9A2C |
| RSA public key blob | 148 bytes (stored ^0xAA), e = 65537 | 0x100E9998 |
| hash algorithm | CALG_SHA1 0x8004 | 0x101027A8 |
| fake handle range | 0xFF000001..=0xFF000100, step 2, 50 slots | 0x10102350, 0x10062540 |
