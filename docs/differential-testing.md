# Differential testing against the original client

Goal: prove that our client sends the same bytes as the original SA:MP 0.3.7
client for the same actions (and decodes the same server traffic).

## 1. Capture

Run a local server (`server/`, UDP 7777) and capture loopback traffic while a
client connects, spawns and performs a scripted set of actions.

- Linux: `dumpcap -i lo -f "udp port 7777" -w original.pcapng`
  (needs the `wireshark` group; `tcpdump -i lo -w original.pcap udp port 7777` as root also works).
- Windows: Wireshark with the Npcap loopback adapter, filter `udp port 7777`,
  save as pcapng or pcap.

Capture our bot the same way (`cargo run -p samp-bot -- 127.0.0.1:7777`).

## 2. Decode and compare

```
cargo run -p samp-dump -- original.pcapng --port 7777 > original.log
cargo run -p samp-dump -- ours.pcapng     --port 7777 > ours.log
diff -u original.log ours.log
```

`samp-dump` de-obfuscates client→server datagrams, drops transport noise
(acks, message numbers, ordering indices, pings, keep-alives, resends) and
prints one line per logical message. Use `--raw` to keep transport details
and timestamps.

## 3. Expected, legitimate differences

- Nickname, client serial, auth challenge/response, join challenge.
- Positions, timings and anything derived from game state.
- Message counts (sync rate depends on streamed players and frame timing).

Everything else — message IDs, reliabilities, channels, field order and
widths, empty-payload quirks — should match exactly. Record any mismatch in
the relevant `docs/re/` spec and fix the code.

## Linux shortcut (Wine environment)

```
tools/capture.sh orig 120 tools/run-samp.sh            # original client: connect, Shift to spawn, walk
tools/capture.sh ours 30  cargo run -q -p samp-bot -- --seconds 25
diff <(grep '^C>S' captures/orig.log) <(grep '^C>S' captures/ours.log)
```

First result (2026-11-04): up to class request, every client→server message matches
the original (after masking auth response and join values); this exposed the
missing empty `0x29` static-data message, now fixed.

Second result (2026-11-04, original client spawned and walked/aimed/shot):
packet types, sizes, reliabilities and channels all match the specs —
207 68 B rel7 ch1, 203 31 B rel7 ch1, 206 40 B rel7 ch0, 205 8 B rel6 (≈1/s),
204 8/12 B rel6, RPC 52 rel9 ch0. The bot now also sends 205 once per second;
it matches the original for 25/128/129/52/205/207. 203/204/206 come from real
aiming, weapon changes and shooting and are not produced by the headless bot.
