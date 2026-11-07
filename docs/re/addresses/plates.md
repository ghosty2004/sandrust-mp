# plates.rs — samp.dll addresses

Spec: docs/re/subsystems/vehicles.md, "Number plates"; measurements in docs/phase3/m6p-notes.md, "Number plates".
Patch rows refer to docs/re/data/gta-patches-r4.csv. Addresses were read in build B; A is not recorded here.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `create_plate_raster` (row 59) | | 0x100A4530 | replaces the `RwRasterCreate` call in `CCustomCarPlateMgr::CreatePlateTexture` (gta 0x6FDED6): one shared 64×16 raster | vehicles "Number plates" |
| `PLATE_RASTER` | | 0x100A4504 | the shared raster's size / depth / flags arguments | vehicles "Number plates" |
| `raster_destroy_stub` (row 60) | | 0x100A4440 | `RwRasterDestroy` hook (`jmp [0x59C721]`) that keeps the shared raster | vehicles "Number plates" |
| `GUARD` | | 0x10113AD8 | keep-raster guard, cleared during a device loss | vehicles "Number plates" |
| `RASTER_D3D_TEXTURE` | | 0x100A4480 | reads the D3D texture at raster + 0x34 | vehicles "Number plates" |
| `DEFAULT_TEXT` | | 0x100E645C | default plate text `XYZSR998` | vehicles "Number plates" |
| `tick` | | 0x1001ED20 | per-frame pool pass that paints plates (only while pool +0x17894 is set) | remote-control-vehicles "A. CGame::Process pre-step" |
| `tick` (per vehicle) | | 0x100B81B0 | per-vehicle plate creation, calls the painter | vehicles "Number plates" |
| `tick` `paint` argument | | 0x1006D840 | plate painter (D3D texture + text) | vehicles "Number plates" |
| `State::device_ok` | — | — | vehicle pool +0x17894 device flag | vehicles "Number plates" |
| `on_lost` | | 0x100C4171 | device-loss path: guard and pool flag off | vehicles "Number plates" |
| `on_lost` | | 0x1001ED90 | releases every vehicle's plate texture | vehicles "Number plates" |
| `on_lost` | | 0x1006D770 | painter releases its own objects (default plate) | vehicles "Number plates" |
| `on_reset` | | 0x100C440D | after reset: guard and pool flag on | vehicles "Number plates" |
| `substitute` | | 0x10074B80 (substitution at 0x10074BC3) | device proxy `SetTexture` | ui-rendering |
| `substitute` (vehicle being rendered) | — | — | recorded by the `CVehicle::Render` hook (patch row 62, `vehhooks.rs`) | vehicles "Number plates" |
