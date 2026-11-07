# samp.dll addresses behind `crates/samp-client/src/scmops.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `op_09c6` | 0x10002090 | 0x10002090 | handler of opcode 0x09C6 (jump-table target) | docs/re/subsystems/gta-patches.md, "Other" |
| `op_09c6` (RPC 96) |  | 0x10001CE0 | sends the SCM-event RPC 96 for types 1–4 | docs/phase3/feata-notes.md |
| `op_09d3_result` |  | 0x10001C30 | handler of opcode 0x09D3 (condition) | docs/phase3/feata-notes.md |
| `op_09d3_result` (helpers) |  | 0x100ABE80, 0x100ABFD0 | "in a vehicle, not as its driver or in a carriage" checks it calls | docs/phase3/feata-notes.md |
