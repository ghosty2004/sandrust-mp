# Phase 3 / FEAT-A — network-visible features from the completeness audit

Branch `network-features` (worktree `samp-rs-wt/feata`), lane veh (prefix-shot, gta-sa-shot, UDP 7820, control
47720/47722). Items FEAT-1, -2, -3, -4, -6, -9 of docs/phase3/feature-sweep.md (rows A1, A2, A5–A10, A19). Static
reading of samp.dll build B (REA/Ghidra + objdump), behaviour described in our own words, addresses in the code.
Measured on the ORIGINAL client first: an exploration session in lane veh (`M6P_CLIENT=orig tools/e2e/m6p-hold.sh`,
`target/vehroot/target/trace/orig-20261105-094437`), then the scripted parity scenario in the trace lane.

## What the original does (and we now do)

| item | behaviour (build B) | ours |
|---|---|---|
| FEAT-1 SCM event 96 C→S | samp's opcode 0x09C6 (types 1–4 only) turns the script's vehicle handle into the pool id (none → nothing sent) and sends RPC 96 (u32 id, arg1, arg2, type; reliability 10, channel 0) — B 0x10001CE0. Type 3 then applies the colours to the pool vehicle (GTA bytes +0x434/5 and the cached colours). Type 4 arg1 ≠ 0 sets the vehicle's "in mod shop" flag (the per-frame colour re-apply pauses); arg1 = 0 clears it and caches the vehicle's current GTA colours. | `scmops.rs` queues `HookEvent::Scm`, `vehicles.rs::scm_event_out` sends in the same frame's tick |
| 0x09D3 (fixed) | condition **false** while the local ped drives a vehicle that is not a train carriage (569/570), and when the in-vehicle flag is set without a vehicle; true otherwise (B 0x10001C30 → 0x100ABE80 / 0x100ABFD0). Our earlier version had it inverted: carmod1.scm then never closed the mod-garage door (camera switched, door stayed open), so no mod shop worked in our client. | `scmops.rs::op_09d3_result` + tests |
| FEAT-2 170 / 168 | RPC 170 stores the enable bit. Every spawned local tick (right after the interior check): disabled → cached ids := none; enabled → at most every >100 ms (GetTickCount) the ray from camera source + front to source + 70·front, shortened per category by `CWorld::ProcessLineOfSight` (objects: buildings; vehicles: buildings+objects; players/actors: buildings+vehicles+objects); objects: the pool ray test B 0x10012E40 (start distance 40); vehicles: live, ≤ 40 m from the camera, not the local ped's own car, bounding-sphere test ×1.0, nearest to the ray start; players and actors: ≤ 20 m, ×1.5. RPC 168 (four u16, reliability 9) only when one id changed. | `camtarget.rs` (pure state machine, tests), `client.rs::camera_targets`, `objects.rs::{pick_within, bound_sphere_hit}` (shared with selection mode, which gained samp's RW-object and model-19300 filters) |
| `/ctd` | B 0x10004830, early in the spawned local tick, inside GTA's scene: one sprite begin; per cached id whose entity exists the editor's axis cross on the entity matrix (B 0x10071D50, length 1, 0xFFCC9999) and `CAMERA_TARGET_TYPE_<KIND>: ID(<id>)` in 0xFFD47B75 at the matrix position (3D-text painter, outline, no LOS); one sprite end. Only with 170 enabled (else the cache is "none"). | `render.rs::camera_target_labels`, `glue.rs::draw_camera_target_labels` |
| FEAT-3 F4 | at the end of the local tick (not while spectating unspawned): if the "class selection after next death" flag (the one RPC 74 sets) is clear and `GetAsyncKeyState(VK_F4) != 0` (either bit) → set it and print "Returning to class selection after next death" (type-4 line). Measured: two presses → one line; the second press's "pressed since" bit stays pending until the flag is consumed and re-arms it once after the class selection (second line). The next death → class selection (RPC 128) instead of the respawn (RPC 52). | `client.rs` (`F4_LINE`, `async_key_any`) |
| FEAT-4 23 | the scoreboard's GUI callback (B 0x100C5590) on DXUT event 0x701 (list-box double click; the list never has keyboard focus — measured: Down/Enter do nothing) → B 0x1006EBD0: atol of the selected row's id text, RPC 23 (u16 id, u8 source 0, reliability 9), scoreboard closed with cursor mode 0 and the delayed (10-frame) input restore. | `glue.rs::scoreboard_click`, `core.rs::scoreboard_activate` |
| FEAT-4 119 | in the Present pass while GTA's pause menu is up (B 0x10075B7B): map page (0xBA6774), `GetAsyncKeyState(VK_RBUTTON) & 1`, 0xBA8298 = 0 → radar-trace entries 1..=175 (0xBA86F0, stride 0x28) scanned for u16 0x1203 at +0x25 (last match wins), x/y from +8/+0xC, z = `CWorld::FindGroundZForCoord(x, y)` + 2; RPC 119 (reliability 8) at once (RakNet's thread transmits it). Measured: a right *press* held ≥ 0.3 s; GTA places the waypoint in the same frame, the message leaves while the menu is still open. | `glue.rs::map_click`; sent and flushed from the Present pass (`dll::send_click_map`) because our session tick does not run while the menu is up (queuing it delayed the RPC until the menu closed — found by the acceptance test) |
| FEAT-6 jack | the remote driver store (B 0x100172F0) calls B 0x10016220 on a state change to driver (19): from on foot (17), with the local ped driving the vehicle the remote now drives → WARP_CHAR_FROM_CAR_TO_COORD to its own position + 1 m z, CLEAR_PRINTS, AddBigMessage("~r~Car Jacked~w~!", 1000, 5). Order: store, seat warp, then the check. | `vehicles.rs::car_jacked` (the only change in the driver store: the old state is kept for the check) |
| FEAT-9 112 | B 0x100A17A0: the scanner's stand-in ped (CSV row 55, 0x7A0 bytes) zeroed; 39 bytes at 0x4E7529 NOPed (row 361: the on-foot description in `AddAudioEvent`); in a vehicle with 400 ≤ model < 612 a temporary vehicle (samp's vehicle constructor at 0,0,0: busy load, CREATE_CAR, heading 0, 09C4/07FF/053F 0, door lock / dirt / z + 0.25, colour (colour, 0xFF)) is removed from the world and becomes the stand-in's vehicle (+0x58C, flag +0x46C bit 8); then the local player's `CWanted` scanner (+0x21C) gets `AddAudioEvent(0xA4, crime, pos)`; the temporary vehicle is deleted (DELETE_CAR). `suspectId` is unused. | `client.rs::crime_report`, `vehicles.rs::create_detached` |
| FEAT-9 60 | B 0x100A1760: only while the HUD clock is off, `CTimer::m_snTimeInMilliseconds` (0xB7CB84) := time & 0x3FFFFFFF (plus a write-only flag). open.mp sends it on join and periodically. | `client.rs` |
| FEAT-9 48 | B 0x1001DC70 stores the world in the custom-model manager (0x1026EA10), which R4 never creates (session-misc.md "179") → no effect. open.mp does not even send it to 0.3.7 clients (`setVirtualWorld`). | logged only |

## Scenario and tools

* `tools/e2e/featscen.sh` — the shared scenario (gamemode tools/trace/tracegm.pwn, filterscript tools/trace/uifs.pwn,
  1280x720): F4 ×2 + kill + Shift; camera target on a vehicle (416), object, actor, bot player, `/ctd` (screenshots,
  D3D capture); scoreboard double click on the bot's row; Esc → Map → right press; crime report (suspect on foot);
  Car Jacked (a `samp-bot --mode drive --vehicle V` enters the ambulance the local player drives) + crime report
  with the suspect driving; Wheel Arch Angels with a Sultan (throttle taps until inside, handbrake taps until the
  garage closes; Space ×2 preview paint job 1 → 96 types 1 + 3, Space buy, Enter leave → 96 type 4); RPC 48.
* `tools/e2e/feata.sh` — acceptance on our client (asserts every callback / client-log line).
* `tools/e2e/featparity.sh orig|ours` — the same scenario traced (orig: trace lane; ours: lane veh via the
  m6p-hold re-rooting; `FEAT_LANE=veh` runs the original in lane veh for exploration).
* `tools/trace/uifs.pwn` — `OnPlayerClickPlayer`, `OnEnterExitModShop`, `camtgt|P` (the four
  GetPlayerCameraTarget* natives; open.mp's `GetPlayerCameraTargetPlayer` reads the aim target, not the 168 field,
  so the player target is checked in the capture / client log).
* `tools/diff/netscen.sh` — new windows f4, camtarget, scoreboard_click, map_click, modshop (before quit);
  `tools/diff/network.sh` gained `NET_OURS_LANE=veh` and `NET_GAMEMODE=tracegm`.
* Avoided vehicles with a music radio where the car is not the subject (GTA's radio stream thread divides by zero
  at 0x4F1464 under Wine, ROADMAP "m5 occasional failure"); the mod shop needs a Sultan.

## Verification
Parity runs (same scenario, tracegm, 1280x720): original `target/trace/orig-20261105-103210` (trace lane), ours
`target/vehroot/target/trace/ours-20261105-103212` (lane veh).

* C→S RPCs of the game client (net-*.log, everything but 25/155): **identical** sequence incl. payloads —
  `128 129 52 53 115 128 129 52`, 168 ×7 (`ffff0500ffffffff` vehicle 5, none, `0100ffff…` object 1, none,
  `…ffff0000` actor 0, none, `ffffffff0100ffff` player 1), `23 010000`, `119 0000df4400e018c500e06841`
  (1784, −2446, 14.55), 96 type 4 (1, 3) / 118 03 / 96 type 1 / 96 type 3 (1, 1) / 96 type 4 (0) / 118 00.
* Server callbacks (class / spawn / state / camtgt / click-player / click-map / modshop / paintjob / respray):
  identical.
* Tracer (`tools/trace/diff.py --from crime_vehicle`): CREATE_CAR 416 at 0,0,0, 0175, 09C4/07FF/053F, CWorld::Remove,
  `AddAudioEvent(164, 14, pos)`, `PlayPoliceScannerDialogue`, DELETE_CAR — identical. Car Jacked: 0362 to (x, y, z+1),
  00BE, `AddBigMessage("~r~Car Jacked~w~!", 1000, 5)` — identical. Remaining lines in these windows are other
  per-frame records (ambient objects, frame alignment).
* `/ctd` D3D (`tools/trace/d3d_diff.py diff … --only 'sprite\.|font\.'` on d3d-ctd_vehicle.jsonl): 189 calls,
  identical except the caller module name (samp.dll / samp_client.dll); same label rect [640,359,641,361].
* RPC 60: `CTimer::m_snTimeInMilliseconds` peeked twice per client in lane veh — both at server-clock ms & 0x3FFFFFFF
  (offset to the host clock 620896.70 s on both).
* `NET_OURS_LANE=veh NET_GAMEMODE=tracegm tools/diff/network.sh` (orig `target/netdiff/orig-20261105-103233`, ours
  `…/ours-20261105-104711`): **checks 403, matching 403, accepted 0, open 0** (exit 0). New windows: f4 (53 > 115 >
  128 > 129 > 52), camtarget (168), scoreboard_click (155 155 23), map_click (119, trigger +0.040 / +0.030 s),
  modshop (96 > 118 > 96 > 96 > 96 > 118). The first ours session hit GTA's radio division by zero in the Sultan
  (ROADMAP "m5 occasional failure"); the rerun was clean.
* `tools/e2e/feata.sh` (lane veh): PASS (f4 ×3, camtgt ×4, click_player, click_map, crime ×2, jack, modshop ×3, vw).
* e2e suite on this branch, lane veh: m1, saa, m2, m3, goal, m4, m5, m6, m7, m8a, m8b, m9 (M9_ORIG = main's
  target/m9-orig), m10 — all PASS.
* `cargo test --workspace` 584 passed; clippy host (all targets) and i686 (samp-client, tracer, launcher) 0 warnings.

## Files

`crates/samp-client/src/`: `camtarget.rs` (new), `client.rs` (F4, 170/168 tick, `/ctd` items, 60, 48, 112, 119
send), `scmops.rs` (09C6 → hook event, 09D3 fix), `vehhooks.rs` (`HookEvent::Scm`), `vehicles.rs` (96 sender + type
3/4 effects, Car Jacked, `create_detached`, `sent_96`/`jacked` in the `vehicles` header — the only vehicle changes;
the driver store keeps the previous state for the check), `objects.rs` (`pick_within`, `bound_sphere_hit`,
`pick_list`), `ui/glue.rs` (scoreboard click, map click, `/ctd` draw entry), `ui/core.rs` (`ClickPlayer`,
`ClickMap`, `scoreboard_activate`), `ui/render.rs` (`camera_target_labels`), `dll.rs` (`send_click_map`,
`scanner_dummy_ped`), `world.rs` (`TIMER_MS`), `behaviourclient.rs` (`big_message` crate-visible), `lib.rs`.
Tools: `tools/e2e/{feata,featscen,featparity}.sh`, `tools/trace/uifs.pwn`, `tools/diff/{netscen.sh,network.sh,
net_report.py}`. Docs: this file, ROADMAP (lanes, queue, FEAT-D1..D3), WORKLOG, feature-sweep.md status,
player-state.md (F4, 23, 119, 168 details), vehicles.md (0x09D3).
