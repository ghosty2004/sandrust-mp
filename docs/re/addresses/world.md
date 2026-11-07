# world.rs — samp.dll addresses

Clock and weather state (`crates/samp-client/src/world.rs`). A = 2021 build, B = 2019 build.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| module (clock/weather state) | 0x100A0B60 | — | Game method called with 0 on the first frame (clock off) | player-state.md, "Object map used in this document" |
| `World::toggle_clock` | 0x100A14A0 | — | Game method: clock on/off (`CClock::Update` byte, HUD string, Game +0x69) | player-state.md, "30 TogglePlayerClock" |
| `World::weather_writes` | 0x100A0BA0 | — | Game method: set weather (forced; old/new too while the clock is off) | player-state.md, "152 SetPlayerWeather" |
| module (clock/weather state) | 0x100A0B10 | — | Game method of the same clock/weather group | player-state.md, "Object map used in this document" |
| `World::default` | 0x100C47D0 | — | first-frame init: calls the clock toggle with 0 (patch table rows 354/355) | gta-patches.md, "Full table" |
| `World::default` | 0x1000B930 | — | netgame constructor: hour 12, minute 0, weather 1, settings +0x1f := 1 | player-state.md, "Object map used in this document" |
| `CONNECTING_WEATHER` | 0x100089B0 | — | per-frame while not connected: weather := 1 | player-state.md, "152 SetPlayerWeather" |
| `World::weather` (InitGame) | 0x10010660 | — | InitGame handler: weather := InitGame weather | handshake.md, InitGame |
| `World::time_of_day` | 0x1000B5B0 | — | netgame tick: clock bytes + `SET_TIME_OF_DAY` while the server holds the time | player-state.md, "60 SendGameTimeUpdate" |
| `World::on_toggle_clock_rpc` | 0x1000F500 | — | RPC 30 handler (calls 0x100A14A0) | player-state.md, "30 TogglePlayerClock" |
| `TIMER_MS` | — | 0x100A1760 | RPC 60 handler writing `CTimer::m_snTimeInMilliseconds` while the HUD clock is off | player-state.md, "60 SendGameTimeUpdate" |
