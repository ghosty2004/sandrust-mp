#!/usr/bin/env bash
# TRACE-1 scenario runner (tracing lane only: wine/prefix-trace, wine/gta-sa-trace,
# server UDP 7790, tracer control channel 47701, headless gamescope display).
#   tools/trace/run.sh orig [join|spawn|remote]   original samp.dll 0.3.7-R4 + samp-tracer.dll
#     remote (TRACE-2): after the spawn, target/debug/samp-bot joins as "RemoteBot", is moved
#     next to the traced player (tools/trace/remotefs.pwn), walks its circle for 40 s and leaves.
#     SAMP_TRACER_GUARD=<budget> adds the guard-page watch of CWorld::Players[2..209].
#   tools/trace/run.sh orig|ours idle      (TRACE-4, SP-OFF) after the spawn, tools/trace/spoff.pwn puts
#     the player in Grove Street for TRACE_IDLE_SECS (default 300) s, then at the Alhambra
#     entry-exit; the scenario walks it in (W) and idles 30 s inside. Population/wanted
#     state is in the `watch` records; tools/trace/spoff_report.py summarises.
#   tools/trace/run.sh orig|ours vehicles  (TRACE-4) tools/trace/vehfs.pwn: the player enters a Sultan (F),
#     drives (W) and exits (F); then target/debug/examples/drive_bot joins as "DriveBot", is put
#     into a second Sultan and drives a circle (200), is removed / put back, streams out and in
#     with the player, and its car is destroyed. Loopback traffic is captured (net.pcapng,
#     decoded to net.log with samp-dump to net-<client port>.log).
#   tools/trace/run.sh orig|ours hold|holdjoin  (TRACE-3/5) interactive: tools/trace/uifs.pwn on the server,
#     the scenario joins + spawns (holdjoin: only boots) and then waits; drive it with
#     tools/trace/ui.sh (keys, typing, screenshots, `d3d` logs, filterscript commands, bots).
#     The session is private: run.sh prints "session: ID"; ui.sh needs TRACE_SESSION=ID and
#     SAMP_RS_TRACE_LOCK_HELD=1 (owner only). TRACE_GAMEMODE=tracegm / TRACE_LAGCOMP=N (TRACE-6).
#     E2E_RES=WxH sets the headless display size; TRACE_NO_SERVER=1 starts no server (holdjoin). d3d.jsonl is copied to the output dir.
#   tools/trace/run.sh orig|ours objmap   (CRASH-1) tools/e2e/probe/objmap.pwn object-map stress; the game must
#     survive until "[objmap] done" (TRACE_OBJMAP_MINIMIZE=1: minimise/restore the window during the churn).
#   tools/trace/run.sh ours [join|spawn|remote]   our samp_client.dll + samp-tracer.dll
#   tools/trace/run.sh bare [SECONDS]      no client: stock single-player boot (baseline), default 40 s
# SAMP_TRACER_SCRIPTS=main,… (or *) adds per-command `stock_cmd` records for those stock scripts.
# Steps: boot -> join -> class selection [-> Shift -> spawn -> walk 3 s] -> quit.
# Output: target/trace/<client>-<stamp>/ (trace.jsonl, logs, screenshots, state.txt)
# and target/trace/<client>-latest -> that directory. Exit 0 = all steps reached.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/lib.sh"
# The trace lane is shared by several agents: one run at a time (blocking lock for the whole
# run, inherited by the scenario). tools/trace/with-lock.sh takes it for multi-command sessions.
trace_lock
CLIENT="${1:-orig}"; STEPS="${2:-spawn}"
case "$CLIENT" in
    orig) NAME=TraceOrig ;;
    ours) NAME=TraceOurs ;;
    bare) NAME=TraceBare ;;
    *) echo "usage: $0 orig|ours [join|spawn] | bare [SECONDS]" >&2; exit 2 ;;
esac
OUT="$TRACE_OUT/$CLIENT-$(date +%Y%m%d-%H%M%S)"
trace_ports "$CLIENT"
if [ "$CLIENT" = ours ] && ! grep -rq SAMP_RS_CTL_PORT "$TRACE_ROOT/crates/samp-client/src"; then
    # Otherwise our client would bind the main lane's control port 47700.
    trace_say "samp-client does not read SAMP_RS_CTL_PORT yet; refusing to run it in the trace lane"
    exit 1
fi
mkdir -p "$OUT"

trace_say "building ($CLIENT)"
trace_build "$CLIENT" > "$OUT/build.out" 2>&1 || { cat "$OUT/build.out"; exit 1; }
trace_install "$CLIENT"
for p in "$E2E_CTL_PORT" "$SAMP_TRACER_PORT"; do
    if [ "$(E2E_CTL_PORT=$p e2e_ctl ping)" = pong ]; then trace_say "port $p already answers; refusing"; exit 1; fi
done
trace_wine_kill
rm -f "$GTA_DIR/trace.jsonl" "$GTA_DIR/d3d.jsonl" "$GTA_DIR/samp-tracer.log"

finish() {
    if [ -n "${DUMPCAP_PID:-}" ]; then
        kill "$DUMPCAP_PID" 2>/dev/null; wait "$DUMPCAP_PID" 2>/dev/null
        # One decoded log per client (samp-dump assumes a single client per capture).
        for cp in $(tshark -r "$OUT/net.pcapng" -T fields -e udp.srcport -Y "udp.dstport==$E2E_PORT" 2>/dev/null | sort -u); do
            tshark -r "$OUT/net.pcapng" -Y "udp.port==$cp" -w "$OUT/net-$cp.pcapng" 2>/dev/null
            ( cd "$TRACE_ROOT" && cargo run -q -p samp-dump -- "$OUT/net-$cp.pcapng" --port "$E2E_PORT" > "$OUT/net-$cp.log" 2>&1 )
        done
    fi
    e2e_server_stop
    trace_wine_kill
    for f in trace.jsonl d3d.jsonl samp-tracer.log samp-rs-launcher.log samp-rs.log; do
        [ -f "$GTA_DIR/$f" ] && cp "$GTA_DIR/$f" "$OUT/"
    done
    cp "$E2E_LOG" "$OUT/server.log" 2>/dev/null
    ln -sfn "$(basename "$OUT")" "$TRACE_OUT/$CLIENT-latest"
    trace_say "output: $OUT ($(wc -l < "$OUT/trace.jsonl" 2>/dev/null || echo 0) records)"
}
trap 'finish' EXIT

if [ "$CLIENT" != bare ] && [ "$STEPS" = vehicles ]; then
    [ -x "$TRACE_ROOT/target/debug/examples/drive_bot" ] || { trace_say "target/debug/examples/drive_bot missing (cargo build -p samp-bot --example drive_bot)"; exit 1; }
    trace_server_start_fs vehfs.pwn traceveh || exit 1
    dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$OUT/net.pcapng" > /dev/null 2>&1 &
    DUMPCAP_PID=$!
    trace_say "server on UDP $E2E_PORT (+ traceveh filterscript, capture pid $DUMPCAP_PID)"
elif [ "$CLIENT" != bare ] && { [ "$STEPS" = hold ] || [ "$STEPS" = holdjoin ]; } && [ "${TRACE_NO_SERVER:-0}" = 1 ]; then
    trace_say "no server (TRACE_NO_SERVER=1)"
elif [ "$CLIENT" != bare ] && { [ "$STEPS" = hold ] || [ "$STEPS" = holdjoin ]; }; then
    trace_server_start_fs uifs.pwn traceui || exit 1
    dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$OUT/net.pcapng" > /dev/null 2>&1 &
    DUMPCAP_PID=$!
    trace_say "server on UDP $E2E_PORT (+ traceui filterscript, capture pid $DUMPCAP_PID)"
elif [ "$CLIENT" != bare ] && [ "$STEPS" = objmap ]; then
    # shellcheck disable=SC2086
    trace_server_start_fs ../e2e/probe/objmap.pwn objmap ${TRACE_OBJMAP_DEFINES:-} || exit 1
    trace_say "server on UDP $E2E_PORT (+ objmap filterscript)"
elif [ "$CLIENT" != bare ] && [ "$STEPS" = idle ]; then
    trace_server_start_fs spoff.pwn tracespoff "IDLE_MS=$(( ${TRACE_IDLE_SECS:-300} * 1000 ))" || exit 1
    trace_say "server on UDP $E2E_PORT (+ tracespoff filterscript, idle ${TRACE_IDLE_SECS:-300} s)"
elif [ "$CLIENT" != bare ] && [ "$STEPS" = remote ]; then
    [ -x "$TRACE_ROOT/target/debug/samp-bot" ] || { trace_say "target/debug/samp-bot missing"; exit 1; }
    trace_server_start_remote || exit 1
    trace_say "server on UDP $E2E_PORT (+ traceremote filterscript)"
elif [ "$CLIENT" != bare ]; then
    e2e_server_start || exit 1
    trace_say "server on UDP $E2E_PORT"
fi
e2e_headless_run "$HERE/scenario.sh" "$CLIENT" "$NAME" "$STEPS" "$OUT"
rc=$?
trace_say "scenario exit $rc"
exit "$rc"
