# behaviour.rs

Most of these were read in build B only; "—" means not recorded for that build.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Bounds` | — | 0x100088E0 | InitGame sets the bounds to ±20000 (no bounds); stored at netgame Settings +0x05/+0x09/+0x0D/+0x11 | player-state.md "17 SetWorldBounds" |
| `BOUNDS_TEXT`, `BOUNDS_TEXT_MS`, `BOUNDS_TEXT_STYLE` | — | 0x10003F9D | `CMessages::AddBigMessage` call of the out-of-bounds message | player-state.md "17 SetWorldBounds" |
| `outside` | — | 0x1009F5F0 | outside-bounds test | player-state.md "17 SetWorldBounds" |
| `push_back`, `PushBack` | — | 0x1009F450 | push-back of an entity outside the bounds | player-state.md "17 SetWorldBounds" |
| `DRUNK_THRESHOLD` | — | 0x100ADE06 | compare against 2000 | player-state.md "35 SetPlayerDrunkLevel" |
| `DRUNK_WOBBLE_MS` | — | 0x100ADEC9 | elapsed > 200 ms check | player-state.md "35 SetPlayerDrunkLevel" |
| `drunk_value` | — | 0x100ADE31 | `SET_PLAYER_DRUNKENNESS` argument | player-state.md "35 SetPlayerDrunkLevel" |
| `drunk_wobble` | — | 0x100ADED4 | steering wobble on turn speed z | player-state.md "35 SetPlayerDrunkLevel" |
| `drunk_moving` | — | 0x100ADF23 | speed > 0.05 on x or y | player-state.md "35 SetPlayerDrunkLevel" |
| `MsvcRand` | — | 0x100CB096 | statically linked MSVC CRT `rand()` | — |
| `camera_cut` | — | 0x1001A18C | cut argument clamp of RPC 158 / 82 | player-state.md (camera RPCs 82, 158) |
| `valid_position` | — | 0x100B4B60 | coordinate validator | player-state.md |
| `SpecTarget`, `Spectate` | LocalPlayer +0x30F type, +0x310 id, +0x314 applied, +0x30E camera mode | — | spectate state fields | remote-players-sync.md "124 TogglePlayerSpectating", "126", "127" |
| `SPEC_NO_TARGET_POS`, `SPEC_NO_TARGET_LOOK` | — | 0x1000674E | "no target" spectator camera | remote-players-sync.md "124 TogglePlayerSpectating" |
| `SPEC_PED_ABOVE` | — | 0x1000669C | ped parked 20 above the camera | remote-players-sync.md "124 TogglePlayerSpectating" |
| `SPEC_PED_HEALTH` | — | 0x10006688 | ped health forced to 100 | remote-players-sync.md "124 TogglePlayerSpectating" |
| `SPEC_WARP_OUT` | — | 0x1000674E → 0x100AC580 | warp out of the vehicle to (0, 0, 10) | remote-players-sync.md "124 TogglePlayerSpectating" |
| `client_check_in_limits` | — | 0x10011AE3 | offset / count limits of ClientCheck | session-misc.md "103 ClientCheck" |
| `client_check_sum` | 0x1000EA80 | — | XOR-with-0xCC checksum | session-misc.md "103 ClientCheck" |
| `client_check_exe_range` | 0x10011CD8 | — | type 5 address range check | session-misc.md "103 ClientCheck" |
| `CLIENT_CHECK_MODULE_LIMIT` | 0x10011D06 | — | type 0x45 offset limit | session-misc.md "103 ClientCheck" |
| `CLIENT_CHECK_LOAD_POLLS` | 0x10011B9A | — | type 0x47 `HAS_MODEL_LOADED` poll loop | session-misc.md "103 ClientCheck" |
| `client_check_time_arg` | — | 0x10011DC9; clock 0x1002E7A0 | type 0x48 reply argument; QueryPerformanceCounter ms clock | session-misc.md "103 ClientCheck" |
