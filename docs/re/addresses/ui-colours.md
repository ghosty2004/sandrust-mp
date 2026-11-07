# ui/colours.rs — player colour table

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `TABLE_LEN`, `PlayerColours` | 0x101516A0 | | global u32 RGBA colour per player id, 4000 entries | player-state.md, "72 SetPlayerColor" |
| `DEFAULT_COLOURS` | 0x10113B48 | | static 1000 × u32 table in .data; repeats the 100 values with period 100 (both R4 builds) | vehicles.md (radar trace colour) |
| `PlayerColours::new` | 0x100A72C0 | | start-up copy of the default table into the first 1000 entries | session-misc.md |
| `PlayerColours::set` | 0x100A7250 | | setter, ignores `id >= 4000` | player-state.md, "72 SetPlayerColor" |
| `PlayerColours::rgba` | 0x100A7270 | | getter; ids 0x3EC/0x3ED/0x3EE return fixed colours (not used by chat) | ui-chat-audio.md, "101 PlayerChatMessage"; vehicles.md |
