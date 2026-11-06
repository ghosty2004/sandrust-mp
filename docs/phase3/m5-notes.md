# Phase 3 / M5 — vehicles (in game)

Status: `tools/e2e/m5.sh` passes in the "veh" lane (prefix-shot / gta-sa-shot, server 7820, control 47720,
headless gamescope); `m1.sh`, `m2.sh`, `m3.sh` and `goal.sh` still pass there. The scripts now take the lane from
`E2E_TAG` / `E2E_PORT` / `E2E_CTL_PORT` (defaults: main lane) and export `SAMP_RS_CTL_PORT`; m1 connects to
`127.0.0.1:$E2E_PORT` instead of a fixed 7777.

Typical m5 run: car A streams in 1.5 s after the spawn; DriveBot is seated in car B by `WARP_CHAR_INTO_CAR` on its
first 200 (`warps_in=1`); car B's in-game position is 0.3–1.8 m from the server's (≤ 0.5 s old) position and moves
≈ 30 m in 5 s. F → RPC 26 → server state 2 → W for 4 s moves car A ≈ 140 m server-side → F → RPC 154 → state 1.

## Pieces (crates/samp-client)
* `vehicle.rs` (pure, unit-tested): kinds by vtable (A 0x100B6220), model sets, the driver correction
  (A 0x10015140: dead zone 0.05, gain 0.06, 8/8/0.5 or 2.0, min component 0.01, teleport when not in the moving
  list), turn clamp ±0.02, train teleport rule, the 209 validation and apply (0.1 / 6,6,3 / gain 0.06), the shared
  duplicate gate (one timestamp, one copy per packet type, 501 ms), in-car interval (`inCarRate + N`), 200/211/209
  builders, who-sends-209, the 106 cache, engine/lights rules, the `vehicles` line.
* `vehpatches.rs` (table-driven, every row checked against the annotated CSV and `gta_sa.exe` by tests):
  * init-main: rows 156, 168, 169, 171, 188, 190, 194–197, 204, 207, 218–222;
  * rows 148–154: the seven colour-table operands → `COLOUR_TABLE`, samp's 256-entry palette copied from
    samp.dll .data 0x101169C8 (identical in both R4 builds; a test compares it with both DLLs when present);
  * rows 140–147 at InitGame when manual engine/lights is on (147 is applied over 140's NOPs, in order);
  * row 356 at the first frame, after GTA's `CCheat::VehicleSkillsCheat` (0x4399D0) / `WeaponSkillsCheat` (0x439940).
* `vehhooks.rs`:
  * rows 26–35: one `ProcessControl` replacement for the ten vehicle vtables with the three branches of
    remote-control-vehicles §2.3. A (remote player driver, PlayerInFocus 0): slot pad → `Pads[0]`, look-left/right
    from the pad, camera mode and weapon mode (4 → 0), camera-0 vector block from the slot; engine-off heli status /
    throttle zeroing; bike lean saved; audio `Service` as an AI car (ped type 4, status byte 0x1A, then restored);
    PlayerInFocus := slot; original; z / move-speed z restored when > 200 m (2-D) away or without collision
    (not rail), always for boats/planes/helis; bike lean blend; everything restored. B (local driver / nested /
    player without slot): engine-off zeroing on the real pad, `Service`, original. C (no driver / AI): only when
    towed by a seated driver, rail, or near with collision; otherwise skipped.
  * rows 48/49: `jmp [slot]` hooks on the enter-as-driver and leave-car task constructors (handler stored in GTA's
    padding slots 0x6919BB / 0x63B8BA, as samp). Naked stubs pass (return address, task, vehicle) to Rust; only the
    player-input callers (0x570A20/0x570A99, 0x5704A1/0x5703FC) are filtered. The driver gate refuses locked cars,
    model 570, cars without a samp id, AI drivers and remote NPC drivers; a refused task is deleted and control
    continues at 0x570A9E. Accepted enter / exit queue an event; the client tick sends RPC 26 / 154 in the same
    frame.
  * rows 36–40 (siren/alarm/horn status swap), 92–94 (`UsesSiren` || stream-in siren flag), 70–74
    (`BreakTowLink` suppressed unless the local ped drives the cab or samp detaches), 84–91
    (`ProcessEntityCollision` → 0 between two driven vehicles while RPC 167 is set), 47 (+188: train speed clamped
    to ±1.0 before the jump past the derail block), 64 (`FindPlayerSpeed` = zero while a passenger).
* `vehicles.rs`:
  * RPC 164 in the measured order: HAS_MODEL_LOADED (→ REQUEST_MODEL + LOAD_ALL_MODELS_NOW + busy-wait),
    CREATE_CAR, SET_CAR_HEADING, 09C4/07FF/053F (0), door lock := 0, dirt := 0, matrix z + 0.25 (not bikes/BMX),
    colours, health, 0840, damage + tyres, mods (automobiles, 20 ms mod load), paintjob, body colours. Trains:
    the five rail models, CREATE_MISSION_TRAIN, carriages from `m_pNextCarriage`.
  * RPC 165: local ped warped to the car, remote occupants warped to their own position, cabs towing it detached,
    then the destructor (blips, remaining occupants to pos + 2, trailer, DELETE_MISSION_TRAIN / DELETE_CAR).
  * RPCs 24, 26, 57, 65, 70, 71, 91, 96 (types 1–3), 98, 106, 123 (text only), 147, 148, 149, 154, 159, 160,
    161, 167.
  * Packets: 200 stored (timestamp rule shared with 207) and the ped seated by `WARP_CHAR_INTO_CAR` if on foot;
    211 stored + `…_AS_PASSENGER(seat − 1)`; 207 while in a car (special action ≠ 3/4) →
    `WARP_CHAR_FROM_CAR_TO_COORD` to the ped's own position; 209 and 210 applied on receive. After a ped is
    re-created it is seated from the stored driver/passenger sync.
  * Per frame for remote drivers: on a new 200 the velocity write, the correction (speed or teleport), Hydra
    nozzle, bike desired lean, siren bit, landing gear, health, trailer detach; every frame turn clamp + 25 % slerp
    (not trains), NPC zero velocity, keys into the slot pad, horn counter. Passengers: keys only.
  * Local: 200 every `inCarRate + N` ms through the duplicate gate (+ 210 when towing), 106 poll, 211 as passenger,
    the passenger poll on the DPadUp (GROUP_CONTROL_FWD, G) edge → TASK_ENTER_CAR_AS_PASSENGER(−1) + RPC 26(1),
    last-driven tracking for 136. On-foot sync is now sent only in ped states 1 and 12, so nothing is sent during
    the exit animation (ped state 0) or while seated.
  * Pool tick: wasted → local delete after 15001 ms; SET_CAR_PROOFS + SET_CAN_BURST_CAR_TYRES every frame (off /
    burstable for the car the local ped drives); health 0 → 136; IS_CAR_IN_WATER every 4th frame (≤ 200 m, not
    boats); 209 (≤ 3 per tick, > 100 ms per car, closest-player / first-passenger rules); engine/lights; colour
    re-assertion; objective / unoccupied blips (colour 0x3EE / 0x3EC).
* Control channel: `vehicles` (header with ProcessControl branch counters remote/local/none-run/none-skipped and
  send counters, then id, model, handle, CVehicle*, pos, heading, health, driver id / `local`, local occupancy);
  `remote` lines gained `vehicle`, `seat`, `veh_sync`, `veh_corrections`, `veh_teleports`, `warps_in/out`.
* players.rs: a remote ped that is still in a vehicle is warped out to (100, 100, 10) before its destructor
  (onfoot §1.5; missing since M3). Without it a bot that quits while driving left a dangling `m_pDriver` and the
  game died in the next vehicle update (seen once in m5: exit code 0xC0000354 one second after the local enter).
* Shared files touched (for the merge): client.rs (fields, routing of vehicle RPCs/syncs, the state-1/12 rule),
  players.rs (accessors, `status` extra fields), script.rs (opcodes), control.rs + dll.rs (`vehicles`, install),
  lib.rs, tools/e2e/m1–m3.sh (lane variables).

## Tests
* `tools/e2e/probe/m5helper.pwn`: places the first player next to car A on the empty Verdant Meadows sand
  (300, 2500), creates car B 50 m north-east (in view) for "DriveBot" and puts it in, logs `[m5] veh …` twice a
  second, state changes and enter/exit requests. The first version used the LS airport tarmac of
  `tools/trace/vehfs.pwn`; without the SP-OFF rows (main's) one run had car A and the player hit by airport
  traffic, so the scene moved.
* `tools/e2e/m5.sh`: see the header; screenshots `target/m5-{remote,enter,drive,exit}.png`.

## Network check (loopback capture of an m5 run, samp-dump --raw)
* F → RPC 26 at 28.95 s; 207 continues at the on-foot rate during the enter animation; first 200 at 31.89 s
  (TRACE-4: ≈ 3.3 s).
* F → RPC 154 at 41.304 s, one 200 in the same millisecond, then nothing until the next 207 at 43.221 s: a 1.92 s
  silence (TRACE-4: last 200 4 ms after 154, next 207 1.9 s later).
* 200 gaps 31–36 ms while driving (inCarRate 30 + 1 streamed ped, at 60 fps frame granularity; TRACE-4 saw 30 ms),
  one 533 ms gap while standing still with no input (duplicate suppression; TRACE-4 ≈ 530 ms).
* 209 after car A streamed in and settled: 6 sends 115 ms apart in the first version (TRACE-4: 4 sends 150 ms
  apart). Explained by the tracer diff below (pool tick every 4th frame); after the fix the "> 100 ms" rule is
  evaluated every 4th frame as in the original.

## Tracer diff (`tools/trace/run.sh ours vehicles` → target/trace/ours-20261104-214234 vs orig-20261104-204143)
* Same: stream-in opcode sequence (HAS_MODEL_LOADED/REQUEST_MODEL/LOAD_ALL_MODELS_NOW, CREATE_CAR, SET_CAR_HEADING,
  09C4, 07FF, 053F) and the +0.25 z; blip opcodes and arguments (0161(car, 1, 2) + 0165(1004), 0164 on seating);
  WARP_CHAR_INTO_CAR on the first 200 and WARP_CHAR_FROM_CAR_TO_COORD to the ped's own position (task / Teleport /
  Remove/Add sequence identical); the remote-driver ProcessControl context (PlayerInFocus 2, pad LeftStickX −64 and
  Cross 255, camera mode 4, status 0) in every call; rotation written after the world update every frame and speed
  on about every second frame; z restored only while the local player is far away; stream-out deletes.
* Fixed after the diff: the original's vehicle pool tick (proofs/tyres pair, IS_CAR_IN_WATER, 209, blips…) runs on
  every 4th frame only (1123 of 1124 gaps between pair frames are exactly 4); ours ran every frame with
  IS_CAR_IN_WATER every 4th. TRACE-4's "one pair per car per frame" counted frames that had a pair. Consequences now
  reproduced: a new car is first processed on the next pool frame (blip 1–3 frames after CREATE_CAR), the blip goes
  one pool frame after a warp.
* Different but not client behaviour: the order of the RPC 165s / 164s inside one server packet (server streaming
  order) and handles (pool slots differ because our branch still has ambient traffic: no SP-OFF rows).

## TRACE-5 batch 3 (checked, no code change needed)
* Train via RPC 70: WARP_CHAR_INTO_CAR, camera reset, POINT_CAMERA_AT_CAR(car, 3, 2) — as implemented; train `extra`
  = `m_fTrainSpeed` as sent. Local train exit is still unmeasured (our leave hook warps beside the train per the
  static spec).
* 0x0164 once when the local player becomes driver or passenger — matches the blip rule above.
* 203 (aim) in vehicles (camera mode 18, ≈ 1 s) is not sent by this client yet: the 203 sender belongs to M7.

## TRACE-5 batch 2 applied
* 210 is sent on every in-car tick while towing (not only after a sent 200) and refreshes the shared duplicate
  timestamp, so a parked tow sends one 200 and then only 210s, as measured.
* Already matching: skills 1000 + SetStatValue RET; bike `extra` = the vehicle's own lean value; plane gear from
  `m_fLandingGearStatus`; 200 layout. The 15 s local delete of a wreck stays as specified statically (unobservable
  against open.mp, which respawns the car first).

## TRACE-5 batch 1 applied
* No proofs/tyres pair at all for the car the local ped sits in as a passenger (MR-a).
* Blip removed when the local ped is seated as a passenger, too (MR-b; G is not blocked by the door lock — we have no
  lock check there, as samp).
* Enter-gate refusal: no constructor and no stale-vtable call; the raw task memory goes back to the task pool with
  `CTask::operator delete` (0x61A5B0) — decision: same observable effect (no task, no RPC) without samp's call
  through whatever slot 0 the unconstructed memory holds (MR-c).
* RPC 136 after dying in the driven car: "last driven" is now also set when the local driver dies (the sync tick
  does not run while dead), so the next pool frame reports the wreck (MR-d). The RPC 53 before it is M7.

## Deviations / not done (TODO-unknown unless a milestone is named)
* Not applied: row 107 (main table), SP-OFF rows 181/182/189/199 (main), 2/3 (SAA-2), 51 (radar colours: needs
  samp's player palette — so the 0x3EC/0x3EE blips use GTA's colour path for now), 59/60/62/379 (plates / D3D
  proxy, after the M4 merge), 24 (surf glue), 375 (dead in samp), 61 (EU only).
* Enter refusal: the half-built task is completed by GTA's constructor before being deleted (MR-c). The gate's
  cuffed / carry rule is implemented since 2026-11-06 (see "Enter gate, RC driver, 570, mod shop").
* The train exit warp and the train camera run from the client tick, not inside the constructor hook.
* `HasCollisionLoaded` gets the current area (samp passes an uninitialised value, MR-5.12).
* Not reproduced: weapon switch / drive-by / cuffs of remote occupants and the 203 sender while driving (M7),
  "Car Jacked" ejection (method not specified), the one-shot "force siren" global, model
  release on stream-out (MR-5.9), plate textures, spectating a vehicle, the second towable test (A 0x100B7E10), heli
  rotor handling of engine params. (RC-driver world removal, SCM event type 4 and the 570 passenger warp: done
  2026-11-06.)
* RPC 24 doors: `OpenDoor(null ped, node, door, ratio, no sound)` — samp's ped/sound arguments are not documented.
* RPC 165 for the local driver uses WARP_CHAR_FROM_CAR_TO_COORD to the car position (A 0x100AC5B0 not specified).
* Blip "occupied" = a driver ped is set (TRACE-4 local enter/exit is consistent with it; passengers unmeasured).
* The 209 interval follows the spec (> 100 ms); TRACE-4 saw 150 ms gaps (frame-rate dependent?).

## Measurement requests (sent to main, forwarded to TRACE-3 part B)
MR-a passengers (G edge vs RPC 26, task opcode/args, 211 rate, exit packets, 203), MR-b locked cars (F and G),
MR-c enter refusal internals, MR-d 209 ownership / 106 / 136 / 15 s delete, MR-e trailers, trains, bikes,
heli/plane, MR-f per-frame opcodes while a passenger.

## CRASH-F3: crash during the F3 truck/trailer steps (2026-11-06)
Report: run `xclient-20261106-161806`, our client, +251 s, read of 0x38C at gta_sa.exe 0x6D321C.
* **The faulting call**: `CVehicle::ClearVehicleUpgradeFlags` (0x6D3210, `this` = NULL) from
  `AddVehicleUpgrade` (0x6DF930) from the handler of opcode 0x07FF (SET_CAR_HYDRAULICS, 0x4732F7); our side
  `Vehicles::on_rpc` → `stream_in` → `Runner::run` (DLL +0x30473 / +0x50980 in that build). The vehicle handle was
  dead because `busy_load` gave up after 10 s on model 403 / 435, CREATE_CAR (0x00A5) faulted on the unloaded model
  and the next opcodes ran on its leftover handle; the 11th guarded fault of the process is not caught any more (samp's
  filter handles 10), so 0x07FF became the crash. Not a trailer RPC: 148/149 match the original (B 0x1001AE00: both
  slots must hold a vehicle, then the cab's trailer pointer and opcode 0x0893 via B 0x100B7C20; 149 detaches only if
  the cab has one).
* **Why the models never loaded**: 10 s earlier `CGame::Process` had faulted inside the streaming eviction of a
  random traffic model (`CStreaming::RemoveModel` 0x4089A0 → `CClumpModelInfo::DeleteRwObject` 0x4C4E70 →
  `RpClumpDestroy` → `RpGeometryDestroy` → `RpMaterialDestroy` 0x74DA25, material pointer 1 / 0xC47A0000); the half
  evicted model left the streaming state broken (later faults in `RpClumpDestroy` 0x74A336 and 0x73485D).
* **Root cause** (found with temporary per-frame checks of every loaded model's material lists, of every ped's
  animation list and task fields, and a quarantine of freed `CAnimBlendAssociation`s, vtable 0x85C6D0):
  `handsup` → `handsup_off` → `skin` leaves the hands-up task (vtable 0x85A29C) holding the association of the old
  clump, which the skin change frees; at `combat_death_A` (RPC 166 → EXPLODE_CHAR_HEAD) and at the local death GTA
  aborts the task and writes +0x1C / +0x30..+0x38 into that freed block. When the block had been reused for a traffic
  model's material list, evicting that model (when the trucks' models were loaded) faulted. The original keeps the
  same dangling pointer (trace lane, hold session `orig-20261106-175618`: task kept, pointer no longer listed, block
  intact) — its heap simply did not reuse the block in our runs.
* **Fix**: `pedanims.rs` — before the skin change's `DeleteRwObject`, associations that one of the ped's tasks
  (task-manager slots + sub-task chains) points at are unlinked from the clump's list, so they survive with the same
  contents the original's tasks see; the later writes land in live memory (leak: one 0x3C block + node array per
  skin change while an animation task runs). `busy_load` now reports failure and stream-in / PlayCrimeReport create
  nothing for a model that never loads (the original would wait forever) instead of running CREATE_CAR on it.
  Guarded faults log registers and return-address candidates (`swap::last_fault_details`).
* **Reproduction**: `tools/e2e/skin-death-trailers.sh [ours|orig] [ROUNDS]` (trace lane): hands up/off, skin change,
  probe, health 0, trailer attach before the trailer streams in, attach + destroy, model churn. Results: ours 3/3 and
  4/4 rounds PASS (association kept: vtable 0x85C6D0, not listed; 0 guarded faults; `pedanims: kept` per round);
  original 3/3 PASS (game alive) and its probe shows the use-after-free directly: after the skin change the task's
  pointer read vtable 0x1 in round 1 (block already reused) and, in round 2, pointed at an association of the *new*
  clump (the freed block reused for it).
* **The original hits the same failure**: in run `xclient-20261106-183028` (after the fix, 53/66) it was the
  ORIGINAL that broke at `veh_trucks`: its trace (`A-trace.jsonl` of that run) shows it polling
  `HAS_MODEL_LOADED(403)` from B 0x100B84B6 (the vehicle constructor's endless wait) every frame from t = 230.9 s
  until the harness killed it at 313 s (frame 11492 never advanced) — the model never loaded, i.e. the same broken
  streaming state, where ours used to time out and crash. Our client logged no fault in that run. The
  "game still running 8 s after quit" lines of the original in earlier runs (161806, 163843, 165212) fit the same hang.

## Enter gate, RC driver, 570, mod shop (2026-11-06)
Static analysis of samp.dll A (B checked where the trace lane runs it), then the parity scenario
`tools/diff/combatveh.sh` (original in the trace lane, ours in lane ui; `tools/diff/combatveh_eval.py`).
* **RC driver removal** (A 0x10016F0A → ped wrapper vtable slot 2 = A 0x1009F460): every frame of a remote driver whose
  vehicle is an RC model (A 0x100B7E40), a live ped that is in the moving list goes through `CWorld::Remove`. Ours:
  `vehicles.rs` `remove_from_world` after the rotation slerp. Measured (samp-bot driving an RC Bandit): CPhysical +0xB0
  and +0xB4 of the remote ped are 0 in all 6 samples on both clients, ped flags show "in vehicle" on both.
* **570 passenger enter** (A 0x10006FE0 → A 0x100AC450(handle, 1)): the local ped warps with SCM 0x0430 and seat −1.
  Measured (middle carriage of the Unity Station Brown Streak, G): original and ours both PLAYER_STATE_PASSENGER
  0.39 s after G (warp; the animated enter takes ~2–3 s), server seat 1, GTA passenger slot 0 of the 570 carriage.
* **RPC 96 type 4** (A 0x10001A60 case 4, spec vehicles.md 96): implemented as `Vehicles::mod_shop_follow`. Measured
  with `samp-bot --modshop 12,10,1` driving and the local player as passenger (open.mp relays the driver's C→S event to
  the other players): both clients go to area 1 (`CGame::currArea`, ped and vehicle area code 1) and Pad 0 +0x10E =
  0x20 (player controls disabled) while "inside", back to 0 / 0x00 after "left"; OnPlayerInteriorChange 0→1 and 1→0 on
  the server for both.
* **Driver-enter gate: cuffed / carrying, not surfing.** The gate (A 0x100A3700; B 0x100A3757..0x100A3795) calls
  A 0x100ACF30 / 0x100ACE80 on the local **ped object** (local player +0x104), whose +0x2DE / +0x2E2 are the cuffed /
  carry flags of special actions 24 / 25 (set by the local special-action code A 0x1000333A / 0x1000335A, cleared by
  action 0 at A 0x10003193). The earlier spec reading ("surf glue / latch", same offsets on the local *player* object)
  was wrong. Measured on the original (`tools/diff/combatveh.sh`): surfing a player-driven Infernus (server surf vehicle
  5) → F does nothing (GTA itself starts no enter task from the roof; ours logs no gate call either); surfing a
  moving platform object (server surf object 1 in 3 samples) → F **enters** (enter request, 1→2); special action 24 →
  no enter request; 25 → none; 0 → enters. Ours now refuses from `combat.cuffs` (`vehhooks::Shared::local_cuffed /
  local_carrying`, `vehicle::driver_enter_refused`) and logs the refusal; the same six cases match.
