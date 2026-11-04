#!/usr/bin/env bash
# M3 acceptance test: a remote player (samp-bot) appears inside the game as a
# GTA ped in its own Players slot and moves with its on-foot sync.
#  1. build (client DLL + launcher for Windows, samp-bot for the host); isolated
#     server copy with the e2e probe and the m3helper filterscript (lane "main",
#     port 7777, control port 47700)
#  2. inside a headless gamescope: start our client, spawn it (Shift, as m2.sh)
#  3. start samp-bot "RemoteBot"; m3helper puts it 5 m in front of our player
#  4. via the control channel `remote`: the bot is listed with a ped in slot
#     >= 2, its in-game position is within ${M3_TRACK_M:-4} m of the server-side
#     position (probe log), and its path over 5 s is >= 2 m
#  5. screenshots target/m3-remote-{1,2}.png
#  6. the bot leaves (end of its run, or kill by PID): the remote ped is removed
# M3_WALK=1 (used by goal.sh) adds, after the spawn, "hold W and the probe must see
# us ONFOOT >= 1 m away" (as m2.sh). M3_SHOT names screenshots/logs (default m3).
# Exit status 0 = pass. Usage: tools/e2e/m3.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
BOT=RemoteBot
BOT_SECONDS="${M3_BOT_SECONDS:-45}"
TRACK_M="${M3_TRACK_M:-4}"
CLIENT_LOG="$GTA_DIR/samp-rs.log"
# goal.sh reuses this script with M3_WALK=1 and its own output names.
SHOT="${M3_SHOT:-m3}"
OUT="$ROOT/target/$SHOT"
say() { printf '[%s] %s\n' "$SHOT" "$*"; }

# `remote` reply: header line + one line per remote player.
ctl_remote() {
    python3 -I - "$E2E_CTL_PORT" <<'PY'
import re, socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.settimeout(3)
    s.sendall(b"remote\n")
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

# Start the server with both filterscripts (e2e_server_start + m3helper).
m3_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/m3helper.pwn" \
        -iqawno/include -ofilterscripts/m3helper.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m3helper"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[m3\] helper loaded' 5 || { say "m3helper filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    BOT_PID=""
    fail() { say "FAIL: $*"; [[ -n "$BOT_PID" ]] && kill "$BOT_PID" 2>/dev/null; exit 1; }
    probe_state() {   # probe_state ID → "st x y z" (latest server-side state)
        grep -E "\[e2e\] state id=$1 " "$E2E_LOG" | tail -n 1 |
            sed -E 's/.* st=([0-9]+) pos=([-0-9.]+),([-0-9.]+),([-0-9.]+).*/\1 \2 \3 \4/'
    }
    probe_pos() {   # probe_pos ID → "x y z" (latest server-side position)
        grep -E "\[e2e\] state id=$1 " "$E2E_LOG" | tail -n 1 |
            sed -E 's/.* pos=([-0-9.]+),([-0-9.]+),([-0-9.]+).*/\1 \2 \3/'
    }
    bot_line() { ctl_remote | grep -E "^remote id=[0-9]+ name=$BOT " | head -n 1; }
    field() { sed -nE "s/.* $1=([^ ]+).*/\1/p" <<< "$2"; }
    dist() { awk -v a="$1" -v b="$2" -v c="$3" -v d="$4" -v e="$5" -v f="$6" \
        'BEGIN { printf "%.2f", sqrt((d-a)^2 + (e-b)^2 + (f-c)^2) }'; }

    SKIP_BUILD=1 timeout --kill-after=10 400 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3   # past the 2000 ms class-selection guard
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    say "spawned"
    sleep 2
    if [[ "${M3_WALK:-0}" == 1 ]]; then
        # goal.sh: the server must see our own movement first (as m2.sh).
        read -r st0 x0 y0 z0 <<< "$(probe_state "$PID_")"
        say "before walking: st=$st0 pos=$x0,$y0,$z0"
        walk() {   # walk KEY: hold it ~3 s, then let the probe log two more lines
            e2e_keydown "$1"; sleep 3; e2e_keyup "$1"; sleep 2.5
            read -r st1 x1 y1 z1 <<< "$(probe_state "$PID_")"
            wdist="$(dist "$x0" "$y0" "$z0" "$x1" "$y1" "$z1")"
            say "after holding $1: st=$st1 pos=$x1,$y1,$z1 (server-side distance $wdist m)"
        }
        walk w
        awk -v d="$wdist" 'BEGIN { exit !(d >= 1.5) }' || walk s
        [[ "$st1" == 1 ]] || fail "player state $st1, want 1 (ONFOOT)"
        awk -v d="$wdist" 'BEGIN { exit !(d >= 1.0) }' || fail "we moved only $wdist m server-side (want >= 1 m)"
        e2e_shot "$ROOT/target/$SHOT-walk.png" || say "warning: walk screenshot failed"
    fi
    say "starting $BOT for $BOT_SECONDS s"
    ( cd "$ROOT" && exec "${CARGO_TARGET_DIR:-target}/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name "$BOT" --seconds "$BOT_SECONDS" \
        --auth-table "${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt" ) \
        > "$OUT/bot.out" 2>&1 &
    BOT_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$BOT" 20 || fail "bot did not connect"
    BID="$(grep -E "\[e2e\] connect id=[0-9]+ name=$BOT" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[m3\] bring id=$BID " 30 || fail "bot was not brought next to the client"
    say "bot id $BID: $(grep -E "\[m3\] bring id=$BID " "$E2E_LOG" | tail -n 1 | sed 's/.*\[m3\]/[m3]/')"

    # 1. the bot appears with a ped in slot >= 2.
    line=""
    for _ in $(seq 1 40); do
        line="$(bot_line)"
        slot="$(field slot "$line")"; ped="$(field ped "$line")"
        [[ "$slot" =~ ^[0-9]+$ && "$slot" -ge 2 && -n "$ped" && "$ped" != 0x0 && "$(field pos "$line")" != - ]] && break
        line=""; sleep 0.5
    done
    [[ -n "$line" ]] || fail "bot never got a ped (last reply: $(ctl_remote | tr '\n' '|'))"
    say "remote: $line"
    sleep 2

    # 2. tracking: in-game position vs the server's (probe, once per second).
    good=0
    for i in 1 2 3 4; do
        line="$(bot_line)"; IFS=, read -r rx ry rz <<< "$(field pos "$line")"
        read -r sx sy sz <<< "$(probe_pos "$BID")"
        d="$(dist "$rx" "$ry" "$rz" "$sx" "$sy" "$sz")"
        say "track $i: game $rx,$ry,$rz server $sx,$sy,$sz -> $d m (slot $(field slot "$line"), age $(field sync_age_ms "$line") ms, speed $(field speed "$line"), teleports $(field teleports "$line"))"
        awk -v d="$d" -v t="$TRACK_M" 'BEGIN { exit !(d <= t) }' && good=$((good + 1))
        sleep 1
    done
    (( good >= 3 )) || fail "in-game position off by more than $TRACK_M m in $((4 - good)) of 4 samples"
    e2e_shot "$ROOT/target/$SHOT-remote-1.png" || say "warning: screenshot 1 failed"

    # 3. movement: path length over 5 s.
    path=0; px=""
    for _ in $(seq 0 10); do
        IFS=, read -r x y z <<< "$(field pos "$(bot_line)")"
        if [[ -n "$px" && -n "$x" && "$x" != - ]]; then
            path="$(awk -v p="$path" -v d="$(dist "$px" "$py" "$pz" "$x" "$y" "$z")" 'BEGIN { printf "%.2f", p + d }')"
        fi
        [[ -n "$x" && "$x" != - ]] && { px=$x; py=$y; pz=$z; }
        sleep 0.5
    done
    say "path over 5 s: $path m"
    awk -v p="$path" 'BEGIN { exit !(p >= 2.0) }' || fail "remote ped moved only $path m in 5 s"
    e2e_shot "$ROOT/target/$SHOT-remote-2.png" || say "warning: screenshot 2 failed"
    say "client: $(e2e_ctl net)"
    say "state: $(e2e_ctl state)"

    # 4. the bot leaves: its ped must go (PlayerQuit 138 / stream-out 163).
    for _ in $(seq 1 "$BOT_SECONDS"); do kill -0 "$BOT_PID" 2>/dev/null || break; sleep 1; done
    if kill -0 "$BOT_PID" 2>/dev/null; then say "bot still running; killing PID $BOT_PID"; kill "$BOT_PID"; fi
    wait "$BOT_PID" 2>/dev/null; BOT_PID=""
    removed=0
    for _ in $(seq 1 30); do
        line="$(bot_line)"
        if [[ -z "$line" || "$(field ped "$line")" == 0x0 ]]; then removed=1; break; fi
        sleep 0.5
    done
    (( removed )) || fail "remote ped still present after the bot left: $line"
    say "bot gone: $(ctl_remote | head -n 1)"
    grep -E "players: (quit|stream-out|id=$BID .*deleted)" "$CLIENT_LOG" | tail -n 3 | sed 's/^/    /'

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

say "building (i686-pc-windows-gnu release + samp-bot)"
{ (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
  (cd "$ROOT" && cargo build -p samp-bot); } > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$ROOT"/target/"$SHOT"-*.png
m3_server_start || exit 1
say "server up on port $E2E_PORT"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
