#!/usr/bin/env bash
# M9 parity loop, original side: tools/e2e/m9scen.sh on the ORIGINAL client (samp.dll 0.3.7-R4 +
# samp-tracer with the BASS call log) in the trace lane, with the same filterscript
# (tools/trace/uifs.pwn) on our own server copy at the trace lane's port 7790 and
# tools/trace/icyserve.py on 18801. Run it under the trace-lane lock:
#   /home/ghosty2004/samp-rs/tools/trace/with-lock.sh tools/e2e/m9-orig.sh
# Output: target/m9-orig/ (trace.jsonl, server.log, icy.log, run.out).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MAIN="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
T="$ROOT/tools/trace"
OUT="$ROOT/target/m9-orig"
mkdir -p "$OUT"
say() { printf '[m9-orig] %s\n' "$*"; }
[ "${SAMP_RS_TRACE_LOCK_HELD:-0}" = 1 ] || { say "run me under $MAIN/tools/trace/with-lock.sh"; exit 1; }
source "$ROOT/tools/e2e/m9scen.sh"
ICY_PORT=18801
export URL_BASE="http://127.0.0.1:$ICY_PORT"

export SAMP_RS_ROOT="$MAIN" E2E_TAG=m9orig E2E_PORT=7790 E2E_CTL_PORT=47701
source "$ROOT/tools/e2e/lib.sh"
e2e_server_prepare
( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$T/uifs.pwn" \
    -iqawno/include -ofilterscripts/traceui.amx -d0 > /dev/null ) || exit 1
python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/traceui"]
json.dump(d, open(p, "w"), indent=4)
PY
mkdir -p "$E2E_SERVER/scriptfiles"
m9_silence_mp3 "$ROOT/target/m9-silence.mp3" || exit 1
python3 -I "$T/icyserve.py" "$ICY_PORT" "$ROOT/target/m9-silence.mp3" "$OUT/icy.log" &
ICY=$!
( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
SRV=$!
cleanup() {
    [ -n "${SESSION:-}" ] && TRACE_SESSION="$SESSION" "$T/ui.sh" stop > /dev/null 2>&1
    wait "${RUN:-}" 2>/dev/null
    kill "$SRV" "$ICY" 2>/dev/null; wait "$SRV" "$ICY" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null
    local d="$ROOT/target/trace/${SESSION%%.*}"
    [ -n "${SESSION:-}" ] && cp -f "$d/trace.jsonl" "$OUT/trace.jsonl" 2>/dev/null
    say "output $OUT (run ${SESSION%%.*})"
}
trap cleanup EXIT
e2e_wait_log '\[ui-fs\] loaded' 20 || { say "server did not start"; exit 1; }

SAMP_TRACER_BASS=1 TRACE_NO_SERVER=1 "$T/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
RUN=$!
for _ in $(seq 1 120); do
    SESSION="$(grep -oE 'session: [^ ]+' "$OUT/run.out" | head -n 1 | cut -d' ' -f2)"
    [ -n "$SESSION" ] && break
    sleep 1
done
[ -n "$SESSION" ] || { say "no session id"; cat "$OUT/run.out"; exit 1; }
export TRACE_SESSION="$SESSION"
ui() { "$T/ui.sh" "$@"; }
SF="$E2E_SERVER/scriptfiles/uicmd.txt"
SEQ=0
fs() {
    SEQ=$((SEQ + 1)); printf '%d %s\n' "$SEQ" "$1" >> "$SF"
    e2e_wait_log "\[ui-fs\] $SEQ done" 10 || say "command $SEQ ($1) not run"
}
key() { ui key "$@" > /dev/null; }
mark() { ui mark "$*" > /dev/null; say "mark $*"; }
e2e_wait_log "\[e2e\] class id=[0-9]+ " 120 || { say "original did not reach class selection"; exit 1; }
PID_="$(grep -E '\[e2e\] connect id=[0-9]+ name=TraceOrig' "$E2E_LOG" | tail -n 1 | sed -E 's/.*id=([0-9]+).*/\1/')"
sleep 3
for _ in 1 2 3 4 5 6; do ui key shift; e2e_wait_log "\[e2e\] spawn id=$PID_\b" 4 && break; done
mark spawned
m9_scenario
say "scenario done"
