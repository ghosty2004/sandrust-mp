# ui/mattext.rs — object material text

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `TextSpec` (record) | | 0x100A86C0 | material-text record: fresh 2x2 RW texture (`RwRasterCreate(2,2,0,0)` + `RwTextureCreate`) as slot texture, colour cleared | world-entities.md, "84 SetPlayerObjectMaterial" |
| `ensure` (object pass) | | 0x100A9EF0 | per-object pass from samp's CGame::Process wrapper (wrapper B 0x100A2670) | world-entities.md, "RE-10 additions" |
| `render`, `SPRITE` | | 0x100708E0 | sign-text renderer (owns one sprite at renderer +4; "SignText: Can't create font" debug line) | world-entities.md, "RE-10 additions"; docs/phase3/m6-notes.md |
| `material_size` | 0x100A8CC0 | 0x100A8C90 | material size code → (w, h) table | world-entities.md, "84 SetPlayerObjectMaterial" |
| `target_format` | | 0x1007090B | A8R8G8B8 for background 0, else X8R8G8B8 | docs/phase3/m6-notes.md |
| `draw_format` | | 0x100709B7 | DrawText format per alignment (0x100 / 0x105 / 0x102) | docs/phase3/m6-notes.md |
| `D3DRS_DESTBLEND` | | 0x10070A7C | SetRenderState(DESTBLEND, BLENDFACTOR) after sprite.Begin for background 0 | docs/phase3/m6-notes.md |
| `on_lost` | | 0x100C3F60 | lost-device handler → object pool loop 0x10012B20 → per-object release 0x100A8EE0 | ui-rendering.md, "2.2 On lost / reset device" |
| `on_lost` (per object) | | 0x100A8EE0 | releases one object's material render targets | — |
| device proxy `SetTexture` (in ui/d3d.rs) | 0x10074BB0 | | swaps the 2x2 slot texture for the render target | world-entities.md, "RE-10 additions" |
