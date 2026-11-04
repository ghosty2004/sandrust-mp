# BOT-2 — samp-bot as a scriptable second player

`crates/samp-bot` is now a library (`lib.rs`) plus the CLI (`main.rs`). Prerequisites P-BOT-DRIVE and P-BOT-SHOOT
of `measurement-requests.md` (MR-5.1, MR-5.5, MR-7.2, MR-7.6) are built on it.

| module | content |
|---|---|
| `net.rs` | UDP socket, per-port cipher, cookie handshake |
| `client.rs` | `Client`: RakNet `Connection`, auth key, `0x22`, PlayerConnect (25); turns messages into `Event`s (`Rpc`, `Sync`, `Closed`) |
| `lib.rs` | `Outbox` trait (live client or test recorder), `send_rpc` (NET-7 `OutgoingRpc::SEND_PARAMS`), `send_sync` (`sync::policy::send_params`) |
| `gate.rs` | duplicate suppression (shared 501 ms window, per-type copy) and the 1e-5 rotation jitter filter |
| `world.rs` | remote players (join/quit/stream, 207/200/211 positions) and streamed vehicles (164/165/159/160/147/24, remote 200/209) |
| `weapons.rs` | inventory (21/22/67/145 and spawn weapons), clip/reload, 204 delta encoder |
| `path.rs` | drive paths (circle, 50 m straight + 90° left turn) with an accelerate/cruise/brake speed profile |
| `bot.rs` | class selection, spawn, death/respawn, spectating, the modes and every sync sender |

## CLI

Unchanged defaults: `samp-bot HOST:PORT --name N --seconds S [--class N] [--auth-table P]` walks the 3 m circle.
New: `--mode walk|idle|drive|passenger|shoot`, `--sync-ms MS` (fixed foot/in-car interval, measurements only);
drive/passenger: `--vehicle ID --seat N --path circle|line --radius M --speed M/S --keys N --siren --drive-by
--exit-after S`; shoot: `--target ID|NAME --weapon W --hit-type T --hit-id N --offset X,Y,Z --fire-ms MS`.
At the end of `--seconds` a seated bot brakes, sends 154 and gets out first (up to 6 s more).

MR-5.1 profile: `--mode drive --path line --speed 20 --sync-ms 40 [--keys 2|128]`. Run it once more with `--sync-ms 150`.
MR-7.2: `--mode shoot --target <traced> --hit-type 1` and `--hit-type 0`. MR-7.6: `--mode passenger --drive-by`.

## Spec rules implemented

* **Send parameters**: RPCs through `OutgoingRpc` (26/154 rel-sequenced ch 0, 115/52/53 rel-ordered, 128/129 reliable),
  sync through `policy::send_params` (200/203/207/211 unrel-sequenced ch 1, 206 ch 0, 204/205 unreliable, 212 ch 1).
* **Rates** (remote-players-sync §0.4): interval = rate + N streamed players (15 ms in LAN mode); 207 when keys/lr/ud
  change or the interval elapsed; 200/211 when elapsed > in-car interval; 203 on foot every 100 ms with FIRE/AIM
  (weapon rate if lag-comp selector is 2), else 1000 ms; driver 203 after each 200 for models 432/564/407/601, plus a
  1000 ms tick; passenger 203 every 1000 ms (100 ms with drive-by); 205 every > 1000 ms; 204 every tick (100 ms throttle,
  sent only on a target or slot change); 212 while spectating.
* **Duplicate suppression** for 207/203/200/211 (shared timestamp) and the rotation jitter filter for 207/200.
  Consequence: an idle bot sends 207 about every 0.5 s, and identical idle 203s are mostly suppressed.
* **Enter (vehicles.md 26 C→S, remote-control-vehicles §5)**: the bot runs to the vehicle, then sends 26 (passenger = 0
  driver / 1 passenger; the passenger poll only fires when the vehicle is < 8.0 away). The driver gate refuses (no RPC)
  for doors locked by RPC 24 and for model 570. After 26 the ped walks to the door with special action 3, plays the
  get-in animation, and then sends 200 / 211. **The server does not answer 26 to the requester**: open.mp checks
  stream-in and on-foot state, calls OnPlayerEnterVehicle and relays RPC 26 to the *other* players. The client does not
  wait for a reply either. The server learns the seat from the first 200/211 (state DRIVER/PASSENGER).
* **Exit**: brake to a stop, then send 154 (not for RC models). The leave-car task keeps 200/211 going, then 207 from the
  door. RPC 71 runs the same task without 154. RPC 70 warps into the seat (seat 0 driver, n passenger n; 211 seat = n).
* **211**: seat 1-based (open.mp rejects 0 and seats above the model's capacity); bit 6 drive-by only with a weapon
  other than 0/46.
* **200 fields**: keys accelerate = 8, brake = 32, plus `--keys`; left stick steers; velocity in GTA units per 1/50 s;
  vehicle health from 164/147; trailer 0; siren 0/1 from `--siren`.
* **Shooting**: the weapon must come from the server (22 / spawn weapons; the bot only switches to a carried weapon).
  FIRE is held only with ammo. 206 per hitscan shot (22–34, 38), never for a sniper miss. Offset in the target's local
  frame (lag-comp selector 0) or hit − target position. 115 (taking = 0) after a player hit, damage = GTA damage × 0.33,
  torso. 203 weaponState = 3 while reloading, else min(clip, 2). zoom = trunc((70 − 35)/35·63) = 63.
* **Victim**: an incoming 206 with hit type 1 on our id → armour then health, 115 (taking = 1, attacker id). At health
  ≤ 0: one forced 200 if driving, 53 (reason/killer from the last damage per player-state.md 53), and a respawn (52)
  after the wasted sequence, or class selection after RPC 74.
* **Class selection**: 129 only ≥ 2000 ms after class selection began. Spawn clears weapons and gives the spawn weapons
  (slot 3, 2, 1).

## Plausible values (spec silent; replace when measured)

| item | value used |
|---|---|
| run speed to a vehicle | 4.0 m/s, `upDown` −128 |
| distance at which enter is pressed | 4.0 m (driver; GTA's own search radius unknown) |
| get-in / get-out animation | 1500 ms / 1200 ms; enter task given up after 6 s without reaching the door |
| door positions | ±1.4 m to the side, +0.4 m (front) / −0.9 m (rear) along the car |
| special action during exit | none (the ped is still seated; 4 is never sent) |
| drive profile | accel 5 m/s², brake 8 m/s², default cruise 5 m/s on a 10 m circle; line: 50 m + 90° turn of radius 20 m |
| steering stick | −128 × min(1, 5 m / radius); 0 on straights |
| landing gear | 0 for every model |
| 200 `extra` | bikes: f32 lean = stick/128 × 0.5; trains: f32 speed in units per frame; Hydra and others 0 |
| aim camera | on foot idle mode 4, cam 3.5 m behind + 1 m up, looking slightly down; aiming mode 53 (sniper 7, RPG 8, HS rocket 51, camera 46), cam 2 m behind, 0.5 m right, 0.7 m up, front toward the target's torso; in a car mode 18 (drive-by 55), cam 6 m behind + 2 m up |
| aim sway | ±0.004 / ±0.003 sinusoid on the aim front vector while aiming (without it every aim struct is identical and the 501 ms gate suppresses them) |
| aimZ | asin(front.z) while aiming, else 0 |
| aspect | 198 (16:9) |
| bullet origin | ped + 0.45 m forward + 0.15 m right + 0.55 m up; hit point = target position + 0.3 m z |
| hit type 0 | hit position = target torso, offset = the same world point, hit id 0xFFFF |
| weapon table | GTA damage (SA-MP reported × 0.33), fire interval, clip at skill 1000, reload: `weapons.rs::info` (e.g. Deagle 140 / 800 ms / 7 / 1300 ms) |
| aim raise before the first shot | 300 ms |
| 204 target player | the shoot target while AIM is held (lock-on target semantics unknown), actor 0xFFFF |
| wasted sequence | 4000 ms, then respawn |
| animation field of 207 | 0 (the real client reports its current animation) |
| 212 position | ped position (the camera position from 157 etc. is not tracked) |
| order within a tick | 206 (+115) before 207 before 203 |
| foot-sync timer | measured from the last attempt (sent or suppressed) |

Not implemented: 209 unoccupied sync (the bot simulates no physics), 210 trailer sync, 106 damage status, surfing,
special actions other than 3, melee/projectile weapons (keys only, no 206).

## Verification

* `cargo test -p samp-bot`: 18 tests, incl. offline state-machine tests with a recording `Outbox` (26 before 200, 200
  only after the get-in time, special action 3 while entering, in-car interval, braking → 154 → 200 → 207; 211 seat and
  duplicate suppression; shoot cycle 206 + 115, 203 every ≥ 100 ms, 204 deltas and target; idle 207 ≥ 501 ms, 205;
  victim 115 / 53 / respawn 52). `cargo clippy -p samp-bot --all-targets`: 0 warnings.
* `tools/e2e/bot2.sh` (lane `bot`, UDP 7799, `tools/e2e/probe/bot2helper.pwn`), 2026-11-04: **PASS**.
  Drive: OnPlayerEnterVehicle(driver) → state 2 → vehicle up to 20 m from its spawn → OnPlayerExitVehicle → state 1.
  Passenger: enter(passenger = 1) → state 3 → exit. Shoot: 14 OnPlayerWeaponShot (weapon 24, hit type 1, hit id =
  target, offset 0,0,0.3), each with OnPlayerGiveDamage and OnPlayerTakeDamage 46.2. Clip 7 then a reload gap.
* Ad hoc on the same lane: the default walk is ONFOOT and moving. `--path line --speed 20 --sync-ms 40`: 50 m straight
  north, then a turn to the west (server-side GetVehiclePos), then exit.

`examples/drive_bot.rs` (TRACE-4) is still there. `--mode drive` plus RPC 70 handling covers it.
