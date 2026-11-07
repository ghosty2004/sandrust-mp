# samp.dll addresses behind `crates/samp-client/src/mainhooks.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `correct` | 0x100A2440 | 0x100A2410 | HUD scale correction (aspect ratio) | docs/re/subsystems/ui-rendering.md, "Other hooks that matter for UI" |
| `radar_site` | 0x100A2540 | 0x100A2510 | radar draw wrapper around `CHud::DrawRadar` | docs/re/subsystems/ui-rendering.md, "Other hooks that matter for UI" |
| `crosshairs` | 0x100A2590 | 0x100A2560 | crosshair draw wrapper (camera mode 0x35 only) | docs/re/subsystems/ui-rendering.md, "Other hooks that matter for UI" |
| `ENABLED` | 0x10117470 |  | HUD scale fix flag (initially 1) | docs/re/subsystems/remote-control-combat.md, "M7 implementation notes" |
| `set_enabled` (start-up) | 0x100C49DD |  | start-up clears the flag when sa-mp.cfg `nohudscalefix` is exactly 1 | docs/re/subsystems/remote-control-combat.md, "M7 implementation notes" |
| `install` (`unprotect`) | 0x100AAEF0 |  | makes the `.rdata` float ranges writable once at start-up | docs/re/subsystems/gta-patches.md, "Full table" |
| `idle_wait_time` | 0x100A2420 | 0x100A23F0 | Idle busy-wait hook: `Sleep(1)`, then `CTimer::GetCurrentTimeInCycles` | docs/re/subsystems/gta-patches.md, "Full table" |
| `cheat_stub` | 0x100A3AA0 | 0x100A3A70 | `CCheat::AddToCheatString` hook: keep the flag write, skip the cheat | docs/re/subsystems/gta-patches.md, "Full table" |
