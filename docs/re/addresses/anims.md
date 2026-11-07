# anims.rs and animtable.rs

Read in build B; the animation table is at the same address in both builds.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `animtable::NAMES` | 0x101039E8 – 0x101138B4 | 0x101039E8 (index 1) – 0x101138B4 (index 1812) | 36-byte `"LIB:NAME"` records (build B fv0.3.7.3, 2019-11-21) | remote-control-onfoot.md §4.1 |
| `record`, `animtable::COUNT` | — | 0x1009CF00 | table accessor, indexes 1..=0x714 | remote-control-onfoot.md §4.1 |
| `name_part` | — | 0x1009CF30 | start-up hashing of the NAME parts | remote-control-onfoot.md §4.1 |
| `index_of` | — | 0x1009CE20 | `"%s:%s"` string search | remote-control-onfoot.md §4.1 |
| `index_of_hash` | — | 0x1009D000 | hash search | remote-control-onfoot.md §4.1 |
| `ftol` | — | 0x100C7464 | MSVC `_ftol2` | — |
| `encode_task` | — | 0x100AE600 | script animation encoder (task flags +0x5C, time +0x40) | remote-control-onfoot.md §4.1 |
| `encode_plain` | — | 0x100AE6B0 | plain-animation encoder | remote-control-onfoot.md §4.1 |
| `decode`, `Decoded` | — | 0x100AE7E0 | remote decoder / player | remote-control-onfoot.md §4.1 |
| `VEND_USE_VALUE` | — | 0x1001680A | VEND_USE → VEND_EAT_P special case | remote-control-onfoot.md §4.1 |
| `Latch` | LocalPlayer +0xFC / +0x100 | — | value and "sent" flag | remote-players-sync.md "207 PlayerFootSync" |
| `Latch::update` | — | 0x100077F5 (encoder call at 0x100077EF) | per-frame latch update in the local tick | remote-control-onfoot.md §4.1 |
| `Latch::take` | — | 0x10005131 | read by the 207 builder | remote-players-sync.md "207 PlayerFootSync" |
| `RemoteState` | remote +0x105 / +0x1B4 (last) / +0x1C1 (by RPC) | — | remote animation state | remote-control-onfoot.md §4.1 |
| `RemoteState::step` | — | 0x100167E4 → 0x100AE7E0 | one-shot apply of a stored 207 | remote-control-onfoot.md §4.1 |
| `refused_library` | — | 0x100AD270 | case-insensitive "SEX" compare | player-state.md "86 ApplyPlayerAnimation" |
