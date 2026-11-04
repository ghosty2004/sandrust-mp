#!/usr/bin/env bash
# M1 acceptance test: the samp-rs client boots GTA SA straight into gameplay.
#  1. tools/run-client.sh (build, launcher, injected DLL) under a timeout
#  2. poll the control channel (127.0.0.1:47700) with `state` until the local
#     player position is non-zero (fail after ${M1_TIMEOUT:-120} s)
#  3. check samp-rs.log contains per-second tick lines
#  4. `ping` and `quit`, wait for the game to exit, clean up Wine
# Exit status 0 = pass, 1 = fail. No screenshots: logs + control channel only.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
source "$ROOT/tools/wine-env.sh"
LOG="$GTA_DIR/samp-rs.log"
OUT="$(mktemp -d "${TMPDIR:-/tmp}/samp-rs-m1.XXXXXX")"
TIMEOUT="${M1_TIMEOUT:-120}"
CTL_PORT=47700

say() { printf '[m1] %s\n' "$*"; }

# One request/response on the control channel; prints the reply line.
ctl() {
    python3 -I - "$CTL_PORT" "$1" <<'PY'
import socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.settimeout(5)
    s.sendall((sys.argv[2] + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = s.recv(4096)
        if not chunk:
            break
        buf += chunk
    print(buf.decode(errors="replace").strip())
except OSError:
    sys.exit(1)
PY
}

RUN_PID=""
finish() {
    local rc=$1
    if [[ -n "$RUN_PID" ]] && kill -0 "$RUN_PID" 2>/dev/null; then
        kill "$RUN_PID" 2>/dev/null
    fi
    wineserver -k 2>/dev/null || true
    sleep 1
    cp -f "$LOG" "$OUT/samp-rs.log" 2>/dev/null || true
    if pgrep -x 'gta_sa.exe|samp-rs-launche' >/dev/null; then
        say "warning: Wine processes still present after wineserver -k"
    fi
    if [[ $rc -eq 0 ]]; then say "PASS"; else
        say "FAIL — last log lines:"; tail -n 25 "$LOG" 2>/dev/null | sed 's/^/    /'
        say "run-client output and log copy: $OUT/"
    fi
    exit "$rc"
}
trap 'finish 1' INT TERM

# Build first, so the timeout only covers the game run.
say "building (i686-pc-windows-gnu, release)"
(cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) >"$OUT/build.out" 2>&1 \
    || { cat "$OUT/build.out"; say "build failed"; finish 1; }

if ctl ping >/dev/null 2>&1; then
    say "something already answers on port $CTL_PORT; refusing to run"; finish 1
fi
rm -f "$LOG"
say "starting client (timeout $((TIMEOUT + 60)) s)"
SKIP_BUILD=1 timeout --kill-after=10 "$((TIMEOUT + 60))" "$ROOT/tools/run-client.sh" >"$OUT/run.out" 2>&1 &
RUN_PID=$!

start=$SECONDS
state=""
ok=0
while (( SECONDS - start < TIMEOUT )); do
    if ! kill -0 "$RUN_PID" 2>/dev/null; then
        say "client exited early"; break
    fi
    if reply="$(ctl state 2>/dev/null)" && [[ -n "$reply" ]]; then
        [[ "$reply" != "$state" ]] && say "after $((SECONDS - start)) s: $reply"
        state="$reply"
        if [[ "$reply" =~ pos=(-?[0-9.]+),(-?[0-9.]+),(-?[0-9.]+) ]]; then
            if awk -v x="${BASH_REMATCH[1]}" -v y="${BASH_REMATCH[2]}" -v z="${BASH_REMATCH[3]}" \
                'BEGIN { exit !(x != 0 || y != 0 || z != 0) }'; then
                ok=1; break
            fi
        fi
    fi
    sleep 2
done
if (( ! ok )); then say "no non-zero player position within $TIMEOUT s"; finish 1; fi
say "player position reached after $((SECONDS - start)) s"

# Wait (up to 20 s) for at least two per-second tick lines from the frame hook.
ticks=0
for _ in $(seq 1 20); do
    ticks=$(grep -c '\] tick [0-9]\+ (' "$LOG" 2>/dev/null || true)
    (( ticks >= 2 )) && break
    sleep 1
done
if (( ticks < 2 )); then say "log shows $ticks tick lines (want >= 2)"; finish 1; fi
say "log shows $ticks per-second tick lines: $(grep '\] tick [0-9]\+ (' "$LOG" | tail -n 1 | sed 's/.*\] tick/tick/')"

st="$(ctl state 2>/dev/null)"
say "state: $st"
[[ "$st" =~ ticks=[1-9] ]] || { say "control channel reports no frame ticks"; finish 1; }
[[ "$st" == *frame_hook=installed* ]] || { say "frame hook not installed at 0x53EB12"; finish 1; }
say "call site: $(ctl 'peek 0x53EB12 5' 2>/dev/null)"

pong="$(ctl ping 2>/dev/null)"
[[ "$pong" == "pong" ]] || { say "ping answered '$pong'"; finish 1; }
bye="$(ctl quit 2>/dev/null)"
[[ "$bye" == "bye" ]] || { say "quit answered '$bye'"; finish 1; }
say "quit acknowledged; waiting for the game to exit"
for _ in $(seq 1 30); do
    kill -0 "$RUN_PID" 2>/dev/null || break
    sleep 1
done
if kill -0 "$RUN_PID" 2>/dev/null; then say "game did not exit within 30 s after quit"; finish 1; fi
wait "$RUN_PID"; rc=$?
say "run-client.sh exited with $rc"
grep -q 'quit requested' "$LOG" || { say "log lacks the quit record"; finish 1; }
finish 0
