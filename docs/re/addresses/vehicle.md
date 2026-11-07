# vehicle.rs — samp.dll addresses

Specs: docs/re/subsystems/vehicles.md, docs/re/subsystems/remote-control-vehicles.md,
docs/re/subsystems/remote-players-sync.md. Measurements: docs/phase3/m5-notes.md, docs/phase3/trace-notes.md
"Vehicles (TRACE-4)" and "Vehicles II (TRACE-5)".

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `MODEL_FREIGHT_CARRIAGE`, `passenger_enter_warps` | 0x100AC450 | | passenger enter / warp helper (called as (handle, 1)) | remote-control-vehicles 5 |
| `passenger_enter_warps` | 0x10006FE0 | | local passenger-enter poll | vehicles "Passenger-enter key" |
| `MODEL_FLAT_CARRIAGE`, `counts_as_passenger` | 0x100AC000 | | "is a passenger" test | remote-control-vehicles 3.1 |
| `MAX_REMOTE_PLAYER_ID` | 0x10001B1F | | `id ≤ 0x3EC` check of RPC 96 type 4 | vehicles "96 SCMEvent (S→C)" |
| `mod_shop_follow_applies` | 0x10001A60 (case 4) | | RPC 96 SCM event handler | vehicles "96 SCMEvent (S→C)" |
| `mod_shop_follow` (vehicle) | 0x100B70D0 | | vehicle interior setter (full value) | vehicles "96 SCMEvent (S→C)" |
| `mod_shop_follow` (ped) | 0x100AD380 | | ped interior setter (low byte) | vehicles "96 SCMEvent (S→C)" |
| `DRIVER_*`, `correct_driver` | 0x10015140 | | driver sync correction | remote-players-sync 2.1 |
| `TURN_CLAMP` | 0x100FE4B8 | | turn-speed clamp switch (value 1) | remote-control-vehicles 2.2 |
| `UNOCC_*`, `correct_unoccupied` | 0x100158D0 | | unoccupied (209) apply | remote-players-sync 209 |
| `train_needs_teleport` | 0x10015650 | | train path of the driver apply | remote-control-vehicles 2.2 |
| `Kind`, `kind_from_vtable` | 0x100B6220 | | vehicle kind classifier by vtable | remote-control-vehicles 2.3 |
| `tag_at_vehicle` | 0x10074270, 0x10075390 (with 0x100B7E40) | | nametag anchor selection | ui-rendering / m8a-notes |
| `driver_enter_refused` | 0x100A3700 | | driver-enter gate | remote-control-vehicles 5 |
| `is_towable_list` | 0x100B7DB0 | | towable trailer model list | remote-players-sync 210 |
| `unoccupied_seat`, `local_is_closest` | 0x10006E00 | | who sends 209 for a vehicle | remote-control-vehicles 3.2 |
| `unoccupied_seat` (`moving`) | 0x1009FE40 | | "stopped" test (moving list, zero speed) | remote-control-vehicles 3.2 |
| `local_is_closest` (`None`) | 0x1009F430 | | ped-in-world test | remote-control-vehicles 3.2 |
| `RAIL_MODELS` | — | — | vehicles.md "Model sets" | vehicles "Overview" |
| `TRAIN_SYNC_MODELS` | — | — | train path, step 2 | remote-control-vehicles 2.2 |
| `train_type` | — | — | CREATE_MISSION_TRAIN types | remote-control-vehicles 1.2 |
| `engine_wanted` | — | — | engine/lights application | vehicles "24 SetVehicleParams" |
