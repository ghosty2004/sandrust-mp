#!/usr/bin/env bash
# SAA-2 acceptance test: our client serves game files from the user's samp.saa
# exactly like samp.dll (CreateFileA redirection by lower-case basename), incl.
# samp's own main.scm and script.img, and the game still reaches gameplay.
#  1. build; isolated server copy with the e2e probe filterscript (lane "main",
#     port 7777, control port 47700); samp.saa must be next to gta_sa.exe
#  2. run the client inside a headless gamescope, join, class selection, Shift
#  3. assert the probe saw the spawn and `state` is IDLE with a local ped
#  4. `files` (control channel): mode=archive, signature valid, >= 23 distinct
#     entries served, among them main.scm, script.img, gta.dat, peds.ide,
#     loadscs.txd and vehicle.txd; no failed opens; each served size equals the
#     archive entry's size from the client log; our fallback script not used
#  5. screenshot target/saa-spawn.png, quit
# Exit status 0 = pass. Usage: tools/e2e/saa.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
NAME=RustClient
CLIENT_LOG="$GTA_DIR/samp-rs.log"
OUT="$ROOT/target/saa"
REQUIRED=(main.scm script.img gta.dat peds.ide loadscs.txd vehicle.txd)
MIN_DISTINCT=23
say() { printf '[saa] %s\n' "$*"; }

# `files` reply: header line + one line per served entry (read until quiet).
ctl_files() {
    python3 -I - "$E2E_CTL_PORT" <<'PY'
import socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.sendall(b"files\n")
    s.settimeout(1.5)
    buf = b""
    while True:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            break
        buf += chunk
    sys.stdout.write(buf.decode(errors="replace"))
except OSError:
    pass
PY
}

# ---- inner part: runs inside the headless display ------------------------
if [[ "${1:-}" == --inner ]]; then
    fail() { say "FAIL: $*"; exit 1; }
    SKIP_BUILD=1 timeout --kill-after=10 300 "$ROOT/tools/run-client.sh" "127.0.0.1:$E2E_PORT" "$NAME" \
        > "$OUT/run.out" 2>&1 &
    RUN_PID=$!
    e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no connect within 120 s"
    PID_="$(grep -E "\[e2e\] connect id=[0-9]+ name=$NAME" "$E2E_LOG" | tail -n 1 | sed -E 's/.*connect id=([0-9]+).*/\1/')"
    say "server sees $NAME as id $PID_"
    e2e_wait_log "\[e2e\] class id=$PID_ " 30 || fail "no class request"
    sleep 3   # past the 2000 ms guard
    spawned=0
    for attempt in 1 2 3; do
        e2e_key shift
        if e2e_wait_log "\[e2e\] spawn id=$PID_\b" 8; then spawned=1; break; fi
        say "no spawn after Shift press $attempt"
    done
    (( spawned )) || fail "no spawn after Shift"
    sleep 3
    e2e_shot "$ROOT/target/saa-spawn.png" || say "warning: spawn screenshot failed"
    st="$(e2e_ctl state)"
    say "state: $st"
    [[ "$st" == *"game_state=9 "* && "$st" == *"frame_hook=installed"* && "$st" =~ ped=0x[0-9a-f]+ ]] \
        || fail "not in gameplay"
    net="$(e2e_ctl net)"
    [[ "$net" == *"spawned=true"* ]] || fail "client does not report spawned: $net"

    files="$(ctl_files)"
    printf '%s\n' "$files" > "$OUT/files.txt"
    header="$(printf '%s\n' "$files" | head -n 1)"
    say "files: $header"
    [[ "$header" == "files mode=archive signature=valid "* ]] || fail "not serving from samp.saa: $header"
    distinct="$(sed -nE 's/.* distinct=([0-9]+).*/\1/p' <<< "$header")"
    failed="$(sed -nE 's/.* failed=([0-9]+).*/\1/p' <<< "$header")"
    lines="$(printf '%s\n' "$files" | grep -c '^file name=')"
    (( lines == distinct )) || fail "header says $distinct entries, got $lines lines"
    (( distinct >= MIN_DISTINCT )) || fail "only $distinct archive entries served (want >= $MIN_DISTINCT)"
    [[ "$failed" == 0 ]] || fail "$failed failed archive opens"
    for n in "${REQUIRED[@]}"; do
        line="$(printf '%s\n' "$files" | grep -E "^file name=$n " | head -n 1)"
        [[ -n "$line" ]] || fail "$n was not served from the archive"
        say "  $line"
    done
    # Every served size must equal the entry size the client listed at attach.
    while read -r name size; do
        want="$(grep -E "files:   slot +[0-9]+ hash 0x[0-9a-f]+ size +[0-9]+ $name\$" "$CLIENT_LOG" | awk '{print $(NF-1)}' | head -n 1)"
        [[ "$want" == "$size" ]] || fail "$name served with size $size, archive lists ${want:-nothing}"
    done < <(printf '%s\n' "$files" | sed -nE 's/^file name=([^ ]+) hash=[^ ]+ size=([0-9]+) .*/\1 \2/p')
    say "all $distinct served entries match the archive sizes"
    grep -q "files: ARCHIVE mode" "$CLIENT_LOG" || fail "log lacks the ARCHIVE mode line"
    ! grep -q "serving our main.scm" "$CLIENT_LOG" || fail "our fallback main.scm was used"
    bye="$(e2e_ctl quit)"
    say "quit: $bye"
    for _ in $(seq 1 20); do kill -0 "$RUN_PID" 2>/dev/null || break; sleep 1; done
    say "all checks passed"
    exit 0
fi

# ---- outer part -----------------------------------------------------------
mkdir -p "$OUT"
rc=1
finish() {
    e2e_cleanup
    cp -f "$CLIENT_LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    cp -f "$E2E_LOG" "$OUT/server.log" 2>/dev/null || true
    if (( rc == 0 )); then say "PASS"; else
        say "FAIL — last client log lines:"; tail -n 25 "$CLIENT_LOG" 2>/dev/null | sed 's/^/    /'
        say "logs: $OUT/"
    fi
}
trap finish EXIT

[[ -f "$GTA_DIR/samp.saa" ]] || { say "no samp.saa in $GTA_DIR (copy it from your SA:MP install)"; exit 1; }
say "building (i686-pc-windows-gnu, release)"
(cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) > "$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; exit 1; }
[[ -f "$ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on port $E2E_CTL_PORT; refusing to run"; exit 1; fi
rm -f "$CLIENT_LOG" "$ROOT"/target/saa-*.png
e2e_server_start || exit 1
say "server up on port $E2E_PORT"
e2e_headless_run bash "$0" --inner
rc=$?
exit "$rc"
