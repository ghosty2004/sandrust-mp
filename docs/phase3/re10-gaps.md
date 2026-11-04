# RE-10 — static gap check for M5 (vehicles), M6 (world entities), M7 (combat)

Purpose: before the M5/M6/M7 implementation agents start, list every behaviour each milestone has to reproduce, say where it
is specified, close what static analysis of samp.dll A / gta_sa.exe 1.0 US can close, and hand the rest to the trace lane
(`docs/phase3/measurement-requests.md`). New static facts are in the "RE-10 additions" sections of
`docs/re/subsystems/{vehicles,remote-control-vehicles,remote-players-sync,world-entities,remote-control-combat,player-state}.md`.

Status: **S** = specified (behaviour and GTA side are written down, implementable without guessing), **P** = partially
(layout or main path known, a detail or a GTA-side consequence is open), **M** = missing. "before" = state of the specs
at the start of RE-10, "after" = after the RE-10 additions. MR-n.m = measurement request.

## Summary

| milestone | items | before | after | CSV rows to apply | of which applied now |
|---|---|---|---|---|---|
| M5 vehicles | 55 | S 36 / P 12 / M 7 | S 48 / P 6 / M 1 | 83 | 1 |
| M6 world | 39 | S 16 / P 12 / M 11 | S 31 / P 6 / M 2 | 61 | 0 |
| M7 combat | 35 | S 23 / P 9 / M 3 | S 29 / P 5 / M 1 | 40 | 9 |

Rows shared between milestones (counted in each): 24 (surf glue: M5/M6), 51 (radar colours: M5/M6), 356 (stats: M5/M7),
379 (Direct3D proxy: M5/M6, also UI).

Main RE-10 static results (details in the spec additions):
* M5: passenger key = `GROUP_CONTROL_FWD` (G); plate pipeline (one shared GTA raster + per-vehicle D3D texture, lazy, text
  fixed at creation, default `XYZSR998`); radar colour table (0x3EC/0x3ED/0x3EE + player colours); passenger radar zoom;
  driving skills forced to 1000; CGame::Process pre-step order; RPC 24/106 GTA function names (window 0 = open, Fix,
  SetupDamageAfterLoad); row 375 is dead code.
* M6: object streaming split (GTA render list vs samp's own RenderScene passes for material/far/timed objects); MoveObject
  integration formula; re-attach every frame; sync-rotation byte; material texture/colour application at RW level and
  material text at D3D level; attached-object bone math; invulnerable-actor pinning; ped fog; RemoveBuilding internals
  (area 1, z − 2000, occluders, IPL → model 19300); opcode names (0x0321 = EXPLODE_CHAR_HEAD); row 79 purpose corrected.
* M7: pad masks belong to the local ped object (carry: jump/crouch/sprint, cuffs: sprint); same-team filter always on;
  vehicle→player mapping includes passengers; weapon skills 1000 at start / 999 after reset; remote drive-by task
  parameters; 204 target = `CPlayerPed` +0x79C.

## M5 — vehicles

| # | item | behaviour | before | after | spec pointer | open / MR |
|---|---|---|---|---|---|---|
| 1 | RPC 164 | StreamInVehicle layout + creation sequence (create car, heading, 3 opcodes, door lock, dirt, z+0.25, trains) | S | S | vehicles.md 164; remote-control-vehicles.md §1.2 |  |
| 2 | RPC 164 | locomotive carriages id+1..+3 (GTA creates them; cursor 0x1026DE44) | S | S | remote-control-vehicles.md §1.2.5 |  |
| 3 | RPC 164 | blocking model load (busy-wait) before CREATE_CAR | S | S | remote-control-vehicles.md §1.2.2 | MR-5.2 |
| 4 | RPC 165 | StreamOutVehicle (eject local/remote, detach cabs, delete, dtor steps) | P | P | vehicles.md 165; remote-control-vehicles.md §1.3 | global 0x1026EA50 purpose unknown; MR-5.7 |
| 5 | dtor | model release rule (>80 loaded vehicle models, object +0x87) | P | P | remote-control-vehicles.md §1.3.5, RE-10 C | MR-5.9 |
| 6 | RPC 24 | SetVehicleParams incl. bonnet/boot/doors/windows | P | S | vehicles.md 24 + RE-10 'GTA function names' | window 0 = open now named |
| 7 | RPC 24 | objective / unoccupied radar blips and their colours (0x3EC/0x3EE) | P | S | vehicles.md RE-10 'radar blips' |  |
| 8 | tick | engine / lights application (manual mode, heli rotor, DoVehicleLights rewrite rows 140-147) | S | S | vehicles.md 24; gta-patches.md Vehicles |  |
| 9 | RPC 70 | PutPlayerInVehicle (warp, camera reset, trains) | S | S | vehicles.md 70 |  |
| 10 | RPC 71 | RemovePlayerFromVehicle (TASK_LEAVE_CAR, no 154) | S | S | vehicles.md 71; remote-control-vehicles.md §5 |  |
| 11 | RPC 154 | ExitVehicle relay (remote leave task, 200 m, vehicle id ignored) | S | S | vehicles.md 154 |  |
| 12 | RPC 26 | EnterVehicle relay (remote enter tasks, 3000 ms) | S | S | vehicles.md 26 |  |
| 13 | C→S 26 | driver enter gate (task-ctor hook, return-address filter, refusal rules) | S | S | remote-control-vehicles.md §5 |  |
| 14 | C→S 26 | passenger enter poll (key, distance, parachute) | P | S | vehicles.md RE-10 'passenger-enter key' | key = GROUP_CONTROL_FWD (G) |
| 15 | C→S 154 | exit hook (return-address filter, trains, RC, last-driven id) | S | S | vehicles.md 154; remote-control-vehicles.md §5 |  |
| 16 | RPC 106 | SetVehicleDamageStatus apply (Fix / SetupDamageAfterLoad, tyres) | P | S | vehicles.md 106 + RE-10 names |  |
| 17 | C→S 106 | damage-status poll while driving | S | S | vehicles.md 106 |  |
| 18 | RPC 57/65/91 | remove component, link to interior, set velocity | S | S | vehicles.md |  |
| 19 | RPC 96 S→C | SCMEvent apply (paintjob, mod with 20 ms load, colours, mod-shop follow) | S | S | vehicles.md 96 |  |
| 20 | C→S 96 | SCMEvent send from samp opcode stubs (CSV rows 2/3) raised by samp's main.scm | P | P | vehicles.md 96 C→S | trigger depends on samp main.scm (SAA-2); MR-5.10 |
| 21 | RPC 123 | SetVehiclePlate: when the text takes effect, default plate | P | S | vehicles.md RE-10 'number plates' |  |
| 22 | C→S 136 | VehicleDeath (health 0 / sunk, last driver only, 15 s local delete) | S | S | vehicles.md 136; remote-control-vehicles.md §1.4 | MR-5.7 |
| 23 | RPC 147/148/149/159/160 | health, trailer attach/detach, position, z-angle | S | S | vehicles.md |  |
| 24 | RPC 161/98 | old 3-field params, tyre status | S | S | session-misc.md |  |
| 25 | RPC 167 | DisableRemoteVehicleCollisions consumer (ProcessEntityCollision hooks) | P | S | remote-control-vehicles.md §2.4 |  |
| 26 | pkt 200 C→S | driver sync send (rate, dup suppression, extra field, trailer link) | S | S | remote-players-sync.md §0.4, §200 | MR-5.1 |
| 27 | pkt 200 S→C | driver sync store + one-shot apply + per-frame slerp/clamp | S | S | remote-control-vehicles.md §2.2; remote-players-sync.md §2.1 | MR-5.1 |
| 28 | pkt 211 | passenger sync send/store/apply | S | S | remote-players-sync.md §211; remote-control-vehicles.md §3.1 |  |
| 29 | pkt 209 | unoccupied send rules (closest player, 3/tick, 100 ms) and apply | S | S | remote-control-vehicles.md §3.2; remote-players-sync.md §209 | two-sender case runtime: MR-5.4 |
| 30 | pkt 210 | trailer sync send/apply | S | S | remote-players-sync.md §210; remote-control-vehicles.md §4.1 | MR-5.5 |
| 31 | pkt 203 | aim sync while driving / passenger rates | S | S | remote-players-sync.md §0.4 |  |
| 32 | hook | vehicle ProcessControl redirect (10 vtables) + 3 branches + context swap | S | S | remote-control-vehicles.md §2.3 | MR-5.1 |
| 33 | hook | audio Service NOPs + 'service as AI car' swap | S | S | remote-control-vehicles.md §2.3 A.5 |  |
| 34 | hook | siren/horn/alarm hooks, UsesSiren | S | S | remote-control-vehicles.md §2.4 |  |
| 35 | hook | BreakTowLink (5 vtables) | S | S | remote-control-vehicles.md §2.4 |  |
| 36 | hook | ProcessEntityCollision (8 vtables) | S | S | remote-control-vehicles.md §2.4 |  |
| 37 | hook | train derail clamp (0x6F8CF8) | S | S | remote-control-vehicles.md §2.4, §4.2 |  |
| 38 | hook | plate RwRasterCreate/Destroy, SetDirtTextures, D3D SetTexture substitution | P | S | vehicles.md RE-10 'number plates' |  |
| 39 | hook | radar FindPlayerSpeed (passenger: no speed zoom) | M | S | vehicles.md RE-10 'radar zoom' |  |
| 40 | hook | radar trace colour replacement (shared with M6) | M | S | vehicles.md RE-10 'radar blips' |  |
| 41 | hook | CPlayerPed ProcessCollision (row 23) / SetupRender RET (row 375) | M | S | remote-control-vehicles.md RE-10 B | row 375 dead |
| 42 | tick | per-vehicle proofs + unburstable tyres every frame (damage authority) | S | S | remote-control-vehicles.md §1.4 | MR-5.8 |
| 43 | tick | CGame::Process pre-step order (objects, plates, materials before CGame::Process) | M | S | remote-control-vehicles.md RE-10 A |  |
| 44 | init | driving/flying/bike/cycling skill stats := 1000, SetStatValue RET (row 356) | M | S | vehicles.md RE-10 'driving skills' | MR-5.11 |
| 45 | tick | 'Car Jacked' ejection on remote driver sync | S | S | remote-control-vehicles.md §2.1; remote-players-sync.md §2.1 |  |
| 46 | tick | heli/plane/boat z freeze, landing gear, Hydra nozzle, bike lean | S | S | remote-control-vehicles.md §2.3/§4.3 |  |
| 47 | tick | trains: no slerp, speed from extra, exit warp, 570 rules, camera timer | S | S | remote-control-vehicles.md §4.2; vehicles.md 26/154 |  |
| 48 | init | pools (Vehicles 710, VehicleStruct 127), 256-colour palette | S | S | gta-patches.md §4.1/§4.5 |  |
| 49 | init | player-only GTA path patches (rows 168,169,173,176,204,207,218-222) | S | S | gta-patches.md Purpose analysis |  |
| 50 | init | single-player vehicle features off (rows 171,181,182,189,190,199) | S | S | gta-patches.md Purpose analysis |  |
| 51 | gap | GTA's own door lock vs passenger enter task (no samp gate) | M | M | — | MR-5.3 |
| 52 | gap | remote seat change without on-foot sync (original ignores it) | S | S | remote-control-vehicles.md §2.1 |  |
| 53 | gap | remote driver ped's own ProcessControl vs in-car tasks (drive-by, horn) | M | P | remote-players-sync.md RE-10 drive-by | MR-7.6 |
| 54 | gap | HasCollisionLoaded area argument (garbage stack value) | P | P | remote-control-vehicles.md §9 | MR-5.12 |
| 55 | gap | A 0x10151484 (set before every enter task, never read) | P | P | remote-control-vehicles.md §9 | no observable effect expected |

### CSV rows M5 must apply (83; 1 already applied)

| row | GTA addr | GTA function | applied now | note |
|---|---|---|---|---|
| 2 | 0x47bf54 | 0x47a760 CRunningScript::ProcessCommands2500To25 | no |  |
| 3 | 0x47bf88 | 0x47a760 CRunningScript::ProcessCommands2500To25 | no |  |
| 24 | 0x5689fd | 0x5684a0 CWorld::Process | no |  |
| 26 | 0x871148 | vtable CAutomobile @0x871120 slot +0x28 | no |  |
| 27 | 0x8721c8 | vtable CBoat @0x8721a0 slot +0x28 | no |  |
| 28 | 0x871388 | vtable CBike @0x871360 slot +0x28 | no |  |
| 29 | 0x871970 | vtable CPlane @0x871948 slot +0x28 | no |  |
| 30 | 0x8716a8 | vtable CHeli @0x871680 slot +0x28 | no |  |
| 31 | 0x871550 | vtable CBmx @0x871528 slot +0x28 | no |  |
| 32 | 0x871800 | vtable CMonsterTruck @0x8717d8 slot +0x28 | no |  |
| 33 | 0x871b10 | vtable CQuadBike @0x871ae8 slot +0x28 | no |  |
| 34 | 0x872398 | vtable CTrain @0x872370 slot +0x28 | no |  |
| 35 | 0x871c50 | vtable CTrailer @0x871c28 slot +0x28 | no |  |
| 36 | 0x501b1d | 0x501ab0 CAEVehicleAudioEntity::ProcessSpecialVe | no |  |
| 37 | 0x501b42 | 0x501ab0 CAEVehicleAudioEntity::ProcessSpecialVe | no |  |
| 38 | 0x501fc2 | 0x501e10 CAEVehicleAudioEntity::ProcessVehicle | no |  |
| 39 | 0x502067 | 0x501e10 CAEVehicleAudioEntity::ProcessVehicle | no |  |
| 40 | 0x5021ae | 0x501e10 CAEVehicleAudioEntity::ProcessVehicle | no |  |
| 47 | 0x6f8cf8 | 0x6f86a0 CTrain::ProcessControl (virtual) | no |  |
| 48 | 0x6402f0 | 0x6402f0 CTaskComplexEnterCarAsDriver::CTaskComp | no |  |
| 49 | 0x63b8c0 | 0x63b8c0 CTaskComplexLeaveCar::CTaskComplexLeave | no |  |
| 51 | 0x584770 | 0x584770 CRadar::GetRadarTraceColour | no |  |
| 59 | 0x6fded6 | 0x6fdea0 CCustomCarPlateMgr::CreatePlateTexture | no |  |
| 60 | 0x7fb020 | 0x7fb020 RwRasterDestroy | no |  |
| 61 | 0x7fb060 | 0x7fb060 RwRasterPushContext | no | not applied on 1.0 US by samp itself (EU-only stub) → justify 'not applicable' |
| 62 | 0x6d0e7e | 0x6d0e60 CVehicle::Render (virtual) | no |  |
| 64 | 0x586c0a | 0x586b00 CRadar::DrawMap | no |  |
| 70 | 0x871218 | vtable CAutomobile @0x871120 slot +0xf8 | no |  |
| 71 | 0x871778 | vtable CHeli @0x871680 slot +0xf8 | no |  |
| 72 | 0x8718d0 | vtable CMonsterTruck @0x8717d8 slot +0xf8 | no |  |
| 73 | 0x871a40 | vtable CPlane @0x871948 slot +0xf8 | no |  |
| 74 | 0x871be0 | vtable CQuadBike @0x871ae8 slot +0xf8 | no |  |
| 84 | 0x871178 | vtable CAutomobile @0x871120 slot +0x58 | no |  |
| 85 | 0x8716d8 | vtable CHeli @0x871680 slot +0x58 | no |  |
| 86 | 0x8719a0 | vtable CPlane @0x871948 slot +0x58 | no |  |
| 87 | 0x871b40 | vtable CQuadBike @0x871ae8 slot +0x58 | no |  |
| 88 | 0x8713b8 | vtable CBike @0x871360 slot +0x58 | no |  |
| 89 | 0x871580 | vtable CBmx @0x871528 slot +0x58 | no |  |
| 90 | 0x871830 | vtable CMonsterTruck @0x8717d8 slot +0x58 | no |  |
| 91 | 0x8721f8 | vtable CBoat @0x8721a0 slot +0x58 | no |  |
| 92 | 0x6e0954 | 0x6e0950 CVehicle::ProcessSirenAndHorn | no |  |
| 93 | 0x6b2bcb | 0x6b1880 CAutomobile::ProcessControl (virtual) | no |  |
| 94 | 0x4f77da | 0x4f7670 CAEVehicleAudioEntity::Initialise | no |  |
| 107 | 0x551024 | 0x550f10 CPools::Initialise | no |  |
| 140 | 0x6e1be3 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 141 | 0x6e1c38 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 142 | 0x6e1d98 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 143 | 0x6e1dbc | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 144 | 0x6e1ba0 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 145 | 0x6e1bb1 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 146 | 0x6e1bd2 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 147 | 0x6e1be5 | 0x6e1a60 CVehicle::DoVehicleLights | no |  |
| 148 | 0x44b1c1 | 0x44aa50 CGarage::Update | no |  |
| 149 | 0x4c8390 | 0x4c8220 CVehicleModelInfo::SetEditableMaterials | no |  |
| 150 | 0x4c8399 | 0x4c8220 CVehicleModelInfo::SetEditableMaterials | no |  |
| 151 | 0x4c83a3 | 0x4c8220 CVehicleModelInfo::SetEditableMaterials | no |  |
| 152 | 0x5817cc | 0x5816e0 CMenuSystem::DisplayGridMenu | no |  |
| 153 | 0x582176 | 0x5820e0 CMenuSystem::FillGridWithCarColours | no |  |
| 154 | 0x6a6ffa | 0x6a6dc0 CAutomobile::dmgDrawCarCollidingParticl | no |  |
| 156 | 0x5b8fe4 | 0x5b8f00 CVehicleModelInfo::SetupCommonData | no |  |
| 168 | 0x56e0fa | 0x56e0d0 FindPlayerVehicle | no |  |
| 169 | 0x6bc9eb | 0x6bc930 CBike::ProcessAI (virtual) | no |  |
| 171 | 0x44ac75 | 0x44aa50 CGarage::Update | no |  |
| 173 | 0x5e63a6 | 0x5e6370 CPed::RemoveWeaponWhenEnteringVehicle | yes |  |
| 181 | 0x53c06a | 0x53bee0 CGame::Process | no |  |
| 182 | 0x434272 | 0x4341c0 CCarCtrl::GenerateRandomCars | no |  |
| 189 | 0x53c1c1 | 0x53bee0 CGame::Process | no |  |
| 190 | 0x540040 | 0x540040 CPad::GetHandBrake | no |  |
| 194 | 0x6b18f1 | 0x6b1880 CAutomobile::ProcessControl (virtual) | no |  |
| 195 | 0x6b9298 | 0x6b9250 CBike::ProcessControl (virtual) | no |  |
| 196 | 0x6f1793 | 0x6f1770 CBoat::ProcessControl (virtual) | no |  |
| 197 | 0x6f86b6 | 0x6f86a0 CTrain::ProcessControl (virtual) | no |  |
| 199 | 0x53a984 | 0x53a570 CFire::ProcessFire | no |  |
| 204 | 0x63adc8 | 0x63ad70 CTaskComplexEnterCarAsDriverTimed::Cons | no |  |
| 207 | 0x6a8500 | 0x6a8500 CAutomobile::ProcessFlyingCarStuff (vir | no |  |
| 218 | 0x6d1874 | 0x6d16a0 CVehicle::SetDriver | no |  |
| 219 | 0x64db49 | 0x64db30 CTaskSimpleCarGetIn::ProcessPed (virtua | no |  |
| 220 | 0x64bc9f | 0x64b950 CTaskSimpleCarSetPedInAsDriver::Process | no |  |
| 221 | 0x64b872 | 0x64b5d0 CTaskSimpleCarSetPedInAsPassenger::Proc | no |  |
| 222 | 0x5e7d62 | 0x5e7cb0 CPed::AttachPedToEntity | no |  |
| 356 | 0x55a070 | 0x55a070 CStats::SetStatValue | no |  |
| 375 | 0x6d64f0 | 0x6d64f0 CVehicle::SetupRender | no | dead code (stub unreferenced) → justify |
| 379 | 0xc97c28 | data _RwD3DDevice | no | D3D device proxy; shared with UI (M4) — needed for plates (M5) and material text (M6) |

## M6 — world entities

| # | item | behaviour | before | after | spec pointer | open / MR |
|---|---|---|---|---|---|---|
| 1 | RPC 44 | CreateObject layout, create/rotation opcodes, '?' model, draw distance | S | S | world-entities.md 44; RE-10 §1 |  |
| 2 | RPC 44 | attachment storage and per-frame (re)attach to car/object | P | S | world-entities.md RE-10 §3 |  |
| 3 | RPC 44 | sync-rotation byte (object→object) | M | S | world-entities.md RE-10 §10 |  |
| 4 | RPC 44/84 | material texture/colour records and their render-time application | P | S | world-entities.md 44 step 4; RE-10 §4 | MR-6.3 |
| 5 | RPC 44/84 | material text (render target creation + D3D substitution) | P | P | world-entities.md RE-10 §4 | text layout/rendering in render target not analysed (0x100A86F0); MR-6.3 |
| 6 | RPC 45/46/47 | set position / rotation / destroy | S | S | world-entities.md |  |
| 7 | RPC 99/122 | MoveObject / StopObject setup | S | S | world-entities.md 99; RE-10 §3 |  |
| 8 | tick | object motion integration (MoveObject per frame) | M | S | world-entities.md RE-10 §3 | MR-6.2 |
| 9 | tick | object streaming (draw distance, manual stream for materials/far/timed) | M | S | world-entities.md RE-10 §2 | MR-6.1 |
| 10 | hook | render-list filter + samp's own RenderScene passes, draw-last sorting | M | S | world-entities.md RE-10 §2 | MR-6.1 |
| 11 | hook | CObject Create/DeleteRwObject hooks (AddEntity suppression, escalators) | M | S | world-entities.md RE-10 §2 |  |
| 12 | RPC 169 / 44 f6 | no-camera-collision flag and camera-collision hooks (rows 81-83) | P | P | world-entities.md 44 step 5 | hook bodies not analysed; MR-6.12 |
| 13 | RPC 75 | AttachObjectToPlayer | S | S | world-entities.md 75 |  |
| 14 | RPC 113 | SetPlayerAttachedObject store/create | S | S | world-entities.md 113 |  |
| 15 | tick | attached-object bone placement per frame | M | S | world-entities.md RE-10 §5 | MR-6.4 |
| 16 | RPC 27/28/117/116 | select / edit RPCs and sends | S | S | world-entities.md |  |
| 17 | UI | object editor gizmo (0x100735B0 / 0x10073270) and selection highlight | M | M | — | UI; MR-6.11 |
| 18 | RPC 95/63 | create / destroy pickup | S | S | world-entities.md 95/63 |  |
| 19 | C→S 131 | pickup collection: collision hook + poll, cooldown 15 | S | S | world-entities.md 131 | MR-6.6 |
| 20 | init | pickup visibility 10000, IPL/interior pickups off (rows 172, 208, 211) | S | S | gta-patches.md |  |
| 21 | RPC 171-176/178 | actors: create, hide, anim, clear, facing, pos, health | S | S | world-entities.md; RE-10 §1 |  |
| 22 | RPC 178 | health ≤ 0 → EXPLODE_CHAR_HEAD through the damage hook (cancelled for actors?) | P | P | world-entities.md RE-10 §1 | MR-6.7 |
| 23 | hook | CCivilianPed ProcessControl (invulnerable actors pinned) | M | S | world-entities.md RE-10 §6 |  |
| 24 | hook | ped Render fog switch (rows 11-15) | M | S | world-entities.md RE-10 §6 |  |
| 25 | C→S 177 | OnPlayerDamageActor | S | S | world-entities.md 177; remote-control-combat.md §6.1 |  |
| 26 | RPC 107/37/38/39 | checkpoints and race checkpoints (per-frame 0x00FE, blips) | S | S | world-entities.md Checkpoints | MR-6.8 |
| 27 | hook | blip colour mapping (0x3ED checkpoint, player-colour table, RGBA ≥ 4000) | M | S | world-entities.md RE-10 §8 |  |
| 28 | RPC 108/120/121/85 | gang zones + radar/pause-map drawing, 500 ms flash | S | S | world-entities.md Gang zones | MR-6.9 |
| 29 | RPC 56/144 | map icons (style → opcode, colour rules) | P | S | world-entities.md 56; RE-10 §1, §8 | MR-6.10 |
| 30 | RPC 36/58 | 3D text labels store + per-frame placement | S | S | world-entities.md 36/58 |  |
| 31 | UI | 3D text label drawing (0x1006BC90), colour-code pass 0x100B6140 | P | P | world-entities.md 36 | RE-5/UI scope; MR-6.11 |
| 32 | RPC 43 | RemoveBuilding: immediate effect (occluders, pools, area 1, z−2000) | P | S | world-entities.md RE-10 §7 | MR-6.5 |
| 33 | hook | RemoveBuilding replay for later IPL loads (model 19300) | P | S | world-entities.md RE-10 §7 | MR-6.5 |
| 34 | data | SAMP.img / samp.ide models (18631, 19300, …) via samp.saa gta.dat/default.dat | P | S | world-entities.md RE-10 §9; trace-notes saa | needs SAA-2 |
| 35 | init | pools/model stores (objects 3000, buildings, dummies, colmodels, TXD, model-info relocation) | S | S | gta-patches.md §4.1-4.4 |  |
| 36 | hook | RequestModel hook / negative model ids / CREATE_OBJECT jge→jne (rows 97, 348, 362, 364) | P | P | gta-patches.md §4.4 | dormant custom models; static only |
| 37 | hook | ProcessVerticalLine hook (row 96) | M | M | — | purpose unknown; MR-6.13 |
| 38 | hook | escalator constants (row 371), SetupEntityVisibility suppression (row 374) | P | P | gta-patches.md runtime table | MR-6.13 |
| 39 | RPC 179-185 | custom-model download (dormant, must not be registered) | S | S | world-entities.md; session-misc.md |  |

### CSV rows M6 must apply (61; 0 already applied)

| row | GTA addr | GTA function | applied now | note |
|---|---|---|---|---|
| 11 | 0x86d1b0 | vtable CPlayerPed @0x86d168 slot +0x48 | no |  |
| 12 | 0x86c0f0 | vtable CCivilianPed @0x86c0a8 slot +0x48 | no |  |
| 13 | 0x86c168 | vtable CCopPed @0x86c120 slot +0x48 | no |  |
| 14 | 0x86c248 | vtable CEmergencyPed (inferred) @0x86c200 slot + | no |  |
| 15 | 0x86c3a0 | vtable CPed @0x86c358 slot +0x48 | no |  |
| 20 | 0x86c0d0 | vtable CCivilianPed @0x86c0a8 slot +0x28 | no |  |
| 24 | 0x5689fd | 0x5684a0 CWorld::Process | no |  |
| 25 | 0x53ea03 | 0x53e920 Idle | no |  |
| 41 | 0x5869bf | 0x586880 CRadar::DrawRadarMap | no |  |
| 42 | 0x5759e4 | 0x575130 CMenuManager::PrintMap | no |  |
| 44 | 0x4579c6 | 0x457410 CPickup::Update | no |  |
| 51 | 0x584770 | 0x584770 CRadar::GetRadarTraceColour | no |  |
| 63 | 0x866fa8 | vtable CObject @0x866f60 slot +0x48 | no |  |
| 65 | 0x538090 | 0x538090 CFileLoader::LoadObjectInstance[inst] | no |  |
| 66 | 0x718599 | 0x718580 CEscalators::Update | no |  |
| 67 | 0x866f7c | vtable CObject @0x866f60 slot +0x1c | no |  |
| 68 | 0x866f80 | vtable CObject @0x866f60 slot +0x20 | no |  |
| 69 | 0x8585e8 | vtable CBuilding @0x8585c8 slot +0x20 | no |  |
| 75 | 0x5648d3 | 0x5647f0 CWorld::ProcessAttachedEntities | no |  |
| 76 | 0x53dfdd | 0x53df40 RenderScene | no |  |
| 77 | 0x53e019 | 0x53df40 RenderScene | no |  |
| 78 | 0x5534b0 | 0x5534b0 CRenderer::AddEntityToRenderList | no |  |
| 79 | 0x5342f9 | 0x5342b0 CEntity::RenderEffects | no |  |
| 81 | 0x41b02e | 0x41b000 CCollision::CameraConeCastVsWorldCollis | no |  |
| 82 | 0x41af80 | 0x41ac40 CCollision::BuildCacheOfCameraCollision | no |  |
| 83 | 0x41ab78 | 0x41ab20 CCollision::CheckCameraCollisionObjects | no |  |
| 96 | 0x5674e0 | 0x5674e0 CWorld::ProcessVerticalLine | no | purpose unknown (MR-6.13) |
| 97 | 0x4087ea | 0x4087e0 CStreaming::RequestModel | no |  |
| 108 | 0x55105f | 0x550f10 CPools::Initialise | no |  |
| 109 | 0x5510cf | 0x550f10 CPools::Initialise | no |  |
| 113 | 0x551097 | 0x550f10 CPools::Initialise | no |  |
| 119 | 0x551106 | 0x550f10 CPools::Initialise | no |  |
| 128 | 0x4083c0 | 0x4083c0 CStreaming::InitImageList | no | needed once the client registers SAMP.img itself (SAA-2) |
| 131 | 0x731f60 | 0x731f20 CTxdStore::Initialise | no |  |
| 132 | 0x4c67ad | 0x4c67a0 CModelInfo::AddPedModel | no |  |
| 133 | 0x4c63f2 | 0x4c63e0 CModelInfo::ShutDown | no |  |
| 172 | 0x5b47b0 | 0x5b47b0 CFileLoader::LoadPickup | no |  |
| 208 | 0x593d7c | 0x593bf0 Interior_c::Init | no |  |
| 211 | 0x454cc9 | 0x454c70 CPickup::IsVisible | no |  |
| 248 | 0x4c63f2 | 0x4c63e0 CModelInfo::ShutDown | no |  |
| 249 | 0x4c662d | 0x4c6620 CModelInfo::AddAtomicModel | no |  |
| 250 | 0x4c6822 | 0x4c6810 CModelInfo::Initialise | no |  |
| 251 | 0x4c6829 | 0x4c6810 CModelInfo::Initialise | no |  |
| 252 | 0x4c6877 | 0x4c6810 CModelInfo::Initialise | no |  |
| 253 | 0x4c6881 | 0x4c6810 CModelInfo::Initialise | no |  |
| 254 | 0x4c6890 | 0x4c6810 CModelInfo::Initialise | no |  |
| 255 | 0x4c68a5 | 0x4c6810 CModelInfo::Initialise | no |  |
| 256 | 0x4c68f3 | 0x4c6810 CModelInfo::Initialise | no |  |
| 257 | 0x4c6932 | 0x4c6810 CModelInfo::Initialise | no |  |
| 258 | 0x4c6971 | 0x4c6810 CModelInfo::Initialise | no |  |
| 259 | 0x4c69b0 | 0x4c6810 CModelInfo::Initialise | no |  |
| 260 | 0x4c69ef | 0x4c6810 CModelInfo::Initialise | no |  |
| 261 | 0x4c6a2e | 0x4c6810 CModelInfo::Initialise | no |  |
| 348 | (707 sites) | many (CModelInfo::ms_modelInfoPtrs users) | no | 707 model-pointer sites; dormant custom models but also extended ids |
| 362 | 0x46977f | 0x469390 CRunningScript::ProcessCommands200To299 | no | temporary, only for negative (custom) model ids — dormant in R4 |
| 364 | 0x41b1d0 | 0x41b1d0 ClumpCollisionStreamRead | no | temporary during samp model loads |
| 371 | 0x858ba4,0x858f84 | data (.rdata constant) | no |  |
| 372 | 0x4096aa | 0x409650 CStreaming::AddEntity | no |  |
| 373 | 0x71791f | 0x717900 CEntity::RemoveEscalatorsForEntity | no |  |
| 374 | 0x5543c0 | 0x554230 CRenderer::SetupEntityVisibility | no |  |
| 379 | 0xc97c28 | data _RwD3DDevice | no | D3D device proxy; shared with UI (M4) — needed for plates (M5) and material text (M6) |

## M7 — combat

| # | item | behaviour | before | after | spec pointer | open / MR |
|---|---|---|---|---|---|---|
| 1 | found. | remote peds in Players[2..209] + per-slot context swap | S | S | remote-control-combat.md §1 |  |
| 2 | pkt 203 | aim store (camera mode, vectors, zoom, aspect, weapon state) | S | S | remote-control-combat.md §2; remote-players-sync.md §203 | MR-7.7 |
| 3 | pkt 203 C→S | aim send rules | S | S | remote-players-sync.md §0.4, §203 | MR-7.7 |
| 4 | keys | key injection into slot pad (FIRE/AIM/SECONDARY/CROUCH) | S | S | remote-control-combat.md §3.1 |  |
| 5 | hook | FireInstantHit/FireSniper call-site suppression (rows 98-100,104) | S | S | remote-control-combat.md §3.2 |  |
| 6 | pkt 206 S→C | bullet apply: weapon, synthetic shot with swapped camera | S | S | remote-control-combat.md §3.3 | MR-7.2 |
| 7 | hook | ProcessLineOfSight hook: lag-comp redirect, local-hit suppression, capture | S | S | remote-control-combat.md §3.4 | MR-7.1, MR-7.2 |
| 8 | hook | AddBullet ×50 + immediate update; sniper shake RET | S | S | remote-control-combat.md §3.4 |  |
| 9 | gap | FireInstantHit internal condition choosing the camera ray for remote peds | M | M | remote-control-combat.md §11 | MR-7.2 |
| 10 | pkt 206 C→S | bullet send (per shot, sniper-miss rule, offsets per lag-comp) | S | S | remote-players-sync.md §206 | MR-7.1 (pellets per blast) |
| 11 | hook | satchel/detonator, TakePhotograph, crosshair, lock-on RETs, aimed-at NOP | S | S | remote-control-combat.md §3.5 |  |
| 12 | weapons | remote weapon set/give (9999 ammo, model busy-wait) | S | S | remote-control-combat.md §4 |  |
| 13 | pkt 204 | weapons update: slots + targets | P | S | remote-players-sync.md §204 + RE-10 | MR-7.8 |
| 14 | pkt 205 | stats sync | S | S | remote-players-sync.md §205 |  |
| 15 | RPC 21/22/67/145 | local weapon RPCs | S | S | player-state.md Weapons |  |
| 16 | RPC 34 | skill levels (local + per-slot remote) | S | S | player-state.md 34; remote-control-combat.md §1.2 |  |
| 17 | init | weapon skills 1000 at first frame, 999 after reset, SetStatValue RET | M | S | remote-control-combat.md RE-10 §4 | MR-7.5 |
| 18 | hook | ComputeDamageResponse decision table → RPC 115/177, cancel rules | S | S | remote-control-combat.md §6.1 | MR-7.3 |
| 19 | hook | CEventDamage::AffectsPed | S | S | remote-control-combat.md §6.2 |  |
| 20 | rule | same-team filter (Settings+0x22 always on) | P | S | remote-control-combat.md §6.3 + RE-10 §2 |  |
| 21 | rule | vehicle → player mapping includes passengers | P | S | remote-control-combat.md RE-10 §3 |  |
| 22 | tick | remote health forced to 1000 each frame (one-frame gap) | S | S | remote-control-combat.md §6.4 | MR-7.10 |
| 23 | hook | CVehicle::InflictDamage same-team filter (row 106) | S | S | remote-control-combat.md §6.5 |  |
| 24 | C→S 53 | death detection, killer/reason table | S | S | remote-control-combat.md §7; player-state.md 53 | MR-7.4 |
| 25 | RPC 166 | remote death (EXPLODE_CHAR_HEAD under focus, state 32) | P | P | remote-control-combat.md §7 + RE-10 §5 | visual result runtime: MR-7.4 |
| 26 | respawn | wasted sequence → Spawn / class selection, hospital override (row 385), Fade RET (row 365) | S | S | remote-control-combat.md §7; gta-patches.md | MR-7.9 timing |
| 27 | RPC 55 | death message (kill list UI) | P | P | player-state.md 55 | UI scope (RE-5) |
| 28 | pkt 211 bit6 | remote passenger drive-by start/stop | P | S | remote-players-sync.md RE-10 drive-by | MR-7.6 |
| 29 | C→S 211 bit6 | local drive-by detection (+0x1CB) | P | P | remote-players-sync.md §211 | setter of +0x1CB not traced; MR-7.6 |
| 30 | masks | carry / cuffs pad masks (+0x2F6/7/8) | M | S | remote-control-combat.md RE-10 §1 | MR-7.12 |
| 31 | hook | GetWeaponSkill render swap (row 22), ControlGunMove timestep (row 210) | S | S | remote-control-combat.md §1.3, §2 |  |
| 32 | init | bullet world bounds ±20000, fire-failed sounds NOPs, Wasted text NOP, ClearWeaponTarget NOP | S | S | gta-patches.md Weapons |  |
| 33 | dead | 0x858EF0 0.95→2.0 (row 370) only in unreferenced code | S | S | remote-control-combat.md §10 | F5: justify as dead |
| 34 | gap | melee / projectile parity (simulated from keys only) | P | P | remote-control-combat.md §3.1 | MR-7.11 |
| 35 | gap | P4 one-frame health gap (explosion > 1000 kills remote locally?) | P | P | remote-control-combat.md §9.2 P4 | MR-7.10 |

### CSV rows M7 must apply (40; 9 already applied)

| row | GTA addr | GTA function | applied now | note |
|---|---|---|---|---|
| 4 | 0x60dc50 | 0x60dc50 CPlayerPed::FindWeaponLockOnTarget | no |  |
| 5 | 0x60e530 | 0x60e530 CPlayerPed::FindNextWeaponLockOnTarget | no |  |
| 8 | 0x58fbbf | 0x58fae0 CHud::Draw | no |  |
| 19 | 0x86d190 | vtable CPlayerPed @0x86d168 slot +0x28 | yes |  |
| 21 | 0x86d744 | vtable CTaskSimpleUseGun @0x86d724 slot +0x20 | yes |  |
| 22 | 0x7330a2 | 0x732f30 CVisibilityPlugins::RenderWeaponPedsFor | no |  |
| 45 | 0x73c252 | 0x73c1f0 CWeapon::TakePhotograph | no |  |
| 46 | 0x73ace2 | 0x73aac0 CWeapon::FireSniper | no |  |
| 53 | 0x4b5ac0 | 0x4b5ac0 CPedDamageResponseCalculator::ComputeDa | no |  |
| 56 | 0x738f3a | 0x738b20 CProjectileInfo::Update | no |  |
| 57 | 0x738877 | 0x738860 CProjectileInfo::RemoveDetonatorProject | no |  |
| 80 | 0x4b35a0 | 0x4b35a0 CEventDamage::AffectsPed (virtual) | no |  |
| 98 | 0x742495 | 0x742300 CWeapon::Fire | no |  |
| 99 | 0x7424ec | 0x742300 CWeapon::Fire | no |  |
| 100 | 0x742548 | 0x742300 CWeapon::Fire | no |  |
| 101 | 0x740721 | 0x73fb10 CWeapon::FireInstantHit | no |  |
| 102 | 0x740b69 | 0x73fb10 CWeapon::FireInstantHit | no |  |
| 103 | 0x736247 | 0x7360d0 CBulletInfo::Update | no |  |
| 104 | 0x7424cb | 0x742300 CWeapon::Fire | no |  |
| 105 | 0x73ac4b | 0x73aac0 CWeapon::FireSniper | no |  |
| 106 | 0x6d7c90 | 0x6d7c90 CVehicle::InflictDamage | no |  |
| 174 | 0x621aea | 0x621960 (unnamed) | yes |  |
| 175 | 0x62d331 | 0x62cce0 CTaskComplexKillPedOnFootArmed::Control | yes |  |
| 176 | 0x741fff | 0x741fd0 CVehicle::DoDriveByShootings | yes |  |
| 191 | 0x56e5ad | 0x56e580 CPlayerInfo::KillPlayer | no |  |
| 192 | 0x609cb4 | 0x609c80 CPlayerPed::ClearWeaponTarget | no |  |
| 200 | 0x60f289 | 0x60ea90 CPlayerPed::ProcessControl (virtual) | no |  |
| 201 | 0x60f29d | 0x60ea90 CPlayerPed::ProcessControl (virtual) | no |  |
| 210 | 0x61e0c2 | 0x61e0c0 CTaskSimpleUseGun::ControlGunMove | no |  |
| 240 | 0x7361b2 | 0x7360d0 CBulletInfo::Update | no |  |
| 241 | 0x7361cb | 0x7360d0 CBulletInfo::Update | no |  |
| 242 | 0x7361e0 | 0x7360d0 CBulletInfo::Update | no |  |
| 243 | 0x7361f5 | 0x7360d0 CBulletInfo::Update | no |  |
| 247 | 0x4c3a5b | 0x4c3870 CEventHandler::ComputeEventResponseTask | no |  |
| 356 | 0x55a070 | 0x55a070 CStats::SetStatValue | no |  |
| 365 | 0x50ac20 | 0x50ac20 CCamera::Fade | yes |  |
| 366 | 0x5e92f4 | 0x5e8cd0 CPed::ProcessControl | yes |  |
| 367 | 0x6884c4,0x688200 | 0x6883d0 CTaskSimplePlayerOnFoot::PlayerControlZ | yes |  |
| 370 | 0x858ef0 | data (.rdata constant) | no | dead code (only in unreferenced 0x100A2FA0) → justify |
| 385 | 0xa4326c(u16),0xa43318(3 | data NumberOfHospitalRestarts; data HospitalRest | yes |  |

## Notes for F5 (patch coverage)

* "Applied now" was read (read-only) from `crates/samp-client/src/patches.rs` (table entries with `csv_row`),
  `sectors.rs` (rows 138, 262–346 without 269/325, 347), `swap.rs` (19, 21, 23, 366, 367), `dll.rs` (6, 120, 357,
  377, 378) and `client.rs` (363, 365, 385). It does not check whether the applied behaviour matches samp's (e.g. row 23 is
  a pass-through).
* Rows proposed for an F5 justification instead of application: 61 (EU-only stub, not applied by samp on 1.0 US), 370 and
  375 (dead code in samp), 380–384 (marked dead in the CSV). Rows 362/364/348 belong to the dormant custom-model path and
  can be justified for R4 if no extended model ids are used.
* Rows not listed in M5–M7 belong to other milestones (start-up/M1, SP-OFF, M3 on-foot, M4/M8a UI, M8b camera/input/
  animation, M9 audio).
