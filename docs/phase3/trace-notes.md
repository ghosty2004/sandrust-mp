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
* Pause map: not measured.

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

### Still open (M6)
* Edit-gizmo drag updates (117/116 response 2 rate), pause-map gang zones, camera collision (MR-6.12),
  `ProcessVerticalLine` (MR-6.13), the 19300 IPL substitution (not watched).

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
  | nothing (sky) | no 206 |

  * 206 is rel 7 (unreliable sequenced) ch 0, in the shot frame.
  * Quirk: the first missed Deagle shot after a sniper sky-miss sent a 206 with hit_type 0, offset (0, 0, 0) and a
    garbage hitPos (an uninitialised colpoint).
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
  mode 1.
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

### Still open (M7)
* Remote hit_type 0 with a ray that really crosses the local ped (the local-hit suppression, modes 1/2).
* Remote hit_type 1 in modes 0/2 (the bot produced no shots there).
* Drive-by (MR-7.6), carry/cuffs masks (MR-7.12), projectiles/satchels from keys (MR-7.11; `AddProjectile` can't be
  hooked), the one-frame health gap (MR-7.10), respawn after `ForceClassSelection` or while spectating (MR-7.9),
  skills after RPC 34 (MR-7.5).
* The garbage-colpoint 206 after a sniper sky-miss: seen once, not reproduced.

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
