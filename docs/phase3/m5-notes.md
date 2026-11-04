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
* Enter refusal: the half-built task is completed by GTA's constructor before being deleted (MR-c). Local surfing
  is not part of the gate (no local surf state yet).
* The train exit warp and the train camera run from the client tick, not inside the constructor hook.
* `HasCollisionLoaded` gets the current area (samp passes an uninitialised value, MR-5.12).
* Not reproduced: weapon switch / drive-by / cuffs of remote occupants and the 203 sender while driving (M7),
  "Car Jacked" ejection (method not specified), RC-driver world removal, the one-shot "force siren" global, model
  release on stream-out (MR-5.9), SCM event type 4 (mod shop), plate textures, spectating a vehicle, the model-570
  passenger warp seat, the second towable test (A 0x100B7E10), heli rotor handling of engine params.
* RPC 24 doors: `OpenDoor(null ped, node, door, ratio, no sound)` — samp's ped/sound arguments are not documented.
* RPC 165 for the local driver uses WARP_CHAR_FROM_CAR_TO_COORD to the car position (A 0x100AC5B0 not specified).
* Blip "occupied" = a driver ped is set (TRACE-4 local enter/exit is consistent with it; passengers unmeasured).
* The 209 interval follows the spec (> 100 ms); TRACE-4 saw 150 ms gaps (frame-rate dependent?).

## Measurement requests (sent to main, forwarded to TRACE-3 part B)
MR-a passengers (G edge vs RPC 26, task opcode/args, 211 rate, exit packets, 203), MR-b locked cars (F and G),
MR-c enter refusal internals, MR-d 209 ownership / 106 / 136 / 15 s delete, MR-e trailers, trains, bikes,
heli/plane, MR-f per-frame opcodes while a passenger.
