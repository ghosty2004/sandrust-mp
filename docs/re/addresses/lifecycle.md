# samp.dll addresses behind `crates/samp-client/src/lifecycle.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| (module: per-frame main loop) |  | 0x1000B5A0 | netgame main loop: packet handler, "not connected" scene, connect tick | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `CONNECT_INTERVAL_MS`, `NetGame::connect_due` |  | 0x10008930 | connect tick (`cmp eax, 3000; jbe` at B 0x10008942) | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::attempt_started` | 0x10008985 |  | tick count read again after RakClient::Connect returns | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::new` |  | 0x1000B920 | netgame construction (state 1, "Started") | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::on_packet` |  | 0x1000B250 | RakNet packet handler | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::on_packet` (`ServerClosed`) |  | 0x10008DD0 | server closed the connection: no state change, no reset | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::on_packet` (`ConnectionLost`) |  | 0x1000ACE0 | lost connection: disconnect, reset, delete players, back to state 1 | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `Action::Reset` |  | 0x1000A530 | RPC 40 reset routine | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `Action::DeletePlayers` |  | 0x1000AD20 | loop deleting remote players 0..1004 | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `NetGame::on_rejected` |  | 0x10010520 | RPC 130 ConnectionRejected handler | docs/phase3/trace-notes.md, "Connection lifecycle" |
| `Action::Debug` |  | 0x100680B0 | chat helper, type 8 line | docs/re/subsystems/ui-chat-audio.md |
| `Action::Info` |  | 0x10068030 | chat helper, type 4 line | docs/re/subsystems/ui-chat-audio.md |
