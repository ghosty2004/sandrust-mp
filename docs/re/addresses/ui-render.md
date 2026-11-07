# samp.dll addresses: `crates/samp-client/src/ui/render.rs`

A = 2021 build, B = 2019 build; "—" = not looked up for that build. Spec paths are under `docs/re/`.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Renderer::new` | 0x100C47D0 | 0x100C47B0 | first-frame UI construction (creates every UI object) | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::new` (font renderer) | 0x1006BAF0 | 0x1006BAB0 | font renderer constructor: four fonts, sprite, plate font | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::new` (chat window) | 0x10068380 | 0x10068340 | chat window constructor: two sprites, render target, metrics | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::new`, `Renderer::create_rt` | — | 0x10068210 | `directmode` check: the chat render target is not created | subsystems/ui-rendering.md, "Render-to-texture cache" |
| `Renderer::new` (chat input) | 0x100693D0 | 0x10069390 | chat input constructor: GUI sprite, `OnResetDevice`, state block | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::new` (death window) | 0x1006A650 | 0x1006A610 | death window constructor: sprite, names font, GTAWEAPON3 fonts | subsystems/ui-rendering.md, "UI objects" |
| `Logo`, `Renderer::new` | — | 0x10070C20 | `logo.png` texture from samp.saa plus its recorded state block (run twice) | samp-saa.md, "Contents of the 0.3.7-R4 archive" |
| `Renderer::new` (object editor) | — | 0x10071B90 | object editor constructor: `ID3DXLine`, then two SAMPAUX3 fonts | subsystems/world-entities.md, "Object editor and selection UI" |
| `Renderer::new` (plate render target) | 0x1006D9B0 | 0x1006D970 | number-plate painter constructor (128×32 render target) | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::on_reset` | 0x100C41F0 | 0x100C41C0 | device reset pass (leaves the chat GUI dialog's size alone) | subsystems/ui-rendering.md, "On lost / reset device" |
| `Renderer::chat_gui`, `Renderer::on_reset` | 0x1026EA9C | — | global: the full-screen chat / input GUI dialog | subsystems/ui-rendering.md, "UI objects" |
| `Renderer::class_panel` | 0x1026EAA4 | — | global: the class-selection GUI dialog | subsystems/ui-rendering.md, "GUI dialogs created by samp"; subsystems/player-state.md, "128 PlayerRequestClass" |
| `Renderer::small_text`, `Renderer::small_text_plain` | 0x1006B4E0 | 0x1006B4A0 | font renderer small-font draw (outline on / off) | subsystems/ui-rendering.md, "Outline (\"shadow\") patterns" |
| `Renderer::audio_title` | 0x10066AB0 | 0x10066A70 | audio stream title line | subsystems/ui-rendering.md, "Smaller overlays" |
| `Renderer::update_chat_rt` | 0x10067ED0 | 0x10067E90 | chat render-target update (game frame) | subsystems/ui-rendering.md, "Render-to-texture cache" |
| `chat_rt_failure_line` | 0x100681D0 | 0x10068190 | chat render-target create, with its two failure debug lines | subsystems/ui-rendering.md, "Render-to-texture cache" |
| `Renderer::tag_bars`, `Frame::name_tag_passes` | 0x10074270 | 0x10074240 | nametag bars pass (and its guard) | subsystems/ui-rendering.md, "Who gets one" |
| `Renderer::tag_bars` | 0x1006D0A0 | 0x1006D060 | one nametag's bars | subsystems/ui-rendering.md, "Bars" |
| `Renderer::tag_bars` | 0x1006D6B0 | 0x1006D670 | bar painter begin: creates and captures its `D3DSBT_ALL` state block | subsystems/ui-rendering.md, "Bars" |
| `Renderer::tag_bars` | 0x1006CD50 | — | bar painter end: sprite `End`, then the state block is applied | subsystems/ui-rendering.md, "Bars" |
| `Renderer::tag_names`, `Frame::name_tag_passes` | 0x10075390 | 0x10075360 | nametag names pass (and its guard) | subsystems/ui-rendering.md, "Who gets one" |
| `Renderer::tag_names` | 0x1006CDA0 | 0x1006CD60 | one nametag's name and AFK icon | subsystems/ui-rendering.md, "Name text" |
| `Renderer::net_stats` | 0x10060D70 | 0x10060D20 | F5 network stats overlay | subsystems/ui-rendering.md, "Smaller overlays" |
| `Renderer::text3d` | 0x1006BC90 | 0x1006BC50 | 3D text painter (labels, bubbles, `/dl`, selection, `/ctd`) | subsystems/ui-rendering.md, "Smaller overlays" |
| `Renderer::bubbles` | 0x10066ED0 | 0x10066E90 | chat bubbles draw | subsystems/ui-rendering.md, "Chat bubbles" |
| `Renderer::labels` | 0x10001350 | — | 3D text label pool pass | subsystems/ui-rendering.md, "Present: per-frame draw order"; subsystems/world-entities.md, "36 PlayerShowTextLabel" |
| `Renderer::labels` | 0x100013BC | — | label pool pass: outline flag set only when the colour's alpha is FF | subsystems/ui-rendering.md, "Smaller overlays" |
| `Renderer::vehicle_labels` | — | 0x1006B5F7 | `/dl` main pass: colour-code font wrapper call without a sprite pointer | docs/phase3/featb-notes.md |
| `Renderer::frame` (`/dl`) | — | 0x100757E6 | Present: `/dl` label batch while the flag is on | docs/phase3/featb-notes.md |
| `Renderer::paint_plate` | — | 0x1006D840 | number-plate painter: render target, copy into a 64×32 texture | subsystems/vehicles.md, "Number plates"; subsystems/ui-rendering.md, "Smaller overlays" |
| `Renderer::paint_plate` | — | 0x1006B610 | font renderer plate text draw (Arial 38/10, outline passes) | subsystems/vehicles.md, "Number plates" |
| `Renderer::editor_handle` | — | 0x10072570 | object editor: one handle (background glyph, handle glyph, hit rect) | subsystems/world-entities.md, "Object editor and selection UI" |
| `Renderer::editor_gizmo` | — | 0x10072C60 | object editor gizmo (axis lines, handles, buttons) | subsystems/world-entities.md, "Object editor and selection UI" |
| `Renderer::select_highlight` | — | 0x1006DA20 | object selection highlight | subsystems/world-entities.md, "Object editor and selection UI" |
| `Renderer::select_highlight`, `Renderer::camera_target_labels` | — | 0x10071D50 | editor axis cross (length 1, one colour) | subsystems/world-entities.md, "Object editor and selection UI" |
| `Renderer::camera_target_labels` | — | 0x10004830 | `/ctd` camera-target labels | subsystems/session-misc.md, "168 OnPlayerCameraTarget"; docs/phase3/feata-notes.md |
| `Renderer::death_window` | 0x1006A2E0 | 0x1006A2A0 | death window draw | subsystems/ui-rendering.md, "Data and geometry" |
| `Renderer::death_name` | 0x10069C40 | 0x10069C00 | death window name with the 4-way outline | subsystems/ui-rendering.md, "Outline (\"shadow\") patterns" |
| `Renderer::death_icon` | 0x10069D40 | 0x10069D00 | death window icon box | subsystems/ui-rendering.md, "Address table (A / B)" |
