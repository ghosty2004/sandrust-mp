# samp-tracer address table

The tracer (`crates/samp-tracer`) observes a SA:MP client from outside. It reads samp.dll data only where a
cross-client query needs the original's own state; everything else it watches is gta_sa.exe (hook table in
`hooks.rs`, named constants in `gta.rs` / `watch.rs`). A = 2021 build, B = 2019 build (fv0.3.7.3, the build
of the diff lanes). "—" = not mapped by the tracer.

## Chat bubbles and synced health

Used by the `bubbles` and `synced` control queries (`src/sampstate.rs`).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `B_BUBBLE_POOL` | — | 0x1026E9F0 | pointer to the chat-bubble pool: 1004 slots of 0x118 bytes | ui-chat-audio.md, "59 SetPlayerChatBubble" |
| (source of the bubble layout) | — | 0x1000F960 | RPC 59 handler; its reads give the wire order colour, distance, expire time | ui-chat-audio.md, "59 SetPlayerChatBubble" |
| (source of the bubble layout) | — | 0x10066DD0 | bubble add function; its stores give the slot field order | ui-chat-audio.md, "59 SetPlayerChatBubble" |
| `bubble::ACTIVE` … `bubble::LINES` | — | slot +0 / +4 / +0x104 / +0x108 / +0x10C / +0x110 / +0x114 | active (u32), text (wrapped, NUL-terminated, 256 bytes), start tick, duration (ms), colour (ARGB), distance (f32), line count | ui-chat-audio.md, "59 SetPlayerChatBubble" |
| `B_NETGAME` | — | 0x1026EA0C | netgame pointer; pools at netgame+0x3DE, player pool at pools+8 | player-state.md, "PlayerPool" |
| `pool::SLOT_USED`, `pool::ENTRY`, `pool::ENTRY_REMOTE` | — | 0x100010B0 / 0x100010F0 | player-pool accessors: slot used = u32 at pool+0xFDE+4·id; entry at [pool+0x2E+4·id]; remote player at [entry+0x10] | player-state.md, "PlayerPool" |
| `pool::REMOTE_HEALTH`, `pool::REMOTE_ARMOUR` | — | 0x1001725D / 0x10017390 | on-foot sync (207) stores of synced health (remote+0x1B0) and armour (remote+0x1AC), f32 | ui-rendering.md, "16.2 Bars" |
| (spawn default) | — | 0x100171B4 | spawn sets synced health to 100 | ui-rendering.md, "16.2 Bars" |

## Patch tables read by the hook-table tests

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `hooks.rs` test of model-pointer patch sites | .data 0x10114B10 | — | 707 records {u32 code address, u8 opcode} that samp patches in gta_sa.exe | gta-patches.md, "4.4 Model-info pointer array" |
