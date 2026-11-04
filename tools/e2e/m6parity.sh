#!/usr/bin/env bash
# M6 parity runs: the world-entity scenario (tools/e2e/m6scen.sh) traced on the ORIGINAL client and on ours.
#   tools/e2e/m6parity.sh orig   original samp.dll in the trace lane: SAMP_RS_ROOT/tools/trace/run.sh orig hold,
#                                driven by its ui.sh (takes the trace-lane lock for the whole run)
#   tools/e2e/m6parity.sh ours   our samp_client.dll + the same samp-tracer build in lane veh (prefix-shot,
#                                gta-sa-shot, UDP 7820, control 47720/47722): a private copy of the trace tools
#                                rooted at target/vehroot with the lane names replaced; nothing is rebuilt there
# Both print the run directory (trace.jsonl, server.log, net-*.log, screenshots). Compare with
#   SAMP_RS_ROOT/tools/trace/diff.py ORIG_RUN OURS_RUN --from objs-create --to scenario-end …
# M6_ONLY selects scenario blocks like m6.sh.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
CLIENT="${1:?usage: $0 orig|ours}"
say() { printf '[m6parity] %s\n' "$*"; }

if [ "$CLIENT" = ours ]; then
    VR="$ROOT/target/vehroot"
    BIN="$VR/target/i686-pc-windows-gnu/release"
    mkdir -p "$BIN" "$VR/target/locks"
    rm -rf "$VR/tools"; cp -r "$DATA/tools" "$VR/tools"
    for d in wine data server re; do ln -sfn "$DATA/$d" "$VR/$d"; done
    ln -sfn "$ROOT/crates" "$VR/crates"
    # Same tracer build as the original's run (main's target), our client from this worktree.
    # M6_TRACER_DIR: a saved copy of the tracer + launcher the original's run used (main may rebuild them).
    TD="${M6_TRACER_DIR:-$DATA/target/i686-pc-windows-gnu/release}"
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
    "$TR/run.sh" "$CLIENT" hold > "$log" 2>&1 &
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
    fs() { "$TR/ui.sh" fs "$1" > /dev/null || say "fs failed: $1"; }
    mark() { "$TR/ui.sh" mark "$1" > /dev/null; }
    shot() { "$TR/ui.sh" shot "w-$1" > /dev/null; }
    fire() { "$TR/ui.sh" fire "$1" > /dev/null; }
    check() { :; }
    source "$M6_WT/tools/e2e/m6scen.sh"
    sleep 2
    m6_scenario
    sleep 2
    "$TR/ui.sh" stop > /dev/null
    wait "$pid"
    tail -n 3 "$log"
    say "run: $(readlink -f "$out")"
}
export -f inner say
M6_WT="$ROOT"
export TR CLIENT M6_WT
exec "$TR/with-lock.sh" bash -c inner
