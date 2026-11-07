# pedhooks.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `anim_group_stub` | 0x100A3F60 | — | stub called from `ProcessAnimGroups` (GTA 0x609A4E): stores the move-anim group only when InitGame "CJ walk" is set; patch-table rows 43 (call) and 187 (NOP of the store) | remote-control-onfoot.md §4.4; gta-patches.md "Player / ped control" |
