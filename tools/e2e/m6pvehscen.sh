#!/usr/bin/env bash
# M6-P vehicle measurements (both clients, same input): a bot tows a trailer (200 trailer id + 210, AttachTrailer)
# and a bot flies a Dodo at z 60 in a straight line. Drive a hold session: tools/e2e/m6pvehscen.sh UI_SH
# (UI_SH = the session's tools/trace/ui.sh wrapper with TRACE_SESSION set). Compare the `rveh` records between
# the marks `tow` / `plane` / `veh-end` of the two trace.jsonl files (tools/trace/veh_report.py or by hand).
set -uo pipefail
U="${1:?usage: $0 UI_SH}"
fsid() { "$U" fs "$1" | sed -nE 's/.*veh-created ([0-9]+).*/\1/p' | head -n 1; }
"$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null; sleep 2
CAB="$(fsid 'veh|403|1700|-2630|13.6|90|1|1')"
TRL="$(fsid 'veh|435|1709|-2630|13.6|90|1|1')"
PLANE="$(fsid 'veh|593|1680|-2650|60|90|1|1')"
echo "cab=$CAB trailer=$TRL plane=$PLANE"
"$U" bot TowBot --mode drive --path line --speed 4 --seconds 45 --trailer "$TRL" > /dev/null
"$U" wait 'connect id=[0-9]+ name=TowBot' 30 > /dev/null; sleep 3
"$U" fs 'pos|B1|1695|-2630|13.6|0' > /dev/null; sleep 2
"$U" mark tow > /dev/null
"$U" fs "putin|B1|$CAB|0" > /dev/null; sleep 2
"$U" fs "attachtrailer|$TRL|$CAB" > /dev/null; sleep 15
"$U" shot v-tow > /dev/null
"$U" bot FlyBot --mode drive --path line --speed 15 --seconds 40 > /dev/null
"$U" wait 'connect id=[0-9]+ name=FlyBot' 30 > /dev/null; sleep 3
"$U" fs 'pos|B2|1675|-2650|13.6|0' > /dev/null; sleep 2
"$U" mark plane > /dev/null
"$U" fs "putin|B2|$PLANE|0" > /dev/null; sleep 12
"$U" shot v-plane > /dev/null
"$U" mark veh-end > /dev/null
"$U" log 60 | grep -E "state|veh" | grep -v "e2e\] state" | tail -n 12
