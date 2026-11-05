# Vehicles — RPC layouts and client behaviour (SA:MP 0.3.7-R4)

Clean-room notes. A = samp.dll 2021 build (primary), B = 2019 build. Layouts were taken from
open.mp `Shared/NetCode/vehicle.hpp` (+ `SCMEvent` from `core.hpp`) and then checked against the
bit-read/bit-write sequence of the A handler / send site. All multi-byte fields are little-endian
and read **raw** (no compression, no alignment) unless stated; see handshake.md §6 for the RPC
envelope. "GTA" addresses are gta_sa.exe 1.0 US. "SCM 0xNNNN" means the client runs that GTA
script opcode through its script-command helper (A 0x100B2350); the opcode number is the first
u16 of the descriptor passed to the helper (descriptor address given for verification).

## Overview

The client keeps a vehicle pool of 2000 slots (IDs 0–1999; every handler bounds-checks `id < 2000`
and the pool's "slot used" flag + object pointer before acting, silently ignoring bad IDs; the
reverse lookup GTA-vehicle → ID starts at 1, so ID 0 is effectively unusable). Vehicles exist on
the client only between **StreamInVehicle (164)** and **StreamOutVehicle (165)**; every other
vehicle RPC mutates an already-streamed vehicle. Most S→C vehicle RPCs map directly onto GTA state
(health float, door-lock field, damage-manager bytes, colour bytes, move/turn speed) or onto GTA
script opcodes (create car, z-angle, link to interior, add/remove mod, paintjob, trailer
attach/detach, enter/leave car tasks). The client sends five vehicle RPCs: **26** enter request
(from a hook on GTA's enter-car-as-driver task constructor, or the passenger-key poll),
**154** exit notification (hook on GTA's leave-car task constructor), **106** damage-status change
(polled every frame while driving), **96** SCM event (paintjob / mod / respray / mod-shop
enter-exit, raised by SA-MP's modified main.scm through a hooked script opcode) and **136**
vehicle death (health reached 0 or the car sank, only for the vehicle the local player last
drove). All read/write sequences are identical in B (call-by-call comparison of widths and reliability
arguments for every handler and sender below).

| ID | open.mp name | Dir | A handler / send site | B counterpart | Confidence |
|---|---|---|---|---|---|
| 24 | SetVehicleParams | S→C | 0x100112F0 | 0x100112D0 | high |
| 26 | EnterVehicle | S→C | 0x100115D0 | 0x100115B0 | high |
| 26 | OnPlayerEnterVehicle | C→S | 0x10005AD0 (callers 0x100A3700 hook path, 0x10006FE0) | 0x10005AC0 | high |
| 57 | RemoveVehicleComponent | S→C | 0x1001C5C0 | 0x1001C570 | high |
| 65 | LinkVehicleToInterior | S→C | 0x10019F30 | 0x10019EE0 | high |
| 70 | PutPlayerInVehicle | S→C | 0x10019600 | 0x100195B0 | high |
| 71 | RemovePlayerFromVehicle | S→C | 0x10017FF0 | 0x10017FA0 | high |
| 91 | SetVehicleVelocity | S→C | 0x10018950 | 0x10018900 | high |
| 96 | SCMEvent | S→C | 0x1000F280 → dispatcher 0x10001A60 | 0x1000F250 | high |
| 96 | SCMEvent | C→S | 0x10001CE0 (called from hooked opcode stub 0x10002090) | 0x10001CE0 | high (layout), medium (trigger) |
| 106 | SetVehicleDamageStatus | S→C | 0x100111C0 | 0x100111A0 | high |
| 106 | SetVehicleDamageStatus | C→S | 0x10005DE0 (caller 0x100074C0) | 0x10005DD0 | high |
| 123 | SetVehiclePlate | S→C | 0x1001C230 | 0x1001C1E0 | high (layout), medium (when it takes effect) |
| 136 | VehicleDeath | C→S | 0x1001F230 (caller 0x1001F3C0) | 0x1001F1E0 | high |
| 147 | SetVehicleHealth | S→C | 0x1001AD70 | 0x1001AD20 | high |
| 148 | AttachTrailer | S→C | 0x1001AE50 | 0x1001AE00 | high |
| 149 | DetachTrailer | S→C | 0x1001AF90 (no Ghidra function; read from disassembly) | 0x1001AF40 | high |
| 154 | ExitVehicle | S→C | 0x100116F0 | 0x100116D0 | high |
| 154 | OnPlayerExitVehicle | C→S | 0x10005BF0 (caller: hook stub 0x100A38C0) | 0x10005BE0 | high |
| 159 | SetVehiclePosition | S→C | 0x10019C70 | 0x10019C20 | high |
| 160 | SetVehicleZAngle | S→C | 0x10019D80 | 0x10019D30 | high |
| 164 | StreamInVehicle | S→C | 0x1000E670 (no Ghidra function; read from disassembly) | 0x1000E640 | high |
| 165 | StreamOutVehicle | S→C | 0x10010EE0 | 0x10010EC0 | high |

Out of scope here (vehicle-adjacent, other subsystems): 127 PlayerSpectateVehicle,
167 DisableRemoteVehicleCollisions, sync packets 200/209/210/211.

**No wire-layout disagreement with open.mp was found for any ID above.** Behavioural notes that a
server author or our Rust client must respect are flagged with ⚠ per item.

---

## Shared client state referenced below

SA-MP vehicle object (one per pool slot; field offsets A build, for cross-reference only):

| Offset | Meaning (observed use) |
|---|---|
| +0x44 | GTA script handle of the vehicle |
| +0x48 | trailer: pointer to the SA-MP object of the attached trailer (set by 148, cleared by 149/165) |
| +0x4C | pointer to the GTA `CVehicle` |
| +0x50 | engine param (int; −1 default, 0 off, 1 on) |
| +0x54 | lights param (int; −1 default, 0 off, 1 on) |
| +0x60 | doors-locked flag (mirrors GTA door-lock field) |
| +0x64 | objective flag (byte) |
| +0x71 | "in mod shop" flag (suppresses colour enforcement) |
| +0x79/+0x7A/+0x7B | cached body colours 1/2 + "colours set" flag; re-applied every frame |
| +0x8B | siren flag from stream-in |
| +0x8F/+0x93 | plate texture handle / plate text (32 chars + NUL) |
| +0xB4 | radar blip handle |

Pool (A 0x1001xxxx methods, `this` = netgame → pools → vehicle pool): object pointer array,
slot-used array, GTA-vehicle pointer array, spawn-position array, "wasted" flag + wasted-time arrays.

GTA vehicle-kind classifier (A 0x100B6220, by vtable): 0x871120→1 (automobile), 0x871360→2 (bike),
0x871680→3 (heli), 0x8721A0→4 (boat), 0x871948→5 (plane), 0x871528→6 (BMX), 0x872370→7 (train),
else 0. (Class names are inferred from the usual SA layout; the numeric kinds are what matters.)
Many setters silently do nothing unless kind == 1 (automobile).

Model sets used: trains/trams = {537, 538, 569, 570, 449}; locomotives that spawn carriages =
{537 → carriages 569, 538 → carriages 570}; RC models = {441, 464, 465, 501, 564, 594}.

---

### 164 StreamInVehicle (S→C)

- A 0x1000E670 (not a Ghidra function; read from objdump), B 0x1000E640. Same reads in B.
- Read as two raw byte blocks (40 bytes, then 23 bytes) into a packed struct; field split below
  is from how the struct is consumed. Total 63 bytes.

| Off | Field | Type | Notes |
|---|---|---|---|
| 0 | vehicle_id | u16 | |
| 2 | model | u32 | must be 400..611 inclusive, else whole RPC ignored |
| 6 | pos x, y, z | 3×f32 | |
| 18 | angle | f32 | degrees (z rotation) |
| 22 | colour1 | u8 | 0xFF = keep model default |
| 23 | colour2 | u8 | 0xFF = keep model default |
| 24 | health | f32 | written to GTA vehicle health |
| 28 | interior | u8 | 0 = none |
| 29 | door_damage | u32 | ⚠ doors **before** panels here (opposite of RPC 106) |
| 33 | panel_damage | u32 | |
| 37 | light_damage | u8 | |
| 38 | tyre_damage | u8 | bit layout as in RPC 106 |
| 39 | siren | u8 | non-zero = vehicle has siren |
| 40 | mods[14] | 14×u8 | 0 = empty, else component ID − 999 |
| 54 | paintjob | u8 | 0 = none, else paintjob + 1 |
| 55 | body_colour1 | u32 | 0xFFFFFFFF = unset |
| 59 | body_colour2 | u32 | 0xFFFFFFFF = unset |

Client behaviour (observed):
1. If the pool slot already holds a vehicle, logs "Warning: vehicle %u was not deleted" and
   deletes it first (pool add, A 0x1001F080).
2. Creates the SA-MP vehicle object (A 0x100A02B0 → ctor 0x100B8410). For ordinary models: request
   and synchronously load the model (busy-waits), SCM 0x00A5 create car at (x,y,z), SCM 0x0175
   set z-angle, then SCM 0x09C4, 0x07FF, 0x053F with zero/false args (purposes not identified),
   clears GTA door-lock, re-applies the position (z + 0.25 for non-bike/BMX kinds). For trains
   (537/538/449 and carriages): loads all five train/tram models, then SCM 0x06D8 create train
   (train type 3 for 537, 5 for 538, 9 for 449; direction flag = angle > 180).
   If creation fails: "Warning: couldn't create vehicle type: %u" and the slot stays empty.
3. Siren byte non-zero → object siren flag set (and a global "some vehicle has a siren" flag).
4. colour1/colour2: if either is not 0xFF, write both to the GTA vehicle colour bytes and cache
   them (re-applied every frame by the pool process, see 96).
5. Health → GTA vehicle health float. Spawn position cached in the pool.
6. Interior ≠ 0 → SCM 0x0840 link car to interior.
7. If panels, doors or lights are non-zero → damage setter (same as RPC 106); tyres always applied.
8. Locomotive special case: model 537 or 538 creates **four** vehicles: the loco at `id`, and
   carriages (569 resp. 570) at `id+1`, `id+2`, `id+3` with the same struct. ⚠ The server must keep
   those three IDs reserved for the carriages (open.mp does).
9. Only if the vehicle kind is automobile: each non-zero mod byte → add component (`byte + 999`,
   see 96 type 2), paintjob byte ≠ 0 → apply paintjob `byte − 1` (only 0..3 accepted).
10. If body_colour1 or body_colour2 ≠ 0xFFFFFFFF → set colours (low bytes) as in 96 type 3.

open.mp vs client: layout identical. open.mp sends the remaining non-RPC mod slots (front/rear
bullbars) as SCM events type 2 afterwards, then plate (123), trailer (148), params (24) and
interior (65) RPCs — the client handles each of these as separate RPCs.

### 165 StreamOutVehicle (S→C)

- A 0x10010EE0, B 0x10010EC0.

| Field | Type |
|---|---|
| vehicle_id | u16 |

Client behaviour (observed):
1. If the local ped is inside this vehicle: read the vehicle matrix and teleport the local ped to
   the vehicle's position (A 0x100AC5B0), set a global flag (A 0x1026EA50, purpose unknown).
2. If the local player object's spectate/target vehicle field equals this vehicle → A 0x1000E420
   (Inferred: stop spectating it).
3. Every remote player whose ped is in this vehicle is removed from it (A 0x100148F0).
4. Every other vehicle whose trailer is this vehicle gets its trailer detached and cleared.
5. Delete the vehicle from the pool (A 0x1001EA80). If its model is 537 or 538 also delete
   `id+1..id+3` (carriages).

### 24 SetVehicleParams (S→C)

- A 0x100112F0, B 0x100112D0. Reads u16 then a 16-byte raw block. Ignored if pool absent or ID invalid.

| # | Field | Type | Client action for 0 / 1 (any other value, e.g. 0xFF = "unset", is ignored) |
|---|---|---|---|
| — | vehicle_id | u16 | |
| 0 | engine | u8 | object engine param := 0/1 (applied by the pool process, below) |
| 1 | lights | u8 | object lights param := 0/1 |
| 2 | alarm | u8 | 1: set GTA siren/alarm bit (byte +0x42D bit 7) and alarm timer (+0x45C) := 20000; 0: clear both |
| 3 | doors (locked) | u8 | 1: GTA door-lock (+0x4F8) := 2 (locked), object lock flag := 1; 0: both := 0 |
| 4 | bonnet | u8 | GTA 0x6A6AE0 open/close door #0 (node 0x10), ratio 1.0 / 0.0 |
| 5 | boot | u8 | same, door #1 (node 0x11) |
| 6 | objective | u8 | object objective flag := 0/1 (radar marker, below) |
| 7 | siren | u8 | GTA siren bit (byte +0x42D bit 7) := 0/1 |
| 8 | door_driver | u8 | door #2 (node 10) open 1.0 / closed 0.0 |
| 9 | door_passenger | u8 | door #3 (node 8) |
| 10 | door_backleft | u8 | door #4 (node 11) |
| 11 | door_backright | u8 | door #5 (node 9) |
| 12 | window_driver | u8 | 0: GTA 0x6D3080 (node 10) / 1: GTA 0x6D30B0 (node 10) |
| 13 | window_passenger | u8 | same, node 8 |
| 14 | window_backleft | u8 | same, node 11 |
| 15 | window_backright | u8 | same, node 9 |

Bonnet/boot/door open-close calls only happen for vehicle kind 1 (automobile). Window: 0 =
open (Inferred from SA-MP semantics: first GTA call sets, second clears the window-open flag).

Engine/lights application (pool per-frame process, A 0x1001F3C0, observed): if the server's
"manual engine and lights" setting (InitGame) is off: engine −1 → on iff a driver ped is seated;
0 → off; 1 → on. Lights 0/1 → off/on, −1 → untouched. If manual mode is on: engine on iff param > 0,
lights on iff param > 0. Engine = GTA flag byte +0x428 bit 0x10 (helis get extra rotor handling);
lights = +0x428 bit 0x40 together with +0x4A8 bit 0x08 inverted.

Objective (A 0x100B7720): objective = 1 → remove any normal marker, SCM 0x0161 add blip for car
(args 1, 3), SCM 0x0165 colour 0x3EE, SCM 0x0168 size 3. objective = 0 → remove that blip (SCM 0x0164).
Without objective the same function adds a blip (SCM 0x0161 args 1, 2; colour 0x3EC) to
**unoccupied** vehicles within 200 units and removes it when occupied/far (meaning of colour code
0x3EC in SA-MP's radar hooks: unknown).

open.mp vs client: identical order and widths.

### 70 PutPlayerInVehicle (S→C)

- A 0x10019600, B 0x100195B0.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u16 | |
| seat | u8 | 0 = driver, n>0 = passenger seat n−1 |

Client behaviour (observed, local ped method A 0x100AC2D0): requires the vehicle and its GTA
handle to exist. If holding the parachute (weapon 46) it is removed first. Refused if vehicle health
is 0, or (for automobile/bike kinds) seat > the vehicle's max passengers, or the vehicle's GTA
vtable is 0x863C40. Seat 0: refused if another ped is already seated as driver; else SCM 0x036A
(warp ped into car as driver). Seat n: SCM 0x0430 (warp into passenger seat n−1). Afterwards, if
the local ped is now in a vehicle, the SA-MP camera is reset (A 0x1009D7A0: SCM 0x0930(0), 0x0925,
0x0373 camera behind player, 0x02EB restore camera jumpcut). If the vehicle is a train model, SCM
0x0158 point camera at the car (mode 3, switch 2). No reply RPC is sent (the next sync packet
reflects the new state).

### 71 RemovePlayerFromVehicle (S→C)

- A 0x10017FF0, B 0x10017FA0. **Empty payload** (nothing read).
- Behaviour (A 0x100AC520): if the local ped is in a vehicle that still exists and its model is not
  538 or 570 (streak train loco / carriage), run SCM 0x05CD (task: leave car). Because the
  resulting leave-car task is not created from the player-control code path, the exit hook does
  **not** send RPC 154 for it (Inferred from the return-address filter described under 154 C→S).

### 154 ExitVehicle (S→C)

- A 0x100116F0, B 0x100116D0.

| Field | Type |
|---|---|
| player_id | u16 |
| vehicle_id | u16 |

Behaviour (observed): only if `player_id < 1005`, the remote player exists, has a ped, and the
ped is closer than 200.0 to the local player. Then (A 0x10016230) if the remote ped is in a vehicle:
clear its synced keys and run SCM 0x05CD leave-car task. ⚠ vehicle_id is read but **not used**: the
remote ped leaves whatever vehicle it is in.

### 154 OnPlayerExitVehicle (C→S)

- Sender A 0x10005BF0 (B 0x10005BE0); reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0.

| Field | Type |
|---|---|
| vehicle_id | u16 |

When sent (observed): SA-MP hooks GTA's leave-car task constructor (GTA 0x63B8C0, hook stub A
0x100A38C0, installed at A 0x100A68A6). The stub only reacts when the caller's return address is
GTA 0x5704A1 or 0x5703FC (player-input exit), a one-shot suppress flag (A 0x10151488) is clear, the
vehicle passed is the local ped's current vehicle and maps to a SA-MP ID. Then:
- train model: no RPC; instead the ped is placed beside the train (A 0x100AC5B0) and the task is
  told to proceed differently (flag A 0x10113AC0 = 1).
- otherwise A 0x10005BF0: if the local ped is the driver (not passenger; train carriages count as
  passenger) remember this ID as "last driven vehicle" (local player +0xFA, used by RPC 136); if the
  vehicle is a train reset the camera; if the model is **not** an RC model send RPC 154.

### 26 EnterVehicle (S→C)

- A 0x100115D0, B 0x100115B0.

| Field | Type | Notes |
|---|---|---|
| player_id | u16 | |
| vehicle_id | u16 | |
| passenger | u8 | non-zero = enters as passenger |

Behaviour (observed): same remote-player checks as 154 S→C (`< 1005`, has ped, distance < 200).
Then (A 0x100161A0) if the vehicle exists and the remote ped is not already in a vehicle: clear its
synced keys and start the walk-in task — SCM 0x05CB enter as driver (3000 ms) or SCM 0x05CA enter
as passenger (any seat, 3000 ms). (Same helper used for the local ped; for the local ped entering
model 570 as passenger it warps with SCM 0x0430 instead.)

### 26 OnPlayerEnterVehicle (C→S)

- Sender A 0x10005AD0 (B 0x10005AC0); reliability 10, priority 1, channel 0.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u16 | |
| passenger | u8 | 0 = driver, 1 = passenger (bool) |

When sent (observed):
- **Driver**: SA-MP hooks GTA's enter-car-as-driver task constructor (GTA 0x6402F0 → stub A
  0x100A3860). When the caller is the player-input code (return address GTA 0x570A20 or 0x570A99)
  the gate A 0x100A3700 decides; if it returns 0 the task creation is skipped (ped does not enter).
  Gate refuses (no RPC) when: the vehicle has no SA-MP ID; the SA-MP doors-locked flag is set; model
  is 570; two local-player state checks (A 0x100ACF30 / 0x100ACE80, meaning unknown) are true; the
  vehicle's current driver has a non-zero field +0x598, or is a SA-MP remote player for whom
  A 0x1000E470 is true (unknown). Otherwise: drop parachute if held, send RPC 26 (passenger = 0),
  and allow the task.
- **Passenger**: polled every frame while on foot (local player process A 0x100074C0 → A 0x10006FE0):
  on the passenger-enter key edge (key-state +0x10 set and +0x40 clear; GUESS: the G key) find the
  closest streamed vehicle; if closer than 8.0, drop parachute, start SCM 0x05CA enter-as-passenger
  and send RPC 26 (passenger = 1).
- After any send: if the vehicle is a train, SCM 0x0158 camera on car (3, 2) and a 5-second timer
  starts; if after 5 s the ped is still on foot the camera is reset.

### 106 SetVehicleDamageStatus (S→C)

- A 0x100111C0, B 0x100111A0.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u16 | |
| panels | u32 | ⚠ panels first here (doors first in 164) |
| doors | u32 | |
| lights | u8 | |
| tyres | u8 | see bit layout below |

Behaviour (observed, A 0x100B7B00 + 0x100B7980):
- Panels/doors/lights only for kinds automobile and plane: if all three are 0 while the vehicle
  currently has damage → GTA 0x6A3440 (Inferred: CAutomobile fix). Otherwise write panels to
  damage-manager +0x5B4 (u32), doors to +0x5A9 (u32 covering the first four door-status bytes),
  lights to +0x5B0 (zero-extended), then GTA 0x6B3E90 (Inferred: rebuild damage model).
- Tyres, automobile: bit0 → wheel byte +0x5A8, bit1 → +0x5A7, bit2 → +0x5A6, bit3 → +0x5A5
  (each byte := 1 if bit set else 0). Bike: bit0 → +0x65D (rear), bit1 → +0x65C (front).
  Other kinds: ignored.

### 106 SetVehicleDamageStatus (C→S)

- Sender A 0x10005DE0 (B 0x10005DD0); reliability 8 (RELIABLE), priority 1, channel 0.
- Same field order and widths as S→C: u16 vehicle_id, u32 panels, u32 doors, u8 lights, u8 tyres.

When sent (observed): called every frame from the local player process (A 0x100074C0) while the
ped state is "driving" (GTA ped state 50) and the local ped is the driver. The sender keeps a cache
{vehicle ID, panels, doors, lights, tyres} in the local player object (+0x318…+0x323):
```
if id != cached_id: refresh cache from the vehicle; cached_id = id   // no send on vehicle change
else if any of the four values differs from the cache: refresh cache; send RPC 106
```
Values come from the getters: panels = u32 at +0x5B4, doors = u32 at +0x5A9, lights = byte at
+0x5B0 (automobile only, else 0); tyres = automobile: bit3 +0x5A5, bit2 +0x5A6, bit1 +0x5A7,
bit0 +0x5A8 (each bit = byte non-zero); bike: bit1 front, bit0 rear; others 0.

### 57 RemoveVehicleComponent (S→C)

- A 0x1001C5C0, B 0x1001C570.

| Field | Type |
|---|---|
| vehicle_id | u16 |
| component | u16 (component model ID 1000..1193) |

Behaviour: if the GTA vehicle exists → SCM 0x06E8 remove vehicle mod (descriptor A 0x100ED678).
⚠ open.mp also sends this as the "veto" reply when a client's mod SCM event (96 type 2) is rejected.

### 65 LinkVehicleToInterior (S→C)

- A 0x10019F30, B 0x10019EE0.

| Field | Type |
|---|---|
| vehicle_id | u16 |
| interior | u8 |

Behaviour: if slot used and GTA handle valid → SCM 0x0840 link car to interior. The pool's stored
interior is not changed.

### 91 SetVehicleVelocity (S→C)

- A 0x10018950, B 0x10018900. **No vehicle ID**: applies to the vehicle the local ped is in.

| Field | Type | Notes |
|---|---|---|
| type | u8 | 0 = linear, 1 = angular, other = ignored |
| x, y, z | 3×f32 | GTA units |

Behaviour: only if the local ped is in a vehicle that maps to a valid SA-MP ID. Type 0 writes
GTA move speed (CPhysical +0x44..+0x4C), type 1 writes turn speed (+0x50..+0x58) directly.

### 96 SCMEvent (S→C)

- A 0x1000F280 → dispatcher A 0x10001A60; B 0x1000F250.

| Field | Type | Notes |
|---|---|---|
| player_id | u16 | origin player (INVALID for server-initiated) |
| event_type | u32 | 1 paintjob, 2 add component, 3 colour, 4 mod-shop enter/exit |
| vehicle_id | u32 | |
| arg1 | u32 | |
| arg2 | u32 | |

Behaviour (observed): whole RPC ignored until the local player has spawned once (global A
0x100FE0A0 starts at 1 and is cleared in the spawn sender A 0x10003C20), or when player/vehicle
pools are absent. Then by type:
1. vehicle exists → paintjob `arg1` via SCM 0x06ED (only automobile kind and arg1 < 4).
2. vehicle exists → add component `arg1` (automobile only): if the component model is not loaded,
   request it and wait up to ~20 ms (SCM 0x06EA test / 0x06E9 request, `Sleep(1)` loop); if still not
   loaded the mod is **silently dropped**; else SCM 0x06E7 add vehicle mod.
3. vehicle exists → body colours (arg1, arg2): written to GTA colour bytes +0x434/+0x435 and
   cached in the object; the vehicle pool process re-applies the cached colours every frame unless
   the "in mod shop" flag is set.
4. remote player `player_id` (< 1005) is driving the vehicle the local ped is a passenger in, both in
   the same vehicle: arg1 ≠ 0 (entering) → link car to interior arg2 (only if the remote has a vehicle
   object) and set the local ped's interior to arg2, then A 0x100AC7D0(0); arg1 = 0 (leaving) →
   interior 0 for both, A 0x100AC7D0(1). (Inferred: passengers follow the driver into the mod shop.)

### 96 SCMEvent (C→S)

- Sender A 0x10001CE0 (B identical address 0x10001CE0); reliability 10, priority 1, channel 0.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u32 | SA-MP ID widened to 32 bits |
| arg1 | u32 | |
| arg2 | u32 | |
| event_type | u32 | 1..4 as above; other values send nothing |

⚠ Order differs from the S→C form (vehicle first, type last, no player ID) — matches open.mp's
separate read()/write().

When sent (observed): SA-MP patches two entries of a GTA script-opcode jump table
(GTA 0x47BF54 → stub A 0x10002090, GTA 0x47BF88 → stub A 0x10001C30; installed at A 0x10002100).
Stub 0x10002090 collects four script parameters (GTA 0x464080) and calls the sender with
(vehicle handle = param1, arg1 = param2, arg2 = param3, type = param0). Thus the events are raised by
SA-MP's modified game script in mod/paint shops. The handle is mapped to a SA-MP ID; if none, nothing
is sent. Local side effects: type 3 also applies the colours locally; type 4 sets/clears the
"in mod shop" flag (on exit it re-caches the current GTA colours so they persist).
The other stub (0x10001C30) answers a script condition: false while the local ped drives a vehicle that is not a train carriage (569/570) or has the in-vehicle flag without a vehicle, true otherwise (B 0x100ABE80 / 0x100ABFD0; verified by the mod-shop garage, feata-notes.md)
(sets the script compare flag via GTA 0x4859D0); no network traffic.

### 123 SetVehiclePlate (S→C)

- A 0x1001C230, B 0x1001C1E0.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u16 | |
| len | u8 | must be ≤ 32, else ignored |
| text | len bytes | not NUL-terminated on the wire |

Behaviour (observed): text copied (max 32 chars) into the vehicle object's plate buffer. The plate
texture is created lazily once (A 0x100B81E0, only when no plate texture exists yet and the text
is non-empty). Inferred: a plate change after the texture already exists has no visible effect
until the vehicle is re-streamed, so the RPC is effectively meaningful right after 164.

### 136 VehicleDeath (C→S)

- Sender A 0x1001F230 (B 0x1001F1E0); reliability 10, priority 1, channel 0.

| Field | Type |
|---|---|
| vehicle_id | u16 |

When sent (observed, vehicle pool process A 0x1001F3C0, every frame for every streamed vehicle):
- if the vehicle's health is 0.0 → call sender, and mark the slot "wasted" with a timestamp;
- else if the vehicle is not a boat, is within 200 units, and GTA reports it sunk
  (SCM 0x02BF) → call sender;
- the sender only transmits if `vehicle_id` equals the local player's "last driven vehicle"
  (+0xFA; set when leaving the driver seat, when going on foot after driving, or by the exit RPC),
  and then resets that field to 0xFFFF — so each death is reported at most once and only by the
  last driver.
- 15 s after a vehicle was marked wasted the client deletes it from its own pool (no RPC).

### 147 SetVehicleHealth (S→C)

- A 0x1001AD70, B 0x1001AD20.

| Field | Type |
|---|---|
| vehicle_id | u16 |
| health | f32 |

Behaviour: writes the GTA vehicle health float (+0x4C0) if the GTA vehicle exists. No clamping.

### 148 AttachTrailer (S→C)

- A 0x1001AE50, B 0x1001AE00.

| Field | Type | Notes |
|---|---|---|
| trailer_id | u16 | ⚠ trailer first |
| vehicle_id | u16 | cab / tractor |

Behaviour: both must exist. The cab object's trailer pointer := trailer object; SCM 0x0893 attach
trailer (trailer handle) to cab (cab handle).

### 149 DetachTrailer (S→C)

- A 0x1001AF90, B 0x1001AF40 (not Ghidra functions; read from disassembly). Same reads.

| Field | Type | Notes |
|---|---|---|
| vehicle_id | u16 | the **cab** (open.mp sends the cab's pool ID) |

Behaviour: if the cab has a trailer whose GTA vehicle exists → SCM 0x07AC detach trailer from cab
(guarded by a global "detaching" flag while the opcode runs), then cab trailer pointer := null.

### 159 SetVehiclePosition (S→C)

- A 0x10019C70, B 0x10019C20.

| Field | Type |
|---|---|
| vehicle_id | u16 |
| x, y, z | 3×f32 |

Behaviour (A 0x1009F0A0): for models 537, 538, 449 → SCM 0x07C7 (set train position); otherwise
the GTA entity's virtual teleport (vtable slot +0x38, args x, y, z, false). Skipped if the GTA
vtable is 0x863C40.

### 160 SetVehicleZAngle (S→C)

- A 0x10019D80, B 0x10019D30.

| Field | Type |
|---|---|
| vehicle_id | u16 |
| angle | f32 (degrees) |

Behaviour: if the GTA handle is valid → SCM 0x0175 set car z-angle.

---

## Summary of open.mp vs client

No layout disagreements. Points worth keeping in mind (all consistent with open.mp, but easy to
get wrong in a reimplementation):
- Door/panel order: 164 = doors, panels; 106 (both directions) = panels, doors.
- 96 S→C = {u16 player, u32 type, u32 vehicle, u32 arg1, u32 arg2}; 96 C→S = {u32 vehicle, u32 arg1,
  u32 arg2, u32 type}.
- 148 = trailer first, cab second; 149 carries the cab ID.
- 154 S→C carries a vehicle ID the client ignores.
- 164 paintjob is +1-encoded (0 = none); 96 type 1 carries the raw paintjob (0..3).
- 164 mods are `component − 999` in a byte; 57 / 96 carry the full component ID.
- Locomotive 537/538 stream-in silently occupies IDs id+1..id+3.
- The client sends all vehicle RPCs on ordering channel 0 (open.mp's channel constant only matters for
  its own sends).

---

## RE-10 additions (static, 2026-11-04)

Static analysis of A (objdump + decompiled dump) for milestone M5. Tags as elsewhere: **Observed** = read from
samp.dll A / gta_sa.exe 1.0 US code; **Inferred** = reasoning beyond the code. GTA names via gta-reversed / plugin-sdk.

### Passenger-enter key (resolves the "GUESS: the G key" in 26 C→S) — Observed
* The pad pointer used by the poll A 0x10006FE0 is the global A 0x10114AE8, whose initial value is 0xB73458
  (`CPad::Pads[0]`). Offsets +0x10 / +0x40 are `NewState.DPadUp` / `OldState.DPadUp` (`CControllerState` layout).
  The poll fires on the press edge (new ≠ 0, old = 0) of DPadUp.
* On foot, GTA maps the action `GROUP_CONTROL_FWD` to DPadUp (`CControllerConfigManager`, default keyboard key **G**).
  So the passenger request is "group-control-forward pressed", whatever key the user bound to it.
* Order inside the poll: closest streamed vehicle (A 0x1001EC00, must be < 2000 and used) → distance < 8.0 → if the
  current weapon is 46 (parachute) the armed weapon becomes 0 → TASK_ENTER_CAR_AS_PASSENGER via the ped helper
  A 0x100AC450(handle, 1) → RPC 26 (passenger = 1). No lock check (see the measurement request about GTA's own lock).

### Number plates (refines 123 SetVehiclePlate "when it takes effect") — Observed
* Each vehicle's plate is a samp-owned **Direct3D texture**, not a GTA plate raster. A 0x1006D880 creates a 64×32
  texture, clears it to ARGB 0xFFBEB6A8 and draws the plate text into it; the handle goes to the vehicle object +0x8F.
* Creation is lazy and happens in the CGame::Process pre-step (see `remote-control-vehicles.md`, RE-10 additions):
  pool pass A 0x1001ED70 runs only while the pool flag +0x17894 is set (set by the pool constructor, cleared on D3D
  device loss and set again after reset). It first creates the default plate texture with the text `XYZSR998` (stored
  at +0x20 of the object at A 0x1026EA18) and then, for every used slot, A 0x100B81E0: create the texture only if the
  vehicle has none yet and its plate text is non-empty.
* Consequence: RPC 123 that arrives before the next frame's pre-step (i.e. in the same network tick as 164, which is
  what open.mp does) is shown; a later 123 changes only the stored text, never the texture (until the vehicle is
  re-streamed). Vehicles with an empty plate text show `XYZSR998`.
* How the texture reaches the screen: GTA's `CCustomCarPlateMgr::CreatePlateTexture` asks `RwRasterCreate` for a plate
  raster; samp's hook (A 0x100A4560, CSV row 59) returns **one shared raster** (64×16, depth 32, flags 0x604, created
  once, cached at A 0x10151540) for every car. The `RwRasterDestroy` hook (A 0x100A4470, row 60) refuses to destroy that
  shared raster while A 0x10113AD8 is set. The `SetDirtTextures` call hook (row 62, A 0x100A45A0) records the vehicle
  being rendered (A 0x10151550). samp's Direct3D device proxy (row 379) intercepts `SetTexture` (vtable +0x104,
  A 0x10074BB0): when the texture being bound is the shared plate raster's D3D texture, it binds the current vehicle's
  own plate texture (or the default one) instead and sets the RW texture filter to linear (render state 9 := 2).
  Inferred: a reimplementation can do the same substitution at RenderWare level, but the observable rule is "one plate
  texture per vehicle, text fixed at creation".

### Vehicle radar blips (resolves "meaning of colour code 0x3EC") — Observed
* samp replaces `CRadar::GetRadarTraceColour` (row 51, A 0x100A3A80 → 0x100A7270). Colour index 0x3EC → RGBA
  0x89A4C440 (unoccupied-vehicle blip), 0x3ED → 0xAA0000FF (checkpoint), 0x3EE → 0xE2C063FF (objective vehicle).
  Other indices < 4000 are looked up in a 1000-entry table (A 0x101516A0, initialised from samp's default player palette
  A 0x10113B48 and overwritten by every player-colour setter), so index *n* means "player *n*'s colour". Values ≥ 4000
  are used as RGBA directly. Indices 1000..3999 read past the table (samp bug; never produced by the client itself).

### Radar zoom while a passenger (CSV row 64) — Observed
* The `FindPlayerSpeed` call in `CRadar::DrawMap` goes to A 0x100A4A50. When the local ped is a passenger (in a vehicle and
  not its driver, or in carriage model 569/570; A 0x100AC000) it returns a zero vector (A 0x10151588, never written), so
  the radar does not zoom out with speed. Otherwise the original runs.

### Driving skills (applies to every local vehicle) — Observed
* At the first frame (A 0x100C47D0 → A 0x100A0C40, CSV row 356) samp runs GTA's own `CCheat::VehicleSkillsCheat`
  (0x4399D0: driving, flying, bike and cycling skill stats := 1000) and `CCheat::WeaponSkillsCheat` (0x439940), then turns
  `CStats::SetStatValue` into RET. The local player therefore drives with maximum skills, and GTA can never change any
  stat afterwards. A client that skips this has different handling (e.g. bike fall-off, plane control).

### GTA function names for RPC 24 / 106 (resolves "Inferred") — Observed (gta-reversed)
0x6A6AE0 `CAutomobile::OpenDoor`, 0x6D3080 `CVehicle::SetWindowOpenFlag`, 0x6D30B0 `CVehicle::ClearWindowOpenFlag`,
0x6A3440 `CAutomobile::Fix`, 0x6B3E90 `CAutomobile::SetupDamageAfterLoad`. So in RPC 24 a window byte of 0 **opens** the
window (sets the open flag) and 1 closes it; RPC 106 with all-zero panels/doors/lights repairs the car with `Fix`, and any
other value is applied by writing the damage-manager fields and calling `SetupDamageAfterLoad` (rebuilds the visible
damage state from those fields).
