# Player state, world and camera RPCs (SA:MP 0.3.7-R4 client) — clean-room spec

This document covers the RPCs that change the local player's state (position, health, armour,
skin, weapons, money, team, colour, name, interior, virtual world, animations, special
actions), the world settings that the server drives (time, clock, weather, gravity, world bounds,
explosions, sounds, removed buildings, crime reports), the camera RPCs, class selection and
spawning, death reporting, and the small client→server notifications for damage, interior
changes, map and player clicks and camera targets. For every item the wire layout was taken from the
open.mp NetCode headers (reference only) and then **checked against the read/write sequence in the
2021 client (A)**. A 2019 build (B) was spot-checked: all 54 server→client handlers and all 10
client→server send sites in scope have the same sequence of bit-widths in B (checked
automatically on the decompiled call sequences, and by hand for the raw-byte reads of 33/68/128/86
and the tiny handlers 20/21/74/162/167/170). Only two width mismatches were found. Both are
compatible on the wire (RPC 128 C→S class index: u32 sent, open.mp reads u16; RPC 129 S→C spawn
response: open.mp writes u32, client reads u8). There are no packet IDs in this scope; on-foot and
other sync packets belong elsewhere.

Wire conventions are the ones in `handshake.md` (bitstream MSB-first within a byte; multi-byte
values are little-endian; "bit" = 1 bit; floats are IEEE-754 single, 32 bits; reads are **not**
byte-aligned unless stated). "raw[n]" means n bytes copied verbatim, not aligned. Every
handler builds a bitstream over the RPC payload and reads with "right-aligned" reads (0x1001f9b0 for
n bits, 0x1001fea0 for raw bytes, 0x10008420 for a single bit). All client→server sends use priority
1, ordering channel 0, no timestamp. The reliability column uses the shifted enum (8 = RELIABLE,
9 = RELIABLE_ORDERED, 10 = RELIABLE_SEQUENCED).

## Summary table

| ID | open.mp name | dir | A handler / send site | B | confidence |
|---|---|---|---|---|---|
| 11 | SetPlayerName | S→C | 0x1001dfd0 | 0x1001df80 | high |
| 12 | SetPlayerPosition | S→C | 0x10019320 | 0x100192d0 | high |
| 13 | SetPlayerPositionFindZ | S→C | 0x10019440 | 0x100193f0 | high |
| 14 | SetPlayerHealth | S→C | 0x10019550 | 0x10019500 | high |
| 15 | TogglePlayerControllable | S→C | 0x1001a290 | 0x1001a240 | high |
| 16 | PlayerPlaySound | S→C | 0x1001a330 | 0x1001a2e0 | high (layout), medium (sound ranges) |
| 17 | SetWorldBounds | S→C | 0x1001a410 | 0x1001a3c0 | high (layout), enforcement not traced |
| 18 | GivePlayerMoney | S→C | 0x1001a500 | 0x1001a4b0 | high |
| 19 | SetPlayerFacingAngle | S→C | 0x1001a5a0 | 0x1001a550 | high |
| 20 | ResetPlayerMoney | S→C | 0x10018090 (thunk) | 0x10018040 | high |
| 21 | ResetPlayerWeapons | S→C | 0x100180a0 (thunk) | 0x10018050 | high |
| 22 | GivePlayerWeapon | S→C | 0x1001a640 | 0x1001a5f0 | high |
| 23 | OnPlayerClickPlayer | C→S | 0x1006ec10 | 0x1006ebd0 | high |
| 29 | SetPlayerTime | S→C | 0x1000f420 | 0x1000f3f0 | high |
| 30 | TogglePlayerClock | S→C | 0x1000f500 | 0x1000f4d0 | high |
| 33 | SetPlayerShopName | S→C | 0x10017e50 | 0x10017e00 | high |
| 34 | SetPlayerSkillLevel | S→C | 0x1000f5e0 | 0x1000f5b0 | high |
| 35 | SetPlayerDrunkLevel | S→C | 0x10018db0 | 0x10018d60 | high |
| 43 | RemoveBuildingForPlayer | S→C | 0x1001d530 | 0x1001d4e0 | high (layout), medium (removal internals) |
| 48 | SetPlayerVirtualWorld | S→C | 0x1001dcc0 | 0x1001dc70 | high (layout), low (purpose of storage object) |
| 52 | PlayerSpawn | C→S | 0x10003c20 (inside local Spawn) | 0x10003c10 | high |
| 53 | OnPlayerDeath | C→S | 0x10005810 | 0x10005800 | high |
| 55 | SendDeathMessage | S→C | 0x1001dd70 | 0x1001dd20 | high |
| 60 | SendGameTimeUpdate | S→C | 0x1000ff30 | 0x1000ff10 | high (layout), medium (meaning) |
| 66 | SetPlayerArmour | S→C | 0x1001ab60 | 0x1001ab10 | high |
| 67 | SetPlayerArmedWeapon | S→C | 0x10018e50 | 0x10018e00 | high |
| 68 | SetSpawnInfo | S→C | 0x10017f50 | 0x10017f00 | high |
| 69 | SetPlayerTeam | S→C | 0x10019710 | 0x100196c0 | high |
| 72 | SetPlayerColor | S→C | 0x10019800 | 0x100197b0 | high |
| 74 | ForcePlayerClassSelection | S→C | 0x100180d0 (no payload) | 0x10018080 | high |
| 79 | CreateExplosion | S→C | 0x1001bd10 | 0x1001bcc0 | high |
| 81 | AttachCameraToObject | S→C | 0x10019ff0 | 0x10019fa0 | high |
| 82 | InterpolateCamera | S→C | 0x1001a0f0 | 0x1001a0a0 | high |
| 86 | ApplyPlayerAnimation | S→C | 0x1001a950 | 0x1001a900 | high |
| 87 | ClearPlayerTasks | S→C | 0x10018580 | 0x10018530 | high |
| 88 | SetPlayerSpecialAction | S→C | 0x10018690 | 0x10018640 | high |
| 89 | SetPlayerFightingStyle | S→C | 0x10018740 | 0x100186f0 | high |
| 90 | SetPlayerVelocity | S→C | 0x10018850 | 0x10018800 | high |
| 94 | SetPlayerWorldTime | S→C | 0x1000eef0 | 0x1000eec0 | high |
| 96 | SCMEvent | S→C and C→S | handler 0x1000f280; send 0x10001ce0 | 0x1000f250; 0x10001ce0 | high |
| 104 | EnableStuntBonusForPlayer | S→C | 0x10017d50 | 0x10017d00 | high |
| 111 | ToggleWidescreen | S→C | 0x10018ac0 | 0x10018a70 | high |
| 112 | PlayCrimeReport | S→C | 0x10019050 | 0x10019000 | high (layout), medium (behaviour) |
| 115 | OnPlayerGiveTakeDamage | C→S | 0x100068a0 (take), 0x100069b0 (give) | 0x10006890, 0x100069a0 | high |
| 118 | OnPlayerInteriorChange | C→S | 0x10005970 | 0x10005960 | high |
| 119 | OnPlayerClickMap | C→S | inside 0x10075790 | 0x10075760 | high |
| 128 | PlayerRequestClass / …Response | C→S and S→C | send 0x100058d0; handler 0x1000ffd0 | 0x100058c0; 0x1000ffb0 | high, **width note** |
| 129 | PlayerRequestSpawn / …Response / ImmediatelySpawnPlayer | C→S and S→C | send 0x10003b60; handler 0x100100a0 | 0x10003b60; 0x10010080 | high, **width note** |
| 133 | SetPlayerWantedLevel | S→C | 0x1001ccf0 | 0x1001cca0 | high |
| 145 | SetPlayerAmmo | S→C | 0x1001ac10 | 0x1001abc0 | high |
| 146 | SetPlayerGravity | S→C | 0x1001acd0 | 0x1001ac80 | high |
| 152 | SetPlayerWeather | S→C | 0x1000f370 | 0x1000f340 | high |
| 153 | SetPlayerSkin | S→C | 0x10019190 | 0x10019140 | high |
| 156 | SetPlayerInterior | S→C | 0x10019a00 | 0x100199b0 | high |
| 157 | SetPlayerCameraPosition | S→C | 0x10019aa0 | 0x10019a50 | high |
| 158 | SetPlayerCameraLookAt | S→C | 0x10019b70 | 0x10019b20 | high |
| 162 | SetPlayerCameraBehindPlayer | S→C | 0x10018080 (thunk) | 0x10018030 | high |
| 166 | PlayerDeath | S→C | 0x10010d40 | 0x10010d20 | high |
| 167 | DisableRemoteVehicleCollisions | S→C | 0x10017de0 | 0x10017d90 | high (layout), consumer not traced |
| 168 | OnPlayerCameraTarget | C→S | 0x10004700 | 0x100046e0 | high |
| 170 | SetPlayerCameraTargeting | S→C | 0x1000ea20 | 0x1000e9f0 | high |

## Object map used in this document (A addresses)

These are the names I use below. They are my labels for the client's objects, not original names.

| label | how it is reached | notes |
|---|---|---|
| NetGame | pointer at 0x1026ea0c | the client's network/game-session object |
| Settings | NetGame+0x3d5 (pointer) | server settings filled by InitGame (RPC 139). Offsets used here: +5/+9/+0xd/+0x11 world bounds; +0x1f "server holds the time" flag; +0x22 same-team damage block (set to 1 by the NetGame constructor); +0x23 class count; +0x2c hour; +0x2d minute; +0x2e weather |
| Pools | NetGame+0x3de (pointer) | +4 player pool, +0xc object pool |
| PlayerPool | Pools+4 | +4 local player ID (u16); +6 local name (string); +0x26 LocalPlayer; +0x2a+4·id "slot used" flags; +0x1f8a+4·id remote entry (entry+0x10 = RemotePlayer, entry+0x14 = remote name string). Remote IDs are accepted if < 1004 (some handlers check < 1005) |
| LocalPlayer | PlayerPool+0x26 | +0x104 LocalPed; +0x10c team; +0x14f..+0x17c stored spawn info (46 bytes); +0x17d "spawn info valid"; +0x143 "class info received"; +0x2fa "go to class selection on next respawn"; +0x302 "spawn request pending"; +0x306 "in class selection"; +0x2fe current class index; +0xf0 "spawned"; +0xf4 "dead"; +0x2da/+0x2de/+0x2e2 surfing/attachment state cleared on teleport; +0x314 "camera under script control"; +0x1cf last reported interior |
| RemotePlayer | entry+0x10 | +0x1dd its ped object (0 if not streamed in); +0x109 team; +0x1c1 "animation applied by script"; +0x10a state byte |
| Ped object | LocalPlayer+0x104 or RemotePlayer+0x1dd | client wrapper around a GTA CPed: +0x40/+0x2a4 GTA ped pointer, +0x44 script handle, +0x2b0 player slot number (0 = local), +0x2c9 drunk level |
| Game | pointer at 0x1026ea24 | client wrapper for GTA. Game+8 is the local ped object (created on demand by 0x10001010). Game+4 is the Camera object, *Game is the Audio object, Game+0x69 "HUD clock enabled" |
| ScriptCommand | 0x100b2350(descriptor, args…) | runs one GTA SCM opcode. The descriptor is a 16-bit opcode followed by a parameter-type string in .rdata. Opcode numbers below were read from those descriptors |

GTA addresses (0x4xxxxx–0xcxxxxx) are those of gta_sa.exe 1.0 US. Names given for GTA functions or
variables are my best identification; anything not certain is marked "(inferred)".

---

## Spawn, class selection, death

### 128 PlayerRequestClass (C→S)
- Send site A 0x100058d0, B 0x100058c0. Reliability 8.
- Fields:

| field | type | notes |
|---|---|---|
| classIndex | u32 | index into the server's class list (0 … classCount−1) |

- Client behaviour (observed):
  - The class-selection tick (A 0x10006100) sends this on the first tick of class selection, with the current index (LocalPlayer+0x2fe). That first tick also shows the class-selection UI (an object at 0x1026eaa4, flag +0x13) and sets LocalPlayer+0x306.
  - Right arrow (VK 0x27) goes to the next class (A 0x10005ff0); after the last class it wraps to 0. Left arrow (VK 0x25) goes to the previous class (A 0x10006080); before index 0 it wraps to classCount−1. classCount is Settings+0x23 from InitGame. Each press clears "class info received" (+0x143), plays sound 1052 (next) or 1053 (previous) at the ped's position through the PlayerPlaySound helper, and sends RPC 128.
  - Arrow keys are ignored during the first 2000 ms of class selection and need ≥ 250 ms between presses. They also only work after the server has answered the last request with "selectable" (+0x143 set).
  - **Dialog gate** (JOIN-P, static: A 0x10006100): everything after the HUD/cursor part of the tick (the first RPC 128, Shift, the arrows) runs only while the dialog object (A global 0x1026e9c8) is absent or its "open" flag (+0x28) is 0. A dialog the server shows on connect therefore holds back the first RPC 128 until it is answered. The close (A 0x1006f860) clears +0x28 at once (the cursor mode-0 request with its 10-frame restore does not delay it), and the dialog's RPC 62 is sent from the window procedure, so the first RPC 128 follows RPC 62 in the next local tick (measured with tools/e2e/joinburst.sh, original client 2/2 runs: RPC 62 in its own datagram, RPC 128 4.2 ms later in the next one; server-side 5 ms). Nothing else is tested: no chat-input, menu, textdraw-selection or input-lock state holds the first request (the frontend-menu flag only skips the HUD/cursor part). Shift additionally needs the chat input closed (+0x14e0 = 0).
- **open.mp vs client:** open.mp reads `Classid` as **u16**. The client writes **32 bits**. Because values are little-endian, open.mp gets the low 16 bits and ignores the rest. A Rust client should send u32 to match the original.

### 128 PlayerRequestClassResponse (S→C)
- A handler 0x1000ffd0, B 0x1000ffb0.
- Fields (open.mp non-DL layout, confirmed):

| field | type | notes |
|---|---|---|
| selectable | u8 | 0 = server refused the class |
| spawnInfo | raw[46] | identical to the RPC 68 body (see below); read as one 46-byte block |

- Client behaviour: if selectable ≠ 0, the client stores the spawn info (same routine as RPC 68, 0x10003be0). It then, if the local ped exists, clears all weapons and sets the ped model to the class skin. Finally it sets "class info received" (LocalPlayer+0x143 = 1), which re-enables arrows and Shift. If selectable = 0 nothing changes, and the player cannot change class or spawn until something else re-enables it.
- open.mp writes an extra `CustomModelID` u32 only for DL clients. For 0.3.7 the size is 1 + 46 bytes, which matches.

### 68 SetSpawnInfo (S→C)
- A 0x10017f50, B 0x10017f00.
- Fields: one raw[46] block with this internal layout (byte offsets):

| off | field | type | notes |
|---|---|---|---|
| 0 | team | u8 | |
| 1 | skin | u32 | ped model |
| 5 | unused | u8 | open.mp `Unknown1` |
| 6 | x, y, z | 3×f32 | spawn position |
| 18 | rotation | f32 | degrees |
| 22 | weapons | 3×u32 | weapon ID; 0xFFFFFFFF = none |
| 34 | ammo | 3×u32 | |

- Client behaviour (LocalPlayer method 0x10003be0): the block is accepted only if the position is inside −20000 < x < 20000, −20000 < y < 20000, −10000 < z < 100000 (validator 0x100b4b90, also used elsewhere). If it is accepted, the 46 bytes are copied to LocalPlayer+0x14f and "spawn info valid" (+0x17d) is set. Nothing is spawned yet.

### 129 PlayerRequestSpawn (C→S)
- Send site A/B 0x10003b60. Reliability 8. **Empty payload.** (handshake.md §6: empty RPCs carry a second compressed 0.)
- Client behaviour: sent from 0x10004060 when Shift (VK 0x10) is pressed during class selection. Shift only works when the chat input is not open, "class info received" is set, no request is pending, and at least 2000 ms have passed since class selection began. Sending sets "spawn request pending" (LocalPlayer+0x302 = 1).

### 129 PlayerRequestSpawnResponse / ImmediatelySpawnPlayer (S→C)
- A handler 0x100100a0, B 0x10010080.
- Fields:

| field | type | notes |
|---|---|---|
| response | u8 (client reads 8 bits) | 0 = refused, 1 = allowed, 2 = force spawn |

- Client behaviour: on response = 2, or on any non-zero response while a request is pending, it calls the local **Spawn** routine (below). Otherwise it clears the pending flag.
- **open.mp vs client:** open.mp writes `Allow` as **u32**, and ImmediatelySpawnPlayer writes u32 2. The client reads only the first byte (the low byte, since values are LE). The trailing 24 bits are ignored. Compatible.

### Local Spawn routine and 52 PlayerSpawn (C→S)
- Spawn = LocalPlayer method A 0x10003c20 (B 0x10003c10). RPC 52 is sent from inside it with an **empty payload**, reliability 9.
- Called (a) from the RPC 129 handler, (b) by the local player's per-frame logic after a death when the player may respawn directly, and (c) from one more caller (0x100041c0, not analysed).
- Sequence (my summary of the observed order):
  1. Do nothing (return 0) unless "spawn info valid" is set.
  2. If in class selection: leave it (clear +0x306, hide the class-selection UI, reset a HUD/radar mode via 0x100a0750(0,0)).
  3. Restore the camera (opcode 0x02EB via 0x1009d890), put the camera behind the player, re-enable HUD drawing (0x100a1df0(1)) and make the player controllable.
  4. Reset or restore the GTA ped (0x100abc10), unless the one-shot flag at 0x100fe0a0 is set (that flag is then cleared). Request the area around the spawn x,y (0x100a0c80). Clear the "cuffed" state. Set the ped's position and heading (0x100ac790). Set the skin from the spawn info. Clear all weapons. Call 0x100ad8c0 (not analysed). Set special action 0 (none).
  5. Clear the surfing/attachment fields. Give weapons slot 3, then 2, then 1 (each only if the ID ≠ −1), using the same routine as RPC 22.
  6. Call 0x100a0c70. Patch GTA 0x50AC20 with the bytes `C2 08 00` (an early `ret 8`; inferred: disables a GTA routine that would interfere).
  7. Teleport the ped to (x, y, z+0.5) and set the facing angle to the spawn rotation.
  8. Set "spawned" (+0xf0 = 1), clear "dead" (+0xf4 = 0) and the pending request (+0x302 = 0).
  9. **Send RPC 52**, then set a timestamp at LocalPlayer+0x30a = now + 1000 ms (inferred: a grace period after spawn).

### F4 (no RPC; same flag as 74)
- B 0x100074B0, at the end of the local tick (also while dead / in class selection; not while spectating unspawned):
  if LocalPlayer+0x2fa is 0 and `GetAsyncKeyState(VK_F4)` is non-zero (either bit), the flag is set and the type-4
  client line "Returning to class selection after next death" is printed. A second press while the flag is set is
  not read, so its "pressed since the last call" bit re-arms the flag once the death consumed it (measured on the
  original: two presses → one line, then a second line after the class selection).

### 74 ForcePlayerClassSelection (S→C)
- A 0x100180d0 (tiny function, no bitstream), B 0x10018080. **No payload.**
- Client behaviour: only sets LocalPlayer+0x2fa = 1. Nothing visible happens right away. The next time the player dies, the post-death logic sees the flag and enters class selection (0x10004080: clears "class info received", resets the ped, sets health to 100.0, makes the player uncontrollable, restarts the class-selection timers) instead of respawning. The flag is cleared at that point.

### 53 OnPlayerDeath (C→S)
- Send site A 0x10005810, B 0x10005800. Reliability 9.
- Fields (open.mp read order matches):

| field | type | notes |
|---|---|---|
| reason | u8 | weapon/death reason |
| killerId | u16 | 0xFFFF = none |

- Client behaviour: the local player's per-frame routine (A 0x100074c0) detects death when "dead" is not yet set and either the GTA ped state is 54 (dying) or the ped reports dead. It then:
  - stops special states (cellphone, jetpack, carrying, and so on) and sets drunk level 0;
  - makes the player controllable again;
  - if in a vehicle as a passenger, records the vehicle ID;
  - sends RPC 53 and sets "dead" (+0xf4 = 1).
- Reason/killer derivation (A 0x100ac890), from the GTA ped's last-damage weapon type (ped+0x760) and last-damage entity (ped+0x764):
  - weapon < 43, or 50, or 51: the killer is the player that owns the damaging ped, or else the driver of the damaging vehicle. If no player is found, the client sends reason 255 and killer 0xFFFF.
  - weapon 53: killer = driver of the damaging vehicle with reason 49, if there is one; otherwise 255/0xFFFF.
  - weapon 54: killer = driver of the damaging vehicle with reason 54. If the damaging entity is not a vehicle, killer 0xFFFF with reason 54.
  - anything else: 255/0xFFFF.
- After death, once the wasted sequence finishes, the same per-frame routine does one of two things:
  - If class info is valid, no forced class selection is pending, the connection state is 5 (connected) and the ped's health is > 0, it calls Spawn.
  - Otherwise it enters class selection (0x10004080) and clears the force flag.

### 166 PlayerDeath (S→C)
- A 0x10010d40, B 0x10010d20.
- Fields: `playerId` u16.
- Client behaviour: applies only to an existing **remote** player (ID < 1005 with a RemotePlayer object). The local ID is not special-cased, so it is effectively ignored. The remote player routine 0x10017570 does three things:
  - If the remote ped exists: clear its pad keys, take it out of any vehicle task if needed, teleport it to its own position, set health 0, and run opcode 0x0321 (kill char).
  - Set the remote state byte (+0x10a) to 0x20.
  - Reset the remote player's interpolation/sync caches (0x10015fa0).

### 55 SendDeathMessage (S→C)
- A 0x1001dd70, B 0x1001dd20.
- Fields (match open.mp):

| field | type | notes |
|---|---|---|
| killerId | u16 | 0xFFFF = no killer |
| victimId | u16 | 0xFFFF → message dropped |
| reason | u8 | weapon/reason icon |

- Client behaviour:
  - Each ID is resolved to a name and colour. The local ID uses the local name and local colour. A remote ID needs a used slot (< 1004) and uses that player's name and colour; unknown IDs give a null name with colour 0.
  - Calls the kill-list (death window) object at 0x1026ea00 to add an entry (killerName, victimName, killerColour, victimColour, reason). A 0x1006a5d0 is not analysed further here; it belongs to the HUD/UI scope.

---

## Position, physics, health

### 12 SetPlayerPosition (S→C)
- A 0x10019320, B 0x100192d0.
- Fields: `x, y, z` 3×f32.
- Client behaviour:
  - Clear LocalPlayer+0x2e2, +0x2da and +0x2de (surfing/attachment state).
  - Teleport the local ped (ped wrapper method 0x1009f0a0). This calls the GTA entity's virtual teleport (vtable +0x38) with (x, y, z, 0).
  - Exception: for the train models 449, 537 and 538 the wrapper uses opcode 0x07C7 (set mission-train coordinates) on the script handle instead.
  - Ignored if no local player exists.

### 13 SetPlayerPositionFindZ (S→C)
- A 0x10019440, B 0x100193f0.
- Fields: `x, y, z` 3×f32.
- Client behaviour: z is replaced with the ground height at (x, y, z) from opcode 0x02CE (get ground Z for 3D coordinate), plus **1.5**. Then the same steps as RPC 12.

### 19 SetPlayerFacingAngle (S→C)
- A 0x1001a5a0, B 0x1001a550.
- Fields: `angle` f32 (degrees).
- Client behaviour: for the local ped, sets the GTA ped's current and target heading to the angle converted to normalised radians (ped+0x558/+0x55c), then runs opcode 0x0173 (set char heading) with the angle in degrees.

### 90 SetPlayerVelocity (S→C)
- A 0x10018850, B 0x10018800.
- Fields: `vx, vy, vz` 3×f32.
- Client behaviour: if the local ped exists and its GTA flags word (ped+0x46c) has either of the low two bits set (inferred: "on ground" bits), both bits are flipped, so the ped leaves the ground state. The three values are then written directly into the entity's move-speed vector (entity+0x44/+0x48/+0x4c). The values are not scaled; they are GTA move-speed units.

### 14 SetPlayerHealth (S→C)
- A 0x10019550, B 0x10019500.
- Fields: `health` f32.
- Client behaviour: writes the float into the local GTA ped's health field (ped+0x540). No clamping.

### 66 SetPlayerArmour (S→C)
- A 0x1001ab60, B 0x1001ab10.
- Fields: `armour` f32.
- Client behaviour: writes into the local GTA ped's armour field (ped+0x548).

### 15 TogglePlayerControllable (S→C)
- A 0x1001a290, B 0x1001a240.
- Fields: `enable` u8.
- Client behaviour (ped method 0x100ac7d0; only if the ped's script handle is valid):
  - enable = 0: opcode 0x01B4 (set player control) with (slot, 0), then opcode 0x04D7 (freeze char position) with (ped, 1).
  - enable ≠ 0: opcode 0x01B4 with (slot, 1) and opcode 0x04D7 with (ped, 0). If the ped is not in a vehicle (flag 0x100 in ped+0x46c clear), it is also teleported to its own current position.

### 87 ClearPlayerTasks (S→C)
- A 0x10018580, B 0x10018530.
- Fields: `playerId` u16.
- Client behaviour:
  - Target is the local ped if the ID is local; otherwise the ped of a streamed-in remote player, whose "animation applied" flag (+0x1c1) is cleared.
  - The ped is teleported to its own current position (matrix read via 0x1009eb70, then teleport 0x1009f0a0). In GTA a teleport flushes the ped's tasks, which is how the clear happens.

---

## Appearance and identity

### 153 SetPlayerSkin (S→C)
- A 0x10019190, B 0x10019140.
- Fields (0.3.7 / non-DL layout):

| field | type | notes |
|---|---|---|
| playerId | u32 | full 32 bits read; compared with the local ID |
| skin | u32 | ped model ID |

- Client behaviour:
  - If the model is not a valid ped model (0x100b3e10), the client logs "Warning: SetPlayerSkin %d isn't a valid ped model." and stops.
  - Otherwise it applies the model to the local ped, or to a remote player's ped if it is streamed in.
  - Model change (ped method 0x100aff90): invalid models fall back to 0. The client patches GTA 0x5A82C0 with `ret` (0xC3), calls 0x100ae060, sets the model with streaming (0x1009efb0) and calls GTA 0x4E68D0 on the ped (inferred: rebuilds ped data after the model change).
- open.mp writes u16+u32+u32 only for DL clients; the non-DL path (u32 ID, u32 skin) matches the client.

### 11 SetPlayerName (S→C)
- A 0x1001dfd0, B 0x1001df80.
- Fields:

| field | type | notes |
|---|---|---|
| playerId | u16 | |
| nameLen | u8 | the whole RPC is ignored if ≥ 25 |
| name | raw[nameLen] | not NUL-terminated on the wire |
| success | u8 | |

- Client behaviour:
  - If success = 1 and the ID is a known remote player (< 1005), the remote player's stored name is replaced.
  - If the ID is the local player, the local name (PlayerPool+6) is replaced, **regardless of success**.

### 69 SetPlayerTeam (S→C)
- A 0x10019710, B 0x100196c0.
- Fields: `playerId` u16, `team` u8.
- Client behaviour: stores the team in LocalPlayer+0x10c or RemotePlayer+0x109. The damage hook (A 0x100a3bf0) uses it: when Settings+0x22 is set (the NetGame constructor sets it to 1), the local team is not 255, and the remote team equals the local team, damage between the two is cancelled locally and not reported.

### 72 SetPlayerColor (S→C)
- A 0x10019800, B 0x100197b0.
- Fields: `playerId` u16, `colour` u32 (RGBA as written by open.mp).
- Client behaviour: writes the colour into a global per-player colour table (u32 array at 0x101516a0, indices < 4000). This happens for the local ID or for any remote ID with an entry.

### 34 SetPlayerSkillLevel (S→C)
- A 0x1000f5e0, B 0x1000f5b0.
- Fields:

| field | type | notes |
|---|---|---|
| playerId | u16 | |
| skillType | u32 | GTA weapon-skill index; must be < 11 |
| level | u16 | stored as float |

- Client behaviour:
  - Local: the level is stored in the client's local skill array (0x10146a04) and in the GTA stats float array (base pointer at 0x101039cc, +0x114+4·type).
  - Remote, when the ped is streamed in: stored in a per-ped-slot table (slot < 210, 11 floats per slot, at 0x10146d78). The client swaps it in when that ped fires.

### 89 SetPlayerFightingStyle (S→C)
- A 0x10018740, B 0x100186f0.
- Fields: `playerId` u16, `style` u8.
- Client behaviour: on the local ped or a streamed-in remote ped, runs opcode 0x07FE (give melee attack style) with (ped, style, 6).

### 33 SetPlayerShopName (S→C)
- A 0x10017e50, B 0x10017e00.
- Fields: `name` raw[32] (fixed, NUL-padded; open.mp writes the 32-byte array).
- Client behaviour (local ped only):
  - Empty name: clears the GTA ped's shop-name pointer (ped+0x78c).
  - Otherwise, a name of length ≤ 31 is copied to a static 32-byte buffer (0x1026b260) and the ped's pointer is set to it.
  - Then, if the name is ≤ 8 characters long, GTA 0x49BBE0 is called with it (inferred: CShopping shop-data load).

### 35 SetPlayerDrunkLevel (S→C)
- A 0x10018db0, B 0x10018d60.
- Fields: `level` i32.
- Client behaviour: stores the level in the local ped object (+0x2c9). The ped's per-frame routine (A 0x100ade30) does the rest:
  - Above 2000, it runs opcode 0x052C (set player drunkenness) with a value derived from the level and clamped to 5…250. When the player is driving, it also adds a random ±0.012 steering wobble (±0.015 when the level is ≥ 5000) to the vehicle's turn speed at most every 200 ms, but only when the vehicle is moving faster than 0.05. The level then drops by 1.
  - At 2000 or below, the level drops by 1 per tick while it is > 0, and drunkenness is set to 0.
  - (Inferred: the level is also what the client reports in stats sync; not traced in this scope.)

### 88 SetPlayerSpecialAction (S→C)
- A 0x10018690, B 0x10018640.
- Fields: `action` u8.
- Client behaviour: LocalPlayer method 0x100030f0. The values match the public SPECIAL_ACTION_* constants. Observed cases:

| value | effect in the client |
|---|---|
| 0 | stop every active special state: jetpack, cellphone, carry/drink/smoke item, dance, cuffed, carry, piss |
| 2 | give a jetpack if the player is not already using one |
| 5–8 | start dance style 0–3 |
| 10 | hands up |
| 11 | start cellphone (only if not in a vehicle) |
| 13 | stop cellphone |
| 20 / 21 / 22 / 23 | hold item 1 / 4 / 2 / 3 (beer / cigar / wine / sprunk) if not in a vehicle |
| 24 | cuffed on |
| 25 | carry on |
| 68 | pissing (only if not in a vehicle) |
| other | ignored |

### 86 ApplyPlayerAnimation (S→C)
- A 0x1001a950, B 0x1001a900.
- Fields (match open.mp):

| field | type | notes |
|---|---|---|
| playerId | u16 | |
| libLen | u8 | |
| lib | raw[libLen] | animation library |
| nameLen | u8 | |
| name | raw[nameLen] | animation name |
| delta | f32 | |
| loop | bit | |
| lockX | bit | |
| lockY | bit | |
| freeze | bit | |
| time | u32 | ms |

- Client behaviour:
  - Target is the local ped, or a streamed-in remote ped (whose "animation applied" flag is then set).
  - Ignored if the library name equals "SEX" (case-insensitive).
  - If the animation block is not loaded, the client requests it and polls with Sleep(1) up to 15 times. If it is still not loaded, it gives up.
  - Then it runs opcode 0x0812 (task play anim, non-interruptible) with the ped, name, library, delta, loop, lockX, lockY, freeze and time. The four flags are passed in a different internal order, but each keeps its meaning.

---

## Weapons and money

### 22 GivePlayerWeapon (S→C)
- A 0x1001a640, B 0x1001a5f0.
- Fields: `weapon` u32, `ammo` u32.
- Client behaviour (ped method 0x100af7d0, local ped):
  - Needs a valid ped and a weapon with a model.
  - If the weapon model is not loaded, it requests it (priority 2), runs the loader and busy-waits with Sleep(1) until it is loaded.
  - Inside the per-player context swap (0x1009cb90 / 0x1009cd80 / 0x1009cc20; global slot byte at *0x10113978), calls GTA 0x5E6080 (CPed::GiveWeapon) with (weapon, ammo, 1).
  - Then makes that weapon current with the SetArmedWeapon routine (forced variant).

### 21 ResetPlayerWeapons (S→C)
- A 0x100180a0 (thunk to 0x100ac050), B 0x10018050. **No payload.**
- Client behaviour: calls GTA 0x5E6320 (CPed::ClearWeapons) on the local ped, within the player-slot context.

### 67 SetPlayerArmedWeapon (S→C)
- A 0x10018e50, B 0x10018e00.
- Fields: `weapon` u32. Values outside 0…46 are ignored.
- Client behaviour (ped method 0x100ac0a0, not forced): if the ped is on foot, opcode 0x01B9 (set current char weapon). If it is in a vehicle, GTA 0x5E6280 (CPed::SetCurrentWeapon) is called directly.

### 145 SetPlayerAmmo (S→C)
- A 0x1001ac10, B 0x1001abc0.
- Fields: `weapon` u8, `ammo` u16.
- Client behaviour: finds the local GTA ped's weapon slot that holds this weapon (0x100ad1d0) and sets that slot's total ammo (+0xc) to the value. Ignored if the weapon is not carried.

### 18 GivePlayerMoney (S→C)
- A 0x1001a500, B 0x1001a4b0.
- Fields: `amount` i32 (signed delta).
- Client behaviour: opcode 0x0109 (add score) with player 0 and the amount.

### 20 ResetPlayerMoney (S→C)
- A 0x10018090 (thunk to Game method 0x100a2100), B 0x10018040. **No payload.**
- Client behaviour: reads the current money from GTA 0xB7CE50 and adds its negation with opcode 0x0109, so the result is 0. Does nothing if the money is already 0.

---

## World state

### 29 SetPlayerTime (S→C)
- A 0x1000f420, B 0x1000f3f0.
- Fields: `hour` u8, `minute` u8.
- Client behaviour: writes GTA clock hours (0xB70153) and minutes (0xB70152), runs opcode 0x00C0 (set time of day), and stores both values in Settings+0x2c and +0x2d.

### 94 SetPlayerWorldTime (S→C)
- A 0x1000eef0, B 0x1000eec0.
- Fields: `hour` u8.
- Client behaviour: only stores the hour in Settings+0x2c (the same field InitGame fills). Inferred: the NetGame per-frame code applies it while the server controls the time.

### 30 TogglePlayerClock (S→C)
- A 0x1000f500, B 0x1000f4d0.
- Fields: `enable` u8.
- Client behaviour (Game method 0x100a14a0):
  - enable ≠ 0: un-patch GTA's clock update (byte at 0x52CF10 set to 0x56, so the routine runs), restore the HUD clock format string at 0x859A6C to `%02d:%02d`, set Game+0x69 = 1, and clear Settings+0x1f.
  - enable = 0: patch 0x52CF10 to `ret` (0xC3) so the GTA clock stops, blank the HUD clock string, clear Game+0x69, set Settings+0x1f = 1, and copy the current GTA hour/minute into Settings+0x2c/+0x2d. Inferred: the time is then held at that value.

### 60 SendGameTimeUpdate (S→C)
- A 0x1000ff30, B 0x1000ff10.
- Fields: `time` i32.
- Client behaviour: only when the HUD clock is **disabled** (Game+0x69 = 0), writes `time & 0x3FFFFFFF` to GTA 0xB7CB84 (inferred: CTimer millisecond counter / game-clock base) and sets a client flag at 0x1015051c. Ignored when the clock is enabled.

### 152 SetPlayerWeather (S→C)
- A 0x1000f370, B 0x1000f340.
- Fields: `weather` u8.
- Client behaviour: stores the value in Settings+0x2e and writes it to GTA 0xC81318 (inferred: forced weather). If the clock is disabled (Game+0x69 = 0), it is also written to 0xC8131C and 0xC81320 (old/new weather), so the change is immediate. With the clock enabled, GTA blends into it.

### 146 SetPlayerGravity (S→C)
- A 0x1001acd0, B 0x1001ac80.
- Fields: `gravity` f32.
- Client behaviour: un-protects and writes the float to GTA 0x863984 (the global gravity). InitGame also applies Settings+0x16 the same way.

### 17 SetWorldBounds (S→C)
- A 0x1001a410, B 0x1001a3c0.
- Fields (open.mp Vector4 order):

| field | type | stored at |
|---|---|---|
| maxX | f32 | Settings+0x05 |
| minX | f32 | Settings+0x09 |
| maxY | f32 | Settings+0x0d |
| minY | f32 | Settings+0x11 |

- Client behaviour: only stores the four floats. The defaults, ±20000.0 (set by 0x100088f0), mean "no bounds". The per-frame enforcement code that reads them was not traced (unknown).

### 133 SetPlayerWantedLevel (S→C)
- A 0x1001ccf0, B 0x1001cca0.
- Fields: `level` u8.
- Client behaviour: if Game exists, writes the byte to GTA address 0x58DB60. That is inside GTA's HUD code (an immediate operand), so this is a **display-only** wanted level; GTA's police logic is not changed. (Inferred from the address being in .text.)

### 48 SetPlayerVirtualWorld (S→C)
- A 0x1001dcc0, B 0x1001dc70.
- Fields: `worldId` i32.
- Client behaviour: if the object at 0x1026ea10 exists and the value differs from its field +0x217, the value is stored there and two byte flags (+0x220, +0x221) are cleared. The only reader found (0x10008a60) checks whether +0x217 ≠ 0 to decide on a "freeze the player and camera while something is pending" step.
- **Unknown:** what the 0x1026ea10 object is. Inferred: a world-dependent loader/streaming manager. Streaming of other entities by virtual world is server-side in 0.3.7.

### 104 EnableStuntBonusForPlayer (S→C)
- A 0x10017d50, B 0x10017d00.
- Fields: `enable` bit.
- Client behaviour: un-protects and writes the value as u32 to GTA 0xA4A474 (inferred: GTA's stunt-bonus enable flag).

### 111 ToggleWidescreen (S→C)
- A 0x10018ac0, B 0x10018a70.
- Fields: `enable` u8.
- Client behaviour: logs "Widescreen = %i" to the client log and runs opcode 0x02A3 (switch widescreen) with the value.

### 16 PlayerPlaySound (S→C)
- A 0x1001a330, B 0x1001a2e0.
- Fields: `soundId` u32, `x, y, z` 3×f32.
- Client behaviour (Audio object method 0x100a2300). The Audio object keeps two state words: [0] "mission audio playing" and [1] a byte flag.

| soundId | action |
|---|---|
| 0 | if mission audio is playing, opcode 0x040D (clear mission audio slot 1) and clear [0]; call GTA 0x4D6D60 (inferred: stop the current frontend track); clear the byte flag |
| 1 | set the byte flag (meaning unknown) |
| 2…999 | GTA 0x4D6D50(soundId) (inferred: start a frontend/music track) |
| 1000…1999 | opcode 0x018C (add one-off sound) at (x, y, z) with this ID |
| ≥ 2000 | opcode 0x040D on slot 1, opcode 0x03CF (load mission audio slot 1, soundId), opcode 0x03D7 (set mission audio position, slot 1, x, y, z); set [0] = 1 |

- Class selection uses this routine itself (sounds 1052/1053, see RPC 128).

### 79 CreateExplosion (S→C)
- A 0x1001bd10, B 0x1001bcc0.
- Fields (match open.mp): `x, y, z` 3×f32, `type` u32, `radius` f32.
- Client behaviour: opcode 0x0948 (descriptor "fffii") with (x, y, z, type, radius). The radius float is passed through as-is.

### 43 RemoveBuildingForPlayer (S→C)
- A 0x1001d530, B 0x1001d4e0.
- Fields (match open.mp): `modelId` u32, `x, y, z` 3×f32, `radius` f32.
- Client behaviour (0x1009d430):
  - Applies the removal to the current world at once (0x1009d1e0, internals not analysed; modelId −1 is the public "all models" convention).
  - Appends {model, x, y, z, radius} to a list (entries at 0x1014ade0, 20 bytes each, count at 0x1014fc00). The list has room for 1000 entries, and **there is no bounds check** on the count. Inferred: the list is replayed for buildings that stream in later.
  - A Rust client should cap the list at 1000 entries.

### 112 PlayCrimeReport (S→C)
- A 0x10019050, B 0x10019000.
- Fields (match open.mp):

| field | type | notes |
|---|---|---|
| suspectId | u16 | read but **unused** by the client |
| inVehicle | u32 | |
| vehicleModel | u32 | |
| vehicleColour | u32 | |
| crimeId | u32 | |
| x, y, z | 3×f32 | |

- Client behaviour (Game method 0x100a17d0):
  - Clears a client buffer (0x10150b78, 0x7a0 bytes). NOPs 39 bytes at GTA 0x4E7529 (inferred: removes a check in the police-scanner code).
  - If inVehicle ≠ 0 and 400 ≤ model ≤ 611, it builds a temporary vehicle object of that model, applies the colour (0x100b7320(colour, −1)) and records it in client globals, so that the scanner can describe the vehicle.
  - If the local ped exists, calls GTA 0x4E71E0 with (0xA4, crimeId, &pos) (inferred: police-scanner report).
  - Finally destroys the temporary vehicle.

### 167 DisableRemoteVehicleCollisions (S→C)
- A 0x10017de0, B 0x10017d90.
- Fields: `disable` bit.
- Client behaviour: stores the bit in NetGame+0x232. The collision code that reads it was not traced.

---

## Interior

### 156 SetPlayerInterior (S→C)
- A 0x10019a00, B 0x100199b0.
- Fields: `interior` u8.
- Client behaviour: local ped method 0x100ad380 (interior, refresh = 1). For the local ped (slot 0) it runs:
  - opcode 0x04BB (select interior) with the ID;
  - opcode 0x0860 (set char area visible) with (ped, ID);
  - because refresh = 1, it reads the ped position and runs opcode 0x04E4 (refresh/load scene at x, y).

  For non-local peds only the area-visible opcode is used.

### 118 OnPlayerInteriorChange (C→S)
- Send site A 0x10005970, B 0x10005960. Reliability 8.
- Fields: `interior` u8.
- Client behaviour: the local per-frame routine reads GTA's current area ID (0x100a0f20). When it differs from the last reported value (LocalPlayer+0x1cf), the client stores the new value and sends this RPC. So it is sent on **any** interior change, including ones caused by RPC 156.

---

## Camera

All camera RPCs act on the Camera object (Game+4). Its first field holds the entity the camera is
attached to (0 = none).

### 157 SetPlayerCameraPosition (S→C)
- A 0x10019aa0, B 0x10019a50.
- Fields: `x, y, z` 3×f32.
- Client behaviour (0x1009d7e0): opcode 0x0925, clear the attachment, then opcode 0x015F (set fixed camera position) with (x, y, z, 0, 0, 0).

### 158 SetPlayerCameraLookAt (S→C)
- A 0x10019b70, B 0x10019b20.
- Fields: `x, y, z` 3×f32, `cut` u8. A cut of 0 or > 2 is replaced with 2.
- Client behaviour (0x1009d840): opcode 0x0925, clear the attachment, then opcode 0x0160 (point camera at point) with (x, y, z, cut).

### 162 SetPlayerCameraBehindPlayer (S→C)
- A 0x10018080 (thunk to Camera method 0x1009d7a0), B 0x10018030. **No payload.**
- Client behaviour: opcodes 0x0930(0) and 0x0925, clear the attachment, then 0x0373 (set camera behind player) and 0x02EB (restore camera jumpcut).

### 82 InterpolateCamera (S→C)
- A 0x1001a0f0, B 0x1001a0a0.
- Fields (match open.mp):

| field | type | notes |
|---|---|---|
| isPosition | bit | 1 = interpolate camera position, 0 = interpolate look-at |
| from | 3×f32 | |
| to | 3×f32 | |
| time | i32 | ms; ≤ 0 → RPC ignored |
| cut | u8 | 0 or > 2 → replaced with 2 |

- Client behaviour (only if time > 0): sets LocalPlayer+0x314 = 1 (camera under script control). Then:
  - isPosition = 1: Camera method 0x1009d5f0 runs opcodes 0x0925 and 0x0930, then opcode 0x0936 (vector move) with (from, to, time, cut).
  - isPosition = 0: 0x1009d660 runs opcode 0x092F(1), then opcode 0x0920 (vector track) with (from, to, time, cut).

### 81 AttachCameraToObject (S→C)
- A 0x10019ff0, B 0x10019fa0.
- Fields: `objectId` u16.
- Client behaviour: only if the ID is < 1001 and the object exists in the object pool (Pools+0xc).
  - Set LocalPlayer+0x314 = 1.
  - Store the object's GTA entity (object pool +0xfa4+4·id) in the camera's attachment field (0x1009d6c0).
  - If the entity is non-null and its position passes the coordinate validator, snap the camera there with a 100 ms vector move (cut 1) from the position to itself.
  - The follow logic is per-frame (not traced).

### 170 SetPlayerCameraTargeting (S→C)
- A 0x1000ea20, B 0x1000e9f0.
- Fields: `enabled` bit.
- Client behaviour: stores the bit in NetGame+0x233. This enables or disables RPC 168.

### 168 OnPlayerCameraTarget (C→S)
- Send site A 0x10004700, B 0x100046e0. Reliability 9.
- Fields (written as one raw[8] block, which equals 4×u16 LE; matches open.mp):

| field | type | notes |
|---|---|---|
| objectId | u16 | 0xFFFF = none |
| vehicleId | u16 | 0xFFFF = none |
| playerId | u16 | 0xFFFF = none |
| actorId | u16 | 0xFFFF = none |

- Client behaviour:
  - Called every local-player tick. If NetGame+0x233 is 0, the cached targets are set to 0xFFFF and nothing is sent.
  - Otherwise, at most every 100 ms, it computes targets (0x10004440). A ray starts at the camera source and runs 70 units along the camera front vector. It is shortened by a GTA line-of-sight test (0x100b3b50) with different collision flags for each category. The client then searches:
    - the object pool (radius 40.0);
    - the vehicle pool (radius 40.0, tolerance 1.0);
    - the player pool (radius 20.0, tolerance 0.75);
    - the actor pool (radius 20.0, tolerance 0.75).
  - The RPC is sent only when one of the four IDs differs from the last sent set.
  - Details (build B 0x10004420, FEAT-2, verified against the original — docs/phase3/feata-notes.md): the ray runs
    from camera source + front (one unit ahead) to source + 70·front (front 0xB6F32C, source 0xB6F338). Before each
    category the end is replaced by the hit point of `CWorld::ProcessLineOfSight` from the start with these flags:
    objects — buildings only; vehicles — buildings and objects; players (and then actors, same end) — buildings,
    vehicles and objects. Objects use the pool ray test of selection mode (B 0x10012E40) with 40 as its start
    distance; vehicles and peds need a live entity (not a deleted placeable, in the moving list) closer to the
    camera position (0xB6F9CC) than 40 (vehicles) / 20 (peds), not the vehicle the local ped sits in, and a hit of
    the bounding-sphere line test (B 0x100B65E0 with the frustum check; radius scale 1.0 / 1.5); the hit nearest to
    the ray start wins. The "tolerance" arguments (1.0 / 0.75) are copied into the line's unused fourth components.
  - Called only in the spawned branch of the local tick, after the interior check (RPC 118).

---

## Damage, clicks, script events

### 115 OnPlayerGiveTakeDamage (C→S)
- Send sites A 0x100068a0 (taking = 1) and 0x100069b0 (taking = 0); B 0x10006890 / 0x100069a0. Reliability 9.
- Fields (match open.mp read order):

| field | type | notes |
|---|---|---|
| taking | bit | 1 = local player was damaged; 0 = local player damaged someone |
| playerId | u16 | other player; 0xFFFF if the attacker is not a player |
| damage | f32 | **GTA damage value × 0.33** (scaled on the client before sending) |
| weaponId | u32 | |
| bodypart | u32 | |

- Client behaviour: both are sent from the GTA damage-event hook (A 0x100a3bf0). It receives GTA's damage info: source entity, damage f32, body part, weapon. Then:
  - **Local ped is the victim:** send with taking = 1 and the attacker's player ID. The report is skipped when the attacker entity is the vehicle the local player is surfing on (LocalPlayer+0x2da), and when the weapon is 54 while surfing.
  - **Local ped is the attacker and the victim is a remote player:** send with taking = 0.
  - **Local ped is the attacker and the victim is an actor:** RPC 177 is sent instead (actor scope).
  - After reporting, the hook cancels the damage locally (returns "ignore") in two cases: the victim is a remote player with a non-zero field +0x10c (inferred: "invulnerable/remote handles its own health"), or the same-team rule from RPC 69 applies. Whether GTA applies the damage locally is a separate matter from the report.

### 119 OnPlayerClickMap (C→S)
- Send site inside A 0x10075790 (a per-frame UI routine), B 0x10075760. Reliability 8.
- Fields (open.mp `readPosVEC3`): `x, y, z` 3×f32.
- Client behaviour: sent when all of these hold:
  - the GTA pause menu is active (0xBA67A4 ≠ 0);
  - the map page is shown (0xBA6774 ≠ 0);
  - the right mouse button was just pressed (GetAsyncKeyState(VK_RBUTTON) & 1);
  - 0xBA8298 is 0.

  The client then scans 175 GTA radar-blip entries starting at 0xBA873D (= entries 1..175 of the radar trace array 0xBA86F0, stride 0x28, field +0x25) for the entry tagged 0x1203 (the user waypoint; the last match wins). It takes x and y from that entry and sets z = GTA ground height at (x, y) (GTA 0x569660, through a pointer at 0x10102bf4) + 2.0. If no waypoint entry is found, x/y are uninitialised stack values (a client quirk; a Rust client should simply not send).

### 23 OnPlayerClickPlayer (C→S)
- Send site A 0x1006ec10, B 0x1006ebd0. Reliability 9.
- Fields (match open.mp): `playerId` u16, `source` u8 (always 0 = scoreboard).
- Client behaviour: a scoreboard method. When a row is chosen, the client takes the ID from that row's ID column (text converted with atol), sends the RPC, and then closes the scoreboard (restores input/cursor state).
- "Chosen" = DXUT list-box event 0x701 in the scoreboard's GUI callback (B 0x100C5590): a double click on a row. The
  list never has keyboard focus (measured: Down/Enter do nothing). The close passes (0, 0) to the cursor-mode
  setter: cursor hidden, input restored after the 10-frame countdown.

### 96 SCMEvent (S→C)
- A handler 0x1000f280, B 0x1000f250.
- Fields (match open.mp write order):

| field | type | notes |
|---|---|---|
| playerId | u16 | player who caused the event |
| eventType | u32 | 1…4 |
| vehicleId | u32 | |
| arg1 | u32 | |
| arg2 | u32 | |

- Client behaviour (0x10001a60). Ignored unless NetGame, the player pool and the vehicle pool exist and the flag at 0x100fe0a0 is 0. Then:

| eventType | action on the vehicle (if it exists) |
|---|---|
| 1 | apply paintjob arg1 (0x100b8130) |
| 2 | add component arg1 (0x100b8000) |
| 3 | set colours (arg1, arg2) (0x100b7320) |
| 4 | mod-shop enter/exit, see below |

  Event 4 (mod-shop enter/exit): the client acts only if all of these hold: the sender is a streamed-in remote player, both peds are in a vehicle, the sender is the driver and the local player a passenger, and it is the same vehicle. Then:
  - arg1 ≠ 0 (enter): set the vehicle's interior to arg2, move the local ped to interior arg2 (without refresh), and make the local player uncontrollable.
  - arg1 = 0 (exit): interior 0, controllable again.

### 96 SCMEvent (C→S)
- Send site A/B 0x10001ce0. Reliability **10** (RELIABLE_SEQUENCED).
- Fields (match open.mp read order):

| field | type | notes |
|---|---|---|
| vehicleId | u32 | SA:MP vehicle ID (converted from the GTA vehicle; the RPC is not sent if there is no ID) |
| arg1 | u32 | |
| arg2 | u32 | |
| eventType | u32 | 1 paintjob, 2 component, 3 respray colours, 4 mod-shop enter(arg1 = 1)/exit(0) |

- Client behaviour:
  - Event 3: after sending, the colours are also applied locally.
  - Event 4: the vehicle's "in mod shop" flag (+0x71) is set on enter. On exit it is cleared and the vehicle's stored colours are re-applied.
  - No direct callers were found in the decompilation. Inferred: it is called from hooks on GTA's garage/mod-shop script handlers.
- Note: the field order differs between the two directions (S→C puts eventType first, C→S puts it last). open.mp agrees with the client for both.

---

## open.mp vs client — disagreements and notes

1. **RPC 128 C→S (PlayerRequestClass):** the client writes **u32** classIndex; open.mp reads **u16**. Wire-compatible (LE low half). Send u32.
2. **RPC 129 S→C (spawn response / ImmediatelySpawnPlayer):** open.mp writes **u32**; the client reads only **u8**. Compatible (LE low byte). A Rust client should read 8 bits and ignore the rest.
3. RPC 115: the client scales GTA damage by 0.33 before sending. open.mp stores the float as received. This is not a layout difference, but a reimplementation must apply the same scaling.
4. RPC 13: the client adds 1.5 to the found ground Z. RPC 119: the client adds 2.0 to the ground Z.
5. RPC 82 / 158: cut values 0 or > 2 become 2. RPC 82 with time ≤ 0 is ignored entirely.
6. RPC 11: the local name is overwritten even when success = 0. Names of ≥ 25 bytes cause the whole RPC to be dropped.
7. RPC 112: suspectId is read and ignored.
8. RPC 74 does not open class selection immediately; it takes effect at the next death.
9. RPC 133 changes only the displayed wanted level (HUD patch), not GTA's police state.

## Unknown / not traced
- Per-frame use of the world bounds (Settings+5…+0x11), of NetGame+0x232 (remote vehicle collisions), and of the camera attachment.
- The object at 0x1026ea10 that stores the virtual world.
- Exact identity of GTA 0x4D6D50 / 0x4D6D60 (sound IDs 2…999 / stop) and 0x4E71E0 (crime report); the descriptions above are inferred.
- Internals of the building-removal routine 0x1009d1e0.
- Who calls the SCMEvent sender 0x10001ce0 (likely GTA hook stubs).

---

## RE-10 additions (static, 2026-11-04)

* **43 RemoveBuildingForPlayer**: internals of A 0x1009D1E0 and the IPL replay (model 19300) are now in
  `world-entities.md` RE-10 additions §7.
* **166 PlayerDeath / 178**: opcode 0x0321 is `EXPLODE_CHAR_HEAD` (damage event through the 0x4B5AC0 hook), see
  `remote-control-combat.md` RE-10 additions §5.
* **34 SetPlayerSkillLevel**: the starting values are 1000 (first frame, `CCheat::WeaponSkillsCheat`) and 999 after the
  session reset of RPC 40; `CStats::SetStatValue` is disabled (CSV row 356). See `remote-control-combat.md` RE-10 §4.
* **167 DisableRemoteVehicleCollisions**: consumer = the ProcessEntityCollision hooks (`remote-control-vehicles.md` §2.4).
* **15 / 88 pad masks**: carry and cuffs block jump/crouch/sprint through the local ped object's +0x2F6/+0x2F7/+0x2F8
  (`remote-control-combat.md` RE-10 §1).
