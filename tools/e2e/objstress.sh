#!/usr/bin/env bash
# Object-map stress regression test (CRASH-1): tools/e2e/probe/objmap.pwn builds an island of objects far from
# the mainland (global + player objects, custom models, materials, material text, MoveObject right after
# creation), teleports the player there, churns create/destroy/move like a streamer and walks the player
# across the island, twice. The client must survive the whole scenario: the server keeps receiving its
# updates ("[objmap] alive N updates >0"), the game process stays up, no samp-rs-crash.txt is written.
#  1. build (client DLL + launcher for Windows); isolated server copy with the e2e probe and objmap.pwn
#  2. inside a headless gamescope: start our client, spawn it (Shift), wait for "[objmap] done"
#  3. checks: game alive, updates in every second after the teleport, control channel `world` answers,
#     no crash report, no failed CreateVertexBuffer; a forced device loss (`devreset` on the control channel)
#     in the middle of the hops must end in successful Resets
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default veh / 7820 / 47720); WINEPREFIX / GTA_DIR from the environment
# (lane veh: wine/prefix-shot + wine/gta-sa-shot). SAMP_RS_ROOT: checkout with wine/, server/, data/.
# OBJSTRESS_DEFINES="OBJMAP_GLOBAL=400 …" passes pawncc defines. Exit status 0 = pass. Usage: tools/e2e/objstress.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
export E2E_TAG="${E2E_TAG:-veh}" E2E_PORT="${E2E_PORT:-7820}" E2E_CTL_PORT="${E2E_CTL_PORT:-47720}"
export WINEPREFIX="${WINEPREFIX:-$SAMP_RS_ROOT/wine/prefix-shot}" GTA_DIR="${GTA_DIR:-$SAMP_RS_ROOT/wine/gta-sa-shot}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
CRASH_FILE="$GTA_DIR/samp-rs-crash.txt"
SHOT="${OBJSTRESS_SHOT:-objstress}"
OUT="$ROOT/target/$SHOT"
say() { printf '[%s] %s\n' "$SHOT" "$*"; }

objstress_server_start() {
    e2e_server_prepare
    local defs=()
    read -r -a defs <<< "${OBJSTRESS_DEFINES:-}"
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/objmap.pwn" "${defs[@]}" \
        -iqawno/include -ofilterscripts/objmap.amx -d0 > /dev/null ) || { say "objmap.pwn does not compile"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/objmap"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 9>&- ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[objmap\] loaded' 5 || { say "objmap filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; e2e_shot "$OUT/fail.png" > /dev/null 2>&1; exit 1; }
    SKIP_BUILD=1 timeout --kill-after=10 900 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3
    spawned=0
    for _ in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
    done
    (( spawned )) || fail "no spawn after Shift"
    say "spawned as id $PID_"
    e2e_wait_log "\[objmap\] phase 2 teleport" 30 || fail "no teleport"
    say "teleported; streaming burst sent"
    reset_done=0
    for ((t = 0; t < 240; t++)); do
        if [[ -e "$CRASH_FILE" ]] || ! kill -0 "$RUN_PID" 2>/dev/null \
            || grep -q "exited with code" "$GTA_DIR/samp-rs-launcher.log" 2>/dev/null; then
            say "game crashed or exited: $(grep -E 'exited with code' "$GTA_DIR/samp-rs-launcher.log" 2>/dev/null)"
            [[ -e "$CRASH_FILE" ]] && tr -d '\r' < "$CRASH_FILE" | sed -n '2p;/^Diagnostics/,/^Loaded Modules/p' | head -n 40 | sed 's/^/    /'
            grep -aE "Unhandled|page fault" "$OUT/run.out" | head -n 5 | sed 's/^/    /'
            fail "the client died during the object stress"
        fi
        # A device loss + Reset while the island (with material text render targets) is loaded.
        if (( !reset_done )) && grep -q "\[objmap\] hop 1 " "$E2E_LOG"; then
            say "device reset: $(e2e_ctl 'devreset 15')"; reset_done=1
        fi
        grep -q "\[objmap\] done" "$E2E_LOG" && break
        sleep 1
    done
    grep -q "\[objmap\] done" "$E2E_LOG" || fail "scenario did not finish"
    [[ -e "$CRASH_FILE" ]] && fail "crash report written: $(head -n 3 "$CRASH_FILE" | tr '\r\n' '  ')"
    # Every second after the teleport must show client updates (a hung or dead client sends none).
    tele="$(grep -nE "\[objmap\] phase 2 teleport" "$E2E_LOG" | head -n 1 | cut -d: -f1)"
    zero="$(tail -n +"$tele" "$E2E_LOG" | grep -cE "\[objmap\] alive [0-9]+ updates 0 ")"
    total="$(tail -n +"$tele" "$E2E_LOG" | grep -cE "\[objmap\] alive ")"
    (( zero <= 2 )) || fail "$zero of $total seconds after the teleport without client updates"
    h="$(e2e_ctl world)"; [[ -n "$h" ]] || fail "control channel does not answer"
    # RenderWare must never be asked for an impossible vertex buffer (a failed create leaves a dead record in
    # its dynamic-buffer list: the 0x7F5A3A crash), and every device Reset must succeed.
    grep -q "d3d: CreateVertexBuffer" "$CLIENT_LOG" && fail "failed CreateVertexBuffer: $(grep -m 1 'd3d: CreateVertexBuffer' "$CLIENT_LOG")"
    resets="$(grep -c 'ui: device Reset -> ' "$CLIENT_LOG")"
    (( resets > 0 )) || fail "the forced device loss caused no Reset"
    grep -q 'ui: device Reset -> 0x[1-9a-f]' "$CLIENT_LOG" && fail "a device Reset failed: $(grep -m 1 'ui: device Reset -> 0x[1-9a-f]' "$CLIENT_LOG")"
    say "device resets: $resets, all succeeded"
    say "world: $h"
    say "seconds without updates after the teleport: $zero of $total"
    say "client: $(e2e_ctl net)"
    e2e_ctl quit > /dev/null
    say "all checks passed"
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUT"
rc=1
finish() {
    e2e_headless_stop
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null || true
    cp -f "$CRASH_FILE" "$OUT/" 2>/dev/null || true
    cp -f "$GTA_DIR/samp-rs-launcher.log" "$OUT/" 2>/dev/null || true
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    say "building (i686-pc-windows-gnu release)"
    (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) > "$OUT/build.out" 2>&1 \
        || { cat "$OUT/build.out"; say "build failed"; exit 1; }
fi
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$CRASH_FILE" "$GTA_DIR/samp-rs-launcher.log"
objstress_server_start || exit 1
say "server up on port $E2E_PORT (lane $E2E_TAG, prefix $WINEPREFIX)"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
