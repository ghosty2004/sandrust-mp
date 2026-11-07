# samp.dll addresses: `crates/samp-client/src/ui/d3d.rs`

A = 2021 build, B = 2019 build; "—" = not looked up for that build. Spec paths are under `docs/re/`.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `install_device_proxy` | 0x100ED8F0 | 0x100ED908 | samp's proxy device vtable | subsystems/ui-rendering.md, "Device proxy" |
| `install_device_proxy` (Present override) | 0x10075790 | 0x10075760 | proxy `Present`: the per-frame UI pass | subsystems/ui-rendering.md, "Present: per-frame draw order" |
| `install_device_proxy` (Reset override) | 0x10074630 | 0x10074600 | proxy `Reset` | subsystems/ui-rendering.md, "Device proxy" |
| `proxy_end_scene` | 0x10074890 | 0x10074860 | proxy `EndScene` (two integrity checks around the real call) | subsystems/ui-rendering.md, "Device proxy" |
| `SET_TEXTURE_FILTER`, `proxy_set_texture` | 0x10074BB0 | — | proxy `SetTexture`: material text and plate texture substitution | subsystems/world-entities.md, "Materials (RPC 44/84) at render time"; subsystems/vehicles.md, "Number plates" |
| `plate_texture` | — | 0x10074BC3 | proxy `SetTexture`: number-plate substitution step | subsystems/vehicles.md, "Number plates" |
| `set_texture_stub_code` | — | 0x10074C3E, 0x10074C9x | proxy `SetTexture`: tail jumps into the real `SetTexture` (GTA stays the caller) | subsystems/vehicles.md, "Number plates" |
| `proxy_set_transform` | 0x100748E0 | 0x100748B0 | proxy `SetTransform` (slot 44): keeps copies of VIEW / PROJECTION / WORLD | subsystems/ui-rendering.md, "Chat bubbles" |
| `VIEW` | 0x10140BE8 | — | samp's copy of the last VIEW matrix | subsystems/ui-rendering.md, "Chat bubbles" |
| `PROJECTION` | 0x10140C28 | — | samp's copy of the last PROJECTION matrix | subsystems/ui-rendering.md, "Chat bubbles" |
| `WORLD` | 0x10140C68 | 0x10140C68 | samp's copy of the last WORLD matrix (state 256) | subsystems/world-entities.md, "Object editor and selection UI" |
| `Line` | — | 0x10071B90 | object editor constructor: `D3DXCreateLine` before its two fonts | subsystems/world-entities.md, "Object editor and selection UI" |
| `add_font_resources` | 0x100C5100 | — | `AddFontResourceA` of gtaweap3.ttf and sampaux3.ttf | subsystems/ui-rendering.md, "Font assets" |
| `screenshot` | 0x10075620 | 0x100755F0 | F8 screenshot | subsystems/ui-rendering.md, "Screenshot" |
