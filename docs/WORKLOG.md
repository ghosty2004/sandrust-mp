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
  removed by SP-OFF. User asked about libsamp (R5) → EXT-1; needs the R5 installer.
