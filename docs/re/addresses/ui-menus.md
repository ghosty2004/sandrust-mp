# ui/menus.rs — GTA menus and the CText::Get hook

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `build` | 0x100A7780 | | builds the GTA menu (opcodes 0x08D4 / 0x08DB / 0x09DB / 0x08D9) | ui-chat-audio.md, "77 PlayerShowMenu" |
| `pad_buttons` | 0x10008200 | 0x100081F0 | per-frame menu poll (sends RPC 132 / 140) | ui-chat-audio.md, "132 OnPlayerSelectedMenuRow (C→S) and 140 OnPlayerExitedMenu (C→S)" |
| `install_gxt_hook` | 0x100A4070 | 0x100A4040 | `CText::Get` handler; installed at 0x100A69DB (A) / 0x100A69AB (B) as `jmp [0x6A0043]` | gta-patches.md, row 0x6a0050 (docs/re/data/gta-patches-r4.csv line 58) |
