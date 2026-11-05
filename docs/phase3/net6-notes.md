# NET-6 — RakNet timing parity and the F2 network diff

Branch `net-timing`. Files: `crates/samp-proto/src/connection.rs`, `crates/samp-client/src/netclient.rs`,
`tools/diff/{network.sh,netscen.sh,net_report.py,netcap.py,lossproxy.py,net-decisions.tsv}`.
A = samp.dll 0.3.7-R4 2021 build (the decompiled export / `A.asm`); our words only, no code copied.

## 1. The original's RakNet behaviour

### Static (A)

| item | rule | where |
|---|---|---|
| scheduling | RakClient::Connect(host, port, 0, 0, **threadSleepTimer 2**): RakNet runs its own update thread, `RunUpdateCycle` then `Sleep(2)`. Application `Send`s are buffered and enter the reliability layer at the start of the next cycle | A 0x10008988 (push 2), thread A 0x10042700, cycle A 0x10041450 |
| cycle order | receive every pending datagram (acks, dedup, ordering, split reassembly) → buffered sends → keep-alive test → reliability update (datagrams) → dead-connection test → ping test → internal handling of received messages (auth key, `0x22`, ping → pong, pong → ping sample) and the packet queue for the game. Replies made in the last step go out with the **next** cycle | A 0x10041450 |
| datagram generation | per datagram: pending acks first (only when the clock moved since the last full ack flush, so in practice every cycle), then the resend list head-first while due (a due resend that does not fit ends the datagram), then the four priority queues 0..3 FIFO (a message that does not fit waits; the next priority is tried); repeated while messages remain and the bandwidth bucket allows | A 0x10048070, A 0x10049ae0 |
| fit test | datagram bits + header estimate + payload bits ≤ (MTU − 28) × 8; estimate = 4 + 32 (+21 ordered/sequenced) + 17 (+97 instead when split) | A 0x10043ec0 |
| MTU | default 576 (A 0x1003de50); SetMTUSize (A 0x10038860, RakClient vtable slot 42) is never called by samp (no `[vtable+0xa8]` call on the RakClient object) | |
| split | payload > MTU − 28 − ⌈estimate/8⌉ bytes → pieces of MTU − 28 − ⌈split estimate/8⌉ bytes; first piece keeps the number, the rest take the next numbers; one split id per message (counter); the ordering index is shared | A 0x10046b00, A 0x10045800 |
| ordering | separate per-channel counters for ordered (9) and for sequenced (7 and 10 share one) | A 0x10046b00 |
| resend timeout | 3 × ping where ping = the last accepted pong sample, raised to ≥ 30 ms, 500 when above the timeout; 1000 before any sample (reset), so 3000 ms at first and 90 ms on a LAN. Resent messages are due again one timeout later: **no backoff** | SetPing A 0x10045c00 → A 0x10044070; reset A 0x10047780 |
| pong sample | accepted when its 5-slot ring entry is empty or the sample is < 3 × that entry and < 1200 ms; lowest ping tracked | A 0x10041ea0 |
| internal ping | sent by RakNet on `0x22`; the cycle pings again (5 s apart) only while no pong was accepted (`lowestPing == 0xffff`) or occasional ping is enabled (samp never enables it) | A 0x10041a2c |
| keep-alive | connected, more than 5000 ms since the last reliable send **and the resend list empty** → reliable `0x17`, low priority; base moved to now + 10000 | A 0x10041853 (statistics A 0x100479d0 +0x9c) |
| bandwidth | token bucket, 512 kbit/s start, floor 144.4 kbit/s, halved/raised on resend ratios in 0.25–0.75 s windows | A 0x10049ae0 |

### Measured (live, `tools/diff/network.sh`, trace lane; loopback through `lossproxy.py`)

| item | original |
|---|---|
| ack latency (server datagram with reliable messages → client ack) | p50 1.1 ms, p90 2.0 ms |
| ack datagrams without messages | 100 % (acks only piggy-back when a send happens in the same cycle, e.g. `acks 3,5 + RPC 25`) |
| messages per client datagram | 1.06 (one game frame's sends normally land in one cycle: `52 + 205 + 207 + 203` after spawn) |
| largest client datagram | 131–142 bytes |
| `0x22` answer | one datagram `0x1e, 0x06, 0x29, 0x17` 2.2 ms after the ack of `0x22`; a second `0x06` 2.4 ms later; then never again; RPC 25 from the game thread in its next frame |
| pong to a server ping | next cycle (2–3 ms) |
| resend after a dropped first transmission | 91.4 ms (ping on loopback < 30 ms → 3 × 30) |
| repeated loss (first 3 transmissions dropped) | resends +90.3, +180.6, +272.2 ms after the first send: no backoff |
| keep-alive | > 5 s after the last reliable send, then 15 s apart (m10-notes) |

## 2. Our implementation

* `Connection` (sans-IO) implements the rules above: priority queues (`send_with`, `priority::*`;
  every samp RPC/sync uses HIGH, internal messages SYSTEM, keep-alive LOW), the fit test with
  RakNet's header estimate, multi-datagram updates, resend list FIFO with 3 × ping, the pong ring,
  outgoing splits, keep-alive only with an empty resend list and only once connected
  (`Connection::connecting` + `mark_connected`), the update-cycle ping only until a pong, pongs
  and the cycle ping queued after the update. `Connection::new` stays "connected" for the bots.
* `NetClient` runs RakNet's part (`Core::update`) on a thread every 2 ms (the cycle order
  above); the game thread queues sends and takes the packets in `tick`. PlayerConnect is sent
  from `tick` (samp's packet handler) on `0x22`. `/quit` still sends `0x20` at once from the
  calling thread. Host unit tests run without the thread (synthetic clocks).
* Not reproduced: the bandwidth bucket. The client sends ≈ 30 datagrams/s of ≤ 150 bytes
  (≈ 40 kbit/s with headers), below the 144.4 kbit/s floor; a burst would need > 170 datagrams
  in a second. Decision NET6-D1 in ROADMAP.

### Keep-alive clock (follow-up to an m10 failure: 4.997 s on main)

RakNet keeps the keep-alive base in whole milliseconds: when the update cycle takes a buffered
reliable send it stores the cycle's start time (QPC µs / 1000, A 0x1003cc13), and the test runs
in a later cycle on the same truncated clock (`now_ms − base > 5000`, A 0x10041853). So the gap
between the two cycle starts is always more than 5000 ms; on the wire it differs from that only by
where in its cycle each datagram goes out (< 1 ms). Measured on the original (3 sessions, 33 gaps):
5.0004–5.0030 s and 15.0002–15.0024 s. The old client used the frame's start `Instant` as the base
while the datagram left at the end of the frame, so a keep-alive at the start of a later frame
could come up to one frame's work early (4.997 s). Now `Connection` keeps the base as whole ms of
its own clock taken at the update that takes the send (`clock_ms`), and the update runs on the
2 ms thread right before the datagram is sent; m10's window is 4.999–5.010 / 14.999–15.010 s.

## 3. F2: `tools/diff/network.sh`

Scenario (both clients, `tools/diff/netscen.sh`): join, spawn, walk, idle (17 s), chat + command,
dialog response, scoreboard, pickup, checkpoint, object edit + Escape, drive (enter/exit an
ambulance), passenger, shoot (Desert Eagle at samp-bot "NetBot"), loss_cs (first transmission of
two chat lines dropped), loss_repeat (one chat line's first three transmissions dropped),
loss_sc (server reliable datagrams dropped once), death, `/q`.

Checks (`tools/diff/net_report.py`, tolerances = ROADMAP NET6-D2): per window every C→S key
(kind + reliability + channel): event messages exact count, bit lengths and order, trigger latency
from the input; periodic ones count, most common length, median cadence; per session totals; RakNet:
ack latency p50/p90, share of ack-only datagrams, messages per datagram, largest datagram, never-acked
server datagrams, the pings (count and time after `0x1e`), the `0x22` answer datagram, resend delay,
resends per dropped message, the repeated-loss intervals, keep-alives while idle.

Final run (orig-20261105-054855 / ours-20261105-054855, after merging main and the keep-alive clock
fix): 303 checks, 296 match, 7 accepted (NET6-D3 object Escape → pause, D4 idle 203, D5 spectator
203; D6 209 after a passenger exit was accepted in the previous run), 0 open, exit 0.
RakNet level: ack 1.1/2.0 ms vs 1.1/2.0 ms, ack-only datagrams 100 % vs 99.9 %, 1.06 vs 1.03 messages
per datagram, 2 pings at +0 / +2 ms after `0x1e` in both, resend 91.0 vs 91.0 ms, repeated loss
intervals 91/91/91 vs 90/92/91 ms, keep-alive 5.002 s in m10 (original 5.002), every session total identical except 203 and 209.
Before NET-6 (m10 `full` capture of the previous client): only 42 % of the ack datagrams were
ack-only (acks rode on the frame's sends), a ping every 5 s, resends after 3 × smoothed RTT
clamped to 100–1000 ms.

