# Phase 3 / EFX-1 — effect tracer

Goal: capture everything one client function does to the game in a short window, and compare
it with what our client does for the same event. Everything here is runtime observation of the
original client (samp.dll 0.3.7-R4 build B, fv0.3.7.3, as installed in `wine/gta-sa-trace`).
The effect logs are behavioural data (addresses written, values, game calls); no client code is
copied or described instruction by instruction.

## Design

Two capture modes in `samp-tracer` (`src/efx.rs` platform-independent, `src/win/efx.rs`
Windows side), both driven through the control channel and written to `<GTA_DIR>/efx.jsonl`
(`SAMP_TRACER_EFX_OUT`).

### Step mode (original client): `efx arm TARGET [opts]`

* TARGET: `rpc:N` (handler VA of the loaded samp.dll build, read from `docs/re/rpc-ids.md`
  which is compiled into the tracer; the build is picked by the PE timestamp), `0x1001…` (VA in
  the client's preferred-base space), `module+0xRVA`, `gta:0xADDR` (a game function) or
  `event:frame` / `event:game_process` (the client function the `Idle` call sites 0x53EB12 /
  0x53E981 were retargeted to — found from the game side, so it also works for our client).
* One-shot `int3` at the target. When it is hit, the tracer
  1. replaces the target's return address with a one-byte `int3` stub (the real one is kept in a
     per-thread list); when the stub is reached, the capture ends;
  2. sets the trap flag and single-steps that thread in a vectored exception handler.
* At every step the instruction at EIP is decoded with **iced-x86** (`efx::analyse`): all memory
  operands (base + index×scale + disp, `fs:` via the TEB, string operands), access kind
  (read/write/read-write), control flow (direct/indirect call and jump targets, returns,
  condition-code users). The written bytes are read before the instruction and again at the
  next trap → one `efx_write` with old/new bytes.
  * iced-x86 1.22, `default-features = false, features = ["std", "decoder", "instr_info"]`: the
    tracer's own `x86.rs` is a prologue-length decoder; computing the memory operands of every
    instruction samp and GTA execute (x87, SSE, string ops, SIB forms, prefixes) needs a complete
    decoder with operand info. Decoder-only, no encoder/formatter; pure Rust, no unsafe
    dependencies added to the DLL.
* What is stepped and what is stepped over:
  * client code and gta_sa.exe code: stepped (writes, calls and returns tracked with a shadow
    stack);
  * other modules (system DLLs, d3d9, the tracer): stepped over with the return stub
    (`efx_ext` record), the trap flag is off meanwhile;
  * private executable memory (trampolines): stepped; the tracer's own hook thunks are skipped
    (EIP moved to the trampoline) so hooked game functions are stepped like the others;
  * game calls are stepped into, except `DEFAULT_STEP_OVER` (model streaming / CD-stream I/O),
    `over=` addresses, calls deeper than `depth=` (24) and `shallow=1`; a top-level game call
    that exceeds `sub=` steps (400 000) is run freely to its end (`efx_truncated`);
  * `rep movs/stos` with DF=0 are performed by the handler in one go (one `efx_write`).
* Records (`fn`): `efx_begin` (target, args, the player objects' addresses), `efx_call`
  (game call: id, depth `d`, target `to` + `loc`, call site `from`, `ecx`/`this`, 6 stack
  arguments + their symbols), `efx_ret` (id, `eax`, `st0` when the x87 stack is not empty),
  `efx_write` (depth, writer `at`, `by` client|game, symbolic address `sym`, `addr`, size,
  old/new hex, `same`), `efx_branch` (a game-memory value read by client code that decides the
  next conditional branch: symbol, value, taken; ≤ `branches=`), `efx_icall` (client-internal
  calls, for context; ≤ `icalls=`), `efx_ext`, `efx_stop`, `efx_end` (steps, ms, counts).
* Which writes are logged: everything except the thread's stack and TEB, the client module's
  own image, and client-code writes to heap memory that is not a known game object (`heap=1`
  logs those too). Game-code writes to any heap block are logged as `heap:0x…`.
* Symbolic addresses (`efx::World`, read when the capture starts): `player_ped`,
  `player_ped.matrix`, `player_veh(.matrix)`, `player_intel`, `player_data`, pool slots
  `ped#N`/`veh#N`/`obj#N`/`task#N`/`event#N`/`intel#N`…, `matrix#N` (gMatrixList), `gta:0xADDR`
  for the exe image. Sizes and pool pointers: gta-reversed (Pools.cpp, VALIDATE_SIZE).

### Window mode (our client): `efx watch SPECFILE [frames=N ms=N faults=N calls=N]`

samp_client.dll has no symbol table and samp-client must not get a tracer hook, so our RPC
dispatch cannot be armed directly. Instead the same GTA-side effects are watched for a window:

* The spec is made from the original's step capture (`efx_diff.py spec`): `mem SYM SIZE` for
  every player object / exe-global address it wrote, `fn 0xADDR NAME` for every game function it
  called at depth ≤ 2. Symbols are resolved in the running game (our player ped etc.).
* The pages holding those addresses get `PAGE_GUARD` (never game code or a stack). On a guard
  fault the handler decodes the instruction, records old bytes of its writes on watched pages,
  single-steps it, records the new bytes and re-arms the page. Every write on a watched page is
  logged (not only the exact addresses), so extra writes of our client to the same objects show.
* The functions get one-shot `int3` call hooks (re-armed after one single step); a hit logs
  `efx_call` with arguments and, through the same return stub, `efx_ret`.
* The window stays open until both `frames` frames and `ms` milliseconds have passed (it is
  opened just before the RPC is fired). At the end, `efx_net` lists the watched addresses whose
  value changed over the window.
* Attribution: `by=client` when the writing instruction is in the client module; otherwise
  `via` = innermost client return address on the stack and `gta_outer` = the outermost game
  return address below it (inside the game function the client called). Both clients run the
  whole game frame from their own frame hooks (`CGame::Process`, `Render2dStuff`), so the diff
  treats effects reached through those (`BACKGROUND` in efx_diff.py) as game processing.
* The same window can be run on the original, so window-vs-window is also comparable.

### Tools (`tools/efx/`)

* `run.sh orig|ours|both`: trace lane (lock, prefix, game copy, server copy of the lane checkout
  `EFX_LANE_ROOT`; DLLs from this checkout's build). `efxfs.pwn` fires exactly one RPC per
  command. For each step of `EFX_STEPS` (default `pos hp skin putin` = RPC 12, 14, 153, 70) both
  clients get the same RPC sequence: capture 1 (first value: orig = step capture of the handler,
  ours = window with the spec of the original's capture 1), capture 2 (second value: window
  capture for both). Every window capture is preceded by a baseline window with the same spec
  and nothing fired (`<step>-<n>-base.jsonl`). `both` then writes `diff-<step>-<n>.txt`.
* `efx_diff.py dump|spec|diff`: drops window records that are background (key also in the
  baseline, or reached through the game frame the client hook runs: `CGame::Process`,
  `Render2dStuff`, `CPlayerPed::ProcessControl`), normalises (pool indexes of non-player objects → `#?`, heap →
  `heap`, pointer-like values → `ptr` or the object they point to, floats rendered, names of
  game functions/globals from gta-reversed) and prints the aligned write table + call diff.

## How to use it for any function

Interactive (private hold session of the trace lane; the session id is printed by run.sh):

```
tools/trace/with-lock.sh bash -c 'tools/trace/run.sh orig hold & …'   # prints "session: ID"
export TRACE_SESSION=ID SAMP_RS_TRACE_LOCK_HELD=1
tools/trace/ui.sh ctl efx arm rpc:61     # or samp.dll+0x1234, 0x1001xxxx, gta:0x5E4110, event:frame
… trigger it (ui.sh key/fs …) …
tools/trace/ui.sh ctl efx status         # "idle done=N" when finished
python3 tools/efx/efx_diff.py dump <segment of wine/gta-sa-trace/efx.jsonl>
```

Scripted: add a step to `tools/efx/scenario.sh` (an `efxfs.pwn` command that fires the RPC and
a `capture STEP 1 RPCID 'cmd|args'` line), then `tools/efx/run.sh both`.

Options of `efx arm`: `budget=` (steps, 3 M), `sub=`, `depth=`, `shallow=1`, `over=0xA,0xB`,
`heap=1`, `branches=`, `icalls=`, `count=` (hits). Keep windows short under Wine (see below).

## Performance and stability (Wine 11, trace lane)

* Step mode: 14–20 µs per single step (175 steps = 9 ms for SetPlayerHealth, 17.7 k steps =
  250 ms for SetPlayerPos, 28 k = 275 ms for PutPlayerInVehicle, 56–63 k ≈ 1 s for SetPlayerSkin
  with `CStreaming::LoadAllRequestedModels` stepped over). The game freezes for that time only.
  No capture reached the 3 M budget or the 400 k per-call budget.
* Window mode: about 30 µs per guard fault. Hot pages (player ped, CWorld/CTimer globals,
  streaming info) give 70 k–400 k faults per 20-frame window (2.5–14 s, a few fps meanwhile);
  `faults=` (400 k) caps it (`efx_stop` "fault budget", the guards are then left off).
* Stability fixes found on the way (all in `win/efx.rs`):
  * the handler never blocks on the window state (`win_lock`: bounded `try_lock`) and reads
    memory with an uncached VirtualQuery check (`veh_read`) — the tracer's readability cache
    could predate a guard, so the handler faulted on a watched page while holding the lock
    (deadlock of the game, first skin window);
  * guards left by a race between a thread re-arming a page and the window closing are
    recognised (`EVER_PAGES`), dropped, and all ever-watched pages are unguarded once more one
    frame after the close; an access violation on such a page (seen once from the CD-stream
    thread, 0x406570 reading 0x8E4004) restores the page's protection instead of crashing.
  * late `int3` hits (another thread, while the byte is being restored) resume at the same
    address; window call hooks hit while the state is busy run unrecorded.
* Under single-stepping only the traced thread is slowed; other threads (CD stream, audio) run.
  Foreign code (system DLLs, d3d9, the tracer) is never stepped.

## Results: four RPCs, original vs our client (2026-11-04)

Runs: `tools/efx/run.sh both` (orig `target/efx/orig-20261104-230355` + ours `…-230554` for
pos/hp; orig `…-233018` + ours `…-233131` for skin on foot; orig `…-235216` + ours `…-235325`
for putin). Left = original step capture of the samp.dll handler, right = our client's window.

### RPC 14 SetPlayerHealth — identical

* Original (175 steps): the handler decodes the float and writes `player_ped+0x540`
  (`CPed::m_fHealth`) directly: 100 → 57.5. No game call, nothing else written.
* Ours: the same single write, same value (both capture 1 and capture 2 = 33.0).

### RPC 12 SetPlayerPos — same effect

* Original (17.7 k steps): one game call, `CPed::Teleport(player_ped, x, y, z, false)` with the
  RPC's position; inside it `CPedIntelligence::FlushImmediately`, `CWorld::Remove` /
  `CWorld::Add` of the player ped; `player_ped.matrix+0x30..0x38` get the new position.
  Before calling it the handler checks the ped's vtable and model index (branch reads).
* Ours: `CPed::Teleport` called with the same arguments, same nested calls, same final matrix
  position. Differences are pre-existing state, not the RPC: the original's ped has an IK
  look-at chain and extra tasks that `FlushImmediately` releases (`g_ikChainMan` free count
  31 → 32, `player_intel+0x2c/+0x10/+0x74` cleared); our ped has none. The original's
  per-frame code sets that look-at (`IKChainManager_c::LookAt`, called from samp.dll+0xae01e)
  — a per-frame behaviour our client does not have (head tracking of the local player).
  The original also runs a per-frame world scan (samp.dll+0x74cb1 → `CWorld` scan-code
  globals) in the frames after the teleport.

### RPC 153 SetPlayerSkin — not implemented in our client

* Original (56–63 k steps, ≈ 1 s): through script commands `HAS_MODEL_LOADED`, `REQUEST_MODEL`,
  `LOAD_ALL_MODELS_NOW`, `HAS_MODEL_LOADED` (model 29); destroys the ped's IK manager task;
  `CPed::DeleteRwObject`, `CPed::SetModelIndex(29)` (→ `CEntity::SetModelIndex`,
  `RpAnimBlendClumpInit`, decision-maker reset, `CAnimManager::AddAnimation`,
  `UpdateRpHAnim`), then `CAEPedSpeechAudioEntity::Initialise` (voice fields
  `player_ped+0x324…0x348`). Model index `player_ped+0x22` 0 → 29, anim frame pointers
  `+0x48c…0x4d0`, `+0x4d4` (anim group) 54 → 118, `+0x560`, `+0x756`.
* Ours: nothing — `client: … last_rpcs=[…,153]` is received, but samp-client has no handler
  for the local player's skin (only remote players). Window: 0 effects besides background.
* Side observation (orig, putin run): SetPlayerSkin while the local player sits in a car left
  the player dead (health 0, state 9) a few frames later.

### RPC 70 PutPlayerInVehicle — same script sequence, two differences

* Original (28 k steps): validates the car (vtable, `+0x488`, `+0x460`, model 560 via branch
  reads), then script commands `GET_CURRENT_CHAR_WEAPON`, `WARP_CHAR_INTO_CAR`,
  `CAMERA_PERSIST_POS`, `CAMERA_RESET_NEW_SCRIPTABLES`, `SET_CAMERA_BEHIND_PLAYER`,
  `RESTORE_CAMERA_JUMPCUT`. `player_ped+0x58c` (m_pVehicle) = the car, `+0x530` state 1 → 50,
  camera target `TheCamera+0x958` = the car.
* Ours: the same five commands in the same order with the same results (m_pVehicle, state,
  camera target). Differences: ours does not run `GET_CURRENT_CHAR_WEAPON` first; ours changes
  `player_ped+0x55c` (aiming rotation, 4.74 → −1.54) where the original leaves it; from the next
  frame ours runs per-frame vehicle script checks (opcodes 0x02AC, 0x053F, 0x02BF
  `IS_CAR_IN_WATER`) that the step capture of the handler cannot show for the original
  (compare with a window capture of the original if needed).

## Limits

* Step mode follows one call of one function; work the client defers to later frames is not in
  it (use a window). Window mode sees only the watched pages and hooked functions, attributes
  by stack (not by instruction flow), and needs the baseline to drop the game's own frame work
  — writes of the same instruction to the same field through the same client frame as in the
  baseline are hidden.
* `near_veh` (car within 30 m at capture start) was not found in two original runs (begin
  record now has a `world` note to diagnose); the diff renames the car the player ends up in
  (`player_ped+0x58c`) to `near_veh` instead, also when making specs.

## Weapon helpers under the local slot context (moved from a code comment in `combatclient.rs`)

The effect tracer shows, for each of samp's weapon helpers run for the local player (A 0x100AC0A0 / 0x100AF7D0 /
0x100AC050): two writes per weapon-skill stat (save the slot's value in, restore) and two `PlayerInFocus` writes
per helper call. `combatclient::local_context` reproduces that sequence.
