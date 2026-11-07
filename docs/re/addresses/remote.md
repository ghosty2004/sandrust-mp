# remote.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `NUDGE_GAIN` | 0x100FE4B0 | — | the 0.1 gain constant of the position correction | remote-control-onfoot.md §3.4 |
| `collision_flags` | 0x1009F6E0 (on), 0x1009F690 (off) | — | samp's two collision helpers | remote-control-onfoot.md §3.6 |
| `SlotAllocator` | 0x100A0170 | — | first-free ped slot allocation | remote-control-onfoot.md §1.2 |
| `SwapTable::slot_of` | 0x100B43A0 | — | ped → slot reverse lookup | remote-control-onfoot.md §1.1 |
| `samp_deg_to_rad` | 0x100B59B0 | — | degrees → radians | remote-control-onfoot.md §3.2 |
| `Correction` (position correction) | 0x10014C40 | — | per-packet position correction | remote-control-onfoot.md §3.2 |
| `RemotePlayer::store_foot` | 0x10017260 | — | 207 store rule | remote-players-sync.md §2.1 |
| `SpecialStep`, `special_action_steps` | 0x10014950 | — | per-frame special-action pass | remote-control-onfoot.md §4.2 |
| `RemotePlayer::phone` | ped object +0x48 | — | phone flag | remote-control-onfoot.md §4.2 |
| `RemotePlayer::marker_blip` | remote +0x1F9 | — | 208 marker blip | remote-players-sync.md "208 PlayerMarkersSync" |
| `RemotePlayer::marker_active` | remote +0x1E9 | — | 208 record had a position | remote-players-sync.md "208 PlayerMarkersSync" |
| `RemotePlayer::ped_blip` | ped object +0x2CD | — | blip on the ped | — |
