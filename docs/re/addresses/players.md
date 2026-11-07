# players.rs

"Row N" is the row of docs/re/data/gta-patches-r4.csv.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Players::frame` (per-frame apply) | 0x100166B0 | — | remote player update (netgame tick → player pool → per remote) | remote-players-sync.md §2.1 |
| `PREVIEW_PED_SLOT`, `create_preview_ped` | 0x100B0D20 (ped constructor); preview call at 0x1006C1A0 / 0x1006C1A5 (slot 0xD0) | — | model-preview ped | ui-rendering.md |
| `create_preview_ped` (swap entry) | 0x100B4360 | — | slot swap entry bind | remote-control-combat.md §1.2 |
| `destroy_preview_ped` | 0x100B0FE0 | — | ped wrapper destructor | remote-control-onfoot.md §1.5 |
| `look_at` | 0x10002FA0 (local player, 2000 ms) | — | head look via `g_ikChainMan.LookAt` | remote-control-onfoot.md §5.2 |
| `PLAYER_PED_DATA` | — | — | offset read from `ProcessAnimGroups` at GTA 0x609BFB | — |
| `marker_mode` | netgame settings +0x30 | — | InitGame player-marker mode | session-misc.md "139 PlayerInit" |
| `on_markers` | — | 0x10009F00; blip removal 0x100144B0(0); coordinate blip 0x100144F0 | 208 PlayerMarkersSync handler | remote-players-sync.md "208 PlayerMarkersSync" |
| `spawn_blips` | — | 0x10017164…0x100171A1; ped blip 0x100AF740 | spawn part of the stream-in | remote-players-sync.md "32 PlayerStreamIn" |
| `reset_marker` | — | 0x10015F50 (reset), 0x10016610 (destructor) | marker blip removal | remote-players-sync.md "208 PlayerMarkersSync" |
| RPC 32 NPC proofs (in the stream-in) | — | 0x10010CC9…0x10010CEC → 0x100ABD10 | `SET_CHAR_PROOFS(0, 1, 1, 0, 0)` for NPCs | remote-players-sync.md "32 PlayerStreamIn" |
| spawn colour (in the stream-in) | — | 0x1001714E | colour table write | remote-players-sync.md "32 PlayerStreamIn" |
| `despawn_all` | — | 0x1000A540 (loop) | RPC 40 reset | session-misc.md "40 PlayerClose" |
| `create_ped` (spawn failed) | — | 0x10010B9C | "Warning: Couldn't spawn player(%d)" | remote-players-sync.md "32 PlayerStreamIn" |
| `destroy_ped` (parachute / item) | — | 0x100B0FB0 | ped destruction | remote-control-onfoot.md §1.5 |
| `destroy_ped` (anim-block field) | — | — | `m_pPlayerData` +0x4C cleared; `ProcessAnimGroups` patch row 167 | remote-control-onfoot.md §1.5 |
| one-shot weapon / ped state | 0x100166B0 | — | in the per-frame apply | remote-control-onfoot.md §4.3 |
| synced animation | — | 0x100167E4 | one-shot animation apply | remote-control-onfoot.md §4.1 |
| `special_actions` | 0x10014950 | — | special-action pass | remote-control-onfoot.md §4.2 |
| `special_actions` (rest of the pass) | — | 0x10014A38… | piss / items / parachute | remote-control-onfoot.md §4.2 |
| `special_actions` (phone) | 0x100AE0B0 | — | phone flag + `TASK_USE_MOBILE_PHONE` | remote-control-onfoot.md §4.2 |
| `set_skill` | 0x1000F5E0 | — | RPC 34 for a remote player | player-state.md "34 SetPlayerSkillLevel" |
| `arm_shooter` | 0x10016370 (step 3) | — | 206 apply, weapon step | remote-control-combat.md §3.3 |
| `kill` | 0x10017570 → 0x100B0010 | — | RPC 166 → ped helper | remote-control-combat.md §7 |
| `handle_faults` | 0x100A2800 | — | "Removed player %u due to error." | remote-control-onfoot.md §2.2 |
| `add_coord_blip` | — | 0x100A0DA0 | coordinate blip | remote-players-sync.md "208 PlayerMarkersSync" |
| `current_weapon` | 0x100ABC90 | — | `GET_CURRENT_CHAR_WEAPON` | — |
| `with_focus` | 0x100AC0A0 / 0x100AF7D0 | — | weapon helpers under the slot's context | remote-control-combat.md §1.2 |
| `set_weapon` | 0x100AC0A0 | — | SET_CURRENT_CHAR_WEAPON under the slot's context | remote-control-combat.md §4 |
| `apply_weapon` | 0x100166B0 → 0x100AF7D0 | — | one-shot weapon check | remote-control-onfoot.md §4.3 |
| `JETPACK_DELETING_DTOR` | 0x100ACDA0 | — | where samp calls it | remote-control-combat.md §7 |
| `teleport_in_place` | 0x100ADB60 | — | `Teleport` virtual to own position | — |
| player markers colour index | — | — | radar colour hook, row 51 | docs/phase3/fixb-notes.md |
