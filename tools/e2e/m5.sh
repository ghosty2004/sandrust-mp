#!/usr/bin/env bash
# M5 acceptance test: vehicles.
#  1. build (client DLL + launcher for Windows, samp-bot + drive_bot for the host); isolated server copy with the
#     e2e probe and the m5helper filterscript (tools/e2e/probe/m5helper.pwn)
#  2. inside a headless gamescope: start our client, spawn it (Shift); m5helper puts it on the LS airport
#     tarmac next to a Sultan (car A)
#  3. remote driver: drive_bot "DriveBot" joins, m5helper creates car B (in view) and puts the bot in it; the bot drives
#     a 10 m circle with driver sync (200). Control channel: `remote` shows the bot seated (vehicle = car B,
#     seat 0), `vehicles` shows car B with the bot as driver, its in-game position tracks the server-side one
#     (≤ ${M5_TRACK_M:-6} m in 3 of 4 samples) and it moves (≥ 5 m in 5 s). Then the bot is stopped (by PID).
#  4. local driver: press F next to car A → the server sees an enter request and our state DRIVER (2); hold W
#     4 s → the server-side car moved ≥ 10 m; press F → exit request, state ONFOOT (1)
#  5. screenshots target/m5-{remote,enter,drive,exit}.png (isolated display only)
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default main / 7777 / 47700); WINEPREFIX / GTA_DIR from the
# environment (tools/wine-env.sh defaults). Exit status 0 = pass. Usage: tools/e2e/m5.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
BOT=DriveBot
TRACK_M="${M5_TRACK_M:-6}"
CLIENT_LOG="$GTA_DIR/samp-rs.log"
SHOT="${M5_SHOT:-m5}"
OUT="$ROOT/target/$SHOT"
say() { printf '[%s] %s\n' "$SHOT" "$*"; }

# Multi-line control replies (`remote`, `vehicles`): header line with count=N, then N lines.
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

m5_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/m5helper.pwn" \
        -iqawno/include -ofilterscripts/m5helper.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m5helper"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[m5\] helper loaded' 5 || { say "m5helper filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    BOT_PID=""
    fail() { say "FAIL: $*"; [[ -n "$BOT_PID" ]] && kill "$BOT_PID" 2>/dev/null; exit 1; }
    probe_state() {   # probe_state ID → "st x y z"
        grep -E "\[e2e\] state id=$1 " "$E2E_LOG" | tail -n 1 |
            sed -E 's/.* st=([0-9]+) pos=([-0-9.]+),([-0-9.]+),([-0-9.]+).*/\1 \2 \3 \4/'
    }
    veh_pos() {   # veh_pos ID → "x y z driver" (server side)
        grep -E "\[m5\] veh id=$1 " "$E2E_LOG" | tail -n 1 |
            sed -E 's/.* pos=([-0-9.]+),([-0-9.]+),([-0-9.]+) driver=([-0-9]+).*/\1 \2 \3 \4/'
    }
    field() { sed -nE "s/.* $1=([^ ]+).*/\1/p" <<< "$2"; }
    dist() { awk -v a="$1" -v b="$2" -v c="$3" -v d="$4" -v e="$5" -v f="$6" \
        'BEGIN { printf "%.2f", sqrt((d-a)^2 + (e-b)^2 + (f-c)^2) }'; }
    veh_line() { ctl_multi vehicles | grep -E "^vehicle id=$1 " | head -n 1; }
    bot_line() { ctl_multi remote | grep -E "^remote id=[0-9]+ name=$BOT " | head -n 1; }
    press_f() { e2e_keydown f; sleep 0.4; e2e_keyup f; }

    SKIP_BUILD=1 timeout --kill-after=10 600 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    e2e_wait_log "\[m5\] local ready id=$PID_ " 10 || fail "m5helper did not place the player"
    CAR_A="$(grep -E "\[m5\] local ready id=$PID_ " "$E2E_LOG" | tail -n 1 | sed -E 's/.*carA=([0-9]+).*/\1/')"
    say "spawned; car A = vehicle $CAR_A"
    line=""
    for _ in $(seq 1 40); do line="$(veh_line "$CAR_A")"; [[ -n "$line" && "$(field ptr "$line")" != 0x0 ]] && break; line=""; sleep 0.5; done
    [[ -n "$line" ]] || fail "car A never streamed in (vehicles: $(ctl_multi vehicles | tr '\n' '|'))"
    say "car A in game: $line"
    sleep 2
    # M6-P: number plates (rows 59/60): the default texture and car A's own one exist after the stream-in.
    wh="$(e2e_ctl world | head -n 1)"
    [[ "$wh" =~ plates\ raster=0x[0-9a-f]+\ default=0x([0-9a-f]+)\ vehicles=([0-9]+) ]] \
        && [[ "${BASH_REMATCH[1]}" != 0 && "${BASH_REMATCH[2]}" -ge 1 ]] || fail "no plate textures: $wh"
    say "plates: ${wh##*plates }"

    # ---- (a) remote driver -------------------------------------------------------------------------
    say "starting $BOT"
    ( cd "$ROOT" && exec target/debug/examples/drive_bot "127.0.0.1:$E2E_PORT" --name "$BOT" --seconds 60 ) \
        > "$OUT/bot.out" 2>&1 &
    BOT_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$BOT" 20 || fail "bot did not connect"
    BID="$(grep -E "\[e2e\] connect id=[0-9]+ name=$BOT" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[m5\] bot in id=$BID " 30 || fail "bot was not put into car B"
    CAR_B="$(grep -E "\[m5\] bot in id=$BID " "$E2E_LOG" | tail -n 1 | sed -E 's/.*carB=([0-9]+).*/\1/')"
    say "bot id $BID drives car B = vehicle $CAR_B"
    line=""
    for _ in $(seq 1 40); do
        line="$(bot_line)"
        [[ "$(field vehicle "$line")" == "$CAR_B" && "$(field seat "$line")" == 0 ]] && break
        line=""; sleep 0.5
    done
    [[ -n "$line" ]] || fail "bot never seated in car B (remote: $(ctl_multi remote | tr '\n' '|'))"
    say "remote: $line"
    vl="$(veh_line "$CAR_B")"
    say "car B: $vl"
    [[ "$(field driver "$vl")" == "$BID" ]] || fail "vehicles does not show the bot as car B's driver"
    sleep 2
    good=0
    for i in 1 2 3 4; do
        vl="$(veh_line "$CAR_B")"; IFS=, read -r gx gy gz <<< "$(field pos "$vl")"
        read -r sx sy sz _ <<< "$(veh_pos "$CAR_B")"
        d="$(dist "$gx" "$gy" "$gz" "$sx" "$sy" "$sz")"
        say "track $i: game $gx,$gy,$gz server $sx,$sy,$sz -> $d m (driver $(field driver "$vl"))"
        awk -v d="$d" -v t="$TRACK_M" 'BEGIN { exit !(d <= t) }' && good=$((good + 1))
        sleep 1
    done
    (( good >= 3 )) || fail "car B off by more than $TRACK_M m in $((4 - good)) of 4 samples"
    path=0; px=""
    for _ in $(seq 0 10); do
        IFS=, read -r x y z <<< "$(field pos "$(veh_line "$CAR_B")")"
        if [[ -n "$px" && -n "$x" && "$x" != - ]]; then
            path="$(awk -v p="$path" -v d="$(dist "$px" "$py" "$pz" "$x" "$y" "$z")" 'BEGIN { printf "%.2f", p + d }')"
        fi
        [[ -n "$x" && "$x" != - ]] && { px=$x; py=$y; pz=$z; }
        sleep 0.5
    done
    say "car B path over 5 s: $path m"
    awk -v p="$path" 'BEGIN { exit !(p >= 5.0) }' || fail "car B moved only $path m in 5 s"
    e2e_shot "$ROOT/target/$SHOT-remote.png" || say "warning: remote screenshot failed"
    say "remote: $(bot_line)"
    kill "$BOT_PID" 2>/dev/null; wait "$BOT_PID" 2>/dev/null; BOT_PID=""
    say "bot stopped"
    sleep 3

    # ---- (b) local driver ----------------------------------------------------------------------------
    read -r ax0 ay0 az0 _ <<< "$(veh_pos "$CAR_A")"
    say "car A server-side before: $ax0,$ay0,$az0; player: $(probe_state "$PID_")"
    entered=0
    for attempt in 1 2 3; do
        press_f
        if e2e_wait_log "\[m5\] state $PID_ [0-9]+->2 veh $CAR_A" 10; then entered=1; break; fi
        say "not seated after F press $attempt ($(grep -cE "\[m5\] enter-request $PID_ " "$E2E_LOG") enter requests)"
    done
    (( entered )) || fail "server never saw us as driver of car A"
    grep -E "\[m5\] enter-request $PID_ veh $CAR_A passenger 0" "$E2E_LOG" > /dev/null || fail "no enter request (RPC 26) for car A"
    say "seated: $(veh_line "$CAR_A")"
    e2e_shot "$ROOT/target/$SHOT-enter.png" || say "warning: enter screenshot failed"
    e2e_keydown w; sleep 4; e2e_keyup w
    sleep 1.5
    e2e_shot "$ROOT/target/$SHOT-drive.png" || say "warning: drive screenshot failed"
    read -r st _ <<< "$(probe_state "$PID_")"
    read -r ax1 ay1 az1 drv <<< "$(veh_pos "$CAR_A")"
    moved="$(dist "$ax0" "$ay0" "$az0" "$ax1" "$ay1" "$az1")"
    say "after W: state $st, car A server-side $ax1,$ay1,$az1 driver $drv (moved $moved m); game: $(veh_line "$CAR_A")"
    [[ "$st" == 2 ]] || fail "player state $st while driving, want 2 (DRIVER)"
    [[ "$drv" == "$PID_" ]] || fail "server does not see us as car A's driver ($drv)"
    awk -v d="$moved" 'BEGIN { exit !(d >= 10.0) }' || fail "car A moved only $moved m server-side (want >= 10 m)"
    sleep 1
    exited=0
    for attempt in 1 2 3; do
        press_f
        if e2e_wait_log "\[m5\] state $PID_ 2->1 " 10; then exited=1; break; fi
        say "still driving after F press $attempt"
    done
    (( exited )) || fail "server never saw us on foot again"
    grep -E "\[m5\] exit-request $PID_ veh $CAR_A" "$E2E_LOG" > /dev/null || fail "no exit request (RPC 154)"
    sleep 2
    read -r st _ <<< "$(probe_state "$PID_")"
    [[ "$st" == 1 ]] || fail "player state $st after exit, want 1 (ONFOOT)"
    e2e_shot "$ROOT/target/$SHOT-exit.png" || say "warning: exit screenshot failed"
    say "vehicles: $(ctl_multi vehicles | head -n 1)"
    say "client: $(e2e_ctl net)"
    grep -E "vehicles: (enter|exit|stream)" "$CLIENT_LOG" | tail -n 6 | sed 's/^/    /'
    bye="$(e2e_ctl quit)"
    say "quit: $bye"
    for _ in $(seq 1 20); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 1; done
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
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

say "building (i686-pc-windows-gnu release + samp-bot + drive_bot)"
{ (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
  (cd "$ROOT" && cargo build -p samp-bot --examples); } > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "$ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$ROOT"/target/"$SHOT"-*.png
m5_server_start || exit 1
say "server up on port $E2E_PORT (lane $E2E_TAG, prefix $WINEPREFIX)"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
