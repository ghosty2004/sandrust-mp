# Session-level and miscellaneous RPCs — SA:MP 0.3.7-R4 client (clean-room notes)

Targets: A = samp.dll fv0.3.7.4 (2021), B = samp.dll fv0.3.7.3 (2019) — see `rpc-ids.md`.
Bitstream conventions are those of `handshake.md` (§3/§6): fields are written/read
MSB-first, not byte-aligned, multi-byte values little-endian; "u16/u32" below means a
raw 16/32-bit read (A bit reader 0x1001f9b0, B 0x1001f960); "bytes(n)" means a raw
n-byte read (A 0x1001fea0, B 0x1001fe50). Client writes use A 0x1001ffe0 (bits) /
0x10020090 (bytes); B 0x1001ff90 / 0x10020040. All client→server RPCs below use
priority 1, ordering channel 0, no timestamp; reliability uses the shifted enum
(8 RELIABLE, 9 RELIABLE_ORDERED, 10 RELIABLE_SEQUENCED).

## Overview

This file covers the session layer and everything that no other subsystem owns: the
join/init RPCs (cross-check only), the "server restarting" RPC 40, the anti-cheat
ClientCheck RPC 103 (layout only), the in-game RCON packet 201, the scoreboard
round-trip (155 both ways, 23), map click (119), camera-target report (168), and every
client-registered RPC that has **no** open.mp NetCode struct (64, 92, 98, 102, 125, 130,
150, 151, 161, 169) plus the struct-less client→server RPC 97. It also resolves the
"variable-ID" send site A 0x1000d800: it sends RPC **181** or **182** (custom-model
DFF/TXD requests). The biggest finding is that the 0.3.7-R4 DLL contains a complete
custom-model download client (RPCs 179/181/182/183/184, open.mp `custommodels.hpp`)
whose server→client handlers are compiled in but **never registered**, so in R4 the
download manager is never created and 181/182/184 are never actually sent. Likewise the
"dropped weapon pickup" path (RPC 97 send, RPC 151 receive) is reachable only through a
pickup-creation routine that nothing calls, so in practice 97 is never sent and 151 is a
no-op. Two registered RPCs (64, 125) are pure no-ops in the client. Read sequences were
compared for every handler/sender below against B: all identical.

| ID | open.mp name (or mine) | dir | A handler / send site | B | confidence |
|---|---|---|---|---|---|
| 25 | PlayerConnect | C→S | 0x1000ad80 | 0x1000ad70 | high (cross-check of handshake §5) |
| 54 | NPCConnect | C→S | not present | not present | high (absent from client) |
| 139 | PlayerInit | S→C | 0x10010660 | 0x10010640 | high (cross-check) |
| 130 | (none) "ConnectionRejected" | S→C | 0x10010540 | 0x10010520 | high |
| 40 | PlayerClose | S→C | 0x1000e650 → 0x1000a540 | 0x1000e620 → 0x1000a530 | high (layout), medium (reset details) |
| 103 | ClientCheck | S→C + C→S | 0x10011a60 / send 0x10011990 | 0x10011a40 / 0x10011970 | high (layout only, by rule) |
| 201 | PlayerRconCommand (packet) | C→S | 0x10069030 (`/rcon` command) | 0x10068ff0 (send at 0x1006909b) | high |
| 155 | OnPlayerRequestScoresAndPings / SendPlayerScoresAndPings | C→S + S→C | send 0x10008f10, handler 0x100103e0 | 0x10008f00, 0x100103c0 | high |
| 23 | OnPlayerClickPlayer | C→S | 0x1006ec10 | 0x1006ebd0 | high |
| 119 | OnPlayerClickMap | C→S | 0x10075790 (send at 0x10075cae) | 0x10075760 | high (layout), medium (trigger) |
| 168 | OnPlayerCameraTarget | C→S | 0x10004700 | 0x100046e0 | high |
| 64 | (none) | S→C | 0x1001a700 | 0x1001a6b0 | high: no-op |
| 92 | (none) "drunk visuals" (public name, unverified) | S→C | 0x10018c50 | 0x10018c00 | high (layout), medium (meaning) |
| 98 | (none) "vehicle tyre status" (mine) | S→C | 0x10018b70 | 0x10018b20 | high |
| 102 | (none) "server net stats" (mine) | S→C | 0x1000e5c0 | 0x1000e590 | high (layout), low (use) |
| 125 | (none) | S→C | 0x100180c0 | 0x10018070 | high: no-op |
| 150 | (none) "drunk handling" (public name, unverified) | S→C | 0x10018d00 | 0x10018cb0 | high (layout), medium (meaning) |
| 151 | (none) "destroy dropped-weapon pickups" (mine) | S→C | 0x1000f1e0 | 0x1000f1b0 | high (layout), effectively no-op |
| 161 | (none) "vehicle params for player, old form" (mine) | S→C | 0x10019e60 | 0x10019e10 | high |
| 169 | (none) "object: no camera collision" (mine) | S→C | 0x1001c170 | 0x1001c120 | high (layout), medium (meaning) |
| 97 | (none) "picked up dropped weapon" (mine) | C→S | 0x10013520 | 0x100134f0 | high (layout), never sent in practice |
| 179 | ModelRequest | S→C | 0x10011eb0 — **unregistered** | 0x10011e90 — unregistered | high (layout), dormant |
| 183 | ModelUrl (+ other download sub-messages) | S→C | 0x100120b0 — **unregistered** | (same, not located exactly) | high (layout), dormant |
| 185 | DownloadCompleted | S→C | candidate 0x1000eba0 — unregistered | — | low |
| 181 | RequestDFF | C→S | 0x1000d800 (variable-ID site), kind 1 | 0x1000d7f0 | high (layout), dormant |
| 182 | RequestTXD | C→S | 0x1000d800, kind 2 | 0x1000d7f0 | high (layout), dormant |
| 184 | FinishDownload | C→S | 0x1000d920 | 0x1000d910 | high (layout), dormant |

Global objects referenced below (A): `0x1026ea0c` netgame (RakClient interface at +0,
state at +0x3cd, settings struct pointer at +0x3d5, pools struct pointer at +0x3de);
`0x1026ea24` game wrapper; `0x1026e9f8` chat window; `0x1026e9c4` scoreboard;
`0x1026ea10` custom-model download manager. Pools struct: +0 vehicle pool, +4 player
pool, +8 pickup pool, +0xc object pool, +0x1c textdraw pool.

---------------------------------------------------------------------------------------

## Join / init (cross-check of handshake.md §5 only)

### 25 PlayerConnect (C→S)
- A 0x1000ad80, B 0x1000ad70. Reliability 8.
- Fields: as in handshake.md §5 (u32 version 4057, u8 mod=1, u8-len nick, u32 challenge
  response, u8-len serial, u8-len version string "0.3.7-R4", u32 challenge response again).
- open.mp comparison: open.mp reads the same first six fields; its **write** stops after
  the version string, and its **read** treats a trailing u32 as optional: if present it sets
  `IsUsingOfficialClient = true` and stores the value as an "omp version" (used only when
  the peer was already flagged as an open.mp-launcher client). So the R4 trailing duplicate
  challenge response is exactly what makes open.mp classify the peer as an official
  client. Not a disagreement in layout, but naming differs.
- Server-side acceptance rules in open.mp that the Rust client must satisfy (from
  `legacy_network_impl.cpp`, behaviour only): the serial string, parsed as base-16, must be
  divisible by 1001, and the quotient printed in base 16 must be non-empty and < 50 chars;
  the version string must be ≤ 24 chars. Failing either → kick and a 15 s IP ban. Hence the
  ×1001 step in the client serial is mandatory (the ×1000 fallback in handshake.md would be
  rejected).

### 54 NPCConnect (C→S)
- Not registered and never sent by A or B (no .rdata cell with value 54 is passed to the
  send call). It is the NPC (bot) join message; a player client must use 25.

### 139 PlayerInit (S→C)
- A 0x10010660, B 0x10010640. The field order/widths in handshake.md §5 match open.mp's
  struct exactly: 4 bits, f32, bit, f32, 3 bits, u32, u16, bit, u32, u8, u8, f32, bit, u32,
  bit, 5×u32, u8-len string, 212 bytes, u32. open.mp names: #9 is "DisableNameTagLOS",
  #11 is the spawn-info (class) count, #25 is lag compensation, #27 is the per-model
  preload table (`MAX_VEHICLE_MODELS` = 212), #28 vehicle friendly fire. No disagreement.
- Also note: 139 is re-sent by open.mp after a gamemode reload (see RPC 40), so the
  handler must be re-entrant (it is in the client: it just overwrites settings and sets
  state 5).

---------------------------------------------------------------------------------------

## Connection / session control

### 130 (no open.mp struct) "ConnectionRejected" (S→C)
- A 0x10010540, B 0x10010520.

| field | type | notes |
|---|---|---|
| reason | u8 | 1 incorrect version, 2 unacceptable nickname, 3 bad mod version, 4 no free player slot |

- Behaviour (observed): prints a "CONNECTION REJECTED: …" line to chat (reason 2 adds
  three hint lines: 3–20 chars, only a-z A-Z 0-9, use /quit), other values print nothing;
  then in all cases calls RakClient Disconnect (vtable +8) with a 500 ms wait.
- open.mp has no struct; the RPC ID is only listed in its raw-RPC hook table. Servers based
  on open.mp reject by kicking instead (not verified further).

### 40 PlayerClose (S→C) — "server restarting"
- A handler 0x1000e650 (a 2-instruction thunk: ECX = netgame, jump to 0x1000a540);
  B 0x1000e620 → 0x1000a530. **No payload is read** (open.mp writes none). Match.
- Client behaviour (observed, A 0x1000a540):
  1. For each remote player slot 0…1003 that has a ped object, marks it (byte +0x109 = 0xFF
     in the remote-player object) and calls 0x10015fa0 (remote player teardown).
  2. Calls a series of pool/reset helpers (0x10002e90, 0x100041c0(0), 0x1009ccf0,
     0x10013b10, 0x100137c0) and **sets netgame state = 11**.
  3. Deletes and recreates the entity pools (helpers 0x10009080, 0x10009490, 0x10009110,
     0x10009250, 0x100093f0, 0x1000a2c0, 0x100092f0, 0x100091b0, 0x10009350 — each frees
     one pool and allocates a new empty one; which pool is which was not mapped).
  4. Restores server-settings defaults in the settings struct (+0x3d5): enter/exits
     enabled (+0x1a = 0), name-tag distance 60.0 (+0x27), hour 12 / minute 0 (+0x2c/+0x2d),
     weather 1 (+0x2e), +0x1f = 1, name-tag LOS = 1 (+0x2f), CJ-walk off (+0x00 = 0),
     gravity 0.008 (+0x16), death-drop money 0 (+0x01), zone names off (+0x21 = 0); clears
     two game-wrapper dwords (+0x4d, +0x29).
  5. Game-side resets via the game wrapper: 0x100ad380, 0x100a2100, 0x100b0010,
     0x100abdf0(0), 0x100a1560(0), 0x100a72c0, gravity applied through 0x100a1440 (writes
     GTA's gravity global 0x00863984), 0x100a1460(0) (touches GTA 0x0058DB60),
     0x100a14a0(0).
  6. Prints "The server is restarting.." in the chat window.
- State 11 semantics (inferred from the main loop A 0x1000b5b0): not 5, so in-game
  processing stops and the "not connected" camera scene (0x100089b0) runs; not 1, so no
  reconnect timer. The RakNet connection stays up and the client simply waits: open.mp's
  reload path re-sends **139 PlayerInit** to every connected player, whose handler sets
  state 5 again. The auth packet 0x0c is not processed in state 11 (handshake.md §5: only
  states 1–6).
- Rust client: on 40, drop all world entities and remote players, reset settings to the
  defaults above, go to a "waiting for InitGame" state without disconnecting.

### 103 ClientCheck (S→C request, C→S reply)
- Reply computation (types 5 / 0x45 / 0x46 / 0x47, our own words) and the decision to answer it honestly
  over the client's own memory: `docs/phase3/m10-notes.md` "ClientCheck (item 5)".
- Request handler A 0x10011a60 / B 0x10011a40; reply sender A 0x10011990 / B 0x10011970
  (ID cell A 0x100e5e90 = 103), reliability 9.
- Request (S→C):

| field | type | notes |
|---|---|---|
| type | u8 | check kind |
| arg | u32 | address / model id / parameter, meaning depends on type |
| offset | u16 | |
| count | u16 | |

- Reply (C→S):

| field | type | notes |
|---|---|---|
| type | u8 | echoes request type |
| arg | u32 | usually echoes request arg; one type returns a client-computed value instead |
| result | u8 | check result |

- Observed facts (no answer derivation recorded, per project rule): the request is dropped
  silently (no reply) when offset > 256, count > 256 or count < 2; only type codes 0x02,
  0x05, 0x45, 0x46, 0x47, 0x48 produce a reply; other codes are ignored. Read sequence
  identical in B. open.mp layout: request write u8/u32/u16/u16, reply read u8/u32/u8 —
  **matches**.

### 201 PlayerRconCommand (packet, C→S)
- A: the chat command `/rcon` is registered by 0x10069110 (string at A 0x100ea2c8) with
  handler 0x10069030; B equivalent writes 0xC9 at B 0x10069049 and sends at 0x1006909b.
- This is a **raw packet**, not an RPC: sent via RakClient vtable +0x18 (Send bitstream),
  priority 1, reliability 8 (RELIABLE), channel 0.

| field | type | notes |
|---|---|---|
| packet id | u8 | 0xC9 = 201 |
| length | u32 | strlen of the argument text |
| text | bytes(length) | the text typed after `/rcon `, no terminator |

- Behaviour: nothing is sent if the argument pointer is null or the netgame does not exist.
  The client does not handle any RCON reply packet; open.mp answers with ordinary client
  messages (RPC 93), e.g. "SERVER: You are logged in as admin." for `login <password>`.
- open.mp reads `readDynStr32` = u32 length + bytes → **matches**.
- Unknown: whether the command dispatcher passes "" or NULL when `/rcon` has no argument
  (open.mp replies "You forgot the RCON command!" to an empty command from a logged-in
  admin, so an empty packet is legal).

---------------------------------------------------------------------------------------

## Scoreboard, map click, camera target (client→server reports)

### 155 OnPlayerRequestScoresAndPings (C→S) / SendPlayerScoresAndPings (S→C)
- C→S: A 0x10008f10 / B 0x10008f00 (ID cell A 0x100e5b28 = 155), reliability 8, **empty
  payload**. Called from the scoreboard update (A 0x1006f0b0) while the scoreboard is
  open; throttled to at most once per 3000 ms (global tick stamp A 0x10118a18).
- S→C handler A 0x100103e0 / B 0x100103c0. Record count = floor(payload_bytes / 10)
  where payload_bytes = floor(bits / 8). Each record:

| field | type | notes |
|---|---|---|
| player id | u16 | ignored if ≥ 1004 |
| score | i32 | |
| ping | u32 | |

- Behaviour: if id equals the local player id (player pool +4), stores score in player pool
  +0 and ping in +0x22; otherwise stores into the remote-player entry (pool +0x1f8a table):
  score at entry +4, ping at entry +0xc. After all records, refreshes the scoreboard if it
  exists (0x1006ed30). open.mp writes exactly u16/i32/u32 per player → **match**.

### 23 OnPlayerClickPlayer (C→S)
- A 0x1006ec10 / B 0x1006ebd0 (ID cell A 0x100ea6a8 = 23), reliability 9.

| field | type | notes |
|---|---|---|
| player id | u16 | parsed (decimal) from the first column of the selected scoreboard row |
| source | u8 | always 0 (scoreboard) |

- Behaviour: invoked from the scoreboard when a row is activated; after sending, the
  scoreboard is closed (cursor/visibility off, 0x100a0750(0,0)). Matches open.mp.

### 119 OnPlayerClickMap (C→S)
- A 0x10075790 (send at 0x10075cae) / B 0x10075760 (ID cell A 0x100e9b9c = 119),
  reliability 8.

| field | type | notes |
|---|---|---|
| x | f32 | |
| y | f32 | |
| z | f32 | GTA ground height at (x,y) + 2.0 |

- Behaviour (observed): checked every frame by the main per-frame function while GTA's
  frontend map is up (game global 0x00BA6774 non-zero, 0x00BA8298 zero) and on the
  right-mouse-button press edge: scans GTA's radar-trace table (175 entries, stride 0x28,
  base 0x00BA86F0) for the entry whose 16-bit field at +0x25 is 0x1203 (inferred: the
  player-placed waypoint) and takes its X/Y; Z = `CWorld::FindGroundZForCoord` (GTA
  0x00569660, through the pointer at A 0x10102bf4) + 2.0 (constant A 0x100e6118).
- open.mp reads `readPosVEC3` = three f32 → **match**.

### 168 OnPlayerCameraTarget (C→S)
- A 0x10004700 / B 0x100046e0 (ID cell A 0x100e5840 = 168), reliability 9.

| field | type | notes |
|---|---|---|
| object id | u16 | 0xFFFF = none |
| vehicle id | u16 | 0xFFFF = none |
| player id | u16 | 0xFFFF = none |
| actor id | u16 | 0xFFFF = none |

- Written as one 8-byte raw block. Only active when the netgame flag +0x233 is set
  (enabled by server RPC 170); otherwise cached values reset to 0xFFFF. At most every
  100 ms it casts from the camera 70 units forward (computer 0x10004440: object within
  40.0, vehicle within 40.0, player/actor within 20.0) and sends only if any of the four
  ids changed. Matches open.mp (four u16 in this order). (May overlap with player-state.)

---------------------------------------------------------------------------------------

## Registered RPCs without an open.mp struct

All are reachable only via raw RPCs from server plugins (open.mp lists them only in its
raw-RPC hook table), but the client must still parse them.

### 64 (S→C) — no-op
- A 0x1001a700 / B 0x1001a6b0. Reads one u16 and discards it. Nothing else.
- Rust client: accept and ignore.

### 125 (S→C) — no-op
- A 0x100180c0 / B 0x10018070: the handler is a single `ret`; no payload is read.

### 92 (S→C) — script opcode 0x052C on the local player
- A 0x10018c50 / B 0x10018c00.

| field | type | notes |
|---|---|---|
| value | u32 | passed as the second integer parameter |

- Behaviour: if the local player object (0x10001010 on the game wrapper) exists, runs GTA
  script opcode **0x052C** with parameters (player 0, value) through the script-command
  helper 0x100b2350 (descriptor at A 0x100e6220: opcode 0x052C, format "ii").
- Meaning: public RPC lists call 92 "set player drunk visuals" and 0x052C is publicly
  documented as the player drunkenness opcode — **unverified** here.

### 150 (S→C) — script opcode 0x03FD on the local player
- A 0x10018d00 / B 0x10018cb0. Same shape as 92: u32 value → opcode **0x03FD**
  (descriptor A 0x100e6234, "ii") with (player 0, value) if the local player exists.
- Meaning: public RPC lists call 150 "set player drunk handling" — **unverified**.

### 98 (S→C) — vehicle tyre status
- A 0x10018b70 / B 0x10018b20.

| field | type | notes |
|---|---|---|
| vehicle id | u16 | must be < 2000 and exist in the vehicle pool |
| tyre bits | u8 | 1 bit per tyre, 1 = burst |

- Behaviour (observed, 0x100b7980 on the vehicle wrapper; GTA vehicle at wrapper +0x4c):
  vehicle class is identified by GTA vtable (0x100b6220: 0x00871120 → automobile,
  0x00871360 → bike, …).
  - Automobile: bit0 → byte +0x5A8, bit1 → +0x5A7, bit2 → +0x5A6, bit3 → +0x5A5 of the
    GTA vehicle (the four wheel-status bytes of the damage manager; inferred order
    rear-right, front-right, rear-left, front-left — same bit order as open.mp's tyre
    field in SetVehicleDamageStatus).
  - Bike: bit0 → +0x65D, bit1 → +0x65C (rear, front — inferred).
  - Other classes: ignored. Bits 4–7 unused.

### 102 (S→C) — server network statistics blob
- A 0x1000e5c0 / B 0x1000e590. Reads **296 raw bytes** (`bytes(0x128)`) into the global
  A 0x1012e7e0.
- 296 bytes is the size of a 32-bit RakNet statistics structure (inference). The buffer is
  zeroed by the constructor 0x10071280; no other direct reader was found, so the client
  appears to just store it. Rust client: read and keep (or ignore).

### 151 (S→C) — destroy dropped-weapon pickups of a player
- A 0x1000f1e0 / B 0x1000f1b0.

| field | type | notes |
|---|---|---|
| owner | u8 | zero-extended to 16 bits and compared with the pickup's owner field |

- Behaviour: on the pickup pool (pools +8), for each of the 4096 slots that holds a GTA
  pickup **and** has the "dropped weapon" flag (byte at +0xC004 + 3·i) set **and** whose u16
  owner (at +0xC005 + 3·i) equals the value, destroys the GTA pickup (opcode 0x0215 via
  descriptor A 0x100e60cc) and clears the slot.
- In R4 the only code that sets the "dropped weapon" flag is A 0x10013180 (B 0x10013150),
  which has **no callers or pointer references** in either build, so no slot ever matches:
  151 is effectively a no-op. Note the owner is only 8 bits on the wire.

### 161 (S→C) — vehicle params for player (old 3-field form)
- A 0x10019e60 / B 0x10019e10.

| field | type | notes |
|---|---|---|
| vehicle id | u16 | |
| objective | u8 | non-zero → show objective marker |
| doors locked | u8 | non-zero → locked |

- Behaviour (0x1001eb30 on the vehicle pool): only if id < 2000, the slot exists, a vehicle
  wrapper exists and the per-slot value at pool +0xBD14 is non-zero (meaning unverified;
  likely "created in game"). If objective ≠ 0: wrapper byte +0x64 = 1 (same field RPC 24
  uses for its objective byte) and wrapper dword +0x65 = 0. **Objective 0 does not clear an
  existing objective.** Then door lock (0x100b7880): GTA vehicle +0x4F8 = 2 (locked) or 0,
  wrapper +0x60 = 1/0.
- open.mp never sends 161; it uses RPC 24 (16 bytes of params) instead.

### 169 (S→C) — object excluded from camera collision
- A 0x1001c170 / B 0x1001c120.

| field | type | notes |
|---|---|---|
| object id | u16 | accepted if ≤ 1000 (check is `< 1001`) and the slot exists |

- Behaviour: sets byte +0x53 = 1 on the object wrapper. The only reader found is
  0x10012de0, a segment-vs-object test used by camera code (called at 0x100a5a74/0x100a5aa3),
  which skips flagged objects. Inferred purpose: SetObjectNoCameraCol (public name).
- No way to clear the flag via this RPC.

---------------------------------------------------------------------------------------

## Client→server RPC without an open.mp struct

### 97 (C→S) — picked up a dropped weapon
- A 0x10013520 / B 0x100134f0 (ID cell A 0x100e60e4 = 97), reliability 10.

| field | type | notes |
|---|---|---|
| owner | u16 | the owner field of the dropped-weapon pickup slot |

- Behaviour: the pickup pool's per-frame process (0x10013520) polls GTA's "pickup
  collected" opcode 0x0214 (descriptor A 0x100e60b8) for every live pickup that is either a
  dropped-weapon slot or a normal pickup of type 14; on collection it sends 97 (dropped
  weapon) or **131** with a u32 pickup index (normal pickup, owned by world-entities).
- Because dropped-weapon slots are never created in R4 (see 151), 97 is never sent in
  practice. A Rust client can omit it.

---------------------------------------------------------------------------------------

## Custom-model download machinery (dormant in 0.3.7-R4)

The DLL contains a full custom-model download client (strings "DownloadRedirect…",
"%s/0x%X.dff", "%s/0x%X.txd", "[url 0x%X]: %s", "AddURLRequest"). Its manager object
(0x206D bytes, global A 0x1026ea10) is created **only** inside the 179 handler below, and
neither that handler nor the 183 handler is referenced anywhere (no call, no pointer,
no .rdata ID cell with value 179/183/185; same in B). Consequently A 0x1026ea10 stays
NULL, the main loop takes the normal branch, and 181/182/184 are never sent. Layouts are
recorded for completeness (they all match open.mp); a Rust R4 profile should **not**
register 179/183/185.

### 179 ModelRequest (S→C) — dormant handler A 0x10011eb0 (B 0x10011e90)
Reads, in order: u32 pool id, u32 total count, u8 type, i32 virtual world, i32 base id,
i32 new id, u32 DFF checksum, u32 TXD checksum, u32 DFF size, u32 TXD size, u8 time on,
u8 time off. Stores the entry (0x1000dae0) and records the expected total (0x1000d4c0);
when the stored count reaches the total it marks the list ready, scans the local cache
directory (0x1000d080) and enters "ready" state 2. Matches open.mp's write order.

### 183 ModelUrl and related sub-messages (S→C) — dormant handler A 0x100120b0
First u8 is a sub-type:
- 6: u8 file type, u32 checksum, u8 length + bytes URL → "DownloadRedirect" (0x1000d650).
  Matches open.mp ModelUrl (which writes sub-type 6 first).
- 5: u8, u32 checksum → 0x1000d750 ("Can't fill download info for CRC").
- 1: u8, u32, i32 length (0…4096), bytes(length) → 0x1000d500 (in-band file chunk; inferred).
- 0: reset + 0x1000cbf0(0, 6001, 0); 2: 0x1000e160(0). Meanings not analysed.

### 185 DownloadCompleted (S→C) — candidate dormant handler A 0x1000eba0 (low confidence)
Reads nothing; if the manager exists, re-initialises the cache dirs (0x1000bd60) and
calls 0x1000db30(0). Also unreferenced. Mapping to 185 is a guess from shape only.

### 181 RequestDFF / 182 RequestTXD (C→S) — the variable-ID site A 0x1000d800 (B 0x1000d7f0)
- Function (netgame-download manager method) takes (entry index, kind, checksum, size);
  kind 1 → ID cell A 0x100e5d90 = **181**, kind 2 → cell A 0x100e5d8c = **182**; any other
  kind sends nothing. Reliability 9.

| field | type | notes |
|---|---|---|
| checksum | u32 | CRC of the DFF (181) or TXD (182) file wanted |

- Before sending, opens the download-progress UI if not yet shown; afterwards sets the
  manager's state to 1 (transfer in progress) and calls 0x1000c0f0.
- Caller 0x1000d920 (per tick, only in state 2 = ready): for each entry not disabled and
  matching the current virtual world (or −1), requests the first missing DFF, else the first
  missing TXD, one request per tick, skipping files already cached or already requested.
- open.mp reads one u32 for each → **match**.

### 184 FinishDownload (C→S) — A 0x1000d920 (B 0x1000d910), cell A 0x100e5d94
- Payload: a single 1 bit (corrected: the send site writes one 1 bit before sending; see
  world-entities.md and crates/samp-proto rpcs::world), reliability 9. Sent once when the list is ready and nothing is pending
  (0x1000ce40 < 1); sets a "finished" flag so it is not repeated. open.mp reads nothing, so the extra bit is
  ignored → compatible.

---------------------------------------------------------------------------------------

## open.mp vs client disagreements (summary)

1. **RPC 25 trailing u32**: open.mp's writer omits it; its reader treats it as an optional
   "official client / omp version" marker. R4 always sends it (= challenge response). Not a
   wire conflict, but the meaning differs.
2. **Custom models (179/181/182/183/184/185)**: open.mp defines them; the R4 client has the
   code but never registers the S→C handlers, so the feature is inert with R4.
3. **RPC 151 owner width**: the client reads only 8 bits for the owner while its pickup
   table and the matching C→S RPC 97 use 16 bits (moot since the path is dead).
4. No layout disagreements for 40, 103, 139, 155, 23, 119, 168, 201.

## Unknown / not chased
- Exact purpose of opcodes 0x052C/0x03FD in RPC 92/150 (no opcode database available here).
- Meaning of vehicle-pool table +0xBD14 (gate in RPC 161) and of wrapper +0x65.
- Which pool each delete/recreate helper in the RPC 40 reset handles.
- Whether `/rcon` with no argument sends an empty packet or nothing.
- Any consumer of the RPC 102 stats buffer (only a zeroing constructor references it).
