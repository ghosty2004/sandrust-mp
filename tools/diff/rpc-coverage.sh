#!/usr/bin/env bash
# DIFF-2 / criterion F1: per-RPC reaction diff, original client vs ours.
#
#   tools/diff/rpc-coverage.sh [--orig-runs N] [--ours-runs N] [--only REGEX] [--skip-build]
#   tools/diff/rpc-coverage.sh --report --orig RUN [--orig RUN2] [--ours RUN]   (report only)
#
# Lane "diff" only: WINEPREFIX=wine/prefix-diff, GTA_DIR=wine/gta-sa-diff, server UDP 7795
# (target/e2e-server-diff), control ports 47705 (our client) / 47706 (tracer), isolated
# headless gamescope display. Never touches the main or trace lanes.
#
# Each run: server (tools/diff/diffbase.pwn gamemode + e2eprobe + tools/diff/rpcfire.pwn,
# RCON on) -> game via samp-launcher with --dll samp-tracer.dll --dll <client> -> join,
# spawn -> tools/diff/drive.py fires every step of rpcfire.pwn, marking the trace per step
# and collecting `state`, a peek snapshot (tools/diff/gtastate.py) and a screenshot.
# Default: the original client six times (calibration: what differs between original runs is not
# judged, docs/ROADMAP.md DIFF2-CAL) and ours once; then
# tools/diff/rpc_report.py writes target/diff/rpc-coverage.{md,json}.
# Exit 0 only if every triggered RPC is MATCH (DIFF and NOT-IMPLEMENTED fail).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export WINEPREFIX="$ROOT/wine/prefix-diff"
export GTA_DIR="$ROOT/wine/gta-sa-diff"
export E2E_TAG=diff E2E_PORT=7795 E2E_CTL_PORT=47705
DIFF_TRACER_PORT=47706
DIFF_OUT="$ROOT/target/diff"
DIFF_BIN="${DIFF_CARGO_TARGET:-$ROOT/target/diff-cargo}/i686-pc-windows-gnu/release"
DIFF_NAME=DiffPlayer
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"

say() { printf '[diff] %s\n' "$*"; }
lane_wine_kill() { WINEPREFIX="$ROOT/wine/prefix-diff" wineserver -k 2>/dev/null || true; }
# Replace a top-level file of the lane's game dir (never write through a hard link).
lane_put() { rm -f "$GTA_DIR/$2"; cp "$1" "$GTA_DIR/$2"; }
user_files() { echo "$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files"; }

# ---- inner part: runs inside e2e_headless_run --------------------------------
if [ "${1:-}" = __inner ]; then
    CLIENT="$2"; OUT="$3"; ONLY="${4:-}"
    export SAMP_TRACER_PORT="$DIFF_TRACER_PORT"
    dlls=(--dll "$(winepath -w "$GTA_DIR/samp-tracer.dll")")
    if [ "$CLIENT" = ours ]; then
        export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
        [ -f "$ROOT/data/auth_table.txt" ] && SAMP_RS_AUTH_TABLE="$(winepath -w "$ROOT/data/auth_table.txt")" \
            && export SAMP_RS_AUTH_TABLE
        dlls+=(--dll "$(winepath -w "$GTA_DIR/samp_client.dll")")
        CLIENT_PORT="$E2E_CTL_PORT"
    else
        dlls+=(--dll "$(winepath -w "$GTA_DIR/samp.dll")")
        CLIENT_PORT="$DIFF_TRACER_PORT"   # the tracer answers `state` for the original (DIFF-1)
    fi
    ( cd "$GTA_DIR" && wine_launch samp-diff "$GTA_DIR/samp-rs-launcher.exe" \
        --gta "$(winepath -w "$GTA_DIR/gta_sa.exe")" "${dlls[@]}" \
        --name "$DIFF_NAME" --host 127.0.0.1 --port "$E2E_PORT" ) > "$OUT/launch.out" 2>&1 &
    LPID=$!
    args=(--client "$CLIENT" --out "$OUT" --log "$E2E_LOG" --port "$E2E_PORT"
          --rcon-pass "$(cat "$OUT/.rcon")" --client-port "$CLIENT_PORT" --tracer-port "$DIFF_TRACER_PORT")
    [ -n "$ONLY" ] && args+=(--only "$ONLY")
    python3 -I "$HERE/drive.py" "${args[@]}"
    rc=$?
    E2E_CTL_PORT="$CLIENT_PORT" e2e_ctl quit > /dev/null
    E2E_CTL_PORT="$DIFF_TRACER_PORT" e2e_ctl quit > /dev/null
    for ((i = 0; i < 30; i++)); do kill -0 "$LPID" 2>/dev/null || break; sleep 1; done
    kill -0 "$LPID" 2>/dev/null && { say "game still running; killing lane wine"; lane_wine_kill; }
    wait "$LPID" 2>/dev/null
    exit "$rc"
fi

# ---- outer part ----------------------------------------------------------------
if [ "${1:-}" = --report ]; then
    shift
    exec python3 -I "$HERE/rpc_report.py" --out "$DIFF_OUT" "$@"
fi

ORIG_RUNS=6; OURS_RUNS=1; ONLY=""; SKIP_BUILD=0; REUSE_ORIG=6
while [ $# -gt 0 ]; do
    case "$1" in
        --orig-runs) ORIG_RUNS="$2"; shift 2 ;;
        --ours-runs) OURS_RUNS="$2"; shift 2 ;;
        --only) ONLY="$2"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --reuse-orig) REUSE_ORIG="$2"; shift 2 ;;
        *) echo "usage: $0 [--orig-runs N] [--ours-runs N] [--reuse-orig N] [--only REGEX] [--skip-build] | --report RUN..." >&2; exit 2 ;;
    esac
done
# Lanes diff/diff2 are shared (DIFF harnesses, COV-1): hold target/locks/diff.lock for the run.
source "$HERE/lane-lock.sh"
diff_lane_lock
mkdir -p "$DIFF_OUT/runs"

if [ "$SKIP_BUILD" != 1 ]; then
    say "building tracer, launcher, client (CARGO_TARGET_DIR=${DIFF_BIN%/i686-pc-windows-gnu/release})"
    if ! (cd "$ROOT" && CARGO_TARGET_DIR="${DIFF_BIN%/i686-pc-windows-gnu/release}" cargo build -q --release \
            --target i686-pc-windows-gnu -p samp-tracer -p samp-launcher -p samp-client) > "$DIFF_OUT/build.out" 2>&1; then
        cat "$DIFF_OUT/build.out"; say "build failed"; exit 1
    fi
fi
for f in samp_tracer.dll samp-launcher.exe samp_client.dll; do
    [ -f "$DIFF_BIN/$f" ] || { say "missing $DIFF_BIN/$f"; exit 1; }
done
lane_put "$DIFF_BIN/samp_tracer.dll" samp-tracer.dll
lane_put "$DIFF_BIN/samp-launcher.exe" samp-rs-launcher.exe
lane_put "$DIFF_BIN/samp_client.dll" samp_client.dll
[ -f "$ROOT/data/auth_table.txt" ] || say "warning: data/auth_table.txt missing; our client cannot join"

# Lane server: base gamemode + probe + RPC filterscript, RCON on (local only).
diff_server_start() {
    local pass="$1"
    e2e_server_prepare || return 1
    ( cd "$E2E_SERVER" \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$HERE/diffbase.pwn" -iqawno/include -ogamemodes/diffbase.amx -d0 > /dev/null \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$HERE/rpcfire.pwn" -iqawno/include -ofilterscripts/rpcfire.amx -d0 > /dev/null ) \
        || { say "pawn compile failed"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" "$pass" <<'PY'
import json, sys
p, pw = sys.argv[1], sys.argv[2]
d = json.load(open(p))
d["pawn"]["main_scripts"] = ["diffbase 1"]
d["pawn"]["side_scripts"].append("filterscripts/rpcfire")
d["rcon"]["enable"] = True
d["rcon"]["password"] = pw
d["max_bots"] = 2
# Periodic RPC 60 every 30 s would land in random steps; once at connect is enough.
d["network"]["time_sync_rate"] = 86400000
# The same for 208 player markers: open.mp sends them from the receiving player's own update
# (markers before streaming in onPlayerUpdate), so a marker for a just-spawned NPC races its
# stream-in RPC 32 (coordinate blip or not, then shifted blip handles): once at connect is enough.
d["network"]["player_marker_sync_rate"] = 86400000
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[diff\] rpcfire loaded' 5 || { say "rpcfire filterscript not loaded"; return 1; }
}

# one_run CLIENT -> prints the run directory on the last line
one_run() {
    local client="$1" out rc pass uf
    out="$DIFF_OUT/runs/$client-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$out"
    for p in "$E2E_CTL_PORT" "$DIFF_TRACER_PORT"; do
        if [ "$(E2E_CTL_PORT=$p e2e_ctl ping)" = pong ]; then say "port $p already answers; refusing"; return 1; fi
    done
    lane_wine_kill
    rm -f "$GTA_DIR/trace.jsonl" "$GTA_DIR/samp-tracer.log" "$GTA_DIR/samp-rs.log"
    uf="$(user_files)"
    rm -f "$uf/SAMP/chatlog.txt"
    pass="diff$RANDOM$RANDOM"; echo "$pass" > "$out/.rcon"
    sha256sum "$GTA_DIR/samp-tracer.dll" "$GTA_DIR/samp_client.dll" "$GTA_DIR/samp.dll" > "$out/binaries.sha256"
    diff_server_start "$pass" || { e2e_server_stop; return 1; }
    say "$client run -> $out"
    e2e_headless_run "$0" __inner "$client" "$out" "$ONLY"
    rc=$?
    e2e_server_stop
    lane_wine_kill
    for f in trace.jsonl samp-tracer.log samp-rs-launcher.log samp-rs.log; do
        [ -f "$GTA_DIR/$f" ] && cp "$GTA_DIR/$f" "$out/"
    done
    [ -f "$uf/SAMP/chatlog.txt" ] && cp "$uf/SAMP/chatlog.txt" "$out/"
    cp "$E2E_LOG" "$out/server.log" 2>/dev/null
    rm -f "$out/.rcon"
    ln -sfn "runs/$(basename "$out")" "$DIFF_OUT/$client-latest"
    say "$client run exit $rc ($(wc -l < "$out/trace.jsonl" 2>/dev/null || echo 0) trace records)"
    RUN_DIR="$out"
    return "$rc"
}

trap 'e2e_headless_stop; e2e_server_stop; lane_wine_kill' EXIT
orig_dirs=(); ours_dirs=()
# A failed ORIGINAL run (e.g. no join) has no steps and would break the report: leave it out.
for ((n = 0; n < ORIG_RUNS; n++)); do
    if one_run orig; then orig_dirs+=("$RUN_DIR"); else say "orig run $((n + 1)) failed; not used in the report"; fi
done
for ((n = 0; n < OURS_RUNS; n++)); do
    one_run ours || say "ours run $((n + 1)) failed"
    ours_dirs+=("$RUN_DIR")
done
[ "${#orig_dirs[@]}" -gt 0 ] || { say "no usable original run; no report"; exit 1; }
# Calibration data accumulates: up to REUSE_ORIG earlier complete original runs of the same scenario (started after
# the last change of the scenario files and the driver, same samp.dll and tracer, every step fired, full step list) join the report.
if [ -z "$ONLY" ] && [ "$REUSE_ORIG" -gt 0 ]; then
    mapfile -t reuse < <(python3 -I - "$DIFF_OUT/runs" "$REUSE_ORIG" "${orig_dirs[@]}" \
        "$HERE/rpcfire.pwn" "$HERE/diffbase.pwn" "$0" "$ROOT/tools/e2e/probe/e2eprobe.pwn" "$HERE/drive.py" \
        "$GTA_DIR/samp.dll" "$GTA_DIR/samp-tracer.dll" <<'PY'
import glob, hashlib, json, os, sys
runs, n, rest = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
dlls, scen = rest[-2:], rest[-7:-2]
fresh = set(rest[:-7])
since = max(os.path.getmtime(f) for f in scen)
want = {hashlib.sha256(open(f, "rb").read()).hexdigest() for f in dlls}
out = []
for d in sorted(glob.glob(os.path.join(runs, "orig-*")), reverse=True):
    if d in fresh or os.path.getmtime(os.path.join(d, "binaries.sha256")) < since:
        continue
    try:
        sums = {l.split()[0] for l in open(os.path.join(d, "binaries.sha256"))}
        steps = [json.loads(l) for l in open(os.path.join(d, "steps.jsonl"))]
        listed = json.load(open(os.path.join(d, "step-list.json")))
    except (OSError, ValueError):
        continue
    if want <= sums and len(steps) == len(listed) and all(s.get("fired") for s in steps):
        out.append(d)
    if len(out) >= n:
        break
print("\n".join(out))
PY
)
    for d in "${reuse[@]}"; do [ -n "$d" ] && { say "calibration: reusing $d"; orig_dirs+=("$d"); }; done
fi
args=()
for d in "${orig_dirs[@]}"; do args+=(--orig "$d"); done
for d in "${ours_dirs[@]}"; do args+=(--ours "$d"); done
python3 -I "$HERE/rpc_report.py" --out "$DIFF_OUT" "${args[@]}"
