#!/usr/bin/env bash
# BOT-2 acceptance: samp-bot as a scriptable second player, against an isolated
# open.mp server copy (lane "bot", UDP 7799) with the e2eprobe and bot2helper
# filterscripts. No game.
#   1. drive:     the server sees OnPlayerEnterVehicle (driver), PLAYER_STATE_DRIVER
#                 and the vehicle moving >= 5 m; at the end the bot leaves via RPC 154.
#   2. passenger: OnPlayerEnterVehicle (passenger) and PLAYER_STATE_PASSENGER.
#   3. shoot:     an idle target bot and a shooter with a Deagle → OnPlayerWeaponShot
#                 events with weapon 24, hit type 1, hit id = target, small offsets.
# Exit code 0 when all three pass. Logs: target/e2e-bot2/.
set -u
export E2E_TAG="${E2E_TAG:-bot}" E2E_PORT="${E2E_PORT:-7799}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/tools/e2e/lib.sh"
OUT="$ROOT/target/e2e-bot2"
BOT="$ROOT/target/debug/samp-bot"
mkdir -p "$OUT"
BOT_PIDS=()
FAILS=0

say() { echo "[bot2] $*"; }
fail() { say "FAIL: $*"; FAILS=$((FAILS + 1)); }

cleanup() {
    local p
    for p in "${BOT_PIDS[@]}"; do kill "$p" 2>/dev/null; done
    e2e_server_stop
}
trap cleanup EXIT

bot2_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/bot2helper.pwn" \
        -iqawno/include -ofilterscripts/bot2helper.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/bot2helper"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[b2\] helper loaded' 5 || { say "bot2helper not loaded"; return 1; }
}

# start_bot NAME ARGS... — runs in the background; sets LAST_PID.
start_bot() {
    local name="$1"; shift
    ( cd "$ROOT" && exec "$BOT" "127.0.0.1:$E2E_PORT" --name "$name" "$@" ) > "$OUT/$name.log" 2>&1 &
    LAST_PID=$!
    BOT_PIDS+=("$LAST_PID")
}

# id_of NAME — player id from the probe's connect line.
id_of() { grep -E "\[e2e\] connect id=[0-9]+ name=$1( |$)" "$E2E_LOG" | tail -n 1 | sed -E 's/.*id=([0-9]+).*/\1/'; }

# wait_bot PID NAME — the bot must exit 0.
wait_bot() {
    if ! wait "$1"; then fail "$2 exited non-zero (see $OUT/$2.log)"; return 1; fi
}

say "building samp-bot"
( cd "$ROOT" && cargo build -q -p samp-bot ) || { say "build failed"; exit 1; }
[ -s "$ROOT/data/auth_table.txt" ] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
say "starting server (lane $E2E_TAG, port $E2E_PORT)"
bot2_server_start || exit 1

# ---- 1. drive ------------------------------------------------------------
say "1/3 drive"
start_bot B2Drive --mode drive --seconds 14
wait_bot "$LAST_PID" B2Drive
id=$(id_of B2Drive)
veh=$(grep -E "\[b2\] vehicle id=[0-9]+ for=$id " "$E2E_LOG" | tail -n 1 | sed -E 's/.*vehicle id=([0-9]+).*/\1/')
if [ -z "$id" ] || [ -z "$veh" ]; then
    fail "drive: bot or vehicle not set up (id=$id veh=$veh)"
else
    grep -qE "\[b2\] enter id=$id veh=$veh passenger=0" "$E2E_LOG" || fail "drive: no OnPlayerEnterVehicle(driver)"
    grep -qE "\[b2\] statechange id=$id new=2 " "$E2E_LOG" || fail "drive: never PLAYER_STATE_DRIVER"
    moved=$(grep -E "\[b2\] veh id=$veh pos=" "$E2E_LOG" | sed -E 's/.*pos=//' | python3 -I -c '
import sys, math
pts = [tuple(map(float, l.split(","))) for l in sys.stdin if l.strip()]
print(f"{max((math.dist(pts[0], p) for p in pts), default=0.0):.2f}")')
    say "   vehicle $veh max displacement ${moved} m"
    python3 -I -c "import sys; sys.exit(0 if float('$moved') >= 5.0 else 1)" || fail "drive: vehicle moved < 5 m"
    grep -qE "\[b2\] exit id=$id veh=$veh" "$E2E_LOG" || fail "drive: no OnPlayerExitVehicle (RPC 154)"
    grep -qE "\[b2\] statechange id=$id new=1 old=2" "$E2E_LOG" || fail "drive: not back on foot after exit"
fi

# ---- 2. passenger ----------------------------------------------------------
say "2/3 passenger"
start_bot B2Pass --mode passenger --seconds 10
wait_bot "$LAST_PID" B2Pass
id=$(id_of B2Pass)
if [ -z "$id" ]; then
    fail "passenger: bot did not connect"
else
    grep -qE "\[b2\] enter id=$id veh=[0-9]+ passenger=1" "$E2E_LOG" || fail "passenger: no OnPlayerEnterVehicle(passenger)"
    grep -qE "\[b2\] statechange id=$id new=3 " "$E2E_LOG" || fail "passenger: never PLAYER_STATE_PASSENGER"
fi

# ---- 3. shoot --------------------------------------------------------------
say "3/3 shoot"
start_bot B2Target --mode idle --seconds 22
TARGET_PID=$LAST_PID
sleep 3
start_bot B2Shoot --mode shoot --target B2Target --weapon 24 --seconds 16
wait_bot "$LAST_PID" B2Shoot
wait_bot "$TARGET_PID" B2Target
sid=$(id_of B2Shoot); tid=$(id_of B2Target)
if [ -z "$sid" ] || [ -z "$tid" ]; then
    fail "shoot: bots did not connect"
else
    grep -E "\[b2\] shot id=$sid " "$E2E_LOG" > "$OUT/shots.txt"
    n=$(wc -l < "$OUT/shots.txt")
    good=$(python3 -I - "$OUT/shots.txt" "$tid" <<'PY'
import re, sys, math
tid = int(sys.argv[2]); good = 0
for l in open(sys.argv[1]):
    m = re.search(r"weapon=(\d+) hittype=(\d+) hitid=(\d+) off=([-0-9.]+),([-0-9.]+),([-0-9.]+)", l)
    if m and m[1] == "24" and m[2] == "1" and int(m[3]) == tid and math.dist((0, 0, 0), tuple(map(float, m.groups()[3:]))) < 2.0:
        good += 1
print(good)
PY
)
    dmg=$(grep -cE "\[b2\] takedamage id=$tid issuer=$sid " "$E2E_LOG")
    gdmg=$(grep -cE "\[b2\] givedamage id=$sid damaged=$tid " "$E2E_LOG")
    say "   $n shot event(s), $good plausible (weapon 24, player hit on $tid, |offset| < 2); take/give damage $dmg/$gdmg"
    [ "$good" -ge 3 ] || fail "shoot: fewer than 3 plausible OnPlayerWeaponShot events"
    [ "$good" -eq "$n" ] || fail "shoot: implausible shot events (see $OUT/shots.txt)"
fi

cp "$E2E_LOG" "$OUT/server.log"
if [ "$FAILS" -eq 0 ]; then say "PASS (logs in $OUT)"; exit 0; fi
say "$FAILS failure(s) (logs in $OUT)"; exit 1
