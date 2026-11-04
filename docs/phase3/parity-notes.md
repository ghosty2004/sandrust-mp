# Phase 3 / M2-P + M3-P — local player and remote ped parity

Branch `player-parity`. Every change below is backed by static analysis of samp.dll A (decompiled exports +
objdump; addresses are A unless marked B) and/or by an earlier measurement of the original client; nothing is
filled in by guess. Open items are listed under "Open / measurement requests".

## Local player

| item | original (evidence) | ours now |
|---|---|---|
| Clock | first frame (A 0x100c47d0 → 0x100a0b60(0), 0x100a14a0(0)): `CClock::Update` := RET (CSV row 354), HUD clock string zeroed (row 355); netgame ctor: settings hour 12 / minute 0 / weather 1 / "server holds the time" | `world.rs`; per frame (netgame tick A 0x1000b5b0) clock bytes + `SET_TIME_OF_DAY(hour, minute)` only while the server holds the time |
| Weather | forced while connecting (weather 1, A 0x100089b0), InitGame weather (A 0x10010660), RPC 152; with the clock off also old/new weather (A 0x100a0ba0). DIFF-2 idle: original `10/10/forced 10`, ours cycled | same writes (`world.rs`) |
| RPC 29 / 30 / 94 | 29: clock bytes + `00C0` + settings; 30: toggle + flags, off copies GTA's time into the settings (A 0x1000f500); 94: settings hour only | same |
| Join timing | netgame created on the 2nd `Render2dStuff` call; first connect attempt 3000 ms later (A 0x10008940, prints "Connecting to …"). Trace: original joins at frame 271, ours at 50–76 | session start prints "Started", the connect (and the "Connecting to" line) waits 3000 ms |
| Per-frame commands while spawned | local tick A 0x100074c0, trace order per frame `052C(0,0)`, [`09BA`], `0470`, `077E`, `077E`, [`0470` on a 207 send] (TRACE-1 "Walking"; orig-20261104-205141) | `Client::local_frame` + `sync_tick`: 052C (drunk path A 0x100ade30 with level 0), 09BA(InitGame zone names) from spawn + 1000 ms (A 0x10003c20 sets +0x30a), 0470 (the 204 check A 0x10006290; the 204 packet itself is not sent yet), 077E + RPC 118 on change (A 0x100a0f20 / 0x10005970), 077E on foot (bounds check A 0x10003f50; the bounds themselves are M8b), 0470 in the 207 builder (A 0x10004f00) |
| Stock pickups | CSV rows 172 (`CFileLoader::LoadPickup` := RET) and 208 (no `Interior_c::AddPickups`). DIFF-2 idle: ours had 4 (358 sniper, 3 × 366) | applied (patches.rs) |
| CJ walk | rows 187 (NOP the move-anim-group store) + 43 (call A 0x100A3F60: store only if InitGame "CJ walk"); DIFF-3: samp walks skin 60 with group 118, ours with 54 | `pedhooks.rs` (naked stub, flag from InitGame) |
| RPC 153 | invalid model → "Warning: SetPlayerSkin %d isn't a valid ped model." (A 0x10019190; valid = model-info vtable 0x85BDC0, A 0x100b3e10); local or streamed-in remote ped; skin path A 0x100aff90 deletes the IK secondary task first (A 0x100ae060) | same (`client::set_ped_skin` shared by local/remote/stream-in) |
| RPC 88 | A 0x100030f0: 10 → `TASK_HANDS_UP(ped,−2)` if on foot; 11/13 phone (`TASK_USE_MOBILE_PHONE`, flag +0x48); 0 stops phone (and states we never create) but **not** hands up (DIFF-3: the original still reports sa=10 after `SetPlayerSpecialAction(0)`); spawn calls it with 0 | same for 0/10/11/13; hands up + Triangle → control on (teleport in place) as in the local tick |
| 207 special action | A 0x10003410 order: jetpack 2, dance, hands up 10 (primary task vtable 0x85A29C), phone 11, piss, duck 1 … | 2, 10, 11, 1 |
| Class-selection input lock | measured (trace lane, orig-20261104-230048, peeks of 0x541DF5): the keyboard→pad call is NOPed from the join frame (250) on and restored at frame 456, 10 frames after the spawn frame 446. Static: class selection requests input-lock mode 2 every frame while the menu flag (read as a dword at 0xBA67A4, A 0x100a0980) is 0 (A 0x10006100); the spawn requests mode 0 with the 10-frame delay (A 0x100a0750(0,0)). Without it the spawn Shift reached the pad in ours: a jump (`CTaskComplexJump`) and a 3 m move after `SpawnPlayer` (DIFF-2 flow_spawn, COV-1 item 7) | `ui::class_selection_lock` / `class_selection_release` (glue on the M4 `InputLock`) |
| HUD / radar flags in class selection | measured: 0xBA6769/0xBAA3FB flip to 0/1 only inside the frame and read 1/0 otherwise: samp's tick hides the HUD (A 0x10006100 / 0x100089b0 → 0x100a1df0(0)) before the original `Render2dStuff`, and its UI render (A 0x10075790) sets it back every frame from the chat display mode | ours hides it after `Render2dStuff` and keeps it hidden (tracer watch: `script_no_radar=1` through class selection). Visible result the same; the per-frame order (samp: netgame tick before `Render2dStuff`, ours after) and the UI-render HUD restore belong to the frame hook / M4 UI — open, see "Open" |
| Systems only our client ran (COV-1) | CSV rows 158 (`CPlantMgr::Update`), 159 (`CCoronas::DoSunAndMoon`), 165 (real-time shadows), 155 (ambient gunfire), 192 (`ClearPlayerWeaponMode`), 244 (idle camera), 172/208 (IPL/interior pickups) | applied (patches.rs) |
| Local head look | A 0x10002fa0: on foot, driving or as passenger, "disableheadmove" off (Game +0x59 = 1 from the constructor), no drink item, no object edit: every 1000 ms offset := 20 × camera front (0xB6F32C), every 200 ms `LookAt("FollowPedSA", ped, 0, 2000 ms, −1, pos + offset, 0, 0.1, 500, 3, 0)` (COV-1: 39 IK functions only in the original; DIFF-2: the `SetTaskSecondary(…, 5)` after skin changes) | `Client::head_look` |
| Frame timer / limiter | A 0x100A1C50 (see "Speed magnitude"); Game constructor (A 0x1009ffe0): frame limit +0x5d = 90 until the first-frame init sets sa-mp.cfg `fpslimit` (only when in 20..90) | `frametimer.rs` in the `CGame::Process` hook; measured afterwards: frame time median 19 ms in both clients (was 17 ms in ours), remote apply cadence like the original |
| Connect cookie | capture orig-20261104-204143 (game port 59452): request at 0 s, cookie at 0.001 s, the answer only with the next 1000 ms retry | `netclient.rs` stores the cookie and answers on the retry timer; join now in frame 251 in both (NetGame start → join 4126 ms orig / 4097 ms ours) |
| Weapon / vehicle skill stats = 1000, `CStats::SetStatValue` := RET (row 356) | first frame A 0x100a0c40: `CCheat` vehicle + weapon skill cheats, then the RET | **already in the vehicles work** (`vehicles::first_frame`, now on main); not duplicated here |
| Vehicle blips (DIFF-2 idle) | vehicle-entity blips | vehicles work (M5) |
| REMOVE_BLIP ×2 in the spawn frame | checkpoint object fields that are never initialised (A 0x100a1f60) | not reproduced (decided earlier) |

## Remote peds

| item | original (evidence) | ours now |
|---|---|---|
| Special actions per frame (A 0x10014950, after head look) | duck: `SetTaskDuckSecondary(0)` / clear `bIsDucking` (start ends the pass); phone; hands up start `05C4(ped,−2)` / stop = `Teleport` to own position (A 0x100adb60) | `remote::special_action_steps` + `players::special_actions` |
| One-shot weapon check on a new 207 (A 0x100166B0) | `0470`; if ≠ synced → `01B9` under PlayerInFocus = slot + slot skills (A 0x100ac0a0); still ≠ → load model, `CPed::GiveWeapon(w, 9999)` under the same context, arm again (A 0x100af7d0) | `players::apply_weapon` |
| Sniper ped state | aim key + weapon 34/35/36 → ped state 12, else 12 → 1 | `players::sniper_state` |
| RPC 153 | skin path on the streamed-in ped | `Players::set_skin` |
| Fault containment | A 0x100A2800: SEH frame around GTA's ProcessControl; filter prints "Warning(playerped): Exception 0x%X at 0x%X.", handles the first 10 faults of the process; handler streams the player out ("Removed player %u due to error.") | `swap::guarded_thiscall` (own SEH registration in naked asm, resumes after the call, context restored as usual); the stream-out runs on the next client tick (samp: inside the hook). `SAMP_RS_SEH_SELFTEST=1` runs two deliberate faults at install |
| `06BD` ×2 per frame | name-tag line of sight | M8a (name tags) |
| Local surf probe | A 0x10003730 → 0x100aca50: on foot every frame a vertical `CWorld::ProcessLineOfSight` from z−0.25 to z−2 (vehicles + objects) | `client::surf_probe`; using a hit (surfing samp's vehicles/objects) needs the M5/M6 pools |

## Speed magnitude (TRACE-4: original |v| median 0.050 vs ours 0.030) — resolved, no difference

Static, A 0x100A1C50 (called from samp's `CGame::Process` wrapper A 0x100A26A0 before the original): a time-step
estimate once per 250 ms (`n = 4 × frames − 4`; 3.0 if n < 17, else 50 / n), a 30 ms counter 0…4 written to 0xB7356E,
and the frame limiter (details in `frametimer.rs`). The estimate alone cannot explain a 66 % larger correction.

MR-P.1, measured with identical input (hold sessions, local player and RemoteBot `--radius 3` teleported to the LSIA
runway (1800/1803, −2500), 25 s, scripts `scratchpad/mrp/inner.sh` + `speed.py`):

| run | timestep median | disp/(|v|·ts) median / mean | blocked frames | written |v| median | apply gaps (frames) |
|---|---|---|---|---|---|
| original orig-20261104-230048 | 0.950 | 1.000 / 0.953 | 2 / 1240 | **0.0301** | 1:126 2:397 3:35 4:60 |
| ours ours-20261104-231244 (worktree) | 0.950 | 1.000 / 0.954 | 3 / 1236 | **0.0301** | 1:122 2:369 3:33 4:70 |

The 0.050 of TRACE-4 came from the original's bot walking into an obstacle (zero displacement on 22 % of frames at
(−1316, 2528)); on open ground the original writes the same |v| as ours. TRACE-4's frame-rate difference (orig 0.95
vs ours 0.85 timestep) is gone with the frame limiter.

## Verification (worktree build, final DLL sha256 2c29d966…)

* `cargo build/test --workspace` (480 tests), clippy host + i686 (samp-client, samp-tracer, samp-launcher): 0 warnings.
* Lane main (headless): m1 (`e2e_headless_run`), m2, saa, m3, goal PASS; lane ui: m4 PASS.
* DIFF-3 `tools/diff/cross-client.sh --skip-build` (DIFF_CARGO_TARGET = worktree): **24/24 step-directions MATCH**
  (baseline 13/24): walk/turn (anim group 118 both ways), crouch, handsup, handsup_off, skin.
* DIFF-2 `rpc-coverage.sh --only 'skin_101|time_…|clock_…|worldtime_15|weather_…|specialaction_phone…'`, 2 original
  runs: MATCH 29, 30, 94, 152 (weather; was a standing difference), 88 phone/phone_stop, idle steps; standing
  differences left: weapon skills (vehicles work on main), see the final numbers in the agent report.
* Tracer, `run.sh ours spawn` vs orig-20261104-205141: join in frame 251 in both; per-frame signature while walking
  identical (50.3 % / 49.7 % orig vs 50.0 % / 50.0 % ours: `00c0 04ee×9 0248×9 052c 09ba 0470 077e 077e [0470]`);
  frame time median 19 ms in both; `SetTaskSecondary` (IK) 223 vs 222. Remaining: REMOVE_BLIP ×2 (decided).
* Tracer, `run.sh ours remote` vs orig-20261104-195734: remote 0470 on every applied 207 (every other frame), the
  same cadence; per-frame timestep 0.95 in both.
* COV-1 `COV_CLIENT_DLL=… tools/cov/run.sh both` (both-20261104-231020) vs the main-build baseline
  (both-20261104-224308), only-ours / only-orig per window: boot 15/0 → 2/0, connecting 91/92 → 3/20,
  class_selection 76/47 → 2/5, spawn 227/46 → 4/11, grove_idle 129/55 → 3/3, walk 100/54 → 3/0,
  interior_enter 96/305 → 53/264, interior_exit 65/59 → 85/7, vehicle 85/66 → 4/837 (no vehicles on this branch; the
  vehicles work is on main), idle_end not reached (no car).

## Open / measurement requests

* Interior entry (COV-1 interior_enter): the original enters the Alhambra through `CEntryExit::TransitionStarted`
  with a scripted `CTaskComplexGotoDoorAndOpen` and a fixed camera ~266 frames after the walk, and ends inside at
  y −21.8; ours is in an interior transition 24 frames after the walk (`WeAreInInteriorTransition`, then `CPed::Say`
  338 from `CPlayerPed::ProcessControl` 0x60F896) and ends at y −19.2. In one of three coverage runs ours then
  missed the exit marker. Cause not identified; needs a tracer run of the walk-in for both clients.
* `CCamera::ProcessFade` / `ProcessMusicFade` during class selection only with ours (COV-1 item 11): not analysed.
* HUD flag order: done (session before `Render2dStuff`, HUD = chat mode != hidden after it, as samp's UI render);
  the other UI-render HUD conditions (F5/F10/dialog/scoreboard, drunk > 5000) belong to the UI work.
* RPC 35 drunk level > 2000 (x87 expression in A 0x100ade30) — M8b.
* 207 animation field (samp's 1812-entry anim table) — still 0 in ours (the cross-client harness shows it as
  "server sees anim=0").
* One more `CObject` allocation before any world RPC in the original (world-entity agent): the spawn traces add the
  same 122 objects to the world (`CWorld::Add`, same models, same frames) in both clients, so the extra one is
  probably not a world object; samp's object-pool capacity patch (not applied before M6) changes the pool itself —
  check with M6.
* SetPlayerSpecialAction other than 0/10/11/13 (jetpack, dances, drinks, cuffs, carry, piss) — not implemented.
