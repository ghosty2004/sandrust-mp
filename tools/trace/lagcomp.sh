#!/usr/bin/env bash
# TRACE-6: per-lag-comp-mode checks in a running hold session (start it with TRACE_GAMEMODE=tracegm
# TRACE_LAGCOMP=N tools/trace/run.sh orig hold). 1) the local player fires an un-aimed combat
# shotgun at an idle bot 4.5 m ahead (206 offset semantics), and a Deagle at the ground;
# 2) a shooting bot sends 206 hit type 1 and then hit type 0 at the local player (local damage / 115).
set -uo pipefail
U="$(cd "$(dirname "$0")" && pwd)/ui.sh"
mark() { "$U" mark "$1 wall=$(date +%s.%N)" > /dev/null; }
"$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null
"$U" bot IdleBot --mode idle --seconds 3000
sleep 9
"$U" fs 'pos|B1|1700.12|-2595.5|13.55|180' > /dev/null
sleep 2
for i in 1 2; do
    "$U" fs 'weapon|L|27|300' > /dev/null; "$U" fs 'hp|B1|100' > /dev/null
    sleep 1.5; mark "lc27-$i"; "$U" fire 150; sleep 2
done
"$U" fs 'pos|B1|1720|-2595.5|13.55|180' > /dev/null
"$U" fs 'weapon|L|24|300' > /dev/null
sleep 1.5
"$U" mouse mousedown 3; sleep 1
for i in 1 2 3 4 5 6 7 8 9 10; do "$U" mouse mousemove_relative -- 0 -4; sleep 0.05; done
sleep 0.5; mark "lc24ground"; "$U" fire 150; sleep 1.5
"$U" mouse mouseup 3
OUT="$(cd "$(dirname "$0")/../.." && pwd)/target/trace/${TRACE_SESSION%%.*}"
for p in $(cat "$OUT/bots.pids"); do kill "$p" 2>/dev/null; done
: > "$OUT/bots.pids"
sleep 2
for ht in 1 0; do
    "$U" fs 'hp|L|100' > /dev/null
    "$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null
    "$U" bot "ShootBot$ht" --mode shoot --target TraceOrig --weapon 24 --hit-type "$ht" --fire-ms 1500 --seconds 3000
    sleep 9
    "$U" fs "pos|B1|1700|-2590|13.55|180" > /dev/null
    mark "lcbot-ht$ht"
    "$U" fs 'weapon|B1|24|500' > /dev/null
    sleep 3.5
    for p in $(cat "$OUT/bots.pids"); do kill "$p" 2>/dev/null; done
    : > "$OUT/bots.pids"
    mark "lcbot-ht$ht-done"
    sleep 6
done
mark "lcdone"
"$U" log 120 | grep -E "shot|damage|death" | cut -c30-
