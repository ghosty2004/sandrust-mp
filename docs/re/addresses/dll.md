# samp.dll addresses behind `crates/samp-client/src/dll.rs`

A = 2021 build, B = 2019 build (docs/re/build-diff-r4.md). An empty column means the address was only
recorded for the other build. Code comments cite at most one of these per item.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `MODULE_BASE`, `init_thread` (`ui::preload`) | 0x100C5100 |  | DLL attach: keeps its module handle, registers fonts, starts the init thread | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `version_error_thread` |  | 0x100A08C0 | start-up thread that runs the version check | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `version_error_thread` |  | 0x100AA720 | version check: poll the probe, message box, `ExitProcess(1)` | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `init_thread` (sector grid first) | 0x100AAAC0 |  | sector relocation, first routine of samp's main block | docs/re/subsystems/gta-patches.md, "Sector array relocation" |
| `init_thread` (`CGame::Shutdown` hook) | 0x100A6904 |  | writes `jmp dword [slot]` + NOP at `CGame::Shutdown` | docs/re/subsystems/gta-patches.md, "Full table" |
| `game_shutdown_hook` |  | 0x100A3AB0 | `CGame::Shutdown` replacement: cleanup, up to 2000 ms of 100 ms sleeps, `ExitProcess(0)` | docs/re/subsystems/gta-patches.md, "Full table" |
| `camera_process_hook` |  | 0x100A2640 | `CCamera::Process` call hook in `CGame::Process` (camera follow first) | docs/re/subsystems/gta-patches.md, "Full table" |
| `game_process_hook` | 0x100A26A0 | 0x100A2670 | `CGame::Process` call hook in `Idle` (SEH scope "s007"; also updates the chat render target) | docs/re/subsystems/ui-rendering.md, "Other hooks that matter for UI" |
| `FPS_LIMIT` | 0x1009FFE0 |  | Game constructor: frame limit +0x5d = 90 | docs/phase3/parity-notes.md |
| `frame_timer_step` | 0x100A1C50 |  | samp's step before `CGame::Process` (see frametimer) | docs/phase3/parity-notes.md, "Speed magnitude" |
| `frame_timer_step` (`timeBeginPeriod(1)`), `client_tick` (netgame on second call) | 0x100C47D0 |  | first-frame init of the per-frame hook; creates the netgame | docs/re/subsystems/gta-patches.md, "When patches are applied" |
| `limiter_task_gate` | 0x100ACD10 |  | local ped on foot with task type 0x10C in intelligence +0xC | docs/phase3/parity-notes.md, "Speed magnitude" |
| `limiter_task_gate` (vtable range) | 0x100B40E0 |  | samp's vtable sanity range check | docs/phase3/parity-notes.md, "Speed magnitude" |
| `first_frame_ped_flags` | 0x10001010 |  | local-ped object created on demand | docs/phase3/close-notes.md |
| `first_frame_ped_flags` | 0x100ABAB0 |  | local-ped object constructor that sets the two ped flags | docs/phase3/close-notes.md |
| `frame_hook`, `quit_tick` | 0x100C5030 | 0x100C5010 | per-frame entry from the `Render2dStuff` call retarget | docs/re/subsystems/ui-rendering.md, "Per-frame game entry" |
| `frame_hook` (pre-step) | 0x100C4FB0 |  | pre-step called from the per-frame entry | docs/re/subsystems/remote-control-onfoot.md, "Per-frame order for a remote on-foot player" |
| `tick` (audio) |  | 0x100C4FF0 | per-frame step after the netgame tick: radio calls, mission-audio poll | docs/re/subsystems/ui-chat-audio.md |
| `request_quit` | 0x100C3EC0 |  | `/quit`: Disconnect(500) in state 5, quit flag + tick count | docs/phase3/m10-notes.md |
