#!/usr/bin/env bash
# M10 acceptance: client features and the connection lifecycle of our in-game client
# (lane "ui": prefix-ui, gta-sa-ui, port 7800, control 47710; run it as
# `flock target/locks/ui.lock tools/e2e/m10.sh`). Expected values: docs/phase3/trace-notes.md
# "Client features (TRACE-8)" (measured on the original). Five game sessions:
#  cfg      test sa-mp.cfg (SAMP_RS_SAMP_CFG): directmode line before "Started", frame rate of
#           fpslimit=30, process affinity 1 (multicore=0), BASS proxy off (audioproxyoff=1, BASS
#           call log), no "Audio stream:" line (audiomsgoff), page size 15; local commands and
#           the rewritten cfg; /rs on foot and in a car (rawvehicles.txt in the game directory);
#           F8 screenshot; chatlog.txt; gmx (RPC 40: restart line, reset script commands, a
#           second InitGame); kick (server closed: line, ack, no reconnect, world kept)
#  lost     server stopped (SIGSTOP) → lost 10 s after the first unacked reliable send (RakNet's
#           ack timer; keep-alives > 5 s after a reliable send, then 15 s apart), reset, reconnect
#           attempts with a new socket each; server resumed → the old player still exists → nick
#           rejected, no retry. sa-mp.cfg without audioproxyoff → created (0) after BASS started
#  full     max_players 1 taken by samp-bot → "The server is full. Retrying..." every ~3.04 s
#           (attempt time taken after Connect returns), join once the bot left; then in a car
#           `/quit`: one UNRELIABLE 0x20, nothing after it, the pools deleted (car occupant
#           warped out, car deleted) > 1000 ms later in the exit frame
#  nosrv    no server: "Connecting to" at +3 s, then every ~6.05 s with 6 requests 1 s apart on
#           a new socket; then a server with a password → "Wrong server password.", no retry
# Each session's loopback traffic is captured and decoded (samp-dump) and checked.
# Exit 0 = pass. Output: target/m10/<session>/.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
export E2E_TAG="${E2E_TAG:-ui}" E2E_PORT="${E2E_PORT:-7800}" E2E_CTL_PORT="${E2E_CTL_PORT:-47710}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT" SAMP_TRACER_PORT="${SAMP_TRACER_PORT:-47711}" SAMP_TRACER_BASS=1
export WINEPREFIX="${WINEPREFIX:-$SAMP_RS_ROOT/wine/prefix-ui}" GTA_DIR="${GTA_DIR:-$SAMP_RS_ROOT/wine/gta-sa-ui}"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUTROOT="$ROOT/target/m10"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
USER_FILES="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files"
SAMP_DIR="$USER_FILES/SAMP"
CFG="$SAMP_DIR/sa-mp.cfg"
say() { printf '[m10] %s\n' "$*"; }

ctl_chat() {
    python3 -I - "$E2E_CTL_PORT" "$1" <<'PY'
import re, socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.settimeout(3)
    s.sendall((sys.argv[2] + "\n").encode())
    f = s.makefile(errors="replace")
    head = f.readline().strip()
    print(head)
    m = re.search(r"count=(\d+)", head)
    for _ in range(int(m.group(1)) if m else 0):
        print(f.readline().strip())
except OSError:
    pass
PY
}

# m10_server_start [password] [max_players]
m10_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/uifs.pwn" \
        -iqawno/include -ofilterscripts/traceui.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" "${1:-}" "${2:-50}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/traceui"]
d["password"] = sys.argv[2]
d["max_players"] = int(sys.argv[3])
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$E2E_SERVER/scriptfiles"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[ui-fs\] loaded' 5 || { say "uifs filterscript not loaded"; return 1; }
    echo "$E2E_SERVER_PID" > "$OUT/server.pid"
}

# ---- inner part: one session inside the headless display -----------------------------
if [[ "${1:-}" == --inner ]]; then
    S="$2"
    OUT="$OUTROOT/$S"
    fail() { say "FAIL ($S): $*"; exit 1; }
    SF="$E2E_SERVER/scriptfiles/uicmd.txt"
    SEQ=0
    fs() {
        SEQ=$((SEQ + 1))
        printf '%d %s\n' "$SEQ" "$1" >> "$SF"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "filterscript command $SEQ ($1) not run"
    }
    mark() { E2E_CTL_PORT="$SAMP_TRACER_PORT" e2e_ctl "mark $*" > /dev/null; }
    chat() { ctl_chat "chat 100"; }
    wait_chat() {
        local re="$1" secs="$2" i
        for ((i = 0; i < secs * 2; i++)); do
            chat | grep -qE -- "$re" && return 0
            sleep 0.5
        done
        return 1
    }
    wait_input() {
        local re="$1" secs="$2" i
        for ((i = 0; i < secs * 2; i++)); do
            e2e_ctl input | grep -qE -- "$re" && return 0
            sleep 0.5
        done
        return 1
    }
    say_cmd() { e2e_ctl "say $1" > /dev/null; sleep 0.5; }
    launch() {
        export SAMP_RS_PRE_DLL="$GTA_DIR/samp-tracer.dll"
        SKIP_BUILD=1 timeout --kill-after=10 400 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
            > "$OUT/run.out" 2>&1 &
        RUN_PID=$!
    }
    join_and_spawn() {
        e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" "${1:-120}" || fail "no connect"
        PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
        e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
        sleep 3
        local spawned=0 n
        n=$(grep -cE "\[e2e\] spawn id=$PID_\b" "$E2E_LOG")
        for _ in 1 2 3 4 5 6; do
            e2e_key shift
            sleep 4
            (( $(grep -cE "\[e2e\] spawn id=$PID_\b" "$E2E_LOG") > n )) && { spawned=1; break; }
        done
        (( spawned )) || fail "no spawn after Shift"
    }
    # Lines of `chat 100` with their text only, in order.
    texts() { chat | sed -nE 's/.* text="(.*)" runs=.*/\1/p'; }
    gta_pid() {
        local p
        local want="$(basename "$GTA_DIR")\\gta_sa.exe"
        for p in $(pgrep -f 'gta_sa.exe' 2>/dev/null); do
            [[ "$(tr '\0' '\n' < "/proc/$p/cmdline" 2>/dev/null | head -n 1)" == *"$want" ]] || continue
            tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qxF "WINEPREFIX=$WINEPREFIX" && { echo "$p"; return; }
        done
    }

    case "$S" in
    cfg)
        launch
        join_and_spawn
        mark spawned
        texts > "$OUT/chat-boot.txt"
        dm=$(grep -n '^ChatWindow: Using direct drawing mode\.$' "$OUT/chat-boot.txt" | head -n 1 | cut -d: -f1)
        st=$(grep -n 'SA-MP {B9C9BF}0.3.7-R4 {FFFFFF}Started' "$OUT/chat-boot.txt" | head -n 1 | cut -d: -f1)
        [[ -n "$dm" && -n "$st" ]] && (( dm < st )) || fail "directmode line not before Started: $(head -n 3 "$OUT/chat-boot.txt" | tr '\n' '|')"
        say "directmode line before Started"
        e2e_ctl input | grep -q ' page_size=15 ' || fail "page size not 15: $(e2e_ctl input)"
        p="$(gta_pid)"
        [[ -n "$p" ]] || fail "gta_sa.exe of this lane not found"
        aff="$(taskset -p "$p" | sed -E 's/.*: //')"
        [[ "$aff" == 1 ]] || fail "affinity of gta_sa.exe ($p) is $aff, want 1 (multicore=0)"
        say "multicore=0: affinity $aff"
        sleep 4
        fps=$(grep -oE 'tick [0-9]+ \(([0-9.]+) fps\)' "$CLIENT_LOG" | tail -n 3 | sed -E 's/.*\(([0-9.]+) fps\)/\1/' | sort -n | sed -n 2p)
        say "fpslimit=30: $fps fps (original 31-32)"
        awk -v f="$fps" 'BEGIN { exit !(f >= 28 && f <= 34) }' || fail "frame rate $fps with fpslimit=30"
        fs 'audio|L|http://127.0.0.1:9/none.mp3|0.0|0.0|0.0|50.0|0'
        sleep 1
        e2e_ctl audio | grep -q 'playing=1' || fail "stream not started: $(e2e_ctl audio)"
        chat | grep -q 'Audio stream:' && fail "'Audio stream:' printed with audiomsgoff=1"
        fs 'audiostop|L'
        say "audiomsgoff=1: stream without chat line"
        # Local commands (run 003749): lines and cfg keys.
        say_cmd '/timestamp'
        say_cmd '/fpslimit 40'
        say_cmd '/headmove'
        say_cmd '/audiomsg'
        say_cmd '/logurls'
        say_cmd '/nametagstatus'
        say_cmd '/mem'
        say_cmd '/interior'
        say_cmd '/cmpstat'
        say_cmd '/togobjlight'
        say_cmd '/hudscalefix'
        rm -f "$SAMP_DIR/rawpositions.txt" "$GTA_DIR/rawvehicles.txt"
        say_cmd '/rs m10foot'
        fs 'pos|L|1700.0|-2600.0|13.5|90.0'
        sleep 2
        fs 'veh|560|1705.0|-2600.0|13.5|90.0|3|6'
        VEH=$(grep -E "\[ui-fs\] $SEQ veh-created" "$E2E_LOG" | sed -E 's/.*veh-created ([0-9]+).*/\1/' | tail -n 1)
        sleep 1
        fs "putin|L|$VEH|0"
        sleep 3
        say_cmd '/rs m10car'
        texts > "$OUT/chat-cmds.txt"
        for want in '-> Frame Limiter: 40' '-> Head movements enabled' 'Audio messages: On' 'URL messages: Off' \
            'NameTag Player Status: ON' 'Current Interior: 0' '-> OnFoot pos saved' '-> InCar pos saved'; do
            grep -qxF -- "$want" "$OUT/chat-cmds.txt" || fail "no line '$want'"
        done
        grep -qE '^Memory: [0-9]+$' "$OUT/chat-cmds.txt" || fail "no /mem line"
        say "local command lines ok"
        tr -d '\r' < "$CFG" > "$OUT/cfg-after.txt"
        for kv in timestamp=0 fpslimit=40 disableheadmove=0 audiomsgoff=0 logurls=0 nonametagstatus=0 nohudscalefix=0; do
            grep -qx "$kv" "$OUT/cfg-after.txt" || fail "sa-mp.cfg lacks $kv: $(tr '\n' ' ' < "$OUT/cfg-after.txt")"
        done
        grep -q $'\r$' "$CFG" || fail "sa-mp.cfg not CR LF"
        say "sa-mp.cfg rewritten: $(tr '\n' ' ' < "$OUT/cfg-after.txt")"
        tr -d '\r' < "$SAMP_DIR/rawpositions.txt" | grep -qE '^-?[0-9]+\.[0-9]{4},-?[0-9]+\.[0-9]{4},-?[0-9]+\.[0-9]{4},[0-9]+\.[0-9]{4} ; m10foot$' \
            || fail "rawpositions.txt: $(cat "$SAMP_DIR/rawpositions.txt" 2>/dev/null)"
        tr -d '\r' < "$GTA_DIR/rawvehicles.txt" 2>/dev/null | grep -qE '^560,-?[0-9.]+,-?[0-9.]+,-?[0-9.]+,[0-9.]+,3,6 ; m10car$' \
            || fail "rawvehicles.txt in the game directory: $(cat "$GTA_DIR/rawvehicles.txt" 2>/dev/null)"
        say "/rs: rawpositions.txt (user files) and rawvehicles.txt (game directory)"
        # F8.
        before=$(ls "$SAMP_DIR/screens" 2>/dev/null | wc -l)
        want=$(printf 'sa-mp-%03d.png' "$before")
        e2e_key F8
        wait_chat "text=\"Screenshot Taken - $want\"" 10 || fail "no 'Screenshot Taken - $want'"
        python3 -I - "$SAMP_DIR/screens/$want" <<'PY' || fail "screenshot format"
import struct, sys
d = open(sys.argv[1], "rb").read()
assert d[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
w, h, depth, ctype = struct.unpack(">IIBB", d[16:26])
chunks, i = [], 8
while i < len(d):
    n, t = struct.unpack(">I4s", d[i:i + 8]); chunks.append(t.decode()); i += 12 + n
print("[m10] F8: %s %dx%d depth %d colour type %d chunks %s" % (sys.argv[1].rsplit("/", 1)[1], w, h, depth, ctype, sorted(set(chunks))))
assert (w, h, depth, ctype) == (1280, 720, 8, 2), "want 1280x720 8-bit RGB"
assert not {"tEXt", "zTXt", "iTXt"} & set(chunks), "text chunks"
PY
        # chatlog.txt: this run's lines, [HH:MM:SS] + text with codes + CR CR LF.
        head -c 300 "$SAMP_DIR/chatlog.txt" | od -c | head -n 3 > "$OUT/chatlog-head.txt"
        python3 -I - "$SAMP_DIR/chatlog.txt" <<'PY' || fail "chatlog.txt"
import re, sys
d = open(sys.argv[1], "rb").read()
lines = d.split(b"\r\r\n")
assert lines[-1] == b"", "last line not CR CR LF terminated"
lines = lines[:-1]
assert all(re.match(rb"^\[\d\d:\d\d:\d\d\] ", l) for l in lines), "line without [HH:MM:SS]"
assert lines[0].endswith(b"ChatWindow: Using direct drawing mode."), "first line is not this run's first line: %r" % lines[0]
assert any(l.endswith(b"{FFFFFF}SA-MP {B9C9BF}0.3.7-R4 {FFFFFF}Started") for l in lines), "colour codes not kept"
assert any(b"Screenshot Taken - " in l for l in lines)
print("[m10] chatlog.txt: %d lines, first %r" % (len(lines), lines[0][:60]))
PY
        # gmx (RPC 40).
        fs 'pos|L|1700.0|-2600.0|13.5|90.0'
        sleep 2
        mark gmx
        n_conn=$(chat | grep -c 'text="Connected to {B9C9BF}')
        fs 'rcon|gmx'
        wait_chat 'type=4 .*text="The server is restarting\.\."' 10 || fail "no 'The server is restarting..'"
        e2e_ctl net | grep -q 'phase=connected' || fail "the RakNet connection must stay up: $(e2e_ctl net)"
        wait_chat 'class_selection=1|text="Connected to \{B9C9BF\}' 1 || true
        for ((i = 0; i < 60; i++)); do
            (( $(chat | grep -c 'text="Connected to {B9C9BF}') > n_conn )) && break
            sleep 1
        done
        (( $(chat | grep -c 'text="Connected to {B9C9BF}') > n_conn )) || fail "no second InitGame after gmx"
        mark gmx_done
        say "gmx: restart line, InitGame again"
        join_and_spawn 5 || true
        sleep 2
        # Kick: the server closes the connection; nothing else happens.
        nveh_before="$(e2e_ctl vehicles | head -n 1)"
        # (the gmx makes the server run OnPlayerConnect again for the connected player)
        n_before=$(grep -cE "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG")
        mark kick
        fs 'kick|L'
        wait_chat 'type=8 .*text="Server closed the connection\."' 10 || fail "no 'Server closed the connection.'"
        sleep 15
        n=$(grep -cE "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG")
        (( n == n_before )) || fail "reconnected after the kick ($n_before → $n connects)"
        [[ "$(chat | grep -c 'text="Connecting to ')" == 1 ]] || fail "a new 'Connecting to' after the kick"
        [[ "$(e2e_ctl vehicles | head -n 1)" == "$nveh_before" ]] || fail "vehicles changed after the kick: $nveh_before → $(e2e_ctl vehicles | head -n 1)"
        e2e_ctl state | grep -qE 'state=1 ' || say "note: ped state $(e2e_ctl state)"
        say "kick: line, no reconnect, world kept ($nveh_before)"
        texts > "$OUT/chat-end.txt"
        ;;
    lost)
        launch
        join_and_spawn
        mark lost
        kill -STOP "$(cat "$OUT/server.pid")"
        say "server stopped (SIGSTOP)"
        wait_chat 'text="Lost connection to the server\. Reconnecting\.\."' 20 || { kill -CONT "$(cat "$OUT/server.pid")"; fail "no lost-connection line"; }
        sleep 8
        kill -CONT "$(cat "$OUT/server.pid")"
        say "server resumed"
        wait_chat 'text="CONNECTION REJECTED: Unacceptable NickName"' 30 || fail "no nick rejection after the resume"
        sleep 10
        texts > "$OUT/chat-end.txt"
        ;;
    full)
        ( cd "$SAMP_RS_ROOT" && exec "$TB/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name FullBot --seconds 50 ) > "$OUT/bot.out" 2>&1 &
        e2e_wait_log "\[e2e\] connect id=[0-9]+ name=FullBot" 20 || fail "bot did not join"
        # open.mp blocks an IP whose previous successful open-connection request (the bot's) is
        # less than 30 s old while one slot is left ("server full attack (2)"): our join after the
        # bot leaves must come later than that.
        launch
        join_and_spawn 90
        texts > "$OUT/chat-end.txt"
        # ClientCheck (RPC 103): the request set of the original's run orig-20261105-035923.
        while read -r t a o c; do
            fs "check|L|$t|$a|$o|$c"
            sleep 1.5
        done <<'R'
2 0 0 4
5 400000 0 4
5 400000 0 256
5 401000 16 256
5 53E981 0 8
5 856E00 0 4
5 3FFFFF 0 4
5 856E01 0 4
45 0 0 4
45 1000 0 64
46 19B 0 4
46 190 0 256
46 0 0 16
46 4E21 0 4
47 19B 0 4
47 190 16 64
47 539 0 32
48 0 0 4
5 400000 257 4
5 400000 0 1
R
        sleep 2
        grep -E "\[ui-fs\] check-response" "$E2E_LOG" | sed -E 's/.*check-response [0-9]+ //' > "$OUT/checks.txt"
        # /quit in a car (original: orig-20261105-003749, static A 0x100c3ec0 / 0x100c5030 / 0x10009880).
        fs 'pos|L|1700.0|-2600.0|13.5|90.0'
        sleep 2
        fs 'veh|560|1705.0|-2600.0|13.5|90.0|3|6'
        VEH=$(grep -E "\[ui-fs\] $SEQ veh-created" "$E2E_LOG" | sed -E 's/.*veh-created ([0-9]+).*/\1/' | tail -n 1)
        sleep 1
        fs "putin|L|$VEH|0"
        sleep 3
        mark quit
        say_cmd '/quit'
        for ((i = 0; i < 20; i++)); do [[ -n "$(gta_pid)" ]] || break; sleep 0.5; done
        [[ -z "$(gta_pid)" ]] || fail "the game still runs 10 s after /quit"
        say "/quit: the game ended"
        exit 0
        ;;
    nosrv)
        launch
        sleep 30
        m10_server_start secret || fail "server"
        e2e_wait_log "incoming connection" 20 || true
        wait_chat 'text="Wrong server password\."' 20 || fail "no 'Wrong server password.'"
        sleep 15
        texts > "$OUT/chat-end.txt"
        ;;
    esac
    cp -f "$CFG" "$OUT/sa-mp.cfg.end" 2>/dev/null
    e2e_ctl quit > /dev/null
    sleep 4
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUTROOT"
rc=0
CAP_PID=""
finish() {
    e2e_headless_stop
    [[ -n "$CAP_PID" ]] && { kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; }
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    rm -f "$GTA_DIR/samp-tracer.dll"
}
trap finish EXIT

say "building (i686-pc-windows-gnu release: client, launcher, tracer; samp-dump, samp-bot)"
{ (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher -p samp-tracer) &&
  (cd "$ROOT" && cargo build -p samp-dump -p samp-bot); } > "$OUTROOT/build.out" 2>&1 \
    || { cat "$OUTROOT/build.out"; say "build failed"; exit 1; }
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$GTA_DIR/samp-tracer.dll"
cp "$TB/i686-pc-windows-gnu/release/samp_tracer.dll" "$GTA_DIR/samp-tracer.dll"

# Decoded capture → per-socket summary (python, from samp-dump's output and tshark timing).
sockets() {
    tshark -r "$1" -T fields -e frame.time_epoch -e udp.srcport -e udp.dstport -e udp.length 2>/dev/null
}

run_session() {
    local s="$1" cfg="${2:-}" srv="${3:-yes}" pw="${4:-}" maxp="${5:-50}"
    OUT="$OUTROOT/$s"
    rm -rf "$OUT"; mkdir -p "$OUT"
    say "---- session $s"
    rm -f "$CLIENT_LOG" "$GTA_DIR/trace.jsonl"
    if [[ "$srv" == yes ]]; then m10_server_start "$pw" "$maxp" || return 1; fi
    dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$OUT/net.pcapng" > "$OUT/dumpcap.out" 2>&1 &
    CAP_PID=$!
    sleep 1
    local r=0
    SAMP_RS_SAMP_CFG="$cfg" E2E_SERVER="$E2E_SERVER" OUT="$OUT" e2e_headless_run bash "$0" --inner "$s" || r=$?
    sleep 1
    kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; CAP_PID=""
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null
    cp -f "$GTA_DIR/trace.jsonl" "$OUT/trace.jsonl" 2>/dev/null
    sockets "$OUT/net.pcapng" > "$OUT/udp.txt"
    "$TB/debug/samp-dump" "$OUT/net.pcapng" --port "$E2E_PORT" > "$OUT/net.dump" 2>&1 || true
    # Per client socket: samp-dump --raw (times relative to the socket's first packet).
    for cp in $(tshark -r "$OUT/net.pcapng" -T fields -e udp.srcport -Y "udp.dstport==$E2E_PORT" 2>/dev/null | sort -u); do
        tshark -r "$OUT/net.pcapng" -Y "udp.port==$cp" -w "$OUT/net-$cp.pcapng" 2>/dev/null
        "$TB/debug/samp-dump" "$OUT/net-$cp.pcapng" --port "$E2E_PORT" --raw > "$OUT/net-$cp.raw" 2>/dev/null || true
    done
    python3 -I "$ROOT/tools/e2e/m10_net.py" "$s" "$OUT" "$E2E_PORT" || r=1
    (( r == 0 )) && say "session $s: PASS" || { say "session $s: FAIL (logs $OUT)"; tail -n 15 "$OUT/samp-rs.log" | sed 's/^/    /'; }
    return "$r"
}

# Without audioproxyoff: samp creates the key only after a successful BASS_Init.
NOPROXYCFG="$OUTROOT/noproxy-sa-mp.cfg"
printf 'pagesize=10\r\nfpslimit=48\r\nmulticore=1\r\n' > "$NOPROXYCFG"
TESTCFG="$OUTROOT/test-sa-mp.cfg"
printf 'fpslimit=30\r\nmulticore=0\r\naudioproxyoff=1\r\ndisableheadmove=1\r\nnonametagstatus=1\r\ntimestamp=1\r\naudiomsgoff=1\r\nlogurls=1\r\nnohudscalefix=1\r\npagesize=15\r\nfontsize=1\r\nfontweight=1\r\ndirectmode=1\r\n' > "$TESTCFG"
for s in ${M10_SESSIONS:-cfg lost full nosrv}; do
    case "$s" in
        cfg) run_session cfg "$TESTCFG" || rc=1 ;;
        lost) run_session lost "$NOPROXYCFG" || rc=1 ;;
        full) run_session full "" yes "" 1 || rc=1 ;;
        nosrv) run_session nosrv "" no || rc=1 ;;
    esac
done
(( rc == 0 )) && say "PASS" || say "FAIL"
exit "$rc"
