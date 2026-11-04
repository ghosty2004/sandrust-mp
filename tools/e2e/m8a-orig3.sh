#!/usr/bin/env bash
# M8a leftovers + input lock, original side (trace lane, samp.dll 0.3.7-R4 + samp-tracer, our server
# copy with tools/e2e/probe/m8afs.pwn on the trace port 7790). Takes the trace-lane lock itself.
#   tools/e2e/m8a-orig3.sh
# Measures:
#  * input lock (samp's setter A 0x100a0750 / B 0x100a0720, restore A 0x100a0630): game object
#    (0x1026ea24) +0x61 mode / +0x65 countdown and the patched bytes at 0x541DF5, 0x53F417,
#    0x53F41F, 0x6194A0, 0x53F47A/9A/B3 in: idle, dialog, after ESC (at once and 1 s later),
#    SelectTextDraw + ESC, EditObject (+ while the mouse moves) + ESC, SelectObject + ESC
#    → lock.txt
#  * F1 (held) help dialog, F5 (held) net stats, F10 (held), F8 screenshot (d3d all of the
#    following frames, file names in <user files>\SAMP\screens) → d3d-*.jsonl, *.png
#  * clipboard: Ctrl+V into the chat input from the X clipboard; Ctrl+C after Shift+Home
#  * `mdl123:abc` sprite textdraw (font 4), model preview (font 5) creation frames (d3d all)
#  * remote driver nametag: samp-bot --mode drive in a car next to us → d3d-vehtag
# Output: target/m8a-orig3/.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/wine" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/wine" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
T="$ROOT/tools/trace"
if [ "${SAMP_RS_TRACE_LOCK_HELD:-0}" != 1 ]; then
    exec "$T/with-lock.sh" "$0" "$@"
fi
OUT="$ROOT/target/m8a-orig3"
rm -rf "$OUT"; mkdir -p "$OUT"
say() { printf '[m8a-orig3] %s\n' "$*" | tee -a "$OUT/log.txt"; }

export E2E_TAG=m8aorig E2E_PORT=7790 E2E_CTL_PORT=47701
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
SCREENS="$SAMP_RS_ROOT/wine/prefix-trace/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP/screens"
cleanup() {
    [ -n "$BOT" ] && kill "$BOT" 2>/dev/null
    [ -n "${SESSION:-}" ] && "$T/ui.sh" stop > /dev/null 2>&1
    sleep 8
    kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null
    ls -la "$SCREENS" > "$OUT/screens-after.txt" 2>&1
}
trap cleanup EXIT
e2e_wait_log '\[ui-fs\] loaded' 20 || { say "server did not start"; exit 1; }
ls -la "$SCREENS" > "$OUT/screens-before.txt" 2>&1

TRACE_NO_SERVER=1 "$T/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
for _ in $(seq 1 180); do
    SESSION="$(grep -oE 'session: [^ ]+' "$OUT/run.out" | head -n 1 | cut -d' ' -f2)"
    [ -n "$SESSION" ] && break
    sleep 1
done
[ -n "${SESSION:-}" ] || { say "no session id"; cat "$OUT/run.out"; exit 1; }
export TRACE_SESSION="$SESSION"
TDIR="$ROOT/target/trace/${SESSION%%.*}"
ln -sfn "$TDIR" "$OUT/trace"
ui() { "$T/ui.sh" "$@"; }
say "session $SESSION"
DISPLAY="$(grep -E '^E2E_HEADLESS_DISPLAY=' "$TDIR/hold.env" | cut -d= -f2)"
export DISPLAY
SF="$E2E_SERVER/scriptfiles/uicmd.txt"
SEQ=0
fs() {
    SEQ=$((SEQ + 1)); printf '%d %s\n' "$SEQ" "$1" >> "$SF"
    e2e_wait_log "\[ui-fs\] $SEQ done" 10 || say "command $SEQ ($1) not run"
}
peek() { ui ctl "peek $1 $2" | awk '{print $3}'; }
lock() {
    local obj mode
    obj=$(peek 0x1026ea24 4)
    obj=$(python3 -c "import sys; print(hex(int.from_bytes(bytes.fromhex('$obj'), 'little')))")
    mode=$(peek $((obj + 0x61)) 8)
    say "lock $1: obj=$obj mode+countdown=$mode 541df5=$(peek 0x541DF5 5) 53f417=$(peek 0x53F417 5) 53f41f=$(peek 0x53F41F 4) 6194a0=$(peek 0x6194A0 1) 53f47a=$(peek 0x53F47A 4) 53f49a=$(peek 0x53F49A 4) 53f4b3=$(peek 0x53F4B3 4) b7340c=$(peek 0xB7340C 12)"
}
cap() { ui d3d "$2" "${3:-3}" "$1" > /dev/null; ui shot "$1" > /dev/null; cp -f "$TDIR/d3d-$1.jsonl" "$TDIR/$1.png" "$OUT/" 2>/dev/null; }

e2e_wait_log "\[e2e\] class id=[0-9]+ " 180 || { say "original did not reach class selection"; exit 1; }
PID_="$(grep -E '\[e2e\] connect id=[0-9]+ name=TraceOrig' "$E2E_LOG" | tail -n 1 | sed -E 's/.*id=([0-9]+).*/\1/')"
lock class-selection
sleep 3
for _ in 1 2 3; do ui key shift; e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8 && break; done
sleep 2
fs 'pos|L|1700.0|-2600.0|13.5|90.0'
sleep 2
lock idle

# ---- input lock ----
fs 'dialog|10|0|Caption|Body|OK|Cancel'
sleep 1
lock dialog
ui key Escape
lock dialog-esc-0s
sleep 1
lock dialog-esc-1s
fs 'td|0|100.0|150.0|1|1|Select me'
fs 'tdsel|0|1'
fs 'select|FF0000FF'
sleep 1
lock textdraw-select
ui key Escape
lock textdraw-esc-0s
sleep 1
lock textdraw-esc-1s
fs 'tdhide|0'
fs 'obj|1337|1697.0|-2600.0|13.5|0.0|0.0|0.0|200.0'
OBJ=$(grep -E "\[ui-fs\] $SEQ object " "$E2E_LOG" | tail -n 1 | sed -E 's/.* object ([0-9]+).*/\1/')
say "object $OBJ"
sleep 1
ui mark objedit
fs "objedit|L|$OBJ"
sleep 1
lock edit
cap edit ui
xdotool mousemove_relative -- 30 10; sleep 0.5
lock edit-mouse
xdotool keydown space; sleep 0.5
lock edit-space-held
xdotool keyup space; sleep 0.3
lock edit-space-up
ui mark edit-esc
ui key Escape
lock edit-esc-0s
sleep 1
lock edit-esc-1s
grep -E "edit-object|select-object" "$E2E_LOG" | tail -n 3 | while read -r l; do say "server: $l"; done
ui mark objsel
fs 'objsel|L'
sleep 1
lock select
ui key Escape
lock select-esc-0s
sleep 1
lock select-esc-1s
fs "objdel|$OBJ"

# ---- F keys ----
ui down F1; sleep 0.5
cap help ui
ui up F1; sleep 0.3
lock help
ui key Return; sleep 0.5
grep -c "dialog-response" "$E2E_LOG" | while read -r n; do say "dialog responses so far: $n (help must add none)"; done
ui down F5; sleep 1.5
cap netstats ui
ui shot netstats-2 > /dev/null
ui up F5; sleep 0.3
ui down F10; sleep 0.5
cap f10 ui
ui up F10; sleep 0.3
ui d3d-start all 3 f8 > /dev/null
ui key F8
ui d3d-collect f8 > /dev/null
cp -f "$TDIR/d3d-f8.jsonl" "$OUT/" 2>/dev/null
sleep 1
ui key F8; sleep 1
cap after-f8 ui
ls -la "$SCREENS" > "$OUT/screens-f8.txt" 2>&1

# ---- clipboard ----
printf 'clip text 1' | xclip -selection clipboard
sleep 0.3
ui key t; sleep 0.5
ui key ctrl+v; sleep 0.5
cap paste ui
ui key shift+Home; sleep 0.2
ui key ctrl+c; sleep 0.5
say "clipboard after ctrl+c: '$(timeout 3 xclip -o -selection clipboard 2>&1)'"
ui key Escape; sleep 0.5

# ---- sprites / model preview ----
fs 'td|2|500.0|20.0|4|1|mdl123:abc'
fs 'tdsize|2|100.0|60.0'
sleep 1
cap mdl all
fs 'tdhide|2'
fs 'td|3|300.0|200.0|5|1|_'
fs 'tdsize|3|100.0|100.0'
ui d3d-start all 4 preview-create > /dev/null
fs 'tdmodel|3|411'
ui d3d-collect preview-create > /dev/null
cp -f "$TDIR/d3d-preview-create.jsonl" "$OUT/" 2>/dev/null
sleep 1
cap preview all
fs 'tdhide|3'

# ---- remote driver nametag ----
fs 'veh|411|1696.0|-2592.0|13.5|0.0|1|1'
( cd "$SAMP_RS_ROOT" && exec "${CARGO_TARGET_DIR:-$ROOT/target}/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name Bot1 --mode drive --speed 0.0 --seconds 90 ) > "$OUT/bot.out" 2>&1 &
BOT=$!
e2e_wait_log "\[e2e\] connect id=[0-9]+ name=Bot1" 30 || say "bot did not join"
BID="$(grep -E '\[e2e\] connect id=[0-9]+ name=Bot1' "$E2E_LOG" | tail -n 1 | sed -E 's/.*id=([0-9]+).*/\1/')"
e2e_wait_log "\[e2e\] spawn id=$BID\b" 30 || say "bot did not spawn"
fs 'pos|B1|1694.0|-2592.0|13.5|0.0'
for _ in $(seq 1 40); do
    fs 'where|B1'
    grep -E "\[ui-fs\] $SEQ where " "$E2E_LOG" | grep -q "state 2 " && break
    sleep 1
done
say "bot: $(grep -E "\[ui-fs\] $SEQ where " "$E2E_LOG" | tail -n 1)"
sleep 2
cap vehtag ui
say "done; output $OUT"
