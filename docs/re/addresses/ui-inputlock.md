# ui/inputlock.rs — samp.dll addresses

Byte values were read from the setter / restore bodies of build A. GTA sites are gta_sa.exe 1.0 US and are named
constants in the code; the full patch rows are in docs/re/data/gta-patches-r4.csv.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `InputLock::set_mode` | 0x100a0750 | 0x100a0720 | cursor / input mode setter `set(mode, flag)` | ui-rendering.md §5 |
| `InputLock::frame` | 0x100a0630 | 0x100a0600 | per-frame restore of GTA's input code after the countdown | ui-rendering.md §5 |
| `InputLock::frame` (operand rewrite) | 0x100a0590 | | restore part that points the three mouse store operands back at 0xB73410 / 0xB73414 / 0xB7340C | gta-patches.md §5 |
| `InputLock::request_mode4` | 0x100a0540 | | mode 4 mouse redirect | ui-rendering.md §5; gta-patches-r4.csv (0x53f47a,…) |
| `Op::MouseSink` | 0x101501a8 | | samp's dummy mouse store targeted by the mode 4 redirect | gta-patches-r4.csv (0x53f47a,…) |
| `MOUSE_STATE_CALL_ORIGINAL` | 0x1026e9a0 | | detected exe version flag (1 = 1.0 US → `E8 B4 7A 20 00`; 2 = EU → `E8 04 7B 20 00`) | gta-patches.md §2 |
| `InputLock` (mode, countdown) | 0x1026ea24 +0x61 / +0x65 | | game object fields holding the cursor mode and restore countdown | ui-rendering.md §5 |
| `restore_now` | 0x10072480 | | object editor start: mode (0, ·), countdown forced to 0, restore run at once | world-entities.md (object editor) |
| (caller, mode 0 with flag 1) | 0x10072c20 | | object editor stop, passes (0, 1) | world-entities.md (object editor) |
| (callers, mode 3 / mode 0 flag 1) | 0x1006dc00, 0x100737e0 | | object selection / object editor per-frame routines | world-entities.md; ui-rendering.md §1 |

Callers in R4 (every `call 0x100a0750` in both builds) pass only modes 0, 2 and 3; modes 1 and 4 have no caller.

| GTA site (1.0 US) | locked | original | what |
|---|---|---|---|
| 0x541DF5 | 5 × NOP | `E8 46 F3 FE FF` | keyboard → pad call |
| 0x53F417 | 5 × NOP | `E8 B4 7A 20 00` | mouse-state call |
| 0x53F41F | `33 C0 0F 84` | `85 C0 0F 8C` | branch after it |
| 0x6194A0 | `C3` | `E9` | `RsMouseSetPos` |
| 0x53F47A / 0x53F49A / 0x53F4B3 | → samp dummy | 0xB73410 / 0xB73414 / 0xB7340C | mouse store operands (mode 4 only) |
