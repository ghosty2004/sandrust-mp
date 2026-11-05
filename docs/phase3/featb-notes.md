# FEAT-B — player-visible features of the completeness audit

Branch `player-features` (worktree `featb`). Items from `docs/phase3/feature-sweep.md` table A: A3 (FEAT-5),
A4 (FEAT-7), A11/A12 (FEAT-8), A13/A16 (FEAT-10), A15/A17 (FEAT-11), A14 (FEAT-12), A18 (FEAT-13), A21 (FEAT-14).
Static reading: samp.dll build B through a private copy of the FEAT-SWEEP Ghidra project (decompiled exports,
objdump); behaviour is described in our own words in the module headers. Dynamic: the original client in the trace
lane, ours in lane ui, same scenario (`tools/e2e/featbscen.sh`), samp-tracer on both.

## Tooling added

* `tools/e2e/featb.sh orig|ours|(all)` — runs the scenario on the original (trace lane, `tools/trace/run.sh orig
  hold`) or ours (lane ui through a re-rooted copy of the trace tools in `target/uiroot`), copies `chatlog.txt` and
  our `samp-rs-crash.txt` into the run; the scenario runs in a subshell so an error never leaves a hold session (and
  the lane lock) running. Without an argument: ours, then the report against `FEATB_ORIG` (default the newest
  original run).
* `tools/e2e/featbscen.sh` — the scenario, blocks `items, remote, para, guard, dl, crash` (`FEATB_ONLY`).
* `tools/e2e/featb_report.py RUN [--orig RUN]…` — per-feature assertions (script commands per step from
  `trace.jsonl`, server `pstate` lines, `chatlog.txt`, `window.txt`, `crash-window.txt`, `d3d-dl/testdw.jsonl`) and
  the per-step parity diff of the feature script commands (handles and positions normalised; presence per step,
  since per-frame commands depend on the frame rate). Decisions: `tools/e2e/featb-decisions.tsv` (empty).
* `tools/e2e/featb-version.sh` — FEAT-10 "Version Error" on both clients (lane ui, a copy of gta_sa.exe with the
  probe byte changed, as a new top-level file of the lane's game dir, removed afterwards).
* `tools/diff/visual-featb.sh` + `VISUAL_SCENARIO_FILE` / `VISUAL_DIR` in `tools/diff/visual.sh` and the elements
  `dl`, `dloff`, `testdw` in `tools/diff/visual_diff.py` — the D3D call-stream diff of `/dl` and `/testdw`.
* samp-tracer: `window [click ID]` control command (top-level windows of the game process: class, title, window
  and class style, dialog controls with texts; `click` posts a button click and refuses any "Report" button).
* uifs.pwn: `pstate|P` (drunk level, special action, animation index, weapon as the server sees them).
* samp-bot: `--anim-seq W,W,… --anim-ms MS` (207 animation words in turn; remote parachute measurement).
* Fault injection for FEAT-11/12 through the tracer's `poke`: `mov eax,[0]` over the entry of
  `CVehicle::AddVehicleUpgrade` 0x6E3290 (reached only from ADD_VEHICLE_MOD), the `CTaskComplexUseMobilePhone`
  constructor 0x6348A0 (only from TASK_USE_MOBILE_PHONE, special action 11) and `CWeather::Update` 0x72B850 (every
  frame, inside `CGame::Process`).

## Measurements on the original (trace lane)

Runs `target/trace/orig-20261105-100040` (items), `-100640` (guard, /dl, /testdw, crash), `-101602` (remote items,
remote parachute); all with samp.dll build B.

* Items: RPC 88 20/22/23/21 → `CREATE_OBJECT(1543/1486/1546/1485)` at the ped, `TASK_PICK_UP_OBJECT(ped, obj,
  offset, 6, 16, "NULL", "NULL", -1)`; fire → `TASK_PLAY_ANIM_NON_INTERRUPTABLE(ped, "dnk_stndM_loop", "BAR", 4.0,
  0,0,0,0,-1)` (cigar `"smkcig_prtl","GANGS",700.0,…,2750`), drunk level +1250 per drink (not sprunk), 207 special
  action 20/22/23/21, `SET_CURRENT_CHAR_WEAPON(ped, 0)` every frame while held; the enter/exit key drops it
  (`TASK_PICK_UP_OBJECT(…, 0,0,0, …, 0)`, object left in the world). The male anim is used for every skin: the
  "ped type" samp compares is a vtable slot of the ped model info (B 0x100B4110), never 5 or 22.
* Piss: anim `PAULNMAC:PISS_LOOP` (local only), `CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION("PETROLCAN", ped, 0, 0.58,
  −0.08, 0, 0.01, 0, 1)`, `PLAY_FX_SYSTEM`; 207 special 68; enter/exit key → `KILL_FX_SYSTEM`. A remote pissing
  ped's effect is not killed when the ped is deleted (samp leaves it).
* Remote items: `CREATE_OBJECT` + pick-up for the remote ped, drop when the synced action changes; no animation and
  no drunk level for remotes (the animation comes with the 207 animation field).
* Remote parachute (bot with weapon 46 and PARACHUTE anims in its 207): the anim, then on PARA_OPEN the helper object
  (`CREATE_OBJECT(3131)`, `ATTACH_OBJECT_TO_CHAR` zero offsets, hidden, scale 0, `PLAY_OBJECT_ANIM("PARA_OPEN_O",
  1000, 0, 1)`, shown, scale 1) and `ADD_ONE_OFF_SOUND(local + (remote − local) × 0.1, 1039)`; the other PARA_* anims
  play their `_O` object anim (speed 1, loop; LAND_WATER not looped); weapon no longer 46 → `DETACH_OBJECT` +
  `REMOVE_OBJECT_ELEGANTLY`.
* Guards: "Warning(add_car_component 1010): Exception 0xC0000005 at 0x6E3290", "Warning(opcode 0x729): Exception
  0xC0000005 at 0x6348A0"; the `CWeather::Update` fault is caught 8 times as "Warning(s007): …" (samp's guarded
  per-frame hook around `CGame::Process`, B 0x100A2670) — together with the two above the filter's 10 handled faults —
  and the 11th reaches the top-level filter: dialog "SA:MP An exception occured" (style 0x14CA0AC4, class style
  0x808), edit 1000 with the report, static text, Close 1001, Report Crash 1002.
* `/dl`: one label per vehicle within 20 m, colour 0xFF358BD4, text
  `[id: 6, type: 411 subtype: 1 Health: 1000.0 preloaded: 0]\nDistance: 12.00m\nPassengerSeats: 1\ncPos: …\nsPos: …`,
  four black outline draws + the main draw through the colour-code font wrapper (no sprite pointer).
* `/testdw`: eight fixed rows ("Pooper", "Pooper333", "PooperPooperPooper0001", reasons 1, 5, 15, 14, 2, 5, 255, 0).
* Window: title "GTA:SA:MP", class style 0x2008 (CS_DBLCLKS | CS_OWNDC).
* Version Error: with the probe byte changed, the box "Version Error" appears within 2 s on both clients.

## Results (ours vs original)

| feature | result | evidence |
|---|---|---|
| FEAT-5 items 20–23, 68 (local) | done, parity diff empty | featb_report: all script commands per step equal; server-side drunk level / special action / anim index sequences match |
| FEAT-5 remote items / piss | done, parity diff empty | same |
| FEAT-7 remote parachute | done, parity diff empty | same (object, anims, sound, removal) |
| FEAT-8 `/dl`, `/testdw` | done | visual diff: `testdw`, `dloff` empty; `dl` text identical except the live vehicle z (physics), glyph-atlas batch split (existing decision) |
| FEAT-10 window class/title | done | `window`: same title, class style 0x2008 on both |
| FEAT-10 Version Error | done | box on both within 2 s (`featb-version.sh`); dismissing it cannot be automated under gamescope (both sides) |
| FEAT-10 FrameNode guard | not reproduced | B 0x100A5EC0 has no reference in R4 (dead code; sweep: 0 live) |
| FEAT-11 guarded opcodes + add_car_component | done | identical chat lines; one shared 10-fault counter with the remote-ped guard and the new s007 guard |
| FEAT-11 debug lines in chat | done where the condition exists | vehicle/actor not deleted, couldn't create vehicle type, model wouldn't load in time, couldn't spawn player, chat render-target fallbacks, SignText font |
| FEAT-12 crash handler | done | same dialog layout/styles/caption, same report fields, Close only, `samp-rs-crash.txt` written |
| FEAT-13 IME | not implemented | see below |
| FEAT-14 `-d` | not implemented | see below |

Also found and fixed on the way: the InitGame vehicle-model preload (B 0x100A1620: models 400–611 released, the 80
most used requested) was missing; `/dl` reports its flag. The playerped warning had a trailing period samp does not
print.

## Decisions (also in ROADMAP "Decisions made autonomously")

* FEAT-12: no "Report Crash" button and no upload; the full report (with the module list samp only adds to the
  upload) goes to `samp-rs-crash.txt` next to `samp-rs.log`, and the static text says so. The header names our
  client. "LastRendObj" is 0 (no render hook here records entity models) and the remote-player entries of "State
  Information" are left out (two fields of samp's remote-player object, +0x10A / +0x1E7, not identified).
* FEAT-11: "bad train carriages" and "All vehicle waiting slots are consumed!" have no corresponding condition here
  (carriages come from the locomotive, no deferred vehicle queue); BASS 3D attribute errors are dead code in samp.
* FEAT-10: an EU executable keeps today's behaviour (logged, not patched) until the version profiles (M11); only an
  unknown executable gets the box. The FrameNode guard is dead code in R4.
* FEAT-8: `/dl` cPos is the live vehicle position (physics settles differently by a few mm); the D3DX glyph-atlas
  batch split is the existing visual decision.
* FEAT-13 (IME): DXUT's IME path needs a CJK input method with composition / reading / candidate windows; Wine in
  the test prefixes has no such IME (only the "Wine IME" default window), so neither client can be driven into it
  and nothing can be measured or diffed. Untestable here; stays open for a Windows run with an installed IME.
* FEAT-14 (`-d`): developer-only offline mode (vehicle selector, debug commands, DEBUGSCRIPT loader, a heightmap from
  a hard-coded developer path). Not done in this step (lowest priority, needs its own original measurement of an
  offline `-d` boot); the entry points are listed in feature-sweep.md A21.
