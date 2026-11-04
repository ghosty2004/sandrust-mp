#!/usr/bin/env bash
# DIFF-3 / criterion F3: the ORIGINAL client and OUR client on one server at the
# same time; each side's view of the other player is compared with that player's
# own local state, both directions (tools/diff/xclient.py).
#
#   tools/diff/cross-client.sh [--skip-build] [--calibrate] [--patches-only]
#   tools/diff/cross-client.sh --report RUN_DIR        (report only)
#
# Lanes (never touches main/ui/veh/trace):
#   A: original samp.dll + samp-tracer   WINEPREFIX=wine/prefix-diff  GTA_DIR=wine/gta-sa-diff   ctl 47706
#   B: our samp_client.dll               WINEPREFIX=wine/prefix-diff2 GTA_DIR=wine/gta-sa-diff2  ctl 47705
#      (--calibrate: a second ORIGINAL client + tracer in lane B, ctl 47705 = its tracer, to
#       measure the original-vs-original noise floor the tolerances are set from)
#   one server: E2E_TAG=diff, UDP 7795 (target/e2e-server-diff), diffbase gamemode +
#   e2eprobe + tools/diff/xclient.pwn, RCON on with a random per-run password.
#   Each game runs on its own isolated headless gamescope (display tags xc-a / xc-b).
# Only top-level files of the game dirs are replaced (rm + cp): their subdirectories are
# hard links shared with other lanes. Cleanup: wineserver -k with these two prefixes,
# the server by its exact binary path, gamescope by process group. No pkill.
#
# Output: target/diff/cross-client.{md,json} (--calibrate: cross-client-calibration.{md,json}; later
# reports learn tolerances and the original's own relations from target/diff/xcalib-latest); run data in target/diff/runs/xclient-<stamp>/
# (samples.jsonl, meta.json, patch-bytes-{A,B}.json, server.log, launch logs).
# Exit 0 only if every enabled step matches in both directions.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
A_PREFIX="$ROOT/wine/prefix-diff";  A_GTA="$ROOT/wine/gta-sa-diff";  A_PORT=47706
B_PREFIX="$ROOT/wine/prefix-diff2"; B_GTA="$ROOT/wine/gta-sa-diff2"; B_PORT=47705
export WINEPREFIX="$A_PREFIX" GTA_DIR="$A_GTA"
export E2E_TAG=diff E2E_PORT=7795 E2E_CTL_PORT=47705
DIFF_OUT="$ROOT/target/diff"
DIFF_BIN="${DIFF_CARGO_TARGET:-$ROOT/target/diff-cargo}/i686-pc-windows-gnu/release"
A_NAME=XcOrig; B_NAME=XcOurs
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"

say() { printf '[xc] %s\n' "$*"; }
lane_wine_kill() { WINEPREFIX="$A_PREFIX" wineserver -k 2>/dev/null; WINEPREFIX="$B_PREFIX" wineserver -k 2>/dev/null; true; }
put() { rm -f "$2/$3"; cp "$1" "$2/$3"; }   # never write through a hard link

# Original-vs-original baseline (--calibrate) used by the report, if present.
calib_args() { [ -f "$DIFF_OUT/xcalib-latest/samples.jsonl" ] && echo --calib "$(readlink -f "$DIFF_OUT/xcalib-latest")"; }
if [ "${1:-}" = --report ]; then
    # shellcheck disable=SC2046
    exec python3 -I "$HERE/xclient.py" report --run "$2" --out "$DIFF_OUT" $(calib_args)
fi

SKIP_BUILD=0; CALIBRATE=0; PATCHES_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1; shift ;;
        --calibrate) CALIBRATE=1; shift ;;
        --patches-only) PATCHES_ONLY=1; shift ;;
        *) echo "usage: $0 [--skip-build] [--calibrate] [--patches-only] | --report RUN_DIR" >&2; exit 2 ;;
    esac
done
# Lanes diff/diff2 are shared (DIFF harnesses, COV-1): hold target/locks/diff.lock for the run.
source "$HERE/lane-lock.sh"
diff_lane_lock
[ "$CALIBRATE" = 1 ] && B_NAME=XcOrig2
B_KIND=ours; [ "$CALIBRATE" = 1 ] && B_KIND=orig
mkdir -p "$DIFF_OUT/runs"

if [ "$SKIP_BUILD" != 1 ]; then
    say "building tracer, launcher, client (CARGO_TARGET_DIR=${DIFF_BIN%/i686-pc-windows-gnu/release})"
    if ! (cd "$ROOT" && CARGO_TARGET_DIR="${DIFF_BIN%/i686-pc-windows-gnu/release}" cargo build -q --release \
            --target i686-pc-windows-gnu -p samp-tracer -p samp-launcher -p samp-client) > "$DIFF_OUT/xc-build.out" 2>&1; then
        cat "$DIFF_OUT/xc-build.out"; say "build failed"; exit 1
    fi
fi
for f in samp_tracer.dll samp-launcher.exe samp_client.dll; do
    [ -f "$DIFF_BIN/$f" ] || { say "missing $DIFF_BIN/$f"; exit 1; }
done
[ -f "$ROOT/data/auth_table.txt" ] || say "warning: data/auth_table.txt missing; our client cannot join"
for p in "$A_PORT" "$B_PORT"; do
    if [ "$(E2E_CTL_PORT=$p e2e_ctl ping)" = pong ]; then say "port $p already answers; refusing"; exit 1; fi
done
for g in "$A_GTA" "$B_GTA"; do
    put "$DIFF_BIN/samp_tracer.dll" "$g" samp-tracer.dll
    put "$DIFF_BIN/samp-launcher.exe" "$g" samp-rs-launcher.exe
    put "$DIFF_BIN/samp_client.dll" "$g" samp_client.dll
    rm -f "$g/trace.jsonl" "$g/samp-tracer.log" "$g/samp-rs.log"
done

OUT="$DIFF_OUT/runs/xclient-$(date +%Y%m%d-%H%M%S)"
[ "$CALIBRATE" = 1 ] && OUT="$DIFF_OUT/runs/xcalib-$(date +%Y%m%d-%H%M%S)"
[ "$PATCHES_ONLY" = 1 ] && OUT="$DIFF_OUT/runs/xpatch-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
sha256sum "$A_GTA/samp-tracer.dll" "$A_GTA/samp.dll" "$B_GTA/samp_client.dll" > "$OUT/binaries.sha256"
RCON_PASS="xc$RANDOM$RANDOM"

xc_server_start() {
    e2e_server_prepare || return 1
    ( cd "$E2E_SERVER" \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$HERE/diffbase.pwn" -iqawno/include -ogamemodes/diffbase.amx -d0 > /dev/null \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$HERE/xclient.pwn" -iqawno/include -ofilterscripts/xclient.amx -d0 > /dev/null ) \
        || { say "pawn compile failed"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" "$RCON_PASS" <<'PY'
import json, sys
p, pw = sys.argv[1], sys.argv[2]
d = json.load(open(p))
d["pawn"]["main_scripts"] = ["diffbase 1"]
d["pawn"]["side_scripts"].append("filterscripts/xclient")
d["rcon"]["enable"] = True
d["rcon"]["password"] = pw
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[xc\] xclient loaded' 5 || { say "xclient filterscript not loaded"; return 1; }
}

# launch_game PREFIX GTA DISPLAY CLIENT(orig|ours) NAME CTLPORT LOG
launch_game() {
    local prefix="$1" gta="$2" disp="$3" client="$4" name="$5" port="$6" log="$7"
    (
        export WINEPREFIX="$prefix" GTA_DIR="$gta" E2E_HEADLESS=1 DISPLAY="$disp"
        unset WAYLAND_DISPLAY
        # Per lane: private Documents (gta_sa.set, sa-mp.cfg, chatlog) and the headless video mode.
        source "$ROOT/tools/wine-env.sh"
        dlls=()
        if [ "$client" = ours ]; then
            export SAMP_RS_CTL_PORT="$port"
            SAMP_RS_AUTH_TABLE="$(winepath -w "$ROOT/data/auth_table.txt")"; export SAMP_RS_AUTH_TABLE
            dlls+=(--dll "$(winepath -w "$gta/samp_client.dll")")
        else
            export SAMP_TRACER_PORT="$port"
            dlls+=(--dll "$(winepath -w "$gta/samp-tracer.dll")" --dll "$(winepath -w "$gta/samp.dll")")
        fi
        cd "$gta" && exec wine "$(winepath -w "$gta/samp-rs-launcher.exe")" \
            --gta "$(winepath -w "$gta/gta_sa.exe")" "${dlls[@]}" \
            --name "$name" --host 127.0.0.1 --port "$E2E_PORT"
    ) > "$log" 2>&1 &
    LAUNCH_PID=$!
}

GS_A=; GS_B=
cleanup() {
    E2E_CTL_PORT=$A_PORT e2e_ctl quit > /dev/null; E2E_CTL_PORT=$B_PORT e2e_ctl quit > /dev/null
    local i
    for ((i = 0; i < 20; i++)); do
        { [ -n "${LA:-}" ] && kill -0 "$LA" 2>/dev/null; } || { [ -n "${LB:-}" ] && kill -0 "$LB" 2>/dev/null; } || break
        sleep 1
    done
    lane_wine_kill
    [ -n "$GS_A" ] && { E2E_GAMESCOPE_PID=$GS_A; e2e_headless_stop; }
    [ -n "$GS_B" ] && { E2E_GAMESCOPE_PID=$GS_B; e2e_headless_stop; }
    GS_A=; GS_B=
    e2e_server_stop
}
trap cleanup EXIT

lane_wine_kill
xc_server_start || exit 1
E2E_DISPLAY_TAG=xc-a e2e_headless_start sleep infinity || exit 1
GS_A=$E2E_GAMESCOPE_PID; DISP_A=$E2E_HEADLESS_DISPLAY; WL_A=$E2E_GAMESCOPE
E2E_DISPLAY_TAG=xc-b e2e_headless_start sleep infinity || exit 1
GS_B=$E2E_GAMESCOPE_PID; DISP_B=$E2E_HEADLESS_DISPLAY; WL_B=$E2E_GAMESCOPE
say "displays: A=$DISP_A B=$DISP_B; run -> $OUT"

launch_game "$A_PREFIX" "$A_GTA" "$DISP_A" orig "$A_NAME" "$A_PORT" "$OUT/launch-A.out"; LA=$LAUNCH_PID
launch_game "$B_PREFIX" "$B_GTA" "$DISP_B" "$B_KIND" "$B_NAME" "$B_PORT" "$OUT/launch-B.out"; LB=$LAUNCH_PID

args=(drive --out "$OUT" --log "$E2E_LOG" --port "$E2E_PORT" --rcon-pass "$RCON_PASS"
      --a-port "$A_PORT" --a-display "$DISP_A" --a-name "$A_NAME" --a-kind orig --a-gamescope "$WL_A"
      --b-port "$B_PORT" --b-display "$DISP_B" --b-name "$B_NAME" --b-kind "$B_KIND" --b-gamescope "$WL_B")
[ "$PATCHES_ONLY" = 1 ] && args+=(--patches-only)
python3 -I "$HERE/xclient.py" "${args[@]}"
rc=$?
cleanup
for f in trace.jsonl samp-tracer.log samp-rs.log samp-rs-launcher.log; do
    [ -f "$A_GTA/$f" ] && cp "$A_GTA/$f" "$OUT/A-$f"
    [ -f "$B_GTA/$f" ] && cp "$B_GTA/$f" "$OUT/B-$f"
done
cp "$E2E_LOG" "$OUT/server.log" 2>/dev/null
if [ "$PATCHES_ONLY" = 1 ]; then ln -sfn "runs/$(basename "$OUT")" "$DIFF_OUT/xpatch-latest"
elif [ "$CALIBRATE" = 1 ]; then ln -sfn "runs/$(basename "$OUT")" "$DIFF_OUT/xcalib-latest"
else ln -sfn "runs/$(basename "$OUT")" "$DIFF_OUT/xclient-latest"; fi
say "driver exit $rc"
[ "$PATCHES_ONLY" = 1 ] && exit "$rc"
[ "$rc" = 0 ] || { say "scenario incomplete; report covers what ran"; }
if [ "$CALIBRATE" = 1 ]; then
    python3 -I "$HERE/xclient.py" report --run "$OUT" --out "$DIFF_OUT"
else
    # shellcheck disable=SC2046
    python3 -I "$HERE/xclient.py" report --run "$OUT" --out "$DIFF_OUT" $(calib_args)
fi
rep=$?
[ "$rc" = 0 ] && exit "$rep"
exit 1
