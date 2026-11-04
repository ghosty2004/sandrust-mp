# Roadmap / task board

Single source of truth for what is done, in progress and next. The end-to-end
plan, definition of done ("1:1") and per-milestone acceptance live in `docs/PLAN.md`. Updated at the
end of every work step (see `docs/WORKLOG.md` for the history).

Status: `done` · `in-progress` · `todo` · `blocked` (needs the user) · `deferred`.
Verification: how completion was (or will be) proven — tests, live server,
differential capture against the original client, e2e script.

## Lanes now

| Slot | Running | Next |
|---|---|---|
| samp-client (lane main) | SAA-2 → SP-OFF (same agent) | M2-P/M3-P → UX-1 |
| samp-client worktree (lane ui) | M4 (chat, input, local commands, game text) | M4-P | UI parity from TRACE-3 batch 2 + M4 deviations: chat render-to-texture cache (2048x1024 RT like samp), per-frame HUD flag order, connect timing (Started → +3.0 s Connecting; retry every ~6.05 s with "didn't respond" line), history keeps duplicates and position across close/send, input keeps text on ESC/F6, password mask U+25CF, ctrl+A / ctrl+word moves, /fontsize recreates GUI fonts immediately, /timestamp layout, IME/clipboard | todo | D3D call-stream diff (tools/trace/d3d_diff.py) of chat/input scenarios = no differences | measured values in trace-notes "UI (TRACE-3)" |
| M8a |
| measurement (lane trace) | TRACE-4 (SP-OFF baseline + vehicles, original) | TRACE-3: UI runtime facts for RE-5 gaps |
| harness (lane diff + diff2) | DIFF-3 cross-client + F5 patch coverage | DIFF-4 visual (after TRACE-3's D3D logger) |
| no-game #1 | BOT-2 (bot drives/shoots) | NET-6 ack/resend timing → rename to sandrust-mp (if confirmed) |
| no-game #2 | M4-core (`crates/samp-ui`: pure UI logic) | — |

## Next up (ordered queue — the scheduler takes from the top)

1. SAA-2 — client serves the 23 files from samp.saa (after M3, needs SAA-1)
2. SP-OFF — single-player systems off like samp — PRIORITY: TRACE-4 saw 10 cops spawn and ARREST the local player in our client (frame 1746+); original: 0 cops (CSV rows: CCarCtrl::GenerateRandomCars NOP, CPopulation::AddPed/AddPedInCar, CEntryExit::GenerateAmbientPeds NOP, police scanner audio hook, CGame::Process NOP, wanted/dispatch-related rows, …); verify with the tracer that no random ped/car/cop is created during a 5-min idle run, original vs ours
3. M2-P — tracer re-diff of spawn after M3 changes; close remaining gaps
4. UX-1 — SA:MP loading screen (needs SAA-2)
5. TRACE-3 — measure UI behaviour at runtime for RE-5's unknowns (trace lane)
6. M4 — chat window, input, local commands, game text (needs RE-5, TRACE-3)
7. DIFF-3 — cross-client harness (needs M3)
8. F5 — `tools/diff/patch-coverage.sh` (CSV vs applied patch table)
9. TRACE-4 + M5 — vehicles (needs M3)
10. M8a — dialogs, textdraws, menus, scoreboard, nametags (needs M4)
11. TRACE-5 + M6 — world entities (needs SAA-2)
12. TRACE-6 + M7 — combat (needs M3, M5)
13. M8b, M9, M10 — remaining behaviour, audio, client features
14. DIFF-4 — visual diff harness (needs UX-1, M4)
15. SOAK — stability (needs M7)

## Phase 0 — Reverse engineering (specs in `docs/re/`)

| ID | Task | Status | Verification | Notes |
|---|---|---|---|---|
| RE-1 | Handshake, cipher, RakNet framing, join, RPC framing (`handshake.md`) | done | live server + differential | `0x29` static data added after diff |
| RE-2 | All 160 RPC + sync packet layouts (`subsystems/*.md`) | done | coverage critic + spot checks | |
| RE-3 | samp.dll → gta_sa.exe memory patches, annotated (384 rows) | done | byte-level vs gta_sa.exe | `gta-patches-r4-annotated.csv` |
| RE-4 | Remote player control: on-foot / vehicles / combat | done | byte-level checks of key hooks | `remote-control-*.md` |
| RE-5 | UI rendering specs | done | `docs/re/subsystems/ui-rendering.md` (~900 lines): Present-wrapper render path, draw order, fonts, colour codes, DXUT-based GUI skinned by sampgui.png, all dialog styles, scoreboard, nametags, death window, keys, sa-mp.cfg keys | runtime unknowns (pixel metrics under Wine/DXVK, skin colour slots, textdraw hit rects…) → TRACE-3 |
| RE-6 | Startup/config (cmdline, sa-mp.cfg, registry, chatlog, screenshots) | todo | — | partly needed for M1/M2 |
| RE-7 | Audio streams (BASS usage, 3D audio) | deferred | — | |
| RE-8 | Other client versions (R1, R3, R5, 0.3.DL) | blocked | — | needs the binaries |

## Phase 1–2 — Network client (`crates/samp-proto`, `samp-bot`, `samp-dump`)

| ID | Task | Status | Verification | Notes |
|---|---|---|---|---|
| NET-1 | Cipher, cookie, RakNet reliability, auth, join, InitGame | done | live + differential | |
| NET-2 | Typed codecs for all RPCs + sync packets, Huffman strings | done | 223 tests, strcomp live | |
| NET-3 | Bot: class selection, spawn, on-foot sync, stats sync | done | probe filterscript + differential | |
| NET-4 | `samp-dump` + `tools/capture.sh` differential pipeline | done | used against original client | |
| NET-5 | Differential: vehicles, passengers, dialogs, commands, death | todo | captures of original client | needs user gameplay or M3+ automation |
| NET-6 | RakNet ack/resend timing parity | todo | `--raw` capture comparison | |
| NET-7 | Unified outgoing (C→S) RPC enum with send params per payload | done | main session: 334 tests pass, clippy 0; capture-verified send params for 25/128/129/52, the other 28 from specs (listed in `docs/phase3/net7-notes.md` for differential confirmation) | RPC 131 has two senders (rel 9 hook / rel 10 poll) |

## Phase 3 — In-game client (Rust DLL in gta_sa.exe under Wine)

| ID | Task | Status | Verification | Notes |
|---|---|---|---|---|
| ENV-1 | Wine prefix, GTA 1.0 US copy, original SA:MP client for diffs | done | original client joins local server | `tools/run-gta.sh`, `run-samp.sh` |
| ENV-2 | Isolated headless display for visual checks + input injection | done | `tools/e2e/m1.sh` exit 0 inside headless gamescope; screenshot shows GTA rendered on the isolated display only | `e2e_headless_run CMD`; games launch without Wine virtual desktop when `E2E_HEADLESS=1`; never `pkill -f` patterns that match the caller's own command line |
| M1 | Launcher + client DLL skeleton, boot into gameplay, control channel | done | `tools/e2e/m1.sh` exit 0 (state 9 in ~13 s, ticks ~60 fps, ped at Grove St, clean quit); 246 tests; clippy 0 (host + i686-windows) | `docs/phase3/m1-notes.md`; deferred rows: 122/123 (→ UX-1), 128, 131; extra CdStreamSync deadlock fix (rescue path untested); open: stock main.scm moves the player after spawn (→ M2) |
| RE-9 | samp.saa archive format | done | spec `docs/re/samp-saa.md`: XTEA (32 rounds, per-block key XOR), 256-slot directory keyed by Jenkins lookup2 of the lower-case basename, RSA-1024/SHA-1 signature; redirection = `CreateFileA` hook by basename | 28 live entries incl. loadscs.txd (SA:MP loading screen), vehicle.txd, tracks2/4.dat, logo.png |
| SAA-1 | `crates/samp-saa` pure-Rust reader | done | main session: signature valid on the user's samp.saa, 28 entries listed; tests incl. real-archive ones pass; captures match (14 byte-for-byte, 12 differ only by tracer reconstruction) | |
| M3-P | Remote-ped parity left by TRACE-4: per-packet speed magnitude (orig |v| ≈ 0.050 vs ours 0.030; suspect samp's timestep rewrite at A 0x100A1C50), apply cadence; SEH containment around swapped ProcessControl | todo | tracer re-diff speed/cadence within tolerance | measure per-frame physics first |
| SAA-2 | Client integration: redirect the same 23 file opens to samp.saa (incl. samp's main.scm/script.img); fallback to own script without SA:MP | todo | trace diff of boot/spawn shows no file/script differences; m1–m3 still pass | after M3 (touches samp-client) |
| UX-1 | SA:MP loading screen: splash names → `title`, remove RenderSplash rect, load SA:MP's `title` image from the user's SA:MP install (fallback: GTA splash) | todo | headless screenshots during boot vs original client; acceptance: no GTA loading slideshow/intro visible at startup (user-reported 2026-11-04) | RE first: where samp.dll gets `title` (samp.saa?) and any overlay text; user asked for it (2026-11-04); after M1 (touches patches.rs) |
| TRACE-1 | Behavioural differential tracer (52 GTA hooks, JSONL, multi-`--dll` launcher, `tools/trace/run.sh` + `diff.py`) | done | original client + ours both run join→spawn→walk traced; spawn order matches first 15 steps | `docs/phase3/trace-notes.md` |
| ENV-3 | Second test lane for tracing: `wine/prefix-trace` + `wine/gta-sa-trace` (hard-linked data, real copies of top-level files) | done | prefix boots | use `WINEPREFIX`, `GTA_DIR`, `E2E_TAG=trace`, `E2E_PORT=7790`, `E2E_CTL_PORT=47701` |
| ENV-4 | Third test lane "diff": `wine/prefix-diff` + `wine/gta-sa-diff` (hard-linked), server 7795, control 47705/47706 | done | prefix boots | for DIFF-2/3/4 harness runs |
| ENV-5 | Fourth lane "ui" (wine/prefix-ui, wine/gta-sa-ui, 7800/47710) + git worktree ~/samp-rs-wt/m4 (branch ui-render) for a parallel samp-client writer | done | prefix boots; worktree created at 0a4acdd | main session merges worktree branches |
| DIFF-1 | Same state queries for the ORIGINAL client: tracer serves the control-channel protocol | done | `state` text identical between our client and tracer in the same run (except ticks) | port 47701/47702 in trace lane |
| DIFF-2 | Per-RPC coverage harness (F1) | done | `tools/diff/rpc-coverage.sh` (~23 min; original ×2 + ours); 131/141 S→C ids fired (10 never sent by open.mp); original determinism 150/176 steps identical, noise learned; baseline ours (pre-M3 build): 15 MATCH / 20 DIFF / 96 NOT-IMPLEMENTED | `docs/phase3/diff-notes.md`; re-run after each milestone |
| DIFF-3 | Cross-client test: original client and ours on one server at the same time; each sees the other correctly | todo | both directions: position/anim/vehicle via state queries | after M3 |
| DIFF-4 | Visual diff: same resolution + identical gta_sa.set in both lanes; screenshots at the same scenario steps compared with tolerance | todo | image diff report | after UX-1 |
| M2 | Network inside the game: join, class select, spawn, local on-foot sync | done | `tools/e2e/m2.sh` exit 0 (main session): Shift → spawn, W held → server-side 16.12 m, st=1; agent: 3 consecutive passes; 289 tests; clippy 0 host + i686 | own minimal main.scm via redirect of the main.scm open; spawn order from TRACE-1; deviations in `docs/phase3/m2-notes.md` (anim field 0, no 203/204, no death handling, no clothes) |
| M2-P | 1:1 parity fixes from the tracer diff (frame-1 ped flags, connecting-screen block, model 596 release, always-SetModelIndex via samp's direct path, RPC 67, time of day, preload polls) | in-progress | agent: m2.sh 3× pass after the changes; pending: tracer re-diff `--from shift --to walk_start` | intentionally not reproduced: 2× REMOVE_BLIP with garbage handles (uninitialised memory in samp); tracks*.dat now covered by D-4=A; + local pistol skill stat must be 1000 after spawn (ours 0, TRACE-4); DIFF-2 idle gaps: weather not forced, weapon skills 0 (orig 1000), 4 stock weapon pickups, 2 vehicle blips missing |
| TRACE-2 | Trace the ORIGINAL client's remote-player handling | done | stream-in, per-frame, stream-out/quit and Players[] guard-page measurements relayed to M3; `docs/phase3/trace-notes.md` "Remote players (TRACE-2)" | corrected Players[] end address in remote-control-onfoot.md (0xB915B8) |
| M3 | Remote players on foot: sector relocation, Players slots, CREATE_PLAYER stream-in, ProcessControl/UseGun swap hooks, 207 apply, destroy | done | main session: `tools/e2e/m3.sh` exit 0 (bot slot 2, tracking 0.2 m, path 8.38 m/5 s, removed on quit) and `tools/e2e/goal.sh` exit 0 (combined scenario); 337 tests; clippy 0 host + i686; agent: 3/3 each; TRACE-4 re-diff: stream-in/swap/timing/destroy match | `docs/phase3/m3-notes.md`; open parity items → M3-P |
| RE-10 | Static gap check for M5/M6/M7 + measurement requests + CSV rows per milestone | done | `docs/phase3/re10-gaps.md`: specified/partial/missing M5 36/12/7→48/6/1, M6 16/12/11→31/6/2, M7 23/9/3→29/5/1; `docs/phase3/measurement-requests.md` (MR-5.1, 6.1, 7.1, 5.2, 6.2, 7.2…); CSV rows to apply M5 83 / M6 61 / M7 40 | rows 61, 370, 375 to justify for F5 |
| BOT-2 | samp-bot drive/passenger/shoot modes | done | main session: `tools/e2e/bot2.sh` exit 0 (driver state + vehicle moved, passenger state, 14/14 plausible shots with damage) | plausible values (enter timing, aim camera/sway, 200 extra) queued for measurement on the original (TRACE-3 Part B) |
| M4-core | `crates/samp-ui`: renderer-independent UI logic for M4/M8a (chat buffer + paging + F7 modes, colour-code parsing incl. alpha rule, text wrapping/limits, ~k~ expansion hook, input box editing/history, local command parser + sa-mp.cfg read/write, dialog models for all 6 styles incl. list/tablist parsing, scoreboard model, death-window model) | done | main session: 420 workspace tests pass, clippy 0; 83 unit tests from RE-5 tables; exact strings from samp.dll's string table; ~39 unknown constants listed in `docs/phase3/m4-core-notes.md` for TRACE-3 (incl. U-LIST-CODES: assumed list rows render colour codes) | no D3D; consumed by samp-client in M4 |
| M4 | Chat window + input, local commands, game text, connect lines, cursor | done (branch ui-render 544cb8b, merge pending) | main session: `tools/e2e/m4.sh` exit 0 in lane ui (chat round trip, colours, /pagesize saved, server command); agent: 3×; 432 tests, clippy 0; 1280x720 screenshot matches TRACE-3 numbers to the pixel; RPC 101/50 bytes identical to the original | deviations → M4-P |
| M5 | Vehicles: stream in/out, driving, passengers, unoccupied, trailers | todo | e2e + differential | |
| M6 | World entities: objects, pickups, actors, checkpoints, 3D labels, gang zones, map icons | todo | e2e | needs SA:MP assets decision |
| M7 | Combat: aim/bullet/weapons sync, damage, death | todo | e2e + differential | |
| M8 | Nametags, scoreboard, spectating, camera RPCs, remaining RPC behaviours | todo | e2e | |
| M9 | Audio streams | deferred | — | |
| M10 | Windows validation run | deferred | user on Windows | final check outside Wine |

## Decisions waiting for the user

| ID | Decision | Notes |
|---|---|---|
| D-1 | SA:MP assets (`SAMP/` folder, loading screen) | **resolved (2026-11-04): load from the user's existing SA:MP install, never redistribute** (user asked for the SA:MP loading screen; confirm if this reading is wrong) |
| D-2 | Legal review before any public release | licence (f) "no derivative works" vs EU interoperability right |
| D-3 | Git commits | **resolved (2026-11-04): Claude (main session) commits; plain messages, no task IDs, no session/attribution trailers; push only on request; remote ghosty2004/sandrust-mp** |
| D-4 | samp.saa data overrides (10 of 23 files differ; handling/weapon identical) | **resolved (2026-11-04): option A — read ALL redirected files from the user's samp.saa at runtime, exactly like the original (user: "I want 1:1"); never redistribute; fallback to our own minimal script when SA:MP isn't installed** |

## Decisions made autonomously

Since 2026-11-04 the user delegates technical decisions to Claude with one goal:
**rebuild the original client 1:1**. Rule of thumb: when in doubt, match the
original client's observed behaviour (tracer, captures); never ship SA:MP or GTA
files (read them from the user's installs). Still escalated to the user: pushing,
legal/publishing, system installs, irreversible actions outside the project.

| Date | Decision | Rationale |
|---|---|---|
| 2026-11-04 | Do not reproduce samp's 2× REMOVE_BLIP with uninitialised handles at spawn | they operate on garbage values (a samp bug); no observable effect to match |
| 2026-11-04 | On remote-ped destruction, clear `CWorld::Players[slot].m_pPed`, which samp leaves dangling | measured by TRACE-2: no observable effect (only read while PlayerInFocus = slot); avoids a dangling pointer |
| 2026-11-04 | Pass the remote slot to CREATE_PLAYER/GET_PLAYER_CHAR as an immediate, not via script locals like samp | TRACE-4: only the opcode argument encoding differs; GTA-internal sequence and all effects are identical |
| 2026-11-04 | Functional data tables from samp.dll (vehicle 256-colour palette, player/radar palette) may be embedded as constants with source address + build recorded, like the datagram S-box | needed for 1:1 colours; interop constants, not code |
| 2026-11-04 | Every Wine prefix gets a private Documents folder (Wine had symlinked it to the user's ~/Documents); the test-created `GTA San Andreas User Files` was moved out of ~/Documents into wine/archive/; headless runs use 1280x720 | lanes must not share gta_sa.set / sa-mp.cfg / chatlog, and must not write into the user's personal folders |
| 2026-11-04 | Test runs are muted (Wine audio → private null sink `samp_rs_null`) | user heard test audio; `SAMP_RS_AUDIBLE=1` restores sound for manual play |
