# footsync.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `LocalSample::surf_offset` | 0x100050E8 | — | surf offset / surf id written into the 207 | remote-players-sync.md "Local surfing" |
| `key_word` | 0x100AF610 | — | u16 key word builder | remote-players-sync.md §0.3 |
| `FootSendState`, `on_foot_interval` | 0x100074C0 | — | local player tick with the send-rate machinery | remote-players-sync.md §0.4 |
