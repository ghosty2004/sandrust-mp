# CLOSE — F1 (DIFF-2) and F5 (patch coverage) closing

Branch `parity-close` (worktree `samp-rs-wt/close`). Static reads are samp.dll A (2021 build) with objdump;
B addresses are A − 0x30 in the hook region. Runs: F5 `tools/diff/patch-coverage.sh --skip-build`, DIFF-2
`tools/diff/rpc-coverage.sh --skip-build` (lanes diff/diff2, `target/locks/diff.lock`), our build from
`DIFF_CARGO_TARGET=samp-rs-wt/close/target`.

## F5: rows applied

| row | what | samp | ours |
|---|---|---|---|
| 179, 203, 212 | NOPs: `DeleteRwObjectsAfterDeath` call, one `PrintMap` DrawRect, `Interior_c::Exit` call | init-main A 0x100AB291 / 0x100AB5FF / 0x100AB74C | `patches.rs` table |
| 199 | `CFire::ProcessFire`: `jmp +0x77` over the BMX branch | A 0x100AB53D | `patches.rs` |
| 245 | birds' line of sight also tests objects | A 0x100ABA04 | `patches.rs` |
| 52 | `CGame::Shutdown` entry: now samp's form `jmp [0x53C8F1]` + NOP with the handler in the padding slot (was `jmp rel32`) | A 0x100A6904 | `dll.rs` via `worldhooks::jmp_indirect` |
| 106 | `CVehicle::InflictDamage` jmp: samp installs it from InitGame **only when vehicle friendly fire is on** (A 0x10010A51: settings +0x1B, InitGame's last field), independent of the lag-compensation mode; ours had tied it to lag comp 1/2 | A 0x100A7200 | `combathooks::install_vehicle_friendly_fire` |
| 359 | stunt-bonus dword from InitGame (A 0x100109E8 → 0x100A1580) | | `behaviourclient::on_init_game` |
| 358 | gravity: already written by InitGame/RPC 146; annotated | | |
| 54 | `CAnimManager::UncompressAnimation`: returns at once for a NULL hierarchy, else the two displaced instructions and 0x4D41C5 | A 0x100A3E00 | `behaviourclient.rs` stub |
| 95 | `CAEWeatherAudioEntity::UpdateParameters` vtable slot: `CGame::currArea` (0xB72914) forced to 1 around the call while samp's audio-object flag (+4, set by PlayerPlaySound id 1, cleared by id 0) is set — weather sounds muted as in an interior | A 0x100A5E70 | `audio.rs` |
| 24 | `CWorld::Process` → `ProcessAttachedEntities` through samp: surfing (below) | A 0x100A4990 | `worldhooks.rs` + `surf.rs` |

## Surfing (row 24)

Static (A): probe A 0x10003730 (local on-foot tick), local glue A 0x10003570, remote glue A 0x10014650, 207
fields A 0x100050E8, resets: spawn A 0x10003D5E, RPC 12/13 A 0x100193E4/0x100194FB, RPC 165 A 0x10011041,
RPC 47 A 0x1001BCB7.
* Probe: the vertical line-of-sight hit is a pool vehicle (id 1..1999) that is player-driven (A 0x100B7120: a
  driver in the vehicle with ped type 0) or a 569/570 carriage, and nearer than 30.0 (A 0x100E5808) to the local
  player → contact "vehicle"; otherwise a pool object that "moves" (A 0x100A8500: attached to a moving object, or
  attached to a vehicle, or MoveObject running) → contact "object". Tasks that cancel (surface cleared, latch kept):
  physical-response task 208 (on foot), primary task 704, enter-car tasks 700/701/712/713.
* First contact: the glue runs once with glue off (offset := inverse(surface matrix) × ped position, through
  `RwMatrixInvert` 0x7F2070 on a copy with flags 0), then latched, glue on. Without contact the latch holds
  2000 ms (re-resolving the surface by id), then everything is cleared (id 0xFFFF).
* Glue per frame (after `ProcessAttachedEntities`): with glue on, both left-stick axes 0 and the primary task
  not 211 → ped position := surface × offset (Remove, matrix, UpdateRW, Add), ped move speed x/y := surface's,
  turn speed z := surface's; otherwise the offset is re-measured.
* 207 while latched: offset and id (2000 + object id for objects).
* Remote: state 17 (on foot), surf id 1..2999 (objects ≤ 1000), every offset component in [−100, 100]
  (A 0x100E6110/14) → ped position := surface × offset.

Parity: DIFF-2 steps `surf_platform` (a 10 × 10 m floor 19129 30 m up, the player put on it), `surf_move` (8 m
east at 1 m/s), `surf_destroy`.

## F5: rows justified (tools/diff/patch-justifications.tsv, ROADMAP decisions)

97, 348, 362, 364 (dormant custom-model system: the manager A 0x1026EA10 has one writer, A 0x10011F4B, inside
the unreferenced handler A 0x10011EB0; every caller tests it for NULL), 374 (handler A 0x100A4FA0 is not
0x100A4F10 as the CSV says; it has no reference, its filter global A 0x101515E8 is never written), 380–382 and
384 (dead writers), 383 (dead duplicate of rows 225–239), 269 (samp's write into an opcode of a RET'd function).

## DIFF-2 (F1)

Code fixes:
* RPC 79: samp pushes the radius as a double but the descriptor `fffii` reads one dword: the low half of the
  double (0 for 2.0). Ours passed the float bits.
* RPC 40 reset routine (A 0x1000A540), the game-side part in samp's order: interior 0, money to 0
  (`ADD_SCORE(0, −money)`, A 0x100A2100), kill, armour 0 (A 0x100ABDF0), `DISPLAY_ZONE_NAMES(0)` (A 0x100A1560),
  player colour table to the defaults (A 0x100A72C0), gravity, HUD wanted 0 (A 0x100A1460), HUD clock off
  (A 0x100A14A0); earlier among the reset helpers the 11 weapon-skill stats := 999.0 written directly
  (A 0x1009CCF0).
* `MARK_MODEL_AS_NO_LONGER_NEEDED(596)` only on InitGame, not when a death leads to class selection.
* CREATE_PLAYER / GET_PLAYER_CHAR take the slot through script locals like samp.
* The first-frame ped flags (0x087E / 0x0446) are set when samp first creates its local-ped object, i.e. in the
  local `CPlayerPed::ProcessControl` hook of frame 1, before that frame's object streaming.

Harness (`tools/diff/rpc_report.py`, `rpc-coverage.sh`):
* `rpc-decisions.tsv`: residual differences and untriggerable ids are accepted only as rows whose decision id is
  in ROADMAP (DIFF2-D1 garbage REMOVE_BLIP handles at spawn, DIFF2-U1 the 10 ids open.mp never sends);
  `UNTRIGGERABLE-UNDECIDED` fails the run.
* Calibration: 4 original runs by default; numeric fields that differ between original runs in lines of the same
  template are not compared (`unstable_fields`), see DIFF2-CAL.
* `CFileMgr::OpenFile` compared regardless of attribution; Create/DeleteRwObject through the object vtable hooks
  and the surf-probe `los` records compared as sets only.

Scenario determinism (`rpcfire.pwn`, `rpc-coverage.sh`): surfing steps added (the floor leaves only after
`pos_base_2`; a 30 m fall killed the player), `object_move` reaches its target within the step (the end position
had depended on when the driver fired `object_stop`: 2.01–2.09 s in the original runs, 2.20 s in one of ours),
208 markers once per session (the NPC marker / stream-in race).
Harness robustness: `rpc-coverage.sh` reuses up to 6 earlier complete original runs of the same scenario
(`--reuse-orig`); the screenshot check uses all original runs; `lib.sh` starts gamescope without the host's
`WAYLAND_DISPLAY` (same change as on main).

## Results

* F5 `tools/diff/patch-coverage.sh --skip-build`: main before (run `xpatch-20261105-115151`) 337 matching,
  22 static, **2 different, 20 missing**, 3 justified → this branch (run `xpatch-20261105-143521`) 346 matching,
  24 static, 0 different, 0 missing, 14 justified, **exit 0**.
* DIFF-2: main before (`ours-20261105-120658` vs 2 original runs) MATCH 112 / DIFF 19 / 10 untriggerable
  without a decision → this branch: full run `rpc-coverage.sh --skip-build` (6 fresh original runs from
  orig-20261105-224532 + 6 reused from orig-20261105-214920, ours-20261105-233128) MATCH 130 / DIFF 1
  (`explosion` screenshot, the ped's idle pose at a harness-shifted shot time; trace/state match) → decision
  DIFF2-D3 → `rpc-coverage.sh --report` over the same runs: MATCH 131, 10 untriggerable as DIFF2-U1, exit 0.
  Earlier full runs on this branch: 1 residual each (camera z on a still-moving object → scenario fixed; truck /
  trailer order → order calibration; the same explosion screenshot).
* Surfing: original and ours carry the player exactly 8.000 m with the floor (`surf_move`, 18 original runs,
  3 of ours).
* e2e (lane ui): m1, m2, m3, goal, m4, m5, m6, m7, m8a, m8b, m9, m10, saa, ux1, feata exit 0; tests 590+;
  clippy 0 (host + i686).
