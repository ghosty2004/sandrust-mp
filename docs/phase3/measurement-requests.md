# Measurement requests for the trace lane (from RE-10)

Requests that static analysis (RE-10, `docs/phase3/re10-gaps.md`) could not settle. Each one is run with the
**original** client (samp.dll 0.3.7-R4) in the trace lane (`wine/prefix-trace`, UDP 7790, control 47701) with the
behavioural tracer (`crates/samp-tracer`) and, where a second player is needed, `samp-bot` and a test filterscript.
Grouped by milestone, ordered by impact on a 1:1 implementation (highest first). Each request names the scenario, what
to hook or sample, and what the static spec predicts; a result that differs from the prediction is the interesting
outcome and must be written back into the spec file named under "Spec".

Conventions: "hook X" = add an inline hook / typed wrapper to the tracer for GTA function X (1.0 US address);
"sample" = read memory every frame (or at the named event) through the tracer; "peek" = `tools/trace/peek_dump.py`.
Expected values come from the specs; addresses are samp.dll build A unless noted. The traced build is B: use the B
counterparts (CSV `samp_site_B`, spec tables) where a samp address is needed.

Prerequisites that several requests share:
* **P-BOT-DRIVE**: `samp-bot` mode that enters a vehicle (RPC 26 + 200) and sends driver sync (200) every 40 ms along a
  scripted path (straight 50 m at 20 m/s, then a 90° turn), optionally with keys (horn, handbrake), and passenger sync (211).
* **P-BOT-SHOOT**: `samp-bot` mode that sends aim (203) and bullet (206) packets with chosen hitType/hitId/offset.
* **P-FS**: a filterscript in `tools/trace/` that runs a fixed list of server calls on `mark` commands (rcon or chat
  command), e.g. `CreateVehicle`, `SetVehicleParamsEx`, `CreateObject`, `MoveObject`, `SetObjectMaterial(Text)`,
  `SetPlayerAttachedObject`, `CreateActor`, `RemoveBuildingForPlayer`, `GivePlayerWeapon`, `SetPlayerTeam`.

---------------------------------------------------------------------------------------------------

## M5 — vehicles

### MR-5.1 Remote driver: correction and ProcessControl context (highest impact)
* Scenario: P-BOT-DRIVE drives a Sultan (560) past the traced player (stand 15 m from the path), then an NRG-500 (522),
  a Maverick (487) and a Dinghy (473); 30 s each. Repeat once with 150 ms send interval to force larger corrections.
* Hook/sample: wrap the vehicle `ProcessControl` vtable redirect (samp's single handler; in B the CSV row 26 target) like
  the `pc` record of TRACE-2: before/after samp's function and around the original — `PlayerInFocus`, `Pads[0].NewState`
  non-zero fields, cam-0 mode, vehicle matrix pos/rot, `m_vecMoveSpeed`, `m_vecTurnSpeed`, entity status byte (+0x36),
  driver ped type (+0x598). Per frame at `CGame::Process` entry: same vehicle fields. Mark each received 200 (tracer
  sees the samp tick; log the frame at which the store happens).
* Expected (remote-control-vehicles §2.2/§2.3): once per packet move speed := synced, then +Δ·0.06 on axes with |Δ| > 0.05
  if within 8/8/(0.5 or 2.0), else teleport by matrix write; every frame rotation = slerp 25 % toward synced and turn
  speed clamped to ±0.02; inside the original call `PlayerInFocus` = the bot's slot, `Pads[0]` = the bot's keys, status
  byte 0x1A and ped type 4 only around the audio `Service`; boats/helis keep z and move-speed z unchanged by the call.
* Output: per-frame CSV of (frame, Δ position, move speed before/after, applied branch) + any field that differs from
  the prediction. Spec: `remote-control-vehicles.md` §2.

### MR-5.2 Stream-in order and timing of a fully-modded vehicle
* Scenario: P-FS creates, out of stream range, an Elegy (562) with colours 6/1, 3 tuning components (1034, 1036, 1080),
  paintjob 1, plate "RE10TEST", `SetVehicleParamsEx(engine 1, lights 1, alarm 0, doors 1, bonnet 1, boot 0, objective 1)`,
  interior 0; then a freight train (538) at Unity Station; then the player teleports next to both.
* Hook/sample: existing tracer script/streaming hooks (`ProcessOneCommand` with opcodes, `CStreaming::RequestModel`,
  `LoadAllRequestedModels`, `CWorld::Add`), plus the frame number of each received RPC (164, 123, 148, 24, 65, 96).
  Screenshot the plate after 2 s; then send `SetVehicleNumberPlate("CHANGED")` and screenshot again.
* Expected: 0x0248/0x0247/0x038B busy-wait, 0x00A5, 0x0175, 0x09C4, 0x07FF, 0x053F, door-lock/dirt writes, then
  colours/health/0x0840/damage/mods (0x06E7 per component)/0x06ED/colours, all inside the 164 frame; plate texture created
  in the next frame's CGame::Process pre-step; the second plate text is **not** shown; train: 0x06D8 once, no extra
  0x00A5 for ids +1..+3.
* Spec: `vehicles.md` 164/123 + RE-10 "number plates", `remote-control-vehicles.md` §1.2.

### MR-5.3 Local enter/exit, including locked cars and passengers
* Scenario: the traced player walks to an unlocked Sultan and presses F (driver), exits (F), presses G (passenger) on a
  second car that has a bot driver, exits; then P-FS locks a third car (`SetVehicleParamsEx` doors 1) and the player tries
  F and G on it.
* Hook: `CTaskComplexEnterCarAsDriver` ctor 0x6402F0, `CTaskComplexLeaveCar` ctor 0x63B8C0, `CTaskComplexEnterCarAsPassenger`
  ctor (find via gta-reversed), `CTaskManager::SetTask`, network sends of RPC 26/154 (tracer `mark` around key presses; take
  the send frame from `samp-dump` capture).
* Expected: F → RPC 26 (passenger 0) in the frame of the task ctor, before the task runs; G → RPC 26 (passenger 1) in the
  frame of the DPadUp edge; exit → RPC 154 in the leave-task ctor frame. Locked car: F creates no enter task and sends
  nothing; **G is the open question**: samp has no lock check for passengers — does GTA's own `m_eDoorLock = 2` stop the
  passenger task (and is RPC 26 still sent)?
* Spec: `vehicles.md` 26/154 + RE-10, `remote-control-vehicles.md` §5.

### MR-5.4 Unoccupied sync ownership (209)
* Scenario: two players near an empty car: the traced player and the bot (bot only sends on-foot sync). The traced player
  pushes the car by walking into it, then drives it 20 m and leaves it rolling; then the bot stands closer than the traced
  player while the car rolls.
* Sample: outgoing 209 packets (capture), vehicle pool "last syncer" array (+0x8E34 equivalent in B), vehicle velocity.
* Expected: 209 only while the car moves, ≥ 100 ms apart, only while the traced player is the closest streamed player
  (≤ 90 units), none while the bot is closer; ≤ 3 vehicles per tick.
* Spec: `remote-control-vehicles.md` §3.2, `remote-players-sync.md` §209.

### MR-5.5 Trailer sync (210) and tow-link breaking
* Scenario: traced player drives a Linerunner (403) and attaches an Artict1 (435); later the bot (P-BOT-DRIVE) does the
  same in view of the traced player and brakes hard so GTA would break the link.
* Hook: `BreakTowLink` vtable slots, ATTACH/DETACH opcodes (0x0893/0x07AC), outgoing 210 after each 200.
* Expected: 210 sent immediately after each 200 while attached; remote trailer never detaches locally unless a 200 says
  trailer 0 or RPC 149 arrives.
* Spec: `remote-control-vehicles.md` §4.1.

### MR-5.6 Engine/lights, manual mode, heli rotor
* Scenario: server with `ManualVehicleEngineAndLights()` on and off; P-FS toggles engine/lights; a bot heli with engine 0.
* Sample: `CVehicle` flag byte +0x428, +0x4A8, entity status for the heli, rotor speed field.
* Expected: `vehicles.md` 24 "Engine/lights application"; heli with engine off → status ABANDONED, rotor winds down.

### MR-5.7 Vehicle death, sinking and 15 s local deletion
* Scenario: traced player drives a car into the sea at Santa Maria Beach; second run sets the car's health to 0 with
  P-FS while the player is the last driver; third run: another player (bot) was the last driver.
* Sample: outgoing RPC 136 (capture), vehicle pool wasted flags, `CWorld::Remove` / DELETE_CAR 15 s later, global
  0x1026EA50 equivalent around RPC 165.
* Expected: one 136 per death, only from the last driver; local deletion after 15001 ms without any RPC.

### MR-5.8 Damage authority cost and effect
* Scenario: 20 streamed vehicles, traced player on foot, then driving one of them, 60 s.
* Hook: count SET_CAR_PROOFS (0x02AC) and SET_CAN_BURST_CAR_TYRES (0x053F) calls per frame; shoot a parked car.
* Expected: two opcodes per streamed vehicle per frame; the parked car takes no damage while the player is not its driver.

### MR-5.9 Model release on stream-out
* Scenario: stream 30 different car models in and out (teleport loop), with and without many models resident.
* Hook: `CStreaming::RemoveModel` 0x4089A0 with the model id, and the count of `ms_aInfoForModel[400..610]` entries with
  non-zero first dword at that moment.
* Expected: release only when ref count 0, object +0x87 clear and count > 80; record the +0x87 value per model to
  identify its writer.

### MR-5.10 SCMEvent C→S (96) from samp's main.scm
* Scenario: Transfender (paint, mod, colour, enter/exit) with a car; needs samp's main.scm (trace lane already serves it).
* Hook: the two samp opcodes (0x09C6 / 0x09D3 handler slots at 0x47BF54 / 0x47BF88) with their four parameters; capture.
* Expected: event types 1–4 as in `vehicles.md` 96 C→S, colours applied locally first.

### MR-5.11 Stats after the first frame
* Peek `CStats::StatTypesFloat` (0xB79380) at state 9: driving (160), flying (223), bike (229), cycling (230) and the 11
  weapon skills (69–79). Expected all 1000.0; first byte of `CStats::SetStatValue` (0x55A070) = 0xC3. After RPC 40
  (server restart) the weapon skills should read 999.0.

### MR-5.12 `HasCollisionLoaded` argument in the vehicle hook
* Scenario: bot drives into an area whose collision is not loaded for the traced player (> 300 m away, then teleport the
  traced player near).
* Hook `CColStore::HasCollisionLoaded` 0x410CE0 with both arguments when called from samp's vehicle hook.
* Expected: second argument is an arbitrary stack value; record it and whether the result ever differs from a call with
  the correct area code.

---------------------------------------------------------------------------------------------------

## M6 — world entities

### MR-6.1 Which objects are drawn by GTA and which by samp (highest impact)
* Scenario: P-FS creates near the player: (a) a plain object, draw distance 0; (b) the same model with draw distance 500;
  (c) the same with a material colour; (d) a draw-last (alpha) model, e.g. 19325 glass, with draw distance 500; then the
  player walks 400 m away and back.
* Hook: `CRenderer::AddEntityToRenderList` 0x5534B0 (entity, model), `CEntity::CreateRwObject` / `DeleteRwObject` for
  `CObject`s, `RenderEverythingBarRoads` / `RenderFadingInEntities` return points; sample per object the RW object pointer
  and samp object +0x49/+0x68 equivalents (B offsets).
* Expected: (a) goes through the render list; (b)(c)(d) never do and are drawn after the two RenderScene passes, (d) in
  the second one, far → near; out-of-range RW objects of (b)(c)(d) are deleted after ~1000 frames; screenshots identical
  in visibility to the spec.
* Spec: `world-entities.md` RE-10 §2.

### MR-6.2 MoveObject trajectory
* Scenario: P-FS `MoveObject(obj, +10 m x, speed 2.0)` with and without rotation (0, 0, 90); a second run at 20 fps
  (frame limiter) and one with `StopObject` halfway, and a `MoveObject` issued while another is running.
* Sample per frame: object matrix pos/rot, `m_vecMoveSpeed`, `m_vecTurnSpeed`, `CTimer::ms_fTimeStep`, samp's tick time.
* Expected: arrival at t ≈ 5.0 s, snap to target, move/turn speed zeroed; velocity formula and the ±10 % per-0.1-unit
  schedule correction of `world-entities.md` RE-10 §3; rotation slerped by distance progress.

### MR-6.3 Materials and material text
* Scenario: P-FS `SetObjectMaterial` on slots 0 and 1 (texture from another model; colour 0xFFFF0000), then
  `SetObjectMaterialText` (size 256x128, Arial 24, bold, white on black, centred, "RE10"), on a model with ≥ 2 materials.
* Hook: Direct3D `SetTexture` (vtable +0x104 of the device samp proxies — count substitutions), RW material texture/colour
  of the object's atomics before/after `CObject::Render`; screenshots at 5 m.
* Expected: RW material texture/colour replaced only during the object's render; material text substituted at D3D level;
  record the render-target size and the exact text placement for the M6 implementation (not analysed statically).

### MR-6.4 Attached objects on bones
* Scenario: `SetPlayerAttachedObject` on the traced player and on the bot: slot 0 bone 2 (head) offset (0.1,0,0) rot
  (0,90,0) scale (1,1,1); slot 1 bone 6 (right hand) rot (10,20,30) scale (2,1,1); walk/crouch.
* Sample per frame after `ProcessPedsAfterPreRender`: object matrix and `m_apBones[bone]` LTM.
* Expected: matrix = bone LTM × offset, rotations applied X then Y then Z, then scale (`world-entities.md` RE-10 §5).

### MR-6.5 RemoveBuildingForPlayer
* Scenario: `RemoveBuildingForPlayer(playerid, 4019, 1777.8, -1773.9, 12.5, 0.25)` (a known LS building) and a −1 radius
  20 call elsewhere; then teleport 3 km away and back so the IPL streams again.
* Sample: matching building entities' area code, z, `NumOccludersOnMap` and zeroed occluders; hook
  `CFileLoader::LoadObjectInstance` to see model 19300 substitutions.
* Expected: area code 1, z − 2000, occluders within 500 zeroed; re-streamed instances have model 19300.

### MR-6.6 Pickups
* Scenario: create type 2 and type 14 pickups and a money pickup (1212); walk through each, wait, walk again.
* Capture outgoing 131 (reliability per path), hook `CPickup::Update` patch site 0x4579C6 stub and HAS_PICKUP_BEEN_COLLECTED.
* Expected: collision path rel 9 with cooldown 15 polls; type 14 via the poll path rel 10.

### MR-6.7 Actors and RPC 178 health ≤ 0
* Scenario: `CreateActor` (invulnerable and not), `ApplyActorAnimation`, `SetActorHealth(actor, 0)`, shoot the actor,
  push the actor with a car.
* Hook: `CPedDamageResponseCalculator::ComputeDamageResponse` entry (0x4B5AC0) result, `CPed` health, ped state, task
  changes; screenshots.
* Expected: invulnerable actor never moves; 177 sent on local hits; the open question is what EXPLODE_CHAR_HEAD does to
  an actor when samp's hook cancels the response (rule 1): dies / falls / head removed / nothing.

### MR-6.8 Checkpoints
* Scenario: `SetPlayerCheckpoint` size 3; `SetPlayerRaceCheckpoint` types 0, 1, 3; disable both.
* Hook: 0x00FE, 0x06D5/0x06D6, blip opcodes per frame; screenshot of the radar.
* Expected: one 0x00FE per frame while active; blip colour 0xAA0000FF; race checkpoint recreated only by the RPC.

### MR-6.9 Gang zones
* Scenario: show 3 zones, flash one, open the pause map.
* Hook `CRadar::DrawAreaOnRadar` 0x5853D0 (rect, colour, flag); expected 500 ms phase toggle and the same calls from the
  pause-map hook.

### MR-6.10 Map icons and blip colours
* Scenario: `SetPlayerMapIcon` with styles 0–3 and types 0 / 56, colours 0xFF0000FF, 5 and 1500.
* Hook: blip opcodes and `CRadar::GetRadarTraceColour` results.
* Expected: colour 5 → player 5's colour, 1500 → garbage from past the table, RGBA used as is (`world-entities.md` RE-10 §8).

### MR-6.11 UI-side world elements (feeds RE-5)
* Object editor gizmo (EditObject / EditAttachedObject): screenshots per mode, sends of 117/116 with rates (≤ 1 per 250 ms
  for response 2). 3D text labels: draw distance, LOS test, colour codes, attachment z offset. Hand results to RE-5/M8a.

### MR-6.12 Camera collision with objects
* Scenario: object with `CreateObject(..., noCameraCol)` between camera and player vs. normal object.
* Hook the three camera-collision hooks (rows 81–83) and record which entities are tested; expected: no-cam-col objects
  skipped, only objects within 200 units tested.

### MR-6.13 Unknown world hooks
* `CWorld::ProcessVerticalLine` (row 96): log callers and samp's result vs the original for 60 s of normal play and during
  RPC 13 (SetPlayerPosFindZ). Escalators: compare escalator movement near Commerce with and without the constants of
  row 371. Record `CRenderer::SetupEntityVisibility` suppression frequency (row 374).

---------------------------------------------------------------------------------------------------

## M7 — combat

### MR-7.1 Bullet sync volume and lag-comp modes (highest impact)
* Scenario: traced player shoots (Deagle, Shotgun, Combat Shotgun, M4, Sniper hit/miss) at the bot, at a car and at the
  ground; repeat on servers with `lagcompmode` 0, 1, 2.
* Capture outgoing 206 (count per trigger pull, timing relative to the fire frame), hook `CWorld::ProcessLineOfSight`
  (the three samp-hooked call sites) with start/end/result entity.
* Expected: one 206 per LOS call (shotgun: one per pellet?), none for a sniper miss, offsets per mode
  (`remote-players-sync.md` §206).

### MR-7.2 Remote shots against the local player
* Scenario: P-BOT-SHOOT sends 203 + 206 with hitType 1 / hitId = traced player, then hitType 0 aimed at the traced
  player, in each lag-comp mode.
* Hook: `CWeapon::FireInstantHit` 0x73FB10 (args), `Find3rdPersonCamTargetVector` 0x514970, LOS results, local health,
  `ComputeDamageResponse` entry.
* Expected: hitType 1 → the ray passes through the local ped and damage + RPC 115 (taking) follow; hitType 0 → no local
  damage unless mode 2. Record which branch inside `FireInstantHit` builds the ray for a remote shooter (static unknown).

### MR-7.3 Damage reports (115/177)
* Scenario: traced player hits the bot with fist, Deagle (torso, head), Sniper, grenade; bot hits traced player (melee via
  keys); same-team run (`SetPlayerTeam` both 1); NPC victim run.
* Capture 115 (amount, weapon, body part) and local health deltas.
* Expected: amount = GTA damage × 0.33; same team → report but cancelled locally; NPC victim → report, no local effect.

### MR-7.4 Death and kill attribution
* Scenario: local player killed by (a) bot gunfire, (b) car driven by bot, (c) car whose bot is a passenger only,
  (d) drowning, (e) fall; remote death via `SetPlayerHealth(bot, 0)` → server sends 166.
* Capture 53 (reason, killer); hook EXPLODE_CHAR_HEAD (0x0321) effects on the remote ped (screenshot: head present?).
* Expected: table in `remote-control-combat.md` §7 (+ RE-10 §3: (c) may name the passenger).

### MR-7.5 Weapon skills and dual wield
* Peek stats after the first frame and after spawn (expected 1000), after RPC 34 (value), after RPC 40 (999). Give the bot a
  Colt and set its skill to 999 vs 1000 via RPC 34: screenshot single vs dual pistols on the remote ped.

### MR-7.6 Drive-by
* Scenario: traced player as passenger with an Uzi aims and fires (local drive-by) → capture 211 bit 6 and the field
  that drives it; P-BOT-DRIVE passenger with bit 6 set/cleared.
* Hook TASK_DRIVE_BY (0x0713) parameters and the secondary task slot; expected parameters (−1, −1, 0, 0, 0, 300.0, 8, 1, 20).

### MR-7.7 Aim sync content and rates
* Scenario: on foot idle, aiming, aiming + firing, sniper zoom in/out, in a car, as passenger with drive-by.
* Capture 203 rate and fields (camMode, zoom/weaponState, aspect).
* Expected: rates of `remote-players-sync.md` §0.4; zoom = trunc((FOV−35)/35×63).

### MR-7.8 Weapons update (204)
* Scenario: pick weapons, shoot until a clip empties, switch weapons, target the bot (lock aim) and an actor.
* Capture 204: target ids, slot records, rate (≥ 100 ms unless a change).

### MR-7.9 Respawn timing
* Scenario: die with class info valid; die after `ForceClassSelection`; die while spectating.
* Record frames from death (ped state 54) to RPC 53, to RPC 52, the hospital override fields, `CCamera::Fade` patch state.

### MR-7.10 One-frame health gap
* Scenario: P-FS `CreateExplosion` type 7 at the bot while it stands next to fuel barrels.
* Sample the remote ped's health before samp's end-of-frame reset; expected: never ≤ 0 locally (if it is, record the
  sequence).

### MR-7.11 Melee and projectiles from keys
* Scenario: bot sends on-foot sync with FIRE pressed holding a grenade (16), a satchel (39) + detonator, a spraycan, a fist.
* Hook `CProjectileInfo::AddProjectile`, satchel hooks, damage events.
* Expected: projectiles exist only locally per client, satchels blow only for their owner (`remote-control-combat.md` §3.5).

### MR-7.12 Carry and cuffs masks
* Scenario: `SetPlayerSpecialAction` 25 (carry) and 24 (cuffed) on the traced player; press jump/crouch/sprint.
* Sample `Pads[0]` Square/ShockL/Cross after samp's mask and the ped's move state.
* Expected: carry blocks all three; cuffs blocks sprint and forces fists.
