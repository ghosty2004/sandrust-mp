# swap.rs

"Row N" is the row of docs/re/data/gta-patches-r4.csv.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `player_ped_process_control` | 0x100A29A0 | — | `CPlayerPed::ProcessControl` hook (vtable slot 0x86D190, row 19) | remote-control-onfoot.md §2.2 |
| `guarded_thiscall`, `take_faults`, `FAULTS_HANDLED` | 0x100A2800 | — | SEH wrapper around the remote ped's `ProcessControl` (first 10 faults handled; message "Warning(playerped): Exception 0x… at 0x….") | remote-control-onfoot.md §2.2 |
| `fault_line` | — | 0x10060960 | samp's exception filter / chat line | — |
| `use_gun_set_ped_position` | — | — | `CTaskSimpleUseGun::SetPedPosition` hook (vtable slot 0x86D744, row 21) | remote-control-onfoot.md §2.3 |
| `player_ped_process_collision` | — | — | `CPlayerPed::ProcessCollision` hook (vtable slot 0x86D194, row 23) | remote-control-onfoot.md §3.6 |
| `addr::UPDATE_POSITION_CALL` | — | — | GTA 0x5E92F4, row 366 | remote-control-onfoot.md §2.2 |
| `addr::AIM_ROT_STORE_ZELDA`, `AIM_ROT_STORE_DUCKED` | — | — | GTA 0x6884C4 / 0x688200, rows 213 / 214 and 367 | remote-control-onfoot.md §2.2 |
| `apply_local_masks` | LocalPlayer +0x2F6 / +0x2F7 / +0x2F8 | — | carry / cuffs pad masks | remote-control-combat.md "Local pad masks" |
