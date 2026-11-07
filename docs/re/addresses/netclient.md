# samp.dll addresses behind `crates/samp-client/src/netclient.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `OPEN_RETRY_MS` | 0x1004165D |  | open-connection timer: skip while `next >= now` | docs/phase3/reconnect-notes.md, "What the original does" |
| `OPEN_RETRY_MS` | 0x1004169C |  | open-connection timer: next = now + 1000 after a request | docs/phase3/reconnect-notes.md, "What the original does" |
| `OPEN_RETRY_MS` | 0x1003E6A0 |  | pending record starts with next = Connect time | docs/phase3/reconnect-notes.md, "What the original does" |
| `OPEN_REQUESTS` | 0x10041739 … 0x1004175A |  | after the 6th request the same test fires the failure `0x1D` | docs/phase3/reconnect-notes.md, "What the original does" |
| `RAKNET_STOP` | 0x10040700 |  | RakPeer::Disconnect polls the update thread's exit with `Sleep(15)` | docs/phase3/m10-notes.md |
| `RAKNET_START` | 0x10042934 |  | RakPeer::Initialize polls the new thread's start with `Sleep(10)` | docs/phase3/m10-notes.md |
| `NetClient` (update thread) | 0x10042700 |  | RakPeer's update thread | docs/phase3/net6-notes.md |
| `UPDATE_SLEEP` | 0x10008988 |  | connect tick pushes `threadSleepTimer` 2 | docs/phase3/net6-notes.md |
| `COOKIE` | 0x10118E20 |  | process-global open-connection cookie (`.data`, initially 0) | docs/phase3/reconnect-notes.md, "What the original does" |
| `COOKIE` (writer) | 0x1003FDC2 | 0x1003FD72 | the `0x1a` cookie-reply handler stores the cookie | docs/phase3/reconnect-notes.md, "What the original does" |
| `COOKIE` (reader) | 0x100416A5 | 0x10041655 | the open-connection request reads the cookie | docs/phase3/reconnect-notes.md, "What the original does" |
| `NetClient::connect` | 0x10034130 |  | RakClient::Connect | docs/phase3/reconnect-notes.md, "What the original does" |
| `NetClient::connect` (caller) | 0x10008940 | 0x10008930 | samp's connect tick | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetClient::tick`, `Event::Packet` |  | 0x1000B250 | samp's netgame packet handler | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetClient::disconnect` | 0x100C3EC0 |  | `/quit`: RakClient `Disconnect(500, 0)` | docs/phase3/m10-notes.md |
| `NetClient::disconnect` (notification) | 0x1003E8F0 |  | RakNet sends the disconnection notification with reliability 6 (UNRELIABLE) | docs/phase3/m10-notes.md |
| `Core::update` | 0x10041450 |  | RakPeer update cycle | docs/phase3/net6-notes.md |
| `Core::on_message` (auth key) | 0x10041C16 |  | RakNet answers the auth key itself while connecting | docs/re/handshake.md, "Post-connect auth key" |
| `Core::send_player_connect` | 0x1000AD80 |  | packet handler on `0x22`: PlayerConnect (RPC 25) | docs/re/handshake.md, "Join" |
| `SERIAL_INPUT`, `client_serial` | 0x1005FFA0 | 0x1005FF50 | client serial (gpci), called with game address 0x00C9236C | docs/re/handshake.md, "Client serial" |
