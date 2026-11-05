#!/usr/bin/env bash
# FEAT-A parity runs: the scenario of tools/e2e/featscen.sh traced on the ORIGINAL client and on ours.
#   tools/e2e/featparity.sh orig   original samp.dll in the trace lane (this worktree's tools/trace/run.sh orig hold,
#                                  SAMP_RS_ROOT's wine/, server/ and lane lock; takes the trace-lane lock)
#   tools/e2e/featparity.sh ours   our samp_client.dll + samp-tracer in lane veh (prefix-shot, gta-sa-shot, UDP 7820,
#                                  control 47720/47722) through the re-rooted tools of tools/e2e/m6p-hold.sh
#                                  (wrap it in `flock SAMP_RS_ROOT/target/locks/veh.lock`)
#   FEAT_LANE=veh tools/e2e/featparity.sh orig   the original in lane veh instead (exploration when the trace lane
#                                  is busy; the same re-rooted tools)
# Both use TRACE_GAMEMODE=tracegm. Prints the run directory (trace.jsonl, d3d-*.jsonl, server.log, net-*.log,
# screenshots). Compare with tools/trace/diff.py / d3d_diff.py and the net logs (feata-notes.md).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
CLIENT="${1:?usage: $0 orig|ours}"
say() { printf '[featparity] %s\n' "$*"; }

if [ "$CLIENT" = ours ] || [ "${FEAT_LANE:-trace}" = veh ]; then
    VR="$(SAMP_RS_ROOT="$DATA" "$ROOT/tools/e2e/m6p-hold.sh" prepare)" || exit 1
    unset SAMP_RS_ROOT
    TR="$VR/tools/trace"
    FEAT_TAG=veh
    LOCKCMD=(env)
else
    export SAMP_RS_ROOT="$DATA"
    TR="$ROOT/tools/trace"
    FEAT_TAG=trace
    LOCKCMD=("$TR/with-lock.sh")
fi

inner() {
    local log sess out pid i
    log="$(mktemp)"
    TRACE_GAMEMODE=tracegm "$TR/run.sh" "$CLIENT" hold > "$log" 2>&1 &
    pid=$!
    for ((i = 0; i < 900; i++)); do
        sess="$(sed -n 's/.*session: \([^ ]*\).*/\1/p' "$log" | head -n 1)"
        [ -n "$sess" ] && break
        kill -0 "$pid" 2>/dev/null || { cat "$log"; say "run.sh ended early"; return 1; }
        sleep 1
    done
    [ -n "$sess" ] || { cat "$log"; say "no session id"; return 1; }
    export TRACE_SESSION="$sess" SAMP_RS_TRACE_LOCK_HELD=1
    out="$(dirname "$(dirname "$TR")")/target/trace/${sess%%.*}"
    for ((i = 0; i < 200; i++)); do [ -e "$out/ready" ] && break; sleep 1; done
    [ -e "$out/ready" ] || { cat "$log"; say "session not ready"; return 1; }
    say "session $sess ($out)"
    FEAT_LOG="$(dirname "$(dirname "$TR")")/target/e2e-server-$FEAT_TAG/e2e-server.log"
    say "server log $FEAT_LOG"
    fs() { "$TR/ui.sh" fs "$1" || say "fs failed: $1"; }
    mark() { "$TR/ui.sh" mark "$1" > /dev/null; }
    shot() { "$TR/ui.sh" shot "f-$1" > /dev/null; }
    key() { "$TR/ui.sh" key "$@" > /dev/null; }
    keydown() { "$TR/ui.sh" down "$1" > /dev/null; }
    keyup() { "$TR/ui.sh" up "$1" > /dev/null; }
    mouse() { "$TR/ui.sh" mouse "$@" > /dev/null; }
    chat() { key t; sleep 0.6; "$TR/ui.sh" type "$1"; sleep 0.3; key Return; }
    bot() { "$TR/ui.sh" bot "$@" > /dev/null; "$TR/ui.sh" wait "\\[ui-fs\\] connect [0-9]+ $1" 20 > /dev/null || say "bot $1 did not join"; }
    waitlog() { "$TR/ui.sh" wait "$1" "$2" > /dev/null; }
    d3dcap() { "$TR/ui.sh" d3d all 2 "$1" > /dev/null; }
    check() { :; }
    export FEAT_LOG
    source "$FEAT_WT/tools/e2e/featscen.sh"
    sleep 2
    feat_scenario
    sleep 2
    "$TR/ui.sh" stop > /dev/null
    wait "$pid"
    tail -n 3 "$log"
    cp -f "$FEAT_LOG" "$out/server.log" 2>/dev/null
    say "run: $(readlink -f "$out")"
}
export -f inner say
FEAT_WT="$ROOT"
export TR CLIENT FEAT_WT ROOT FEAT_TAG
exec "${LOCKCMD[@]}" bash -c inner
