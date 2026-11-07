# specials.rs

All read in build B; "—" means not recorded for that build.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `item_for_action`, `local_action` | — | 0x100030F0 (cases 0x14…0x17) | RPC 88 for the local player | player-state.md "88 SetPlayerSpecialAction" |
| `action_for_item`, `local_report` | — | 0x10003410 | 207 special-action report of the local player | remote-control-onfoot.md §4.2 |
| `item_object`, `start_item` | — | 0x100B02E0; hand offsets (doubles) 0x100ED298…0x100ED2C0 | hold-item ped method | remote-control-onfoot.md §4.2 |
| `remote_pass` (items) | — | 0x10014A78; pass 0x10014A38…0x10014B39 | remote special-action pass | remote-control-onfoot.md §4.2 |
| `remote_pass` (parachute check) | — | 0x100AD450 | parachute object removed when the weapon is no longer 46 | remote-control-onfoot.md §4.2 |
| `drop_item` | — | 0x100ADC10 | drop the held item | remote-control-onfoot.md §4.2 |
| `use_item` | — | 0x100ADCB0 | use the held item (fire = pad Circle, +0x22) | remote-control-onfoot.md §4.2 |
| `DRINK_DRUNK`, `DRUNK_CAP`, `after_drink` | — | 0x100ADD8F | +1250 per drink, cap 50000 | remote-control-onfoot.md §4.2 |
| `female_drink` | — | 0x100ADD71 (compare), 0x100B4110 (reads `[[info] + 0x28]`) | female drink-anim choice | remote-control-onfoot.md §4.2 |
| `arm_fists` | — | 0x100AC070 | set-armed helper | — |
| `start_piss` | — | 0x100AE110 | `PISS_LOOP` + `CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION("PETROLCAN", ped, 0, 0.58, −0.08, 0, 0.01, 0, 1)` | remote-control-onfoot.md §4.2 |
| `stop_piss` | — | 0x100AE1F0 | `KILL_FX_SYSTEM`, position self-set | remote-control-onfoot.md §4.2 |
| `local_tick` | — | 0x1000772C…0x100077AB | local tick's piss / item part | remote-control-onfoot.md §4.2 |
| `remote_para_anim`, `para_object_anim` | — | 0x100AD630 (called from the remote anim decoder 0x100AE7E0) | parachute object animations | remote-control-onfoot.md §4.2 |
| `para_sound_pos` | — | 0x100AD6C5 | PARA_OPEN sound position | remote-control-onfoot.md §4.2 |
| `para_object` | — | 0x100AD4E0 | parachute helper object (`CREATE_OBJECT(3131)`, `PLAY_OBJECT_ANIM("PARA_OPEN_O", 1000, 0, 1)`) | remote-control-onfoot.md §4.2 |
| `remote_destroyed` | — | 0x100B0FB0 | ped wrapper destruction | remote-control-onfoot.md §1.5 |
| `PedExtras` | — | ped object +0x2B9 parachute object, +0x2BD piss effect, +0x2C1 item kind, +0x2C5 item object, +0x2F2 pissing | per-ped state | remote-control-onfoot.md §4.2 |
