# Phase 3 / F7: soak (`tools/e2e/soak.sh`)

PLAN F7: "2-hour soak on each bundled gamemode (gungame, derby, simpletdm) plus scripted
reconnect/kick/restart cases, no crash, no desync". Lane main by default:

    flock target/locks/main.lock tools/e2e/soak.sh                 # 3 × 2 h
    SOAK_SECS=600 flock target/locks/main.lock tools/e2e/soak.sh   # trial, 10 min per gamemode

Exit 0 only when all three gamemodes pass. `SOAK_CAPTURE=1` adds a loopback capture of the server port
(`capture.pcapng`, ~3.5 MB/min with both proxy legs) for samp-dump. Output: `target/soak/<gamemode>/` (`summary.json`
verdict, `samples.jsonl`, `events.jsonl`, `client-N/` logs of each game session, `bots/`,
`server.log`, `proxy.log`, `soak.out`).

## Files

| file | role |
| --- | --- |
| `tools/e2e/soak.sh` | build, server copy per gamemode (`e2e_server_prepare`, then `main_scripts` = the gamemode, side scripts `e2eprobe` + `soakfs`, server bound to 127.0.0.1), headless display (`e2e_headless_run`), verdict over the gamemodes |
| `tools/e2e/soak_mon.py` | the run for one gamemode: starts and tracks (by PID) the server, the proxy, the bots and our client; samples, scripted cases, crash detection, report (`soak_mon.py report DIR` recomputes the verdict) |
| `tools/e2e/soak_udpproxy.py` | userspace UDP proxy between our client and the server; SIGUSR1 drops everything, SIGUSR2 resumes |
| `tools/e2e/probe/soakfs.pwn` | filterscript: `dump` (every player: state, position, velocity, health, armour, vehicle/seat, streamed-in for our client; every vehicle: model, position, velocity, health, streamed-in), `kick`, `rcon` (gmx), `vehspot` (parked cars kept alive for the drive/passenger bots in gungame) |

The proxy listens on `127.0.0.2:<server port>`: samp's packet cipher is keyed with the port the
client dials, so a proxy on another port is undecodable for the server (measured: a samp-bot
through `127.0.0.1:7779 → 7777` gets no answer; the server's query answer comes back fine).

## Players

Our client (`RustClient`, via the proxy) plus samp-bot instances, each restarted with a random
`--seconds` of 240–720 s and 5–15 s pause (so remote players join and leave all through the soak):
`SkWalk` (walk), `SkIdle` (idle, special action 1), `SkShootC` (shoot at us, every 2.5 s),
`SkShootB` (shoot at SkWalk); gungame adds `SkDrive` / `SkPass` on three Landstalkers parked by
soakfs at the spawn; derby adds two drivers (the gamemode puts every spawned player into a car);
simpletdm a second walker. What a player does with our client (soak_mon `assist`): Shift in class
selection, F when spectating for 8 s (gungame's respawn key), `W` for 2 s every 40 s when on foot or
driving.

## Sample (every `SOAK_SAMPLE_SECS` = 5 s)

`dump` on the server (filterscript, polled every 100 ms; 30–90 ms), then at once the control channel
`state`, `net`, `remote`, `vehicles` (~60 ms). Plus: the game process (gta_sa.exe whose environment
has this lane's `WINEPREFIX`), its VmRSS, the frame counter (`state ticks=`; fps = Δticks/Δt).

Comparisons (category: what must hold):

| category | check |
| --- | --- |
| `local_pos` | our ped vs the server's GetPlayerPos (on foot or in a car) |
| `local_hp` | our health vs GetPlayerHealth, ±1 |
| `remote_known` | every connected player is in the client's pool with the same name |
| `remote_ghost` | the client lists no player the server does not have |
| `remote_stream` | a player streamed in for us (IsPlayerStreamedIn) has a ped in the game |
| `remote_pos` | that ped vs the server's position |
| `remote_hp` | synced health vs the server's, ±7 (the server relays health packed with armour into one byte, 4 bits each in steps of 7 HP: a 43 HP bot arrives as 49) |
| `remote_vehicle` | a seated remote player sits in the same vehicle id in the game |
| `veh_stream` | a vehicle streamed in for us exists in the game |
| `veh_pos` | its position vs GetVehiclePos |
| `veh_hp` | its health vs GetVehicleHealth, ±10 |
| `veh_driver` | the game's driver (local / player id / none) is the server's driver |

Position tolerance: 4 m on foot, 6 m in a vehicle (the acceptance values of `m3.sh` / `m5.sh`) plus
the server-side speed × 0.6 s (a remote entity is shown one sync interval + network delay behind the
server's copy; the two snapshots are up to ~0.15 s apart). For a seated player the vehicle's
velocity is used (GetPlayerVelocity is 0 in a vehicle).

Only samples in phase `steady` count: not during a scripted case and its recovery, nor 10 s after it.

## Scripted cases (each at least once; every `SOAK_EVENT_PERIOD` = 900 s in a 2 h soak, spread evenly in a trial)

| case | action | expected (measured on the original: trace-notes TRACE-8, m10-notes) |
| --- | --- | --- |
| `gmx` | `SendRconCommand("gmx")` (RPC 40) | "The server is restarting..", InitGame again, class selection, spawn |
| `netloss` | proxy drops everything for `SOAK_NETDROP_SECS` = 15 s; 1 s in, a chat line is typed (`say`) | "Lost connection to the server. Reconnecting.." (RakNet ack timer, ~10 s after the unacked reliable send); the server still holds the old player at the reconnect → "CONNECTION REJECTED: Unacceptable NickName", no retry (as samp) → the game is started again |
| `srvkill` | server SIGKILL, start again after 5 s | lost connection, reconnect by itself, spawn |
| `kick` | `Kick` | "Server closed the connection.", the game stays up, no reconnect; then `quit` (clean exit within 25 s) and a new game session joins |
| `srvrestart` | server SIGTERM, start again | "Server closed the connection." (open.mp notifies on shutdown), no reconnect → new game session |

An idle client rides out a 15 s gap without noticing (first smoke run: no lost line, the connection
simply resumed; neither side times out, the server's `player_timeout` did not fire either) — hence
the typed chat line.

## Pass criteria (per gamemode)

* no client crash: the launcher/game process gone, a `winedbg` process with this lane's prefix, or
  Wine's `wine: Unhandled …` / `err:seh` lines (`WINEDEBUG=-all,err+seh`); no `PANIC:` in the DLL log;
* no stuck state: unhealthy (control channel silent, not connected, or unknown to the server) for
  more than 150 s, every case recovered with its expected lines, no frozen frame counter (two samples
  without a new frame);
* desync: ≥ 99 % of the steady comparisons within tolerance in every category, and no persistent
  desync (the same entity off in 3 consecutive samples, ~15 s). 99 % and not 100 %: a sample can land
  between a teleport/warp/respawn on the server (SetPlayerPos, PutPlayerInVehicle, gamemode respawns
  every few seconds in gungame and derby) and the client applying it (one sync interval, up to a few
  hundred ms), and a remote car can be held for a moment by GTA's collision before the next sync moves
  it; in the trials these transients were 0–1.2 % of a category. A real desync (wrong world, lost
  entity, frozen remote player) does not heal and is caught by the persistence rule;
* fps: median ≥ 20 and 5th percentile ≥ half the median;
* memory: the least-squares slope of the game's RSS over a client session, after its first 300 s
  (streaming warm-up: 160 → 250 MB in the first minutes), ≤ 40 MB/h; only judged on a session with at
  least 600 s after the warm-up (a 2 h soak), reported otherwise.

## Trial results (SOAK_SECS=600, 2026-11-05)

Lane main, `SOAK_SKIP_BUILD=1` with the client built from the main checkout at the start of the
session. Outputs kept in `target/soak-trial1/`, `target/soak-trial2/` (trial 3: `target/soak/gungame/`).
The cases ran and recovered in every gamemode of every trial: gmx 22 s, netloss 11–41 s, srvkill
17–30 s, kick 27–29 s (quit clean within 25 s, no automatic reconnect), srvrestart 33–38 s.
Frame rate in every run: median 52.2 fps, 5th percentile ≥ 52.0, minimum 51.9 (fpslimit 48 baseline).

| run | gamemode | crashes | steady comparisons within tolerance | verdict |
| --- | --- | --- | --- | --- |
| 1 | gungame | 0 | 99.86 % of 2826 (remote_pos 98.8 %: seated bots judged without their car's speed, tool fixed) | FAIL (tool) |
| 1 | derby | **2** | 99.77 % | FAIL |
| 1 | simpletdm | 0 | 97.4 % (remote_hp: relay quantisation, tool fixed; remote_pos: stale SkShootB, below) | FAIL |
| 2 | gungame | 0 | 98.25 % of 2792 (veh_pos 93.3 %, veh_stream 95 %, remote_pos 96.2 %: findings 2–4) | FAIL |
| 2 | derby | **2** | 99.91 % of 3187 | FAIL |
| 2 | simpletdm | 0 | 100 % of 1582 | PASS |
| 3 | gungame (300 s, fixed soakfs, capture) | 0 | 100 % of 1300 | PASS |

Memory (VmRSS of gta_sa.exe): every game session climbs from ~160 MB to 240–275 MB in its first
2–3 minutes (streaming) and then stays flat; the longest session (simpletdm run 2, 458 s) went
168 → 264 MB, slope after the 300 s warm-up 33 MB/h over only 159 s (not judged: < 600 s; the 10-minute
trials restart the game in kick / srvrestart / netloss). RSS step across in-process reconnects: gmx
+3.5 … +18.6 MB, srvkill +1 … +8 MB, netloss +2.5 … +31 MB (run 1 simpletdm). A 2 h soak will
show whether these steps accumulate.

## Findings of the trials (resolution: next section)

1. **Crash in derby, 4 times in 2 trials (2 per 10-minute run), always right after a local respawn into a
   car.** `run.out`: `wine: Unhandled division by zero at address 004F1464 (thread 01e4), starting
   debugger...`; the game hangs in winedbg. 0x4F1464 is `div ecx` in gta_sa.exe where ecx is the
   return of the object's vtable+8 call (a count / length of 0), on a worker thread (0x1e4, not the
   main thread), in code that calls 0x4E0D20 / 0x5026B0 (the audio area: likely the audio stream
   thread / radio track decoder). DLL log tail each time: `client: respawn after death` → `spawned at
   (-279.9, 2167.0, 106.7)` → `vehicles: stream-in id=N model=495` (derby's OnPlayerSpawn creates the car
   and PutPlayerInVehicle), crash within ~1 s; in run 1 the previous car with the same id had just been
   streamed out and deleted. Evidence: `target/soak-trial1/derby/client-{1,2}/`,
   `target/soak-trial2/derby/client-*/run.out`, `events.jsonl` (`client-crash` with log tail). Earlier
   respawns in the same sessions did not crash (2 of 4 respawns in run 1). Check the original samp.dll
   on the same scenario (derby, drive off the platform, respawn) before blaming our client: the radio
   in a fresh car may hit a track of the Steam 3.0 audio data with length 0.
2. **Exploded unoccupied car disappears for good** (run 2 gungame, `samples.jsonl` t 145–240): soakfs
   had put two Landstalkers on the same spot after the gmx (soakfs bug, fixed: OnGameModeInit no longer
   resets the ids); in our game veh 2 lost health 786 → 0, exploded and was deleted
   (`vehicles: id=2 handle=0x202 deleted`, +162 s); the server kept it streamed in with 1000 HP and never
   respawned it (80 s, until the netloss reconnect). Either RPC 136 (vehicle destroyed) was not sent, or
   not accepted; samp reports a wrecked vehicle and the server respawns it after the respawn delay (30 s
   here). Check `vehicles` header `sent_136`.
3. **Unoccupied car moved locally, server never told** (same run, veh 4): pushed 6.05 m by the overlap,
   the server's position stayed at the spawn spot for 17 samples. samp would send unoccupied sync (209)
   for a car it moved while being the closest player; worth checking our 209 policy.
4. **Remote player without sync** (run 1 simpletdm after the gmx, run 2 gungame in a fresh session):
   SkShootB (shoot mode, target SkWalk) listed with a ped but `sync_age_ms` growing from the gmx on
   (27 → 117 s) or `sync_age_ms=-` (never) with remote `state=33` (spawned, never on foot), the ped
   6–8 m from the server's position, while the server lists it on foot and streamed in for us and the bot
   kept shooting. Other remote players updated normally. A bots-only capture (gungame, observer bot)
   shows the server relaying SkShootB's 207 normally (419 relays in 70 s), so the bot does send.
   Not reproduced in run 3 (with capture). Next time: `SOAK_CAPTURE=1` and decode the proxy leg of our
   client for that player id (samp-dump), and check the late-packet rule (`accept_timestamp`) against
   the relayed timestamps.
5. Harness-only notes: a 15 s gap does not drop an idle connection (neither side times out) — the
   netloss case now types a chat line during the gap; after the reconnect the server sometimes still
   holds the old player (nick rejected, game restarted, as samp) and sometimes has timed it out
   (reconnect joins directly). samp-bot exits non-zero on a server SIGKILL (`Connection refused`).

## Resolution of the findings (2026-11-05)

1. **Derby crash at 0x4F1464 — test data, not the client; fixed in the test environment.**
   - What divides: 0x4F1464 lies in the audio stream thread's service routine (gta_sa.exe 0x4F1290,
     `CAEStreamThread::Service` in gta-reversed's naming). When it starts a track it asks the new decoder for
     its length in ms (decoder vtable +8) and seeks to "start position modulo length" (`div ecx`); a track of
     length 0 divides by zero. The radio starts a station mid-track when the player gets into a car.
   - Why the length is 0: the game copy is the 1.0 US exe with the Steam 3.0 audio data. Its track table
     (`audio/CONFIG/TrakLkup.dat`, 1922 entries of {pack, offset, Ogg size}) points 122 entries at stub tracks
     that replaced the removed licensed songs: 4429-byte Ogg Vorbis streams holding zero samples (last
     granule position 0) in the packs CH 7, CR 14, DS 21, MH 13, MR 14, RE 14, RG 38, BEATS 1
     (`tools/audio-stub-tracks.py check`). Steam's own exe copes with them; the 1.0 exe does not. Nothing in
     samp.dll's patch table touches the stream thread or the radio (the only audio hooks are the
     siren/horn/UsesSiren calls and the police scanner), and the original crashed at the same address
     before (TRACE-7, M10-P, and the m5 note in ROADMAP "Decisions"). So the audio device (null sink) is not
     involved, and the crash needs a stub to be the first track of a station start.
   - (a) Derby with the original client: trace lane, `TRACE_GAMEMODE=derby` (tools/trace/lib.sh now runs a
     bundled gamemode as is), an explosion at the player every ~12 s in the derby car, respawn, repeat for
     600 s: original 32 respawns, ours 31, 0 crashes each — but both ran with the stubs already aliased
     (the trace lane's table had been patched by a stray `ui.sh` call sourcing the new wine-env.sh; restored
     since), so they only show the fix holds for the original too. The unpatched rerun (`derby2`) could not
     get the trace lane (held by an orphaned headless session of another lane) and is still open. Unpatched
     trace-lane runs in the uifs gamemode did not crash either (original: 80 car entries and 20
     die-in-car → respawn → new car cycles; ours: 7 cycles), so the rate at which the original hits a stub in
     derby is not measured; the soak lane (ours, stubs in place) had 4 crashes in ~20 respawns over 2 trials.
     Which track a station starts on depends on its play position at that moment.
   - Fix: `tools/audio-stub-tracks.py patch GTA_DIR` aliases every stub entry to the nearest real track of
     the same pack (previous one, else next). `tools/wine-env.sh` applies it once per game copy (skipped when
     `TrakLkup.dat.steam` exists; opt-out `SAMP_RS_KEEP_AUDIO_STUBS=1`). The table is written as a new file
     (rename over), so the other lanes' hard links to the shared original stay untouched; the original is kept
     as `TrakLkup.dat.steam` (`restore` puts it back). Audio routing is unchanged (still the private null sink;
     the user's default sink is never touched).
   - Evidence after the fix (lane main): derby 600 s in runs B, C, E below: 0 crashes in 13 + 14 + 11
     respawns (before: 4 crashes in 2 runs).

2. **Exploded unoccupied car stays on the server — same as samp.** RPC 136 (VehicleDeath) is sent only for
   the local player's last driven vehicle (vehicles.md §136: A 0x1001F230 transmits only when the id equals
   the "last driven" field, then clears it); a wasted car is deleted locally 15 s later without any RPC. Our
   client implements exactly that (`vehicles.rs` `vehicle_death`, `WASTED_DELETE`). Measured on the original
   (trace lane, uifs, `CreateExplosion` at a Sandking the player sits in as driver): the server gets
   OnVehicleDeath from the original for the car it drives (`vehicle-death 5 by 0`, 20 of 20 cycles); the
   never-driven case could not be run on the original (trace lane blocked, see 1) and rests on the static
   rule. In the soak case the car had never been driven
   by our player (it exploded from the overlap of two cars soakfs had spawned on one spot, fixed earlier),
   so no client reports it and the server keeps it until its own respawn timer — not a client bug.

3. **Pushed parked car not reported (no 209).** The car was pushed by the same overlap while our player was
   in class selection after the gmx and then lay still. Whether a 209 goes out is the unoccupied-sync rule
   (closest player, moving/stopped, passenger seat) that the netp lane is reworking (NET6-P3); not duplicated
   here. The trigger (two cars on one spot) is gone with the soakfs fix.

4. **Remote player without updates — the bots, not the client.** Reproduced with `SOAK_CAPTURE=1`
   (simpletdm, SkShootC after the gmx, `target/soak-evidence/A-simpletdm`): on our client's proxy leg the
   server relays **no** 207 for that player after the gmx (170 before, 0 after, while it relayed the other
   players'); on the bot's own leg SkShootC sent 16 × 207 but 1145 × 203 after the gmx. Cause: samp-bot's
   shooter stands still and aims with a swaying camera; its on-foot struct is byte-identical tick after tick
   while every aim struct differs, and the duplicate-suppression timestamp is shared by 207/203 (samp's
   rule, remote-players-sync §0.4), so the identical 207 is never re-sent. The server keeps its last state
   (on foot at the teleport spot) and our ped, created by RPC 32 there, has no sync to follow (it then drops
   to the floor or slides by GTA physics). Fix in samp-bot: the aiming ped turns with its aim sway (a real
   aiming player's on-foot struct changes with the camera), so 207 keeps going out (unit test in
   `shoot_cycle`). A second bot artefact showed up in derby (run B): a bot driving at the gmx kept syncing
   in the restart gap; the restarted gamemode received an on-foot 207 before its class request, took the
   player as on foot and ignored the request, so the bot stayed in class selection with a stale server
   state. Fix: samp-bot stops syncing on RPC 40 until the next InitGame, as the client's reset routine does.
   Diagnostics added to the client: `remote` lines carry `foot_rx` / `foot_late` / `last_ts` per player,
   `net` carries `syncs_bad` (undecodable sync packets, the first 20 logged with their bytes).

5. Harness: a local seat change in flight (derby's OnPlayerSpawn → PutPlayerInVehicle: the client sits in
   the car at once, the server only after the next driver sync) made one sample's `local_pos` / `veh_driver`
   fail (run C derby: 1 of 66 → 98.5 %). soak_mon now skips the local comparisons of the first sample in
   which the client's driver seat and the server's state disagree; a mismatch that lasts into the next
   sample is judged as before.

### Runs (lane main, branch soak-fixes, SOAK_SECS=600)

| run | gamemodes | crashes | result |
| --- | --- | --- | --- |
| A (capture) | gungame, simpletdm | 0 | gungame PASS; simpletdm FAIL (finding 4 reproduced, bot fix not yet in) |
| B | derby | 0 (13 respawns) | FAIL remote_pos 90.8 % (bot stuck in class selection after the gmx) |
| C | all three | 0 (derby 14 respawns) | gungame PASS, simpletdm PASS, derby FAIL local_pos 98.5 % (1 of 66, seat transition) |
| D | all three | 0 | interrupted at derby 386 s by a session restart (gungame PASS) |
| E (final build) | all three | 0 (derby 11 respawns) | **PASS** gungame, derby, simpletdm |
