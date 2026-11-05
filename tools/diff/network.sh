#!/usr/bin/env bash
# F2 / NET-6: network diff of the ORIGINAL samp.dll and OUR client for scripted scenarios.
#
#   tools/diff/network.sh [--orig-only|--ours-only|--report-only] [--orig DIR] [--ours DIR]
#
# Each client plays the same scripted session (tools/diff/netscen.sh): join, class selection,
# spawn, walk, idle, chat + command, dialog response, scoreboard, pickup, checkpoint, object
# edit cancel, drive (enter/exit), passenger, shoot (with samp-bot "NetBot" as the other
# player), loss window (the proxy drops client reliable datagrams), death, /q.
# The server (open.mp copy with e2eprobe + tools/trace/uifs.pwn) binds 127.0.0.2; the client
# connects to 127.0.0.1 through tools/diff/lossproxy.py (same port, so the cipher key holds);
# dumpcap records the client leg only. tools/diff/net_report.py decodes every datagram
# (tools/diff/netcap.py) and diffs every C→S message per scenario window (format, reliability,
# channel, count, cadence, order, trigger latency) plus the RakNet-level behaviour (acks,
# packing, resends, pings, keep-alives) with explicit tolerances; residuals listed in
# tools/diff/net-decisions.tsv (each naming a ROADMAP decision) are accepted.
# Exit 0 = every check matches or is an accepted decision.
# NET_SCEN=FILE plays another scenario script (same helpers and env as netscen.sh) instead, for
# focused measurements; the report then covers that script's windows.
#
# Lanes: original = trace lane (tools/trace/run.sh orig holdjoin with TRACE_NO_SERVER=1, under
# tools/trace/with-lock.sh; UDP 7790); ours = lane "ui" (prefix-ui, gta-sa-ui, UDP 7800,
# control 47710; under target/locks/ui.lock unless SAMP_RS_UI_LOCK_HELD=1).
# Output: target/netdiff/{orig,ours}-<stamp>/ (net.pcapng, marks.tsv, logs), target/netdiff/
# report.md, and target/netdiff/{orig,ours}-latest.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
NETOUT="$ROOT/target/netdiff"
mkdir -p "$NETOUT"
say() { printf '[net] %s\n' "$*"; }

MODE=both; ORIG_DIR=""; OURS_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --orig-only) MODE=orig; shift ;;
        --ours-only) MODE=ours; shift ;;
        --report-only) MODE=report; shift ;;
        --orig) ORIG_DIR="$2"; shift 2 ;;
        --ours) OURS_DIR="$2"; shift 2 ;;
        --inner-orig|--inner-ours) INNER="${1#--inner-}"; shift; break ;;
        *) sed -n '2,24p' "$0"; exit 2 ;;
    esac
done

# ---- server, proxy and capture (both lanes) ------------------------------------------
# net_server_start DIR PORT — server copy in DIR bound to 127.0.0.2:PORT with e2eprobe + uifs.
net_server_start() {
    local dir="$1" port="$2"
    mkdir -p "$dir"
    rsync -a --delete --exclude 'log.txt' --exclude 'e2e-server.log' "$SAMP_RS_ROOT/server/" "$dir/"
    ( cd "$dir" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/e2eprobe.pwn" \
        -iqawno/include -ofilterscripts/e2eprobe.amx -d0 > /dev/null &&
      LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/uifs.pwn" \
        -iqawno/include -ofilterscripts/traceui.amx -d0 > /dev/null ) || return 1
    python3 -I - "$dir/config.json" "$port" <<'PY'
import json, sys
p, port = sys.argv[1], int(sys.argv[2])
d = json.load(open(p))
d["announce"] = False
d["network"]["port"] = port
d["network"]["bind"] = "127.0.0.2"
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/traceui"]
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$dir/scriptfiles"; : > "$dir/scriptfiles/uicmd.txt"
    ( cd "$dir" && exec ./omp-server > "$dir/e2e-server.log" 2>&1 ) &
    NET_SERVER_PID=$!
    local i
    for ((i = 0; i < 40; i++)); do grep -q '\[ui-fs\] loaded' "$dir/e2e-server.log" 2>/dev/null && return 0; sleep 0.5; done
    say "server in $dir did not start"; return 1
}

net_stop_all() {
    [ -n "${NET_BOT_PID:-}" ] && kill "$NET_BOT_PID" 2>/dev/null
    [ -n "${NET_CAP_PID:-}" ] && { kill "$NET_CAP_PID" 2>/dev/null; wait "$NET_CAP_PID" 2>/dev/null; }
    [ -n "${NET_PROXY_PID:-}" ] && kill "$NET_PROXY_PID" 2>/dev/null
    [ -n "${NET_SERVER_PID:-}" ] && { kill "$NET_SERVER_PID" 2>/dev/null; wait "$NET_SERVER_PID" 2>/dev/null; }
    NET_BOT_PID=; NET_CAP_PID=; NET_PROXY_PID=; NET_SERVER_PID=
}

# net_infra_start OUT SRVDIR PORT — server, proxy and capture for one session.
net_infra_start() {
    local out="$1" srv="$2" port="$3"
    net_server_start "$srv" "$port" || return 1
    echo off > "$out/loss.ctl"
    python3 -I "$HERE/lossproxy.py" 127.0.0.1 "$port" 127.0.0.2 "$out/loss.ctl" "$out/loss.jsonl" \
        > "$out/proxy.out" 2>&1 &
    NET_PROXY_PID=$!
    dumpcap -q -i lo -f "udp port $port and not host 127.0.0.2" -w "$out/net.pcapng" > "$out/dumpcap.out" 2>&1 &
    NET_CAP_PID=$!
    sleep 1
}

# ---- inner part: the original in the trace lane ----------------------------------------
if [ "${INNER:-}" = orig ]; then
    OUT="$1"
    source "$ROOT/tools/trace/lib.sh"   # trace lane env: prefix-trace, gta-sa-trace, 7790
    SRV="$NETOUT/server-trace"
    trap net_stop_all EXIT
    net_infra_start "$OUT" "$SRV" "$E2E_PORT" || exit 1
    say "original: server 127.0.0.2:$E2E_PORT, proxy, capture up; starting tools/trace/run.sh orig holdjoin"
    date +%s.%N > "$OUT/launch.t"
    TRACE_NO_SERVER=1 TRACE_HOLD_SECS=900 "$ROOT/tools/trace/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    tout=""
    for ((i = 0; i < 240; i++)); do
        tout="$(sed -nE 's/.*\[trace\] output: ([^ ]+).*/\1/p' "$OUT/run.out" | head -n 1)"
        sess="$(sed -nE 's/.*session: ([^ ]+) .*/\1/p' "$OUT/run.out" | head -n 1)"
        [ -n "$sess" ] && break
        kill -0 "$RUN_PID" 2>/dev/null || break
        sleep 0.5
    done
    [ -n "${sess:-}" ] || { say "no hold session: $(tail -n 5 "$OUT/run.out")"; exit 1; }
    TOUT="$ROOT/target/trace/${sess%%.*}"
    for ((i = 0; i < 120; i++)); do [ -f "$TOUT/ready" ] && break; sleep 0.5; done
    E2E_HEADLESS_DISPLAY="$(sed -n 's/^E2E_HEADLESS_DISPLAY=//p' "$TOUT/hold.env")"
    export E2E_HEADLESS_DISPLAY
    rc=0
    NET_CLIENT=orig NET_NAME=TraceOrig NET_SRV="$SRV" NET_OUT="$OUT" NET_PORT="$E2E_PORT" NET_CTL_PORT="$E2E_CTL_PORT" \
        bash "${NET_SCEN:-$HERE/netscen.sh}" || rc=$?
    touch "$TOUT/stop"
    wait "$RUN_PID" 2>/dev/null
    cp -f "$SRV/e2e-server.log" "$OUT/server.log" 2>/dev/null
    cp -f "$TOUT/launch.out" "$OUT/launch.out" 2>/dev/null
    echo "$TOUT" > "$OUT/trace-dir.txt"
    exit "$rc"
fi

# ---- inner part: our client in lane ui (runs inside e2e_headless_run) -------------------
if [ "${INNER:-}" = ours ]; then
    OUT="$1"
    source "$ROOT/tools/wine-env.sh"
    source "$ROOT/tools/e2e/lib.sh"
    SRV="$NETOUT/server-ui"
    trap 'net_stop_all; ( wineserver -k 2>/dev/null ) || true' EXIT
    net_infra_start "$OUT" "$SRV" "$E2E_PORT" || exit 1
    rm -f "$GTA_DIR/samp-rs.log"
    date +%s.%N > "$OUT/launch.t"
    SKIP_BUILD=1 timeout --kill-after=10 900 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" TraceOurs \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    rc=0
    NET_CLIENT=ours NET_NAME=TraceOurs NET_SRV="$SRV" NET_OUT="$OUT" NET_PORT="$E2E_PORT" NET_CTL_PORT="$E2E_CTL_PORT" \
        bash "${NET_SCEN:-$HERE/netscen.sh}" || rc=$?
    for ((i = 0; i < 20; i++)); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 0.5; done
    kill "$RUN_PID" 2>/dev/null; wait "$RUN_PID" 2>/dev/null
    cp -f "$GTA_DIR/samp-rs.log" "$OUT/samp-rs.log" 2>/dev/null
    cp -f "$SRV/e2e-server.log" "$OUT/server.log" 2>/dev/null
    exit "$rc"
fi

# ---- outer part ------------------------------------------------------------------------
stamp="$(date +%Y%m%d-%H%M%S)"
build() {
    say "building (i686 release: client, launcher, tracer; host: samp-bot)"
    { (cd "$ROOT" && cargo build -q --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher -p samp-tracer) &&
      (cd "$ROOT" && cargo build -q -p samp-bot); } > "$NETOUT/build.out" 2>&1 \
        || { tail -n 30 "$NETOUT/build.out"; say "build failed"; return 1; }
}

run_orig() {
    ORIG_DIR="$NETOUT/orig-$stamp"; mkdir -p "$ORIG_DIR"
    say "original (trace lane) → $ORIG_DIR"
    SAMP_RS_ROOT="$SAMP_RS_ROOT" "$ROOT/tools/trace/with-lock.sh" bash "$HERE/network.sh" --inner-orig "$ORIG_DIR"
    local r=$?
    ln -sfn "$(basename "$ORIG_DIR")" "$NETOUT/orig-latest"
    return "$r"
}

run_ours() {
    OURS_DIR="$NETOUT/ours-$stamp"; mkdir -p "$OURS_DIR"
    say "ours (lane ui) → $OURS_DIR"
    local r=0
    (
        export WINEPREFIX="$SAMP_RS_ROOT/wine/prefix-ui" GTA_DIR="$SAMP_RS_ROOT/wine/gta-sa-ui"
        export E2E_TAG=ui E2E_PORT=7800 E2E_CTL_PORT=47710 SAMP_RS_CTL_PORT=47710
        source "$ROOT/tools/wine-env.sh"
        source "$ROOT/tools/e2e/lib.sh"
        if [ "${SAMP_RS_UI_LOCK_HELD:-0}" = 1 ]; then
            e2e_headless_run bash "$HERE/network.sh" --inner-ours "$OURS_DIR"
        else
            mkdir -p "$SAMP_RS_ROOT/target/locks"
            SAMP_RS_UI_LOCK_HELD=1 flock "$SAMP_RS_ROOT/target/locks/ui.lock" \
                bash -c 'source "$1/tools/wine-env.sh"; source "$1/tools/e2e/lib.sh"; e2e_headless_run bash "$2" --inner-ours "$3"' \
                _ "$ROOT" "$HERE/network.sh" "$OURS_DIR"
        fi
    ) || r=$?
    ln -sfn "$(basename "$OURS_DIR")" "$NETOUT/ours-latest"
    return "$r"
}

case "$MODE" in
    both) build || exit 1; run_orig || say "original session failed"; run_ours || say "our session failed" ;;
    orig) build || exit 1; run_orig || say "original session failed" ;;
    ours) build || exit 1; run_ours || say "our session failed" ;;
    report) ;;
esac
ORIG_DIR="${ORIG_DIR:-$NETOUT/orig-latest}"
OURS_DIR="${OURS_DIR:-$NETOUT/ours-latest}"
[ "$MODE" = orig ] && exit 0
[ "$MODE" = ours ] && [ ! -e "$ORIG_DIR" ] && exit 0
python3 -I "$HERE/net_report.py" "$ORIG_DIR" "$OURS_DIR" "$HERE/net-decisions.tsv" "$ROOT/docs/ROADMAP.md" \
    --out "$NETOUT/report.md"
