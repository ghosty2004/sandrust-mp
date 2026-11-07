# samp.dll addresses behind `crates/samp-client/src/frametimer.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `FrameTimer::step` | 0x100A1C50 |  | samp's step before `CGame::Process` (time step, pad counter, frame limiter) | docs/phase3/parity-notes.md, "Speed magnitude" |
| `FrameTimer::step` (caller) | 0x100A26A0 |  | samp's `CGame::Process` wrapper that calls the step | docs/re/subsystems/gta-patches.md, "Hooks overview" |
