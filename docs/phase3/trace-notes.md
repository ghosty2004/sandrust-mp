# Phase 3 / TRACE-1 + DIFF-1 — behavioural differential tracer

Black-box observation of which gta_sa.exe 1.0 US functions a SA:MP client calls, in what
order and with which arguments. Everything below was observed at runtime with the original
client (samp.dll 0.3.7-R4, build B fv0.3.7.3, as installed in `wine/gta-sa-trace`) and is
described in our own words. GTA names are named via gta-reversed / plugin-sdk. SCM opcode
names come from gta-reversed `Enums/eScriptCommands.h`.

## Tools

* `crates/samp-tracer` (`samp_tracer.dll`, i686-pc-windows-gnu). It is loaded **before** the
  client: `samp-launcher --dll samp-tracer.dll --dll samp.dll …`. `--dll` may now be given
  several times; the DLLs load in order.
  * It inline-hooks 56 GTA functions (52 for TRACE-1, plus 4 for TRACE-2: see "Remote players"). The table is in `hooks.rs`: exact prologue bytes,
    checked by tests against `re/gta/1.0-US/gta_sa.exe`, against the x86 length decoder in
    `x86.rs`, against every samp.dll patch/read site (annotated CSV, the 707-entry
    model-pointer table, the auth read at 0x4D3AA0, and samp-client's CdStreamSync patch),
    and against any rel32 branch into the stolen bytes.
  * Each hook is a 64-byte thunk: `pushad`, a call to the dispatcher, `popad`, then either
    the trampoline or a typed wrapper. Typed wrappers are used where post-call data is
    needed: `ProcessOneCommand`, `CRunningScript::Process`, and the CFileMgr reads.
  * The functions hooked:
    * Scripts: ProcessOneCommand (samp's executor), CRunningScript::Process, CTheScripts
      Init / Process / StartNewScript ×2 / StartTestScript, ShutdownThisScript.
    * Game init: CGame::Process (frame boundary), InitialiseWhenRestarting,
      ReInitGameObjectVariables, ShutDownForRestart.
    * Player and peds: CPlayerPed ctor / SetupPlayerPed / Remove / Deactivate / Reactivate;
      CPed ctor / dtor / SetModelIndex / SetPedState / ClearAll / Teleport / GiveWeapon /
      ClearWeapons / SetCurrentWeapon ×2; CPlayerInfo MakePlayerSafe / SetPlayerSkin /
      LoadPlayerSkin.
    * World and streaming: CWorld::Add / Remove; CPlaceable::SetPosn ×2 / SetHeading;
      CStreaming RequestModel / RemoveModel / LoadAllRequestedModels / LoadScene.
    * Camera: CCamera Restore / RestoreWithJumpCut /
      SetCameraDirectlyBehindForFollowPed_CamOnAString / TakeControl / SetWideScreenOn / Off.
    * Other: CRestart::OverrideNextRestart, and CFileMgr OpenFile / Read / ReadLine / Seek /
      CloseFile / LoadFile.
  * Output is `<GTA_DIR>/trace.jsonl`. It is buffered and flushed at every CGame::Process
    and every 250 ms. Each record has:
    * `seq`, `frame` (`CTimer::m_FrameCounter`), `t` (ms) and `fn`;
    * `caller` (module+offset of the return address) and `origin` (up to 3 client return
      addresses found on the stack);
    * `via` (`scm 0xNNNN` when the call happens inside a client script command,
      `stock <script>` inside a stock script);
    * `args` (values; pointers only as `*_ptr`).
  * Extra record types:
    * `scm`: decoded opcode + parameters. The exact length comes from the IP advance; the
      values of output variables are read after the call.
    * `watch`: per-frame diff of game globals: game state, pauses, menu, player
      ped/model/state/area/weapon slot, `DisablePlayerControls`, widescreen, fade, camera
      mode, HUD flags, active script names, script IPs, and whether stock scripts /
      CTheScripts::Process ran.
    * `globals`: changed main.scm global variables.
    * `pos`: every 30 frames.
    * `mark`: written by the scenario scripts.
  * Options (environment variables):
    * `SAMP_TRACER_PORT`: control channel port (default 47701).
    * `SAMP_TRACER_OUT`: trace file path.
    * `SAMP_TRACER_SCRIPTS=name,…|*` (+ `SAMP_TRACER_SCRIPT_FRAMES`): per-command
      `stock_cmd` records. It redirects the 27-entry SCM handler table at 0x8A6168 to
      thunks.
    * `SAMP_TRACER_CAPTURE_DIR`: rebuilds every file the game reads through CFileMgr.
    * `SAMP_TRACER_BOOT=1`: no client; boots a stock new game the way M1 does (CSV rows
      120, 121, 377, 378) for a baseline.
* DIFF-1: the tracer serves the samp-client control protocol (`ping`, `state`, `quit`,
  `peek`) from a background thread, plus tracer-only `mark TEXT`.
  * `state` has the same format as samp-client: `ticks` = CGame::Process count, and
    `frame_hook` = whether the tracer's CGame::Process hook is still in place. A host test
    pins the text to samp-client's own test fixture.
  * Verified on the original client: `state game_state=9 (IDLE) ticks=976
    ped=0x8d60020 pos=-1309.590,2528.375,87.625 heading=2.9922 health=100.0 armour=0.0
    frame_hook=installed`.
* `tools/trace/run.sh orig|ours [join|spawn]` and `run.sh bare [SECONDS]`. They use only the
  trace lane: `wine/prefix-trace`, `wine/gta-sa-trace`, UDP 7790, control port 47701,
  headless gamescope.
  * Steps: boot → join → class selection → Shift → spawn → walk 3 s (W) → quit, with
    `mark`s, screenshots, `state-*.txt` and a ScriptSpace dump.
  * Output goes to `target/trace/<client>-<stamp>/` (+ `<client>-latest`).
  * `ours` runs our client with `SAMP_RS_CTL_PORT=47701` and moves the tracer to 47702. It
    refuses to run until samp-client reads that variable.
* `tools/trace/diff.py LEFT RIGHT [--from MARK --to MARK --skip RE --only RE]` prints an
  aligned diff of normalised lines. It drops pointers, timing and caller offsets, rounds
  floats, names opcodes, and collapses repeated lines and repeated per-frame blocks.
  `--dump` prints one side.
* `tools/trace/peek_dump.py PORT ADDR LEN OUT`: memory dump through `peek`.
* TRACE-2 additions (details in "Remote players"):
  * `run.sh orig remote` starts `target/debug/samp-bot` as "RemoteBot" after the spawn. The bot is started and
    killed by PID only.
  * `tools/trace/remotefs.pwn` is compiled into the lane's server copy (`trace_server_start_remote`). It moves the
    bot next to the traced player, 15 s later moves it 1500 m away (stream-out), and 10 s after that brings it back
    (stream-in again). The bot leaves after 40 s.
  * `tools/trace/remote_report.py DIR` prints the four answers from a trace.
* TRACE-4 additions:
  * `run.sh orig|ours idle` (SP-OFF, `spoff.pwn`, `spoff_report.py`, `spoff_sites.py`);
  * `run.sh orig|ours vehicles` (`vehfs.pwn`, the `drive_bot` example, a network capture per client,
    `veh_report.py`);
  * the hooks and records listed in "SP-OFF baseline" and "Vehicles".

## Original client: results

The original client joins, selects a class, spawns and walks with the tracer loaded:
3 of 3 full scenario runs, plus 4 `join` runs. All 52 hooks were installed with no failures.
No hook site collides with samp's patches.

### How the stock story script is kept from running

**samp does not stop main.scm. It replaces it, together with 22 other data files.**

* `CTheScripts::Init` still opens "main.scm", and `CTheScripts::Process` and
  `CRunningScript::Process` run every frame. But the script loaded into
  `CTheScripts::ScriptSpace` is a different, small SCM:
  * its first jump is to 46276 (stock: 43808);
  * it ends at about 104 KB (stock: 2.9 MB);
  * segment 2 carries a "SANNY BUILDER 3.04" tag;
  * it has only 6 named threads: MAIN, INITIAL, INITIL2, HJ, INTMAN, CASHWIN.
* Of the files the game opens through `CFileMgr::OpenFile`, 23 never reach
  `CreateFileW`, while `samp.saa` is opened at boot (Wine `+file` trace). samp serves these
  files from its archive through a hook below `CFileMgr` (exact layer not identified).
* What the replacement script does. Per-command trace, `SAMP_TRACER_SCRIPTS=*`; frame
  numbers are game frames:
  * **Frame 0, MAIN:**
    * fade, mission/progress/respect totals, max wanted level, death/arrest state;
    * `SET_TIME_OF_DAY`, car and ped density, `REQUEST_COLLISION`, a few stats;
    * **`CREATE_PLAYER`** (→ `CPlayerPed::SetupPlayerPed(0)` → `CPlayerPed(0)` →
      `CPed::CPed(0)`, `SetModelIndex(0)`, `CWorld::Add`);
    * vital-stats button, tag status;
    * 30 × `SWITCH_ENTRY_EXIT`;
    * `GET_PLAYER_CHAR`, `SET_CHAR_HEADING`;
    * `LOAD_AND_LAUNCH_MISSION` (INITIAL: map objects, stunt jumps, garages).
  * **Frame 1, MAIN:**
    * a second mission (INITIL2);
    * many global-variable sets, `SWITCH_ROADS_OFF`, military-zone and SAM-site switches;
    * 16 × `REGISTER_STREAMED_SCRIPT`, 13 × `ALLOCATE_STREAMED_SCRIPT_TO_OBJECT`;
    * `REQUEST_IPL`, object visibility, attractor script brains;
    * `LAUNCH_MISSION` ×2 + `START_NEW_SCRIPT`. These are the helper threads: HJ, INTMAN,
      and one unnamed thread that clears help text and sets two float stats while the
      player is playing.
  * **Frame 2:**
    * `SWITCH_WORLD_PROCESSING`, fade colour, max wanted level, `FORCE_WEATHER_NOW`;
    * 4 clothes items, `INCREASE_PLAYER_MAX_HEALTH`, `BUILD_PLAYER_MODEL`;
    * `SET_AREA_VISIBLE`, `SET_PLAYER_CONTROL`, roads and ped roads off, densities;
    * `SET_FIXED_CAMERA_POSITION` / `POINT_CAMERA_AT_POINT`, `WAIT`.
  * **Frame 34:** `DO_FADE`, then MAIN loops forever over three `WAIT` blocks:
    * player-playing / area / stat / day / language / city / timer checks;
    * 5 × `IS_GARAGE_OPEN`;
    * `HAS_CHAR_GOT_WEAPON` with `MARK_STREAMED_SCRIPT_AS_NO_LONGER_NEEDED`.
  * No story mission is ever started.
* Stock baseline on the same boot path (`run.sh bare`): stock MAIN starts INITIAL and
  INITIL2, then about 30 threads in frame 2. The player stays at Grove Street.
* **Consequence for our client:**
  * We cannot ship samp.saa. We need our own minimal replacement main.scm, which needs a
    file-redirect hook like samp's or a way to load our SCM into ScriptSpace.
  * Or we keep the stock main.scm and stop it from starting story threads. That is
    harder, because stock MAIN also creates the player.
  * Minimum the replacement must do, from the observed behaviour: `CREATE_PLAYER` (the
    client's spawn code assumes `CWorld::Players[0].m_pPed` exists before frame 1), set
    densities 0 / roads off, give player control, and then idle.

### samp.saa data overrides

How this was captured: `SAMP_TRACER_CAPTURE_DIR`, comparing what the game read through
CFileMgr with the original client against a stock boot. Captures are local only
(`target/trace/saa-files/`, gitignored); differences are summarised in our own words.

| file | vs stock | difference |
|---|---|---|
| main.scm (+ the `data\script\main.scm` mission reads) | different | replacement script described above (≈104 KB, 6 threads) |
| DATA\SCRIPT\SCRIPT.IMG | different | 18 entries instead of 79: only object/ambient helper scripts (vending/slot machines, roulette, blackjack, video poker, ammu, carmod, parachute, …) |
| DATA\GTA.DAT | different | the car-recording IMG line is replaced by SAMP\SAMPCOL.IMG; SAMP\samp.IDE, SAMP\custom.IDE and SAMP\samp.ipl are added; one interior splash line is removed |
| DATA\DEFAULT.DAT | different | adds IMG lines for SAMP\CUSTOM.IMG, SAMP\SAMP.IMG, MODELS\GTA3.IMG, MODELS\GTA_INT.IMG (samp registers the archives itself, CSV row 128) |
| DATA\DEFAULT.IDE | different | the 20 cutscene-object slots (300…319) are removed |
| DATA\PEDS.IDE | different | 23 extra ped models (ids 1–6, 8, 42, 65, 86, 119, 149, 208, 265–273, 289: story characters and special peds) become ordinary skins |
| DATA\AR_STATS.DAT | different | every stat-increase amount is 0 (no stat growth from activity) |
| DATA\MAPS\LA\LAn2.IDE, LAw2.IDE, LAxref.IDE | different | 4 building models swapped for SA:MP's own (an office tower with glass and floors, a beach-side building) |
| DATA\MAPS\interior\props.IDE | different | 10 shop-shelf/cooler objects lose the 0x4 object flag |
| stream.ini | different | streaming memory 13500 → 256000, vehicles 12 → 20 |
| HANDLING.CFG, DATA\WEAPON.DAT, DATA\melee.dat, DATA\OBJECT.DAT, DATA\PED.DAT, DATA\VEHICLES.IDE, DATA\CARMODS.DAT, DATA\shopping.dat, DATA\surface.dat, TIMECYC.DAT | identical | served from the archive but byte-identical to the stock files |

Not in the redirect list, but noticed: `data\paths\tracks2.dat` / `tracks4.dat` return only
1 byte under samp.

### Boot (state 5 → 9), frame 0

1. `CPlayerInfo::LoadPlayerSkin`.
2. `CTheScripts::Init` (main.scm, served by samp).
3. `StartTestScript`, then `StartNewScript(offset 0)` (MAIN).
4. MAIN runs once during loading: it creates the player, then
   `DeactivatePlayerPed` + `SetPosn` + `ReactivatePlayerPed` + `SetHeading`, and launches
   INITIAL.

Frame 1 (first CGame::Process):
* `scm 0x087E SET_CHAR_DROPS_WEAPONS_WHEN_DEAD(1,1)`;
* `0x0446 SET_CHAR_SUFFERS_CRITICAL_HITS(1,0)`;
* samp's script runs (frame 2: `CPed::SetModelIndex(0)` re-add,
  `MakePlayerSafe(false)`).

### Connecting screen (until the server answers), every frame

samp's per-frame code, all as single script commands (`scm`) unless marked direct:
* `0x00C0 SET_TIME_OF_DAY(12,0)`;
* pre-loading: `0x04EE`/`0x04ED` for 9 anim blocks (PARACHUTE, BAR, GANGS, PAULNMAC,
  VENDING, FIGHT_B…E);
* `0x0248`/`0x0247` for models 3131, 1485, 1486, 1543, 1546, 343, 342, 344, 330;
* direct `CPed::Teleport` of the player to (1133.05, -2038.40, 69.1);
* `0x0925`, `0x015F SET_FIXED_CAMERA_POSITION(1093,-2036,90)`, `0x0925`,
  `0x0160 POINT_CAMERA_AT_POINT(384,-1557,20, 2)`;
* `0x09BA DISPLAY_ZONE_NAMES(0)` once.

### Join (InitGame) → class selection

In one frame:
1. `0x0249 MARK_MODEL_AS_NO_LONGER_NEEDED(596)`.
2. `CPed::SetPedState(player, 1)`.
3. `0x01B4 SET_PLAYER_CONTROL(0,0)` → `MakePlayerSafe(true,10)`.
4. `0x04D7 FREEZE_CHAR_POSITION(ped,1)`.

Next frame:
* `DisablePlayerControls` becomes 32;
* direct `CPed::Teleport` to the class-selection position from the server;
* direct `CPed::ClearWeapons`, `SetCurrentWeapon(slot 0)`;
* direct `CPed::SetModelIndex(class skin)`.

Each later class request repeats ClearWeapons + SetModelIndex. The camera comes from the
server's camera RPCs via `0x015F`/`0x0160`.

### Spawn (Shift → RPC 129), in order, one frame

1. `0x02EB RESTORE_CAMERA_JUMPCUT` → `CCamera::RestoreWithJumpCut`
2. `0x0930 CAMERA_PERSIST_POS(0)`, `0x0925 CAMERA_RESET_NEW_SCRIPTABLES`
3. `0x0373 SET_CAMERA_BEHIND_PLAYER` → `SetCameraDirectlyBehindForFollowPed_CamOnAString`
4. `0x02EB` again
5. `0x01B4 SET_PLAYER_CONTROL(0,1)` → `MakePlayerSafe(false,10)`; `0x04D7 FREEZE_CHAR_POSITION(ped,0)`
6. direct `CPed::Teleport` to its current position (`CWorld::Remove`/`Add`)
7. `0x04E4 REQUEST_COLLISION(x,y)` (→ ~13 `RequestModel` flags 24)
8. `0x016C ADD_HOSPITAL_RESTART(x,y,z,rot,0)` (build B; build A writes the restart arrays directly)
9. direct `CPed::SetModelIndex(skin)`, direct `CPed::ClearWeapons` (→ `SetCurrentWeapon(slot 0)`)
10. `0x06D7 SWITCH_RANDOM_TRAINS(0)`
11. direct `CPed::Teleport(x, y, z+0.5)`
12. `0x0173 SET_CHAR_HEADING(ped, deg)` → `CPlaceable::SetHeading`
13. `0x0164 REMOVE_BLIP` ×2

Next frame:
* `0x01B9 SET_CURRENT_CHAR_WEAPON(ped,0)`;
* teleport + heading again (the server's spawn position);
* camera reset again (`0930`, `0925`, `0373`, `02EB`);
* weapon models via `0x0247` + `0x038B LOAD_ALL_MODELS_NOW`;
* `CPed::GiveWeapon` / `SetCurrentWeapon` for the class weapons.

`DisablePlayerControls` is back to 0 and the camera mode becomes 4 (follow ped).

### Walking

No GTA calls of interest: samp only reads state, plus its per-frame commands
(`0x077E GET_AREA_VISIBLE`, `0x0470 GET_CURRENT_CHAR_WEAPON`,
`0x052C SET_PLAYER_DRUNKENNESS(0,0)`). The ped moves under normal pad control.

## Remote players (TRACE-2)

What the original client does with a remote player's ped. Everything here was measured at runtime with samp.dll
0.3.7-R4 (build B) and described in our own words. Static background: `docs/re/subsystems/remote-control-onfoot.md`
and `remote-control-combat.md`. Those are reverse-engineered from the code; this section confirms them with
measurements and refines them.

**Runs.** All are `SAMP_TRACER_GUARD=400 tools/trace/run.sh orig remote`, about 52 fps, with the bot sending on-foot
sync every 30 ms.
* `target/trace/orig-20261104-195734`: the full scenario (stream-in, walk, stream-out, stream-in again, quit).
* `orig-20261104-195115`: stream-in, walk, quit.
* `orig-20261104-194723`: the same without guard pages.

The traced player stands still. The bot walks a 3 m circle with its left stick fully forward.

### Tracer additions

* **New hooks:**
  * `CPlayerPed::ProcessControl` 0x60EA90, inline and wrapped;
  * `CWorld::Process` 0x5684A0, wrapped;
  * `CTaskManager::SetTask` 0x681AF0 and `SetTaskSecondary` 0x681B60. These log the owner ped and the task's vtable
    (the class is named via gta-reversed `RH_ScopedVirtualClass`).
* **Player peds in `Relevant` hooks:** these hooks now also log calls that involve any player ped, not only the
  local one. Entities carry `"slot":N` when they are `CWorld::Players[N].m_pPed`. The remote-ped cache is refreshed
  at every frame boundary, and destroyed peds are excluded.
* **`pc` record (one per call):** samp redirects the CPlayerPed vtable slot 0x86D190 to its own function. At the
  first frame the tracer puts an outer wrapper in front of that function (`vtable_wrap` record).
  * `pre.before` / `pre.after`: samples taken before samp's function runs and after it returns.
  * `in` / `out`: samples taken around GTA's original function.
  * Each sample holds:
    * `CWorld::PlayerInFocus`;
    * the non-zero fields of `CPad::Pads[0].NewState`;
    * camera mode, weapon-camera mode, FOV, aspect ratio, the pistol-skill stat and camera 0's front vector;
    * the raw bytes at the `CPed::UpdatePosition` call site 0x5E92F4 and at the two aiming-rotation stores
      0x6884C4 / 0x688200;
    * the ped's position, move speed, heading, heading goal, ped state and move state.
  * Remote peds are logged on every call, the local ped every 30 frames.
* **`rped` record (per remote ped, per frame):** position, speed and heading at `CGame::Process` entry
  (`frame_start`) and around `CWorld::Process` (`world_in` / `world_out`).
* **`players` record:** the slot → ped list, written when it changes. **`players_mem`:** a per-frame diff of
  `Players[2..209]` memory.
* **`SAMP_TRACER_GUARD=<budget>`:** the pages 0xB7D000..0xB92000 (the tail of `Players[1]` plus all of
  `Players[2..209]`) get PAGE_GUARD. A vectored handler records the instruction address of every access,
  single-steps it and re-arms the page. `guard` is written for each new (instruction, slot, offset, R/W) site, and
  `guard_summary` every 300 frames and at quit. In the runs above the budget was never reached (0 frames over
  budget), so every access in the range was recorded.
  Use the guard option only with a client that relocates the sector array. Without the relocation that range is
  GTA's live sector grid, and the guard pages would fire constantly.

### 1. Stream-in (one frame)

The run had two stream-ins, at frames 747 and 2073, identical apart from pointers and handles. All of it happens
in one game frame and goes through samp's script-command executor (`scm` records), except where marked direct:

1. `0x0053 CREATE_PLAYER(2, x, y, z)` at the stream-in position, with z as received. The player index is the
   first free slot, 2. Inside GTA, in order:
   1. `CPlayerPed::SetupPlayerPed(2)` → `CPlayerPed(2, false)` (no ped group) → `CPed(ped type 0)`.
   2. `GiveWeapon(0, 0)`.
   3. `SetTask(CTaskSimpleStandStill, primary 4)`.
   4. `SetModelIndex(0)` (the special player model).
   5. `SetPedState(1)`.
   6. `SetTask(CTaskSimplePlayerOnFoot, 4)`.
   7. `SetTaskSecondary(CTaskComplexFacial, 3)`.
   8. `SetPedState(1)`, `CWorld::Add`.
   9. The opcode's own placement: `DeactivatePlayerPed(2)` (`CWorld::Remove`), `SetPosn(x, y, z + 1)`,
      `ReactivatePlayerPed(2)` (`CWorld::Add`).
   10. `SetTask(CTaskSimplePlayerOnFoot, 4)` again.
2. `0x01F5 GET_PLAYER_CHAR(2)` → ped handle (257 the first time, 513 the second).
3. On that handle:
   * `0x087E SET_CHAR_DROPS_WEAPONS_WHEN_DEAD(1)`;
   * `0x02AB SET_CHAR_PROOFS(0,0,1,0,0)`;
   * `0x0446 SET_CHAR_SUFFERS_CRITICAL_HITS(0)`;
   * `0x03FE SET_CHAR_MONEY(0)` twice.
4. Skin: `0x0248 HAS_MODEL_LOADED(skin)`, then a **direct** `CPed::SetModelIndex(skin)` (caller samp.dll+0x9f02b).
   The skin was 0 and already loaded, so no `RequestModel` / load call appeared.
5. Heading, the z correction of the matrix and the pad are written to memory directly. No GTA function is called
   for them: no `SetHeading`, `Teleport` or `SetPosn` from samp.
6. The same frame then starts the per-frame work: `0x06BD IS_LINE_OF_SIGHT_CLEAR` ×2 from the remote ped's head to
   the camera position.
7. Within the next frames, once: `CTaskManager::SetTaskSecondary(CTaskSimpleIKManager, 5)`, reached through
   `CIKChainManager::LookAt` (GTA 0x618A07) from samp code. This is a head look-at.
8. `Players[2]` memory is changed only by the GTA constructor code:
   * `m_pPed` (+0x0);
   * the CPlayerPedData fields +0x4…+0xA4 (+0x4, +0x8, +0x18, +0x1C, +0x28, +0x38, +0x44, +0x48, +0x50, +0x90,
     +0xA0, +0xA4).
   The whole range `Players[2..209]` was zero from frame 1 on, so the old sector array is cleared before the first
   CREATE_PLAYER.

### 2. Every frame while the bot walks

These figures are from the 1606 frames with a remote ped in the full run (2084 frames in the walk-only run).

* **No GTA function calls for the remote ped.** There is no `Teleport`, `SetPosn`, `SetHeading`, task call or
  model call. samp's only per-frame commands that involve the remote ped:
  * `0x06BD IS_LINE_OF_SIGHT_CLEAR` twice per frame, from the remote ped's position + (0, 0, 0.85) to the camera
    position (3.5 m behind the local ped) (Inferred: name-tag visibility);
  * `0x0470 GET_CURRENT_CHAR_WEAPON(remote handle)` about every other frame.
* **Direct writes, all between the end of `CWorld::Process` (frame N) and the next `CGame::Process` (frame N+1),
  never inside the world update:**
  * heading `+0x558` and heading goal `+0x55C`: every frame (1604 of 1604), about 0.01 rad per frame, so the
    rotation is smooth;
  * move speed `m_vecMoveSpeed` (+0x44): in 901 of 1604 frames, with gaps of 1–2 frames, which matches the arrival
    of 207 packets. The value is the synced velocity plus a small correction; for example the synced 0.022 became
    0.020;
  * position (matrix translation): only on large errors, as a raw jump with no GTA call (no `CWorld::Remove`/`Add`):
    * 3.0 m when the bot's own reported position jumped by 3 m;
    * 14.3 m when the server moved the bot;
    * 1500 m when the bot was sent away, about 55 frames before its stream-out.
  * While the ped walks normally, its position comes only from GTA's physics, which integrates the move speed
    inside `CWorld::Process`.
* **ProcessControl:** samp's vtable function (samp.dll+0xa2970) calls GTA's 0x60EA90 from samp.dll+0xa280a for
  remote peds and from +0xa2c50 for the local ped. During every remote call (1606 of 1606) the context is:

  | value | before samp's wrapper / after it | inside GTA's ProcessControl (remote) |
  |---|---|---|
  | `CWorld::PlayerInFocus` | 0 | **2** (the slot) |
  | `CPad::Pads[0].NewState` | local pad (empty) | **remote keys** (LeftStickY = −128 from the sync; empty only on the first frame) |
  | pistol-skill stat (`StatTypesFloat[69]`) | 1000 | **999** (per-slot default) |
  | camera 0 front vector | local camera | **(0,0,0)** (per-slot aim block) |
  | call at 0x5E92F4 (`CPed::UpdatePosition`) | `e8 17 88 ff ff` | **NOPed** (`90`×5) |
  | aiming-rotation stores 0x6884C4 / 0x688200 | NOPed | NOPed. They are restored (`d9 96 5c 05 00 00`) only inside the **local** ped's call |
  | camera mode / weapon mode / FOV / aspect | 4 / 0 / 70 / 1.333 | same values. The per-slot defaults (mode 4, zoom 1 → 70, aspect 0.333 + 1) equal the local ones, so a swap cannot be seen here |

  Everything is restored after samp's wrapper returns. GTA's ProcessControl changes only the move speed
  (gravity/collision) and never the position or heading.
* For a reimplementation, a remote ped needs exactly two kinds of per-frame work:
  1. In the ProcessControl wrapper: swap in PlayerInFocus, the pad, the skills and the aim block, and NOP
     `UpdatePosition`.
  2. Outside the world update: write heading/heading goal (interpolated), move speed (on each sync) and the
     matrix translation (only on large errors).

### 3. Stream-out and quit

Stream-out (the bot moved 1500 m away, frame 1599) and quit (the bot disconnects, frame 2827) use the same samp
routine. Only the outer caller differs: samp.dll+0x17504 for stream-out, +0x16653 for quit, both through
+0xb13e8 → +0xb12e0. It runs in samp's window after `CWorld::Process`, in one frame, with no script commands. In
order:

1. samp writes `Players[2] + 0x50` := 0. This is the ped's `m_pPlayerData + 0x4C`, written through the ped's own
   pointer (guard record: samp.dll+0xb10b3).
2. samp deletes the ped through its virtual deleting destructor:
   1. `~CPlayerPed` (GTA 0x6093B0…) reads `Players[2]` +0x4C/+0x50/+0x3C;
   2. `CPed::~CPed` → `CWorld::Remove`, `ClearWeapons` → `SetCurrentWeapon(slot 0)` (writes +0x24).
   No `RemovePlayerPed`, no `RemoveModel`, no blip or object commands (the bot had none).
3. **`Players[2].m_pPed` is not cleared.** Over the whole run +0x0 was written only by the GTA constructor
   (0x60D7EA):
   * 0 → ped A at the first stream-in;
   * ped A → ped B at the second stream-in (frame 2073).
   Between the two the pointer dangles.
4. On the next stream-in the slot is reused (first free slot: 2 again), with a new ped and a new handle.

### 4. `CWorld::Players[slot]` beyond index 1

With guard pages over 0xB7D000..0xB92000 for 3174 frames there were 104 402 accesses: 104 400 from gta_sa.exe and
2 from samp.dll.

* **samp.dll's only access is the destruction write above:** samp.dll+0xb10b3, W `Players[2]+0x50`, once per
  destroyed ped. samp never forms `Players[]` addresses from 0xB7CD98. It relies on GTA's own indexing:
  `CREATE_PLAYER` / `SetupPlayerPed(slot)`, `m_pPlayerData`, and `Players[PlayerInFocus]` while it is swapped.
* **GTA code accesses (156 distinct sites, all in slot 2):**
  * the CPlayerPed constructor and `SetupPlayerPed` (writes +0x0 … +0xA4);
  * `FindPlayerVehicle` 0x56E0E5 and `FindPlayerPed` 0x56E225, which read `Players[2].m_pPed` about 5200 times in
    total (Inferred: through `Players[PlayerInFocus]` while it is 2; the first hits were recorded with
    PlayerInFocus = 2);
  * CPlayerPed / ProcessControl / `ProcessAnimGroups` code, which reads and writes CPlayerPedData fields
    (+0x4…+0x92) on every remote ProcessControl call.
* **Slot 1:** only GTA's own reads of its last bytes (+0x14F / +0x158) occur, every frame and with PlayerInFocus 0.
  This is GTA code for the second player.
* **Nothing else:** no access to slots 3…209 or to any other part of the old sector array up to 0xB92000.
* **Conclusion:** this confirms the overlap. `Players[2]` = 0xB7D0B8 = the old `CWorld::ms_aSectors` base, stride
  0x190, and 210 slots end at 0xB915B8. The area is only usable because samp relocates the sector array and zeroes
  it beforehand. Note: `remote-control-onfoot.md` §1.3 gives the end as 0xB91638. The correct value is 0xB7CD98 +
  210 × 0x190 = 0xB915B8.

## Our client (samp_client.dll, first trace)

`tools/trace/run.sh ours spawn` passed: join, class selection, spawn and a 3 s walk. Our
client served `SAMP_RS_CTL_PORT=47701`, the tracer served 47702, and the auth table came
from `data/auth_table.txt`.

**DIFF-1 check:** in the same run, our client's `state` and the tracer's `state` print the
same text. Only `ticks` differs, because the two count different hooks. Example:
`… pos=-1321.073,2526.114,87.438 heading=-3.0832 health=100.0 armour=0.0 frame_hook=installed`.

`diff.py target/trace/orig-<run> target/trace/ours-latest` (2026-11-04, normalised):

* **Boot:**
  * Same script lifecycle (Init, StartTestScript, StartNewScript(0)), with our main.scm
    served by our client.
  * Ours lacks `0x087E SET_CHAR_DROPS_WEAPONS_WHEN_DEAD(1,1)` and
    `0x0446 SET_CHAR_SUFFERS_CRITICAL_HITS(1,0)` in frame 1.
  * Under our client the game loads `data\paths\tracks*.dat` through `CFileMgr::LoadFile`
    (46384 bytes each). samp's versions are 1 byte.
* **Connecting screen:** ours lacks samp's per-frame block: teleport to (1133, -2038, 69.1),
  the fixed camera (`0x015F`/`0x0160` with `0x0925`), and the anim/model pre-loading
  polls.
* **Join:** ours lacks `0x0249 MARK_MODEL_AS_NO_LONGER_NEEDED(596)`. The rest of the join
  frame matches: `SetPedState`, `SET_PLAYER_CONTROL(0,0)`, `FREEZE_CHAR_POSITION`,
  teleport, `ClearWeapons`. Ours does not call `CPed::SetModelIndex` when the class skin
  equals the current model; samp always calls it.
* **Spawn:** the first 15 steps match the original order. Differences:
  * no `CPed::SetModelIndex(skin)`;
  * `0x01B9 SET_CURRENT_CHAR_WEAPON(ped,0)` is issued in the spawn frame instead of the
    next frame;
  * no `0x0164 REMOVE_BLIP` ×2;
  * the second camera reset (`0930`/`0925`/`0373`/`02EB`) runs in the spawn frame, while
    samp runs it in the next one.
  * The weapons match: `GiveWeapon` 4 then 22. Ours omits the intermediate
    `SET_CURRENT_CHAR_WEAPON(ped,4)`.
* **Walking:** no differences of interest.

## M3 re-diff: remote players, ours vs original (TRACE-4 part 1)

How it was measured: `SAMP_TRACER_GUARD=400 tools/trace/run.sh ours remote` with the M3 build
(`target/trace/ours-20261104-200349`). It was compared with `orig-20261104-195734` using
`remote_report.py` and per-frame statistics of the `rped` and `pc` records.

**Matches:**
* **Stream-in.** The order of GTA calls inside `CREATE_PLAYER(2)` is identical. So is the
  command sequence that follows: `GET_PLAYER_CHAR`, `087E`, `02AB(0,0,1,0,0)`, `0446(0)`,
  `03FE` ×2, `HAS_MODEL_LOADED(0)`, then a direct `CPed::SetModelIndex`. All of it happens in
  one frame.
* **ProcessControl swap.** Inside GTA's 0x60EA90, on every remote call (1779 of 1779):
  * PlayerInFocus = 2;
  * the pad holds the remote keys (empty only on the first frame);
  * pistol skill = 999 and camera front = (0, 0, 0);
  * `UpdatePosition` is NOPed and the aim stores are NOPed.
  The local call restores the aim stores, and everything is restored after the wrapper.
* **Write timing.** Heading and heading goal are written every frame, and the move speed when
  a sync arrives. All of these writes happen between `CWorld::Process` and the next frame. The
  matrix is written only on large jumps (the 1500 m jump came 48 frames before stream-out; the
  original: 55).
* **Destruction.** It happens in the same place in the frame (after the world update), with
  the same sequence: `~CPed` → `CWorld::Remove` → `ClearWeapons` → `SetCurrentWeapon(0)`. The
  write to `Players[2]+0x50` matches. Slot 2 is reused at the next stream-in, and slots 3 and
  up are never touched.

**Differences:**
1. Missing per-frame commands:
   * `0x06BD IS_LINE_OF_SIGHT_CLEAR` ×2 per frame (name tags, M8a);
   * `0x0470 GET_CURRENT_CHAR_WEAPON`, on the remote handle about every other frame and on the
     local one;
   * the background `0x077E`, `0x052C` and `0x09BA`.
2. No head look-at. The original calls `CIKChainManager::LookAt` once after stream-in, which
   sets `SetTaskSecondary(CTaskSimpleIKManager, 5)`.
3. Move-speed magnitude. The original writes |v| with a median of 0.050 (synced velocity 0.030
   plus a large pull toward the position). Ours writes a median of 0.0304, which is close to the
   bare synced velocity. The path radius on the bot's 3 m circle is 2.48 for the original and
   2.85 for ours.
4. Cadence of applying syncs. The original applies a sync on 901 of 1606 frames (gaps of
   2 frames: 643, 1 frame: 232, 3 frames: 20). Ours applies one on 795 of 1779 frames (gaps of
   2: 533, 3: 95, 4: 64, 1: 89), so ours is burstier.
5. Ours clears `Players[2].m_pPed` (a decided deviation).
6. Ours passes the slot of `CREATE_PLAYER` and `GET_PLAYER_CHAR` as an immediate value; samp
   uses local variables (cosmetic).
7. **Cops arrested the local player** (an SP-OFF gap, see below). From frame 1746, 10 `CCopPed`
   were created (SWAT 285 with weapon 28, and 283). At frame 2452 the local ped state became 55
   (arrested), with camera mode 29 for 180 frames, and then GTA's restart path ran. Because of
   this the second remote handle was 1537 instead of 513.
8. After that restart, ours alternates `SET_CURRENT_CHAR_WEAPON(1, 22)` and `(1, 4)` every 2–3
   frames.
9. The local pistol-skill stat is 0 in ours; the original sets 1000 after the spawn.

## SP-OFF baseline: single-player population and police (TRACE-4 part 2)

**Runs** (all 5 minutes of idling after the spawn):

* `TRACE_IDLE_SECS=300 tools/trace/run.sh orig|ours idle`
  * `tools/trace/spoff.pwn` lifts the gungame world bounds and puts the player in Grove Street
    (2495, −1675).
  * After 300 s it moves the player in front of the Alhambra entry-exit ("BAR1"). The scenario
    walks the player in with W and idles 30 s inside (area 17).
* `tools/trace/run.sh bare 300`: the stock new game at Grove Street. The intro mission starts
  after about 115 s.
* Reports: `tools/trace/spoff_report.py DIR [--from grove]`. Live patch bytes:
  `tools/trace/spoff_sites.py` (`spoff-sites.txt` in each run directory).

**Tracer additions:**
* 29 hooks:
  * ped/vehicle constructors: `CCopPed`, `CVehicle`;
  * the CPopulation generators;
  * the CCarCtrl, car-generator, plane, train and roadblock generators;
  * the CWanted setters;
  * the police scanner;
  * `CEntryExit::GenerateAmbientPeds` and `CPopulation::PopulateInterior`.
* Per-frame generators are `Count` hooks, reported on/off in `watch`.
* Creation records carry `gta_stack`: up to 6 gta_sa.exe return addresses.
* `watch` also reports:
  * ped and vehicle pool usage;
  * `CPopulation` / `CCarCtrl` counters;
  * the density multipliers;
  * `CWanted::MaximumWantedLevel`;
  * the local player's wanted level and chaos.
* Hooks that are deliberately not used:
  * `CPopulation::AddPed`: samp rewrites its entry.
  * `CCivilianPed::CCivilianPed`: a rel32 points into its first bytes.
  * `CCarCtrl::GenerateOneRandomCar` and `GenerateOneEmergencyServicesCar`: the exe's protection
    stubs read their entry bytes as data. A hook on 0x42B7D0 broke the decryption at 0x403796
    and crashed `SET_PLAYER_CONTROL`.
* New test `no_absolute_reference_into_stolen_bytes` scans the whole image for such
  references.

| per 5 min after the spawn | original | ours (M3 build) | stock (bare) |
|---|---|---|---|
| peds created (excluding players) | **0** | 21 cops (12 on foot via `AddPed`, 9 SWAT from police helicopters) + 8 interior peds | 128 (civilians, gangs, 6 cops) |
| vehicles created | **0** | 11 police helicopters (created_by PERMANENT) + 5 parked cars before the spawn | 54 (24 parked, 12 random traffic, 7 mission, 9 train cars, …) |
| `MaximumWantedLevel` | **0** (`SET_MAX_WANTED_LEVEL` from samp's main.scm in frames 0 and 2) | 6 | 6, then 4 (story script) |
| wanted level | 0 | **4** (set in frame 18 by `CGameLogic::SetPlayerWantedLevelForForbiddenTerritories`: the class-selection position 1958, 1343 is in Las Venturas, a locked city) | 0 |
| arrests | 0 | 4 (ped state 54 → 55, camera mode 29, restart) | 0 |
| `CCarCtrl::GenerateRandomCars` | never called (CSV row 189 NOP) | never called (`CGame::Process` skips it when the streaming update takes ≥ 4 ms; not NOPed) | every frame |
| `CTheCarGenerators::Process` | never (row 181 NOP) | **every frame** | every frame |
| `CPlane` / `CTrain` generation | never | never | 65 % of frames |
| `CPopulation::Update` / `ManagePopulation` | every frame (not patched) | every frame | every frame |
| `CPopulation::AddToPopulation` | 0 frames after the spawn (ped density 0) | 4 frames | 65 % of frames |
| `CRoadBlocks::GenerateRoadBlocks`, `CWanted::Update` | every frame (harmless at wanted level 0) | every frame | every frame |
| interior entry (`TransitionFinished`) | no `GenerateAmbientPeds` (row 164 NOPs the call) | `GenerateAmbientPeds` → `PopulateInterior(9)` → 8 dancers/patrons | — |
| police scanner (`AddAudioEvent` / dialogue) | 0 | 0 | 0 |
| ped / car density | 0 / 0 | 0 / 0 | 1 / 1 |

**What samp neutralises, and how** (live bytes from `spoff-sites.txt`; CSV rows of
`gta-patches-r4-annotated.csv`):
* **By code patches at init:**
  * row 189: the `GenerateRandomCars` call in `CGame::Process`;
  * row 181: the `CTheCarGenerators::Process` call;
  * row 182: the plane generation inside `GenerateRandomCars`;
  * row 185: `CPopulation::AddPed` → `xor eax,eax; ret`;
  * row 186: the `SetPedInCarDirect` call in `AddPedInCar`;
  * row 164: the `GenerateAmbientPeds` call in `CEntryExit::TransitionFinished`;
  * row 55: the police scanner's `FindPlayerPed` call;
  * row 246: cutscenes;
  * row 163: replay.
* **By its replacement main.scm:**
  * `SET_MAX_WANTED_LEVEL 0`, so the wanted level is capped at 0 (this also neutralises the
    forbidden-territory wanted level);
  * ped and car density 0;
  * `SWITCH_ROADS_OFF` / ped roads off;
  * `SWITCH_RANDOM_TRAINS(0)` at spawn.
* **Not patched, still running every frame:** `CPopulation::Update` / `ManagePopulation`,
  `CWanted::Update`, `CRoadBlocks::GenerateRoadBlocks`. With density 0, maximum wanted level 0
  and `AddPed` returning 0, they create nothing.

**SP-OFF acceptance gaps of our client** (all measured in `ours-20261104-203122`):
1. `MaximumWantedLevel` is still 6. The original sets 0 through its script. Result:
   * wanted level 4 for locked cities;
   * police helicopters with SWAT rappelling, cops on foot;
   * an arrest about every 60 s.
   This also explains the arrest seen in the M3 re-diff.
2. Row 181 is missing: `CTheCarGenerators::Process` runs, and 5 parked cars were created during
   class selection.
3. Row 185 is missing: `CPopulation::AddPed` creates peds. These were the 12 cops on foot. With
   density 0 the only other route is attractor/interior population.
4. Row 164 is missing: entering an interior generates ambient peds (8 in the Alhambra).
5. Rows 189 and 182 are missing. `GenerateRandomCars` happened not to run in our trace (timing
   gate), but it is not disabled.
6. Rows 186, 55 and 163 (replay) are missing. No effect was seen in this run, because nothing
   was created in cars and no crime was reported.
7. Already present in ours: row 246 (cutscenes), row 202 (wanted HUD), densities 0, roads off.

## Vehicles (TRACE-4)

What the original client does with vehicles. Everything was measured at runtime with samp.dll 0.3.7-R4 (build B)
and is described in our own words. Static background: `docs/re/subsystems/remote-control-vehicles.md` (build A).
This section confirms that spec at runtime and refines it.

**Runs:** `tools/trace/run.sh orig vehicles`.
* `target/trace/orig-20261104-204143`: the full scenario.
* `orig-20261104-203744`: the local exit failed (F was too short).
* Per-frame figures agree within a few frames between the two runs. The local exit and the network figures come
  from the first one.

**Scenario** (`tools/trace/vehfs.pwn`, LS airport tarmac):
* The traced player stands next to a Sultan (car A). It enters with F, drives 4 s with W and exits with F.
* Then `target/debug/examples/drive_bot` (new samp-bot example) joins as "DriveBot". The server creates car B 40 m
  away and calls `PutPlayerInVehicle(bot, B, 0)`.
* The bot sends driver sync (200) every 30 ms on a circle: radius 10 m, 0.5 rad/s, synced velocity 0.1 per frame,
  accelerate key held, steering −64.

**Timeline after the bot's spawn:**
* +21.5 s `RemovePlayerFromVehicle`. The bot then sends foot sync.
* +26.5 s put back in the car.
* +31.5 s the traced player is moved 1500 m away (stream-out of both cars and the bot).
* +41.5 s the player is brought back (stream-in again).
* +51.5 s `DestroyVehicle(B)` with the bot inside.

**Tools:**
* Loopback traffic is captured to `net.pcapng`, split per client port and decoded with samp-dump (`net-<port>.log`;
  the game's file is the one without "DriveBot" in its RPC 25).
* `tools/trace/veh_report.py DIR` prints the report.

**Tracer additions:**
* `CAutomobile::ProcessControl` (0x6B1880) gets an inline hook (inner). There is also an outer wrapper in front of
  samp's CAutomobile vtable slot 0x871148. Together they produce one `vpc` record per call on a mission car:
  * the samples `pre.before` / `in` / `out` / `pre.after`;
  * PlayerInFocus, `Pads[0]`, camera mode / weapon mode / look-left-right / front;
  * the car's position, speed, turn speed, forward vector, status, driver, health, steer/gas/brake and raw flags;
  * `orig_called: false` when samp skipped GTA's function.
* An `rveh` record per mission car per frame, at frame start, `world_in` and `world_out`.

### 1. Stream-in (RPC 164), one frame, all through script commands

1. Model: `0x0248 HAS_MODEL_LOADED(560)`.
   * If the model is missing: `0x0247 REQUEST_MODEL(560)`, `0x038B LOAD_ALL_MODELS_NOW`
     (→ `CStreaming::LoadAllRequestedModels`), then `0x0248` again in the same frame. That is the busy-wait; one poll
     was enough here.
   * If the model is loaded (car B, and the second stream-in): only `0x0248`.
2. `0x00A5 CREATE_CAR(560, x, y, z)` → `CVehicle::CVehicle(created_by 2 = MISSION)`, `CWorld::Add`. Handles were 1,
   257, 258, 513.
3. `0x0175 SET_CAR_HEADING(h, angle)` → `CPlaceable::SetHeading`. On the second stream-in the angle is the server's
   current angle (91.77).
4. `0x09C4 SET_PETROL_TANK_WEAKPOINT(h, 0)`, `0x07FF SET_CAR_HYDRAULICS(h, 0)`, `0x053F SET_CAN_BURST_CAR_TYRES(h, 0)`.
5. The z of the matrix is raised by 0.25 with no GTA call: CREATE_CAR at z 13.55 gives the car at z 13.80 in the next
   frame (13.259 → 13.509 for the second stream-in of car A). The door lock reads 0.
6. When a player and a car stream in together (RPC 164 followed by RPC 32 in the same packet), `CREATE_CAR` comes
   first and `CREATE_PLAYER` follows in the same frame.
7. Unoccupied sync: right after car A streamed in, the local client sent 4 × packet 209 for it, 150 ms apart, and
   then stopped (the car had settled).

### 2. Every frame, for every vehicle (samp's pool tick)

* For each car: `0x02AC SET_CAR_PROOFS(h, 1,1,1,1,1)` and `0x053F SET_CAN_BURST_CAR_TYRES(h, 0)`. When the local ped
  drives the car, these become `(h, 0,0,0,0,0)` and `(h, 1)`.
  * This is one pair per car per frame (e.g. 259 frames → 259 pairs per car).
  * The switch happens as soon as GTA seats the local ped, 6 frames before the server reports state 2. It switches
    back on exit.
* `0x02BF IS_CAR_IN_WATER(h)` every 4th frame (the sunk check).
* Blips: on the local enter, `0x0164 REMOVE_BLIP`. On the exit, `0x0161` (blip for the car, handle 196610) and then
  `0x0165` (blip colour 1004). This is samp's own vehicle-marker handling.

### 3. Remote driver

* **Seating:**
  * The server's `PutPlayerInVehicle` for the bot is **not** relayed as an animation; no RPC 26/154 reached the game.
  * When the first driver sync arrives (44–46 frames after `CREATE_PLAYER`), samp runs `0x036A WARP_CHAR_INTO_CAR(ped, car)`
    → `CTaskSimpleCarDrive`.
  * After `RemovePlayerFromVehicle`, the bot sends foot sync. samp then runs
    `0x0362 WARP_CHAR_FROM_CAR_TO_COORD(ped, x, y, z)`. The coordinates are **the ped's own current position**, not the
    car's position + 2.
  * A later driver sync seats the ped again with `0x036A`.
* **ProcessControl swap** (1886 of 1886 calls with the remote driver). Before samp's handler and after it the game
  context is the local one: PlayerInFocus 0, local pad, camera mode 18 while the local player drives and 4 on foot.
  Inside GTA's `CAutomobile::ProcessControl`:
  * PlayerInFocus = **2** (the slot);
  * `Pads[0]` holds **the remote keys**: LeftStickX = −64 (steering) and ButtonCross = 255 (accelerate, from
    KEY_SPRINT);
  * camera mode **4** and camera front **(0, 0, 0)** (per-slot defaults);
  * look-left/right 0;
  * car status 0 (STATUS_PLAYER).
* **z protection:** samp changes the car's z or move-speed z after GTA's function only when the car is far away or
  has no collision. It happened in every call while the local player was 1500 m away (69–74 calls) and never
  while it was near; the few calls counted just before the `local_away` mark are mark lag.
* **Unoccupied cars:**
  * GTA's ProcessControl runs while the car is near the local player.
  * It is skipped (`orig_called: false`) when the car is far: car A, left 170 m north, was frozen for 1305–1321 calls.
* **Corrections, all between `CWorld::Process` and the next frame (never inside the world update):**
  * the forward vector (rotation) is rewritten every frame (1873 of 1886): about 0.56° per frame, which equals the
    synced turn rate, so it is a slerp toward the synced quaternion;
  * the move speed is written when a sync arrives (1042 of 1886 frames; gaps of 2 frames: 760, 1 frame: 255). The
    written |v| has a median of 0.1001 for a synced 0.1000 (min 0.095, max 0.187): the velocity plus a small pull;
  * the position was **never** written outside the world update; it comes only from GTA physics;
  * turn speed stayed below 0.014, so the ±0.02 clamp was never reached.

### 4. Local driver: enter, drive (packet 200), exit

* **Enter (F):**
  * GTA sets the enter task from the on-foot task (`SetTask(…, 3)` from 0x570A41, frame 0).
  * samp sends **RPC 26 EnterVehicle (vehicle 1, passenger 0)** at the moment F is pressed.
  * Foot sync 207 keeps going at 30 ms while the enter animation plays (≈3.3 s).
  * Seated at frame +167: ped state 50, `CTaskSimpleCarDrive`, camera mode 18. The first 200 is sent in the same
    frame.
* **Driving:**
  * Packet 200 is sent every 30 ms (in-car rate) while anything changes (227 of 264 gaps were 30 ms).
  * When the car stands still with no input it is sent every ~530 ms: identical structs are suppressed for 501 ms.
  * Aim sync 203 is also sent about once per second while driving.
* **Exit (F):**
  * samp sends **RPC 154 ExitVehicle (vehicle 1)** when F is pressed.
  * GTA sets `CTaskComplexLeaveCar` (index 3) and ped state 0.
  * **No 200 and no 207 are sent during the exit animation**: the last 200 is 4 ms after RPC 154, and the next 207
    comes 1.9 s later.
  * The ped is on foot after ≈100 frames (`CTaskSimplePlayerOnFoot`, camera mode 4). Then the car blip described in
    §2 is added.

### 5. Stream-out and destroy

* **Stream-out** (local player moved away): RPC 165 for car B, RPC 165 for car A, then RPC 163 for the bot, all in one
  packet. In the trace, in one frame:
  1. `0x0362 WARP_CHAR_FROM_CAR_TO_COORD(remote ped, its own position)` (task reset, `CPed::Teleport`, Remove/Add);
  2. `0x00A6 DELETE_CAR(257)`, then `0x00A6 DELETE_CAR(1)` (`CWorld::Remove` from 0x467B41);
  3. the remote ped's destructor (as in TRACE-2).
  No `RemoveModel` for the car model, and no blip calls.
* **Stream-in again:** both cars are re-created with `CREATE_CAR` at the server's current position and angle. The
  remote ped is created again and immediately warped in with `0x036A` (the stored driver sync).
* **`DestroyVehicle(B)` with the bot inside:** RPC 165 → `0x0362` (ped's own position) then `0x00A6 DELETE_CAR(258)`.
  The ped stays streamed in, on foot.

### 6. For the M5 implementation

* Use the opcode sequence of §1, including the busy-wait load and the +0.25 z.
* The per-frame proofs/tyres pair (§2) is applied to every car.
* Seating, unseating and ejection are warps; the coordinates are the ped's own position.
* Remote-driver swap: PlayerInFocus, pad (steer → LeftStickX, KEY_SPRINT → Cross), camera mode 4, front (0, 0, 0);
  restore z only far away. Unoccupied cars far away are not processed.
* Corrections happen after the world update: slerp the rotation every frame, and on each sync write the velocity
  plus a small pull. No position writes in normal driving.
* Local driver:
  * RPC 26 when the enter starts (F), RPC 154 when the exit starts;
  * 200 at the in-car rate, with identical packets suppressed for about 500 ms;
  * no sync during the exit animation;
  * 203 about once per second.
* Not measured yet: passengers (211), trailers, trains, bikes, aircraft, damage. They need more scenario steps.

## UI (TRACE-3)

Everything here was measured on the ORIGINAL client (samp.dll 0.3.7-R4 build B, as installed in
`wine/gta-sa-trace`) in the trace lane. GTA ran at 1280x720 (video mode 32, see below) with
`fontsize` 0 and `pagesize` 10 unless stated otherwise. Rects are `[left, top, right, bottom]`.
Quads are written as `x0,y0–x1,y1` and come from the decoded vertices of the actual draw calls.
`B+0x…` = samp.dll build B offset.

### Tools

* **D3D call-stream logger** (`crates/samp-tracer/src/d3d.rs` tables/decoding, `src/win/d3d.rs` runtime).
  It is installed from the first `CGame::Process` that finds a device in `_RwD3DDevice` (0xC97C28).
  * It reaches the **real** device through `GetSwapChain(0)` → `GetDevice`, so a client proxy in
    0xC97C28 is bypassed. It patches the device vtable (calls made by d3d9.dll itself are
    ignored). It finds the D3DX font, sprite and render-to-surface vtables and the driver's
    vertex-buffer vtable by creating one throw-away object of each class (class vtables are shared).
  * It inline-hooks the D3DX factories of the loaded `d3dx9_NN.dll` (hot-patch prologue):
    `D3DXCreateFont{A,W,IndirectA,IndirectW}`, `D3DXCreateSprite`, `D3DXCreateRenderToSurface`,
    `D3DXCreateTexture`, `D3DXCreateTextureFromFileEx{A,W}`, `D3DXCreateTextureFromFileInMemoryEx`
    and `D3DXLoadSurfaceFromFileA`. These calls are logged with their arguments, and textures and
    fonts are labelled by file name or face.
  * Output: `d3d.jsonl`. Records have `pr` (Present count), `f` (game frame), `sc` (scene:
    `game` = begun by GTA/proxy, `d3dx` = D3DX render-to-surface, `none` = outside any scene,
    which is where samp's Present pass runs), `fn`, `by` (caller module+offset), `org`
    (client return addresses) and `args`.
    * Draws carry `vertices`: decoded `x,y,z[,rhw] #AARRGGBB u,v` up to 1024 vertices, otherwise
      a hash. This works for FVF and vertex-declaration formats. Dynamic VB contents are shadowed
      from Lock/Unlock while logging.
    * They also carry `tex0`: the stage-0 texture as `tex WxH fFMT … file:NAME`.
    * `font.DrawText{A,W}` records the font descriptor, text, count, rect, fmt and colour, then a
      `.ret` record with the height (and the rect for DT_CALCRECT). The glyph `sprite.Draw`s
      between the two belong to that text.
  * Always logged: factories, cursor calls (`ShowCursor` de-duplicated with `repeats_before`),
    Reset, GetFrontBufferData, state-block recording (`recording_stateblock`). Everything else
    only while logging is on.
  * Logging is switched with the control command `d3d ui|all|text [FRAMES]` / `d3d off`
    (`d3d busy` → busy/idle). It runs from Present to Present. `ui` = calls outside GTA's scenes
    + D3DX scenes + every font/sprite call. `all` = everything. `text` = DrawText only.
  * Each Present record carries `hud` = bytes 0xBA6769 / 0xBAA3FB.
  * The install record lists RenderWare's video-mode table.
* New GTA hooks: `CMessages::AddBigMessage` (0x69F2B0, with `text_addr`), and `__MainWndProc`
  (0x747EB0; only keyboard messages 0x100–0x109 are written).
* New control commands: `poke ADDR HEX` (test harness) and `d3d …`.
* `tools/trace/run.sh orig|ours hold|holdjoin` starts an interactive session. The server runs
  `tools/trace/uifs.pwn`; holdjoin only boots; `TRACE_NO_SERVER=1` starts no server.
  `tools/trace/ui.sh` drives it: key/type/shot/ctl/mark, `fs 'cmd|args'` (filterscript commands
  through `scriptfiles/uicmd.txt`), `d3d MODE N NAME`, `d3d-start`/`d3d-collect`, bots, stop.
  `tools/trace/uicap.sh` adds `cap NAME FRAMES MODE -- ACTION`.
* `tools/trace/d3d_diff.py dump|diff|texts|quads`: readable listing with render/TSS/sampler
  states named; a normalised diff of one logged frame per side; all DrawTexts; draws as screen
  rects with texel rects.
* `tools/trace/setmode.py GTA_SA_SET INDEX` sets `m_nDisplayVideoMode`.
  * Under headless gamescope 1280x720 the RW list is: 0 = 1280x720x32 (windowed), 1–16 = the
    16-bit modes, 17–32 = the 32-bit modes. 27 = 864x486, **32 = 1280x720**.
  * The shared gta_sa.set had 27, so earlier headless runs rendered at 864x486.
* Lane hygiene found on the way:
  * `drive_c/users/$USER/Documents` was a symlink to the user's `~/Documents` in prefix,
    prefix-trace and prefix-diff, so all three shared gta_sa.set, sa-mp.cfg and chatlog.txt.
    They are now private per prefix (main session fixed the others).
  * `run.sh` now holds `target/locks/trace.lock` for the whole run (`tools/trace/with-lock.sh`
    for wrappers).

### U-constants (docs/phase3/m4-core-notes.md) and RE-5 unknowns

| id / item | measured |
|---|---|
| U-HEXCASE | lowercase hex digits are accepted (`{00ff00}` colours) |
| U-CHATBOTTOM | input box top = 10 + (pageSize+1)·(h+1): 197 (page 10, h 16), 282 (page 15), 241 (fontsize 2, h 20); the GUI host dialog is (inputW+100) × (that + 50) |
| U-HISTORY-DUP | duplicates are pushed |
| U-HISTORY-RESET | no reset: the history index survives close/reopen and sending; the live line is not saved (Down past the newest entry shows an empty line) |
| input close | ESC/F6 close does **not** clear the edit text (reopen shows it, caret at end, a selection survives); only sending clears it |
| U-EDITKEYS | ctrl+A select all; ctrl+Left/Right stop at word starts (stock DXUT); Home/End/Delete stock. Insert not measured |
| U-EDIT-SCROLL | DXUT first-visible-char scrolling: DrawTextW gets the text from the first visible char, rect fixed [49,206,799,232]; scroll happens only when the caret leaves the box (examples in "Chat input") |
| U-EDITKEY-CONSUME | input open: no WM_KEYDOWN/WM_CHAR reaches GTA's `__MainWndProc`; every WM_KEYUP does |
| U-MASK | password mask U+25CF `●` |
| edit colour slots | 0xFF969696 = caret, 0xFFB92228 = selection background, 0xFF0A0A0F = selected text; the caret does not blink |
| U-LIST-CODES | list rows (styles 2/4/5) **do** apply `{RRGGBB}` per glyph; the text drawn is stripped; base colour 0xFFFFFFFF. Dialog captions apply codes too |
| U-LIST-TRAILING | neither a trailing `\n` nor an inner empty line makes a row (`a\n\nb` = 2 rows) |
| U-LIST-BORDER | first row at list top + 6; row pitch 18 (GUI font 1) |
| U-TAB-EDGES | leading tab: no cell; trailing tab: no extra cell; runs of tabs = one separator |
| U-TAB-COLUMNS | style-4 column count = the **first** row's cell count; extra cells of later rows are dropped (tested 3-cell and 2-cell first rows) |
| U-SB-FIRST-REQUEST | RPC 155 (rel8, 0 bits) on the opening frame, then every 3.008 s while open |
| U-BUBBLE-WRAP | wrap at the last space before 36 chars, counting `{RRGGBB}` chars; the codes stay in the text and are coloured |
| U-DW-SLOTS / death window | first row top = round(0.3·H); row step 31 (icon height 26 + 5); see "Nametags, scoreboard, death window" |
| U-GAMETEXT-DURATION | not measured visually (GTA-owned); args reach `AddBigMessage` unchanged |
| AFK icon rect (RE-5 §21.5) | glyph `C` (SAMPAUX3 22, black) at [nameLeft−7, Y+28, +1, +2] and `E` (SAMPAUX3 20, 0xFFE3E1E3) at (−1,−2), DT_CENTER\|DT_VCENTER\|DT_NOCLIP, i.e. left of the bars |
| U-CONNECT-TIMING | "Connecting to …" 3.00 s after "Started" (chat created at frame 33, first drawn frame 37); server down: every ~6.05 s "The server didn't respond. Retrying.." then a new "Connecting to …" |
| U-HUD-STEP9 | Present writes 0xBA6769=1 / 0xBAA3FB=0 every frame from the 2nd Present on (connecting screen, class selection, spawned); F7 mode 0 → 0/1 from the next frame |
| U-LOCK-INIT | an input-lock restore runs at start-up (a poked NOP at 0x541DF5 was restored within the first 36 frames); class selection then locks until spawn |
| U-SAVE-HEADING | /save heading = ped heading in degrees (`AddPlayerClass(0,x,y,z,123.0000,0,0,0,0,0,0); // comment`); /rs writes `x,y,z,heading ; comment` where x,y,z is **not** the ped position (camera position) |
| font sizes (RE-5 §3.2) | confirmed at W 864 and 1280; `/fontsize` also rebuilds the GUI fonts immediately |
| D3D cursor | `CreateOffscreenPlainSurface(32,32,A8R8G8B8,SYSTEMMEM)`, `D3DXLoadSurfaceFromFileA("mouse.png", filter 1)`, `SetCursorProperties(0,0,…)`; `ShowCursor(TRUE)` every frame while the input is open; one FALSE on close |
| game text (RPC 73) | `0x00BE` then `CMessages::AddBigMessage(text, time, style)` in the same frame, unchanged values, the same static samp buffer every time |

### Fonts (D3DXCreateFontA; width 0, mip 1, italic 0, out-precision 0, pitch 0)

W = 1280: main Arial 16/700/cs1/q4 (×2), small 14 (×2), plate Arial 38 w10, death names Arial 14 q0,
GTAWEAPON3 22 and 26 (wt400 cs2 q0), GUI font0 Arial 20 q0, font1 18 q0. SAMPAUX3 22 and 28 are created at
start by the object-editor constructor (B 0x10071b90), not lazily for the AFK icon. W = 864: main 14,
small 12, aux 12 (GTAWEAPON3 20/24). `/fontsize 2` at 1280: main 20, small 18, death 18, GTAWEAPON3 26/30,
GUI 24/22 (all recreated at once).

### Chat window

* Render target: `D3DXCreateTexture(2048,1024,1,RENDERTARGET,A8R8G8B8,DEFAULT)` +
  `D3DXCreateRenderToSurface(2048,1024,A8R8G8B8,depth,D16)`. It is redrawn only when dirty or
  scrolled. The Present pass blits it: sprite `Begin(0x10)`, `Draw(src null, pos null,
  0xFFFFFFFF)`, i.e. one quad 0,0–2048,1024.
* Each line is one DrawTextA per pass on the **stripped** text, main font, fmt 0x120, rect
  `[45, top, 550, bottom]`.
  * Line 0 is [45,10,550,110]; line k>0 is `[45, 10+k·(h+1), 550, top+h+1]` (h 16).
  * Mode 2 (W ≤ 1280): 4 outline passes, in the order (0,−1), (0,+1), (−1,0), (+1,0), colour
    0xFF000000. Only left/top move.
  * Mode 1: the main pass only. Mode 0: nothing.
* Glyphs: one sprite `Draw` per visible glyph (none for spaces or codes): tex = 256x256 glyph
  atlas, src = glyph rect, center = cell/2, pos = glyph centre. Example "ab {FF0000}cd": 4 Draws
  per pass; main pass a,b 0xFFFFFFFF, c,d 0xFFFF0000. One flush draws the quads with integer
  pixel corners.
* RPC 93 colour alpha is forced to FF (0x00FF0011 → 0xFF00FF00). `{GGGGGG}` prints literally.
  In a run of codes the last one wins. A 135-char message is one unwrapped line. open.mp drops
  longer client messages.
* Player chat: the name is measured with `DrawTextA(DT_CALCRECT)` ("TraceOrig:" → 71); text at
  45+71+5.
* `/timestamp`: `[HH:MM:SS]` at 45 in the entry colour, content at 118 (tw = 68).
* `/timestamp`, `/pagesize N` and `/fontsize N` print no chat line on success.
* Plate painter (M5): at start `XYZSR998` is drawn into the 128x32 X8R8G8B8 RT. Outline at
  (0,1), (2,3), (0,5), (−2,3) in 0xB4F0EEE4, then the main pass at (0,3) in 0xEE444470, fmt 0x105.

### Chat input, scrollbar, cursor

* Edit box (input open, "hello", page 10): outer 40,197–808,241 (W = min(800, 0.6·1280) = 768,
  H = 14 + 1.5·20 = 44). Nine sampgui pieces, border 5 px, all 0xFFFFFFFF except the centre:
  * corners (8,82,14,90), (241,82,246,90), (8,113,14,121), (241,113,246,121);
  * edges (14,82,241,90), (8,90,14,113), (241,90,246,113), (14,113,241,121);
  * centre 45,202–803,236 tex (14,90,241,113) in 0xF0050505.
* Text: DrawTextW, GUI font0, rect [49,206,799,232] (inset 4), fmt 0x4, 0xFFFFFFFF.
  * The caret is a DrawPrimitiveUP fan (FVF 0x144) from caretX−1.5 to caretX+0.5, y 205.5–231.5,
    0xFF969696. Always on (no blink over 3 s).
  * Selection: a 0xFFB92228 quad over the selected run (48.5–90.5 × 205.5–231.5), then the full
    text, then the selected substring again in 0xFF0A0A0F, then the caret.
* 90-char text: at the end the first visible char is 18 (caret 788.5); Home → 0 (47.5);
  Right×75 → 4 (778.5); End → 18; Left×30 from the end → still 18 (479.5).
* Scrollbar (shown with the input open):
  * up 10,40–30,60 tex (243,124,265,144); track 10,60–30,(40+(h+1)·page−60−20) tex
    (243,144,265,155); down below the track tex (243,155,265,176); all 0xFFFFFFFF.
  * Thumb: tex (266,123,286,167), 0xFFB92228, 20 wide, height max(8, trunc(trackH·page/100)),
    top = trackTop + trunc((pos−1)·(trackH−thumbH)/89) for page 10. pos 90 → 122, 60 → 101, 50 → 94.
  * PageUp from the bottom: 90 → 80 → 70 → 60 → 50.
* Cursor: see the table. Cursor-mode setter B+0xa0791 (show), close B+0xa08ae (hide).

### Dialogs (centred in the 1280x720 client rect, C = 20)

* Common:
  * Background: DrawPrimitiveUP fan 0xDC050505 over the whole dialog.
  * Caption bar: sampgui (17,269,241,287) in 0xC80A0A0A, 20 tall. Caption text: GUI font1,
    rect [X+5,Y,X+W,Y+20], fmt 0x4, 0xFFD2D2D2, codes applied.
  * Buttons 100x30, face (0,0,136,54) in 0xC8FFFFFF. Label: GUI font0, fmt 0x5, 0xFFC8C8C8,
    drawn twice. y = Y+H−35.
  * Two buttons at X+W/2−110 and X+W/2+10; a single button at X+W/2−50.
* Style 0 ("Line one\nLine {00FF00}two\tTabbed"): 525,309–755,412. W = 230 (minimum),
  H = 20+28+25+30. Body: small font, rect [X+20,Y+C+5,X+W,Y+H], fmt 0x140, 0xFFA9C4E4,
  4-way outline.
* Style 1 / 3 (one body line): H = 131. The extra term is 42 = trunc(1.6·18+14), not round.
  * Edit box: X+5, Y+C+27, 220x44, same pieces as the chat input; text rect inset 9; focused
    caret.
  * Responses: 'hi there' (style 1), 'abc' (style 3, drawn `●●●`).
* Style 2: W = 400, H = 265 (LH = 200 minimum).
  * List box 445,253–835,453 tex (13,124,241,265) in 0xC8FFFFFF.
  * Selection X+11,top–X+373,top+18, tex (17,269,241,287), 0xFFB92228.
  * Rows: DrawText GUI font1, rect [X+16, top, X+368, top+18], fmt 0x40.
  * Scrollbar 16 wide at the right inner edge; thumb colour 0xFF95B0D0 (not drawn when nothing
    scrolls).
  * Response: listitem + stripped row text.
* Style 4: cells at x 456 / 671 / 754 (X+16, then + column widths) for the rows used.
* Style 5: headers in the small font, fmt 0x120, 0xFFA9C4E4, at (X+16+Σw, Y+C+3). Header height
  14 (H 279, list top Y+C+19). Response text = first cell.


### 1920x1080 (mode 56 under a 1920x1080 gamescope; `E2E_RES=1920x1080 TRACE_VIDEO_MODE=56 run.sh …`)

* Fonts: main 20, small 18, death 14, GUI 20/18.
* Chat: pitch 21. Eight outline passes in the order (0,−2), (−1,−1), (+1,−1), (0,+2), (−1,+1),
  (+1,+1), (−2,0), (+2,0); the whole rect shifts (at ≤ 1280 only left/top move).
* Input: 40,241–840,285 (width 800); host 900x291.
* Dialog style 0 (2 body lines): 230x113 with a 24-px caption.
* Scoreboard at (560,240).
* Death-window origin round(0.75·1920) = 1440, reason box at 1627.

### Nametags, scoreboard, death window, bubbles

* **Nametag bars** (FVF 0x44 DrawPrimitiveUP fans, z 0, rhw 1): exactly as RE-5 §16.2.
  * Without armour: border X−20..X+21 × Y+18..Y+24; bg X−19..X+19 × Y+19..Y+23 in 0xFF4B0B14;
    fill X−19..X−19+hp·0.3846154 in 0xFFB92228.
  * With armour: health moves down 8, and the armour set (black / 0xFF282828 / 0xFFC8C8C8) is
    drawn at Y+18.
  * Render states are set directly before the batch (no state-block Apply).
  * hp is the remote's synced value: server hp 50 showed 56, because the on-foot sync quantises
    it.
* **Name**: `"%s (%d)"`, small font. First a DT_CALCRECT measure, then 4 outline passes + main,
  fmt 0x100, colour = player colour | FF000000. Rect left = X − w/2, top = Y+3 (bottom = Y).
* **AFK icon**: SAMPAUX3 20/22 are created lazily on the first nametag frame (B+0x69ba0). Every
  tag measures "C" (CALCRECT, colour 0) each frame. After ≥ 3 s without updates the two glyphs
  are drawn (see the table). A bot paused 6 s was dropped by open.mp.
* **Scoreboard** (800x600 at W > 800, X = W/2−400, Y = H/2−300):
  * List box at (X, Y+60), rows in GUI font1, fmt 0x40, pitch 18, cells at X+11/+79/+429/+641.
  * All cells are drawn in the player colour | FF000000; the local player is first.
  * Server name and `Players: N` (left = X+800−width("Players: 10000")) in 0xFFBEBEBE; headers in
    0xFF95B0D0 at y Y+43; all small font with outline.
  * HUD flags 0/1 while open; nametags are still drawn below it.
  * Cursor: one ShowCursor(TRUE) on open (mode 3), FALSE on close.
* **Death window** at 1280x720:
  * Killer name right-aligned to x 1062 (= round(0.75·W) + nickW).
  * Icon box at 1065: GTAWEAPON3 26 `G` black, drawn twice (fmt 0x100, 0x500).
  * Reason glyph GTAWEAPON3 22 at [1065, top−5, 1099, top+21], fmt 0x105 (white; 200 →
    0xFF1111AA, 201 → 0xFFAA1111).
  * Victim name at x 1099; with no killer, the victim takes the killer slot.
  * Names use the death font (Arial 14 q0) with a 4-way outline.
* **Chat bubble**: the pre-wrapped text (with `\n`), 4-way outline + main, small font, rect
  [px, py, px+1, py+2], fmt 0x101, colour = bubble RGBA → ARGB.
* **/save**:
  * On foot: `AddPlayerClass(0,x,y,z,heading,0,0,0,0,0,0); // comment` with the ped heading in
    degrees.
  * In a vehicle: `AddStaticVehicle(model,x,y,z,vehHeading,c1,c2); // comment`.
  * Chat lines `-> OnFoot position saved` / `-> InCar position saved`.
* **/rs**:
  * On foot: `<user files>\SAMP\rawpositions.txt` gets `x,y,z,heading ; comment` (x,y,z not the
    ped position).
  * In a vehicle: `rawvehicles.txt` **in the game directory** gets
    `model,x,y,z,heading,c1,c2 ; comment`.
  * Chat `-> OnFoot pos saved` / `-> InCar pos saved`.

## Vehicles II (TRACE-5)

Original client, `run.sh orig hold` with `uifs.pwn` (`veh`, `putin`, `params`, `vhp`, `attachtrailer`), samp-bot
`--mode idle` as a driver, key presses through `ui.sh`. Sends were decoded from the loopback capture (`samp-dump --raw`). They
were aligned to trace frames through `mark … wall=EPOCH` records and `capinfos` (`tools/trace/events.py` lists a mark window
minus the per-frame background). About 49 game frames per second.

* **Passenger enter (G)**:
  * The WM_KEYDOWN reaches GTA in frame +1. Two frames later samp runs `0x05CA TASK_ENTER_CAR_AS_PASSENGER(1, car, 3000, −1)`.
  * The next frame `SetTask(3, CTaskComplexEnterCarAsPassengerTimed)`.
  * RPC 26 (rel10 ch0, passenger 1) is sent ~0.24 s after the key.
  * 207 continues (~38 ms) until the seating frame (`SetPedState 50`, `CTaskSimpleCarDrive`); from then on 211 at the same
    cadence (duplicates suppressed when parked).
  * 203 every 1.0 s, 205 every 1.0 s.
  * Per-frame client commands as passenger: 0x0470 +1, 0x077E −1, otherwise unchanged.
* **Passenger exit (F)**:
  * GTA's own input: `SetTask(3, CTaskComplexLeaveCar)` 2 frames after the key, ped state 0.
  * RPC 154 ~0.17 s after the key. No sync at all for ~1.56 s, then 207.
  * `CTaskSimplePlayerOnFoot` ~82 frames later.
* **Locked car** (`SetVehicleParamsEx` doors 1):
  * F → nothing (no task, no RPC).
  * G → not blocked: 0x05CA, RPC 26 passenger 1, `CTaskComplexWalkRoundCar`, seated as passenger after ~3.3 s.
* **Driver enter gate refusal** (static, A 0x100A3860):
  * The ctor body never runs. samp reads the first dword of the freshly allocated task, i.e. a
    stale pool content, and calls its slot 0 with 1 when non-zero.
  * It then drops the 2 arguments and jumps to 0x570A9E.
* **Damage**:
  * RPC 106 (rel8) is sent at each panel/door/light/tyre change while driving (5 sends in 1.8 s
    in a wall crash), interleaved with 200.
* **Vehicle death** (server SetVehicleHealth 0):
  * With the player driving: the car burns ~5 s and explodes; RPC 53, then 11 ms later 136 (rel10 ch0).
  * With the player as the last driver standing 25 m away: 136 too.
  * open.mp re-creates dead vehicles after 6–10 s (RPC 165+164 in one frame) whatever the
    respawn delay, so the client's 15 s local delete was not observable.
* **209**: a freshly created car that settles next to the player gets 4–6 sends of 209 (rel7 ch1),
  ~153 ms apart, while it moves.
* **Trailer**:
  * RPC 148 → `0x0893 ATTACH_TRAILER_TO_CAB(trailer, cab)`, then GTA moves the trailer to the hitch.
  * The 200 trailer field = trailer id.
  * 210 is sent on every in-car tick (~38 ms) while the player is in the cab with the trailer
    attached, even when the 200s are suppressed as duplicates.
* **200 extra field**:
  * Bike: smoothed lean, about −0.67 … +0.80 at full stick; 0 straight.
  * Train: signed speed in units/frame (= |velocity|, negative in reverse).
  * Heli/plane: 0.
  * Hydra landing gear byte: 0, then 1 after the gear key.
* **Train** (AddStaticVehicle; open.mp CreateVehicle refuses 538):
  * RPC 70 → `0x036A` + camera reset + `0x0158 CAMERA_ON_VEHICLE(mode 3, switch 2)`.
  * GTA logs `CWanted::RegisterCrime(6)`.
  * F does nothing in a moving train (exit path not reached; open).
* **Stats at state 9**: float stats 69–79 = 1000.0. Int stats 160/223/229/230 (`StatTypesInt` 0xB79000 + 4·(id−120)) = 1000.
  `CStats::SetStatValue` starts with 0xC3.
* **Local aim sync (203)** for BOT-2:
  * Aspect byte 85 (GTA 4:3).
  * Cam modes: 4 on foot, 15 class selection, 18 in vehicles, 3 in the train (samp camera),
    53 aiming, 7 sniper.
  * Rate 1.0 s when not aiming, ~0.115 s aiming.
  * Zoom 63 unzoomed (ramps on aim-in; bikes follow the speed FOV).
  * weaponState 2 (M4/Deagle), 1 (sniper), 0 (fists).
* **Input lock at start-up**: a poked NOP at 0x541DF5 before the first frame was restored within
  36 frames. Class selection locks again until the spawn.


## World entities (TRACE-6)

Original client (samp.dll 0.3.7-R4 build B), `run.sh orig hold` with `uifs.pwn` (TRACE-6 commands: `obj`, `objmove`,
`objmat`, `objtext`, `att`, `pickup`, `actor`, `cp`, `rcp`, `gz…`, `icon`, `label`, `rmb`, …; list at the top of the file),
driven by `ui.sh`. Runs: `target/trace/orig-20261104-220730` (objects, pickups, checkpoints, zones, icons, actors,
labels) and `orig-20261104-222115` (material colour, attachments, RemoveBuilding, combat). LS airport tarmac, about 49
frames/s. The server is open.mp 1.6.7, which never sends RPC 84; see 44 below.

**Tracer additions** (`crates/samp-tracer/src/win/world.rs`, hooks in `hooks.rs`):
* `robj` / `robj_gone`: every object whose entity flags have bit 31 set. The record is written whenever the sampled state
  changes: position, matrix rows, move/turn speed, RW object present, area, flags, attachment, `ms_fTimeStep`. Sampled at
  `CGame::Process` entry.
* `orender`: which objects were drawn in the last frame, and through which function (`object` = `CObject::Render`,
  `entity` = `CEntity::Render`, `nonroad` = `CRenderer::RenderOneNonRoad`), with the return addresses above the call.
  Written when that set changes, and every 500 frames.
* `hp`: health, armour, ped state, current weapon slot/type/state/clip/ammo of the local ped, every remote ped and every
  other ped (actors), on change.
* `los`: every `CWorld::ProcessLineOfSight` call made from a client module, with its result (hit, entity, colpoint) and
  the GTA return addresses (call site).
* Opt-in:
  * `SAMP_TRACER_BONES=2,6`: per-frame `bones` record with the ped matrix and the world LTM of those `ePedNode` bones
    (RpHAnim matrix of `CPed::m_apBones[id]`'s node).
  * `SAMP_TRACER_WATCH_MODELS=4019,…`: `wmodel` records for building/dummy/object pool entries of those models, every
    25 frames.
* New hooks: `CObject::Render`, `CEntity::Render`, `CRenderer::RenderOneNonRoad` (aggregated),
  `CEntity::Create/DeleteRwObject`, `CRadar::DrawAreaOnRadar`, `CWeapon::Fire` / `FireInstantHit` / `FireSniper` /
  `DoBulletImpact`, `CCamera::Find3rdPersonCamTargetVector`, `CWorld::ProcessLineOfSight` (wrapped),
  `CWorld::GetIsLineOfSightClear`, `CPedDamageResponseCalculator` ctor and `ComputeWillKillPed`, `CPed::RemoveBodyPart`.
  * `CProjectileInfo::AddProjectile` can't be hooked: the exe's protection code reads its entry bytes.
  * `ComputeDamageResponse`, `AddEntityToRenderList` and `LoadObjectInstance` are samp patch sites. Their effect is read
    through the hooks around them instead (the damage-calc ctor, `ComputeWillKillPed`, the render-path hooks, the
    watched-model records).
* `crates/samp-bot/examples/c2s_decode.rs`: decodes the client→server hex in `samp-dump --raw` output (RPCs through
  `rpcs::decode_outgoing`, sync packets through their readers). `--only 206,115,…`, `--c2s`.
* `ui.sh mouse …` / `ui.sh fire [MS]`: mouse buttons and relative moves. RMB aims, LMB fires; mouse Y is inverted
  relative to xdotool (move −y to aim lower). `TRACE_LAGCOMP=0|1|2` sets the server's `lag_compensation_mode`.

### Objects (44/45/46/47/99/122)
* **RPC 44**, one frame:
  * `0x0107 CREATE_OBJECT(model, x, y, z)`, then `0x0453 SET_OBJECT_ROTATION(rx, ry, rz)`.
  * The RW object is created 1–3 frames later. `CEntity::CreateRwObject` is called from samp's CObject vtable hook
    (`samp.dll+0xa4b30` in B), and is sometimes repeated for 2–3 frames while the model streams.
  * Entity flags of a samp object: 0x88000085 typical; 0x80004085 for 19325 (glass); bit 15 is set for one frame
    after creation.
  * Bit 31 is not exclusive to samp: GTA's own pickup objects have it too (0x98220084).
* **Material changes**:
  * open.mp implements SetObjectMaterial / SetObjectMaterialText by **re-sending RPC 44** for the same object id, with
    the material records. RPC 84 never appears.
  * For an existing id, the client runs `DELETE_OBJECT(old handle)` and `CREATE_OBJECT` (new handle) in the same frame,
    then streams it like a new object.
* **Render path** (return addresses of the draw calls):

  | object | path |
  |---|---|
  | 1271, draw distance 0, no material | GTA render list: `CRenderer::RenderEverythingBarRoads` → `RenderOneNonRoad` → `CObject::Render` (rets 0x553c57, 0x553308) |
  | 1271, draw distance 500 | samp's pass in its `RenderScene` hook after `RenderEverythingBarRoads`: samp calls `RenderOneNonRoad` itself (rets 0x53dfe2 / 0x553308) |
  | 19325 glass (draw-last), dd 500 | samp's second pass, after `RenderFadingInEntities` (ret 0x53e01e) |
  | 19353 with material text, dd 0 | samp's first pass (0x53dfe2) |
  | 1271 with a material **colour** (index 0, 0xFFFF0000) | entity bit 28 set (flags 0x98000085). Drawn once through samp's first pass, then never again through `RenderOneNonRoad`, `CEntity::Render` or `CObject::Render`, although it stays visible. samp draws it at RW level from its own pass |

* **Static check of the manual draw** (A 0x100A9FD0; it explains the last row):
  * Object without materials, or whose flag +0x1CF is clear, closer than 600: entity vtable slot +0x44, then
    `CRenderer::RenderOneNonRoad(entity)`. This is the "samp's pass" path above.
  * Object with materials and +0x1CF set (the material-colour object), closer than 600:
    1. set entity bit 28 and call entity vtable slot +0x4C (+0x1194 := 1, A 0x100A9080);
    2. render the RW object directly: atomic or clump by its RW type byte (A 0x100B6370);
    3. undo the step-1 setup (A 0x100A90C0).
  * 600 or farther: direct RW render without that setup.
  * Material pre-/post-pass (A 0x100A8C40 / 0x100A9EB0) around all of these, from the second render on (+0x118F).

* **Streaming out of range**:
  * The player was moved 400 m away for 35 s. Only the samp-drawn object whose draw distance was exceeded (the
    material-text object, dd 0) lost its RW object, exactly 1000 frames after it left range (`DeleteRwObject` from
    samp, `robj.rw` false).
  * dd 500 objects at 400 m kept theirs. The GTA-list object (dd 0, no material) is GTA's business.
  * Back in range, the RW object was recreated 5 frames after the teleport.
* **MoveObject (99)**, object 1271 moved +10 m z at speed 2.0, then back with rotation (0, 0, 90):
  * Motion starts 4 frames after the RPC.
  * Move speed = speed × 0.02 exactly (0.04), constant (no schedule correction was needed at 50 fps).
  * Every frame the position advances by move speed × `CTimer::ms_fTimeStep`. Checked frame by frame: Δz = 0.038 at
    timestep 0.95, 0.036 at 0.9, so samp writes the matrix itself.
  * The 5.0 s move took 263 frames (4.98 s). In the arrival frame the position is exactly the target and move/turn
    speed are 0. `OnObjectMoved` fired on the server after 5000 ms.
  * With rotation: turn speed z = 0.001 (the clamp) every frame. The forward vector is slerped by progress and is
    exactly the target rotation at arrival.
  * **StopObject (122)**: the object stops where it is (move speed 0, no snap).
  * open.mp sends 122 immediately before a new 99 when the object is still moving. RPC 99 carries the server's
    estimate of the current position (15.70 vs the client's 15.695). The new move continued from the client position.
    The spec's "snap to the old target when a move is still running" is not reachable with open.mp.

### Attached objects (113), bones
* RPC 113 → `0x0107 CREATE_OBJECT(model, ped position)` + `0x0453(rx, ry, rz)`. From the next frame the matrix is
  rewritten every frame.
* **Matrix formula**, verified on 1300 frames each, with `SAMP_TRACER_BONES=2,6`. Same-frame sample; the bone LTM and
  the object match with lag 0.
  * slot 0: 18645, bone 2, offset (0.1, 0, 0), rotation (0, 90, 0). Max error 0.0012 (position) / 0.0005 (rows).
  * slot 1: 336, bone 6, rotation (10, 20, 30), scale (2, 1, 1). Max error 0.0014.
  * With B = bone LTM rows (right, up, at) and RwMatrix row-vector convention:
    * position = B.pos + ox·B.right + oy·B.up + oz·B.at (offset in the bone frame, before rotation);
    * rows = S · Rz(rz) · Ry(ry) · Rx(rx) · B, where Rx(a) = [[1,0,0],[0,c,s],[0,−s,c]],
      Ry(a) = [[c,0,−s],[0,1,0],[s,0,c]], Rz(a) = [[c,s,0],[−s,c,0],[0,0,1]]. That is: rotate about the local X first,
      then Y, then Z (pre-concatenated). S scales row i by scale i (right × sx, up × sy, at × sz).
  * The object matrix rows map 1:1 onto the CMatrix rows (right, forward, up).

### Pickups (95/63/131)
* RPC 95 → `0x0213 CREATE_PICKUP(model, type, x, y, z)` in the RPC frame.
* Walking through type 2 (1240) and type 19 (1212): **RPC 131 rel 9 (RELIABLE_ORDERED) ch 0**, the collision path.
  * Sent ~1.1 s after starting to walk 4 m.
  * The pickup id is the per-player id the server put in 95 (it differs from the server-side id).
* Type 14 (vehicle pickup) is never collected on foot.
* samp polls only the type-14 pickup with `0x0214 HAS_PICKUP_BEEN_COLLECTED(handle)`, **every 7 frames** (313 of 313
  gaps). Types 2/19 are never polled.

### Checkpoints (107/37/38/39)
* **107**:
  * every frame: `0x00FE LOCATE_CHAR_ANY_MEANS_3D(player, x, y, z, size, size, size, 1)`;
  * once: `0x04CE` blip (sprite 0), `0x0165 CHANGE_BLIP_COLOUR(1005)` (0x3ED) and `0x0168 CHANGE_BLIP_SCALE(3)`.
* **37**: `0x0164 REMOVE_BLIP`; the per-frame `0x00FE` stops.
* Enter/leave send nothing to the server (open.mp detects them from sync).
* **38** (types 0, 1, 3):
  * first: `0x06D5 CREATE_CHECKPOINT(type, x, y, z, nx, ny, nz, size)`, then `0x04CE` / `0x0165(1005)` / `0x0168(3)`;
  * replacing: `0x0164` (old blip), new blip ops, `0x06D6 DELETE_CHECKPOINT(old)`, `0x06D5(new)`;
  * nothing per frame.
* **39**: `0x0164` + `0x06D6`.

### Gang zones (108/120/121/85)
* `CRadar::DrawAreaOnRadar(rect, rgba, false)` is called by samp every frame per shown zone, in zone-id order.
* rect = [minX, maxY, maxX, minY].
* The CRGBA bytes are the server RRGGBBAA unchanged (0xFF000080 → R ff, G 00, B 00, A 80).
* **Flash**: the colour alternates every ~514 ms (27 frames), the flash colour first (~27 frames after 121).
* 85: normal colour from the next frame. 120: no longer drawn.
* Pause map: MEASURED in TRACE-7, see "World entities: TRACE-7 additions".

### Map icons (56/144)
* Opcode by style: 0 → `0x04CE`, 1 → `0x02A8`, 2 → `0x0570`, 3 → `0x02A7` (x, y, z, type).
* Type 0 only:
  * `0x0165(colour as int)` (0xFF0000FF → −16776961; 5, 1500, 3999 and 4000 are passed unchanged);
  * then `0x0168` scale 3, except scale **2 for colour 5**. This confirms the spec rule (scale 2 when colour < 1004,
    else 3) and shows that the comparison is **unsigned**: 0xFF0000FF (negative as int) → 3.
* Type ≠ 0: no colour or scale calls.
* 144: `0x0164`.

### Actors (171–178)
* **171**:
  * `0x0248`/`0x0247`/`0x038B` model load, then `0x009A CREATE_CHAR(5, model, x, y, z − 1.0)` (→ CPed ctor,
    SetModelIndex, SetPosn at the real z, `CWorld::Add`);
  * `0x0173` heading, `0x0446(h, 0)`, `0x060B(h, 65542)`;
  * `0x02AB` proofs: (1, 1, 1, 1, 1) when invulnerable, (0, 0, 0, 1, 0) when vulnerable.
* open.mp's SetActorInvulnerable re-streams the actor: 172 + 171, CPed dtor + a new CREATE_CHAR.
* 173 → `0x04EE(lib)` + `0x0812 TASK_PLAY_ANIM_NON_INTERRUPTABLE(h, name, lib, delta, loop, lx, ly, freeze, time)`.
* 174: no script command (direct `FlushImmediately`).
* **178 health 0** (MR-6.7):
  * `0x0321 EXPLODE_CHAR_HEAD` → damage calc (damager null, 1000.0, weapon 34, piece 9). `ComputeWillKillPed` is
    **not** reached (samp cancels the response).
  * The ped's health is 0 at once and ped state goes 1 → 54 → 55 (died) within 2 frames; the weapons are cleared
    30 frames later.
  * `CPed::RemoveBodyPart` is never called: the actor falls dead with its head on.
  * Same for invulnerable and vulnerable actors. An actor re-created with stored health 0 gets `0x0321` right after
    `0x060B`, before its proofs.
  * Setting health 100 afterwards changes the health but the actor stays dead (state 55).
* **Local player shoots a vulnerable actor** (Colt, 4 hits):
  * GTA builds the damage calc (25.0, pieces 3/5), then nothing: `ComputeWillKillPed` is not called and the actor's
    health stays 100.
  * Per hit, in this order: 206 (hit_type 0, hit_id 0xFFFF, offset = hit_pos = world hit point, rel 7 ch 0); RPC 177
    (rel 9 ch 0, damage 25.0 **unscaled**, weapon 22, body part 3/5); 204 (target_actor = actor id + the slot's new
    ammo).
  * Aim released → 204 with target_actor 0xFFFF and no slot records.

### RemoveBuildingForPlayer (43)
* `RemoveBuildingForPlayer(4019, 1777.8, −1773.9, 12.5, 0.25)` with the building streamed in, with
  `SAMP_TRACER_WATCH_MODELS=4019`:
  * within 18 frames the building entity has area 1 and z − 2000 in both placement and matrix
    (12.52 → −1987.48);
  * its RW object is kept and the entity is not deleted.
* The player went 2.8 km away: the IPL instance was freed 22 frames later. After coming back, no model-4019 instance
  was loaded again. This is consistent with the 19300 replacement of `LoadObjectInstance`; the 19300 instance itself
  was not watched.

### 3D text labels (36)
* Drawn in samp's Present pass with the small font (Arial 14, wt700, q4):
  * 4-way outline (0,−1), (0,+1), (−1,0), (+1,0) in 0xFF000000, then the main pass;
  * the text is stripped of `{RRGGBB}` codes; rect [x, y, x+1, y+1], fmt 0x101 (centre, no clip);
  * colour = label RGBA → ARGB.
* Beyond the draw distance (48 m for 30) the label is not drawn.
* **LOS flag**: a label behind an object is not drawn; the same position without the flag is drawn.
* The test is not a `ProcessLineOfSight` call from samp (`GetIsLineOfSightClear` is hooked since run 2 to confirm).

### Material text (44 with text records), run `orig-20261104-225641`, `tools/trace/world2.sh`
`SetObjectMaterialText(19353, "RE10 {FF0000}red", index 0, size 130 = 256x128, "Arial", 24, bold, FFFFFFFF on
FF000000, centre)`. One render, in the frame after the RPC, from samp's object pass (`samp.dll+0xa9f77` in B):
1. `D3DXCreateTexture(512, 256, 1, RENDERTARGET, X8R8G8B8, DEFAULT)` and
   `D3DXCreateRenderToSurface(512, 256, X8R8G8B8, depth, D16)`. The render target is **twice** the material size
   (256x128 → 512x256).
2. `D3DXCreateFontA("Arial", h 24, w 0, weight 700, mip 1, italic 0, charset 1, out-precision 0, quality 0, pitch 0)`.
   The font size is passed unchanged.
3. `rts.BeginScene` → `Clear(colour = background ARGB 0xFF000000)` → `sprite.Begin(0x10)`.
4. `DrawTextA(stripped text, rect [0, 0, 512, 256], fmt 0x105, 0xFFFFFFFF)`. With alignment 1 (centre) the format is
   DT_CENTER | DT_VCENTER | DT_NOCLIP. The colour codes are applied per glyph (the glyphs after `{FF0000}` are
   0xFFFF0000).
5. `sprite.Flush`, `rts.EndScene`.

Not redrawn on later frames. The texture is then substituted at render time (RE-10 §4).

**TRACE-7, exact D3D order** (run `orig-20261105-000102`, `ui.sh d3d all`; request from the world-entity implementer).
The render does **not** happen inside a scene: it runs in samp's `CGame::Process` wrapper (B 0x100a2670, A 0x100a26a0, before the original `CGame::Process`) → object-pool
pass 0x10012ae0 → per object whose first-render flag +0x118B is set → 0x100a9ef0 → 0x100708e0), i.e. after the previous
frame's `EndScene` + `Present` and before RenderWare's `BeginScene` of the new frame. `samp.dll+0xa9f77` is that call, not
the render pass (TRACE-6 wording corrected). In one frame:
1. previous frame: `dev.EndScene`, sprite/UI passes, `dev.Present`;
2. `dev.CreateTexture(512, 256, 1, RENDERTARGET, X8R8G8B8, DEFAULT)` (from `D3DXCreateTexture`),
   `D3DXCreateRenderToSurface(512, 256, X8R8G8B8, depth 1, D16)`, `D3DXCreateFontA`;
3. `rts.BeginScene(rt surface, viewport null)`, which issues on the device: `BeginStateBlock`, `SetViewport(0,0,512,256)`,
   `EndStateBlock` (D3DX's state capture), `SetRenderTarget(0, rt 512x256)`, `SetDepthStencilSurface(D16 512x256)`,
   `SetViewport(0,0,512,256,0,1)`, `BeginScene`;
4. `dev.Clear(0 rects, TARGET|ZBUFFER, 0xFF000000 = background ARGB, z 0.0, stencil 0)`;
5. `sprite.Begin(0x10)` (+ the sprite's own state blocks), `font.DrawTextA(…, 0x105)` (the font creates its 256x256
   glyph texture here), `sprite.End`, `sprite.Flush` (draws the glyph quads: one `DrawIndexedPrimitive`);
6. `rts.EndScene(mip filter 0)`, which issues: `dev.EndScene`, `SetRenderTarget(0, back buffer 1280x720)`,
   `SetDepthStencilSurface(original 1280x720 D24S8)`;
7. then RenderWare's frame: `SetViewport(0,0,1280,720)`, `Clear(ZBUFFER|STENCIL, z 1.0)`, `SetViewport`, `BeginScene`.
So samp never nests scenes: the RT scene opens and closes completely between two RenderWare frames.

### Edit / select (117 / 116 / 27), `world2.sh`
* `EditObject` → ESC: OnPlayerEditObject response 0 (cancel) with the object's current position and rotation.
* `EditAttachedObject` → ESC: OnPlayerEditAttachedObject response 0 with the slot's current model, bone, offset,
  rotation and scale.
* `SelectObject` → mouse move onto the object + left click: OnPlayerSelectObject type 1 (global object), id, model,
  position.
* Screenshots: `w-objedit.png`, `w-attedit.png`, `w-objsel*.png`. Drag updates (response 2 rate) were not measured.

### 3D label LOS and attachment
* LOS test, per frame, twice: `0x06BD IS_LINE_OF_SIGHT_CLEAR(label pos → camera pos, buildings 1, vehicles 0,
  peds 0, objects 1, particles 0)`. Nametags use the same opcode with the remote ped's head as the start.
* A per-player label attached to the local player itself (`CreatePlayer3DTextLabel(L, …, attachedplayer = L)`) is not
  drawn for that player.

### World entities: TRACE-7 additions (run `orig-20261104-234135`, `tools/trace/combat2.sh rmb` + manual `ui.sh` steps)

* **Edit-object drag (117 response 2) — MEASURED.** `EditObject` on a 1271 crate, then the X handle of the gizmo was
  dragged with the left button held (xdotool: press on the handle, 100 relative moves of 2 px every 30 ms, release):
  * 13 RPC 117 with response 2 in 3.65 s, rel 9 ch 0. Intervals 250–287 ms (frame-quantised just above 250 ms), which
    confirms the static 250 ms `GetTickCount` limit (`world-entities.md` §117 C→S).
  * The first one is sent 126 ms after the button press, still at the unchanged position (1702.5); later ones carry the
    current matrix position (1702.64, 1702.80, … 1704.32), rotation unchanged (0, 0, 0).
  * Clicking the save icon of the gizmo bar sends response 1 with the final position (1704.48, i.e. newer than the
    last response-2 update). The cancel case (response 0) is in TRACE-6.
  * Screenshots `e-edit.png`, `e-drag.png`. The attached-object editor (116) drag was not repeated; it shares the
    editor loop (static), so the same rate is expected but is **not measured**.
* **Pause-map gang zones — MEASURED.** Gang zone (1650, −2650)–(1750, −2550) shown in 0xFF000080:
  * In the pause menu's MAP page samp calls `CRadar::DrawAreaOnRadar(rect, colour, in_menu = true)` once per shown zone
    per frame (797 frames, one call each), from the same samp site as the radar pass (`samp.dll+0xa1469` in B), with the
    same rect ([minX, maxY, maxX, minY]) and the unchanged colour bytes. The zone is drawn on the map (`pm-map.png`).
  * On the other pause-menu pages (the menu list) no gang-zone call is made at all; in game the radar pass resumes
    (`in_menu = false`).
* **RemoveBuilding 19300 substitution — MEASURED** (`SAMP_TRACER_WATCH_MODELS=4019,19300`):
  * `RemoveBuildingForPlayer(4019, 1777.8, −1773.9, 12.5, 0.25)` → within 10 frames the streamed building has area 1 and
    z −1987.48 (as in TRACE-6).
  * Player moved to (1500, 1500) (3.3 km): the lowered 4019 entity is freed 18 frames after the teleport.
  * Back at the airport: 27 frames after the teleport a **building of model 19300** appears at the exact original spot
    (1777.84, −1773.91, 12.52), area 0, not lowered, RW object present; no model-4019 instance is loaded again.
  * 19300 is `blankmodel` in `SAMP/SAMP.ide` (`19300, blankmodel, MatTextures, 100, 0`). So the IPL hook swaps the
    model id and leaves position, area and everything else of the instance unchanged.

* **blanktex — MEASURED** (run `orig-20261105-001923`, `combat2.sh world7`; static B 0x100a8540 / 0x100b2ba0):
  * Source: the loose file `SAMP\blanktex.txd` (not SAMP.img). samp's TXD helper creates a TXD slot named `blanktex`
    (`CTxdStore::AddTxdSlot("blanktex")`, slot 3801 here) and loads it with `CTxdStore::LoadTxd(slot, "SAMP\blanktex.txd")`
    (the helper's other directory prefix is `models\txd\`), then takes texture `blanktex` from it. Loaded once, on first
    use, 4 frames after the RPC; the texture pointer is cached in a global and never released with the object.
  * When: `SetObjectMaterial` with **model 0** (`objmat|ID|0|0|none|none|FF00FF00`). Model > 0 loads that model's TXD and
    texture instead (with a bounded wait: up to 250 × `Sleep(2)`); model < 0 (−1) stores no texture and loads nothing
    (measured: no TXD activity).
* **Camera collision (CSV rows 81–83, MR-6.12)**:
  * Static (B): row 81 (call of `BuildCacheOfCameraCollision` in `CameraConeCastVsWorldCollision`, B 0x100a5960) runs
    the original first; only when it found nothing and the camera spheres are inside ±3000 units does samp add its own
    tests: client objects (pool 1), per-player objects (pool 2), and vehicles unless netgame byte +0x232 is set. The
    per-object no-camera-collision flag is consulted only in samp's own object test. Row 82 (`CheckCameraCollisionVehicles`
    call, B 0x100a5ad0): skipped (returns 0) when netgame byte +0x232 is set. Row 83 (`CameraToIgnoreThisObject` call in
    `CheckCameraCollisionObjects`, B 0x100a5b00): objects attached to an entity (+0xFC ≠ 0) are always ignored.
  * Live: player at (1700, −2600) facing north, camera behind at y −2603.50 (CCamera matrix position, peek 0xB6F9CC).
    Wall 19353 at y −2602.3 → camera pulled in to y −2600.58 (z 14.32 → 14.18). The same wall re-created by open.mp with
    `no_camera_collision = 1` (open.mp re-sends RPC 44 for `SetObjectNoCameraCol`) → camera **still** at −2600.58 after a
    camera reset. Wall removed → −2603.50 again. So for an object GTA's own camera pass already sees, the flag changes
    nothing (consistent with the static reading: it only affects samp's supplementary test). Which objects GTA's own pass
    misses (the case the flag is for) and the +0x232 byte's source are **not measured**. Note: the camera stays pulled in
    while the player stands still; it is re-evaluated after `SetCameraBehindPlayer` / movement.
* **ProcessVerticalLine (CSV row 96, MR-6.13) — MEASURED**:
  * Static (B 0x100a5fd0): for every caller except return address 0x59E8F9 (`BreakObject_c::Init`), when the
    `buildings` argument is set samp forces the `objects` argument to true, then runs the original.
  * Live: `SetPlayerPosFindZ(1710, −2600, 60)` with a 1271 crate at (1710, −2600, 13.9): RPC 13 → SCM 0x02CE
    `GET_GROUND_Z_FOR_3D_COORD` → `CWorld::FindGroundZFor3DCoord` (which itself asks for buildings + dummies only) → ground
    z = **14.250 (top of the crate)** → `CPed::Teleport(z + 1.5 = 15.75)`. Four metres away (no object): 12.547 → 14.047.
    So every building-ground query (ground-Z opcodes, car generators, shadows/point lights, weather, camera ground checks,
    explosion ground checks) also hits client objects.

### Still open (M6)
* 116 (attached-object editor) drag rate: not measured (see above).
* Camera collision: which objects GTA's own camera pass misses (where samp's no-camera-collision flag matters) and the
  source of netgame byte +0x232 (vehicle camera collision off): not measured.

## Combat (TRACE-6)

Original client, `run.sh orig hold`, LS airport tarmac. Bots are samp-bot `--mode idle|shoot`. Runs:
`orig-20261104-220730` (actor shots), `orig-20261104-222115` (gungame, first bot fights), and `orig-20261104-223133`
(neutral `TRACE_GAMEMODE=tracegm`, lag-comp 1, `combat.sh`). The server's lag-comp mode 1 means samp's selector S = 0
(`remote-control-combat.md` §3.4); modes 0 and 2 are in "Lag-comp modes" below.
* Sends were decoded with `samp-dump --raw | c2s_decode`.
* Aiming in the headless display: RMB = `ui.sh mouse mousedown 3`, relative mouse moves (−y aims lower).
* The camera pitch persists across aims and is reset by a weapon change.
* An un-aimed shot is LMB alone.

### Local player shooting
* **Call chain**:
  1. `CWeapon::Fire` (from `CTaskSimpleUseGun`, ret 0x61ECD2).
  2. samp's replacement of the `FireInstantHit` call site.
  3. GTA `FireInstantHit`, called by samp (`samp.dll+0xb6515` in B) with **origin = right-hand bone position**.
     With dual Colts the left pistol's shot also gets the right-hand origin; `Fire`'s own start differs.
  4. Aimed (RMB): `Find3rdPersonCamTargetVector(range)` (Colt 105, shotguns 120), then `ProcessLineOfSight` at site
     0x740721.
  5. Un-aimed (LMB only): `ProcessLineOfSight` at site 0x740B69, along the ped's facing, with the weapon range as
     length (Deagle 35, M4 ~91).
  6. `DoBulletImpact` (start = muzzle).
* **206 is sent only when that samp-hooked LOS hit something**:

  | hit | 206 |
  |---|---|
  | remote ped | hit_type 1, hit_id = player id, offset in the victim's local frame (e.g. (0.156, 0.091, −0.779) for a leg) |
  | ground / building / actor | hit_type 0, hit_id 0xFFFF, offset = hitPos = world point |
  | nothing (sky), sniper | no 206 |
  | nothing (sky), any other instant-hit weapon | 206 hit_type 0, offset (0, 0, 0), hitPos = stale colpoint (TRACE-7 correction) |

  * 206 is rel 7 (unreliable sequenced) ch 0, in the shot frame.
  * Quirk: the first missed Deagle shot after a sniper sky-miss sent a 206 with hit_type 0, offset (0, 0, 0) and a
    garbage hitPos (an uninitialised colpoint). **TRACE-7:** this is the normal miss path, not a quirk: every missed
    non-sniper shot sends such a 206 (measured for every drive-by miss; static B 0x10006bb0 drops only weapon 34 without a
    hit entity). The TRACE-6 "sky → no 206" observation was the sniper.
* **Shotguns** (25, 27):
  * one samp-hooked LOS (the main ray), then 6–7 `DoBulletImpact` pellets handled inside GTA, then **one** damage
    calc with the summed damage (90 / 105 / 120);
  * → exactly one 206 and one 115 per blast (115 damage = 0.33 × sum, e.g. 29.7).
* **Sniper** (34): `FireSniper` → the bullet is created at 50× velocity.
  * Sky miss: samp's LOS hook (site 0x736247 in `CBulletInfo::Update`) traces the flying bullet every frame for ~53
    frames, until it leaves the world. No 206.
  * Ground hit: 206 hit_type 0.
* **Melee** (fists, 2 punches): two 115 (taking 0, weapon 0, damage 1.32, part 3); no 206.
* **Damage to a remote**:
  * GTA's damage calc (Deagle 140, Colt 25, shotgun pellet sum) → the original response runs. `ComputeWillKillPed`
    sees the damage × 0.33 (player-ped victim).
  * The remote ped's health stays forced (no `hp` change).
  * Order in the same frame: 206, RPC 115 (rel 9 ch 0, taking 0, victim id, 0.33 × damage, weapon, body part from the
    colpoint), 204 (rel 6) with the slot's new ammo.
* **Same team** (`SetPlayerTeam` 1 for both): 206 and 115 (39.6) are still sent, but the response is cancelled
  (`ComputeWillKillPed` not called). Confirmed in a clean run, see "Lag-comp modes".
* **Actors**: see "World entities (TRACE-6) / Actors". 177 is unscaled; the response is always cancelled.
* **204** (rel 6, unreliable):
  * on aim: target_player = remote id (target_actor = actor id when aiming at an actor);
  * a slot record (slot, weapon, ammo) after every shot (ammo change) and on a new weapon;
  * on release: targets 0xFFFF, no slots.
* Every frame samp makes its own LOS from the local ped (`samp.dll+0xacabb`): z−0.25 → z−2.0, buildings + vehicles
  only (flags 01010000). This is a ground probe under the ped.

### Remote player shooting the local player (MR-7.2), lag-comp 1
* **206 hit_type 1 at our id, offset (0, 0, 0.3), Deagle**:
  1. First 206: in the packet frame samp gives the remote ped the weapon (`GiveWeapon(24, 9999)` +
     `SetCurrentWeapon`).
  2. GTA `FireInstantHit(remote ped)`, called by samp with **PlayerInFocus = 0**: the camera swap does not touch
     PlayerInFocus.
  3. LOS (site 0x740B69) start = the remote's hand origin, end = 2P − start with P = local ped matrix × offset.
     The ray hits the local ped (redirect confirmed).
  4. `DoBulletImpact(victim local)` → damage calc 140 → `ComputeWillKillPed` 46.2 → local health −46.2.
  5. RPC 115 taking 1 (attacker id, 46.2, weapon 24, part 3) ~19 ms after the 206 arrived. It is sent from the
     traced client; the bot's own 115 is separate.
* **206 hit_type 0** (hitPos = our torso): `FireInstantHit` → `Find3rdPersonCamTargetVector` → LOS at site 0x740721.
  The ray is built from the remote's synced camera (the 206 hitPos is not used). With the bot's aim it passed above
  the local ped and hit a building 50 m away. No damage, no 115. The suppression case (a ray that really crosses the
  local ped) is not reproduced yet.
* While the bot holds AIM, `Find3rdPersonCamTargetVector` runs every frame with PlayerInFocus = the bot's slot (the
  remote ped's aiming task). The remote `CWeapon::Fire` from the injected FIRE key runs with PlayerInFocus = slot, and
  samp drops its `FireInstantHit` (no bullet; damage comes only from 206).

### Lag-comp modes (`TRACE_LAGCOMP`, `tools/trace/lagcomp.sh`; runs `orig-20261104-224449` = 0, `orig-20261104-225641` = 2)

| server mode | InitGame lagcomp | local 206 | local hit → 206 offset | samp LOS hook / FireInstantHit suppression |
|---|---|---|---|---|
| 1 (S 0) | 1 | sent on any hit | victim's local frame: (−0.096, 0.296, 0.469) for a bot facing 180° | active |
| 2 (S 1) | 2 | sent on any hit | hitPos − victim position, world axes: (0.097, −0.293, 0.469), the same hit unrotated | active |
| 0 (S 2) | 0 | **never sent**, only 115 | — | **inactive**: no LOS call from samp at all; remote peds' `CWeapon::Fire` → `FireInstantHit` → `DoBulletImpact` run in GTA (17 remote shots simulated locally from keys/aim) |

* Mode 0: the server relayed no 206 to the traced client.
* Mode 2, hit_type 0 from the bot: the same camera-ray behaviour as mode 1 (passed above the local ped, no damage).
* The bot's hit-type-1 run in modes 0 and 2 produced no shots (bot-side), so remote hit_type 1 is measured only in
  mode 1. **TRACE-7:** measured in modes 0 and 2, see "TRACE-7 additions".
* **Same team**, repeated cleanly in the mode-2 session: team 1 for both → 206 + 115 (39.6) sent, response cancelled
  (no `ComputeWillKillPed`). Teams reset to 255 → the response runs (`ComputeWillKillPed` 39.6).

Note: in run `orig-20261104-223133` another agent drove the session after 22:37:00 (teleport + its own bot). Nothing
measured after that time in that run is used here.

### Death and respawn
* **Local, `SetPlayerHealth(0)`**:
  * frame 0 (RPC 14): `0x01B4(0, 1)`, `MakePlayerSafe(false, 10)`, `0x04D7(1, 0)`, `CPed::Teleport` to its own
    position, task reset, Remove/Add (samp's health-0 handling);
  * frame +1: GTA damages the ped with weapon 54 / 10.0 (damager null, from `CPlayerPed::ProcessControl`) → ped state
    54. **RPC 53 (reason 255, killer 0xFFFF)** is sent 21 ms after the RPC 14 arrived. **RPC 115** (taking 1, attacker
    0xFFFF, weapon 54, 3.3, part 3) follows 15 ms after 53;
  * +31 frames: state 55;
  * +189 frames: respawn in one frame: `ClearWeapons`, `MakePlayerSafe(false, 10000)`, ped state 1, `Teleport` to
    spawn z + 1, camera `SetCameraDirectlyBehind…` + `RestoreWithJumpCut`, `LoadScene`, then samp's spawn sequence
    (0x04E4, 11 × `RequestModel` flags 24, 0x016C, `SetModelIndex`, `ClearWeapons`, 0x06D7, `Teleport` spawn z+0.5,
    0x0173 …);
  * **RPC 52 3.62 s after 53**. No 129 (class info valid).
* **Killed by the bot's Deagle**: 115 taking 1 (46.2) for each hit; 53 (reason 24, killer = bot id) 2 ms after the
  fatal 115; then the weapon-54 115 (3.3).
* **Remote death (RPC 166)**, same frame:
  * `CPed::Teleport(own position)`, task reset, Remove/Add;
  * `0x0321 EXPLODE_CHAR_HEAD` under PlayerInFocus = slot → damage calc (null, 1000, 34, head) → `ComputeWillKillPed`
    **is reached** (330);
  * next frame: health 0, `SetPedState(54)`;
  * no `RemoveBodyPart`: the head stays on.

### TRACE-7 additions (runs `orig-20261104-231851`, `orig-20261104-234135` lag-comp 1; `orig-20261104-235739` mode 2; `orig-20261105-000102` mode 0)

Tools: `tools/trace/combat2.sh PART…` (parts `cross ht1 proj satchel rproj gap masks skills driveby rmb respawn`, header
of the script), `uifs.pwn` commands `explodeN` / `specoff`, tracer records `proj` / `proj_end` / `pad` / `hp_mid` and hooks
`CWeapon::FireProjectile`, `CExplosion::AddExplosion`, `CCamera::Process` (counted; `hp_mid` sampling point after
`CWorld::Process`), all in `crates/samp-tracer/src/win/combat.rs` / `hooks.rs`; plus `CTxdStore::LoadTxd(file)` /
`AddTxdSlot` and `CWorld::FindGroundZFor3DCoord` for the world follow-ups, and the `combat2.sh world7` part (`findz` uifs
command). `CTxdStore::LoadTxd(file)` steals only its first 5 bytes: the protection code reads 0x7320B5.

#### Remote 206 hit type 0 whose ray crosses the local ped — MEASURED (mode 1)
* Geometry: the shooting bot (Deagle, 10 m ahead, facing the local player) aims its 203 front at a second bot that stands
  2.2 m below ground 8 m behind the local player. GTA's crosshair ray is the synced front turned 4.57° up and 2.56° to the
  right (measured from TRACE-6's ray), so the ray passes through the local ped's torso.
* Per 206 (4 of 4): `FireInstantHit(remote, pif 0)` → `Find3rdPersonCamTargetVector(105)` → original LOS (site 0x740721)
  **hits the local ped** (col point ≈ (1700.0, −2599.84, 13.66), torso) → samp clears the result: `DoBulletImpact` gets
  victim null and hit point (0, 0, 0). No damage calc, no `ComputeWillKillPed`, no 115, health unchanged.
* So the local-hit suppression of `remote-control-combat.md` §3.4 is confirmed live: a remote shot that did not report
  the local player never hurts him, even when its ray goes through him.
* **Mode 2** (selector S 1, run `orig-20261104-235739`): identical, 5 of 5 shots: the original LOS hits the local ped
  (torso col point), `DoBulletImpact` gets victim null / hit (0, 0, 0), no damage.
* **Mode 0** (selector S 2, run `orig-20261105-000102`): the server relays **no 206** (0 received). samp's LOS hook is
  inactive; the bot's FIRE key alone makes GTA fire for the remote ped (`FireInstantHit` from `CWeapon::Fire`'s site
  0x74254D, PlayerInFocus = remote slot, 26 shots, one every ~0.72 s = Deagle rate) along the synced camera ray. With the
  crossing geometry 12 of them hit the local ped: damage 140 → 46.2 each, RPC 115 taking 1 (attacker = bot id, body part
  3/6/4 from the col point), death 53 (reason 24, killer = bot id) after three hits.

#### Remote 206 hit type 1 in modes 0 and 2 — MEASURED
* TRACE-6's mode-0/2 bot runs failed for a script reason: the previous idle bot had not left yet, so `weapon|B1` armed the
  wrong bot. `combat2.sh` now waits for the server's disconnect line before starting the next bot.
* **Mode 2**: 206 hit type 1 at the local id with offset (0, 0, 0.3) (world axes): `FireInstantHit(remote, pif 0)` →
  LOS start = remote hand origin, end = 2P − start with **P = local ped position + offset** (1700.0, −2600.0, 13.847) →
  hits the local ped → damage 140 → `ComputeWillKillPed` 46.2 → 115 taking 1 (46.2, weapon 24, part 3) per 206; death 53
  (24, bot id) after the third.
* **Mode 0**: no 206 reaches the client. The bot's own key-driven shots are simulated locally (35 `FireInstantHit`, pif =
  remote slot); with the bot aiming its camera front at our torso the crosshair ray passes above the head, so none hit
  (35 × `DoBulletImpact` victim null), no damage, no 115.

#### Projectiles — MEASURED (local and remote)
* **Local** (grenade 16, molotov 18, satchel 39 + detonator 40, RPG 35, heat seeker 36; `combat2.sh proj` / `satchel`):
  * nothing combat-specific is sent: **no 206, no 115** (no other player was hit); only 207/203 and a 204 slot record
    after each ammo change. The detonator appears as slot 12, weapon 40, ammo = satchels thrown.
  * GTA path, all with PlayerInFocus 0: `CWeapon::Fire` (thrown: from the throw task, ret 0x62B12F; rockets: from the
    gun task, ret 0x61ECD2) → `CWeapon::FireProjectile` (force 0.76–0.90 for thrown, 0.0 for rockets) → a
    `gaProjectileInfo` entry the next frame. Rockets and heat seekers are stored as weapon type **19** (model 345);
    grenade 342, molotov 344, satchel 363. Initial move speed: grenade (0, 0.347, 0.263), molotov (0, 0.347, 0.201),
    satchel (0, 0.16, 0.11), rocket (0, 0.400, −0.020) units/frame.
  * Explosion types (`CExplosion::AddExplosion`, creator = local ped): grenade 0, molotov 1, rocket/HS 2, satchel 0.
    Grenade/molotov explode ~105/61 frames after the throw, rockets on impact (~45 frames at 25 m).
  * Satchel: with two satchels a second LMB throws the second one (no detonation). With one satchel the next LMB fires
    the detonator: `CWeapon::Fire(40)` → `AddExplosion(type 0)` at the satchel in the same frame (caller 0x7388BC,
    `RemoveDetonatorProjectiles`), entry gone the next frame. The local player's thrown satchels are removed when he
    respawns (both `proj_end` in the respawn frame).
* **Remote** (`combat2.sh rproj`: a bot holding AIM+FIRE with weapon 16 / 35 / 39 for two 900 ms "shots"):
  * Created locally from the keys, inside the remote's processing: `CWeapon::Fire` and `CWeapon::FireProjectile` run
    with **PlayerInFocus = the remote slot** (2), creator = the remote ped; one projectile per FIRE press (GTA ammo for
    the remote is 9999). Rockets fly along the remote's synced camera front.
  * Their explosion damages the local ped through GTA: grenade 196.24 and RPG 250.0 per damage event, and there are
    **two damage events for one explosion** in the same frame → two RPC 115 (taking 1, attacker = the bot's id, weapon
    51, 64.758 / 82.5 each = 0.33 × damage), then 53 (reason 51, killer = bot id) 6 ms later, then 115 (weapon 54, 3.3)
    from the fall. Nothing comes from the network for these projectiles.
  * A remote's satchel lies at the throw spot with the remote as creator; it was removed (`proj_end`, no explosion) when
    the bot left the server. Remote detonation not measured (the bot cannot switch to the detonator).

#### MR-7.10 one-frame health gap — MEASURED (unreachable by explosions)
* `CreateExplosion` type 7 (1× and 3× in one server tick) and type 6 (6× in one tick, 0.5 m from the ped) at an idle
  remote ped: `AddExplosion` runs (via SCM 0x0948) but the remote ped gets **no damage event at all** (no damage calc,
  no movement, health stays 1000, no `hp_mid` record).
* Reason: at stream-in samp runs SCM 0x02AB `SET_CHAR_PROOFS(ped, bullet 0, fire 0, explosion 1, collision 0, melee 0)`
  on every remote player. Peeked physical flags of a remote ped: 0x10800212 (bit 23 `bExplosionProof` set; uses-collision
  bit of +0x1C set). GTA's explosion code skips explosion-proof entities.
* `hp_mid` (remote health ≤ 0 or state 54/55 after `CWorld::Process`) never fired in either run (gunfire, melee from
  TRACE-6 scenarios, explosions). The gap stays theoretical; no measured path reaches it.

#### Carry / cuffs masks (MR-7.12) — MEASURED (`combat2.sh masks`, `pad` records = `Pads[0]` at frame start)
| action | sprint (Space → Cross) | jump (Shift → Square) | crouch (C → ShockL) | weapon |
|---|---|---|---|---|
| none | Cross 255, move state 7 | Square 255, in air | ShockL 255, ducking | — |
| 25 carry | Cross stays 0 (new and old), move state 6 | Square 0, never in air | ShockL 0, never ducking | — |
| 24 cuffs | Cross stays 0, move state 6 | works (in air) | works (ducking) | Deagle given while cuffed → armed weapon reset to fists the next frame (`SetCurrentWeapon(0)`); after special action 0 a new Deagle stays |

#### Weapon skills (MR-7.5) — MEASURED
* `peek 0xB79494 44` (the 11 weapon-skill stats): all 1000.0 after spawn and after every respawn; RPC 34 writes the exact
  value (pistol 500 → 500.0, silenced 0 → 0.0). RPC 40 (session reset to 999) not measured (needs a reconnect).
* Remote Colt (bot aiming, `SetPlayerSkillLevel(bot, 0, n)` → RPC 34 for the bot): skill 0 → one pistol, the other arm
  down; 999 and 1000 → dual pistols (999 is already GTA's top level). So the per-slot skill table filled by RPC 34 drives
  the remote ped's dual wield. Screenshots `s-remote-colt-{0,999,1000,1000b}.png`.

#### Drive-by (MR-7.6) — MEASURED except remote firing (`combat2.sh driveby`, police car 596, Uzi 28)
* **Local passenger**: LMB alone does nothing. The in-car horn key (H → `Pads[0].ShockButtonL`) starts it: in the same
  frame samp re-arms the weapon (`SetCurrentWeapon(28)`) and runs SCM 0x0713 `TASK_DRIVE_BY(ped, −1, −1, 0.0, 0.0, 0.0,
  300.0, 8, 1, 20)`. Static (B 0x10007b76 / 0x100acfa0): start when ShockL is pressed, not active and the ped is in a
  vehicle; weapon 46 → armed 0, no task; active flag cleared (task deleted) when the weapon is 0 or a weapon type > 15
  (≠ 46) has no total ammo.
  * Sync: 211 `drive_by` bit set from the start frame on; 203 cam_mode 55, zoom 63, weapon_state 2, sent about every
    110 ms while the camera moves (identical ones dropped by the 501 ms duplicate gate, so ~1/s when still).
  * Shots: `CWeapon::Fire` from the gang drive-by task (ret 0x62832D) → samp's `FireInstantHit` site (0x74254D) → samp
    LOS hook (site 0x740721) → 206 for every shot (9 frames apart), plus 204 slot records. Misses send 206 hit type 0
    with offset (0, 0, 0) and a garbage hitPos (see the correction in "Local player shooting").
* **Local driver** (Q or E held + LMB / Ctrl): GTA fires through `FireInstantHitFromCar2` (`DoBulletImpact` caller
  0x73CD97), not through `CWeapon::Fire` → no samp LOS hook, **no 206**. 200 keys 256 + 4 (look left + fire) and 64 + 1
  (look right + action); 203 cam_mode 18 about every 1 s with the front pointing to the side; shots every 4 frames.
* **Remote passenger** (bot bit 6): samp gives the weapon (`GiveWeapon(28, 9999)` + `SetCurrentWeapon`), then SCM 0x0713
  with the same parameters; from then on `Find3rdPersonCamTargetVector` runs every frame with PlayerInFocus = the remote
  slot (the synced camera aims the drive-by). Firing not measured (the bot cannot press FIRE as a passenger); remote
  driver drive-by not measured.

#### Respawn variants (MR-7.9) — MEASURED (`combat2.sh respawn`)
* `ForceClassSelection` then health 0: 53 (reason 255, killer 0xFFFF) 11 ms after RPC 14, 115 (weapon 54, 3.3) +17 ms,
  then **RPC 128 (request class 0) 3.63 s after 53** instead of 52; spawn only after Shift (129 → 52).
* Health 0 while spectating: the client sends **52 first** (23 ms after RPC 14), then 53 and 115 (weapon 54) in the same
  burst, and a normal 52 again 3.61 s later. After `TogglePlayerSpectating(0)` nothing more is sent.

### Still open (M7)
* All TRACE-6 open items are measured (see TRACE-7 additions). Remaining, each TODO-unknown with the reason:
  * remote passenger drive-by **firing** and remote driver drive-by: samp-bot cannot press FIRE while seated;
  * remote satchel detonation: samp-bot cannot switch to the detonator;
  * RPC 40 resetting the skill stats to 999: needs a reconnect inside one session (not scripted);
  * the 116 (attached-object editor) drag rate (world entities).
* Hazards seen in the trace lane (original client): entering a Sultan as a passenger crashed in GTA's radio stream
  thread (division by zero at 0x4F1464, `CAEStreamThread`); a `ui.sh d3d all` capture across a material-text render
  ended in a crash in the next frame (EIP 0x222C3330, ESI = the D3DX font's glyph texture). Use emergency vehicles for
  passenger tests and `ui` / `text` d3d modes for long captures.

## Remaining behaviour (TRACE-8)

Original client (samp.dll 0.3.7-R4 build B), `SAMP_TRACER_BASS=1 TRACE_GAMEMODE=tracegm run.sh orig hold`, run
`target/trace/orig-20261105-003749` (LS airport tarmac, ~53 frames/s), driven with `ui.sh` and the TRACE-8 `uifs.pwn`
commands (list at the top of the file). Statics are build B addresses (`B 0x…`). "Per frame" means once per
`CGame::Process`. Unless stated otherwise every script command below is issued by samp.dll through its executor.

**Tracer additions (TRACE-8, additive):** hooks `CAEAmbienceTrackManager::Play/StopSpecialMissionAmbienceTrack`
(0x4D6D50/0x4D6D60), `CAudioEngine::StartRadio/StopRadio` (0x507DC0/0x506F70, client callers only), `CShopping::LoadShop`
(0x49BBE0), `CEntryExit::TransitionStarted/TransitionFinished` (0x43FFD0/0x4404A0), `CCamera::SetCamPositionForFixedMode`
(0x50BEC0), `TakeControlNoEntity` (0x50C8B0), `VectorMoveLinear` / `VectorTrackLinear` (0x50D160/0x50D1D0; GTA's first
argument is the END point); the opt-in BASS call log (`Audio (TRACE-8)`); `tools/trace/icyserve.py` (local stream server).
`uifs.pwn` TRACE-8 commands (bounds, gravity, drunk, fstyle, shop, wide, campos/camlook/ipos/ilook/camobj,
specon/specpl/specveh, anim/clranim, sound, audio/audiostop, check, paint/vmod/vcolor, crime, vw, wanted, stunt, vel,
money, clock, vehcol, camtarget, kick, ban, rcon) plus `OnClientCheckResponse` / `OnPlayerInteriorChange` logging.
Every trace launch writes the SA:MP client config it used (source, sha256, content) to `$OUT/samp-cfg.txt`
(`trace_log_cfg`); cfg experiments pass `TRACE_SAMP_CFG=/path` (installed by `tools/wine-env.sh`), never edit the
prefix.

### Camera RPCs — MEASURED
* **157 SetPlayerCameraPos**: `0x0925`, then `0x015F(x, y, z, 0, 0, 0)` → `CCamera::SetCamPositionForFixedMode(pos, up=0)`.
  No camera-mode change until the look-at arrives.
* **158 SetPlayerCameraLookAt**: `0x0925`, then `0x0160(x, y, z, cut)` → `TakeControlNoEntity(pos, switch = cut, who = 1)`;
  `cam_mode` becomes 15 (fixed) in the next frame. Cut 0 was sent as 2 (`0x0160 … 2`), cut 1 stays 1 (switch 1 =
  interpolate), 2 = jump cut.
* **162 SetCameraBehindPlayer**: `0x0930(0)`, `0x0925`, `0x0373` → `SetCameraDirectlyBehindForFollowPed_CamOnAString`,
  `0x02EB` → `RestoreWithJumpCut`; `cam_mode` 4 next frame. SetPlayerPos (12) from the filterscript is always followed
  by this sequence because `uifs.pwn` calls SetCameraBehindPlayer.
* **82 InterpolateCameraPos** (`isPosition` = 1): `0x0925`, `0x0930`, `0x0936(from, to, ms, cut)` →
  `VectorMoveLinear(end = to, start = from, time_ms = ms, ease = 1)` in the same frame. Cut 1 and cut 2 both give
  ease = 1 at the GTA level. **InterpolateCameraLookAt** (`isPosition` = 0): `0x092F(1)` then `0x0920(from, to, ms, cut)`
  → `VectorTrackLinear(end = to, start = from, ms, 1)`. time = 0 → nothing at all (no script command).
* **81 AttachCameraToObject**: from the next frame on, **every frame** while attached: `0x0925`, `0x0930`,
  `0x0936(objpos, objpos, 100, 1)` → `VectorMoveLinear(objpos, objpos, 100 ms, 1)` with the object's current position
  (it follows a moving object frame by frame; observed during MoveObject). Ends with 162.
* **111 TogglePlayerWidescreen**: chat line `Widescreen = 1` / `Widescreen = 0` (also in chatlog.txt), `0x02A3(v)` →
  `CCamera::SetWideScreenOn/Off` in the same frame; `watch widescreen` flips one frame later.

### Spectating (124/126/127) — MEASURED (bot `SpecBot` walking, police car 596)
* **124 on**: the first spectator tick (2 frames after the RPC) teleports the local ped to the camera matrix
  position + (0, 0, 20), then with no target: `0x0925`, `0x015F(50, 50, 50, 0, 0, 0)`, `0x0160(60, 60, 50, 2)`
  (cam_mode 15). **Every frame** afterwards the local ped is teleported to (camera position + 20 m on z) — with this
  camera (50, 50, 70); `CTaskComplexInAirAndLand` (vtable 0x8704AC) is set on it each frame because it is falling.
  The server then streams everything out (the synced position is at 50,50).
* **126 PlayerSpectatePlayer**: once the target is streamed in (here 5 frames later, after the server streamed the bot
  back in): `0x04BB(0)`, `0x0860(ped, 0)` (interior 0, no scene load at the target), then
  `0x0159(target handle, mode, 2)` → `CCamera::TakeControl(target ped, mode, switch 2, who 1)`. Modes measured:
  NORMAL → 4, FIXED → 15, SIDE → 14 (cam_mode follows next frame). When the target streams out the client falls back
  to the (50,50,50) → (60,60,50) view (seen in the FIXED step).
* **127 PlayerSpectateVehicle**: `0x04BB(0)`, `0x0860(ped, 0)`, `0x0158(vehicle handle, mode, 2)` →
  `TakeControl(vehicle, mode, 2, 1)`; NORMAL → 3, FIXED → 15.
* **124 off**: `0x02EB` → `RestoreWithJumpCut` (×2), `0x0930(0)`, `0x0925`, `0x0373`, then the spawn request (RPC 52 path:
  `0x0053` create at the spawn point) 2–3 frames later; cam_mode 4.

### World bounds (17) — MEASURED (static B 0x10003F40 on foot, 0x10003FB0 in a vehicle; helper B 0x1009F450)
* The RPC only stores the four floats. The check runs from the local player's sync update, **only when
  `0x077E GET_AREA_VISIBLE` returns 0** (outside interiors; measured: in interior 1 no reaction while outside the
  bounds, in interior 0 immediate reaction).
* Out of bounds (strict compare: x > maxX, else x < minX, else y > maxY, else y < minY): if the entity's move speed on
  that axis is non-zero it is set to ∓0.2 on that axis (towards the inside) and speed z := 0.1; the entity's matrix is
  then re-set with position z += 0.04; on foot `0x01B9(player char, 0)` (SET_CURRENT_CHAR_WEAPON fist); then
  `0x00BE` + `CMessages::AddBigMessage("Stay within the ~r~world boundries", 1000 ms, style 5)`.
* Rate: every frame while walking against the boundary (135 messages in 2.6 s); while standing still outside it ran
  only 3 times in 4 s (the check is tied to the on-foot send cadence). Walking north into maxY = −2595 bounced the
  player up to z ≈ 23.5 and back south.

### Gravity (146 and SetGravity) — MEASURED
* `SetPlayerGravity(0.002)` → float at 0x863984 = 0x3B03126F (0.002) the next frame; global `SetGravity(0.005)` →
  0x3BA3D70A; 0.008 = 0x3C03126F is the default. No script command, no other write.

### Interiors (156, enex) — MEASURED
* **156 SetPlayerInterior(i)**: `0x04BB(i)`, `0x0860(player char, i)`, `0x04E4(x, y)` (load scene at the ped) in the
  same frame; `watch cur_area/player_area` follow; the client then sends **RPC 118** (rel 8, payload = interior byte)
  once per change (seen 01 then 00).
* **Entry-exit walk-in** (Alhambra `BAR1`, outside 1833.4,−1681.75): GTA's own flow, **no samp script command in it**:
  `CEntryExit::TransitionStarted(player)` → camera `SetCamPositionForFixedMode` + `TakeControlNoEntity(switch 2)`,
  `controls_disabled` = 8, then `CTaskComplexGotoDoorAndOpen` (vtable 0x86FFC0) set as primary task;
  `CEntryExit::TransitionFinished` is then called every frame during the fade (55 frames ≈ 1.0 s); `cur_area` = 17
  one frame after the start; at the end `CTaskSimpleUninterruptable`, `RestoreWithJumpCut`, `CStreaming::LoadScene`,
  `CPed::Teleport` into the interior (493.39, −22.72, 1000.69), the `vending` script starts. What samp changes in the
  flow is only static (CSV rows: `GenerateAmbientPeds` NOP in TransitionFinished, `Interior_c::AddPickups` NOP,
  `InteriorGroup_c::Exit` NOP) plus InitGame's "disable enter/exits" (B 0x100A1570 clears the flags word +0x30 of
  every `CEntryExitManager::mp_poolEntryExits` entry when Settings+0x1A is set). RPC 118 reports area 17.
* Walking out runs the same functions in reverse (TransitionStarted → GotoDoorAndOpen → TransitionFinished × N →
  LoadScene + Teleport to the outside marker, `vending` shut down).

### Animations (86/87) and the 207 animation table — MEASURED
* **86 ApplyAnimation**, library loaded: `0x04EE(lib)` (HAS_ANIMATION_LOADED) then
  `0x0812(player char, name, lib, delta, loop, lockX, lockY, freeze, time)` (flags in this order as script params;
  e.g. `WALK_DRUNK 4.1 1 1 1 0 0`, `KO_skid_front 4.1 0 1 1 1 0`, `IDLE_CHAT 4.1 0 0 0 1 1500`) →
  `CTaskManager::SetTask(CTaskSimpleRunNamedAnim 0x86D54C, primary 3)` next frame.
* Library **not loaded** (first `DANCING` call): `0x04EE` false → `0x04ED(lib)` (REQUEST_ANIMATION →
  `CStreaming::RequestModel(25631, 4)`) → 16 more `0x04EE` polls with `Sleep(1)` inside the same frame (~15 ms) →
  gives up: **no 0x0812, the animation is not played**. The same RPC a few seconds later (block now loaded) plays.
* Library `SEX` → nothing at all. Unknown library (`NOSUCHLIB`) → nothing at all (no script command).
* **87 ClearAnimations** (local): `CPed::Teleport` to its own position (flushes tasks; `CTaskSimplePlayerOnFoot`
  0x8708EC re-set), nothing else.
* **Anim-name table (where samp gets it, how it is used):** a static 1812 × 36-byte table of `"LIBRARY:NAME"`
  upper-case strings in samp.dll's `.data` at **B 0x101039E8** (index 1 = `AIRPORT:THRW_BARL_THRW`, 1660 =
  `VENDING:VEND_USE`, 1812 = `SAMP:FISHINGIDLE`); 1-based (accessor B 0x1009CF00 returns `0x101039C4 + 36·i` for
  1 ≤ i ≤ 0x714). At start-up (B 0x1009CF30, from B 0x100A091F) samp computes for every entry
  `CKeyGen::GetUppercaseKey(NAME part)` (GTA 0x53CF30) into a 1812-dword array (B 0x10149190).
  * Sending (207 animation field): the local clump's `CAnimManager::AddAnimation/BlendAnimation` hooks record
    (group, id); `CAnimManager::GetAnimAssociation(group, id)` (GTA 0x4D3A60) → hierarchy hash (`+0x10 → +0`) →
    first table index with that hash (B 0x1009D000; names that exist in several libraries resolve to the first).
    While a script anim (`CTaskSimpleRunNamedAnim`) runs, the index comes from a linear upper-case
    `"LIB:NAME"` string search instead (B 0x1009CE20) and the flags carry loop/lock/freeze/time.
  * Measured sent values (index, upper 16 bits): IDLE_STANCE 1189 / 0x80E8 (idle), RUN_PLAYER 1231 / 0x80E8,
    FALL_LAND 1133 / 0x8064, WALK_DRUNK 1260 / 0x1704 (scripted: delta 4, loop+lockX+lockY, time code 1),
    KO_SKID_FRONT 1209 / 0x1E04 (lockX+lockY+freeze), IDLE_CHAT 1184 / 0x2804 (freeze, time code 2 = time > 0),
    DNCE_M_A 411 / 0x1104 (loop). Bit 15 set = "plain blend anim" (no script flags).
  * Remote side: `remote-control-onfoot.md` §4.1 (index → name via the same table, played with `0x0812`).

### Drunk (35) — MEASURED
* samp issues `0x052C(player 0, value)` **every frame** (value 0 when sober). Level > 2000: value = level / 50
  (5000 → 100, 2100 → 42), the level drops by 1 per frame (value 100 → 99 → … one step per ~50 frames); at ≤ 2000
  the value becomes 0 (2100 reached 0 after ~100 frames).
* Driving (Sultan, level 5000→4900, W held): the vehicle's turn speed z gets ±0.012 added by the client between
  frames, every ~11 frames (≈ 210 ms; some 22-frame gaps), only while moving (speed 0.09–0.51 observed). The ±0.015
  (level ≥ 5000) case was not seen (the level is below 5000 one frame after the RPC).

### Fighting style, shop name, controllable, misc — MEASURED
* **89**: `0x07FE(player char, style, 6)` (style 6 and 4 measured).
* **33 SetPlayerShopName**: `"FDPIZA"` → `CShopping::LoadShop("FDPIZA")` (opens `data\shopping.dat` twice); an
  11-char name → no LoadShop; **empty name → `LoadShop("")`** too (length ≤ 8 includes 0).
* **15 TogglePlayerControllable 0**: `0x01B4(0, 0)` → `CPlayerInfo::MakePlayerSafe(1, 10.0)` (sets
  `CTaskSimpleStandStill`-type task 0x86DD2C), `0x04D7(char, 1)`; `controls_disabled` = 32. **1**: `0x01B4(0, 1)` →
  `MakePlayerSafe(0, 10.0)`, `0x04D7(char, 0)`, then `CPed::Teleport` to its own position.
* **133 wanted 3** → byte 0x58DB60 = 3 (HUD only); **104 stunt 1/0** → dword 0xA4A474 = 1/0; **18 money 500** →
  `0x0109(0, 500)` (both money fields at 0xB7CE50 = 500); **30 clock 1/0** → byte 0x52CF10 = 0x56 / 0xC3;
  **90 velocity (0,0,0.5)** → no script command (direct write; the ped was launched and fall damage followed);
  **167 / 170** → no GTA-side effect in the trace (flags only; no RPC 168 was sent: nothing targeted).
* **96 SCM events (S→C)**: open.mp sends them with player id 0xFFFF (paintjob: arg2 = garbage). On a streamed-in
  Sultan, same frame as the RPC:
  * type 1 paintjob 1 → `0x06ED(car, 1)`;
  * type 2 component 1010 → `0x06EA(1010)` (HAS_VEHICLE_MOD_LOADED, false), `0x0247(1010)` →
    `RequestModel(1010, 12)` + `RequestModel(20064, 12)`, `0x038B` → `LoadAllRequestedModels(0)`, `0x06EA` ×2,
    `0x06E9(1010)`, `0x06E7(car, 1010, out)` (ADD_VEHICLE_MOD); 1080 the same;
  * type 3 colour (3, 6) → no script command and no hooked call: static B 0x100B72F0 writes the colours directly to
    `CVehicle+0x434/+0x435` (primary/secondary) and keeps them in samp's vehicle object (+0x79/+0x7A, flag +0x7B);
    a per-vehicle tick (B 0x100B7340) re-writes them whenever GTA's bytes differ, except while the car is in a
    mod shop (+0x71);
  * after a stream-out/in the re-created car gets the components (`0x06EA`/`0x06E7`) and `0x06ED(car, 1)` in its
    creation frame.
* **103 ClientCheck** — layout and reply behaviour measured (`check|L|type|arg|offset|count`; replies seen in
  `OnClientCheckResponse` and in the capture, rel 9 ch 0, sent one frame (~16 ms) after the request):
  * reply payload = `u8 type, u32 arg, u8 result` (e.g. `02 12020010 01`).
  * type 2 (no inputs used): arg = the local ped's (on foot) or vehicle's (in a vehicle) dword at entity +0x40
    (measured 0x10000212 on foot, 0x10000202 driving), result = 1 on foot / 2 in a vehicle.
  * type 5: arg = a gta_sa.exe address in [0x400000, 0x856E00] (else no reply), result = an 8-bit checksum of
    `count` bytes at arg + offset (0x400000/0/4 → 0x84).
  * type 0x45: arg ≤ 800000 = offset into samp.dll's image; same checksum (0/0/4 → 0x84, the "MZ" header like 5).
  * type 0x46: arg = model id; checksum over that model-info entry (411/0/4 → 0x88).
  * type 0x47: arg = model id; the model is requested and loaded (Sleep(1) polling up to 1000 ms, then released if
    unused); checksum over its collision data (411/0/4 → 0x0C).
  * type 0x48: no inputs; arg = `(tick count & 0x0FFFFFFF) | 0x30000000` (two requests 2.1 s apart differed by 0x83B),
    result 0.
  * no reply: offset > 256, count > 256 (count 300 measured), count < 2, any other type (0 measured).
  * The checksum function (B 0x1000EA50) is not described here (anti-cheat; derivation intentionally not recorded,
    as in `session-misc.md`); TODO-unknown for an implementation that wants to answer types 5/0x45/0x46/0x47.
* Not sent by open.mp 1.6.7 in these steps: 48 (SetPlayerVirtualWorld — the server streams instead), 112
  (PlayCrimeReportForPlayer with a bot suspect) → TODO-unknown (needs a server that sends them).

### Per-frame noise seen in every window
samp runs these script commands every frame regardless of the scenario (exclude them when diffing):
`0x00C0(12, 0)` (time, while the clock is server-held), `0x04EE` for PARACHUTE, BAR, GANGS, PAULNMAC, VENDING,
FIGHT_B…E, `0x0248` for models 3131, 1485, 1486, 1543, 1546, 343, 342, 344, 330, `0x052C(0, drunk)`, `0x09BA(0)`,
`0x0470(1, …)`, `0x077E` (GET_AREA_VISIBLE, 2–3×).

## Audio (TRACE-8)

Same run (`orig-20261105-003749`). Audio stayed inaudible: Wine output goes to the private `samp_rs_null` sink
(`tools/wine-env.sh`); everything below comes from the BASS call log, not from listening.

**Tool:** `SAMP_TRACER_BASS=1` makes the tracer redirect the client module's 17 `bass.dll` import-address-table slots
to logging wrappers (`crates/samp-tracer/src/win/audio.rs`, decoding in `src/bass.rs`; neither samp.dll's code nor
bass.dll is modified). Record `bass`: `call`, decoded arguments, return value, `dur_ms`, `thread` (`main` = the
`CGame::Process` thread, `worker` = samp's stream thread). Calls identical to the previous call of the same function
are folded (`repeats_before`, `repeat_span_ms`). `tools/trace/icyserve.py PORT FILE LOG` serves `/plain.mp3`
(plain HTTP), `/icy.mp3` (`ICY 200 OK`) and `/icyh.mp3` (Icecast-style `HTTP/1.0 200 OK` + `icy-*` headers +
`StreamTitle` metadata every 8192 bytes, title changes every 10 s) and logs each request.

### BASS imports and start-up — MEASURED
* samp.dll imports from bass.dll (static import): SetEAXParameters, SetConfig, SetConfigPtr, Init, Free, StreamFree,
  ChannelGetTags, StreamGetFilePosition, ChannelSetSync, ChannelPlay, StreamCreateURL, ChannelStop, Apply3D,
  ChannelSet3DPosition, ChannelSet3DAttributes, Set3DPosition, Set3DFactors.
* The five 3D functions are referenced only from B 0x10066B90 (Set3DFactors(1, 1, 2), Set3DPosition from the camera,
  Set3DAttributes(mode 1, min 10, max 100, 360, 360, 1.0), ChannelSet3DPosition, Apply3D, with debug prints), which has
  **no caller** (dead code); **none was called at runtime**. 3D falloff is done by volume (below).
* Boot (frame 34, main thread, from the boot routine B 0x100C47B0 after the D3D device exists): `BASS_Free()`,
  `BASS_Init(-1, 44100, 0, NULL, NULL)` = 1 (6 ms), `SetConfigPtr(NET_AGENT, "SA-MP/0.3")`,
  `SetConfig(GVOL_STREAM, radioVolume × 7000)` (= 7000 with radio volume 1.0), `SetConfig(NET_PLAYLIST, 1)`,
  `SetConfig(NET_TIMEOUT, 10000)`, `SetEAXParameters(-1, 0.0, -1.0, -1.0)` (EAX off). With `audioproxyoff=1` the
  client also calls `SetConfigPtr(NET_PROXY, NULL)` (static, B 0x100664E8; not exercised: default 0).
* HTTP request seen by the server: `GET /x.mp3 HTTP/1.1`, `Host: …`, `User-Agent: SA-MP/0.3`, `Icy-MetaData: 1`.

### PlayAudioStreamForPlayer (41) / StopAudioStream (42) — MEASURED
* Chat line `Audio stream: <url>` (also in chatlog.txt) unless `audiomsgoff`. The RPC starts the worker
  (2–4 frames ≈ 60–150 ms later the first BASS call appears). Worker sequence:
  `StreamCreateURL(url, 0, 0x940000 = BLOCK|STATUS|AUTOFREE, NULL, NULL)` (52–82 ms, blocking) →
  `ChannelPlay(h, 0)` → `ChannelGetTags(h, ICY)` → `ChannelGetTags(h, HTTP)` → `ChannelGetTags(h, META)` →
  `ChannelGetTags(h, OGG)` → `ChannelSetSync(h, META, 0, proc, 0)` → `ChannelSetSync(h, OGG_CHANGE, 0, proc, 0)`, then
  a loop every **20 ms**: `StreamGetFilePosition(h, 4 = BASS_FILEPOS_CONNECTED)` + `SetConfig(GVOL_STREAM, v)`
  (479 iterations in 9.59 s).
* Station / title: with the Icecast-style answer `ChannelGetTags(HTTP)` returned the `icy-name`/`icy-url` lines and
  `META` returned `StreamTitle='Trace Artist - Title 0';`; the META sync callback then calls `ChannelGetTags(META)`
  on every metadata block (~0.46 s here). The bottom-left line `Trace Artist - Title 0 - TraceStation` was drawn
  (screenshot `audio_icyh.png`, x ≈ 16, y ≈ 700 at 1280x720; exact rect TODO-unknown: no d3d text capture). With the
  Shoutcast `ICY 200 OK` answer BASS returned no ICY/META tags (only `HTTP/1.0 200 OK`), so no title line.
* Volume (`SetConfig(GVOL_STREAM)` values written by the worker):
  * no position: 7000 (= radio volume 1.0 × 7000);
  * with position, distance 50: 7945 at d ≈ 0.33 m, 4799 at d = 20 m, **0 at 60 m**, 7991/7992 back at ≈ 0.1 m —
    i.e. `8000 × (1 − d/distance)` for d < distance, else 0 (linear falloff, no BASS 3D);
  * GTA menu open (Esc): 0 within one 20 ms tick; back to 7000 one tick after the menu closed.
* **Replace** (second 41 one second after the first): the worker of the old stream calls `ChannelStop(old)`
  (130 ms), the main thread `StreamFree(0)` (handle already cleared), then the new worker starts; the old HTTP
  connection closes ~0.3 s after the new request arrives.
* **42 StopAudioStream**: worker `ChannelStop(h)` = 0 and main thread `StreamFree(h)` = 1 (119–213 ms, it blocks the
  main thread that long); the server sees the disconnect.
* Unreachable URL: `StreamCreateURL` returns 0 after 16 ms; samp still calls ChannelPlay/GetTags/SetSync with handle 0
  and keeps looping `StreamGetFilePosition(0, 4)` (= −1) every 20 ms until 42 / the next 41 (no chat error).
* **Radio:** while a stream is "playing" (from the RPC until stop, also for a failed URL) samp calls, **every frame**,
  `CAudioEngine::StartRadio(-1, 0)` and then `CAudioEngine::StopRadio(NULL, 0)` (B 0x10066A40). In a Sultan
  (Radio Los Santos shown on entry) the station name did not reappear after 42; samp makes no radio call after the
  stop (whether GTA's radio resumes by itself: TODO-unknown — needs an audio-level check). samp does not otherwise
  disable the radio (no patch in the CSV touches the radio code).

### PlayerPlaySound (16) — MEASURED (Audio object method B 0x100A22D0)
* 1000–1999 (1052 at a position): `0x018C(x, y, z, id)` (ADD_ONE_OFF_SOUND), same frame.
* 2…999 (5): `CAEAmbienceTrackManager::PlaySpecialMissionAmbienceTrack(5)` (direct call, this = 0x8AC15C).
* 0: `CAEAmbienceTrackManager::StopSpecialMissionAmbienceTrack()`; if a mission sound was started before, `0x040D(1)`
  first (CLEAR_MISSION_AUDIO).
* 1: no observable call (sets a flag byte).
* ≥ 2000 (2000, 3200 at a position): `0x040D(1)`, `0x03CF(1, id)` (LOAD_MISSION_AUDIO), `0x03D7(1, x, y, z)`
  (SET_MISSION_AUDIO_POSITION) in the RPC frame, then **per frame** `0x03D0(1)` (HAS_MISSION_AUDIO_LOADED) until it
  returns true (3 frames here) and then `0x03D1(1)` (PLAY_MISSION_AUDIO) once.

## Client features (TRACE-8)

Runs: `orig-20261105-003749` (local commands, F8, /quit), `orig-20261105-010108` (test sa-mp.cfg at boot, gmx,
lost connection), `orig-20261105-011721` (no-server start, full, kick; own server `max_players` 1 on UDP 7790) and `orig-20261105-012511` (`TRACE_SAMP_CFG` with a quoted fontface; server password). Statics are build B.

### sa-mp.cfg — MEASURED (boot routine B 0x100C47B0, parser B 0x10065F00, writer B 0x10066040)
* Path `<user files>\SAMP\sa-mp.cfg` (`%s\sa-mp.cfg`, same folder as `chatlog.txt`, `screens\`). Lines `key=value`
  written with CR LF. Value types: `"…"` (double-quoted) = string, contains `.` = float, else integer (atol).
  An unquoted string (`fontface=Courier New`) is an integer 0: the face fell back to Arial and the client later
  rewrote the file with `fontface=0`. The client rewrites the whole file whenever it stores a key (local commands,
  creation of missing defaults).
* Created with defaults at boot when missing: `fpslimit=48` (B 0x100C4E4E), `multicore=1`, `audioproxyoff=0`
  (this is the content of a fresh prefix).
* Boot effects measured with a test cfg (`fpslimit=30 multicore=0 audioproxyoff=1 disableheadmove=1
  nonametagstatus=1 timestamp=1 audiomsgoff=1 logurls=1 nohudscalefix=1 pagesize=15 fontsize=1 fontweight=1
  directmode=1`):
  * `fpslimit=N` (20–90, else ignored): samp keeps N (Game+0x5D) and sets GTA's own limiter `RsGlobal.frameLimit`
    (0xC1704C) to 200, i.e. samp limits the frame rate itself. Measured CGame::Process rates: 48 → 53–54/s,
    30 → 31–32/s, 90 → 62/s (renderer-bound). `/fpslimit N` applies immediately.
  * `multicore=0` → `SetProcessAffinityMask(GetCurrentProcess(), 1)` (affinity of gta_sa.exe was `1`; other
    lanes' games `fffffff`).
  * `audioproxyoff=1` → `BASS_SetConfigPtr(NET_PROXY, NULL)` at boot (returned 0 under Wine).
  * `directmode=1` → chat line `ChatWindow: Using direct drawing mode.` printed **before** the `SA-MP … Started`
    line; the chat render-to-texture is not created (B 0x10068210, flag +0x63B2 = 0).
  * `fontsize=1` → main chat font 18 (base 16 at width 1280: < 1024 → 14, < 1400 → 16, < 1600 → 18, else 20;
    plus 2·fontsize), small 16, death-window 16, GTAWEAPON3 24/28, GUI 22/20; SAMPAUX3 22/28 and the plate font
    38/w10 unchanged.
  * `fontweight`: 1 → weight 400 for the main/small fonts (measured), any other value or missing → 700.
  * `fontface="Courier New"` (quoted) → main 16/14, death 14, GUI 20/18 fonts created as `Courier New`; the plate
    font (Arial 38/w10), GTAWEAPON3 and SAMPAUX3 keep their faces. Default `Arial` (B 0x1011748C).
    `fontweight=2` → 700.
  * `timestamp=1` → chat shows `[HH:MM:SS] ` prefixes on screen (screenshot); `pagesize=15` → 15-line page.
  * `nonametagstatus=1` → NetGame+0x234 = 0 at connect (AFK/status icon off); `disableheadmove=1` → Game+0x59 = 0
    (head tracking off); `nohudscalefix=1` → flag B 0x10117470 = 0 (aspect-ratio fix of `gta-patches.md` off);
    `audiomsgoff=1` → no `Audio stream:` line; `logurls=1` → `[url 0x%X]: %s` lines for HTTP model downloads
    (dormant in R4). Their visual effects (head, HUD scale, icons) were not screenshot-compared.
* Local commands (run 003749; chat lines and cfg keys written): `/fpslimit N` → `-> Frame Limiter: N` (fpslimit),
  `/headmove` → `-> Head movements disabled|enabled` (disableheadmove 1/0), `/nametagstatus` →
  `NameTag Player Status: OFF|ON` (nonametagstatus 1/0), `/timestamp` → **no chat line** (timestamp 1/0),
  `/audiomsg` → `Audio messages: Off|On` (audiomsgoff), `/logurls` → `URL messages: On|Off` (logurls),
  `/hudscalefix` → no line (nohudscalefix), `/togobjlight` → no line, no cfg key, `/mem` → `Memory: 1073741824`,
  `/interior` → `Current Interior: 0`, `/cmpstat` → nothing.
* `/save [c]` → `<user files>\SAMP\savedpositions.txt` (`AddStaticVehicle(560,1655.3934,-2514.9600,13.2516,
  38.5217,3,6); // tr8` in a car, CR LF); `/rs [c]` on foot → `<user files>\SAMP\rawpositions.txt`
  (`x,y,z,heading ; comment`), **in a car → `rawvehicles.txt` in the game's current directory** (relative path,
  B string `rawvehicles.txt`; `560,1655.3934,-2514.9600,13.2516,38.5217,3,6 ; tr8`), chat `-> InCar pos saved`.

### chatlog.txt — MEASURED
* `<user files>\SAMP\chatlog.txt`, truncated at every client start (each run's file begins with the first line of
  that run). One line per chat-window line: `[HH:MM:SS] ` (local time; always, independent of `timestamp`) + the
  text **with embedded `{RRGGBB}` codes kept** (the line's own colour is not written) + `\r\r\n` (text-mode file
  given `\r\n`). Debug/info lines (`Connecting to …`, `Audio stream: …`, `Widescreen = 1`, `Screenshot Taken - …`,
  command feedback) are logged like server messages. Written as the lines are added (visible in the file while
  the game runs).

### F8 screenshot — MEASURED
* F8 → `<user files>\SAMP\screens\sa-mp-NNN.png` (`%s\screens\sa-mp-%03i.png`, NNN = next free number; 003 and
  004 after 000–002 existed), chat line `Screenshot Taken - sa-mp-003.png` (error text `Unable to save screenshot.`).
  PNG, 1280x720 (the back buffer size), 8-bit RGB without alpha, ~580 KB, no text chunks. It captures the frame
  including samp's UI (chat). Two presses 2 s apart → two files.

### Connection lifecycle — MEASURED
* **Start**: `SA-MP 0.3.7-R4 Started`, state 1 with the last-attempt time = construction time, so the first
  `Connecting to H:P...` comes 3.0 s later (main loop B 0x1000B5A0 → B 0x10008930: state 1 and > 3000 ms since the
  last attempt → print, RakClient Connect, state 2).
* **No answer**: each attempt opens a new UDP socket (new source port) and sends the 12-byte open-connection
  request 6 times at 1.0 s intervals; after ~6.05 s RakNet reports connection-attempt-failed (0x1D) →
  `The server didn't respond. Retrying..` and state 1 → the next `Connecting to …` immediately (6 s since the last
  attempt > 3 s). Cadence measured: 6.05–6.07 s per attempt (8 attempts).
* **Lost connection** (server process stopped with SIGSTOP 60 s): the client kept resending (~90 ms) on the old
  socket; 12.0 s after the last server packet RakNet reported connection lost (0x21) — M10-P: really 10.05 s
  after the first reliable client message left unacked (here the keep-alive 2.4 s after the stop; RakNet's ack
  timer, m10-notes.md) → in one frame:
  RakClient Disconnect(0), `Lost connection to the server. Reconnecting..`, the RPC 40 reset routine (B 0x1000A530:
  prints `The server is restarting..` too), deletion of all remote players, audio stream stop (wait), state 1 →
  `Connecting to …` in the same second, then the 6-s retry cadence above. When the server resumed, the pending
  attempt connected (`Connected. Joining the game...`) but open.mp still had the old player (timed out 10 s later,
  reason 0), so the join got `CONNECTION REJECTED: Unacceptable NickName` + 3 hint lines
  (`Please choose another nick between and 3-20 characters`, `Please use only a-z, A-Z, 0-9`,
  `Use /quit to exit or press ESC and select Quit Game`) and the client **stopped retrying** (stayed on the
  connecting camera, dead ped).
* **Server restart (gmx, RPC 40)**: chat `The server is restarting..`; same frame: `0x04BB(0)`, `0x0860(char, 0)`,
  `0x04E4` at the ped, `0x0321(player char)` (EXPLODE_CHAR_HEAD: the local ped is killed, damage 1000 weapon 34),
  then **every frame** the "not connected" scene: local ped teleported to (1133.05, −2038.403, 69.1), `0x0925`,
  `0x015F(1093, −2036, 90)`, `0x0160(384, −1557, 20, 2)`. No reconnect (same RakNet connection); 12.0 s later
  open.mp re-sent InitGame (139) → `Connected to {B9C9BF}open.mp server`, model preload checks (`0x0248` for 400…)
  and class selection again.
* Packet handler texts (B 0x1000B250, statically): 0x1F no free incoming → `The server is full. Retrying...` and
  state 1; 0x20 disconnection notification → `Server closed the connection.`, audio stop (wait), RakClient
  Disconnect(2000 ms); 0x23 → `Failed to initialize encryption.`; 0x24 banned → `You are banned from this server.`
  (no state change); 0x25 invalid password → `Wrong server password.` + Disconnect(0).
* **Server full** (max_players 1, the client's old slot still taken after a lost connection): the server answers
  the open-connection request at once (10-byte reply) → `The server is full. Retrying...`, state 1 → next attempt
  (new socket) **3.04 s** later (37.18 → 40.22 → 43.26 → 46.30 → 49.33 s), until a slot is free; then a normal join
  (`Connected. Joining the game...`, InitGame, class selection).
* **Kicked** (`Kick()`): server disconnection notification (16 bytes) → client ack 2 ms later →
  `Server closed the connection.`; the audio stream stops (per-frame radio calls end); **no reconnect, no reset, no
  script command** for 30 s observed: the ped stays where it was (state IDLE, health 100), the world is frozen in
  its last network state. Server reason 2 (kick).
* **Wrong password** (server `password: secret`, client without one): cookie exchange, connection request,
  server reply (16 bytes) 18 ms later → `Wrong server password.`; RakClient Disconnect(0); **no retry** (35 s observed).
* **Banned / wrong version**: TODO-unknown live — open.mp 1.6.7 wrote `127.0.0.1` to bans.json on `banip` but did
  not refuse the loopback reconnect (the client got "full" then joined), and it accepts the R4 version string, so
  neither 0x24 nor RPC 130 reason 1 could be produced locally; texts and handlers are static (above and
  `session-misc.md` RPC 130).
* Attempts with no server running at all (fresh start, run 011721): `Connecting to …` 3.0 s after `Started`, then
  `The server didn't respond. Retrying..` + `Connecting to …` every 6 s.
* **/quit**: the client sends the RakNet disconnection notification (0x20, rel 6 = UNRELIABLE) — the server
  logged reason 1 (quit) — at once; ≥ 1001 ms later it deletes its pools (vehicles deleted, `0x0362` remove char
  from car, ped teleport) and the game exits (M10-P, m10-notes.md: static A 0x100c3ec0 / 0x100c5030).

## Open problems / notes

* `origin` stack scanning is heuristic. For samp's script commands the first two entries
  are samp's own executor frames.
* The stock interpreter inlines `ProcessOneCommand` in `CRunningScript::Process`, so stock
  commands are only visible through the optional handler-table redirect
  (`SAMP_TRACER_SCRIPTS`).
* samp's file-redirect layer (below `CFileMgr::OpenFile`, above `CreateFileW`) was not
  identified exactly.
* TRACE-3/5 open:
  * the 15 s local vehicle delete (blocked by open.mp's respawn);
  * the train exit/warp path;
  * Insert (overwrite) in the edit box;
  * F1 help dialog, net stats (F5), textdraws and selection, model previews;
  * 1920x1080 runs (needs `setmode.py` with the mode list of a 1920x1080 gamescope);
  * MR-5.1 remote-driver correction (bot drive at 40/150 ms); MR-5.6 engine/lights;
    MR-5.9/5.10/5.12.
* The original client once crashed ("SA:MP An exception occured", EIP in a Wine module) after
  xdotool's Insert key switched the input layout and 140 chars were typed into the chat input.
  Not reproduced.
