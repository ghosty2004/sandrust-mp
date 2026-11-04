#!/usr/bin/env bash
# M4 acceptance test: chat window, chat input, local commands and game text of
# our in-game client (lane "ui": prefix-ui, gta-sa-ui, port 7800, control 47710).
#  1. build; isolated server copy with the e2e probe and the m4helper filterscript;
#     the lane's sa-mp.cfg is reset to a known start (pagesize 10)
#  2. inside a headless gamescope: start our client, spawn it (Shift)
#  3. m4helper sends three client messages (plain, {RRGGBB} codes, alpha AA) and a
#     game text: `chat` must list them with the expected colour runs, `input` the
#     game text; the connect lines are type-8 lines
#  4. T, type "hello world", Enter → the server logs it from RustClient and echoes
#     it back (type-2 line "RustClient:" in the player colour)
#  5. "/pagesize 15" is local (nothing sent): page size 15 and sa-mp.cfg updated;
#     "/m4ping" goes to the server as RPC 50 and is answered; "/save m4" appends
#     to savedpositions.txt; ESC closes the input without sending; F7 cycles the
#     chat modes; PageUp/PageDown scroll
#  6. "/q" quits the game
#  7. the capture (dumpcap, lo, UDP port) is decoded with samp-dump: the C→S RPC
#     101 and 50 must have the original's layout (u8 / u32 length + text, rel 8)
# Screenshots: target/m4-{chat,input,typed,pagesize,mode1,mode0}.png. Logs: target/m4/.
# Run from a git worktree: SAMP_RS_ROOT (checkout with wine/, server/, data/) is
# taken from the main worktree automatically when this checkout has no server/.
# Exit status 0 = pass. Usage: tools/e2e/m4.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
export E2E_TAG="${E2E_TAG:-ui}" E2E_PORT="${E2E_PORT:-7800}" E2E_CTL_PORT="${E2E_CTL_PORT:-47710}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
export WINEPREFIX="${WINEPREFIX:-$SAMP_RS_ROOT/wine/prefix-ui}" GTA_DIR="${GTA_DIR:-$SAMP_RS_ROOT/wine/gta-sa-ui}"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUT="$ROOT/target/m4"
SHOT="$ROOT/target/m4"
USER_FILES="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files"
SAMP_DIR="$USER_FILES/SAMP"
CFG="$SAMP_DIR/sa-mp.cfg"
say() { printf '[m4] %s\n' "$*"; }

# Multi-line control replies: `chat N` (header + count lines).
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

m4_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/m4helper.pwn" \
        -iqawno/include -ofilterscripts/m4helper.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/m4helper"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[m4\] helper loaded' 5 || { say "m4helper filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; exit 1; }
    field() { sed -nE "s/.* $1=([^ ]+).*/\1/p" <<< "$2"; }
    # wait_ctl CMD REGEX SECONDS — until the (multi-line) reply matches.
    wait_chat() {
        local re="$1" secs="$2" i
        for ((i = 0; i < secs * 2; i++)); do
            ctl_chat "chat 30" | grep -qE -- "$re" && return 0
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
    # type_line TEXT [SHOT] — opens the input with T, types TEXT (xdotool type),
    # checks the input text, optionally takes a screenshot, presses Return and
    # (unless SHOT is "quit") waits for the input to close.
    type_line() {
        e2e_key t
        wait_input ' open=1 ' 5 || fail "T did not open the input: $(e2e_ctl input)"
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "$1"
        local want="text=\"$1\""
        wait_input "$(sed 's/[][\.*^$]/\\&/g' <<< "$want")" 5 || fail "input text is not \"$1\": $(e2e_ctl input)"
        [[ -n "${2:-}" && "${2:-}" != quit ]] && { e2e_shot "$SHOT-$2.png" || say "warning: screenshot $2 failed"; }
        e2e_key Return
        [[ "${2:-}" == quit ]] && return 0
        wait_input ' open=0 ' 5 || fail "Return did not close the input: $(e2e_ctl input)"
    }

    SKIP_BUILD=1 timeout --kill-after=10 400 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    # Connect lines (type 8, colour A9C4E4) are on screen during class selection.
    wait_chat 'line type=8 colour=FFA9C4E4 .*text="Connected to \{B9C9BF\}' 10 \
        || fail "no 'Connected to' line: $(ctl_chat 'chat 5' | tr '\n' '|')"
    for t in 'SA-MP \{B9C9BF\}0.3.7-R4 \{FFFFFF\}Started' "Connecting to 127.0.0.1:$E2E_PORT\.\.\." 'Connected. Joining the game\.\.\.'; do
        ctl_chat "chat 30" | grep -qE "line type=8 colour=FFA9C4E4 .*text=\"[^\"]*$t" || fail "missing connect line /$t/"
    done
    say "connect lines ok"
    sleep 3
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    e2e_wait_log "\[m4\] greeted id=$PID_\b" 10 || fail "m4helper did not greet us"

    # 3. client messages and game text.
    wait_chat 'text="M4 alpha cyan"' 10 || fail "client messages not in chat: $(ctl_chat 'chat 5' | tr '\n' '|')"
    chat="$(ctl_chat 'chat 30')"
    printf '%s\n' "$chat" > "$OUT/chat-1.txt"
    grep -qF 'line type=4 colour=FFFFFFFF name_colour=FFFFFFFF name="" text="M4 white line" runs=[FFFFFFFF:"M4 white line"]' <<< "$chat" \
        || fail "white line wrong: $(grep 'M4 white' <<< "$chat")"
    grep -qF 'line type=4 colour=FFFF0000 name_colour=FFFF0000 name="" text="M4 red {00FF00}green {0000FF}blue" runs=[FFFF0000:"M4 red ",FF00FF00:"green ",FF0000FF:"blue"]' <<< "$chat" \
        || fail "colour-code line wrong: $(grep 'M4 red' <<< "$chat")"
    grep -qF 'line type=4 colour=FF33CCFF name_colour=FF33CCFF name="" text="M4 alpha cyan"' <<< "$chat" \
        || fail "alpha line wrong: $(grep 'M4 alpha' <<< "$chat")"
    say "client messages ok (3 lines, colour runs as expected)"
    wait_input 'gametext_style=3 gametext_ms=15000 gametext="~r~M4 ~w~game text"' 5 \
        || fail "game text not shown: $(e2e_ctl input)"
    say "game text accepted: $(e2e_ctl input | grep -o 'gametext_style.*')"
    sleep 1
    e2e_shot "$SHOT-chat.png" || say "warning: chat screenshot failed"

    # 4. chat input → RPC 101 → server echo.
    type_line "hello world" typed
    e2e_wait_log "\[m4\] text id=$PID_ text=hello world" 10 || fail "server did not log our chat text"
    wait_chat 'line type=2 colour=FFFFFFFF name_colour=FF[0-9A-F]{6} name="RustClient:" text="hello world"' 10 \
        || fail "no echoed player chat line: $(ctl_chat 'chat 3' | tr '\n' '|')"
    say "chat round trip ok: $(ctl_chat 'chat 30' | grep 'text="hello world"' | head -n 1)"

    # 5. local and server commands.
    type_line "/pagesize 15"
    wait_chat 'page_size=15 ' 5 || fail "page size is not 15: $(ctl_chat 'chat 0')"
    grep -q $'^pagesize=15\r$' "$CFG" || fail "sa-mp.cfg lacks pagesize=15: $(tr '\r\n' '|.' < "$CFG")"
    say "pagesize 15 applied and saved: $(tr '\r\n' ' ' < "$CFG")"
    sleep 1
    e2e_shot "$SHOT-pagesize.png" || say "warning: pagesize screenshot failed"
    type_line "/m4ping"
    e2e_wait_log "\[m4\] cmd id=$PID_ cmd=/m4ping" 10 || fail "server did not get /m4ping"
    wait_chat 'colour=FFFFFF00 .*text="M4 pong"' 10 || fail "no M4 pong"
    grep -q "\[m4\] cmd id=$PID_ cmd=/pagesize" "$E2E_LOG" && fail "/pagesize was sent to the server"
    say "server command round trip ok; /pagesize stayed local"
    rm -f "$SAMP_DIR/savedpositions.txt"
    type_line "/save m4"
    wait_chat 'type=4 colour=FF88AA62 .*text="-> OnFoot position saved"' 5 || fail "no /save feedback"
    tr -d '\r' < "$SAMP_DIR/savedpositions.txt" 2>/dev/null \
        | grep -qE '^AddPlayerClass\([0-9]+,-?[0-9.]+,-?[0-9.]+,-?[0-9.]+,[0-9.]+,0,0,0,0,0,0\); // m4$' \
        && grep -q $'\r$' "$SAMP_DIR/savedpositions.txt" \
        || fail "savedpositions.txt: $(cat "$SAMP_DIR/savedpositions.txt" 2>/dev/null)"
    say "/save ok: $(tr -d '\r' < "$SAMP_DIR/savedpositions.txt")"
    # ESC closes without sending and keeps the text (as the original does).
    e2e_key t; wait_input ' open=1 ' 5 || fail "T did not open the input"
    DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "not sent"
    e2e_shot "$SHOT-input.png" || say "warning: input screenshot failed"
    e2e_key Escape
    wait_input ' open=0 text="not sent" ' 5 || fail "ESC did not close the input (text kept): $(e2e_ctl input)"
    sleep 1
    grep -q 'text=not sent' "$E2E_LOG" && fail "ESC-closed text reached the server"
    e2e_key t; wait_input ' open=1 text="not sent" ' 5 || fail "T did not reopen with the kept text: $(e2e_ctl input)"
    say "ESC ok (text kept for the next open)"
    # History: Up recalls the last line, Down restores the live line.
    e2e_key Up
    wait_input 'text="/save m4"' 5 || fail "Up did not recall the last line: $(e2e_ctl input)"
    e2e_key Down
    wait_input 'text="not sent"' 5 || fail "Down did not restore the live line: $(e2e_ctl input)"
    for _ in 1 2 3 4 5 6 7 8; do e2e_key BackSpace; done
    wait_input ' open=1 text="" ' 5 || fail "BackSpace did not empty the input: $(e2e_ctl input)"
    e2e_key Escape; wait_input ' open=0 ' 5 || fail "ESC did not close the input"
    say "history and editing ok"
    # F7 modes and paging.
    e2e_key F7; wait_chat ' mode=1 ' 5 || fail "F7 did not switch to mode 1"
    e2e_shot "$SHOT-mode1.png" || true
    e2e_key F7; wait_chat ' mode=0 ' 5 || fail "F7 did not switch to mode 0"
    e2e_shot "$SHOT-mode0.png" || true
    e2e_key F7; wait_chat ' mode=2 ' 5 || fail "F7 did not switch back to mode 2"
    e2e_key Prior; wait_chat ' at_bottom=0 ' 5 || fail "PageUp did not scroll: $(ctl_chat 'chat 0')"
    e2e_key Next; wait_chat ' at_bottom=1 ' 5 || fail "PageDown did not return to the bottom: $(ctl_chat 'chat 0')"
    say "F7 modes and paging ok"
    ctl_chat "chat 30" > "$OUT/chat-2.txt"
    say "net: $(e2e_ctl net)"

    # 6. /q quits.
    type_line "/q" quit
    for _ in $(seq 1 30); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 1; done
    kill -0 "$RUN_PID" 2>/dev/null && fail "/q did not quit the game"
    e2e_wait_log "\[e2e\] disconnect id=$PID_ " 10 || fail "server did not see the disconnect"
    say "/q quit the game"
    say "all in-game checks passed"
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUT"
rc=1
CAP_PID=""
finish() {
    e2e_headless_stop
    [[ -n "$CAP_PID" ]] && { kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; }
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

say "building (i686-pc-windows-gnu release + samp-dump)"
{ (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
  (cd "$ROOT" && cargo build -p samp-dump); } > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
# The lane must have its own user files (a Wine prefix links Documents to ~/Documents
# by default, which every lane would share).
docs="$WINEPREFIX/drive_c/users/$USER/Documents"
if [[ -L "$docs" ]]; then
    say "$docs is a symlink ($(readlink "$docs")); give the lane its own Documents folder first"; exit 1
fi
mkdir -p "$SAMP_DIR"
printf 'pagesize=10\r\nfpslimit=48\r\nmulticore=1\r\naudioproxyoff=0\r\n' > "$CFG"
rm -f "$CLIENT_LOG" "$SHOT"-*.png
m4_server_start || exit 1
say "server up on port $E2E_PORT"
dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$OUT/m4.pcapng" > "$OUT/dumpcap.out" 2>&1 &
CAP_PID=$!
sleep 1
e2e_headless_run bash "$0" --inner
rc=$?
sleep 1
kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; CAP_PID=""
if (( rc == 0 )); then
    # 7. wire format of what we sent.
    "${CARGO_TARGET_DIR:-$ROOT/target}/debug/samp-dump" "$OUT/m4.pcapng" --port "$E2E_PORT" > "$OUT/m4.dump" 2>&1 || true
    grep -E '^C>S .*rpc +(101|50) ' "$OUT/m4.dump" > "$OUT/m4-chat-rpcs.txt"
    sed 's/^/    /' "$OUT/m4-chat-rpcs.txt"
    hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
    grep -qE "^C>S rel8 rpc 101 96 bits 0b$(hex 'hello world')\$" "$OUT/m4.dump" \
        || { say "FAIL: RPC 101 for 'hello world' missing or not u8-length + text, reliable"; rc=1; }
    grep -qE "^C>S rel8 rpc +50 88 bits 07000000$(hex '/m4ping')\$" "$OUT/m4.dump" \
        || { say "FAIL: RPC 50 for '/m4ping' missing or not u32-length + text, reliable"; rc=1; }
    grep -qE "^C>S .*rpc +50 .*$(hex '/pagesize')" "$OUT/m4.dump" && { say "FAIL: /pagesize was sent"; rc=1; }
    grep -qE "^C>S .*rpc +101 .*$(hex 'not sent')" "$OUT/m4.dump" && { say "FAIL: ESC text was sent"; rc=1; }
    (( rc == 0 )) && say "capture: RPC 101 / 50 layouts as specified (UCA §101/§50), nothing sent for local commands"
fi
exit "$rc"
