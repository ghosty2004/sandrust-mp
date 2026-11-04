# DIFF-2 — per-RPC reaction diff (criterion F1)

Goal (PLAN.md §1 F1): for every S→C RPC the R4 client registers (141 ids,
`docs/re/rpc-ids.md`), our client reacts like the original: same GTA call
sequence, same resulting game state, same visible result. The harness fires
one RPC at a time at a joined, spawned player and compares, per step, what the
original client did with what ours did.

## Running it

```
tools/diff/rpc-coverage.sh                    # build, original ×2, ours ×1, report
tools/diff/rpc-coverage.sh --orig-runs 1 --ours-runs 1 --only 'weapon|money' --skip-build
tools/diff/rpc-coverage.sh --report --orig target/diff/runs/orig-A --orig target/diff/runs/orig-B \
    --ours target/diff/runs/ours-C            # report only, from existing runs
```

* Output: `target/diff/rpc-coverage.md` (human), `target/diff/rpc-coverage.json`
  (machine-readable: totals, per-RPC verdicts, per-step details), runs in
  `target/diff/runs/<client>-<stamp>/` (`steps.jsonl`, `step-list.json`,
  `trace.jsonl`, `shots/*.jpg`, `server.log`, `chatlog.txt`, `binaries.sha256`),
  `target/diff/{orig,ours}-latest`.
* Exit 0 only if ours ran and every covered RPC is MATCH (DIFF, NOT-IMPLEMENTED,
  NOT-FIRED and NOT-RUN fail). A summary is printed either way.
* `--only REGEX` limits the fired steps (the idle steps always run). The report
  then marks the RPCs of skipped steps NOT-RUN.
* One full run is about 12–14 minutes (176 steps). The default (3 runs) takes
  about 40 minutes.

## Lane "diff" (ENV-4)

`WINEPREFIX=wine/prefix-diff`, `GTA_DIR=wine/gta-sa-diff` (only top-level files are
replaced, with rm + cp; never through the hard-linked subdirectories), `E2E_TAG=diff`,
server UDP 7795 in `target/e2e-server-diff`, control ports 47705 (our client,
`SAMP_RS_CTL_PORT`) and 47706 (tracer, `SAMP_TRACER_PORT`), headless gamescope via
`e2e_headless_run`. Cleanup is `wineserver -k` with the diff prefix only, plus the server
stopped by its exact binary path. The harness never calls `e2e_cleanup` and never uses
`pkill`.

Binaries are built into `target/diff-cargo` (`CARGO_TARGET_DIR`), so the build never
touches the other lanes' artifacts. `--skip-build` reuses the last build.
`binaries.sha256` in each run records exactly what ran.

## Pieces

| file | role |
|---|---|
| `tools/diff/diffbase.pwn` | Main gamemode for the lane. One class (skin 60) with a fixed spawn at LV (1958.38, 1343.16, 15.37, 270°). No timers and no randomness. |
| `tools/diff/rpcfire.pwn` | Filterscript holding the step table. `diff list` prints the steps; `diff fire <name> ;` runs one step and prints `[diff] fire <rpcid> <name> step=<idx>`. Each step is one native call with fixed arguments (some steps also need a setup call that sends no RPC). |
| `tools/diff/drive.py` | Runs the scenario inside the headless display. It drives the game over RCON (SA:MP query packet `x`, `rcon.enable` in the lane config only), tracer `mark`s, control-channel `state`/`peek`/`net`, `gamescopectl` screenshots and `xdotool` keys. |
| `tools/diff/gtastate.py` | Extended state snapshot through `peek`. It reads GTA 1.0 US globals named via plugin-sdk: money, clock, weather, gravity, pad lock, widescreen, camera mode/position, HUD flags, weapon skills, the ped's model/pos/heading/health/armour/state/area/vehicle/weapons/fighting style/drunkenness/wanted level, the peds, vehicles and objects within 120 m (from the CPools byte maps), active pickups and radar blips. |
| `tools/diff/rpc_report.py` | Compares the runs and writes the report. |
| `tools/diff/rpc-coverage.sh` | Lane setup, build, server, runs, report. |

Lane server config, set on top of `e2e_server_prepare`:
* `main_scripts = ["diffbase 1"]`, side scripts e2eprobe + rpcfire;
* `rcon.enable` with a random per-run password;
* `max_bots = 2`, for the open.mp NPC;
* `network.time_sync_rate = 1 day`. RPC 60 is then sent only at connect, instead of
  landing in a random step every 30 s.

## Step sequence

176 steps in this order:
1. flow_join and flow_spawn: the join and spawn flow, with marks around it;
2. 2 idle steps (baseline);
3. local player;
4. UI (messages, game text, the 6 dialog styles, textdraws, 3D label, gang zone, map
   icon, checkpoints, menu, audio stream, client check, explosion, camera target);
5. camera;
6. per-player objects (including edit/select/material/attach);
7. per-player pickup;
8. actor;
9. vehicles (params, plate, health, damage, colour/component SCM events, angle, position,
   interior, put in, velocity, remote collisions, remove, truck + trailer attach/detach,
   destroy);
10. remote player: an open.mp NPC (`NPC_Create`/`NPC_Spawn` → 137/32, chat bubble,
    name tag, colour, put in vehicle → 26, exit → 154, spectate player/vehicle →
    124/126/127, `NPC_Kill` → 166, other virtual world → 163, destroy → 138);
11. the destructive steps last: `SpawnPlayer` (129), `RemoveBuildingForPlayer`, spawn
    info, force class selection, death → class selection (128), Shift → 129, Tab
    scoreboard (155, answered to the client's request), `GameModeExit` (40, then 139 on
    re-init).

Using NPCs instead of samp-bot keeps the remote player deterministic and server-side, and
needs no third process.

**Coverage: 131 of the 141 registered S→C ids are triggered.** Not triggerable from Pawn on
open.mp 1.6.7, because no open.mp code sends them:
* 64, 92, 98, 102, 125, 150, 151, 161, 169;
* 130 (connection rejected), which only happens on a bad join.

RPC 60 is only covered inside flow_join.

## Comparison

Per step, three components:
* **trace**: the tracer's records between `step N name begin` and `… end`, normalised by
  `tools/trace/diff.py` (pointers, timing and caller offsets dropped, floats rounded,
  opcodes named).
  * Only records the client caused are kept: client caller, client return address on the
    stack (`origin`), client script command (`scm`/`via scm`), and `watch` diffs.
  * Background is removed: lines in ≥ 50 % of all steps, lines of the idle steps, lines
    already running in the gap just before the step (per-frame polls), and lines in both
    neighbouring steps (samp's periodic per-vehicle/per-player polls).
  * The rest is compared as an ordered list of unique lines.
* **state**: `state` (from the client's own control port; the tracer serves it for the
  original) plus the `gtastate` snapshot, taken before and after the step.
  * Keys that changed on either side must end at the same value.
  * Entity lists compare the added/removed items, without positions.
* **visual**: a 32×18 grid of the screenshot change. The step's shot is compared with the
  previous step's shot, and cells with mean |Δ| > 18 count as changed. The two clients'
  masks may differ in ≤ 6 % of the cells.

Determinism: with ≥ 2 original runs, the report first compares the original runs with
each other.
* Trace lines not present in all original runs are noise. Ours must contain the stable
  lines, in order, and nothing outside the union.
* State keys that differ between original runs are ignored.
* The visual check is skipped for steps whose original screenshots disagree.

Verdicts:
* MATCH;
* NOT-IMPLEMENTED: none of the step's RPCs has a match arm in samp-client (static scan of
  samp-proto payload names), or ours shows no reaction at all where the original does;
* DIFF: anything else, including "our client is not running any more";
* NOT-FIRED: a scenario error.

Per RPC, the worst verdict over the steps that cover it.

## Baseline (2026-11-04)

(filled in below after the baseline run)

## Open problems

* The trace only shows hooked GTA functions plus samp's script commands. Effects done
  with direct calls to unhooked functions are visible only through state and screenshots:
  * clock, weather, money (CPlayerInfo write), explosions (`CExplosion::AddExplosion`),
    pickups (`CPickups::GenerateNewOne`), blips, audio (`CAudioEngine`), HUD/text;
  * they would need additive tracer hooks. Not done here, because TRACE-2 owns
    samp-tracer right now.
* Many UI reactions (chat, dialogs, textdraws, 3D labels, gang zones, game text,
  scoreboard, nametags) are drawn by samp itself. Only the screenshot check covers them.
  The 32×18 change mask is coarse; DIFF-4 (per-element visual diff) is the real check.
* `peek` reply formats differ: samp-client prints one hex run, the tracer separates bytes
  with spaces. `gtastate.Peeker` accepts both.
* The RCON text reaching `OnRconCommand` through the query port has stale bytes after
  its real length. drive.py terminates commands with ` ;`, and rpcfire takes only the
  leading `[a-z0-9_]` run.
* Steps run in one session, so a missing reaction in ours can change later steps. For
  example, if ours does not teleport, every later position differs. The delta-based state
  comparison limits this, but does not remove it.
