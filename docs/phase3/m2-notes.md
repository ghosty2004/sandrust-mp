# Phase 3 / M2 — network join, class selection, spawn, local on-foot sync (in game)

Status: `tools/e2e/m2.sh` passes (lane "main": server 7777, control 47700, headless gamescope).
Typical run: world up ~9 s after launch, joined + class selection ~0.1 s later, spawn on the
first Shift press after the 2 s guard, ~10–16 m walked server-side while W is held for 3 s,
probe reports `st=1` (ONFOOT).

## Pieces (crates/samp-client)
* `netclient.rs` — samp-proto's sans-IO `Connection` over a non-blocking `std::net::UdpSocket`
  (on Windows that is Winsock), driven from the frame tick: open-connection request/cookie
  (retried every 1 s), `0x0b`, auth (`SAMP_RS_AUTH_TABLE` = Windows path of `data/auth_table.txt`,
  exported by `tools/run-client.sh`; never embedded), `0x22` → `0x1e` + ping + `0x29` +
  keep-alive + PlayerConnect (25) → InitGame (139). Every other RPC is decoded with
  `samp_proto::rpcs::decode` and handed to the game side. Counters and the last 8 RPC ids feed
  the `net` command. Loopback unit test for the cookie exchange.
* `classsel.rs` — class selection state machine (pure, unit-tested): RPC 128 on entering,
  Left/Right with the 2000 ms guard, 250 ms repeat, wrap-around and the "class info received"
  gate; Shift → 129 (guard, info, not pending); 129 response handling (2 = force).
* `footsync.rs` — 207/205 (pure, unit-tested): `CControllerState` parsing, key word (§0.3 incl.
  FIRE/ammo, LOOK_*/vehicle, Triangle/jetpack gates), additional key, matrix → quaternion
  (inverse of the receiver formula; round-trip test), 1e-5 jitter filter, send rule
  (keys/sticks changed or interval `onFootRate + N` elapsed, N = 0 until M3) with 501 ms
  duplicate suppression; stats every > 1000 ms (money from `CPlayerInfo`, drunk 0).
* `game.rs` — new addresses/offsets (pad, keyboard state, ped pool handle, ped fields) and
  `read_local_sample` (FakeMemory tests).
* `script.rs` — runs single GTA script commands the way samp does: SCM-encoded instruction in
  our buffer, private zeroed 0xE0-byte `CRunningScript`, `ProcessOneCommand` (0x469EB0).
* `scm.rs` — our own minimal replacement for `main.scm` (see below), unit-tested chunk chain.
* `client.rs` (Windows) — the session: connecting view, class selection, spawn routine, sync,
  S→C RPCs 12, 14, 15, 19, 21, 22, 66, 68, 74, 93 (chat → log), 73 (game text → log),
  128, 129, 157, 158, 162.
* `diag.rs` (Windows) — on "no frame tick 10 s after IDLE" every thread's EIP/ESP and the
  code-looking stack words are logged (winedbg attach to the WoW64 game killed it).
* Control channel: `net` (phase, player id, last RPC ids, datagrams sent/received, messages,
  RPCs, syncs sent, class-selection state), `spawn` (debug only: runs the spawn routine with
  the stored spawn info on the next frame). Port from `SAMP_RS_CTL_PORT` (default 47700) so
  the trace lane can use 47701.

## How the story script is kept out (finding + our equivalent)
* Without anything, the stock `main.scm` fades the screen out and starts the intro mission.
  With samp's CSV row 246 (`CCutsceneMgr::Update` NOPed) the intro waits forever for its
  cutscene with the screen faded: `Idle` then skips the world render **and our frame hook**
  (it tests `TheCamera.GetScreenFadeStatus() == 2` at 0x53E9BE), so the game never ticks.
* Observed in the original client (memory read with a ReadProcessMemory helper, then confirmed by
  TRACE-1): samp does *not* stop the script engine; `CTheScripts::Init` still opens "main.scm",
  but the content is samp's own small SCM (served from `samp.saa` by a file redirection), with
  only a few non-story threads (MAIN, INITIAL, INITIL2, HJ, INTMAN, CASHWIN). MAIN sets up the
  world, creates the player and then only waits.
* We may not use samp's script, so we generate our own (`scm.rs`, written to
  `%TEMP%\samp-rs-main.scm` at init) and serve it by retargeting the single
  `call CFileMgr::OpenFile` in `CTheScripts::Init` (0x468EC9; `OpenFile` is a jump to the CRT
  `fopen`, so our hook just opens our absolute path with the game's `fopen`). Chunk layout per
  gta-reversed `SCMChunks.hpp`: globals area 46268 bytes (same size as observed for samp's
  script), one empty object entry, no missions, **no streamed scripts**, then MAIN:
  SET_TIME_OF_DAY 12:00, ped/car density 0, REQUEST_COLLISION, CREATE_PLAYER (Grove Street),
  GET_PLAYER_CHAR, WAIT 0, BUILD_PLAYER_MODEL (without it CJ renders as a bare skeleton),
  SET_PLAYER_CONTROL off, SWITCH_ROADS_OFF over the map, then `WAIT 1000` forever.
* `script.img` stays stock: with zero streamed scripts declared and no script brains
  registered, nothing is ever loaded from it. samp's own `script.img` (18 helper scripts:
  vending/slot machines, casino, ammu, carmod, parachute) is a later milestone.
* Other samp script behaviour reproduced from the TRACE-1 notes: while connecting the ped is
  parked at (1133.05, -2038.40, 69.1) and the camera fixed at (1093, -2036, 90) looking at
  (384, -1557, 20) every frame.

## Per-frame behaviour aligned with the TRACE-1 diff (docs/phase3/trace-notes.md)
* First frame: SET_CHAR_DROPS_WEAPONS_WHEN_DEAD(player,1), SET_CHAR_SUFFERS_CRITICAL_HITS(player,0),
  DISPLAY_ZONE_NAMES(0).
* Every frame: SET_TIME_OF_DAY(server hour, 0) (hour from InitGame / RPC 29 / 94); the
  pre-loading polls (HAS_ANIMATION_LOADED → REQUEST_ANIMATION for PARACHUTE, BAR, GANGS,
  PAULNMAC, VENDING, FIGHT_B…E; HAS_MODEL_LOADED → REQUEST_MODEL for 3131, 1485, 1486, 1543,
  1546, 343, 342, 344, 330).
* Connecting (until InitGame): teleport to (1133.05, -2038.40, 69.1), 0925, 015F (1093,-2036,90),
  0925, 0160 (384,-1557,20, 2); HUD off.
* Join: MARK_MODEL_AS_NO_LONGER_NEEDED(596), `SetInitialState` (→ SetPedState 1), health 100,
  01B4(0,0) + 04D7(ped,1). Class response: `ClearWeapons` + skin change.
* Skin change is samp's direct path, always (even for the same model): model load,
  vtable `DeleteRwObject` + `SetModelIndex`, `CAEPedSpeechAudioEntity::Initialise`.
* RPC 67 SetPlayerArmedWeapon → SET_CURRENT_CHAR_WEAPON (the "next frame" 01B9(ped,0)/(ped,4)
  in the original trace are these RPCs; the post-spawn teleport/heading/camera reset are the
  server's RPCs 12/19/162, not part of the spawn routine).

## Spawn routine (player-state.md "Local Spawn routine", order from TRACE-1)
02EB → 0930(0)/0925/0373/02EB → HUD on → 01B4(0,1) + 04D7(ped,0) + teleport to own position →
(`CPlayerPed::SetInitialState` only on respawns; the original skips it on the first spawn) →
04E4 REQUEST_COLLISION → 016C ADD_HOSPITAL_RESTART (instead of the A build's direct restart
table writes, CSV row 385) → skin (always) → `CPed::ClearWeapons` → last-damage fields reset →
weapons slot 3, 2, 1 (model load + `CPed::GiveWeapon` + 01B9) → 06D7(0) → CSV row 365
`CCamera::Fade := ret 8` → teleport z+0.5 → facing (ped +0x558/+0x55C, 0173) → RPC 52.
Not reproduced: the original's two REMOVE_BLIP calls in the spawn frame, whose handles are
never valid blips (values look like uninitialised data).
Class selection: HUD hidden while no menu is open. Keys: `GetAsyncKeyState` high bit (as the
original) OR GTA's `CPad::NewKeyState`.

## Patches added in M2 (CSV rows)
246 (`CCutsceneMgr::Update`), 157 (zone names), 161 (vehicle name), 180 (`SetHelpMessage` ret —
removes the "Respect lost" help box our empty stats produce), 202 (HUD wanted level from samp,
0), 205 (vital stats); runtime 363, 365. Not a samp patch: the main.scm open retarget above.

## Deviations
* Socket via `std::net` (Winsock underneath) rather than raw windows-sys calls — same API on the
  host, so the network part is unit-tested.
* 207 `animation` is always 0: the original derives it from the ped's current anim through
  samp's 1812-entry anim-name table (third-party data in samp.dll) — needs the table loader (M3).
  Special action: only duck (1) and jetpack (2). Surfing fields 0. N (streamed peds) = 0.
* No 203 (aim) / 204 (weapons) packets yet (the original sends both right after spawn).
* No death handling (53, respawn, ForcePlayerClassSelection only stored).
* The input lock (keyboard→pad off during class selection) is replaced by
  SET_PLAYER_CONTROL off + FREEZE_CHAR_POSITION.
* The game still loads the stock `data\paths\tracks*.dat` (samp serves 1-byte versions from
  samp.saa, i.e. no train tracks); like the other samp.saa overrides this waits for the user's
  decision on samp.saa data.
* CJ has no clothes (our script doesn't give any); the original shows the same base model —
  not compared visually yet.

## For M3
* Remote players: ped slots, `CPlayerPed` creation, swap hooks, interpolation, N in the
  on-foot interval; samp's anim table for the 207 animation field.
* 203/204 senders, death/respawn and forced class selection, input lock.
* Pool/sector/model-store relocations, image list (row 128) + SAMP.img before any samp models.
