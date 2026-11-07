# worldents.rs — samp.dll addresses

Shared world-entity rules and constants (`crates/samp-client/src/worldents.rs`). A = 2021 build, B = 2019
build. Spec: docs/re/subsystems/world-entities.md ("RE-10 additions" subsections cited by title).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `MODEL_UNKNOWN` | 0x100A923B | — | immediate 0x48C7 (18631), the "?" model | world-entities.md, "SA:MP models (SAMP.img, 18631, 19300)" |
| `MAX_MODEL` | 0x100A09D0 | — | compare with 0x4E20 (20000) | world-entities.md, "44 CreateObject (S→C)" |
| `SAMP_PASS_DRAW_DISTANCE` | 0x100E6170 | — | float constant 300.0 | world-entities.md, "Object streaming and the two render paths" |
| `REQUEST_MARGIN` | 0x100E5808 | — | float constant 30.0 | world-entities.md, "Object streaming and the two render paths" |
| `DIRECT_RENDER_DISTANCE` | 0x100ECD88 | — | float constant 600.0 | world-entities.md, "Object streaming and the two render paths" |
| `MOVE_SPEED_SCALE` | 0x100E6130 | — | float constant 0.02 | world-entities.md, "Per-frame object process (MoveObject, attachments)" |
| `MOVE_DRIFT` | 0x100E611C | — | float constant 0.1 (threshold and gain) | world-entities.md, "Per-frame object process (MoveObject, attachments)" |
| `TURN_GAIN` | 0x100E6120 | — | float constant 0.01 | world-entities.md, "Per-frame object process (MoveObject, attachments)" |
| `TURN_CLAMP` | 0x100E60A4, 0x100ECD80 | — | float constants ±0.001 | world-entities.md, "Per-frame object process (MoveObject, attachments)" |
| `GTA_PICKUPS` | 0x100A3F80 | — | RPC 131 stub reading `CPickups::aPickUps` | world-entities.md, "131 OnPlayerPickUpPickup (C→S)" |
| `PICKUP_COOLDOWN` | 0x100134E5 | — | cooldown immediate 15 | world-entities.md, "131 OnPlayerPickUpPickup (C→S)" |
| `PICKUP_TYPE_POLLED` | 0x10013573 | — | type-14 compare in the pickup poll | world-entities.md, "131 OnPlayerPickUpPickup (C→S)" |
| `GANG_ZONE_FLASH_MS` | 0x10002284 | — | 500 ms compare in the flash phase | world-entities.md, "121 FlashGangZone (S→C)" |
| `FlashPhase` | 0x10002270 | — | gang-zone flash phase toggle | world-entities.md, "121 FlashGangZone (S→C)" |
| `GangZone` | 0x10002180 | — | gang-zone record layout (CRect order + colours) | world-entities.md, "108 ShowGangZone (S→C)" |
| `BLIP_COLOUR_CHECKPOINT`, `radar_trace_colour` | 0x100A7270 | — | `CRadar::GetRadarTraceColour` replacement (patch table row 51) | world-entities.md, "Blip colours (map icons, checkpoints)" |
| `drawn_by_samp` | 0x100A5150 | — | render-list filter (patch table row 78) | world-entities.md, "Object streaming and the two render paths" |
| `REMOVED_BUILDING_MODEL` | 0x100A4B03 | — | immediate 0x4B64 (19300), patch table row 65 | world-entities.md, "RemoveBuildingForPlayer (RPC 43) internals" |
| `OCCLUDER_CLEARED` | 0x1009D0F0 | — | occluder clear on building removal | world-entities.md, "RemoveBuildingForPlayer (RPC 43) internals" |
| `samp_actor_heading` | 0x100B59B0 | — | degrees → radians for actor facing | world-entities.md, "175 SetActorFacingAngleForPlayer (S→C)" |
| `invulnerable_flags` | 0x100A2CBA | — | proof-flag test of an invulnerable actor (patch table row 20) | world-entities.md, "Actors — Observed" |
| `MaterialPrePass` | 0x100A8C40, 0x100AA082, 0x100AA124, 0x100A9E9D | — | material pre-pass gate (+0x1D3 / +0x118F) | world-entities.md, "Materials (RPC 44/84) at render time" |
