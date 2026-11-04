#!/usr/bin/env bash
# Tracing lane (ENV-3): own Wine prefix, game copy, server port and control port,
# so it never collides with the main e2e lane (wine/prefix, 7777, 47700).
# Source from tools/trace/*.sh.
TRACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export WINEPREFIX="$TRACE_ROOT/wine/prefix-trace"
export GTA_DIR="$TRACE_ROOT/wine/gta-sa-trace"
export E2E_TAG=trace E2E_PORT=7790 E2E_CTL_PORT=47701
# Read by samp-tracer.dll inside the game (Wine passes the Unix environment through).
# With the original client the tracer serves the lane's control port (DIFF-1);
# with our client, samp_client.dll serves it (SAMP_RS_CTL_PORT) and the tracer
# moves to 47702 (trace_ports ours).
export SAMP_TRACER_PORT="$E2E_CTL_PORT"
trace_ports() {
    if [ "$1" = ours ]; then
        export SAMP_RS_CTL_PORT="$E2E_CTL_PORT" SAMP_TRACER_PORT=47702
        # Our client needs the auth table (third-party data, generated locally; see CLAUDE.md).
        [ -f "$TRACE_ROOT/data/auth_table.txt" ] \
            && SAMP_RS_AUTH_TABLE="$(winepath -w "$TRACE_ROOT/data/auth_table.txt" 2>/dev/null)" \
            && export SAMP_RS_AUTH_TABLE
    else
        export SAMP_TRACER_PORT="$E2E_CTL_PORT"
    fi
}
# Tracer-only commands (mark, peek) always go to the tracer.
trace_ctl() { E2E_CTL_PORT="$SAMP_TRACER_PORT" e2e_ctl "$1"; }
source "$TRACE_ROOT/tools/wine-env.sh"
source "$TRACE_ROOT/tools/e2e/lib.sh"
TRACE_OUT="$TRACE_ROOT/target/trace"
TRACE_BIN="$TRACE_ROOT/target/i686-pc-windows-gnu/release"

trace_say() { printf '[trace] %s\n' "$*"; }

# trace_lock — blocks until this process holds target/locks/trace.lock (on fd 9, so it lasts
# until the caller and its children exit). No-op when SAMP_RS_TRACE_LOCK_HELD=1 (the lock is
# already held by a parent, e.g. tools/trace/with-lock.sh).
trace_lock() {
    [ "${SAMP_RS_TRACE_LOCK_HELD:-0}" = 1 ] && return 0
    mkdir -p "$TRACE_ROOT/target/locks"
    exec 9> "$TRACE_ROOT/target/locks/trace.lock"
    if ! flock -n 9; then
        trace_say "waiting for the trace-lane lock (target/locks/trace.lock)"
        flock 9
    fi
    export SAMP_RS_TRACE_LOCK_HELD=1
}

# trace_build [ours] — tracer + launcher (+ samp_client.dll for our client).
trace_build() {
    local pkgs=(-p samp-tracer -p samp-launcher)
    [ "${1:-}" = ours ] && pkgs+=(-p samp-client)
    (cd "$TRACE_ROOT" && cargo build -q --release --target i686-pc-windows-gnu "${pkgs[@]}")
}

# Replace a top-level file in the lane's game dir (never write through a hard link).
trace_put() { rm -f "$GTA_DIR/$2"; cp "$1" "$GTA_DIR/$2"; }

trace_install() {
    trace_put "$TRACE_BIN/samp_tracer.dll" samp-tracer.dll
    trace_put "$TRACE_BIN/samp-launcher.exe" samp-rs-launcher.exe
    [ "${1:-}" = ours ] && trace_put "$TRACE_BIN/samp_client.dll" samp_client.dll
    true
}

# trace_launch CLIENT NAME — runs gta_sa.exe through our launcher with the tracer
# loaded first, then the client DLL (orig = samp.dll 0.3.7-R4, ours = samp_client.dll,
# bare = no client: the tracer boots a stock single-player game, SAMP_TRACER_BOOT=1).
# Blocks until the game exits; run it in the background.
trace_launch() {
    local client="$1" name="$2" dlls
    dlls=(--dll "$(winepath -w "$GTA_DIR/samp-tracer.dll")")
    case "$client" in
        orig) dlls+=(--dll "$(winepath -w "$GTA_DIR/samp.dll")") ;;
        ours) dlls+=(--dll "$(winepath -w "$GTA_DIR/samp_client.dll")") ;;
        bare) export SAMP_TRACER_BOOT=1 ;;
        *) echo "trace_launch: client must be orig, ours or bare" >&2; return 2 ;;
    esac
    cd "$GTA_DIR" || return 1
    wine_launch samp-trace "$GTA_DIR/samp-rs-launcher.exe" \
        --gta "$(winepath -w "$GTA_DIR/gta_sa.exe")" "${dlls[@]}" \
        --name "$name" --host 127.0.0.1 --port "$E2E_PORT"
}

# trace_wait_ctl SECONDS — until the tracer's control channel answers.
trace_wait_ctl() {
    local i
    for ((i = 0; i < $1 * 2; i++)); do
        [ "$(trace_ctl ping)" = pong ] && return 0
        sleep 0.5
    done
    return 1
}

trace_mark() { trace_ctl "mark $*" > /dev/null; trace_say "mark: $*"; }

# Kill only this lane's Wine processes.
trace_wine_kill() { WINEPREFIX="$TRACE_ROOT/wine/prefix-trace" wineserver -k 2>/dev/null || true; }

# trace_server_start_fs PWN NAME [DEFINES...] — like e2e_server_start, plus the filterscript
# tools/trace/PWN compiled as filterscripts/NAME (pawncc sym=value DEFINES, e.g. IDLE_MS=300000).
trace_server_start_fs() {
    local pwn="$1" name="$2"; shift 2
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$TRACE_ROOT/tools/trace/$pwn" "$@" \
        -iqawno/include -ofilterscripts/"$name".amx -d0 > /dev/null ) || return 1
    python3 -I - "$E2E_SERVER/config.json" "filterscripts/$name" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"].append(sys.argv[2])
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { echo "trace: server did not start" >&2; return 1; }
}

# trace_server_start_remote — tools/trace/remotefs.pwn (brings "RemoteBot" next to the
# traced player after it spawns; TRACE-2).
trace_server_start_remote() { trace_server_start_fs remotefs.pwn traceremote; }
