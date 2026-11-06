#!/usr/bin/env bash
# Focused session for tools/diff/network.sh (NET_SCEN=tools/diff/gapscen.sh), identical for both
# clients: class-selection arrow sounds, RPC 92 / 150 (driven through the proxy's `rpc-sc` rule,
# which renames the server's RPC 35 SetPlayerDrunkLevel), /save and /rs on foot and in a vehicle
# with the ped / vehicle state read at the same moment, and the X cursor around forced device
# losses (TestCooperativeLevel stub, as tools/trace/scenario.sh objmap_devreset) with and without
# the chat input open. Every measurement goes to $NET_OUT/gap.txt; the tracer's scm records are
# marked (`mark gap:<step>`) and its trace.jsonl is copied to $NET_OUT at the end.
# Env as netscen.sh, plus GAP_TRACER_PORT (our client: the port of the pre-loaded samp-tracer.dll).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
LOG="$NET_SRV/e2e-server.log"
CMDF="$NET_SRV/scriptfiles/uicmd.txt"
MARKS="$NET_OUT/marks.tsv"
GAP="$NET_OUT/gap.txt"
: > "$MARKS"; : > "$GAP"
SEQ=0
if [ "$NET_CLIENT" = orig ]; then TPORT="$NET_CTL_PORT"; else TPORT="${GAP_TRACER_PORT:-47711}"; fi
say() { printf '[gap:%s] %s\n' "$NET_CLIENT" "$*"; }
rec() { printf '%s\n' "$*" >> "$GAP"; say "$*"; }
now() { date +%s.%N; }
fail() { say "FAIL: $*"; printf 'fail\t%s\t%s\n' "$(now)" "$*" >> "$MARKS"; exit 1; }
X() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool "$@"; }
key() { X key --delay 80 "$@"; }
ev() { printf '@%s\t%s\n' "$1" "$(now)" >> "$MARKS"; }
waitlog() {
    local pat="$1" secs="$2" i
    for ((i = 0; i < secs * 4; i++)); do grep -qE "$pat" "$LOG" 2>/dev/null && return 0; sleep 0.25; done
    return 1
}
fs() {
    SEQ=$((SEQ + 1))
    printf '%d %s\n' "$SEQ" "$1" >> "$CMDF"
    waitlog "\[ui-fs\] $SEQ done" 10 || fail "filterscript command $SEQ ($1) not run"
}
fs_out() { grep -E "\[ui-fs\] $SEQ " "$LOG" | grep -v " done" | tail -n 1; }
chat() { key t; sleep 0.6; X type --delay 60 -- "$1"; sleep 0.3; key Return; sleep 1; }
loss() { echo "$1" > "$NET_OUT/loss.ctl"; sleep 0.3; }
ctl() {
    python3 -I - "$1" "$2" <<'PY'
import socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.sendall((sys.argv[2] + "\n").encode())
    print(s.makefile().readline().strip())
except OSError:
    pass
PY
}
mark() { ctl "$TPORT" "mark gap:$1" > /dev/null; ev "$1"; }
# peek through the tracer (loaded in both clients): bytes as one hex string.
peekhex() { ctl "$TPORT" "peek $1 $2" | sed -E 's/^peek [^ ]+ //; s/ //g'; }
u32() { python3 -I -c 'import sys; print(hex(int.from_bytes(bytes.fromhex(sys.argv[1]), "little")))' "$(peekhex "$1" 4)"; }
f32s() { python3 -I -c 'import struct, sys; b = bytes.fromhex(sys.argv[1]); print(" ".join("%.4f" % v for v in struct.unpack("<%df" % (len(b) // 4), b)))' "$(peekhex "$1" "$2")"; }
xcur() { DISPLAY="$E2E_HEADLESS_DISPLAY" python3 -I "$ROOT/tools/e2e/xcursor.py" 2>&1 | head -n 1; }
UF="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP"
X0=1600.0; Y0=-2493.0; Z0=13.55
home() { fs "pos|L|$X0|$Y0|$Z0|270"; sleep 1.5; }

# ---- join, class selection arrows (A1) ---------------------------------------------------
waitlog "\[e2e\] connect id=[0-9]+ name=$NET_NAME" 150 || fail "no join"
waitlog "\[e2e\] class id=" 60 || fail "no class request"
sleep 3
for k in Right Right Left; do
    mark "arrow-$k"; ev "arrow_$k"; key "$k"; sleep 1.5
done
rec "class requests: $(grep -cE '\[e2e\] class id=' "$LOG")"
ped="$(u32 0xB7CD98)"
mtx="$(u32 "$((ped + 0x14))")"
rec "class-selection ped $ped pos $(f32s "$((mtx + 0x30))" 12)"
spawned=0
for _ in 1 2 3 4 5 6; do
    key shift
    if waitlog "\[e2e\] spawn id=" 4; then spawned=1; break; fi
done
[ "$spawned" = 1 ] || fail "no spawn after Shift"
sleep 3

if [ "${GAP_ONLY:-}" != cursor ]; then
# ---- RPC 92 / 150 (A12) ------------------------------------------------------------------
home
loss "rpc-sc 92"; mark rpc92; fs 'drunk|L|4321'; sleep 1.5
loss "rpc-sc 150"; mark rpc150; fs 'drunk|L|8765'; sleep 1.5
loss off; mark rpc-off
rec "proxy rewrites: $(grep -c '"rpc"' "$NET_OUT/loss.jsonl" 2>/dev/null)"

# ---- /save and /rs on foot and in a vehicle (A11) --------------------------------------
home; sleep 1
rm -f "$UF/rawpositions.txt" "$UF/savedpositions.txt" "$GTA_DIR/rawvehicles.txt"
ped="$(u32 0xB7CD98)"; mtx="$(u32 "$((ped + 0x14))")"
rec "foot ped pos $(f32s "$((mtx + 0x30))" 12) rot $(f32s "$((ped + 0x558))" 4) rad"
mark save-foot; chat "/save gapfoot"
mark rs-foot; chat "/rs gapfoot"
rec "foot ped pos after $(f32s "$((mtx + 0x30))" 12)"
fs "veh|416|$X0|$(awk "BEGIN{print $Y0-2.5}")|$Z0|237.5|1|3"
veh="$(fs_out | sed -nE 's/.* veh-created ([0-9]+).*/\1/p')"
sleep 1; fs "putin|L|${veh:-1}|0"; sleep 4
ped="$(u32 0xB7CD98)"; v="$(u32 "$((ped + 0x58c))")"; vm="$(u32 "$((v + 0x14))")"
rec "car $v fwd $(f32s "$((vm + 0x10))" 12) pos $(f32s "$((vm + 0x30))" 12)"
mark save-car; chat "/save gapcar"
mark rs-car; chat "/rs gapcar"
rec "car pos after $(f32s "$((vm + 0x30))" 12)"
rec "savedpositions: $(tr -d '\r' < "$UF/savedpositions.txt" 2>/dev/null | tr '\n' '|')"
rec "rawpositions: $(tr -d '\r' < "$UF/rawpositions.txt" 2>/dev/null | tr '\n' '|')"
rec "rawvehicles: $(tr -d '\r' < "$GTA_DIR/rawvehicles.txt" 2>/dev/null | tr '\n' '|')"
fs "vdel|${veh:-1}"; sleep 2
fi
home

# ---- device loss and the X cursor (A10) --------------------------------------------------
devloss() {
    local dev vt orig le stub_le
    hexle() { printf '%08x' "$1" | sed -E 's/(..)(..)(..)(..)/\4\3\2\1/'; }
    dev="$(u32 0xC97C28)"; vt="$(u32 "$dev")"; orig="$(u32 "$((vt + 12))")"
    # mov eax, D3DERR_DEVICENOTRESET (0x88760869); ret 4 — in the int3/nop padding at 0x7F5B03.
    ctl "$TPORT" "poke 0x7F5B03 b869087688c20400" > /dev/null
    stub_le="$(hexle 0x7F5B03)"; le="$(hexle "$orig")"
    mark "devloss-$1"
    ctl "$TPORT" "poke $(printf '0x%x' $((vt + 12))) $stub_le" > /dev/null
    sleep 0.3
    ctl "$TPORT" "poke $(printf '0x%x' $((vt + 12))) $le" > /dev/null
}
X mousemove 640 360; sleep 1
rec "cursor idle: $(xcur)"
# D3D calls (ShowCursor / SetCursorProperties / Reset are logged in every mode) for ~20 s.
rec "d3d log: $(ctl "$TPORT" 'd3d ui 1200')"
key t; sleep 1
rec "cursor chat open: $(xcur)"
devloss chat; sleep 2
rec "cursor chat open, after reset: $(xcur)"
X mousemove_relative -- 7 5; sleep 1
rec "cursor chat open, after reset + mouse move: $(xcur)"
X mousemove_relative -- -7 -5; sleep 1
rec "cursor chat open, after reset + 2nd move: $(xcur)"
key Escape; sleep 1
rec "cursor chat closed: $(xcur)"
devloss idle; sleep 2
rec "cursor idle, after reset: $(xcur)"
X mousemove_relative -- 7 5; sleep 1
rec "cursor idle, after reset + mouse move: $(xcur)"
key t; sleep 1
rec "cursor chat reopened after reset: $(xcur)"
key Escape; sleep 1

# ---- end --------------------------------------------------------------------------------
mark done
sleep 1
cp -f "$GTA_DIR/trace.jsonl" "$NET_OUT/trace.jsonl" 2>/dev/null
cp -f "$GTA_DIR/d3d.jsonl" "$NET_OUT/d3d.jsonl" 2>/dev/null
[ -f "$GTA_DIR/samp-rs.log" ] && cp -f "$GTA_DIR/samp-rs.log" "$NET_OUT/samp-rs.log"
chat "/q"
sleep 3
say "done"
