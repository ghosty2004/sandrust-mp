# Master plan — rebuilding the SA:MP 0.3.7 client 1:1

This is the end-to-end plan Claude follows autonomously. `docs/ROADMAP.md` tracks
the live status of each item; `docs/WORKLOG.md` records what happened.

## 1. Definition of done ("1:1")

The project is finished when all of the following hold, each proven by a script
that exits 0 and whose output is recorded in WORKLOG:

| # | Criterion | Proof |
|---|---|---|
| F1 | Every S→C RPC (141 registered by the R4 client) produces the same client reaction as the original: same GTA call sequence, same resulting game state, same visible result | `tools/diff/rpc-coverage.sh` (DIFF-2): per-RPC trace diff + `state` diff + screenshot diff, zero unexplained differences |
| F2 | Every C→S message (29 RPC ids + sync packets 200–212) has the same format, reliability, channel and trigger conditions as the original in scripted scenarios | `tools/diff/network.sh`: capture-diff of original vs ours per scenario |
| F3 | Cross-client: the original client and ours on one server see each other identically (on foot, vehicles, passengers, combat, objects, chat) | `tools/diff/cross-client.sh` (DIFF-3) |
| F4 | UI matches visually (chat, input, dialogs of every style, textdraws, nametags, scoreboard, game text, loading screen) | `tools/diff/visual.sh` (DIFF-4): screenshot diff under a per-element threshold |
| F5 | Every row of `docs/re/data/gta-patches-r4-annotated.csv` is applied or explicitly justified (bug not reproduced / not applicable) | `tools/diff/patch-coverage.sh`: CSV vs patch table |
| F6 | Data parity: the same files are served from the user's samp.saa as the original does (D-4 = A) | trace diff of file opens + byte comparison |
| F7 | Stability: 2-hour soak on each bundled gamemode (gungame, derby, simpletdm) plus scripted reconnect/kick/restart cases, no crash, no desync | `tools/e2e/soak.sh` |
| F8 | Multi-version: every client build we own (currently R4 2019/2021; later R1/R3/R5/DL when provided) passes F1–F2 | per-version profile + runs |
| F9 | Windows validation: F1–F4 smoke subset passes on real Windows | user-run, `docs/windows-validation.md` |

## 2. Phases and milestones

Each milestone uses the standard loop in §3 and must keep all earlier e2e tests green.

| Phase | Milestone | Scope (summary) | Acceptance (all must exit 0) | Depends on |
|---|---|---|---|---|
| A — Foundations | M1 ✔, M2 ✔, M2-P | boot, join, spawn, on-foot sync, spawn parity | `e2e/m1.sh`, `e2e/m2.sh`, trace diff spawn | — |
| A | M3 | remote players on foot (Players slots, swap hooks, interpolation) | `e2e/m3.sh`, `e2e/goal.sh`, trace diff stream-in vs TRACE-2 | M2 |
| A | SAA-1/2 | samp.saa reader + same 23 redirected files | trace diff of file opens; m1–m3 green | RE-9, M3 |
| A | UX-1 | SA:MP loading screen from samp.saa | screenshot diff during boot | SAA-2 |
| B — UI & chat | RE-5 | UI rendering specs (chat, input, dialogs, textdraws, nametags, scoreboard, fonts) | spec + coverage critic | — |
| B | M4 | chat window, input box, local commands (/q, /save, /pagesize, /fpslimit, /timestamp…), game text, client messages, chat bubbles | `e2e/m4.sh` + visual diff + network diff for chat/commands | RE-5 |
| B | M8a | dialogs (all 6 styles), textdraws (+ selection), menus, scoreboard (Tab), nametags + health/armour bars, death list | `e2e/m8a.sh` + visual diff | M4 |
| C — Vehicles | M5 | vehicle streaming, local driving sync (200), passengers (211), unoccupied (209), trailers (210), trains, enter/exit, damage, mods/paint/plates | `e2e/m5.sh` + network diff + cross-client vehicles | M3 |
| D — World | M6 | objects (incl. SAMP.img models, materials, material text, attachments, edit/select), pickups, actors, checkpoints/race CPs, gang zones, map icons, 3D labels, building removal | `e2e/m6.sh` + per-RPC diffs | SAA-2 |
| E — Combat | M7 | aim (203), weapons (204), bullets (206), damage (115), death/respawn (53/166), lag-comp modes | `e2e/m7.sh` + cross-client combat | M3, M5 |
| F — Remaining behaviour | M8b | spectating, camera RPCs, widescreen, world bounds, gravity, interiors, special actions, animations (incl. anim-name table for 207), SCM events, ClientCheck layout | DIFF-2 coverage of these RPCs | M3 |
| F | M9 | audio streams (BASS-equivalent), 3D audio, sounds, radio behaviour | `e2e/m9.sh` + trace diff | M4 |
| F | M10 | client features: sa-mp.cfg, chatlog, screenshots (F8), keybinds, connection lifecycle (reconnect, server restart RPC 40, kicks/bans, wrong version/password) | `e2e/m10.sh` | M4 |
| G — Verification | DIFF-2 | per-RPC coverage harness (F1) | `diff/rpc-coverage.sh` | M3+ (grows with each milestone) |
| G | DIFF-3 | cross-client harness (F3) | `diff/cross-client.sh` | M3 |
| G | DIFF-4 | visual diff harness (F4) | `diff/visual.sh` | UX-1 |
| G | SOAK | stability (F7) | `e2e/soak.sh` | M7 |
| H — Versions & release | M11 | other client versions (R1/R3/R5/DL profiles) | F8 | binaries from user |
| H | M12 | Windows validation + packaging (launcher, install docs) | F9 | user on Windows |

Phase B–F milestones run in parallel where their files don't overlap (see §4).

## 3. Standard loop per milestone (what Claude runs autonomously)

1. **Gap check** — read the relevant `docs/re/` specs; list unknowns.
2. **Measure the original** (trace lane) — a TRACE-n agent runs scripted scenarios
   with the original client + tracer (+ samp-bot / test filterscripts) and relays
   findings to the main session as they become solid.
3. **Implement** (main lane) — one implementation agent per milestone, owning an
   explicit file set; findings from step 2 are forwarded to it.
4. **Accept** — Claude reruns the milestone's e2e script(s), all earlier e2e scripts,
   `cargo test --workspace`, clippy (host + i686) itself; nothing counts on an
   agent's word.
5. **Diff** — tracer re-diff and/or network/visual diff vs the original; remaining
   differences become a parity task (Mx-P) unless justified in ROADMAP
   "Decisions made autonomously".
6. **Record** — ROADMAP status + WORKLOG entry; update `docs/workflow.svg` at phase ends.

## 3a. Scheduler — what to do at every goal re-injection

The /goal text only states the end condition; the next action is derived from the
repository state, deterministically:

1. **Collect results** of finished agents: verify (§3 step 4), update ROADMAP + WORKLOG.
2. **Fill free slots** from `docs/ROADMAP.md` → "Next up" queue, top to bottom:
   take the first item whose dependencies are `done` and whose resources are free.
   Slots: one implementation agent on `samp-client` (lane main) · one tracer/
   measurement agent (lane trace) · one harness agent (lane diff) · up to two
   no-game agents (specs, pure crates). Never two writers on the same files.
3. **Refill the queue** when it runs low: next milestones from §2 in phase order
   (A → B → C/D/E in parallel where files don't overlap → F → G), each expanded into
   the §3 loop (TRACE-n measure → implement → accept → diff → parity task).
4. **Nothing startable** (all slots busy or everything blocked) → wait for the next
   agent notification; if blocked only on the user, write it under "Decisions waiting".
5. **Done check**: when every F1–F7 script exits 0 and phases A–G are `done`, run
   them all once more in one pass and show the outputs.

## 4. Concurrency rules

- Lanes: **main** (wine/prefix, 7777/47700) for implementation e2e; **trace**
  (wine/prefix-trace, 7790/47701–47702) for the original client and tracer diffs.
  A third lane may be added for long soak/diff harness runs.
- One implementation agent per crate area at a time (samp-client is single-writer);
  pure crates (samp-proto, samp-saa, tools/diff) can run in parallel.
- Shared crates (`samp-proto`, `samp-saa`) are used by every lane's builds: an agent
  editing them must keep `cargo build --workspace` green after EVERY edit (create a
  file before declaring its module, add methods before using them). A broken build
  blocks all other agents' e2e runs (happened once with NET-7).
- Agents never kill processes outside their lane, never `pkill -f` patterns, never
  screenshot the user's desktop, never commit.
- Budget: ≤ ~1.5M tokens per agent step; workflows only for broad sweeps.

## 5. Escalation to the user

Only: git commits (user), legal/publishing, system installs (sudo), irreversible
actions outside the project, new client binaries (versions), Windows runs.
Everything else is decided autonomously and logged.
