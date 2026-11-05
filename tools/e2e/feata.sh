#!/usr/bin/env bash
# FEAT-A acceptance test: the network-visible features of docs/phase3/feature-sweep.md on OUR client.
#  1. build (client DLL + launcher for Windows, samp-bot); isolated server copy with the e2e probe, tools/trace/uifs.pwn
#     and the neutral gamemode tools/trace/tracegm.pwn (the same input as the parity runs, tools/e2e/featparity.sh)
#  2. inside a headless gamescope: start our client, spawn it (Shift), run tools/e2e/featscen.sh
#  3. assert after each step:
#     F4      one "Returning to class selection after next death" line for two presses; after `kill` the client
#             requests a class (OnPlayerRequestClass, RPC 128) instead of spawning (RPC 52); the pending second press
#             re-arms the flag once it is consumed (second line), like the original
#     170/168 GetPlayerCameraTargetVehicle / Object / Actor = the entity in front of the camera; the player target
#             from our RPC 168 (open.mp's GetPlayerCameraTargetPlayer reads another field)
#     23      OnPlayerClickPlayer(local, bot, 0) after a double click on the bot's scoreboard row
#     119     OnPlayerClickMap at the waypoint placed by a right click on the pause-menu map, z = ground + 2
#     112     the crime report runs (on foot and with the suspect in a car)
#     jack    a bot entering the car the local player drives ejects the local player (on foot afterwards)
#     96      OnEnterExitModShop(1), OnVehiclePaintjob + OnVehicleRespray, OnEnterExitModShop(0) at Wheel Arch Angels
#     48      SetPlayerVirtualWorld: open.mp sends nothing to 0.3.7 clients (R4 ignores it anyway); step kept as input
#     screenshots target/feata-*.png (isolated display only)
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default veh / 7820 / 47720); WINEPREFIX / GTA_DIR from the environment
# (lane veh: wine/prefix-shot + wine/gta-sa-shot). SAMP_RS_ROOT: checkout with wine/, server/, data/ (worktrees).
# FEAT_ONLY=f4,camtarget,… runs a subset (featscen.sh). Exit status 0 = pass. Usage: tools/e2e/feata.sh
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
source "$ROOT/tools/e2e/featscen.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUT="$ROOT/target/feata"
say() { printf '[feata] %s\n' "$*"; }
BOT_BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/samp-bot"
export FEAT_LOG="$E2E_LOG"

ctl_multi() {
    python3 -I - "$E2E_CTL_PORT" "$1" <<'PY'
import re, socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.settimeout(3)
    s.sendall((sys.argv[2] + "\n").encode())
    f = s.makefile()
    head = f.readline().strip()
    print(head)
    m = re.search(r"count=(\d+)", head)
    for _ in range(int(m.group(1)) if m else 0):
        print(f.readline().strip())
except OSError:
    pass
PY
}

feat_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/uifs.pwn" -iqawno/include \
        -ofilterscripts/featfs.amx -d0 > /dev/null &&
      LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/tracegm.pwn" -iqawno/include \
        -ogamemodes/tracegm.amx -d0 > /dev/null ) || { say "uifs.pwn / tracegm.pwn do not compile"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/featfs"]
d["pawn"]["main_scripts"] = ["tracegm 1"]
d["max_bots"] = 2
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$E2E_SERVER/scriptfiles"; rm -f "$E2E_SERVER/scriptfiles/uicmd.txt"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[ui-fs\] loaded' 5 || { say "uifs filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; e2e_shot "$OUT/fail.png" > /dev/null 2>&1; exit 1; }
    LAST_INPUT=$SECONDS
    fs() {
        if (( SECONDS - LAST_INPUT >= 20 )); then e2e_key space; LAST_INPUT=$SECONDS; fi
        local seq
        seq=$(( $(wc -l < "$E2E_SERVER/scriptfiles/uicmd.txt" 2>/dev/null || echo 0) + 1 ))
        printf '%d %s\n' "$seq" "$1" >> "$E2E_SERVER/scriptfiles/uicmd.txt"
        e2e_wait_log "\[ui-fs\] $seq done" 10 || fail "uifs command $seq ($1) not run"
        grep -E "\[ui-fs\] $seq " "$E2E_LOG" | grep -v " done " || true
    }
    mark() { :; }
    shot() { e2e_shot "$ROOT/target/feata-$1.png" || say "warning: screenshot $1 failed"; }
    key() { e2e_key "$@"; LAST_INPUT=$SECONDS; }
    keydown() { e2e_keydown "$1"; LAST_INPUT=$SECONDS; }
    keyup() { e2e_keyup "$1"; }
    mouse() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool "$@"; LAST_INPUT=$SECONDS; }
    chat() { e2e_key t; sleep 0.6; DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "$1"; sleep 0.3; e2e_key Return; }
    bot() {
        local name="$1"; shift
        "$BOT_BIN" "127.0.0.1:$E2E_PORT" --name "$name" --auth-table "$SAMP_RS_ROOT/data/auth_table.txt" "$@" \
            > "$OUT/bot-$name.out" 2>&1 &
        BOT_PIDS+=("$!")
        e2e_wait_log "\[ui-fs\] connect [0-9]+ $name" 20 || fail "bot $name did not join"
    }
    waitlog() { e2e_wait_log "$1" "$2"; }
    d3dcap() { :; }
    BOT_PIDS=()
    lines() { ctl_multi "chat 50" | grep -cF "$F4_TEXT"; }
    F4_TEXT="Returning to class selection after next death"
    botid() { grep -E "\[ui-fs\] connect [0-9]+ $1\b" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect ([0-9]+) .*/\1/'; }
    clog() { grep -aE "$1" "$CLIENT_LOG" 2>/dev/null | tail -n 1; }
    check() {
        STEP="$1"
        case "$1" in
            f4_line) (( $(lines) == 1 )) || fail "F4 twice: $(lines) chat lines, want 1" ;;
            f4_class)
                (( $(grep -cE "\[e2e\] class id=$PID_ " "$E2E_LOG") >= 2 )) || fail "no class request after the death"
                (( $(feat_spawns) == SPAWNS0 )) || fail "spawned instead of entering class selection" ;;
            f4_rearm) (( $(lines) == 2 )) || fail "after the class selection: $(lines) chat lines, want 2 (re-armed flag)" ;;
            camtgt_vehicle) grep -q "vehicle $FEAT_VEH " <<< "$CAMTGT" || fail "camera target vehicle: $CAMTGT (want $FEAT_VEH)" ;;
            camtgt_object) grep -q "object $FEAT_OBJ " <<< "$CAMTGT" || fail "camera target object: $CAMTGT (want $FEAT_OBJ)" ;;
            camtgt_actor) grep -qE "actor $FEAT_ACTOR$" <<< "$CAMTGT" || fail "camera target actor: $CAMTGT (want $FEAT_ACTOR)" ;;
            camtgt_player)
                local b; b="$(botid TgtBot)"
                [[ -n "$(clog "RPC 168 object 65535 vehicle 65535 player $b actor 65535")" ]] \
                    || fail "no RPC 168 with player $b: $(clog 'RPC 168')" ;;
            click_player)
                e2e_wait_log "\[ui-fs\] click-player $PID_ $(botid TgtBot) source 0" 5 || fail "no OnPlayerClickPlayer" ;;
            click_map)
                e2e_wait_log "\[ui-fs\] click-map $PID_ " 5 || fail "no OnPlayerClickMap"
                say "map click: $(grep -E 'click-map' "$E2E_LOG" | tail -n 1)" ;;
            crime_foot) [[ -n "$(clog 'RPC 112 crime 14 in_vehicle 0')" ]] || fail "crime report (on foot) not handled" ;;
            crime_vehicle) [[ -n "$(clog 'RPC 112 crime 14 in_vehicle 1 model 416')" ]] || fail "crime report (in vehicle) not handled" ;;
            jack)
                fs "where|L" | grep -qE " veh 0 seat -1 state 1 " || fail "local player still in the car: $(fs 'where|L')"
                ctl_multi vehicles | head -n 1 | grep -q " jacked=1" || fail "no Car Jacked: $(ctl_multi vehicles | head -n 1)" ;;
            modshop_enter) grep -qE "\[ui-fs\] modshop $PID_ enterexit 1" "$E2E_LOG" || fail "no OnEnterExitModShop(1)" ;;
            modshop_paint)
                e2e_wait_log "\[ui-fs\] paintjob $PID_ veh $FEAT_MOD_VEH " 5 || fail "no OnVehiclePaintjob"
                e2e_wait_log "\[ui-fs\] respray $PID_ veh $FEAT_MOD_VEH " 5 || fail "no OnVehicleRespray" ;;
            modshop_exit) e2e_wait_log "\[ui-fs\] modshop $PID_ enterexit 0" 5 || fail "no OnEnterExitModShop(0)" ;;
            # open.mp does not send RPC 48 to 0.3.7 clients (player_impl.hpp setVirtualWorld); R4 ignores it anyway.
            vw) [[ -z "$(clog 'RPC 48 virtual world')" ]] || say "RPC 48 arrived: $(clog 'RPC 48')" ;;
        esac
        say "$STEP ok"
    }

    SKIP_BUILD=1 timeout --kill-after=10 1500 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
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
    SPAWNS0="$(feat_spawns)"
    say "spawned as id $PID_"
    sleep 2
    feat_scenario
    say "client: $(e2e_ctl net)"
    for p in "${BOT_PIDS[@]}"; do kill "$p" 2>/dev/null; done
    e2e_ctl quit > /dev/null
    say "all checks passed"
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUT"
rc=1
finish() {
    e2e_cleanup
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null || true
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    say "building (i686-pc-windows-gnu release + samp-bot)"
    { (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) \
        && (cd "$ROOT" && cargo build -p samp-bot); } > "$OUT/build.out" 2>&1 \
        || { cat "$OUT/build.out"; say "build failed"; exit 1; }
fi
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$ROOT"/target/feata-*.png
feat_server_start || exit 1
say "server up on port $E2E_PORT (lane $E2E_TAG, prefix $WINEPREFIX)"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
