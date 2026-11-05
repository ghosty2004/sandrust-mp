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
| lost connection → first new attempt | 12.48 s after the server's last datagram (`m10_net.py lost` on the original's capture) | 12.01 s (see open items) |
| no-answer retry | 6 requests 1.0 s apart, new socket, 6.05–6.07 s per attempt | identical, 6.06–6.07 s |
| server full retry | 3.04 s | 3.00–3.02 s (see decisions) |
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
