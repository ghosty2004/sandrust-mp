#!/usr/bin/env bash
# M6 acceptance test: world entities (objects, attached objects, building removal, pickups, checkpoints, race
# checkpoints, gang zones, map icons, actors, 3D labels).
#  1. build (client DLL + launcher for Windows); isolated server copy with the e2e probe and tools/trace/uifs.pwn
#     (the TRACE-6 command filterscript, so the parity runs use the identical input)
#  2. inside a headless gamescope: start our client, spawn it (Shift), run tools/e2e/m6scen.sh
#  3. after each block assert the control channel `world` (entity counts, GTA pointers, positions, flags) and
#     server-side events: OnPlayerPickUpPickup (types 2 and 19 by collision), checkpoint / race checkpoint enter,
#     OnPlayerGiveDamageActor (RPC 177), OnObjectMoved; screenshots target/m6-*.png (isolated display only)
# Lane: E2E_TAG / E2E_PORT / E2E_CTL_PORT (default veh / 7820 / 47720); WINEPREFIX / GTA_DIR from the environment
# (lane veh: wine/prefix-shot + wine/gta-sa-shot). SAMP_RS_ROOT: checkout with wine/, server/, data/ (worktrees).
# M6_ONLY=objects,pickups,… runs a subset. Exit status 0 = pass. Usage: tools/e2e/m6.sh
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
source "$ROOT/tools/e2e/m6scen.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
SHOT="${M6_SHOT:-m6}"
OUT="$ROOT/target/$SHOT"
say() { printf '[%s] %s\n' "$SHOT" "$*"; }
# The TRACE-6 filterscript (tools/trace/uifs.pwn with the world commands); from the data checkout until merged.
UIFS="$ROOT/tools/trace/uifs.pwn"
grep -q '"objmove"' "$UIFS" 2>/dev/null || UIFS="$SAMP_RS_ROOT/tools/trace/uifs.pwn"

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

m6_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$UIFS" -iqawno/include -ofilterscripts/m6fs.amx -d0 \
        > /dev/null ) || { say "uifs.pwn ($UIFS) does not compile"; return 1; }
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m6fs"]
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
        # GTA's idle camera takes over (and hides the HUD) after ~90 s without pad input; samp NOPs it (CSV row
        # 244, applied by the player-parity work). Until that is merged, a key tap every 20 s keeps
        # the input timer fresh (a short Space tap = sprint key without a direction: no movement).
        if (( SECONDS - LAST_INPUT >= 20 )); then
            e2e_key space
            LAST_INPUT=$SECONDS
        fi
        SEQ=$((SEQ + 1))
        printf '%d %s\n' "$SEQ" "$1" >> "$E2E_SERVER/scriptfiles/uicmd.txt"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "uifs command $SEQ ($1) not run"
    }
    mark() { :; }
    shot() { e2e_shot "$ROOT/target/$SHOT-$1.png" || say "warning: screenshot $1 failed"; }
    fire() {
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mousedown 1
        sleep "$(awk "BEGIN{print $1/1000}")"
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mouseup 1
    }
    world() { ctl_multi world; }
    wline() { world | grep -E "^$1" | head -n 1; }
    field() { sed -nE "s/.* $1=([^ ]+).*/\1/p" <<< "$2"; }
    head_field() { field "$1" "$(world | head -n 1)"; }
    near() { awk -v a="$1" -v b="$2" -v t="${3:-0.05}" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= t) }'; }
    obj() { wline "object id=$1 "; }
    check() {
        case "$1" in
            objects)
                [[ "$(head_field objects)" == 5 ]] || fail "world shows $(head_field objects) objects, want 5: $(world | tr '\n' '|')"
                for id in 1 2 3 4 5; do
                    l="$(obj "$id")"
                    [[ "$(field ptr "$l")" != 0x0 && -n "$l" ]] || fail "object $id has no GTA object: $l"
                    [[ "$(field rw "$l")" == true ]] || fail "object $id has no RW object: $l"
                    fl="$(field flags "$l")"; (( fl & 0x80000000 )) || fail "object $id lacks entity flag bit 31: $l"
                done
                [[ "$(field samp_pass "$(obj 1)")" == false ]] || fail "object 1 (dd 0) should be drawn by GTA"
                for id in 2 3 4 5; do [[ "$(field samp_pass "$(obj "$id")")" == true ]] || fail "object $id should be in samp's pass: $(obj "$id")"; done
                [[ "$(field materials "$(obj 4)")" == 1 && "$(field materials "$(obj 5)")" == 1 ]] || fail "materials missing"
                (( $(head_field samp_draws) > 0 )) || fail "samp's own render pass drew nothing"
                say "objects: $(world | head -n 1)" ;;
            move1)
                l="$(obj 1)"; IFS=, read -r _ _ z <<< "$(field pos "$l")"
                near "$z" 23.9 0.01 || fail "object 1 at z=$z after MoveObject, want 23.9: $l"
                [[ "$(field moving "$l")" == false ]] || fail "object 1 still moving"
                grep -qE "\[ui-fs\] object-moved 1" "$E2E_LOG" || fail "server saw no OnObjectMoved"
                say "move1: $l" ;;
            move2rot)
                l="$(obj 1)"; IFS=, read -r rx ry _ <<< "$(field right "$l")"; IFS=, read -r _ _ z <<< "$(field pos "$l")"
                near "$z" 13.9 0.01 && near "$rx" 0 0.01 && near "$ry" 1 0.01 || fail "object 1 not at z 13.9 / rotated 90°: $l"
                say "move2rot: $l" ;;
            move3stop)
                l="$(obj 1)"; IFS=, read -r _ _ z <<< "$(field pos "$l")"
                [[ "$(field moving "$l")" == false ]] || fail "object 1 still moving after StopObject"
                awk -v z="$z" 'BEGIN { exit !(z > 14.5 && z < 23.5) }' || fail "StopObject: z=$z not between start and target"
                sleep 1; IFS=, read -r _ _ z2 <<< "$(field pos "$(obj 1)")"
                near "$z" "$z2" 0.001 || fail "object 1 kept moving after StopObject ($z -> $z2)"
                say "move3stop: $l" ;;
            attach)
                l="$(wline "attached player=")"
                [[ -n "$l" && "$(field ptr "$l")" != 0x0 ]] || fail "attached object missing: $(world | tr '\n' '|')"
                IFS=, read -r ax ay az <<< "$(field pos "$l")"
                d="$(awk -v x="$ax" -v y="$ay" -v z="$az" -v a="$M6_X" -v b="$M6_Y" -v c="$M6_Z" 'BEGIN { printf "%.2f", sqrt((x-a)^2+(y-b)^2+(z-c)^2) }')"
                awk -v d="$d" 'BEGIN { exit !(d < 2.0) }' || fail "attached object $d m from the player: $l"
                say "attach: $l (distance $d m)" ;;
            rmb) [[ "$(head_field removals)" == 1 ]] || fail "building removal not recorded" ;;
            pickups-created)
                [[ "$(head_field pickups)" == 3 ]] || fail "world shows $(head_field pickups) pickups, want 3: $(world | tr '\n' '|')" ;;
            pickups)
                grep -qE "\[ui-fs\] pickup-collected [0-9]+ pickup 0" "$E2E_LOG" || fail "no OnPlayerPickUpPickup for the type-2 pickup"
                grep -qE "\[ui-fs\] pickup-collected [0-9]+ pickup 2" "$E2E_LOG" || fail "no OnPlayerPickUpPickup for the type-19 pickup"
                say "pickups: $(grep -E "\[ui-fs\] pickup-collected" "$E2E_LOG" | tr '\n' '|') sent_131=$(head_field sent_131)" ;;
            cp)
                [[ "$(head_field checkpoint)" == 1 ]] || fail "checkpoint not active in world"
                e2e_wait_log "\[ui-fs\] cp-enter" 5 || fail "server saw no checkpoint enter"
                say "checkpoint: $(wline checkpoint)" ;;
            rcp)
                [[ "$(head_field race_checkpoint)" == 1 ]] || fail "race checkpoint not active"
                l="$(wline race_checkpoint)"; [[ "$(field handle "$l")" != 0 ]] || fail "no GTA race checkpoint: $l"
                e2e_wait_log "\[ui-fs\] rcp-enter" 5 || fail "server saw no race checkpoint enter"
                say "race checkpoint: $l" ;;
            zones) [[ "$(head_field zones)" == 3 ]] || fail "world shows $(head_field zones) gang zones, want 3" ;;
            zones-flash)
                a="$(head_field zone_hook)"; sleep 1; b="$(head_field zone_hook)"
                say "radar zone hook (calls/busy/drawn) $a -> $b"
                if [[ "${a##*/}" == "${b##*/}" ]]; then
                    say "HUD flags: menu.hud_on $(e2e_ctl 'peek 0xBA6769 1') radar_off $(e2e_ctl 'peek 0xBAA3FB 1') widescreen $(e2e_ctl 'peek 0xB6F065 1') state $(e2e_ctl state)"
                    fail "gang zones no longer drawn on the radar ($a -> $b)"
                fi ;;
            icons)
                [[ "$(head_field icons)" == 4 ]] || fail "world shows $(head_field icons) map icons, want 4"
                for i in 0 1 2 3; do [[ "$(field blip "$(wline "icon id=$i ")")" != 0 ]] || fail "icon $i has no blip"; done ;;
            actor)
                l="$(wline "actor id=0 ")"
                [[ -n "$l" && "$(field ped "$l")" != 0x0 ]] || fail "actor 0 has no ped: $(world | tr '\n' '|')"
                say "actor: $l" ;;
            actor-damage)
                e2e_wait_log "\[ui-fs\] actor-damage [0-9]+ actor 0 " 5 || fail "server saw no OnPlayerGiveDamageActor (RPC 177)"
                l="$(wline "actor id=0 ")"; hp="$(field health "$l")"
                near "$hp" 100 0.01 || fail "actor health changed locally ($hp); samp cancels the response"
                say "actor damage: $(grep -E "\[ui-fs\] actor-damage" "$E2E_LOG" | head -n 2 | tr '\n' '|') sent_177=$(head_field sent_177)" ;;
            acthp0)
                l="$(wline "actor id=0 ")"; near "$(field health "$l")" 0 0.01 || fail "actor health not 0: $l"
                say "actor health 0: $l" ;;
            blanktex)
                l="$(obj 6)"; [[ "$(field materials "$l")" == 1 && "$(field samp_pass "$l")" == true ]] \
                    || fail "object 6 has no blank material: $l"
                grep -q "blanktex.txd could not be loaded" "$CLIENT_LOG" && fail "SAMP\\blanktex.txd not loaded"
                say "blanktex: $l" ;;
            labels)
                [[ "$(head_field labels)" == 2 ]] || fail "world shows $(head_field labels) labels, want 2"
                n="$(field count "$(wline labels_drawn)")"
                (( n >= 1 )) || fail "no 3D label projected on screen"
                say "labels: drawn $n" ;;
        esac
    }

    SKIP_BUILD=1 timeout --kill-after=10 900 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
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
    m6_scenario
    say "world: $(world | head -n 1)"
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
rm -f "$CLIENT_LOG" "$ROOT"/target/"$SHOT"-*.png
m6_server_start || exit 1
say "server up on port $E2E_PORT (lane $E2E_TAG, prefix $WINEPREFIX, filterscript $UIFS)"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
