# FEAT-SWEEP — features of the original samp.dll that samp-rs does not implement or does not test

Audit, no client changes. The roadmap was built from the RPC list, the sync packets, the CSV of
gta_sa.exe patches and the subsystem specs; this sweep looks for everything outside those. Target:
samp.dll 0.3.7-R4 build B (fv0.3.7.3, 2019, sha256 15db80c5…, the installed one); build A addresses
map through `target/featsweep/a2b.json`. Ours = main as of 2026-11-05 morning (samp-client and
samp-ui as checked out; worktree branches not merged are not counted).

Four independent sweeps:

1. **Static inventory** of all samp.dll functions, clustered into features and joined with what our
   specs cite and what our sources cite.
2. **Dynamic coverage** of samp.dll on the original client: first-hit `int3` on every samp.dll
   function (COV-1 method, new module mode of the tracer) across the existing scenarios.
3. **Every string** in samp.dll checked against our sources.
4. **Every RPC/packet** samp registers or can send, checked against samp-proto + samp-client.

Behaviour is described in our own words; no decompiled code is reproduced.

## Headline numbers

**21 live features are missing or partial (table A); 4 of them are network-visible protocol gaps
(C→S 23, 96, 119, 168 never sent). Sweep 2 executed 1,737 of the 5,113 samp.dll functions in 15
original-client runs; of samp's own live code, 1,052 of 1,240 functions ran (85 %) and 188 never did.**
Five rows were live-checked on the original (F4, `/dl`, `/ctd`, `/testdw`, special actions).

| sweep | result |
|---|---|
| 1 inventory | 5,266 functions in B (5,113 armable entries, 153 thunks): RakNet 1,293, DXUT 746, CRT 617, EH unwind funclets 699, ttmath/RSA/SHA-1 119, detours 56, dormant custom-model machinery 313, **samp's own features 1,423** (1,240 referenced). Our specs (`docs/`) cite 865 functions, our sources/tools 481. Features no spec mentions: crash handler, `-d` debug mode, frame-name guard, window setup (plus the dormant machinery). 583 functions have no reference at all (dead code) |
| 2 coverage | 15 original-client runs (COV-1 scenario, DIFF-2 RPC harness, F2 network scenario, cross-client, M6/M8a×3/M8b/M9 parity scenarios, TRACE vehicles/remote, 2 live-check sessions): 1,737 of 5,113 armed functions executed; samp's own live code 1,052 / 1,240; 188 never executed, grouped with their trigger below |
| 3 strings | 431 rows (424 strings + 7 bulk summaries): 186 implemented, 39 partial, 81 missing, 125 n/a (library, dead, dormant) |
| 4 RPCs/packets | S→C: 141 RPCs registered, 131 handled, 10 decoded only (4 of them behave like samp anyway); 17 app packet ids, all handled. C→S: 32 RPC ids sendable (28 live), 24 sent by us, 4 live ones never sent (23, 96, 119, 168); 11 packets, all sent with samp's parameters |

## Tooling added (additive; `cargo build --workspace` green)

* `crates/samp-tracer/src/win/cov.rs` — **module coverage mode**:
  * `SAMP_TRACER_COV_MOD=<arm list of RVAs>` arms a second set of entries inside a client module
    (`SAMP_TRACER_COV_MOD_NAME`, default `samp.dll`). A poller thread waits for the module (it
    loads ~20 ms after the tracer, rebased to e.g. 0x2f80000 under Wine), rebases the list,
    checks every live byte against the file byte and arms it into the current window; `cov start`
    re-arms both sets. The dump gains a `# module NAME base=0x…` line.
  * `SAMP_TRACER_COV_DUMP=<path>` (`{pid}` → process id) rewrites the dump every 10 s and at
    each window start, so any scenario script produces coverage without sending `cov dump`.
    Nothing is written until something is armed (a game without samp.dll never overwrites the file).
  * Works with or without the gta_sa.exe arm list (`SAMP_TRACER_COV`). Stable in every run below
    (the COV-1 scenario ran with both lists: 21,346 entries armed per window).
* `tools/cov/samp-inventory.sh` + `tools/cov/ghidra/ExportGraph.java` — own headless Ghidra
  project (`target/ghidra-samp`); exports functions, call edges (imports named), string/data refs
  and data pointers to entries → `target/cov/samp.{functions,edges,refs}.tsv`;
  `tools/cov/samp_arm.py` → `target/cov/samp-arm.txt` (5,113 entries; entries whose byte is a
  relocation target are excluded — none were).
* `tools/cov/samp_graph.py` (loader), `samp_docrefs.py` (which functions docs/ and crates/+tools/
  cite; A addresses mapped to B), `samp_cov.py` (merge dumps per scenario),
  `samp_features.py` (clustering), `samp_never.py` (never-executed list with trigger hints).
* `docs/phase3/data/samp-feature-ranges.tsv` — manual labels for the clusters the specs do not
  cover; `docs/phase3/data/samp-never-executed.tsv` — the full never-executed list (generated).

Re-running:

```
tools/cov/samp-inventory.sh                                   # once (Ghidra ~3 min)
export SAMP_TRACER_COV_MOD='Z:\…\target\cov\samp-arm.txt' SAMP_TRACER_COV_DUMP='Z:\…\cov-NAME-{pid}.tsv'
tools/cov/run.sh orig | tools/diff/rpc-coverage.sh --orig-runs 1 --ours-runs 0 | … (any orig scenario)
python3 tools/cov/samp_cov.py merged.tsv NAME=dump …
python3 tools/cov/samp_docrefs.py target/featsweep/a2b.json target/featsweep/docrefs.tsv
python3 tools/cov/samp_features.py merged.tsv target/featsweep/docrefs.tsv target/featsweep/unref.txt target/featsweep/feat
python3 tools/cov/samp_never.py target/featsweep/feat.functions.tsv docs/phase3/data/samp-never-executed.tsv
```

## Missing or untested features

Severity: **U** = user-visible, **N** = network-visible (the server sees a difference), **I** =
internal (no visible effect, robustness only). "Covered" = executed by at least one original-client
scenario of sweep 2 (so the original's behaviour has been seen by our harness). Owner areas are the
roadmap's milestone areas.

### A. Live features we do not implement

| ID | feature (samp behaviour, our words) | evidence (B) | trigger | ours | owner | sev | covered |
|---|---|---|---|---|---|---|---|
| A1 | **F4 = back to class selection after the next death.** While spawned, F4 (polled with GetAsyncKeyState in the local-player tick) sets the local player's "class selection on next respawn" flag once and prints the client line "Returning to class selection after next death"; a second F4 does nothing until the flag is consumed. After the next death the client enters class selection instead of re-spawning (same flag RPC 74 sets) | 0x100074b0 (key test + line), flag local player +0x2fa; string 0x100e5910 | F4 key | missing (RPC 74 sets our `force_class_selection`; no key) | player-state / M8b | U, N (128 instead of 52 after death) | yes, **live-checked**: two F4 presses → one line |
| A2 | **"Car Jacked" ejection.** When a remote player's driver sync (200) claims the vehicle the local player is driving, samp removes the local ped from the car and shows the game text "~r~Car Jacked~w~!" | 0x10016220 (from the remote driver-sync apply path); string 0x100e6158 | another player jacks your car | missing (only a module doc comment in `vehicles.rs`) | vehicles / M5-P | U, N | function ran (4 scenarios); the jack branch never observed |
| A3 | **Held-item special actions.** With special action 20–23 (beer, cigar, wine, sprunk) the fire key plays the drink/smoke animation (BAR:dnk_stndM_loop / dnk_stndF_loop, GANGS:smkcig_prtl) and beer/wine raise the GTA drunk level; special action 68 (pissing) plays PAULNMAC:PISS_LOOP with a particle effect (PETROLCAN). The same functions also run from the remote-player tick (0x10014900), so remote players with these actions show them too | 0x100adcb0, 0x100ae110 (+ callers 0x100ade00 / remote apply) | SetPlayerSpecialAction 20–23 + fire; 68 | missing (actions are stored and synced; no item use, no drunk, no particle) | player-state + remote players / M8b | U, N (207 animation id, drunk level via 205/207) | yes (live runs); **live-checked**: bottle in hand, urine particle |
| A4 | **Remote parachute.** For a remote player in the parachute states samp drives the GTA parachute object and PARACHUTE:PARA_* animations (open, decel, float, steer L/R, land, land in water; the `_O` variants on the object) from the synced animation | 0x100ad630 (table), 0x100ad4e0, 0x100ae7e0 | remote player jumps with weapon 46 | missing (anim names only in `animtable.rs`) | remote players / M3-P | U | no |
| A5 | **Camera target reporting.** RPC 170 enables it; while enabled samp casts the camera ray each tick and sends RPC 168 (object, vehicle, player, actor ids) on change; `/ctd` additionally draws a label at the target | 0x100046e0 (sender), 0x10004830 (/ctd labels, strings CAMERA_TARGET_TYPE_*) | EnablePlayerCameraTarget + looking at entities | missing (170 decoded, no sender; `/ctd` toggles a flag only) | player-state / M8b | N, U (/ctd) | per-frame function ran; **/ctd live-checked** (ray line + red `CAMERA_TARGET_TYPE_VEHICLE: ID(5)`) |
| A6 | **SCMEvent sender (C→S 96).** Mod-shop paint job, component, respray and the other in-garage events go to the server from four send sites (rel 10); the local effects of event types 3 and 4 follow | 0x10001ce0 | mod shop / Pay'n'Spray | missing: `scmops.rs:111-114` installs the hook but never sends (comment "no vehicle pool yet", stale) | vehicles / M5-P | N, U | no |
| A7 | **OnPlayerClickPlayer (C→S 23).** Activating a scoreboard row (double click / Enter) sends the clicked player id | 0x1006ebd0 | TAB, pick a row | missing (selection exists, no send) | UI / M8a | N | no |
| A8 | **OnPlayerClickMap (C→S 119).** Right-click on the pause-menu map: samp finds the user waypoint blip and sends its x/y and ground z + 2 | 0x10075760 | ESC → map → right click | missing | UI / M8b | N | per-frame function ran; branch not observed |
| A9 | **PlayCrimeReport (S→C 112).** Police-scanner report with suspect, vehicle and location; the original also NOPs 39 bytes at 0x4E7529 when it arrives | handler 0x10019000 | PlayCrimeReport | decoded only | audio / M9 | U | yes (rpccov) |
| A10 | **SendGameTimeUpdate (S→C 60).** With the HUD clock off, resyncs GTA's time base to the server time | 0x1000ff10 | server time sync | decoded only | world / M8b | U (minor) | yes, **live-checked**: two F4 presses → one line0 |
| A11 | **`/dl` vehicle debug labels.** Text above every vehicle within range: id, model, type/subtype, health, preload flag, distance, passenger seats, client and server position | 0x10075760 (per-frame), strings 0x100ea7d8… | `/dl` | partial: command toggles, nothing drawn | UI / M10 | U | yes, **live-checked**: two F4 presses → one line1 |
| A12 | **`/testdw`.** Fills the death window with test rows (names "Pooper…", fixed weapon pattern) | 0x100684c0 | `/testdw` | partial: parsed, no rows | UI / M8a | U | yes, **live-checked**: two F4 presses → one line2 |
| A13 | **Window setup.** The GTA window class gets CS_DBLCLKS, the window is subclassed and its title becomes "GTA:SA:MP" | 0x10061eb0 | boot | missing (we subclass; no title, no class style; dialog double clicks are emulated in `glue.rs`) | boot / UX-1 | U | yes |
| A14 | **Crash handler.** samp's unhandled-exception filter (set in 0x100c50e0) writes a report (exception address, module base, registers, stack dump, loaded modules, SCM op, last rendered object, game version) and shows the "SA:MP has crashed" dialog with Close / Report Crash; Report Crash posts to team.sa-mp.com | 0x10060110, 0x10060240, 0x100603d0, 0x10060560, 0x10060750, dialog resource | any crash | missing | client features / M10 | U (on crash) | no (only on crash) |
| A15 | **Guarded game calls with chat warnings.** Script opcodes and add_car_component run under an exception guard; on a fault samp prints "Warning(opcode 0x…): Exception 0x… at 0x…" (or the add_car_component variant) and continues | code at 0x100609b3 / 0x100609dd / 0x100609f8 | a faulting opcode / component | partial (only the remote-player guard; ours logs to file) | M2/M5 | U, I | no |
| A16 | **Fatal checks with message boxes.** Unsupported gta_sa.exe → "Version Error" box ("SA-MP only supports GTA:SA v1.0 USA/EU") and exit; a model frame name longer than 23 chars (samp wraps GTA's frame-name setter 0x72FA50) → "FrameNode name was too long" box and exit | 0x100a08c0, 0x100a5ec0 | wrong exe; bad custom model | partial (version: logged, patches skipped) / missing (frame name) | boot / M1 | U, I | no |
| A17 | **Debug lines in chat.** samp prints these to the chat window (type debug); ours logs them to `samp-rs.log`: "Warning: vehicle/actor %u was not deleted", "couldn't create vehicle type", "Model %u wouldn't load in time!", "Couldn't spawn player(%u)", "bad train carriages", "All vehicle waiting slots are consumed!", ChatWindow direct-mode fallbacks, "SignText: Can't create font", BASS 3D attribute errors | various (strings TSV) | the error conditions | partial | per owner (M3/M5/M6/M9/M4) | U | partly |
| A18 | **IME.** `ime` in sa-mp.cfg enables DXUT's IME support (candidate/reading window, Chinese/Korean IMEs) in the chat input and dialog edit boxes | 0x10069800, 0x1006f9f0, DXUT IME 0x10085ab0… | ime=1 | partial (key read, unused) | UI / M10 | U (CJK users) | no |
| A19 | **SetPlayerVirtualWorld (S→C 48).** Stored; it gates a short freeze in samp's own logic | 0x1001dc70 | SetPlayerVirtualWorld | decoded only | world / M8b | I | yes, **live-checked**: two F4 presses → one line9 |
| A20 | **RPC 92 / 150** (run GTA opcodes 0x052C / 0x03FD with a u32) | 0x10018c00, 0x10018cb0 | custom servers only | decoded only | M8b | U (custom servers) | function ran (4 scenarios); the jack branch never observed0 |
| A21 | **Offline debug mode (`-d`).** Developer mode: `/v`, `/vehicle`, `/vsel` (vehicle select screen "Vehicle Select"), `/player_skin`, `/set_weather`, `/set_time`, the DEBUGSCRIPT loader (reads CreateObject/CreateVehicle/AddStaticVehicle/SetPlayerCameraPos/SetPlayerCameraLookAt/RemoveBuildingForPlayer/SetPlayerInterior lines) and a heightmap setup | 0x1009e140, 0x1009e8d0, 0x1009e9e0, 0x10068cc0…0x10068f71 | `-d` on the command line | missing (no `-d`) | client features / M10 | U (developers only) | no |

### B. Dormant in 0.3.7-R4 (no work needed; listed so nobody re-discovers them)

| feature | evidence | why dormant |
|---|---|---|
| custom-model (artwork) download: manager, RPC 179/181–185, progress UI "Updating models..", HTTP client (CHttpClient), `SAMP\cache`, `model_cache` / `artwork_proxy` registry values, DFF/TXD/COL readers | 0x1000bd50–0x1000e580, 0x1006a720–0x1006b090, 0x100022e0–0x10002e90, 0x100a76f0–0x100a7e30, 0x100b8ce0–0x100c3a10, 0x100c3b00/0x100c3c10 | the manager is never created and the S→C RPCs are not registered (session-misc.md); not executed in any scenario |
| `/togobjlight`, `/cmpstat` | 0x101166c0 flag has no reader; handler is a bare return | no effect in the original |
| RakNet extras (ConsoleServer, EmailSender, FileList, Router…), DXUT sample framework, `rawpositions` variants | library ranges | linked but unused |
| crash report upload | 0x10060750 | **not to be reproduced**: sending reports to a third-party site is outside the project; a local crash report is enough (proposed decision for A14) |

### C. Protocol details found on the way (doc fixes, no client gap)

* `docs/re/rpc-ids.md` C→S table: RPC **27** (OnPlayerSelectObject) is missing — send site B
  0x1006ddc7, ID cell 0x100e5e9c, reliability 9 (ours sends it, `objects.rs:496`).
* RPC 96 is sent from **four** sites inside B 0x10001ce0 (all reliable-ordered, rel 10), not one.
* RPC 140 (menu exit) goes out with a null bitstream.
* Every C→S RPC send site uses priority 1, ordering channel 0, no timestamp shift.
* DIFF-2 tags in `target/audit/diff2` are stale: 13, 16, 18, 20, 33–36, 41, 72, 79, 81, 82, 86, 87,
  89, 90, 103, 111, 121, 124, 126, 127, 133, 145, 146, 156, 166 have handlers in main now; re-run
  `tools/diff/rpc-coverage.sh` (with ours) to re-tag.

## Sweep 1 — function inventory and feature clusters

`tools/cov/samp_features.py` labels every function: manual range labels for libraries and
uncovered clusters (`docs/phase3/data/samp-feature-ranges.tsv`), else the spec that cites it, else
its callers' / callees' label, else its address neighbour. Result (`target/featsweep/feat.features.tsv`):

| cluster | functions | live (referenced) | executed in sweep 2 | never executed (live) | cited by our sources |
|---|---|---|---|---|---|
| RakNet library | 1293 | 1120 | 254 | 866 | 24 |
| DXUT framework + GUI controls | 746 | 635 | 130 | 507 | 2 |
| EH unwind funclets | 699 | 690 | 1 | 689 | 0 |
| MSVC CRT / C++ runtime | 617 | 505 | 205 | 302 | 3 |
| big-int / RSA (ttmath) + gpci serial, SHA-1 | 119 | 84 | 29 | 56 | 1 |
| detours library | 56 | 45 | 0 | 45 | 0 |
| dormant custom-model machinery (5 clusters) | 313 | 276 | 20 | 258 | 5 |
| spec: ui-rendering | 182 | 165 | 142 | 28 | 62 |
| spec: world-entities | 177 | 151 | 149 | 7 | 65 |
| spec: remote-control-onfoot | 167 | 136 | 118 | 27 | 45 |
| spec: player-state | 151 | 136 | 135 | 5 | 55 |
| spec: remote-players-sync | 111 | 101 | 93 | 8 | 43 |
| spec: remote-control-combat | 99 | 82 | 72 | 14 | 29 |
| spec: remote-control-vehicles | 93 | 80 | 80 | 6 | 26 |
| spec: ui-chat-audio | 67 | 61 | 59 | 6 | 23 |
| spec: vehicles | 53 | 52 | 44 | 8 | 12 |
| spec: session-misc | 48 | 41 | 32 | 10 | 6 |
| spec: samp-saa | 48 | 44 | 41 | 7 | 7 |
| spec: gta-patches | 46 | 39 | 24 | 16 | 11 |
| spec: handshake | 9 | 9 | 6 | 3 | 3 |
| phase-3 notes only (ux1, m6p, trace, m10, m8a/b, fix*, …) | 145 | 120 | 102 | 21 | 59 |
| crash handler + crash report (uncited) | 13 | 11 | 0 | 11 | 0 |
| debug mode: vehicle select, debug script (uncited) | 13 | 12 | 1 | 11 | 0 |
| frame-name length guard (uncited) | 1 | 0 | 0 | 0 | 0 |

Reading it:

* samp's own code (spec + notes + the three uncited clusters): **1,423 functions, 1,240 live, 188
  never executed** by any scenario.
* Every spec cluster maps onto an implemented milestone module (`players.rs`/`remote.rs`/
  `footsync.rs` for on-foot, `vehicles.rs`, `combat*.rs`, `objects.rs`/`entities.rs`/`worldents.rs`,
  `ui/*` + `samp-ui`, `audio.rs`, `saafs.rs`, `patches.rs`/`*hooks.rs`); the features of those
  clusters that are still missing are the rows of table A (found by sweeps 3 and 4 and by reading
  the uncited functions).
* The only clusters **no spec mentions** are the crash handler, the `-d` debug mode, the
  frame-name guard, the window setup (inside a cited cluster) and the dormant model machinery.
* 583 functions have no reference at all (no call, no data pointer, no matching dword anywhere in
  the image): dead code, excluded from "live" (`target/featsweep/unref.txt`).
* Inventory blind spot: **97 code blobs** that samp registers by pointer (`push imm32` of a code
  address that is not a Ghidra function): the local-command handlers (`/dl` 0x10068fe0, `/ctd`
  0x100685a0, `/testdw` 0x100684c0, the debug commands 0x10068cc0…), a few RPC handlers that are
  labels (e.g. 0x1000e570), the window procedure 0x10061610, the crash dialog procedure
  0x10060820, hook stubs. They are not armed (`target/featsweep/code-blobs.txt`); a future
  ExportGraph pass could create functions there before exporting.

## Sweep 2 — dynamic coverage of samp.dll on the original client

Original client (samp.dll B + samp-tracer with `SAMP_TRACER_COV_MOD`), 15 runs (m6 twice: one partial run in the trace lane, one complete on the diff lane), every one stable
(no crash, no failed arm; 5,113 entries armed, 0 skipped):

| scenario (label) | script | samp.dll functions hit | only this scenario |
|---|---|---|---|
| covorig | `tools/cov/run.sh orig` (COV-1 windows, idle 60 s) | 1004 | 0 |
| rpccov | `tools/diff/rpc-coverage.sh --orig-runs 1 --ours-runs 0` (176 steps, 131 S→C RPCs) | 1541 | 75 |
| netorig | `tools/diff/network.sh --orig-only` (chat, dialog, scoreboard, pickup, checkpoint, object, drive, passenger, shoot, loss, death, quit) | 1189 | 7 |
| xclient | `tools/diff/cross-client.sh` (original + ours on one server) | 1141 | 10 |
| m8b | `tools/e2e/m8bparity.sh orig` | 1175 | 4 |
| m6 | `tools/e2e/m6parity.sh orig` | 1254 | 15 |
| m8a1/m8a2/m8a3 | `tools/e2e/m8a-orig{,2,3}.sh` | 1111 / 1070 / 1272 | 4 / 0 / 26 |
| m9 | `tools/e2e/m9-orig.sh` | 960 | 5 |
| veh / remote | `tools/trace/run.sh orig vehicles` / `remote` | 1113 / 1046 | 1 / 22 |
| live / live3 | FEAT-SWEEP live checks (F4, /dl, /ctd, /testdw, /togobjlight, /cmpstat, special actions 20–23, 68) | 992 / 1049 | 2 / 5 |
| **union** | | **1737 of 5113** | |

The trace lane was held by another lane for most of the sweep, so the trace-lane scenarios ran on the
diff lane through a re-rooted copy of the trace tools (`target/featroot`: lane names, ports and the
lock replaced by the diff lane's; nothing in `tools/` changed).

Limits of the method: first hit per function only (a per-frame function that contains a feature
branch counts as covered even if the branch never ran — `/dl`, `/ctd`, map click, the jack text);
functions that run only in samp's first ~20 ms (DllMain, before the poller arms) and in the last
≤10 s before a game exits (quit, kick, connection-lost paths) are not seen; the 97 code blobs above
are not armed.

### Never executed (188 live samp functions; full list `docs/phase3/data/samp-never-executed.tsv`)

| area | functions (B) | what triggers them (static) | scenario that would exercise them |
|---|---|---|---|
| crash handler | 0x10060110, 0x10060240, 0x100603d0, 0x10060560, 0x10060a30–0x10060cb0, 0x10017b40 (base64 of the report upload) | an unhandled exception | a deliberate fault in a test build (e.g. a tracer `poke` into a null call) — only after A14 exists |
| debug mode | 0x1009e100–0x1009e9e0 | `-d` command line; `/vsel` | an offline `-d` run (A21) |
| connection lifecycle | 0x10008dd0 (server closed), 0x1000ace0 (lost connection), 0x10010520 (RPC 130 rejected), 0x10009870 / 0x1000ec90 / 0x100140d0 / 0x100180c0 (quit + pool teardown), 0x10008a50 | kick/ban, server stop, network loss, wrong version/nick, `/q` | TRACE-8's lifecycle cases (kick, gmx, wrong password/version, lost connection, /q) on the original with the coverage env; the driver must send `cov dump` before the game exits |
| remote bullets / trailers / drive-by / spectate pads | 0x10009b30 + 0x10016320 (206 bullet sync apply), 0x10009e10 + 0x10015c40 + 0x100b7de0 (210 trailer apply), 0x100053c0 (210 sender), 0x100acfa0 (drive-by), 0x100acd20…0x100acf30, 0x100afc80, 0x100b10f0 | a **remote** player shooting, towing, or doing a drive-by; local towing | `tools/trace/combat2.sh` / `lagcomp.sh` (remote shooter), `tools/e2e/m6pvehscen.sh` (tow) on the original with the coverage env |
| remote parachute, dances, strip | 0x100ad4e0, 0x100ad630, 0x100ae7e0 (parachute), 0x100adae0, 0x100adb80, 0x100b00c0 (DANCE / strip_A), 0x100b0770, 0x100aebc0, 0x100aecc0 | SetPlayerSpecialAction 5–8 (dances), a player in parachute states | a bot or uifs `special L 5..8`, and a remote parachute jump |
| RPCs open.mp never sends | 0x1000e590 (102), 0x1000f1b0 + 0x10013350 (151), 0x10018b20 (98), 0x10018c00 (92), 0x10018cb0 (150), 0x10019e10 + 0x1001eae0 (161), 0x1001a6b0 (64), 0x1001c120 (169), 0x1001dc70 (48) | custom server / raw RPC | a raw-RPC injector (samp-bot as a fake server or a filterscript using `SendRPC`) |
| chat window extras | 0x10061280 (F7 mode cycle), 0x10067260/0x10067290/0x100672f0/0x100670a0 (scroll, PageUp/PageDown), 0x100695f0, 0x10069660 (input history keys) | F7, PageUp/PageDown, Up/Down in the input | `tools/trace/ui.sh key F7 Prior Next` in any hold session |
| `/save`, `/rs` | 0x100689c0, 0x10068b40 | `/save [name]`, `/rs` on foot and in a car | M10 cfg case on the original |
| model preview (font 5) internals | 0x1006c970, 0x100b17e0, 0x100b1810, 0x100b19e0, 0x100b1c90, 0x100b4530, 0x100b6260, 0x100b62c0 | a model-preview textdraw with a vehicle model / rotation change | m8a-orig2 MR-M8A-2 with vehicle models (M8A-PREVIEW) |
| object model loading branches | 0x100a7f60, 0x100a8120, 0x100a81b0, 0x100a8250, 0x100a82d0, 0x100b1dd0…0x100b1fd0, 0x100b4740 | CreateObject / SetPlayerAttachedObject with models that need the custom texture path (SA:MP objects 18631+, 19xxx) | m6 with SA:MP-specific object ids |
| vehicle params branches | 0x100b7590, 0x100b8220…0x100b83b0 (RPC 24 sub-cases), 0x1001f1e0 (vehicle death report) | probably the SetVehicleParamsForPlayer objective / door-lock sub-cases; a vehicle blowing up (VehicleDeath sender) | rpccov with those steps, a vehicle destroyed by fire |
| unknown purpose | 0x100b6ae0 "Gparticle" (1469 bytes) + 0x100b1e50, via the flag-gated stream-read hook 0x100a5f40; 0x1006e050 (loads "bland.png" into a sprite; no live caller chain) | not determined statically | investigate before scheduling (FEAT-INV) |
| boot-time only (method blind spot) | 0x10062930 (ShowCursor detour), 0x10062c60 (CreateFileA detour for samp.saa), 0x10065310, 0x100c45d0 (command line), 0x100c3ed0/0x100c3f20, 0x1009ffb0 | samp's DllMain | not a gap: these ran before the poller armed |

## Sweep 3 — strings

Every ASCII and UTF-16 string of B (`.rdata` 0x100e55fa–0x100ed900, `.rsrc`, a few `.data` tables;
the 1–3 letter command names `q`, `v`, `dl`, `ctd`, `mem`, `rs` taken from the registration
function 0x100690d0), each mapped to its referencing function and checked by hand against
`crates/`. 431 rows:

| kind | implemented | partial | missing | n/a |
|---|---|---|---|---|
| local command | 18 | 9 | 0 | 0 |
| client message | 47 | 11 | 20 | 42 |
| format (UI / debug / crash report) | 32 | 10 | 20 | 10 |
| error / message box | 0 | 3 | 8 | 0 |
| sa-mp.cfg key | 12 | 1 | 0 | 0 |
| file name | 20 | 1 | 2 | 0 |
| registry | 0 | 0 | 2 | 1 |
| URL | 0 | 0 | 3 | 0 |
| key name | 9 | 0 | 0 | 0 |
| other (anim/model names, labels) | 48 | 4 | 26 | 72 |

All local commands exist in `samp-ui/src/commands.rs`; the partial ones are those whose effect is
not applied (`/dl`, `/ctd`, `/testdw`) and the `-d` debug commands (`/v`, `/vehicle`, `/vsel`,
`/player_skin`, `/set_weather`, `/set_time`), which are unreachable without debug mode. Every
sa-mp.cfg key is read; `ime` is read but unused. The missing strings collapse into table A rows
A1–A4, A11–A18, A21 (crash report formats, F4 line, Car Jacked, parachute/drink/smoke/piss
animation names, window title, message boxes, chat debug lines, cache folder/registry). The
dormant download strings, the HTTP/SMTP/RakNet console strings and the DXUT sample strings are n/a.

## Sweep 4 — RPCs and packets

* S→C RPCs: 141 registered (46 "core" + 95 "script", identical in A and B). Ours handles 131; 10 are
  decoded without an effect: 48 (A19), 60 (A10), 64, 92 and 150 (A20), 102, 112 (A9), 125, 151, 170
  (A5). 64 and 125 do nothing in samp either, 102 only stores a value, 151 is a dead path — so
  4 are equivalent. We also decode 179/183/185, which R4 never registers (harmless).
* S→C packets: samp's dispatch (B 0x1000b250) handles 0x0c, 0x1d, 0x1f, 0x20–0x25, 200, 203,
  206–211: all handled by ours, plus RakNet 0x19/0x1a.
* C→S RPCs: 36 send sites, 32 ids (28 live + 97, 181, 182, 184 dormant). Ours sends 24 with matching
  reliability. Live and never sent: **23** OnPlayerClickPlayer (A7), **96** SCMEvent (A6), **119**
  OnPlayerClickMap (A8), **168** OnPlayerCameraTarget (A5).
* C→S packets: 200, 201 (RCON), 203–207, 209–212 — all sent with samp's parameters; RakNet control
  messages (connection request, auth key, static data, ping, keep-alive, disconnect) present.

## Proposed queue (FEAT-*)

Ordered by severity, then cost. Each item follows the parity loop (original first with the given
scenario, then ours, then diff); the coverage env above lets each run prove the original's
functions executed.

| ID | task | rows | owner | scenario / verification |
|---|---|---|---|---|
| FEAT-1 | SCMEvent sender: send RPC 96 from the four mod-shop / respray sites and apply the local effects of event types 3/4 (`scmops.rs` hook already installed) | A6 | vehicles (M5-P) | Transfender + Pay'n'Spray drive on both clients; F2 network diff on 96; EFX on the local effects |
| FEAT-2 | Camera target: RPC 170 enable flag, per-tick ray cast and RPC 168 on change; `/ctd` ray line + label | A5 | player-state (M8b) | uifs `camtarget L 1`, look at a vehicle/object/player/actor; server OnPlayerCameraTarget; D3D diff of `/ctd` |
| FEAT-3 | F4 class reselection (key → flag + client line; next death → class selection) | A1 | player-state (M8b) | F4 twice, then `hp L 0`; chat line once; 128 instead of 52 on the wire |
| FEAT-4 | Scoreboard row activation → RPC 23; pause-map right click → RPC 119 (waypoint blip z + 2) | A7, A8 | UI (M8a/M8b) | netscen scoreboard step + map step; F2 diff |
| FEAT-5 | Held-item special actions 20–23 (drink/smoke anims on fire, drunk increase) and 68 (piss anim + particle), local and remote | A3 | player-state + remote (M8b/M3-P) | uifs `special L 20..23/68` + fire; 207 animation id diff; screenshot diff; remote via a bot |
| FEAT-6 | Car Jacked ejection + game text | A2 | vehicles (M5-P) | drive_bot jacks the local player's car (TRACE-4 vehicles scenario extended) |
| FEAT-7 | Remote parachute object/animations | A4 | remote players (M3-P) | bot with weapon 46 jumping from height (needs bot parachute states) |
| FEAT-8 | `/dl` labels and `/testdw` rows | A11, A12 | UI (M8a/M10) | live3 scenario (this sweep) on both; D3D diff |
| FEAT-9 | PlayCrimeReport audio + the 0x4E7529 NOP; SendGameTimeUpdate clock resync; SetPlayerVirtualWorld store | A9, A10, A19 | audio (M9) / world (M8b) | rpccov steps crime_report / time; BASS + effect diff |
| FEAT-10 | Window class style + "GTA:SA:MP" title; Version Error and FrameNode-name message boxes | A13, A16 | boot (M1/UX-1) | boot trace; window title via `xdotool getwindowname`; frame-name guard by unit test of the wrapper |
| FEAT-11 | Debug lines to chat instead of the log; exception-guarded opcodes/add_car_component with chat warnings | A15, A17 | per owner | fault injection (tracer poke) on both clients |
| FEAT-12 | Crash handler: local report file with samp's fields + crash dialog (Close only; **no upload** — decision to record) | A14 | client features (M10) | deliberate fault in a test build on both clients |
| FEAT-13 | IME support behind `ime=1` | A18 | UI | manual (needs a CJK IME under Wine) |
| FEAT-14 | Offline debug mode `-d` (debug commands, vehicle select, DEBUGSCRIPT, heightmap) | A21 | client features | `-d` boot on both clients — low priority (developer tool) |
| FEAT-COV | Close the measurement gaps: coverage env for the M10 lifecycle cases (force `cov dump` before exit), remote shooter/tow/drive-by scenarios (combat2/lagcomp/m6pvehscen), dance special actions 5–8, a raw-RPC injector for the 10 ids open.mp never sends, F7/PageUp/PageDown/history keys; ExportGraph: create functions at the 97 pointer-registered code blobs so they get armed; re-run DIFF-2 with ours to refresh the stale tags | §Sweep 2 | tooling | union coverage of samp's own live functions ≥ 95 % (now 85 %: 1,052 of 1,240) |
| FEAT-INV | Find out what 0x100b6ae0 ("Gparticle", behind the flag-gated stream-read hook 0x100a5f40) and 0x1006e050 ("bland.png" sprite) do | §Sweep 2 | RE | static + a tracer watch on the flag |
| FEAT-DOC | rpc-ids.md fixes (RPC 27 C→S, 96 ×4 sites, 140 null bitstream) | §C | RE docs | — |

## Files

* New: `docs/phase3/feature-sweep.md` (this file), `docs/phase3/data/samp-feature-ranges.tsv`,
  `docs/phase3/data/samp-strings-sweep.tsv`, `docs/phase3/data/samp-rpc-sweep.tsv`,
  `docs/phase3/data/samp-never-executed.tsv`, `tools/cov/samp-inventory.sh`,
  `tools/cov/ghidra/ExportGraph.java`, `tools/cov/samp_arm.py`, `tools/cov/samp_graph.py`,
  `tools/cov/samp_docrefs.py`, `tools/cov/samp_cov.py`, `tools/cov/samp_features.py`,
  `tools/cov/samp_never.py`.
* Changed: `crates/samp-tracer/src/win/cov.rs` (module mode + periodic dump; additive, the gta
  coverage mode is unchanged).
* Generated (gitignored): `target/cov/samp.*.tsv`, `target/cov/samp-arm.txt`, `target/featsweep/`
  (dumps per scenario `cov/cov-<label>-<pid>.tsv`, `cov/merged.tsv`, `feat.*`, `docrefs.tsv`,
  `a2b.json`, `unref.txt`, `code-blobs.txt`, live screenshots `live/`, `live3/`), `target/featroot/`
  (the re-rooted diff-lane tools; delete when done).
* Sweep tables: `docs/phase3/data/samp-strings-sweep.tsv` (431 rows: address, kind, string,
  referencing function, our status/location, behaviour, trigger, severity) and
  `docs/phase3/data/samp-rpc-sweep.tsv` (213 rows: direction, id, samp function, trigger, send
  parameters, our status/location, severity).
