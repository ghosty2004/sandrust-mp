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
| `remote_pos` | that ped vs the server's position; for a seated player the client's position of its vehicle (the server's position of a seated player is its vehicle's; the ped sits at the model's seat, up to several metres from the car's origin: section "F7 settled") |
| `remote_hp` | synced health vs the server's, ±7 (the server relays health packed with armour into one byte, 4 bits each in steps of 7 HP: a 43 HP bot arrives as 49) |
| `remote_vehicle` | a seated remote player sits in the same vehicle id in the game |
| `veh_stream` | a vehicle streamed in for us exists in the game |
| `veh_pos` | its position vs GetVehiclePos |
| `veh_hp` | its health vs GetVehicleHealth, ±10 (bracketed, see `BRACKETED`) |
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
| `netloss` | proxy drops everything for `SOAK_NETDROP_SECS` = 15 s (a list such as `"15 40 6"` is used by the netloss cases in turn); 1 s in, a chat line is typed on the game window (T, text, Enter; both clients, `chat_say`) | "Lost connection to the server. Reconnecting.." (RakNet ack timer, ~10.05 s after the first reliable send into the gap), then reconnect attempts on a new socket each, carrying the last cookie (docs/phase3/reconnect-notes.md); usually the server has timed the old player out and the rejoin is accepted; if it still holds it → "CONNECTION REJECTED: Unacceptable NickName", no retry (as samp) → the game is started again. A drop < 10 s: no lost line. `tools/e2e/netloss_timeline.py` prints the capture's timeline per case |
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
  least 600 s after the warm-up (a 2 h soak), reported otherwise. Judged within segments (below, (b)); the
  first segment of a game process starts 900 s after the launch (`WARMUP_FRESH_S`, section "F7 settled").

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

## The 2-hour derby run (2026-11-05): desyncs explained (branch soak-desync)

Run: lane veh, derby, 7200 s (`target/soak-early/derby-2h-fail.log`, data in the main checkout's
`target/soak/derby/`). 0 crashes, all five cases recovered, fps median 52.5, but FAIL on desync ratios
(remote_ghost 0 %, remote_known 96.65 %, remote_pos 97.46 %, remote_vehicle 97.80 %, veh_driver 98.73 %,
veh_pos 98.54 %, veh_stream 96.89 %, 24 persistent desyncs) and an odd "memory session 2" slope.

### Classification

| finding | class | evidence | resolution |
| --- | --- | --- | --- |
| every desync category from t = 1594 s to the netloss reconnect (1871 s): the client stopped applying RPCs (`rpcs=564` frozen in the `net` lines for 277 s while `syncs_in` kept rising); joins/quits, stream-in/out and seat changes were all missed, hence remote_known / remote_ghost / veh_stream / remote_vehicle / veh_driver / remote_pos / veh_pos and all 24 persistent desyncs | **(a) client bug** (RakNet layer) | samples 1603–1859; client-1 `samp-rs.log` | fixed, below |
| 2 single-sample remote_ghost (t 416, 1129) | (b) sampling order | the server dump was taken just before SkShootC's (re)join and the client queried ~70 ms after it had applied RPC 137 (join at +414.3 / +1126.7 in the DLL log) | bracketed comparison |
| remote_ghost 0.00 % | (b) tool | the category only produced a comparison when it failed, so any ghost gave 0 % | one comparison per listed player |
| single-sample veh_stream / remote_stream / veh_driver / remote_vehicle (t 375, 1170, 1269, 1337, 1343, 2125, 2528, 2797, 2922) | (b) sampling order | each one coincides with a derby car destroyed and re-created (death / respawn) between the server dump and the client query (e.g. t 375: local death at 13:03:27.55, stream-out of veh 7 at 27.57, dump 77 the same second) | bracketed comparison |
| remote_pos / veh_pos single samples (≈ 30 of 3400, all SkIdle's parked car, 6–12 m from the server's position, the idle bot's sync 0.2–1 s old) | **(c) shared with the original** | the idle bot's car is rammed by the other derby cars; samp only nudges a remote car within 8 m of its synced position (remote-control-vehicles.md §2.2: Δ·0.06 per received driver sync) and the idle bot's syncs are ≥ 0.5 s apart. Original, same configuration: remote_pos 99.46 %, veh_pos 99.54 %, all SkIdle (below) | logged decision, no change |
| veh_hp single samples (burning / exploding cars) | (c) | original 99.85 %, ours 99.6–99.8 % (below) | none |
| "memory session 2: 791 MB/h, warm span 24 s" | (b) the run was stopped from outside | server.log, samples and the DLL log all have the same 3.4 h hole (14:04:28 → 17:28:16 UTC, local 16:04:28 → 19:28:16) with nothing running in between (not a suspend: the journal and other processes kept going): the whole process group was stopped. Session 2 had 275 s of samples before the stop and 24 s after it, so its "warm" part was 24 s of the warm-up climb; also the 2 h soak only covered 4031 s | pause detection + pause-free time axis |
| memory session 1: 16.4 MB/h | — | within the limit; the original's slope in the same window is the same (below) | — |

### (a) The RPC stall: message-number wrap in the duplicate filter

The server numbers every message it sends (syncs included) from one 16-bit counter, so it wraps every
65536 messages (≈ 10 min of derby traffic). `Connection` remembered the numbers of the last 8192
*reliable* messages in a set and dropped any reliable message whose number was in it. After a wrap, a new
RPC whose number equals that of an RPC received one wrap earlier was taken for a duplicate: acked (so the
server never resends it) and dropped. Its ordering index then never arrives, and the ordered channel holds
back every later RPC for good — exactly the frozen `rpcs=` counter. With ~600 RPC numbers remembered per
wrap the chance per RPC is ~1 %, so a long connection hits it within the first hour (here at 173 543
received messages, the 3rd wrap). The bots share the code (`samp-proto`).

The original (A 0x10049250, written up in docs/re/handshake.md §3 "Received message numbers"): a window
with a base number and holes (each given up 10 000 ms after it opened) over all message numbers; a number
less than or equal to 0x8000 ahead of the base is new unless its slot was received, anything further is
behind the base. `Connection::accept_number` implements that rule. Unit tests
`message_numbers_wrap_without_false_duplicates` (3 wraps of syncs with irregular RPCs: the old filter
delivered 1714 of 5076 RPCs, the new one all) and `receive_window_holes`.

Evidence on the client: `SOAK_CASES= ` derby 1800 s (one connection for the whole run): 198 600 messages
received (3 wraps), RPCs applied to the end, remote_known / remote_ghost / veh_stream 100 %.

### (b) Harness changes (tools/e2e/soak_mon.py)

* **Bracketed comparison** (`BRACKETED`: remote_known, remote_ghost, remote_stream, remote_vehicle,
  veh_stream, veh_driver). The client is queried after the server dump; an RPC in flight in between shows
  the client already in the new state. A second dump is taken right after the client query; a discrete
  comparison fails only when the client matches neither dump. Position, health and the local checks are
  judged against the first dump as before (no tolerance changed). A real desync (the stall above) is bad
  against both dumps and still fails at once.
* **remote_ghost** yields one comparison per player the client lists (before: only failures).
* **Pause detection**: a gap of more than 60 s between samples (`PAUSE_S`; cases keep sampling, the
  longest normal gap is 23 s) is an event `paused` and a failure reason ("did not cover its span,
  rerun"); the memory time axis skips the pause.
* **Memory slope** is fitted within segments (session start or the end of an in-process case
  + `WARMUP_S` … next case), pooled, judged when the segments cover ≥ 600 s. Reason: an in-process
  reconnect (gmx, netloss, srvkill) deletes every remote entity and streams the world in again, a warm-up
  of its own; in a 30-minute soak (cases every ~300 s) the whole-session line through those re-warm-ups
  gave 39–52 MB/h (run1 / run2 below) although the RSS of a session without cases grows like the
  original's: ours 18.3 MB/h, original 19.3 MB/h, same 1800 s derby, no cases. In the 2 h run the judged
  slope is 27.2 MB/h over 2440 s; a 30-minute soak no longer judges memory (16 s of segments), it reports.
* **SOAK_CLIENT=orig** (soak.sh + `OrigView`): the same soak with the original samp.dll + samp-tracer.dll
  in the trace lane (its lock is taken). `net` / `remote` / `vehicles` are built from samp.dll's memory
  through the tracer's `peek` (the 2019 build that lane runs; offsets in `OrigView`, read from its code),
  the class-selection and spawned flags come from the server's state; remote_hp is not compared (samp keeps
  the remote CPed's health at 1000 and the synced value elsewhere). `SOAK_CASES` selects the cases (since
  "F7 settled" the orig default is all of them, as for ours: the chat is read from samp's own `chatlog.txt`,
  which the chat window truncates at every game start, and the netloss chat line is typed on the game window).
  `TRACE_HOST` in `trace_launch` points the original at the proxy.

### (c) Original vs ours, same derby configuration (1800 s, bots as in the soak, no cases)

| category | original (`target/soak-runs/orig-1800`) | ours (`ours-nocases`) |
| --- | --- | --- |
| remote_known / remote_ghost / remote_stream / remote_vehicle | 100 % | 100 % |
| veh_stream / veh_driver | 100 % | 100 % |
| remote_pos | 99.46 % (9 bad: 8 SkIdle's rammed car, 1 SkShootC) | 99.76 % (4, SkIdle) |
| veh_pos | 99.54 % (9, SkIdle's car) | 99.80 % (4) |
| veh_hp | 99.85 % (3, burning car) | 99.69 % (6) |
| local_pos / local_hp | 100 % | 100 % |
| fps median | 52.4 | 52.5 |
| memory slope after warm-up | 19.3 MB/h | 18.3 MB/h |

Both pass. The remaining position and vehicle-health misses are the same kind on both clients (a remote
car pushed by others between sparse syncs; health of a burning car between relays): logged in
docs/ROADMAP.md "Decisions made autonomously".

### 30-minute soaks on the branch (`SOAK_GAMEMODES=derby SOAK_SECS=1800`, lane veh)

| run | verdict | notes |
| --- | --- | --- |
| run1 | PASS (verdict) | soak.sh exited 2 afterwards: the script was edited while running (bash reads it lazily) — not counted |
| run2 | FAIL: memory slope 51.6 MB/h (whole-session rule) | every desync category ≥ 99.7 %; with the segment rule: PASS |
| run3 | **PASS** (exit 0) | 99.88 % of 16 850; remote_pos 99.32 %, veh_pos 99.42 % (SkIdle's car), everything else 100 %; 0 persistent; fps 52.4 |
| run4 | **PASS** (exit 0) | 99.90 % of 16 850; remote_pos 99.66 %, veh_pos 99.65 %, veh_hp 99.65 %; 0 persistent; fps 52.4 |

run3 and run4 ran back to back on the final code (outputs `target/soak-runs/run3`, `run4` in the worktree).

E2E on the branch (lane veh, after run4): m1, m2, m3, m4, m5, m6, m7, m8a, m8b, m9 (with
`M9_ORIG=<main checkout>/target/m9-orig/trace.jsonl`; bass_diff 0 differences), m10, goal, saa — all exit 0.

## F7 settled: 2-hour derby, original vs ours (2026-11-06, branch soak-final)

The 2 h derby soak on main (`target/soak-early3/`, lane veh) failed narrowly: `remote_pos` 98.83 % (74 of 6353)
and "memory slope 40.4 MB/h > 40 (session 1, 2438 s judged)", everything else passing. The original ran the
**same** soak (all five cases, same schedule) in the trace lane: `SOAK_CLIENT=orig` now runs every case
(chat from samp's `chatlog.txt`, the netloss chat line typed on the game window; the 600 s trial passed all
five cases on the original: gmx 19 s, netloss 13 s, srvkill 17 s, kick 27 s, srvrestart 43 s).
Outputs: `target/soak-runs/` in the worktree (`orig-trial`, `orig-2h`, `ours-a`, `ours-final`, `orig-final`).

### The 74 remote_pos misses

* Entities: 73 SkIdle (the idle bot, parked in its derby car, rammed by the drivers), 1 SkWalk (a teleport
  sample, 12 m). All with the player seated (server state 2), none dying, respawning or in a case window;
  spread over the run (0–6 per 10 min, per 30-min block 99.4 / 99.4 / 98.2 / 98.4 %).
* Magnitude: 0–8.3 m beyond the 6 m tolerance (median 1.4 m).
* 43 of the 73 SkIdle misses had SkIdle's **car** within tolerance in the same sample (veh_pos ok). The
  ped of a seated player sits at the model's seat, not at the car's origin, while the server's position of
  a seated player is its vehicle's (driver / passenger sync carry the vehicle position). Measured
  ped-to-car distance in our client: Sandking (495) 0.5 m, Dune (573) 2.1 m along the car's forward axis
  (all bots, ped state 50 DRIVING, in-vehicle flag set); the original shows 0.2–0.7 m on the models of its
  run. Derby rotates the spawn car model on a timer (`DerbyChangeSpawnVehicleTimer`), randomly. Per model
  in the failing run: Bus (431) remote_pos 95.4 % but veh_pos 99.8 %, Dune 97.4 % / 99.2 %, Sandking
  99.3 % / 99.5 %. The original's run had neither Bus nor Dune rounds. So the miss rate depended on the
  random model mix, not on the client: **harness error** (seat position compared with a car origin).
* The rest (30) is the shared, known case (c) of "The 2-hour derby run": the parked car pushed 6–12 m by
  the other cars between the idle bot's sparse syncs. How often depends on the spot where the bot parks
  (derby spawn points; e.g. (-282, 2164) gave 16 of ours-final's 27, the original parked there once).

### veh_hp

Of the original's 15 veh_hp misses 14, of ours-a's 34 (+1 unmatched) 29, showed the client already holding
exactly the value of the server dump taken **after** the client query (`client=918 dump1=956.8 dump2=918.2`):
a health step in flight between the first dump and the client query — the sampling-order case the
bracketed comparison handles for RPC state. Nearly all are our own car (local driver: ours 33, original
13): its server copy steps with each of our driver syncs. ours-a's one persistent desync (veh1, 3 samples)
was such a sequence (a rammed bus going 796 → 291 → explosion). Our client queries four control commands
(~68 ms) where the original's `peek`s take ~15 ms, so a step falls into our window more often; with the
bracket both clients are measured against the server's state at the moment of the query.

### Memory: why 40.4 judged vs 18.9 for the same session

`slope_mb_per_h` (18.9) is one line through the whole session after 300 s. The judged slope fits each
segment between in-process reconnects separately (an intercept per segment) and pools them, so every
segment weighs by its own spread of time. Session 1's segments: 300–925 s +17.2 MB (138 MB/h), then
5.1, 8.8, 2.4 MB/h. A fresh game process is still streaming in between 300 s and ~900 s; that one segment
carried the pooled slope to 40.4. In the whole-session line the same 17 MB are spread over 3400 s.
The original shows the same late warm-up (orig-2h fresh segments 300–930 s: +7.7, +3.2, +11.5 MB = 48, 21,
56 MB/h; ours-a +9.4, +8.8, +12.6), both flat afterwards (≤ 10 MB/h) at the same plateau (session ends:
original 302 / 294 / 311 MB, ours 300 / 291 / 304, early3 308 / 307 / 305). Not a leak, on either client.
Change: the first segment of a game process starts at `WARMUP_FRESH_S` = 900 s (in-process segments keep
300 s); the 40 MB/h limit is unchanged.

### Original vs ours, 2 h derby, same cases and schedule

| category | original orig-2h (old rules) | ours early3 (main, old rules) | ours-a (old rules) | **ours-final (new rules)** | original orig-final (new rules) |
| --- | --- | --- | --- | --- | --- |
| crashes / stuck / cases | 0 / 0 / 7 of 7 ok | 0 / 0 / 7 ok | 0 / 0 / 7 ok | 0 / 0 / 7 ok | 0 / 0 / 7 ok |
| remote_known, remote_ghost, remote_stream, veh_stream, local_hp | 100 % | 100 % | 100 % | 100 % | 100 % |
| local_pos | 100 % | 99.91 % | 100 % | 100 % | 100 % |
| remote_pos | 99.75 % (16, SkIdle) | 98.83 % (74) | 99.53 % (30) | **99.57 %** (27, SkIdle) | 99.78 % (14, SkIdle) |
| remote_pos on Sandking rounds | 99.67 % | 99.33 % | 99.60 % | 99.32 % | 99.58 % |
| veh_pos | 99.80 % | 99.57 % | 99.70 % | **99.64 %** | 99.81 % |
| veh_hp | 99.80 % (14 of 15 in flight) | 99.64 % | 99.54 % (29 of 34 in flight) | **100 %** | 99.99 % (1) |
| remote_vehicle / veh_driver | 99.98 / 99.99 % | 99.95 / 99.95 % | 99.98 / 99.96 % | 100 / 100 % | 100 / 100 % |
| persistent desyncs | 0 | 0 | 1 (veh_hp, in flight) | 0 | 0 |
| fps median | 52.4 | 52.4 | 52.4 | 52.4 | 52.4 |
| memory, whole session 1 after 300 s | 10.8 MB/h | 18.9 | 12.8 | 11.5 | 15.5 |
| memory judged (new rule) | 5.2 MB/h | 5.4 | 2.9 | **6.0** (1818 s) | 13.6 (1841 s) |
| verdict | PASS | FAIL | FAIL (persistent veh_hp) | **PASS** | PASS |

(remote_hp is not read from the original. "old rules" columns: report recomputed where the rule is a
report-level one, i.e. the memory row.)

### Decisions

* remote_pos and veh_hp: harness measurement errors, fixed in the measurement (seated players via their
  car; veh_hp bracketed by the two dumps); thresholds unchanged (99 %, 6 m, ±10). Both are applied to the
  original by the same code (OrigView lists the vehicle of a seated remote player).
* The remaining position misses are SkIdle's rammed parked car on both clients (Sandking rounds: ours
  99.32–99.60 % over three 2 h runs, the original 99.46 % (30 min), 99.67 % and 99.58 % (2 h)); how often depends
  on where the idle bot parks, and our block-to-block spread within one run (98.9–99.8 %) covers the
  original's values. Shared behaviour (remote-control-vehicles.md §2.2), no client change.
* Memory: the original shows the same late warm-up of a fresh game process; the judged window starts after
  it (900 s, measured on the original), the limit stays 40 MB/h. No relative threshold was needed: with the
  measurement fixed, both clients clear the absolute thresholds with the same margins.
