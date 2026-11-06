# M7 — combat (aim / weapons / bullets / damage / death)

Implementation of milestone M7 in `crates/samp-client`. Specs: `docs/re/subsystems/remote-control-combat.md` (+ RE-10
additions), `remote-players-sync.md` §0.4 / §203 / §204 / §206 / §2.1, `player-state.md` 53 / 115 / 166 / 34 / 145;
measurements: `docs/phase3/trace-notes.md` "Combat (TRACE-6)" and "TRACE-7 additions".

## Files

| file | what |
|---|---|
| `combat.rs` | platform-independent: lag-comp selector, 206 record / offsets / redirect point, damage decision table (§6.1), RPC 53 killer/reason, 203 builder + intervals, 204 delta encoder, carry/cuffs masks (unit tests) |
| `combathooks.rs` | the GTA hooks (rows 4, 5, 8, 22, 45, 46, 53, 56, 57, 80, 98–106, 191, 200, 201, 210, 240–243, 247), the shot function, outbox of 206 / 115 / 177 |
| `combatclient.rs` | client side: InitGame install, the hooks' shared view, sending, remote 206 apply, 203/204 senders, local drive-by, masks, RPC 145/34/79 helpers |
| `client.rs` | RPCs 22/67/145/34/69/166/79/88, death (53) and respawn after the wasted sequence, send order 205 → 204 → in-car/foot → 203 |
| `players.rs` | RPC 166 (remote death), RPC 34 / 69 for remotes, shooter arming for 206 |
| `vehicles.rs` | weapon one-shot apply for remote drivers/passengers, remote passenger drive-by task, accessors for the hooks |
| `swap.rs` | local pad masks in the local branch of the ProcessControl hook |
| `tools/e2e/m7.sh` | acceptance test (modes 1, 2, 0) |
| `tools/trace/uifs.pwn` | + `wdata|P|slot` (GetPlayerWeaponData), `kick|P` |
| `tools/trace/combat2.sh` | `COMBAT2_TARGET` (bots aim at our client's name) |

## Static details confirmed in samp.dll A (beyond the spec)

* The call-site stubs (A 0x100A5820 / 0x100A58E0) return their 8th / 1st argument; a local shot ignores GTA's origin
  arguments and fires through A 0x100AFAB0: the weapon (`GET_CURRENT_CHAR_WEAPON`) decides between `FireSniper(ped, 0, 0)`
  and `FireInstantHit(ped, origin, muzzle, 0, 0, 0, 0, 1)`; origin = bone 24 + (fire offset z + 0.15), fire offset from
  `GetWeaponInfo(type, 1)` +0x24; muzzle = fire offset through `GetTransformedBonePosition` of bone 24.
* InitGame (A 0x10010909) installs the bullet hooks (rows 98–106) and the lock-on RETs (rows 4/5) **only for lag-comp 1
  and 2**. Mode 0 has none of them (matches TRACE-6: "no LOS call from samp at all").
* The LOS hook returns 0 when it suppresses a hit on the local ped (A 0x100A57D8).
* A local death extinguishes the ped's and its vehicle's fire (A 0x100AD130: `CFire::Extinguish`); the spec's "leaves the
  vehicle" for that helper is wrong. The remote death helper (A 0x100B0010) also extinguishes (not "leave vehicle").
* Death check: `(dead == 0 && ped state == 54) || health ≤ 0`; respawn once the state is neither 54 nor 55: cuffs off,
  then spawn when class info is valid, no forced reselection, connected and health > 0; otherwise class selection.
  On death zone names go off (A 0x100A1560(0)).
* 204 cache starts with targets 0xFFFF (no 204 at spawn in the original captures); the timestamp is updated only on
  an actual send; it is not reset on respawn.
* 203 duplicate gate: shared timestamp (> 500 ms) with 207 on foot (`FootSender::touch`) and with 200/211 in a car
  (`Vehicles::dup_offer`).
* Row 45 (TakePhotograph): the stub's `[esp+8]` at 0x73C252 is the caller's saved EDI = `CWeapon::Fire`'s owner
  (gta_sa 0x74232F loads EDI with the first argument; nothing writes EDI before the call at 0x74278B).
* Row 8 (crosshair): the correction runs only with samp's flag A 0x10117470, cleared at start-up unless sa-mp.cfg has
  a non-zero `nohudscalefix` (A 0x100C49DD, config getter returns the int value). Default: wrapper = original.

## Decision (accepted by main, ROADMAP): 206 hitPos on a miss

samp copies the col point after its hooked LOS call even when nothing was hit; that col point is a stack local of
GTA's `FireInstantHit` written only on a hit, so the original sends stack garbage (per session: e.g. (9.14e34,
1.065e-38 = dword 0x0073FB10, NaN), (5.05e-39, 1.24e-36, 2.2e-44), (7e-45, 1.1e-44, 0)). We replicate the mechanism:
our LOS hook reads the same GTA stack col point at the same moment. With our call chain it read (0, 0, 0) in every run so
far. Everything else of a miss is identical (hit type 0, id 0xFFFF, offset 0, origin, weapon; sniper without entity not
sent).

## Deviations

* Hook state lives on the stack / in atomics instead of samp's fixed globals (spec P1).
* The remote shot's camera context is restored to the saved values (samp restores through its own copies).
* (Resolved 2026-11-06) The surf rows 3/4 of the damage table now read the local surf state (surface entity of a pool
  vehicle or object); hit type 3 is produced with samp's object lookup (ids from 1, A 0x10012A10). See "Object hits and
  surf rows" below.
* RPC 177: the decision and the send are implemented; `combathooks::Shared::actors` must be filled by the M6 actor pool
  (empty now, so actors count as "other victim").
* Remote cuffs (bit 7 of 211, action 24): no visible effect implemented — TODO-unknown (measurement request 2).
* Death while spectating / F4 return-to-class-selection: spectating is M8b; F4 is not wired.

## Measurement requests (sent to main; main asked M7 to measure them itself on the trace lane)

1. Crosshair multipliers 0x859520/0x859524 while aiming with the default sa-mp.cfg — see "Measurements" below.
2. Remote cuffs on a remote ped (tasks, anims, GTA calls) — see "Measurements" below (`samp-bot --special-action 24`).
3. The remote's vehicle id (+0x1E7) after the remote leaves the vehicle — **answered statically**: written by the
   driver store (A 0x1001737D) and the passenger store (A 0x10017475), read by the vehicle→player mapping (A 0x100A3B9C)
   and three apply paths; reset to 0 only by the cache reset A 0x10015FA0 (callers: RPC 166's remote kill A 0x10017570 (tail jump),
   the ped deletion of stream-out A 0x1001753D, the remote reset A 0x10016650, A 0x1000A57B, a branch of the remote
   tick A 0x100170FD) and the
   constructor A 0x100165E0. The foot store does not touch it, so the id survives the exit. Implemented as
   `Vehicles::remote_vehicle` (kept until 166 / stream-out / quit).
4. Row 45: answered statically (above).

## Measurements on the original (trace lane, run `orig-20261105-012348`, lag-comp 1, tracegm)

* **(1) Crosshair, row 8 — MEASURED.** Default sa-mp.cfg of the trace prefix (no `nohudscalefix`): with a Deagle,
  `TheCamera.m_aCams[0].m_nMode` 4 → 53 (0x35) while RMB is held; `0x859520/0x859524` stay `0x3ACCCCCD / 0x3B124925`
  before, during (5 samples) and after the aim. So the correction branch does not run with the default
  configuration; our pass-through wrapper is the same behaviour. (Statically: the flag A 0x10117470 is written only by
  the start-up (A 0x100C49EB, cleared unless `nohudscalefix` ≠ 0) and the `/hudscalefix` toggle A 0x10068870.)
* **(2) Remote cuffs — MEASURED.** samp-bot `--special-action 24` (new measurement option) streamed in next to the
  traced player for 6 s, compared with an identical window of a plain bot: no GTA call, task or script command differs
  (no anim, no task, no weapon reset; a Deagle given to the cuffed remote stays armed). The cuff flag only matters for
  the local player (masks). We implement nothing for remote cuffs — same behaviour.

## Parity results

* **Network diff, ours vs the original, per `combat2.sh` part** (`tools/trace/combat_report.py`, lag-comp 1,
  original runs `orig-20261104-231851` / `orig-20261104-234135`, ours `ours-20261105-010855`; 206/115/53/52/204/211 lines,
  203 compared separately because they are timing-driven):
  * identical: `ht1` (206 ×3, 115 taking 46.2 ×3, 53 reason 24 killer 1, 115 weapon 54 3.3, 52), `cross-ht0` (no 206/115
    from us: the local-hit suppression), `proj16/18/39/40/35/36` (204 records only), `rproj39`, `mask-*` (cuffs weapon
    reset), `skills-999/1000`;
  * `rproj16` / `rproj35`: the same pairs of 115 (weapon 51, taking, attacker = bot), but the grenade exploded 1.6 m
    further away (64.76 → 34.32 per event) and ours got 4 instead of 2 damage events for the RPG: GTA's
    `TriggerExplosionSectorList` (caller 0x56822B) visits the ped once per repeat-sector list it is in, and the player
    stands on the 50 m sector corner (1700, −2600). Geometry of GTA's own projectile/explosion code, not client logic;
  * drive-by: the 211 drive-by bit was missing (fixed: `Vehicles::local_drive_by`, also the cuffed bit); 203 pacing:
    the original's camera position jitters in the last mantissa bit, so its identical-looking 203s pass the 501 ms
    duplicate gate in pairs; ours are bit-identical and are gated (≈ 1/s). Same gate, different GTA camera noise.
* **DIFF-3** (`tools/diff/cross-client.sh`, combat steps added: weapon, aim, fire, death, respawn in both directions):
  34/34 enabled step-directions MATCH (run `xclient-20261105-…`, calibration `xcalib-20261104-211825`).

* **Effect-log diff** (`EFX_STEPS="give armed ammo resetw" tools/efx/run.sh both`, new efx steps for RPCs 22/67/145/21;
  orig `orig-20261105-012728`, ours rerun `ours-20261105-014056`):
  * 145 SetPlayerAmmo: empty (both captures).
  * 22 / 67 / 21: the samp slot-context writes (PlayerInFocus := slot/0 and the 11 skill stats saved/written/restored)
    were missing in ours and are now replicated (`combatclient::local_context`); the same GTA call sequence runs
    (`GiveWeapon` → `RemoveWeaponModel`/`Initialise`/`AddWeaponModel`, `SET_CURRENT_CHAR_WEAPON`, `ClearWeapons` →
    13 × `CWeapon::Shutdown` → `SetCurrentWeapon(0)`). Remaining lines are capture-mode artefacts (step capture of the
    handler vs our window capture of the frame: unused stack "arguments", `ScriptParams` / `CommandsExecuted` from the
    other per-frame script commands, streaming internals written by the CD-stream thread) and two same-value writes in
    every one of our windows — `CPlayerPedData` +0x40/+0x41 and `CTimer::ms_fTimeScale` — which come from our
    per-frame `SET_PLAYER_DRUNKENNESS(0, 0)` (pre-M7 code, local tick "drunk handling"); reported to main, not changed
    here (not combat).

## Acceptance results (2026-11-05)

* `tools/e2e/m7.sh` (lane main): PASS in modes 1, 2 and 0: local → bot 3 × OnPlayerGiveDamage 46.200 / weapon 24 with 3
  hit-type-1 206s (mode 0: no 206); 204 view 46 → SetPlayerAmmo 20 → armed 0 → reset 0/0; bot → local: health
  100 → 53.8, 3 × OnPlayerTakeDamage 46.200, OnPlayerDeath(bot, 24), respawn (3.62 s after 53, as measured); mode 0: no
  damage from the key-simulated shots; SetPlayerHealth(0) → OnPlayerDeath(65535, 255) → respawn.
* Regression: m1, m2, m3, m4, m5, m8a, goal, saa all rc=0 after the last change.
* `cargo build --workspace`, `cargo test --workspace` (11 new combat tests), clippy host + i686: 0 warnings.

## Object hits and surf rows (2026-11-06)
* **206 hit type 3** (A 0x10006BC0): after the player and vehicle lookups the hit entity is looked up in the object pool
  (A 0x10012A10: ids 1..=highest in use, wrapper +0x40 = GTA entity; object id 0 is never found). The offset is the
  record's own (entity frame for selector 0, hitPos − position otherwise), as for vehicles. Ours: `combat::object_of_entity`
  over `combathooks::Shared::objects` (pool objects with a GTA entity). Measured (`tools/diff/combatveh.sh`, container
  2932 as object 1, two un-aimed Deagle shots, server default lag-comp): OnPlayerWeaponShot hittype 3 hitid 1 for every
  shot on both clients in three runs, offsets (−1.555, −0.14…, 0.78…) within 0.003 of each other. `tools/e2e/m7.sh`
  checks the same per mode (2 shots, hit type 3 in modes 1/2, no 206 in mode 0).
* **Damage surf rows** (A 0x100A3BF0 rows 3/4): +0x2DA is the samp wrapper of the surfed pool vehicle **or object**; the
  damager is compared with its GTA entity (+0x40). Ours: `Shared::surface` (`client.rs` `update_combat_shared`, the
  surf state's target as a GTA entity) → `combat::surf_inputs`. Static only (no scenario makes the surfed entity damage
  the player reproducibly); unit test `damage_table`.
