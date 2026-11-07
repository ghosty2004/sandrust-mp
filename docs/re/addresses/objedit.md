# objedit.rs — samp.dll addresses

Object editor and selection mode. All addresses were read in build B; build A was not mapped for this module ("—").
Spec section for every row: docs/re/subsystems/world-entities.md, "Object editor and selection UI".

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Editor` | — | 0x1026E9D8 | global editor object (0x117 bytes) | Object editor and selection UI |
| `Editor` | — | 0x10071B90 | editor object constructor | Object editor and selection UI |
| (Windows side, `ui::render`) | — | 0x100737B0 | editor per-frame draw | Object editor and selection UI |
| (Windows side, `ui::render`) | — | 0x10072C60 | gizmo drawing | Object editor and selection UI |
| `Editor::background` | — | 0x10072570 | handle painter (background colours) | Object editor and selection UI |
| `Editor::hover`, `MouseEffects::hover` | — | 0x100720D0 | hover test of the handle rects | Object editor and selection UI |
| `Editor::on_mouse` | — | 0x10073580 | window-message (mouse) handler | Object editor and selection UI |
| `Editor::drag_delta` | — | 0x100734B0 | drag step (cursor delta → units) | Object editor and selection UI |
| `Action::Drag` | — | 0x10073490 | drag step dispatch | Object editor and selection UI |
| `rotate` | — | 0x10073240 | object drag apply (translate / rotate) | Object editor and selection UI |
| `attached_drag` | — | 0x10072240 | attached-slot drag apply | Object editor and selection UI |
| `UPDATE_INTERVAL_MS` | — | 0x100728E0 | 117 update / final send (`GetTickCount() - last > 0xF9`) | Object editor and selection UI |
| `Editor::stop`, `Action::Stop` | — | 0x10072BF0 | editor stop | Object editor and selection UI |
| `Action::Press` | — | 0x100735F0 | press on an axis handle: object matrix → edit matrix | Object editor and selection UI |
| `Editor::start` | — | 0x10072450 | editor start, object | Object editor and selection UI |
| `Editor::start` | — | 0x10072510 | editor start, attached slot | Object editor and selection UI |
| `Editor::set_mode` | — | 0x10072220 | mode setter (no scale for objects) | Object editor and selection UI |
| `Editor::place` | — | 0x10072070 | handle hit rect, centred on the point | Object editor and selection UI |
| `Editor::set_direction` | — | 0x10072D8F | axis screen-direction store (inside the gizmo drawing) | Object editor and selection UI |
| `Editor::recentred`, `MouseEffects::recentre` | — | 0x100723F0 | cursor re-centre after a drag step | Object editor and selection UI |
| `EditFrame` | — | 0x10073930 | attached slot in translate mode: gizmo on the bone's axes | Object editor and selection UI |
| `GLYPH_BACKGROUND` | — | 0x100E6304 | background glyph "C" (SAMPAUX3 28) | Object editor and selection UI |
| `AXIS_GLYPHS` | — | 0x10071FD0 | axis glyphs per mode | Object editor and selection UI |
| `BUTTON_GLYPHS` | — | 0x10072701…0x100728B2 | button glyphs (translate, rotate, scale, save) | Object editor and selection UI |
| `SCALE_PER_METRE` | — | 0x100EA740 | gizmo half length per metre (0.125) | Object editor and selection UI |
| `PIXEL_UNIT` | — | 0x10073546 | pixels → drag units (× 0.01) | Object editor and selection UI |
| `wrap_degrees` | — | 0x100B48F0 | angle wrap (one ±360 step) | Object editor and selection UI |
| `transform` | — | 0x100B4D20 | matrix × vector helper | Object editor and selection UI |
| `pick_ray` | — | 0x10073F70 | pick ray through the cursor | Object editor and selection UI |
| `pick_radius` | — | 0x100B65E0 | bounding-sphere pick radius rule | Object editor and selection UI |
| `Selection` | — | 0x1026E9DC | global selection object | Object editor and selection UI |
| `Selection` | — | 0x1006D9C0 | selection object constructor | Object editor and selection UI |
| (Windows side, `ui::render`) | — | 0x1006DBC0 | selection per-frame | Object editor and selection UI |
| `PICK_DISTANCE` | — | 0x1006D9D0 | pick (150 m) | Object editor and selection UI |
| `PICK_DISTANCE` | — | 0x1006DC00 | highlight / accept limit (150 m) | Object editor and selection UI |
| `SELECT_CROSS` | — | 0x1006DAD1 | axis cross length (1.0) | Object editor and selection UI |
| `select_label`, `SELECT_LABEL_COLOUR` | — | 0x1006DA20 | highlight: axis cross and label | Object editor and selection UI |
| `Action::Select` | — | 0x1006DC90 | selection click → RPC 27 | Object editor and selection UI |
| (Windows side) | — | 0x1006DE00 | selection click handler | Object editor and selection UI |
| `objects::pick_within` | — | 0x10012E40 | object pool ray test | Object editor and selection UI |
| `PICK_SPHERE_RADIUS` | — | 0x10012E99 | sphere-cast radius (0.25) | Object editor and selection UI |
