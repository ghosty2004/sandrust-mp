# samp.dll addresses behind `crates/samp-client/src/script.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Runner::run` | 0x100B2350 | 0x100B2320 | samp's ScriptCommand: runs one opcode on a private zeroed thread inside its SEH guard | docs/re/subsystems/player-state.md, "ScriptCommand" |
| `LAST_OPCODE` |  | 0x1026B3D4 | ScriptCommand's last-opcode global ("SCM Op" in the crash report) | docs/re/subsystems/player-state.md, "ScriptCommand" |
| `IN_COMMAND` |  | 0x1026B3D8 | ScriptCommand's running flag ("lDbg" in the crash report) | docs/re/subsystems/player-state.md, "ScriptCommand" |
| `op::RPC150_PLAYER_COMMAND` | 0x100E6234 |  | descriptor of the RPC 150 command (0x03FD, "ii") | docs/re/subsystems/player-state.md, "ScriptCommand" |
| `op::ADD_BLIP_FOR_CHAR` … (player markers) |  | 0x100AF740, 0x100A0DA0 | samp functions that use the marker opcodes | docs/re/subsystems/player-state.md |
| `op::CREATE_OBJECT` … (held items, piss, parachute) |  | 0x100B02E0, 0x100AE110, 0x100AD4E0 | samp functions for held items, piss and the remote parachute | docs/phase3/featb-notes.md |
