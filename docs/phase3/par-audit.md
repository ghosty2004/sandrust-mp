# PAR-AUDIT — parity audit of main (excluding combat)

Retroactive parity loop over everything in main at 9abe8a7 (`crates/samp-client` is unchanged up to
0c92f5c; the later commits change docs and the tracer only): protocol/bot, boot/join/spawn, local and remote players, samp.saa serving,
SP-OFF, chat, dialogs/textdraws/menus/nametags/scoreboard, loading screen, vehicles and world entities.
Combat (M7, worktree m7) is excluded. This is a measurement document: it lists the remaining
differences, their evidence and root cause, a proposed fix and the owning area. Residuals already
covered by a ROADMAP "Decisions made autonomously" row are not repeated as findings.

## Method

All DLLs were built from a clean export of 9abe8a7 (`git archive` → `target/audit-src`,
`CARGO_TARGET_DIR=target/audit-cargo`), so the uncommitted tracer work of other lanes in the main
checkout is not part of the measurement. Every run used the baseline `sa-mp.cfg`
(`fpslimit=48 multicore=1 audioproxyoff=0`) in all three prefixes (every EFX run records `source: baseline`, sha256 4888b719… in `samp-cfg.txt`); an EFX run that booted the
original with a leftover test cfg (orig-20261105-010542) was discarded and re-run.

| tool | command | run directory |
|---|---|---|
| DIFF-2 (F1) | `DIFF_CARGO_TARGET=target/audit-cargo tools/diff/rpc-coverage.sh --skip-build`, then `--orig-runs 2 --ours-runs 0`, report `rpc-coverage.sh --report` over the two complete original runs | ours `target/diff/runs/ours-20261105-011043`; original `orig-20261105-013339`, `orig-20261105-014057` (the first original run `orig-20261105-010048` crashed inside samp at step 141 `vehicle_put_player` — "SA:MP has crashed", SCM op 0x470 — and a second one never joined; both left out); report `target/audit/diff2/rpc-coverage.{md,json}` |
| DIFF-3 (F3) | `tools/diff/cross-client.sh --skip-build` | `target/diff/runs/xclient-20261105-012126`, report `target/audit/cross-client.md` |
| F5 | `tools/diff/patch-coverage.sh --from <DIFF-3 run>` | `target/audit/f5/patch-coverage.{md,json}` |
| COV-1 | `COV_BIN=target/audit-cargo/... tools/cov/run.sh both --skip-build` (calibration: existing `calib-20261104-225108`) | `target/cov/runs/both-20261105-012535`, report `target/audit/cov/coverage-diff.md` |
| network (F2) | `dumpcap -i lo -f "udp port 7795"` during the COV run, split per client port, `samp-dump --raw`, `target/audit/net/c2s_compare.py` | `target/audit/cov-net.pcapng`; original = port 59075 (`CovOrig`), ours = 58110 (`CovOurs`); report `target/audit/net/cov-c2s.md` |
| EFX-1 | `target/audit/efx-chain.sh g1 g2 g3` and `efx-g4.sh g4` (= `EFX_STEPS=… tools/efx/run.sh both` from the clean export, `EFX_LANE_ROOT` = main checkout) | `target/audit-cargo/efx/{orig,ours}-*` (see "EFX") |
| row 165 | scratch worktree `samp-rs-wt/audit` (row 165 added to `patches.rs`), `rpc-coverage.sh --orig-runs 0 --only 'flow_join|flow_spawn|idle_a|idle_b'` | `target/diff/runs/ours-20261105-012052` |
| interior | COV traces + coverage + gta-reversed (`EntryExit.cpp`, `EntryExitManager.cpp`, `Population.cpp`) | `target/cov/runs/both-20261104-234335`, `both-20261105-012535` |

DIFF-4 (visual.sh) was not re-run: its "ours" side needs lane ui, which was held by another lane for the
whole audit; its last result (all 25 elements clean) is from the UI merge (820e431).

## Headline numbers

| criterion | result |
|---|---|
| F5 patch coverage (`target/audit/f5`) | 384 rows: **292 applied** (273 live-matching + 19 applied at runtime/static only), **1 different** (row 52, equivalent hook form), **88 missing**, 3 justified. Missing by owner: 27 M7 (combat, out of scope), 6 M6, 4 M5, 1 M5/M6, 50 other (start-up/SP-OFF/UI/M8b/M9). Before the scanner fix: 284 applied / 96 missing (rows 349–353, 371–373 were false positives) |
| DIFF-2 per RPC (F1, `target/audit/diff2`) | 131/141 S→C ids fired; **MATCH 64 · DIFF 37 · NOT-IMPLEMENTED 30 · UNTRIGGERABLE 10**; original determinism 155/176 steps identical (2 runs). The same runs read 27/66/38 before the two report fixes |
| DIFF-3 cross-client (F3) | **24/24** enabled step-directions MATCH; 18 PENDING (vehicles, objects, chat steps never added; combat) |
| COV-1 (only-ours / only-orig per window, 3 original runs incl. calibration) | boot 1/0 · connecting 1/57 · class_selection 78/4 · spawn 25/1 · grove_idle 31/19 · walk 32/119 · interior_enter 78/270 · interior_exit 77/8 · vehicle 42/10 · idle_end 44/4. Never hit by the original in any window: 44 functions (real-time shadows 23, interior-transition speech/collision/attractors 18, SpeedFX, SetDirtTextures, `CSprite2d::Draw2DPolygon`); never hit by ours: 4 (`CDoor::Process` ×2, `RemoveWeaponWhenEnteringVehicle`, a streaming helper) |
| Network C→S (F2) | same message set, order, reliability and channel for connect/join/class/spawn/walk/drive; differences: P1–P4 (ping, keep-alive, disconnect reliability, 207 idle cadence after exit) and P5 (207 animation id) |
| EFX per RPC (26 RPC ids attempted in 4 groups) | same effect: 19, 15, 164, 44, 47, 73, 122 · same calls, other order: 24 · differs (internal): 99 · not implemented: 13, 18, 146, 156 · no GTA-side effect in either: 61, 134, 135 · camera 157/158/162: same end state · not measured (scenario): 147, 159, 160, 165, 71, 95, 63 |
| Interior entry | root cause found: RPC 156 not implemented (details below) |
| Row 165 | world renders normally with row 165 applied — safe to apply (with 184) |

### Priority list (non-combat, by severity)

1. I-1 RPC 156 SetPlayerInterior (M8b) — breaks every interior/enex flow and cascades into COV/DIFF-2.
2. B1 HUD aspect ratio (rows 7/16/368/369) — elliptical radar at 16:9.
3. R3 remote player radar markers missing (`players.rs`/`remote.rs`).
4. V7 remote driver exit warps instead of the exit task (`vehicles.rs`).
5. V6 AddVehicleComponent load/add order (`vehicles.rs`).
6. V2 car door does not swing after exit (unknown, `vehicles.rs`).
7. V1 SpeedFX / dirt textures (rows 177/62, `vehpatches.rs`).
8. L6 CFont line buffer rows 225–239 (`patches.rs`).
9. J1 cheat interception row 50; L3/L4/L5 phone model, facial expressions, `CPlayerInfo::Process` rows (`patches.rs`).
10. L14 SetWeather visible change (unknown, `world.rs`).
11. L1 + L2 real-time shadows (165/184) and stencil alpha (166) (`patches.rs`).
12. W3 attached-object editor gizmo (M6-P).
13. P1/P2 ping and keep-alive cadence, P3 disconnect reliability (`connection.rs`, NET-6).
14. P4 207 flood after leaving a car (unknown).
15. P5 207 animation id (M8b).

## Harness and tooling fixes made during the audit

| file | fix | why |
|---|---|---|
| `tools/diff/rpc_report.py` | handler scan also matches nested match arms (`Incoming::Ui(ui::Incoming::ShowDialog(d)) =>`) | dialogs, textdraws, menus, client messages and the scoreboard were reported NOT-IMPLEMENTED although `ui/core.rs` handles them (10 RPC ids) |
| `tools/diff/rpc_report.py` | `los` lines drop the `gta_stack` field | the stack scan of a `los` record picks up stale return addresses in our client, so every frame looked different: 31 steps were DIFF only because of this |
| `tools/diff/rpc-coverage.sh` | a failed original run (no join) is left out of the report; no report without a usable original run | the report crashed (`ValueError: 'flow_spawn' is not in list`) after one original run failed to join |
| `tools/diff/patch_report.py` (F5) | static scan is recursive (`ui/`), also reads `Row N` comment lines and `out.push((N, …))` installers, and accepts a runtime row whose code address appears as a constant | the input-lock rows 349–353 (ui/inputlock.rs) and the escalator rows 371–373 (worldhooks.rs) were reported missing |
| `tools/cov/run.sh` | `COV_BIN=DIR` takes tracer/launcher/client from a prebuilt release dir | measure a clean build without rebuilding into `target/cov-cargo` |
| `tools/efx/efxfs.pwn`, `tools/efx/scenario.sh` | 26 new single-RPC commands and the steps `posz angle ctrl money grav int campos vehin vparams vhp vpos remove vehdel obj pickup dlg gametext td` | EFX coverage of the non-combat RPCs main handles |

## Interior entry — root cause (COV-1 interior_enter: 78 only-ours / 270 only-orig)

**Root cause: RPC 156 SetPlayerInterior is not implemented in samp-client.** The COV scenario walks south
from Grove Street into CJ's house (GTA enex, area 3) in the `walk` window, then `cov enex` does
`SetPlayerInterior(0)` + `SetPlayerPos` outside the Alhambra. The original runs `0x04BB(0)`,
`0x0860(player, 0)`, `0x04E4(x, y)` in that frame (trace `both-20261104-234335/A-trace.jsonl`, window
`interior_enter` frame 0); ours runs none of them, and its next `0x077E GET_AREA_VISIBLE` returns **3**
(original 0). Our client therefore stands outside the Alhambra while GTA still renders and streams
area 3 (CJ's house). Everything else follows from that, in GTA's own code (gta-reversed):

1. `CPopulation::ManageDummy` converts a dummy into a real object only when `IsInCurrentArea()`; the
   Alhambra door dummies (model 1533, `Gen_doorEXT12`, generic/dynamic2.ide) are area 0, so in ours they never become
   `CObject`s (original: `CreateRwObject` of three model-1533 objects 12–14 frames after the teleport).
2. `CEntryExit::TransitionStarted` (both clients reach it at the same point, 265 / 267 frames after the
   teleport, ~51 frames after W) calls `CEntryExitManager::FindNearestDoor(…, 10 m)`. Original: the door
   is found → `CTaskComplexGotoDoorAndOpen` (vtable 0x86FFC0) + `SetCamPositionForFixedMode` +
   `TakeControlNoEntity` + `CTaskSimpleUninterruptable`; 54 frames of fade; teleport. Ours: no door and
   the exit is > 10 m away → the warp branch: it sets the enex's `bUnknownPairing` flag permanently and
   returns without a door task, fixed camera or uninterruptable task. `TransitionFinished` then takes the
   warp path (fade-in from an unfaded screen completes at once) and teleports **2 frames** later.
3. Because no uninterruptable task holds the ped, W (still held for ~75 frames) keeps walking it inside:
   ours ends at y −19.21, the original at −21.01 (the "2.6 m" of the earlier note is this extra walk, not
   a short stop). The persistent `bUnknownPairing` also explains the missed exit marker seen in one of
   three earlier runs, and `CPed::Say` (context 338) from `CPlayerPed::ProcessControl` 0x60F896 after
   `WeAreInInteriorTransition` (only ours).
4. The 270 only-orig functions are the original being in area 0 at that time: outside rendering (radar
   sections, water, clouds, 3D enex markers), `CStreaming::RemoveBuildingsNotInArea`/`RequestCollision`
   (from 0x04BB/0x04E4), outdoor vehicle/ped streaming, the door task, fixed camera, door audio. The same
   missing 0x04BB/0x04E4 pair shows in `grove_idle` and `vehicle` (only-orig `RemoveBuildingsNotInArea`
   from `ProcessCommands1200To1299`) and in DIFF-2 `interior_1`/`interior_0` (NOT-IMPLEMENTED, area 1 vs
   0, 391 changed screenshot cells vs 0).

No CSV row is involved: TRACE-8 measured the walk-in as pure GTA flow, and all the enex rows (164, 208)
are applied; 212 (`InteriorGroup_c::Exit` NOP) is missing but is not on this path.

**Fix:** implement RPC 156 as measured (TRACE-8 "Interiors (156, enex)": `0x04BB(i)`, `0x0860(player, i)`,
`0x04E4(x, y)` in the same frame, then RPC 118 once per change). Owner: M8b (worktree m8b,
`crates/samp-client/src/client.rs` RPC routing + a local-player handler). Re-run `tools/cov/run.sh both`:
the interior_enter/grove_idle/vehicle only-orig groups above should disappear.

## CSV row 165 re-test (real-time shadows)

Scratch worktree `samp-rs-wt/audit` (detached 9abe8a7, never committed) with row 165 added to the
main patch table (`Idle: NOP mov ecx,g_realTimeShadowMan; call CRealTimeShadowManager::Update`, 10
bytes at 0x53EA08, original bytes checked). Log: `main: applied 0x53ea08 … (experiment)`. Screenshot
after spawn on the isolated display (`ours-20261105-012052/shots/002-idle_a.jpg`): **the world renders
normally** — no black world / white sky. Mean absolute difference to the same step without row 165
(`ours-20261105-011043`): 0.096; to the original: 2.61 (0.12 outside the radar, which differs for
another reason, see B1). The earlier black world was the D3D state-block bug fixed in 831455f.
**Row 165 (and its companion row 184, the shadow release in the `CPhysical` destructor) can be applied**;
COV shows ~20–30 real-time-shadow functions (`CRealTimeShadowManager::Update`, `CShadowCamera::*`,
`CShadows::StoreRealTimeShadow`, `CastRealTimeShadowSectorList`) as only-ours in every window from
`connecting` on. Owner: `crates/samp-client/src/patches.rs` (remove the "NOT applied" note, add the
entries) and the ROADMAP decision row (delete it).

## Differences per feature

Severity: **visible** (a player sees or hears it), **network** (server or other clients see it),
**internal** (game state/calls only). "Evidence" names the tool, run and the line or step.

Combat rows (CSV 4, 5, 8, 22, 45, 46, 56, 57, 80, 98–106, 191, 200, 201, 210, 240–243, 247; RPCs
115/145/…; sync 203/204/206) are M7 and left out. "Planned" marks RPCs that ROADMAP assigns to M8b/M9 and
that have no handler yet; they are listed once (L-0) and only repeated where they cause another
difference.

### Protocol / network (C→S, F2)

Evidence: `target/audit/net/cov-c2s.md` (capture `target/audit/cov-net.pcapng`, COV run
`both-20261105-012535`, identical input on both clients; per COV window).

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| P1 | Ours sends RakNet `0x06` INTERNAL_PING every 5.0 s for the whole session (91 in the run); the original only twice, during the connection handshake | cov-c2s.md: `msg 0x06 rel6` 0 vs 60 in grove_idle, 0 vs 12 in idle_end | `connection.rs` `PING_INTERVAL = 5000 ms` applied after connect | stop the periodic ping once connected (confirm the original's rule with `--raw` over a longer idle) — `crates/samp-proto/src/connection.rs` (NET-6) | network |
| P2 | Ours sends `0x17` KEEP_ALIVE every 5 s without other reliable traffic (88); the original every **15.0 s** (25 of 32 gaps exactly 15.00 s, independent of other reliable sends) | cov-c2s.md `msg 0x17 rel8` grove_idle 20 vs 60, gap 15.001 vs 5.008 | `KEEP_ALIVE_AFTER = 5000 ms` and the "since last reliable" rule | fixed 15 s cadence (measure whether it is reset by anything) — `connection.rs` (NET-6) | network |
| P3 | Disconnect: ours sends `0x20` as RELIABLE_ORDERED on channel 0 before the game quits; the original sends it UNRELIABLE (rel6) after `quit` | cov-c2s.md idle_end/after-end: `msg 0x20 rel9 ch0` vs `msg 0x20 rel6` | our disconnect path uses the reliable ordered send | send unreliable like the original — `crates/samp-client/src/netclient.rs` / `connection.rs` | network |
| P4 | On-foot 207 after leaving a car: ours sends every **76 ms** (366 in 60 s), the original every 532 ms (112) | cov-c2s.md idle_end `sync 207` 112 vs 366; raw `c-58110.raw` 455.2 s ff.: the z float alternates `b0b25741`/`b1b25741` (1 ULP) between consecutive packets, defeating duplicate suppression | unknown — needs measurement: per-frame full-precision ped z of both clients after exiting (peek `player_ped.matrix+0x38` every frame); candidates: our surf probe or the exit warp leaving the ped in contact jitter | (owner: `vehicles.rs` exit path / 207 builder in `client.rs`) | network |
| P5 | 207 animation id is 0 in ours; the original sends the playing animation (walk 1189, jump 1133, crouch 1274, hands-up 1169) | DIFF-3 report `target/audit/cross-client.md` "server sees sender anim=…" rows | samp's 1812-entry animation table not implemented (parity-notes open item) | M8b animation table (`client.rs` 207 builder) | network |
| P6 | C→S RPC 118 after `SetPlayerInterior(0)` missing (walk window 2 vs 1) | cov-c2s.md walk `rpc 118 rel8` 2 vs 1 | consequence of I-1 (RPC 156) | — (I-1) | network |
| — | connection, join, class selection, spawn (`rpc 25/128/129/52`, `0x0b/0x1e/0x29/0x17`), driving (200 at 38 ms, 26/106/154 order and reliability), 205 cadence (1.008 vs 1.012 s): same | cov-c2s.md | — | — | — |

Not compared: RakNet ack/resend timing (NET-6, planned) and 203 aim sync (M7: the original sends 203 on foot
unarmed every ~10 s idle and ~1 s while walking; ours none).

### Boot and loading screen

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| B1 | HUD aspect-ratio correction missing: at 1280×720 the radar is an ellipse in ours, a circle in the original | screenshots `orig-20261105-010048/shots/002-idle_a.jpg` vs `ours-20261105-012052/…`, crop `target/audit/radar-cmp.png`; F5 rows 7 (`CHud::DrawRadar` call → samp), 16 (Idle hook), 368/369 (scale floats 0x859520, HUD floats 0x866B74 chosen by aspect) missing | samp updates the screen-scale constants when width/height ≥ 1.6 around the radar/crosshair draws; not implemented | implement rows 7/16/368/369 (+8 crosshair, M7) — `patches.rs` / `ui/` (HUD) | visible |
| B2 | CSV row 9: `CGame::Process` → `CCamera::Process` goes through samp (camera control around the game camera update) | F5 row 9 missing | not implemented | with the M8b camera work (`client.rs` camera RPCs) | internal (camera RPCs) |
| B3 | CSV rows 17/18/54 (+376 read): `CAnimManager::AddAnimation`/`BlendAnimation`/`UncompressAnimation` guards for invalid/unloaded animations from the network | F5 rows 17, 18, 54 missing | not implemented | M8b animations — `pedhooks.rs` | internal (crash hardening) |
| B4 | CSV row 52 `CGame::Shutdown`: ours `jmp rel32`, samp `jmp [slot]` | F5 applied/different (`e9…` vs `ff25…`) | different but equivalent hook form | none, or justify in `patch-justifications.tsv` | internal |
| — | loading screen, connecting screen, first UI frame: same (UX-1 boot diff 0 lines; COV `boot` 1 only-ours = stencil-shadow helper, see L3) | ux1-notes; `coverage-diff.md` boot | — | — | — |

### Join and class selection

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| J1 | CSV row 50: single-player cheat activation is not intercepted (samp hooks `CCheat::AddToCheatString` right before the cheat function runs) — a typed cheat string runs unchecked in ours | F5 row 50 missing | not implemented; what samp does in the hook is not specified yet | measure the hook on the original (EFX `gta:0x438576` with a typed cheat), then implement — `patches.rs` | visible |
| — | handshake, InitGame, class camera, class request: same (network table; DIFF-2 `flow_join` trace diff is a harness attribution artifact, see "Not findings") | | | | |

### Spawn

No new difference. DIFF-2 `flow_spawn` only shows the two decided `REMOVE_BLIP` calls; network sequence
`129 → 52` identical.

### Local player on foot

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| L-0 | Planned RPCs without a handler (M8b/M9): 13 PosFindZ, 16 PlaySound, 17 WorldBounds, 18/20 money, 33 shop name, 34 skill level, 35 drunk, 81/82 camera attach/interpolate, 86/87 ApplyAnimation/Clear, 89 fighting style, 90 velocity, 111 widescreen (samp also prints `Widescreen = %i`), 112 crime report, 133 wanted level, 146 gravity, 156 interior, 170 camera target, 41/42 audio stream, 103 ClientCheck, 79 explosion, 48 virtual world | DIFF-2 NOT-IMPLEMENTED list; EFX `diff-posz-1` (orig `0x02CE` + `CPed::Teleport`), `diff-money-*` (orig `0x0109 ADD_SCORE`, `Players+0xB8`), `diff-grav-*` (orig writes `GAME_GRAVITY` 0x863984), `diff-int-*` (orig `0x04BB/0x0860/0x04E4`, `currArea`, streaming) — ours no effect | not implemented | M8b / M9 per ROADMAP | visible |
| L1 | Real-time ped shadows run in ours (rows 165/184 not applied) | COV only-ours `CRealTimeShadowManager::Update`, `CRealTimeShadow::*`, `CShadowCamera::*`, `CShadows::StoreRealTimeShadow` in every window from `connecting`; F5 rows 165, 184 missing | row 165 was held back for the black world, which is gone (re-test above) | apply rows 165 + 184 — `patches.rs`; drop the ROADMAP decision row | internal (GTA computes the shadows every frame; with/without row 165 the measured spawn scene differs by 0.096 mean abs, so no shadow was visibly drawn there) |
| L2 | Stencil-shadow overlay alpha 50 in ours, 80 (0x50) in the original | F5 row 166 missing; COV boot only-ours `FUN_00711160` (in `RegisterStencilShadows`) | not applied | apply row 166 — `patches.rs` | visible |
| L3 | Mobile-phone model 330 still added/removed by GTA's phone code (samp passes −1) | F5 rows 223/224 missing | not applied | apply rows 223/224 — `patches.rs` | visible (phone in hand) |
| L4 | Event facial expressions run in ours (`CEventHandler::SetEventResponseTask` → `HasEditableResponse` forced false in samp) | F5 row 170 missing | not applied | apply row 170 — `patches.rs` | visible (faces) |
| L5 | `CPlayerInfo::Process`: samp NOPs a hold/fight test (row 215) and a 175-byte block that posts a script-command task to the player (row 217); `CPed::Render` weapon while holding an object (row 216) | F5 rows 215, 216, 217 missing | not applied | apply — `patches.rs` (217's purpose is still unidentified in the CSV) | visible (single-player interaction) |
| L6 | GTA text lines longer than the stock 0x12C-byte stack buffer: samp enlarges `CFont::ProcessCurrentString`'s frame to 0x32C (15 byte patches) | F5 rows 225–239 missing (383 is the dead duplicate) | not applied | apply rows 225–239 — `patches.rs` (UI) | visible (long game text truncated / unsafe) |
| L7 | Stunt-bonus flag (SCM global 709, ScriptSpace+0xB14) not written from InitGame | F5 row 359 missing (orig wrote it, ours not) | InitGame handler does not set it | write the flag from InitGame like samp — `client.rs` | visible (stunt bonus) |
| L8 | Weather audio: `CAEWeatherAudioEntity::UpdateParameters` vtable slot → samp | F5 row 95 missing | not applied | with M9 audio | audible |
| L9 | Birds' line-of-sight test also checks objects in samp | F5 row 245 missing | not applied | apply row 245 — `patches.rs` | visible (minor) |
| L10 | `CGameLogic::SortOutStreamingAndMemory` still calls `DeleteRwObjectsAfterDeath` | F5 row 179 missing | not applied | apply row 179 — `patches.rs` | internal |
| L11 | Pause map: samp NOPs one `CSprite2d::DrawRect` in `CMenuManager::PrintMap` | F5 row 203 missing | not applied | apply row 203 — `patches.rs` | visible (ESC map) |
| L12 | Camera "point at char" for player peds (row 198, spectating), row 9 camera wrapper | F5 rows 198, 9 missing | M8b spectating | M8b | visible (spectate) |
| L13 | Row 269 (samp's sector relocation corrupts an instruction in `CStreaming::DeleteAllRwObjects`) | F5 row 269 missing | open since F5 baseline | decide: reproduce or justify (dead path?) | internal |
| L14 | 152 SetWeather: the original's screen changes much more after the RPC (203 / 200 changed cells vs 39 / 40, stable in both original runs) | DIFF-2 steps 37/38 `weather_8`/`weather_10` (trace and state reactions match) | unknown — needs measurement: EFX window of 152 (weather globals 0xC81318/1C/20 and `CWeather::ForceWeatherNow`-style writes over 20 frames) | `world.rs` weather | visible |
| L15 | Death via `SetPlayerHealth(0)`: ours shows GTA's "Wasted" big message; samp NOPs it (row 191) and toggles player control/freeze (`0x01B4`, `0x04D7`) around the death | DIFF-2 step 172 `kill_to_class_selection` (+`CMessages::AddBigMessage Wasted` in ours); F5 row 191 missing | death flow belongs to M7 | M7 (row 191 → `patches.rs`) | visible |
| L16 | GameModeRestart (40) + InitGame (139): the original also resets the interior (`0x04BB(0)`, `0x0860`, `0x04E4`) | DIFF-2 step 175 `gamemode_restart` | restart path calls samp's interior setter; ours has none (I-1) | with I-1 (M8b) / restart handling (M10) | visible (interior after restart) |
| — | 19 SetFacingAngle, 15 TogglePlayerControllable: same script commands (`0x0173`; `0x01B4` + `0x04D7`) | EFX `diff-angle-1/2`, `diff-ctrl-1/2` (remaining lines are script-param scratch and the ped's motion state) | — | — | — |

### Interiors

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| I-1 | Area not switched by `SetPlayerInterior`; GTA enex entry through the warp branch, no door walk, persistent `bUnknownPairing` | see "Interior entry" above | RPC 156 not implemented | M8b | visible |
| I-2 | `InteriorGroup_c::Exit` → `Interior_c::Exit` not NOPed | F5 row 212 missing | not applied | apply row 212 — `patches.rs` | internal (interior furniture groups) |

### Remote players

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| R1 | Surf glue: `CWorld::Process` → `ProcessAttachedEntities` through samp (players standing on vehicles/objects) | F5 row 24 missing | not implemented (m6-notes: player-parity item) | player parity / M5 — `pedhooks.rs` | visible (surfing) |
| R2 | 207 animation id (P5) | | | | network |
| R3 | No radar marker for other players: on stream-in the original adds a coordinate blip (`0x04CE` sprite 0, colour 49, scale 2/3) and for streamed peds `0x0187 ADD_BLIP_FOR_CHAR` (colour 49, display 2); ours adds none | DIFF-2 step 153 `npc_spawn` (missing `0x04CE/0x0165/0x0168`, `snap.blips` +col49 vs none), 163 `npc_stream_out`, 161 `spectate_npc` | player markers (ShowPlayerMarkers, radar colours CSV 51) not implemented | `players.rs`/`remote.rs` (+ radar colours) | visible |
| R4 | 166 remote death (`npc_kill`): no reaction in ours (orig teleports and re-tasks the dead ped) | DIFF-2 step 164 | not implemented (M7 death handling) | M7 | visible |
| — | DIFF-3 24/24 MATCH both directions (walk, turn, jump, crouch, hands-up, skin) | `target/audit/cross-client.md` | — | — | — |

DIFF-3 still has 18 PENDING step-directions for features that are in main now (vehicles M5, objects M6,
chat M4): the harness steps were never added (`tools/diff/xclient.py` `PENDING`). Cross-client parity of
those features is unmeasured — owner `tools/diff/xclient.py` / `xclient.pwn`.

### Files (samp.saa, F6)

No difference: same file opens at the same times in both DIFF-2 traces (incl. `SAMP\CUSTOM.IMG`); COV boot
identical. Byte comparison not repeated (SAA-2: 188 opens identical).

### SP-OFF

No population, traffic, police or car-generator function is only-ours in any COV window. Remaining
single-player behaviour rows are listed under L3–L11 and J1.

### Chat, dialogs, textdraws, menus, nametags, scoreboard

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| U1 | Long `GameText`/help strings (L6) | | | | visible |
| U2 | 121 GangZoneFlash: no visible flash in ours | DIFF-2 `gangzone_flash` NOT-IMPLEMENTED ("no reaction in ours"), screenshot change 34 vs 3 cells | unknown — needs measurement: D3D/radar draw of a flashing zone (visual.sh element); the radar shape (B1) also changes these cells | `entities.rs` gang zones | visible |
| — | 61 dialogs (all 6 styles + hide), 73 game text, 76/77 menus, 83 select textdraw, 101 player message, 117 object edit: MATCH | DIFF-2 per-RPC table | — | — | — |

DIFF-4 (D3D call stream) was not re-run in this audit (lane ui busy); its last result was clean at
820e431. The DIFF-2 screenshot verdicts of UI steps after step 16 are not usable as UI evidence: the
player stands elsewhere and the camera differs because of earlier unimplemented RPCs (13, 18, 35, 111, …),
see "Harness limitations".

### Vehicles

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| V1 | Speed blur and dirt textures run in ours | COV only-ours `CPostEffects::SpeedFX`, `CLinkedUpgradeList::SetDirtTextures` (vehicle, idle_end); F5 rows 177, 62 missing | not applied (M5-P item) | apply rows 177 and 62 — `vehpatches.rs` | visible |
| V2 | Car door swinging (`CDoor::Process` from `CAutomobile::ProcessSwingingDoor`) only in the original after the exit | COV only-orig `CDoor::Process` (vehicle f549, idle_end f0) | unknown — needs measurement: door state (`CAutomobile` door angle) after the exit in both (trace `vpc` door fields); candidate: our exit leaves the door closed | `vehicles.rs` exit | visible |
| V3 | `CPed::RemoveWeaponWhenEnteringVehicle` called by samp on enter | COV only-orig (vehicle f385, samp.dll+0xac13d) | not implemented (M5-P item) | `vehicles.rs` | visible (weapon in car) |
| V4 | Number plates (rows 59/60), `CVehicle::Render` dirt (62), `CFire::ProcessFire` BMX branch (199) | F5 rows 59, 60, 62, 199 missing | 59/60 decided (PLATE-1); 62/199 not applied | rows 62/199 — `vehpatches.rs` | visible |
| V5 | Idle 207 after exit (P4) | | | | network |
| V6 | 96 AddVehicleComponent: the original loads the part with `0x0247 REQUEST_MODEL(1010)` + `0x038B LOAD_ALL_MODELS_NOW` and adds it with `0x06E7 ADD_VEHICLE_MOD` in the RPC frame; ours goes through `0x06E9 REQUEST_VEHICLE_MOD` and does not add the mod in the step window | DIFF-2 step 135 `vehicle_component_add` (orig runs 013339/014057 vs ours 011043) | different load path | follow the original's order — `vehicles.rs` | visible (tuning part appears later / not at all) |
| V7 | 154 remote exit: the original lets the remote ped leave with a task (`SetTask` index 4) and re-adds the car blip; ours warps it out with `0x0362 WARP_CHAR_FROM_CAR_TO_COORD` (`CPed::Teleport`) | DIFF-2 step 159 `npc_exit_vehicle` | different exit path for remote drivers | measured exit path (TRACE-4/5) — `vehicles.rs` remote occupants | visible (no exit animation) |
| V8 | `ADD_BLIP_FOR_CAR_OLD` output in script local `l2` in ours, `l0` in the original (164/24/71) | DIFF-2 steps 128–130, 145 | argument encoding only | none (same result; like the decided CREATE_PLAYER immediate) — or log a decision | internal |
| — | 159 SetVehiclePos, 160 ZAngle, 147 health, 57 component remove: MATCH; drive/enter/exit network identical (P table) | DIFF-2, cov-c2s.md | — | — | — |


### World entities

| # | difference | evidence | root cause | proposed fix (owner) | severity |
|---|---|---|---|---|---|
| W1 | Object camera collision (row 81) not implemented | F5 row 81 missing; m6-notes | needs samp's geometry routine (A 0x10012DE0) | M6-P — `worldhooks.rs` | visible (camera through objects) |
| W2 | Dormant custom-model rows 97, 348, 362, 364 and row 374 (camera attach) not applied | F5 missing (m6-notes give the reasons) | decided in m6-notes, not in ROADMAP | move the m6-notes reasons into `patch-justifications.tsv` / ROADMAP decisions (374 → M8b) | internal |
| W3 | Object editor gizmo for attached objects (116 / 28): the original draws the editor, ours does not | DIFF-2 steps 58/59 `attached_edit(_cancel)`: screenshot change 86/100 cells vs 5/9 | M6-P open item (editor gizmo + drag, ROADMAP queue) | M6-P — `objects.rs` | visible |
| — | 44/46/47/99/122 objects, 84 materials, 75 attach, 95/63 pickups, 171–176 actors, 117 edit: MATCH | DIFF-2 per-RPC table | — | — | — |

Radar-drawn entities (checkpoints 107/37/38/39, map icons 56/144, gang zones 108/120/85/121) show
screenshot-only DIFFs in DIFF-2 (e.g. `checkpoint_set` 36 vs 21 changed cells, `racecp_disable` "ours
changes 0 of the 4 cells every original run changes"); their trace reactions match. The radar shape (B1)
changes exactly those cells, so they are not separate findings until B1 is fixed and DIFF-2 re-run.


## EFX (effect diff per RPC)

`tools/efx/run.sh both` per group (original: single-step capture of samp's handler + a window; ours: window
with the original's spec). Group 1 `orig-20261105-013359` / `ours-20261105-013705`, group 2
`orig-20261105-014257` / `ours-20261105-020145`, group 3 `orig-20261105-020920` / `ours-20261105-021853`, group 4 (camera, re-run) `orig-20261105-022729` / `ours-20261105-022817`; diffs `ours-*/diff-<step>-<n>.txt`.

| RPC | verdict | detail |
|---|---|---|
| 19 SetPlayerFacingAngle | same effect | both `0x0173 SET_CHAR_HEADING`; remaining lines are script-parameter scratch (ScriptParams, `CommandsExecuted`) |
| 15 TogglePlayerControllable | same effect | both `0x01B4 SET_PLAYER_CONTROL` + `0x04D7 FREEZE_CHAR_POSITION`; ped motion fields differ because the original was falling (see limitation 4) |
| 13 SetPlayerPosFindZ | not implemented | original `0x02CE GET_GROUND_Z_FOR_3D_COORD` + `CPed::Teleport`; ours nothing |
| 18 GivePlayerMoney | not implemented | original `0x0109 ADD_SCORE` → `CWorld::Players[0]+0xB8`; ours nothing |
| 146 SetPlayerGravity | not implemented | original writes `GAME_GRAVITY` (0x863984) 0.008 → 0.002 directly; ours nothing |
| 156 SetPlayerInterior | not implemented | original `0x04BB`/`0x0860`/`0x04E4` (407 k steps: streaming requests, `ms_EntityAreaCode`, `currArea`); ours nothing (I-1) |
| 164 StreamInVehicle | same effect | same script sequence (`0x0248`, `0x0247`, `0x038B`, `0x0248`, `0x00A5 CREATE_CAR`, `0x0175`, `0x09C4`, `0x07FF`, `0x053F`); the blip (`0x0161`/`0x0165`) and the per-frame `0x053F` polls come later in the original (outside its handler capture) — DIFF-2 shows both |
| 24 SetVehicleParamsEx | same calls, order differs | original `CAutomobile::OpenDoor(node 17, door 1)` then `(16, 0)` and lock byte `+0x4F8` := 2; ours the two `OpenDoor` calls in the opposite order (lock written by `vehicles.rs:443`; not watched in the window because the spec names the car `veh#?`) — internal |
| 147 / 159 / 160 / 165 | not measured | open.mp sends 147/159/160 to the driver only; the on-foot original never received them (limitation 4) |
| 157 / 158 / 162 camera | same end state | group 4 (player re-placed first): all 30–42 camera fields the original writes (`TheCamera+0x86C…` fixed position 1500/−1650/30, look-at point, `+0x120` heading) end with the same values in ours; the original runs `0x0925`+`0x015F`, `0x0925`+`0x0160`, `0x0930`/`0x0925`/`0x0373`/`0x02EB`, which our code issues too (`client.rs` camera arms), but the window's call hooks did not record them in ours (EFX window limitation, not a client difference). Ours also writes `TheCamera+0xE4/+0xE8` in the window (not watched in the original's step capture) |
| 44 CreateObject | same effect | both `0x0107 CREATE_OBJECT` + `0x0453 SET_OBJECT_ROTATION`; only pool-lookup arguments differ |
| 47 DestroyObject | same effect | both `0x0108 DELETE_OBJECT`; ours polls `0x0248 HAS_MODEL_LOADED` 10× in the window (original once) — internal |
| 99 MoveObject | differs (internal) | the original's handler reads the object's current rotation (`CMatrix::ConvertToEulerAngles`, RW matrix rotate helpers) at RPC time; ours does nothing GTA-side in the RPC frame and works per frame (`objects.rs` motion). Matters for MoveObject with a rotation target: measure the rotation interpolation start (owner `objects.rs`) |
| 122 StopObject | same (per m6-notes) | original zeroes move/turn speed `+0x44…+0x58`; ours' writes are not watched (spec names the object `obj#?`) |
| 73 GameText | same effect | both `0x00BE CLEAR_PRINTS` + `CMessages::AddBigMessage(text, 3000, style 3)` + `InsertPlayerControlKeysInString`; ours' extra `0x00C0` in the window is its per-frame clock |
| 61 / 134 / 135 (dialog, player textdraw show/hide) | no GTA-side effect in either | UI-only; covered by DIFF-4 (not re-run) |
| 95 / 63 pickups | not measured | the group-3 player was not near the pickup position, so the server never streamed it (limitation 4) |

## Harness limitations found (owner: tooling)

1. **DIFF-2 state cascade.** A state-changing RPC that ours does not implement changes the starting point of
   every later step: after step 16 (`pos_findz`, RPC 13) the original stands at z 20.5 and ours at 15.5,
   after 18/111/camera steps money, HUD and camera differ. Trace reactions stay comparable, but the
   screenshot verdicts of most later UI/world steps (e.g. `label_create`, `skin_101`, `specialaction_phone`,
   gang zones, checkpoints) are not usable as evidence. Fix: `tools/diff/rpcfire.pwn` re-places the player,
   resets camera/money/widescreen before each step group (or `drive.py` runs `pos_base` + `camera_behind`
   between groups).
2. **DIFF-2 handles.** Vehicle steps are "unstable" in the original only because two cars created in one
   step get their pool handles in either order (`CREATE_CAR [403] → 257/513`); `rpc_report.py` should map
   handles by model/position before comparing.
3. **DIFF-3 coverage.** The vehicle, object and chat steps (`PENDING` in `tools/diff/xclient.py`) were never
   added although M4/M5/M6 are in main: F3 is unmeasured for them.
4. **EFX scenario.** 147/159/160 are only sent to the driver, so the original's armed captures never fired
   with the player on foot (`vhp-1`, `vpos-1`, `vang-1` "did not finish"); the steps need `putlast` first.
   In both groups the original's ped did not stay put: in group 1 it fell under the map after `posz`
   (`GET_GROUND_Z` on unstreamed ground → z −43) and later died; in group 2 it rose steadily after the
   `vehin` capture (server-side z 54 → 168 → 3760 m, then class selection) — probably the long single-step
   freezes (several seconds for a vehicle stream-in) feeding a large time step into the ped physics; ours
   (window mode, no freeze) stayed in place. Captures after that point in the original are invalid (its
   157/158 captures caught the class-selection camera). Fix: re-place the player before every step (done
   for the camera steps, re-run as group 4) and keep step captures short (`over=` for the model loads).
5. **COV windows.** The driver switches windows by wall clock/server log, so the `connecting` →
   `class_selection` boundary falls a few frames differently per client (see "Not findings").
6. **Original stability under the tracer.** 1 of 3 full DIFF-2 original runs crashed in samp
   (`vehicle_put_player`, SCM op 0x470); 1 of 3 did not join. Neither was seen with our client.

## Not findings (checked and explained)

* DIFF-2 `flow_join` "missing" `RANDOM.ped`, `SAMP\CUSTOM.IMG`, `DATA\water.dat` opens: present in both
  traces at frame 0 (orig t=1342/7811/8779 ms, ours 1181/7614/8582 ms); the report's client attribution
  drops them on one side. F6 (files): no difference found.
* COV `connecting`/`class_selection` (57 only-orig / 78 only-ours apart from shadows): the join-time ped
  setup (`CPed::SetModelIndex`, `CAEPedSpeechAudioEntity::Initialise`, class camera 0x015F/0x0160) runs at
  absolute frame 252 (orig) / 251 (ours); the windows start at 255 / 251, so it lands on different sides
  of the boundary. Window-boundary artifact.
* COV `walk` only-orig car streaming (`StreamOneNewCar`, `LoadSpecificDriverModelsForCar`) and birds: both
  clients run these functions in other windows (not in the "never" lists); timing/random.
* DIFF-3: 24/24 enabled step-directions MATCH (walk/turn/jump/crouch/hands-up/skin both ways).
* SP-OFF: no ped, traffic, police or car-generator function is only-ours in any window.
* COV-1 open items: `CCamera::ProcessFade`/`ProcessMusicFade` in class selection (item 12) no longer
  differ; `CPed::Say` on interior entry (item 11) is explained by I-1.
