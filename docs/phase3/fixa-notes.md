# FIX-A — parity fixes from the audit (par-audit.md)

Branch `fix-patches` (worktree `samp-rs-wt/fixa`). Static reads are samp.dll A (2021 build) unless noted;
measurements are on the trace lane (original vs ours, same scenario) and the diff lanes.

## 1. HUD aspect correction (CSV rows 7, 16, 368, 369; row 8 completed)

Static (A):
* 0x100A2440 (correction): runs only with the flag A 0x10117470 (initial value **1** in .data; start-up clears it
  when sa-mp.cfg `nohudscalefix` equals exactly 1 — `cmp eax, ebx` with ebx = 1 at A 0x100C49E7; `/hudscalefix`
  toggles it). It saves 0x859520/0x859524, reads `RsGlobal` width/height (0xC17044/48), returns when either is ≤ 0;
  aspect = w/h on the x87 unit; below 1.6 → HUD floats 0x866B74/78 := 76/94 only; otherwise HUD floats := 82/96,
  0x859524 := 0x3B1E98DD, 0x859520 := 0.00222/aspect and a "corrected" mark.
* 0x100A2540 (row 7 radar wrapper): correction, `CHud::DrawRadar`, restore (saved floats back, HUD floats 76/94,
  mark cleared) when marked.
* 0x100A2590 (row 8 crosshair wrapper): the same around `CHud::DrawCrossHairs`, but only while the local camera
  mode (`*0xB6F1A8`, through A 0x1009C920 for the local ped) is 0x35.
* Row 16 (A 0x100A2420) is **not** an aspect hook (CSV text is wrong): it calls `Sleep(1)` and then
  `CTimer::GetCurrentTimeInCycles` — a 1 ms sleep in every iteration of Idle's 14 ms busy-wait. Applied
  (`mainhooks::idle_wait_time`); it exposed the world-bounds jump gap below, which is now fixed.
* samp makes both .rdata ranges writable once at start-up (A 0x100AAEF0); ours too (`mainhooks.rs`).
* The earlier M7 note ("flag is 0 unless nohudscalefix ≠ 0; crosshair wrapper = original") read the start-up test
  inverted. Its measurement (0x859520 unchanged while aiming) peeked between frames, after the wrapper had restored
  the values, so it cannot see the correction; the crosshair wrapper now applies it like samp.
* The aspect test and the division run on the x87 unit (inline asm), so 16:10 (exactly 1.6) follows the game's FPU
  precision like samp's code does.

`nohudscalefix`: `ui/glue.rs` sets the flag at UI creation from the cfg (`!= 1`, like samp — `UiSettings` uses
`!= 0`); `Effect::SetHudScaleFix` (`/hudscalefix`) is applied in `ui/core.rs`. The m9 lane owns the cfg keys: if
it moves this, keep `mainhooks::set_enabled` as the sink.

Result (DIFF-2 ours `ours-20261105-030413` vs original `orig-20261105-013339`, step `idle_a`, 640×360 shots):
mean abs difference full frame 2.57 → **0.14**, radar box 45.6 → **0.0** (round radar); the remaining pixels are the
ped's idle pose and one chat glyph.

### Row 16 and the world bounds (bounds jump, fixed)

With row 16 the m8b bounds step (walk east into maxX 1720 for 3 s; pass = x < 1721.5) failed 5 of 6 runs, main
1 of 3. Measured with the effect tracer (trace hold, `pos 1716,-2600`, `bounds 1720|1680|-2580|-2620`, W 3 s):
* call window (`CTaskComplexJump`/`CTaskSimpleJump`/`CTaskSimpleInAir` ctors, `CPad::JumpJustDown`): in the
  original `JumpJustDown` returns 1 once (frame 738) and `CTaskComplexJump` is created from `PlayerControlZelda`
  (0x6887BF) in the same frame; in ours it never returns 1 (261 calls). Original ends at x 1708–1712 with 10–37
  health lost to the landing, ours at 1732.
* memory window on `Pads[0]` NewState/OldState Square (0xB73474 / 0xB734A4): samp B+0xA2B70 / +0xA2B7C writes
  NewState.Square := 0xFF and OldState.Square := 0 inside `CGame::Process`, the first time in the frame of the jump,
  then every frame (6 frames, 12 writes) while outside.
* static B 0x100A2AF9 (the local branch of samp's `CPlayerPed::ProcessControl` hook, before the call): when the
  local player is spawned (+0xF0), outside the bounds (B 0x1009F5F0, strict compares), GET_AREA_VISIBLE is 0 and
  the ped flags +0x46C have bit 8 clear and bit 0 or 1 set, samp presses jump on pad 0 (pointer A/B data → 0xB73458);
  the carry/cuffs masks follow.
Fix: `behaviourclient::bounds_jump` from the local branch of `swap::player_ped_process_control`, before the masks;
the client tick publishes the bounds while spawned. Re-measured on ours: first pad write and `CTaskComplexJump` in
the same frame, 12 writes, ends at x 1712.6 with health 90 (original 1711.9 / 90 in the same window).

## 2. Shadows (rows 165, 184, 166)

Applied as table rows. `idle_a` after spawn: no black world; outside the radar 0.15 mean abs to the original (only
the ped's pose differs; neither client draws a visible ped shadow in that scene).

## 3. Byte rows

Rows 225–239 (CFont line buffer 0x12C → 0x32C), 223/224 (phone model −1), 170 (no editable event response),
215/216/217 (`CPlayerInfo::Process` / `CPed::Render` NOPs) as table rows.
Row 50 (A 0x100A3AA0): keeps `m_bHasPlayerCheated := 1` and resumes at 0x438581, skipping both the cheat function
call and the toggle branch — typed cheats never run. samp also stores EAX in a global with no other reference; not
replicated.

## 4. SetWeather (audit L14)

Same-position measurement (trace hold, `pos 2495,-1680,13.4`, time 15:00, weather 10 → 8 → 10; peeks of the
`CWeather` block 0xC812E0..0xC81347 at +0/1/2/4/8/16 s): both clients hold identical weather state (old/new/forced,
interpolation, rain/fog/cloud ramps — values differ only by sample timing) and change the screen equally
(mean brightness 114 → 105–137 in both, fluctuating with weather 8's lightning). The DIFF-2 difference was the
scene: ours stood under the roofed bridge because **RPC 13 SetPlayerPosFindZ was not implemented** (step
`pos_findz`; the original lands on top at z 20.5, ours stayed at 15.5), so less sky/rain was visible. Fixed by
implementing RPC 13 (0x02CE ground Z + 1.5, then the RPC 12 teleport).

## 5. 207 after leaving a car (audit P4)

Captures (samp-dump `--raw`, gaps between consecutive C→S 207 after the last 200):
| run | client | cadence after exit | struct |
|---|---|---|---|
| trace `vehicles` orig-20261105-030230 | original | 0.531–0.537 s | identical (z 0x4158E000) |
| trace `vehicles` ours-20261105-030539 | ours (this branch) | 0.531–0.537 s | identical (z 0x4158E000) |
| COV scenario (20 s idle, 40 s end), capture `cov1` | ours (this branch) | 0.532 s, 3 gaps of ~1.03 s (a 203 on the shared timestamp) | identical |
| same, capture `cov0` | ours, main d1ec649 (no FIX-A) | 0.533 s, 4 gaps of ~1.03 s | identical |
| same capture, another lane's COV `both` run | original / ours | 0.532 s, 3 / 3 gaps of ~1.03 s, 4 / 4 203s | identical |

The 76 ms flood with a 1-ULP z alternation (audit run `both-20261105-012535`, client built from 9abe8a7) does not
reproduce with current main nor with this branch in five sessions; the send logic (`footsync.rs`) already matches
§0.4. Not fixed by a code change here; if it reappears, the audit's suggested per-frame z peek is the next step.

## Results

* F5 (`tools/diff/patch-coverage.sh --skip-build`, report regenerated with this worktree's sources so runtime rows
  368/369 are seen statically): main d1ec649 **56 missing / 2 different** (run xpatch-20261105-030127) → this
  branch **27 missing / 2 different** (final run xpatch-20261105-050508): rows 7, 16, 50, 165, 166, 170, 184,
  215–217, 223–239 live-matching, 368/369 applied (static only). The two "different" rows are unchanged and not
  from this work: 52 (`jmp rel32` vs samp's `jmp [slot]` on `CGame::Shutdown`) and 106 (InitGame-time
  `InflictDamage` hook present in ours, not in the original at the snapshot).
* m8b with row 16 and the bounds jump: 5 PASS of 5 in a row (lane main).
* DIFF-2 per RPC (ours runs against the audit's two original runs): main d1ec649 `ours-20261105-031703`
  MATCH 82 / DIFF 44 / NOT-IMPLEMENTED 5; this branch `ours-20261105-033052` **MATCH 102 / DIFF 24 /
  NOT-IMPLEMENTED 5** (RPC 13 removes the position cascade: 152 weather, 153 skin, 156 interior, 35 drunk,
  checkpoints, gang zones, labels, … now MATCH). Changed the other way, all noise: 70 `vehicle_put_player` (ped x
  1965.228/1965.249 in ours vs 1965.259/1965.289 in the originals and 1965.252 in main — the snapshot rounds to 0.5 m
  across 1965.25), 167 `vehicle_collisions_on` (7 cells; MATCH in this branch's first run), 28 `object_edit_cancel`
  (DIFF in main and in the first run, NOT-IMPLEMENTED in the second).
* Exposed by RPC 13: after step 52 `velocity_up` (RPC 90) ours lost 5 health in DIFF-2, both original runs none.
  Measured in isolation (trace hold, health 50 / armour 75, `pos 1958.4,1343.2,21.2`, `vel 0,0,0.4`, call window on
  `CPedDamageResponseCalculator` ctor / `ComputeDamageResponse`), five paired runs: both clients identical every
  time — 4× one fall-damage response (weapon 54, 15.0 → health 50 → 40) from 0x5E303E, 1× none; landing x
  1955.597/1955.598. RPC 90 matches samp (B 0x10018800: ground bits flipped, speed set). The damage depends on the
  landing frame, not on the client; DIFF-2 lists `velocity_up` as unstable in the original. No change.
