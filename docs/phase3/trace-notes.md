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

## Open problems / notes

* `origin` stack scanning is heuristic. For samp's script commands the first two entries
  are samp's own executor frames.
* The stock interpreter inlines `ProcessOneCommand` in `CRunningScript::Process`, so stock
  commands are only visible through the optional handler-table redirect
  (`SAMP_TRACER_SCRIPTS`).
* samp's file-redirect layer (below `CFileMgr::OpenFile`, above `CreateFileW`) was not
  identified exactly.
