# SA:MP 0.3.7-R4 client — connection, transport and join protocol (clean-room spec)

Source binaries (read-only references, 32-bit PE, base 0x10000000, no symbols):

* **A** = `re/samp-dll/0.3.7-R4_fv0.3.7.4_2021-05-07/samp.dll` (sha256 0b0e409c…9088). All addresses below are A unless marked B.
* **B** = `re/samp-dll/0.3.7-R4_fv0.3.7.3_2019-11-21/samp.dll` (sha256 15db80c5…12ab). Protocol-identical to A, see `build-diff-r4.md`.

Method: REA (Ghidra 12.1.2) full pseudocode export to a private scratch dir, targeted reads, plus local `objdump` disassembly of the same files for instruction-level confirmation. No pseudocode is reproduced here; layouts and algorithms are restated in my own words.

REA Evidence IDs (A unless noted):

| What | Evidence ID |
|---|---|
| full pseudocode export A (`decompile_all`, all 5571 procs) | `ev_effc083a09f1a2cc16801e5554196309c5e712e82f429aa52b6883bedc717efa` (first page `ev_ee24bfbf…`) |
| full pseudocode export B | `ev_51b698f5e4081f48bfa501de67cae66637707dc376a77092e934db90df5fe724` |
| datagram cipher functions 0x1001f630/660/6b0/710 | `ev_685355d01ddec2e759357a1e2fe00da0fed91318b03f9556b33d1d48af5e2e8f` |
| cipher S-box bytes A @0x100fe500 | `ev_27f4ca5658a40644421f370e31fdc216d51cda8069a7b268dd0076566ab0fe8b` |
| cipher S-box bytes B @0x100fe500 | `ev_a6bbaa7bb5eba9489152197e134f19e8318ec675fa74fad021ccb4d963dd0d1c` |
| recvfrom wrapper assembly 0x10053a00 | `ev_30f7328cb2858b5fe85b81bc31ff6d72586839f19e05e22cd36fdb309aaeaaef` |
| "Connecting to" xref → 0x10008940 | `ev_99336ec9619f15699c44c7f1d8e6f08cdd84eaef57a44b54b747ceb0bc32300d` |
| "Incorrect Version" xref → 0x10010540 | `ev_2e1dd69edc6f387732958d73b5906e927fbde4076434a790c5faa252201fd2d4` |

Snapshots: `re/snapshots/R4_2021.snapshot`, `re/snapshots/R4_2019.snapshot`.

Conventions: all multi-byte integers on the wire are **little-endian** (RakNet bitstream copies native x86 memory order; no byte swapping seen in the read/write primitives). Bits inside a byte are filled **MSB first** (bit reader tests `0x80 >> (pos & 7)`, A 0x10008420). "uN" = N-bit unsigned field written with the bitstream "write N bits" primitive (A 0x1001ffe0 write / 0x1001f9b0 read); fields are *not* byte aligned unless said so. "bit" = single bit (write 1: 0x1001ff30, write 0: 0x1001ff10).

---------------------------------------------------------------------------------------------------

## 1. Connection establishment at the UDP level — status: ANSWERED (server-side ordering partly inferred)

### Observed

**Application layer driver.** The netgame object (constructor A 0x1000b930) stores host (≤0x101 chars), port, nickname and password, creates the RakClient, sets the password (vtable +0x10) and seeds the datagram cipher key from the port (§2). Its tick (0x10008940) runs when the netgame state (field +0x3cd) is 1 ("waiting to connect"): if more than **3000 ms** (GetTickCount) passed since the last attempt it logs "Connecting to %s:%d...", calls RakClient::Connect(host, port, clientPort=0, 0, threadSleep=2), and sets state 2.

**RakPeer connect request.** Connect creates a pending-request record (0x1003e6a0): remote address, next-attempt time = now, attempt counter = 0, password bytes and length copied into the record.

**Request loop** (inside the RakPeer update, 0x10041450). For every pending request whose next-attempt time has passed:

* if attempts < 6 and the address is valid: increment attempt counter, next attempt = now + **1000 ms**, and send a 3-byte *unconnected* datagram (via the normal encrypted sendto path, §2):

  | offset | size | value |
  |---|---|---|
  | 0 | u8 | `0x18` (open-connection request) |
  | 1 | u16 LE | `0x6969 XOR cookie` |

  `cookie` is a process-global u16 (A 0x10118e20, .data, initial value 0). The constant 0x6969 is read from A 0x100fe7e0 (bytes `69 69 00 00`).
  The cookie has exactly one writer (the `0x1a` handler, A 0x1003fdc2; B 0x1003fd72) and one reader (this send, A 0x100416a5; B 0x10041655): nothing clears it on Connect / Initialize / Disconnect. So after the first join every later attempt of the process — including the reconnect after a lost connection, on its new socket — starts with the last issued cookie, and a server whose cookie for this address has not changed (open.mp reseeds its cookie table every `network.cookie_reseed_time`, default 300 s) accepts the very first request (`0x19`) without another `0x1a` round. Measured: docs/phase3/reconnect-notes.md.
* if attempts reached 6: the request is dropped and the application receives a synthetic one-byte packet `0x1d` (connection attempt failed), which the client UI reports as "The server didn't respond. Retrying.." and returns to state 1 (so the next try is after 3000 ms).

**Unconnected datagrams accepted by the client** (ProcessNetworkPacket, 0x1003f440; sender not a connected system):

| first byte | length rule | client action |
|---|---|---|
| `0x1a` | exactly 3 bytes | store bytes 1–2 (u16 LE) into the global cookie. No address check, no immediate resend; the next periodic `0x18` (≤1 s later) carries `0x6969 ^ cookie`. |
| `0x19` | ≤ 2 bytes | for every pending request to that address: create/reuse the remote-system slot, put it in "requested connection" mode (value 4), and send **reliably** (reliability 8, system priority, channel 0) a message `0x0b` followed by the raw password bytes (no length prefix; may be empty). This is the first message that goes through the reliability layer (§3). |
| `0x1d`, `0x1f`, `0x24` | ≤ 2 bytes | if a matching pending request exists, it is removed and the app receives a one-byte packet with the same ID (0x1d "didn't respond", 0x1f "server is full", 0x24 "banned"). |
| `0x07`, `0x08` | exactly 5 bytes | offline ping: answered with `0x27` + u32 time + offline-ping data (server-role behavior; client normally not pinged). |
| `0x27` | 5…0x194 bytes | forwarded to the app (pong). |
| `0x37` | 2…0x191 bytes | forwarded to the app. |
| `0x18` | exactly 3 bytes | server-role handling (reply `0x19 00` / `0x1f 00` / `0x1d 00`); the 2 cookie bytes are **not validated** in this binary's server-role code. |
| any, length > 512 | — | sender is temp-banned for 10000 ms (server-role guard). |

**Connected-mode handshake messages** (all inside reliability-layer datagrams; parsed in 0x10041450):

* `0x0c` (auth challenge) – forwarded to the app; see §4.
* `0x22` connection-request-accepted — accepted only if the message is **exactly 13 bytes** and the slot is in "requested connection" or "handling request" mode. Layout:

  | offset | size | field |
  |---|---|---|
  | 0 | u8 | `0x22` |
  | 1 | u32 | client's address as seen by the server (stored as "my external IP") |
  | 5 | u16 | client's port as seen by the server |
  | 7 | u16 | player index assigned by the server |
  | 9 | u32 | SA:MP join challenge (only the app reads it, §5) |

  On receipt RakPeer sets the slot to "connected" (mode 8), enables RakNet block encryption only if a secure-connection key had been negotiated (it is not in the SA:MP flow, see §2), forwards the 13-byte packet to the app, then sends reliably (rel 8, system priority) `0x1e` + u32 server IP + u16 server port (the address it received from), and immediately sends an internal ping (§3).
* `0x20` disconnection notification → slot goes to "disconnect on no ack" mode; app later shows "Server closed the connection." (app handler 0x10008de0).
* `0x0b`/`0x1e` in server-role modes, `0x0f`/`0x10` secure-connection messages (stock RakNet; lengths 0x39/0x35), `0x0a` static-data request, `0x29` static data (to app), `0x11` (dropped), `0x17` (dropped) are also recognised.
* Any other message with ID < 0x14 is dropped; IDs ≥ 0x14 go to the app (RPC 0x14 is intercepted earlier, §6).

**Timeouts / retries seen**

| item | value | where |
|---|---|---|
| app reconnect interval | 3000 ms | 0x10008940 |
| open-connection request retry | every 1000 ms, max 6 sends | 0x10041450 |
| handshake-mode timeout (slot modes 4–7) | normal processing is skipped once > 10000 ms passed since the slot's connect time; inferred: slot is dropped | 0x10041450 (compare 0x2711) |
| keep-alive | if connected and no reliable traffic for > 5000 ms, send 1-byte `0x17` (rel 8, low priority) | 0x10041450 |
| periodic ping | every 5000 ms while connected (when occasional ping enabled or no ping sample yet) | 0x10041450 → 0x1003f1a0 |
| "connection lost" app reaction | logs "Lost connection to the server. Reconnecting..", tears down players, state 1 | 0x1000acf0 (ID 0x21) |
| wrong password | app calls Disconnect | dispatch 0x1000b260 |

### Inferred

Expected client-side sequence against a server that implements the cookie scheme:

```
C→S  18 69 69                     (cookie unknown, 0x6969^0)
S→C  1a <cookie u16>              (server issues cookie)
C→S  18 <0x6969^cookie u16>       (next 1 s tick)
S→C  19 00                        (open-connection reply)
C→S  [reliable] 0b <password>     (connection request)
S→C  [reliable] 0c <len><challenge>      (§4)
C→S  [reliable] 0c <len><response>
S→C  [reliable] 22 <13 bytes>     (accepted)
C→S  [reliable] 1e <ip><port>  ;  [unreliable] 06 <time>
                                  ;  [reliable] 29  ;  [reliable] 17   (see note below)
C→S  [RPC 25 ClientJoin]          (§5)
S→C  [RPC 139 InitGame] or [RPC 130 rejection]
```
Each line is one or more logical messages; every C→S datagram is additionally ciphered (§2). The order of `0c` vs `22` on the server side is inferred (client accepts `0c` while its netgame state is 1…6, which covers both orders).

Note that a `0x1a` reply consumes one of the 6 attempts; the client never resets the global cookie between servers.

### Command line and server password (observed, static; A addresses, B in docs/re/addresses/cmdline.md)

**Command line.** At game start samp.dll (A 0x100C45F0) takes the raw `GetCommandLineA` string, zeroes its
argument block (A 0x1026DE60, 0x511 bytes) and walks the string one character at a time. Wherever it meets
`-` or `/` followed by one of `c d h l n p z` it handles a switch: `c` sets "connect mode", `d` "debug mode",
`h` / `n` / `p` / `z` copy a value into fixed 0x101-byte buffers (host 0x1026DF6D, name 0x1026E16F, port text
0x1026E06E, **password 0x1026DE6C**), `l` copies a double-quoted value into a fifth buffer. Other letters
are ignored. The walk does not skip over a value it has copied, so it also scans inside values (and the exe path).

The value copy (A 0x100C3F00) starts right after the switch letter (`-zPASS` and `-z PASS` both work), skips
leading spaces, then copies characters up to the first **space, `-` or `/`** (or the end), and terminates the
buffer. It has no length check. Consequences: a password cannot contain a space, `-` or `/` (it is cut
there, and a `-x` / `/x` inside it is read as another switch); quotes are not removed (`-z "a b"` gives `"a`);
`-z` followed directly by another switch gives an empty password; without `-z` the buffer stays empty.
The string is the ANSI command line, so bytes are those of the system code page.

**Into RakNet.** The game-init routine (A 0x100C47D0) constructs the netgame with host, `atol(port)`, name and
the password buffer; the netgame constructor calls RakClient::SetPassword (vtable +0x10, A 0x10034220) once.
SetPassword resets its password bitstream; if the string is non-empty it writes `strlen + 1` bytes, i.e.
**the password with its terminating NUL**; an empty (or null) password leaves the bitstream empty.
Every RakClient::Connect (A 0x10034130; also each reconnect) passes `(bits + 7) / 8` bytes of that bitstream to
RakPeer::Connect (A 0x10040550), which cuts the length to **255 bytes** (so for passwords of 255+ characters
the NUL and the tail are lost) and copies the bytes into the pending-request record. On the server's
open-connection reply `0x19` the client sends, reliably (rel 8, system priority, channel 0):

| case | `0x0b` message |
|---|---|
| no `-z` / empty | `0b` |
| `-z pw1` | `0b 70 77 31 00` |
| 300 × `a` | `0b` + 255 × `61` (no NUL) |

The server side (open.mp's RakNet fork, rakserver.cpp `SetPassword`) also stores `strlen + 1` bytes, and its
connection-request handler accepts only if the length and all bytes match; otherwise it sends `0x25`
(ID_INVALID_PASSWORD, reliable, system priority) and drops the connection silently. A server without a
password stores length 0, so a client that sends any password there is refused the same way.

**Wrong password on the client.** RakNet hands `0x25` to the packet handler (A 0x1000B260), which prints
"Wrong server password." and calls RakClient::Disconnect; there is no retry path from this handler.

**Differential capture (2026-11-04, original client under Wine vs open.mp 1.6.7):** right after
`0x22` the original client sends, in this order and in one burst: `1e`+ip+port (rel 8), `06`+time
(rel 6), **`29` with no payload (rel 8, RakNet static data — missing from the static analysis
above)**, **`17` (rel 8, sent immediately, not only after 5 s idle)**, a second `06`, then RPC 25.
The server also sends an empty `29`. RPC 25 layout matched byte-for-byte in structure, including
the trailing duplicate challenge response.

### Unknown

* Exact content the server expects in `0x19` (client accepts 1 or 2 bytes and ignores content).
* Whether a cookie mismatch produces a server reply at all.

---------------------------------------------------------------------------------------------------

## 2. Datagram encryption / obfuscation — status: ANSWERED (client→server); server→client is plaintext

### Observed

* **Every** datagram leaves through one sendto wrapper (A 0x10053ab0; RakPeer passes all traffic, connected and unconnected, through it). Before sendto it calls the encoder 0x1001f6b0 which writes into a global buffer (A 0x10119a38) and the wire length becomes **n + 1**. The send is retried while sendto returns 0.
* The receive wrapper (0x10053a00, recvfrom into a 0x5d4 = 1492-byte stack buffer) hands the bytes **unchanged** to ProcessNetworkPacket — there is no decoding of server→client datagrams in this client and the checksum is not verified on receive.
* Key setup (0x1001f710, called once from the netgame constructor with the server port): `key16 = port XOR 0xCCCC`, stored at A 0x10118b74. The encoder loads only the **low byte** (`mov al, [0x10118b74]`), so effectively `k = (port & 0xFF) XOR 0xCC`.

**Algorithm** (input plaintext P[0..n-1], output W[0..n]):

```
W[0] = XOR over i of (P[i] AND 0xAA)                 ; checksum of the plaintext
for i in 0..n-1:
    t = SBOX[P[i]]
    W[i+1] = t            if i is even
           = t XOR k      if i is odd
```

* SBOX = 256-byte table at A **0x100fe500** (.data; only referenced read-only by 0x1001f660). It is a permutation of 0..255 (verified: 256 distinct values). Dumped to `data/datagram_sbox.hex` (sha256 of the 256 bytes: 76c2d10769fe41a4ab9dc2bfe60fcc316d266f52daf659e23dc97a1c702efa81). Identical bytes at the same VA in B.
* The even-position XOR value is literally the constant 0 passed by the caller (`push 0`); i.e. even positions are only substituted.

Worked example (computed from the dumped table): port 7777 → k = 0xAD; plaintext `18 69 69` → wire `08 1e 77 da`.

A server must invert this: check `W[0]`, then `P[i] = SBOX⁻¹[W[i+1] XOR (i odd ? k : 0)]` (inverse table derivable from the dump; the client binary contains no inverse table).

### Also present (stock RakNet, inactive in SA:MP flow)

RakNet's block encryptor (A 0x10056990 flag / 0x10056cd0 encrypt) is only switched on when a key exists from the RakNet secure-connection exchange (`0x0f`/`0x10`). The SA:MP client never initiates that; app string "Failed to initialize encryption." appears only as the reaction to ID 0x23.

### Unknown

* Nothing material for client→server. Whether real servers ever send `0x23`.

---------------------------------------------------------------------------------------------------

## 3. RakNet layer — status: ANSWERED (formats observed; timing, packing, resend and ping rules in docs/phase3/net6-notes.md §1, measured)

Evidence that this is a RakNet 2.x-era library: RakNet console strings (e.g. RakPeer::Connect), RakPeer/ReliabilityLayer object layout, the "connect mode" values 0…8, RangeList-style acks, DataBlockEncryptor and secure-connection messages 0x0f/0x10 of the stock design. I did not diff against stock RakNet source; differences listed below are relative to what the binary does, with "stock" claims marked as belief.

### Datagram format (connected peers; parse A 0x10049250, build A 0x10048070)

Datagram length must be ≥ 2 bytes. Bit-level layout after removing the §2 cipher (client→server) / as received (server→client):

| field | encoding |
|---|---|
| hasAcks | bit |
| if hasAcks: ack ranges | count = compressed u16 (written before the ranges); then per range: bit `minEqualsMax`, u16 min, (u16 max if bit = 0). Message numbers are 16-bit. (writer 0x10046f40, reader 0x10047d20) |
| then 0…N messages until fewer than 16 bits remain | see next table |

Per-message header (writer 0x10045530, reader 0x10045650):

| field | encoding |
|---|---|
| messageNumber | u16 |
| reliability | **4 bits** (width stored in the layer object, initialised to 4 at A 0x10048e10). Reader rejects values ≤ 5. |
| if reliability ∈ {7, 9, 10} | orderingChannel u5, orderingIndex u16 |
| isSplit | bit |
| if isSplit | splitPacketId u16, splitPacketIndex compressed u32, splitPacketCount compressed u32 |
| dataBitLength | compressed u16 |
| data | **aligned** to the next byte boundary, then ⌈bits/8⌉ bytes; rejected if ≥ 0x5d4 bytes |

Reliability values on the wire (inferred from which carry ordering fields and which are acked: ordering for 7/9/10, acks for 8/9/10): 6 UNRELIABLE, 7 UNRELIABLE_SEQUENCED, 8 RELIABLE, 9 RELIABLE_ORDERED, 10 RELIABLE_SEQUENCED. I believe stock RakNet numbers these 0–4; the +6 shift is consistent with the public hypothesis but the stock numbering was not checked against source.

**Compressed integers** (writer 0x10020130, reader 0x1001fa70; stock RakNet algorithm, unsigned variant used everywhere here): process the value's little-endian bytes from most- to least-significant; while the current high byte is 0x00 (and it is not the last byte), emit bit 1 and drop it; on the first non-zero byte emit bit 0 followed by all remaining bytes (`(k+1)*8` bits). For the final (least-significant) byte: if its high nibble is 0 emit bit 1 + low 4 bits, else bit 0 + 8 bits.

**MTU / splitting**: default MTU 576 (A 0x1003de50 initialiser), SetMTU clamps to 1492 (0x10038860). Split payload per fragment = MTU − 28 − header bytes (− 36 if block encryption on) (0x10045800). The SA:MP client never calls SetMTU (RakClient vtable slot 42 has no caller), so the MTU is 576. Note the cipher adds 1 byte to every client datagram.

### Received message numbers: duplicate filter (A 0x10049250; observed)

Every message the peer sends takes the next value of ONE 16-bit message-number counter, whatever its
reliability (syncs included), so the counter wraps every 65536 messages (≈ 10 minutes of a busy server
stream). The receiver keeps a window: a base number B (the oldest number not yet accounted for; 0 on a
new connection) and one slot per number from B on, each "received" or "hole" (a hole stores its give-up
time, receive time + the layer's timeout of 10000 ms, layer +0x2d3 set at A 0x10048e10). For every
message, before split reassembly and ordering, with d = number − B (16-bit wrapping):

* d = 0: the slot of B is taken (dropped from the window) and B advances;
* 1 ≤ d ≤ 0x8000: if d lies inside the window, a "received" slot means a duplicate (dropped, counted at
  layer +0x3d3) and a hole is marked received; beyond the window, holes are appended up to d and the
  slot d is appended as received;
* d > 0x8000 (behind B): duplicate, dropped.

After an accepted message B advances over leading received slots and over holes whose give-up time has
passed. Acks (reliability 8/9/10) are queued before this test, so a duplicate is acknowledged again.
A filter that instead remembers recently seen numbers (without a base) takes a new message for an old
one once the counter has wrapped; a reliable ordered RPC dropped that way never comes again (it was
acked) and holds back every later message of its ordering channel.

### Internal ping / time sync (0x10041450, 0x1003f1a0)

| msg | layout | reliability | behaviour |
|---|---|---|---|
| `0x06` internal ping | u8 0x06, u32 sender time (ms) — exactly 5 bytes | 6 (unreliable), system prio | receiver answers with `0x09` (next update cycle). The client pings on `0x22` and once more from its update cycle, then only while no pong has been accepted (5 s apart): measured 2 pings per session |
| `0x09` connected pong | u8 0x09, u32 echoed ping time, u32 responder time — exactly 9 bytes | 6 | RTT = now − echoed; sample stored if the ring slot about to be overwritten is empty, or RTT < 3 × that slot's value and RTT < 1200 ms; clock offset = responderTime − (echoed + now)/2; ring of 5 samples; lowest ping tracked |
| `0x17` keep-alive | 1 byte | 8, low prio | sent when connected, nothing awaits an ack and > 5000 ms passed since the last reliable send; then 15 s apart |

Time base: RakNet time in ms derived from QueryPerformanceCounter (0x1002e7f0 ms, 0x1002e850 µs).

### Timestamp prefix

A message may start with `0x28` + u32 time; the real ID is then at byte 5 (app dispatcher and RPC handler both honour this).

### Timing (answered, NET-6)

Update thread every 2 ms; acks every cycle (standalone unless a send coincides); resend timeout 3 × max(ping, 30 ms) (3000 ms before the first pong), no backoff; priority queues; packing by RakNet's header estimate within (MTU − 28) × 8 bits. Details, addresses and measurements: docs/phase3/net6-notes.md §1. Not analysed further: the bandwidth bucket (never limits the client's rates), sequencing/ordering buffer limits, split reassembly limits.

---------------------------------------------------------------------------------------------------

## 4. Post-connect auth key — status: PARTIAL (framing answered; response computation contains anti-tamper, deliberately not specified)

### Observed

App handler for ID `0x0c` (0x1000a100), active only while the netgame state is 1…6:

* Incoming: `u8 0x0c`, `u8 L`, `L` bytes challenge string (not NUL-terminated on the wire).
* Outgoing (RakClient::Send, reliability 8, system priority, channel 0): `u8 0x0c`, `u8 M`, `M` bytes response string. M is 40 in practice: the response is the **uppercase hexadecimal** rendering (nibble → '0'–'9','A'–'F', 0x100576a0/0x100576b0) of a 20-byte value.
* The 20-byte value is produced by 0x100576f0. It starts from **SHA-1 of the challenge string** (standard SHA-1 initial values and round constants observed in 0x10063ae0/0x10063b70) and then combines it with data obtained from the running game process and from a client resource lookup. That combination is integrity/anti-tamper logic; per the clean-room rules its details are not documented. The whole computation is gated on game-side objects existing; if they don't, no value is written.

### Inferred

A client that does not run inside the original game process cannot reproduce the response from the challenge alone using only what is documented here. The client DLL's own server-role auth check (0x10038dd0) is a stub, so the binary gives no hint of how servers validate the response.

### Unknown

How samp-server/open.mp verify the response (public hypothesis: a fixed table of challenge/response pairs — UNVERIFIED, nothing in this client supports or refutes it). No auth tables were found in the client to dump.

---------------------------------------------------------------------------------------------------

## 5. Join — status: ANSWERED

### App-level packet dispatch (A 0x1000b260, B 0x1000b250)

The app pulls packets from RakClient (vtable +0x20 receive, +0x24 deallocate). ID = byte 0, or byte 5 when byte 0 is 0x28.

| ID | handler (A) | behaviour |
|---|---|---|
| 0x0c | 0x1000a100 | auth challenge (§4) |
| 0x1d | inline | "The server didn't respond. Retrying.." → state 1 |
| 0x1f | inline | "The server is full. Retrying..." → state 1 |
| 0x20 | 0x10008de0 | "Server closed the connection." |
| 0x21 | 0x1000acf0 | "Lost connection to the server. Reconnecting.." → cleanup, state 1 |
| 0x22 | 0x1000ad80 | accepted → send join RPC (below) |
| 0x23 | inline | "Failed to initialize encryption." |
| 0x24 | inline | "You are banned from this server." |
| 0x25 | inline | "Wrong server password." + Disconnect |
| 0xc8 | 0x1000aa10 | game sync packet (contents not analysed) |
| 0xcb | 0x10009c40 | 〃 |
| 0xce | 0x10009b40 | 〃 |
| 0xcf | 0x1000a740 | 〃 |
| 0xd0 | 0x10009f10 | 〃 |
| 0xd1 | 0x10009a40 | 〃 |
| 0xd2 | 0x10009e20 | 〃 |
| 0xd3 | 0x10009d30 | 〃 |

Rejection reasons arrive as **RPC 130** (handler 0x10010540), not as packets: payload `u8 reason`: 1 "Incorrect Version", 2 "Unacceptable NickName" (+ hints: 3–20 chars, a-z A-Z 0-9), 3 "Bad mod version", 4 "Unable to allocate a player slot"; any value → client disconnects after 500 ms.

### Join request (built in 0x1000ad80 on receipt of 0x22)

Reads from the 0x22 packet: u8 id, u32 ip, u16 port, u16 player index, **u32 challenge**. Computes `resp = challenge XOR 0x0FD9` (4057). Logs "Connected. Joining the game...", sets netgame state 6, then sends **RPC 25** (ID cell at A 0x100e5b88 = 25) with priority 1, reliability 8 (RELIABLE), channel 0, no timestamp. Payload, in order (byte-aligned only by coincidence; written with the generic bit writer):

| # | field | encoding | value |
|---|---|---|---|
| 1 | netgame version | u32 | **4057** (0xFD9), immediate in code |
| 2 | mod flag | u8 | **1** |
| 3 | nickname length | u8 | strlen(nick) |
| 4 | nickname | bytes | local player name (no NUL) |
| 5 | challenge response | u32 | challenge XOR 4057 |
| 6 | client serial length | u8 | strlen(serial) |
| 7 | client serial ("gpci") | bytes | see below |
| 8 | version string length | u8 | **8** |
| 9 | version string | bytes | `0.3.7-R4` |
| 10 | challenge response (again) | u32 | same value as #5 |

Field #10 was confirmed at assembly level (A 0x1000afc3–0x1000afd8 reload the same stack slot that #5 used).

**Client serial** (0x1005ffa0, called with a pointer to game-process memory 0x00c9236c and the constant 1001): SHA-1 over the NUL-terminated string found at that game address; each of the 20 digest bytes is rewritten by ordering its two 2-bit pairs within each nibble (for each nibble: the larger 2-bit half goes high); the 20 bytes are rendered as 40 uppercase hex chars, parsed as a big integer in base 16, multiplied by 1001 (1000 if the netgame object does not exist yet), and printed back in base 16 (output letter case not verified). **Verified** (NET-6 follow-up, against the original's RPC 25 in `target/netdiff/orig-20261105-064601`): the string at 0x00c9236c is GTA's user-files path from its fifth character on (`sers\<user>\Documents\GTA San Andreas User Files` under Wine), hashed without its NUL; the 20 digest bytes are the five SHA-1 state words in memory order (little-endian each); output upper case. Ours: `samp_proto::session::samp_serial`.

### Server reply: InitGame = RPC 139 (handler 0x10010660)

Fields read in this exact order (bit = 1-bit bool; u32 values that are floats are read as raw 32 bits). The "public meaning" column is a hypothesis from public knowledge and is **UNVERIFIED**; the sequence of widths is observed and is consistent with it.

| # | width | stored at (netgame settings struct off.) | public meaning (UNVERIFIED) |
|---|---|---|---|
| 1 | bit | +0x21 | zone names |
| 2 | bit | +0x00 | CJ walk |
| 3 | bit | +0x15 | allow weapons |
| 4 | bit | +0x39 | limit chat radius |
| 5 | u32 | +0x34 | chat radius (float) |
| 6 | bit | local → later applied | stunt bonus |
| 7 | u32 | +0x27 | name-tag distance (float; default 60.0) |
| 8 | bit | +0x1a | disable enter/exits |
| 9 | bit | +0x2f (then forced to 1 by the client) | name-tag LOS |
| 10 | bit | +0x2b | manual vehicle engine/lights |
| 11 | u32 | +0x23 | spawns available |
| 12 | u16 | local player id (observed: stored as local player's ID) | player id |
| 13 | bit | +0x38 | show name tags |
| 14 | u32 | +0x30 | player-marker mode |
| 15 | u8 | +0x2c | world time hour (default 12) |
| 16 | u8 | +0x2e | weather (default 1) |
| 17 | u32 | +0x16 | gravity (float; default 0.008) |
| 18 | bit | local (sets netgame flag +0x239) | LAN mode |
| 19 | u32 | +0x01 | death-drop money |
| 20 | bit | +0x20 | instagib |
| 21–24 | 4 × u32 | globals A 0x100fe0a8/ac/b0/b4 | on-foot / in-car / weapon send rates, multiplier |
| 25 | u32 | selector: 1→mode 0, 2→mode 1, other→mode 2 (and skips an init call) | lag compensation |
| 26 | u8 L + L bytes | hostname buffer (netgame +0x131) — **observed**: printed as "Connected to {B9C9BF}%.64s" | server name |
| 27 | 212 bytes raw | passed to a table loader (0x100a1650) | per-vehicle-model (400–611) data — 212 = model count (inference) |
| 28 | u32 | +0x1b | vehicle friendly fire |

After reading: netgame state = **5** (connected/in game). Defaults in parentheses come from the netgame constructor's initial values.

### Netgame state values (field +0x3cd) observed

1 waiting to connect (3 s retry) · 2 connecting · 6 join sent, awaiting InitGame · 5 connected (after InitGame). 0x0c is processed only in states 1–6.

---------------------------------------------------------------------------------------------------

## 6. RPC mechanism — status: ANSWERED (table in `rpc-ids.md`)

### Observed

* Send (RakPeer RPC, A 0x1003ecb0): optional `0x28` + u32 RakNet time when the shift-timestamp flag is set; `u8 0x14`; **RPC ID = 1 byte**; payload bit length as compressed u32; payload bits copied **unaligned** (bit writer with "right-aligned = false"). If the payload is empty, a second compressed 0 is written (observed quirk). The resulting message is queued with the caller's priority/reliability/channel.
* Receive: RakPeer's packet getter (0x10040fd0) intercepts ID 0x14 (or 0x28 … 0x14) and calls 0x1003ade0, which skips 8 (or 40) bits, reads the 1-byte ID, looks it up in the registration map, reads the compressed u32 bit count, reads that many bits into a buffer (stack up to 1 MiB) and calls the handler with {buffer, bit count, sender}. Unknown IDs/short data are dropped.
* Registration: RakClient vtable +0x58 (ID pointer, handler). 141 IDs registered (two registration functions, 46 + 95). 27 distinct IDs are sent by the client from 34 call sites (all priority 1).

### Unknown

Payload layouts of RPCs other than 25, 130, 139 were not analysed.

---------------------------------------------------------------------------------------------------

## Contradictions / confirmations vs public hypotheses

* Netgame version 4057 — **confirmed** (0xFD9 immediate, also used as challenge XOR key).
* Cookie handshake — **confirmed client-side** (0x18 + u16 `0x6969^cookie`, cookie from 0x1a). The client's own server-role code does *not* validate the cookie.
* Port-keyed substitution encryption — **confirmed for client→server only**; note only the *low byte* of `port^0xCCCC` matters and even-indexed bytes are not XORed. No decryption on the client receive path (server→client datagrams are plain).
* Shifted packet IDs / reliability enum — consistent (observed IDs listed above; reliability 6–10 on the wire).
* ClientJoin RPC = 25 — **confirmed**, with an extra trailing u32 (duplicate challenge response) after the version string; if a public definition omits it, that is a contradiction.
* Auth key exchange — framing confirmed (ID 0x0c, u8-length strings, 40 hex chars); response derivation mixes in anti-tamper data, so "SHA-1 of challenge" alone (if that is the public claim) is **not** what this client sends.
