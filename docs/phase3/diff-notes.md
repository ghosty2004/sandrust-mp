# DIFF-2 — per-RPC reaction diff (criterion F1)

Goal (PLAN.md §1 F1): for every S→C RPC the R4 client registers (141 ids,
`docs/re/rpc-ids.md`), our client reacts like the original: same GTA call
sequence, same resulting game state, same visible result. The harness fires
one RPC at a time at a joined, spawned player and compares, per step, what the
original client did with what ours did.

## Running it

```
tools/diff/rpc-coverage.sh                    # build, original ×2, ours ×1, report
tools/diff/rpc-coverage.sh --orig-runs 1 --ours-runs 1 --only 'weapon|money' --skip-build
tools/diff/rpc-coverage.sh --report --orig target/diff/runs/orig-A --orig target/diff/runs/orig-B \
    --ours target/diff/runs/ours-C            # report only, from existing runs
```

* Output: `target/diff/rpc-coverage.md` (human), `target/diff/rpc-coverage.json`
  (machine-readable: totals, per-RPC verdicts, per-step details), runs in
  `target/diff/runs/<client>-<stamp>/` (`steps.jsonl`, `step-list.json`,
  `trace.jsonl`, `shots/*.jpg`, `server.log`, `chatlog.txt`, `binaries.sha256`),
  `target/diff/{orig,ours}-latest`.
* Exit 0 only if ours ran and every covered RPC is MATCH (DIFF, NOT-IMPLEMENTED,
  NOT-FIRED and NOT-RUN fail). A summary is printed either way.
* `--only REGEX` limits the fired steps (the idle steps always run). The report
  then marks the RPCs of skipped steps NOT-RUN.
* One run takes about 7.5 minutes (176 steps); the default (3 runs) about 23 minutes.
  `--orig-runs 3` gives a better noise model (see "Validation").

## Lane "diff" (ENV-4)

`WINEPREFIX=wine/prefix-diff`, `GTA_DIR=wine/gta-sa-diff` (only top-level files are
replaced, with rm + cp; never through the hard-linked subdirectories), `E2E_TAG=diff`,
server UDP 7795 in `target/e2e-server-diff`, control ports 47705 (our client,
`SAMP_RS_CTL_PORT`) and 47706 (tracer, `SAMP_TRACER_PORT`), headless gamescope via
`e2e_headless_run`. Cleanup is `wineserver -k` with the diff prefix only, plus the server
stopped by its exact binary path. The harness never calls `e2e_cleanup` and never uses
`pkill`.

Binaries are built into `target/diff-cargo` (`CARGO_TARGET_DIR`), so the build never
touches the other lanes' artifacts. `--skip-build` reuses the last build.
`binaries.sha256` in each run records exactly what ran.

## Pieces

| file | role |
|---|---|
| `tools/diff/diffbase.pwn` | Main gamemode for the lane. One class (skin 60) with a fixed spawn at LV (1958.38, 1343.16, 15.37, 270°). No timers and no randomness. |
| `tools/diff/rpcfire.pwn` | Filterscript holding the step table. `diff list` prints the steps; `diff fire <name> ;` runs one step and prints `[diff] fire <rpcid> <name> step=<idx>`. Each step is one native call with fixed arguments (some steps also need a setup call that sends no RPC). |
| `tools/diff/drive.py` | Runs the scenario inside the headless display. It drives the game over RCON (SA:MP query packet `x`, `rcon.enable` in the lane config only), tracer `mark`s, control-channel `state`/`peek`/`net`, `gamescopectl` screenshots and `xdotool` keys. |
| `tools/diff/gtastate.py` | Extended state snapshot through `peek`. It reads GTA 1.0 US globals named via plugin-sdk: money, clock, weather, gravity, pad lock, widescreen, camera mode/position, HUD flags, weapon skills, the ped's model/pos/heading/health/armour/state/area/vehicle/weapons/fighting style/drunkenness/wanted level, the peds, vehicles and objects within 120 m (from the CPools byte maps), active pickups and radar blips. |
| `tools/diff/rpc_report.py` | Compares the runs and writes the report. |
| `tools/diff/rpc-coverage.sh` | Lane setup, build, server, runs, report. |

Lane server config, set on top of `e2e_server_prepare`:
* `main_scripts = ["diffbase 1"]`, side scripts e2eprobe + rpcfire;
* `rcon.enable` with a random per-run password;
* `max_bots = 2`, for the open.mp NPC;
* `network.time_sync_rate = 1 day`. RPC 60 is then sent only at connect, instead of
  landing in a random step every 30 s.

## Step sequence

176 steps in this order:
1. flow_join and flow_spawn: the join and spawn flow, with marks around it;
2. 2 idle steps (baseline);
3. local player;
4. UI (messages, game text, the 6 dialog styles, textdraws, 3D label, gang zone, map
   icon, checkpoints, menu, audio stream, client check, explosion, camera target);
5. camera;
6. per-player objects (including edit/select/material/attach);
7. per-player pickup;
8. actor;
9. vehicles (params, plate, health, damage, colour/component SCM events, angle, position,
   interior, put in, velocity, remote collisions, remove, truck + trailer attach/detach,
   destroy);
10. remote player: an open.mp NPC (`NPC_Create`/`NPC_Spawn` → 137/32, chat bubble,
    name tag, colour, put in vehicle → 26, exit → 154, spectate player/vehicle →
    124/126/127, `NPC_Kill` → 166, other virtual world → 163, destroy → 138);
11. the destructive steps last: `SpawnPlayer` (129), `RemoveBuildingForPlayer`, spawn
    info, force class selection, death → class selection (128), Shift → 129, Tab
    scoreboard (155, answered to the client's request), `GameModeExit` (40, then 139 on
    re-init).

Using NPCs instead of samp-bot keeps the remote player deterministic and server-side, and
needs no third process.

**Coverage: 131 of the 141 registered S→C ids are triggered.** Not triggerable from Pawn on
open.mp 1.6.7, because no open.mp code sends them:
* 64, 92, 98, 102, 125, 150, 151, 161, 169;
* 130 (connection rejected), which only happens on a bad join.

RPC 60 is only covered inside flow_join.

## Comparison

Per step, three components:
* **trace**: the tracer's records between `step N name begin` and `… end`, normalised by
  `tools/trace/diff.py` (pointers, timing and caller offsets dropped, floats rounded,
  opcodes named).
  * Only records the client caused are kept: client caller, client return address on the
    stack (`origin`), client script command (`scm`/`via scm`), and `watch` diffs.
  * Background is removed: lines in ≥ 50 % of all steps, lines of the idle steps, lines
    already running in the gap just before the step (per-frame polls), and lines in both
    neighbouring steps (samp's periodic per-vehicle/per-player polls).
  * The rest is compared as an ordered list of unique lines.
* **state**: `state` (from the client's own control port; the tracer serves it for the
  original) plus the `gtastate` snapshot, taken before and after the step.
  * Keys that changed on either side must end at the same value.
  * Entity lists compare the added/removed items, without positions.
* **visual**: a 32×18 grid of the screenshot change. The step's shot (640×360 grey) is
  compared with the previous step's shot.
  * A cell counts as changed when > 8 % of its pixels changed by > 60.
  * Masks match when the cells not covered by the other side (allowing a 1-cell shift)
    are ≤ max(4, 40 % of the changed cells).
  * Steps where the original's own masks disagree are judged only on the cells that
    every original run changed.

Determinism: with ≥ 2 original runs, the report first compares the original runs with
each other.
* Trace lines not present in all original runs are noise. Ours must contain the stable
  lines, in order, and nothing outside the union.
* State keys that differ between original runs are ignored.
* The visual check is skipped for steps whose original screenshots disagree.

Steps where nothing can be judged fall back to the handler scan: no trace or state
reaction in the original, and only an unstable screenshot change.

Verdicts:
* MATCH. It is flagged "no observable reaction in the original" when the original showed
  nothing in any channel;
* NOT-IMPLEMENTED: none of the step's RPCs has a match arm in samp-client (static scan of
  samp-proto payload names), or ours shows no reaction at all where the original does;
* DIFF: anything else, including "our client is not running any more";
* NOT-FIRED: a scenario error.

Per RPC, the worst verdict over the steps that cover it.

## Validation of the harness (original side)

The original client was run 3 times with the full scenario (`target/diff/runs/orig-20261104-201031`,
`-201831`, `-202550`; the first had one scenario bug, since fixed: the RCON terminator,
so `team_3` was not fired).

* **Determinism, the two baseline original runs** (201831 vs 202550):
  * 150 of 176 steps are identical in all three components; per component, trace 169,
    state 167 and visual 163.
  * The rest is physics and timing noise: the jump/drunk-walk/vehicle-velocity
    positions, nametag line-of-sight polls with ped positions, per-frame drunkenness
    values, remote-ped positions while spectating, which models are unloaded at gmx,
    flashing gang zones, chat-line timing.
* **Held-out check.** Each original run was judged as "ours" against the other two:
  * 111–112 of 130 fired RPCs come out MATCH.
  * About 6 of the rest are steps whose only reaction is an unstable screenshot change.
    The report does not call these MATCH; it falls back to the samp-client handler scan.
  * The others are the harness's false-positive floor with 2 learning runs.
  * Recurring: 126 spectate_npc, 40/139 gamemode_restart, 90 velocity_up, 26
    npc_put_in_vehicle, 166 npc_kill (trace/state noise from physics and remote
    peds); 58, 121, 103 (visual timing).
  * Treat a DIFF on these as "check by hand" until more original runs are learned.
* The visual metric and thresholds were tuned with the same held-out check. Mean
  cell |Δ| > 18 gave 41 visual false positives over the 3 checks; the pixel-fraction
  metric gives 16, while chat lines and textdraws still register.

## Baseline, our client (2026-11-04)

`target/diff/rpc-coverage.md` from originals 201831 + 202550 vs `ours-20261104-203303`
(samp_client.dll built at 20:06, before M3 landed; hashes in the run's `binaries.sha256`).
Our client survived all 176 steps, including gmx; only its final `quit` needed the lane kill.

| verdict | RPCs |
|---|---|
| MATCH | 15 (12 of them: the original shows no observable reaction either) |
| DIFF | 20 |
| NOT-IMPLEMENTED | 96 |
| UNTRIGGERABLE | 10 |

* MATCH with an actual reaction: 19 SetPlayerFacingAngle, 21 ResetPlayerWeapons,
  66 SetPlayerArmour.
  * Health (14) is a DIFF only through the kill step.
* DIFF examples:
  * 22 GivePlayerWeapon: samp also issues `GET_CURRENT_CHAR_WEAPON`.
  * 29/94 time: ours does not apply the minute / world time.
  * 12 SetPlayerPosition: different follow-up tasks.
  * 15 TogglePlayerControllable, camera 157/158/162, join/spawn flow 60/68/128/129/139,
    remote player 32/137/138/163.
* **Standing differences of ours at idle** (useful for M3+):
  * weather is not forced (ours cycles 10→12, the original keeps forced 10);
  * wanted level 4 (original 0);
  * weapon skills 0 (original 1000);
  * 4 stock weapon pickups exist (the original has none);
  * the two vehicle-entity blips the original has are missing.
  * Ambient traffic in ours also adds visual noise to unrelated steps (e.g. 11, 137, 138).

## Time-varying elements (2026-11-06)

A screenshot samples one moment. Everything that moves on its own clock made the change mask depend on
when the shot landed, so the F1 visual check passed or failed by luck (each full run had one different
flaky step). Measured on the six original runs + ours of the final run (ff689b8):
* **The local ped's idle animation** was the main source, not the gang-zone flash. The changed cells of
  `gangzone_stopflash` (and of player_message, ptextdraw_*, label_*, mapicon_*, gangzone_show/hide,
  menu_show, client_check …) were the ped's outline (breathing/head), right in the middle of the
  picture. In the failing run both clients drew the zone identically (red after the stop); ours just
  missed 3 of the 4 outline cells all six original runs happened to change. DIFF2-D3 (`explosion`) was
  the same effect.
* **Gang-zone flash**: samp redraws the zone every frame (`CRadar::DrawAreaOnRadar`, rgba `0x800000ff`
  zone colour, `0x80ff0000` flash colour) and swaps the colour every ~0.5 s on its own clock (27 frames
  at 52 fps; the first phase after RPC 121 is cut short). The shot catches either colour.

What the harness does now:
* `drive.py` sends `hideped on` to the tracer after `flow_spawn` (new additive tracer hook on
  `CPed::Render` 0x5E7680, wrapped: returns without drawing for `CWorld::Players[0].m_pPed` while the flag
  is set; nothing logged, no game state touched; same on both clients). The screenshots show the scene
  and the UI without the ped; what the ped does is judged by trace and state as before.
  `DIFF_SHOW_PED=1` keeps it. A 2+2 subset run (gangzone/mapicon/ptextdraw/label/checkpoint/racecp/
  menu/client_check/chat steps): 23/23 steps visually identical between the original runs, every
  ours mask equal to the original's (18 of these 23 steps were visually unstable across the six original
  runs of the final run).
* `rpc_report.py` judges the radar draw stream (`radar` component, per step and drawn element
  (function, rect, in_menu)): still drawn at the end of the step; the set of colours (ours must contain
  the colours every original run drew and none no original run drew); flash phase length (median of the
  whole phases, within the original runs' medians ± 2 frame intervals, no phase longer than the
  original's longest + 2 frames); end state (steady colour = last run longer than the longest flash
  phase of any original run, else still flashing), same as every original run. Independent of the
  phase a shot hits.
* The radar region of the screenshot (pixel box 24,256–116,348 of the 640×360 shot) is not judged in a
  step when a radar element changes colour within this step or the previous one, on any side.
* State positions are recorded quantised (camera 1 m, ped 0.5 m, heading 0.05 rad). The first full run
  after the changes failed on `surf_move` with `snap.cam_pos: 1996,1340,48 vs 1996,1341,48`: the camera
  on the moving platform sat on a rounding boundary. These keys now compare per component with a
  tolerance of one quantum (adjacent quanta match; two or more quanta apart still fail).
* `vehicle_remove_player`: GTA's own `CPlaceable::SetPosn` of the local ped at 0x5E09AF after RPC 71 appears in
  2 of 46 original runs and 1 of 3 ours runs; a 12-run calibration without one of them called it extra in ours.
  Logged as DIFF2-D4 (that template only).
* Full runs after the changes (6 new + 6 reused original runs each): original visual determinism 173–177 of
  179 steps (was 156); `rpc-coverage.sh` exit 1 (surf_move rounding, fixed), 0, 1 (DIFF2-D4 case);
  `--report` of the two failing runs with the final report: exit 0, MATCH 131.
* DIFF2-D3 (`explosion` screenshot = the ped's breathing pose) is superseded: row removed from
  `rpc-decisions.tsv`, the step is judged in full (0 changed cells on both sides).
* Negative tests (`tools/diff/negative_tests.py f1`, on copies of a recorded ours run): zone keeps
  flashing after RPC 85, flash ignored, wrong flash colour, phase twice as long, zone not hidden by
  RPC 120, a player textdraw not drawn (screenshot of the previous step), SetPlayerPos landing 1 m off
  (two quanta) — every case turns the step from MATCH into DIFF / NOT-IMPLEMENTED.

## Open problems

* Some entities are placed outside the camera view (actor at +6/+3, car at +7/+6 relative
  to a player looking east). Steps like `actor_anim_clear` and `vehicle_damage` therefore
  show no visual reaction even in the original. Placing them in view would make the
  visual check stronger.
* The noise model is learned from the original runs. More `--orig-runs` means fewer
  false positives on the physics/remote-player steps listed under "Validation".

* The trace only shows hooked GTA functions plus samp's script commands. Effects done
  with direct calls to unhooked functions are visible only through state and screenshots:
  * clock, weather, money (CPlayerInfo write), explosions (`CExplosion::AddExplosion`),
    pickups (`CPickups::GenerateNewOne`), blips, audio (`CAudioEngine`), HUD/text;
  * they would need additive tracer hooks. Not done here, because TRACE-2 owns
    samp-tracer right now.
* Many UI reactions (chat, dialogs, textdraws, 3D labels, gang zones, game text,
  scoreboard, nametags) are drawn by samp itself. Only the screenshot check covers them.
  The 32×18 change mask is coarse; DIFF-4 (per-element visual diff) is the real check.
* `peek` reply formats differ: samp-client prints one hex run, the tracer separates bytes
  with spaces. `gtastate.Peeker` accepts both.
* The RCON text reaching `OnRconCommand` through the query port has stale bytes after
  its real length. drive.py terminates commands with ` ;`, and rpcfire takes only the
  leading `[a-z0-9_]` run.
* Steps run in one session, so a missing reaction in ours can change later steps. For
  example, if ours does not teleport, every later position differs. The delta-based state
  comparison limits this, but does not remove it.

# DIFF-3 — cross-client (criterion F3)

The original client and ours on ONE server at the same time. Each player's own
local ped is compared with the ped the OTHER client shows for it, both directions.

```
tools/diff/cross-client.sh [--skip-build]       # ours vs original (~4 min), report
tools/diff/cross-client.sh --calibrate          # original vs original: noise floor + the original's own relations
tools/diff/cross-client.sh --report RUN_DIR     # report only
```

* Lanes: A = original `samp.dll` + tracer (`wine/prefix-diff`, `wine/gta-sa-diff`, ctl 47706);
  B = our client (`wine/prefix-diff2`, `wine/gta-sa-diff2`, ctl 47705; `--calibrate`: a second
  original + tracer on 47705). One server: `E2E_TAG=diff`, UDP 7795, `diffbase` +
  e2eprobe + `tools/diff/xclient.pwn` (RCON `xc tp|hpar|skin|sa|info`). Each game runs on its own
  headless gamescope (`E2E_DISPLAY_TAG=xc-a` / `xc-b`; `tools/e2e/lib.sh` now supports several displays
  at once). `wine-env.sh` is sourced per lane (private Documents, headless video mode).
* Driver `tools/diff/xclient.py`: join, Shift on both until spawned, teleport both to the LSIA runway
  (5 m apart, facing west), dump the patch bytes (F5), then 17 steps. Keys go to the actor's display only.
* What is read: the same code on both sides, through `peek` (gta_sa.exe 1.0 US, plugin-sdk offsets):
  model, position, heading, health, armour, move state, `bIsDucking` / `bIsInTheAir` / `bIsStanding`,
  and the RpClump's animation blend list (hierarchy hash, group, id, blend). The other player's ped
  comes from `remote` (ours: the line with that name; tracer: new `remote` command, header +
  one line per cached `Players[slot>=1]` ped). The tracer serves one connection at a time, so the
  driver keeps one pipelined connection per side (also for `state`).
* Checks per step: visible, pos, heading, skin, health, armour, ducking, in_air, move, anims (the
  sender's new animations must appear on the observer's ped within the window), anim_final.
  Animations are compared by hierarchy hash (what is drawn), not by association group.
* Calibration (`--calibrate`, 3 runs, identical results): the original keeps every relation except
  health/armour (a remote ped's GTA health is always 1000 and armour 0; samp keeps the synced values
  elsewhere). Largest original deviation 0.075 m / 0.011 rad. The report then uses
  tolerances = 1.5 × that (floors 0.1 m, 0.03 rad), judges relations the original does not keep by
  comparing our observer with the original's observer, and adds `sender_vs_original`: the acting
  player's own ped must play what the original's did in the same step (else "sender == observer"
  holds trivially when neither side applied an RPC).
* Vehicle (M5), combat (M7), objects (M6) and chat (M4) steps are listed as PENDING.
* Output: `target/diff/cross-client.{md,json}` (+ `cross-client-calibration.*`), runs in
  `target/diff/runs/xclient-*` / `xcalib-*` (`samples.jsonl`, `meta.json`, patch dumps, logs).
  Exit 0 only if every enabled step matches both ways.

## Animations: what the original observer shows (2026-11-06)

The final run's `veh_enter_B ours→orig` DIFF ("missing on observer `1ccd46ef/0:63`") was not a sampling
miss. `1ccd46ef` is `CAR_sit_pro` and `218ee4bb` is `CAR_sit` (CRC of the names in `anim/ped.ifp`). The
sender ends the enter in `CAR_sit_pro` and holds it for the rest of the window (> 20 samples); the
ORIGINAL observer in lane A shows `CAR_sit` instead, for the whole window, whoever the lane-B sender is:
2 of 2 ours→orig runs and 3 of 3 valid original-vs-original runs (orig2→orig). Lane B's observer shows
`CAR_sit_pro` for lane A (orig→orig2 and orig→ours). The vehicle steps simply had no calibration on
main (`xcalib-20261104-211825` predates them), so the original's own behaviour was judged as ours.

Report rules now (`xclient.py report`):
* Several calibration runs: `cross-client.sh` passes every `runs/xcalib-*`; the report uses the newest
  three with the current step list. A calibration run whose two originals are further apart than the
  default tolerance (0.5 m / 1.0 m vehicle) is not used: `xcalib-20261106-010557` lost the respawned
  player on the observer (`combat_respawn_A` 1137 m) and every later step with it — the original can
  desync like that, but it measures a failure, not the noise floor.
* `anims` where the original observer missed sender animations in calibration: ours must miss exactly
  what every calibration run missed, and nothing no calibration run missed (`must ⊆ missing ⊆ may`).
  Before, such a check was "not judged".
* Sub-sample tolerance, measured: a sender animation is required on the observer when held (blend ≥ 0.5)
  for at least `min_hold` samples (~100 ms each); `min_hold` = 1 + the longest animation the original
  observer showed in one calibration run and missed in another (a miss that can only be sampling, both
  observers being the original), at least 2. Currently 2: no such transient miss in the three valid calibration runs.
* `sender_vs_original` (multi-run): the anims every calibration sender played must be played by ours.
* **Respawn after `combat_death_*`**: in 2 of 9 runs (one original-vs-original, one ours run; both times the
  ORIGINAL in lane A) the original never requested its spawn after the death. Its trace shows GTA's own
  restart at +3.61 s (`CPed::Teleport` to 2488.6,-1666.9) and then no samp spawn, where a good run spawns at
  the gamemode position in the same frame; the server keeps the player wasted (state 7), the timed `tp`
  is lost and every later step is off by ~1137 m. The driver now checks at +2.0 s of `combat_respawn_*`
  whether the server saw a spawn since the death began; if not, it spawns the player (`xc spawn` →
  SpawnPlayer), teleports at +3.5 s (window 6.5 s) and records `forced_spawn`. The report adds
  `self_spawn`: not judged for an original sender (its own flake), DIFF for ours.
* Negative tests (`tools/diff/negative_tests.py f3`): our observer without the door/get-in anims
  (`veh_enter_A`), our sender without them (`veh_enter_B`), the original observer showing `CAR_sit_pro`
  for our sender, our client not requesting its spawn after the death — each turns MATCH into DIFF.

## Baseline (2026-11-04, run xclient-20261104-212008, calibration xcalib-20261104-211825)

13/24 enabled step-directions MATCH, 11 DIFF, 18 pending.
* MATCH both ways: spawn_idle, health_armour, tp_A, tp_B; orig→ours jump/uncrouch; ours→orig
  jump/crouch/uncrouch. Positions within 0.09 m, headings within 0.011 rad.
* Our local ped walks with other animations than samp's (hash f46a419c/a9f89004, group 54, vs
  samp's a38efa5f/66296341, group 118; idle is the same hash). Our view of the original also uses
  group-54 walk anims (walk/turn orig→ours DIFF); the original shows our walk with its own anims
  (walk/turn ours→orig DIFF + sender_vs_original). Probably the CJ-walk anim-group rows 43/187.
* Crouch: the original shows our crouch; ours does not show the original's (ducking 1/0).
* SetPlayerSpecialAction(HANDSUP) and SetPlayerSkin are applied neither to our local ped nor to the
  remote ped in ours (handsup, handsup_off, skin DIFF both ways).

# F5 — patch coverage

```
tools/diff/patch-coverage.sh [--skip-build]     # boots both (cross-client.sh --patches-only), dumps, reports
tools/diff/patch-coverage.sh --from RUN_DIR     # from any cross-client run (they dump too)
```

* Live bytes at every CSV row's address (row = CSV line number), read 5 times ~0.3 s apart on each
  side after join + spawn + teleport: original via the tracer's `peek`, ours via its own; unmodified
  bytes from `re/gta/1.0-US/gta_sa.exe`. Pointer / call / jmp rows count as matching when both
  clients redirect out of gta_sa.exe. Rows the original had not applied at the snapshot (runtime,
  temporary, conditional, dead) and rows without plain addresses (348, 382, 383) fall back to the static
  tables of samp-client (`csv_row` in patches.rs / sectors.rs / stores.rs, "CSV row N" comments).
  Row 347 (sector-grid zero fill) is not live-checkable (the memory holds Players[2..] afterwards).
* `tools/diff/patch-justifications.tsv` (row, reason, decision link): rows 61 (EU-only stub), 370 and
  375 (dead code in samp), reasons from re10-gaps.md and the specs. Their ROADMAP decision entries
  are still to be written by the main session.
* Owning milestone of missing rows: re10-gaps.md "CSV rows Mx must apply".
* Rows 0x6884C4/0x688200 (213/214/367): samp toggles them around remote-ped processing; one
  snapshot once caught the restored bytes (before multi-snapshot dumps). Rows the tracer itself
  rewrites in the original's lane are annotated (19, 26).

## Baseline (2026-11-04, run xpatch-20261104-212400)

applied/matching 173, applied (static only) 3, applied/different 0, missing 205, justified 3.
Missing by owner: M5 70, M6 37, M7 29, M5/M6 2, M5/M7 1, not M5–M7 66 (start-up/SP-OFF/M3/UI/M8b/M9).
To look at: row 269 (samp writes it live; sectors.rs deliberately skips it — justify or apply),
rows 380–384 (marked dead in the CSV). But 383 (0x71A223 := 03) and 384 (0x704E8A NOP ×5,
0x609A4E call → samp) ARE written in the original's live memory, so the "dead" label is wrong for
them or another live path writes the same bytes: needs RE before a justification.
348/362/364: dormant custom-model path, justify for R4 per re10-gaps.
