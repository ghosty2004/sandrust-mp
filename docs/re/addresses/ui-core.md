# ui/core.rs — platform-independent UI state

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `msg` (all strings), `UiCore::print_debug` | 0x100680F0 | | type-8 chat print helper (debug colour); every `msg` string is printed through it | ui-rendering.md, "10. Client messages and player chat on screen" |
| `msg::STARTED` | 0x100E596C | | string "SA-MP 0.3.7-R4 Started"; printed at 0x1000BAFE when the netgame object is created | ui-chat-audio.md, "Overview" |
| `msg::connecting` | 0x100E599C | | string "Connecting to %s:%d..."; printed at 0x1000896D (connect tick) | handshake.md, §1 |
| `msg::JOINING` | 0x100E5B98 | | string "Connected. Joining the game..."; printed at 0x1000AE70 on packet 0x22 | handshake.md, §5 |
| `msg::connected_to` | 0x100E6060 | | string "Connected to {B9C9BF}%.64s"; printed at 0x10010A87 on InitGame | session-misc.md, "139 PlayerInit" |
| `msg::SERVER_CLOSED` | 0x100E5A80 | | string "Server closed the connection." | session-misc.md, "Connection / session control" |
| `SUBMIT_SCROLL_POS`, `UiCore::submit` | 0x10067390 | | Enter in the chat input: scroll position 90 | ui-rendering.md, "7.4 Scrollbar and paging" |
| `NetStats`, `net_stats_text` | 0x10060D70 | | F5 network-statistics overlay text (labels and order measured on the original) | ui-rendering.md, "18. Smaller overlays" |
| `UiCore::audio_proxy_key` | 0x10066480 | | after `BASS_Init`: create / read `audioproxyoff` | ui-chat-audio.md, "41 PlayAudioStreamForPlayer" |
| `UiCore::print_info` | 0x10068070 | | type-4 chat print helper (info colour) | ui-rendering.md, "10. Client messages and player chat on screen" |
| `UiCore::open_help` | 0x1006BB30 | | F1: local help dialog | ui-rendering.md, "13.6 Local dialogs" |
| `UiCore::open_help` (show path) | 0x10070010 | | dialog show path (closes the chat input first) | ui-rendering.md, "13.2 Geometry per style" |
| `UiCore::screenshot_target` | 0x10062270 | | F8 file name loop `sa-mp-%03i.png` | ui-rendering.md, "6.4 Screenshot" |
| `UiCore::screenshot_done` | 0x10075620 | | F8 screenshot and result line | ui-rendering.md, "6.4 Screenshot" |
| `UiCore::on_init_game` (LOS) | | 0x10010778 | InitGame stores 1 into settings +0x2F after reading the LOS bit; nametag LOS (opcode 0x06BD) always runs | docs/phase3/m6p-notes.md |
| `UiCore::on_reset` | 0x1000A540 | 0x1000A530 | RPC 40 reset (handler A 0x1000E650 / B 0x1000E620) | session-misc.md, "40 PlayerClose" |
| `UiCore::effect` (`/hudscalefix`) | 0x10117470 | | HUD aspect-fix flag (toggled by the command at A 0x10068870) | ui-chat-audio.md, "Local chat input and built-in commands"; remote-control-combat.md |
| `UiCore::effect` (`/testdw`) | 0x10068500 | 0x100684C0 | eight fixed death-window rows | ui-chat-audio.md, "Local chat input and built-in commands" |
| `UiCore::effect` (`/rs` in a car) | | | `rawvehicles.txt` written relative to the current directory (B string reference, measured) | ui-chat-audio.md, "Local chat input and built-in commands" |
| `UiCore::scoreboard_activate` | 0x1006EC10 | 0x1006EBD0 | scoreboard row activated → RPC 23 | session-misc.md, "23 OnPlayerClickPlayer" |
| `UiCore::scoreboard_activate` (close) | 0x100A0750 | 0x100A0720 | cursor / input mode setter, called `(0, 0)` (delayed input restore) | ui-rendering.md, "5. Cursor and input-lock modes" |
