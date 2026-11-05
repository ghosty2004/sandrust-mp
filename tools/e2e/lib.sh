#!/usr/bin/env bash
# Shared helpers for end-to-end tests. Source from tools/e2e/<id>.sh.
#   e2e_server_start / e2e_server_stop  — isolated server copy in target/e2e-server
#   e2e_wait_log PATTERN SECONDS        — wait for a server log line (regex)
#   e2e_ctl CMD                         — send a line to the client DLL control channel
#   e2e_cleanup                         — stop server, bots and Wine
E2E_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Parallel test lanes: set E2E_TAG (e.g. "trace") to get a separate server dir,
# headless env file and logs; set E2E_PORT / E2E_CTL_PORT to distinct values too.
E2E_TAG="${E2E_TAG:-main}"
# Checkout holding the gitignored server/ and data/ (SAMP_RS_ROOT, for git worktrees).
E2E_DATA_ROOT="${SAMP_RS_ROOT:-$E2E_ROOT}"
E2E_SERVER="${E2E_SERVER:-$E2E_ROOT/target/e2e-server-$E2E_TAG}"
E2E_LOG="$E2E_SERVER/e2e-server.log"
E2E_PORT="${E2E_PORT:-7777}"
E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"

e2e_server_prepare() {
    mkdir -p "$E2E_SERVER"
    rsync -a --delete --exclude 'log.txt' --exclude 'e2e-server.log' "$E2E_DATA_ROOT/server/" "$E2E_SERVER/"
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$E2E_ROOT/tools/e2e/probe/e2eprobe.pwn" \
        -iqawno/include -ofilterscripts/e2eprobe.amx -d0 > /dev/null )
    python3 -I - "$E2E_SERVER/config.json" "$E2E_PORT" <<'PY'
import json, sys
p, port = sys.argv[1], int(sys.argv[2])
d = json.load(open(p))
d["announce"] = False
d["network"]["port"] = port
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe"]
json.dump(d, open(p, "w"), indent=4)
PY
}

e2e_server_start() {
    e2e_server_prepare
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 9>&- ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { echo "e2e: server did not start" >&2; return 1; }
}

e2e_server_stop() {
    [ -n "${E2E_SERVER_PID:-}" ] && kill "$E2E_SERVER_PID" 2>/dev/null; wait "${E2E_SERVER_PID:-}" 2>/dev/null || true
    # Match the binary path exactly, never a shell command line.
    pgrep -x omp-server | while read -r pid; do
        [ "$(readlink -f "/proc/$pid/exe" 2>/dev/null)" = "$E2E_SERVER/omp-server" ] && kill "$pid"
    done
    true
}

# e2e_wait_log REGEX SECONDS — succeeds once a matching line appears.
e2e_wait_log() {
    local pat="$1" secs="$2" i
    for ((i = 0; i < secs * 2; i++)); do
        grep -qE "$pat" "$E2E_LOG" 2>/dev/null && return 0
        sleep 0.5
    done
    return 1
}

# e2e_ctl "state" — prints the control channel's reply (one line), empty on failure.
e2e_ctl() {
    python3 -I - "$E2E_CTL_PORT" "$1" <<'PY'
import socket, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=3)
    s.sendall((sys.argv[2] + "\n").encode())
    print(s.makefile().readline().strip())
except OSError:
    pass
PY
}

e2e_cleanup() {
    # Only this lane's bots (their command line targets this lane's server port).
    local pid
    for pid in $(pgrep -x samp-bot 2>/dev/null); do
        tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | grep -q ":$E2E_PORT\b" && kill "$pid" 2>/dev/null
    done
    e2e_headless_stop
    e2e_server_stop
    ( source "$E2E_ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true  # uses $WINEPREFIX (override per lane)
}

# ---- Isolated headless display (gamescope) -------------------------------
# e2e_headless_start CMD...  — run CMD inside a headless gamescope (1280x720 by
#                              default, E2E_RES=WxH); nothing appears on the
#                              user's desktop. Sets E2E_HEADLESS_DISPLAY (X11)
#                              and E2E_GAMESCOPE (wayland socket for gamescopectl).
# e2e_shot FILE.png          — screenshot of the headless display only.
# e2e_key KEY...             — send key presses (xdotool names, e.g. shift Left w).
# e2e_keydown/e2e_keyup KEY  — hold / release a key.
# Several displays at once (e.g. two games on one server, DIFF-3): give each
# e2e_headless_start call its own E2E_DISPLAY_TAG (default: E2E_TAG; it names the
# env file, child script and gamescope log), save E2E_GAMESCOPE_PID,
# E2E_HEADLESS_DISPLAY and E2E_GAMESCOPE after each start, and set them back
# before e2e_shot / e2e_key / e2e_headless_stop for that display.
E2E_HEADLESS_ENV="$E2E_ROOT/target/e2e-headless-$E2E_TAG.env"

e2e_headless_start() {
    local res="${E2E_RES:-1280x720}" w h child dtag="${E2E_DISPLAY_TAG:-$E2E_TAG}"
    w="${res%x*}"; h="${res#*x}"
    E2E_HEADLESS_ENV="$E2E_ROOT/target/e2e-headless-$dtag.env"
    mkdir -p "$E2E_ROOT/target"; rm -f "$E2E_HEADLESS_ENV"
    child="$E2E_ROOT/target/e2e-headless-child-$dtag.sh"
    {
        echo '#!/usr/bin/env bash'
        echo "env | grep -E '^(DISPLAY|GAMESCOPE_WAYLAND_DISPLAY)=' > '$E2E_HEADLESS_ENV'"
        printf 'exec'; printf ' %q' "$@"; echo
    } > "$child"
    chmod +x "$child"
    # Own process group, so e2e_headless_stop can take down gamescope and its child.
    # 9>&-: a lane lock held on fd 9 (with-lock.sh) must not outlive the run through gamescope.
    setsid gamescope --backend headless -W "$w" -H "$h" -w "$w" -h "$h" -- "$child" > "$E2E_ROOT/target/e2e-gamescope-$dtag.log" 2>&1 9>&- &
    E2E_GAMESCOPE_PID=$!
    local i
    for ((i = 0; i < 40; i++)); do [ -s "$E2E_HEADLESS_ENV" ] && break; sleep 0.25; done
    [ -s "$E2E_HEADLESS_ENV" ] || { echo "e2e: gamescope did not start" >&2; return 1; }
    E2E_HEADLESS_DISPLAY="$(sed -n 's/^DISPLAY=//p' "$E2E_HEADLESS_ENV")"
    E2E_GAMESCOPE="$(sed -n 's/^GAMESCOPE_WAYLAND_DISPLAY=//p' "$E2E_HEADLESS_ENV")"
    export E2E_HEADLESS_DISPLAY E2E_GAMESCOPE
}

e2e_headless_stop() {
    [ -n "${E2E_GAMESCOPE_PID:-}" ] || return 0
    kill -TERM -- "-$E2E_GAMESCOPE_PID" 2>/dev/null
    local i; for ((i = 0; i < 20; i++)); do kill -0 "$E2E_GAMESCOPE_PID" 2>/dev/null || break; sleep 0.25; done
    kill -KILL -- "-$E2E_GAMESCOPE_PID" 2>/dev/null
    wait "$E2E_GAMESCOPE_PID" 2>/dev/null || true
    E2E_GAMESCOPE_PID=
}

# e2e_headless_run CMD... — run CMD with the isolated display (games launched by
# the tools/run-*.sh scripts go there directly, no Wine virtual desktop); returns
# CMD's exit code and always stops gamescope afterwards.
e2e_headless_run() {
    e2e_headless_start sleep infinity || return 1
    local rc=0
    env -u WAYLAND_DISPLAY DISPLAY="$E2E_HEADLESS_DISPLAY" E2E_HEADLESS=1 "$@" || rc=$?
    e2e_headless_stop
    return "$rc"
}

e2e_shot() { GAMESCOPE_WAYLAND_DISPLAY="$E2E_GAMESCOPE" gamescopectl screenshot "$1" > /dev/null 2>&1; sleep 1; [ -s "$1" ]; }
e2e_key() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool key --delay 80 "$@"; }
e2e_keydown() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool keydown "$1"; }
e2e_keyup() { DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool keyup "$1"; }
