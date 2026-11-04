# docs/re — SA:MP 0.3.7-R4 client protocol specs (index)

Clean-room specs written in our own words (see the project `CLAUDE.md`). Targets: A = samp.dll 0.3.7-R4 2021 build (primary), B = 2019 build (protocol-identical). Wire layouts were taken from open.mp (`re/openmp-src/Shared/NetCode`) and confirmed against the client's handlers and send sites.

## Spec files

| file | contents |
|---|---|
| `handshake.md` | Connection: §1 unconnected handshake/cookie, §2 datagram cipher, §3 RakNet reliability layer, §4 auth challenge, §5 join (RPC 25 / InitGame 139), §6 RPC framing. |
| `rpc-ids.md` | Master list of all 141 client-registered S→C RPC IDs and the C→S send sites, with A/B handler addresses, reliabilities and open.mp names. |
| `build-diff-r4.md` | How the 2019 and 2021 R4 builds were compared and where they differ (protocol-identical). |
| `subsystems/player-state.md` | Local player state: spawn and class selection, death, position/health, appearance, weapons and money, world state (time, weather, gravity, bounds), interior, camera, damage, clicks and SCM events (61 RPC IDs). |
| `subsystems/remote-players-sync.md` | Remote-player lifecycle RPCs and all sync packets 200–212: framing, quantised encodings, send rates, and how stored sync is applied each frame. |
| `subsystems/vehicles.md` | Vehicle RPCs (19 IDs): stream in/out, params, enter/exit, damage status, components, trailers, SCM events, vehicle death. |
| `subsystems/world-entities.md` | Objects, object select/edit, pickups, actors, checkpoints, gang zones, map icons, 3D labels, and the dormant custom-model RPCs (43 IDs). |
| `subsystems/ui-chat-audio.md` | Chat window and local commands, client messages, game text, textdraws, dialogs (incl. the Huffman-compressed body), menus, audio streams, clock and widescreen toggles (20 IDs + packet 201). |
| `subsystems/ui-rendering.md` | How the UI is drawn and driven: D3D9 Present proxy and per-frame draw order, textdraw pass after `Render2dStuff`, fonts (sizes, faces, TTF/PNG assets), `{RRGGBB}` colouring and outlines, the DXUT-based GUI skin, cursor/input-lock modes, window procedure and key table, layout of chat, input, dialogs, scoreboard, nametags/bars, death window and bubbles, `sa-mp.cfg` keys (RE-5). |
| `subsystems/session-misc.md` | Join cross-check, connection rejection, server restart, ClientCheck, RCON packet, scoreboard and report RPCs, registered RPCs that open.mp does not define, and the dormant custom-model download machinery. |
| `subsystems/gta-patches.md` | Every write samp.dll makes into gta_sa.exe memory (hooks, NOPs, byte patches, pool and model-store relocations) and when each is applied. |
| `data/gta-patches-r4.csv` | Full machine-readable list of the GTA patches (384 rows) behind `gta-patches.md`. |
| `data/datagram_sbox.hex` | Substitution table used by the datagram cipher (`handshake.md` §2). |

## Coverage (ID → spec file)

Built by grepping the section headings of every file in `subsystems/`, not from the agents' self-reports. Every S→C RPC the client registers (141 IDs) and every C→S RPC in the `rpc-ids.md` send table (29 IDs) has at least one section. "Client status" says whether the R4 client actually registers or sends the ID.

### RPCs

| ID | dir | open.mp name | client status | spec file(s) |
|---|---|---|---|---|
| 11 | S→C | SetPlayerName | registered | subsystems/player-state.md, subsystems/remote-players-sync.md |
| 12 | S→C | SetPlayerPosition | registered | subsystems/player-state.md |
| 13 | S→C | SetPlayerPositionFindZ | registered | subsystems/player-state.md |
| 14 | S→C | SetPlayerHealth | registered | subsystems/player-state.md |
| 15 | S→C | TogglePlayerControllable | registered | subsystems/player-state.md |
| 16 | S→C | PlayerPlaySound | registered | subsystems/player-state.md |
| 17 | S→C | SetWorldBounds | registered | subsystems/player-state.md |
| 18 | S→C | GivePlayerMoney | registered | subsystems/player-state.md |
| 19 | S→C | SetPlayerFacingAngle | registered | subsystems/player-state.md |
| 20 | S→C | ResetPlayerMoney | registered | subsystems/player-state.md |
| 21 | S→C | ResetPlayerWeapons | registered | subsystems/player-state.md |
| 22 | S→C | GivePlayerWeapon | registered | subsystems/player-state.md |
| 23 | C→S | OnPlayerClickPlayer | sent | subsystems/player-state.md, subsystems/session-misc.md |
| 24 | S→C | SetVehicleParams | registered | subsystems/vehicles.md |
| 25 | C→S | PlayerConnect | sent | subsystems/session-misc.md |
| 26 | C→S | OnPlayerEnterVehicle | sent | subsystems/vehicles.md |
| 26 | S→C | EnterVehicle | registered | subsystems/vehicles.md |
| 27 | C→S | OnPlayerSelectObject | sent (A 0x1006dcd0, rel 9) — missing from rpc-ids.md send table | subsystems/world-entities.md |
| 27 | S→C | PlayerBeginObjectSelect | registered | subsystems/world-entities.md |
| 28 | S→C | PlayerCancelObjectEdit | registered | subsystems/world-entities.md |
| 29 | S→C | SetPlayerTime | registered | subsystems/player-state.md |
| 30 | S→C | TogglePlayerClock | registered | subsystems/player-state.md, subsystems/ui-chat-audio.md |
| 32 | S→C | PlayerStreamIn | registered | subsystems/remote-players-sync.md |
| 33 | S→C | SetPlayerShopName | registered | subsystems/player-state.md |
| 34 | S→C | SetPlayerSkillLevel | registered | subsystems/player-state.md |
| 35 | S→C | SetPlayerDrunkLevel | registered | subsystems/player-state.md |
| 36 | S→C | PlayerShowTextLabel | registered | subsystems/world-entities.md |
| 37 | S→C | DisableCheckpoint | registered | subsystems/world-entities.md |
| 38 | S→C | SetRaceCheckpoint | registered | subsystems/world-entities.md |
| 39 | S→C | DisableRaceCheckpoint | registered | subsystems/world-entities.md |
| 40 | S→C | PlayerClose | registered | subsystems/session-misc.md |
| 41 | S→C | PlayAudioStreamForPlayer | registered | subsystems/ui-chat-audio.md |
| 42 | S→C | StopAudioStreamForPlayer | registered | subsystems/ui-chat-audio.md |
| 43 | S→C | RemoveBuildingForPlayer | registered | subsystems/player-state.md |
| 44 | S→C | CreateObject | registered | subsystems/world-entities.md |
| 45 | S→C | SetObjectPosition | registered | subsystems/world-entities.md |
| 46 | S→C | SetObjectRotation | registered | subsystems/world-entities.md |
| 47 | S→C | DestroyObject | registered | subsystems/world-entities.md |
| 48 | S→C | SetPlayerVirtualWorld | registered | subsystems/player-state.md |
| 50 | C→S | PlayerRequestCommandMessage | sent | subsystems/ui-chat-audio.md |
| 50 | S→C | PlayerCommandMessage | not registered by the client | subsystems/ui-chat-audio.md |
| 52 | C→S | PlayerSpawn | sent | subsystems/player-state.md |
| 53 | C→S | OnPlayerDeath | sent | subsystems/player-state.md |
| 54 | C→S | NPCConnect | not sent, not registered | subsystems/session-misc.md |
| 55 | S→C | SendDeathMessage | registered | subsystems/player-state.md |
| 56 | S→C | SetPlayerMapIcon | registered | subsystems/world-entities.md |
| 57 | S→C | RemoveVehicleComponent | registered | subsystems/vehicles.md |
| 58 | S→C | PlayerHideTextLabel | registered | subsystems/world-entities.md |
| 59 | S→C | SetPlayerChatBubble | registered | subsystems/remote-players-sync.md, subsystems/ui-chat-audio.md |
| 60 | S→C | SendGameTimeUpdate | registered | subsystems/player-state.md |
| 61 | S→C | ShowDialog | registered | subsystems/ui-chat-audio.md |
| 62 | C→S | OnPlayerDialogResponse | sent | subsystems/ui-chat-audio.md |
| 63 | S→C | PlayerDestroyPickup | registered | subsystems/world-entities.md |
| 64 | S→C | — (no-op (reads u16)) | registered | subsystems/session-misc.md |
| 65 | S→C | LinkVehicleToInterior | registered | subsystems/vehicles.md |
| 66 | S→C | SetPlayerArmour | registered | subsystems/player-state.md |
| 67 | S→C | SetPlayerArmedWeapon | registered | subsystems/player-state.md |
| 68 | S→C | SetSpawnInfo | registered | subsystems/player-state.md |
| 69 | S→C | SetPlayerTeam | registered | subsystems/player-state.md |
| 70 | S→C | PutPlayerInVehicle | registered | subsystems/vehicles.md |
| 71 | S→C | RemovePlayerFromVehicle | registered | subsystems/vehicles.md |
| 72 | S→C | SetPlayerColor | registered | subsystems/player-state.md |
| 73 | S→C | SendGameText | registered | subsystems/ui-chat-audio.md |
| 74 | S→C | ForcePlayerClassSelection | registered | subsystems/player-state.md |
| 75 | S→C | AttachObjectToPlayer | registered | subsystems/world-entities.md |
| 76 | S→C | PlayerInitMenu | registered | subsystems/ui-chat-audio.md |
| 77 | S→C | PlayerShowMenu | registered | subsystems/ui-chat-audio.md |
| 78 | S→C | PlayerHideMenu | registered | subsystems/ui-chat-audio.md |
| 79 | S→C | CreateExplosion | registered | subsystems/player-state.md |
| 80 | S→C | ShowPlayerNameTagForPlayer | registered | subsystems/remote-players-sync.md |
| 81 | S→C | AttachCameraToObject | registered | subsystems/player-state.md |
| 82 | S→C | InterpolateCamera | registered | subsystems/player-state.md |
| 83 | C→S | OnPlayerSelectTextDraw | sent | subsystems/ui-chat-audio.md |
| 83 | S→C | PlayerBeginTextDrawSelect | registered | subsystems/ui-chat-audio.md |
| 84 | S→C | SetPlayerObjectMaterial | registered | subsystems/world-entities.md |
| 85 | S→C | StopFlashGangZone | registered | subsystems/world-entities.md |
| 86 | S→C | ApplyPlayerAnimation | registered | subsystems/player-state.md |
| 87 | S→C | ClearPlayerTasks | registered | subsystems/player-state.md |
| 88 | S→C | SetPlayerSpecialAction | registered | subsystems/player-state.md |
| 89 | S→C | SetPlayerFightingStyle | registered | subsystems/player-state.md |
| 90 | S→C | SetPlayerVelocity | registered | subsystems/player-state.md |
| 91 | S→C | SetVehicleVelocity | registered | subsystems/vehicles.md |
| 92 | S→C | — (GTA opcode 0x052C on local player) | registered | subsystems/session-misc.md |
| 93 | S→C | SendClientMessage | registered | subsystems/ui-chat-audio.md |
| 94 | S→C | SetPlayerWorldTime | registered | subsystems/player-state.md |
| 95 | S→C | PlayerCreatePickup | registered | subsystems/world-entities.md |
| 96 | C→S | SCMEvent | sent | subsystems/player-state.md, subsystems/vehicles.md |
| 96 | S→C | SCMEvent | registered | subsystems/player-state.md, subsystems/vehicles.md |
| 97 | C→S | — (picked up dropped weapon (never sent)) | send site exists, never reached | subsystems/session-misc.md |
| 98 | S→C | — (vehicle tyre status) | registered | subsystems/session-misc.md |
| 99 | S→C | MoveObject | registered | subsystems/world-entities.md |
| 101 | C→S | PlayerRequestChatMessage | sent | subsystems/ui-chat-audio.md |
| 101 | S→C | PlayerChatMessage | registered | subsystems/ui-chat-audio.md |
| 102 | S→C | — (server net-stats blob) | registered | subsystems/session-misc.md |
| 103 | C→S | ClientCheck | sent | subsystems/session-misc.md |
| 103 | S→C | ClientCheck | registered | subsystems/session-misc.md |
| 104 | S→C | EnableStuntBonusForPlayer | registered | subsystems/player-state.md |
| 105 | S→C | PlayerTextDrawSetString | registered | subsystems/ui-chat-audio.md |
| 106 | C→S | SetVehicleDamageStatus | sent | subsystems/vehicles.md |
| 106 | S→C | SetVehicleDamageStatus | registered | subsystems/vehicles.md |
| 107 | S→C | SetCheckpoint | registered | subsystems/world-entities.md |
| 108 | S→C | ShowGangZone | registered | subsystems/world-entities.md |
| 111 | S→C | ToggleWidescreen | registered | subsystems/player-state.md, subsystems/ui-chat-audio.md |
| 112 | S→C | PlayCrimeReport | registered | subsystems/player-state.md |
| 113 | S→C | SetPlayerAttachedObject | registered | subsystems/world-entities.md |
| 115 | C→S | OnPlayerGiveTakeDamage | sent | subsystems/player-state.md |
| 116 | C→S | OnPlayerEditAttachedObject | sent | subsystems/world-entities.md |
| 116 | S→C | PlayerBeginAttachedObjectEdit | registered | subsystems/world-entities.md |
| 117 | C→S | OnPlayerEditObject | sent | subsystems/world-entities.md |
| 117 | S→C | PlayerBeginObjectEdit | registered | subsystems/world-entities.md |
| 118 | C→S | OnPlayerInteriorChange | sent | subsystems/player-state.md |
| 119 | C→S | OnPlayerClickMap | sent | subsystems/player-state.md, subsystems/session-misc.md |
| 120 | S→C | HideGangZone | registered | subsystems/world-entities.md |
| 121 | S→C | FlashGangZone | registered | subsystems/world-entities.md |
| 122 | S→C | StopObject | registered | subsystems/world-entities.md |
| 123 | S→C | SetVehiclePlate | registered | subsystems/vehicles.md |
| 124 | S→C | TogglePlayerSpectating | registered | subsystems/remote-players-sync.md |
| 125 | S→C | — (no-op) | registered | subsystems/session-misc.md |
| 126 | S→C | PlayerSpectatePlayer | registered | subsystems/remote-players-sync.md |
| 127 | S→C | PlayerSpectateVehicle | registered | subsystems/remote-players-sync.md |
| 128 | C→S | PlayerRequestClass | sent | subsystems/player-state.md |
| 128 | S→C | PlayerRequestClassResponse | registered | subsystems/player-state.md |
| 129 | C→S | PlayerRequestSpawn | sent | subsystems/player-state.md |
| 129 | S→C | ImmediatelySpawnPlayer / PlayerRequestSpawnResponse | registered | subsystems/player-state.md |
| 130 | S→C | — (connection rejected) | registered | subsystems/session-misc.md |
| 131 | C→S | OnPlayerPickUpPickup | sent | subsystems/world-entities.md |
| 132 | C→S | OnPlayerSelectedMenuRow | sent | subsystems/ui-chat-audio.md |
| 133 | S→C | SetPlayerWantedLevel | registered | subsystems/player-state.md |
| 134 | S→C | PlayerShowTextDraw | registered | subsystems/ui-chat-audio.md |
| 135 | S→C | PlayerHideTextDraw | registered | subsystems/ui-chat-audio.md |
| 136 | C→S | VehicleDeath | sent | subsystems/vehicles.md |
| 137 | S→C | PlayerJoin | registered | subsystems/remote-players-sync.md |
| 138 | S→C | PlayerQuit | registered | subsystems/remote-players-sync.md |
| 139 | S→C | PlayerInit | registered | subsystems/session-misc.md |
| 140 | C→S | OnPlayerExitedMenu | sent | subsystems/ui-chat-audio.md |
| 144 | S→C | RemovePlayerMapIcon | registered | subsystems/world-entities.md |
| 145 | S→C | SetPlayerAmmo | registered | subsystems/player-state.md |
| 146 | S→C | SetPlayerGravity | registered | subsystems/player-state.md |
| 147 | S→C | SetVehicleHealth | registered | subsystems/vehicles.md |
| 148 | S→C | AttachTrailer | registered | subsystems/vehicles.md |
| 149 | S→C | DetachTrailer | registered | subsystems/vehicles.md |
| 150 | S→C | — (GTA opcode 0x03FD on local player) | registered | subsystems/session-misc.md |
| 151 | S→C | — (destroy dropped-weapon pickups (dead path)) | registered | subsystems/session-misc.md |
| 152 | S→C | SetPlayerWeather | registered | subsystems/player-state.md |
| 153 | S→C | SetPlayerSkin | registered | subsystems/player-state.md |
| 154 | C→S | OnPlayerExitVehicle | sent | subsystems/vehicles.md |
| 154 | S→C | ExitVehicle | registered | subsystems/vehicles.md |
| 155 | C→S | OnPlayerRequestScoresAndPings | sent | subsystems/remote-players-sync.md, subsystems/session-misc.md |
| 155 | S→C | SendPlayerScoresAndPings | registered | subsystems/remote-players-sync.md, subsystems/session-misc.md |
| 156 | S→C | SetPlayerInterior | registered | subsystems/player-state.md |
| 157 | S→C | SetPlayerCameraPosition | registered | subsystems/player-state.md |
| 158 | S→C | SetPlayerCameraLookAt | registered | subsystems/player-state.md |
| 159 | S→C | SetVehiclePosition | registered | subsystems/vehicles.md |
| 160 | S→C | SetVehicleZAngle | registered | subsystems/vehicles.md |
| 161 | S→C | — (old 3-field vehicle params for player) | registered | subsystems/session-misc.md |
| 162 | S→C | SetPlayerCameraBehindPlayer | registered | subsystems/player-state.md |
| 163 | S→C | PlayerStreamOut | registered | subsystems/remote-players-sync.md |
| 164 | S→C | StreamInVehicle | registered | subsystems/vehicles.md |
| 165 | S→C | StreamOutVehicle | registered | subsystems/vehicles.md |
| 166 | S→C | PlayerDeath | registered | subsystems/player-state.md |
| 167 | S→C | DisableRemoteVehicleCollisions | registered | subsystems/player-state.md |
| 168 | C→S | OnPlayerCameraTarget | sent | subsystems/player-state.md, subsystems/session-misc.md |
| 169 | S→C | — (object excluded from camera collision) | registered | subsystems/session-misc.md, subsystems/world-entities.md |
| 170 | S→C | SetPlayerCameraTargeting | registered | subsystems/player-state.md |
| 171 | S→C | ShowActorForPlayer | registered | subsystems/world-entities.md |
| 172 | S→C | HideActorForPlayer | registered | subsystems/world-entities.md |
| 173 | S→C | ApplyActorAnimationForPlayer | registered | subsystems/world-entities.md |
| 174 | S→C | ClearActorAnimationsForPlayer | registered | subsystems/world-entities.md |
| 175 | S→C | SetActorFacingAngleForPlayer | registered | subsystems/world-entities.md |
| 176 | S→C | SetActorPosForPlayer | registered | subsystems/world-entities.md |
| 177 | C→S | OnPlayerDamageActor | sent | subsystems/world-entities.md |
| 178 | S→C | SetActorHealthForPlayer | registered | subsystems/world-entities.md |
| 179 | S→C | ModelRequest | handler exists, never registered (dormant) | subsystems/session-misc.md, subsystems/world-entities.md |
| 181 | C→S | RequestDFF | variable-ID send site A 0x1000d800, dormant | subsystems/session-misc.md, subsystems/world-entities.md |
| 182 | C→S | RequestTXD | variable-ID send site A 0x1000d800, dormant | subsystems/session-misc.md, subsystems/world-entities.md |
| 183 | S→C | ModelUrl | handler exists, never registered (dormant) | subsystems/session-misc.md, subsystems/world-entities.md |
| 184 | C→S | FinishDownload | dormant (custom-model code, never reached) | subsystems/session-misc.md, subsystems/world-entities.md |
| 185 | S→C | DownloadCompleted | handler identity disputed, never registered (dormant) | subsystems/session-misc.md, subsystems/world-entities.md |


### Sync packets

| packet | dir | open.mp name | client status | spec file(s) |
|---|---|---|---|---|
| 200 | both | PlayerVehicleSync | handled + sent | subsystems/remote-players-sync.md |
| 201 | C→S | PlayerRconCommand | sent (/rcon) | subsystems/remote-players-sync.md, subsystems/session-misc.md, subsystems/ui-chat-audio.md |
| 202 | — | — (no open.mp struct) | neither handled nor sent | — (out of scope; see gaps) |
| 203 | both | PlayerAimSync | handled + sent | subsystems/remote-players-sync.md |
| 204 | C→S | PlayerWeaponsUpdate | sent only | subsystems/remote-players-sync.md |
| 205 | C→S | PlayerStatsSync | sent only | subsystems/remote-players-sync.md |
| 206 | both | PlayerBulletSync | handled + sent | subsystems/remote-players-sync.md |
| 207 | both | PlayerFootSync | handled + sent | subsystems/remote-players-sync.md |
| 208 | S→C | PlayerMarkersSync | handled only | subsystems/remote-players-sync.md |
| 209 | both | PlayerUnoccupiedSync | handled + sent | subsystems/remote-players-sync.md |
| 210 | both | PlayerTrailerSync | handled + sent | subsystems/remote-players-sync.md |
| 211 | both | PlayerPassengerSync | handled + sent | subsystems/remote-players-sync.md |
| 212 | C→S | PlayerSpectatorSync | sent only | subsystems/remote-players-sync.md |

## Gaps

No registered or sent ID lacks a spec. Remaining gaps are in the index and in depth, not in coverage:

1. **`rpc-ids.md` send table is incomplete.** C→S RPC 27 (OnPlayerSelectObject) is sent from A 0x1006dcd0 (ID cell 0x100e5e9c, reliability 9, priority 1), but the table has no row for it. That send site is not a Ghidra function, which explains why the original scan missed it; `world-entities.md` documents it. The table also omits 181/182 (variable-ID site A 0x1000d800, dormant). The heading of that table says "27 distinct IDs" while it lists 29.
2. **Packet 202** (the remaining ID in 200–212) is neither handled by the packet dispatcher (A 0x1000b260 handles only 200, 203, 206–211 plus RakNet control IDs) nor sent by the client; open.mp has no struct for it. Nothing needs to be specified.
3. **Not specified in depth** (listed as unknown in the relevant file): per-frame world-bounds enforcement, per-frame object motion/attachment and the object-editor gizmo, the Huffman tree tie-break order, the quantisation *encoders* (only decoders were observed), and most GTA hook/NOP purposes (gta_sa.exe was not available).
4. **Overlapping sections** (same ID in two files): 11, 59 (player-state/remote-players-sync, ui-chat-audio); 23, 119, 168 (player-state, session-misc); 30, 111 (player-state, ui-chat-audio); 155 (remote-players-sync, session-misc); 169 and the custom-model IDs 179–185 (world-entities, session-misc); packet 201 (three files). The layouts agree. See the critic notes below for one wording conflict.

## open.mp disagreements

Wire-layout differences (each still interoperates):

| ID | dir | difference | file |
|---|---|---|---|
| 128 | C→S | Client writes the class index as u32; open.mp reads u16 (low 16 bits arrive first, so it works). Send u32 like the original. | player-state |
| 129 | S→C | open.mp writes u32; the client reads only 8 bits. | player-state |
| 25 | C→S | Client always sends a trailing u32 (challenge response again); open.mp's reader treats it as optional and reads it as "official client" + omp version. | session-misc |
| 45 | S→C | Client reads a fourth f32 after the position; open.mp sends only id + vec3. The extra read fails harmlessly. | world-entities |
| 99 | S→C | The "current position" vec3 that open.mp sends is read but ignored; a rotation component ≤ −999 means "no rotation". | world-entities |
| 27 | C→S | Client always writes select type 0; open.mp ignores the field. | world-entities |
| 116 | C→S | If the slot is invalid or unused, the client omits the 52-byte slot block (8-byte payload) and open.mp's reader fails. | world-entities |
| 151 | S→C | No open.mp struct; client reads the owner as 8 bits while the pickup table and C→S 97 use 16 bits (dead path). | session-misc |
| 169 | S→C | No open.mp struct; open.mp folds the flag into CreateObject's u8 field. | world-entities, session-misc |

Behavioural differences that a compatible client or server must know:

- **Not registered by R4:** S→C 50 (PlayerCommandMessage), 54 NPCConnect (never sent), and the whole custom-model set 179/181/182/183/184/185 (handlers exist as dormant code; an R4 profile must not register them).
- **Timestamp prefix:** only S→C 207 and 200 accept the 0x28 + u32 timestamp prefix; other sync handlers would misparse a prefixed packet.
- **Reliabilities:** 204 and 205 are sent UNRELIABLE; 204 carries only changed slots and is never resent. The client sends vehicle RPCs on ordering channel 0, while open.mp tags its own sends with its sync-RPC channel.
- **Ignored fields:** the 138 quit reason, the 154 S→C vehicle id, and the RPC 112 suspect id.
- **Client-side limits open.mp does not express:** chat 144 chars, game text 1..200 with an unchecked style, textdraw text < 800 (SetString has an off-by-one), menus capped at 12 items; ShowDialog is dropped if the title or body is empty, and a negative dialog id closes the dialog.
- **Client-side value changes:** RPC 115 damage is scaled by 0.33 before sending; RPC 13 adds 1.5 to Z; RPC 119 sends ground Z + 2.0; RPC 82/158 cut values of 0 or >2 become 2.
- **Field orders that differ by direction:** SCMEvent 96 (S→C: player, type, vehicle, arg1, arg2; C→S: vehicle, arg1, arg2, type) and door/panel order in 164 vs 106. open.mp matches the client in each case.
- **Locomotives:** stream-in of model 537/538 creates three carriages at id+1..id+3, so the server must keep those IDs free.
- **Colour inconsistency in open.mp:** attached-object colours (113/116) are written ABGR and read back ARGB by open.mp; the client echoes the bytes it was given.
- **TextDraw alignment:** the client reads bit1 = left, bit2 = right, bit3 = centre. open.mp's mapping is consistent, but reusing its netcode struct with raw 1/2/3 values would be wrong.

## Top unresolved questions

1. **Huffman tie-break order** for the dialog-body string codec (A 0x10026ea0). Needs a round-trip test against a server before the Rust codec is trusted.
2. **Quantised encoders** (normalised quaternion, compressed vector, health/armour byte): only decoders were observed, and open.mp's SDK bitstream helpers are missing from the checkout. The VEC4 quaternion order in packet 210 is unchecked against open.mp.
3. **RPC 185 handler identity:** world-entities proposes A 0x1000ec30, session-misc A 0x1000eba0. Both are unregistered, so this only matters for a future profile that enables custom models.
4. **ClientCheck (103)** semantics beyond the layout: which request types return what. Only the layout and the accepted type set are documented.
5. **World bounds:** where the per-frame enforcement reads Settings+5..+0x11.
6. **GTA identities** marked "inferred" or "believed": script opcodes 0x052C/0x03FD (RPC 92/150), 0x09C4/0x07FF at vehicle creation, GTA functions 0x4D6D50/0x4D6D60/0x4E71E0, radio-volume and CPad offsets, and vtable class identities in `gta-patches.md`.
7. **GTA patch purposes:** most of the ~63 hooks and the NOP/byte patches, because gta_sa.exe was not available. This blocks stage 3 (DLL on Windows) more than the headless client.
8. **Unidentified client gates:** the unoccupied-sync model set (A 0x100ed574), the enter-as-driver checks (A 0x100A3700), the vehicle-pool table at +0xBD14 that gates RPC 161, and the netgame flag +0x232 (DisableRemoteVehicleCollisions consumer).

## Completeness-critic notes (2026-11-04)

- **Spot-check:** nine fields tables were compared against open.mp and the A decompilation: 55 SendDeathMessage and 96 SCMEvent S→C (player-state), 11 SetPlayerName and 137 PlayerJoin (remote-players-sync), 27 PlayerBeginObjectSelect, 58 PlayerHideTextLabel and 56 SetPlayerMapIcon (world-entities, including the 0x04CE/0x02A8/0x0570/0x02A7/0x0165/0x0168 opcode descriptors read from the DLL), 78 PlayerHideMenu and 73 SendGameText (ui-chat-audio), 24 SetVehicleParams (vehicles, byte-to-action mapping and GTA door/window calls), and 155 SendPlayerScoresAndPings (session-misc). No layout or bound errors were found.
- **Wording conflict:** for RPC 111, `player-state.md` says "Widescreen = %i" is written "to the client log". The handler calls the type-8 debug chat printer (A 0x100680f0), so the line is visible in chat, as `ui-chat-audio.md` says.
- **Resolved across files:** `player-state.md` lists the remote-player field +0x10c (the damage hook cancels local damage when it is set) as unknown. `remote-players-sync.md` shows that PlayerJoin (137) sets this field from `isNPC`, and the accessor A 0x100010f0 returns the same remote-player object. So damage to NPC players is reported via RPC 115 but cancelled locally.
- **Clean room:** no subsystem file contains decompiler identifiers (`FUN_`, `DAT_`, `param_N`, `local_N`, `undefinedN`, Ghidra temporaries) or pasted open.mp C++ (bitstream calls, struct bodies). The two fenced blocks are a two-line paraphrase of the RPC 106 send rule (vehicles.md) and the Huffman frequency table, an interop constant with its source address (ui-chat-audio.md).

## Verification log

- 2026-11-04 — byte-level check of the remote-control hook claims (main session,
  reading gta_sa.exe 1.0 US and samp.dll A directly; names via gta-reversed):
  - `0x86D190` (CPlayerPed vtable +0x28) originally points to 0x60EA90
    `CPlayerPed::ProcessControl`; samp A stores 0x100A29A0 there (store at A 0x100A6370+3/+25). **Confirmed.**
  - `0x86D744` (CTaskSimpleUseGun vtable +0x20) originally points to 0x624ED0
    `CTaskSimpleUseGun::SetPedPosition`; samp redirects it to A 0x100A2E20. **Confirmed (CSV row + original target).**
  - `0x53EB12` is `call 0x53E230 Render2dStuff` inside Idle; samp rewrites its rel32
    at 0x53EB13 (`mov [0x53eb13], eax` at A 0x100A6163). **Confirmed.**
  - `CWorld::Players` (0xB7CD98, 2 × 0x190) ends exactly at `ms_aSectors`
    (0xB7D0B8, 115 200 B), which samp relocates and zeroes → room for slots 2..209. **Consistent.**
- Bot heading convention h ↔ (cos h/2, 0, 0, −sin h/2) from remote-control-onfoot.md:
  **confirmed live** (server-side GetPlayerFacingAngle now tracks the bot's heading).
