# combathooks.rs

"Row N" is the row of docs/re/data/gta-patches-r4.csv. Hook names H3–H9 as in remote-control-combat.md §9.1.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `site::FIRE_INSTANT_HIT` / `fire_instant_hit_site` | 0x100A5820 | — | H3, rows 98–100 | remote-control-combat.md §3.2 |
| `site::FIRE_SNIPER` / `fire_sniper_site` | 0x100A58E0 | — | H3, row 104 | remote-control-combat.md §3.2 |
| `site::LOS` / line-of-sight hook | 0x100A5640; local-hit suppression returns 0 at 0x100A57D8 | — | H4, rows 101–103 | remote-control-combat.md §3.4 |
| `site::ADD_BULLET` / `add_bullet_site` | 0x100A5930 | — | H5, row 105 | remote-control-combat.md §3.5 |
| `site::INFLICT_DAMAGE` / `inflict_decide` | 0x100A5280; cancel with `ret 0x18` at 0x100A53BE | — | H8, row 106 | remote-control-combat.md §6.5 |
| `install_vehicle_friendly_fire` | 0x100A7200 (called at 0x10010A56) | — | installs row 106 when InitGame enables vehicle friendly fire (settings +0x1B) | remote-control-combat.md §6.5 |
| `site::COMPUTE_DAMAGE_RESPONSE` / `damage_decide` | entry → 0x100A3D50 → decision 0x100A3BF0 | — | H6, row 53 (`jmp [0x4B5ABC]`) | remote-control-combat.md §6.1 |
| `site::AFFECTS_PED` / AffectsPed stub | 0x100A53E0 | — | H7, row 80 (`jmp [0x4B3433]`) | remote-control-combat.md §6.2 |
| `site::RENDER_WEAPON_SKILL` / `render_weapon_skill` | 0x100A2D80 | — | H9, row 22 | remote-control-combat.md §1.3 |
| `site::CROSSHAIRS` / `crosshairs_site` | 0x100A2590 | — | row 8 | ui-rendering.md |
| `site::SNIPER_CAM_SHAKE` / `no_cam_shake` | 0x100A3DA0 | — | bare `ret`, row 46 | remote-control-combat.md §3.5 |
| `site::TAKE_PHOTO` / TakePhotograph stub | 0x100A3DB0 | — | row 45 | remote-control-combat.md §3.5 |
| `site::SATCHEL_UPDATE` / satchel stub | 0x100A3FC0 | — | row 56 (`jmp [0x738B1B]`) | remote-control-combat.md §3.5 |
| `site::DETONATOR` / detonator stub | 0x100A4000 | — | row 57 (`jmp [0x73885B]`) | remote-control-combat.md §3.5 |
| `site::FIND_LOCK_ON`, `FIND_NEXT_LOCK_ON` | 0x100A1C10 | — | lock-on RETs, rows 4 / 5 | remote-control-combat.md §3.5 |
| `install_init_game` | 0x10010909 | — | InitGame: rows 4/5 and 98–106 only for lag-comp 1 and 2 | remote-control-combat.md §3.4 |
| `install_main` | — | — | rows 8, 22, 45, 46, 53, 56, 57, 80, 191, 200, 201, 210, 240–243, 247 | gta-patches.md |
| `TWO`, `BOUND_MIN`, `BOUND_MAX` | — | — | rows 210 and 240–243 | gta-patches.md "Weapons / bullets" |
| `RemoteHook::vehicle` | remote +0x1E7 | — | vehicle of the last driver/passenger sync | remote-control-combat.md "Vehicle → player mapping" |
| `Shared::surface` | local surf state +0x2DA | — | surfed entity | remote-control-combat.md §6.1 |
| `Shared::player_of_entity` | 0x100A3B20 | — | entity → player | remote-control-combat.md §6.1 |
| `fire_shot`, `ShotContext` | 0x100AFAB0 | — | fires one shot | remote-control-combat.md §3.3 |
| `origin_and_muzzle` | 0x100AFA00 | — | shot origin / muzzle | remote-control-combat.md §3.3 |
| `force_remote_collisions` / `restore_remote_collisions` | 0x10013C90 / 0x10013D10 | — | collision for remote peds around a local shot | remote-control-combat.md §3.4 |
| `capture_local` | 0x100A5450 → 0x10006BC0 | — | shot record → 206 | remote-control-combat.md §3.4 |
| `extinguish` | 0x100AD130 | — | ped and vehicle fire off | remote-control-combat.md §7 |
