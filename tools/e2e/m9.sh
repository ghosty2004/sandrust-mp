#!/usr/bin/env bash
# M9 acceptance: audio streams (RPC 41/42), the stream volume, the radio calls, the title line and
# PlayerPlaySound (RPC 16) of our in-game client (lane "ui": prefix-ui, gta-sa-ui, port 7800,
# control 47710; run it as `flock target/locks/ui.lock tools/e2e/m9.sh`).
#  1. build client + launcher + samp-tracer; isolated server copy with the e2e probe and
#     tools/trace/uifs.pwn; tools/trace/icyserve.py serves a silent MP3 (audio stays inaudible:
#     Wine plays into the private null sink of tools/wine-env.sh)
#  2. inside a headless gamescope: our client with samp-tracer.dll loaded first
#     (SAMP_TRACER_BASS=1: every bass.dll call of the client module is logged), spawn
#  3. tools/e2e/m9scen.sh: global stream with ICY title, replace, positional at ~0/20/60 m,
#     another global stream, GTA menu, stop, unreachable URL + stop, sounds 1052 / 5 / 0 / 2000 / 0
#  4. checks on our side: chat lines, `audio` control state, chatlog
#  5. tools/trace/bass_diff.py: BASS call log, radio calls and sound script commands per step
#     against the original's run of the same scenario (tools/e2e/m9-orig.sh → target/m9-orig)
# Exit 0 = pass. Output: target/m9/ (trace.jsonl, bass-diff.txt, screenshots, logs).
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
source "$ROOT/tools/e2e/m9scen.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUT="$ROOT/target/m9"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
ICY_PORT="${M9_ICY_PORT:-18800}"
export URL_BASE="http://127.0.0.1:$ICY_PORT"
say() { printf '[m9] %s\n' "$*"; }

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

m9_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/trace/uifs.pwn" \
        -iqawno/include -ofilterscripts/traceui.amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/traceui"]
json.dump(d, open(p, "w"), indent=4)
PY
    mkdir -p "$E2E_SERVER/scriptfiles"
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { say "server did not start"; return 1; }
    e2e_wait_log '\[ui-fs\] loaded' 5 || { say "uifs filterscript not loaded"; return 1; }
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; exit 1; }
    SF="$E2E_SERVER/scriptfiles/uicmd.txt"
    SEQ=0
    fs() {
        SEQ=$((SEQ + 1))
        printf '%d %s\n' "$SEQ" "$1" >> "$SF"
        e2e_wait_log "\[ui-fs\] $SEQ done" 10 || fail "filterscript command $SEQ ($1) not run"
    }
    key() { e2e_key "$@"; }
    mark() { E2E_CTL_PORT="$SAMP_TRACER_PORT" e2e_ctl "mark $*" > /dev/null; say "mark $*"; }
    export SAMP_RS_PRE_DLL="$GTA_DIR/samp-tracer.dll"
    SKIP_BUILD=1 timeout --kill-after=10 600 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3
    spawned=0
    for _ in 1 2 3 4 5 6; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 4; then spawned=1; break; fi
    done
    (( spawned )) || fail "no spawn after Shift"
    mark spawned
    say "audio after boot: $(e2e_ctl audio)"
    e2e_ctl audio | grep -q 'ready=1' || fail "BASS not ready: $(e2e_ctl audio)"

    # Checks in the middle of the scenario run in its pauses (it sleeps after each step).
    ( sleep 9; e2e_ctl audio > "$OUT/audio-global.txt"; e2e_shot "$OUT/title.png" ) &
    PROBE=$!
    m9_scenario
    wait "$PROBE"
    say "audio during the global stream: $(cat "$OUT/audio-global.txt")"
    grep -q 'playing=1' "$OUT/audio-global.txt" || fail "stream not playing"
    # BASS hands the header over as "icy-name: TraceStation"; samp keeps everything after the colon.
    grep -q 'title="Trace Artist - Title [0-9]*" station=" TraceStation"' "$OUT/audio-global.txt" \
        || fail "title / station not parsed: $(cat "$OUT/audio-global.txt")"
    a="$(e2e_ctl audio)"
    say "audio at the end: $a"
    grep -q 'playing=0' <<< "$a" || fail "stream still playing after audiostop"
    grep -q 'mission_state=0' <<< "$a" || fail "mission audio state not reset by sound 0"
    chat="$(ctl_chat 'chat 60')"
    printf '%s\n' "$chat" > "$OUT/chat.txt"
    n=$(grep -c 'line type=4 .*text="Audio stream: http://' <<< "$chat")
    (( n == 6 )) || fail "want 6 'Audio stream:' lines (type 4), got $n"
    say "chat: $n 'Audio stream:' lines"
    e2e_ctl quit > /dev/null
    sleep 5
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUT"
rc=1
ICY_PID=""
finish() {
    e2e_headless_stop
    [[ -n "$ICY_PID" ]] && { kill "$ICY_PID" 2>/dev/null; wait "$ICY_PID" 2>/dev/null; }
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null || true
    cp -f "$GTA_DIR/trace.jsonl" "$OUT/trace.jsonl" 2>/dev/null || true
    cp -f "$GTA_DIR/samp-tracer.log" "$OUT/samp-tracer.log" 2>/dev/null || true
    rm -f "$GTA_DIR/samp-tracer.dll"
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

say "building (i686-pc-windows-gnu release: client, launcher, tracer)"
(cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher -p samp-tracer) \
    > "$OUT/build.out" 2>&1 || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$GTA_DIR/samp-tracer.dll" "$GTA_DIR/trace.jsonl" "$GTA_DIR/d3d.jsonl" "$CLIENT_LOG" "$OUT"/*.png
cp "$TB/i686-pc-windows-gnu/release/samp_tracer.dll" "$GTA_DIR/samp-tracer.dll"
m9_silence_mp3 "$ROOT/target/m9-silence.mp3" || { say "ffmpeg failed"; exit 1; }
python3 -I "$ROOT/tools/trace/icyserve.py" "$ICY_PORT" "$ROOT/target/m9-silence.mp3" "$OUT/icy.log" &
ICY_PID=$!
m9_server_start || exit 1
say "server up on port $E2E_PORT, stream server on $ICY_PORT"
E2E_SERVER="$E2E_SERVER" e2e_headless_run bash "$0" --inner
rc=$?
cp -f "$GTA_DIR/trace.jsonl" "$OUT/trace.jsonl" 2>/dev/null || true
if (( rc == 0 )); then
    UA="$(grep -c 'User-Agent: SA-MP/0.3' "$OUT/icy.log")"
    (( UA >= 5 )) || { say "FAIL: stream requests without 'User-Agent: SA-MP/0.3' ($UA)"; rc=1; }
    ORIG="${M9_ORIG:-$ROOT/target/m9-orig/trace.jsonl}"
    if [[ -f "$ORIG" ]]; then
        python3 -I "$ROOT/tools/trace/bass_diff.py" "$ORIG" "$OUT/trace.jsonl" > "$OUT/bass-diff.txt" 2>&1
        d=$?
        cat "$OUT/bass-diff.txt"
        (( d == 0 )) || { say "FAIL: BASS / audio call diff against the original (target/m9/bass-diff.txt)"; rc=1; }
    else
        say "FAIL: no original BASS call log ($ORIG). Regenerate it (trace lane, ~1-2 min) with:"
        say "    $SAMP_RS_ROOT/tools/trace/with-lock.sh $ROOT/tools/e2e/m9-orig.sh"
        say "  (writes $ROOT/target/m9-orig/trace.jsonl; or point M9_ORIG at an existing capture)"
        rc=1
    fi
fi
exit "$rc"
