# Phase 3 / M10 — sa-mp.cfg, local commands, chatlog, F8, connection lifecycle

Branch `audio-and-client` (git worktree `~/samp-rs-wt/m9`, lane "ui"). Acceptance: `tools/e2e/m10.sh`
(five game sessions: `cfg`, `lost`, `full`, `nosrv`; capture checks in `tools/e2e/m10_net.py`).
Spec: `docs/phase3/trace-notes.md` "Client features (TRACE-8)" (measured) plus the static reads below
(build B addresses, objdump).

## sa-mp.cfg (`samp-ui/src/config.rs`)

* Reader = samp's loader (B 0x10066230) and classifier (B 0x10065f20): lines of at most 255 bytes,
  leading blanks skipped, `;` / `[` / empty lines ignored, key up to blank/`=`/`;`/tab, lower-cased;
  blanks, `=`, blanks, value to the line end with trailing blanks/CR removed; `"…"` (first and last
  character a quote) → string, a `.` anywhere → float (`atof`), else integer (`atol`): an unquoted
  `fontface=Courier New` is 0 and is written back as `fontface=0` (measured). Lookups lower-case the key
  (B 0x10065c50).
* Defaults created at start-up and the CR LF rewrite were already in place (M4).
* Boot effects (B 0x100c4e2c..0x100c4efa, `ui/glue.rs` `boot_settings`): `multicore` = 0 →
  `SetProcessAffinityMask(GetCurrentProcess(), 1)`; the audio object and BASS start-up with
  `audioproxyoff` (m9-notes); `disableheadmove` clears the head-look gate (Game+0x59, tested first by
  B 0x10002fa0; `client.rs` `sync_tick`); `fpslimit` (frame limiter, M4/parity) unchanged.
* `directmode` (chat construction B 0x10068190): type-8 line `ChatWindow: Using direct drawing mode.`
  (before "Started"; again after a device reset, which re-runs that routine) and no chat render target
  (`Renderer::new(.., direct_mode)`).
* fonts (`fontsize`, `fontweight`, quoted `fontface`), `timestamp`, `pagesize`, `nonametagstatus`,
  `audiomsgoff` were already read at start-up (M4/M8a); `nohudscalefix` and `logurls` are stored only
  (HUD-scale patch owner / dormant in R4).

## Local commands, files

* `/rs` in a car writes `rawvehicles.txt` relative to the game's current directory (measured), the other
  files stay in `<user files>\SAMP` (`ui/core.rs`).
* Lines/keys of `/fpslimit`, `/headmove`, `/nametagstatus`, `/timestamp` (no line), `/audiomsg`,
  `/logurls`, `/hudscalefix` (no line), `/togobjlight` (no line, no key), `/mem`, `/interior`, `/cmpstat`
  (nothing) match the measured run (checked by `m10.sh` session `cfg`).
* chatlog.txt (truncated at start, `[HH:MM:SS] ` + text with codes + CR CR LF) and the F8 file name /
  format (`screens\sa-mp-NNN.png`, next free number, D3DX PNG of the front buffer: 1280×720, 8-bit RGB,
  no text chunks) were already implemented and are now checked.

## Connection lifecycle (`crates/samp-client/src/lifecycle.rs`, `netclient.rs`, `client.rs`)

samp's netgame state machine replaces the M4 connect timer: state 1 → connect tick when more than
3000 ms passed since the last attempt (B 0x10008930: print `Connecting to H:P...`, RakClient Connect,
remember the time, state 2); `0x22` → state 6 + `Connected. Joining the game...`; InitGame → state 5;
in-game processing runs only in state 5, the "not connected" scene in every other state.
`NetClient` now has no socket until `connect`, and every attempt opens a new one (measured: new source
port), sends at most 6 open-connection requests 1.0 s apart and fails 1.05 s after the 6th (measured
6.05–6.07 s per attempt), and reports a lost connection after 12.0 s without a server datagram.
Reactions (packet handler B 0x1000b250 and its callees):

| event | lines | other effects |
|---|---|---|
| 0x1D no answer | `The server didn't respond. Retrying..` (8) | state 1 → next attempt in the same frame |
| 0x1F full | `The server is full. Retrying...` (8) | state 1 → next attempt ~3.04 s after the previous |
| 0x20 server closed (kick) | `Server closed the connection.` (8) | audio stop (wait), Disconnect: the ack of the 0x20 still goes out, then nothing; no state change, no reset: the world stays |
| 0x21 lost | `Lost connection to the server. Reconnecting..` (8), `The server is restarting..` (4) | Disconnect, RPC-40 reset, all remote players deleted, audio stop (wait), state 1 → reconnect at once |
| 0x23 / 0x24 | `Failed to initialize encryption.` / `You are banned from this server.` (8) | none |
| 0x25 | `Wrong server password.` (8) | Disconnect; no retry |
| RPC 130 | `CONNECTION REJECTED: …` (4; reason 2 + 3 hint lines) | Disconnect(500); no retry |
| RPC 40 (gmx) | `The server is restarting..` (4, at the end) | reset: remote peds torn down (players kept), local player not spawned, object/entity/vehicle pools cleared, textdraws and menus cleared, server settings to samp's defaults (12:00, weather 1, zone names off, CJ walk off, gravity 0.008, name tags on / 60 m / LOS), interior 0 (`0x04BB(0)`, `0x0860(char, 0)`, `0x04E4` at the ped), the local ped killed (fires out, teleport to its own position, health 0, `0x0321(char)` with `PlayerInFocus` around it); state 11 until InitGame |

Static sources: lost B 0x1000ace0, server closed B 0x10008dd0, reset B 0x1000a530, interior
B 0x100ad350, kill B 0x100affe0, RPC 130 B 0x10010520 (all lines are samp's strings).

## Parity results (ours vs the original's TRACE-8 runs)

| item | original (measured) | ours (`tools/e2e/m10.sh`) |
|---|---|---|
| gmx frame (tracer, `diff.py --from gmx --to gmx_done`) | `0x04BB(0)`, `0x0860(1,0)`, `0x04E4(x,y)`, `CPed::Teleport` own position, `0x0321(1)` (damage 1000, weapon 34), then the not-connected scene every frame (`orig-20261105-010108`) | identical sequence (plus our own vehicle pool deletion, the original had no vehicle) |
| kick: client datagrams after the server's 0x20 | one ack after 2 ms, nothing else for 30 s | one after 3.5–4.9 ms, nothing else; no reconnect, vehicles/world kept |
| lost connection → first new attempt | 12.48 s after the server's last datagram (`m10_net.py lost` on the original's capture) | 12.01 s (superseded by M10-P: RakNet's ack-timer rule) |
| no-answer retry | 6 requests 1.0 s apart, new socket, 6.05–6.07 s per attempt | identical, 6.06–6.07 s |
| server full retry | 3.04 s | 3.00–3.02 s (superseded by M10-P: 3.03–3.04 s) |
| wrong password | line, Disconnect(0), no retry | identical (one answered attempt, no further socket) |
| nick rejected after the resume | 4 type-4 lines, no retry | identical |
| cfg boot effects, local commands, chatlog, F8 | trace-notes values | all checked by session `cfg` (fps 31.1 at `fpslimit=30`, affinity 1) |

## Decisions / open items

* Server-full retry cadence 3.00–3.02 s vs the original's 3.04 s: samp stores the attempt time after
  `RakClient::Connect` returns (B 0x10008987), so the original's cadence includes the time its RakNet
  takes to start a connection (~30–40 ms, not measured separately); ours connects without a delay.
* Lost connection: we report it 12.0 s after the last server datagram (trace-notes); in the original's
  capture the reconnect came 12.41–12.48 s after it, so RakNet's exact rule (last ack vs oldest unacked
  reliable send) is still open — measurement request.
* open.mp blocks a same-IP open-connection request within 30 s of the last accepted one when one slot is
  left ("server full attack (2)"); `m10.sh` session `full` keeps its samp-bot for 50 s so our join comes
  later than that (server behaviour, the same for both clients).

* `/quit`: we keep sending the disconnection notification RELIABLE_ORDERED on channel 0 (as captured
  for the original's `/quit` in `orig-20261105-003749`, net-55311); trace-notes say "rel 6", which
  other original captures show for game ends without `/quit`. Measurement request: the original's
  `/quit` packet once more, plus its pool teardown order.
* RPC 40 reset: the kill routine's task branch (B 0x100acd70 when the ped's primary task has vtable
  0x8705C4) and the game-wrapper resets 0x100a20d0 / 0x100abdc0 / 0x100a7290 / 0x100a1430 /
  0x100a1470 are not reproduced (no observable effect in the measured gmx frame); the pool deletion of
  pickups/labels/gang zones goes through our M6 `clear()`.
* Banned (0x24) and wrong version (RPC 130 reason 1): implemented from the static texts only; could not
  be produced live against open.mp (TODO-unknown in trace-notes).

## M10-P — lifecycle parity follow-ups

Branch `lifecycle-fixes` (worktree `~/samp-rs-wt/m10p`, lane "ui"). Static reads are build A (2021,
decompiled export + objdump); live runs of the original: `orig-20261105-035923` (ClientCheck request set,
server SIGSTOP 40 s) plus the earlier TRACE-8 captures. Checks: `tools/e2e/m10.sh` / `m10_net.py`.

### audioproxyoff (item 1)
samp's audio set-up (A 0x10066480) creates `audioproxyoff=0` only inside the `BASS_Init` success branch,
then reads it. Ours: `SampCfg::apply_startup_defaults` creates `fpslimit`/`multicore` only;
`SampCfg::ensure_audio_proxy_key` runs from `audio::init`'s success branch (`UiCore::audio_proxy_key`
rewrites the file). m10 session `lost` starts from a cfg without the key: it is the file's last key at the
end and the log shows `BASS ready`. The failure branch is covered by the unit test only: GTA itself does not
start under Wine without an audio device (tried: no Pulse server, no ALSA config → no game window).

### Lost connection: RakNet's rule (item 2)
* Dead connection (reliability layer update A 0x10049ae0, A 0x10049b8b): an "ack timer" (64-bit µs time)
  is set to the transmission time of a reliable message entering an empty resend list (A 0x10045b70), moved
  to the receive time of every acknowledged number while messages stay pending, cleared when none are
  (A 0x10049390). The connection is dead when the resend list is non-empty, the timer is set and now − timer
  > timeout × 1000 µs. Timeout = 10000 ms (reliability-layer default A 0x10048e10; samp never calls
  RakClient::SetTimeoutTime, vtable slot 46). RakPeer's update cycle (A 0x10041450) then queues `0x21`.
* Keep-alive (A 0x10041853): in the connected state, when more than 5000 ms passed since the last reliable
  send the application queued (RakPeer::Send stores the time for reliabilities 8/9/10, A 0x1003cbf9), a
  reliable `0x17` is sent and the base moved to now + timeout, so idle keep-alives are 15 s apart.
* So the 12.0/12.41–12.48 s "after the server's last packet" were the keep-alive phase plus 10 s: measured
  on the original 10.050 s (010108: keep-alive 112.508 → new socket 122.558) and 10.049 s (035923, 20.96 s
  after the server's last packet) from the first unacked reliable message to the reconnect; keep-alive gaps
  5.002 / 15.002 s.
* Ours (`samp-proto` `Connection::is_dead` / keep-alive in `poll`, `NetClient::tick`): 10.030 s; keep-alive
  5.013 s. `m10_net.py lost` checks 10.0–10.12 s and the keep-alive gaps (5.00–5.08 / 15.00–15.08).

### Connect timing (item 3)
RakClient::Connect (A 0x10034130) first shuts RakNet down (RakPeer::Disconnect(100): one `Sleep(15)` poll for
the update thread's exit, A 0x10040700) and starts it again (Initialize: socket, update thread with a 2 ms
sleep, one `Sleep(10)` poll for its start, A 0x10042934), then queues the request; samp reads GetTickCount
again after Connect returns (A 0x10008985). Ours: `NetClient::connect` makes the same two waits on the main
thread (our network code has no thread; the original blocks its main thread the same way), Disconnect paths
make the stop wait, `NetGame::attempt_started` takes the time after `connect` returns, the no-answer
failure is "more than 1000 ms after the 6th request" (RakNet's rule) instead of the old 1050 ms fudge.
Result: server-full cadence 3.03–3.04 s (median 3.039; original 3.03–3.04), no-answer cadence 6.04 s
(original 6.05–6.07). Single frame stalls of the loading world appear as outliers (3.20/3.32 s seen once).

### /quit (item 4)
* Packet: every original capture's client `0x20` is UNRELIABLE (`rel6`): `orig-20261105-003749` net-42564
  (TraceOrig, the original; a 5-byte datagram), 001409, 001923, 002451. The `rel9 ch0` packet at net-55311 is
  SpecBot (our samp-bot), which explains the earlier contradiction. Statically RakNet's notification
  (A 0x1003e8f0) sends reliability 6 on the given ordering channel (0); RakClient::Disconnect with block time 0
  sends none.
* Order (static): `/quit` (A 0x100c3ec0) calls RakClient Disconnect(500, 0) only in netgame state 5, sets the
  quit flag and its GetTickCount; the frame routine (A 0x100c5030) keeps running the game and once
  ≥ 1001 ms passed deletes the netgame (A 0x10009880: Disconnect(0) again, then the pools players, vehicles,
  actors, textdraws, gang zones, labels, pickups, objects, menus) and calls `ExitProcess(0)`. The trace of
  003749 shows the exit frame: vehicle marker removal, the cars deleted in id order, the local driver warped
  out (0x0362) before its car's 0x00a6 — the trace-notes line "tears down its pools first" was wrong about
  the order relative to the packet.
* Ours (`dll.rs` `request_quit` / `quit_tick`, `Client::begin_quit` / `quit_teardown`): the same; m10 session
  `full`: `rel6 0x20` 0.9–1.1 ms after `/quit`, nothing after it, exit 1.006–1.015 s later, exit frame
  `0x0362` then `0x00a6`.

### ClientCheck (item 5) — decision: answered honestly
* Algorithm (A 0x10011A60, checksum A 0x1000EA80, in our words): the 8-bit result is the XOR of every byte
  of the range ANDed with 0xCC. Type 5: range = arg + offset in the process (arg must lie in
  0x400000..0x856E00, else no reply); 0x45: range = the answering module's own image base + arg + offset
  (arg ≤ 800000); 0x46: GTA's model info entry of model arg (0..20000) + offset (no entry → result 0);
  0x47: the model is requested/loaded if needed (HAS_MODEL_LOADED polled up to 1000 × Sleep(1)), the range
  is its collision model + offset, the model is marked no longer needed if its reference count is 0; no
  model → result 0. Reply arg = request arg. Confirmed behaviourally ("MZ\x90\0" → 0x84).
* Ours (`behaviourclient.rs` `client_check`): the same algorithm on our own process's real memory; 0x45
  reads samp_client.dll's image (never samp.dll's values); unreadable ranges get no reply (samp would
  fault). No reply value is stored, replayed or substituted.
* Parity on the original's request set (m10 session `full`): 16/16 replies, the 4 requests without a reply
  match; equal values for type 2, 5 (header, code, upper bound), 0x45 at 0 ("MZ"), 0x46/0x47 model 411 and
  1337, 0x46 model 0 and 20001, 0x48. Different, as they must be: 0x53E981 (both clients rewrite that call
  with a call into their own module: ours 0x04, samp 0xC4), 0x45 at 0x1000 (our module), model 400 ranges
  that contain heap pointers (CVehicleModelInfo RwObject/col pointers, CColModel data pointer).

### m5 watch (item 6)
Reproduced once in three m5 runs (run 2 of the suite): not a hang of our client but a crash of GTA's own
radio stream thread right after the local player entered car A as driver — `wine: Unhandled division by zero
at address 004F1464` (`CAEStreamThread`: the play position modulo the decoder's stream length, which is 0),
after which the Wine debugger freezes the process (control channel and log silent, which looked like a hang).
The identical crash was recorded for the ORIGINAL client in the trace lane (trace-notes "Hazards seen in the
trace lane": entering a Sultan, division by zero at 0x4F1464). It is GTA-under-Wine behaviour (a radio track
whose decoder reports length 0), independent of samp/our client; no client change. Run logs:
`target/runs/m5-{0,1,2}.log` of the worktree (exit 0, 0, 1).

### Verification (lane ui, under the lock)
`tools/e2e/m10.sh`: PASS (cfg, lost 10.044 s / keep-alive 5.008 s, full median 3.040 s + `/quit` + ClientCheck
16/16, nosrv 6.04 s). Suite: m1, saa, m2, m3, goal, m4, m5 (5 runs: 4 exit 0, 1 = GTA radio-thread crash above),
m6, m7, m8a, m9 (with `M9_ORIG` = the main tree's `target/m9-orig`: bass_diff 0/16), ux1, m10 exit 0. m8b fails at
"walked through maxX 1720" (world bounds) on this branch AND identically on the unmodified branch head d7f3173
(stash run): pre-existing, not caused by M10-P. `cargo test --workspace` 564 passed; clippy host + i686 0 warnings.
