# behaviourclient.rs

Most of these were read in build B only; "—" means not recorded for that build. "Row N" is the row of
docs/re/data/gta-patches-r4.csv.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `CAMERA_MATRIX_POS` | — | 0x1009FF92 | samp's camera object +4 points at `TheCamera` matrix 0xB6F99C | player-state.md (camera RPCs) |
| `GRAVITY`, `set_gravity` | — | 0x100A1410 | global gravity write (row 358) | player-state.md "146 SetPlayerGravity" |
| `LOAD_SHOP`, `PED_SHOP_NAME`, `shop_name` | — | 0x10017E00 → 0x100AE390 / 0x100AE310; shop-name field read at 0x100AE43E | RPC 33 handler → shop-name setter / `LoadShop` caller | player-state.md "33 SetPlayerShopName" |
| `ADD_BIG_MESSAGE` | — | 0x100A0D55 | call to `CMessages::AddBigMessage` | player-state.md "17 SetWorldBounds" |
| `big_message` | — | 0x100A0D10 | CLEAR_PRINTS + AddBigMessage wrapper | player-state.md "17 SetWorldBounds" |
| `GET_UPPERCASE_KEY`, `hash_table` | — | 0x1009CF30 (called from 0x100A091F) → 0x100B4490 | start-up hashing of the animation table | remote-control-onfoot.md §4.1 |
| `Behaviour::hashes` | — | 0x10149190 | samp's table of hashes | remote-control-onfoot.md §4.1 |
| `GET_ANIM_ASSOCIATION` | — | 0x100B4440 | samp's wrapper of `CAnimManager::GetAnimAssociation` | remote-control-onfoot.md §4.1 |
| `ENTITY_RW_OBJECT` | — | 0x100A4220 | clump compare in the animation recorders | remote-control-onfoot.md §4.4 |
| `ENTITY_CHECK_WORD` | — | 0x10011BB7 | ClientCheck type 2 reads entity +0x40 | session-misc.md "103 ClientCheck" |
| `TRAIN_MODELS` | — | 0x1009F086 | train model check in the teleport wrapper | remote-control-vehicles.md §4.2 |
| `teleport_entity` | — | 0x1009F070 | teleport wrapper (`Teleport` virtual, vtable +0x38) | — |
| `TASK_NAMED_ANIM`, `TASK_NAMED_ANIM_2` | — | 0x100AE711 | encoder's task-type check (401, 934) | remote-control-onfoot.md §4.1 |
| `TASK_ANIM_*` | — | 0x100AE600 | reader of `CTaskSimpleRunNamedAnim` fields | remote-control-onfoot.md §4.1 |
| `ANIM_LOAD_POLLS` | — | 0x100AD2B3 | 15 × Sleep(1) poll loop | remote-control-onfoot.md §4.1 |
| `in_vehicle` | — | 0x100ABE80 | ped flags +0x46C bit 8 | — |
| `is_passenger` | — | 0x100ABFD0 | passenger test | — |
| `task_virtual` | — | 0x100B4060 / 0x100B40B0 | GetSubTask / GetTaskType wrappers with the vtable range check | — |
| `find_task` | — | 0x100AE6F0 (encoder) / 0x100ADA50 (remote stop, 401 only) | primary task chain search | remote-control-onfoot.md §4.1 |
| `StaticText` (shop name) | — | 0x1026B260 (32 bytes) | static shop-name buffer | player-state.md "33 SetPlayerShopName" |
| `StaticText` (big message) | — | 0x101501AC (0x200 bytes) | static big-message buffer | — |
| `REC_GROUP`, `REC_ID`, `REC_DELTA` | — | 0x101514D0 / 0x101514D4 / 0x101514D8; kind (1 add / 2 blend) at 0x101514CC | last recorded animation call | remote-control-onfoot.md §4.4 |
| `add_animation_stub`, `blend_animation_stub` | — | — | `AddAnimation` / `BlendAnimation` recorders (rows 17 / 18) | remote-control-onfoot.md §4.4 |
| `uncompress_animation_stub` | 0x100A3E00 | — | NULL-hierarchy guard in `UncompressAnimation`, continues at GTA 0x4D41C5 (row 54) | gta-patches.md "Other" |
| `install` (POINT_CAMERA_AT_CHAR) | — | — | non-player branch for spectating (row 198) | gta-patches.md |
| `Behaviour::drunk` | local ped object +0x2C9 | — | drunk level | player-state.md "35 SetPlayerDrunkLevel" |
| `Behaviour::wobble_tick` | — | 0x1026B284 | last wobble GetTickCount | player-state.md "35 SetPlayerDrunkLevel" |
| `Behaviour::attached` | samp camera object +0 | — | attached object id | player-state.md "81 AttachCameraToObject" |
| `Behaviour::anim` | LocalPlayer +0xFC / +0x100 | — | 207 animation field | remote-players-sync.md "207 PlayerFootSync" |
| `on_init_game` | 0x100109E8 / 0x10010A24 (stunt bonus row 359, gravity row 358) | 0x100109E9 → 0x100088E0 (bounds), then 0x100A1410 (gravity) | InitGame world state | session-misc.md "139 PlayerInit" |
| `interpolate` | — | 0x1001A0A0 → 0x1009D5C0 / 0x1009D630 | RPC 82 handler; "applied" flag at LocalPlayer +0x314 | player-state.md (camera RPCs) |
| `attach_camera` | — | 0x10019FA0 → 0x1009D690 | RPC 81 handler | player-state.md "81" |
| `camera_follow` | — | 0x100A2640 → 0x1009D6E0 | camera-follow before `CCamera::Process` (row 9) | player-state.md "81" |
| `toggle_spectating` | — | 0x100041A0 | RPC 124 | remote-players-sync.md "124" |
| `spectate_player` | — | 0x10004260 | RPC 126 | remote-players-sync.md "126" |
| `spectate_vehicle` | — | 0x10004210 | RPC 127 | remote-players-sync.md "127" |
| `spectator_tick` | 0x10006620 (203 only after a 212) | 0x10006530 | spectator tick | remote-players-sync.md "212" |
| `camera_position` | — | 0x1009D7B0 | RPC 157 camera method | player-state.md (camera RPCs) |
| `camera_look_at` | — | 0x1009D810 | RPC 158 camera method | player-state.md (camera RPCs) |
| `bounds_on_foot` | — | 0x10003F40 (called at 0x10007C2E) | on-foot bounds check | player-state.md "17 SetWorldBounds" |
| `bounds_driver` | — | 0x10003FB0 (called at 0x1000793D; pool check at 0x10003FD3) | driving bounds check | player-state.md "17 SetWorldBounds" |
| `drunk_tick` | — | 0x100ADE00 (called at 0x100077B0) | drunk effects | player-state.md "35 SetPlayerDrunkLevel" |
| `drunk_not_spawned` | — | 0x10007A70 | reset while not spawned | player-state.md "35 SetPlayerDrunkLevel" |
| `anim_tick` | — | 0x100077EF | call of the encoder from the local tick | remote-control-onfoot.md §4.1 |
| `encode_local` | — | 0x100AE7A0 | local animation encoder | remote-control-onfoot.md §4.1 |
| `apply_animation` | — | 0x1001A900 | RPC 86 | player-state.md "86 ApplyPlayerAnimation" |
| `clear_animations` | — | 0x10018530 | RPC 87 | player-state.md "87 ClearPlayerTasks" |
| `forget_remote` | — | 0x100165E0 | remote player constructor | remote-players-sync.md §1 |
| `client_check` | 0x10011A60 | 0x10011A40 | RPC 103 handler | session-misc.md "103 ClientCheck" |
| `model_info` | 0x100A7A80 | — | model-info lookup with bounds | session-misc.md "103 ClientCheck" |
| `collision_checksum` | 0x10011B4C | — | type 0x47 path | session-misc.md "103 ClientCheck" |
| `remote_anim` | — | 0x100167E4 | one-shot apply of a stored 207's animation | remote-control-onfoot.md §4.1 |
| `remote_anim` (parachute) | — | 0x100AE7E0 → 0x100AD630 | parachute object follows PARACHUTE animations | remote-control-onfoot.md §4.2 |
| `REMOTE_ANIMS` | remote +0x1B4 / +0x1C1 | — | remote animation state | remote-control-onfoot.md §4.1 |
| `vector_move` | — | 0x1009D5C0 | camera vector move | player-state.md (camera RPCs) |
| `LOCAL_BOUNDS` | local player +0xF0 | — | bounds while spawned | player-state.md "17 SetWorldBounds" |
| `bounds_jump` | — | 0x100A2AF9; pad write at B+0xA2B70; uses 0x1009F5F0 (outside) and 0x100A0EF0 (GET_AREA_VISIBLE) | local branch of the `CPlayerPed::ProcessControl` hook | remote-control-onfoot.md §2.2 |
| `push_back_entity` | — | 0x1009F450 | push-back on a live entity | player-state.md "17 SetWorldBounds" |
| `play` | — | 0x100AD240 | animation player | player-state.md "86 ApplyPlayerAnimation" |
| `set_stunt_bonus` | 0x100A1580 | — | stunt-bonus dword (row 359) | player-state.md "104 EnableStuntBonusForPlayer" |
| `widescreen` | — | 0x10018A70 | RPC 111 | player-state.md "111 ToggleWidescreen" |
| `fighting_style` | — | 0x100AE0E0 | RPC 89 | player-state.md "89 SetPlayerFightingStyle" |
| `set_velocity` | — | 0x10018800 | RPC 90 | player-state.md "90 SetPlayerVelocity" |
| `set_interior` | — | 0x100199B0 → 0x100AD350 (with refresh) | RPC 156 | player-state.md "Interior" |
| `qpc_ms` | — | 0x1002E7A0 | QueryPerformanceCounter ms clock | session-misc.md "103 ClientCheck" |
