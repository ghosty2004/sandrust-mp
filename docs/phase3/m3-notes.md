# Phase 3 / M3 — remote players on foot (in game)

Status: `tools/e2e/m3.sh` and `tools/e2e/goal.sh` pass in lane "main" (server 7777, control 47700, headless
gamescope). `m1.sh` and `m2.sh` still pass. Typical run: samp-bot "RemoteBot" is brought 5 m in front of our
player, streams in within about 1 s, and gets ped slot 2. Its in-game position is 0.1–1.9 m from the probe's
server-side position, which is up to 1 s old. The ped walks about 8–10 m of sampled path in 5 s. The bot quits,
and the ped is deleted in the same frame as RPC 138.

## Pieces (crates/samp-client)
* `sectors.rs`: relocation of `CWorld::ms_aSectors`, table-driven from the annotated CSV. The table has 84
  references: 13 absolute pointers (rows 262–275 without 269), 55 imm32 (rows 276–331 without the duplicate 325),
  11 for the +4 field (rows 332–342), row 138 (repeat sectors → buffer + 115200) and rows 343–346 (loop bounds →
  buffer end).
  * The buffer is 160000 bytes, leaked from the heap. Row 347 then zeroes the original 115200-byte grid.
  * The relocation is all-or-nothing: every original is checked before anything is written.
  * Tests check every row against the CSV (address, size, original bytes, samp's target relative to its buffer) and
    against `gta_sa.exe`.
  * It runs at gGameState 7, first in the main block (gta-patches.md §2 step 3).
* `patches.rs` (Main phase): row 178 (`DeleteAllRwObjects` := RET), the ped-related pools 110–118 (ptr nodes, entry
  info, peds 240, ped intelligence 240, tasks 1524, events 456, 6000 matrices), and row 183 (no ped group in the
  `CPlayerPed` ctor). Also:
  * the `Players[-1]` / NULL-player-info NOPs 193, 173–176;
  * 206 (`UpdatePosition` ignores standing-on entity);
  * 213/214 (heading-goal stores NOPed);
  * 162 ("playidles");
  * 167 (`ProcessAnimGroups` anim-block switch NOPed).
  * The CSV/exe test helpers moved into `patches::testutil`.
* `swap.rs`: hooks on the vtable slots 0x86D190 (`CPlayerPed::ProcessControl`, row 19), 0x86D744
  (`CTaskSimpleUseGun::SetPedPosition`, row 21) and 0x86D194 (`CPlayerPed::ProcessCollision`, row 23: skipped
  while entity flag bit 31 is set, otherwise `CPhysical::ProcessCollision` 0x54DFB0).
  * Remote ped (found in the slot table, `PlayerInFocus == 0`): the hook saves `Pads[0]`, Cams[0] looking-left/right,
    the camera mode (byte) + `m_PlayerWeaponMode` (4 → 0), FOV (zoom×35+35), the aspect ratio (+1), the Cams[0]
    Front/Source/SourceBeforeLookBehind/Up vectors, the 11 weapon-skill stats and `PlayerInFocus`. It writes the
    slot's values, NOPs 0x5E92F4 (ProcessControl only), calls the original, and restores everything.
  * Local ped: the 0x6884C4/0x688200 stores are restored around the original call, then NOPed again (row 367).
  * The three toggle sites are made RWX once at install. After that, toggling is plain stores.
* `remote.rs` (pure, unit-tested):
  * slot allocator 2…209 (first free);
  * per-slot shadow defaults (combat §1.2: camera mode 4, zero aim block, zoom 1.0, aspect 0.3333, skills 999 / the
    stream-in values);
  * key → shadow-pad mapping with the edge rule (§2.1);
  * receiver quaternion → matrix rows, D3DX-style slerp, heading from forward with samp's degree → radian rule;
  * position correction (dead zone 1e-5, nudge 0.1 × error, teleport if Δx > 2, Δy > 2 or Δz > 1, or not in the
    moving list);
  * timestamp late-drop, 207 store, 3000 ms stale rule;
  * the `remote` reply line.
* `players.rs` handles 137/138/32/163 (+11 name) and the 207/203 store, then runs the per-frame apply from the
  client tick. That is before the local player's tick, as in samp's netgame order, so after `CWorld::Process`.
  * Stream-in follows the trace order: `CREATE_PLAYER(slot,x,y,z)`, `GET_PLAYER_CHAR`, 087E(1), 02AB(0,0,1,0,0),
    0446(0), 03FE(0), 03FE(death-drop money). Then the direct skin change (shared `client::set_ped_skin`), both
    heading fields, and the matrix z at +0.15. Then the slot is bound in the swap table.
  * Destroy (163 and 138 share it): `m_pPlayerData(+0x480)->+0x4C := 0`, then the scalar deleting destructor
    (vtable slot 0). After that `Players[slot].m_pPed := 0` and the slot is freed.
  * Per frame, on foot:
    * on a new 207: move speed := synced velocity, then the correction;
    * rotation slerp 25 %, written to the matrix rows and both heading fields;
    * head look (not for NPCs): every 1000 ms the offset := 20 × the slot's camera Front; every 200 ms
      `IKChainManager_c::LookAt(g_ikChainMan, "FollowPedSA", ped, null, 1000, −1, &(pos+offset), 0, 0.1, 500, 3, 0)`;
    * keys → shadow pad (sticks zeroed when stale);
    * collision gating (§3.6): not surfing → on (flags &= ~(bit31|bit1), |= bit0), unless special action 3 →
      off; surfing a vehicle → off; surfing an object → unchanged;
    * zero synced velocity → move speed 0;
    * stale → stop and snap;
    * health := 1000 (state ≠ wasted).
* `netclient.rs`: decodes sync packets (`samp_proto::sync::decode`, incl. the 0x28 timestamp) into `Event::Sync`.
  `net` shows `syncs_in`.
* The on-foot send interval uses N = used ped slots (`Players::streamed`).
* Control channel `remote`: a header line `remote count=N streamed=S process_control_calls=R/L`, then one line per
  player with id, name, slot, ped, pos (ped matrix), heading, synced health, ped health, state, sync age, synced
  position and the teleport count.

## Tests
* `tools/e2e/probe/m3helper.pwn` (new filterscript, next to e2eprobe). 1.5 s after a "RemoteBot*" spawns (after
  gungame's own random placement), it puts the bot 5 m in front of the first spawned non-bot player, in the same
  interior and virtual world, and logs `[m3] bring`.
* `tools/e2e/m3.sh`:
  1. build;
  2. start the server with both filterscripts;
  3. start our client and spawn it with Shift;
  4. start `target/debug/samp-bot --name RemoteBot --seconds 45`;
  5. check `remote`: slot ≥ 2 with a ped; 3 of 4 position samples within 4 m of the probe; ≥ 2 m path in 5 s;
  6. take screenshots `target/m3-remote-{1,2}.png`;
  7. after the bot's run ends (or a kill by PID), its line must disappear;
  8. quit.
  It never uses `pkill` for the bot.
* `tools/e2e/goal.sh` = m3.sh with `M3_WALK=1`: after the spawn, hold W and require that the probe sees us ONFOOT
  ≥ 1 m away (as m2), then run the remote-player part. Output goes to `target/goal*`.

## Deviations (on purpose or not yet)
* No SEH containment around the original ProcessControl. samp catches a fault there and streams the player out;
  Rust cannot catch SEH, so a fault ends the game.
* Not implemented yet:
  * surfing (0x5689FD hook);
  * the cuffs/carry bone pass (0x53EA03);
  * weapon sync, the 207 animation field, special actions, head look;
  * fighting style, colours / blips / name tags;
  * the CJ-walk anim-group hook (rows 43/187);
  * vehicles (0x56E0FA, 0x6BC9EB);
  * pool rows other than the ped-related ones.
* `Players[slot].m_pPed` is cleared after deletion; samp leaves it dangling (main-session decision after TRACE-2).
* Every swapped global is restored to its saved value (samp writes `PlayerInFocus := 0`). There are no
  register-clobbering stubs.
* The swap copies the slot data out of a mutex (`try_lock`). If the table were busy, a remote ped would run one
  frame unswapped. This has not been observed.

## Observations
* The trace-observed values inside the remote ProcessControl match: focus 2, pistol skill 999, Front zero, mode 4,
  weapon mode 0, FOV 70, aspect 1.333. `process_control_calls` shows about one remote call per frame.
* Re-diff against the original by TRACE-4 (ours-20261104-200349 vs orig-20261104-195734). These match: the
  stream-in GTA sequence, the full swap, the write timing, big-jump writes, the destroy order and slot reuse.
  Follow-ups:
  * Head look was missing. Added; the trace shows one `SetTaskSecondary(CTaskSimpleIKManager)` after stream-in.
  * Speed magnitude: the original writes |v| ≈ 0.05 (synced 0.03 + correction) and tracks the 3 m circle at radius
    2.48. Ours writes ≈ 0.030–0.032, so our ped is almost on target at every packet. Checked against samp A
    0x10014C40: the gain is 0.1 (0x100FE4B0), the dead zone 1e-5, the thresholds 2.0/2.0/1.0. The order is
    velocity store, then correction, with the nudge added to the current move speed; this is identical. Collision
    gating, which samp runs every frame and we lacked, is now added; it did not change the measured |v|. The
    remaining difference is in GTA-side physics between packets (the original ped falls behind its velocity). The
    likely candidate is samp's `CGame::Process` wrapper, which rewrites `CTimer::ms_fTimeStep` every 250 ms
    (A 0x100A1C50, not analysed or hooked). Open; needs a per-frame speed/position trace in both clients.
  * Packet-apply cadence: we drain the socket completely every frame and apply the latest 207 once per frame, as
    samp does. Burstiness (gaps of 3–4 frames) comes from arrival times. Draining more often would not change it,
    because application happens only in the frame tick. Open.
  * `SET_CURRENT_CHAR_WEAPON` flapping (1,22)↔(1,4) after the arrest restart is not a client loop. GTA's restart
    removed our weapons, so our 207 reports weapon 0. gungame's `OnPlayerUpdate` then alternates
    `SetPlayerArmedWeapon(knife)` / `(level weapon)` every update, and we apply each 67 with 01B9, as samp does
    (non-forced variant). It disappears with SP-OFF (no police / arrests); M3 runs are short (about 60 s after
    spawn) and did not hit it.
* The first 207 after creation teleports once (creation position ≠ synced position). After that, movement is
  velocity + nudge only. When the walk circle runs into a wall, GTA collision holds the ped back until the error
  exceeds 2 m, and then it teleports. This matches samp, which has no collision gating on foot.
* The bot sends `up_down = -128`, so GTA plays the run animation while the synced speed is 1.5 m/s.

## Next
* The speed-magnitude difference above (needs per-frame physics traces); samp's timestep rewrite (0x100A1C50).
* Name tags, markers and colours (M4/M8); 203/204 senders and weapon/aim application (M7); vehicles (M5).
* ProcessCollision gate, surfing, special actions, animation field (needs samp's anim table loader).
* SEH containment (vectored handler, or a small asm shim around the original).
