# samp-proto: samp.dll addresses

Addresses behind `crates/samp-proto` (except `rpcs/`). A = 2021 build, B = 2019 build ("—" = not
recorded; see docs/re/build-diff-r4.md for the offset pattern). The code cites at most one per item.

## RakNet reliability layer (`connection.rs`)

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Connection::poll` | 0x10049ae0 | 0x10049a90 | reliability-layer update (also holds the bandwidth bucket we do not reproduce: floor 144400 bit/s, 1-second burst) | handshake.md §3; phase3/net6-notes.md |
| `Connection::poll` (cadence) | 0x10008988 | — | RakClient::Connect called with threadSleepTimer 2 (update thread every 2 ms) | handshake.md §1; net6-notes.md |
| `Connection::poll` (cycle) | 0x10041450 | — | RakNet update cycle: receive, buffered sends, keep-alive, update, dead test, ping, internal handling | net6-notes.md |
| `Connection::generate`, `DATAGRAM_BITS`, `ACK_BITS` | 0x10048070 | 0x10048020 | datagram build: acks, due resends, new messages by priority; (MTU − 28) × 8 message bits, MTU × 8 − 0xE1 ack bits | handshake.md §3 |
| `header_estimate` | 0x10043ec0 | — | per-message header size estimate used by the fit test | net6-notes.md |
| `MTU` | 0x1003de50 | — | default MTU 576 initialiser (SetMTUSize 0x10038860 is never called) | handshake.md §3 "MTU / splitting" |
| `Connection::send_with` (split) | 0x10045800 | 0x100457b0 | split into pieces of MTU − 28 − ⌈header estimate / 8⌉ bytes | handshake.md §3 |
| `Connection::send_with` (split) | 0x10046b00 | — | send entry that decides whether to split | handshake.md §3 |
| `Connection::send_with` (empty) | 0x10046b2e | — | empty sends are dropped | handshake.md §3 |
| `Connection::resend_timeout` | 0x10044070 | — | resend timeout = 3 × ping, no backoff | net6-notes.md |
| `MIN_PING_MS`, `OVER_TIMEOUT_PING_MS` | 0x10045c00 | — | SetPing: ping ≥ 30 ms, 500 ms when above the timeout | net6-notes.md |
| `INITIAL_PING_MS` | 0x10047780 | — | reliability-layer reset: SetPing(1000) | net6-notes.md |
| `TIMEOUT` | 0x10048e10 | 0x10048dc0 | reliability-layer init: default timeout 10000 ms | handshake.md §3 |
| `KEEP_ALIVE_AFTER_MS` | 0x10041853 | — | keep-alive test (`5000 <`), statistics block 0x100479d0 +0x9c | handshake.md §3; m10-notes.md |
| `KEEP_ALIVE_AFTER_MS` (clock) | 0x1003cc13 | — | RakNet ms clock (µs time / 1000) | net6-notes.md "Keep-alive clock" |
| `KEEP_ALIVE_AFTER_MS` (clock) | 0x10041660 | — | RakNet ms clock use in the cycle | net6-notes.md "Keep-alive clock" |
| `Connection::poll` (keep-alive send) | 0x1003cbf9 | — | the `0x17` keep-alive send | m10-notes.md |
| `PING_INTERVAL` | 0x10041a2c | — | update cycle's next ping at `+ 5000` while no pong was accepted | net6-notes.md |
| `Connection::ping_sample` | 0x10041ea0 | — | pong sample acceptance | net6-notes.md |
| `MAX_PING_SAMPLE_MS` | 0x10041ee0 | — | samples ≥ 1200 ms ignored unless the ring slot is empty | net6-notes.md |
| `PING_RING` | 0x10041f40 | — | 5-entry ping ring | net6-notes.md |
| `Connection::accept_number`, `MAX_AHEAD`, `recv_base` | 0x10049250 | 0x10049200 | datagram parse with the received-number duplicate filter | handshake.md §3 "Received message numbers" |
| `Connection::is_dead` | 0x10049b8b | — | dead-connection test (ack timer older than the timeout) | m10-notes.md |
| `Connection::is_dead` (ack timer) | 0x10045b70 | — | ack timer start on first reliable send | m10-notes.md |
| `Connection::handle_datagram` (ack timer) | 0x10049390 | — | every acked number moves the ack timer | m10-notes.md |

## RakNet messages (`raknet.rs`)

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `connection_request` | 0x10034220 | 0x100341d0 | RakClient::SetPassword: a non-empty password is stored with its terminator, an empty one as nothing | handshake.md §1 "Command line and server password" |
| `MAX_PASSWORD_BYTES` | 0x10040581 | 0x10040531 | RakPeer::Connect cuts the password data to 255 bytes | handshake.md §1 "Command line and server password" |

## Session, strings, sync

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `session::samp_serial` | 0x1005ffa0 | 0x1005ff50 | client serial (gpci); input is the game string at gta_sa.exe 0x00C9236C (user-files path from its fifth character) | handshake.md §5 "Client serial" |
| `strcomp::FREQUENCIES_LOW` | 0x10101de8 | — | 256-entry Huffman frequency table | ui-chat-audio.md "Shared: compressed string" |
| `strcomp::Tree::build` | 0x10026ea0 | — | node insertion routine (advances while the listed weight is strictly less) | ui-chat-audio.md "Shared: compressed string" |
| `strcomp::decode` | 0x100542d0 | — | compressed-string decoder | ui-chat-audio.md "Shared: compressed string" |
| `sync::policy` | 0x100074c0 | — | local player tick (send-rate machinery) | remote-players-sync.md §0.4 |
| `sync::encoding::QUAT_COMPONENT_SCALE` | 0x10009680 | — | quaternion reader constant 1/65535 | remote-players-sync.md §0.2 |
| `sync::encoding::VEC_COMPONENT_SCALE` | 0x1000a3b0 | — | compressed-vector reader constant 1/32767.5 | remote-players-sync.md §0.2 |
| `sync::rcon::RconCommand` | 0x10069030 | — | `/rcon` send | remote-players-sync.md §2 "201" |
| `sync::vehicle::VehicleSync` | 0x10007080 | — | 200 driver sync send | remote-players-sync.md §2 "200" |
| `sync::vehicle::RemoteVehicleSync` | 0x1000aa10 | 0x1000aa00 | 200 receive handler | remote-players-sync.md §2 "200" |
| `sync::vehicle::PassengerSync` | 0x10005590 | — | 211 send | remote-players-sync.md §2 "211" |
| `sync::vehicle::RemotePassengerSync` | 0x10009d30 | 0x10009d20 | 211 receive handler | remote-players-sync.md §2 "211" |
| `sync::vehicle::UnoccupiedSync` | 0x10004d30 | — | 209 send | remote-players-sync.md §2 "209" |
| `sync::vehicle::RemoteUnoccupiedSync` | 0x10009a40 | 0x10009a30 | 209 receive handler | remote-players-sync.md §2 "209" |
| `sync::vehicle::TrailerSync` | 0x100053d0 | — | 210 send | remote-players-sync.md §2 "210" |
| `sync::vehicle::RemoteTrailerSync` | 0x10009e20 | 0x10009e10 | 210 receive handler | remote-players-sync.md §2 "210" |
| `sync::player::RemoteFootSync` | 0x1000a740 | 0x1000a730 | 207 receive handler | remote-players-sync.md §2 "207" |
| `sync::player::RemoteAimSync` | 0x10009c40 | 0x10009c30 | 203 receive handler | remote-players-sync.md §2 "203" |
| `sync::player::RemoteBulletSync` | 0x10009b40 | 0x10009b30 | 206 receive handler | remote-players-sync.md §2 "206" |
| `sync::player::WeaponsUpdate` | 0x10006290 | — | 204 send | remote-players-sync.md §2 "204" |
| `sync::player::StatsSync` | 0x10005d00 | — | 205 send | remote-players-sync.md §2 "205" |
| `sync::player::MarkersSync` | 0x10009f10 | 0x10009f00 | 208 receive handler | remote-players-sync.md §2 "208" |
