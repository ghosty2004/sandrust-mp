# Remote players and sync packets (SA:MP 0.3.7-R4 client) — clean-room spec

Overview. This file covers how the client learns about other players (join/quit, stream in/out, name, chat bubble, name-tag visibility, score/ping table, spectating) and the whole family of high-rate "sync" packets 200–212 in both directions. The central findings: (1) client→server sync packets are **not** bit-packed — each is `u8 packetId` followed by a fixed-size raw copy of an internal struct (byte-aligned only by coincidence), sent unreliable-sequenced; (2) server→client foot and driver sync use the compact encodings (conditional fields, packed health/armour nibbles, RakNet "normalised quaternion" and "compressed vector"), while the other S→C sync packets are the raw struct again; (3) incoming sync is only *stored* by the packet handler; a per-frame remote-player update (A 0x100166b0) applies it with simple proportional velocity correction plus teleport thresholds and a 0.25-per-frame rotation slerp. All addresses are build A (2021) unless marked B; B was spot-checked (dispatch, read/write sequences, sizes, reliabilities, interpolation constants) and is identical.

| ID | open.mp name | dir | A handler / send site | confidence |
|---|---|---|---|---|
| RPC 137 | PlayerJoin | S→C | 0x1000fce0 | high |
| RPC 138 | PlayerQuit | S→C | 0x1000fe70 | high |
| RPC 32 | PlayerStreamIn | S→C | 0x10010b00 | high |
| RPC 163 | PlayerStreamOut | S→C | 0x10010e00 | high |
| RPC 11 | SetPlayerName | S→C | 0x1001dfd0 | high |
| RPC 59 | SetPlayerChatBubble | S→C | 0x1000f990 | high |
| RPC 80 | ShowPlayerNameTagForPlayer | S→C | 0x1001be20 | high |
| RPC 155 | SendPlayerScoresAndPings / OnPlayerRequestScoresAndPings | S→C / C→S | 0x100103e0 / send 0x10008f10 | high |
| RPC 124 | TogglePlayerSpectating | S→C | 0x1001c350 | high |
| RPC 126 | PlayerSpectatePlayer | S→C | 0x1001c400 | high |
| RPC 127 | PlayerSpectateVehicle | S→C | 0x1001c4e0 | high |
| pkt 200 | PlayerVehicleSync (driver) | both | recv 0x1000aa10 / send 0x10007080 | high |
| pkt 201 | RCON command | C→S | send 0x10069030 | high |
| pkt 203 | PlayerAimSync | both | recv 0x10009c40 / send 0x10005210 | high |
| pkt 204 | PlayerWeaponsUpdate | C→S | send 0x10006290 | high |
| pkt 205 | PlayerStatsSync | C→S | send 0x10005d00 | high |
| pkt 206 | PlayerBulletSync | both | recv 0x10009b40 / send 0x10006bc0 | high |
| pkt 207 | PlayerFootSync | both | recv 0x1000a740 / send 0x10004f00 | high |
| pkt 208 | PlayerMarkersSync | S→C | recv 0x10009f10 | high |
| pkt 209 | PlayerUnoccupiedSync | both | recv 0x10009a40 / send 0x10004d30 | high |
| pkt 210 | PlayerTrailerSync | both | recv 0x10009e20 / send 0x100053d0 | high |
| pkt 211 | PlayerPassengerSync | both | recv 0x10009d30 / send 0x10005590 | high |
| pkt 212 | PlayerSpectatorSync | C→S | send 0x10006540 | high |

B counterparts: packet dispatch B 0x1000b250 (200→0x1000aa00, 203→0x10009c30, 206→0x10009b30, 207→0x1000a730, 208→0x10009f00, 209→0x10009a30, 210→0x10009e10, 211→0x10009d20); senders B 207 0x10004ef0, 203 0x10005200, 204 0x10006280, 205 0x10005cf0, 206 0x10006bb0, 209 0x10004d10, 210 0x100053c0, 211 0x10005580, 212 0x10006530, 200 0x10007070. RPC B handlers per rpc-ids.md.

---------------------------------------------------------------------------------------------------

## 0. Common conventions and primitives

Bitstream semantics as in handshake.md (MSB-first bits, LE multi-byte, nothing aligned). "raw[N]" = N bytes copied with the generic byte reader/writer (A 0x1001fea0 read / 0x10020090 write); when the stream is not on a byte boundary this degrades to an N×8-bit read, so raw blocks are **not** aligned either. All floats are IEEE-754 f32 LE.

### 0.1 Packet framing

* Sync packets are plain RakNet user packets: first byte = ID (200–212). Incoming handlers (dispatched by A 0x1000b260 only when netgame state == 5 "connected", checked inside each handler at +0x3cd) read `u8 id`, `u16 playerId`, then the body.
* Only the **foot (207)** and **driver (200)** handlers honour an optional leading `0x28` + `u32 timestamp` (RakNet ID_TIMESTAMP): if byte 0 is 0x28 they read u8 0x28, u32 time, then u8 id. The timestamp is used for reordering (see §2.1). Other sync handlers do not skip it — a server must not prefix those.
* Every incoming sync handler drops the packet unless `playerId < 1005` (0x3ed) and the player-pool slot holds a remote player object with a remote-player sub-object; bullet (206) and markers (208) use `< 1004` (0x3ec) and the "slot in use" table instead.
* Outgoing: built in a fresh bitstream (`u8 id` via the N-bit writer, then the body) and sent with RakClient vtable +0x18 `Send(bs, priority, reliability, orderingChannel)`; no timestamp prefix.

| pkt | priority | reliability (shifted enum, handshake.md §3) | channel |
|---|---|---|---|
| 200, 203, 207, 209, 210, 211 | 1 (HIGH) | 7 UNRELIABLE_SEQUENCED | 1 |
| 206 bullet | 1 | 7 UNRELIABLE_SEQUENCED | 0 |
| 204 weapons, 205 stats | 1 | 6 UNRELIABLE | 0 |
| 212 spectator | 1 | 6 UNRELIABLE | 1 |
| 201 rcon | 1 | 8 RELIABLE | 0 |

### 0.2 Quantised encodings (decoders observed in A; client never encodes these)

**Normalised quaternion** (reader A 0x10009680, B 0x10009670) — used in S→C 207 and 200:

| # | field | width |
|---|---|---|
| 1 | wNeg | bit |
| 2 | xNeg | bit |
| 3 | yNeg | bit |
| 4 | zNeg | bit |
| 5 | qx | u16 |
| 6 | qy | u16 |
| 7 | qz | u16 |

Decode: `x = qx / 65535` (constant 1.5259022e-05 = 1/65535), same for y, z; apply the sign bits; `w = sqrt(max(0, 1 − x² − y² − z²))`, negated if wNeg. Output order is **(w, x, y, z)** — this is the in-memory order of every quaternion in the client's sync structs. Encoder implied for a server/test harness: `qx = trunc(|x| * 65535)` (stock RakNet behaviour; truncation vs rounding not observable from the client).

**Compressed vector** (reader A 0x1000a3b0, B 0x1000a3a0) — velocity in S→C 207 and 200:

| # | field | width | note |
|---|---|---|---|
| 1 | magnitude | f32 | |
| 2–4 | cx, cy, cz | u16 each | **present only if magnitude > 1e-5** |

Decode: if `magnitude <= 1e-5` → vector (0,0,0) and nothing more is read. Otherwise each component `= (c / 32767.5 − 1) * magnitude` (constant 3.0518044e-05 = 1/32767.5). Implied encoder: `c = (clamp(v_i/|v|, −1, 1) + 1) * 32767.5` truncated to u16.

**Packed health/armour byte** (inline in 207 and 200 readers): one u8; **high nibble = health, low nibble = armour**. Nibble n decodes to `0 if n==0; 100 if n==15; 7*n otherwise` (7..98). Stored as floats for name-tag bars. (Implied encoder: n = 15 if value ≥ 100 else value/7 — not observable from client.)

**Weapon byte**: low 6 bits weapon id, high 2 bits "additional key". The S→C readers keep only the low 6 bits (additional key is discarded on receive).

### 0.3 Key word (u16) built by the client (A 0x100af610)

From the game pad state (analog sticks are signed shorts):

| bit | value | source | SA-MP name |
|---|---|---|---|
| 0 | 1 | LeftShoulder1 | ACTION |
| 1 | 2 | left stick click (ShockL) | CROUCH |
| 2 | 4 | Circle — only if current weapon is melee (<16), parachute (46), or has ammo | FIRE |
| 3 | 8 | Cross | SPRINT |
| 4 | 16 | Triangle — suppressed on foot while a particular task (vtable 0x8705c4) is active | SECONDARY_ATTACK |
| 5 | 32 | Square | JUMP |
| 6 | 64 | RightShoulder2 — only while in a vehicle | LOOK_RIGHT |
| 7 | 128 | RightShoulder1 | HANDBRAKE / AIM |
| 8 | 256 | LeftShoulder2 — only while in a vehicle | LOOK_LEFT |
| 9 | 512 | right stick click (ShockR) | SUBMISSION |
| 10 | 1024 | ped-walk flag | WALK |
| 11 | 2048 | right stick Y < 0 | ANALOG_UP |
| 12 | 4096 | right stick Y > 0 | ANALOG_DOWN |
| 13 | 8192 | right stick X < 0 | ANALOG_LEFT |
| 14 | 16384 | right stick X > 0 | ANALOG_RIGHT |

`leftRight` / `upDown` = raw left-stick X / Y shorts. "Additional key" (A 0x100abc30): 1 if pad DPadRight, else 2 if DPadLeft, else 3 if DPadDown, else 0 (server maps these to YES/NO/CTRL_BACK). Incoming keys are applied to a remote ped's pad via A 0x100af380(keys, lr, ud) (inverse mapping).

### 0.4 Send-rate machinery (local player tick A 0x100074c0)

Globals from InitGame (handshake.md §5 fields 21–25): onFootRate A 0x100fe0a8, inCarRate 0x100fe0ac, weaponRate 0x100fe0b0, multiplier 0x100fe0b4 (defaults 30, 30, 30, 2 in .data; the multiplier is stored but never read in A). Lag-comp selector A 0x10118990: InitGame value 1 → 0, 2 → 1, anything else → 2.

* `onFootInterval()` (A 0x10003af0) = 1000 ms if the local ped object does not exist; 15 ms in LAN mode (netgame +0x239); else `onFootRate + N` ms, where N = number of game player-ped slots currently allocated (slots 2..209 of the table at A 0x101501b8, counted by A 0x100a0150; i.e. roughly "streamed-in remote players").
* `inCarInterval()` (A 0x10003ab0) = same with inCarRate.
* **Duplicate suppression** (A 0x10003a60 and inline copies in each sender): a shared timestamp (local-player +0x13f) holds the time of the last foot/aim/driver/passenger/trailer send. If less than 501 ms have elapsed **and** the new struct is byte-identical to the last one sent *of that type*, nothing is sent. Otherwise the packet is sent and both the timestamp and the per-type copy are updated. (Note: because the timestamp is shared, a burst of one type can keep suppressing identical packets of another.)

Per-tick decisions (only when spawned and not in class selection; ped state from the game ped, 50 = driving):

| local situation | packets | rule |
|---|---|---|
| driver (state 50, not passenger) | 200 | when elapsed > inCarInterval (timestamp +0x10f) |
| driver | 210 | immediately after each 200, if a trailer is attached (id 1..1999) |
| driver | 203 | after each 200 if vehicle model is 432, 564, 407 or 601 (armed vehicles); otherwise at most every 1000 ms |
| driver | 203 | additionally from the tick every 1000 ms (timestamp +0x117) |
| passenger | 211 | when elapsed > inCarInterval |
| passenger | 203 | every 1000 ms, or every 100 ms while drive-by is active |
| on foot (ped state 1 or 12) | 207 | when keys/lr/ud differ from the last sent foot struct, or elapsed > onFootInterval |
| on foot | 203 | if keys & (FIRE\|AIM): every 100 ms (lag-comp selector == 2: every weaponRate ms); else every 1000 ms (attempt; there is no forced resend: an unchanged 203 passes the duplicate gate only when the shared timestamp is ≥ 501 ms old, i.e. on a frame between the last 207 send + 501 ms and the next 207 attempt. Measured idle on the original at ~19 ms frames: one 203 every 10.08 s, sent 512–516 ms after the last 207 — NET-6 notes §4) |
| spectating (+0x108) | 212 | when elapsed > onFootInterval (timestamp +0x113) |
| spectating | 203 | only in a tick that just sent a 212, when elapsed > 2 × onFootInterval since the last 203 attempt (the shared timestamp +0x117); then the duplicate gate: ~532 ms apart while nothing changes (A 0x10006540) |
| always (alive) | 205 | every > 1000 ms (timestamp +0x11b) |
| always (alive) | 204 | evaluated every tick, see §204 |
| on death while driving | 200 | one forced send |
| hitscan shot fired | 206 | immediately per bullet |
| unoccupied vehicles near you | 209 | see §209 (vehicle pool tick) |

---------------------------------------------------------------------------------------------------

## 1. Remote player lifecycle RPCs

### 137 PlayerJoin (S→C)
- A 0x1000fce0, B 0x1000fcc0.

| # | field | type |
|---|---|---|
| 1 | playerId | u16 |
| 2 | colour | u32 (RGBA) |
| 3 | isNPC | u8 |
| 4 | nameLen | u8 |
| 5 | name | raw[nameLen] |

- Behaviour (observed): name NUL-terminated; ignored entirely if strlen(name) > 24. Creates the pool entry (A 0x10014250): only if id < 1005; allocates the remote-player object, records its id, sets the NPC flag (remote +0x10c) when isNPC != 0, marks slot in use, recomputes the highest used id (A 0x10013750). If colour != 0 and id < 1004, sets the player's colour (A 0x10016150). No chat message is printed.
- open.mp agrees.

### 138 PlayerQuit (S→C)
- A 0x1000fe70, B 0x1000fe50.

| # | field | type |
|---|---|---|
| 1 | playerId | u16 |
| 2 | reason | u8 |

- Behaviour: pool remove (A 0x10014090), id < 1004 and slot in use: if the local player is spectating this player, spectating is turned off (A 0x100041c0(0) — see 124). Slot cleared, object destroyed (ped deleted), highest id recomputed. `reason` is read but **unused** by the client (no message).

### 32 PlayerStreamIn (S→C)
- A 0x10010b00, B 0x10010ae0.

| # | field | type |
|---|---|---|
| 1 | playerId | u16 |
| 2 | team | u8 |
| 3 | skin | u32 |
| 4 | x, y, z | 3 × f32 |
| 5 | facingAngle | f32 (degrees) |
| 6 | colour | u32 (RGBA) |
| 7 | fightingStyle | u8 (default 4 if absent) |
| 8 | skillLevels | 11 × u16 (raw[22]) |

- Behaviour: requires id < 1004, slot in use, remote object present. Spawn (A 0x10017130): validates the skin model (fails → prints "Warning: Couldn't spawn player(%u)"); deletes an existing ped; allocates a game player-ped slot and creates the ped at pos/angle (A 0x100a0170, NPC flag passed through); applies colour if non-zero; removes any marker-sync blip (§208); creates the player's radar blip if the server's marker mode (netgame settings +0x30) is non-zero; sets synced health to 100; zeroes pad state; applies fighting style if != 4; remote state := 33 (SPAWNED). Then sets the 11 weapon skill levels on the ped (A 0x100ae490), stores team (remote +0x109) unless 255, and if the remote-entry flag at +8 is set calls a ped helper (A 0x100abd40(0,1,1,0,0); purpose unknown).
- open.mp: extra u32 CustomSkin only for DL clients — 0.3.7 clients must receive the non-DL layout. Otherwise identical.

### 163 PlayerStreamOut (S→C)
- A 0x10010e00, B 0x10010de0. Field: `u16 playerId`.
- Behaviour: if id < 1005 and present (A 0x10017530): cleanup (A 0x10015fa0, also marks the AFK flag as 2) and delete the game ped, free its ped slot; remote state := 0; **name-tag visibility flag (remote +4) reset to 1**.

### 11 SetPlayerName (S→C)
- A 0x1001dfd0, B 0x1001df80.

| # | field | type |
|---|---|---|
| 1 | playerId | u16 |
| 2 | nameLen | u8 (must be ≤ 24, else whole RPC ignored) |
| 3 | name | raw[nameLen] |
| 4 | success | u8 |

- Behaviour: if success == 1 and the remote entry exists (id ≤ 1004) its name string is replaced. Independently, if playerId == local player id, the local name is replaced (regardless of success).

### 59 SetPlayerChatBubble (S→C)
- A 0x1000f990, B 0x1000f960.

| # | field | type |
|---|---|---|
| 1 | playerId | u16 |
| 2 | colour | u32 (RGBA) |
| 3 | drawDistance | f32 |
| 4 | expireTime | u32 (ms) |
| 5 | textLen | u8 (must be ≤ 144, else ignored) |
| 6 | text | raw[textLen] |

- Behaviour: id < 1004 and slot in use → chat-bubble manager add (A 0x10066e10(id, text, colour, distance, expire)). Rendering belongs to the UI subsystem.

### 80 ShowPlayerNameTagForPlayer (S→C)
- A 0x1001be20, B 0x1001bdd0. Fields: `u16 playerId`, `u8 show`.
- Behaviour: id < 1004 and slot in use → remote +4 := (show != 0). Name tags drawn by A 0x10075390 ("%s (%d)" name and id; also reads the AFK flag, §2.6).

### 155 SendPlayerScoresAndPings (S→C) / OnPlayerRequestScoresAndPings (C→S)
- S→C A 0x100103e0, B 0x100103c0. Body = repeated records, count = floor(floor(bits/8) / 10):

| field | type |
|---|---|
| playerId | u16 |
| score | i32 |
| ping | u32 |

  For the local id: local score (player pool +0) and local ping (pool +0x22); for remote ids < 1004: entry +4 (score), +0xc (ping). Ids ≥ 1004 skipped. Afterwards, if the scoreboard is visible, it is refreshed (A 0x1006ed30).
- C→S: A 0x10008f10 (B 0x10008f00), called from the scoreboard code (A 0x1006f0b0). Empty payload, RPC send with priority 1, reliability 8, channel 0; throttled to once per > 3000 ms (global A 0x10118a18).

### 124 TogglePlayerSpectating (S→C)
- A 0x1001c350, B 0x1001c300. Field: `u32 enable`.
- Behaviour (A 0x100041c0): if currently spectating and enable == 0: if "return to class selection" is pending (+0x2fa) the client goes back to class selection; otherwise it immediately requests a spawn (A 0x10003c20, which sends RPC 52). Then spectating flag (+0x108) := enable, spectate target cleared (type 0, id 0xffffffff, "applied" 0).
- While spectating, the local tick sends only 212 (+203) — §212.

### 126 PlayerSpectatePlayer (S→C)
- A 0x1001c400, B 0x1001c3b0. Fields: `u16 playerId`, `u8 mode`.
- Mode mapping to GTA camera mode stored at local +0x30e: 2 (FIXED) → 15, 3 (SIDE) → 14, anything else (NORMAL) → 4 (follow ped).
- Target accepted (A 0x10004280) only if id < 1004, slot in use, and the remote state is neither 0 (none) nor 32 (wasted); then target type := 1, id stored, "applied" := 0. The camera is attached during the next spectator tick (§212).

### 127 PlayerSpectateVehicle (S→C)
- A 0x1001c4e0, B 0x1001c490. Fields: `u16 vehicleId`, `u8 mode`. Mapping: 2 → 15, 3 → 14, else → 3 (behind car). Accepted (A 0x10004230) if vehicleId < 2000 and the vehicle exists; target type := 2.

---------------------------------------------------------------------------------------------------

## 2. Sync packets

### 207 PlayerFootSync

**C→S** (send A 0x10004f00, B 0x10004ef0): `u8 207` + raw[68]:

| off | field | type | source |
|---|---|---|---|
| 0 | leftRight | i16 | left stick X |
| 2 | upDown | i16 | left stick Y |
| 4 | keys | u16 | §0.3 |
| 6 | position | 3 × f32 | ped matrix translation |
| 18 | rotation | 4 × f32 (w,x,y,z) | from ped matrix, normalised; if every component differs from the last sent by < 1e-5 the old quaternion is reused (jitter filter) |
| 34 | health | u8 | ped health truncated to int |
| 35 | armour | u8 | ped armour truncated |
| 36 | weapon \| addKey<<6 | u8 | current weapon (6 bits) + §0.3 additional key |
| 37 | specialAction | u8 | A 0x10003410 |
| 38 | velocity | 3 × f32 | ped move speed |
| 50 | surfOffset | 3 × f32 | 0 if not surfing |
| 62 | surfId | u16 | 0 if not surfing; vehicle id 1..1999, or 2000+objectId |
| 64 | animation | u32 | low 16 = anim index, high 16 = anim flags; 0 when none |

Surfing detection (A 0x10003730): the ped's ground-contact entity is a vehicle (occupied by a driver, or train carriage models 569/570) within 30 units, or a *moving* script object; latched for 2000 ms after contact is lost. The offset is the ped position in the surface's local frame (inferred from the receiver, which multiplies it by the surface matrix).

**S→C** (recv A 0x1000a740, B 0x1000a730):

| # | field | encoding |
|---|---|---|
| 0 | [0x28 + u32 time] | optional, see §0.1 |
| 1 | id 207 | u8 |
| 2 | playerId | u16 |
| 3 | hasLR | bit; if 1: leftRight u16 (else 0) |
| 4 | hasUD | bit; if 1: upDown u16 (else 0) |
| 5 | keys | u16 |
| 6 | position | raw[12] (3 × f32) |
| 7 | rotation | normalised quaternion §0.2 |
| 8 | health/armour | packed byte §0.2 |
| 9 | weapon | u8 (low 6 bits kept) |
| 10 | specialAction | u8 |
| 11 | velocity | compressed vector §0.2 |
| 12 | hasSurf | bit; if 1: surfId u16 then surfOffset raw[12]; else surfId := 0xffff |
| 13 | hasAnim | bit; if 1: animation u32 (else 0) |

The decoded struct has the same 68-byte layout as C→S. open.mp's writer (two u16 for anim id + flags) is bit-identical to the client's u32 read. **Agrees with open.mp.**

### 200 PlayerVehicleSync (driver)

**C→S** (send A 0x10007080, B 0x10007070): `u8 200` + raw[63]; sent only if the local vehicle id is valid (< 2000, exists):

| off | field | type | source |
|---|---|---|---|
| 0 | vehicleId | u16 | |
| 2 | leftRight | i16 | |
| 4 | upDown | i16 | |
| 6 | keys | u16 | §0.3 |
| 8 | rotation | 4 × f32 (w,x,y,z) | vehicle matrix, normalised, same 1e-5 jitter filter |
| 24 | position | 3 × f32 | |
| 36 | velocity | 3 × f32 | |
| 48 | vehicleHealth | f32 | |
| 52 | playerHealth | u8 | |
| 53 | playerArmour | u8 | |
| 54 | weapon \| addKey<<6 | u8 | |
| 55 | siren | u8 (0/1) | A 0x100b75a0 |
| 56 | landingGear | u8 (0/1) | A 0x100b7670 |
| 57 | trailerId | u16 | 0 if no attached trailer (the trailer link is also refreshed here) |
| 59 | extra | 32 bits | model 520 (Hydra): raw u32 thrust-nozzle value (CPlane +0x86c); models 538/537/449 (trains/tram): f32 train speed; bike/BMX subtypes: f32 lean/steer value (A 0x100b82f0); otherwise 0.0 |

**S→C** (recv A 0x1000aa10, B 0x1000aa00):

| # | field | encoding |
|---|---|---|
| 0 | [0x28 + u32 time] | optional |
| 1 | id 200, playerId | u8, u16 |
| 2 | vehicleId, leftRight, upDown, keys | 4 × u16 |
| 3 | rotation | normalised quaternion |
| 4 | position | raw[12] |
| 5 | velocity | compressed vector |
| 6 | vehicleHealth | u16 → converted to float |
| 7 | health/armour | packed byte |
| 8 | weapon | u8 (low 6 bits) |
| 9 | siren | bit |
| 10 | landingGear | bit |
| 11 | hasExtra | bit; if 1: extra u32 (else 0) |
| 12 | hasTrailer | bit; if 1: trailerId u16 (else 0) |

**Agrees with open.mp** (order siren, gear, hydra/extra, trailer).

### 211 PlayerPassengerSync

**C→S** (send A 0x10005590, B 0x10005580): `u8 211` + raw[24]:

| off | field | type |
|---|---|---|
| 0 | vehicleId | u16 |
| 2 | seat \| driveBy<<6 \| cuffed<<7 | u8: low 6 bits seat (A 0x100ac240), bit 6 drive-by active (+0x1cb), bit 7 cuffed special action |
| 3 | weapon \| addKey<<6 | u8 |
| 4 | health | u8 |
| 5 | armour | u8 |
| 6 | leftRight | i16 |
| 8 | upDown | i16 |
| 10 | keys | u16 |
| 12 | position | 3 × f32 (ped matrix) |

**S→C** (recv A 0x10009d30): `u8 id, u16 playerId, raw[24]` — same layout. open.mp reads bytes 2–3 as one u16 and 4–5 as its "compressed percent pair"; byte sizes agree (its write emits u8 health, u8 armour).

### 203 PlayerAimSync

**C→S** (send A 0x10005210, B 0x10005200): `u8 203` + raw[31]:

| off | field | type | source |
|---|---|---|---|
| 0 | camMode | u8 | active GTA camera mode |
| 1 | camFront | 3 × f32 | camera front vector (game camera data pointer A 0x101039c8 → 0xb6f32c) |
| 13 | camPos | 3 × f32 | camera source position (same block) |
| 25 | aimZ | f32 | ped aim pitch (ped weapon-aim data +0x54) |
| 29 | zoom \| weaponState<<6 | u8 | zoom = trunc(((FOV − 35) / 35) × 63) (FOV at game 0xb6f250); weaponState = 3 if the current weapon is reloading, else min(ammoInClip, 2) |
| 30 | aspect | u8 | trunc((aspectRatio − 1.0) × 255) (aspect at game 0xc3efa4) |

**S→C** (recv A 0x10009c40): `u8 id, u16 playerId, raw[31]` (no timestamp handling). Store (A 0x10015760): per-ped-slot camera mode; aim vectors handed to the ped's aim state (A 0x100abc70); aimZ applied (A 0x100ad970); per-slot zoom stored as `(b & 0x3f)/63` and aspect as `b/255` — when that ped's shot is processed the game camera FOV is temporarily set to `stored*35 + 35` and aspect to `stored + 1` (A 0x1009c8b0); weaponState: 3 → weapon state RELOADING, 2 → ammo-in-clip at least 2, 0/1 → ammo-in-clip := value. **Agrees with open.mp** (field order camMode, front, pos, aimZ, zoom/state, aspect).

### 206 PlayerBulletSync

**C→S** (send A 0x10006bc0, B 0x10006bb0; triggered per hitscan shot via the weapon-fire hook A 0x100a5450 → A 0x100af2c0 for the local ped): `u8 206` + raw[40]:

| off | field | type |
|---|---|---|
| 0 | hitType | u8: 0 none, 1 player, 2 vehicle, 3 object |
| 1 | hitId | u16 (0xffff when none) |
| 3 | origin | 3 × f32 |
| 15 | hitPos | 3 × f32 |
| 27 | offset | 3 × f32 |
| 39 | weaponId | u8 (current weapon) |

Offset: when an entity was hit, lag-comp selector 0 → hit point transformed into the entity's local frame (inverse matrix); otherwise → hitPos minus entity position. If the hit entity is not a known player/vehicle/object, hitType stays 0 and the offset is converted to world coordinates. Not sent when the weapon is 34 (sniper) and nothing was hit. No throttle.

**S→C** (recv A 0x10009b40): `u8 id, u16 playerId, raw[40]`, playerId < 1004. Apply (A 0x10016370): if hitType != 0 every offset component must be within [−300, 300] or the bullet is dropped; target resolved (1: local ped if hitId == local id; ignored if hitId == shooter; else that remote ped; 2: vehicle). The shooter ped is switched to `weaponId` (given with 9999 ammo if missing) and a synthetic shot is fired from origin toward hitPos (A 0x100af2c0 + A 0x100afab0). **Agrees with open.mp.**

### 204 PlayerWeaponsUpdate (C→S only; not dispatched on receive)
- Send A 0x10006290, B 0x10006280. Bit layout:

| # | field | type |
|---|---|---|
| 1 | id 204 | u8 |
| 2 | targetPlayer | u16 (player whose ped the local ped targets, 0xffff none) |
| 3 | targetActor | u16 (actor id, 0xffff none) |
| 4… | per changed slot: slot u8, weaponId u8, ammo u16 | 0–13 records |

- Rule: called every tick. If targets and armed weapon are unchanged and < 100 ms since the last send, return. Otherwise slots 0..12 are compared with the last-sent copy (ped weapon slot type and total ammo); changed slots are appended and the copy updated. Sent (UNRELIABLE) only if the targets changed or at least one slot record was appended. open.mp reads records until the stream ends — **agrees**.

### 205 PlayerStatsSync (C→S only)
- Send A 0x10005d00, B 0x10005cf0: `u8 205, i32 money (A 0x100a0fd0), i32 drunkLevel (A 0x100adfe0)`; every > 1000 ms. **Agrees.**

### 212 PlayerSpectatorSync (C→S only)
- Send A 0x10006540, B 0x10006530: `u8 212` + raw[18] = `i16 leftRight, i16 upDown, u16 keys, 3 × f32 position` where position = current camera matrix translation. Rate §0.4.
- Side effects every spectator tick: local ped health forced to 100, ped teleported to camera position + 20 on Z (keeps it out of the way). Target handling: if the spectated player vanished / state 0 / wasted, target cleared. When not yet applied: no target → camera placed at (50,50,50) looking at (60,60,50) and ped moved to (0,0,10); player target → camera attached to the remote ped with mode +0x30e (script-command descriptor A 0x100e57e4); vehicle target → attached to the vehicle (descriptor A 0x100e57d0). **Agrees with open.mp** (18-byte read).

### 208 PlayerMarkersSync (S→C only)
- Recv A 0x10009f10, B 0x10009f00 (no playerId after the packet id):

| # | field | type |
|---|---|---|
| 1 | id 208 | u8 |
| 2 | count | u32 |
| 3… | per record: playerId u16, hasPos bit, if 1: x i16, y i16, z i16 | |

- Apply (ids < 1004 and in use): hasPos 0 → remove the marker blip (A 0x10014500). hasPos 1 (A 0x10014540): remove old blip; **if the player is not streamed in**, create a radar blip at (x,y,z) converted to float, keyed by the player id (A 0x100a0dd0). Streamed-in players keep their ped blip. **Agrees with open.mp.**

### 209 PlayerUnoccupiedSync

**C→S** (send A 0x10004d30, B 0x10004d10): `u8 209` + raw[67]:

| off | field | type |
|---|---|---|
| 0 | vehicleId | u16 |
| 2 | seat | u8 (0 = nobody in the car, k+1 = local player in passenger seat k) |
| 3 | roll | 3 × f32 — vehicle matrix **right** vector |
| 15 | direction | 3 × f32 — vehicle matrix **forward** vector (RW "up" row) |
| 27 | position | 3 × f32 |
| 39 | velocity | 3 × f32 |
| 51 | turnVelocity | 3 × f32 |
| 63 | health | f32 |

After sending, the vehicle pool records the local player as last unoccupied-syncer of that vehicle and sets a per-vehicle flag.

When (vehicle-pool tick A 0x1001f3c0 → A 0x10006e00): for each existing, streamed vehicle, at most **3 sends per tick** and **> 100 ms per vehicle**; vehicle not a train/tram (models 537, 538, 569, 570, 449) and not towed by a driven vehicle; additional gate `(boat || distance ≥ 200 || model not in set A 0x100ed574)` (meaning of that set unknown). In A 0x10006e00: no seated driver, distance from local player ≤ 90, vehicle **not stopped** (A 0x1009fe40: stopped = in GTA's moving list (+0xB4) with a zero move speed), not towed. Passenger seats: the first seat holding a player ped (`m_nPedType` +0x598 == 0; AI peds are skipped) decides — send only if it is the local ped (seat = index+1). Otherwise send with seat 0 only if the local player is the closest player: ids 0 ..= max(highest id, local id) in order, each counted only while its ped is in the world (A 0x1009f430: in the moving list); id 0 seeds the minimum, later ids replace it only when strictly closer; the local id must hold it at ≤ 90. The 209s around a passenger entry/exit come from GTA's own physics (the door/seat push wakes the car; on exit the car gets a move speed of exactly (0.001, 0.001, 0.001) for a frame): their number depends on the frame timing (NET-6 notes §4).

**S→C** (recv A 0x10009a40): `u8 id, u16 playerId, raw[67]`. Apply (A 0x100158d0): vehicleId 1..1999, vehicle exists, no driver, not towed; validation: roll and direction components in [−1, 1], position x,y in (−20000, 20000) and z in (−10000, 100000), velocity and turn velocity components in (−100, 100) — otherwise ignored. Build matrix with right = roll, forward = direction (third axis kept), then:
- if |Δx|, |Δy|, |Δz| all ≤ 0.1 → do nothing;
- else if |Δx| ≤ 6 and |Δy| ≤ 6 and |Δz| ≤ 3 → keep current position with the new rotation, set velocity and turn velocity to the synced values, then for each axis with |Δ| > 0.05 add Δ × 0.06 to velocity;
- else teleport: position := synced, velocity and turn velocity := synced.

**Agrees with open.mp** (open.mp calls the two vectors Roll/Rotation).

### 210 PlayerTrailerSync

**C→S** (send A 0x100053d0, B 0x100053c0; called right after a 200 when a trailer is attached): `u8 210` + raw[54]: `u16 trailerId, 3 × f32 position, 4 × f32 quaternion (w,x,y,z, normalised), 3 × f32 velocity, 3 × f32 turnVelocity`. Duplicate suppression as §0.4.

**S→C** (recv A 0x10009e20): `u8 id, u16 playerId, raw[54]`. Apply immediately (A 0x10015c90), only if the remote player has a vehicle: trailerId 1..1999 and exists; trailer must be a towable model (A 0x100b7db0 list: 435, 450, 584, 591, 606, 607, 608, 610, 611, or A 0x100b7e10); attach to the remote's vehicle if not already. Then:
- all |Δ| ≤ 0.5 → nothing;
- |Δx| ≤ 6, |Δy| ≤ 6, |Δz| ≤ 3 → rotation := synced (position kept), turn velocity := synced, velocity := current + Δ × 0.025 per axis with |Δ| > 0.05 (synced velocity not applied);
- else teleport with synced rotation, velocity and turn velocity.

open.mp: VEC4 quaternion component order cannot be checked here (SDK sources absent); **the client expects w first**.

### 201 RCON command (C→S only, note)
- Send A 0x10069030 (handler of the "/rcon" chat command): `u8 201, u32 length, raw[length]` (no NUL), reliability RELIABLE. Not sent if no argument. No dedicated receive path.

---------------------------------------------------------------------------------------------------

## 2.1 How stored sync is applied (remote player object, per frame A 0x100166b0)

Remote state byte (+0x10a): 0 none, 17 on foot, 18 passenger, 19 driver, 32 wasted, 33 spawned. "Update type" byte (+0x1b8) is set by the store function (16 foot, 17 driver, 18 passenger) and cleared after the one-shot apply. Last-update tick (+0x1b9) = GetTickCount at store.

**Store, foot** (A 0x10017260): if the packet had a timestamp and `(int)(ts − lastTs) < 0`, drop it (late); else copy 68 bytes, health/armour to floats, special action, state := 17. If the animation u32 is negative (flags bit 15 set) it is treated as 0. If the ped is still in a vehicle and the special action is not 3/4 (enter/exit vehicle), it is pulled out (A 0x100148f0).

**Store, driver** (A 0x10017340): same timestamp rule; resolves the vehicle (< 2000), seat 0, special action 0; warps the ped into the vehicle if not inside (A 0x10014800: sets weapon, then seat-warp). On transition 17 → 19, if that vehicle is the one the local player is driving, the local player is ejected (placed 1 unit above) and game text "~r~Car Jacked~w~!" (1000 ms, style 5) is shown (A 0x10016270).

**Store, passenger** (A 0x10017440): no timestamp check; seat = byte2 & 0x3f; warps into vehicle if needed; state := 18. Position is stored but not used while seated.

**One-shot apply, on foot** (new data, ped not in a vehicle):
1. Target position/velocity := synced; ped move speed := synced velocity (A 0x100150d0).
2. Unless surfing: position correction (A 0x10014c40). If the ped has no model/RW object yet → teleport. Else Δ = |target − current| per axis; if all ≤ 1e-5 nothing; else if Δx ≤ 2, Δy ≤ 2, Δz ≤ 1 → `speed_i += (target_i − current_i) × 0.1` for each axis with Δ > 1e-5 (constant A 0x100fe4b0); otherwise teleport (set matrix translation).
3. Weapon: if the ped's current weapon differs, switch; if that fails give it with 9999 ammo.
4. If keys has AIM (128) and weapon is 34/35/36 → ped state 12; otherwise ped state 12 is reverted to 1.
5. Animation: if changed, play it (A 0x100ae810); fallback "CARRY / crry_prtial" when carrying with no animation.

**Every frame, on foot:** rotation slerp (A 0x10014ff0): `q := slerp(synced, current, 0.75)` — i.e. the ped turns 25 % of the remaining angle per frame; heading = atan2(−forward.x, forward.y) in degrees, normalised to [0, 360), written to the ped's current/target heading. Head-look toward aim (A 0x100143a0, not for NPCs): every 1000 ms compute a point 20 units along that player's last aim front vector; every 200 ms issue a look-at task to that point. Special actions applied/cleared (A 0x10014950: 1 duck, 2 jetpack, 10 hands up, 11 phone, 20–23 drink/smoke carry objects, 24 cuffed, 25 carry, 68 piss). Keys applied to the ped pad. Collision flags toggled off while entering a vehicle (special action 3) or surfing a vehicle. Surf position applied each frame from a game hook (A 0x100a4980 area → A 0x10014650): if surfId 1..1999 (vehicle) or 2000..2999 (object surfId−2000 ≤ 1000) exists and every offset component is within [−100, 100], ped position := surfaceMatrix × offset.

**One-shot apply, driver:** quaternion normalised → matrix; models 538/537/449: train path (A 0x10015650: teleport if |Δx| or |Δy| > 10, set velocity, set train speed from `extra` if within ±100). Other vehicles (A 0x100155e0 + A 0x10015140): store target rotation (+0x16c), target position and velocity, set vehicle velocity := synced; then if all |Δ| ≤ 0.05 nothing; z-threshold = 2.0 for heli/plane/boat, else 0.5; if Δx ≤ 8, Δy ≤ 8, Δz ≤ zthr: `vel_i += (target_i − current_i) × 0.06` for axes with Δ > 0.05 (constant A 0x100fe4b4), applied only if some component exceeds 0.01 in magnitude; else teleport (matrix translation := target, velocity := synced). Model 520: thrust value written from `extra`; bike/BMX: lean from `extra` (clamped [−1, 1]). Siren, landing gear, vehicle health applied. Trailer id 0 or ≥ 2000 → detach any trailer. Weapon set as on foot.

**Every frame, driver:** turn velocity clamped to ±0.02 per axis (flag A 0x100fe4b8 = 1), then rotation `slerp(target, current, 0.75)`, normalised, written into the vehicle matrix (A 0x10015460) — not for trains. NPC drivers with zero synced velocity get vehicle velocity zeroed. Keys/lr/ud applied to the ped pad.

**One-shot apply, passenger:** weapon; if ped is in vehicle state: driveBy bit → start drive-by task, else stop it; cuffed bit → cuffs on/off. Every frame: keys only (lr/ud = 0).

**Timeouts / AFK:** in every state, flag +0x1c5 becomes 2 when no sync for > 3000 ms while synced velocity is zero, or > 1500 ms while moving (passenger: > 3000 ms); reset to 0 once an update arrives within 1500 ms. The name-tag renderer (A 0x10075390) reads it (AFK/paused indicator, inferred). On foot, after > 3000 ms without sync: lr/ud := 0, velocity := 0, ped snapped back to the last synced position.

**Invulnerability:** every frame, for any state except 32 (wasted), the remote ped's local health is set to 1000.0 (damage is server-authoritative; displayed health comes from sync).

## 3. open.mp vs client disagreements

No wire-layout disagreement was found for any item. Points to watch:
1. C→S sync is raw fixed-size structs (68/63/24/31/40/67/54/18 bytes), unlike open.mp's S→C writers; open.mp's `read()`s match these sizes and orders.
2. The client honours a 0x28 timestamp prefix only on 207 and 200.
3. Bullet and markers validate player ids with `< 1004`; other handlers use `< 1005`.
4. PlayerQuit `reason` is ignored by the client; StreamOut resets name-tag visibility to shown.
5. 204 (weapons) and 205 (stats) are sent UNRELIABLE; 204 is delta-encoded against the last copy, so a lost packet is never retransmitted.

## 4. Unknown / not verified
- Exact numerical encoders for the quaternion, compressed vector and health nibbles (client only decodes them; encoders given are implied).
- Meaning of the model set at A 0x100ed574 used as a gate for unoccupied sync, and of task vtable 0x8705c4 used to mask SECONDARY_ATTACK.
- The ped helper A 0x100abd40 called on stream-in when remote-entry flag +8 is set.
- Where the local surf offset is computed (only the receiver's use was traced).
- Exact rendering of the AFK indicator.

---

## RE-10 additions (static, 2026-11-04)

### Local surfing (resolves §4 "where the local surf offset is computed/applied") — Observed
`CWorld::ProcessAttachedEntities` is called through samp's hook A 0x100A4990 (CSV row 24). After the original:
1. Local surf glue A 0x10003570 on the local player object: when it has a surf entity (+0x2DA) it reads the entity's
   matrix, move speed and turn speed. If the "stick" flag (+0x2DE) is set, the local keys give lr = ud = 0 and the ped
   is not in the state tested by A 0x100ACC50, the ped matrix becomes surface matrix × stored local offset (+0x2E6),
   and the ped takes the surface's move speed and turn speed. Otherwise the offset is recomputed from the current ped
   position in the surface frame (A 0x100B5780). This is the offset later sent in 207 (surfOffset).
2. For every remote id < 1004 with a ped and state ≠ 0: the remote surf apply A 0x10014650 (§2.1).

### 204 target fields (refines §204) — Observed
* targetPlayer: the remote player whose ped equals the local `CPlayerPed` field +0x79C (the first `CPlayerPed` member
  after the 0x79C-byte `CPed`, the ped the player is targeting; plugin-sdk size check), looked up with A 0x100138C0. It is
  0xFFFF if the local ped has no RW object (+0x18) or no target.
* targetActor: the actor id whose ped equals that same pointer (A 0x100018B0), else 0xFFFF.
* Slot compare: 13 `CWeapon`s at `CPed` +0x5A0 (stride 0x1C); a slot is "changed" when its weapon type (low byte) or its
  total ammo (+0xC) differs from the cached copy (local player +0x18A bytes / +0x197 dwords). The armed-weapon byte is
  cached at +0x189: a change of the armed weapon alone bypasses the 100 ms throttle but sends nothing unless a slot or
  a target changed.

### Remote passenger drive-by and cuffs (refines §2.1 "one-shot apply, passenger") — Observed
* Applied only while the remote ped's state is 50 (in vehicle). Drive-by active = the ped's task manager (CPed +0x47C →
  +0x10, the first secondary-task slot) holds a task of type 0x3FE (`TASK_SIMPLE_GANG_DRIVEBY`).
* bit 6 set and not active → if the ped is in a vehicle and its weapon is not 46: re-arm the current weapon, then SCM
  0x0713 `TASK_DRIVE_BY` (ped, −1, −1, 0.0, 0.0, 0.0, 300.0, 8, 1, 20); remote +0x110 := 1. Weapon 46 → armed weapon 0,
  no task.
* bit 6 clear and active → the task is destroyed directly through its deleting destructor (GTA 0x6275E0, flag 1) and
  the secondary slot is nulled; remote +0x110 := 0.
* bit 7 (cuffed) → ped-object cuff flag (+0x2DE) set/cleared on change (A 0x100ACF40, which also sets the local
  sprint mask +0x2F8 when that ped is the local one; see `remote-control-combat.md` RE-10 additions).
