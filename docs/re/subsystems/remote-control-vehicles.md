# Vehicles inside GTA: lifecycle, remote drivers and passengers, unoccupied cars, enter/exit (SA:MP 0.3.7-R4, build A)

Clean-room notes. They cover how samp.dll turns server vehicles into GTA `CVehicle`s and how it drives them for remote and
local players. Wire layouts are not repeated here. See `vehicles.md` (RPCs 24…165), `remote-players-sync.md` §2 (packets
200/209/210/211) and §2.1 (stored sync, per-frame apply), and `gta-patches.md` (patch list). This file adds the GTA-side
mechanics: the vehicle `ProcessControl` hook, the per-player context swap, the GTA calls each step makes, the frame order,
and what a Rust DLL must reproduce.

A = samp.dll 0.3.7-R4 2021 build (base 0x10000000). GTA = gta_sa.exe 1.0 US. GTA names come from `re/gta-reversed` /
`re/plugin-sdk` ("named via …"). SCM opcode names come from `gta-reversed/source/game_sa/Enums/eScriptCommands.h`, and
each opcode number was read from the first u16 of the descriptor that samp passes to its script-command helper
A 0x100B2350. Tags: **Observed** = read from disassembly/decompilation; **Inferred** = reasoning beyond that;
**Unknown** = open.

---------------------------------------------------------------------------------------------------

## 0. Mechanism in a nutshell

* A server vehicle is a GTA mission car created through **script opcodes** (CREATE_CAR / CREATE_MISSION_TRAIN). samp keeps a
  2000-slot pool `samp id → samp vehicle object → CVehicle*` and does a linear reverse scan for `CVehicle* → id`.
* A remote player's ped is a real `CPlayerPed` (ped type PLAYER1). It is **warped** into the seat (WARP_CHAR_INTO_CAR /
  …_AS_PASSENGER) whenever driver/passenger sync arrives and the ped is on foot. The enter/leave **animations** are played
  only when the server relays RPC 26/154.
* GTA physics **does simulate** remote-driven vehicles, with the remote player's keys as input. samp redirects
  `ProcessControl` of all ten vehicle classes to one hook (A 0x100A3140). When the driver is a remote `CPlayerPed`, the hook
  swaps the remote player's pad, camera mode, look-left/right flags, aim vectors and `CWorld::PlayerInFocus` into the
  globals the game reads, calls the class's original `ProcessControl`, and then restores everything.
* The network data only **corrects** that simulation. Once per received packet (applied in samp's per-frame tick): the
  synced velocity is written, a proportional pull toward the synced position is added (factor 0.06), or the car is
  teleported past a threshold. Every frame: the rotation is slerped 25 % toward the synced quaternion and the turn speed is
  clamped to ±0.02.
* Damage is owned by the driver. Every vehicle the local player is **not** driving gets all damage proofs on and
  unburstable tyres, re-asserted every frame. Remote vehicles' health comes from sync.
* Unoccupied cars are simulated only near the local player (or while towed, or trains). The closest player (or the first
  passenger) reports them with packet 209. Receivers apply 209/210 immediately in the packet handler, with the same
  "nudge or teleport" logic.

---------------------------------------------------------------------------------------------------

## 1. Vehicle lifecycle

### 1.1 Pool and id mapping (Observed)

Vehicle pool object (reached as netgame +0x3DE → pools → [0]); byte offsets in A:

| offset | per-id array (2000 entries) | meaning |
|---|---|---|
| +0x0000 | — | highest used id (loop bound) |
| +0x1134 | ptr ×2000 | samp vehicle object (0xB8 bytes, vtable A 0x100ED728) |
| +0x3074 | u32 ×2000 | slot-used flag |
| +0x4FB4 | ptr ×2000 | cached `CVehicle*`; refreshed every frame by the pool tick from object +0x4C |
| +0x8E34 | u16 ×2000 | last remote player whose 209 was applied to this car |
| +0xBD14 | u32 ×2000 | set to 1 at stream-in (gate used by RPC 161) |
| +0xDC54 / +0xFB94 | u32 ×2000 | "wasted" flag / time it was set |
| +0x11AD4 (+0xC·id) | 3×f32 | spawn position from 164 |

* id → object: bounds `< 2000`, then slot-used, then the object pointer. id → GTA script handle: object +0x44
  (A 0x1001EBC0 returns 0xFFFF if no CVehicle). Handle → `CVehicle*`: GTA `CPool::GetAt` 0x4048E0 on the vehicle pool
  (A 0x100B3C10). `CVehicle*` → handle: GTA 0x424160 (`CPool::GetRef`, named via gta-reversed Pool.h) (A 0x100B3C40).
* `CVehicle*` → samp id (A 0x1001EB90): **linear scan** of the +0x4FB4 array for ids 1..maxId. It returns 0xFFFF when
  nothing matches. Id 0 is never found.
* samp vehicle object fields used below: +0x40/+0x4C `CVehicle*` (the generic entity wrapper uses +0x40), +0x44 script
  handle, +0x48 attached-trailer object, +0x58 "proofs on" flag, +0x60 locked, +0x71 in-mod-shop, +0x79/+0x7A/+0x7B cached
  colours, +0x87 per-model flag passed at creation (meaning Unknown), +0x8B siren-from-stream-in, +0x8F plate texture
  object, +0xB4 radar blip.
* GTA pool capacity: the `Vehicles` pool is raised from 110 to **710** (gta-patches). samp allows 2000 ids but only about
  700 can exist in GTA at the same time. The server's streamer keeps the count below that.

### 1.2 Stream-in (RPC 164 → pool add A 0x1001F080 → ctor A 0x100B8410) (Observed)

1. If the slot is occupied, the old vehicle is deleted first ("Warning: vehicle %u was not deleted").
2. Ordinary models: if HAS_MODEL_LOADED (0x0248) is false, it issues REQUEST_MODEL (0x0247) and LOAD_ALL_MODELS_NOW (0x038B),
   then **busy-waits** with `Sleep(0)` until 0x0248 reports the model loaded. This blocks the game thread.
3. CREATE_CAR (0x00A5) at x,y,z, then SET_CAR_HEADING (0x0175, angle). Then three opcodes with false/0 arguments:
   SET_PETROL_TANK_WEAKPOINT (0x09C4), SET_CAR_HYDRAULICS (0x07FF), SET_CAN_BURST_CAR_TYRES (0x053F). (Names via
   gta-reversed. This resolves the "purposes not identified" note in vehicles.md.)
4. Direct writes to the CVehicle: `m_eDoorLock` (+0x4F8) := 0 and `m_fDirtLevel` (+0x4B0) := 0. The position is written
   again into the matrix, with z + 0.25 except for bikes/BMX (kind 2/6).
5. Trains: model 538 → train type 5, 537 → 3, 449 → 9. It loads all five rail models (537, 538, 569, 570, 449) and then
   runs CREATE_MISSION_TRAIN (0x06D8, direction = angle > 180). GTA creates the carriages itself. For the carriage
   ids id+1..id+3 (models 569/570), samp creates **no** new GTA vehicle. Each carriage object takes `CTrain::m_pNextCarriage`
   (+0x5D4) of the previously created train vehicle (a global cursor, A 0x1026DE44) and gets its handle through
   `CPool::GetRef`. Tram 449 has no carriages.
6. Back in pool add: colours (if either of colour1/colour2 is not 0xFF) are written to `m_nPrimaryColor`/`m_nSecondaryColor`
   (+0x434/+0x435) and cached; `m_fHealth` (+0x4C0) := health; slot-used := 1; spawn pos cached; interior ≠ 0 →
   SET_VEHICLE_AREA_VISIBLE (0x0840); damage panels/doors/lights (A 0x100B7B00) and tyres (A 0x100B7980) as in RPC 106;
   +0xBD14 := 1; wasted := 0.
7. The locomotive's carriages, mods (ADD_VEHICLE_MOD 0x06E7, automobiles only), paintjob (GIVE_VEHICLE_PAINTJOB 0x06ED),
   body colours, plate (123), trailer (148), params (24) and interior (65) are then handled as described in vehicles.md.
   The plate texture is created lazily (the RwRasterCreate call in `CCustomCarPlateMgr::CreatePlateTexture` is hooked).
8. **Virtual world** has no client-side representation. The server streams vehicles in or out per world. **Interior** is only
   GTA's area code (0x0840); the pool's stored interior is not updated by RPC 65.

### 1.3 Stream-out / delete (RPC 165 → A 0x1001EA80 → object dtor A 0x100B8930) (Observed)

RPC 165 first ejects the local ped (warp to the car's position), remote peds (§2.1) and any cab that tows this vehicle
(vehicles.md). The destructor then does the following:
1. Removes the radar blip (REMOVE_BLIP 0x0164).
2. Warps every occupant out: driver and each passenger slot get WARP_CHAR_FROM_CAR_TO_COORD (0x0362) to the car position
   + 2.0 on z (A 0x100B7F20).
3. Detaches its trailer if it has one (DETACH_TRAILER_FROM_CAB 0x07AC, under the "samp is detaching" flag) and destroys the
   plate texture object.
4. Models 537/538 → DELETE_MISSION_TRAIN (0x07BD), which removes the carriages too. Otherwise DELETE_CAR (0x00A6).
5. Model release: if the model's `CBaseModelInfo` ref count (+8) is 0, object flag +0x87 is clear, and more than 80 of the
   `CStreaming::ms_aInfoForModel` entries for models 400–610 (0x8E6C10.., stride 20, named via gta-reversed) have a non-zero
   first dword, it calls `CStreaming::RemoveModel` (0x4089A0). (The meaning of that count is Unknown.)

### 1.4 Per-frame pool tick (A 0x1001F3C0, from the netgame tick) (Observed)

For every used slot with a samp object:
* **Wasted**: if the wasted flag is set and 15001 ms have passed, the vehicle is deleted locally (no RPC).
* **Ownership of damage**: A 0x100B71D0(driver ≠ local ped). When the local player is *not* the driver: SET_CAR_PROOFS
  (0x02AC) with all five proofs on, and SET_CAN_BURST_CAR_TYRES (0x053F) false. When the local player drives: all proofs
  off and tyres burstable. This happens **every frame** for every vehicle.
* **Death**: health == 0.0 → send RPC 136 (only the last driver actually sends it) and set wasted. Otherwise, if the car is
  not a boat, is within 200 units and IS_CAR_IN_WATER (0x02BF) is true → RPC 136. **Correction**: A 0x100ED574, which
  `remote-players-sync.md` calls a "model set" gating unoccupied sync, is the descriptor of opcode 0x02BF. The gate is the
  "sunk → report death" branch, and unoccupied sync runs only on the not-sunk branch.
* Not sunk: unoccupied 209 sending (§3.2), engine/lights (vehicles.md), the `CVehicle*` cache refresh, colour
  re-assertion (A 0x100B7370), and objective/unoccupied blips (A 0x100B7720).

---------------------------------------------------------------------------------------------------

## 2. Remote driver

### 2.1 Getting into and out of the seat (Observed)

* **Warp on sync**: the driver store (A 0x10017340, timestamp-ordered) and the passenger store (A 0x10017440) set the
  remote's vehicle id/object. If the ped is not in any vehicle (`CPed` flag word +0x46C bit 8 = `bInVehicle`), they call
  A 0x10014800. That function switches the weapon, then A 0x100AC2D0: seat 0 → WARP_CHAR_INTO_CAR (0x036A), which is
  refused if a ped already sits in the driver seat; seat n → WARP_CHAR_INTO_CAR_AS_PASSENGER (0x0430, n−1). The same
  method serves RPC 70 for the local ped. It is refused for health 0, or for seat > max passengers on automobile/bike kinds.
* **Animated enter/exit** happens only through the server relays: RPC 26 → A 0x100161A0 → TASK_ENTER_CAR_AS_DRIVER (0x05CB,
  3000 ms) or TASK_ENTER_CAR_AS_PASSENGER (0x05CA, 3000 ms, any seat). RPC 154 → A 0x10016230 → TASK_LEAVE_CAR (0x05CD),
  skipped for models 570 and 538. Both first clear the remote's keys. Both relays are dropped beyond 200 units.
* **Pulled out on foot sync**: if foot sync (207) arrives while the ped is in a car and the special action is not 3/4
  (entering/exiting), A 0x100148F0 runs WARP_CHAR_FROM_CAR_TO_COORD (0x0362) to the ped's own position. RPC 165 does the
  same for occupants of a vehicle that streams out.
* Pitfall (Observed): the stores re-warp only when the ped is **on foot**. A driver sync for a *different* vehicle, or a
  passenger seat change, does not move a ped that is already seated. A foot sync (or stream-out) has to come first.
* "Car Jacked": on the transition on-foot → driver, if the remote's vehicle is the one the local player drives, the local
  ped is ejected and "~r~Car Jacked~w~!" is shown (A 0x10016270).

### 2.2 Applying driver sync (A 0x100166B0; constants in remote-players-sync §2.1) (Observed)

One-shot, only when a new 200 arrived (update type 17) and the ped is seated and the vehicle handle is valid:
1. The quaternion is normalised and converted to a matrix.
2. Trains (537/538/449) go through A 0x10015650: if |Δx| or |Δy| > 10 → teleport via SET_MISSION_TRAIN_COORDINATES (0x07C7).
   Then `m_vecMoveSpeed` := synced velocity and `CTrain::m_fTrainSpeed` (+0x5A4) := `extra` if within ±100. No rotation
   handling.
3. Other vehicles (A 0x100155E0): the target quaternion is stored at remote +0x16C, the target position and velocity are
   stored, and `m_vecMoveSpeed` (+0x44) := synced velocity. Then A 0x10015140:
   * if the car is **not** in GTA's moving list (`CPhysical::m_pMovingList` +0xB4 == 0, i.e. asleep/not processed) →
     teleport by writing the matrix translation directly;
   * else if every |Δ| ≤ 0.05 → nothing;
   * else if |Δx| ≤ 8, |Δy| ≤ 8 and |Δz| ≤ zthr (2.0 for heli/plane/boat, 0.5 otherwise) → read the current move speed
     and add Δ·0.06 on axes with |Δ| > 0.05. The result is written only if some component exceeds 0.01;
   * else → teleport (matrix translation := target, move speed := synced).
   Teleports are plain matrix writes, not `CEntity::Teleport`. The car is not re-sectored until GTA does it.
4. Model 520 (Hydra): `extra` → CAutomobile +0x86C (`m_wMiscComponentAngle` and its "prev" copy, named via plugin-sdk;
   the thrust nozzle). Bike/BMX: `extra` (clamped to [−1,1]) → `CBike` ride-anim `DesiredLeanAngle` (+0x64C, named via
   gta-reversed `CRideAnimData`). The C→S side sends `LeanAngle` (+0x648).
5. Siren → `CVehicle` flag byte +0x42D bit 7. Landing gear (planes only, kind 5): wire bit 1 means "gear up"
   (`m_fLandingGearStatus` +0x9CC ≠ 0). The receiver calls `CPlane::SetGearUp` (0x6CAC20) or `SetGearDown` (0x6CAC70) only
   on a mismatch. Vehicle health → `m_fHealth` (+0x4C0) directly.
6. Trailer id 0 or ≥ 2000 while GTA reports a trailer (`m_pTrailer` +0x4C8 maps to a samp id) → DETACH_TRAILER_FROM_CAB
   (0x07AC) and clear the link. Attaching is done by packet 210 (§4.1), not here.
7. Weapon: switched, or given with 9999 ammo.

Every frame while driving (any update type):
* A 0x10015460 (not for trains): clamp `m_vecTurnSpeed` (+0x50) to ±0.02 per axis (global switch A 0x100FE4B8 = 1), then
  `q := slerp(target, current, 0.75)` normalised and written back as the matrix rotation (position kept).
* A 0x100AD420 removes a samp-attached object (DETACH_OBJECT 0x0682 + DELETE_OBJECT 0x0108) if the ped has one. (Inferred:
  it clears a hand-held object when seated.)
* RC models (441, 464, 465, 501, 564, 594): the remote ped object's virtual slot 2 runs. It is the generic "remove from
  world" (`CWorld::Remove` 0x563280), so the RC "driver" is taken out of the world.
* NPC drivers whose synced velocity is zero get the vehicle move speed zeroed.
* Keys/lr/ud are written into the remote's **slot pad** (A 0x100AF380). The horn is applied from that pad: `m_nHornCounter`
  (+0x514) := pad ShockButtonL ≠ 0 (A 0x100AE2B0). AFK timers are in remote-players-sync.

### 2.3 The vehicle `ProcessControl` hook (A 0x100A3140, all ten vehicle vtables slot +0x28) (Observed)

Entry: the `CVehicle*` is saved, the original `ProcessControl` is chosen by vtable (CAutomobile 0x6B1880, CBoat 0x6F1770,
CBike 0x6B9250, CPlane 0x6C9260, CHeli 0x6C7050, CBmx 0x6BFA30, CMonsterTruck 0x6C8250, CQuadBike 0x6CDCC0, CTrain 0x6F86A0,
CTrailer 0x6CED20), and `CWorld::PlayerInFocus` (0xB7CD74) is snapshotted. There are three branches.

**A. Remote player driver** (driver ≠ null, driver `m_nPedType` (+0x598) == 0 PLAYER1, driver ≠ local ped, and
PlayerInFocus was 0):
1. `slot` := the driver's index in samp's game-player table (A 0x1026BD88, 210 entries, filled when samp creates the ped
   with CREATE_PLAYER 0x0053).
2. **Context swap in**:
   * save `CPad::Pads[0]` (0xB73458, 0x134 bytes), then copy the slot's pad (A 0x10152778 + slot·0x134) into it;
   * `TheCamera.m_aCams[0].m_bLookingLeft/Right` (0xB6F1A4/0xB6F1A5) := slot pad LeftShoulder2/RightShoulder2 ≠ 0;
   * `m_aCams[0].m_nMode` (0xB6F1A8, byte) and `TheCamera.m_PlayerWeaponMode.m_nMode` (0xB6F858) := the slot's camera mode
     from aim sync (A 0x10143E50[slot]). Weapon mode 4 becomes 0;
   * save `m_aCams[0]` front/source/source-before-look-behind/up (0xB6F32C, 0x30 bytes) and load the slot's copy
     (A 0x101442A0 + slot·0x30).
   (Offsets named via plugin-sdk `CCamera`/`CCam`/`CPad`.)
3. If the engine is off (`CVehicle` flags +0x428 bit 0x10, `bEngineOn`): for helis set entity status bit 0x20 (status |= 4 →
   ABANDONED, so the rotor runs down); zero Cross and Square (accelerate/brake) in the swapped pad, new and old state. Engine
   on → clear bit 0x20.
4. Save bike lean values (+0x648/+0x64C).
5. **Audio service as an AI car**: PlayerInFocus := 0, driver ped type := 4 (CIVMALE), entity type/status byte (+0x36) :=
   0x1A (vehicle, STATUS_PHYSICS); call `CAEVehicleAudioEntity::Service` (0x502280) on `m_vehicleAudio` (+0x138); restore
   the status byte and ped type 0. (The game's own Service calls at the top of Automobile/Bike/Boat/Train ProcessControl are
   NOPed by samp.) Inferred: this keeps remote cars from using the single "player vehicle" audio path.
6. PlayerInFocus := slot. Save the 64-byte matrix and `m_vecMoveSpeed`. **Call the original ProcessControl.**
7. After the call:
   * if (not within 200 units 2-D of the local ped, or `CColStore::HasCollisionLoaded` (0x410CE0) is false at the car's
     position) and not a rail model → **restore matrix z and move-speed z** (the car does not fall through unloaded
     ground);
   * else, for boats, planes and helis → **always restore z and move-speed z**. Vertical motion of remote-driven aircraft
     and boats therefore comes only from the synced velocity and the correction in §2.2;
   * bikes/BMX: restore lean values, then `LeanAngle += (DesiredLeanAngle − LeanAngle)·0.5` (the game's own lean result
     is discarded).
8. **Context swap out**: PlayerInFocus := 0, `Pads[0]` restored, camera mode bytes and aim block restored.

**B. Local driver** (driver is the local ped, or PlayerInFocus ≠ 0 at entry): if the engine is off, the same heli/pad
zeroing applies to the real `Pads[0]` (saved/restored around the call). Then audio Service and the original call.

**C. No driver, or a non-player driver**: run Service and the original call only if the car is towed by a seated driver
(`m_pTractor` +0x4C4 has a driver with `bInVehicle`), is a rail model, or is near the local ped with collision loaded.
Otherwise **ProcessControl is skipped**: distant or unloaded unoccupied cars are frozen.

Consequences (Inferred):
* GTA reads `Pads[0]` for any `STATUS_PLAYER` car, because `GetPadNumber()` is 0 for PLAYER1 peds. Steering, throttle,
  handbrake, hydraulics, nitro, heli/plane controls and train gas/brake for a remote driver all come from that player's
  synced keys/lr/ud. The vehicle runs the same handling locally, so between packets it drives like the real one.
* Vehicles with a remote driver are `STATUS_PLAYER` because the warp opcode seats a `CPlayerPed`. Several "player only"
  GTA paths are patched to accept non-`Players[0]` drivers: `CBike::ProcessAI` jne NOP (0x6BC9EB), the `FindPlayerVehicle`
  remote branch (0x56E0FA), `CPlayerPed::ProcessControl` remote-vehicle lookup (0x60F2C4), and the drive-by player-info
  checks (0x741FFF, 0x5E63A6).
* The remote driver *ped* also runs samp's `CPlayerPed::ProcessControl` hook (A 0x100A29A0), which performs the same
  pad/camera/aim swap. That belongs to the on-foot spec.

### 2.4 Other per-vehicle hooks that matter for remote cars (Observed unless marked)

| GTA site | A handler | behaviour |
|---|---|---|
| `ProcessEntityCollision` (vtable +0x58) of CAutomobile/CHeli/CPlane/CQuadBike (→0x6ACE70), CBike/CBmx (→0x6BDEA0), CMonsterTruck (→0x6C8AE0), CBoat (→0x546D00) | 0x100A5B50 / 0x5BF0 / 0x5C90 / 0x5D30 | If netgame +0x232 (RPC 167 DisableRemoteVehicleCollisions) is set, the other entity is a vehicle model 400..611, and **both** cars have a driver → return 0 (no collision). Otherwise the original runs. This resolves README gap 8 (consumer of +0x232). |
| `BreakTowLink` (vtable +0xF8) of 5 classes | 0x100A4CB0 | Unless samp itself is detaching (flag A 0x101515BC), the trailer has health > 0, its `m_pTractor` maps to a samp vehicle that the **local ped does not drive**, and that cab has a trailer object → the break is suppressed. Only the cab's driver (or the server) can break a tow. |
| `CVehicle::InflictDamage` entry (jmp, InitGame) | 0x100A5370 → 0x100A5280 | Damage is cancelled when the local ped drives this vehicle, the damager maps to a remote player, the server's team setting (settings +0x22) is on, and both players share a team (team ≠ 0xFF). Friendly-fire filter; see the combat spec. |
| `ProcessVehicleSirenAlarmHorn` calls (5 sites) | 0x100A3690 | During the call the entity status := PLAYER if the driver is seated, else ABANDONED; restored afterwards (Inferred: makes horns/sirens of remote-driven cars audible). |
| `UsesSiren` calls (3 sites) | 0x100A5DD0 | A one-shot global forces true. Otherwise the original runs; if false, it answers from the samp object's siren flag (stream-in byte). |
| `CTrain::ProcessControl` 0x6F8CF8 | 0x100A3DE0 | Reached only when a `STATUS_PLAYER` train has `|m_fTrainSpeed|` > 1.0. samp clamps the speed to ±1.0 (sign kept) and jumps past the block. **Correction to gta-patches**: the skipped block is the **derail-on-curve** check (gta-reversed `CTrain::ProcessControl`: speed > 1 on a bend sets `bNotOnARailRoad`), not station passengers. Trains never derail. |
| `CVehicle::Render` → SetDirtTextures 0x6D0E7E | 0x100A45A0 | Records the vehicle being rendered (global A 0x10151550) and returns. Dirt textures are never applied. |

---------------------------------------------------------------------------------------------------

## 3. Passengers (211) and unoccupied vehicles (209)

### 3.1 Remote passengers (Observed)
* Store A 0x10017440 (no timestamp check). It saves the 24-byte struct and the vehicle id/object, seat = byte2 & 0x3F, and
  warps the ped in if it is on foot (§2.1, WARP_CHAR_INTO_CAR_AS_PASSENGER seat−1).
* The one-shot apply sets the weapon, starts or stops drive-by (bit 6) and sets or clears cuffs (bit 7) (see the combat
  spec). Every frame: keys only (lr/ud = 0) into the slot pad. The synced position is ignored while seated.
* Passengers never move the vehicle. The car follows its driver's sync, or 209 if nobody drives.

### 3.2 Unoccupied sync (Observed; send rules also in remote-players-sync §209)
* **Who sends** (pool tick → A 0x10006E00): a car with no seated driver, not a rail model, not a towed trailer, within 90
  units, not stopped (stopped = in the moving list with zero move speed). If a passenger seat holds a player ped (ped type
  0), only the first such passenger sends, and only if that is the local ped (seat = index + 1). Otherwise the local player
  sends with seat 0 only if it is the closest player whose ped is in the world (ties go to the lower id); details in
  remote-players-sync §209. At most 3
  sends per tick, ≥ 100 ms apart per vehicle, and only on the "not sunk" branch (§1.4).
* **Who applies**: the 209 packet handler (A 0x10009A40) calls A 0x100158D0 on the sending remote player's object
  **immediately** (inside the network receive at the start of samp's tick, not in the per-frame update). It records the
  sender in pool +0x8E34[id]. It is refused if a player-type ped drives the car (A 0x100B7120) or the car is towed by a
  driven cab (A 0x100B7D30). Then range checks, then "nothing / nudge (keep position, set rotation, synced velocity + Δ·0.06,
  synced turn speed) / teleport" with the thresholds 0.1 / (6, 6, 3).
* The health field (offset 63) is **not** applied on receive (no read of it in A 0x100158D0).
* Pitfall (Inferred): two clients that each think they are closest can both send. The server picks. Receivers apply
  whatever arrives, including the local player's own vehicle if it is unoccupied locally.

---------------------------------------------------------------------------------------------------

## 4. Trailers, trains, aircraft, boats

### 4.1 Trailers (Observed)
* RPC 148 attaches with ATTACH_TRAILER_TO_CAB (0x0893). RPC 149 and the driver-sync "trailer 0" detach with 0x07AC under the
  "samp detaching" flag, which `BreakTowLink` lets through.
* Packet 210 apply (A 0x10015C90, immediately in the receive handler, only if the sender has a vehicle): the trailer model
  must be towable. It attaches with 0x0893 if not already linked, then "nothing (≤ 0.5) / nudge (rotation and turn speed
  synced, velocity += Δ·0.025) / teleport" with thresholds (6, 6, 3).
* Locally, a towed trailer is simulated (ProcessControl branch C runs because the tractor's driver is seated). Its tow link
  cannot break unless the local player drives the cab.
* The local driver sends 210 after each 200 while `m_pTrailer` maps to a samp id 1..1999.

### 4.2 Trains (Observed)
* Created as mission trains (§1.2). Positioned by SET_MISSION_TRAIN_COORDINATES (RPC 159, driver sync teleport > 10).
  Speed comes from `extra` (`m_fTrainSpeed`). No rotation slerp, because the rail determines orientation.
* Unoccupied sync is never sent for rail models. Rail models always run ProcessControl (branch C).
* Local exit from a train (the leave-car hook, §5) does not send RPC 154. The ped is warped to train position +2.5 on x and y
  (WARP_CHAR_FROM_CAR_TO_COORD). Model 570 cannot be entered as driver. As passenger, the local ped warps instead of
  animating.
* The init patch and hook at 0x6F8CF8 replace the derail block (§2.4). Whether a separate station-passenger block is
  disabled elsewhere was not checked (gta-patches lists the train passenger block as skipped; that row is the same site).

### 4.3 Aircraft and boats (Observed)
* Remote-driven planes/helis/boats: z position and z velocity are frozen to their pre-ProcessControl values each frame
  (§2.3 A.7). The z correction threshold is 2.0.
* Helis with the engine off are forced to ABANDONED status (rotor stops) for remote drivers and the local driver.
* Hydra nozzle via +0x86C. Landing gear via SetGearUp/SetGearDown. Planes with a remote driver also get the
  ProcessFlyingCarStuff RET patch (all vehicles).
* Boats are exempt from the "sunk" death check. Boats are eligible for unoccupied sync even when the sunk check would
  apply.

---------------------------------------------------------------------------------------------------

## 5. Enter / exit (local hooks and remote relays)

| direction | trigger | GTA site / samp | behaviour |
|---|---|---|---|
| C→S 26 driver | `CTaskComplexEnterCarAsDriver` ctor entry 0x6402F0, `jmp [0x6919BB]` → A 0x100A3860 | Filtered by return address (0x570A20 or 0x570A99 = player-input code in `CPlayerInfo::Process`, Inferred). The gate A 0x100A3700 refuses when: the car has no samp id; the samp locked flag (+0x60) is set; model 570; the local ped is surfing (fields +0x2DE/+0x2E2 set by the surf detector A 0x10003730; this resolves the vehicles.md unknown); the current driver is a non-player ped; or the current driver is a remote **NPC** (player-pool entry +8 = isNPC, set by PlayerJoin). On refusal the half-built task is deleted through its vtable slot 0 and control jumps to 0x570A9E (no enter). On accept: drop the parachute, send 26 (passenger 0), resume the original ctor at 0x6402F7. |
| C→S 26 passenger | per-frame poll A 0x10006FE0 | Closest car < 8 units → TASK_ENTER_CAR_AS_PASSENGER (0x05CA, −1 seat) and send 26 (vehicles.md). |
| C→S 154 | `CTaskComplexLeaveCar` ctor 0x63B8C0 → A 0x100A38C0 | Only for callers 0x5704A1 / 0x5703FC (player input) and when the vehicle is the local ped's `m_pVehicle` (+0x58C). Trains: warp beside the train, no RPC. Otherwise A 0x10005BF0 (records the last driven vehicle; sends 154 unless RC). The hook then resumes the ctor at 0x63B8C7. It pushes the value of A 0x10113AC0 (−1 normally, 1 for trains) where the original pushed the SEH try-level −1 (Inferred: no functional effect). The "one-shot suppress flag" A 0x10151488 is only read and cleared, never set in A (dead). |
| local forced | RPC 70 / 71 | 70: warp (0x036A / 0x0430) and camera reset. 71: TASK_LEAVE_CAR; no 154, because the caller address does not match. |
| remote | RPC 26 / 154 relays | Animated tasks (§2.1). Driver sync warps if the animation did not finish in time. |

Doors and locks: lock = samp flag +0x60 plus `m_eDoorLock` (+0x4F8) := 2 (RPC 24). It is only enforced locally by the
driver-enter gate. GTA's own door-lock check also blocks the enter task (Inferred). Passenger entry is **not** gated on the
samp lock flag in the poll (vehicles.md). Door and bonnet open ratios and windows come from RPC 24 only.
`CTaskComplexEnterCarAsDriverTimed` no longer registers its vehicle reference (0x63ADC8 NOP), and the car-entry tasks keep the
ped's weapon (0x64DB49/0x64BC9F/0x64B872 NOPs) so drive-by works.

---------------------------------------------------------------------------------------------------

## 6. Per-frame order (one GTA frame, `Idle` 0x53E920)

1. `CTimer::Update`, then `CGame::Process` → … → `CWorld::Process`:
   a. **ProcessControl pass** over the moving list: every vehicle goes through the samp hook (§2.3). A remote-driven car
      gets that player's pad/camera/aim swapped in. Ped `ProcessControl` (including remote drivers) goes through the ped
      hook.
   b. **ProcessCollision pass**: GTA applies `m_vecMoveSpeed` and moves the cars. The ProcessEntityCollision hooks
      (RPC 167) apply here.
   c. Camera: the `CCamera::Process` call is hooked (camera spec).
2. Pre-render, render (vehicle render hooks: dirt, plates).
3. At the `Render2dStuff` call (0x53EB12, retargeted to A 0x100C5030) **samp's tick** runs. Netgame Process A 0x1000B5B0
   performs:
   a. Network receive: packets are dispatched; 207/200/211 are *stored*; 209/210 and RPCs (26, 154, 70, 164, 165, 24,
      106, …) are *applied immediately*.
   b. Player pool A 0x100137C0: every remote player → A 0x100166B0 (one-shot apply plus per-frame slerp/clamp/keys,
      §2.2). Then the local player tick A 0x100074C0 (sends 200/210/211/206/26-passenger poll/106).
   c. Vehicle pool tick A 0x1001F3C0 (§1.4: proofs, death, 209 sends, engine/lights, colours, blips).
4. 2-D/HUD.

So sync written in frame N's samp tick (velocity, matrix, slerped rotation) is integrated by GTA physics in frame N+1. The
remote's keys stored in frame N drive frame N+1's ProcessControl.

---------------------------------------------------------------------------------------------------

## 7. Design implications for the Rust DLL

Minimal GTA-side hooks/patches for vehicles (all from gta-patches; the behaviour to implement is above):
1. **Vehicle ProcessControl redirect** (10 vtables, slot +0x28), with the three-branch logic and the full context swap
   (Pads[0], two look flags, two camera-mode fields, 0x30-byte cam-0 vector block, PlayerInFocus, temporary ped-type/status
   swap around audio Service). Save and restore must be exception-safe and must also cover the case where the original
   re-enters samp code (the hook guards with the PlayerInFocus snapshot).
2. The four **audio Service NOPs** in the class ProcessControls (otherwise audio is serviced twice). The siren/horn and
   UsesSiren hooks are needed for audible remote sirens.
3. **Player-only path patches**: CBike::ProcessAI, FindPlayerVehicle, CPlayerPed::ProcessControl remote vehicle,
   drive-by/RemoveWeaponWhenEnteringVehicle, SetDriver freebies, AddPedInCar, ProcessFlyingCarStuff.
4. **Enter/exit task ctor hooks** with return-address filtering (driver enter gate, exit RPC). This is the only way to
   learn about player-initiated enter/exit, because GTA has no event for it.
5. **BreakTowLink** redirect (5 vtables), **ProcessEntityCollision** redirects (only if RPC 167 is supported), the
   **CTrain derail clamp**, and the **InflictDamage** filter (only for team friendly fire).
6. The Vehicles pool raised to 710. No vehicle-specific relocation is needed beyond that and the 256-colour table.

Pitfalls:
* **Script-opcode dependence**: creation, deletion, warps, tasks, proofs, trailers, interiors and trains all go through
  CRunningScript opcodes executed from samp's own script context (A 0x100B2350). Reimplementing them as direct GTA calls is
  possible, but the side effects of those opcodes (mission-vehicle flags, `m_nCreatedBy`, reference registration,
  CCarEnterExit bookkeeping) then have to be matched. The simplest faithful route is a small script-command executor.
* **Blocking model load** (busy-wait on HAS_MODEL_LOADED) stalls the frame. Keep the behaviour (ordering guarantees) or
  replace it with a deferred stream-in, but then every vehicle RPC that arrives before the car exists must be queued.
* **Context swap correctness**: the swapped `PlayerInFocus` takes the remote's game-player slot (up to 209). GTA code that
  calls `FindPlayerPed()`/`FindPlayerInfo()` during that vehicle's ProcessControl indexes `CWorld::Players[slot]`. How samp
  makes slots ≥ 2 safe is **Unknown** (see §9); copying the swap without solving this risks memory corruption.
* **Damage authority** is enforced only by re-applying SET_CAR_PROOFS every frame. If our DLL skips this, remote cars take
  local damage and diverge (health is overwritten by the next 200, but panels/doors/tyres are not).
* **Frozen unoccupied cars**: branch C skips physics for unoccupied cars beyond 200 units or without collision. Without it
  they fall through unloaded ground.
* **Teleport by matrix write** does not update sector lists or collision. That is acceptable only because GTA re-sectors
  moving entities. Cars that are not in the moving list are handled through the same matrix write (§2.2), so the next
  `CWorld::Add`/process must pick them up.
* **Linear id lookup** (`CVehicle*` → id) runs from several hooks every frame. Our implementation should use a hash map, but
  must keep "id 0 never matches".
* **Seat changes without on-foot sync** are not handled by the original (§2.1). Matching it is the compatible behaviour;
  improving it is optional.

---------------------------------------------------------------------------------------------------

## 8. Corrections to other specs

* `remote-players-sync.md` §209 and §4: "model set A 0x100ED574" is the descriptor for IS_CAR_IN_WATER (0x02BF), used for the
  sunk-vehicle death report (§1.4). It is not a model list.
* `gta-patches.md` / CSV row 0x6F8CF8: the skipped `CTrain::ProcessControl` block is the derail check for a player train
  above speed 1.0. samp's hook clamps the speed to ±1.0 (§2.4).
* `vehicles.md` 164 step 2: opcodes 0x09C4/0x07FF/0x053F are SET_PETROL_TANK_WEAKPOINT, SET_CAR_HYDRAULICS and
  SET_CAN_BURST_CAR_TYRES (all off). Carriages are not created separately (§1.2.5).
* `vehicles.md` 26 C→S gate: the two "local-player state checks" are the surf state, and A 0x1000E470 is the remote isNPC
  flag.
* `vehicles.md` 154 C→S: flag A 0x10113AC0 only replaces the pushed SEH try-level (Inferred: no functional effect). The
  "one-shot suppress flag" A 0x10151488 is never set.
* README gap 8: netgame +0x232 is consumed by the ProcessEntityCollision hooks (§2.4).

## 9. Unknown / not verified

* How `CWorld::Players` indexing is kept safe for game-player slots ≥ 2 (CREATE_PLAYER with slot n and the PlayerInFocus
  swap both index it; no relocation of 0xB7CD98 appears in the patch list). Belongs to the on-foot/ped-creation spec, but it
  blocks a faithful reimplementation of §2.3.
* `CColStore::HasCollisionLoaded` is called by samp with only the position argument. The area-code argument is whatever is
  on the stack (Unknown effect).
* The meaning of the "> 80 loaded vehicle models" count before `RemoveModel` (§1.3.5), and of the per-model flag at object
  +0x87.
* The purpose of A 0x10151484 (set to 1 before every enter task, never read in A's .text).
* The behaviour of the plate (RwRasterCreate/RwRasterDestroy) and SetupRender hooks beyond what gta-patches states.
* Whether GTA's own door lock blocks the passenger enter task for locked cars (no samp gate exists for passengers).
* Exact interaction of the remote driver ped's own ProcessControl hook with in-car tasks (drive-by, horn via ped), which is
  left to the on-foot/combat specs.

---

## Cross-check (main session, after combat spec): `CWorld::Players` for slots ≥ 2

Risk #1 above is resolved by remote-control-combat.md plus arithmetic on known
addresses (named via gta-reversed / plugin-sdk):

- `CWorld::Players` = 0xB7CD98, `sizeof(CPlayerInfo)` = 0x190, so GTA's two
  slots end at 0xB7D0B8 — exactly where `CWorld::ms_aSectors` starts.
- samp relocates the sector array to its own buffer and zeroes the original
  115 200 bytes at 0xB7D0B8 (gta-patches-r4-annotated.csv rows on
  `ms_aSectors`), which leaves room for 115 200 / 0x190 = 288 more
  `CPlayerInfo` entries.
- 210 slots need 84 000 bytes, below the 116 000 available (2 original slots + the old sector array), so slots 2..209
  live in the former sector array. Setting `PlayerInFocus` up to 209 is
  therefore safe **only after** the sector relocation patch has been applied.
  Our DLL must apply that relocation before creating any remote ped.

Status: inferred (consistent addresses and sizes; the relocation is observed).

---

## RE-10 additions (static, 2026-11-04)

### A. CGame::Process pre-step (missing from §6) — Observed
`Idle` calls `CGame::Process` through samp's wrapper A 0x100A26A0 (CSV row 10). Before the original
`CGame::Process` (0x53BEE0) it runs, in this order:
1. game object pre-step A 0x100A1C50;
2. object pool motion pass A 0x10012A50 (moving / attached objects, see `world-entities.md` RE-10 additions);
   the same for the DEBUGSCRIPT object pool (A 0x1014FE24, only in samp's offline debug mode);
3. vehicle pool plate pass A 0x1001ED70 (lazy plate textures, see `vehicles.md` RE-10 additions);
4. object material pass A 0x10012B10 (objects with materials);
5. A 0x10067ED0 on the object at A 0x1026E9F8 (chat/debug window; not vehicle-related).
After the original it runs the textdraw pool pass (pools +0x1C, 2304 entries, A 0x1001E870; UI scope).
So §6 step 1 is preceded by these samp steps, and they run *before* `CWorld::Process` of the same frame.

### B. Rows 23 and 375 (`CPlayerPed::ProcessCollision`, `CVehicle::SetupRender`) — Observed
* Row 23 (A 0x100A4130): skips `CPhysical::ProcessCollision` only when the entity flag word +0x1C has bit 31 set, and
  otherwise jumps to the original. samp sets bit 31 only on its own `CObject`s (A 0x1009F780, called from the object
  constructor A 0x100A9150), so for player peds the hook is a pass-through.
* Row 375: the `ret` write to 0x6D64F0 lives in the stub A 0x100A4150, which has **no reference** in A (no call, jump or
  absolute pointer anywhere in the image). It is dead code; `CVehicle::SetupRender` is never patched. This answers the
  gta-patches.md unknown "why the ProcessCollision hook turns SetupRender into RET": it does not. F5: justify row 375 as
  "dead code".

### C. §9 updates
* Plate hooks (RwRasterCreate / RwRasterDestroy / SetDirtTextures): resolved, see `vehicles.md` RE-10 additions.
* Object +0x87 (per-model flag passed to the ctor): it is the byte `game[model − 0x122]` of the samp game object
  (A 0x100A02B0), a per-model table owned by samp's game wrapper. Inferred: "model loaded/kept by samp" (it gates the
  model release in §1.3.5). Writer not traced → still Unknown in detail.
* A 0x10151484, `HasCollisionLoaded` area argument, GTA door lock for passengers: unchanged (see
  `docs/phase3/measurement-requests.md`, M5).

### D. Radar and HUD details that matter for vehicles — Observed
* Vehicle blip colours 0x3EC/0x3EE are samp's own RGBA values (`vehicles.md` RE-10 additions, radar trace colour).
* Passenger radar zoom: CSV row 64, see `vehicles.md` RE-10 additions.

### E. Local driving skills — Observed
Driving/flying/bike/cycling skill stats are forced to 1000 at the first frame and `CStats::SetStatValue` is disabled
(CSV row 356); see `vehicles.md` RE-10 additions. Remote drivers use the same global stats while their car's
`ProcessControl` runs (no per-slot driving skills exist; only the 11 weapon skills are swapped).
