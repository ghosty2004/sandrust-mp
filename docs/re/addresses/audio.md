# audio.rs — samp.dll addresses

Most audio work was read on build B; A is filled in where the spec records it. GTA addresses (radio volume, menu
flag, window handle, `CAudioEngine`, weather-audio vtable slot) are named constants in the code.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `init` | 0x10066480 | 0x10066440 | audio set-up: `BASS_Free`, `BASS_Init`, config, `audioproxyoff`, EAX off | ui-chat-audio.md "41"; m9-notes.md |
| `NET_AGENT` | | 0x101027c8 | user agent string `SA-MP/0.3` | m9-notes.md |
| `play` | 0x10066960 | 0x10066920 | RPC 41 body: stop, store URL / position, chat line, start the worker | ui-chat-audio.md "41" |
| `on_play_rpc` | 0x1001d3c0 | 0x1001d370 | RPC 41 handler | ui-chat-audio.md "41"; rpc-ids.md |
| `stop` | 0x10066560 | 0x10066520 | stop flag, optional wait, `StreamFree` | ui-chat-audio.md "42" |
| `STOP_WAIT_MS` | | 0x1006654b | `Sleep(2)` in the waiting stop | m9-notes.md |
| (RPC 42 handler) | | 0x100180a0 | calls the stop without waiting | ui-chat-audio.md "42" |
| `worker` | 0x10066700 | 0x100666c0 | stream thread | ui-chat-audio.md "41" |
| `VOLUME_TICK_MS` | | 0x100668e7 | `Sleep(20)` of the worker's volume loop (measured 479 iterations in 9.59 s) | trace-notes.md "Audio" |
| `stream_volume` | | 0x10066823..0x100668e2 | per-step stream volume | m9-notes.md; trace-notes.md "Audio" |
| `GLOBAL_SCALE` / `POSITIONAL_SCALE` | | 0x100e9bb4 / 0x100e9be0 | 7000.0 / 8000.0 volume scales | m9-notes.md |
| `station_from_headers` | | 0x1006674a..0x100667c3 | ICY / HTTP header parsing | m9-notes.md |
| `title_from_meta`, `update_title` | | 0x10066580 | title refresh (META `StreamTitle='`) | m9-notes.md |
| `title_from_ogg` | | 0x100665f0..0x1006669f | OGG `artist=` / `title=` parsing | m9-notes.md |
| `title_text`, `title_line` | 0x10066ab0 | 0x10066a70 | Present title line | ui-rendering.md (address table); m9-notes.md |
| `TITLE_COLOUR`, `TITLE_FLAGS` | | 0x10066b04 | colour and DrawText flags of the title line | m9-notes.md |
| `READY` | | audio object +0 | audio object exists and `BASS_Init` succeeded | ui-chat-audio.md "41" |
| `PLAYING` | | 0x1012e7b8 | worker-running flag | m9-notes.md |
| `STOP` | | 0x101027d2 | worker stop request | m9-notes.md |
| `HANDLE` | | 0x1012e7b4 | stream handle | m9-notes.md |
| `Stream` | | 0x1012e398.. | URL / title / station buffers (256 + NUL) | m9-notes.md |
| (audio object global) | 0x1026ea04 | | samp's audio object pointer | ui-chat-audio.md "41" |
| `frame` | | 0x100c4ff0..0x100c500a | per-frame radio calls after the netgame tick | m9-notes.md |
| (radio calls) | | 0x10066a40 (called from 0x100c4ffa) | `StartRadio(-1, 0)` / `StopRadio(NULL, 0)` while a stream plays | m9-notes.md |
| `MissionSound`, `play_sound` | 0x100a2300 | 0x100a22d0 | PlayerPlaySound on samp's game audio object | player-state.md (PlayerPlaySound); trace-notes.md "PlayerPlaySound (16)" |
| `MissionSound::frame` | | 0x100a21e0 | per-frame mission-audio poll | trace-notes.md "PlayerPlaySound (16)" |
| `MissionSound` (fields) | | | game audio object u32 +0 (mission state) and byte +4 (sound-id-1 flag) | player-state.md (PlayerPlaySound) |
| `gta::RADIO_VOLUME` | | 0x100a2290 | samp's game wrapper that reads GTA's radio volume | m9-notes.md |
| `weather_update_parameters_stub` | 0x100a5e70 | 0x100a5e40 | weather-audio `UpdateParameters` replacement (vtable slot redirect) | gta-patches.md "10. Full table" (0x872a74) |
| (not imported) | | 0x10066B90 | dead code that references the BASS 3D functions | m9-notes.md |
