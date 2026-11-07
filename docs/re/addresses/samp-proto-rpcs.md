# samp-proto `rpcs` — samp.dll addresses

Send sites of the client→server RPCs typed in `crates/samp-proto/src/rpcs/`. A is the 2021 build, B the 2019
build (both 0.3.7-R4). The code cites only the A address (`Mirrors samp.dll 0x….`); every other address is here.
Reliability, priority and channel per RPC are in each item's doc comment and in docs/re/rpc-ids.md.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `player::PlayerRequestClass` (RPC 128) | 0x100058d0 | 0x100058c0 | send site, rel 8 | subsystems/player-state.md "128 PlayerRequestClass (C→S)" |
| `player::PlayerRequestSpawn` (RPC 129) | 0x10003b60 | 0x10003b60 | send site, rel 8 | subsystems/player-state.md "129 PlayerRequestSpawn (C→S)" |
| `player::PlayerSpawn` (RPC 52) | 0x10003c20 | 0x10003c10 | local spawn routine (sends 52), rel 9 | subsystems/player-state.md "Local Spawn routine and 52 PlayerSpawn (C→S)" |
| `player::OnPlayerDeath` (RPC 53) | 0x10005810 | 0x10005800 | send site, rel 9 | subsystems/player-state.md "53 OnPlayerDeath (C→S)" |
| `player::OnPlayerInteriorChange` (RPC 118) | 0x10005970 | 0x10005960 | send site, rel 8 | subsystems/player-state.md "118 OnPlayerInteriorChange (C→S)" |
| `player::OnPlayerGiveTakeDamage` (RPC 115) | 0x100068a0 | 0x10006890 | first send site, rel 9 | subsystems/player-state.md "115 OnPlayerGiveTakeDamage (C→S)" |
| `player::OnPlayerGiveTakeDamage` (RPC 115) | 0x100069b0 | 0x100069a0 | second send site, rel 9 | subsystems/player-state.md "115 OnPlayerGiveTakeDamage (C→S)" |
| `session::PlayerConnect` (RPC 25) | 0x1000ad80 | 0x1000ad70 | send site, rel 8 | subsystems/session-misc.md "25 PlayerConnect (C→S)"; handshake.md §5 |
| `session::ClientCheckResponse` (RPC 103) | 0x10011990 | 0x10011970 | reply send site, rel 9 | subsystems/session-misc.md "103 ClientCheck" |
| `session::OnPlayerRequestScoresAndPings` (RPC 155) | 0x10008f10 | 0x10008f00 | send site, rel 8 | subsystems/session-misc.md "155" |
| `session::OnPlayerClickPlayer` (RPC 23) | 0x1006ec10 | 0x1006ebd0 | send site, rel 9 | subsystems/session-misc.md "23" |
| `session::OnPlayerClickMap` (RPC 119) | 0x10075790 | 0x10075760 | function containing the send, rel 8 | subsystems/session-misc.md "119" |
| `session::OnPlayerCameraTarget` (RPC 168) | 0x10004700 | 0x100046e0 | send site, rel 9 | subsystems/session-misc.md "168" |
| `session::PickedUpDroppedWeapon` (RPC 97) | 0x10013520 | 0x100134f0 | send site (pickup poll), rel 10; never reached in R4 | subsystems/session-misc.md "97" |
| `ui::PlayerRequestChatMessage` (RPC 101) | 0x10005a10 | 0x10005a00 | sender, rel 8 | subsystems/ui-chat-audio.md §101 |
| `ui::PlayerRequestCommandMessage` (RPC 50) | 0x10069900 | 0x100698c0 | sender, rel 8 | subsystems/ui-chat-audio.md §50 |
| `ui::OnPlayerSelectTextDraw` (RPC 83) | 0x100714e0 | 0x100714b0 | sender, rel 9 | subsystems/ui-chat-audio.md §83 |
| `ui::OnPlayerDialogResponse` (RPC 62) | 0x10070690 | 0x10070660 | sender, rel 9 | subsystems/ui-chat-audio.md §62 |
| `ui::OnPlayerSelectedMenuRow` (RPC 132) | 0x10008200 | 0x100081f0 | per-frame menu poll, rel 8 | subsystems/ui-chat-audio.md §132/§140 |
| `ui::OnPlayerExitedMenu` (RPC 140) | 0x10008200 | 0x100081f0 | same menu poll, rel 8 | subsystems/ui-chat-audio.md §132/§140 |
| `vehicle::OnPlayerEnterVehicle` (RPC 26) | 0x10005AD0 | 0x10005AC0 | sender, rel 10 | subsystems/vehicles.md "26 OnPlayerEnterVehicle (C→S)" |
| `vehicle::OnPlayerExitVehicle` (RPC 154) | 0x10005BF0 | 0x10005BE0 | sender, rel 10 | subsystems/vehicles.md "154 OnPlayerExitVehicle (C→S)" |
| `vehicle::SetVehicleDamageStatus` (RPC 106 C→S) | 0x10005DE0 | 0x10005DD0 | sender, rel 8 | subsystems/vehicles.md "106 SetVehicleDamageStatus" |
| `vehicle::VehicleDeath` (RPC 136) | 0x1001F230 | 0x1001F1E0 | sender, rel 10 | subsystems/vehicles.md "136 VehicleDeath" |
| `vehicle::OnPlayerScmEvent` (RPC 96 C→S) | 0x10001CE0 | 0x10001CE0 | sender, rel 10 | subsystems/vehicles.md "96 SCMEvent (C→S)" |
| `world::OnPlayerSelectObject` (RPC 27 C→S) | 0x1006dcd0 | 0x1006dc90 | send site (not a Ghidra function), rel 9 | subsystems/world-entities.md §27; README.md "Gaps" |
| `world::OnPlayerSelectObject` (RPC 27 C→S) | 0x100e5e9c | — | .rdata RPC-id cell passed by that send site | subsystems/world-entities.md §27 |
| `world::OnPlayerEditObject` (RPC 117 C→S) | 0x10072910 | 0x100728e0 | send site, rel 9 | subsystems/world-entities.md §117 |
| `world::OnPlayerEditAttachedObject` (RPC 116 C→S) | 0x10072b20 | 0x10072af0 | send site, rel 9 | subsystems/world-entities.md §116 |
| `world::OnPlayerPickUpPickup` (RPC 131) | 0x10013440 | 0x10013410 | collision-hook send path, rel 9 | subsystems/world-entities.md §131 |
| `world::OnPlayerPickUpPickup::POLL_SEND_PARAMS` (RPC 131) | 0x10013520 | 0x100134f0 (same function as RPC 97) | type-14 pickup poll (every ~6 netgame ticks), rel 10 | subsystems/world-entities.md §131 |
| `world::OnPlayerDamageActor` (RPC 177) | 0x10006ac0 | 0x10006ab0 | send site, rel 9 | subsystems/world-entities.md §177 |
| `world::RequestDff` (RPC 181) | 0x1000d800 | not checked | variable-ID send site, kind 1, rel 9; dormant in R4 | subsystems/session-misc.md §181/182 |
| `world::RequestTxd` (RPC 182) | 0x1000d800 | not checked | variable-ID send site, kind 2, rel 9; dormant in R4 | subsystems/session-misc.md §181/182 |
| `world::FinishDownload` (RPC 184) | 0x1000d920 | 0x1000d910 | send site (writes one 1 bit), rel 9; dormant in R4 | subsystems/session-misc.md §184; world-entities.md "Custom models" |
| `outgoing::tests::MISSING_FROM_TABLE` | 0x1006dcd0, 0x1000d800 | — | the send sites of 27 and 181/182 missing from the rpc-ids.md table | README.md "Gaps" |
