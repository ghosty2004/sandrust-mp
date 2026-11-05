#!/usr/bin/env bash
# M8b acceptance test: remaining behaviour (widescreen, camera RPCs + object attachment, world bounds, gravity, drunk,
# fighting style, shop name, wanted / stunt / money / velocity / controllable / interior, animations + the 207 field,
# SCM events, ClientCheck, spectating).
#  1. build (client DLL + launcher for Windows, samp-bot); isolated server copy with the e2e probe and
#     tools/trace/uifs.pwn (the TRACE-8 command filterscript, so the parity runs use the identical input)
#  2. inside a headless gamescope: start our client, spawn it (Shift), run tools/e2e/m8bscen.sh
#  3. after each step assert the control channel `behaviour` line (samp state + the GTA values the RPC changes), the
#     chat, and server-side callbacks (OnClientCheckResponse, OnPlayerInteriorChange, OnPlayerSpawn after spectating,
#     on-foot positions); screenshots target/m8b-*.png (isolated display only)
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default veh / 7820 / 47720); WINEPREFIX / GTA_DIR from the environment
# (lane veh: wine/prefix-shot + wine/gta-sa-shot). SAMP_RS_ROOT: checkout with wine/, server/, data/ (worktrees).
# M8B_ONLY=camera,bounds,… runs a subset. Exit status 0 = pass. Usage: tools/e2e/m8b.sh
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
source "$ROOT/tools/e2e/m8bscen.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
SHOT="${M8B_SHOT:-m8b}"
OUT="$ROOT/target/$SHOT"
say() { printf '[%s] %s\n' "$SHOT" "$*"; }
# The TRACE-8 filterscript (tools/trace/uifs.pwn with the remaining-behaviour commands).
UIFS="$ROOT/tools/trace/uifs.pwn"
grep -q '"specpl"' "$UIFS" 2>/dev/null || UIFS="$SAMP_RS_ROOT/tools/trace/uifs.pwn"
BOT_BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/samp-bot"

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

m8b_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$UIFS" -iqawno/include -ofilterscripts/m8bfs.amx -d0 \
        > /dev/null ) || { say "uifs.pwn ($UIFS) does not compile"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m8bfs"]
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
    SEQ=0
    LAST_INPUT=$SECONDS
    fs() {
        # GTA's idle camera hides the HUD after ~90 s without pad input (see m6.sh): a Space tap every 20 s.
        if (( SECONDS - LAST_INPUT >= 20 )); then
            e2e_key space
            LAST_INPUT=$SECONDS
        fi
        # The sequence number comes from the command file (fs also runs in command substitutions).
        SEQ=$(( $(wc -l < "$E2E_SERVER/scriptfiles/uicmd.txt" 2>/dev/null || echo 0) + 1 ))
        printf '%d %s\n' "$SEQ" "$1" >> "$E2E_SERVER/scriptfiles/uicmd.txt"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "uifs command $SEQ ($1) not run"
        grep -E "\[ui-fs\] $SEQ " "$E2E_LOG" | grep -v " done " || true
    }
    mark() { :; }
    shot() { e2e_shot "$ROOT/target/$SHOT-$1.png" || say "warning: screenshot $1 failed"; }
    keydown() { e2e_keydown "$1"; LAST_INPUT=$SECONDS; }
    keyup() { e2e_keyup "$1"; }
    bot() {
        "$BOT_BIN" "127.0.0.1:$E2E_PORT" --name "$1" --seconds 600 --auth-table "$SAMP_RS_ROOT/data/auth_table.txt" \
            > "$OUT/bot-$1.out" 2>&1 &
        BOT_PID=$!
        e2e_wait_log "\[ui-fs\] connect [0-9]+ $1" 20 || fail "bot $1 did not join"
    }
    beh() { e2e_ctl behaviour; }
    field() { sed -nE "s/.* $1=([^ ]+).*/\1/p" <<< "$2"; }
    near() { awk -v a="$1" -v b="$2" -v t="${3:-0.05}" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= t) }'; }
    xyz() { IFS=, read -r X Y Z <<< "$1"; }
    expect() { # expect KEY VALUE LINE
        [[ "$(field "$1" "$3")" == "$2" ]] || fail "$STEP: $1=$(field "$1" "$3"), want $2: $3"
    }
    check() {
        STEP="$1"
        local l; l="$(beh)"
        [[ -n "$l" ]] || fail "$STEP: no behaviour line"
        case "$1" in
            wide_on)
                expect widescreen 1 "$l"
                ctl_multi chat | grep -q "Widescreen = 1" || fail "no 'Widescreen = 1' chat line: $(ctl_multi chat | tail -n 3)" ;;
            wide_off) expect widescreen 0 "$l" ;;
            camlook) expect cam_mode 15 "$l"; xyz "$(field cam "$l")"; near "$X" 1700 0.1 && near "$Z" 25 0.1 || fail "camera not at the fixed position: $l" ;;
            cambehind) expect cam_mode 4 "$l"; expect attached - "$l" ;;
            ipos) xyz "$(field cam "$l")"; near "$X" 1720 0.5 && near "$Y" -2625 0.5 || fail "interpolated camera not at its end: $l" ;;
            camobj) expect attached "$obj" "$l"; xyz "$(field cam "$l")"; near "$X" 1700 0.2 && near "$Y" -2590 0.2 || fail "camera not at the object: $l" ;;
            camobj_moved) xyz "$(field cam "$l")"; near "$Z" 23.9 0.3 || fail "camera did not follow the moved object: $l" ;;
            cambehind3) expect attached - "$l"; expect cam_mode 4 "$l" ;;
            bounds_walk)
                xyz "$(field pos "$l")"; awk -v x="$X" 'BEGIN { exit !(x < 1721.5) }' || fail "walked through maxX 1720: $l"
                say "bounds walk: $l" ;;
            bounds_out) xyz "$(field pos "$l")"; say "standing outside maxX 1690: pos $X,$Y,$Z" ;;
            bounds_interior) expect area 1 "$l" ;;
            bounds_reset) [[ "$(field bounds "$l")" == "20000,-20000,20000,-20000" ]] || fail "bounds not reset: $l" ;;
            grav_player) near "$(field gravity "$l")" 0.002 0.00001 || fail "gravity: $l" ;;
            grav_reset) near "$(field gravity "$l")" 0.008 0.00001 || fail "gravity: $l" ;;
            drunk_5000)
                d="$(field drunk "$l")"; (( d > 4800 && d < 5000 )) || fail "drunk level $d after 2 s, want 4800..5000: $l" ;;
            drunk_2100) d="$(field drunk "$l")"; (( d <= 2000 && d >= 1900 )) || fail "drunk level $d after 3 s from 2100: $l" ;;
            shop) [[ "$(field shop "$l")" != 0x0 ]] || fail "shop name pointer not set: $l" ;;
            wanted3) expect wanted 3 "$l" ;;
            stunt1) expect stunt 1 "$l" ;;
            money) (( $(field money "$l") >= 500 )) || fail "money: $l" ;;
            interior1)
                expect area 1 "$l"
                e2e_wait_log "\[ui-fs\] interior-change [0-9]+ 0->1" 5 || fail "server saw no RPC 118 (interior 1)" ;;
            anim_walk)
                expect anim 0x170404ec "$l"
                say "anim field while WALK_DRUNK runs: $(field anim "$l")" ;;
            anim_dance) expect anim 0x1104019b "$l" ;;
            checks)
                e2e_wait_log "\[ui-fs\] check-response [0-9]+ type 0x2 " 3 || fail "no ClientCheck reply for type 2"
                e2e_wait_log "\[ui-fs\] check-response [0-9]+ type 0x48 " 3 || fail "no ClientCheck reply for type 0x48"
                grep -E "\[ui-fs\] check-response" "$E2E_LOG" | grep -qE "type 0x1 " && fail "type 1 must not be answered"
                say "checks: $(grep -E "\[ui-fs\] check-response" "$E2E_LOG" | tr '\n' '|')" ;;
            spec_on)
                expect spectate 1 "$l"; expect applied 1 "$l"; expect cam_mode 15 "$l"
                xyz "$(field cam "$l")"; local cz="$Z"; xyz "$(field pos "$l")"
                near "$Z" "$(awk -v c="$cz" 'BEGIN{print c+20}')" 1.5 || fail "ped not parked above the camera: $l" ;;
            spec_pl) [[ "$(field target "$l")" == player:* ]] || fail "no player target: $l"; expect applied 1 "$l"; expect cam_mode 4 "$l" ;;
            spec_veh) [[ "$(field target "$l")" == vehicle:* ]] || fail "no vehicle target: $l"; expect cam_mode 3 "$l" ;;
            spec_off)
                expect spectate 0 "$l"; expect spawned 1 "$l"
                [[ "$(grep -cE "\[ui-fs\] spawn $PID_$" "$E2E_LOG")" -ge 2 ]] || fail "no respawn after leaving spectator mode" ;;
        esac
        say "$STEP ok"
    }

    SKIP_BUILD=1 timeout --kill-after=10 1200 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
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
    say "spawned as id $PID_"
    sleep 2
    m8b_scenario
    say "behaviour: $(beh)"
    say "client: $(e2e_ctl net)"
    [[ -n "${BOT_PID:-}" ]] && kill "$BOT_PID" 2>/dev/null
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
rm -f "$CLIENT_LOG" "$ROOT"/target/"$SHOT"-*.png
m8b_server_start || exit 1
say "server up on port $E2E_PORT (lane $E2E_TAG, prefix $WINEPREFIX, filterscript $UIFS)"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
