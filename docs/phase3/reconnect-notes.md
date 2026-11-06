# Reconnect after a network gap: original vs ours (NET-R)

Trigger: the 2 h derby soak (`docs/phase3/soak-notes.md` "F7 settled"). In the `netloss` case (the soak proxy
drops everything for 15 s) the original opened one new socket at +12.2 s and rejoined on it; ours opened
one at +10.1 s and another at +16.1 s, and once that second attempt reached the server while it still held the
old player ("CONNECTION REJECTED: Unacceptable NickName", game relaunched).

## What the original does (static, build A unless noted; spec facts in docs/re/handshake.md §1)

* Lost connection: RakNet's ack timer (m10-notes "M10-P"): the connection is dead > 10000 ms after the first
  reliable message that entered an empty resend list (moved by every ack). RakPeer queues `0x21`; samp's
  handler (A 0x1000acf0 / B 0x1000ace0) disconnects, prints `Lost connection to the server. Reconnecting..`,
  runs the reset routine (`The server is restarting..`), sets state 1; the connect tick (A 0x10008940) starts
  the next attempt at once (> 3000 ms since the last one): RakClient::Connect (A 0x10034130) shuts RakNet down
  and initialises it again, so every attempt has a **new socket**.
* Requests of one attempt (RakPeer update A 0x10041450): the pending record starts with next = Connect time
  (A 0x1003e6a0); in every update cycle whose RakNet ms time is **strictly greater** than next
  (A 0x1004165d), a request goes out and next = now + 1000 (A 0x1004169c); after the 6th, the same test
  fires the failure `0x1d` instead (A 0x10041739…0x1004175a) → `The server didn't respond. Retrying..` →
  next attempt (new socket).
* **The cookie is process-global and never reset**: one writer (the `0x1a` handler, A 0x1003fdc2 /
  B 0x1003fd72), one reader (the request, A 0x100416a5 / B 0x10041655). Every attempt after the first join
  therefore starts with the last cookie the server issued. open.mp keeps its cookie table for
  `network.cookie_reseed_time` (default 300000 ms), so a reconnect within that time is accepted at its first
  answered request; after a reseed the first answered request gets a new `0x1a` and the attempt needs one
  more request.

## What ours did differently

1. `NetClient::connect` reset the cookie to 0 for every attempt. So the first request that got through after
   the gap (the 6th of the attempt started at +10.1 s, sent at +15.1 s, just after the drop ended) only
   fetched a cookie; the attempt then failed 1 s later and a second socket followed at +16.1 s. That is the
   extra socket in the soak, and the extra ~1–2 s that let the join land before the server had timed the old
   player out in one case (open.mp timed it out at +24 s there, after its own ack timer).
2. The harness, not the client: `soak_mon.py` had two methods named `say` — the chat sender (defined later)
   replaced the log printer, so every monitor event and status line was also sent as chat: through the
   control channel at once for ours (the `case-start` line went out reliably ~40 ms after the drop began,
   hence ours' lost connection at +10.07 s), typed for seconds on the game window for the original (hence its
   +12.2 s). The two clients never had the same input.
3. Request timer: ours sent requests at whole seconds from the attempt's first one (`>=`), the original at
   "previous request's ms + 1000, strictly passed" (1.001–1.002 s apart on the wire). Small, fixed as well.

## Changes

* `crates/samp-client/src/netclient.rs`: the cookie is a process-global (`static COOKIE`), only written by a
  cookie reply; the open-connection timer follows RakNet's rule (`next_open_ms`, whole-ms clock, strict
  `>`; the failure uses the same test after the 6th request). Unit tests: the cookie survives a new attempt and
  a new session; the 2nd–6th requests and the failure are not due at +1000 ms but at +1001 ms.
* `tools/e2e/soak_mon.py`: the chat sender is `chat_say` (the log `say` prints again), typed on the game
  window for **both** clients (T, text, Enter — the same input and timing); the netloss case records
  `drop_on` / `chat_sent` / `drop_off` wall times; `SOAK_NETDROP_SECS` may be a list used by the netloss
  cases in turn (e.g. `"15 40 6"`); a drop shorter than 10 s expects no lost connection; the lost line is
  only counted when it is new since the drop (our `chat` reply's header line is dropped before comparing).
* `crates/samp-dump`: `--raw` shows the cookie a request carries and the one a cookie reply issues.
* `tools/e2e/netloss_timeline.py OUTDIR`: per netloss case of a `SOAK_CAPTURE=1` soak, the client-leg
  timeline relative to the drop: sockets, requests with their cookie, replies, `0x0b`, `0x22`, RPC 25 / 139 /
  130, the chat send, the first reliable message in the gap, "first new socket − first reliable" and the
  attempt intervals.

## Measurements (derby, lane trace for the original, lane veh for ours, `SOAK_CAPTURE=1`)

    SOAK_CLIENT=orig SOAK_GAMEMODES=derby SOAK_CASES="netloss netloss netloss netloss" \
      SOAK_NETDROP_SECS="15 40 6 15" SOAK_SECS=480 SOAK_CAPTURE=1 tools/e2e/soak.sh      # original
    (same with lane veh env for ours)
    tools/e2e/netloss_timeline.py target/soak{-orig,}/derby

Times relative to `drop on`; chat Enter at +2.2 s in every case. In run 2 the server timed the old player out (reason 0) at +10…+12 s in every lost case, for both clients, so every rejoin was accepted. Runs `target/runs/{orig,ours}-netloss{1,2}`
of the worktree (run 1 of ours already had the cookie fix, run 2 also the timer fix).

| case | original | ours (fixed) |
|---|---|---|
| 15 s drop | 1 new socket at +11.00 / +11.63; requests carry the old cookie; the request at +15.0x is answered with `0x19` → `0x0b` → `0x22` → RPC 25 → InitGame; no second socket | 1 new socket at +10.51 / +12.22; old cookie; answered at +15.5 / +15.2 → join; no second socket |
| 40 s drop | 5 silent attempts (6 requests each), new socket each, then the 6th socket's request at +40.4/+40.6 accepted; attempt intervals 6.047–6.063 s | 5–6 silent attempts, intervals 6.027–6.038 s; request at +40.4/+40.8 accepted |
| 6 s drop | no new socket, no lost line (the chat's resends get through) | same |
| 15 s drop after the server reseeded its cookies (run 2, 4th case) | the 6th request (+15.71) gets a new `0x1a`, the attempt fails, 2nd socket at +16.75 joins at once with the new cookie | identical: `0x1a` at +15.76, 2nd socket at +16.78, join |
| lost connection: first new socket − first reliable send in the gap | 10.043, 10.049, 10.058, 10.046, 10.046 s | 10.035, 10.019, 10.027, 10.022, 10.028 s |

The first reliable send in the gap is whatever the game sent first after the drop: the chat line, or in derby
often a vehicle damage report (RPC 106), a death (RPC 53) or our spawn (RPC 52) — both clients follow the same
rule from it. Before the fix ours made 2 sockets in every 15 s case (`ours-a`, `ours-final`: +10.1 and +16.1).

## Residuals (ROADMAP "Decisions made autonomously")

* Ours is ~15–25 ms quicker from "connection dead" / "attempt failed" to the next attempt's first request
  (lost: 10.02–10.04 vs 10.04–10.06 s; silent attempt: 6.03–6.04 vs 6.05–6.06 s). It sits in the path packet
  → samp's frame → RakClient::Connect (stop/start waits A 0x10040700 `Sleep(15)` loop, A 0x10042934
  `Sleep(10)` loop); not measured further (needs a tracer hook on those waits). No effect on the outcome.
* `0x19` → `0x0b`: the original sends the connection request 11–12 ms after the open-connection reply, ours
  0–2 ms (both every join, not specific to the reconnect; also `0x22` → RPC 25: original 11–14 ms, ours
  3–15 ms). Open measurement request (same RakNet layer path as above).
* Nick rejection after a gap: the server times the old player out by its own ack timer (seen at +10…+24 s
  after the drop began, depending on when the server last sent something reliable). A reconnect that lands
  earlier is rejected with `Unacceptable NickName` and samp does not retry — the same for both clients; with
  the fix ours lands within ~0.5 s of the original in the same situation.

## Verification (2026-11-06)

* Ours, derby, `SOAK_CASES="netloss netloss netloss netloss" SOAK_NETDROP_SECS="15 15 40 6" SOAK_SECS=480
  SOAK_CAPTURE=1` (lane veh, `target/runs/ours-netloss3`): every case ok; 15 s drops: one new socket each
  (+11.38, +12.22), answered at +15.4 / +15.2 with the kept cookie, joined; 40 s: 5 silent attempts
  (6.028–6.036 s), the last one got a reseeded cookie (`0x1a` +40.36) and joined with it at +41.36, like the
  original's run-2 case; 6 s: no new socket, no lost line. No nick rejection, 0 crashes. The run's verdict
  failed only on remote_pos 98.96 % (3 of 288 samples, all SkIdle's rammed parked car: the shared behaviour
  of soak-notes (c)).
* `tools/e2e/m10.sh` PASS (lost 10.020 s after the first unacked reliable send, no-answer cadence 6.04 s,
  server-full median 3.039 s); `tools/diff/network.sh` (NET_OURS_LANE=veh) 387/387 checks, exit 0;
  `cargo test --workspace` 607 passed; clippy host + i686-pc-windows-gnu 0 warnings.
