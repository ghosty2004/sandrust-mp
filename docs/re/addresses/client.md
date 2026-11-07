# samp.dll addresses behind `crates/samp-client/src/client.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `CAMERA_FRONT` | 0x101039C8 |  | samp's camera-front pointer (holds 0xB6F32C) | docs/re/subsystems/player-state.md |
| `ZONE_NAMES_DELAY` | 0x10003C20 |  | spawn sets the zone-names start (+0x30a := now + 1000) | docs/re/subsystems/player-state.md, "Local Spawn routine" |
| `OP_SELECT_INTERIOR`, `OP_LINK_CHAR_TO_INTERIOR` |  | 0x100AD350 | interior helper used by the RPC 40 reset | docs/re/subsystems/session-misc.md |
| `OP_EXPLODE_CHAR_HEAD`, `kill_local_ped` |  | 0x100AFFE0 | local ped kill used by the RPC 40 reset (fires out, teleport, health 0, explode head) | docs/re/subsystems/session-misc.md |
| `PED_FIRE`, `VEHICLE_FIRE`, `PED_FLAGS_IN_VEHICLE` |  | 0x100AD100, 0x100AFFFF | fire / in-vehicle reads in the kill helper | docs/re/subsystems/session-misc.md |
| `GTA_PLAYER_IN_FOCUS` |  | 0x10113978 | samp's pointer to `CWorld::PlayerInFocus` | docs/re/subsystems/session-misc.md |
| `DEFAULT_GRAVITY` |  | (settings +0x3d5 +0x16) | server-settings default gravity 0x3C03126F = 0.008 | docs/re/subsystems/session-misc.md |
| `VK_F4` |  | 0x100074B0 | F4 test (`GetAsyncKeyState(0x73)`) in the local tick | docs/re/subsystems/player-state.md |
| `F4_LINE` |  | 0x100E5910 | string of the F4 client line | docs/re/subsystems/player-state.md |
| `F4_LINE` (helper) |  | 0x10068030 | type-4 chat helper | docs/re/subsystems/ui-chat-audio.md |
| `Client::zone_names` | 0x100074C0 |  | local tick: DISPLAY_ZONE_NAMES every frame | docs/re/subsystems/player-state.md |
| `Client::look_offset`, `head_look` | 0x10002FA0 |  | local head look (offset every 1000 ms, LookAt every 200 ms) | docs/re/subsystems/player-state.md |
| `Client::pre_process` (plate pass) |  | 0x100A2741 → 0x1001ED20 | vehicle pool's plate pass before the object process | docs/re/subsystems/world-entities.md, "Per-frame object process" |
| `Client::begin_quit` | 0x100C3EC0 |  | `/quit`: Disconnect(500) in netgame state 5 | docs/phase3/m10-notes.md |
| `Client::quit_teardown` | 0x10009880 |  | netgame destruction before `ExitProcess` | docs/phase3/m10-notes.md |
| `Client::tick` |  | 0x1000B5A0 | netgame main loop: packet handler first | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `Client::tick` (first frame) | 0x100C47D0 |  | first-frame init: GTA clock and HUD clock string off | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `Client::tick` (time of day, preload) | 0x1000B5B0 |  | netgame tick: server time of day, preloads | docs/phase3/trace-notes.md |
| `Client::tick` (spectating unspawned) |  | 0x10007A9A | spectating without being spawned: leave class selection, spectator tick | docs/re/subsystems/player-state.md |
| `Client::tick` (death) | 0x100074C0 |  | local tick: death and respawn | docs/re/subsystems/remote-control-combat.md |
| `Client::tick` (F4) |  | 0x100074B0 | F4 at the end of the local tick | docs/re/subsystems/player-state.md |
| `Client::tick` (connect tick) | 0x10008940 | 0x10008930 | connect tick in state 1 | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `Client::tick` (attempt time) | 0x10008985 |  | tick count read again after RakClient::Connect | docs/phase3/m10-notes.md |
| `Client::tick` (`/dl` labels) |  | 0x10075760 | `/dl` vehicle labels, rebuilt every tick | docs/re/subsystems/ui-rendering.md |
| `Client::tick` (driver-enter gate) | 0x100A3700 |  | driver-enter gate refuses while cuffed or carrying | docs/re/subsystems/remote-control-vehicles.md |
| `Client::on_event` (InitGame) | 0x10010660 |  | RPC 139 InitGame handler: hour, weather, zone names, CJ walk | docs/re/subsystems/session-misc.md, "139 PlayerInit" |
| `Client::on_event` (vehicle-model preload) |  | 0x100A1620 | InitGame vehicle-model preload | docs/re/subsystems/vehicles.md |
| `Client::reset_routine` |  | 0x1000A530 | RPC 40 reset routine | docs/re/subsystems/session-misc.md, "40 PlayerClose" |
| `Client::reset_routine` (weapon skills) | 0x1009CCF0 |  | 11 weapon-skill stats := 999.0 | docs/re/subsystems/remote-control-combat.md |
| `Client::reset_routine` (interior 0) |  | 0x100AD350 | interior helper called with (0, 1) | docs/re/subsystems/session-misc.md |
| `Client::reset_routine` (money) | 0x100A2100 |  | money back to 0 (ADD_SCORE with the negated amount) | docs/re/subsystems/player-state.md |
| `Client::reset_routine` (armour) | 0x100ABDF0 |  | armour := 0 | docs/re/subsystems/player-state.md |
| `Client::reset_routine` (zone names) | 0x100A1560 |  | zone names off (called with 0) | docs/re/subsystems/player-state.md |
| `Client::reset_routine` (colours) | 0x100A72C0 |  | player colour table back to the defaults | docs/re/subsystems/ui-rendering.md |
| `Client::reset_routine` (gravity) |  | 0x100A1410 | gravity write to 0x863984 | docs/re/subsystems/session-misc.md |
| `Client::reset_routine` (gravity page) |  | 0x100AA4D0 | unprotects the gravity constant's page | docs/re/subsystems/gta-patches.md, "Patch helper functions" |
| `Client::reset_routine` (HUD) | 0x100A1460, 0x100A14A0 |  | HUD wanted level 0; HUD clock off | docs/re/subsystems/gta-patches.md, "Runtime patches (summary)" |
| `Client::kill_local_ped` |  | 0x100AFFE0 | local ped kill: fires out, teleport, health 0, explode head | docs/re/subsystems/session-misc.md |
| `Client::on_sync` (bullet) | 0x10009B40 → 0x10016370 |  | bullet sync fires its shot at once | docs/re/subsystems/remote-control-combat.md, "Per-frame order" |
| `Client::on_rpc` (colour table) |  | 0x100A7220 | player colour table | docs/re/subsystems/ui-rendering.md |
| `Client::on_rpc` (PlayerJoin colour) |  | 0x1000FDE2 | PlayerJoin sets the colour for a non-zero colour and an id below 1004 | docs/re/subsystems/remote-players-sync.md |
| `Client::on_rpc` (TogglePlayerSpectating) |  | 0x100041A0 | spectating off with the class-selection flag set → class selection | docs/re/subsystems/player-state.md |
| `Client::on_rpc` (RPC 92 / 150) | 0x10018C50, 0x10018D00 |  | opcode 0x052C / 0x03FD on the local player | docs/re/subsystems/session-misc.md, "92" and "150" |
| `Client::on_rpc` (RPC 60) |  | 0x1000FF10 → 0x100A1760 | GTA millisecond timer := time & 0x3FFFFFFF with the HUD clock off | docs/re/subsystems/player-state.md, "60 SendGameTimeUpdate" |
| `Client::on_rpc` (RPC 48) |  | 0x1001DC70 | stored in the custom-model manager (never created in R4) | docs/re/subsystems/player-state.md, "48 SetPlayerVirtualWorld" |
| `Client::on_rpc` (RPC 12) | 0x100193E4 |  | surfing detached before the teleport | docs/re/subsystems/player-state.md, "12 SetPlayerPosition" |
| `Client::on_rpc` (SetPlayerTime) | 0x1000F420 |  | GTA clock, SET_TIME_OF_DAY, settings | docs/re/subsystems/player-state.md |
| `Client::on_rpc` (SetPlayerColor) |  | 0x100197B0 | local id, or a remote id up to 1004 in the pool | docs/re/subsystems/player-state.md |
| `Client::on_rpc` (SetPlayerArmedWeapon) | 0x10018E50 |  | 0…46 only | docs/re/subsystems/player-state.md |
| `Client::on_rpc` (set-armed-weapon), `give_weapon` | 0x100AC0A0 |  | set-armed-weapon, not forced | docs/re/subsystems/player-state.md |
| `Client::connecting_tick` | 0x100089B0 |  | not-connected scene: weather 1, parked ped, fixed camera | docs/phase3/m2-notes.md |
| `Client::class_selection_tick` (menu flag) | 0x100A0980 |  | reads `FrontEndMenuManager+0x5C` as a dword | docs/re/subsystems/player-state.md |
| `Client::class_selection_tick` | 0x10006100 |  | class-selection tick: HUD off, dialog (+0x28) and chat-input (+0x14e0) tests | docs/re/subsystems/player-state.md |
| `Client::class_selection_tick` (dialog close) | 0x1006F860 |  | dialog close clears the flag at once | docs/re/subsystems/ui-rendering.md |
| `Client::class_selection_tick` (class change sound) | 0x10005FF0, 0x10006080 |  | sound at the ped position, then RPC 128 | docs/re/subsystems/player-state.md |
| `Client::class_selection_tick` (sound) | 0x100A2300 |  | game audio object call (the RPC 16 path) | docs/re/subsystems/ui-chat-audio.md |
| `Client::spawn` (surf) | 0x10003D5E |  | surf state cleared at spawn | docs/phase3/close-notes.md |
| `Client::spawn` (leave class selection) | 0x100A0750 |  | input lock ends 10 frames later (called with 0, 0) | docs/re/subsystems/player-state.md, "Local Spawn routine and 52 PlayerSpawn" |
| `Client::death_tick` (cuffs) | 0x100ACF40 |  | cuffs off (called with 0) | docs/re/subsystems/remote-control-combat.md |
| `Client::on_local_death` | 0x100AC890 |  | killer / reason for RPC 53 | docs/re/subsystems/player-state.md, "53 OnPlayerDeath" |
| `Client::surf_step`, `surf_probe` | 0x10003730 |  | local surf probe and state step | docs/re/subsystems/remote-players-sync.md, "Local surfing" |
| `surf_probe` | 0x100ACA50 |  | vertical line-of-sight probe | docs/re/subsystems/remote-players-sync.md, "Local surfing" |
| `Client::surf_publish` | 0x100A4990 |  | surfing hook that reads this frame's surfaces | docs/re/subsystems/remote-players-sync.md, "Local surfing" |
| `Client::sync_tick` (spectating) |  | 0x100078E9 | spectator tick replaces the on-foot / driver branches | docs/re/subsystems/player-state.md |
| `Client::sync_tick` (head-move flag) |  | 0x10002FBD, 0x100C4EF7 | head look tests Game+0x59; set at boot | docs/re/subsystems/ui-rendering.md |
| `Client::sync_tick` (205 / 204) | 0x100078A8, 0x100078C9 |  | 205 and 204 before the per-state packets | docs/re/subsystems/remote-players-sync.md |
| `Client::sync_tick` (drive-by) | 0x10007B68 |  | local passenger drive-by before the 211 | docs/re/subsystems/remote-control-combat.md |
| `Client::sync_tick` (driving bounds) |  | 0x1000793D | world bounds on the vehicle | docs/re/subsystems/player-state.md |
| `Client::sync_tick` (RemoveWeaponWhenEnteringVehicle) |  | 0x10007903 … 0x1000792F | every driving frame unless passenger or train carriage | docs/re/subsystems/remote-control-vehicles.md |
| `Client::sync_tick` (carriage test) |  | 0x100ABFD0 | 569/570 train carriage test | docs/phase3/feata-notes.md |
| `Client::sync_tick` (on-foot bounds) |  | 0x10003F40 | on-foot world-bounds check | docs/re/subsystems/player-state.md |
| `Client::sync_tick` (207 builder) | 0x10004F00 |  | 207 builder reads the weapon with GET_CURRENT_CHAR_WEAPON | docs/re/subsystems/remote-players-sync.md |
| `Client::sync_tick` (special action) | 0x10003410 |  | special-action order: jetpack, dance, hands up, phone, piss, duck | docs/re/subsystems/player-state.md |
| `Client::sync_tick` (203) | 0x10007C9A |  | 203 after the 207 | docs/re/subsystems/remote-control-combat.md |
| `Client::stats_tick` |  | 0x100078A3 | 205 every > 1000 ms | docs/re/subsystems/remote-players-sync.md |
| `Client::stats_tick` (drunk) |  | 0x100ADFB0 | drunk level read | docs/re/subsystems/player-state.md |
| `Client::local_frame` | 0x100074C0 |  | local tick (spawned branch) | docs/phase3/trace-notes.md |
| `Client::local_frame` (`/ctd`) |  | 0x10004830 | `/ctd` labels | docs/re/subsystems/ui-rendering.md |
| `Client::local_frame` (hands up + Triangle) | 0x100AC7D0 |  | control on again (called with 1) | docs/re/subsystems/player-state.md, "15 TogglePlayerControllable" |
| `Client::local_frame` (piss / item) |  | 0x1000772C … 0x100077AB | Triangle or a vehicle ends them | docs/phase3/featb-notes.md |
| `Client::local_frame` (drunk) |  | 0x100ADE00 | drunk handling | docs/re/subsystems/player-state.md |
| `Client::local_frame` (animation) |  | 0x100AE7A0 | 207 animation encoder | docs/re/subsystems/remote-players-sync.md |
| `Client::local_frame` (weapons) | 0x10006290 |  | weapons-update check | docs/re/subsystems/remote-control-combat.md |
| `Client::local_frame` (interior) | 0x100A0F20, 0x10005970 |  | interior change → RPC 118 | docs/re/subsystems/player-state.md |
| `Client::local_frame` (camera target) |  | 0x100046E0 | camera target, after the interior check | docs/re/subsystems/session-misc.md, "168 OnPlayerCameraTarget" |
| `Client::crime_report` |  | 0x10019000 → 0x100A17A0 | RPC 112 handler | docs/re/subsystems/player-state.md, "112 PlayCrimeReport" |
| `Client::head_look` | 0x10002FA0 |  | local head look | docs/re/subsystems/player-state.md |
| `Client::current_weapon` | 0x100ABC90 |  | GET_CURRENT_CHAR_WEAPON on the local ped | docs/re/subsystems/player-state.md |
| `Client::hands_up` | 0x100ACBA0 |  | primary task is the hands-up task | docs/re/subsystems/player-state.md |
| `Client::on_set_player_skin` | 0x10019190 |  | RPC 153 handler | docs/re/subsystems/player-state.md, "153 SetPlayerSkin" |
| `Client::on_special_action` | 0x100030F0 | 0x100030F0 | RPC 88 (LocalPlayer method) | docs/re/subsystems/player-state.md, "88 SetPlayerSpecialAction" |
| `Client::set_phone` | 0x100AE0B0 |  | phone flag + TASK_USE_MOBILE_PHONE | docs/re/subsystems/player-state.md |
| `Client::set_controllable` | 0x100AC7D0 |  | RPC 15 ped method | docs/re/subsystems/player-state.md, "15 TogglePlayerControllable" |
| `Client::camera_behind_player` | 0x1009D7A0 |  | RPC 162 camera method | docs/re/subsystems/player-state.md, "162 SetPlayerCameraBehindPlayer" |
| `Client::clear_weapons` | 0x100AC050 |  | ClearWeapons with PlayerInFocus only | docs/re/subsystems/player-state.md |
| `Client::give_weapon` | 0x100AF7D0 |  | GiveWeapon in the slot context, then set-armed-weapon | docs/re/subsystems/player-state.md, "22 GivePlayerWeapon" |
| `ped_in_world` | 0x1009F430 |  | "ped is live" test | docs/re/subsystems/player-state.md |
| `set_hud` | 0x100A1DF0 |  | HUD on/off (menu flag + script radar flag) | docs/re/subsystems/player-state.md |
| `set_ped_skin` | 0x100AFF90 |  | ped skin change (invalid model → 0) | docs/re/subsystems/player-state.md, "153 SetPlayerSkin" |
| `set_ped_skin` (IK task) | 0x100AE060 |  | IK secondary task deleted first | docs/re/subsystems/remote-control-onfoot.md, "Skin / model change" |
| `set_ped_skin` (debug line) |  | 0x1009EF80 | "Model … wouldn't load in time!" debug chat line | docs/phase3/featb-notes.md |
| `CAMERA_POS` |  | 0x1009F1C0 | samp's camera distance | docs/re/subsystems/session-misc.md, "168 OnPlayerCameraTarget" |
| `camera_targets` |  | 0x10004420 | camera-target search: object, vehicle, player, actor | docs/re/subsystems/session-misc.md, "168 OnPlayerCameraTarget" |
| `camera_targets` (own vehicle) |  | 0x100B7430 | the local ped's own vehicle is skipped | docs/re/subsystems/session-misc.md, "168 OnPlayerCameraTarget" |
