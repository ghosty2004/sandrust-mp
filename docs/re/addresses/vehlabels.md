# vehlabels.rs — samp.dll addresses

Spec: docs/re/subsystems/ui-rendering.md, "2.3 Present: per-frame draw order"; measurements in
docs/phase3/featb-notes.md. Addresses were read in build B unless an A column entry is given.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `draws` (UI pass) | 0x10075790 | 0x10075760 | per-frame D3D UI pass (proxy `Present`) that draws the label batch after the audio title, before scoreboard / F-key screens | ui-rendering 2.3 |
| `crate::ui::vehicle_labels_on` | | 0x1026EA30 | `/dl` toggle flag | ui-rendering 2.3 |
| `tick` (spectating) | | 0x1009F0F0 | distance reference: camera while spectating | ui-rendering 2.3 |
| `LABEL_ARGB` | | 0x10075910 | label colour 0xFF358BD4 | ui-rendering 2.3 |
| `LABEL_RANGE` | | 0x100E5804 | 20.0 m range float | ui-rendering 2.3 |
| `subtype` | 0x100B6220 | 0x100B61F0 | vehicle subtype from the GTA vtable | ui-rendering 2.3 |
| `text` | | 0x100EA7D8 | label format string | ui-rendering 2.3 |
| `preload` | | 0x100A1620 (called from 0x10010992 in InitGame) | release loaded vehicle models, preload the 80 most used | featb-notes |
| `PRELOADED` | — | — | samp's game object +0x6E… per-model preload flags | featb-notes |
| `tick` (health / seats) | — | — | `CVehicle` +0x4C0 health, +0x488 max passengers | ui-rendering 2.3 |
