# SA:MP 0.3.7-R4 client — RPC IDs (clean-room spec notes)

Targets: A = samp.dll fv0.3.7.4 (2021-05-07, sha256 0b0e409c…9088); B = samp.dll fv0.3.7.3 (2019-11-21, sha256 15db80c5…12ab).
Evidence: REA `decompile_all` A `ev_effc083a09f1a2cc16801e5554196309c5e712e82f429aa52b6883bedc717efa`, B `ev_51b698f5e4081f48bfa501de67cae66637707dc376a77092e934db90df5fe724`; RPC ID values read from the 4-byte .rdata cells whose addresses are passed to the registration/send calls (file bytes, VA = file offset + 0x10000000 for these sections).

## Encoding and dispatch (see handshake.md §6 for details)

* RPC message on the wire (inside a reliability-layer message): optional `0x28` + u32 timestamp, then `0x14`, then **RPC ID as 1 byte** (8 bits, not byte-aligned), then payload bit-length as RakNet "compressed" u32, then the payload bits (not aligned).
* Each RPC ID is stored in .rdata as a 32-bit int; registration passes a pointer to it plus a handler function (RakClient vtable slot +0x58). Sending uses vtable slot +0x64 with (pointer to ID, bitstream, priority, reliability, ordering channel, shift-timestamp flag).
* Incoming dispatch (A 0x1003ade0 / B 0x1003ad90): reads ID byte, maps it to a registered entry, reads compressed bit length, copies that many bits into a buffer and calls the handler with {data pointer, bit count, sender}. Handlers rebuild a bitstream of `(bits/8)+1` bytes.
* The ID set and registration order are **identical in A and B** (141 registrations, same order); every handler pair has the same sequence of bitstream read calls (struct-offset differences only). A handlers are at B address + 0x20…0x50.

## Server→client RPCs registered by the client (141)

"core" = registered by A 0x10012450 (B 0x10012420), 46 entries; "script" = registered by A 0x1001e130 (B 0x1001e0e0), 95 entries. The grouping label is mine (inferred from which function registers them), not an original name. Purposes are "unknown" unless a string or observed behavior supports them; GUESS marks weak evidence. The purpose column now gives the open.mp struct name for each ID as established (layout checked against the client) by the subsystem specs in `subsystems/`; IDs that open.mp does not define are marked "no open.mp struct". See `README.md` for the ID → spec-file map.

| ID (dec) | hex | A handler (2021) | B handler (2019) | group | purpose |
|---|---|---|---|---|---|
| 11 | 0x0b | 0x1001dfd0 | 0x1001df80 | script | open.mp: SetPlayerName |
| 12 | 0x0c | 0x10019320 | 0x100192d0 | script | open.mp: SetPlayerPosition |
| 13 | 0x0d | 0x10019440 | 0x100193f0 | script | open.mp: SetPlayerPositionFindZ |
| 14 | 0x0e | 0x10019550 | 0x10019500 | script | open.mp: SetPlayerHealth |
| 15 | 0x0f | 0x1001a290 | 0x1001a240 | script | open.mp: TogglePlayerControllable |
| 16 | 0x10 | 0x1001a330 | 0x1001a2e0 | script | open.mp: PlayerPlaySound |
| 17 | 0x11 | 0x1001a410 | 0x1001a3c0 | script | open.mp: SetWorldBounds |
| 18 | 0x12 | 0x1001a500 | 0x1001a4b0 | script | open.mp: GivePlayerMoney |
| 19 | 0x13 | 0x1001a5a0 | 0x1001a550 | script | open.mp: SetPlayerFacingAngle |
| 20 | 0x14 | 0x10018090 | 0x10018040 | script | open.mp: ResetPlayerMoney |
| 21 | 0x15 | 0x100180a0 | 0x10018050 | script | open.mp: ResetPlayerWeapons |
| 22 | 0x16 | 0x1001a640 | 0x1001a5f0 | script | open.mp: GivePlayerWeapon |
| 24 | 0x18 | 0x100112f0 | 0x100112d0 | core | open.mp: SetVehicleParams |
| 26 | 0x1a | 0x100115d0 | 0x100115b0 | core | open.mp: EnterVehicle |
| 27 | 0x1b | 0x1000e8b0 | 0x1000e880 | core | open.mp: PlayerBeginObjectSelect |
| 28 | 0x1c | 0x1000e970 | 0x1000e940 | core | open.mp: PlayerCancelObjectEdit |
| 29 | 0x1d | 0x1000f420 | 0x1000f3f0 | core | open.mp: SetPlayerTime |
| 30 | 0x1e | 0x1000f500 | 0x1000f4d0 | core | open.mp: TogglePlayerClock |
| 32 | 0x20 | 0x10010b00 | 0x10010ae0 | core | open.mp: PlayerStreamIn |
| 33 | 0x21 | 0x10017e50 | 0x10017e00 | script | open.mp: SetPlayerShopName |
| 34 | 0x22 | 0x1000f5e0 | 0x1000f5b0 | core | open.mp: SetPlayerSkillLevel |
| 35 | 0x23 | 0x10018db0 | 0x10018d60 | script | open.mp: SetPlayerDrunkLevel |
| 36 | 0x24 | 0x1000f720 | 0x1000f6f0 | core | open.mp: PlayerShowTextLabel |
| 37 | 0x25 | 0x1000e5a0 | 0x1000e570 | core | open.mp: DisableCheckpoint |
| 38 | 0x26 | 0x10010280 | 0x10010260 | core | open.mp: SetRaceCheckpoint |
| 39 | 0x27 | 0x1000e5b0 | 0x1000e580 | core | open.mp: DisableRaceCheckpoint |
| 40 | 0x28 | 0x1000e650 | 0x1000e620 | core | open.mp: PlayerClose |
| 41 | 0x29 | 0x1001d3c0 | 0x1001d370 | script | open.mp: PlayAudioStreamForPlayer |
| 42 | 0x2a | 0x100180f0 | 0x100180a0 | script | open.mp: StopAudioStreamForPlayer |
| 43 | 0x2b | 0x1001d530 | 0x1001d4e0 | script | open.mp: RemoveBuildingForPlayer |
| 44 | 0x2c | 0x1001b340 | 0x1001b2f0 | script | open.mp: CreateObject |
| 45 | 0x2d | 0x1001ba10 | 0x1001b9c0 | script | open.mp: SetObjectPosition |
| 46 | 0x2e | 0x1001bb20 | 0x1001bad0 | script | open.mp: SetObjectRotation |
| 47 | 0x2f | 0x1001bc20 | 0x1001bbd0 | script | open.mp: DestroyObject |
| 48 | 0x30 | 0x1001dcc0 | 0x1001dc70 | script | open.mp: SetPlayerVirtualWorld |
| 55 | 0x37 | 0x1001dd70 | 0x1001dd20 | script | open.mp: SendDeathMessage |
| 56 | 0x38 | 0x1001a790 | 0x1001a740 | script | open.mp: SetPlayerMapIcon |
| 57 | 0x39 | 0x1001c5c0 | 0x1001c570 | script | open.mp: RemoveVehicleComponent |
| 58 | 0x3a | 0x1000f8d0 | 0x1000f8a0 | core | open.mp: PlayerHideTextLabel |
| 59 | 0x3b | 0x1000f990 | 0x1000f960 | core | open.mp: SetPlayerChatBubble |
| 60 | 0x3c | 0x1000ff30 | 0x1000ff10 | core | open.mp: SendGameTimeUpdate |
| 61 | 0x3d | 0x1000faf0 | 0x1000fad0 | core | open.mp: ShowDialog |
| 63 | 0x3f | 0x1000f140 | 0x1000f110 | core | open.mp: PlayerDestroyPickup |
| 64 | 0x40 | 0x1001a700 | 0x1001a6b0 | script | no open.mp struct; reads u16, no effect (see subsystems/session-misc.md) |
| 65 | 0x41 | 0x10019f30 | 0x10019ee0 | script | open.mp: LinkVehicleToInterior |
| 66 | 0x42 | 0x1001ab60 | 0x1001ab10 | script | open.mp: SetPlayerArmour |
| 67 | 0x43 | 0x10018e50 | 0x10018e00 | script | open.mp: SetPlayerArmedWeapon |
| 68 | 0x44 | 0x10017f50 | 0x10017f00 | script | open.mp: SetSpawnInfo |
| 69 | 0x45 | 0x10019710 | 0x100196c0 | script | open.mp: SetPlayerTeam |
| 70 | 0x46 | 0x10019600 | 0x100195b0 | script | open.mp: PutPlayerInVehicle |
| 71 | 0x47 | 0x10017ff0 | 0x10017fa0 | script | open.mp: RemovePlayerFromVehicle |
| 72 | 0x48 | 0x10019800 | 0x100197b0 | script | open.mp: SetPlayerColor |
| 73 | 0x49 | 0x100198f0 | 0x100198a0 | script | open.mp: SendGameText |
| 74 | 0x4a | 0x100180d0 | 0x10018080 | script | open.mp: ForcePlayerClassSelection |
| 75 | 0x4b | 0x1001c6a0 | 0x1001c650 | script | open.mp: AttachObjectToPlayer |
| 76 | 0x4c | 0x1001c870 | 0x1001c820 | script | open.mp: PlayerInitMenu |
| 77 | 0x4d | 0x1001cb90 | 0x1001cb40 | script | open.mp: PlayerShowMenu |
| 78 | 0x4e | 0x1001cc40 | 0x1001cbf0 | script | open.mp: PlayerHideMenu |
| 79 | 0x4f | 0x1001bd10 | 0x1001bcc0 | script | open.mp: CreateExplosion |
| 80 | 0x50 | 0x1001be20 | 0x1001bdd0 | script | open.mp: ShowPlayerNameTagForPlayer |
| 81 | 0x51 | 0x10019ff0 | 0x10019fa0 | script | open.mp: AttachCameraToObject |
| 82 | 0x52 | 0x1001a0f0 | 0x1001a0a0 | script | open.mp: InterpolateCamera |
| 83 | 0x53 | 0x1001d650 | 0x1001d600 | script | open.mp: PlayerBeginTextDrawSelect |
| 84 | 0x54 | 0x1001b6a0 | 0x1001b650 | script | open.mp: SetPlayerObjectMaterial |
| 85 | 0x55 | 0x1001d310 | 0x1001d2c0 | script | open.mp: StopFlashGangZone |
| 86 | 0x56 | 0x1001a950 | 0x1001a900 | script | open.mp: ApplyPlayerAnimation |
| 87 | 0x57 | 0x10018580 | 0x10018530 | script | open.mp: ClearPlayerTasks |
| 88 | 0x58 | 0x10018690 | 0x10018640 | script | open.mp: SetPlayerSpecialAction |
| 89 | 0x59 | 0x10018740 | 0x100186f0 | script | open.mp: SetPlayerFightingStyle |
| 90 | 0x5a | 0x10018850 | 0x10018800 | script | open.mp: SetPlayerVelocity |
| 91 | 0x5b | 0x10018950 | 0x10018900 | script | open.mp: SetVehicleVelocity |
| 92 | 0x5c | 0x10018c50 | 0x10018c00 | script | no open.mp struct; u32 value → GTA opcode 0x052C on local player (see subsystems/session-misc.md) |
| 93 | 0x5d | 0x1000ef90 | 0x1000ef60 | core | open.mp: SendClientMessage |
| 94 | 0x5e | 0x1000eef0 | 0x1000eec0 | core | open.mp: SetPlayerWorldTime |
| 95 | 0x5f | 0x1000f080 | 0x1000f050 | core | open.mp: PlayerCreatePickup |
| 96 | 0x60 | 0x1000f280 | 0x1000f250 | core | open.mp: SCMEvent |
| 98 | 0x62 | 0x10018b70 | 0x10018b20 | script | no open.mp struct; vehicle tyre status (u16 vehicle, u8 tyre bits) (see subsystems/session-misc.md) |
| 99 | 0x63 | 0x1001bf00 | 0x1001beb0 | script | open.mp: MoveObject |
| 101 | 0x65 | 0x100122f0 | 0x100122d0 | core | open.mp: PlayerChatMessage |
| 102 | 0x66 | 0x1000e5c0 | 0x1000e590 | core | no open.mp struct; stores 296-byte server net-stats blob (see subsystems/session-misc.md) |
| 103 | 0x67 | 0x10011a60 | 0x10011a40 | core | open.mp: ClientCheck (request) |
| 104 | 0x68 | 0x10017d50 | 0x10017d00 | script | open.mp: EnableStuntBonusForPlayer |
| 105 | 0x69 | 0x1001cf70 | 0x1001cf20 | script | open.mp: PlayerTextDrawSetString |
| 106 | 0x6a | 0x100111c0 | 0x100111a0 | core | open.mp: SetVehicleDamageStatus |
| 107 | 0x6b | 0x10010170 | 0x10010150 | core | open.mp: SetCheckpoint |
| 108 | 0x6c | 0x1001d080 | 0x1001d030 | script | open.mp: ShowGangZone |
| 111 | 0x6f | 0x10018ac0 | 0x10018a70 | script | open.mp: ToggleWidescreen |
| 112 | 0x70 | 0x10019050 | 0x10019000 | script | open.mp: PlayCrimeReport |
| 113 | 0x71 | 0x10018f00 | 0x10018eb0 | script | open.mp: SetPlayerAttachedObject |
| 116 | 0x74 | 0x100117e0 | 0x100117c0 | core | open.mp: PlayerBeginAttachedObjectEdit |
| 117 | 0x75 | 0x100118a0 | 0x10011880 | core | open.mp: PlayerBeginObjectEdit |
| 120 | 0x78 | 0x1001d1a0 | 0x1001d150 | script | open.mp: HideGangZone |
| 121 | 0x79 | 0x1001d250 | 0x1001d200 | script | open.mp: FlashGangZone |
| 122 | 0x7a | 0x1001c0b0 | 0x1001c060 | script | open.mp: StopObject |
| 123 | 0x7b | 0x1001c230 | 0x1001c1e0 | script | open.mp: SetVehiclePlate |
| 124 | 0x7c | 0x1001c350 | 0x1001c300 | script | open.mp: TogglePlayerSpectating |
| 125 | 0x7d | 0x100180c0 | 0x10018070 | script | no open.mp struct; no-op handler (see subsystems/session-misc.md) |
| 126 | 0x7e | 0x1001c400 | 0x1001c3b0 | script | open.mp: PlayerSpectatePlayer |
| 127 | 0x7f | 0x1001c4e0 | 0x1001c490 | script | open.mp: PlayerSpectateVehicle |
| 128 | 0x80 | 0x1000ffd0 | 0x1000ffb0 | core | open.mp: PlayerRequestClassResponse |
| 129 | 0x81 | 0x100100a0 | 0x10010080 | core | open.mp: PlayerRequestSpawnResponse / ImmediatelySpawnPlayer |
| 130 | 0x82 | 0x10010540 | 0x10010520 | core | no open.mp struct; OBSERVED: connection rejected; reads u8 reason (1 incorrect version, 2 bad nickname, 3 bad mod, 4 no player slot), then disconnects after 500 ms |
| 133 | 0x85 | 0x1001ccf0 | 0x1001cca0 | script | open.mp: SetPlayerWantedLevel |
| 134 | 0x86 | 0x1001cd90 | 0x1001cd40 | script | open.mp: PlayerShowTextDraw |
| 135 | 0x87 | 0x1001cec0 | 0x1001ce70 | script | open.mp: PlayerHideTextDraw |
| 137 | 0x89 | 0x1000fce0 | 0x1000fcc0 | core | open.mp: PlayerJoin |
| 138 | 0x8a | 0x1000fe70 | 0x1000fe50 | core | open.mp: PlayerQuit |
| 139 | 0x8b | 0x10010660 | 0x10010640 | core | open.mp: PlayerInit; OBSERVED: InitGame (prints "Connected to {B9C9BF}%.64s", sets netgame state 5); field order in handshake.md section 5 |
| 144 | 0x90 | 0x1001a8b0 | 0x1001a860 | script | open.mp: RemovePlayerMapIcon |
| 145 | 0x91 | 0x1001ac10 | 0x1001abc0 | script | open.mp: SetPlayerAmmo |
| 146 | 0x92 | 0x1001acd0 | 0x1001ac80 | script | open.mp: SetPlayerGravity |
| 147 | 0x93 | 0x1001ad70 | 0x1001ad20 | script | open.mp: SetVehicleHealth |
| 148 | 0x94 | 0x1001ae50 | 0x1001ae00 | script | open.mp: AttachTrailer |
| 149 | 0x95 | 0x1001af90 | 0x1001af40 | script | open.mp: DetachTrailer |
| 150 | 0x96 | 0x10018d00 | 0x10018cb0 | script | no open.mp struct; u32 value → GTA opcode 0x03FD on local player (see subsystems/session-misc.md) |
| 151 | 0x97 | 0x1000f1e0 | 0x1000f1b0 | core | no open.mp struct; u8 owner; destroys dropped-weapon pickups (dead path) (see subsystems/session-misc.md) |
| 152 | 0x98 | 0x1000f370 | 0x1000f340 | core | open.mp: SetPlayerWeather |
| 153 | 0x99 | 0x10019190 | 0x10019140 | script | open.mp: SetPlayerSkin |
| 154 | 0x9a | 0x100116f0 | 0x100116d0 | core | open.mp: ExitVehicle |
| 155 | 0x9b | 0x100103e0 | 0x100103c0 | core | open.mp: SendPlayerScoresAndPings |
| 156 | 0x9c | 0x10019a00 | 0x100199b0 | script | open.mp: SetPlayerInterior |
| 157 | 0x9d | 0x10019aa0 | 0x10019a50 | script | open.mp: SetPlayerCameraPosition |
| 158 | 0x9e | 0x10019b70 | 0x10019b20 | script | open.mp: SetPlayerCameraLookAt |
| 159 | 0x9f | 0x10019c70 | 0x10019c20 | script | open.mp: SetVehiclePosition |
| 160 | 0xa0 | 0x10019d80 | 0x10019d30 | script | open.mp: SetVehicleZAngle |
| 161 | 0xa1 | 0x10019e60 | 0x10019e10 | script | no open.mp struct; u16 vehicle, u8 objective, u8 doors locked (see subsystems/session-misc.md) |
| 162 | 0xa2 | 0x10018080 | 0x10018030 | script | open.mp: SetPlayerCameraBehindPlayer |
| 163 | 0xa3 | 0x10010e00 | 0x10010de0 | core | open.mp: PlayerStreamOut |
| 164 | 0xa4 | 0x1000e670 | 0x1000e640 | core | open.mp: StreamInVehicle |
| 165 | 0xa5 | 0x10010ee0 | 0x10010ec0 | core | open.mp: StreamOutVehicle |
| 166 | 0xa6 | 0x10010d40 | 0x10010d20 | core | open.mp: PlayerDeath |
| 167 | 0xa7 | 0x10017de0 | 0x10017d90 | script | open.mp: DisableRemoteVehicleCollisions |
| 169 | 0xa9 | 0x1001c170 | 0x1001c120 | script | no open.mp struct; u16 object id excluded from camera collision (see subsystems/session-misc.md, world-entities.md) |
| 170 | 0xaa | 0x1000ea20 | 0x1000e9f0 | core | open.mp: SetPlayerCameraTargeting |
| 171 | 0xab | 0x1000eab0 | 0x1000ea80 | core | open.mp: ShowActorForPlayer |
| 172 | 0xac | 0x10011e00 | 0x10011de0 | core | open.mp: HideActorForPlayer |
| 173 | 0xad | 0x1001d750 | 0x1001d700 | script | open.mp: ApplyActorAnimationForPlayer |
| 174 | 0xae | 0x1001d930 | 0x1001d8e0 | script | open.mp: ClearActorAnimationsForPlayer |
| 175 | 0xaf | 0x1001d9f0 | 0x1001d9a0 | script | open.mp: SetActorFacingAngleForPlayer |
| 176 | 0xb0 | 0x1001dad0 | 0x1001da80 | script | open.mp: SetActorPosForPlayer |
| 178 | 0xb2 | 0x1001dbe0 | 0x1001db90 | script | open.mp: SetActorHealthForPlayer |

## Client→server RPCs sent by the client (34 call sites, 27 distinct IDs)

Found as calls through RakClient vtable +0x64 whose first argument is the address of an .rdata ID cell. Priority argument is 1 at every site; reliability values use the shifted enum (8 = RELIABLE, 9 = RELIABLE_ORDERED, 10 = RELIABLE_SEQUENCED; see handshake.md §3); ordering channel 0; shift-timestamp 0. One further send site (A 0x1000d800; B counterpart not checked) passes a variable ID and is not resolved. Identical set in B.

| ID | A sender fn | B sender fn | reliability | purpose |
|---|---|---|---|---|
| 23 | 0x1006ec10 | 0x1006ebd0 | 9 | open.mp: OnPlayerClickPlayer |
| 25 | 0x1000ad80 | 0x1000ad70 | 8 | open.mp: PlayerConnect; OBSERVED: ClientJoin-style join request (handshake.md §5) |
| 26 | 0x10005ad0 | 0x10005ac0 | 10 | open.mp: OnPlayerEnterVehicle |
| 50 | 0x10069900 | 0x100698c0 | 8 | open.mp: PlayerRequestCommandMessage |
| 52 | 0x10003c20 | 0x10003c10 | 9 | open.mp: PlayerSpawn |
| 53 | 0x10005810 | 0x10005800 | 9 | open.mp: OnPlayerDeath |
| 62 | 0x10070690 | 0x10070660 | 9 | open.mp: OnPlayerDialogResponse |
| 83 | 0x100714e0 | 0x100714b0 | 9 | open.mp: OnPlayerSelectTextDraw |
| 96 | 0x10001ce0 | 0x10001ce0 | 10 | open.mp: SCMEvent |
| 97 | 0x10013520 | 0x100134f0 | 10 | no open.mp struct; u16 owner; picked up dropped weapon (path never reached) (see subsystems/session-misc.md) |
| 101 | 0x10005a10 | 0x10005a00 | 8 | open.mp: PlayerRequestChatMessage |
| 103 | 0x10011990 | 0x10011970 | 9 | open.mp: ClientCheck (reply) |
| 106 | 0x10005de0 | 0x10005dd0 | 8 | open.mp: SetVehicleDamageStatus |
| 115 | 0x100068a0, 0x100069b0 | 0x10006890/0x100069a0 | 9 | open.mp: OnPlayerGiveTakeDamage |
| 116 | 0x10072b20 | 0x10072af0 | 9 | open.mp: OnPlayerEditAttachedObject |
| 117 | 0x10072910 | 0x100728e0 | 9 | open.mp: OnPlayerEditObject |
| 118 | 0x10005970 | 0x10005960 | 8 | open.mp: OnPlayerInteriorChange |
| 119 | 0x10075790 | 0x10075760 | 8 | open.mp: OnPlayerClickMap |
| 128 | 0x100058d0 | 0x100058c0 | 8 | open.mp: PlayerRequestClass |
| 129 | 0x10003b60 | 0x10003b60 | 8 | open.mp: PlayerRequestSpawn |
| 131 | 0x10013440, 0x10013520 | 0x10013410/0x100134f0 | 10/9 | open.mp: OnPlayerPickUpPickup |
| 132 | 0x10008200 | 0x100081f0 | 8 | open.mp: OnPlayerSelectedMenuRow |
| 136 | 0x1001f230 | 0x1001f1e0 | 10 | open.mp: VehicleDeath |
| 140 | 0x10008200 | 0x100081f0 | 8 | open.mp: OnPlayerExitedMenu |
| 154 | 0x10005bf0 | 0x10005be0 | 10 | open.mp: OnPlayerExitVehicle |
| 155 | 0x10008f10 | 0x10008f00 | 8 | open.mp: OnPlayerRequestScoresAndPings |
| 168 | 0x10004700 | 0x100046e0 | 9 | open.mp: OnPlayerCameraTarget |
| 177 | 0x10006ac0 | 0x10006ab0 | 9 | open.mp: OnPlayerDamageActor |
| 184 | 0x1000d920 | 0x1000d910 | 9 | open.mp: FinishDownload |
