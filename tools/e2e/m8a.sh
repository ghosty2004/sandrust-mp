#!/usr/bin/env bash
# M8a acceptance test: dialogs (all 6 styles), textdraws + selection, menus, scoreboard,
# nametags, death window, chat bubbles and game text of our in-game client
# (lane "ui": prefix-ui, gta-sa-ui, port 7800, control 47710).
#  1. build; isolated server copy with the e2e probe and tools/e2e/probe/m8afs.pwn (a
#     superset of tools/trace/uifs.pwn: the same command lines drive the original)
#  2. inside a headless gamescope: start our client and spawn it (Shift)
#  3. dialogs: the six TRACE-3 dialogs (same ids, titles, bodies) answered with ESC,
#     typing + Return, list keys; the server must log the RPC 62 answers the original
#     gave in TRACE-3 (docs/phase3/trace-notes.md "UI (TRACE-3)")
#  4. textdraws: shown (`ui` lists them), selection with a hover colour, a mouse click on
#     a selectable textdraw → OnPlayerClickTextDraw, ESC → 65535 (RPC 83)
#  5. menu: shown, Sprint selects a row (RPC 132), Enter/exit leaves (RPC 140)
#  6. death window rows (RPC 55), a bot joins next to us (nametag), its chat bubble,
#     the scoreboard (Tab; RPC 155 on open and every 3 s), game text
#  7. the capture is decoded with samp-dump: RPC 62/83/132/140/155 layouts and reliability
# Screenshots: target/m8a/*.png. Logs: target/m8a/. Exit status 0 = pass.
# M8A_HOLD=SECS keeps the session for manual inspection before quitting.
# M8A_D3D=1 loads samp-tracer.dll (built from the main checkout's source into
# target/main-tracer) before our DLL and logs the Direct3D call stream at every
# screenshot (target/m8a/d3d-NAME.jsonl); tools/e2e/m8a_d3d_diff.py then diffs them
# against the original client's TRACE-3 logs.
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
OUT="$ROOT/target/m8a"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
USER_FILES="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files"
SAMP_DIR="$USER_FILES/SAMP"
CFG="$SAMP_DIR/sa-mp.cfg"
say() { printf '[m8a] %s\n' "$*"; }

m8a_server_start() {
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
    : > "$E2E_SERVER/scriptfiles/uicmd.txt" 2>/dev/null || { mkdir -p "$E2E_SERVER/scriptfiles"; : > "$E2E_SERVER/scriptfiles/uicmd.txt"; }
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[ui-fs\] loaded' 5 || { say "m8afs filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; say "ui: $(e2e_ctl ui)"; exit 1; }
    SF="$E2E_SERVER/scriptfiles/uicmd.txt"
    SEQ=0
    # fs 'cmd|args' — one m8afs command; waits until the filterscript ran it.
    fs() {
        SEQ=$((SEQ + 1))
        printf '%d %s\n' "$SEQ" "$1" >> "$SF"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "filterscript command $SEQ ($1) not run"
    }
    wait_ui() {
        local re="$1" secs="$2" i
        for ((i = 0; i < secs * 2; i++)); do
            e2e_ctl ui | grep -qE -- "$re" && return 0
            sleep 0.5
        done
        return 1
    }
    tctl() { E2E_CTL_PORT="$SAMP_TRACER_PORT" e2e_ctl "$1"; }
    # cap NAME — 3 logged Presents (tracer "d3d ui 3") → $OUT/d3d-NAME.jsonl.
    cap() {
        local f="$GTA_DIR/d3d.jsonl" start i
        start=$(wc -l < "$f" 2>/dev/null || echo 0)
        local mode=ui
        case "$1" in textdraws|menu|sprite) mode=all ;; esac
        tctl "d3d $mode 3" > /dev/null
        for ((i = 0; i < 100; i++)); do [[ "$(tctl 'd3d busy')" == idle ]] && break; sleep 0.1; done
        sleep 0.4
        tail -n +"$((start + 1))" "$f" > "$OUT/d3d-$1.jsonl"
    }
    shot() {
        [[ "${M8A_D3D:-0}" == 1 ]] && cap "$1"
        e2e_shot "$OUT/$1.png" || say "warning: screenshot $1 failed"
    }
    key() { e2e_key "$@"; sleep 0.3; }
    mouse() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mousemove "$1" "$2"; sleep 0.3; }
    click() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mousemove "$1" "$2" click 1; sleep 0.3; }
    resp() { e2e_wait_log "\[ui-fs\] dialog-response player $PID_ $1" 10 || fail "no dialog response /$1/ (server: $(grep 'dialog-response' "$E2E_LOG" | tail -n 1))"; }

    if [[ "${M8A_D3D:-0}" == 1 ]]; then
        export SAMP_RS_PRE_DLL="$GTA_DIR/samp-tracer.dll"
    fi
    SKIP_BUILD=1 timeout --kill-after=10 900 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    sleep 2
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    shot base

    # 3. Dialogs (TRACE-3 bodies).
    fs 'dialog|10|0|Caption {FF0000}red|Line one\nLine {00FF00}two\tTabbed|OK|Cancel'
    wait_ui 'dialog_open=1 dialog_id=10 dialog_style=MsgBox' 5 || fail "style 0 not open"
    shot dlg0
    key Escape
    resp 'id 10 response 0 listitem -1 text '"''"
    wait_ui 'dialog_open=0' 5 || fail "dialog 10 did not close"
    say "style 0 ok (ESC → response 0)"
    fs 'dialog|11|1|Input caption|Enter some text:|OK|Cancel'
    wait_ui 'dialog_id=11 dialog_style=Input' 5 || fail "style 1 not open"
    sleep 0.5
    shot dlg1
    DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "hi there"
    wait_ui 'dialog_text="hi there"' 5 || fail "dialog edit text wrong"
    key Return
    resp "id 11 response 1 listitem -1 text 'hi there'"
    say "style 1 ok"
    fs 'dialog|13|3|Password|Secret:|OK|'
    wait_ui 'dialog_id=13 dialog_style=Password' 5 || fail "style 3 not open"
    DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "abc"
    wait_ui 'dialog_text="abc"' 5 || fail "password text wrong"
    shot dlg3
    key Return
    resp "id 13 response 1 listitem -1 text 'abc'"
    say "style 3 ok (masked)"
    fs 'dialog|20|2|List caption|Row 0\n{FF0000}Row 1 red\nRow {00FF00}2 green\n\nRow 4 after empty\n|Select|Close'
    wait_ui 'dialog_id=20 dialog_style=List dialog_rows=4 dialog_columns=1 dialog_selected=0' 5 || fail "style 2 not open with 4 rows"
    shot dlg2
    key Down; key Down; key Down
    wait_ui 'dialog_selected=3' 5 || fail "list keys did not move the selection"
    key Return
    resp "id 20 response 1 listitem 3 text 'Row 4 after empty'"
    say "style 2 ok (empty rows skipped, list keys)"
    fs 'dialog|40|4|Tablist|a\tb\tc\nlong cell number one\t2\t3\n{00FF00}x\ty\tz\tw\tv\n\tlead\ttrail\t|OK|Cancel'
    wait_ui 'dialog_id=40 dialog_style=Tablist dialog_rows=4 dialog_columns=3' 5 || fail "style 4 not open (3 columns)"
    shot dlg4
    key Escape
    resp "id 40 response 0 listitem 0 text 'a'"
    say "style 4 ok"
    fs 'dialog|50|5|Headers|H1\tHeader two\tH3\nr1c1\tr1c2\tr1c3\nr2\t\tr2c3|OK|Cancel'
    wait_ui 'dialog_id=50 dialog_style=TablistHeaders dialog_rows=2 dialog_columns=3' 5 || fail "style 5 not open"
    shot dlg5
    key Down
    key Return
    resp "id 50 response 1 listitem 1 text 'r2'"
    say "style 5 ok"
    # Mouse: button 2 of a style-0 dialog (230×103 centred at 1280×720: Cancel at 650–750 × 377–407).
    fs 'dialog|60|0|Mouse|Click cancel|OK|Cancel'
    wait_ui 'dialog_id=60' 5 || fail "dialog 60 not open"
    sleep 0.5
    click 700 392
    resp 'id 60 response 0 listitem -1'
    say "mouse click on button 2 ok"
    # List mouse (MR-M8A-3, measured on the original): a click selects, a double click
    # answers with that row.
    fs 'dialog|21|2|List caption|Row 0\n{FF0000}Row 1 red\nRow {00FF00}2 green\n\nRow 4 after empty\n|Select|Close'
    wait_ui 'dialog_id=21' 5 || fail "dialog 21 not open"
    sleep 0.5
    click 500 286; sleep 1.2
    wait_ui 'dialog_selected=1' 5 || fail "click did not select row 1"
    click 500 286; sleep 1
    wait_ui 'dialog_id=21' 2 || fail "a second single click 1 s later answered (the original does not)"
    DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool mousemove 500 304 click --repeat 2 --delay 80 1
    resp "id 21 response 1 listitem 2 text 'Row 2 green'"
    say "list mouse ok (click selects, double click answers)"

    # 4. Textdraws and selection.
    fs 'td|0|100.0|150.0|1|1|~g~M8A ~w~textdraw'
    TD0=$(grep -E "\[ui-fs\] $SEQ td 0 = " "$E2E_LOG" | tail -n 1 | sed -E 's/.* = ([0-9]+).*/\1/')
    fs 'td|1|320.0|300.0|2|2|Centred'
    fs 'td|2|500.0|20.0|4|1|LD_SPAC:white'
    TD2=$(grep -E "\[ui-fs\] $SEQ td 2 = " "$E2E_LOG" | tail -n 1 | sed -E 's/.* = ([0-9]+).*/\1/')
    fs 'tdsize|2|100.0|60.0'
    wait_ui 'textdraws=3' 5 || fail "textdraws not shown"
    sleep 1
    shot sprite
    fs 'tdsel|2|1'
    fs 'select|FF0000FF'
    mouse 1100 80
    shot spritesel
    click 1100 80
    e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id $TD2\b" 10 || fail "no click on the sprite textdraw"
    key Escape
    e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id 65535" 10 || fail "ESC did not cancel"
    fs 'tdhide|2'
    wait_ui 'textdraws=2' 5 || fail "sprite textdraw not hidden"
    say "sprite textdraw ok (drawn, selectable)"
    say "textdraw 0 has server id $TD0"
    sleep 1
    shot textdraws
    fs 'tdsel|0|1'
    fs 'select|FF0000FF'
    wait_ui 'select=1' 5 || fail "selection mode not on"
    mouse 300 255
    wait_ui "hovered=$TD0 " 5 || say "warning: textdraw $TD0 not hovered at 300,255"
    shot select
    click 300 255
    e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id $TD0\b" 10 || fail "no click on textdraw $TD0"
    key Escape
    e2e_wait_log "\[ui-fs\] click-textdraw player $PID_ id 65535" 10 || fail "ESC did not cancel the selection (65535)"
    wait_ui 'select=0' 5 || fail "selection still on"
    fs 'tdset|0|~y~changed'
    fs 'tdhide|1'
    wait_ui 'textdraws=1' 5 || fail "textdraw 1 not hidden"
    say "textdraws ok (show, select, click, ESC, set string, hide)"

    # 5. Menu.
    fs 'menu|Shop|200.0|150.0|200.0|0.0|Apple;Pear;Plum'
    fs 'menushow'
    wait_ui 'menu=[0-9]+' 5 || fail "menu not current"
    sleep 1
    shot menu
    e2e_keydown space; sleep 0.2; e2e_keyup space
    if e2e_wait_log "\[ui-fs\] menu-row player $PID_ row [0-9]+" 6; then
        say "menu row selected: $(grep -E 'menu-row' "$E2E_LOG" | tail -n 1)"
    else
        fail "no menu row selection (RPC 132)"
    fi
    wait_ui 'menu=-1' 5 || fail "menu did not close after the selection"
    fs 'menushow'
    wait_ui 'menu=[0-9]+' 5 || fail "menu not shown again"
    e2e_keydown Return; sleep 0.2; e2e_keyup Return
    e2e_wait_log "\[ui-fs\] menu-exit player $PID_" 6 || fail "no menu exit (RPC 140)"
    say "menu ok"

    # 6. Death window, bot, nametag, bubble, scoreboard, game text.
    fs 'death|-|L|200'
    fs 'death|L|L|24'
    wait_ui 'deaths=2 death_visible=1' 5 || fail "death rows missing"
    sleep 0.5
    shot dw
    ( cd "$SAMP_RS_ROOT" && exec "$TB/debug/samp-bot" "127.0.0.1:$E2E_PORT" --name Bot1 --seconds 120 ) > "$OUT/bot.out" 2>&1 &
    BOT_PID=$!
    e2e_wait_log "\[ui-fs\] spawn [0-9]+" 30 || true
    e2e_wait_log "connect [0-9]+ Bot1" 30 || fail "bot did not join"
    sleep 3
    fs 'pos|B1|1696.0|-2600.0|13.5|270.0'
    fs 'color|B1|00FF00FF'
    fs 'armour|B1|100'
    fs 'death|B1|L|31'
    fs 'bubble|B1|FFFF00FF|50.0|30000|Hello from the bot bubble'
    wait_ui 'bubbles=1' 10 || fail "bubble not stored"
    sleep 2
    wait_ui 'tags=1' 5 || fail "bot not visible to the UI"
    shot tags
    fs 'gametext|3|5000|~r~M8A ~w~game text'
    key Tab
    wait_ui 'scoreboard_open=1 scoreboard_rows=2' 5 || fail "scoreboard not open with 2 rows"
    sleep 4
    shot scoreboard
    key Tab
    wait_ui 'scoreboard_open=0' 5 || fail "scoreboard did not close"
    say "death window, bot, bubble, scoreboard ok"
    # The chat input last: opening it resizes the chat GUI dialog for good (as in the
    # original), which the other captures must not see.
    e2e_key t
    DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "hello"
    sleep 0.5
    shot input
    e2e_key Escape
    sleep 0.5
    # INPUT-1 (measured on the original): samp sends a chat line on the Enter key-up, so a
    # dialog the server shows in reply is not answered by that Enter, even with the key held
    # 300 ms; a dialog shown from OnDialogResponse is not answered by the Enter that closed
    # the first one either. No RPC 62 may come within 2 s.
    no_resp() { sleep 2; grep -qE "\[ui-fs\] dialog-response player $PID_ id $1 " "$E2E_LOG" && fail "$2"; true; }
    enter_held() { e2e_keydown Return; sleep 0.3; e2e_keyup Return; }
    # T + clear the line (closing the input with ESC keeps its text, as in the original) + TEXT.
    chat_line() {
        e2e_key t; sleep 0.5
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool key --repeat 12 --delay 30 BackSpace; sleep 0.2
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool type --delay 60 -- "$1"; sleep 0.3
    }
    fs 'chatdlg|90|1|0|0'
    chat_line "/dlg"
    enter_held
    e2e_wait_log "\[ui-fs\] command $PID_ '/dlg'" 5 || fail "chat command /dlg not sent"
    wait_ui 'dialog_open=1 dialog_id=90 dialog_style=Input' 5 || fail "reply dialog 90 not open (last answer: $(grep -E "dialog-response player $PID_ id 90 " "$E2E_LOG" | tail -n 1))"
    no_resp 90 "the Enter that sent the chat line answered the reply dialog 90"
    wait_ui 'dialog_id=90' 1 || fail "reply dialog 90 closed by the chat Enter"
    fs 'chatdlg|-1|0|0|0'
    key Escape
    resp 'id 90 response 0'
    fs 'chatdlg|93|0|0|1'
    chat_line "plain"
    enter_held
    e2e_wait_log "\[ui-fs\] text $PID_ 'plain'" 5 || fail "chat text not sent"
    wait_ui 'dialog_open=1 dialog_id=93' 5 || fail "reply dialog 93 not open (last answer: $(grep -E "dialog-response player $PID_ id 93 " "$E2E_LOG" | tail -n 1))"
    no_resp 93 "the Enter that sent the chat text answered the reply message box 93"
    fs 'chatdlg|-1|0|0|0'
    key Escape
    resp 'id 93 response 0'
    fs 'chaindlg|91|92|1'
    fs 'dialog|91|1|First|First dialog:|OK|Cancel'
    wait_ui 'dialog_id=91' 5 || fail "dialog 91 not open"
    enter_held
    resp 'id 91 response 1'
    wait_ui 'dialog_open=1 dialog_id=92' 5 || fail "chained dialog 92 not open (last answer: $(grep -E "dialog-response player $PID_ id 92 " "$E2E_LOG" | tail -n 1))"
    no_resp 92 "the Enter that answered dialog 91 also answered the chained dialog 92"
    fs 'chaindlg|-1|0|0'
    key Escape
    resp 'id 92 response 0'
    say "INPUT-1 ok (chat Enter / dialog chain do not answer the next dialog)"
    [[ -n "${M8A_HOLD:-}" ]] && sleep "$M8A_HOLD"
    kill "$BOT_PID" 2>/dev/null
    e2e_ctl quit > /dev/null
    for _ in $(seq 1 30); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 1; done
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
    [[ "${M8A_D3D:-0}" == 1 ]] && { cp -f "$GTA_DIR/d3d.jsonl" "$OUT/d3d-all.jsonl" 2>/dev/null; cp -f "$GTA_DIR/trace.jsonl" "$OUT/trace.jsonl" 2>/dev/null; rm -f "$GTA_DIR/samp-tracer.dll"; }
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

if [[ "${M8A_D3D:-0}" == 1 ]]; then
    export SAMP_TRACER_PORT=47711
    say "building samp-tracer from $SAMP_RS_ROOT (into $ROOT/target/main-tracer)"
    ( cd "$SAMP_RS_ROOT" && CARGO_TARGET_DIR="$ROOT/target/main-tracer" cargo build -q --release \
        --target i686-pc-windows-gnu -p samp-tracer ) > "$OUT/tracer-build.out" 2>&1 \
        || { cat "$OUT/tracer-build.out"; say "tracer build failed"; exit 1; }
    rm -f "$GTA_DIR/samp-tracer.dll" "$GTA_DIR/d3d.jsonl" "$GTA_DIR/trace.jsonl"
    cp "$ROOT/target/main-tracer/i686-pc-windows-gnu/release/samp_tracer.dll" "$GTA_DIR/samp-tracer.dll"
    rm -f "$OUT"/d3d-*.jsonl
fi
say "building (i686-pc-windows-gnu release + samp-dump + samp-bot)"
{ (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
  (cd "$ROOT" && cargo build -p samp-dump -p samp-bot); } > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
docs="$WINEPREFIX/drive_c/users/$USER/Documents"
if [[ -L "$docs" ]]; then
    say "$docs is a symlink ($(readlink "$docs")); give the lane its own Documents folder first"; exit 1
fi
mkdir -p "$SAMP_DIR"
printf 'pagesize=10\r\nfpslimit=48\r\nmulticore=1\r\naudioproxyoff=0\r\n' > "$CFG"
rm -f "$CLIENT_LOG" "$OUT"/*.png
m8a_server_start || exit 1
say "server up on port $E2E_PORT"
dumpcap -q -i lo -f "udp port $E2E_PORT" -w "$OUT/m8a.pcapng" > "$OUT/dumpcap.out" 2>&1 &
CAP_PID=$!
sleep 1
E2E_SERVER="$E2E_SERVER" e2e_headless_run bash "$0" --inner
rc=$?
sleep 1
kill "$CAP_PID" 2>/dev/null; wait "$CAP_PID" 2>/dev/null; CAP_PID=""
if (( rc == 0 )); then
    # 7. What we sent: decode only our client's port (the bot shares the capture).
    # Our client's UDP port: the connection the server gave to RustClient.
    cport=$(grep -B3 "\[e2e\] connect id=[0-9]* name=$NAME" "$E2E_LOG" | grep -oE 'incoming connection: 127.0.0.1:[0-9]+' | tail -n 1 | cut -d: -f3)
    tshark -r "$OUT/m8a.pcapng" -Y "udp.port==$cport" -w "$OUT/client.pcapng" 2>/dev/null
    "$TB/debug/samp-dump" "$OUT/client.pcapng" --port "$E2E_PORT" > "$OUT/m8a.dump" 2>&1 || true
    grep -E '^C>S .*rpc +(62|83|132|140|155) ' "$OUT/m8a.dump" > "$OUT/m8a-ui-rpcs.txt"
    sed 's/^/    /' "$OUT/m8a-ui-rpcs.txt" | head -40
    hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
    # 62: u16 id, u8 response, i16 listitem, u8 len, text (reliable ordered).
    grep -qE "^C>S rel9 .*rpc +62 .* 0b0001ffff08$(hex 'hi there')\$" "$OUT/m8a.dump" \
        || { say "FAIL: RPC 62 for dialog 11 'hi there' missing or not rel9 u16/u8/i16/u8+text"; rc=1; }
    grep -qE "^C>S rel9 .*rpc +62 .* 1400010300" "$OUT/m8a.dump" \
        || { say "FAIL: RPC 62 list answer (id 20, listitem 3) missing"; rc=1; }
    grep -qE "^C>S rel9 .*rpc +83 .* [0-9a-f]{2}00\$" "$OUT/m8a.dump" || { say "FAIL: RPC 83 click missing"; rc=1; }
    grep -qE "^C>S rel9 .*rpc +83 .* ffff\$" "$OUT/m8a.dump" || { say "FAIL: RPC 83 cancel (0xFFFF) missing"; rc=1; }
    grep -qE "^C>S rel8 .*rpc +132 " "$OUT/m8a.dump" || { say "FAIL: RPC 132 missing / not rel8"; rc=1; }
    grep -qE "^C>S rel8 .*rpc +140 " "$OUT/m8a.dump" || { say "FAIL: RPC 140 missing / not rel8"; rc=1; }
    n155=$(grep -cE "^C>S rel8 .*rpc +155 " "$OUT/m8a.dump")
    (( n155 >= 2 )) || { say "FAIL: RPC 155 sent $n155 times while the scoreboard was open ~4 s (want ≥ 2)"; rc=1; }
    (( rc == 0 )) && say "capture: RPC 62/83/132/140/155 layouts and reliability as specified ($n155 × 155)"
fi
exit "$rc"
