#!/usr/bin/env bash
# DIFF-4 / F4: visual parity harness. Runs tools/diff/visual-scenario.sh on the ORIGINAL client
# (samp.dll 0.3.7-R4 + samp-tracer, trace lane, under its lock) and on OUR client (lane "ui",
# under target/locks/ui.lock), both with the Direct3D call-stream logger loaded from the start
# (boot: every call until gGameState 9, then the UI pass), then diffs every UI element with
# tools/diff/visual_diff.py. Exit 0 only when every element diff is empty or each remaining line
# is covered by tools/diff/visual-decisions.tsv (each row names its ROADMAP decision).
#   tools/diff/visual.sh                 ours + diff (runs the original first if target/visual/orig
#                                        has no complete capture)
#   tools/diff/visual.sh --orig          re-capture the original too
#   tools/diff/visual.sh --diff-only     only the diff of the existing captures
#   tools/diff/visual.sh --ours-only     only capture our side
#   VISUAL_ONLY=a,b                      diff only these elements
#   VISUAL_SCENARIO_FILE=PATH            another scenario file defining visual_scenario (default
#                                        tools/diff/visual-scenario.sh, every element); VISUAL_DIR=PATH
#                                        its own output root
# Output: target/visual/{orig,ours}/ (d3d-NAME.jsonl, NAME.png, boot/ timeline, logs),
# target/visual/report.txt.
set -uo pipefail

# Takes the ui lane lock unless the caller already holds it (SAMP_RS_UI_LOCK_HELD=1), so a
# script run inside `flock ui.lock` does not wait on its own parent.
ui_lane_locked() {
    if [ "${SAMP_RS_UI_LOCK_HELD:-}" = 1 ]; then
        "$@"
    else
        SAMP_RS_UI_LOCK_HELD=1 flock "$SAMP_RS_ROOT/target/locks/ui.lock" "$@"
    fi
}
SELF="$(realpath "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/wine" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/wine" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
V="${VISUAL_DIR:-$ROOT/target/visual}"
say() { printf '[visual] %s\n' "$*"; }
# Boot logging (UX-1): all calls until gGameState 9, then the UI pass, for this many Presents.
export SAMP_TRACER_D3D_BOOT="${SAMP_TRACER_D3D_BOOT:-all 1500}"

# ------------------------------------------------------------------ primitives -------------
# Set by the side drivers: SIDE, OUT, GTA_DIR, TPORT (tracer control port), E2E_LOG, SF (uicmd),
# DISPLAY, GAMESCOPE (wayland socket of the headless display), PORT (server).
LOGMARK=0
tctl() { python3 -I - "$TPORT" "$1" <<'PY'
import socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=5)
    s.sendall((sys.argv[2] + "\n").encode())
    print(s.makefile().readline().strip())
except OSError:
    pass
PY
}
# Measurement (no effect on the game): every remote ped's position and, for a ped in a vehicle,
# the vehicle's position (CPed +0x58C → CPlaceable matrix +0x14 → position +0x30), appended to
# $OUT/NAME-remote.txt. The driver nametag is anchored at that vehicle position on both clients.
remote_probe() { python3 -I - "$TPORT" >> "$OUT/$1-remote.txt" <<'PY'
import socket, struct, sys
port = int(sys.argv[1])
def ask(cmd, lines=1):
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        s.sendall((cmd + "\n").encode())
        f = s.makefile()
        out = [f.readline().strip()]
        if lines is None and out[0].startswith("remote count="):
            out += [f.readline().strip() for _ in range(int(out[0].split("=")[1]))]
        return out
    except OSError:
        return []
def peek(addr, n):
    r = ask("peek %#x %d" % (addr, n))
    if not r or not r[0].startswith("peek "):
        return None
    return bytes(int(b, 16) for b in r[0].split()[2:])
for line in ask("remote", None):
    print(line)
    if not line.startswith("remote slot="):
        continue
    ped = int(line.split("ped=")[1].split()[0], 16)
    veh = peek(ped + 0x58C, 4)
    veh = struct.unpack("<I", veh)[0] if veh else 0
    mat = peek(veh + 0x14, 4) if veh else None
    pos = peek(struct.unpack("<I", mat)[0] + 0x30, 12) if mat else None
    if pos:
        print("  vehicle=%#x pos=%.3f,%.3f,%.3f" % ((veh,) + struct.unpack("<3f", pos)))
PY
}
mark_log() { LOGMARK=$(wc -l < "$E2E_LOG"); }
SEQ=0
fs() {
    SEQ=$((SEQ + 1)); mark_log
    printf '%d %s\n' "$SEQ" "$1" >> "$SF"
    local i
    for ((i = 0; i < 20; i++)); do grep -qE "\[ui-fs\] $SEQ done" "$E2E_LOG" && return 0; sleep 0.5; done
    say "$SIDE: command $SEQ ($1) not run"
}
waitlog() {
    local i
    for ((i = 0; i < $2 * 4; i++)); do
        tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -qE "$1" && return 0
        sleep 0.25
    done
    say "$SIDE: no server line /$1/ within $2 s"
    return 1
}
key() { DISPLAY="$DISPLAY" xdotool key --delay 80 "$@"; sleep 0.3; }
kdown() { DISPLAY="$DISPLAY" xdotool keydown "$1"; }
kup() { DISPLAY="$DISPLAY" xdotool keyup "$1"; }
move() { DISPLAY="$DISPLAY" xdotool mousemove "$1" "$2"; sleep 0.3; }
click() { DISPLAY="$DISPLAY" xdotool mousemove "$1" "$2" click 1; sleep 0.3; }
typ() { DISPLAY="$DISPLAY" xdotool type --delay 60 -- "$1"; }
clip() { printf '%s' "$1" | DISPLAY="$DISPLAY" timeout 5 xclip -selection clipboard -loops 4 & sleep 0.3; }
shot() { GAMESCOPE_WAYLAND_DISPLAY="$GAMESCOPE" gamescopectl screenshot "$OUT/$1.png" > /dev/null 2>&1; sleep 0.5; }
cap_start() {
    tctl "mark d3d-begin $1" > /dev/null
    wc -l < "$GTA_DIR/d3d.jsonl" > "$OUT/.d3d-$1.start"
    tctl "d3d $2 ${3:-3}" > /dev/null
}
cap_end() {
    local i
    for ((i = 0; i < 200; i++)); do [[ "$(tctl 'd3d busy')" == idle ]] && break; sleep 0.1; done
    sleep 0.4
    tctl "mark d3d-end $1" > /dev/null
    sleep 0.2
    tail -n +"$(( $(cat "$OUT/.d3d-$1.start") + 1 ))" "$GTA_DIR/d3d.jsonl" > "$OUT/d3d-$1.jsonl"
    rm -f "$OUT/.d3d-$1.start"
}
cap() { cap_start "$1" "$2" "${3:-3}"; cap_end "$1"; shot "$1"; }
BOTS=()
BOTNAMES=()
# Bots started so far in this session: m8afs.pwn names the n-th connected bot `Bn`.
BOTCOUNT=0
bot() {
    local name="$1"; shift
    BOTCOUNT=$((BOTCOUNT + 1))
    mark_log
    ( cd "$SAMP_RS_ROOT" && exec "$TB/debug/samp-bot" "127.0.0.1:$PORT" --name "$name" "$@" ) > "$OUT/bot-$name.out" 2>&1 &
    BOTS+=($!)
    BOTNAMES+=("$name")
}
# Kills the bots and waits until the server has dropped each of them (`[part] NAME has left`,
# the bot's connection times out): the next element must not race the remote player's removal
# on the client (state-aligned, the same on both sides).
killbots() {
    local p n i
    for p in "${BOTS[@]}"; do kill "$p" 2>/dev/null; done
    for n in "${BOTNAMES[@]}"; do
        for ((i = 0; i < 120; i++)); do
            grep -qE "\[part\] $n has left the server" "$E2E_LOG" && break
            sleep 0.25
        done
        (( i < 120 )) || say "$SIDE: bot $n not dropped by the server within 30 s"
    done
    BOTS=(); BOTNAMES=()
    sleep 2
}
# Boot timeline: a screenshot (with a `mark shot-N` before it) every 0.4 s until the class
# selection is requested.
boot_timeline() {
    mkdir -p "$OUT/boot"
    local i=0 start now
    start=$(date +%s.%N)
    while (( i < 200 )); do
        i=$((i + 1))
        local n; n=$(printf '%03d' "$i")
        tctl "mark shot-$n" > /dev/null
        now=$(date +%s.%N)
        GAMESCOPE_WAYLAND_DISPLAY="$GAMESCOPE" gamescopectl screenshot "$OUT/boot/shot-$n.png" > /dev/null 2>&1
        printf '%s\t%.2f\n' "$n" "$(echo "$now - $start" | bc)" >> "$OUT/boot/shots.tsv"
        grep -qE '\[e2e\] class id=' "$E2E_LOG" 2>/dev/null && break
        sleep 0.4
    done
    tctl "mark boot-end" > /dev/null
}
spawn_local() {
    local name="$1" i
    for ((i = 0; i < 240; i++)); do grep -qE "\[e2e\] class id=[0-9]+ " "$E2E_LOG" && break; sleep 0.5; done
    sleep 3
    # The class-selection screen (world camera + class panel): the whole frame.
    cap class all 2
    for i in 1 2 3; do
        mark_log
        key shift
        waitlog "\[e2e\] spawn id=" 8 && break
    done
    sleep 2
}
server_prepare() {
    # $1 = E2E tag; m8afs on the lane's port.
    export E2E_TAG="$1" E2E_PORT="$PORT"
    source "$ROOT/tools/e2e/lib.sh"
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/m8afs.pwn" \
        -iqawno/include -ofilterscripts/m8afs.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m8afs"]
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$E2E_SERVER/scriptfiles"; : > "$E2E_SERVER/scriptfiles/uicmd.txt"
    SF="$E2E_SERVER/scriptfiles/uicmd.txt"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    SRV=$!
    e2e_wait_log '\[ui-fs\] loaded' 20
}
source "$ROOT/tools/e2e/lib.sh"
source "${VISUAL_SCENARIO_FILE:-$ROOT/tools/diff/visual-scenario.sh}"

# ------------------------------------------------------------------ original side ----------
run_orig() {
    SIDE=orig; OUT="$V/orig"; PORT=7790; TPORT=47701
    rm -rf "$OUT"; mkdir -p "$OUT"
    local T="$ROOT/tools/trace"
    server_prepare visorig || { say "orig: server did not start"; return 1; }
    TRACE_NO_SERVER=1 "$T/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
    local RUN=$! SESSION="" i
    for ((i = 0; i < 1200; i++)); do
        SESSION="$(grep -oE 'session: [^ ]+' "$OUT/run.out" | head -n 1 | cut -d' ' -f2)"
        [[ -n "$SESSION" ]] && break
        kill -0 "$RUN" 2>/dev/null || break
        sleep 0.1
    done
    [[ -n "$SESSION" ]] || { say "orig: no session"; cat "$OUT/run.out"; kill "$SRV"; return 1; }
    local TDIR="$ROOT/target/trace/${SESSION%%.*}"
    ln -sfn "$TDIR" "$OUT/trace"
    DISPLAY="$(sed -n 's/^E2E_HEADLESS_DISPLAY=//p' "$TDIR/hold.env")"
    GAMESCOPE="$(sed -n 's/^E2E_GAMESCOPE=//p' "$TDIR/hold.env")"
    GTA_DIR="$SAMP_RS_ROOT/wine/gta-sa-trace"
    boot_timeline
    spawn_local TraceOrig
    [[ "${VISUAL_SCENARIO:-1}" == 1 ]] && visual_scenario
    killbots
    TRACE_SESSION="$SESSION" SAMP_RS_TRACE_LOCK_HELD=1 "$T/ui.sh" stop > /dev/null
    wait "$RUN"
    cp -f "$TDIR/d3d.jsonl" "$OUT/d3d-full.jsonl" 2>/dev/null
    kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log"
    touch "$OUT/complete"
}

# ------------------------------------------------------------------ our side ---------------
run_ours_inner() {
    # Inside the headless display (lane ui).
    SIDE=ours; OUT="$V/ours"; PORT=7800; TPORT=47711
    GAMESCOPE="$E2E_GAMESCOPE"
    E2E_SERVER="$E2E_SERVER_DIR"; E2E_LOG="$E2E_SERVER/e2e-server.log"; SF="$E2E_SERVER/scriptfiles/uicmd.txt"
    export SAMP_RS_PRE_DLL="$GTA_DIR/samp-tracer.dll" SAMP_TRACER_PORT=$TPORT
    SKIP_BUILD=1 timeout --kill-after=10 1200 "$ROOT/tools/run-client.sh" "127.0.0.1:$PORT" TraceOrig \
        > "$OUT/run.out" 2>&1 &
    local RUN=$! i
    for ((i = 0; i < 600; i++)); do [[ "$(tctl ping)" == pong ]] && break; sleep 0.1; done
    boot_timeline
    spawn_local TraceOrig
    [[ "${VISUAL_SCENARIO:-1}" == 1 ]] && visual_scenario
    killbots
    E2E_CTL_PORT=47710 e2e_ctl quit > /dev/null
    for ((i = 0; i < 30; i++)); do kill -0 "$RUN" 2>/dev/null || break; sleep 1; done
}
run_ours() {
    SIDE=ours; OUT="$V/ours"; PORT=7800
    rm -rf "$OUT"; mkdir -p "$OUT"
    export E2E_CTL_PORT=47710 SAMP_RS_CTL_PORT=47710
    export WINEPREFIX="$SAMP_RS_ROOT/wine/prefix-ui" GTA_DIR="$SAMP_RS_ROOT/wine/gta-sa-ui"
    source "$ROOT/tools/wine-env.sh"
    say "ours: building (client, launcher, tracer, bot)"
    { (cd "$ROOT" && cargo build -q --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher -p samp-tracer) &&
      (cd "$ROOT" && cargo build -q -p samp-bot); } > "$OUT/build.out" 2>&1 || { cat "$OUT/build.out"; return 1; }
    rm -f "$GTA_DIR/samp-tracer.dll" "$GTA_DIR/d3d.jsonl" "$GTA_DIR/trace.jsonl"
    cp "$TB/i686-pc-windows-gnu/release/samp_tracer.dll" "$GTA_DIR/samp-tracer.dll"
    local USER_FILES="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files"
    mkdir -p "$USER_FILES/SAMP"
    printf 'pagesize=10\r\nfpslimit=48\r\nmulticore=1\r\naudioproxyoff=0\r\n'"${VISUAL_CFG_EXTRA:-}" > "$USER_FILES/SAMP/sa-mp.cfg"
    server_prepare ui || { say "ours: server did not start"; return 1; }
    E2E_SERVER_DIR="$E2E_SERVER" e2e_headless_run bash -c "source '$SELF' --functions; run_ours_inner"
    ( wineserver -k 2>/dev/null ) || true
    cp -f "$GTA_DIR/d3d.jsonl" "$OUT/d3d-full.jsonl" 2>/dev/null
    cp -f "$GTA_DIR/samp-rs.log" "$OUT/" 2>/dev/null
    rm -f "$GTA_DIR/samp-tracer.dll"
    kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log"
    touch "$OUT/complete"
}

[[ "${1:-}" == --functions ]] && return 0 2>/dev/null

case "${1:-}" in
    --diff-only) ;;
    --ours-only)
        mkdir -p "$SAMP_RS_ROOT/target/locks"
        ui_lane_locked bash -c "source '$SELF' --functions; run_ours" \
            || { say "our side failed"; exit 1; }
        exit 0
        ;;
    *)
        if [[ "${1:-}" == --orig || ! -f "$V/orig/complete" ]]; then
            say "original side (trace lane; waits for its lock)"
            "$ROOT/tools/trace/with-lock.sh" bash -c "source '$SELF' --functions; run_orig" \
                || { say "original side failed"; exit 1; }
        fi
        say "our side (lane ui; waits for its lock)"
        mkdir -p "$SAMP_RS_ROOT/target/locks"
        ui_lane_locked bash -c "source '$SELF' --functions; run_ours" \
            || { say "our side failed"; exit 1; }
        ;;
esac
python3 -I "$ROOT/tools/diff/visual_diff.py" "$V/ours" "$V/orig" --decisions "$ROOT/tools/diff/visual-decisions.tsv" \
    ${VISUAL_ONLY:+--only "$VISUAL_ONLY"} | tee "$V/report.txt"
exit "${PIPESTATUS[0]}"
