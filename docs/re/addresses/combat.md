# combat.rs

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `DAMAGE_SCALE` | 0x100E5900 (constant); used by 0x100068A0 / 0x100069B0 | — | 0.33 damage scale of RPC 115 | remote-control-combat.md §6.1 |
| `MAX_OFFSET`, `remote_bullet_valid` | 0x10016370 | — | 206 receive-side apply (offsets within ±300) | remote-control-combat.md §3.3 |
| `SNIPER_VELOCITY_SCALE` | 0x100EA63C (constant); hook 0x100A5930 | — | AddBullet hook's sniper velocity factor | remote-control-combat.md §3.5 |
| `ORIGIN_Z_EXTRA`, `BONE_RIGHT_HAND` | 0x100AFA00 | — | shot origin from the hand bone (+0.15 z; bone 24, 34 for the other caller) | remote-control-combat.md §3.4 |
| `lag_comp_selector` | 0x10118990 | — | selector S global | remote-control-combat.md §3.4 |
| `bullet_hooks_active` | InitGame 0x10010909; installer 0x100A7050; lock-on RETs 0x100A1C10 | — | hooks installed only for lag-comp 1 and 2 | remote-control-combat.md §3.4 |
| `Matrix::transform` | 0x100B4D50 | — | matrix × point | — |
| `Matrix::inverse_transform` | 0x100B5780 (inverse), then 0x100B4D50 | — | inverse rigid transform | — |
| `ShotRecord`, `capture` | 0x100A5450 | — | 44-byte shot record of the line-of-sight hook | remote-control-combat.md §3.4 |
| `HitTarget`, `bullet_sync` | 0x10006BC0 | — | 206 builder | remote-players-sync.md "206 PlayerBulletSync" |
| `redirect_end` | 0x100A5640 | — | line-of-sight hook redirect | remote-control-combat.md §3.4 |
| `DamageCtx`, `surf_inputs`, `decide` | 0x100A3BF0 | — | damage decision | remote-control-combat.md §6.1 |
| `DamageCtx::surface_is_damager`, `surfing` | local surf state +0x2DA | — | surfed entity | remote-control-combat.md §6.1 |
| `same_team` | Settings +0x22 (always 1) | — | same-team rule switch | remote-control-combat.md §6.3, "Same-team rule" |
| `object_of_entity` | 0x10012A10 | — | object pool lookup by entity | world-entities.md |
| `death_reason` | 0x100AC890 | — | killer / reason of RPC 53 | player-state.md "53 OnPlayerDeath" |
| `AimSample::aim_z` | CPlayerPed +0x480 → +0x54 | — | look pitch | remote-players-sync.md "203 PlayerAimSync" |
| `build_aim` | 0x10005210 | — | 203 builder | remote-players-sync.md "203 PlayerAimSync" |
| `AimContext`, `aim_interval` | local tick 0x10007CB3; passenger 0x100057B0; spectating 0x10006665 | — | 203 send sites | remote-players-sync.md §0.4 |
| `WeaponsUpdate` (204 encoder) | 0x10006290 | — | 204 delta encoder | remote-players-sync.md "204 target fields" |
| `LocalMasks` | local ped object +0x2F6 / +0x2F7 / +0x2F8 | — | pad masks | remote-control-combat.md "Local pad masks" |
| `MaskState` (carry) | 0x100ACE90 | — | carry: all three masks | remote-control-combat.md "Local pad masks" |
| `MaskState` (cuffs) | 0x100ACF40 | — | cuffs: sprint mask | remote-control-combat.md "Local pad masks" |
