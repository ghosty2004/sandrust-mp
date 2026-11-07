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

## 2026-11-05 — Combat and remaining behaviour merged

- Parity audit of main (docs/phase3/par-audit.md): F5 292 applied / 88 missing / 3 justified;
  DIFF-2 64 match / 37 diff / 30 not implemented / 10 untriggerable; DIFF-3 24/24; row 165 safe
  (the black world was the D3D state-block bug); interior entry gap = RPC 156 missing (now in).
- Combat merged (bf9a588) after merging main into it: aim, weapons, bullets per lag-comp mode,
  damage, death/respawn, drive-by, projectiles. Integration fix: the damage hook was installed
  twice (combat + world entities); one hook now, fed with the world-entity actor list.
- Remaining behaviour merged (3b1ffad): camera RPCs, widescreen, spectating, world bounds,
  gravity, interiors, animations + table, drunk, fighting style, SCM events, ClientCheck,
  misc RPCs. Conflicts resolved (stats packet keeps combat's position with the drunk level).
- Verified on main in lane main under the lock: m1, saa, m2, m3, goal, m4, m5, m6, m7, m8a exit 0;
  m8b failed once on the frame-rate-sensitive drunk check, exit 0 on rerun (test to be fixed);
  555 tests; clippy 0 (host + i686).

## 2026-11-05 — Audio and client features merged; every milestone implemented

- Audio + client features merged (9ac9007): BASS call stream 0 differences in 16 segments
  vs the original; sa-mp.cfg loader/rewrite; chat log; kick/lost/full/password/name/gmx/quit.
- Full e2e on the complete main (lane ui, under the lock): m1, saa, m2, m3, goal, m4, m5, m6,
  m7, m8a, m10, ux1 exit 0; m8b and m9 failed on test defects (time-based drunk check, missing
  original audio reference after the worktree was removed). Fixed (d7f3173): m8b counts frames
  (3/3 pass), m9 reference regenerated (pass, 0 BASS differences).
- 562 tests; clippy 0 (host + i686).

## 2026-11-05 — Audit fixes merged (vehicles/radar, lifecycle/ClientCheck, patches/HUD/bounds)

- Radar markers for remote players, remote driver exit task, rows 177/62 (merged 6dbbfb3).
  Cross-client vehicle steps 44/45 (F-enter run vs walk open).
- Lifecycle: RakNet 10 s ack timer, keep-alive 5 s / 15 s, server-full retry 3.04 s,
  unreliable /quit, audioproxyoff after BASS init; ClientCheck answered honestly (7419386).
- Patches: HUD aspect fix (radar round at 16:9, 45.6 → 0.0 vs original), shadows 165/184/166,
  byte rows 225–239/223/224/170/215–217/50, row 16; SetPlayerPosFindZ implemented (DIFF-2
  match 82 → 102); samp's jump at world bounds from its ProcessControl hook (102d456).
  F5 missing 56 → 27.
- Verified on main in lane main under the lock: m1–m9, goal, saa exit 0; m10 failed once on a
  3 ms early keep-alive (4.997 s vs ≥ 5.00 s) — handed to the network-timing work.
- 565 tests; clippy 0 (host + i686).

## 2026-11-05 — M6-P world/vehicle parity follow-ups (worktree m6p, branch world-leftovers)

- Measured on the original first (trace lane runs orig-20261105-042300 / -051538), then implemented and diffed
  with the same input on ours (lane veh): object editor gizmo + drag + save, attached-object editor, selection
  picking (27), CSV row 81 (samp's camera test outside ±3000), number plates (rows 59/60 + SetTexture), train exit
  inside the constructor hook, nametag LOS always on (samp overwrites the InitGame bit). Details:
  docs/phase3/m6p-notes.md.
- Diffs: `tools/diff/editor_d3d_diff.py` MATCH for the editor (154 calls), hover, drag, rotate mode and selection
  (179); plate painter MATCH (58); 117 drag cadence/values and 116/27 equal; DIFF-3 45/45 MATCH incl. veh_enter_B.
- 569 tests; clippy 0 (host + i686).

## 2026-11-05 — NET-6: RakNet timing parity and the F2 network diff (branch net-timing)

- Static (A): samp's RakNet runs an update thread every 2 ms (threadSleepTimer 2); acks every
  cycle, priority queues, packing by RakNet's header estimate in (576 − 28) × 8 bits, resend
  timeout 3 × max(ping, 30 ms) without backoff, update-cycle ping only until a pong, keep-alive
  only with an empty resend list, splits, MTU never changed. Details: docs/phase3/net6-notes.md.
- `samp-proto::connection` rewritten to those rules; `samp-client::netclient` runs RakNet's part on
  a 2 ms update thread (PlayerConnect from the game thread's tick).
- F2: `tools/diff/network.sh` (+ netscen.sh, net_report.py, netcap.py, lossproxy.py,
  net-decisions.tsv): original in the trace lane, ours in lane ui, 17 scenario windows, loss via a
  userspace proxy. Final result: 303 checks, 296 match, 7 accepted (NET6-D3..D5), exit 0. Measured on
  the original and matched: ack p50/p90 1.1/2.0 ms, ack-only datagrams, 2 pings, resend 90 ms
  ×3 without backoff.
- Keep-alive base on RakNet's whole-ms clock at the update that takes the send (m10 had seen
  4.997 s on main); m10 window 4.999–5.010 / 14.999–15.010 s from 33 original gaps; m10 now 5.002 s.
- App-level residuals queued: NET6-P1..P4 (idle 203, spectator 203, 209 after passenger exit,
  Escape in the object editor).

## 2026-11-05 — Soak findings fixed (worktree soakfix, branch soak-fixes)

- Derby crash (gta_sa 0x4F1464, stream thread "start position modulo track length"): the Steam 3.0 audio
  data has 122 zero-sample stub tracks the 1.0 exe cannot start. New `tools/audio-stub-tracks.py`
  (check / patch / restore) aliases them in the game copy's TrakLkup.dat (new inode, shared original kept as
  `.steam`); `tools/wine-env.sh` applies it per game copy. No samp.dll patch touches that path; the original
  crashed there before (TRACE-7). Derby in lane main after the fix: 0 crashes (runs B–D).
- Exploded unoccupied car: our RPC 136 rule = samp's (last driven vehicle only); measured on the original in
  the trace lane (uifs, explosions). No client change.
- 209 for a pushed parked car: same rule as the netp lane's NET6-P3; left to it.
- Remote player without updates: reproduced with a capture; the server relays no 207 because samp-bot's
  standing shooter never re-sends an identical 207 (shared duplicate-suppression timestamp with its
  always-different 203). samp-bot: aim sway turns the ped; no sync between RPC 40 and InitGame (a bot that
  synced in the gmx gap got stuck in class selection). Client diagnostics: `foot_rx`/`foot_late`/`last_ts`
  in `remote`, `syncs_bad` in `net`.
- soak_mon: one-sample grace for a local seat change in flight. tools/trace/lib.sh: TRACE_GAMEMODE may name a
  bundled gamemode.
- Verified: e2e saa, goal, m1–m10 exit 0 (m9 with M9_ORIG = main's original capture); tests, clippy host +
  i686 0 warnings; soak runs in docs/phase3/soak-notes.md "Resolution of the findings".

## 2026-11-05 — F2 proven on main; soak fixes merged

- tools/diff/network.sh on main (ddd8700 + later docs/tools commits, no client change since):
  `network exit=0` — `checks: 311, matching: 311, accepted decisions: 0, open differences: 0`
  (run target/merge-netp/network-rerun.log).
- Soak fixes merged (421913f): audio stub tracks mapped in test game copies (crash identical with
  the original client; environment fix), samp-bot sync gaps fixed. SOAK_SECS=600 soak.sh PASS on
  gungame, derby, simpletdm with 0 crashes (agent run); full 2 h soak pending on final code.
- Lane locks no longer leak into gamescope/test server (ed9a7d3) after an orphaned display held
  the trace lane for an hour.

## 2026-11-05 — FEAT-B player-visible features (branch player-features)
- Implemented: held items 20–23 and piss 68 for the local and remote peds (specials.rs), the remote parachute object
  (specials.rs, from the 207 PARACHUTE anims), `/dl` labels + the InitGame vehicle-model preload (vehlabels.rs),
  `/testdw`, window class style + "GTA:SA:MP" title, the "Version Error" box, SEH-guarded script commands and
  `CGame::Process` ("opcode" / "add_car_component" / "s007" lines, one 10-fault counter), debug lines to chat, the
  crash filter + dialog without upload (crash.rs, `samp-rs-crash.txt`).
- Verified: original runs in the trace lane (orig-20261105-100040 / -100640 / -101602), ours in lane ui:
  `tools/e2e/featb_report.py` 25/25 assertions, per-step script-command parity diff empty; visual diff `dl`, `dloff`,
  `testdw` OK (glyph-atlas split covered); `featb-version.sh` box on both clients. Tooling: tracer `window`
  command, bot `--anim-seq`, uifs `pstate`, visual.sh scenario override. Notes: docs/phase3/featb-notes.md.

## 2026-11-05 — FEAT-A: network-visible features of the completeness audit (worktree feata, branch network-features)

- Measured on the original first (lane veh exploration orig-20261105-094437; trace-lane parity run
  target/trace/orig-20261105-103210), then implemented: SCM event sender 96 (+ type 3/4 local effects) and a fixed
  0x09D3 (it was inverted, so mod garages never closed in our client), camera target 170/168 + `/ctd` labels, F4
  class reselection, scoreboard double click 23, pause-map right click 119 (sent from the Present pass), Car Jacked,
  PlayCrimeReport (stand-in ped, 0x4E7529 NOP, temporary vehicle, scanner event), SendGameTimeUpdate timer resync,
  RPC 48 logged (no effect in R4). Details and evidence: docs/phase3/feata-notes.md.
- Diffs: C→S RPC sequence of the parity runs identical incl. payloads (23 RPCs); server callbacks identical; tracer
  windows (crime report, Car Jacked) identical; `/ctd` D3D sprite/font stream identical but for the caller module;
  `tools/diff/network.sh` (+5 windows, lane veh, tracegm): 403 checks, 403 match.
- New: tools/e2e/feata.sh (PASS), featscen.sh, featparity.sh; uifs.pwn callbacks/camtgt; netscen windows.

## 2026-11-05 — Feature-sweep features merged

- FEAT-B (1c0a90e) and FEAT-A (46300da) merged: special actions, /dl, /testdw, window title, message boxes,
  guarded calls, crash handler (no upload), parachute; SCMEvent + mod-shop opcode fix, camera target, F4,
  scoreboard/map clicks, Car Jacked, crime report, time resync. network.sh with the new scenarios 403/403 (agent).
- Verified on main in lane ui under the lock: m1, saa, m2, m3, goal, m4, m5, m6, m7, m8a, m8b, m9, m10, ux1,
  feata exit 0; 590 tests; clippy 0 (host + i686).
- User crash on a public server (0x7F5A3A RwD3D9DynamicVertexBufferCreate NULL read after a device-loss stall)
  handed to CRASH-1.

## 2026-11-05 — CRASH-1 object-map crash (branch object-crash)
- Symptom (user, public server): `/halloween` teleport to an object island → bursts of RPC 44/99/47 → gta_sa.exe
  0xC0000005 about 1 s later, nothing logged. Wine: read of 0 at 0x7F5A3A (`RwD3D9DynamicVertexBufferCreate`).
- Reproduced locally with tools/e2e/probe/objmap.pwn (400 global + 480 player objects on an island at 4010, 2118,
  custom 18631+/19xxx models, materials / material text, MoveObject right after create, destroy/re-create churn,
  streamer hops, two rounds): ours crashed at 0x7F5A3A every run (trace lane ours-20261105-110743, veh lane
  objstress pre-fix: same report), the original survived (orig-20261105-110616 / -111235, 0 s without updates).
- Root cause (measured with the new CreateVertexBuffer log in ours and a temporary tracer hook of device slot 26 for the
  original, orig-20261105-111235: 700 creates, none of length 0; not kept, it would add records to the D3D diffs): ours asked for
  zero-length dynamic vertex buffers (D3DERR_INVALIDCALL), the original never does. Our material pre-pass applied
  slot colours (geometry PRELIT off, MODULATE on) from the very first draw; a geometry with a pending prelight lock
  (lockedSinceLastInst 0x8, GTA's day/night prelight update) re-instanced with PRELIT off → empty dynamic stream →
  failed create leaves a buffer-less record in RenderWare's dynamic list (0xC97B8C) → the next differently sized
  create releases that NULL buffer → crash. samp gates the pre-pass (+0x118F drawn flag, +0x1D3 last RW object):
  nothing before the first draw, record-only on the first pre-pass after the RW object changed, apply afterwards.
  Fix: `worldents::MaterialPrePass` + worldhooks gating, exactly that state machine.
- Device loss (coordinator lead): forced Reset (orig-20261105-113441 survives; tracer poke of TestCooperativeLevel → NOTRESET, scenario
  TRACE_OBJMAP_DEVRESET=1; our `devreset` control command): the original releases every material-text render target
  in its lost handler (B 0x100C3F60 → 0x10012B20 → 0x100A8EE0) and re-creates them in the object pass; ours kept
  them (D3DPOOL_DEFAULT) so every Reset failed with D3DERR_INVALIDCALL (ours-20261105-113605: 1776 failed resets, game frozen, and RenderWare
  left with buffer-less dynamic records = the same crash site). Fix: `mattext::on_lost` before Reset.
- 6-minute 0 fps stall in the user's log: 10 frames in 348 s while the network thread kept receiving (rpcs 828 →
  2065), no device Reset logged: the game window lost focus (GTA only runs a frame per window message while
  inactive; the original freezes the same way: trace run orig-20261105-113214 with a focus change, 36 s without
  updates). Not a hang, unrelated to the crash.
- Crash diagnostics: crash report "Diagnostics (samp-rs)" section + lock-free breadcrumb ring (crash.rs), failed
  CreateVertexBuffer log (ui/d3d.rs). Regression test tools/e2e/objstress.sh (objmap + forced device loss; fails on
  a crash report, a failed CreateVertexBuffer or a failed Reset); trace lane `tools/trace/run.sh orig|ours objmap`.

## 2026-11-05 — HANG-1 join hang after "Connected. Joining the game..." (branch join-hang)
- Symptom (user, public server): main thread at 100 % after `packet Accepted`, no frame ticks; the crash breadcrumbs
  end in a run of RPC 43 (RemoveBuildingForPlayer) sent from the server's OnPlayerConnect.
- Root cause: our RPC 43 cleared all 0x12 bytes of every map occluder within 500 units, including the 15-bit
  next-occluder link at +0x10. GTA's `COcclusion::ProcessBeforeRendering` (0x7201C0) walks its nearby / far-away
  occluder lists through that link every frame with no bound; link 0 closes a cycle, so the frame never ends.
  samp (A 0x1009D0F0) clears only centre and size (+0..+0xB). Not a network bug.
- Fix: `worldents::OCCLUDER_CLEARED` (0..0xC) used by `Objects::remove_building`; unit test
  `removal_keeps_the_occluder_list_link`; spec world-entities.md §7 corrected.
- Regression: `tools/e2e/joinburst.sh [ours|orig] [RUNS]` + `tools/e2e/probe/joinburst.pwn` (1562-call join burst:
  400 removals incl. a 500-unit grid over the map, 300+200 textdraws, 60 messages, 100 map icons, 300 player
  objects, 621 vehicles, 4 KB split-packet dialog); `JB_LOSS=mix-sc:8` runs it through lossproxy.py's new `mix-sc`
  rule (server→client drop + reorder + duplicate). Before the fix ours hangs (3/3 runs, with and without loss);
  after: ours 6/6 pass (3 plain, 3 with loss, identical 1382 messages / 1377 RPCs), original 2/2 pass.
- Parity note (not fixed here): ours requests a class while the join dialog is still open; the original sends its
  class request only once the dialog is closed (joinburst server logs: class at +4 s vs. with the dialog response).

## 2026-11-05 — INPUT-1 chat Enter answered the dialog the server showed in reply (branch enter-leak)
- Symptom (user, real server): a chat command answered with ShowPlayerDialog was immediately answered by our
  client (RPC 62 response 1) as if Enter had been pressed in the dialog; the original leaves it open.
- Measured on the original first (trace lane, runs orig-20261105-193637 / -195939; new uifs/m8afs commands
  `chatdlg|ID|STYLE|DELAYMS|VIA` and `chaindlg|FROM|TO|STYLE`): styles 0/1/2/3, via OnPlayerCommandText and
  OnPlayerText, server delay 0 / 50 ms, Enter tapped or held 250 / 300 / 1200 ms, and dialog chains (input→input,
  msgbox→input, input→msgbox): never an early RPC 62 (capture: only the six 62s of the dialogs Enter was meant
  for). With Enter held 2.5 s the original sends the chat line only after the key-up (sent_while_held=0).
- samp's rule (static, A): its DXUT edit box raises the "string" event on `WM_KEYUP` VK_RETURN (A 0x10089380)
  and ignores the typed CR; the chat line is therefore sent on the key-up, and a dialog edit box / the window
  procedure's key-up path (A 0x10061360) answer a dialog on the key-up too. Key-downs and auto-repeat do nothing.
- Ours sent the line on the `WM_CHAR` CR, so the reply dialog arrived before the key-up, which answered it.
  Fix (`samp_ui::keys::route`): CR characters do nothing; Enter key-up sends the chat line (input open) or
  answers the dialog. Unit tests `chat_enter_does_not_answer_the_reply_dialog` (core) and keys routing.
- Regression: tools/e2e/m8a.sh INPUT-1 steps (command + held Enter → input dialog 90, text → message box 93,
  chain 91→92; no RPC 62 within 2 s). Specs: ui-rendering.md §6.3/§13.4, ui-chat-audio.md (Enter = key-up).

## 2026-11-05 — SOAK-DESYNC: desyncs of the 2-hour derby soak (branch soak-desync)
- Cause of every persistent desync: our RakNet duplicate filter remembered the last 8192 reliable message
  numbers; the server's single 16-bit message counter (syncs included) wraps every ~10 min, after which a
  new RPC with an already remembered number was acked and dropped, leaving a permanent hole in its ordered
  channel: no RPC applied for 277 s (join/quit, stream in/out, seats). Fix: the original's window rule
  (A 0x10049250: base + holes over all numbers, 0x8000 ahead, holes given up after 10 s) in
  `Connection::accept_number`; spec docs/re/handshake.md §3; 2 unit tests (the old filter delivers 1714 of
  5076 RPCs over 3 wraps). Bots fixed too (shared crate).
- Harness (soak_mon.py): bracketed comparison of discrete state (second server dump after the client
  query), remote_ghost denominator, pause detection (the 2 h run was stopped from outside for 3.4 h: the
  "791 MB/h" session 2), memory slope within segments between in-process reconnects; `SOAK_CLIENT=orig`
  (original samp.dll + tracer in the trace lane, state read via `peek`), `SOAK_CASES`; `TRACE_HOST` in
  trace_launch.
- Parity: same derby soak 1800 s without cases, original vs ours: both PASS, identical categories at
  100 %, the same residual remote-car position / burning-car health misses, memory 19.3 vs 18.3 MB/h
  (docs/phase3/soak-notes.md).
- Verified: cargo test --workspace, clippy host + i686 0 warnings; 30-min derby soaks and e2e in the notes.

## 2026-11-05 — F1/F5 closing (branch parity-close)

- F5: rows 179, 199, 203, 212, 245 (table), 52 (samp's `jmp [slot]` form), 106 (InflictDamage hook only with
  vehicle friendly fire), 359 (stunt bonus from InitGame), 54 (UncompressAnimation NULL guard), 95 (weather audio
  muted by PlayerPlaySound 1), 24 (surfing: probe, local/remote glue, 207 fields) applied; 97/348/362/364
  (dormant custom models), 374/380–384 (dead code), 269 (samp bug) justified. `patch-coverage.sh`: 20 missing /
  2 different → 0 / 0 (346 matching, 24 static, 14 justified), exit 0.
- F1 code fixes: explosion radius as samp's low double half, RPC 40 reset (money, armour, zone names, colours,
  wanted, clock, skills 999), 596 release only on InitGame, CREATE_PLAYER through locals, first-frame ped flags
  from the local ProcessControl hook. Harness: rpc-decisions.tsv (DIFF2-D1/D2/U1), calibration with 6 + 6 reused
  original runs, per-field numeric calibration, ensemble screenshot check, order not judged where the original
  disagrees with itself, 208 markers once, deterministic object_move, surfing steps. DIFF-2: main MATCH 112 /
  DIFF 19 (+10 untriggerable undecided) → last full run MATCH 130 / DIFF 1 (explosion screenshot timing, decision
  DIFF2-D3) → `--report` over the same 12 original runs + ours: MATCH 131, exit 0. Details:
  docs/phase3/close-notes.md, ROADMAP decisions DIFF2-*, F5.
- Verified (lane ui, flock ui.lock): m1–m10, goal, saa, ux1, feata exit 0; cargo test --workspace 595 passed;
  clippy host + i686 0 warnings.

## 2026-11-05 — F6 proven on main

- tools/diff/files.sh on main (9f47445): exit 0 — orig 240 opens (29 from samp.saa, 27 distinct), ours 240;
  29 archive + 176 disk content hashes compared; opens-set, opens-order, archive-order, sizes, hashes,
  ours-log all OK; IDENTICAL, PASS.

## 2026-11-06 — Final proof run (first pass)

- F6 files.sh: exit 0 (IDENTICAL). F2 network.sh: exit 0, `checks: 388, matching: 388, accepted decisions: 0, open differences: 0`.
- F5 exit 1: row 368 frame-local toggle caught by snapshot luck; harness now watches the writes (ff689b8): 2 runs PASS, negative test FAIL as expected.
- F1 exit 1 (130/131: gangzone_stopflash screenshot phase), F3 exit 1 (44/45: veh_enter_B short anim sampling) — handed to a deflaking agent.
- F4 exit 1: vehtag DIFF, netstats DIFF 2 lines, dl/dloff/testdw MISSING (scenario not driven for ours) — handed to a visual agent.

## 2026-11-06 — 2-hour soaks on gungame and simpletdm

- tools/e2e/soak.sh (SOAK_GAMEMODES="gungame simpletdm", 7200 s each, lane veh) on main 0a65b5f: `[soak] results: gungame:PASS simpletdm:PASS` / `[soak] PASS`. Derby 2 h started next.

## 2026-11-06 — F1/F3 deflaking (time-varying elements, calibration-derived anim rules)

- F1: the `gangzone_stopflash` DIFF was the local ped's idle silhouette, not the flash (both clients drew the
  zone identically). Tracer: additive `CPed::Render` wrap + `hideped on|off` control command; drive.py hides
  the local ped after spawn. rpc_report.py: radar draw-stream component (per-frame `CRadar::DrawAreaOnRadar`:
  drawn at end, colours, flash phase medians, end state), radar region of the shot not judged while a zone
  flashes, quantised positions compared with a one-quantum tolerance. DIFF2-D3 superseded (row removed),
  DIFF2-D4 logged (rare GTA SetPosn after RPC 71, original 2/46 runs). Original visual determinism 156/179 →
  173–177/179.
- F3: `veh_enter_B` was not a sampling miss: the original observer in lane A shows `CAR_sit` instead of
  `CAR_sit_pro` for a lane-B driver in every run (3/3 valid orig-vs-orig runs); main had no calibration with
  vehicle steps. xclient.py: several calibration runs (newest 3, desynced ones dropped), expected-miss sets
  from calibration, learned sub-sample hold, `ensure_spawn` recovery + `self_spawn` check for the original's
  occasional missing respawn after death (2/9 runs).
- Runs (main + tools): F1 `rpc-coverage.sh` ×3: exit 1 (surf_move cam rounding; fixed), exit 0, exit 1
  (vehicle_remove_player rare SetPosn; DIFF2-D4) — `--report` of runs 1 and 3 with the final report: exit 0.
  F3 `cross-client.sh` ×4: exit 0, exit 1 (original did not respawn; fixed), exit 0, exit 0 (45/45).
- Negative tests `tools/diff/negative_tests.py f1|f3`: 7 + 4 broken cases, all fail as expected.
- samp-tracer: 49 tests pass, clippy host + i686 clean. Details: docs/phase3/diff-notes.md.

## 2026-11-06 — F4 visual proof closed (worktree visual, branch visual-close)

- One scenario for every element: the FEAT-B steps (`/dl`, `/dl` off, `/testdw`) moved to the end of
  `tools/diff/visual-scenario.sh` (`visual-featb.sh` removed); the chat input still held
  "clip text 1hello" after Escape, so the residual line is sent first (both clients keep it alike).
- vehtag (248 lines): timing. Bot1 was killed without a disconnect and the server dropped it (~14 s
  timeout) inside the capture window: gone after frame 1 on the original, after frame 2 on ours.
  `killbots` now waits for each bot's `[part]` line; the capture waits 8 s after state 1→2.
  Measured with the new `remote_probe` (tracer `remote` + `peek`): remote vehicle at
  1696.000,−2591.1,13.274 on both clients; the samp anchor (A 0x10074240: driver → the samp
  vehicle's matrix position) matches ours.
- netstats: the 2 lines were a glyph-batch split caused by different digits; our F5 printed 0 for
  RWObjects, FreeIDEs, Vehicle Models Loaded, rates, bytes, acks and ShotCompMode 1 (raw InitGame
  value). Implemented samp's values (A 0x10060d20): pool block, GTA tables, lag-comp selector,
  byte-delta rates, RakNet statistics from `samp_proto::connection::Stats` (new). Now e.g.
  RWObjects 406/406, FreeIDEs 3673/3673, ShotCompMode 0/0, messages sent 237/237.
- boot: our plate painter (now implemented) showed up when a vehicle streamed in during the boot
  window; `boot_diff.py` recognises the painter by content on both sides.
- Residual glyph-atlas batch splits (netstats, dloff, testdw; help's 3 lines are the same split,
  run-dependent) covered by the existing decision rows.
- Verified: `tools/diff/visual.sh` exit 0 twice (28/28 OK); negative test on copies of the
  captures (dialog colour, F5 label, missing driver tag, missing element) → exit 1 each;
  workspace tests + clippy (host + i686) 0 warnings.

## 2026-11-06 — Final proof run (second pass, main 43c1a31)

- F5 tools/diff/patch-coverage.sh: exit 0 — `**PASS**: 0 missing, 0 different, 0 unverified.`
- F1 tools/diff/rpc-coverage.sh: exit 0 — `MATCH: 131`, `UNTRIGGERABLE: 10` (logged decision DIFF2-U1).
- F3 tools/diff/cross-client.sh: exit 0 — `45/45 enabled step-directions MATCH, 0 DIFF`.
- F2 tools/diff/network.sh: exit 0 — `checks: 385, matching: 385, accepted decisions: 0, open differences: 0`.
- F6 tools/diff/files.sh: exit 0 — `IDENTICAL`, `PASS`.
- F4 tools/diff/visual.sh: exit 1 — dl/dloff/testdw MISSING on the ORIGINAL side: the run reused an
  original capture made before those elements joined the scenario. Rerun with `--orig` started.
- F7: gungame and simpletdm 2 h PASS; derby 2 h FAIL narrowly (remote_pos 98.83% < 99%, memory slope
  40.4 > 40 MB/h) — a 2 h original-client comparison is running before any decision.

- F4 tools/diff/visual.sh (full run, fresh original capture, glyph-batch merge d7a116b): exit 0 — 28 elements OK, 0 DIFF, 0 MISSING. Negative tests (changed text, changed batch vertex count) exit 1.

## 2026-11-06 — JOIN-P class request held back by the join dialog (worktree joinp, branch request-class)
- Original rule (static, A 0x10006100): after the HUD/cursor part, the class-selection tick runs only while the
  dialog object (A 0x1026e9c8) has "open" (+0x28) = 0; this holds back the first RPC 128 as well as Shift and the
  arrows. The close (A 0x1006f860) clears +0x28 at once; Shift also needs the chat input closed (+0x14e0 = 0).
  No other state (menu, textdraw selection, input lock) holds the first request.
- Measured (tools/e2e/joinburst.sh orig, lane ui, 2/2): no RPC 128 while the dialog is open; RPC 62 in its own
  datagram, RPC 128 4.2 ms later in the next one (5 ms at the server).
- Fix: `ui::dialog_open()` gates `class_selection_tick`, `ui::chat_input_open()` gates the Shift key; the UI's
  queued messages (`ui::send_outgoing`) now go out before the local tick, so RPC 62 precedes RPC 128 like the
  original's window-procedure send. Residual: ours packs 62 and 128 into one datagram (RakNet send timing).
- joinburst.sh: JOIN-P assertions (server-side dialog state + ms since the response, client-leg capture order and
  delta ≤ JB_CLASS_MAX_MS), JB_LANE=ui. Ours: 2/2 (plain + mix-sc:8), 0 ms, same datagram. m1, m2, m8a pass;
  607 tests; clippy 0 host + i686.

## 2026-11-06 — SOAK-3 derby 2 h, original vs ours (worktree soak3, branch soak-final)
- Parallel 2 h derby soaks (original: lane trace, ours: lane veh), same harness, all five cases on both.
- Harness fixes (no threshold changed): seated players compared via their car (server position of a seated
  player is the vehicle's; Bus/Dune seats sit metres from the origin), veh_hp bracketed by two server dumps,
  memory judged from 900 s after a fresh launch (late streaming warm-up on both clients), original's chat read
  from chatlog.txt so it runs the same cases.
- Final: original PASS (remote_pos 99.78 %, judged memory 13.6 / 22.7 MB/h); ours PASS (remote_pos 99.57 %,
  veh_pos 99.64 %, veh_hp 100 %, judged memory 6.0 / 8.4 MB/h, fps 52.4, 0 crashes, all cases OK).
- Found: after the 15 s netloss drop ours opens a reconnect socket at +10.1 s and another at +16.1 s; the
  original one socket at +12.2 s. Once ours reached the server before it had timed the old player out →
  "Unacceptable NickName" → relaunch. Parity task NET-R started (reconnect timeline vs the original).

## 2026-11-06 — NET-R: reconnect after a network gap, original vs ours
- Measured the original (lane trace) and ours (lane veh) in derby with only netloss cases (`SOAK_CASES="netloss …"`,
  drops 15 / 40 / 6 / 15 s, `SOAK_CAPTURE=1`), timelines from the client leg with the new
  `tools/e2e/netloss_timeline.py`. Statics: the open-connection cookie is a process-global with one writer
  (A 0x1003fdc2) and one reader (A 0x100416a5), never reset; request timer "now > last + 1000 ms" (A 0x1004165d).
- Cause of the extra socket: ours reset the cookie per attempt, so the first request answered after the gap only
  fetched a cookie and the attempt failed (second socket 1 s later). Fix: process-global cookie; RakNet's strict
  request timer. Harness: `soak_mon.py` had two `say` methods (log lines were sent as chat — instantly for ours,
  typed for seconds on the original); the chat is now `chat_say`, typed on the game window for both clients.
- Result: same sockets, cookies and outcome as the original in every case (incl. a server cookie reseed); lost
  connection 10.02–10.04 s after the first reliable send in the gap (original 10.04–10.06), residuals logged in
  ROADMAP. Notes: docs/phase3/reconnect-notes.md.

## 2026-11-06 — Soak: orphaned car after a driver quits (gungame 2 h persistent veh_pos)
- Question: the 2 h gungame soak's only persistent desync (veh1 6.26 m off for 32 s after SkDrive braked,
  left the car and quit): client or measurement?
- Static (A 0x100155E0 / 0x10015140, REA): synced velocity written, then nudge Δ·0.06 within 8/8/0.5, teleport
  beyond or when the car is not in the moving list; quitting ped warped out and deleted, car untouched; a car at
  rest sends no 209. Ours matches (`vehicle::correct_driver`).
- Dynamic: same scenario on the original (trace lane) and ours (lane ui): gungame, SkDrive + SkPass restarted every
  45–90 s, 1 s samples (new knobs SOAK_BOTS, SOAK_BOT_SECS; ours now writes client-view.log). Both runs stopped by
  host memory pressure after ~540 s, 5 quits each. Original: car 5.85 / 0.13 / 0.16 / 1.53 / 7.72 m from the
  server's position after the quit, constant; the old harness flagged a persistent veh_pos (7.72 m) and the
  passenger's remote_pos on the original. Ours 0.16–1.17 m in this run, 6.26 m in the 2 h soak. Shared behaviour.
- Harness: orphaned cars (driver left the server while seated, nobody driving) excluded from veh_pos and passenger
  remote_pos until a driver sits in it or the server position changes; no threshold changed. Logged in ROADMAP
  decisions and soak-notes "Orphaned cars". No client code changed.

## 2026-11-06 — Final proofs on main 29fb29b (F1, F2, F4–F7)
- Logs in target/final-proof/ (gitignored). Memory: the host's background-shell reaper stopped the first parallel
  run (three soaks + an agent's two games + 3.3 GB of old scratch files in the /tmp tmpfs); rerun with less load.
- F2 `tools/diff/network.sh`: checks 387, matching 387, accepted 0, open 0 — exit 0.
- F4 `tools/diff/visual.sh`: first run DIFF on dlg5 only (D3DX split one sprite batch of 8 vertices into 4 + 4; the
  sprite.Draw calls were identical) → visual_diff.py merges consecutive D3DX draws with identical state and
  compares the joined vertex lists (negative test: one altered vertex → DIFF, exit 1). Rerun: 28 elements OK, exit 0.
- F5 `tools/diff/patch-coverage.sh`: PASS, 0 missing, 0 different, 0 unverified — exit 0.
- F6 `tools/diff/files.sh`: IDENTICAL, PASS — exit 0.
- F1 `tools/diff/rpc-coverage.sh`: steps 179, RPCs covered 131/141, MATCH 131, UNTRIGGERABLE 10 — exit 0.
- F7 `tools/e2e/soak.sh`, 7200 s per gamemode, one lane each in parallel (gungame veh, derby ui, simpletdm trace):
  gungame PASS (99.996 %), simpletdm PASS (100 %), derby PASS (99.902 %; remote_pos 99.47 %, the rammed-car case
  shared with the original), fps 52.5, 0 crashes, every case ok — each exit 0. A first derby attempt in lane ui
  stayed at gGameState 6 and crashed in GTA's level loader (write to 0x18, gta_sa.exe+0x137d12) after ~160 s
  of loading while the host had ~3 GB available; the rerun joined in 28 s. Not reproduced since; watch item.
- F3 `tools/diff/cross-client.sh`: 45/45 enabled MATCH, exit 0, but 8 step-directions still PENDING (unoccupied /
  trailer, combat hit, objects, chat) although M4–M7 are done → being enabled (F3 not yet counted as proven).

## 2026-11-06 — DIFF-3: unoccupied/trailer, objects, chat, aimed hit enabled (F3 covers PLAN §1)
- The four PENDING groups of `tools/diff/xclient.py` are real steps, both directions (51 steps, 66
  step-directions, 0 pending): `veh_unoccupied_A/B` (+ `veh_reenter_B`), `veh_trucks`, `veh_trailer_A/B`,
  `obj_setup`, `objects`, `chat_A/B`, `bubble`, `hit_setup_A/B`, `combat_hit_A/B`. Checks and the learned
  tolerances: docs/phase3/diff-notes.md "M4–M7 steps".
- New control queries with one reply format on both sides: `bubbles` (samp-tracer reads samp.dll build B's bubble
  pool at 0x1026e9f0; ours its `ChatBubbles`) and `synced` (samp's remote player health/armour +0x1b0/+0x1ac via
  netgame 0x1026ea0c → pools +0x3de → player pool +8; ours `RemotePlayer.health/armour`); offsets from the B build's
  RPC 59 handler / add function and remote-player stores (objdump). Chat lines come from each lane's own
  `chatlog.txt`, objects from GTA's object pool, vehicles/trailers by pointer.
- `xclient.pwn`: truck/trailer, objects, bubble, out commands; vehicle lines in `xc info` (with
  IsVehicleStreamedIn); shot/damage/unoccupied-update logging.
- Measurement fixes (each from original-vs-original evidence, details in diff-notes): aim geometry (free-aim shot
  leaves 2.67° right of the facing angle and rises over the head → turned start + mouse pitch, 3 m gap, own spots
  > 2 m from the previous ones); trucks for both players at once (open.mp streams vehicles on the receiving
  player's own syncs only); trailer window 12 s (rig at rest); bystander of the unoccupied steps 60 m away (open.mp
  refuses a new 209 sender that is not the closest to the stale server position); tolerances learned only from the
  judged step-directions; `sender_vs_original` not judged for an original sender.
- `cross-client.sh` runs from a git worktree (`SAMP_RS_ROOT` for wine/, data/, re/gta, target/diff and the lane
  lock; `lane-lock.sh` locks `$SAMP_RS_ROOT/target/locks/diff.lock`); `XC_STEPS` for development runs.
- Negative tests `negative_tests.py f3` +6 cases (unoccupied car off, no trailer, no head object, no chat line, no
  bubble, our shot misses): all 10 turn MATCH into DIFF.
- No DIFF on our client found by the new steps; no client behaviour changed (only the two control queries added).
- Verified: cargo test --workspace, clippy host + i686 clean; 4 calibration runs (3 used, one skipped: the
  original's known respawn flake); `tools/diff/cross-client.sh` → 66/66 step-directions MATCH, 0 DIFF, 0 pending,
  exit 0 (run xclient-20261106-161102; tolerances pos 0.114 m, uveh 0.167 m, trailer 0.1 m, obj_player 0.437 m /
  0.624 rad). Cost: ~2.5 h wall, ~20 cross-client runs in lane diff.

## 2026-11-06 — CURSOR-1: no mouse cursor with chat input / dialog / scoreboard
- Measured the visible X cursor of the isolated headless display (new `tools/e2e/xcursor.py`, XFixes; gamescope
  screenshots do not contain the cursor) in trace-lane hold sessions: original shows samp's 32×32 arrow with the
  chat input, a dialog and the scoreboard open; ours showed a hidden 1×1 cursor in every state.
- `WINEDEBUG=+cursor`: DXVK 3.1.1 makes the D3D cursor a Win32 cursor (CreateIconIndirect + SetCursor); in ours
  GTA's window procedure had already called `ShowCursor(FALSE)` twice (activation 0x747FE9, `WM_SETCURSOR`
  0x7481CD) → count −2, cursor never visible. The original makes no ShowCursor call: samp stubs
  `user32!ShowCursor` at start-up (B 0x10062930/0x10062920; export seen jumping into samp.dll live). Spec: UR §5.
- Fix: `cursorhook.rs` redirects gta_sa.exe's ShowCursor import to the same counter-neutral stub at attach; the
  crash dialog uses the stub. After the fix: same cursor image in the same states, no NtUserShowCursor calls.
- Verified: cargo test --workspace ok; clippy host + i686-pc-windows-gnu clean; tools/e2e/m8a.sh PASS;
  tools/diff/visual.sh (ours vs the main checkout's original capture) every element OK incl. dlg0–5, input,
  paste, scoreboard. Residuals (device-reset cursor details) in ROADMAP decisions.

## 2026-11-06 — LBL-COL: colour codes in 3D labels and chat bubbles (user report)
- Report: labels with `{RRGGBB}` codes showed in one colour. Static (A build, objdump): the label pool pass
  A 0x10001350 calls the 3D text painter A 0x1006bc90 with the raw text and an outline flag = (alpha == FF);
  the painter's small draw (B 0x1006b4a0) draws the outlines from the stripped text and the main pass from the
  raw text through the colour-code wrapper. Bubbles use the same painter (outline 1, LOS 0). A 0x100b6140 on
  RPC 36 is the `~k~` key-name filter (< 400 chars, kept if ≤ 399), not a colour pass.
- Fix (samp-client `ui/render.rs`): shared `text3d` painter for labels and bubbles (raw text through the
  wrapper, outline only for alpha FF labels, bubble rect bottom y+1); `entities.rs` runs the label `~k~`
  expansion (`samp_ui::text::expand_key_names_limit`, limit 399; `GameExpander` now crate-visible).
- Harness (F4): scenario `visual_labels` (7 world labels: several colours, code at start / mid-word, invalid
  `{GGGGGG}` / `{12345}`, lower-case hex, double code, alpha 80 / 60 with and without codes, `~k~`, a 465-char
  label; then a bot with a coloured alpha-80 bubble and a player-attached label); elements `label3d`,
  `bubblecol` (geo + new `glyphs` option: per-glyph colours compared); `BOTCOUNT` for the bot index; geo merge
  fix (D3DX draw counts not re-added as 0).
- Verified: `tools/diff/visual.sh --orig` (fresh original capture) then `tools/diff/visual.sh`: exit 0, 30/30
  elements OK (dl: 2 lines covered by the glyph-atlas decision); cargo test --workspace; clippy host + i686
  clean; tools/e2e/m6.sh in lane ui. Notes: docs/phase3/diff-notes.md "DIFF-4: 3D labels and bubbles".

## 2026-11-06 — Remote driver nametag position (user report: passenger sees the driver's tag "below the vehicle")
- Static (REA, A 0x10074270 / 0x10075390): the vehicle-position anchor is taken only when A 0x100b7e40 says the
  remote's vehicle is an RC model (441/464/465/501/564/594); every other driver is tagged at bone 8 like a
  pedestrian. Ours anchored every driver at the vehicle's matrix position.
- Tooling: tracer `xform` command (last device VIEW, last perspective PROJECTION, viewport, full precision);
  m8afs `campos`/`camlook`; `tools/diff/vehtag-measure.sh` (scenario for visual.sh: samp-bot drives an Infernus,
  fixed cameras 4/8/16 m, then local player as passenger), `tools/diff/vehtag_eval.py` (bar origins, anchor
  height recovered from the bar Y; `--check ORIG OURS`), `tools/diff/vehtag.sh` (runs both + check).
- Original (trace lane): samp's anchor global A 0x10140bdc = driver's head (z 13.569; ped root 13.065, car
  13.274); bars passenger 581,327, foot 780,317 / 715,325 / 695,327. Ours before: passenger 639,352, foot
  839,376 / 774,352 / 690,340 (check FAIL 5/5). Fix `vehicle::tag_at_vehicle` → ours after: passenger 582,327,
  foot 792,318 / 732,325 / 717,327; recovered anchor +0.50…+0.52 m above the ped root vs original
  +0.43…+0.52 (check PASS 5/5). X differs with the car's lateral rocking (ROADMAP decision VEHTAG).
- Cost: 3 original runs (one wasted on a wrong bot id, one on the ortho projection), 4 ours runs (one lost to a
  gamescope/Xwayland restart under memory pressure).

## 2026-11-06 — CRASH-F3: our client crashed in the F3 truck/trailer steps (heap corruption from a skin change)
- Crash of run `xclient-20261106-161806` (+251 s, read of 0x38C at gta_sa.exe 0x6D321C): mapped with the same
  DLL's symbols to `Vehicles::stream_in` → `Runner::run` → opcode 0x07FF → `AddVehicleUpgrade` 0x6DF930 →
  `ClearVehicleUpgradeFlags` 0x6D3210 on a NULL vehicle; it was the 11th guarded fault (samp's filter handles 10).
  The cascade started 20 s earlier with a guarded fault in `CGame::Process` (streaming eviction → `RpMaterialDestroy`
  0x74DA25 on a garbage material pointer); afterwards no model loaded, `busy_load` timed out and CREATE_CAR / the
  following opcodes ran on an unloaded model. Trailer RPCs 148/149 match the original (B 0x1001AE00 / 0x100B7C20).
- Root cause (temporary per-frame RW / anim checks and a quarantine of freed `CAnimBlendAssociation`s, removed
  again): `handsup` → `handsup_off` → `skin` leaves the hands-up task holding the old clump's association that the
  skin change frees; the death later writes into the freed block. Measured on the original in the trace lane: the
  same dangling pointer exists there (task kept, association intact, no longer listed). Details:
  docs/phase3/m5-notes.md "CRASH-F3", spec docs/re/subsystems/player-state.md RPC 153.
- Fix: `pedanims.rs` (task-held associations are unlinked from the clump list before `DeleteRwObject`, so they stay
  valid; 4 unit tests), `busy_load` failure → no vehicle created (stream-in, PlayCrimeReport), guarded faults log
  registers + return-address candidates. ROADMAP decision "CRASH-F3". `tools/trace/uifs.pwn`: `pskin|P|skin`.
  New reproduction `tools/e2e/skin-death-trailers.sh [ours|orig] [ROUNDS]` (trace lane).
- Verified: cargo test --workspace (all ok, +4 pedanims tests), clippy host --all-targets + i686 clean;
  `tools/e2e/m5.sh` (lane veh) PASS; `tools/e2e/skin-death-trailers.sh ours` PASS 3/3 and 4/4 rounds (0 faults),
  `… orig 3` PASS (its probe shows the dangling pointer reused); `tools/diff/cross-client.sh` with the final build:
  66/66 exit 0 in `xclient-20261106-182119` and `-184745`; the other runs failed through the ORIGINAL only — its
  respawn flake (`-183649`), physics/sender variance (`-181604`, 63/66) and twice (`-183028`, `-184221`) the
  original froze in its endless wait for model 403 at `veh_trucks`, i.e. the same corruption hitting samp.dll.
  Suggestion for main: run `skin` before `handsup` in xclient.py (needs new calibration runs) so the scenario stops
  triggering samp's own use-after-free. Cost: ~5 h wall, ~25 cross-client runs, 4 trace-lane sessions.

## 2026-11-06 — MATTEXT-1: object material text (user report: "Romania Super Stunt" board on samp.rss-ro.com)
- Report: the original draws the board's text large, bold, coloured over the board's light background; ours drew it
  much smaller, left, on black.
- Static (samp.dll B, REA/Ghidra): kind-2 record 0x100A86C0, object pass 0x100A9EF0, renderer 0x100708E0, size table
  0x100A8C90, kind-1 record 0x100A8540. Spec: world-entities.md §44 kind 2 (every field; render target = material
  size; A8R8G8B8 + SetRenderState(DESTBLEND, BLENDFACTOR) when the background is 0; DrawText 0x100 / 0x105 / 0x102;
  the slot gets a fresh 2×2 RW texture, filter mode + 1 = LINEAR, colour cleared; the device proxy swaps that
  texture for the render target). trace-notes.md: the earlier "twice the material size" reading corrected.
- Root causes in ours: render targets twice the material size (text half as large), always X8R8G8B8 (transparent
  background → opaque black, hiding the board behind), DT_VCENTER on left/right, substitution of the model's own
  texture instead of a per-slot texture, point filtering (found by the new `mattextw` element), plus one sprite
  created per fill (samp keeps one).
- Fix: `ui/mattext.rs` (sizes, format, blend state, formats per alignment, persistent sprite, key = 2×2 texture,
  unchanged slots keep their render target), `objects.rs` (kind 2 releases the slot texture, creates the 2×2 RW
  texture with LINEAR filter, clears the colour; kind 1 on a text slot drops the text; one `samp-rs.log` line per
  material record with every field), `worldhooks.rs` (substitution pairs from mattext), `worldents.rs`
  (`material_log_line` + test).
- Harness (F4): `visual-scenario.sh` `visual_mattext` + m8afs `mtscene` / `mtmat` (14 boards: sizes 10..140, five
  fonts, font sizes 20..80, bold 0/1, alignment 0/1/2, backgrounds 0 / opaque / half alpha, colour codes, newlines,
  texture + text on the same / different indices; global objects → RPC 44, player objects → RPC 84);
  `visual_diff.py` kinds `rt` (all render-target fills across the capture) and `rtuse` (render targets bound +
  sampler state on the draws); `visual.sh` `VISUAL_OURS_LANE` / `VISUAL_OURS_PORT|_CTL|_TPORT`.
- Verified: scenario-only runs (orig trace lane `orig-20261106-222919`, ours lane diff): `mattext` 0, `mattextw` 0
  differing lines (13 fills, 568 normalised lines each; 28 bind/draw lines); screenshots differ only in the chat line
  naming the server port. cargo test --workspace ok, clippy host + i686 clean, tools/e2e/m6.sh PASS (lane veh),
  full `tools/diff/visual.sh --orig` exit 0 (32 elements OK incl. mattext / mattextw; captures in the worktree's
  target/visual — the main checkout needs `--orig` once for the new elements).

## 2026-11-06 — GAP-UI: class sounds, device-loss cursor, /save + /rs, RPC 92 / 150
- Static (samp.dll A, objdump): arrows A 0x10005ff0 / 0x10006080 (ped matrix → sound 1052/1053 via A 0x100a2300 →
  RPC 128), /save A 0x10068a00 and /rs A 0x10068b80 (same values: matrix position + 0x0172 / 0x0174), lost A 0x100c3f90
  (closes chat input and scoreboard, re-arms the WM_SETCURSOR flag), reset A 0x100c41f0 (SetCursor(NULL) … mode 2 → 0 +
  restore unless 0xBA67A4), RPC 92 / 150 A 0x10018c50 / 0x10018d00.
- Harness: `tools/diff/gapscen.sh` (NET_SCEN for network.sh; GAP_ONLY=cursor), `lossproxy.py` rule `rpc-sc ID`
  (renames S→C RPC 35 so 92 / 150 can be driven from Pawn), `netcap.decode_datagram` reports each message's byte offset.
- Fix: `classsel.rs` (ChangeClass action with the sound), `client.rs` (sound then RPC 128; RPC 92 / 150 handlers),
  `script.rs` (0x03FD const), `ui/glue.rs` (`LiveGame::position` via GET_CHAR/CAR_HEADING; reset hook: lost-device closes
  + flag re-arm, reset-device SetCursor(NULL) and mode 2 → 0 + restore), `ui/wndproc.rs` (on_lost_device /
  on_reset_device), `vehicles.rs` (`pool_ref` pub(crate)).
- Measured both clients (trace lane / lane ui with the tracer pre-loaded): class-sound scm, 0x052C/0x03FD, saved lines,
  heading opcodes and all ten X-cursor readings identical; our first run still showed the cursor after
  the loss (chat input not closed) — fixed and re-measured. Corrected trace-notes (/rs on foot = ped position).
- ROADMAP decisions: GAP-UI residuals, CDSTREAM-1 (our CdStreamSync lost-wakeup fix, an addition samp lacks).
- Verified: cargo test --workspace ok; clippy host + i686 clean; tools/e2e/m8a.sh PASS and tools/e2e/m10.sh PASS (lane ui,
  under the lock). `tools/diff/rpc-coverage.sh` cannot run from a git worktree (it takes wine/ and the lane game dir
  from the checkout root, not SAMP_RS_ROOT); it has no RPC 128 / arrow step and lists 92 / 150 as untriggerable, so the
  per-RPC evidence for these items is the gap scenario's tracer scm comparison above.

## 2026-11-07 — Audit gaps in vehicles/combat: object hits, surf damage rows, enter gate, RC driver, mod shop, 570
- Static (REA/Ghidra, samp.dll A; B where the trace lane runs it): 206 send A 0x10006BC0 → object pool lookup
  A 0x10012A10 (ids 1..=highest, wrapper +0x40); damage decision A 0x100A3BF0 rows 3/4 (+0x2DA = surfed vehicle **or
  object** wrapper); driver-enter gate A 0x100A3700 (its "surf" checks read the local **ped object** +0x2DE / +0x2E2 =
  cuffed / carry, set by special actions 24 / 25 — the spec and the audit premise were wrong); RC driver: ped wrapper
  vtable slot 2 = A 0x1009F460 (`CWorld::Remove` when in the moving list) from the per-frame driver branch; RPC 96
  type 4 = A 0x10001A60 case 4 (spec written in vehicles.md 96); 570 passenger warp A 0x100AC450 (SCM 0x0430, seat −1).
- Code: `combat.rs` (`object_of_entity`, `surf_inputs`), `combathooks.rs` (hit type 3, `Shared::{objects, surface}`),
  `combatclient.rs` / `client.rs` (`update_combat_shared`), `vehhooks.rs` (gate: cuffed / carrying, logged),
  `vehicle.rs` (`driver_enter_refused`, `passenger_enter_warps`, `Seat`, `mod_shop_follow*`), `vehicles.rs` (RC
  `remove_from_world`, `mod_shop_follow`, 570 warp, header). 4 new unit-test functions.
- Tools: `tools/diff/combatveh.sh` (+ `combatveh-measure.sh`, `combatveh_eval.py`): original (trace lane) vs ours
  (lane ui) — object shot, RC driver links, six enter-gate cases (surf car / beside / surf platform / cuffed / carry /
  free), mod-shop follow, 570 enter. `samp-bot --modshop A,S,I`; m8afs `surf|P` + OnPlayerInteriorChange log;
  `tools/e2e/m7.sh` object step; `tools/e2e/m5.sh` finds the auth table through SAMP_RS_ROOT (worktrees).
- Parity loop: 5 combatveh rounds. Round 1–2: obj/RC/570 equal, harness fixes (peek limit). Round 3: platform
  case showed ours refusing where the original enters → server surf query proved the original *was* surfing → the
  gate reads cuffed/carry → fixed. Final round PASS (all items equal).
- Verified: `cargo test --workspace` 618 passed / 0 failed; clippy host `--all-targets` and i686 0 warnings;
  `tools/diff/combatveh.sh` PASS; `tools/e2e/m7.sh` (lane veh) PASS (modes 1 2 0; object step: 2/2 hit type 3 hitid 1
  in modes 1/2, none in mode 0); `tools/e2e/m5.sh` (lane veh) PASS; `tools/diff/cross-client.sh --skip-build`
  (`xclient-20261107-005834`) 66/66 MATCH, exit 0. Cost: ~2.5 h wall, 6 combatveh rounds (original + ours), 1 m7,
  2 m5 (first failed on the worktree auth-table path), 1 cross-client.

## 2026-11-07 — M8A-PREVIEW: model-preview textdraws; class-selection panel with a dialog
- User report (real server, class selection with a language dialog): the original draws a model-preview
  textdraw (font 5), ours nothing; ours draws the `<<` `>>` `Spawn` panel over the dialog, the original not.
- Static (REA/objdump, A build): preview dispatch A 0x100b34c0/0x100b34e0, renderers A 0x1006c140 (ped),
  0x1006c3c0 (vehicle), 0x1006c9b0 (object), camera set A 0x1006be40, frees A 0x100b2f90 / lost device
  A 0x100c41e5; class step A 0x10006100 (panel + first RPC 128 only on a frame without a dialog), RPC 124
  B 0x100041A0. Spec: UR §14.3a, §4.2.
- Dynamic (trace lane, original; lane ui, ours; `tools/diff/visual.sh` with a subset scenario): ped / vehicle /
  object / several (incl. a player textdraw, a never-rendered id, the 18631 fallback) / rotation update / model
  change / device reset (new tracer `d3d devreset N`) / destroy-hide; class selection entered with a dialog open
  (m8afs `unspec`). Found on the way: model id 20000 crashes the original; ours spawned instead of entering the
  class selection on RPC 124 off with the class-selection flag; plate `SetTexture` must tail-jump like samp; our
  Reset hook could not release the UI resources while a preview render held the UI lock (device reset failed).
- Implemented: `ui/preview.rs` (new), `ui/textdraws.rs`, `ui/glue.rs`, `ui/d3d.rs`, `players.rs`, `vehicles.rs`,
  `worldhooks.rs`, `client.rs`, `samp-ui` textdraw serial (+1 test), tracer `d3d devreset`, m8afs commands,
  F4 elements pv*/mdlpv*/classdlg* (`visual_diff.py` kinds `pvrt`, `preview`). Notes: docs/phase3/m8a-notes.md;
  decisions in ROADMAP.
- Verified: subset F4 (pv*, mdlpv*, classdlg*) 0 differing lines on all 14 elements; preview screen regions
  pixel-identical to the original; cargo test --workspace ok; clippy host + i686 clean; full F4 and m8a below.
- Full F4 (`tools/diff/visual.sh --orig`) found two more: the chat GUI dialog size after a device Reset (ours reset
  it to the video size; fixed, `render.rs`) and SPECULARENABLE toggles in the 411 preview after earlier world
  Infernus renders (ROADMAP decision + `visual-decisions.tsv` rows for pvveh/pvrot/pvreset); `pvmany` now spaces its
  RPCs (two previews in one frame share one scene, network-timing dependent).
- Final: `tools/diff/visual.sh --orig` rc=0 (44 elements OK; pvveh/pvrot/pvreset 4 lines each covered by the
  specular decision, dl 2 by the glyph-atlas decision); `tools/e2e/m8a.sh` PASS (lane ui); cargo test --workspace
  615 passed; clippy host --all-targets and i686 --workspace clean. Cost: ~6 h wall, 6 original + 5 own game runs.
- Rebased onto ed04dcc (with 244cb98 material text, ebf77bf, ed04dcc): the preview kind is now `pvrt` (main's `rt`
  is the material-text kind) and only counts RenderWare render targets / non-D3DX 256x256 textures; scenario order
  visual_mattext → visual_previews → visual_classdlg. On the rebased tree: `tools/diff/visual.sh --orig` rc=0, all
  46 elements OK (dl 2 lines covered; the 411 specular lines no longer occur); m8a PASS; cargo test 620 passed;
  clippy host + i686 clean.

## 2026-11-07 — F1 regressions on 67eba42: velocity_up / skin_101
- Report: F1 on 67eba42 (ours-20261107-022425 vs 4 fresh original runs; no older run reused because the tracer was
  rebuilt) had two DIFFs that matched on 244cb98: `velocity_up` (no fall damage in ours, health 50 vs 45) and
  `skin_101` (no REQUEST_MODEL 101 in ours). Suspects were ed04dcc (surf damage rows) and 67eba42 (model previews).
- velocity_up: GTA never built the damage calculator in ours (`CPedDamageResponseCalculator` ctor from GTA 0x5E303E
  absent), and that happens before the `ComputeDamageResponse` hook (0x4B5AC0) the surf rows live in, so the surf
  rows cannot be involved. Static re-check of A 0x100A3BF0 (rows 3/4: +0x2DA set and wrapper +0x40 == damager or
  weapon 0x36) matches `combat::decide`. The original lands without fall damage in 3 of 10 complete runs on
  record, with the same trajectory as ours. Decision DIFF2-D5 (rows in `tools/diff/rpc-decisions.tsv`).
- skin_101: both clients load the skin only when `HAS_MODEL_LOADED` fails; GTA's ambient ped streaming holds a
  random set of civilian models by step 17 (per-step readout of `ms_aInfoForModel` load states, debug-only, 3
  original + 4 ours subset runs: different sets in every run, all 7 requested 101), and original run
  orig-20261106-231135 has ours' exact step trace. Decision DIFF2-D6.
- Harness: `rpc-coverage.sh` runs from a git worktree (wine/, data/, target/diff from SAMP_RS_ROOT; `DIFF_OUT`
  override for subset runs); `tools/trace/diff.py` finds the opcode-name enum under SAMP_RS_ROOT.
- Verified: report-only on the failing F1 data with the new rows: exit 0, MATCH 131 (velocity_up and skin_101
  MATCH through D5/D6); subset `rpc-coverage.sh --orig-runs 3 --ours-runs 1 --only 'velocity_up|skin_101'`
  (DIFF_OUT=worktree target/diff-steps): both steps MATCH without any accepted line; cargo test --workspace 620
  passed; clippy host --all-targets and i686 -D warnings clean; tools/e2e/m7.sh (lane ui) PASS, modes 1 2 0.
- F2 follow-up (209 unoccupied sync, coordinator request): no regression — ours sent no 209 in 7 of 8 full F2
  sessions already before ed04dcc; the 209 path is unchanged by ed04dcc/67eba42 and its gates match A 0x1001F3C0 /
  A 0x10006E00 (re-read). Focused `network.sh` runs with a 4-round create/passenger script (alone and with the drive
  round first): entry 209 in 16/16 rounds on both clients, settle pairs 2/8 vs 1/8, exit catch 2/8 vs 1/8, 209
  totals 17 vs 14. Notes: docs/phase3/net6-notes.md §5. combatveh.sh PASS (surf rows, gate, RC, mod shop, 570).
- Full F1 from the worktree (`SAMP_RS_ROOT=~/samp-rs tools/diff/rpc-coverage.sh`): first run (5 usable original runs,
  one original failed to join; ours-20261107-051026) exit 1 on `vehicle_put_player` only: the 411 dropped 5 m with
  the player warped in and landed with yaw 1.75 rad, the original 1.60 (14 of 15 runs; 1.65 once), earlier ours
  runs 1.60 ×3 / 1.55 — landing physics, same calls in the trace, velocity_up and skin_101 MATCH. Second run
  (`--orig-runs 1 --ours-runs 1`, reusing those 5 original runs; ours-20261107-054758): exit 0, MATCH 131,
  UNTRIGGERABLE 10, only DIFF2-D1 accepted; report in ~/samp-rs/target/diff/rpc-coverage.md. Cost: ~2.5 h wall,
  ~11 original + 9 own game runs (full and subset), 2 focused F2 pairs, 1 combatveh, 1 m7.

## 2026-11-07 — Final proofs on main 67eba42 (all of F1–F7)
- Code under test: 67eba42 (material text, the user's real-server reports, the audit's class-A gaps, model
  previews); 61b9470 on top adds only F1 decision rows (DIFF2-D5/D6) and docs. Binaries built once beforehand.
- F7 `tools/e2e/soak.sh`, 7200 s per gamemode in parallel (gungame veh, derby ui, simpletdm trace): gungame PASS
  (99.978 %), derby PASS (99.904 %), simpletdm PASS (99.997 %), fps min 52.3, 0 crashes, every case ok — each exit 0.
- F3 `tools/diff/cross-client.sh`: 66/66 MATCH, 0 DIFF, 0 pending — exit 0.
- F4 `tools/diff/visual.sh` (original capture of the 46-element scenario): every element OK — exit 0.
- F5 `tools/diff/patch-coverage.sh`: PASS, 0 missing / different / unverified — exit 0.
- F6 `tools/diff/files.sh`: IDENTICAL, PASS — exit 0.
- F2 `tools/diff/network.sh`: first run 386/387 (session total of unoccupied sync 209: original 3, ours 0);
  rerun 385/385 — exit 0. Analysis (net6-notes §5): the 209s are physics-driven settles/catches, ours has the same
  gates as samp (A 0x1001F3C0 / 0x100B74A0 / 0x10006E00) and emits them less often (focused 4-round scenario:
  original 17, ours 14); left as the open NET6-P3 settling residual, no tolerance row added.
- F1 `tools/diff/rpc-coverage.sh`: the batch run had velocity_up (fall damage) and skin_101 (model request) DIFF;
  both are original-side variance (3 of 10 original runs land without damage; 101 already streamed in 1 of 10
  original runs) → DIFF2-D5 / DIFF2-D6. Report on the final runs (6 original, ours-20261107-054758) with main's
  tooling: MATCH 131, UNTRIGGERABLE 10 — exit 0.
- cargo test 620 passed, clippy 0 (host, i686).
