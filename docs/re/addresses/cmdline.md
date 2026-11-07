# samp.dll addresses behind `crates/samp-client/src/cmdline.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `parse` | 0x100C45F0 | 0x100C45D0 | command-line scan (`GetCommandLineA`, switch table for `c d h l n p z`) | docs/re/handshake.md, "Command line and server password" |
| `parse` (buffers) | 0x1026DE60 … 0x1026E270 |  | flags and 0x101-byte value buffers: password 0x1026DE6C, host 0x1026DF6D, port 0x1026E06E, name 0x1026E16F | docs/re/handshake.md, "Command line and server password" |
| `word` | 0x100C3F00 |  | value copy: skip spaces, stop at space, `-` or `/` (no length limit) | docs/re/handshake.md, "Command line and server password" |
| `ConnectArgs::password` (use) | 0x100C47D0 |  | game-init routine constructs the netgame with host, port, name and the password buffer | docs/re/handshake.md, "Command line and server password" |
