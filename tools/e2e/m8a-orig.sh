#!/usr/bin/env bash
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
OUT="$ROOT/target/m8a-orig"
mkdir -p "$OUT"
say() { printf '[m8a-orig] %s\n' "$*"; }
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

# MR-M8A-4: the style-0 dialog of TRACE-3 again (orig-vs-orig glyph boxes), then
# MR-M8A-3: list mouse — click row 1, click it again (does a second click activate?).
fs 'dialog|10|0|Caption {FF0000}red|Line one\nLine {00FF00}two\tTabbed|OK|Cancel'
sleep 1
cap dlg0 ui
ui key Escape
sleep 0.5
fs 'dialog|20|2|List caption|Row 0\n{FF0000}Row 1 red\nRow {00FF00}2 green\n\nRow 4 after empty\n|Select|Close'
sleep 1
DISPLAY="$(grep -E '^E2E_HEADLESS_DISPLAY=' "$MAIN/target/trace/${SESSION%%.*}/hold.env" | cut -d= -f2)"
DISPLAY="$DISPLAY" xdotool mousemove 500 286 click 1; sleep 1.5
cap listclick1 ui
DISPLAY="$DISPLAY" xdotool mousemove 500 286 click 1; sleep 1.5
if e2e_wait_log "\[ui-fs\] dialog-response player $PID_ id 20" 3; then
    say "MR-M8A-3: second single click (1.5 s later) answered: $(grep 'dialog-response.*id 20' "$E2E_LOG" | tail -n 1)"
else
    say "MR-M8A-3: second single click did not answer"
    DISPLAY="$DISPLAY" xdotool mousemove 500 304 click --repeat 2 --delay 80 1; sleep 1
    e2e_wait_log "\[ui-fs\] dialog-response player $PID_ id 20" 3 \
        && say "MR-M8A-3: double click answered: $(grep 'dialog-response.*id 20' "$E2E_LOG" | tail -n 1)" \
        || { say "MR-M8A-3: double click did not answer either"; ui key Escape; }
fi
sleep 0.5

# Textdraws and selection (same commands as m8a.sh).
fs 'td|0|100.0|150.0|1|1|~g~M8A ~w~textdraw'
TD0=$(grep -E "\[ui-fs\] $SEQ td 0 = " "$E2E_LOG" | tail -n 1 | sed -E 's/.* = ([0-9]+).*/\1/')
fs 'td|1|320.0|300.0|2|2|Centred'
sleep 1
cap textdraws all
fs 'tdsel|0|1'
fs 'select|FF0000FF'
DISPLAY="$DISPLAY" xdotool mousemove 300 255; sleep 0.5
cap select ui
DISPLAY="$DISPLAY" xdotool click 1; sleep 0.5
e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id $TD0\b" 10 && say "click on textdraw $TD0 logged" || say "NO click logged"
ui key Escape
e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id 65535" 10 && say "ESC cancel logged" || say "NO ESC cancel logged"
fs 'tdset|0|~y~changed'
fs 'tdhide|1'
# Menu.
fs 'menu|Shop|200.0|150.0|200.0|0.0|Apple;Pear;Plum'
ui mark menu-show
fs 'menushow'
sleep 1
cap menu all
ui down space; sleep 0.2; ui up space
e2e_wait_log "\[ui-fs\] menu-row player $PID_ row [0-9]+" 6 && say "menu row: $(grep menu-row "$E2E_LOG" | tail -n 1)" || say "NO menu row"
sleep 0.5
fs 'menushow'
ui down Return; sleep 0.2; ui up Return
e2e_wait_log "\[ui-fs\] menu-exit player $PID_" 6 && say "menu exit logged" || say "NO menu exit"
ui mark menu-end
# Death window, bot, bubble.
fs 'death|-|L|200'
fs 'death|L|L|24'
sleep 0.5
cap dw ui
( cd "$MAIN" && exec "$ROOT/target/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name Bot1 --seconds 120 ) > "$OUT/bot.out" 2>&1 &
BOT=$!
e2e_wait_log "connect [0-9]+ Bot1" 30 || say "bot did not join"
sleep 3
fs 'pos|B1|1696.0|-2600.0|13.5|270.0'
fs 'color|B1|00FF00FF'
fs 'armour|B1|100'
fs 'death|B1|L|31'
fs 'bubble|B1|FFFF00FF|50.0|30000|Hello from the bot bubble'
sleep 2
cap tags ui
fs 'gametext|3|5000|~r~M8A ~w~game text'
ui key Tab
sleep 4
cap scoreboard ui
ui key Tab
say "done; output $OUT"
