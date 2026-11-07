# ui/wndproc.rs — window-procedure subclass

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `install` (class style, title) | | 0x10061EB0 | window setup: subclass, CS_DBLCLKS, title "GTA:SA:MP" | ui-rendering.md, "6.1 Install"; docs/phase3/feature-sweep.md A13 |
| `WINDOW_TITLE` | | 0x10061EF4 | reference to the "GTA:SA:MP" title string (string data at 0x100E98E4) | docs/phase3/data/samp-strings-sweep.tsv |
| `translate` (editor first) | | 0x1006186A | window procedure checks object editor / selection before the chat input | ui-rendering.md, "6.2 Routing order" |
| `on_lost_device` | 0x100C3F90 | | lost-device handler | ui-rendering.md, "2.2 On lost / reset device" |
| `on_lost_device` (flag) | 0x100C419B | | store that re-arms the "SetCursor(NULL) once" flag | ui-rendering.md, "5. Cursor and input-lock modes" |
| `on_reset_device` | 0x100C41F0 | 0x100C41C0 | reset-device handler, first call `SetCursor(NULL)` | ui-rendering.md, "2.2 On lost / reset device" |
