#!/usr/bin/env bash
# M2 acceptance test: the samp-rs client joins a server from inside the game,
# selects a class, spawns and walks; the server must see it ONFOOT and moving.
#  1. build; isolated server copy with the e2e probe filterscript (lane "main",
#     port 7777, control port 47700)
#  2. run the client inside a headless gamescope (nothing on the user's screen)
#  3. wait for "[e2e] connect … name=RustClient" and "[e2e] class", press Shift
#     after the 2 s class-selection guard, wait for "[e2e] spawn"
#  4. hold W for ~3 s (if the random spawn faces a wall, also S); assert from
#     the probe log: state 1 (ONFOOT) and the
#     server-side position moved >= 1 m
#  5. screenshots target/m2-{class,spawn,walk}.png, `net` reply, quit
# Exit status 0 = pass. Usage: tools/e2e/m2.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUT="$ROOT/target/m2"
say() { printf '[m2] %s\n' "$*"; }

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; exit 1; }
    # Latest probe state line for our player: "st pos_x pos_y pos_z".
    probe_state() {
        grep -E "\[e2e\] state id=$PID_ " "$E2E_LOG" | tail -n 1 |
            sed -E 's/.* st=([0-9]+) pos=([-0-9.]+),([-0-9.]+),([-0-9.]+).*/\1 \2 \3 \4/'
    }
    SKIP_BUILD=1 timeout --kill-after=10 300 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    say "class selection reached: $(grep -E "\[e2e\] class id=$PID_ " "$E2E_LOG" | tail -n 1 | sed 's/.*\[e2e\]/[e2e]/')"
    sleep 3   # past the 2000 ms guard
    e2e_shot "$ROOT/target/m2-class.png" || say "warning: class screenshot failed"
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    say "spawned"
    sleep 3
    e2e_shot "$ROOT/target/m2-spawn.png" || say "warning: spawn screenshot failed"
    read -r st0 x0 y0 z0 <<< "$(probe_state)"
    say "before walking: st=$st0 pos=$x0,$y0,$z0"
    dist_from_start() {
        awk -v a="$x0" -v b="$y0" -v c="$z0" -v d="$1" -v e="$2" -v f="$3" \
            'BEGIN { printf "%.2f", sqrt((d-a)^2 + (e-b)^2 + (f-c)^2) }'
    }
    walk() {   # walk KEY: hold it ~3 s, then let the probe log two more lines
        e2e_keydown "$1"; sleep 3; e2e_keyup "$1"; sleep 2.5
        read -r st1 x1 y1 z1 <<< "$(probe_state)"
        dist="$(dist_from_start "$x1" "$y1" "$z1")"
        say "after holding $1: st=$st1 pos=$x1,$y1,$z1 (server-side distance $dist m)"
    }
    walk w
    # The gamemode's random spawn point can face a wall; then walk back the other way.
    if ! awk -v d="$dist" 'BEGIN { exit !(d >= 1.5) }'; then walk s; fi
    e2e_shot "$ROOT/target/m2-walk.png" || say "warning: walk screenshot failed"
    say "client: $(e2e_ctl net)"
    [[ "$st1" == 1 ]] || fail "player state $st1, want 1 (ONFOOT)"
    awk -v d="$dist" 'BEGIN { exit !(d >= 1.0) }' || fail "moved only $dist m (want >= 1 m)"
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
    e2e_cleanup
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null || true
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

say "building (i686-pc-windows-gnu, release)"
(cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$ROOT"/target/m2-*.png
e2e_server_start || exit 1
say "server up on port $E2E_PORT"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
