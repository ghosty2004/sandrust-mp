# ui/glue.rs — global UI state, frame entry points, Present / Reset wrappers, session hooks

Specs: docs/re/subsystems/ui-rendering.md ("UR" below, section numbers as in its headings),
docs/re/subsystems/ui-chat-audio.md ("UCA", by RPC heading), docs/re/subsystems/session-misc.md.
A = 2021 build, B = 2019 build; "—" means not recorded for that build (the builds are protocol-identical;
many UI routines were looked up in only one of them).

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `ADD_BIG_MESSAGE` / `show_game_text` | 0x100a0d40 | — | samp's game-text call into `CMessages::AddBigMessage` | UCA "73 SendGameText" |
| `OP_GET_CHAR_HEADING` | 0x100e9dc0 | — | script-command descriptor of `GET_CHAR_HEADING` used by `/save`, `/rs` | UCA "Local chat input and built-in commands" |
| `OP_GET_CAR_HEADING` | 0x100e9d98 | — | script-command descriptor of `GET_CAR_HEADING` used by `/save`, `/rs` | UCA "Local chat input and built-in commands" |
| `LiveGame::position` | 0x10068a00 | — | `/save` | UCA "Local chat input and built-in commands" |
| `LiveGame::position` | 0x10068b80 | — | `/rs` | UCA "Local chat input and built-in commands" |
| `State::class_panel_shown` | 0x1026eaa4 | — | class-selection GUI dialog global (visible flag +0x13) | UR 4.2 |
| `State::class_panel_shown`, `frame_entry`, `dialog_open` | 0x10006100 | — | class-selection per-frame step (does nothing while a dialog is open) | UR 4.2 |
| `State::spawn_release`, `class_selection_release` | 0x10003c20 | — | spawn: ends class selection, mode-0 request with the 10-frame restore | UR 5 |
| `set_input_mode` | 0x100a0750 | — | input-lock mode setter (`mode`, `flag`); the spawn calls it with (0, 0) | UR 5 |
| `RateSampler`, `set_pool_stats` | 0x10060d20 | — | F5 network-statistics overlay (rates, pool block) | UR 18 |
| `user_dir` | 0x100c3cb0 | — | user-files directory (`<user files>\SAMP`, `screens`) | UR 6.4 |
| `MOUSE_SINK` | 0x101501a8 | — | dummy mouse store of input-lock mode 4 | UR 5 |
| `restore_input_now` | 0x10072480 | — | object-editor start (mode 0, restore at once) | session-misc (object editing) |
| `paint_plate` | — | 0x1006D840 | number-plate painter | UR 18 |
| `cancel_textdraw_selection` | — | 0x1000E880 | RPC 27 handler | UCA "83 PlayerBeginTextDrawSelect" |
| `cancel_textdraw_selection` | — | 0x10011880 | RPC 117 handler | UCA "83 PlayerBeginTextDrawSelect" |
| `cancel_textdraw_selection` | — | 0x10071550 | textdraw selection off | UR 14.4 |
| `close_chat_input` | — | 0x10069540 | chat input close (editor end) | UR 8 |
| `close_chat_input` | — | 0x10069440 | chat input close (selection end) | UR 8 |
| `sprint_held` | — | 0x100AF5E0 | on-foot key word test (sprint bit 8) | — |
| `pick_ray` | — | 0x10073F70 | object-selection cursor ray from WORLD / VIEW / PROJECTION | session-misc (object selection) |
| `editor_pass`, `present_pass` step 12 | — | 0x100737B0 | object editor per frame | session-misc (object editing) |
| `editor_pass`, `present_pass` step 12 | — | 0x1006DBC0 | object selection per frame | session-misc (object selection) |
| `editor_message` | — | 0x10073580 | object editor window messages | session-misc (object editing) |
| `editor_message` | — | 0x1006DE00 | object selection window messages | session-misc (object selection) |
| `editor_message` | — | 0x100618A1 | ESC `WM_CHAR` check in samp's window procedure (editor) | UR 6.2 |
| `editor_message` | — | 0x100618FA | ESC `WM_CHAR` check in samp's window procedure (selection) | UR 6.2 |
| `ensure` (`nohudscalefix`) | 0x100C49DD | — | HUD scale fix flag cleared only for `nohudscalefix` = 1 | UR 1.1 |
| `DIRECT_MODE_LINE` | — | 0x100e9d64 | the "ChatWindow: Using direct drawing mode." string | UR 7.6 |
| `DIRECT_MODE_LINE` | — | 0x100680b0 | debug-colour (type 8) chat print | UR 7.6 |
| `boot_settings` | — | 0x100c4e2c..0x100c4efa | rest of the first-frame boot routine (`multicore`, audio, BASS) | UR 1.1 |
| `frame_entry` | 0x100c4fb0 | — | per-frame pre-step before `Render2dStuff` | UR 2.4 |
| `frame_entry` (renderer, `netgame_object`) | 0x100c47d0 | — | first-frame UI construction; netgame object on the next game frame | UR 1.1 |
| `frame_entry`, `reset_hook` (direct-mode line) | — | 0x10068190 | chat construction / render-target re-creation | UR 7.6 |
| `frame_entry` (`preview::init_env`) | 0x1006ccc0 | — | model-preview camera set step of the first-frame pass | UR 14.3a |
| `game_frame` | 0x100a27d0 | — | `CGame::Process` hook routine that calls the preview pass (call at 0x100a27e6, see ui-preview.md) | UR 14.3a |
| `game_frame` | 0x1001e870 | — | per-game-frame model-preview pass | UR 14.3a |
| `reset_hook` | 0x100c3f90 | — | lost-device handling | UR 2.2 |
| `reset_hook` | 0x10069580 | — | chat input close on lost device (cursor mode 0, immediate) | UR 2.2 |
| `reset_hook` | 0x1006e9e0 | — | scoreboard close on lost device (called with 1: cursor mode 0, delayed) | UR 2.2 |
| `reset_hook` | 0x100c419b | — | store that re-arms the "SetCursor(NULL) once" flag of WM_SETCURSOR | UR 2.2 |
| `reset_hook` | 0x100c41e5 | — | model-preview texture release, last in the lost-device pass | UR 2.2, 14.3a |
| `reset_hook` | 0x100c41f0 | — | on lost / reset device | UR 2.2 |
| `reset_hook` | 0x100c43ff..0x100c4431 | — | last reset step: cursor mode 2, then mode 0 with the restore at once | UR 2.2 |
| `CAMERA_POS` | 0x1009f1f0 | — | samp's camera-distance helper | UR 16.1 |
| `nametags` | 0x10074270 | — | nametag bars pass: who gets one | UR 16.1 |
| `nametags` | 0x100b7e40 | — | RC-vehicle test (valid pool handle, models 441/464/465/501/564/594) | UR 16.1 |
| `nametags` | 0x1009f430 | — | moving-list test (`CPhysical` +0xB4) | UR 16.1 |
| `nametags` (driver state) | — | — | samp player state 0x13 = driver | UR 16.1 |
| `map_click`, `present_pass` step 3 | — | 0x10075B7B | pause-menu map right-click (RPC 119) | session-misc "119 OnPlayerClickMap" |
| `present_pass` step 1 | 0x10075620 | — | F8 screenshot | UR 6.4 |
| `present_pass` | 0x10075790 | — | Present per-frame draw order | UR 2.3 |
| `present_pass` step 4b | — | 0x10066a70 | audio stream title line | UR 2.3 |
| `present_pass` textdraw selection | 0x10071470 | — | textdraw selection per frame | UR 14.4 |
| `scoreboard_click` | — | 0x100C5590 | scoreboard GUI callback (event 0x701 sends RPC 23 and closes) | UR 15.3 |
| `scoreboard_click` | — | 0x1006EBD0 | scoreboard list activation | UR 15.3 |
| `camera_target_labels_on` | — | 0x101189A8 | `/ctd` flag | UCA "Local chat input and built-in commands" |
| `draw_camera_target_labels` | — | 0x10004830 | `/ctd` label drawing from the local tick | UCA "Local chat input and built-in commands" |
| `vehicle_labels_on` | — | 0x1026EA30 | `/dl` flag | UCA "Local chat input and built-in commands" |
| `head_move_disabled` | — | — | samp's Game+0x59 head-tracking flag | UCA "Local chat input and built-in commands" |
| `on_init_game` | 0x10118990 | — | lag-compensation selector (InitGame 1 → 0, 2 → 1, else 2) | session-misc (InitGame) |
| `on_reset` | — | 0x1000a530 | RPC 40 / lost-connection reset of pools and server settings | session-misc |
| `reset_colours` | 0x100A72C0 | — | player colour table reset | UCA "Shared: chat window model" |
| `dialog_open` | 0x1026e9c8 | — | dialog object global (open flag +0x28) | UR 13 |
| `chat_input_open` | 0x1026e9fc | — | chat input object global (open flag +0x14e0) | UR 8 |
