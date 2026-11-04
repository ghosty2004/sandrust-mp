# Work log

Newest entries at the bottom. One entry per work step: what was done, how it
was verified, cost, and what changed in `docs/ROADMAP.md`.

## 2026-11-04 — Project bootstrap

- **Reference binaries:** two samp.dll 0.3.7-R4 builds (2019, 2021) stored under
  `re/samp-dll/`; the 2019 build later matched the official installer byte for byte.
  gta_sa.exe 1.0 US stored under `re/gta/`.
- **Tooling:** REA MCP + Ghidra 12.1.2; Rust stable; open.mp 1.6.7 test server in `server/`.
- **RE (RE-1):** handshake/cipher/RakNet/join/RPC framing → `docs/re/handshake.md`,
  `rpc-ids.md`, `build-diff-r4.md` (builds are protocol-identical).
- **NET-1:** cipher + cookie exchange, then RakNet reliability layer, auth via
  open.mp's fixed challenge table (generated locally, not committed), join,
  InitGame. Verified live: the bot joins open.mp.
- **RE-2 (workflow, 8 agents, ~1.7M tokens):** every RPC/sync packet specified per
  subsystem; coverage critic found no gaps.
- **RE-3:** 384 samp.dll memory patches to gta_sa.exe extracted and annotated with
  GTA function names and effects.
- **NET-2 (workflow, 9 agents, ~1.0M tokens):** typed codecs for all 160 RPC ids,
  all sync packets, Huffman string codec (verified against live open.mp traffic);
  221 tests, clippy clean.
- **NET-3:** bot class selection, spawn, on-foot sync; server-side probe filterscript
  confirms state ONFOOT and movement. Heading convention fixed from RE-4 findings.
- **RE-4 (3 agents, ~0.9M tokens):** remote on-foot / vehicle / combat control;
  key hook claims verified byte-for-byte against both binaries.
- **NET-4:** `samp-dump` (pcap/pcapng decoder) + `tools/capture.sh`.
- **ENV-1:** Wine 11 prefix with d3dx9 + DXVK; GTA SA (Steam data + 1.0 US exe) runs;
  official SA:MP 0.3.7-R4 installer extracted (not executed) and installed in the
  prefix; the original client joins the local server.
- **Differential results:** found the missing empty `0x29` static-data message
  (fixed); RPC 25 layout (incl. trailing duplicate response) confirmed; all sync
  packet sizes/reliabilities/channels match the specs; bot now sends 205 stats.
- **Phase 3 start:** i686-pc-windows-gnu target installed; a Rust cdylib's DllMain
  verified running inside the Wine prefix. M1 agent started (launcher, client DLL,
  control channel, `tools/e2e/m1.sh`).

## 2026-11-04 — E2E infrastructure (ENV-2 partial)

- `tools/e2e/lib.sh`: isolated test server (`target/e2e-server`, rsync of `server/`,
  port 7777, announce off) with probe filterscript `tools/e2e/probe/e2eprobe.pwn`
  logging connect/class/spawn/stream-in/out and per-second state; `e2e_wait_log`,
  `e2e_ctl` (client DLL control channel), safe cleanup (exact-binary matching —
  an earlier `pkill -f` killed the test shell itself).
  Verified with samp-bot: server up → spawn observed → clean shutdown.
- User installed gamescope + xdotool. Headless gamescope (GPU) verified with vkcube:
  `gamescopectl screenshot` captures only the virtual display; xdotool reaches its
  Xwayland. Helpers `e2e_headless_start`, `e2e_shot`, `e2e_key[down|up]` added.
  Pending: GTA under Wine inside headless gamescope (after M1 finishes).

## 2026-11-04 — M1 done (launcher + client DLL skeleton)

- Agent (~216k tokens) built `crates/samp-launcher` (suspended start + DLL injection,
  `-c -n -h -p` like the original), `crates/samp-client` (cdylib for i686-pc-windows-gnu:
  logger, in-house patching, startup patch table, per-frame hook at the Render2dStuff
  call site 0x53EB12, control channel on 127.0.0.1:47700 with `ping/state/peek/quit`),
  `tools/run-client.sh`, `tools/e2e/m1.sh`, notes in `docs/phase3/m1-notes.md`.
- Main-session hint during the run: control channel already showed the game in
  state 9 with CJ at Grove Street but `ticks=0`; the agent showed the hook is installed
  and simply fires ~2 s after state 9 (first Idle frame); state now reports `frame_hook`.
- Extra fix beyond samp: an intermittent streaming deadlock (1 in 6–8 boots, state 8)
  replaced by a sliced wait at 0x156CDAE; rescue path not yet exercised.
- Verification (main session): `tools/e2e/m1.sh` exit 0 — player position after 13 s,
  ticks=150, call-site peek `e8a95d3179`, clean quit; `cargo test --workspace` 246 passed;
  clippy 0 warnings on host and i686-pc-windows-gnu.
- Deferred: splash rename (UX-1), InitImageList RET, texture pool resize; stock
  main.scm still moves the player after spawn (M2).

## 2026-11-04 — ENV-2 done, tracing lane, M2 + TRACE-1 started

- ENV-2: games launched by `tools/run-*.sh` skip the Wine virtual desktop when
  `E2E_HEADLESS=1`; `e2e_headless_run CMD` wraps a run on the isolated gamescope
  display and always tears it down (process group). Verified: `tools/e2e/m1.sh` exit 0
  inside headless gamescope; isolated screenshot shows GTA rendered with our DLL
  (it also shows the stock story intro script running — an M2 item).
- Lessons: a bare `wait` waited on gamescope's `sleep` child; `pkill -f` with a
  pattern from the caller's own command line killed the test shell (exit 144) — use
  `pkill -x NAME` / PIDs.
- ENV-3: second lane `wine/prefix-trace` + `wine/gta-sa-trace` (hard-linked data,
  real copies of top-level files), ~0.5 GB extra; `/home` 85 GB free, `/` 17 GB free.
- Roadmap: DIFF-1..4 added (same state queries on the original client, per-RPC
  scenario, cross-client test, visual diff) after the user asked what else a 1:1
  environment needs.
- Started in parallel: M2 (in-game join, class selection, spawn, on-foot sync; lane
  main) and TRACE-1 + DIFF-1 (tracer DLL next to the original samp.dll, multi-`--dll`
  launcher, call-sequence diff; lane trace).

## 2026-11-04 — M2, TRACE-1, DIFF-1 done

- TRACE-1 (~438k tokens): `crates/samp-tracer` hooks 52 GTA functions and logs JSONL;
  launcher takes repeated `--dll`; `tools/trace/run.sh orig|ours|bare` + `diff.py`.
  Key findings, relayed live to M2: samp replaces main.scm (and serves 23 data files
  from samp.saa via a file hook; 10 differ from stock, handling/weapon identical);
  exact spawn call order; connecting-screen camera block.
- DIFF-1: the tracer serves the same `state` protocol for the original client;
  output identical to ours in the same run.
- M2 (~455k tokens): in-game UDP connection from the frame tick, join, InitGame,
  own minimal main.scm (redirected open at CTheScripts::Init), class selection with
  the original key rules, spawn routine in traced order, on-foot sync 207 + stats 205,
  basic S→C player RPCs, `net` control command, `SAMP_RS_CTL_PORT`.
- Verification (main session): `tools/e2e/m2.sh` exit 0 — spawned, W held, server-side
  distance 16.12 m, st=1; `cargo test --workspace` 289 passed; clippy 0 (host + i686).
- Tracer diff of ours vs original after Shift: first 15 steps identical; remaining
  gaps recorded as M2-P. D-4 measurement recorded; recommendation C.

## 2026-11-04 — D-4 decided (A), M2-P parity fixes, RE-9/SAA-1 + M3 + TRACE-2 running

- User chose D-4 = A: read all redirected data files from the user's own samp.saa
  at runtime, exactly like the original ("I want 1:1"); never redistribute; fallback
  to our own minimal script when SA:MP is absent. Added RE-9, SAA-1, SAA-2.
- M2-P: the M2 agent applied the tracer-diff parity fixes (frame-1 ped flags,
  connecting-screen block, model 596 release, always-SetModelIndex via the direct
  path, RPC 67, time of day, preload polls); m2.sh 3× pass per agent. Pending tracer
  re-diff. It edited client.rs/game.rs/script.rs while M3 had started — M3 told to
  re-read and re-verify m2.sh.
- Running in parallel: M3 (lane main), TRACE-2 (trace lane: original client's
  remote-player handling), RE-9 + SAA-1 (samp.saa format + pure-Rust reader, no game runs).

## 2026-11-04 — Master plan + new goal; RE-5 and DIFF-2 started

- `docs/PLAN.md`: definition of done (F1–F9), phases A–H with acceptance per
  milestone, standard autonomous loop, concurrency and escalation rules. User set a
  new /goal: F1–F7 met and phases A–G done.
- TRACE-2 finding 1 relayed to M3 (original stream-in sequence: CREATE_PLAYER slot 2
  via the script executor, GTA's internal ped setup, ped flags, direct SetModelIndex,
  Players[2..209] zeroed from frame 1).
- ENV-4: third lane "diff" (wine/prefix-diff, wine/gta-sa-diff hard-linked, 7795,
  47705/47706).
- Started: RE-5 (UI rendering specs, no game runs, no REA) and DIFF-2 (per-RPC
  coverage harness for F1 in lane diff, with baseline + determinism check).
- Running: M3 (main), TRACE-2 (trace), RE-9/SAA-1 (no game runs, uses REA).

## 2026-11-04 — RE-9 + SAA-1 done; audio incident fixed

- samp.saa format specified (`docs/re/samp-saa.md`) and `crates/samp-saa` implemented
  (~228k tokens). Main-session check: signature valid on the user's archive, 28 entries
  listed (incl. loadscs.txd = SA:MP loading screen), all tests pass incl. real-archive.
  SAA-2 must hook at `CreateFileA` by basename like samp (CFileMgr would miss TXDs,
  streaming reads of script.img and text-mode handling).
- Audio: test GTA audio now goes to a private null sink; creating it made PipeWire
  switch the default sink and Spotify went silent. Restored the user's default sink,
  moved Spotify back; `tools/wine-env.sh` now restores the default after creating it.
- TRACE-2 findings 3–4 relayed to M3; decision logged: clear Players[slot].m_pPed
  after destruction.

## 2026-11-04 — TRACE-2 done; TRACE-4 started; lane hygiene

- TRACE-2 (~284k tokens) done: stream-in, per-frame, stream-out/quit and Players[]
  guard-page measurements (104,400 GTA accesses, 2 samp). Corrected the Players[]
  end address in remote-control-onfoot.md (0xB915B8, was 0xB91638).
- M3 reports m3.sh and goal.sh passing (final report pending).
- Fixed `tools/e2e/lib.sh`: `e2e_cleanup` killed every samp-bot (incl. other lanes');
  it now kills only bots targeting the lane's own server port.
- Started TRACE-4 in the trace lane: (1) M3 parity re-diff vs TRACE-2 runs,
  (2) SP-OFF baseline (5-min idle: original vs stock vs ours), (3) vehicles for M5.

## 2026-11-04 — NET-7, RE-5 done; RE-10 and M4-core started

- NET-7 (~213k tokens): `rpcs::OutgoingRpc` with send params per C→S payload,
  per-module + top-level `Outgoing`, typed RPC 25, Incoming `id()/write()` everywhere.
  Main session: 334 tests pass, clippy 0. A transient build break from this agent
  blocked M3's runs for ~1 min → new PLAN §4 rule: shared crates must stay compiling
  after every edit.
- RE-5 (~412k tokens): `docs/re/subsystems/ui-rendering.md` — Present-wrapper render
  path, draw order, fonts/colour codes/outlines, DXUT-based GUI skinned by sampgui.png,
  all dialog styles, scoreboard, nametags, death window, key table, sa-mp.cfg keys;
  corrections to ui-chat-audio.md (F1 local help dialog, /hudscalefix scope).
- TRACE-4 part 1: M3 matches the original on stream-in, swap, write timing, destroy;
  9 differences triaged (4 sent to M3; cops arrested our player → SP-OFF prioritised;
  pistol skill → M2-P; 2 logged as decisions).
- Started RE-10 (static gap check + measurement requests for M5/M6/M7, CSV rows per
  milestone) and M4-core (`crates/samp-ui`, pure UI logic from RE-5).

## 2026-11-04 — M3 done (phase A core complete)

- M3 (~410k tokens): sector-grid relocation (84 refs, all-or-nothing) + old grid zeroed,
  patch rows 110–118/162/167/173–178/183/193/206/213/214, ProcessControl/UseGun swap
  hooks with full save/restore, ProcessCollision gate, stream-in/destroy/per-frame apply
  in the measured order, head look-at, 207/203 decoding, streamed-ped count in the send
  interval, `remote` control command, `tools/e2e/{m3,goal}.sh`.
- Verification (main session): `tools/e2e/m3.sh` exit 0 (slot 2, 0.2 m tracking,
  8.38 m path in 5 s, ped deleted on quit); `tools/e2e/goal.sh` exit 0 (our client boots,
  connects, spawns, server sees 16 m walk, samp-bot appears as remote player and moves);
  `cargo test --workspace` 337 passed; clippy 0 host + i686.
- Open parity (M3-P): speed magnitude/cadence vs original; SEH containment.
- The weapon flapping seen by TRACE-4 was gungame reacting to our post-arrest state —
  removed by SP-OFF.

## 2026-11-04 — Git, worktrees, more parallel work

- User created github.com/ghosty2004/sandrust-mp; the project will be renamed
  sandrust-mp (crates sandrust-*) at a quiet moment. Claude now commits (plain
  messages, no task IDs, no session/attribution trailers); push only on request.
  Commits so far: initial import + docs/harness/measurement commits.
- Parallel samp-client writers via git worktrees: ~/samp-rs-wt/m4 (branch ui-render,
  lane ui: chat/input/local commands) and ~/samp-rs-wt/m5 (branch vehicles, lane veh);
  main checkout: samp.saa serving then single-player systems off.
- TRACE-4 done (remote re-diff, SP-OFF baseline, vehicles). SP-OFF gaps sent to the
  samp-client writer (max wanted level 0 + CSV rows 181/185/164/189/182/186/55/163).
- DIFF-2 done: per-RPC harness, 131/141 ids, baseline 15/20/96 (pre-M3 build).
- User rule: nothing uncertain is implemented without REA static analysis + tracer
  measurement (PLAN §3b); implementation agents now file measurement requests.
- Visual parity redefined as a Direct3D9 call-stream diff (the original's scoreboard
  itself renders broken under Wine/DXVK — screenshot shown to the user).
- Started: TRACE-3 (REA + tracer: D3D call logger, UI unknowns incl. M4's 8 requests,
  then remaining vehicle cases), DIFF-3 cross-client + F5 patch coverage (lanes diff +
  diff2), BOT-2 (bot drives/shoots).

## 2026-11-04 — Lane isolation incident (user files)

- TRACE-3 found that wine/prefix, prefix-diff, prefix-diff2 and prefix-shot had
  `Documents` symlinked to the user's real ~/Documents: the lanes shared one
  "GTA San Andreas User Files" (gta_sa.set, SAMP/sa-mp.cfg, chatlog, screens), and the
  tests had created that folder in the user's ~/Documents (first GTA run, 17:45).
- Fixed: private Documents per prefix; the folder was moved out of ~/Documents into
  wine/archive/ (nothing of the user's was overwritten — it did not exist before);
  `tools/wine-env.sh` now enforces private Documents and sets GTA video mode 32
  (1280x720) on headless runs. Lanes notified to re-run resolution/config-dependent tests.
- TRACE-3 batch 1 (D3D logger + chat/input/scrollbar/cursor/fonts at pixel level)
  forwarded to M4. BOT-2 done (drive/passenger/shoot; bot2.sh exit 0).

## 2026-11-04 — Data serving, chat UI and tracer merged; next wave started

- Merged into main (all e2e green on the merged code: saa, m1 (isolated), m2, m3, goal,
  m4; 470 tests; clippy 0 host + i686): samp.saa serving (all 188 file opens identical
  to the original; samp's own scripts run; custom opcodes handled), single-player
  systems off (0 peds/cars/cops, wanted 0 — same as the original), in-game chat, input
  box, local commands, game text, cursor (pixel-exact vs TRACE-3 measurements).
- TRACE-3/5 done: D3D9 call-stream logger in the tracer + d3d_diff.py; all UI unknown
  constants settled at 1280x720 and 1920x1080; passenger/locked/trailer/bike/aircraft/
  train/damage/death and aim-sync values measured. Lane lock now automatic in run.sh.
- DIFF-3 + F5 done: cross-client 13/24 match (11 diff, 18 pending); patch coverage
  173 matching / 205 missing / 3 justified (rows 61, 370, 375 logged as decisions).
- Started: M8a + M4-P (worktree m8a, lane ui), M2-P + M3-P (worktree par, lane main),
  TRACE-6 (world entities + combat measurements, trace lane, REA). M5 continues (m5).

## 2026-11-04 — Original client measured for world entities and combat

- Five live sessions on the original client: objects (create, render paths by draw distance,
  material text render target, attachment bone formula, MoveObject/StopObject timing,
  1000-frame RW release), pickups, checkpoints, gang zones, map icons, actors, 3D labels,
  building removal, edit/select; combat (shot call chains, 206/115/204 order and content,
  shotgun = one 206 + one 115, sniper and melee, 0.33 damage scaling, team rule, three
  lag-comp modes, death/respawn timeline, remote death).
- Trace hold sessions made private (session id + lock) after one run was driven by another
  agent; that window's data was discarded.
- Coverage diff (first results) lists GTA systems still running with our client (grass,
  real-time shadows, sun/moon, idle camera, ambient gunfire audio, IPL pickups, input during
  class selection) and the missing local head look; assigned to the player-parity and
  world-entity work.

## 2026-11-04 — Dialogs, textdraws, menus, nametags and scoreboard merged

- Merged the UI branch (86dfabc). Conflicts in client.rs / players.rs resolved keeping both
  sides (vehicle pool + UI snapshots).
- Verified on merged main by the main session in lane ui: m1 (headless), saa, m2, m3, goal,
  m5, m8a exit 0; m4 first failed because a queued m4 from the UI agent ran concurrently
  without my run holding the lane lock — rerun under `flock target/locks/ui.lock`: exit 0.
  496 workspace tests, clippy 0 (host + i686).
- D3D call-stream diffs vs the original: 0 lines for all dialog styles, input, textdraws +
  sprite selection, menus, nametags + bubbles, death window, scoreboard.
- m1.sh now re-runs itself on the isolated headless display (it had opened game windows on
  the user's desktop when run directly; no screenshots were taken).
- Lesson: the main session's own lane runs also take the lane lock.
## 2026-11-04 — EFX-1: effect tracer (worktree efx, trace lane)

- samp-tracer: `efx arm TARGET` single-steps a client function (RPC handler from rpc-ids.md,
  RVA, game function, `event:frame`) through samp.dll and every game function it calls
  (iced-x86 decoder for memory operands; return-address stub; foreign code stepped over), logging
  game-memory writes (old/new, symbolic: player_ped+0x540 …), game calls (args, return),
  branch-deciding reads. `efx watch SPEC` = guard-page + int3 call-hook window for our client.
  New: src/efx.rs, src/win/efx.rs; small hooks in control.rs, win.rs, lib.rs; iced-x86 dep.
- tools/efx: run.sh (orig|ours|both, lane lock), scenario.sh, efxfs.pwn (one RPC per
  command), efx_diff.py (dump/spec/diff with baseline subtraction, gta-reversed names, SCM
  opcode names). Notes: docs/phase3/efx-notes.md.
- Results: RPC 14 identical; RPC 12 same Teleport call + matrix (diffs = original's IK look-at
  state); RPC 153 not implemented for the local player in our client; RPC 70 same 5 script
  commands (ours lacks GET_CURRENT_CHAR_WEAPON, changes aiming rotation).
- Verified: cargo build --workspace, 492 tests, clippy host + i686 0 warnings; 4 lane runs.

## 2026-11-05 — Player parity and the effect tracer merged

- Effect tracer merged (082680c): single-step capture of a samp.dll function and its GTA
  effects, page-guard capture for our client, diff tool. First diffs: SetPlayerHealth
  identical, SetPlayerPos identical apart from the head look, local SetPlayerSkin missing,
  PutPlayerInVehicle skips one weapon query and writes an aim field.
- Player parity merged (359daf5) after its owner merged main into the branch: cross-client
  24/24 match; coverage only-ours at spawn 227 → 4, vehicle 85 → 10; join frame and
  per-frame command signature identical to the original; GTA systems samp switches off
  now switched off (grass, sun/moon, idle camera, ambient gunfire, map pickups, weapon
  camera reset); input lock from join to spawn + 10 frames.
- Verified by the main session in lane main under the lane lock: m1, saa, m2, m3, goal, m4,
  m5, m8a exit 0; 513 tests; clippy 0 (host + i686). Spawn screenshot renders the world.
- Found by looking at screenshots: class selection renders a black world in our client
  (assigned to the UI owner); row 165 broke world rendering and is not applied.

## 2026-11-05 — UX-1, DIFF-4 harness, M8a leftovers (worktree ui2, branch ui-loading)

- Tracer: Direct3D logging from the first Present (`SAMP_TRACER_D3D_BOOT`), `game_state`
  records, late D3DX hooks. Trace-lane scripts accept `SAMP_RS_ROOT` (worktree runs share the lock).
- Measured the original's boot, first UI frame, input-lock modes, F1/F5/F8/F10, clipboard,
  `mdl` sprites and a remote driver's nametag (sessions orig-20261104-235122 / -235538, visual/orig).
- Fixed: missing D3DSBT_ALL state blocks (black class-selection world; patch merged by main as
  831455f); first-frame UI construction in samp's order inside the game scene; nametag/bubble
  passes gated like samp; class-selection lock from the game frame; sampgui.png mip levels;
  timestamp width measured once; F1 help, F5 net stats, F10, F8 screenshots (+ SAMP\screens),
  clipboard (DXUT semantics), `mdl` names, driver nametag anchor, all input-lock setter modes.
- New: `tools/diff/visual.sh` (+ scenario, visual_diff.py, boot_diff.py, decisions TSV),
  `tools/e2e/ux1.sh` (+ ux1_shots.py), original-side session scripts.
- Verified (lane ui under ui.lock, trace lane under trace.lock): visual.sh exit 0 (all 25 elements
  clean, glyph splits covered), ux1.sh exit 0, m1/saa/m4/m8a exit 0, 517 tests, clippy 0 host + i686.

## 2026-11-05 — Loading screen, visual harness and world entities merged

- UI branch merged (820e431): loading screen identical to the original (same 582 loading
  Presents, same-moment screenshot difference 0.00), tools/diff/visual.sh with 25 UI
  elements at 0 open lines, F1/F5/F8/F10, clipboard, input-lock modes, remote-driver tag.
  Verified in lane ui under the lock: m1, saa, m2, m3, goal, m4, m5, m8a exit 0.
- D3D state-block fix (831455f) for the black world in connecting/class selection.
- World entities merged (bafc841, fast-forward of the branch after it merged main):
  objects incl. samp's passes, material colour/text (pre-scene render, blanktex), attached
  objects, building removal, pickups, checkpoints, actors, gang zones, map icons, 3D labels,
  ProcessVerticalLine row 96. Verified by the main session in lane veh under the lock:
  m1, saa, m2, m3, goal, m4, m5, m6, m8a exit 0; 528 tests; clippy 0 (host + i686).
  Parity vs the original on the same scenario: checkpoints and icons identical, objects /
  labels / zones differ only in repeat counts (fps), the rest is server ordering.
