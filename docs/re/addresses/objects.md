# objects.rs — samp.dll addresses

Object pool, attached objects, materials, MoveObject and the editor/selection RPC flows; "—" = not mapped in that build.
Spec: docs/re/subsystems/world-entities.md (sections named per row).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `ATOMIC_MI_VTABLES` | 0x100B4520 | — | "is atomic model info" test (CAtomicModelInfo and derived vtables) | Objects |
| `write_matrix` | 0x1009FD60 | — | matrix write: Remove, rows + position, UpdateRW, Add | Per-frame object process (MoveObject, attachments) |
| `Edit` | 0x1026E9D8 | — | editor global, as cited in objects.rs (objedit.rs cites the same value for B) | Object editor and selection UI |
| `Edit` | 0x1026E9DC | — | selection global, as cited in objects.rs (objedit.rs cites the same value for B) | Object editor and selection UI |
| `Objects::id_of_entity` | 0x10012A10 | — | pool id of a GTA object | Objects |
| `Objects::moving` | 0x100A8500 | — | "is moving" test of a pool object | Per-frame object process (MoveObject, attachments) |
| `on_rpc` PlayerBeginObjectSelect | — | 0x1000E880 | RPC 27 handler | 27 PlayerBeginObjectSelect (S→C) |
| `Objects::select` | — | 0x1006DB40 | selection on / off | Object editor and selection UI |
| `on_rpc` PlayerBeginObjectEdit | — | 0x10011880 | RPC 117 handler | 117 PlayerBeginObjectEdit (S→C) |
| `on_rpc` PlayerBeginAttachedObjectEdit | — | 0x100117C0 | RPC 116 handler | 116 PlayerBeginAttachedObjectEdit (S→C) |
| `Objects::editor_input_start` | — | 0x10072450 | editor start, object | Object editor and selection UI |
| `Objects::editor_input_start` | — | 0x10072510 | editor start, attached slot | Object editor and selection UI |
| `Objects::stop_editing` | — | 0x10072BF0 | editor stop | Object editor and selection UI |
| `Objects::send_edit` | — | 0x100728E0 | RPC 117 send (update / final) | 117 OnPlayerEditObject (C→S) |
| `Objects::send_edit` | — | 0x10072AF0 | RPC 116 send | 116 OnPlayerEditAttachedObject (C→S) |
| `Objects::drag` | — | 0x10073240 | object drag apply | Object editor and selection UI |
| `Objects::drag` | — | 0x10072240 | attached-slot drag apply | Object editor and selection UI |
| `Objects::drag` | — | 0x1009F7B0 | clear physical flag bit 3 | Object editor and selection UI |
| `Objects::send_select` | — | 0x1006DC90 | RPC 27 C→S send | 27 OnPlayerSelectObject (C→S) |
| `Objects::create` | 0x1001B340 | — | RPC 44 handler | 44 CreateObject (S→C) |
| `Objects::create` | 0x100128D0 | — | object pool create | 44 CreateObject (S→C) |
| `Objects::create` | 0x100A9150 | — | object wrapper constructor | 44 CreateObject (S→C) |
| `Objects::create` | 0x1009EC20 | — | exact position into the matrix | 44 CreateObject (S→C) |
| `Objects::create` | 0x100A80E0 | — | rotation through SCM 0x0453 | 44 CreateObject (S→C) |
| `Objects::create` | 0x100B63A0 | — | collision-model bounding radius (colmodel +0x24) | Object streaming and the two render paths |
| `Objects::attached_object` | 0x10018F00 | — | RPC 113 handler | 113 SetPlayerAttachedObject (S→C) |
| `Objects::attached_object` | 0x100AEE90 | — | attached slot set | Attached objects on players (RPC 113), per frame |
| `Objects::attached_object` | 0x100B0B50 | — | attached slot create | Attached objects on players (RPC 113), per frame |
| `Objects::remove_building` | 0x1009D1E0 | — | RPC 43 RemoveBuildingForPlayer | RemoveBuildingForPlayer (RPC 43) internals |
| `Objects::process` | 0x10012A50 | — | per-frame step before CGame::Process | Per-frame object process (MoveObject, attachments) |
| `Objects::process` | 0x100A9690 | — | per-object process | Per-frame object process (MoveObject, attachments) |
| `teleport` | 0x1009F0A0 | — | entity teleport, skipping a bare CPlaceable | 45 SetObjectPosition (S→C) |
| `destroy` | 0x100A9470 | — | object wrapper destructor | 47 DestroyObject (S→C) |
| `apply_material` | 0x100A8570 | — | material record, kind 1 (texture) | Materials (RPC 44/84) at render time |
| `apply_material` | 0x100A86F0 | — | material record, kind 2 (text) | Materials (RPC 44/84) at render time |
| `apply_material` | 0x100A864B | — | model release after the texture lookup | Materials (RPC 44/84) at render time |
| `apply_material` | 0x100A0A00 | — | model release helper | Materials (RPC 44/84) at render time |
| `apply_material` (text) | — | 0x100A86C0 | text slot: release texture and render target | Materials (RPC 44/84) at render time |
| `text_slot_texture` | — | 0x100A8715 | 2x2 raster + texture, filter mode + 1 | Materials (RPC 44/84) at render time |
| `find_texture` | 0x100B2CA0 | — | txd/texture lookup | Materials (RPC 44/84) at render time |
| `stream` | 0x100A95A0 | — | samp's own streaming for its render pass | Object streaming and the two render paths |
| `stream` | 0x100A96C0 | — | model request of ordinary objects | Object streaming and the two render paths |
| `stream` | 0x100A96C9 | — | "local ped in a vehicle" test (ped flags +0x46C bit 8) | Object streaming and the two render paths |
| `stream` | 0x100A8FE0 | — | clear the model's "pre-rendered" flag | Object streaming and the two render paths |
| `pick_within`, GTA collision constants | — | 0x10012E40 | object pool ray test | Object editor and selection UI |
| `pick_within` | — | 0x100A7DE0 | entity has an RW object | Object editor and selection UI |
| `pick_within` | — | 0x1009F060 | not the removed-building model | Object editor and selection UI |
| `PICK_COL_CACHE` | — | 0x1026CDF8 | samp's own collision cache (.bss) | Object editor and selection UI |
| `PICK_COL_CACHE` | — | 0x100B6560 | `gpColCache` swap around the sphere cast | Object editor and selection UI |
| `bound_sphere_hit` | — | 0x100B65E0 | bounding-sphere line test with frustum check | Object editor and selection UI |
| `start_move` | 0x100A9C10 | — | MoveObject setup | 99 MoveObject (S→C) |
| `stop` | 0x100A8070 | — | StopObject | 122 StopObject (S→C) |
| `move_step` | 0x1009FDC0 | — | matrix advance: pos += move speed × time step | Per-frame object process (MoveObject, attachments) |
