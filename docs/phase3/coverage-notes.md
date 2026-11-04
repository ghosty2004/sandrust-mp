# Phase 3 / COV-1: function coverage diff, original client vs ours

Question: what does GTA still do with our client that the original SA:MP client stops, and the
reverse? Method: record which gta_sa.exe functions run at least once in each window of a fixed
scenario, for both clients, then diff the two sets.

## Tools

* **Inventory** (`tools/cov/inventory.sh`)
  * Runs its own headless Ghidra 12.1.2 on `re/gta/1.0-US/gta_sa.exe` (project
    `target/ghidra-cov`; about 10 minutes).
  * `tools/cov/ghidra/ExportFunctions.java` exports every function entry. `tools/cov/names.py`
    then adds names and decides which entries may be armed.
  * Names come from re/gta-reversed (`RH_Scoped*Install`, with the file's `RH_ScopedCategory`)
    and from re/plugin-sdk plugin_sa (`Converted from … 0xADDR`, `META_BEGIN`/`id`).
  * Output: `target/cov/gta-functions.tsv` and `target/cov/arm.txt` (generated, gitignored).
  * Result: **20,823 functions**, 6,846 named (6,619 from gta-reversed), **16,347 armable**.
  * Entries that are not armed, by reason:
    * `section` (4,105): outside `.text`/`_rwcseg`. The other code sections (second `.text`,
      `.data` marked as code, `.HOODLUM`) are the exe's protection layer, and no named game
      function lives there.
    * `data_read` (17): an absolute memory operand anywhere in the image covers the entry byte,
      or Ghidra reports a READ/WRITE reference to it. This is how the protection stubs read code
      bytes as data (for example 0x403794 reads 0x42B7D3).
    * `samp_site` (31): a samp.dll patch or read site covers the entry. Sources: the annotated
      CSV, the 707-entry model-pointer table and the auth read.
    * `client_site` (2): a samp-client patch site with an expected-bytes check covers the entry.
    * `unreferenced` (321): unnamed, with no call/jmp to it and no 32-bit value pointing at it.
* **Tracer coverage mode** (`crates/samp-tracer/src/cov.rs` and `src/win/cov.rs`; additive)
  * Enabled by `SAMP_TRACER_COV=<arm list>`. At attach, after the tracer's own hooks and before
    the client DLL loads, each listed entry gets a one-shot `int3`. This only happens if the
    entry's live byte still equals the exe's byte. Entries inside the tracer's own hook
    prologues are skipped (96).
  * A first-in-chain vectored handler runs on each `int3`. It records the frame, ms, thread
    and return address, writes the original byte back and resumes at the entry. It takes no
    locks and allocates nothing.
  * Control commands:
    * `cov start NAME`: closes the window and re-arms every entry;
    * `cov dump PATH`: writes all windows (`window`/`skip`/`hit` TSV lines);
    * `cov status`.
  * Every window start also writes a `cov_window` trace record.
* **Scenario** (`tools/cov/run.sh [both|orig|ours|calib] [--skip-build] [--no-cov]`)
  * Lanes:
    * original: lane diff (`wine/prefix-diff`, `wine/gta-sa-diff`), samp.dll + tracer;
    * ours: lane diff2 (`wine/prefix-diff2`, `wine/gta-sa-diff2`), samp_client.dll + tracer.
  * One server (`E2E_TAG=diff`, UDP 7795) runs the `tools/diff/diffbase.pwn` gamemode with the
    `tools/cov/covfs.pwn` filterscript. covfs gives each player its own virtual world and
    accepts the RCON commands `grove`, `enex`, `car` and `info`.
  * Each game runs on its own headless gamescope.
  * `tools/cov/covdrive.py` drives each side in its own thread through these windows: `boot`
    (from attach), `connecting` (game state 9), `class_selection`, `spawn` (Shift),
    `grove_idle` (Grove Street, 300 s), `walk` (W 5 s), `interior_enter` (Alhambra entry-exit,
    20 s inside), `interior_exit` (S back out), `vehicle` (Sultan: F, W 3 s, F), `idle_end`
    (60 s).
  * `calib` runs the original in both lanes, which gives the noise floor.
  * Builds go to `target/cov-cargo`. Output goes to `target/cov/runs/<stamp>/`.
* **Report and triage**
  * `tools/cov/report.py` writes `target/cov/coverage-diff.{md,tsv}`. With `--calib` (repeatable,
    other original runs):
    * `only ours` = hit with ours and in no original run;
    * `only orig` = hit in every original run and not with ours;
    * `noise` = hit in some original runs but not all.
    * Functions are grouped by subsystem (name prefix, gta-reversed category, or the named
      neighbours of unnamed functions).
  * `tools/cov/triage.py` writes `target/cov/triage.tsv`. For each `only ours` function it lists
    the CSV rows at the first caller's call site, rows whose text names the function, rows in
    the function itself and rows in the caller. It also shows whether our client applies each
    row (`target/diff/patch-coverage.json`, F5).
* **Locking.** Every run holds `target/locks/diff.lock`. `tools/diff/lane-lock.sh` provides
  `diff_lane_lock`, which `tools/diff/cross-client.sh`, `rpc-coverage.sh` and `tools/cov/run.sh`
  now call. It also accepts an outer `flock … diff.lock …` wrapper, so wrapping a run in flock
  does not deadlock.

## Stability

Coverage mode ran alongside the 101 tracer hooks and completed the full scenario every time:

* `both`: original and ours ran together, all 10 windows, driver exit 0.
  * Original: 21,453 first hits; ours: 21,539. Every hit was handled.
  * Both games were still at game state 9 when the scenario ended (`liveness.log`).
* `calib`: two originals ran together, all 10 windows, exit 0.
* Earlier short runs: `both` passed; single-player boot (`SAMP_TRACER_BOOT=1`) and samp.dll boot
  probes passed.
* No samp-client patch failed. Its log shows no unexpected-bytes errors, and its vehicle hooks
  installed 35/35.

One launch pitfall, now fixed in `run.sh`: a Wine process (`winepath`) started outside the lane's
headless display becomes the prefix's desktop process. The game's fullscreen switch then fails,
and the game exits with code 0 within 0.3 s. `run.sh` now builds Z: paths without Wine.

## Result (runs both-20261104-224308 + calib-20261104-225108; ours = main build of 22:15)

| window | armed | hit orig | hit ours | common | only ours | only orig | noise (orig vs orig) |
|---|---|---|---|---|---|---|---|
| boot | 16251 | 2775 | 2790 | 2775 | 15 | 0 | 3 |
| connecting | 16251 | 1775 | 1907 | 1683 | 91 | 92 | 154 |
| class_selection | 16251 | 1810 | 1619 | 1543 | 76 | 47 | 230 |
| spawn | 16251 | 1779 | 1960 | 1733 | 227 | 46 | 4 |
| grove_idle | 16251 | 1996 | 2071 | 1941 | 129 | 55 | 6 |
| walk | 16251 | 2212 | 2266 | 2156 | 100 | 54 | 136 |
| interior_enter | 16251 | 2339 | 2042 | 1939 | 96 | 305 | 116 |
| interior_exit | 16251 | 2253 | 2259 | 2193 | 65 | 59 | 34 |
| vehicle | 16251 | 2792 | 2819 | 2719 | 85 | 66 | 25 |
| idle_end | 16251 | 1722 | 1806 | 1684 | 114 | 38 | 25 |

Across all windows, **147 functions ran only with ours** and **51 only with the original**.
Most per-window differences that are not in these two lists are timing: the same function runs
in a neighbouring window, for example path-node streaming at spawn.

### Only with ours, triaged (static: CSV row, call site, samp code)

| # | what GTA does with ours | functions | samp mechanism in the original | ours | proposed fix |
|---|---|---|---|---|---|
| 1 | grass/plant manager | `CPlantMgr::Update` and ~30 `CGrassRenderer`/`CPlantLocTri`/`CPlantColEntEntry` | row 158: NOP of the call at 0x53C159 in `CGame::Process` | missing | apply row 158 |
| 2 | real-time ped shadows | `CRealTimeShadowManager::Update`, `CShadows::StoreRealTimeShadow`/`CastShadowEntityXYZ`, `CShadowCamera::*` (~20) | row 165: NOP of 10 bytes at 0x53EA08 in `Idle` | missing | apply row 165 |
| 3 | sun and moon coronas | `CCoronas::DoSunAndMoon` | row 159: NOP at 0x53C136 | missing | apply row 159 |
| 4 | idle camera after ~90 s without input | `CIdleCam::Process/Run/ProcessTargetSelection/…` (~13), `CInterestingEvents::ScanForNearbyEntities` (gated by the idle cam's flag 0xC0B184), `CHandShaker::Process` | row 244: NOP of the call at 0x524582 in `CCam::Process_FollowPed_SA` | missing | apply row 244 |
| 5 | ambient gunfire sounds | `CAEGlobalWeaponAudioEntity::ServiceAmbientGunFire` → `AddAudioEvent` → `CAEWeaponAudioEntity::WeaponFire/PlayGunSounds` and the sound-manager chain (~15) | row 155: 5-byte NOP at 0x4DF7E2 inside `ServiceAmbientGunFire` | missing | apply row 155 |
| 6 | pickups from the IPL files | `CPickups::GenerateNewOne` (from `CFileLoader::LoadPickup`, boot), `CPickup::FindTextIndexForString`, then `CPickup::IsVisible` every frame | row 172: `CFileLoader::LoadPickup` := RET; row 211 (visibility distance 10000) | missing | apply row 172 (start-up); row 211 with M6 |
| 7 | the Shift that spawns also makes the ped jump | `CTaskComplexJump`/`CTaskSimpleJump`/`CTaskComplexInAirAndLand`/`CTaskSimpleLand`/`CTaskSimpleClimb::TestForClimb`, `CEventInAir`, landing audio (~35, 2 frames after the Shift); `CControllerConfigManager::AffectPadFromKeyBoard` and `CPad::GetMouseState` during class selection | rows 349/350: samp's input lock NOPs the calls at 0x541DF5 (keyboard → pad in `CPad::UpdatePads`) and 0x53F417 (`GetMouseState`) while it owns input; during class selection the keyboard never reaches the pad | missing | apply the input lock during class selection; exact release point after the spawn: measure (tracer, original) |
| 8 | GTA's own gang-zone overlay and radar trace colours | `CRadar::DrawRadarGangOverlay` (call at 0x5869BF), `CRadar::GetRadarTraceColour` callees | rows 41/42 (calls redirected to samp, which draws its gang zones), row 51 (`GetRadarTraceColour` → samp colours) | missing | M6 gang zones / M8a radar colours |
| 9 | weapon camera mode cleared | `CCamera::ClearPlayerWeaponMode` (from `CPlayerPed::ClearWeaponTarget`) | row 192: NOP at 0x609CB4 | missing | apply row 192 |
| 10 | vehicle effects | `CPostEffects::SpeedFX`, `CVehicleModelInfo::SetDirtTextures` | row 177 (NOP 0x704E8A), row 62 (call 0x6D0E7E → samp) | missing | M5 (vehicles branch not merged yet) |
| 11 | the player says a line when entering an interior | `CPed::Say` (from `CPlayerPed::ProcessControl` at 0x60F896, after `CEntryExitManager::WeAreInInteriorTransition`), `CAEPedSpeechAudioEntity::*` | unknown: no CSV row covers it; samp's main.scm or a ped field may turn player speech off | — | measure on the original (speech flag of the player ped) |
| 12 | camera fade during class selection | `CCamera::ProcessFade`, `ProcessMusicFade` (per window only) | unknown: the original fades in other windows, not here | — | measure (fade calls in the class-selection trace) |

* Several functions are only unnamed protection fragments in 0x401000–0x40B000 and look like
  noise. Examples: `FUN_00401384`, `FUN_00403961`, `FUN_00405b7e (in CIplStore)`. They are
  pieces of the functions above, entered through the exe's stub jumps.
* Not seen any more, so these SP-OFF gaps from TRACE-4 are closed in main:
  * `CTheCarGenerators::Process`, `CCarCtrl::GenerateRandomCars`, `CPopulation::AddPed`;
  * `CEntryExit::GenerateAmbientPeds`/`PopulateInterior`;
  * the police/wanted chain.
  * No ped other than the player was created with either client (trace `CPed::CPed`).

### Only with the original

* **Head look (IK)**: 39 functions: `IKChainManager_c::LookAt`/`CanAcceptLookAt`/`AddIKChain`,
  `IKChain_c::*`, `BoneNode_c::*`, `CTaskSimpleIKManager`/`CTaskSimpleIKLookAt`.
  * samp calls `IKChainManager_c::LookAt` with the "FollowPedSA" tag through its wrapper
    (build A 0x100AE010; build B call at +0xAE01E).
  * Local player (A 0x10002FA0): every 200 ms, at the point 20 m along the camera's look
    direction (refreshed every 1000 ms), blend 500 ms, 2000 ms duration. It is gated by a
    per-player enable flag (+0x104) and by states checked through 0x100ADCD0.
  * Remote players (A 0x100143A0): the same call with 1000 ms, from synced data.
  * Our client has no head look. This is a feature to add, in M3-P/M8b.
* `CPed::RemoveWeaponWhenEnteringVehicle`, called directly by samp (B +0xAC13D) on vehicle
  enter. Belongs to M5.
* `CBouncingPanel::SetPanel/ProcessPanel`: car body panels after the short drive. All three
  original runs hit them and ours did not. This is probably about how the car was entered or
  hit; recheck after M5 merges.

### Caveats

* `patch-coverage.json` (F5) used for the "ours" column is from the 21:24 run; re-run
  `tools/diff/cross-client.sh --patches-only` for fresh row status.
* Ours is the main checkout as built at 22:15. The vehicles, dialogs-UI and player-parity
  worktree branches were not merged.
* First hit per window only: a function that runs in both clients but much more often in one
  does not show up. Use the tracer's call logs for frequency.
* Entries samp patches at the entry are never armed (`samp_site`), so the "patched at the entry
  by the original" section of the report is empty by construction.

## Re-running

```
tools/cov/inventory.sh                      # once (Ghidra ~10 min), or after names.py changes
flock target/locks/diff.lock tools/cov/run.sh calib --skip-build   # noise floor (original ×2)
flock target/locks/diff.lock tools/cov/run.sh both                 # original + ours, report + triage
```
