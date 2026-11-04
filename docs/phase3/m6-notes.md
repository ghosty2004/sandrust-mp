# M6 — world entities (implementation notes)

Branch `world-entities` (worktree `samp-rs-wt/m6`). Specs: `docs/re/subsystems/world-entities.md` (+ RE-10
additions), `docs/phase3/re10-gaps.md` (M6 table, CSV rows), `docs/phase3/trace-notes.md` "World entities (TRACE-6)",
`remote-control-combat.md` §6.1 (damage decision). Static details read from samp A with objdump are cited by
address in the code.

## Files

| file | content |
|---|---|
| `crates/samp-client/src/world.rs` | platform-independent rules + tests: constants (draw distance 300 split, 1000-frame release, pickup poll 7 frames / cooldown 15, actor ped type / decision maker), map-icon opcode + scale, radar trace colours, label colour, material rules, `rotation_rows` (GTA `CMatrix::SetRotate`), MoveObject step (`Motion`), gang-zone record + flash phase, removal rule |
| `crates/samp-client/src/objects.rs` | object pool (RPC 44/45/46/47/99/122/75/84/169), attached objects (113), select/edit state (27/28/116/117), RemoveBuildingForPlayer (43), per-frame process (streaming, attachments, motion) |
| `crates/samp-client/src/entities.rs` | pickups (95/63, 131 poll), actors (171–178, 177 events), checkpoints (107/37/38/39), gang zones (108/120/121/85), map icons (56/144), 3D labels (36/58: placement, LOS, projection) |
| `crates/samp-client/src/worldhooks.rs` | CSV rows: byte rows and hooks, shared state for the hooks, material pre/post pass, samp's own render passes |
| `crates/samp-client/src/ui/render.rs` | label drawing in the Present pass (small font, 4-way outline moving only left/top, fmt 0x101) |
| `crates/samp-client/src/ui/mattext.rs`, `ui/d3d.rs` | material text: render target per text slot (2× material size, D3DX font, colour codes per glyph), `SetTexture` substitution in the device proxy while the object renders |
| `client.rs`, `dll.rs`, `control.rs`, `vehicles.rs`, `lib.rs` | wiring: RPC routing, CGame::Process pre-step (object process), `world` control command, vehicle handle accessors |
| `tools/e2e/m6.sh`, `m6scen.sh`, `m6parity.sh` | acceptance test, shared scenario, parity runner (original in the trace lane, ours in lane veh) |

## CSV rows applied (`worldhooks::install`, logged as "main: world rows")

11–15 (ped Render fog), 20 (CCivilianPed ProcessControl, invulnerable actors pinned), 25 (ProcessPedsAfterPreRender →
attached objects on bones), 41/42 (gang zones on radar / pause map; the original gang overlay is not drawn), 44
(pickup collision), 51 (GetRadarTraceColour), 53 (ComputeDamageResponse: rules 0/1/2/7/8 only), 63 (CObject::Render
materials), 65 (LoadObjectInstance → 19300), 66 + 371 (escalator constants around CEscalator::Update), 67 + 372
(CreateRwObject with the AddEntity call NOPed), 68/69 + 373 (DeleteRwObject keeps escalators running), 75
(PositionAttachedEntity: sync rotation 0), 76/77 (samp's own passes), 78 (render-list filter), 79 (roadsign null
guard), 82 (vehicle camera collision off while RPC 167 disabled collisions), 83 (camera ignores attached objects),
96 (ProcessVerticalLine: objects tested whenever buildings are, except for BreakObject_c::Init; TRACE-7), 113 (object pool 3000), 211 (pickup visibility 10000).
Rows 172 (LoadPickup := ret, no IPL pickups) and 208 (no interior pickups) are applied by the player-parity work
(patches.rs) (boot / SP-OFF fixes), which M6
depends on for IPL-pickup parity. Already applied before M6: 108/109/119/131 (patches.rs), 128 (archive mode), 132/248–261 (stores.rs), 379 (UI device
proxy).

Not applied (with reason):
* 24 (CWorld::ProcessAttachedEntities → surf glue for local/remote players): player surfing, not an entity feature;
  belongs to the player parity work (remote-players-sync.md "surf").
* 81 (CameraConeCastVsWorldCollision): TRACE-7 measured it — after GTA's BuildCacheOfCameraCollision found nothing
  and the camera is inside ±3000, samp tests client objects (pool 1), per-player objects (pool 2) and vehicles
  (skipped while RPC 167 disabled collisions); the no-camera-collision flag is read only there. Not implemented yet:
  samp's own object/vehicle camera test (A 0x100A5990 → 0x10012DE0) needs its geometry routine analysed.
* 97, 348, 362, 364: dormant custom-model path (R4 never registers RPC 179/183/185; no negative model ids reach GTA).
* 374 (SetupEntityVisibility NOPs): used by samp's wrapper A 0x100A4FA0, which only acts on one entity stored at
  A 0x101515E8 (camera attach, RPC 81 — M8b), not by the row-75 hook as the CSV says.

## Behaviour (TRACE-6 + static)

* RPC 44: model without model info → 18631; draw distance ≠ 0 overrides the atomic model info's draw distance;
  CREATE_OBJECT, exact matrix position, SET_OBJECT_ROTATION, entity flag bit 31; attachments stored; materials applied
  (kind 1: FindTxdSlot + RwTextureRead in the pushed TXD; colour stored as the wire u32); an existing id is destroyed
  first (DeleteRwObject, DELETE_OBJECT, MARK_MODEL_AS_NO_LONGER_NEEDED when the model info's ref count is 0).
* Render path: materials / draw distance > 300 → not in GTA's render list; samp's pass after RenderEverythingBarRoads
  (pool order) and after RenderFadingInEntities (draw-last, far → near). Draw: frustum test of the bounding sphere,
  alpha ref / cull mode from the model flags, fog on, then PreRender + RenderOneNonRoad (< 600 m, no material colour),
  lighting + RW render (material colour, < 600 m: entity bit 28), or RW render (≥ 600 m).
* Streaming of the samp-drawn objects: RW object created via DeleteRwObject + SetModelIndex once in range and loaded;
  released after 1000 consecutive out-of-range frames. Ordinary objects: REQUEST_MODEL when closer than dd − 30.
* MoveObject: speed × 0.02, drift correction ±0.1, samp writes the matrix (pos += move speed × ms_fTimeStep), arrival
  clamps to the target with speeds 0, rotation slerp by progress, turn speed z clamped to ±0.001. StopObject zeroes the
  speeds in place.
* Attached objects (113): CREATE_OBJECT at the ped position + rotation; every frame after ProcessPedsAfterPreRender:
  UpdateRpHAnim once per ped, Remove, rows = S·Rz·Ry·Rx·B, pos = B.pos + offset in the bone frame, Add.
* Actors: ped type 5, z − 1, heading, 0x0446(h, 0), decision maker 65542, health (≤ 0 → EXPLODE_CHAR_HEAD, cancelled by
  the damage hook rule 1), proofs. Damage by the local ped → RPC 177 (unscaled) and the response is cancelled.
* Pickups: CREATE_PICKUP; collision path → 131 rel 9 with cooldown 15 (counted down by the 7-frame poll); type 14
  polled with HAS_PICKUP_BEEN_COLLECTED → 131 rel 10.
* Checkpoints / race checkpoints / zones / icons / labels: as listed in the module header of `entities.rs`.

## Deviations and open items (TODO-unknown unless marked)

* **Material text**: TRACE-6 batch 3 + TRACE-7: rendered once per slot from the CGame::Process pre-step (between
  the previous Present and RenderWare's BeginScene, like samp's wrapper B 0x100A2670): D3DXCreateTexture (2× size,
  RENDERTARGET, X8R8G8B8) + CreateRenderToSurface (D16) + font (face, h = size, wt 700 if bold, cs 1, q 0);
  Clear(TARGET|ZBUFFER, background, 0.0), sprite Begin(0x10), DrawText fmt 0x105 for centre (left/right inferred).
  Not recreated after a device reset.
* **Blank texture** (material model 0, TRACE-7): `SAMP\blanktex.txd` (loose file of the SA:MP install) loaded into
  TXD slot "blanktex" on first use, texture "blanktex" cached and never released. samp loads it 4 frames after the
  RPC; ours loads it in the RPC frame.
* **Object editor gizmo and selection picking** (27/116/117, MR-6.11): the RPC state machine, ESC → 117/116
  response 0 with the current values (TRACE-6 batch 3) and RPC 28 are implemented; the per-frame editor
  (A 0x10073270, response 2 every ≥ 250 ms, drag rates unmeasured) and the mouse picking in selection mode
  (A 0x1006DA10; TRACE-6: a click sends 27 with type 1, id, model, pos) are not.
* **Material pre-pass** records and restores the RW material state around every render (samp records once per RW
  object and applies from the second render on, +0x118F): same visible result, first frame may differ.
* **Label LOS test** runs in the client tick (once per LOS label and frame, as in parity run orig-20261104-233854), not inside the Present pass;
  the label projection uses the device's view/projection transforms read in the client tick (samp uses copies its
  device proxy captured from SetTransform).
* **Ped fog restore**: samp restores FOGENABLE from the RW fog state it tracks through a RwEngine render-state hook
  (A 0x100C3E30, not a CSV row); we read the same state with RwRenderStateGet.
* **Damage hook (row 53)**: only the actor rules; the player rules (RPC 115, same-team cancel, NPC) are M7.
* **206 / 204 around actor damage** (bullet sync / target actor in weapons sync): M7.
* Labels attached to the local player are not drawn for that player (TRACE-6 batch 3).
* Actor destruction (172) deletes the ped through its scalar deleting destructor like samp (A 0x1009C460), no
  DELETE_CHAR; an animation library that does not load within 15 ms plays nothing (DIFF-2).
* Ordinary objects request their model only while the local ped is in a vehicle (A 0x100A96C9); material texture
  models are released after the lookup when unused (A 0x100A866E) — both found by DIFF-2.
* **Escalator / material text objects with timed hours**: timed objects (+0x1199) are not created by RPC 44 (always 0).

## Test notes

* `tools/e2e/m6.sh` taps Space (sprint without a direction) every 20 s: without CSV row 244 (idle camera NOP, owned by the
  player-parity branch) GTA's idle camera hides the HUD after ~90 s without input, which stops the radar (and the
  gang-zone hook). The parity runs (`m6parity.sh`) do not wiggle; their radar windows end ~90 s after spawn on ours.
* TRACE-7 (measured): RPC 117 response 2 every 250–287 ms while dragging (first one 126 ms after the press, unchanged
  position), response 1 with the final position on save — needs the editor gizmo (not implemented). Pause-map
  gang zones: `DrawAreaOnRadar(…, in_menu = TRUE)` per zone per frame from the PrintMap call (row 42, same hook; the
  flag comes from 0xBA67A4 like samp). RemoveBuilding replay: the reloaded IPL instance becomes model 19300 at its
  original position, area 0, not lowered (row 65 replaces only the model id).

## Verification (2026-11-05, after merging main)

* Lane veh, isolated display: m1, saa, m2, m3, goal, m4, m5 (passed on rerun; one run had the server see no car
  movement), m6, m8a all exit 0; 524 tests, clippy host + i686 0 warnings.
* Parity (same `m6scen.sh` input incl. the blanktex step; tracer sha256 9c12c233…): original
  `target/trace/orig-20261105-002451` (trace lane) vs ours `target/vehroot/target/trace/ours-20261105-004327`
  (lane veh), feature-filtered `diff.py --dump` windows with script handles masked:
  objects (incl. blanktex) −1 repeat line; checkpoints 0/0; map icons 0/0; labels one repeat count; gang zones repeat
  counts only (49 vs 60 fps); pickups: the server streamed the three pickups in a different order; actors: the
  original re-streamed the actor for SetActorInvulnerable (server timing of the input). Server events identical
  (2× OnObjectMoved, pickup 0 twice + pickup 2, checkpoint and race-checkpoint enter) except actor damage (unaimed
  shots hit on ours only).
