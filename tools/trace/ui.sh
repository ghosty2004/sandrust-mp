#!/usr/bin/env bash
# TRACE-3/5: drives a session started with `tools/trace/run.sh orig|ours hold` (trace lane only).
#   ui.sh key KEY...            xdotool key presses on the headless display (e.g. t, F7, Escape, Return)
#   ui.sh down KEY | up KEY     hold / release
#   ui.sh type TEXT             type text (xdotool type)
#   ui.sh mouse ARGS…           xdotool mouse command (mousedown 3 | mouseup 1 | click 1 | mousemove_relative -- DX DY)
#   ui.sh fire [MS]             left button held MS ms (default 150)
#   ui.sh shot NAME             screenshot of the headless display → $OUT/NAME.png
#   ui.sh ctl CMD               tracer control line (state, peek, poke, mark, d3d …)
#   ui.sh mark TEXT             mark in trace.jsonl and d3d.jsonl
#   ui.sh fs 'CMD|ARG|…'        run a uifs.pwn command; waits for "[ui-fs] SEQ done"
#   ui.sh d3d MODE FRAMES NAME  log MODE (ui|all|text) for FRAMES frames → $OUT/d3d-NAME.jsonl
#   ui.sh d3d-start … / d3d-collect NAME   the same in two steps (actions in between)
#   ui.sh bot NAME ARGS…        start target/debug/samp-bot as NAME (PID kept in $OUT/bots.pids)
#   ui.sh log [N]               last N server log lines (default 20)
#   ui.sh wait REGEX SECONDS    wait for a server log line
#   ui.sh stop                  kill the bots (by PID) and end the session
# Every call needs TRACE_SESSION=<id from run.sh hold's output> and SAMP_RS_TRACE_LOCK_HELD=1.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/lib.sh"
# Hold sessions are private (TRACE-6): only the owner, who got the session id from run.sh's output and
# runs inside the lane lock, may drive one.
SESSION="${TRACE_SESSION:-}"
[ -n "$SESSION" ] || { echo "ui.sh: refused: set TRACE_SESSION=<id printed by run.sh hold>" >&2; exit 1; }
[ "${SAMP_RS_TRACE_LOCK_HELD:-0}" = 1 ] || { echo "ui.sh: refused: only the session owner (SAMP_RS_TRACE_LOCK_HELD=1) may drive it" >&2; exit 1; }
case "$SESSION" in */*|.*|"") echo "ui.sh: refused: bad session id" >&2; exit 1 ;; esac
ENVF="$TRACE_OUT/${SESSION%%.*}/hold.env"
[ -f "$ENVF" ] && grep -qxF "SESSION=$SESSION" "$ENVF" || { echo "ui.sh: refused: no running hold session $SESSION" >&2; exit 1; }
# shellcheck disable=SC1090
source "$ENVF"
export E2E_HEADLESS_DISPLAY E2E_GAMESCOPE SAMP_TRACER_PORT E2E_CTL_PORT
SF="$E2E_SERVER/scriptfiles"
mkdir -p "$SF"

cmd="${1:-}"; shift || true
case "$cmd" in
    key) e2e_key "$@" ;;
    down) e2e_keydown "$1" ;;
    up) e2e_keyup "$1" ;;
    type) DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "$1" ;;
    mouse) DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool "$@" ;;
    fire)
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mousedown 1
        sleep "$(awk "BEGIN{print ${1:-150}/1000}")"
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mouseup 1
        ;;
    shot) e2e_shot "$OUT/$1.png" && echo "$OUT/$1.png" ;;
    ctl) trace_ctl "$*" ;;
    mark) trace_ctl "mark $*" ;;
    fs)
        seq=$(( $(wc -l < "$SF/uicmd.txt" 2>/dev/null || echo 0) + 1 ))
        printf '%d %s\n' "$seq" "$1" >> "$SF/uicmd.txt"
        e2e_wait_log "\[ui-fs\] $seq done" 10 || { echo "ui.sh: command $seq not run" >&2; exit 1; }
        grep -E "\[ui-fs\] $seq " "$E2E_LOG" | grep -v " done " || true
        ;;
    d3d|d3d-start)
        mode="$1"; frames="$2"; name="$3"
        f="$GTA_DIR/d3d.jsonl"
        trace_ctl "mark d3d-begin $name" > /dev/null
        wc -l < "$f" > "$OUT/.d3d-$name.start"
        r=$(trace_ctl "d3d $mode $frames")
        case "$r" in ok*) ;; *) echo "$r"; exit 1 ;; esac
        [ "$cmd" = d3d-start ] && exit 0
        "$0" d3d-collect "$name"
        ;;
    d3d-collect)
        name="$1"
        f="$GTA_DIR/d3d.jsonl"
        for ((i = 0; i < 1200; i++)); do
            [ "$(trace_ctl 'd3d busy')" = idle ] && break
            sleep 0.1
        done
        sleep 0.4
        trace_ctl "mark d3d-end $name" > /dev/null
        sleep 0.4
        start=$(cat "$OUT/.d3d-$name.start")
        tail -n +"$((start + 1))" "$f" > "$OUT/d3d-$name.jsonl"
        echo "$OUT/d3d-$name.jsonl $(wc -l < "$OUT/d3d-$name.jsonl") records"
        ;;
    bot)
        name="$1"; shift
        ( cd "$TRACE_ROOT" && exec target/debug/samp-bot "127.0.0.1:$E2E_PORT" --name "$name" "$@" ) \
            > "$OUT/bot-$name.out" 2>&1 &
        echo "$!" >> "$OUT/bots.pids"
        echo "bot $name pid $!"
        ;;
    log) tail -n "${1:-20}" "$E2E_LOG" ;;
    wait) e2e_wait_log "$1" "$2" ;;
    stop)
        if [ -f "$OUT/bots.pids" ]; then
            while read -r p; do kill "$p" 2>/dev/null; done < "$OUT/bots.pids"
        fi
        touch "$OUT/stop"
        ;;
    *) sed -n '2,16p' "$0"; exit 2 ;;
esac
