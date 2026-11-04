#!/usr/bin/env bash
# EFX-1 helpers (source from tools/efx/*.sh). Uses the trace lane (tools/trace/lib.sh of the
# checkout that holds the gitignored wine/ and server/: EFX_LANE_ROOT, default this checkout),
# so the lane's lock (target/locks/trace.lock), prefix, game copy, ports and server copy are
# shared with tools/trace/run.sh. The DLLs come from THIS checkout's build (CARGO_TARGET_DIR).
EFX_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EFX_LANE="${EFX_LANE_ROOT:-$EFX_REPO}"
if [ ! -d "$EFX_LANE/wine/prefix-trace" ]; then
    echo "efx: no trace lane under $EFX_LANE (set EFX_LANE_ROOT to the checkout holding wine/)" >&2
    return 1 2>/dev/null || exit 1
fi
# shellcheck disable=SC1091
source "$EFX_LANE/tools/trace/lib.sh"
EFX_TARGET="${CARGO_TARGET_DIR:-$EFX_REPO/target}"
TRACE_BIN="$EFX_TARGET/i686-pc-windows-gnu/release"
EFX_OUT_ROOT="$EFX_TARGET/efx"
export EFX_REPO EFX_LANE EFX_TARGET EFX_OUT_ROOT
export EFX_LANE_ROOT="$EFX_LANE"

efx_say() { printf '[efx] %s\n' "$*"; }

# Overrides the lane's trace_build: build this checkout.
trace_build() {
    local pkgs=(-p samp-tracer -p samp-launcher)
    [ "${1:-}" = ours ] && pkgs+=(-p samp-client)
    (cd "$EFX_REPO" && cargo build -q --release --target i686-pc-windows-gnu "${pkgs[@]}")
}

efx_ctl() { trace_ctl "efx $*"; }

# efx_server_start — lane server copy + tools/efx/efxfs.pwn as filterscripts/efxfs.
efx_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$EFX_REPO/tools/efx/efxfs.pwn" \
        -iqawno/include -ofilterscripts/efxfs.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"].append("filterscripts/efxfs")
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$E2E_SERVER/scriptfiles"
    : > "$E2E_SERVER/scriptfiles/efxcmd.txt"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { echo "efx: server did not start" >&2; return 1; }
}

# efx_fs 'CMD|ARG|…' — runs one efxfs.pwn command; prints its output lines.
efx_fs() {
    local f="$E2E_SERVER/scriptfiles/efxcmd.txt" seq
    seq=$(( $(wc -l < "$f" 2>/dev/null || echo 0) + 1 ))
    printf '%d %s\n' "$seq" "$1" >> "$f"
    e2e_wait_log "\[efx-fs\] $seq done" 10 || { echo "efx: command $seq ($1) not run" >&2; return 1; }
    grep -E "\[efx-fs\] $seq " "$E2E_LOG" | grep -v " done " || true
}

# efx_done_count — N of the tracer's "idle done=N" (empty while busy).
efx_done_count() { efx_ctl status | sed -n 's/^idle done=\([0-9]*\).*/\1/p'; }

# efx_wait_done N SECONDS — until capture/window number N has finished.
efx_wait_done() {
    local want="$1" i n
    for ((i = 0; i < $2 * 4; i++)); do
        n=$(efx_done_count)
        [ -n "$n" ] && [ "$n" -ge "$want" ] && return 0
        sleep 0.25
    done
    return 1
}
