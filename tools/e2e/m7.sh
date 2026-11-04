#!/usr/bin/env bash
# M7 acceptance test: combat.
#  For each lag-comp mode in M7_MODES (default "1 2 0" = server game.lag_compensation_mode), with the neutral
#  gamemode tools/trace/tracegm.pwn and the command filterscript tools/trace/uifs.pwn on an isolated server copy:
#  1. our client spawns at the LS airport tarmac (1700, -2600);
#  2. local shoots bot: an idle samp-bot stands 8 m ahead; the server gives us a Deagle and we fire (LMB, un-aimed:
#     the ray follows the ped's facing). Server: OnPlayerGiveDamage(us → bot, 46.2 = 140 × 0.33, weapon 24) for
#     every hit; OnPlayerWeaponShot(hit type 1, the bot) for each in modes 1/2, none in mode 0 (no 206 at all,
#     TRACE-6 "Lag-comp modes"); the 204 slot record follows the ammo (GetPlayerWeaponData);
#  3. weapon RPCs: SetPlayerAmmo, SetPlayerArmedWeapon, ResetPlayerWeapons → the server's view (204 / 207);
#  4. bot shoots local (modes 1/2): samp-bot --mode shoot with 206 hit type 1 at us: our health drops by 46.2 per
#     hit and we report OnPlayerTakeDamage(46.2, weapon 24) for each; after the third: OnPlayerDeath(killer = bot,
#     reason 24), then a respawn (OnPlayerSpawn) ~3.6 s later. Mode 0: the server relays no 206; the bot's
#     key-driven shots are simulated locally (measured in the original: they pass above the head, no damage);
#  5. SetPlayerHealth(0): OnPlayerDeath(killer INVALID, reason 255), then a respawn.
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default main / 7777 / 47700); WINEPREFIX / GTA_DIR from the
# environment. Wrap in `flock <SAMP_RS_ROOT>/target/locks/main.lock`. Exit status 0 = pass.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
MODES="${M7_MODES:-1 2 0}"
OUT="$ROOT/target/m7"
BIN="${CARGO_TARGET_DIR:-$ROOT/target}"
say() { printf '[m7] %s\n' "$*"; }

m7_server_start() {   # m7_server_start LAGCOMP
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/uifs.pwn" \
        -iqawno/include -ofilterscripts/uifs.amx -d0 > /dev/null &&
      LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/tracegm.pwn" \
        -iqawno/include -ogamemodes/tracegm.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" "$1" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/uifs"]
d["pawn"]["main_scripts"] = ["tracegm 1"]
d["game"]["lag_compensation_mode"] = int(sys.argv[2])
json.dump(d, open(p, "w"), indent=4)
PY
    : > "$E2E_SERVER/scriptfiles/uicmd.txt"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[ui-fs\] loaded' 5 || { say "uifs filterscript not loaded"; return 1; }
}

# ---- inner part: one mode, runs inside the headless display ----------------------------------------------
if [[ "${1:-}" == --inner ]]; then
    MODE="$2"
    O="$OUT/mode$MODE"
    BOT_PID=""
    fail() { say "FAIL (mode $MODE): $*"; e2e_shot "$O/fail.png" > /dev/null 2>&1; [[ -n "$BOT_PID" ]] && kill "$BOT_PID" 2>/dev/null; exit 1; }
    SEQ=0
    fs() {   # fs "cmd|args" — run a uifs command and wait for it; its output lines go to FS_OUT (no subshell:
             # the sequence number must persist)
        SEQ=$((SEQ + 1))
        printf '%d %s\n' "$SEQ" "$1" >> "$E2E_SERVER/scriptfiles/uicmd.txt"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "uifs command $SEQ ($1) not run"
        FS_OUT="$(grep -E "\[ui-fs\] $SEQ " "$E2E_LOG" | grep -v " done " || true)"
    }
    mouse() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool "$@"; }
    fire() { mouse mousedown 1; sleep 0.15; mouse mouseup 1; }
    health() { e2e_ctl state | sed -nE 's/.* health=([-0-9.]+).*/\1/p'; }
    count() { grep -cE "$1" "$E2E_LOG"; }
    start_bot() {   # start_bot NAME ARGS... — sets BOT_PID and BOT_ID
        local name="$1"; shift
        ( cd "$ROOT" && exec "$BIN/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name "$name" --seconds 600 \
            --auth-table "${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt" "$@" ) > "$O/bot-$name.out" 2>&1 &
        BOT_PID=$!
        e2e_wait_log "\[ui-fs\] connect [0-9]+ $name" 20 || fail "bot $name did not connect"
        BOT_ID="$(grep -E "\[ui-fs\] connect [0-9]+ $name" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect ([0-9]+).*/\1/')"
    }
    stop_bot() {   # stop_bot ID — kicked by the server (a killed bot would only time out), then the process ends
        fs "kick|$1"
        e2e_wait_log "\[ui-fs\] disconnect $1 " 20 || fail "bot $1 did not leave"
        kill "$BOT_PID" 2>/dev/null; wait "$BOT_PID" 2>/dev/null; BOT_PID=""
    }

    SKIP_BUILD=1 timeout --kill-after=10 900 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$O/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    L="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[e2e\] class id=$L " 30 || fail "no class request"
    sleep 3
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$L\b" 8; then spawned=1; break; fi
    done
    (( spawned )) || fail "no spawn after Shift"
    say "mode $MODE: we are id $L, spawned; $(e2e_ctl combat)"
    sleep 2
    if [[ -n "${M7_HOLD:-}" ]]; then
        # Debugging: keep the session until $O/stop exists (drive it with uicmd.txt / the control channel /
        # xdotool on $E2E_HEADLESS_DISPLAY, written to $O/display).
        echo "$E2E_HEADLESS_DISPLAY" > "$O/display"; rm -f "$O/stop"
        while [[ ! -f "$O/stop" ]]; do sleep 1; done
        e2e_ctl quit > /dev/null; sleep 3; exit 0
    fi

    # ---- local shoots bot ---------------------------------------------------------------------------------
    start_bot TgtBot --mode idle; B="$BOT_ID"
    say "TgtBot = id $B"
    fs "pos|L|1700|-2600|13.55|0"
    sleep 3
    # The bot first stands aside; one calibration shot (aimed with RMB, camera pitched down by 30 px) hits the
    # ground and tells where the crosshair ray goes; the bot is then put on that ray 8 m ahead and three shots
    # follow within the same RMB hold (TRACE-6 aimed the same way, by hand).
    fs "pos|$B|1690|-2592|13.55|180"
    e2e_wait_log "\[e2e\] streamin id=$B for=$L" 20 || fail "bot never streamed in for us"
    sleep 1
    fs "weapon|L|24|50"
    sleep 2
    mouse mousedown 3; sleep 1.2
    for _ in 1 2 3; do mouse mousemove_relative -- 0 -10; sleep 0.05; done
    sleep 0.6
    c0=$(count "\[ui-fs\] shot $L weapon 24 ")
    fire; sleep 0.8
    (( $(count "\[ui-fs\] shot $L weapon 24 ") > c0 )) || [[ "$MODE" == 0 ]] || fail "no 206 for the calibration shot"
    if [[ "$MODE" == 0 ]]; then
        # No 206 in mode 0: use the ray measured in modes 1/2 of this setup (origin → ground point).
        P="1700.64|-2591.63|13.55"
    else
        line="$(grep -E "\[ui-fs\] shot $L weapon 24 " "$E2E_LOG" | tail -n 1)"
        say "calibration: $line"
        P="$(sed -E 's/.*origin ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) hitpos ([-0-9.]+) ([-0-9.]+) ([-0-9.]+).*/\1 \2 \3 \4 \5 \6/' <<< "$line" |
            awk '{ dx = $4 - $1; dy = $5 - $2; t = 8 / sqrt(dx * dx + dy * dy); printf "%.2f|%.2f|13.55", $1 + dx * t, $2 + dy * t }')"
    fi
    fs "pos|$B|$P|180"
    sleep 1.2
    g0=$(count "\[ui-fs\] give-damage $L to $B ")
    s0=$(count "\[ui-fs\] shot $L weapon 24 ")
    for _ in 1 2 3; do fire; sleep 1.3; done
    mouse mouseup 3
    sleep 1.5
    e2e_shot "$ROOT/target/m7-mode$MODE-local.png" > /dev/null 2>&1 || true
    gives=$(( $(count "\[ui-fs\] give-damage $L to $B ") - g0 ))
    shots=$(( $(count "\[ui-fs\] shot $L weapon 24 ") - s0 ))
    hits=$(grep -E "\[ui-fs\] shot $L weapon 24 hittype 1 hitid $B " "$E2E_LOG" | wc -l)
    say "local → bot ($P): $gives OnPlayerGiveDamage, $shots OnPlayerWeaponShot ($hits hit type 1 at the bot); $(e2e_ctl combat)"
    grep -E "\[ui-fs\] (give-damage $L|shot $L)" "$E2E_LOG" | tail -n 6 | sed 's/^/    /'
    (( gives >= 1 )) || fail "no OnPlayerGiveDamage for our shots"
    bad=$(grep -E "\[ui-fs\] give-damage $L to $B " "$E2E_LOG" | grep -vcE " 46\.200 weapon 24 part [0-9]+")
    (( bad == 0 )) || fail "OnPlayerGiveDamage with an unexpected amount/weapon (want 46.200, weapon 24)"
    if [[ "$MODE" == 0 ]]; then
        (( shots == 0 )) || fail "mode 0 must send no 206 ($shots OnPlayerWeaponShot)"
    else
        (( shots == 3 && hits >= gives )) || fail "want 3 shots with every damaging one a hit-type-1 shot at the bot"
    fi
    fs "wdata|L|2"; w="$FS_OUT"
    say "204 view after 3 shots: $w"
    grep -qE "wdata $L slot 2 weapon 24 ammo 46 armed 24" <<< "$w" || fail "204 slot 2 does not show the Deagle with 46 rounds"

    # ---- weapon RPCs --------------------------------------------------------------------------------------
    fs "ammo|L|24|20"; sleep 1
    fs "wdata|L|2"; w="$FS_OUT"; say "after SetPlayerAmmo(20): $w"
    grep -qE "wdata $L slot 2 weapon 24 ammo 20 " <<< "$w" || fail "SetPlayerAmmo not reflected in 204"
    fs "armed|L|0"; sleep 1.5
    fs "wdata|L|2"; w="$FS_OUT"; say "after SetPlayerArmedWeapon(0): $w"
    grep -qE " armed 0$" <<< "$w" || fail "SetPlayerArmedWeapon(0) not reflected in 207"
    fs "resetw|L"; sleep 1
    fs "wdata|L|2"; w="$FS_OUT"; say "after ResetPlayerWeapons: $w"
    grep -qE "wdata $L slot 2 weapon 0 ammo 0 " <<< "$w" || fail "ResetPlayerWeapons not reflected in 204"
    stop_bot "$B"
    sleep 2

    # ---- bot shoots local -----------------------------------------------------------------------------------
    fs "hp|L|100"
    fs "pos|L|1700|-2600|13.55|0"
    start_bot ShootBot --mode shoot --target "$NAME" --weapon 24 --hit-type 1 --fire-ms 1500; S="$BOT_ID"
    say "ShootBot = id $S"
    sleep 3
    fs "pos|$S|1700|-2590|13.55|180"
    e2e_wait_log "\[e2e\] streamin id=$S for=$L" 20 || fail "shooting bot never streamed in for us"
    sleep 1
    h0="$(health)"
    t0=$(count "\[ui-fs\] take-damage $L from $S ")
    d0=$(count "\[ui-fs\] death $L ")
    fs "weapon|$S|24|500"
    if [[ "$MODE" == 0 ]]; then
        sleep 8
        takes=$(( $(count "\[ui-fs\] take-damage $L from $S ") - t0 ))
        say "mode 0: bot fired for 8 s: $takes OnPlayerTakeDamage, health $h0 → $(health); $(e2e_ctl combat)"
    else
        e2e_wait_log "\[ui-fs\] take-damage $L from $S " 15 || fail "no OnPlayerTakeDamage from the bot's 206"
        sleep 0.3
        h1="$(health)"
        say "first hit: health $h0 → $h1; $(grep -E "\[ui-fs\] take-damage $L from $S " "$E2E_LOG" | tail -n 1)"
        awk -v a="$h0" -v b="$h1" 'BEGIN { d = a - b; exit !(d > 46.1 && d < 46.3 || d > 92.3 && d < 92.5) }' \
            || fail "health dropped by $(awk -v a="$h0" -v b="$h1" 'BEGIN{print a-b}'), want 46.2 per hit"
        e2e_wait_log "\[ui-fs\] death $L killer $S reason 24" 20 || fail "no OnPlayerDeath(killer = bot, reason 24)"
        takes=$(( $(count "\[ui-fs\] take-damage $L from $S ") - t0 ))
        bad=$(grep -E "\[ui-fs\] take-damage $L from $S " "$E2E_LOG" | grep -vcE " 46\.200 weapon 24 part [0-9]+")
        say "bot → local: $takes OnPlayerTakeDamage (46.2 each); death: $(grep -E "\[ui-fs\] death $L " "$E2E_LOG" | tail -n 1)"
        (( bad == 0 )) || fail "OnPlayerTakeDamage with an unexpected amount/weapon"
        (( takes >= 3 )) || fail "want ≥ 3 OnPlayerTakeDamage before the death"
        sp0=$(count "\[e2e\] spawn id=$L\b")
        stop_bot "$S"
        e2e_wait_log "\[e2e\] spawn id=$L\b.*" 1 > /dev/null
        for _ in $(seq 1 30); do (( $(count "\[e2e\] spawn id=$L\b") > sp0 )) && break; sleep 0.5; done
        (( $(count "\[e2e\] spawn id=$L\b") > sp0 )) || fail "no respawn after the death"
        say "respawned; health $(health)"
        S=""
    fi
    [[ -n "$S" ]] && stop_bot "$S"
    sleep 2

    # ---- health 0 ----------------------------------------------------------------------------------------------
    sp0=$(count "\[e2e\] spawn id=$L\b")
    fs "kill|L"
    e2e_wait_log "\[ui-fs\] death $L killer 65535 reason 255" 10 || fail "no OnPlayerDeath(INVALID, 255) after health 0"
    for _ in $(seq 1 30); do (( $(count "\[e2e\] spawn id=$L\b") > sp0 )) && break; sleep 0.5; done
    (( $(count "\[e2e\] spawn id=$L\b") > sp0 )) || fail "no respawn after health 0"
    sleep 1
    say "health 0 → death (255) → respawn; health $(health); $(e2e_ctl combat)"
    e2e_shot "$ROOT/target/m7-mode$MODE-end.png" > /dev/null 2>&1 || true
    say "client: $(e2e_ctl net)"
    e2e_ctl quit > /dev/null
    for _ in $(seq 1 20); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 1; done
    say "mode $MODE: all checks passed"
    exit 0
fi

# ---- outer part -----------------------------------------------------------------------------------------------
mkdir -p "$OUT"
rc=1
finish() {
    e2e_headless_stop
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    if (( rc == 0 )); then say "PASS (modes $MODES)"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    say "building (i686-pc-windows-gnu release + samp-bot)"
    { (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
      (cd "$ROOT" && cargo build -p samp-bot); } > "$OUT/build.out" 2>&1 \
        || { cat "$OUT/build.out"; say "build failed"; exit 1; }
fi
[[ -f "${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt" ]] || { say "data/auth_table.txt missing"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
for MODE in $MODES; do
    mkdir -p "$OUT/mode$MODE"
    rm -f "$CLIENT_LOG"
    m7_server_start "$MODE" || exit 1
    say "mode $MODE: server up on port $E2E_PORT (lane $E2E_TAG)"
    e2e_headless_run bash "$0" --inner "$MODE"
    r=$?
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    cp -f "$CLIENT_LOG" "$OUT/mode$MODE/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/mode$MODE/server.log" 2>/dev/null || true
    (( r == 0 )) || exit 1
done
rc=0
exit 0
