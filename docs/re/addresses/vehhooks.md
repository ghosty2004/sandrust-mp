# vehhooks.rs — samp.dll addresses

Spec: docs/re/subsystems/remote-control-vehicles.md, "2.3 The vehicle `ProcessControl` hook", "2.4 Other
per-vehicle hooks", "5. Enter / exit"; docs/re/subsystems/vehicles.md, "Radar zoom while a passenger".
Measurements: docs/phase3/trace-notes.md "Vehicles (TRACE-4)", docs/phase3/m5-notes.md, docs/phase3/m6p-notes.md
"Train exit". Patch rows refer to docs/re/data/gta-patches-r4-annotated.csv (its `samp_site_A/B` columns hold
the samp.dll install site of each row).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `vehicle_process_control` (rows 26–35) | 0x100A3140 | | `ProcessControl` replacement of all ten vehicle vtables (slot +0x28) | rcv 2.3 |
| `enter_filter`, `gate_driver_enter` (row 48) | 0x100A3700 | | driver-enter gate in the enter-task constructor hook | rcv 5 |
| `gate_driver_enter` (cuffed / carrying) | 0x100ACF30, 0x100ACE80 | | reads of the local ped object +0x2DE / +0x2E2 (special actions 24 / 25) | rcv 5 |
| `leave_filter` (row 49) | 0x100A38C0 | | leave-task constructor hook | rcv 5 |
| `leave_filter` (train warp) | | 0x100A3992 … 0x100A39E0 | WARP_CHAR_FROM_CAR_TO_COORD to train + (2.5, 2.5, 0) inside the constructor | m6p-notes "Train exit" |
| `HookEvent::Scm` | | 0x10001CE0 | script opcode 0x09C6 network event → RPC 96 | vehicles "96 SCMEvent (C→S)" |
| `RENDER_VEHICLE` | | 0x10151550 | samp's "vehicle being rendered" global, read by the plate code | vehicles "Number plates" |
| `render_dirt_stub` (row 62) | | 0x100A4570 | replaces `SetDirtTextures` in `CVehicle::Render` | vehicles "Number plates" |
| `Shared::disable_collisions` | — | — | netgame +0x232 (RPC 167) | rcv 2.4 |
| `siren_alarm_horn` (rows 36–40), `uses_siren` (rows 92–94) | — | — | see CSV `samp_site_A/B` | rcv 2.4 |
| `break_tow_link` (rows 70–74), `pec_*` (rows 84–91) | — | — | see CSV `samp_site_A/B` | rcv 2.4 |
| `radar_find_player_speed` (row 64) | — | — | see CSV `samp_site_A/B` | vehicles "Radar zoom while a passenger" |
| `train_derail_stub` (row 47 over row 188) | — | — | see CSV `samp_site_A/B` | rcv 2.4 |
| `has_collision_loaded` | — | — | samp passes an uninitialised area argument (docs/phase3/measurement-requests.md, `HasCollisionLoaded` argument) | rcv 9 |
