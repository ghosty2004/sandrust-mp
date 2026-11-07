# ui/textdraws.rs — textdraw drawing

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `print` | 0x100B31E0 | 0x100B31B0 | text path: CFont calls through thin thunks (SetScale 0x719380 … PrintString 0x71A700, all checked) | ui-rendering.md, "14.2 Text path" |
| `draw_sprite` | 0x100B30D0 | | sprite / model path: textured quad + hit rect | ui-rendering.md, "14.3 Sprite / model path" |
| `load_sprite` | 0x100B2D10 | | `txd:name` lookup; also the `mdlNNN` special case (only with the custom-model store) | ui-rendering.md, "14.3 Sprite / model path" |
| `load_sprite` (texture) | 0x100B2CA0 | | texture fetch inside Push/Set/PopCurrentTxd | ui-rendering.md, "14.3 Sprite / model path" |
| `ensure_txd` | 0x100B2BD0 | | "ensure loaded": add + load + AddRef | ui-rendering.md, "14.3 Sprite / model path" |
| `load_sprite` (`mdlNNN`) | 0x1026EA10 | | custom-model store global, created only by the unused RPC 179 handler | session-misc.md; world-entities.md |
| `free_sprite` (CSprite2d dtor) | 0x100B39A0 | | samp's temporary CSprite2d release | ui-rendering.md, "14.3 Sprite / model path" |
| `draw_all` (font 5 dispatch) | 0x100B34C0 | | per-textdraw dispatch: preview texture → sprite path, else text path | ui-rendering.md, "14.3a Model previews (font 5)" |
