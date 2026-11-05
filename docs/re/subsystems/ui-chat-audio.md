# UI, chat and audio subsystem (SA:MP 0.3.7-R4 client)

Clean-room spec notes. Targets and conventions as in `../rpc-ids.md` and `../handshake.md`
(A = 2021 build, primary; B = 2019 build). Bit order MSB-first, multi-byte values little-endian,
nothing byte-aligned unless stated. "uN" = N-bit unsigned read/written with the plain bit
reader/writer (A 0x1001f9b0 / 0x1001ffe0); "bytes[n]" = n raw bytes (A 0x1001fea0 read,
0x10020090 write); "bit" = 1-bit bool; "f32" = IEEE float sent as raw 32 bits.
"(c >> 8) | 0xFF000000" means: the wire colour is RGBA (0xRRGGBBAA as a u32 value); the client
turns it into ARGB with alpha forced to 0xFF.

## Overview

This covers everything the client shows as UI text or plays as streamed audio: the chat window
(server messages, player chat, chat bubbles, the local chat/command input line and its built-in
`/commands`), centred "game text", textdraws (including the click-to-select mode), dialogs,
GTA-style menus, the HUD clock and widescreen toggles, and the BASS-based internet audio stream.
All server→client layouts in open.mp's NetCode were confirmed against the A handlers bit by bit
and the read sequences are identical in B (checked mechanically for every handler listed below;
command table and client→server send sites also identical). No wire-layout disagreement with
open.mp was found; differences are limited to client-side limits/quirks (length caps, missing
bounds checks) that the Rust client should reproduce or harden, listed per item.
Strings are not Huffman-compressed except the dialog body (§61); its codec is described in
"Shared: compressed string".

| ID | open.mp name | Dir | A handler / send site | B | Confidence |
|---|---|---|---|---|---|
| 93 | SendClientMessage | S→C | 0x1000ef90 | 0x1000ef60 | high |
| 101 | PlayerChatMessage | S→C | 0x100122f0 | 0x100122d0 | high |
| 101 | PlayerRequestChatMessage | C→S | 0x10005a10 (via default input handler 0x100684e0) | 0x10005a00 | high |
| 50 | PlayerRequestCommandMessage | C→S | 0x10069900 (from input processor 0x100699d0) | 0x100698c0 | high |
| 59 | SetPlayerChatBubble | S→C | 0x1000f990 | 0x1000f960 | high |
| 73 | SendGameText | S→C | 0x100198f0 | 0x100198a0 | high |
| 134 | PlayerShowTextDraw | S→C | 0x1001cd90 | 0x1001cd40 | high |
| 135 | PlayerHideTextDraw | S→C | 0x1001cec0 | 0x1001ce70 | high |
| 105 | PlayerTextDrawSetString | S→C | 0x1001cf70 | 0x1001cf20 | high |
| 83 | PlayerBeginTextDrawSelect | S→C | 0x1001d650 | 0x1001d600 | high |
| 83 | OnPlayerSelectTextDraw | C→S | 0x100714e0 (callers 0x100715d0 click, 0x10071580 cancel) | 0x100714b0 | high |
| 61 | ShowDialog | S→C | 0x1000faf0 → 0x10070010 | 0x1000fad0 | high |
| 62 | OnPlayerDialogResponse | C→S | 0x10070690 | 0x10070660 | high |
| 76 | PlayerInitMenu | S→C | 0x1001c870 | 0x1001c820 | high |
| 77 | PlayerShowMenu | S→C | 0x1001cb90 | 0x1001cb40 | high |
| 78 | PlayerHideMenu | S→C | 0x1001cc40 | 0x1001cbf0 | high |
| 132 | OnPlayerSelectedMenuRow | C→S | 0x10008200 | 0x100081f0 | high |
| 140 | OnPlayerExitedMenu | C→S | 0x10008200 | 0x100081f0 | high |
| 41 | PlayAudioStreamForPlayer | S→C | 0x1001d3c0 → 0x10066960 | 0x1001d370 | high (layout), medium (volume formula) |
| 42 | StopAudioStreamForPlayer | S→C | 0x100180f0 (not a Ghidra function; read from disassembly) | 0x100180a0 | high |
| 30 | TogglePlayerClock | S→C | 0x1000f500 | 0x1000f4d0 | high |
| 111 | ToggleWidescreen | S→C | 0x10018ac0 | 0x10018a70 | high |
| packet 201 (0xC9) | PlayerRconCommand | C→S | `/rcon` handler at 0x10069030 (label) | label 0x10068ff0 | high |

All client→server RPCs here are sent with priority 1, ordering channel 0, no timestamp; the
reliability is given per item (shifted enum: 8 = RELIABLE, 9 = RELIABLE_ORDERED).

Out of scope here but related (not documented): PlayerPlaySound (16), PlayCrimeReport (112),
ShowPlayerNameTagForPlayer (80), SelectObject (27) / object edit (116/117) — those interact with
textdraw selection only as noted under §83.

---------------------------------------------------------------------------------------------------

## Shared: chat window model (A 0x10067be0 and neighbours)

Needed to reproduce what §93/§101/local messages do.

* Global chat object (A global 0x1026e9f8). Scroll-back of **100 entries**, each 0xFC bytes,
  newest last; adding an entry shifts the whole array by one (oldest dropped).
* Entry fields (own naming): timestamp (C `time()` at insertion), prefix/name (max 24 chars;
  if a longer prefix is passed the entry is **silently not added**; when present the client
  appends `:` to it), text (max 144 chars, truncated by strncpy), entry type, text colour,
  prefix colour.
* Entry types: **2** = player chat (prefix = name), **4** = info/client message, **8** = debug
  message. Types 4 and 8 render the same way; each has its own default colour in the chat object
  (used by the local printf-style helpers A 0x10068070 → type 4, A 0x100680f0 → type 8).
* Type-2 wrapping: if the text is longer than 72 chars, the client looks backwards from index 72
  for a space. If that space is at most 12 chars before index 72, line 1 = text up to the space and
  line 2 = text after it; otherwise it hard-splits at 72. Line 2 is a second entry of the same type
  with no prefix.
* Before insertion, control characters (1..31) are replaced by spaces (A 0x10067420 and inline
  copies).
* Logging (A 0x100677d0): if chat logging is enabled, the entry is appended to
  `<user files dir>\chatlog.txt` (path built at A 0x100c47d0) as `[HH:MM:SS] <name> text` for type
  2 and `[HH:MM:SS] text` for types 4/8.
* Rendering (A 0x10067940/0x10067470): optional `[HH:MM:SS]` timestamp prefix (toggle `/timestamp`),
  `{RRGGBB}` embedded colour codes handled at draw time (stripping helper A 0x100b6100 removes
  8-char `{xxxxxx}` sequences for the shadow pass). Visible lines = page size (`/pagesize`).
* Key-name expansion: server messages are passed through A 0x100b6140, which, if the text
  contains `~k~` and is shorter than 400 chars, runs GTA's key-name substitution
  (GTA 0x69E160, believed `CMessages::InsertPlayerControlKeysInString`) and keeps the result only
  if it fits in 255 chars.

## Shared: compressed string (dialog body)

Used by §61 (and by 3D text labels / object material text, outside this scope).

| Field | Type | Notes |
|---|---|---|
| bitLength | compressed u16 (RakNet compressed-integer algorithm, handshake.md §3) | number of Huffman-coded bits that follow |
| data | bitLength bits | Huffman codes, MSB-first, one code per output byte |

* Codec object: RakNet-style string compressor singleton (instance getter A 0x10053c30, decode
  A 0x100542d0, encode A 0x100541a0, language id 0 only). Decode walks the tree bit by bit
  (A 0x10026770): bit 0 → first child, bit 1 → second child; a leaf emits its byte. Output is
  truncated to the caller's buffer (4096 for dialogs) and NUL-terminated.
* Tree built once (A 0x10054460 → A 0x10026fc0) from a 256 × u32 frequency table at
  **0x10101de8** (same address and contents in A and B). Zero weights are replaced by 1, so every
  byte value is encodable. Construction repeatedly removes the two lowest-weight nodes from a
  weight-sorted list; the first removed becomes the 0-branch, the second the 1-branch; the parent
  (sum of weights) is re-inserted. This is the classic RakNet `GenerateFromFrequencyTable`
  algorithm; the tie-break order of the sorted-list insertion (A 0x10026ea0) was not re-derived
  — confirm with a round-trip test against a real server (open.mp uses the same RakNet code).
* Frequency table (index = byte value; all entries 128..255 are 0):

```
  0: 0,0,0,0,0,0,0,0,0,0,722,0,0,2,0,0
 16: 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0
 32: 11084,58,63,1,0,31,0,317,64,64,44,0,695,62,980,266
 48: 69,67,56,7,73,3,14,2,69,1,167,9,1,2,25,94
 64: 0,195,139,34,96,48,103,56,125,653,21,5,23,64,85,44
 80: 34,7,92,76,147,12,14,57,15,39,15,1,1,1,2,3
 96: 0,3611,845,1077,1884,5870,841,1057,2501,3212,164,531,2019,1330,3056,4037
112: 848,47,2586,2919,4771,1707,535,1106,152,1243,100,0,2,0,10,0
128..255: 0
```

## Shared: GTA script-opcode helper

Several handlers drive GTA through its script-command interpreter (A 0x100b2350: takes an
{opcode u16, argument-format string} descriptor plus arguments). Opcodes seen in this subsystem:
0x00BE (clear prints, before game text), 0x02A3 (widescreen on/off), 0x08D4 (create menu),
0x08DB (set menu column), 0x09DB (set menu column width), 0x08D9 (set menu row enabled),
0x08D7 (get active menu row), 0x08DA (delete menu), 0x0174 / 0x0172 (vehicle / ped heading,
for `/save`), 0x077E (active interior, for `/interior`). A DLL build of the Rust client must call
the same opcodes (or the functions they map to); the headless client only needs the state.

---------------------------------------------------------------------------------------------------

### 93 SendClientMessage (S→C)
- A 0x1000ef90, B 0x1000ef60. Display helper A 0x10068170.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | colour | u32 | RGBA |
| 2 | length | u32 | message is dropped if ≥ 256 |
| 3 | text | bytes[length] | not NUL-terminated on the wire |

- Client behaviour (observed): no connection-state check. If the text's strlen is ≤ 144, run
  `~k~` expansion (see shared section, limit 255), replace control chars with spaces, add a
  **type 4** chat entry with colour `(c >> 8) | 0xFF000000` and no prefix. Logged to chatlog.
  Texts of 145..255 chars are read but **not shown**.
- open.mp vs client: layout identical (open.mp: u32 colour, then u32-length string). open.mp's
  128-char hint is a storage hint only; the client limit is 144 displayed / 255 accepted.

### 101 PlayerChatMessage (S→C)
- A 0x100122f0, B 0x100122d0. Remote-player path A 0x10017610; add-entry A 0x10068020.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | playerId | u16 | |
| 2 | length | u8 | |
| 3 | text | bytes[length] | |

- Client behaviour (observed): handled only when the netgame state is 5 (connected; see
  handshake.md). If playerId equals the local player's id → entry with local name and local
  colour. Else if playerId < 1005 and that remote slot exists and is active → entry with that
  player's name and colour. Otherwise ignored. Colour comes from the client's player-colour table
  (A 0x100a7270(id)) converted with `(c >> 8) | 0xFF000000`. Entry is **type 2** (name prefix +
  text, 72-char wrap, chatlog line `<name> text`). Control chars → spaces. No `~k~` expansion.
- Note: the client never shows its own chat locally when sending; it relies on the server echoing
  101 back to the sender (open.mp does).
- open.mp vs client: identical.

### 101 PlayerRequestChatMessage (C→S)
- Built in A 0x10005a10 (B 0x10005a00); reached from the chat input processor (see "Local chat
  input") through the default non-command handler A 0x100684e0 (only if the netgame exists).

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | length | u8 | strlen of the typed text, truncated to 8 bits |
| 2 | text | bytes[length] | |

- Reliability 8 (RELIABLE). Input is capped at 128 chars by the input line, so the u8 never wraps
  in practice.
- open.mp vs client: identical (open.mp `readDynStr8`).

### 50 PlayerRequestCommandMessage (C→S)
- A 0x10069900, B 0x100698c0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | length | u32 | strlen of the full input line including the leading `/` |
| 2 | text | bytes[length] | the whole line exactly as typed (command + arguments) |

- Reliability 8. Sent only when the line starts with `/` and the command word is not one of the
  local commands below (and the netgame exists).
- open.mp vs client: identical layout. open.mp additionally rejects length < 2 or > 256; the
  client's 128-char input cap keeps it inside that.
- The server→client form (open.mp `PlayerCommandMessage`, u32-length string) is **not** registered
  by the client (50 is absent from the client's 141 registered IDs).

### Local chat input and built-in commands
- Input processor A 0x100699d0 (object A global 0x1026e9fc; registration A 0x10069110, B
  0x100690d0 — same table in both builds). Command table: up to 144 entries, names < 32 chars,
  lookup case-insensitive (A 0x10069710).
- Behaviour on Enter (the edit box's string event, raised on the Enter **key-up**; the typed CR and
  key-downs do nothing — INPUT-1, see ui-rendering.md §6.3):
  1. Copy edit-box text (max 128 chars). Empty → just close the input.
  2. Push into a 10-entry history (A 0x100695d0).
  3. Not starting with `/` → default handler → send RPC 101 (above).
  4. Starting with `/`: command word = text after `/` up to the first space; argument string = the
     rest after that space (or empty). If the word is a local command, call it with the argument
     string; nothing is sent. Otherwise send RPC 50 with the **whole original line**; if not
     connected (no netgame) print debug line "I don't know that command.".
  5. Clear the edit box and close the input if it was open.
- Local commands (A handler addresses; most persist a setting into the client config file via
  A 0x10066080(key, value)):

| Command | A handler | Behaviour |
|---|---|---|
| `/quit`, `/q` | 0x100689e0 → 0x100c3ec0 | if connected: RakNet disconnect (500 ms wait); then set the exit flag + tick so the game quits |
| `/save [comment]` | 0x10068a00 | append to `<user dir>\savedpositions.txt`: in a vehicle `AddStaticVehicle(model,x,y,z,heading,col1,col2); // comment`, on foot `AddPlayerClass(skin,x,y,z,heading,0,0,0,0,0,0); // comment`; prints "-> InCar/OnFoot position saved" (error text if file can't be opened) |
| `/rs [comment]` | 0x10068b80 | raw variant: `rawvehicles.txt` (`model,x,y,z,heading,col1,col2 ; comment`) or `rawpositions.txt` (`x,y,z,heading ; comment`) |
| `/rcon <text>` | 0x10069030 | send packet 201 (below) |
| `/mem` | 0x100688b0 | print "Memory: %u" with the u32 at GTA 0x8A5A80 |
| `/fpslimit N` | 0x100688d0 | N in 20..90 → set frame limiter (A 0x100a0c10), save `fpslimit`, print "-> Frame Limiter: %u"; else usage text |
| `/pagesize N` | 0x100685f0 | N in 10..20 → chat page size (setter accepts 10..100), save `pagesize`; else "pagesize [10-20] (lines)" |
| `/fontsize N` | 0x10068670 | N in -3..5 → save `fontsize`, rebuild chat/dialog/scoreboard fonts; else "Valid fontsize: -3 to 5" |
| `/nametagstatus` | 0x10068720 → 0x10008e90 | toggle the AFK/status indicator on name tags; save `nonametagstatus`; prints ON/OFF |
| `/timestamp` | 0x10068730 | toggle chat timestamps; save `timestamp` |
| `/headmove` | 0x10068960 | toggle ped head-tracking of the camera; save `disableheadmove`; prints enabled/disabled |
| `/hudscalefix` | 0x10068870 | toggle a HUD aspect-ratio fix flag (A 0x10117470); save `nohudscalefix` (inverted) |
| `/audiomsg` | 0x10068790 | toggle `audiomsgoff` (suppresses the "Audio stream: %s" line); prints "Audio messages: On/Off" |
| `/logurls` | 0x10068800 | toggle `logurls` (prints "[url 0x%X]: %s" for HTTP download requests, A 0x1000c380); prints "URL messages: On/Off" |
| `/interior` | 0x10068fd0 | print "Current Interior: %u" (opcode 0x077E) |
| `/togobjlight` | 0x10069000 | toggle an object-lighting flag (A 0x101166c0) |
| `/dl` | 0x10069020 | toggle vehicle debug labels (id, model, health, distance, positions; vehicles within 20 m; drawn in A 0x10075790) |
| `/ctd` | 0x100685e0 | toggle camera-target debug labels (A 0x10004850) |
| `/cmpstat` | 0x100689f0 | no-op (returns immediately) |
| `/testdw` | 0x10068500 | add 8 hard-coded test entries to the death window (debug) |
| `/vsel`, `/v N`, `/vehicle N`, `/player_skin N`, `/set_weather N`, `/set_time H M` | 0x10068eb0, 0x10068d70, 0x10068d00, 0x10068ed0, 0x10068f20 | **registered only in debug mode** (command line `-d`, flag A 0x1026de60 set by A 0x100c45f0): local vehicle selector / spawn, skin, weather, time (H 0..23, M 0..59) |

- Unknown: exact on-screen effect of `/togobjlight` and `/hudscalefix` (flags only traced to their
  storage). Config file name/format belongs to the settings subsystem (not covered).

### Packet 201 (0xC9) PlayerRconCommand (C→S, packet not RPC)
- Built inline in the `/rcon` handler (A label 0x10069030, B label 0x10068ff0); sent with RakClient
  Send (vtable +0x18), priority 1, reliability 8, channel 0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | packet id | u8 | 0xC9 |
| 2 | length | u32 | strlen of the argument string (may be 0 for a bare `/rcon`) |
| 3 | text | bytes[length] | everything after `/rcon ` |

- Server replies arrive as ordinary SendClientMessage (93). open.mp vs client: identical
  (`readDynStr32`).

### 59 SetPlayerChatBubble (S→C)
- A 0x1000f990, B 0x1000f960. Bubble pool add A 0x10066e10, per-frame draw A 0x10066ed0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | playerId | u16 | |
| 2 | colour | u32 | RGBA |
| 3 | drawDistance | f32 | |
| 4 | expireMs | u32 | lifetime in ms |
| 5 | length | u8 | ignored if > 144 |
| 6 | text | bytes[length] | |

- Client behaviour (observed): requires the local player pool. Accepted if playerId < 1004 and
  that player slot exists. Stores per-player bubble: text (≤ 144), colour `(c>>8)|0xFF000000`,
  distance, expiry = GetTickCount() + expireMs, line-wrapped (A 0x10017690, args 36 and 12:
  believed max chars per line and max lines). A new bubble replaces the old one. Each frame,
  expired bubbles are cleared; live ones are drawn above streamed-in remote players within the
  distance.
- open.mp vs client: identical.

### 73 SendGameText (S→C)
- A 0x100198f0, B 0x100198a0. Display A 0x100a0d40.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | style | u32 | GTA big-message style |
| 2 | timeMs | u32 | |
| 3 | length | u32 | must be 1..200 (else whole RPC ignored) |
| 4 | text | bytes[length] | |

- Client behaviour (observed): if style (as signed int) < 201: run opcode 0x00BE (clear current
  prints), copy text into a 512-byte static buffer, call GTA 0x69F2B0 (believed
  `CMessages::AddBigMessage(text, time, style)`). GTA's own `~x~` formatting applies.
- Quirk: the client does **not** restrict style to GTA's valid range (0..6); a hardened client
  should ignore or clamp larger values.
- open.mp vs client: identical (open.mp writes Style, Time, then u32-length text).

### 134 PlayerShowTextDraw (S→C)
- A 0x1001cd90, B 0x1001cd40. Pool create A 0x1001e910, textdraw init A 0x100b3720, set-text
  A 0x100b2fa0, draw A 0x100b31e0 (text) / A 0x100b30d0 (sprite).

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | textDrawId | u16 | 0..2047 global, 2048..2303 per-player (pool size 0x900) |
| 2 | data | bytes[63] | fixed block, layout below |
| 3 | textLength | u16 | must be < 800, else ignored with debug "Warning: ignoring large TextDraw size=%u" |
| 4 | text | bytes[textLength] | |

63-byte block (byte offsets; multi-byte LE):

| Off | Size | Field | Client use |
|---|---|---|---|
| 0 | 1 | flags | bit0 box; bit1 align-left; bit2 align-right; bit3 align-centre; bit4 proportional |
| 1 | 4 | letterSizeX f32 | |
| 5 | 4 | letterSizeY f32 | |
| 9 | 4 | letterColour u32 | memory byte order R,G,B,A (open.mp sends `ABGR()` value) |
| 13 | 4 | textSizeX f32 | wrap width / box extent (`SetWrapx`) |
| 17 | 4 | textSizeY f32 | centre-size (`SetCentreSize`) |
| 21 | 4 | boxColour u32 | same byte order |
| 25 | 1 | shadow u8 | used only if outline = 0 |
| 26 | 1 | outline u8 | |
| 27 | 4 | backgroundColour u32 | shadow/outline colour |
| 31 | 1 | font/style u8 | 0..3 GTA fonts, 4 = sprite (`txd:name`), 5 = model preview |
| 32 | 1 | selectable u8 | |
| 33 | 4 | posX f32 | 640×448 virtual canvas |
| 37 | 4 | posY f32 | |
| 41 | 2 | previewModel u16 | |
| 43 | 12 | previewRot x,y,z f32 | |
| 55 | 4 | previewZoom f32 | |
| 59 | 2 | previewColour1 i16 | |
| 61 | 2 | previewColour2 i16 | |

- Client behaviour (observed): requires the textdraw pool. If the id already exists it is
  destroyed first; a new object is created and marked present. Text rules (A 0x100b2fa0): text
  copied up to 800 chars; scanning `~` pairs — a `~k~` marks "needs key expansion"; a tilde code
  longer than 24 chars or an unmatched `~` replaces the text with "Error: unmatched tilde"; a `~k~`
  text longer than 255 chars becomes "Error: can't use k-codes in long string". For style 4 the
  sprite texture is (re)loaded.
- Drawing (per frame; text path A 0x100b31e0): scale = (letterSizeX·W/640, letterSizeY·H/448·0.5);
  colour = hover colour while hovered in select mode else letterColour; orientation: right if bit2,
  else centre if bit3, else left; wrap = textSizeX·W/640; centre size = textSizeY·W/640; background
  box = bit0 with boxColour; proportional = bit4; drop colour = backgroundColour; outline if
  non-zero else shadow; font = style; GTA number/key insertion (0x69DE90 / 0x69E160); print at
  (posX·W/640, posY·H/448) via CFont (GTA 0x7193xx–0x71A700, e.g. SetScale 0x719380, SetColor
  0x719430, SetOrientation 0x719610, PrintString 0x71A700). After drawing the screen-space bounding
  rectangle is stored and the "drawn" flag set — these are what selection hit-tests use.
- Quirks to harden: no bounds check of textDrawId in show/hide (array of 0x900) — Rust must ignore
  ids ≥ 2304.
- open.mp vs client: identical 63-byte layout. Alignment encoding: open.mp maps
  Left→1, Right→2, Center→4 before shifting left by 1, giving exactly the client's bit1/bit2/bit3;
  default (0) renders left-aligned.

### 135 PlayerHideTextDraw (S→C)
- A 0x1001cec0, B 0x1001ce70. Pool delete A 0x1001e7f0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | textDrawId | u16 | |

- Client behaviour: requires the pool; destroys the textdraw (frees sprite texture) and clears
  the slot. "Hide" is a delete — a later show must resend everything. No id bound check (harden).
- open.mp vs client: identical.

### 105 PlayerTextDrawSetString (S→C)
- A 0x1001cf70, B 0x1001cf20.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | textDrawId | u16 | must be < 0x900 and existing |
| 2 | textLength | u16 | accepted if ≤ 800 |
| 3 | text | bytes[textLength] | |

- Client behaviour: replaces the text with the same rules as §134 (tilde validation, `~k~`,
  sprite reload for style 4).
- Quirk: limit here is `< 801` against an 800-byte buffer (a length of exactly 800 writes the
  terminator one byte past it); the setter then ignores texts ≥ 800 chars. Rust: treat ≥ 800 as
  invalid.
- open.mp vs client: identical.

### 83 PlayerBeginTextDrawSelect (S→C) — SelectTextDraw / CancelSelectTextDraw
- A 0x1001d650, B 0x1001d600. Select-mode object A global 0x1026e9e0 (enable A 0x100714a0,
  disable A 0x10071580, per-frame A 0x10071470/0x10071370, mouse A 0x100715d0).

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | enable | bit | single bit, not a byte |
| 2 | hoverColour | u32 | RGBA; client byte-swaps to R,G,B,A memory order |

- Client behaviour (observed):
  1. Always first cancels object selection mode (A global 0x1026e9dc, if active) and object
     editing (A global 0x1026e9d8, if active; that path itself may send RPC 116/117).
  2. enable = 1: mode on, store hover colour, hovered id = 0xFFFF. Each frame while on: cursor
     mode 2 (A 0x100a0750: shows the mouse cursor and patches GTA input — NOPs 5 bytes at GTA
     0x541DF5, makes GTA 0x6194A0 return, zeroes mouse deltas at 0xB73424/0xB73428, calls
     0x541BD0/0x541DD0), then hit-test: for every present textdraw that is **selectable** and has
     been drawn, if the cursor (GetCursorPos → client coords of GTA window 0xC97C1C) is inside its
     last drawn rectangle it becomes hovered (the highest id wins) and is drawn in the hover
     colour.
  3. enable = 0: **unconditionally** runs the disable path: mode off, cursor mode 0, chat input
     closed, hovered id set to 0xFFFF and **RPC 83 (C→S) sent with 0xFFFF**, all hover flags
     cleared. So CancelSelectTextDraw always produces a "clicked INVALID" reply, even if selection
     was not active.
- open.mp vs client: identical.

### 83 OnPlayerSelectTextDraw (C→S)
- Send A 0x100714e0, B 0x100714b0. Reliability 9 (RELIABLE_ORDERED).

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | textDrawId | u16 | hovered id, or 0xFFFF = cancelled |

- Triggers (observed): left button release (window message 0x202) while select mode is on and a
  textdraw is hovered → send its id (selection stays on; message consumed). ESC as WM_CHAR 0x1B
  while on (A 0x100619a3) → disable path → send 0xFFFF. Server RPC 83 with enable = 0, RPC 27
  (A 0x1000e8b0) and RPC 117 (A 0x100118a0) handlers also call the disable path (the latter two
  only if select mode is on) → send 0xFFFF.
- open.mp vs client: identical (ids ≥ 2048 are per-player textdraws, 0xFFFF invalid).

### 61 ShowDialog (S→C)
- A 0x1000faf0 → dialog object (A global 0x1026e9c8, ctor 0x1006f480) method A 0x10070010.
  B 0x1000fad0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | dialogId | i16 | negative → close |
| 2 | style | u8 | 0..5, see below |
| 3 | titleLength | u8 | |
| 4 | title | bytes[titleLength] | client keeps first 64 chars |
| 5 | button1Length | u8 | |
| 6 | button1 | bytes | |
| 7 | button2Length | u8 | |
| 8 | button2 | bytes | |
| 9 | body | compressed string | see shared section; decoded up to 4096 bytes |

- Client behaviour (observed):
  * If decoded **title or body is empty the RPC is ignored entirely** (nothing shown, nothing
    closed).
  * dialogId < 0: if a dialog is open, close it (cursor mode 0, hide, "open" flag cleared), no
    response sent.
  * Otherwise: close the chat input, store id/style, mark "response goes to server" (third state
    flag; local dialogs opened elsewhere set it to 0), copy title, keep a heap copy of the body and
    lay out by style:
    - 0 MSGBOX: body text only.
    - 1 INPUT: body + single-line edit box (cleared).
    - 2 LIST: listbox, one row per `\n` line; width clamped 400..800 px, height 200..400 px.
    - 3 PASSWORD: as INPUT but the edit box masks characters.
    - 4 TABLIST: rows split by `\n`, columns by `\t`, up to 4 columns, column widths from the
      widest cell.
    - 5 TABLIST_HEADERS: as 4, first line is the header row.
  * Buttons: button1 always shown; if button2 is empty only one button is shown.
  * Focus goes to the edit box (1, 3) or listbox (2, 4, 5); cursor mode is enabled; dialog marked
    open.
- open.mp vs client: identical layout. open.mp's hide sends id = -1 with single-space strings,
  which matters because empty title/body would be ignored by the client.

### 62 OnPlayerDialogResponse (C→S)
- A 0x10070690, B 0x10070660. Reliability 9.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | dialogId | u16 | low 16 bits of the stored id |
| 2 | response | u8 | 1 = button1/Enter, 0 = button2/ESC |
| 3 | listItem | i16 | selected row for styles 2/4/5 (data rows, header excluded), else -1 |
| 4 | textLength | u8 | |
| 5 | text | bytes[textLength] | omitted when length 0 |

- Text: styles 1/3 → edit-box contents (≤ 128 chars); styles 2/4/5 → text of the selected row's
  first column (≤ 128) with `{RRGGBB}` colour codes stripped; style 0 → empty.
- Triggers (observed): button1 click (GUI control 20, event 0x101) or list activation (event
  0x701) → 1; Enter in the edit box (control 19, event 0x601, raised on the Enter key-up) → 1 unless the chat input consumed it;
  Enter key (VK 0x0D, A 0x10061399) → 1; button2 click (control 21) → 0; ESC (VK 0x1B, A
  0x100615f9 / WM_CHAR path A 0x10061899) → 0.
- After building/sending (only if the "to server" flag is set) the dialog is always closed
  locally (A 0x1006f860: cursor mode 0, hidden, open flag cleared). The client does not wait for
  the server.
- open.mp vs client: identical. open.mp validates that id matches the active dialog and listItem
  range; the client always sends -1 for non-list styles, as open.mp expects.

### 76 PlayerInitMenu (S→C)
- A 0x1001c870, B 0x1001c820. Menu pool (player-pool sub-object +0x20): create A 0x10007eb0,
  delete A 0x10007f70; menu object init A 0x100a7460, title A 0x100a7570, header A 0x100a7640,
  item A 0x100a75c0.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | menuId | u8 | valid 0..127 |
| 2 | twoColumns | u32 | 0/1 (only low byte used: columns = value+1) |
| 3 | title | bytes[32] | fixed, NUL-padded |
| 4 | posX | f32 | |
| 5 | posY | f32 | |
| 6 | col1Width | f32 | |
| 7 | col2Width | f32 | **present only if twoColumns ≠ 0** |
| 8 | menuInteractive | u32 | 0 = display-only (no selection possible) |
| 9 | rowEnabled[12] | 12 × u32 | |
| 10 | col1Header | bytes[32] | |
| 11 | col1Count | u8 | if > 12 the item list **and the whole second column are skipped** |
| 12 | col1Items | col1Count × bytes[32] | |
| 13 | col2Header | bytes[32] | only if twoColumns |
| 14 | col2Count | u8 | only if twoColumns; > 12 → items skipped |
| 15 | col2Items | col2Count × bytes[32] | only if twoColumns |

- Client behaviour: if menuId < 129 and the slot exists, delete it; create a new menu object
  (stores pos, widths, column count, interactive flag + 12 row flags), then title/headers/items
  (each string accepted if < 33 chars). Not shown until §77.
- Quirk: the create path itself does not range-check menuId (only u8), but show/hide ignore ids
  ≥ 128 — Rust should reject ≥ 128.
- open.mp vs client: identical.

### 77 PlayerShowMenu (S→C)
- A 0x1001cb90 → A 0x10007fc0. B 0x1001cb40.

| # | Field | Type |
|---|---|---|
| 1 | menuId | u8 |

- Client behaviour: id < 128 and existing → if another menu is current, delete its GTA menu
  (opcode 0x08DA); build the GTA menu (A 0x100a7780): opcode 0x08D4 create (title GXT key
  `SAMPHED` or `DUMMY` if empty, x, y, width = (col1Width+col2Width)/columns, columns,
  interactive, 1, 1), opcode 0x08DB per column with 12 GXT keys `SAMP0rr` (column 0) / `SAMP1rr`
  (column 1) or `DUMMY` for empty cells, column header key `SAMPRW1`/`SAMPRW2`, opcode 0x09DB
  column width, opcode 0x08D9 for each of the 12 row-enabled flags. The client's GXT hook resolves
  those keys to the stored strings (key table A 0x100fe0c8). Current menu := id, "awaiting
  server" flag cleared.

### 78 PlayerHideMenu (S→C)
- A 0x1001cc40 → A 0x10008020. B 0x1001cbf0.

| # | Field | Type |
|---|---|---|
| 1 | menuId | u8 |

- Client behaviour: id < 128, some menu is current, and slot exists → delete the GTA menu
  (0x08DA) and set current = none (0x80). Note it hides the *current* GTA menu even if menuId is a
  different existing menu.

### 132 OnPlayerSelectedMenuRow (C→S) and 140 OnPlayerExitedMenu (C→S)
- Both from the per-frame menu poll A 0x10008200 (B 0x100081f0). Reliability 8.

| ID | Field | Type | Notes |
|---|---|---|---|
| 132 | row | u8 | active row from opcode 0x08D7 |
| 140 | — | — | empty payload (sent with a null bitstream) |

- Poll logic (observed; GTA pad 0 state, A global 0x10114ae8 → CPad, offsets in the new/old
  controller state): only while a menu is current.
  * "Cross"/sprint-select just pressed (new +0x20 ≠ 0, old +0x50 = 0): read active row (returns
    0xFF without querying GTA if the menu is not interactive); if row ≠ 0xFF set "awaiting" and
    send 132.
  * else "Triangle"/enter-exit just pressed (new +0x1E ≠ 0, old +0x4E = 0): set "awaiting" and
    send 140.
  * else if "awaiting" is set: hide the menu locally (as §78) and clear the flag — i.e. the menu
    closes one frame after a selection/exit regardless of the server.
- open.mp vs client: identical.

### 41 PlayAudioStreamForPlayer (S→C)
- A 0x1001d3c0 → A 0x10066960 (audio object A global 0x1026ea04, init A 0x10066480, worker
  thread A label 0x10066700). B 0x1001d370.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | urlLength | u8 | |
| 2 | url | bytes[urlLength] | kept up to 256 chars |
| 3 | posX | f32 | |
| 4 | posY | f32 | |
| 5 | posZ | f32 | |
| 6 | distance | f32 | |
| 7 | usePos | u8 | 0/1 |

- Audio init (once at startup, A 0x10066480): `BASS_Init(-1, 44100, 0, 0, 0)`; net user-agent
  `SA-MP/0.3`; stream global volume = GTA radio volume; `BASS_CONFIG_NET_PLAYLIST` = 1;
  net timeout 10000 ms; if config `audioproxyoff` (created with default 0) is set, proxy disabled;
  EAX off. Audio object "ready" only if BASS_Init succeeded.
- Client behaviour (observed):
  1. Ignored if the audio object is missing or not ready.
  2. Stop any current stream and **wait** for the worker to exit.
  3. If GTA's radio volume (float at GTA 0xB5FCC8) is not > 0, do nothing more (stream never
     starts).
  4. Store URL, position, distance, usePos; clear station/title strings. Unless `audiomsgoff`
     is set, print type-4 line "Audio stream: %s" (URL). Start the worker thread.
  5. Worker: `BASS_StreamCreateURL(url, 0, BLOCK|STATUS|AUTOFREE (0x940000), NULL, NULL)` and play.
     Parse HTTP/ICY headers (tags 4 then 3): `icy-name:` → station name, `icy-url:` → station URL.
     Title from meta tag `StreamTitle='…';` or OGG `artist=`/`title=` ("artist - title");
     refreshed by BASS syncs META (4) and OGG_CHANGE (0xC).
  6. Every 20 ms until stopped, set `BASS_CONFIG_GVOL_STREAM`:
     - 0 if GTA's menu is active (GTA 0xBA67A4 ≠ 0) or the game window (GTA 0xC97C1C) is not the
       foreground window;
     - usePos = 1: base = radioVolume × 8000; if the local ped exists, d = 3D distance from ped to
       (posX,posY,posZ); volume = base × (1 − d/distance) when d < distance, else 0; 0 if no ped;
     - usePos = 0: radioVolume × 7000.
  7. On exit: stop and free the stream, clear "playing".
  - While playing, every frame the in-game radio is stopped (A 0x10066a80 → GTA 0x507DC0 and
    0x506F70, believed radio-stop helpers), and a line "<title> - <station>" is drawn at the
    bottom of the screen in 0x99FFFFFF when both are known (A 0x10066ab0).
- No URL validation/whitelisting in the client (any scheme BASS accepts).
- open.mp vs client: identical.

### 42 StopAudioStreamForPlayer (S→C)
- A 0x100180f0 (B 0x100180a0). Empty payload.
- Client behaviour: if the audio object exists → A 0x10066560 with "don't wait": if a stream is
  playing, raise the stop flag for the worker and free the BASS stream immediately.
- open.mp vs client: identical.

### 30 TogglePlayerClock (S→C)
- A 0x1000f500, B 0x1000f4d0. Clock toggle A 0x100a14a0.

| # | Field | Type |
|---|---|---|
| 1 | enable | u8 |

- Client behaviour: enable ≠ 0 → GTA clock runs (first byte of GTA 0x52CF10 restored to 0x56)
  and the HUD clock format string at GTA 0x859A6C is set to `%02d:%02d`; netgame setting
  "server controls time" := 0. enable = 0 → GTA clock frozen (0x52CF10 patched to `ret`), HUD clock
  string emptied (clock hidden), the current game hour/minute (GTA 0xB70153 / 0xB70152) are copied
  into the netgame settings and "server controls time" := 1.
- open.mp vs client: identical.

### 111 ToggleWidescreen (S→C)
- A 0x10018ac0, B 0x10018a70.

| # | Field | Type |
|---|---|---|
| 1 | enable | u8 |

- Client behaviour: prints a **type 8 debug chat line "Widescreen = %i"** (visible in chat), then
  runs opcode 0x02A3 with the value (GTA cinematic letterbox bars).
- open.mp vs client: identical.

---------------------------------------------------------------------------------------------------

## Unknowns / not verified

* Huffman tie-break order (sorted-list insertion) — not re-derived; needs a round-trip test.
* Exact GTA identities of 0x507DC0/0x506F70 (radio stop), 0xB5FCC8 (radio volume) and the pad
  button names at CPad offsets 0x1E/0x20 are public-knowledge beliefs consistent with behaviour.
* Chat bubble wrap parameters (36, 12) are inferred from the call only.
* `/togobjlight`, `/hudscalefix`: effect of the toggled flags not traced.
* Which client paths open local (non-server) dialogs with the "to server" flag = 0 was not
  investigated.
