# Phase 3 / M8b — remaining behaviour (implementation notes)

Branch `remaining-behaviour` (worktree `samp-rs-wt/m8b`), lane veh. Spec: docs/phase3/trace-notes.md "Remaining
behaviour (TRACE-8)" (measured), player-state.md, remote-players-sync.md §1/§2 "212", remote-control-onfoot.md §4.1,
session-misc.md "103". Static details were read from samp.dll build B with objdump (no REA) and are cited by
address in the code.

## Files

| file | content |
|---|---|
| `crates/samp-client/src/animtable.rs` | samp's 1812-record `"LIB:NAME"` table (interop constant, source B 0x101039E8, identical in build A; generated from the binary) |
| `crates/samp-client/src/anims.rs` | pure rules: table lookups (string search B 0x1009CE20, hash search B 0x1009D000), 207 animation encoding (script task B 0x100AE600 / plain blend B 0x100AE6B0), remote decode (B 0x100AE7E0), the local latch (B 0x100077EF / 0x10005131), remote state machine (B 0x100167E4), `_ftol2` |
| `crates/samp-client/src/behaviour.rs` | pure rules: world-bounds push-back (B 0x1009F450), drunk value / wobble (B 0x100ADE00), MSVC `rand`, camera cut / position validator, spectate state, ClientCheck limits / 0x48 argument |
| `crates/samp-client/src/behaviourclient.rs` | in-game side: camera RPCs + attachment follow, spectator tick + 212, bounds (on foot / driver), drunk, RPC 86/87, local anim encoder, remote anim apply, ClientCheck replies, 146/133/104/18/20/111/89/33/90/156; CSV rows 17/18 (AddAnimation / BlendAnimation recorders) and 198 (POINT_CAMERA_AT_CHAR ped branch) |
| `client.rs`, `dll.rs` | wiring; CSV row 9 (`CCamera::Process` call in `CGame::Process` → camera follow first); `behaviour` control command |
| `players.rs` | one line: the synced animation in the one-shot apply (the only touch in the combat owner's area) |
| `vehicles.rs` | add-component sequence of B 0x100B7FD0 (SCM event 2 / RPC 65); `id_of` accessor |
| `objects.rs` | `gta_ptr` accessor (RPC 81) |
| `script.rs` | `Arg<'a>` (borrowed strings), `Arg::Bytes`, 11 opcodes |
| `tools/e2e/m8b.sh`, `m8bscen.sh`, `m8bparity.sh` | acceptance test, shared scenario, parity runner (orig: trace lane; ours: lane veh) |
| `tools/efx/scenario.sh`, `efxfs.pwn` | EFX steps campos/camlook/behind/wide/gravity/specon/specpl |

## Behaviour (static reading, build B)

* Camera: every camera method of samp clears the object attachment (157/158/162/spawn jump cut). 82 with time ≤ 0 is
  ignored; position: `0925, 0930(1), 0936(from, to, ms, cut)`, look-at: `092F(1), 0920(…)`; both (and 81) mark the
  spectator view as applied (LocalPlayer +0x314). 81: ids ≤ 1000 of existing objects, snap `0936(pos,pos,100,1)`
  when the position passes the validator; the follow runs every frame before `CCamera::Process` (row 9).
* Spectating: 124 off while spectating runs the spawn routine at once; 126 accepted for state ∉ {0, 32}; the
  spectator tick (also while not spawned, leaving class selection) sends 212 (> on-foot interval, camera matrix
  position), HUD off, health 100, Teleport to camera + 20; a player target is dropped when not streamed in / state
  0 / wasted; views: none → (warp out of a car) 50,50,50 → 60,60,50 cut 2; player → `0159(ped, mode, 2)`; vehicle →
  `0158(car, mode, 2)`. CSV row 198 makes `0159` use the ped camera for remote CPlayerPeds (without it the camera
  followed the local ped, which the tick parks 20 m above it: runaway climb, found in the e2e run).
* World bounds: on foot every frame before the 207, driving before the driver packets, only when GET_AREA_VISIBLE
  = 0; strict compares x > maxX, x < minX, y > maxY, y < minY; non-zero axis speed → ∓0.2 and speed z 0.1; matrix
  z + 0.04 (raw write); on foot `01B9(char, 0)`; `00BE` + AddBigMessage(text, 1000, 5). InitGame resets the bounds
  to ±20000 and applies the gravity.
* Drunk: level > 2000 → `052C(0, ftol(level·0.02))` (24-bit x87 precision under D3D: 5000 → 100, 2100 → 42 as
  measured), clamp 5..250; driver (not passenger / train carriage) wobble ±0.012 / ±0.015 at most every > 200 ms
  (GetTickCount), only when |vx| or |vy| > 0.05; level −1 per frame; ≤ 2000 → −1 while > 0 and `052C(0,0)`.
  Not spawned: a positive level becomes 0 once. 205 carries the level.
* Animations: play = "SEX" refused, `04EE`, `04ED`, `04EE`, then 14 × (Sleep(1), `04EE`) and a last Sleep(1) →
  give up; `0812(ped, name, lib, delta, loop, lockX, lockY, freeze, time)`. 207 field: script task (types 401/934
  on the primary chain) → string index + task flags (0x2 loop, 0x40 lock x, 0x80 lock y, 0x8 clear = freeze, time
  code); otherwise the last AddAnimation/BlendAnimation on the local clump (group 0 / anim 0xA0 ignored) → hash
  index, bit 31, blend delta. Remote: changed value → play; 0 after VEND_USE → VEND_EAT_P; 0 without RPC 86 mark
  and a running named anim → `CARRY:crry_prtial` (stop).
* Shop name: an empty name writes only the low byte of the ped pointer (+0x78C) to 0 (reproduced); LoadShop for
  names of ≤ 8 characters including the empty one.
* ClientCheck: type 2 = (driver: vehicle +0x40, 2; else ped +0x40, 1); 0x48 = QPC ms & 0x0FFFFFFF | 0x30000000,
  result 0; limits offset ≤ 256, 2 ≤ count ≤ 256.

## Deviations / TODO-unknown

* **ClientCheck 5 / 0x45 / 0x46 / 0x47**: the 8-bit checksum (B 0x1000EA50) is not recorded (anti-cheat, by
  project rule): no reply is sent. Measurement request: whether answering is required for compatibility and, if so,
  a clean-room description of the checksum. Type 0x45 also checksums samp.dll's own image, which our client does
  not have — answering it would mean impersonating samp.dll; logged as a decision.
* Drunk wobble: samp's `rand()` sequence is shared with all of samp's other rand users; ours is an MSVC LCG seeded 1
  used only here, so the sign pattern differs (decision).
* Parachute helper after a remote `PARACHUTE` animation (B 0x100AD630) is not implemented (needs analysis).
* 203 while spectating (2 × interval) is the combat owner's aim code; not sent here.
* RPC 48 / 112: open.mp never sends them (TRACE-8): not implemented.
* SCM event 4 (mod-shop follow for passengers): unchanged (TODO-unknown in vehicles.rs).
* World bounds, walking into maxX: both clients issue the same per-frame sequence from the same frame offset
  (`01B9(char,0)`, `00BE`, AddBigMessage(text, 1000, 5)) and the same speed/matrix writes, but one frame after the
  first push the original's ped got `CTaskComplexJump` (and the pad jump flag), left the ground and was carried
  back over the boundary, while ours kept walking against it (GTA overwrote the written speed each frame; 86 vs
  192 messages in the window). Cause not found statically (samp's bounds helper writes nothing else):
  **measurement request** — EFX step capture of B 0x10003F40 plus a window of the next two frames while walking
  into a bound, to see what sets the jump.
* Out of scope, seen in the diffs: player markers for streamed-out players (`04CE`/`0187` blips, 208) and the
  nametag LOS (`06BD`) for the spectated bot are not issued by ours (remote players / UI owners); CREATE_PLAYER
  argument encoding is a logged decision.

## Verification (2026-11-05, lane veh unless noted)

* `tools/e2e/m8b.sh` PASS (full scenario; values: type 2 reply `0x10000212` result 1 — the measured value; the 207
  field while WALK_DRUNK runs `0x170404EC` = measured 1260/0x1704; idle `0x80E804A5` = measured IDLE_STANCE
  1189/0x80E8; drunk 5000 → `052C(0,100)`, 2100 → 42; spectator view 50,50,50 → 60,60,50 with the ped at 50,50,70).
* m1, saa, m2, m3, goal, m4, m5, m6, m8a: all exit 0 on this branch (saa/m5 need `data/` reachable from the
  worktree: a gitignored symlink to the main checkout's `data/`).
* 544 workspace tests; clippy host + i686 (samp-client, samp-launcher, samp-tracer) 0 warnings.
* Parity (same `m8bscen.sh` input, tracer sha256 9980c09c…): original `target/trace/orig-20261105-020419` (trace
  lane) vs ours `target/vehroot/target/trace/ours-20261105-015155` (lane veh), per-mark windows with the per-frame
  noise skipped: widescreen, all camera windows (157/158 cuts, 162, 82 pos/look/cut 2/time 0, 81 incl. the per-frame
  follow during MoveObject), gravity, drunk (same `052C` values), fighting style, shop, wanted, stunt, money,
  velocity, controllable, interior, every animation window (WALK_DRUNK, DANCING unloaded → give-up then play, SEX,
  IDLE_CHAT, KO_skid_front, clears): **0 differing lines** apart from repeat counts and positions of the walking
  ped (timing). Spectating: same commands/teleports per frame; remaining lines are the out-of-scope markers/LOS.
  Bounds: see above.
* DIFF-2 (`rpc-coverage.sh --skip-build --only …` under the diff lock, DIFF_CARGO_TARGET = this build; report
  regenerated against this branch's sources; runs `target/diff/runs/orig-20261105-022105`, `…-022326`,
  `ours-20261105-022547`): MATCH 157, 158, 162, 82, 81, 111, 146, 17, 156, 35, 89, 33, 133, 104, 18, 20, 30, 90, 15,
  103, 86, 96 (after the `06E7`/`0161` output-local fix), 164. Remaining: 87 = camera position 1 m (trace identical;
  the walk-drunk distance before the clear); 124 = player-marker blips only; 126 = CREATE_PLAYER encoding (logged
  decision) + player-marker blips; 127 = "order" only in this run (MATCH in the previous run).
* EFX (`tools/efx/run.sh both`, EFX_STEPS="campos camlook behind wide gravity specon specpl", orig
  `target/efx/orig-20261105-022121`, ours `target/efx/ours-20261105-022401`): game-call sequences identical for
  157, 158, 162, 111, 146 (both captures); write diffs only the script engine's scratch globals
  (`CommandsExecuted`, `ScriptParams`) and camera state written by the game frame inside the 20-frame window;
  124/126 handlers only store flags (empty specs) — their effects are covered by the trace parity above.
