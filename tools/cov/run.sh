#!/usr/bin/env bash
# COV-1: first-hit coverage of gta_sa.exe functions per scenario window, original client vs ours.
#
#   tools/cov/run.sh [both|orig|ours|calib] [--skip-build] [--no-cov]
#     both (default): the two games run at the same time on one server, each in its own
#     virtual world (tools/cov/covfs.pwn), so they never stream each other.
#     calib: the ORIGINAL in both lanes (B = a second original + tracer): the noise floor
#     tools/cov/report.py --calib uses.
#     --no-cov: same scenario without arming (stability baseline).
#   Env: COV_IDLE_SECS (Grove Street idle, default 300), COV_END_SECS (final idle, default 60).
#
# Lanes (shared with DIFF-2/3: holds target/locks/diff.lock for the whole run):
#   A: original samp.dll + samp-tracer   WINEPREFIX=wine/prefix-diff  GTA_DIR=wine/gta-sa-diff   tracer 47706
#   B: our samp_client.dll + samp-tracer WINEPREFIX=wine/prefix-diff2 GTA_DIR=wine/gta-sa-diff2  client 47705, tracer 47707
#   one server: E2E_TAG=diff, UDP 7795 (target/e2e-server-diff), diffbase + e2eprobe + covfs, RCON on.
# Each game on its own headless gamescope. Cleanup: wineserver -k with these two prefixes, the
# server by its binary path, gamescope by process group. No pkill.
# Windows: boot, connecting, class_selection, spawn, grove_idle, walk, interior_enter,
# interior_exit, vehicle, idle_end (tools/cov/covdrive.py).
# Needs target/cov/arm.txt (tools/cov/inventory.sh). Builds into target/cov-cargo.
# Output: target/cov/runs/<stamp>/ (cov-A.tsv, cov-B.tsv, events.json, logs, trace.jsonl)
# and target/cov/latest; then tools/cov/report.py writes target/cov/coverage-diff.md.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
A_PREFIX="$ROOT/wine/prefix-diff";  A_GTA="$ROOT/wine/gta-sa-diff";  A_TPORT=47706
B_PREFIX="$ROOT/wine/prefix-diff2"; B_GTA="$ROOT/wine/gta-sa-diff2"; B_CPORT=47705; B_TPORT=47707
export WINEPREFIX="$A_PREFIX" GTA_DIR="$A_GTA"
export E2E_TAG=diff E2E_PORT=7795 E2E_CTL_PORT=47705
COV_OUT="$ROOT/target/cov"
BIN="$ROOT/target/cov-cargo/i686-pc-windows-gnu/release"
A_NAME=CovOrig; B_NAME=CovOurs
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
source "$ROOT/tools/diff/lane-lock.sh"

say() { printf '[cov] %s\n' "$*"; }
# Windows path of a Unix path through Wine's Z: drive, without starting Wine: a Wine process
# started here (outside the lane's headless display) would become the prefix's desktop
# process on the wrong display, and the game's fullscreen switch then fails.
zpath() { local p; p="$(readlink -m "$1")"; printf 'Z:%s' "${p//\//\\}"; }
put() { rm -f "$2/$3"; cp "$1" "$2/$3"; }   # never write through a hard link

WHICH=both; SKIP_BUILD=0; NO_COV=0
# COV_CLIENT_DLL=/path/samp_client.dll measures another build of our client (e.g. a worktree).
CLIENT_DLL="${COV_CLIENT_DLL:-$BIN/samp_client.dll}"
while [ $# -gt 0 ]; do
    case "$1" in
        both|orig|ours|calib) WHICH="$1"; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --no-cov) NO_COV=1; shift ;;
        *) echo "usage: $0 [both|orig|ours|calib] [--skip-build] [--no-cov]" >&2; exit 2 ;;
    esac
done
diff_lane_lock
lane_wine_kill() { WINEPREFIX="$A_PREFIX" wineserver -k 2>/dev/null; WINEPREFIX="$B_PREFIX" wineserver -k 2>/dev/null; true; }

[ -s "$COV_OUT/arm.txt" ] || { say "no $COV_OUT/arm.txt: run tools/cov/inventory.sh first"; exit 1; }
if [ "$SKIP_BUILD" != 1 ]; then
    say "building tracer, launcher, client (CARGO_TARGET_DIR=target/cov-cargo)"
    mkdir -p "$COV_OUT"
    if ! (cd "$ROOT" && CARGO_TARGET_DIR="$ROOT/target/cov-cargo" cargo build -q --release \
            --target i686-pc-windows-gnu -p samp-tracer -p samp-launcher -p samp-client) > "$COV_OUT/build.out" 2>&1; then
        cat "$COV_OUT/build.out"; say "build failed"; exit 1
    fi
fi
[ -f "$CLIENT_DLL" ] || { say "missing $CLIENT_DLL"; exit 1; }
for f in samp_tracer.dll samp-launcher.exe; do
    [ -f "$BIN/$f" ] || { say "missing $BIN/$f"; exit 1; }
done
for p in "$A_TPORT" "$B_CPORT" "$B_TPORT"; do
    if [ "$(E2E_CTL_PORT=$p e2e_ctl ping)" = pong ]; then say "port $p already answers; refusing"; exit 1; fi
done
for g in "$A_GTA" "$B_GTA"; do
    put "$BIN/samp_tracer.dll" "$g" samp-tracer.dll
    put "$BIN/samp-launcher.exe" "$g" samp-rs-launcher.exe
    put "$CLIENT_DLL" "$g" samp_client.dll
    put "$COV_OUT/arm.txt" "$g" cov-arm.txt
    rm -f "$g/trace.jsonl" "$g/samp-tracer.log" "$g/samp-rs.log" "$g/d3d.jsonl"
done

OUT="$COV_OUT/runs/$WHICH-$(date +%Y%m%d-%H%M%S)"
[ "$NO_COV" = 1 ] && OUT="$OUT-nocov"
mkdir -p "$OUT"
sha256sum "$BIN/samp_tracer.dll" "$A_GTA/samp.dll" "$CLIENT_DLL" "$COV_OUT/arm.txt" > "$OUT/binaries.sha256"
(cd "$ROOT" && git rev-parse HEAD && git status --short crates) > "$OUT/source.txt" 2>/dev/null
RCON_PASS="cov$RANDOM$RANDOM"

cov_server_start() {
    e2e_server_prepare || return 1
    ( cd "$E2E_SERVER" \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/diff/diffbase.pwn" -iqawno/include -ogamemodes/diffbase.amx -d0 > /dev/null \
        && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$HERE/covfs.pwn" -iqawno/include -ofilterscripts/covfs.amx -d0 > /dev/null ) \
        || { say "pawn compile failed"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" "$RCON_PASS" <<'PY'
import json, sys
p, pw = sys.argv[1], sys.argv[2]
d = json.load(open(p))
d["pawn"]["main_scripts"] = ["diffbase 1"]
d["pawn"]["side_scripts"].append("filterscripts/covfs")
d["rcon"]["enable"] = True
d["rcon"]["password"] = pw
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[cov\] covfs loaded' 5 || { say "covfs filterscript not loaded"; return 1; }
}

# launch_game PREFIX GTA DISPLAY CLIENT(orig|ours) NAME TRACER_PORT DUMP LOG
launch_game() {
    local prefix="$1" gta="$2" disp="$3" client="$4" name="$5" tport="$6" log="$8"
    (
        export WINEPREFIX="$prefix" GTA_DIR="$gta" E2E_HEADLESS=1 DISPLAY="$disp"
        unset WAYLAND_DISPLAY
        source "$ROOT/tools/wine-env.sh"
        export SAMP_TRACER_PORT="$tport"
        if [ "$NO_COV" != 1 ]; then
            SAMP_TRACER_COV="$(winepath -w "$gta/cov-arm.txt")"; export SAMP_TRACER_COV
        fi
        dlls=(--dll "$(winepath -w "$gta/samp-tracer.dll")")
        if [ "$client" = ours ]; then
            export SAMP_RS_CTL_PORT="$B_CPORT"
            SAMP_RS_AUTH_TABLE="$(winepath -w "$ROOT/data/auth_table.txt")"; export SAMP_RS_AUTH_TABLE
            dlls+=(--dll "$(winepath -w "$gta/samp_client.dll")")
        else
            dlls+=(--dll "$(winepath -w "$gta/samp.dll")")
        fi
        cd "$gta" && exec wine "$(winepath -w "$gta/samp-rs-launcher.exe")" \
            --gta "$(winepath -w "$gta/gta_sa.exe")" "${dlls[@]}" \
            --name "$name" --host 127.0.0.1 --port "$E2E_PORT"
    ) > "$log" 2>&1 &
    LAUNCH_PID=$!
}

GS_A=; GS_B=; LA=; LB=
cleanup() {
    [ -n "$LA" ] && { E2E_CTL_PORT=$A_TPORT e2e_ctl quit > /dev/null; }
    [ -n "$LB" ] && { E2E_CTL_PORT=$B_CPORT e2e_ctl quit > /dev/null; E2E_CTL_PORT=$B_TPORT e2e_ctl quit > /dev/null; }
    local i
    for ((i = 0; i < 20; i++)); do
        { [ -n "$LA" ] && kill -0 "$LA" 2>/dev/null; } || { [ -n "$LB" ] && kill -0 "$LB" 2>/dev/null; } || break
        sleep 1
    done
    lane_wine_kill
    [ -n "$GS_A" ] && { E2E_GAMESCOPE_PID=$GS_A; e2e_headless_stop; }
    [ -n "$GS_B" ] && { E2E_GAMESCOPE_PID=$GS_B; e2e_headless_stop; }
    GS_A=; GS_B=; LA=; LB=
    e2e_server_stop
}
trap cleanup EXIT

lane_wine_kill
cov_server_start || exit 1
sides=()
if [ "$WHICH" != ours ]; then
    E2E_DISPLAY_TAG=cov-a e2e_headless_start sleep infinity || exit 1
    GS_A=$E2E_GAMESCOPE_PID; DISP_A=$E2E_HEADLESS_DISPLAY
    launch_game "$A_PREFIX" "$A_GTA" "$DISP_A" orig "$A_NAME" "$A_TPORT" - "$OUT/launch-A.out"; LA=$LAUNCH_PID
    sides+=(--side "A|$A_NAME|$A_TPORT|$DISP_A|$(zpath "$OUT/cov-A.tsv")")
fi
if [ "$WHICH" != orig ]; then
    E2E_DISPLAY_TAG=cov-b e2e_headless_start sleep infinity || exit 1
    GS_B=$E2E_GAMESCOPE_PID; DISP_B=$E2E_HEADLESS_DISPLAY
    B_KIND=ours; [ "$WHICH" = calib ] && { B_KIND=orig; B_NAME=CovOrig2; }
    launch_game "$B_PREFIX" "$B_GTA" "$DISP_B" "$B_KIND" "$B_NAME" "$B_TPORT" - "$OUT/launch-B.out"; LB=$LAUNCH_PID
    sides+=(--side "B|$B_NAME|$B_TPORT|$DISP_B|$(zpath "$OUT/cov-B.tsv")")
fi
say "run -> $OUT (sides: $WHICH, cov $([ "$NO_COV" = 1 ] && echo off || echo on))"
# Liveness: one `state` line per side every 10 s, to see a crash or a hang.
# (Ends with this script; fd 8, the lane lock, is not inherited.)
MAIN_PID=$$
(
    while sleep 10 && kill -0 "$MAIN_PID" 2>/dev/null; do
        [ -n "${LA:-}" ] && echo "$(date +%T) A $(E2E_CTL_PORT=$A_TPORT e2e_ctl state)"
        [ -n "${LB:-}" ] && echo "$(date +%T) B $(E2E_CTL_PORT=$B_TPORT e2e_ctl state)"
    done
) > "$OUT/liveness.log" 2>&1 8>&- &
LIVE=$!
python3 -I "$HERE/covdrive.py" --out "$OUT" --log "$E2E_LOG" --port "$E2E_PORT" --rcon-pass "$RCON_PASS" "${sides[@]}"
rc=$?
kill "$LIVE" 2>/dev/null
cleanup
for k in A B; do
    g="$A_GTA"; [ "$k" = B ] && g="$B_GTA"
    for f in trace.jsonl samp-tracer.log samp-rs.log samp-rs-launcher.log; do
        [ -f "$g/$f" ] && cp "$g/$f" "$OUT/$k-$f"
    done
done
cp "$E2E_LOG" "$OUT/server.log" 2>/dev/null
ln -sfn "runs/$(basename "$OUT")" "$COV_OUT/latest"
say "driver exit $rc; output $OUT"
grep -h 'cov:' "$OUT"/*-samp-tracer.log 2>/dev/null | head -20
if [ "$WHICH" = both ] && [ -s "$OUT/cov-A.tsv" ] && [ -s "$OUT/cov-B.tsv" ]; then
    calib=()
    for k in A B; do
        [ -s "$COV_OUT/calib-latest/cov-$k.tsv" ] && calib+=(--calib "$COV_OUT/calib-latest/cov-$k.tsv")
    done
    python3 -I "$HERE/report.py" --orig "$OUT/cov-A.tsv" --ours "$OUT/cov-B.tsv" "${calib[@]}" \
        && python3 -I "$HERE/triage.py"
fi
[ "$WHICH" = calib ] && ln -sfn "runs/$(basename "$OUT")" "$COV_OUT/calib-latest"
exit "$rc"
