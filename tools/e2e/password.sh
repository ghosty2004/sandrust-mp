#!/usr/bin/env bash
# Server password parity: the ORIGINAL samp.dll and OUR client against a passworded open.mp server copy.
#   tools/e2e/password.sh [--orig-only|--ours-only|--report-only] [CASE...]
# Cases (one game boot each; default all, original first):
#   right   -z with the server's password      → joins
#   wrong   -z with another password           → "Wrong server password.", no retry
#   missing no -z                              → same as the original
# Trace lane (wine/prefix-trace, wine/gta-sa-trace, UDP 7790, control 47701), held through
# target/locks/trace.lock for the whole run; the game runs on the isolated headless display. Each run
# records the client leg (dumpcap, decoded with samp-dump), the client's chatlog.txt and the server log.
# tools/e2e/password_report.py compares, per case: the `0x0b` connection request bytes, the reaction
# (`0x25` / join), open-connection requests after the refusal and the chat lines.
# Output: target/e2e-password/<client>-<case>/ and target/e2e-password/report.md. Exit 0 = all equal.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
OUTROOT="$ROOT/target/e2e-password"
SERVER_PASSWORD="${PW_SERVER_PASSWORD:-Pass123}"
WRONG_PASSWORD="${PW_WRONG_PASSWORD:-Wrong456}"
# Seconds to watch for a retry after the refusal (the client's connect tick would retry after 3 s).
WATCH_SECS="${PW_WATCH_SECS:-20}"
say() { printf '[password] %s\n' "$*"; }

client_password() {
    case "$1" in
        right) echo "$SERVER_PASSWORD" ;;
        wrong) echo "$WRONG_PASSWORD" ;;
        *) echo "" ;;
    esac
}

# ---- inner part: one game on the headless display ---------------------------------------
if [ "${1:-}" = --inner ]; then
    CLIENT="$2"; CASE="$3"; OUT="$4"
    source "$ROOT/tools/trace/lib.sh"
    trace_ports "$CLIENT"
    NAME=PwOrig; [ "$CLIENT" = ours ] && NAME=PwOurs
    TRACE_PASSWORD="$(client_password "$CASE")"
    export TRACE_PASSWORD
    CHATLOG="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP/chatlog.txt"
    ( trace_launch "$CLIENT" "$NAME" > "$OUT/launch.out" 2>&1 ) &
    LPID=$!
    rc=0
    if ! trace_wait_ctl 90; then
        say "$CLIENT/$CASE: tracer control channel did not answer"; rc=1
    elif [ "$CASE" = right ]; then
        if e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120; then
            say "$CLIENT/$CASE: joined"; sleep 5
        else
            say "$CLIENT/$CASE: no join"; rc=1
        fi
    else
        seen=0
        for ((i = 0; i < 240; i++)); do
            grep -q "Wrong server password" "$CHATLOG" 2>/dev/null && { seen=1; break; }
            sleep 0.5
        done
        if [ "$seen" = 1 ]; then
            say "$CLIENT/$CASE: refused; watching ${WATCH_SECS} s for a retry"; sleep "$WATCH_SECS"
        else
            say "$CLIENT/$CASE: no \"Wrong server password.\" line"; rc=1
        fi
    fi
    e2e_ctl quit > /dev/null
    trace_ctl quit > /dev/null
    for ((i = 0; i < 20; i++)); do kill -0 "$LPID" 2>/dev/null || break; sleep 1; done
    kill -0 "$LPID" 2>/dev/null && trace_wine_kill
    wait "$LPID" 2>/dev/null
    cp -f "$CHATLOG" "$OUT/chatlog.txt" 2>/dev/null
    for f in samp-rs.log samp-tracer.log samp-rs-launcher.log; do cp -f "$GTA_DIR/$f" "$OUT/" 2>/dev/null; done
    exit "$rc"
fi

# ---- outer part ---------------------------------------------------------------------------
MODE=both
while [ $# -gt 0 ]; do
    case "$1" in
        --orig-only) MODE=orig; shift ;;
        --ours-only) MODE=ours; shift ;;
        --report-only) MODE=report; shift ;;
        -*) sed -n '2,14p' "$0"; exit 2 ;;
        *) break ;;
    esac
done
CASES=("$@"); [ ${#CASES[@]} -eq 0 ] && CASES=(right wrong missing)
mkdir -p "$OUTROOT"

report() {
    python3 -I "$HERE/password_report.py" "$OUTROOT" "${CASES[@]}" | tee "$OUTROOT/report.md"
    return "${PIPESTATUS[0]}"
}
[ "$MODE" = report ] && { report; exit $?; }

source "$ROOT/tools/trace/lib.sh"
trace_lock
CLIENTS=(orig ours); [ "$MODE" = orig ] && CLIENTS=(orig); [ "$MODE" = ours ] && CLIENTS=(ours)
build_client=orig; [[ " ${CLIENTS[*]} " == *" ours "* ]] && build_client=ours
say "building (tracer, launcher${build_client/orig/}; samp-dump)"
{ trace_build "$build_client" && (cd "$ROOT" && cargo build -q -p samp-dump); } > "$OUTROOT/build.out" 2>&1 \
    || { tail -n 30 "$OUTROOT/build.out"; exit 1; }
trace_install "$build_client"
DUMP="${CARGO_TARGET_DIR:-$ROOT/target}/debug/samp-dump"

CAP_PID=
stop_all() {
    [ -n "$CAP_PID" ] && { kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; }
    CAP_PID=
    e2e_server_stop
    trace_wine_kill
}
trap stop_all EXIT

overall=0
for client in "${CLIENTS[@]}"; do
    for case in "${CASES[@]}"; do
        out="$OUTROOT/$client-$case"
        rm -rf "$out"; mkdir -p "$out"
        avail_kb="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)"
        if [ "$avail_kb" -lt $((6 * 1024 * 1024)) ]; then
            say "only $((avail_kb / 1024)) MB available (< 6 GB); not starting a game"; exit 1
        fi
        trace_wine_kill
        e2e_server_prepare || { say "server prepare failed"; exit 1; }
        python3 -I - "$E2E_SERVER/config.json" "$SERVER_PASSWORD" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["password"] = sys.argv[2]
json.dump(d, open(p, "w"), indent=4)
PY
        ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 9>&- ) &
        E2E_SERVER_PID=$!
        e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; exit 1; }
        dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$out/net.pcapng" > "$out/dumpcap.out" 2>&1 9>&- &
        CAP_PID=$!
        sleep 1
        rm -f "$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP/chatlog.txt"
        say "$client/$case: client password '$(client_password "$case")', server password '$SERVER_PASSWORD'"
        rc=0
        e2e_headless_run bash "$0" --inner "$client" "$case" "$out" || rc=$?
        [ "$rc" = 0 ] || overall=1
        kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; CAP_PID=
        e2e_server_stop
        cp -f "$E2E_LOG" "$out/server.log" 2>/dev/null
        "$DUMP" "$out/net.pcapng" --port "$E2E_PORT" > "$out/net.log" 2>&1
        say "$client/$case: run exit $rc ($(grep -c . "$out/net.log") decoded lines)"
    done
done
report || overall=1
exit "$overall"
