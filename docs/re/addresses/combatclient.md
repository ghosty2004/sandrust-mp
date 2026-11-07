# combatclient.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `Combat::drive_by` | local player +0x1CB | — | passenger drive-by flag | remote-control-combat.md "Drive-by" |
| `on_init_game` | 0x10010660 | — | InitGame: selector and lag-comp patches | remote-control-combat.md §3.4 |
| `on_bullet` | 0x10009B40 → 0x10016370 | — | 206 receive handler → apply | remote-control-combat.md §3.3 |
| `weapons_tick` | 0x10006290 | — | 204 encoder | remote-players-sync.md "204 target fields" |
| `aim_on_foot` | 0x10007C9A | — | 203 after the 207 in the local tick | remote-players-sync.md §0.4 |
| `aim_spectating` | 0x10006620 | — | 203 right after a 212 | remote-players-sync.md §0.4 |
| `send_aim_gated` | 0x10005210 | — | 203 builder and duplicate gate | remote-players-sync.md "203 PlayerAimSync" |
| `aim_in_car` | passenger 0x100057B0; driver 0x10007452 / 0x10007951 | — | in-car 203 send sites | remote-players-sync.md §0.4 |
| `passenger_drive_by` | 0x10007B68 | — | local passenger drive-by | remote-control-combat.md "Drive-by" |
| `on_special_action` | — | — | carry / cuffs masks | remote-control-combat.md "Local pad masks" |
| `cuffed_tick` | 0x100ACF60 | — | cuffed: weapon back to fists | remote-control-combat.md "Local pad masks" |
| `local_context` | 0x100AC0A0 / 0x100AF7D0 / 0x100AC050 | — | samp's weapon helpers under the slot context | remote-control-combat.md §1.2 |
| `set_armed_local` | 0x100AC0A0 | — | set-armed-weapon helper | remote-control-combat.md §4 |
| `set_ammo` | 0x1001AC10 → 0x100B00C0 | — | RPC 145 handler → ped helper | player-state.md "145 SetPlayerAmmo" |
| `set_local_skill` | 0x1000F5E0 | — | RPC 34 | player-state.md "34 SetPlayerSkillLevel" |
| `create_explosion` | 0x1001BD10; radius push at 0x1001BDB3 | — | RPC 79 | player-state.md "79 CreateExplosion" |
