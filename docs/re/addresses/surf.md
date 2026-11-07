# surf.rs — samp.dll addresses

Spec: docs/re/subsystems/remote-control-onfoot.md, "3.7 Surfing"; measurements in docs/phase3/close-notes.md,
"Surfing". B is A − 0x30 in this region where not listed.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `LocalSurf::probe` | 0x10003730 | | on-foot surf probe (every local on-foot tick) | onfoot 3.7 |
| `gta::local_glue` | 0x10003570 | | local glue: hold ped at offset or re-measure | onfoot 3.7 |
| `gta::remote_glue`, `remote_target` | 0x10014650 | | remote glue from a 207 surf id | onfoot 3.7 |
| `gta::after_process_attached_entities` | 0x100A4990 | | samp's hook on the `CWorld::ProcessAttachedEntities` call (gta 0x5689FD) in `CWorld::Process` | close-notes "Surfing" |
| `LocalSurf::sync_fields` | 0x100050E8 | | 207 surf id / offset written while latched | onfoot 3.7 |
| `LATCH_MS` | 0x100038B5 | | 2000 ms contact latch compare | onfoot 3.7 |
| `MAX_VEHICLE_DISTANCE` | 0x100037E3 (float 0x100E5808) | | 30.0 vehicle distance limit | onfoot 3.7 |
| `MAX_REMOTE_OFFSET` | 0x100146EB (floats 0x100E6110 / 0x100E6114) | | ±100 remote offset bounds | onfoot 3.7 |
| `LocalSurf::reset` | 0x10003D5E | | spawn clears the surf state | close-notes "Surfing" |
| `LocalSurf::detach` (teleport) | 0x100193E4 / 0x100194FB → 0x10003710(0, 0) | | RPC 12/13 teleport resets | close-notes "Surfing" |
| `LocalSurf::detach` (stream-out) | 0x10011055 | | RPC 165 stream-out of the surfed vehicle | close-notes "Surfing" |
| `LocalSurf::vehicle_streamed_out` | 0x10011041 | | RPC 165 handler test | close-notes "Surfing" |
| `LocalSurf::object_destroyed` | 0x1001BCB7 | | RPC 47 destroying the surfed object | close-notes "Surfing" |
| `LocalSurf::latch_after_measure` | 0x10003A09 | | glue on after the first measurement | onfoot 3.7 |
| `gta::cancel_tasks` | 0x100ACCD0, 0x100AC680, 0x100AC610 | | the probe's three task tests | close-notes "Surfing" |
| `TASK_NO_GLUE` | 0x100ACC50 | | task 211 test in the primary slot | onfoot 3.7 |
| `gta::task_type` | 0x100B40E0 | | task type with vtable-in-.rdata check | onfoot 3.7 |
| `gta::player_driven` | 0x100B7120 | | "player-driven" vehicle test | close-notes "Surfing" |
| `transform` | 0x100B4D50 | | matrix × vector helper | onfoot 3.7 |
| `RW_MATRIX_INVERT` | 0x100B5780 | | samp's wrapper around `RwMatrixInvert` | onfoot 3.7 |
| `PAD0` | 0x10114AE8 | | samp's pointer to `CPad::Pads[0]` | onfoot 3.7 |
| `LocalSurf` fields | — | — | local-player object +0x2D4 id, +0x2D6 last contact, +0x2DA surface, +0x2DE glue, +0x2E2 latched, +0x2E6 offset | onfoot 3.7 |
