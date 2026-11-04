#!/usr/bin/env bash
# TRACE-6: local player shoots a remote (samp-bot "ShootBot", idle) in a running hold session
# (tools/trace/run.sh orig hold, best with TRACE_GAMEMODE=tracegm and TRACE_LAGCOMP=0|1|2).
# The bot stands 4 m ahead on the ray of an un-aimed shot (LMB without RMB, camera behind the
# player). Per weapon: mark "cw<weapon> wall=<epoch>", one trigger pull; then fists at 0.8 m,
# a sniper miss into the sky and a same-team shot. Outputs are in the session dir; decode with
#   samp-dump OUT/net.pcapng --port 7790 --raw | target/debug/examples/c2s_decode --only 206,115,204
set -uo pipefail
U="$(cd "$(dirname "$0")" && pwd)/ui.sh"
mark() { "$U" mark "$1 wall=$(date +%s.%N)" > /dev/null; }
"$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null
"$U" bot ShootBot --mode idle --seconds 3000
sleep 9
"$U" fs 'pos|B1|1700.5|-2595.5|13.55|180' > /dev/null
sleep 2
for w in 24 25 27 31 22; do
    "$U" fs "weapon|L|$w|300" > /dev/null
    "$U" fs 'hp|B1|100' > /dev/null
    sleep 1.5
    mark "cw$w"
    if [ "$w" = 31 ]; then "$U" fire 450; else "$U" fire 150; fi
    sleep 2
done
# Fists at close range.
"$U" fs 'resetw|L' > /dev/null
"$U" fs 'pos|B1|1700|-2599.1|13.55|180' > /dev/null
sleep 2
mark "cw0"
"$U" fire 150; sleep 0.8; "$U" fire 150
sleep 2
# Sniper: aim up into the sky (miss), then level (ground/bot).
"$U" fs 'pos|B1|1700.5|-2595.5|13.55|180' > /dev/null
"$U" fs 'weapon|L|34|20' > /dev/null
sleep 1.5
"$U" mouse mousedown 3; sleep 1.2
for i in 1 2 3 4 5 6 7 8; do "$U" mouse mousemove_relative -- 0 12; sleep 0.05; done
sleep 0.5
mark "cw34sky"
"$U" fire 150; sleep 1.5
"$U" mouse mouseup 3
sleep 1
# Same team: both team 1, Deagle.
"$U" fs 'team|L|1' > /dev/null; "$U" fs 'team|B1|1' > /dev/null
"$U" fs 'weapon|L|24|300' > /dev/null
"$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null
sleep 2
mark "cwteam"
"$U" fire 150
sleep 2
"$U" fs 'team|L|255' > /dev/null; "$U" fs 'team|B1|255' > /dev/null
mark "cwdone"
"$U" log 80 | grep -E "shot|damage|death" | cut -c30-
