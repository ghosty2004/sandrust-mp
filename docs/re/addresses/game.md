# samp.dll addresses behind `crates/samp-client/src/game.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `version_poll`, `VERSION_GIVE_UP_FLAG` |  | 0x100AA720 | samp's GTA version check | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `VERSION_POLL_MS`, `VERSION_POLLS` |  | 0x100AA7BE | polling loop of the version check (10 ms × 6000) | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `VERSION_ERROR_TEXT` |  | 0x100A08EA | "Version Error" message box, then `ExitProcess(1)` | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `SCANNER_DUMMY_PED_BYTES` |  | 0x100A17BB | zeroed stand-in ped (0x1E8 dwords) for the police scanner hook | docs/re/subsystems/gta-patches.md, "Full table" |
| `VTBL_TASK_HANDS_UP` | 0x100ACBA0 |  | samp's "hands up" primary-task test | docs/re/subsystems/player-state.md |
| `INTELLIGENCE_SET_TASK_DUCK_SECONDARY` | 0x100AC700 |  | samp's duck helper that calls `SetTaskDuckSecondary` | docs/re/subsystems/player-state.md |
| `INTELLIGENCE_SECONDARY_IK`, `TASK_IK_DELETING_DTOR` | 0x100AE060 |  | samp removes the IK chain task before a skin change | docs/re/subsystems/remote-control-onfoot.md, "Skin / model change" |
| `is_ped_model` | 0x100B3E10 |  | samp's valid-ped-model test | docs/re/subsystems/player-state.md |
| `read_local_sample` (special action) | 0x10003410 |  | samp's special-action order (jetpack, dance, hands up, phone, piss, duck, …) | docs/re/subsystems/player-state.md |
