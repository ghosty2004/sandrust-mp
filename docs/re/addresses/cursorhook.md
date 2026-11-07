# cursorhook.rs — samp.dll addresses

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `show_cursor_stub` | 0x10062960 | 0x10062920 | `ShowCursor` replacement: counter untouched, 0 for show, −1 for hide | ui-rendering.md §5 |
| `install` | 0x10062970 | 0x10062930 | start-up routine that inline-hooks `user32!ShowCursor` | ui-rendering.md §5 |
| `install` (caller) | 0x100c5221 | 0x100c5201 | call site of the hook installer during start-up | ui-rendering.md §5 |
| (`ui::glue` object editor) | — | 0x10073610, 0x10073684 | object editor calling the saved original `ShowCursor` (capture / release) | ui-rendering.md §5 |
| (GTA, not samp.dll) | gta_sa.exe 0x747FE9, 0x7481CD, 0x748EAF | | GTA's own `ShowCursor(FALSE)` calls: activation, `WM_SETCURSOR`, shutdown | ui-rendering.md §5 |
