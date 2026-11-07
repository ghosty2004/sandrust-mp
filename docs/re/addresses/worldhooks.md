# worldhooks.rs — samp.dll addresses

Every samp.dll address behind `crates/samp-client/src/worldhooks.rs` (A = 2021 build, B = 2019 build; "—" = not recorded).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `g::CAMERA_POS` | 0x1009D760 | — | samp's camera-distance helper that reads `TheCamera` position | world-entities.md, "Object streaming and the two render paths" |
| `g::RADAR_IN_MENU` | 0x100A1486 | — | where samp reads the "in menu" byte for DrawAreaOnRadar | world-entities.md, "Gang zones" |
| `ENTITY_FLAG_CLIENT_OBJECT` | 0x1009F780 | — | bit 31 set on every samp-created object (object ctor 0x100A9150) | world-entities.md, "Object streaming and the two render paths" |
| `ENTITY_FLAG_LIGHTING` | 0x100A9098 | — | lighting wrapper sets entity bit 28 | world-entities.md, "Materials (RPC 44/84) at render time" |
| `rw_render_state` | 0x100C3E60 | — | samp's RwRenderStateSet wrapper | — |
| `d3d_render_state`, `set_fog` | 0x100C3D70 | — | samp's D3D fog setter | world-entities.md, "Actors" |
| `PREVIEW_VEHICLE` | 0x1026EA58 | — | vehicle-preview flag | — |
| `PREVIEW_VEHICLE` (set site) | 0x1006C608 | — | model-preview render that sets the flag around the vehicle draw | — |
| `samp_fog_render_state`, `rw_fog_enabled` | 0x100C3DC0 | — | samp's RenderWare render-state hook | — |
| `rw_fog_enabled` | 0x1026EA54 | — | samp's fog-enabled flag kept by that hook | world-entities.md, "Actors" |
| `mark_drawn` | 0x100AA124, 0x100A9E9D | — | "drawn" flag +0x118F := 1 (samp pass, CObject::Render hook) | world-entities.md, "Materials (RPC 44/84) at render time" |
| `apply_materials` | 0x100A8C40 | — | material pre-pass | world-entities.md, "Materials (RPC 44/84) at render time" |
| `apply_materials` (gate) | 0x100AA082, 0x100A9E7D | — | +0x118F test in the samp pass / CObject::Render hook | same |
| `apply_materials` (record) | 0x100A8C59, 0x100A8B10 | — | record branch and its atomic callback (+0x118F := 0) | same |
| `apply_materials` (apply) | 0x100A8880 | — | apply atomic callback | same |
| `apply_materials` (`texts`) | — | 0x100A86C0 | 2x2 text-slot texture set as the slot's material texture | same |
| `restore_materials` | 0x100A9EB0 | — | material post-pass | same |
| `ESCALATOR_MODELS` | 0x100A4BB8 | — | escalator model ids 3586, 3743, 8980, 8979 | world-entities.md, "Object streaming and the two render paths" |
| `object_create_rw` | 0x100A4B30 | — | CObject CreateRwObject hook (row 67) | same |
| `entity_delete_rw` | 0x100A4BA0 | — | DeleteRwObject hook (rows 68/69) | same |
| `escalator_update` | 0x100A4C40 | — | CEscalator::Update wrapper (row 66) | — |
| `process_attached_entities` | 0x100A4990 | — | ProcessAttachedEntities + surfing (row 24) | — |
| `position_attached_entity` | 0x100A4F10 | — | PositionAttachedEntity hook (row 75) | world-entities.md, "Two more object hooks" |
| `position_attached_entity` (rotation) | 0x100A7E60 | — | rewrites the rotation from the stored one | same |
| `update_rw` | 0x1009ECE0 | — | UpdateRwMatrix + UpdateRwFrame helper | — |
| `render_list_skip` | 0x100A5150 | — | AddEntityToRenderList filter (row 78) | world-entities.md, "Object streaming and the two render paths" |
| `draw_object` | 0x100A9FD0 | — | one manual draw | same |
| `draw_object` (lighting) | 0x100A9080 | — | lighting wrapper around the RW render | world-entities.md, "Materials (RPC 44/84) at render time" |
| `draw_object` (non-road) | 0x100A8FA0 | — | PreRender + RenderOneNonRoad path | — |
| `render_rw` | 0x100B6370 | — | RW object render (atomic / clump) | — |
| `samp_pass` (draw-last sort) | 0x10012BD0 | — | far → near sort of draw-last objects | same |
| `after_render_everything_bar_roads` | 0x100A50D0 | — | opaque pass (row 76) | same |
| `after_render_fading_in` | 0x100A5110 | — | draw-last pass (row 77) | same |
| roadsign guard (row 79) | 0x100A5430 | — | RenderRoadsignAtomic null guard | world-entities.md, "Two more object hooks" |
| `camera_collision_vehicles_off` | 0x100A5B00 | — | camera vs vehicles (row 82) | — |
| `SPHERE_CAST_VS_ENTITY` | — | 0x1011742C | samp's pointer to SphereCastVsEntity | — |
| `PHYSICAL_MOVING_LIST` | — | 0x1009F400 | samp's vehicle camera test (requires the moving-list link) | — |
| `CAMERA_TEST_LIMIT` | — | 0x100A59DC…0x100A5A18 | ±3000 square test | — |
| `CAMERA_OBJECT_RANGE` | — | 0x10012DB0 | object camera test, distance < 200 | — |
| `CAMERA_VEHICLE_RANGE` | — | 0x1001EDD0 | vehicle camera test, distance < 300 | — |
| `camera_entity_hit` | — | 0x1009FC30 | per-entity camera test | — |
| `build_cache_of_camera_collision` | — | 0x100A5960 | camera collision cache hook (row 81) | — |
| `build_cache_of_camera_collision` (debug pool) | — | 0x1014FE24 | second object pool (`-d` debug script), empty in network play | — |
| `camera_ignore_object_stub` | 0x100A5B30 | — | camera ignores attached objects (row 83) | — |
| `load_object_instance_stub` | 0x100A4AB0 | — | LoadObjectInstance hook (row 65) | world-entities.md, "RemoveBuildingForPlayer (RPC 43) internals" |
| `pickup_touched` | 0x100A3F80 | — | pickup collision hook (row 44) | world-entities.md, "131 OnPlayerPickUpPickup (C→S)" |
| `pickup_touched` (lookup) | 0x10013440 | — | GTA pickup index → samp id and report | same |
| `draw_gang_zones` | 0x100A3F40 | — | DrawRadarGangOverlay replacement (rows 41/42) | world-entities.md, "Gang zones" |
| `draw_gang_zones` (zone loop) | 0x10002270 | — | per-zone DrawAreaOnRadar loop | same |
| `civilian_process_control` | 0x100A2CA0 | — | CCivilianPed::ProcessControl wrapper (row 20) | world-entities.md, "Actors" |
| `ped_render` | 0x100A3E70 | — | ped Render fog wrapper (rows 11–15) | world-entities.md, "Actors" |
| `set_position` | 0x1009FD60 | — | entity position helper (Remove, write, UpdateRW, Add) | — |
| `process_vertical_line_stub` | — | 0x100A5FD0 | ProcessVerticalLine wrapper (row 96) | docs/phase3/trace-notes.md, "World entities: TRACE-7 additions" |
| `PICKUP_VISIBLE_DISTANCE` | 0x1011671C | — | 10000.0 constant for CPickup::IsVisible (row 211) | — |
