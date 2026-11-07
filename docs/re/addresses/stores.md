# samp.dll addresses behind `crates/samp-client/src/stores.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `ATOMIC`, `new_entries` | 0x100AAA50 | 0x100AAA20 | initialises the 20000-entry atomic array (zero + vtable) and points the references at it | docs/re/subsystems/gta-patches.md, "Model-info stores" |
| `PED`, `new_entries` | 0x100AAA00 | 0x100AA9D0 | initialises the 319-entry ped array (zero + vtable) and points the references at it | docs/re/subsystems/gta-patches.md, "Model-info stores" |
| `ATOMIC.refs` |  | 0x10116720 | `.data` list of the atomic-store reference sites | docs/re/subsystems/gta-patches.md, "Model-info stores" |
| `ATOMIC` (buffer) | 0x101CEA50 |  | samp's atomic entry array | docs/re/subsystems/gta-patches.md, "Model-info stores" |
| `PED` (buffer) | 0x101C9590 |  | samp's ped entry array | docs/re/subsystems/gta-patches.md, "Model-info stores" |
