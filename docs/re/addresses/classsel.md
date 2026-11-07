# classsel.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `SOUND_NEXT`, Right arrow in `ClassSelection::tick` | 0x10005FF0 | — | "next class" handler: plays 1052 (`push 0x41c` at 0x1000605E), sends RPC 128 | player-state.md "128 PlayerRequestClass" |
| `SOUND_PREVIOUS`, Left arrow in `ClassSelection::tick` | 0x10006080 | — | "previous class" handler: plays 1053 (`push 0x41d` at 0x100060E1), sends RPC 128 | player-state.md "128 PlayerRequestClass" |
| `Action::ChangeClass` (sound) | 0x100A2300 | — | game audio object's PlayerPlaySound path used for the arrow sounds | player-state.md "16 PlayerPlaySound" |
| `ClassSelection::info_received` | LocalPlayer +0x143 | — | "class info received" flag | player-state.md "128 PlayerRequestClassResponse" |
| `ClassSelection::request_pending` | LocalPlayer +0x302 | — | "spawn request pending" flag | player-state.md "129 PlayerRequestSpawn" |
