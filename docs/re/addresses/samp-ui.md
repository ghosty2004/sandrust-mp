# samp-ui: samp.dll addresses

Every samp.dll address the `crates/samp-ui` comments cite or used to cite. The code keeps at most one per
item (`Mirrors samp.dll 0x….`); the rest live here. A = 2021 build, B = 2019 build; "—" = not recorded.
Spec paths are relative to `docs/re/subsystems/` (UR = `ui-rendering.md`, UCA = `ui-chat-audio.md`).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `colour::parse_code` | 0x100b5fd0 | 0x100b5fa0 | `{RRGGBB}` code parser | UR "Embedded colour codes" |
| `colour::strip_codes` | 0x100b6100 | 0x100b60d0 | colour-code strip helper | UR "Embedded colour codes" |
| `fonts::main_height` | 0x100c52d0 | 0x100c52b0 | main font height | UR "Size and face settings" |
| `fonts::aux_height` | 0x100c5320 | 0x100c5300 | aux (death-window names) font height | UR "Size and face settings" |
| `fonts::gui_height` | 0x100c5360 | 0x100c5340 | GUI font height | UR "Size and face settings" |
| `fonts::weight` | 0x100c5380 | 0x100c5360 | font weight from `fontweight` | UR "Size and face settings" |
| `fonts::face`, `fonts::DEFAULT_FACE` | 0x100c53b0 | 0x100c5390 | font face from `fontface`, default `"Arial"` | UR "Size and face settings" |
| `text::replace_control_chars` | 0x10067420 | — | control characters → spaces | UCA "Shared: chat window model" |
| `text::expand_key_names`, `text::KEY_EXPANSION_MAX_OUTPUT` | 0x100b6140 | — | `~k~` expansion filter (`len <= limit` keeps the result) | UCA "Shared: chat window model" |
| `text::KEY_EXPANSION_MAX_OUTPUT` | 0x10068190 | — | chat caller of the expansion filter, passes limit 255 | UCA "93 SendClientMessage" |
| `text::LABEL_KEY_EXPANSION_MAX_OUTPUT` | 0x100011d0 | — | RPC 36 3D-label store, passes limit 399 | world-entities.md "36" |
| `chat::ChatWindow` | 0x1026e9f8 | — | chat window global | UR "Chat window" |
| `chat::ChatEntry::chatlog_line` | 0x100677d0 | — | chatlog line writer | UCA "Shared: chat window model" |
| `chat::ChatWindow::add_entry` | 0x10068020 | — | add entry (and neighbours) | UCA "Shared: chat window model" |
| `chat::ChatWindow::add_info`, `commands::info` | 0x10068070 | — | type-4 printf-style line helper | UR "State" (chat) |
| `chat::ChatWindow::add_debug`, `commands::debug` | 0x100680f0 | — | type-8 debug line helper | UR "State" (chat) |
| `chat::ChatWindow::cycle_mode` | 0x100612c0 | 0x10061280 | F7 display-mode cycle | UR "Display modes" |
| `chat::ChatWindow::set_page_size`, `chat::PAGE_SIZE_SETTER_RANGE` | 0x100672a0 | 0x10067260 | page-size setter | UR "Scrollbar and paging" |
| `chat::ChatWindow::page_up` | 0x100672d0 | 0x10067290 | PageUp | UR "Scrollbar and paging" |
| `chat::ChatWindow::page_down` | 0x10067330 | 0x100672f0 | PageDown | UR "Scrollbar and paging" |
| `chat::ChatWindow::scroll_to` | 0x10067390 | — | Enter in the chat input scrolls to 90 | UR "Scrollbar and paging" |
| `chat::ChatWindow::wheel` | 0x100673c0 | 0x10067380 | mouse wheel scroll | UR "Scrollbar and paging" |
| `chat::ChatWindow::scrollbar_rect` | 0x10067200 | 0x100671c0 | scrollbar layout | UR "Scrollbar and paging" |
| `chat::ChatWindow::bottom`, `chat::ChatWindow::layout` | 0x10067940 | 0x10067900 | chat entries draw loop | UR "Layout" (chat) |
| `chat::ChatWindow::layout` | 0x10067470 | 0x10067430 | chat line draw | UR "Layout" (chat) |
| `bubbles::WRAP_WIDTH` | 0x10017690 | — | text wrap helper (36, 12) | UCA "59 SetPlayerChatBubble" |
| `bubbles::BubbleTarget::distance` | 0x1009f120 | — | distance from the local player | UR "Chat bubbles" |
| `bubbles::ChatBubbles` | 0x1026e9f0 | — | bubble pool global | UR "Chat bubbles" |
| `commands::LocalCommand::parse` | 0x10069710 | — | case-insensitive command lookup | UCA "Local chat input and built-in commands" |
| `commands::msg` | 0x100e9e00 .. 0x100ea2ec | — | string table of the command feedback texts (approximate range) | UCA "Local chat input and built-in commands" |
| `commands::save_line` | 0x100e9f78 / 0x100e9fcc | — | `AddStaticVehicle(...)` / `AddPlayerClass(...)` format strings (approximate) | UR "Local commands — UI effects and persistence" |
| `config::SampCfg::to_bytes` | 0x10066080 | 0x10066040 | `sa-mp.cfg` writer | docs/phase3/trace-notes.md "sa-mp.cfg" |
| `config::SampCfg::to_bytes` | 0x100e9b88 | — | writer format strings (approximate) | docs/phase3/trace-notes.md "sa-mp.cfg" |
| `config::SampCfg::parse` | — | 0x10066230 | `sa-mp.cfg` loader | docs/phase3/m10-notes.md "sa-mp.cfg" |
| `config::CfgValue` (value classification) | — | 0x10065f20 | value-type classifier | docs/phase3/m10-notes.md "sa-mp.cfg" |
| `config::SampCfg::get` | — | 0x10065c50 | key lookup (lower-cases the name) | docs/phase3/m10-notes.md "sa-mp.cfg" |
| `config::SampCfg::apply_startup_defaults` | 0x100c4e4c | — | start-up key creation in the first-frame routine | UR "Local commands — UI effects and persistence" |
| `config::SampCfg::ensure_audio_proxy_key` | 0x10066480 | — | audio set-up after `BASS_Init` | UCA "41 PlayAudioStreamForPlayer" |
| `config::SampCfg::ensure_audio_proxy_key` | 0x100664f2 | — | `audioproxyoff` creation inside the audio set-up | UCA "41 PlayAudioStreamForPlayer" |
| `config::UiSettings::object_light_flag` | 0x101166c0 | — | `/togobjlight` flag global | UCA "Local chat input and built-in commands" |
| `deathwindow::glyph` | 0x10069e50 | 0x10069e10 | reason → GTAWEAPON3 glyph | UR "Reason → GTAWEAPON3 glyph" |
| `deathwindow::DeathWindow` | 0x1026ea00 | — | death window global | UR "Death window" |
| `deathwindow::DeathWindow::add` | 0x1006a5d0 | 0x1006a590 | add row | UR "Death window" |
| `dialog::HELP_BODY` | 0x100ea470 | — | F1 help body string | UR "Local dialogs" |
| `dialog::Dialog::layout` | 0x1006f500 | 0x1006f4d0 | dialog layout per style | UR "Geometry per style" |
| `dialog::Dialogs` | 0x1026e9c8 | — | dialog object global | UR "Dialogs" |
| `dialog::Dialogs::respond` | 0x10070690 | — | RPC 62 sender / answer | UCA "62 OnPlayerDialogResponse" |
| `dialog::Dialogs::respond` | 0x1006f860 | 0x1006f830 | dialog close (cursor mode 0, 10-frame restore) | UR "Close" (dialogs) |
| `input::ChatInput` | 0x1026e9fc | — | chat input global | UR "Chat input" |
| `input::ChatInput::open` | 0x10069480 | 0x10069440 | open | UR "Chat input" |
| `input::ChatInput::close` | 0x10069580 | 0x10069540 | close | UR "Chat input" |
| `input::ChatInput::history_older` | 0x10069630 | 0x100695f0 | history up | UR "Chat input" |
| `input::ChatInput::history_newer` | 0x100696a0 | 0x10069660 | history down | UR "Chat input" |
| `input::ChatInput::submit` | 0x100699d0 | — | input processor (Enter) | UR "Chat input" |
| `keys::key_up` (Enter) | 0x10089380 | — | edit box raises its "string" event on the Enter key-up | UR "Key and mouse table" |
| `menu::CELL_KEYS` | 0x100fe0c8 | — | `SAMP0rr` / `SAMP1rr` key table | UR "Corrections from M8a" (menus) |
| `menu::Menu` | 0x100a7460 | — | menu object | UR "Game text and menus" |
| `menu::Menu::build` | 0x100a7780 | — | GTA menu build (script commands) | UR "Corrections from M8a" (menus) |
| `menu::Menu::gxt` | 0x100a4070 | — | `CText::Get` hook, forwards `SAMP…` keys | UR "Corrections from M8a" (menus) |
| `menu::Menu::gxt` | 0x10008060 | — | `SAMP…` key → menu text | UR "Corrections from M8a" (menus) |
| `menu::Menus::poll` | 0x10008200 | 0x100081f0 | per-frame menu poll (RPC 132/140) | UCA "132 OnPlayerSelectedMenuRow …" |
| `nametags::bars` | 0x1006d0a0 | 0x1006d060 | nametag bar draw | UR "Bars" (nametags) |
| `nametags::RAISE_PER_METRE`, `nametags::RAISE_BASE` | 0x100ea648 / 0x100e9c90 | — | anchor raise constants 0.0475 / 0.2 | UR "Bars" (nametags) |
| `scoreboard::REFRESH_INTERVAL_MS` | 0x10008f10 | 0x10008f00 | RPC 155 sender | UR "Behaviour" (scoreboard) |
| `scoreboard::Scoreboard` | 0x1026e9c4 | — | scoreboard global | UR "Scoreboard" |
| `scoreboard::Scoreboard::rebuild` | 0x1006ed30 | 0x1006ecf0 | rows rebuild | UR "Rows" (scoreboard) |
| `textdraw::set_text` | 0x100b2fa0 | — | textdraw set-text rules | UR "Corrections from M8a" (textdraw set-text) |
| `textdraw::FontParams` | 0x100b31e0 | 0x100b31b0 | textdraw text path | UR "Text path" |
| `textdraw::hit_rect` | 0x100b33f5 .. 0x100b34aa | — | text hit-rect code inside the text path | UR "Corrections from M8a" (hit rects) |
| `textdraw::sprite_rect` | 0x100b30d0 | 0x100b30a0 | textdraw sprite / model path | UR "Sprite / model path" |
| `textdraw::sprite_hit_rect` | 0x100b3179 .. 0x100b31ce | — | sprite hit-rect code inside the sprite path | UR "Corrections from M8a" (hit rects) |
| `textdraw::TextDraw` (creation number) | 0x100b3720 | — | textdraw object constructor | UCA "134 PlayerShowTextDraw" |
| `textdraw::Selection` | 0x1026e9e0 | — | textdraw selection global | UR "Selection mode" |
| `textdraw::TextDraws` hit test | 0x10071470 | — | per-frame selection hit test | UR "Selection mode" |
