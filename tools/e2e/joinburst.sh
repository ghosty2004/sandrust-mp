#!/usr/bin/env bash
# Join-burst regression test (HANG-1): tools/e2e/probe/joinburst.pwn sends a large public server's
# OnPlayerConnect burst (hundreds of building removals, textdraws, client messages, map icons, player objects
# and a split-packet dialog) before InitGame / class selection. The client must get through the join: Enter
# answers the dialog ("[joinburst] dialog response"; the original requests a class only after it closes), the
# server sees the class request ("[e2e] class id=N") and the control channel answers. Before the
# fix our client's main thread spun forever after "packet Accepted" (occluder list links zeroed by RPC 43).
#   tools/e2e/joinburst.sh [ours|orig] [RUNS]       (default: ours, 1 run)
# JB_LOSS=mix-sc:8 puts tools/diff/lossproxy.py between the client and the server (server on 127.0.0.2)
# with that rule (see lossproxy.py: drop-sc-reliable N, mix-sc N = drop + reorder + duplicate).
# JB_DEFINES="JB_REMOVE=0 …" passes pawncc defines to joinburst.pwn.
# JOIN-P (measured on the original, docs/re/subsystems/player-state.md "128 PlayerRequestClass"): no RPC 128 while
# the join dialog is open; the first one follows the dialog's RPC 62 in the next local tick (original: 4.2 ms
# later on the wire, 5 ms at the server). Checked twice: the server (joinburst.pwn logs every class request with
# the dialog's state and the ms since its response) and a capture of the client leg (dumpcap; tools/diff/netcap.py decodes it): the first C→S RPC 128 comes after the first RPC 62,
# at most JB_CLASS_MAX_MS (default 100) later. The first Enter is pressed JB_DIALOG_WAIT s (default 20) after the
# burst, so class selection is already up and the request was held back by the dialog alone.
# Lane: trace (wine/prefix-trace, wine/gta-sa-trace, UDP 7790, control 47701); the trace-lane lock is taken
# unless SAMP_RS_TRACE_LOCK_HELD=1. JB_LANE=ui: lane ui instead (wine/prefix-ui, wine/gta-sa-ui, UDP 7800,
# control 47710, tracer 47711; target/locks/ui.lock, taken unless SAMP_RS_UI_LOCK_HELD=1).
# Exit status 0 = every run passed.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
source "$ROOT/tools/trace/lib.sh"
if [[ "${JB_LANE:-trace}" == ui ]]; then
    export WINEPREFIX="$SAMP_RS_ROOT/wine/prefix-ui" GTA_DIR="$SAMP_RS_ROOT/wine/gta-sa-ui"
    export E2E_TAG=ui E2E_PORT=7800 E2E_CTL_PORT=47710 SAMP_TRACER_PORT=47710
    E2E_SERVER="$E2E_ROOT/target/e2e-server-ui"; E2E_LOG="$E2E_SERVER/e2e-server.log"
    E2E_HEADLESS_ENV="$E2E_ROOT/target/e2e-headless-ui.env"
    source "$ROOT/tools/wine-env.sh"
    trace_ports() {
        if [ "$1" = ours ]; then
            export SAMP_RS_CTL_PORT="$E2E_CTL_PORT" SAMP_TRACER_PORT=47711
            [ -f "$SAMP_RS_ROOT/data/auth_table.txt" ] \
                && SAMP_RS_AUTH_TABLE="$(winepath -w "$SAMP_RS_ROOT/data/auth_table.txt" 2>/dev/null)" \
                && export SAMP_RS_AUTH_TABLE
        else
            export SAMP_TRACER_PORT="$E2E_CTL_PORT"
        fi
    }
    trace_wine_kill() { WINEPREFIX="$SAMP_RS_ROOT/wine/prefix-ui" wineserver -k 2>/dev/null || true; }
    trace_lock() {
        [ "${SAMP_RS_UI_LOCK_HELD:-0}" = 1 ] && return 0
        mkdir -p "$SAMP_RS_ROOT/target/locks"
        exec 9> "$SAMP_RS_ROOT/target/locks/ui.lock"
        flock -n 9 || { say "waiting for the ui-lane lock"; flock 9; }
        export SAMP_RS_UI_LOCK_HELD=1
    }
fi
say() { printf '[joinburst] %s\n' "$*"; }
JB_CLASS_MAX_MS="${JB_CLASS_MAX_MS:-100}"

# jb_capture_check PCAP — the first C→S RPC 128 after the first RPC 62, at most JB_CLASS_MAX_MS later.
jb_capture_check() {
    python3 -I - "$ROOT/tools/diff" "$1" "$E2E_PORT" "$JB_CLASS_MAX_MS" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import netcap
recs = netcap.read(sys.argv[2], int(sys.argv[3]))
limit = int(sys.argv[4])
first = {}
for di, r in enumerate(recs):
    if r["dir"] != "C>S":
        continue
    for mi, m in enumerate(r["msgs"]):
        if m["kind"] in ("rpc 62", "rpc 128") and m["kind"] not in first:
            first[m["kind"]] = (di, mi, r["t"])
if "rpc 128" not in first or "rpc 62" not in first:
    print("capture: missing %s" % ", ".join(k for k in ("rpc 62", "rpc 128") if k not in first))
    sys.exit(1)
d62, d128 = first["rpc 62"], first["rpc 128"]
dt = (d128[2] - d62[2]) * 1000.0
where = "same datagram" if d62[0] == d128[0] else "%d datagram(s) later" % (d128[0] - d62[0])
print("capture: first RPC 128 %+.1f ms after the first RPC 62 (%s)" % (dt, where))
if d128[:2] < d62[:2]:
    print("capture: RPC 128 sent before the dialog's RPC 62")
    sys.exit(1)
sys.exit(0 if dt <= limit else 1)
PY
}
CLIENT_LOG="$GTA_DIR/samp-rs.log"
CRASH_FILE="$GTA_DIR/samp-rs-crash.txt"

jb_server_start() {
    e2e_server_prepare
    local defs=()
    read -r -a defs <<< "${JB_DEFINES:-}"
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/joinburst.pwn" "${defs[@]}" \
        -iqawno/include -ofilterscripts/joinburst.amx -d0 > /dev/null ) || { say "joinburst.pwn does not compile"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" "${JB_LOSS:+127.0.0.2}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["announce"] = False
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/joinburst"]
if len(sys.argv) > 2 and sys.argv[2]:
    d["network"]["bind"] = sys.argv[2]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 9>&- ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[joinburst\] loaded' 5 || { say "joinburst filterscript not loaded"; return 1; }
    if [[ -n "${JB_LOSS:-}" ]]; then
        echo "${JB_LOSS/:/ }" > "$OUT/loss.ctl"
        python3 -I "$ROOT/tools/diff/lossproxy.py" 127.0.0.1 "$E2E_PORT" 127.0.0.2 "$OUT/loss.ctl" "$OUT/loss.jsonl" \
            > "$OUT/proxy.out" 2>&1 9>&- &
        PROXY_PID=$!
        sleep 0.5
    fi
}

# ---- inner part: one run inside the headless display ---------------------------------------------------------
if [[ "${1:-}" == --inner ]]; then
    CLIENT="$2"; NAME="$3"; RUN_OUT="$4"
    trace_ports "$CLIENT"
    fail() { say "FAIL: $*"; e2e_shot "$RUN_OUT/fail.png" > /dev/null 2>&1; exit 1; }
    trace_launch "$CLIENT" "$NAME" > "$RUN_OUT/launch.out" 2>&1 &
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    id="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[joinburst\] sent [0-9]+ calls to $id\b" 10 || fail "burst not sent"
    say "burst: $(grep -E "\[joinburst\] sent [0-9]+ calls to $id\b" "$E2E_LOG" | tail -n 1 | sed 's/.*\] //')"
    # The burst's dialog must answer (Enter = first button): the original sends its class request only once
    # the dialog is closed, and a hung main thread answers nothing. JOIN-P: no class request while it is open.
    sleep "${JB_DIALOG_WAIT:-20}"
    grep -qE "\[joinburst\] class request id=$id " "$E2E_LOG" && fail "class requested while the join dialog is open"
    for _ in 1 2 3 4 5 6; do
        grep -qE "\[joinburst\] dialog response id=$id " "$E2E_LOG" && break
        e2e_key Return; sleep 5
    done
    e2e_wait_log "\[joinburst\] dialog response id=$id dialog=77 response=1" 2 || fail "no dialog response (join hang)"
    say "dialog answered"
    e2e_wait_log "\[e2e\] class id=$id " 30 || fail "no class request (join hang)"
    say "class selection reached"
    cls="$(grep -E "\[joinburst\] class request id=$id " "$E2E_LOG" | head -n 1)"
    say "first ${cls#*\] }"
    [[ "$cls" == *" dialog_open=0 "* ]] || fail "class requested while the join dialog is open"
    ms="${cls##*after_response_ms=}"
    (( ms >= 0 && ms <= JB_CLASS_MAX_MS )) || fail "class request ${ms} ms after the dialog response (max $JB_CLASS_MAX_MS)"
    [[ -e "$CRASH_FILE" ]] && fail "crash report written"
    # The game keeps running: the control channel answers and (ours) frames keep ticking.
    sleep 5
    if [[ "$CLIENT" == ours ]]; then
        [[ -n "$(e2e_ctl net)" ]] || fail "control channel does not answer"
        acc="$(grep -n 'client: packet Accepted' "$CLIENT_LOG" | tail -n 1 | cut -d: -f1)"
        ticks="$(tail -n +"${acc:-1}" "$CLIENT_LOG" | grep -c '\] tick [0-9]* (')"
        (( ticks >= 3 )) || fail "only $ticks tick lines after the join"
        say "ticks after the join: $ticks; $(e2e_ctl net)"
        e2e_ctl quit > /dev/null
    else
        [[ "$(trace_ctl ping)" == pong ]] || fail "tracer control channel does not answer"
    fi
    exit 0
fi

# ---- outer part ------------------------------------------------------------------------------------------------
CLIENT="${1:-ours}"; RUNS="${2:-1}"
case "$CLIENT" in
    ours) NAME=BurstOurs ;;
    orig) NAME=BurstOrig ;;
    *) echo "usage: $0 [ours|orig] [RUNS]" >&2; exit 2 ;;
esac
trace_lock
trace_ports "$CLIENT"
OUT="$ROOT/target/joinburst/$CLIENT-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
finish() {
    [[ -n "${PROXY_PID:-}" ]] && kill "$PROXY_PID" 2>/dev/null
    [[ -n "${CAP_PID:-}" ]] && kill -INT "$CAP_PID" 2>/dev/null
    e2e_headless_stop
    e2e_server_stop
    trace_wine_kill
}
trap finish EXIT
say "building ($CLIENT)"
trace_build "$CLIENT" > "$OUT/build.out" 2>&1 || { cat "$OUT/build.out"; exit 1; }
trace_install "$CLIENT"
for p in "$E2E_CTL_PORT" "$SAMP_TRACER_PORT"; do
    if [[ "$(E2E_CTL_PORT=$p e2e_ctl ping)" == pong ]]; then say "port $p already answers; refusing"; exit 1; fi
done
pass=0
for ((run = 1; run <= RUNS; run++)); do
    RUN_OUT="$OUT/run$run"; mkdir -p "$RUN_OUT"
    trace_wine_kill
    rm -f "$CLIENT_LOG" "$CRASH_FILE" "$GTA_DIR/samp-rs-launcher.log"
    PROXY_PID=
    jb_server_start || exit 1
    say "run $run/$RUNS: $CLIENT, server UDP $E2E_PORT${JB_LOSS:+, loss proxy $JB_LOSS}"
    dumpcap -q -i lo -f "udp port $E2E_PORT and not host 127.0.0.2" -w "$RUN_OUT/net.pcapng" \
        > "$RUN_OUT/dumpcap.out" 2>&1 9>&- &
    CAP_PID=$!
    sleep 1
    ok=0
    e2e_headless_run bash "$0" --inner "$CLIENT" "$NAME" "$RUN_OUT" && ok=1
    sleep 1; kill -INT "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null
    if (( ok )); then
        jb_capture_check "$RUN_OUT/net.pcapng" | sed 's/^/[joinburst] /' | tee "$RUN_OUT/capture.txt"
        (( PIPESTATUS[0] == 0 )) || ok=0
    fi
    if (( ok )); then
        pass=$((pass + 1)); say "run $run: PASS"
    else
        say "run $run: FAIL"
        tail -n 15 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
    fi
    cp -f "$CLIENT_LOG" "$RUN_OUT/samp-rs.log" 2>/dev/null
    cp -f "$CRASH_FILE" "$RUN_OUT/" 2>/dev/null
    cp -f "$E2E_LOG" "$RUN_OUT/server.log" 2>/dev/null
    [[ -n "${PROXY_PID:-}" ]] && { kill "$PROXY_PID" 2>/dev/null; cp -f "$OUT/loss.jsonl" "$RUN_OUT/" 2>/dev/null; }
    e2e_headless_stop
    e2e_server_stop
    trace_wine_kill
done
say "$pass/$RUNS runs passed; logs: $OUT"
(( pass == RUNS ))
