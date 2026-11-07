# ui/preview.rs — model-preview textdraws (font 5)

Spec: docs/re/subsystems/ui-rendering.md, "14.3a Model previews (font 5)" (abbreviated "UR 14.3a" below).
A = 2021 build, B = 2019 build; "—" means not recorded for that build.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| (caller of `init_env`) | 0x100c47d0 | — | first-frame UI construction | UR 1.1, 14.3a |
| (caller of `init_env`) | 0x1006ccc0 | — | preview set-up step of the first-frame pass | UR 14.3a |
| `init_env`, `Env` | 0x1006be40 | — | builds the preview camera set (+0 camera, +4 frame, +8 light, +0xc texture, +0x10 raster, +0x14 Z raster) | UR 14.3a |
| (caller of `pending`) | 0x100a27e6 | — | call after `CGame::Process` in the 0x53E981 game-frame hook | UR 14.3a |
| `pending` / `render_pending` | 0x1001e870 | — | per-game-frame pass over preview textdraws without a texture | UR 14.3a |
| `render` | 0x100b34e0 | — | per-textdraw renderer choice and model substitutions | UR 14.3a |
| `render_ped` | 0x1006c140 | — | ped preview | UR 14.3a |
| `TALL_SKIN` | 0x1006c1d7 | — | skin 162 drawn at z 50.15 instead of 50.05 | UR 14.3a |
| `begin` | 0x1006c2c6..0x1006c325 | — | render states ZTEST, ZWRITE, SHADEMODE, FOG of a preview render | UR 14.3a |
| `render_vehicle` | 0x1006c3c0 | — | vehicle preview | UR 14.3a |
| `render_object` | 0x1006c9b0 | — | object / weapon preview | UR 14.3a |
| `FALLBACK_OBJECT` | 0x100b3592, 0x100b365c, 0x1006c709 | — | uses of the unknown-model object 18631 | UR 14.3a |
| `render` (negative ids) | 0x100b45b0 | — | custom-model lookup; R4 has no custom-model table | UR 14.3a |
| (slot assignment in `render_pending`) | 0x100b2b60 | — | puts a new texture into the slot table | UR 14.3a |
| `TEXTURE_SLOTS` | 0x1026b3e0, 0x1026b700 | — | 200-entry texture slot table shared with sprite textdraws | UR 14.3a |
| (textdraw draw, in textdraws.rs) | 0x100b34c0 | — | per-textdraw draw: sprite path with a texture, else text path | UR 14.3a |
| (sprite path, in textdraws.rs) | 0x100b30d0 | — | sprite / model draw path | UR 14.3 |
| `sync` | 0x100b2f90 | — | textdraw destructor, frees its texture | UR 14.3a |
| `sync` | 0x100b2b90 | — | slot release | UR 14.3a |
| `destroy_texture` | 0x100b39a0 | — | `CSprite2d` delete wrapper (`RwTextureDestroy`) | UR 14.3a |
| (caller of `on_lost`) | 0x100c41e5 | — | call in the lost-device pass | UR 2.2, 14.3a |
| `on_lost` | 0x1001e8a0 | — | releases every preview texture on device loss | UR 14.3a |
| `on_lost` | 0x100b36f0 | — | per-textdraw texture release on device loss | UR 14.3a |
| `is_ped_model` | 0x100b3e10 | — | ped-model test (model-info vtable 0x85BDC0) | UR 14.3a |
| `render_state` | 0x100c3e60 | — | `RwRenderStateSet` through `RwEngineInstance` | UR 14.3a |
| `begin` (FOG) | 0x100c3dc0 | — | samp's render-state hook (own D3D fog setter) | UR 14.3a |
| (vehicle fog flag, in worldhooks.rs) | 0x1026ea58 | — | flag set while a vehicle preview renders | UR 14.3a |
| `set_camera_rasters` | 0x100b19f0 | — | writes camera +0x60 / +0x64 | UR 14.3a |
| `set_position` | 0x1009f0a0 | — | entity wrapper: position via `Teleport` | UR 14.3a |
| `preview_flags` | 0x1009f6e0, 0x1009f690 | — | entity flag setters (called with 0) | UR 14.3a |
| `rotate_entity` | 0x1009eb70 | — | copy of the entity matrix into an RwMatrix | UR 14.3a |
| `rotate_entity` | 0x1009fd60 | — | matrix write-back (Remove, rows, UpdateRW, UpdateRwFrame, Add) | UR 14.3a |
| `add_to_world` | 0x1009f360 | — | entity wrapper "add" | UR 14.3a |
| `render_entity` | 0x1009fb80 | — | entity render (SetupVehicleVariables, PreRender, RenderOneNonRoad) | UR 14.3a |
| `release_model` | 0x100a0a00 | — | marks a model no longer needed (called with 0) | UR 14.3a |
| `model_refs` | 0x100b4740 | — | model-info reference count read | UR 14.3a |
| `col_sphere` | 0x100b6450, 0x100b63a0 | — | collision sphere centre / radius | UR 14.3a |
