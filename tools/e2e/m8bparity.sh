#!/usr/bin/env bash
# M8b parity runs: the remaining-behaviour scenario (tools/e2e/m8bscen.sh) traced on the ORIGINAL client and on ours.
#   tools/e2e/m8bparity.sh orig   original samp.dll in the trace lane: SAMP_RS_ROOT/tools/trace/run.sh orig hold,
#                                driven by its ui.sh (takes the trace-lane lock for the whole run)
#   tools/e2e/m8bparity.sh ours   our samp_client.dll + the same samp-tracer build in lane veh (prefix-shot,
#                                gta-sa-shot, UDP 7820, control 47720/47722): a private copy of the trace tools
#                                rooted at target/vehroot with the lane names replaced; nothing is rebuilt there
# Wrap `ours` in `flock SAMP_RS_ROOT/target/locks/veh.lock`. Both print the run directory (trace.jsonl, server.log, net-*.log, screenshots). Compare with
#   SAMP_RS_ROOT/tools/trace/diff.py ORIG_RUN OURS_RUN --from campos --to cam_done …
# M8B_ONLY selects scenario blocks like m8b.sh. Both runs use TRACE_GAMEMODE=tracegm (as TRACE-8's run did).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
CLIENT="${1:?usage: $0 orig|ours}"
say() { printf '[m8bparity] %s\n' "$*"; }

if [ "$CLIENT" = ours ]; then
    # The private root holds its own trace.lock (lane veh is serialised by the caller's veh.lock).
    unset SAMP_RS_ROOT
    VR="$ROOT/target/vehroot"
    BIN="$VR/target/i686-pc-windows-gnu/release"
    mkdir -p "$BIN" "$VR/target/locks"
    rm -rf "$VR/tools"; cp -r "$DATA/tools" "$VR/tools"
    for d in wine data server re; do ln -sfn "$DATA/$d" "$VR/$d"; done
    # ui.sh `bot` runs target/debug/samp-bot of the tools' root: main's build.
    ln -sfn "$DATA/target/debug" "$VR/target/debug"
    ln -sfn "$ROOT/crates" "$VR/crates"
    # Same tracer build as the original's run (main's target), our client from this worktree.
    # M8B_TRACER_DIR: a saved copy of the tracer + launcher the original's run used (main may rebuild them).
    TD="${M8B_TRACER_DIR:-$DATA/target/i686-pc-windows-gnu/release}"
    cp "$TD/samp_tracer.dll" "$BIN/"
    cp "$TD/samp-launcher.exe" "$BIN/"
    cp "${CARGO_TARGET_DIR:-$ROOT/target}/i686-pc-windows-gnu/release/samp_client.dll" "$BIN/"
    sha256sum "$BIN"/*.dll "$BIN"/*.exe > "$VR/binaries.sha256"
    L="$VR/tools/trace/lib.sh"
    sed -i -e 's#wine/prefix-trace#wine/prefix-shot#g' -e 's#wine/gta-sa-trace#wine/gta-sa-shot#g' \
        -e 's#E2E_TAG=trace E2E_PORT=7790 E2E_CTL_PORT=47701#E2E_TAG=veh E2E_PORT=7820 E2E_CTL_PORT=47720#' \
        -e 's#SAMP_TRACER_PORT=47702#SAMP_TRACER_PORT=47722#' "$L"
    # No rebuilds in the private root (the binaries above are the ones compared).
    python3 -I - "$L" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"trace_build\(\) \{.*?\n\}", "trace_build() { true; }", s, count=1, flags=re.S)
open(p, "w").write(s)
PY
    grep -q 'prefix-shot' "$L" && grep -q 'E2E_PORT=7820' "$L" && grep -q 'trace_build() { true; }' "$L" \
        || { say "could not re-root the trace tools"; exit 1; }
    TR="$VR/tools/trace"
else
    TR="$DATA/tools/trace"
fi

# Everything below runs under the lane lock (with-lock exports SAMP_RS_TRACE_LOCK_HELD=1).
inner() {
    local log sess out pid i
    log="$(mktemp)"
    TRACE_GAMEMODE=tracegm "$TR/run.sh" "$CLIENT" hold > "$log" 2>&1 &
    pid=$!
    for ((i = 0; i < 400; i++)); do
        sess="$(sed -n 's/.*session: \([^ ]*\).*/\1/p' "$log" | head -n 1)"
        [ -n "$sess" ] && break
        kill -0 "$pid" 2>/dev/null || { cat "$log"; say "run.sh ended early"; return 1; }
        sleep 1
    done
    [ -n "$sess" ] || { cat "$log"; say "no session id"; return 1; }
    export TRACE_SESSION="$sess"
    out="$(dirname "$TR")/../target/trace/${sess%%.*}"
    for ((i = 0; i < 200; i++)); do [ -e "$out/ready" ] && break; sleep 1; done
    [ -e "$out/ready" ] || { cat "$log"; say "session not ready"; return 1; }
    say "session $sess ($out)"
    fs() { "$TR/ui.sh" fs "$1" || say "fs failed: $1"; }
    mark() { "$TR/ui.sh" mark "$1" > /dev/null; }
    shot() { "$TR/ui.sh" shot "w-$1" > /dev/null; }
    keydown() { "$TR/ui.sh" down "$1" > /dev/null; }
    keyup() { "$TR/ui.sh" up "$1" > /dev/null; }
    bot() { "$TR/ui.sh" bot "$1" > /dev/null; "$TR/ui.sh" wait "\\[ui-fs\\] connect [0-9]+ $1" 20 > /dev/null || say "bot $1 did not join"; }
    check() { :; }
    source "$M8B_WT/tools/e2e/m8bscen.sh"
    sleep 2
    m8b_scenario
    sleep 2
    "$TR/ui.sh" stop > /dev/null
    wait "$pid"
    tail -n 3 "$log"
    say "run: $(readlink -f "$out")"
}
export -f inner say
M8B_WT="$ROOT"
export TR CLIENT M8B_WT
exec "$TR/with-lock.sh" bash -c inner
