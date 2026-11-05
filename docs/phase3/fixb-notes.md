# FIX-B — vehicle and player-marker parity fixes (par-audit items 3–7)

Branch `fix-vehicles`. Static details come from samp.dll **B** (2019 build, the one the lanes run; trace
addresses `samp.dll+0x…` are B) read with objdump; every behaviour was compared with the original client's
DIFF-2 traces (`target/diff/runs/orig-20261105-013339`, `orig-20261105-014057`, main checkout).

## 1. Player radar markers (audit R3)

Original (static, B):
* Colour table setter B 0x100A7220 / getter B 0x100A7240; the `CRadar::GetRadarTraceColour` hook (row 51,
  B 0x100A3A50) returns `getter(index)`; already in main (`worldhooks.rs`). Blips of players use the **player id
  as colour index**, so alpha 0 hides the blip, SetPlayerColor / SetPlayerMarkerForPlayer (RPC 72) need no blip
  work.
* PlayerJoin (B 0x1000FDE2): colour set only when non-zero and id < 1004. Spawn of RPC 32 (B 0x1001714E): colour
  set when non-zero, after the ped exists. RPC 72 (B 0x100197B0): local id, or a remote id ≤ 1004 with a pool
  entry. (Ours set colour 0 on join, which turned the default palette colour black/invisible.)
* Spawn (B 0x10017164…0x100171A1): the 208 marker blip (remote +0x1F9) is removed (`0x0164`); if the InitGame
  marker mode (netgame settings +0x30) ≠ 0: ped method B 0x100AF740 → `0x0187 ADD_BLIP_FOR_CHAR`,
  `0x0165(blip, playerId)`, `0x018B CHANGE_BLIP_DISPLAY(blip, 2)`. GTA's `CPed` destructor clears the ped blip.
* 208 (B 0x10009F00, only in game state 5): hasPos 0 → B 0x100144B0(0) removes the marker blip; hasPos 1 →
  B 0x100144F0: flag := 1, old blip removed, and only if the player has no ped: B 0x100A0DA0(sprite 0, x, y, z,
  colour = id, style 0) = `0x04CE ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD`, `0x0165(id)`, `0x0168` scale 2 (3 from
  colour 0x3EC).
* Remote reset B 0x10015F50 (stream-out of a spawned ped, game-mode restart) and destructor B 0x10016610 (quit)
  remove the marker blip.
* No client-side cadence: the server's 208 rate drives the updates.

Ours: `players.rs` (`on_markers`, `spawn_blips`, `reset_marker`), `client.rs` (208 routing, colour rules),
`remote.rs` (`marker_blip` / `ped_blip` in the `remote` status line), `script.rs` (0x0187, 0x018B, 0x04CE).
Also from the same RPC 32 handler (B 0x10010CC9): an NPC's ped gets `SET_CHAR_PROOFS(0, 1, 1, 0, 0)`.

Evidence: DIFF-2 (`--only 'flow_|pos_base|vehicle_|trailer_|npc_|spectate_'`, ours
`samp-rs-wt/fixb/target/diff/runs/ours-20261105-031154`, report against both original runs): `npc_spawn` has the
same `0x04CE [1962,1346,15,0] / 0x0165 [.,49] / 0x0168 [.,2]` before the spawn and `0x0164 / 0x0187 / 0x0165 [.,49] /
0x018B [.,2]` at the spawn; the only remaining lines are pool-handle numbers (513 vs 257) and the decided
`CREATE_PLAYER` immediate argument. `npc_stream_out`, `npc_destroy`, `npc_kill`: MATCH.

## 2. Remote driver exit (audit V7)

Both clients run `0x05CD TASK_LEAVE_CAR` for RPC 154. Ours then warped the ped out with `0x0362` ~80 frames later
from the 207 store: samp's store (B 0x10017288…0x100172D4) skips that warp while the ped's task-manager primary
slot (intelligence +0x10) holds `TASK_COMPLEX_LEAVE_CAR` (0x2C0, B 0x100AC650). Added `leaving_car()` to
`vehicles.rs::after_foot_store`. DIFF-2 `npc_exit_vehicle`: the `0x0362` / `CPed::Teleport` lines are gone; the
re-added car blip `0x0161/0x0165(1004)` matches; only handle numbers differ.

## 3. AddVehicleComponent order (audit V6)

Main already follows B 0x100B7FD0 (`0x06EA`, `0x0247`, `0x038B`, `0x06E9`, ≤ 20 polls with Sleep(1), `0x06E7`).
DIFF-2 `vehicle_component_add` (RPC 96) and `vehicle_component_remove` (RPC 57): MATCH.

## 4. Door does not swing (audit V2)

Not a vehicle bug: in the audit's COV run our player was still inside the interior during the vehicle window (I-1,
RPC 156 missing then; `outside` pos 494,−25,1000 vs 1834,−1681,13). With RPC 156 in main, COV
(`samp-rs-wt/fixb/target/cov/runs/both-20261105-032426`, `COV_IDLE_SECS=40`) shows `CDoor::Process` from
`CAutomobile::ProcessSwingingDoor` in ours too (vehicle window, frame 6554 vs original 6602, while driving). The
remaining idle_end first-hit difference comes from a different drive: our car hit something and lost the door
(`CAutomobile::SpawnFlyingComponent`, `CDamageManager::SetDoorStatus` only-ours in that run).

## 5. Rows 177, 62 and the remove-weapon call (audit V1/V3)

* Row 177: `vehpatches.rs` INIT, NOP of `call CPostEffects::SpeedFX` at 0x704E8A.
* Row 62: `vehhooks.rs`, the `SetDirtTextures` call in `CVehicle::Render` (0x6D0E7E) goes to a stub that, like
  B 0x100A4570, only stores esi (the rendered vehicle, B global 0x10151550, read by samp's plate texture code
  B 0x10074BF6) and returns: per-vehicle dirt textures are never set.
* B 0x10007903…0x1000792F: every driving frame (ped state 50) unless passenger / carriage 569-570 (B 0x100ABFD0)
  samp calls `CPed::RemoveWeaponWhenEnteringVehicle(0)` (0x5E6370) before the driver world-bounds check; in
  `client.rs`.
* COV: `CPostEffects::SpeedFX` and `SetDirtTextures` are no longer only-ours; `RemoveWeaponWhenEnteringVehicle`
  is hit by both in the vehicle window (frame 6413 vs 6436).

## DIFF-3 vehicle steps

`tools/diff/xclient.{pwn,py}`: `xc car / putin / comp`; steps `veh_create`, `veh_enter_*`, `veh_drive_*`,
`veh_exit_*` for A and B, `veh_passenger_B`, `veh_passenger_exit_B`, `veh_component_A`; checks `vehicle`
(in-vehicle, model, seat), `veh_pos`, `veh_heading` (tol 1.0 m / 0.15 rad without calibration), `veh_mods`.
Unoccupied/trailer stay PENDING.

Results (`DIFF_CARGO_TARGET=… tools/diff/cross-client.sh --skip-build`, run
`samp-rs-wt/fixb/target/diff/runs/xclient-20261105-033817`; calibration with the vehicle steps
`--calibrate` → `xcalib-20261105-034226`): **44/45 step-directions MATCH**, every vehicle step MATCH in both
directions except `veh_enter_B` (ours → orig):
* "missing on observer `0:63`" is the original's own behaviour (the orig-vs-orig run shows the same miss in that
  direction; not judged).
* `sender_vs_original`: our LOCAL ped approaches the door with player anim `e5bd31dd/118:5` and never blends in
  `a38efa5f/118:0`, which the original sender plays from 0.6 s until the door anims (both reach the door at
  ≈ 0.85–0.94 s). In our run the ped is already in move state 6 at the first sample (0.25 s); the original is still
  idle there. Cause unknown (GTA's `CTaskComplexGoToCarDoorAndStandStill`, run → slow-down at 4 m; the
  enter-ctor hook resumes at 0x6402F7 as samp does). **Measurement request**: per-frame `m_nMoveState`, pad
  sprint/walk keys and the go-to-door subtask's move state of the local ped on both clients during an F enter.

## Not done (item 6) / measurement requests

* Train exit, remote passenger weapons/drive-by, trailer receive, aircraft remote z: each needs an original run of
  a scenario that does not exist yet (vehfs has no train/aircraft/passenger-weapon steps) — not implemented
  (PLAN §3b).
* PLATE-1: not started (row 59/60 painter + the row-62 vehicle pointer recorded here).
* Radar vehicle colours: done by row 51 (main); DIFF-2 shows the same `0x0165(1004/1006)` blips.

## Checks run on this branch
* e2e (lane veh): m1, m2, m3, m4, m5, m6, m7, m8a, m8b, goal, saa — all PASS (m8b failed once on its drunk-level
  timing window, 1897 vs ≥ 1900, and passed on the re-run).
* `cargo test --workspace` green; clippy host (all targets) and i686 (`samp-client`) 0 warnings.
* DIFF-2 per step (vehicle/NPC subset): remaining lines are pool/blip handle numbers only (harness limitation 2),
  `CREATE_PLAYER`'s immediate argument (decided) and the local `weapon_skills` 500 vs 1000 state (not vehicles).
