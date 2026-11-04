#!/usr/bin/env bash
# M8a parity loop, original side, part 2 (MR-M8A-1 sprite textdraw incl. selection,
# MR-M8A-2 model preview, MR-M8A-5 nametag of a remote driver). Setup as m8a-orig.sh.
# Part 1 header follows.
# M8a parity loop, original side: runs the textdraw / selection / menu / bubble / death-window
# part of tools/e2e/m8a.sh on the ORIGINAL client (samp.dll 0.3.7-R4 + samp-tracer) in the trace
# lane, with the same filterscript (tools/e2e/probe/m8afs.pwn) on our own server copy at the trace
# lane's port 7790. Run it under the trace-lane lock:
#   /home/ghosty2004/samp-rs/tools/trace/with-lock.sh tools/e2e/m8a-orig.sh
# Output: target/m8a-orig/ (screenshots, d3d-NAME.jsonl, server.log, trace.jsonl via run.sh).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MAIN="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
T="$MAIN/tools/trace"
OUT="$ROOT/target/m8a-orig2"
mkdir -p "$OUT"
say() { printf '[m8a-orig2] %s\n' "$*"; }
[ "${SAMP_RS_TRACE_LOCK_HELD:-0}" = 1 ] || { say "run me under $T/with-lock.sh"; exit 1; }

# Our server copy on the trace port (the trace lane's run.sh starts none: TRACE_NO_SERVER=1).
export SAMP_RS_ROOT="$MAIN" E2E_TAG=m8aorig E2E_PORT=7790 E2E_CTL_PORT=47701
source "$ROOT/tools/e2e/lib.sh"
e2e_server_prepare
( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/m8afs.pwn" \
    -iqawno/include -ofilterscripts/m8afs.amx -d0 > /dev/null ) || exit 1
python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m8afs"]
json.dump(d, open(p, "w"), indent=4)
PY
mkdir -p "$E2E_SERVER/scriptfiles"; : > "$E2E_SERVER/scriptfiles/uicmd.txt"
( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
SRV=$!
BOT=""
cleanup() {
    [ -n "$BOT" ] && kill "$BOT" 2>/dev/null
    [ -n "${SESSION:-}" ] && TRACE_SESSION="$SESSION" "$T/ui.sh" stop > /dev/null 2>&1
    sleep 8
    kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null
}
trap cleanup EXIT
e2e_wait_log '\[ui-fs\] loaded' 20 || { say "server did not start"; exit 1; }

TRACE_NO_SERVER=1 "$T/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
RUN=$!
for _ in $(seq 1 120); do
    SESSION="$(grep -oE 'session: [^ ]+' "$OUT/run.out" | head -n 1 | cut -d' ' -f2)"
    [ -n "$SESSION" ] && break
    sleep 1
done
[ -n "$SESSION" ] || { say "no session id"; cat "$OUT/run.out"; exit 1; }
export TRACE_SESSION="$SESSION"
ui() { "$T/ui.sh" "$@"; }
say "session $SESSION"
SF="$E2E_SERVER/scriptfiles/uicmd.txt"
SEQ=0
fs() {
    SEQ=$((SEQ + 1)); printf '%d %s\n' "$SEQ" "$1" >> "$SF"
    e2e_wait_log "\[ui-fs\] $SEQ done" 10 || say "command $SEQ ($1) not run"
}
e2e_wait_log "\[e2e\] class id=[0-9]+ " 120 || { say "original did not reach class selection"; exit 1; }
PID_="$(grep -E '\[e2e\] connect id=[0-9]+ name=TraceOrig' "$E2E_LOG" | tail -n 1 | sed -E 's/.*id=([0-9]+).*/\1/')"
sleep 3
for _ in 1 2 3; do ui key shift; e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8 && break; done
sleep 2
fs 'pos|L|1700.0|-2600.0|13.5|90.0'
sleep 2
cap() { ui d3d "$2" 3 "$1" > /dev/null; ui shot "$1" > /dev/null; cp -f "$MAIN/target/trace/${SESSION%%.*}/d3d-$1.jsonl" "$OUT/" 2>/dev/null; cp -f "$MAIN/target/trace/${SESSION%%.*}/$1.png" "$OUT/" 2>/dev/null; }

DISPLAY="$(grep -E '^E2E_HEADLESS_DISPLAY=' "$MAIN/target/trace/${SESSION%%.*}/hold.env" | cut -d= -f2)"
# MR-M8A-1: sprite textdraw 100×60 at (500, 20), then selection + click.
fs 'td|2|500.0|20.0|4|1|LD_SPAC:white'
TD2=$(grep -E "\[ui-fs\] $SEQ td 2 = " "$E2E_LOG" | tail -n 1 | sed -E 's/.* = ([0-9]+).*/\1/')
fs 'tdsize|2|100.0|60.0'
sleep 1
cap sprite all
fs 'tdsel|2|1'
fs 'select|FF0000FF'
DISPLAY="$DISPLAY" xdotool mousemove 1100 80; sleep 0.5
cap spritesel ui
DISPLAY="$DISPLAY" xdotool click 1; sleep 0.5
e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id $TD2\b" 10 && say "MR-M8A-1: click on sprite $TD2 logged" || say "MR-M8A-1: NO click on sprite"
ui key Escape
sleep 0.5
fs 'tdhide|2'
# MR-M8A-2: model preview 100×100 at (300, 200), model 411.
fs 'td|3|300.0|200.0|5|1|_'
fs 'tdsize|3|100.0|100.0'
fs 'tdmodel|3|411'
sleep 1
cap preview all
fs 'tdhide|3'
# MR-M8A-5: a bot driving next to us.
fs 'veh|411|1696.0|-2592.0|13.5|0.0|1|1'
( cd "$MAIN" && exec "$ROOT/target/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name Bot1 --mode drive --speed 0.0 --seconds 60 ) > "$OUT/bot.out" 2>&1 &
BOT=$!
e2e_wait_log "connect [0-9]+ Bot1" 30 || say "bot did not join"
e2e_wait_log "\[ui-fs\] state [0-9]+ [0-9]+->2 " 40 && say "MR-M8A-5: bot is driving" || say "MR-M8A-5: bot never became a driver"
sleep 2
cap vehtag ui
fs 'where|B1'
say "done; output $OUT"
