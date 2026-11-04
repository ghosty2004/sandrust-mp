# Phase 3 / SAA-2 — files from the user's samp.saa, 1:1 (plus SP-OFF)

Decision D-4 = A: our client serves every file samp.dll serves from `samp.saa`, read at runtime from
the user's own SA:MP install (never redistributed), including samp's own `main.scm` and `script.img`.
Without a usable archive the client falls back to the stock files plus our minimal `main.scm`.

## Pieces (crates/samp-client)

* `saafs.rs` (pure, host-tested): samp's fake-file table and its rules, from samp.dll B
  0x10062540–0x10062C60:
  * handles 0xFF000001, +2 per open (also once outside the recognised range); the other hooks only
    recognise 0xFF000001..=0xFF000100; 50 slots; a full table makes `CreateFileA` return 0;
  * `ReadFile`: an `OVERLAPPED` sets the position from `Offset`, its event is signalled; a read that
    reaches the end copies the rest and sets last error 38. **Correction to samp-saa.md §8:** a read
    that ends *before* the end of the entry does not touch the last error (B 0x10062731 path);
  * `SetFilePointer`: BEGIN sets, CURRENT adds (32-bit wrap), END jumps to the size ignoring the
    distance, other methods keep the position, high word ignored; `GetFileSize` zeroes the high word;
    `CloseHandle` frees the slot; `GetFileType` = 1;
  * `Redirector`: archive (`samp-saa`, signature verified like samp) + decrypted-entry cache (decrypted
    on first open, kept for the process) + per-entry statistics for the `files` control command.
* `filehook.rs` (Windows): at DLL attach (main thread still suspended) opens `samp.saa` next to
  gta_sa.exe and redirects gta_sa.exe's import-table entries of CreateFileA, ReadFile, GetFileSize,
  SetFilePointer, CloseHandle and GetFileType (slots found by `pe.rs`, an import-table walk tested
  against `re/gta/1.0-US/gta_sa.exe`). Like samp, a trailing space of the path is cut in the
  caller's buffer before the lookup (only if the page is writable; samp writes unconditionally).
* Mode is logged once: `files: ARCHIVE mode …` or `files: FALLBACK mode: <reason> …`.
  * Missing / unreadable / bad check word → fallback.
  * **Signature does not verify → fallback** (samp would keep going with a directory decoded without
    the hash term, i.e. garbage sizes/links; we report it and use the stock files instead).
* Control channel `files`: `files mode=archive signature=valid entries=28 distinct=N opens=N failed=N
  open_now=N`, then one `file name=… hash=… size=… opens=… handle=… path=<first path>` line per entry
  served. In fallback: `files mode=fallback … reason=…`.
* `dll.rs`: in archive mode our `main.scm` retarget (0x468EC9) is not installed; samp's script is
  read from the archive. Fallback keeps the M2 behaviour.

### What samp's data needs from the game (samp patches, now applied)
Booting with the archive's gta.dat/default.dat/peds.ide (+ SAMP\SAMP.ide, SAMP.img, SAMPCOL.img,
samp.ipl from the SA:MP folder) needs samp's own capacity patches; without them the stores overflow:
* `stores.rs`: atomic model-info store → 20000 entries (CSV rows 248–261; stock 14000, the data has
  ~15500 `objs`), ped model-info store → 319 entries (row 132; the data has 311 peds). New entries
  are zeroed with the class vtable, as samp writes them.
* Pools (rows 107 vehicles 710, 108 buildings 20000, 109 dummies 4000, 119 col models 32511) and the
  txd pool 20000 (row 131, early).
* Archive mode only (`Phase::EarlyArchive`): row 128 `CStreaming::InitImageList := RET` (the
  archive's default.dat lists CUSTOM/SAMP/GTA3/GTA_INT.IMG itself; otherwise 9 images for 8 slots)
  and rows 122/123 (splash names → `title`: the archive's loadscs.txd only holds `title`).

### samp's custom script opcodes (`scmops.rs`, CSV rows 2/3)
samp's scripts use two opcodes GTA leaves unimplemented (byte scan of the decrypted scripts, each a
complete instruction followed by a valid opcode):
* 0x09C6 (4 params): main.scm ×1 (type 5), carmod1.scm ×11 (types 1–4), player_parachute.scm ×21
  (types 22–43). Without a handler the stock target returns without reading the parameters and the
  script runs into its own arguments.
* 0x09D3 (condition, no params): carmod1.scm ×1 (`NOT 09D3`).
Our handlers (naked stubs in the jump-table slots, script in ESI, all registers preserved, continue at
0x47BF2B like samp): 0x09C6 collects 4 parameters; types 1–4 would send RPC 96 for a vehicle of
samp's pool — we have no vehicle pool before M5, so nothing is sent (logged). 0x09D3 sets the
condition false while the local player is in a vehicle as a passenger (not driver, not a train
carriage 569/570), true otherwise. Not observed at runtime yet: none of the recorded original traces
(boot/spawn/idle/vehicles) executes either opcode.

### Behaviour changes caused by samp's own script
* It fades the screen out in frame 0 and in again at frame ~34; `Idle` skips `Render2dStuff` (our
  per-frame tick) while faded out, so the client starts ~33 frames later than with our script. The
  original sets its two first-frame ped flags (087E/0446) in frame 1 from a hook that runs during the
  fade. We now hook the `CGame::Process` call in `Idle` (CSV row 10, 0x53E981) and set them there
  after the first game frame with a local ped; the connecting-screen block starts with the first
  `Render2dStuff` as in the original (orig frame 35, ours 33).
* Max wanted level 0, roads off, densities, `SWITCH_WORLD_PROCESSING`, clothes, the 18 streamed
  scripts and the helper threads (HJ, INTMAN, CASHWIN, …) now come from samp's script itself.
* Class selection and spawn: no new differences in the tracer diff (below).

## SP-OFF (same pass)
* `patches.rs` (Main): rows 189 (no `GenerateRandomCars`), 181 (no `CTheCarGenerators::Process`),
  163 (no `CReplay::Update`), 182 (no plane generation), 185 (`CPopulation::AddPed` → return 0),
  186 (no `SetPedInCarDirect` in `AddPedInCar`), 164 (no `GenerateAmbientPeds` on interior entry).
* Row 55: the police scanner's `FindPlayerPed` call returns a zeroed 0x7A0-byte stand-in ped (samp:
  a zeroed static of the same size, B 0x100A17BB).
* Fallback script: `SET_MAX_WANTED_LEVEL 0` (samp's main.scm does it in frames 0 and 2).

* Exit: `CGame::Shutdown` (CSV row 52) now does what samp's hook does: wait ~2 s, `ExitProcess(0)`.
  Running GTA's own shutdown crashed in `CModelInfo::ShutDown` (0x4C6202, null vtable): it walks
  the ped store's original 278-entry array with the new count 311. samp never runs it either.

## Verification
* `cargo test --workspace`, clippy host + i686 for samp-client: 0 warnings (71 samp-client tests;
  `saafs` runs a real-archive test when `re/samp-installer/extracted/samp.saa` exists).
* Lane main (headless, samp.saa present): `saa.sh`, `m2.sh`, `m3.sh`, `goal.sh` PASS (twice each
  with the final build); `m1.sh` PASS when run inside the headless display.
  * `m1.sh` on the visible Wine virtual desktop stalls ~49 s in the title fade before state 7 and
    misses its 120 s budget: 50 frames of `CLoadingScreen` fade (loop at 0x590880 → present), ~1 s
    per `Present` (main thread in the D3D9/DXVK present, thread dumps from the new slow-start
    diagnostics). The same happens with `SAMP_RS_NO_SAA=1` (fallback, no archive), and not in the
    headless display → environmental (compositor throttling of the visible window), not SAA-2.
    Accepted with the headless runs (main session).
* `saa.sh`: 26 distinct archive entries served (29 opens, 0 failed), every size equal to the archive
  entry; main.scm ×3 (CTheScripts::Init + 2 mission loads), script.img ×2 (directory + CdStream).
* Tracer, spawn scenario with `SAMP_TRACER_SCRIPTS=*` (orig-20261104-205141 vs ours-20261104-205035, re-run
  with the final build as ours-20261104-214431):
  * all 188 `CFileMgr` opens identical, in the same order (paths and modes);
  * the same script lifecycle (Init, StartTestScript, StartNewScript ×6 at the same ip offsets,
    `initial`/`initil2` shut down in frames 1/2) and the same main.scm threads (main, initial,
    initil2, hj, intman, noname) with the same set of commands in every phase;
  * remaining differences are client-side and pre-existing (M2-P/M3-P): per-frame 052C/0470/077E,
    REMOVE_BLIP ×2, pistol skill 1000, the join comes earlier in ours (frame 50–76 vs 271).
  * with the CGame::Process hook, 087E/0446 are issued in frame 1 as in the original.
* SP-OFF, `TRACE_IDLE_SECS=300 run.sh ours idle` (ours-20261104-212451) vs the original
  (orig-20261104-201902), `spoff_report.py --from grove`: both 0 creations, 0 peds/vehicles/cops,
  wanted 0, `MaximumWantedLevel` 0 (set by samp's main.scm in frames 0 and 2 in ours too), no
  interior peds after entering the Alhambra; `GenerateRandomCars`, `CTheCarGenerators::Process`,
  plane/train generation and `AddToPopulation` never run. Over the whole run ours creates only the
  player ped (before: 5 parked cars, cops, helicopters, 8 interior peds, 4 arrests).

## Open
* m1 on a visible desktop (above).
* 0x09C6 types 1–4 → RPC 96 need the vehicle pool (M5); neither custom opcode has been seen
  executing in a trace yet (needs a Transfender / parachute scenario).
* Fake-handle quirks are reproduced 1:1, including the 128-open handle range limit.
* The archive's SHA-1 basename counter (samp-saa.md §8, [U]) and the dormant 0x10062BF0 wrapper
  are not reproduced.
