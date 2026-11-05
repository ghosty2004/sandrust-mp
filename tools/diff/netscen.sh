#!/usr/bin/env bash
# The scripted session of tools/diff/network.sh, identical for both clients.
# Env: NET_CLIENT (orig|ours), NET_NAME, NET_SRV (server dir), NET_OUT, NET_PORT,
# E2E_HEADLESS_DISPLAY (the client's isolated display).
# Every scenario window is recorded as "NAME<TAB>start<TAB>end" (epoch seconds) in marks.tsv;
# input events as "@NAME<TAB>time" (trigger latencies are measured from them).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LOG="$NET_SRV/e2e-server.log"
CMDF="$NET_SRV/scriptfiles/uicmd.txt"
MARKS="$NET_OUT/marks.tsv"
: > "$MARKS"
SEQ=0
say() { printf '[net:%s] %s\n' "$NET_CLIENT" "$*"; }
now() { date +%s.%N; }
fail() { say "FAIL: $*"; printf 'fail\t%s\t%s\n' "$(now)" "$*" >> "$MARKS"; exit 1; }
X() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool "$@"; }
key() { X key --delay 80 "$@"; }
ev() { printf '@%s\t%s\n' "$1" "$(now)" >> "$MARKS"; }
begin() { CUR="$1"; CUR_T="$(now)"; say "scenario $1"; }
end() { printf '%s\t%s\t%s\n' "$CUR" "$CUR_T" "$(now)" >> "$MARKS"; }
waitlog() {
    local pat="$1" secs="$2" i
    for ((i = 0; i < secs * 4; i++)); do grep -qE "$pat" "$LOG" 2>/dev/null && return 0; sleep 0.25; done
    return 1
}
count() { grep -cE "$1" "$LOG" 2>/dev/null; }
fs() {
    SEQ=$((SEQ + 1))
    printf '%d %s\n' "$SEQ" "$1" >> "$CMDF"
    waitlog "\[ui-fs\] $SEQ done" 10 || fail "filterscript command $SEQ ($1) not run"
}
fs_out() { grep -E "\[ui-fs\] $SEQ " "$LOG" | grep -v " done" | tail -n 1; }
hold() { X keydown "$1"; sleep "$2"; X keyup "$1"; }
chat() { key t; sleep 0.6; X type --delay 60 -- "$1"; sleep 0.3; ev "$2"; key Return; }
loss() { echo "$1" > "$NET_OUT/loss.ctl"; sleep 0.2; }
# A flat open spot: the LS airport runway, facing east.
X0=1600.0; Y0=-2493.0; Z0=13.55
home() { fs "pos|L|$X0|$Y0|$Z0|270"; sleep 1.5; }

# ---- join + class selection ------------------------------------------------------------
begin join
waitlog "\[e2e\] connect id=[0-9]+ name=$NET_NAME" 150 || fail "no join"
waitlog "\[e2e\] class id=" 60 || fail "no class request"
sleep 3
end
begin spawn
spawned=0
for _ in 1 2 3 4 5 6; do
    ev spawn_key; key shift
    if waitlog "\[e2e\] spawn id=" 4; then spawned=1; break; fi
done
[ "$spawned" = 1 ] || fail "no spawn after Shift"
sleep 3
end
home; sleep 1

begin walk
ev walk_key; hold w 4
sleep 1
end

begin idle
sleep 17
end

home
begin chat
chat "hello network diff" chat_key
sleep 1.5
chat "/netcmd 1 2 3" cmd_key
sleep 2
end

begin dialog
fs 'dialog|7|1|NetDiff|Type a word|OK|Cancel'
sleep 1.5
X type --delay 60 -- "abc"
sleep 0.3
ev dialog_key; key Return
waitlog "dialog-response player [0-9]+ id 7" 5 || say "no dialog response logged"
sleep 1.5
end

begin scoreboard
ev tab_key; key Tab
sleep 2.5
key Escape
sleep 1.5
end

home
begin pickup
fs "pickup|1240|2|$(awk "BEGIN{print $X0+3}")|$Y0|$Z0"
sleep 1
ev pickup_walk; hold w 1.5
waitlog "pickup" 3 || true
sleep 1.5
end

home
begin checkpoint
fs "cp|L|$(awk "BEGIN{print $X0+3}")|$Y0|$Z0|3"
sleep 1
hold w 1.5
sleep 1.5
fs 'cpoff|L'
sleep 1
end

home
begin object
fs "obj|1337|$(awk "BEGIN{print $X0+2}")|$Y0|$Z0|0|0|0|100"
obj="$(fs_out | sed -nE 's/.* object ([0-9]+).*/\1/p')"
fs "objedit|L|${obj:-1}"
sleep 1.5
ev object_esc; key Escape
sleep 2
end
# Both clients swallow the Escape key-down while editing (no GTA pause menu; NET6-P4). Safety net
# for a regression: if the client's frame counter (control channel `state`, ticks=N) stands still,
# Escape once more resumes the game so the rest of the session still runs (logged as pause_resume).
ticks() {
    python3 -I -c '
import re, socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=2); s.settimeout(2)
    s.sendall(b"state\n"); m = re.search(r"ticks=(\d+)", s.makefile().readline()); print(m.group(1) if m else "")
except OSError:
    pass' "$NET_CTL_PORT"
}
if [ -n "${NET_CTL_PORT:-}" ]; then
    t1="$(ticks)"; sleep 1; t2="$(ticks)"
    if [ -n "$t1" ] && [ "$t1" = "$t2" ]; then
        say "client paused after Escape (ticks $t1): resuming"; ev pause_resume; key Escape; sleep 2
    fi
fi

# ---- vehicles (an ambulance: no music radio; music decoding in a Sultan hits GTA's known
# divide-by-zero at 0x4F1464 under Wine, ROADMAP "m5 occasional failure") -------------------
home
fs "veh|416|$X0|$(awk "BEGIN{print $Y0-2.5}")|$Z0|270|1|1"
veh="$(fs_out | sed -nE 's/.* veh-created ([0-9]+).*/\1/p')"
sleep 2
begin drive
ev enter_key; key f
sleep 4
ev drive_key; hold w 3
sleep 2
ev exit_key; key f
sleep 3
end
fs "vdel|${veh:-1}"
sleep 1

home
fs "veh|416|$X0|$(awk "BEGIN{print $Y0-2.5}")|$Z0|270|1|1"
veh="$(fs_out | sed -nE 's/.* veh-created ([0-9]+).*/\1/p')"
sleep 2
begin passenger
ev passenger_key; key g
sleep 5
sleep 3
ev pexit_key; key f
sleep 3
end
fs "vdel|${veh:-1}"
sleep 1

# ---- combat with samp-bot ----------------------------------------------------------------
home
"$HERE/../../target/debug/samp-bot" "127.0.0.2:$NET_PORT" --name NetBot --mode idle \
    > "$NET_OUT/bot.out" 2>&1 &
BOT=$!
waitlog "\[e2e\] spawn id=[0-9]+" 30 || true
sleep 3
fs "pos|B1|$(awk "BEGIN{print $X0+6}")|$Y0|$Z0|90"
fs 'weapon|L|24|200'
sleep 2
begin shoot
ev aim_key; X mousedown 3
sleep 1
for _ in 1 2 3; do ev fire_key; X mousedown 1; sleep 0.15; X mouseup 1; sleep 0.6; done
X mouseup 3
sleep 2
end
kill "$BOT" 2>/dev/null
sleep 2

# ---- loss: the proxy drops client / server reliable datagrams ---------------------------
home
begin loss_cs
loss "drop-cs-reliable 1"
chat "lost one" loss1_key
sleep 1.5
chat "lost two" loss2_key
sleep 1.5
loss off
end
begin loss_repeat
loss "drop-cs-repeat 3"
chat "lost three times" loss3_key
sleep 3
loss off
sleep 1
end
begin loss_sc
loss "drop-sc-reliable 1"
for i in 1 2 3; do fs "msg|FFFFFFFF|server line $i"; sleep 0.7; done
sleep 1
loss off
end

begin death
n=$(count "\[e2e\] spawn id=")
ev kill_cmd; fs 'kill|L'
for ((i = 0; i < 60; i++)); do (( $(count "\[e2e\] spawn id=") > n )) && break; sleep 0.5; done
sleep 3
end

begin quit
chat "/q" quit_key
sleep 4
end
say "done"
