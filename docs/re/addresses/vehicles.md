# vehicles.rs — samp.dll addresses

Specs: docs/re/subsystems/vehicles.md (rcv = remote-control-vehicles.md, rps = remote-players-sync.md).
Measurements: docs/phase3/m5-notes.md, docs/phase3/trace-notes.md "Vehicles (TRACE-4)" / "Vehicles II (TRACE-5)".

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `CAR_JACKED_TEXT` | | 0x100E6158 | "~r~Car Jacked~w~!" string | rps 2.1 |
| `car_jacked` | | 0x10016220 (from 0x100172F0) | car-jacked warp + game text (1000 ms, style 5) | rps 2.1 |
| `remove_from_world` | 0x1009F460 | | entity "remove from world" virtual (also remote ped wrapper slot 2, called from 0x10016F0A for RC drivers) | rcv 2.2 |
| `seat_of` | 0x100ABEB0 | | ped in-vehicle flag helper | rcv 5 |
| `Veh::last_unocc_sender` | — | — | vehicle pool +0x8E34 | rps 209 |
| `Veh::spawn_pos` | — | — | vehicle pool +0x11AD8 (`/dl` "sPos") | ui-rendering 2.3 |
| `Vehicles::local_drive_by` | — | — | local player +0x1CB | rps "Remote passenger drive-by and cuffs" |
| `Vehicles::remote_vehicle` / `last_vehicle` | — | — | remote player +0x1E7 (mod-shop follow reads +0x1E1) | vehicles "96 SCMEvent (S→C)" |
| `set_init_game` (manual lights) | — | — | patch rows 140–147 (`vehpatches::MANUAL_LIGHTS`) | gta-patches "Vehicles" |
| `first_frame` | — | — | patch row 356 (`vehpatches::STATS`) | vehicles "Driving skills" |
| `stream_in` | 0x1001F080 → 0x100B8410 | | pool add → vehicle constructor | rcv 1.2 |
| `stream_out` (local ped) | 0x100AC5B0 | | warp of the local ped to the vehicle position | rcv 1.3 |
| `delete`, `delete_handle` | 0x100B8930 | | vehicle object destructor (model release over 80 loaded models at object +0x87, not reproduced) | rcv 1.3 |
| `mod_shop_follow` | 0x10001A60 (case 4) | | RPC 96 type 4 | vehicles "96 SCMEvent (S→C)" |
| `mod_shop_follow` (interior) | 0x100AD380 (interior, 0) | | local interior without scene refresh | vehicles "96 SCMEvent (S→C)" |
| `mod_shop_follow` (control) | 0x100AC7D0 | | set controllable | vehicles "96 SCMEvent (S→C)" |
| `put_local_in`, `seat_allowed` | 0x100AC2D0 | | seat warp with seat checks (RPC 70, remote seating) | vehicles "70 PutPlayerInVehicle" |
| `put_local_in`, `passenger_poll` (weapon) | 0x100ABC90 | | GET_CURRENT_CHAR_WEAPON read (efx RPC 70 diff) | vehicles "70 PutPlayerInVehicle" |
| `seat_remote` | 0x10014800 | | remote driver / passenger seat warp | rps 2.1 |
| `after_foot_store` | | 0x10017288 … 0x100172D4 | not while the leave-car task runs | rps 2.1 |
| `leaving_car` | | 0x100AC650 | leave-car task test (intelligence +0x10, type 0x2C0) | rps 2.1 |
| `reset_remote_vehicle` | 0x10015FA0 | | remote cache reset (RPC 166, stream-out) | rps 2.1 |
| `apply_unoccupied` | 0x100158D0 | | 209 apply | rps 209 |
| `apply_trailer` | 0x10015C90 | | 210 apply | rps 210 |
| `apply_trailer` (second towable test) | 0x100B7E10 | | unspecified second towable test | rps 210 |
| `remote_tick` | 0x100166B0 | | remote apply per frame (driver / passenger part) | rps 2.1, rcv 2.2 |
| `force_driver_send` | 0x10007080 | | last 200 on death while driving | rps 0.4 |
| `scm_event_out`, `vehhooks::HookEvent::Scm` | | 0x10001CE0 | RPC 96 C→S | vehicles "96 SCMEvent (C→S)" |
| `passenger_poll` | 0x10006FE0 | | passenger-enter poll | vehicles "Passenger-enter key" |
| `passenger_poll` (seat) | 0x100AC450 (handle, 1) | | 570 warps, others animate | vehicles "Passenger-enter key" |
| `pool_tick` | 0x1001F3C0 | | vehicle pool per-frame tick | rcv 1.4 |
| `unoccupied_send` (`moving`) | 0x1009FE40 | | "stopped" test | rcv 3.2 |
| `unoccupied_send` (ped in world) | 0x1009F430 | | ped-in-world test | rcv 3.2 |
| `blips` | 0x100B7720 | | objective / unoccupied radar blips | vehicles "Vehicle radar blips" |
| `create_detached` | | 0x100A17A0 | PlayCrimeReport's temporary vehicle | session-misc |
| `create_at` | 0x100B8410 | 0x100B83E0 | vehicle constructor | rcv 1.2 |
| `busy_load` | | 0x100B8474 … 0x100B84B6 | unbounded model busy-wait | rcv 1.2 |
| `add_component` | | 0x100B7FD0 | add-component vehicle method | vehicles "164 StreamInVehicle" |
| `samp_type` | 0x100B6220 | 0x100B61F0 | vehicle type code by vtable | rcv 2.3 |
| `camera_behind` | 0x1009D7A0 | | camera reset | vehicles "70 PutPlayerInVehicle" |
| `pool_ref` | 0x100B3C40 | | `CPool::GetRef` wrapper (gta 0x424160) | rcv 1.1 |
| `apply_damage` | 0x100B7B00 | | damage setter | vehicles "106 SetVehicleDamageStatus (S→C)" |
| `apply_tyres` | 0x100B7980 | | tyre setter | vehicles "106 SetVehicleDamageStatus (S→C)" |
| `stop_drive_by` | 0x100AD0C0 | | stop drive-by task | rps "Remote passenger drive-by and cuffs" |
| `start_drive_by` | 0x100ACFD0 | | start drive-by task | rps "Remote passenger drive-by and cuffs" |
