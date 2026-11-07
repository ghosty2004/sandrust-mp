# pedanims.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `keep_before_delete` (runs inside our skin change) | 0x100AFF90 → 0x1009EFB0 | — | ped skin change (RPC 153): ped-wrapper method → `DeleteRwObject` + `SetModelIndex`. The old code comment labelled this chain "B"; the spec lists the same numbers as A | remote-control-onfoot.md §1.4; player-state.md "153 SetPlayerSkin" |

GTA (not samp.dll) addresses from the old module header, kept for reference: hands-up task vtable 0x85A29C; the
crash site in `RpMaterialDestroy` 0x74DA25 (full story in docs/phase3/m5-notes.md).
