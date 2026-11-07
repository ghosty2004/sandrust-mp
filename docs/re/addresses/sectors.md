# samp.dll addresses behind `crates/samp-client/src/sectors.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `relocate` | 0x100AAAC0 |  | zeroes samp's buffer and redirects the grid references from its `.data` lists | docs/re/subsystems/gta-patches.md, "Sector array relocation" |
| `BUFFER_BYTES` (buffer) | 0x101A2490 |  | samp's 160000-byte sector buffer | docs/re/subsystems/gta-patches.md, "Sector array relocation" |
