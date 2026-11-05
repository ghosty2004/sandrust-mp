#!/usr/bin/env bash
# Tracing lane (ENV-3): own Wine prefix, game copy, server port and control port,
# so it never collides with the main e2e lane (wine/prefix, 7777, 47700).
# Source from tools/trace/*.sh.
TRACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# From a git worktree, SAMP_RS_ROOT names the main checkout that holds wine/, server/, data/ and
# the lane lock (target/locks/trace.lock is shared by every checkout).
TRACE_DATA_ROOT="${SAMP_RS_ROOT:-$TRACE_ROOT}"
export WINEPREFIX="$TRACE_DATA_ROOT/wine/prefix-trace"
export GTA_DIR="$TRACE_DATA_ROOT/wine/gta-sa-trace"
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
        [ -f "$TRACE_DATA_ROOT/data/auth_table.txt" ] \
            && SAMP_RS_AUTH_TABLE="$(winepath -w "$TRACE_DATA_ROOT/data/auth_table.txt" 2>/dev/null)" \
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
TRACE_BIN="${CARGO_TARGET_DIR:-$TRACE_ROOT/target}/i686-pc-windows-gnu/release"

trace_say() { printf '[trace] %s\n' "$*"; }

# trace_lock — blocks until this process holds target/locks/trace.lock (on fd 9, so it lasts
# until the caller and its children exit). No-op when SAMP_RS_TRACE_LOCK_HELD=1 (the lock is
# already held by a parent, e.g. tools/trace/with-lock.sh).
trace_lock() {
    [ "${SAMP_RS_TRACE_LOCK_HELD:-0}" = 1 ] && return 0
    mkdir -p "$TRACE_DATA_ROOT/target/locks"
    exec 9> "$TRACE_DATA_ROOT/target/locks/trace.lock"
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

# trace_log_cfg — records which SA:MP client config the next launch reads (tools/wine-env.sh installs
# tools/sa-mp.cfg.baseline or $TRACE_SAMP_CFG into the prefix): source, sha256 and content go to
# $OUT/samp-cfg.txt when $OUT is set (TRACE-8).
trace_log_cfg() {
    local f="$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP/sa-mp.cfg"
    [ -n "${OUT:-}" ] && [ -d "$OUT" ] && [ -f "$f" ] || return 0
    { echo "source: ${TRACE_SAMP_CFG:-${SAMP_RS_SAMP_CFG:-baseline}}"; echo "sha256: $(sha256sum < "$f" | cut -d' ' -f1)"
      tr -d '\r' < "$f"; } > "$OUT/samp-cfg.txt"
}

# trace_launch CLIENT NAME — runs gta_sa.exe through our launcher with the tracer
# loaded first, then the client DLL (orig = samp.dll 0.3.7-R4, ours = samp_client.dll,
# bare = no client: the tracer boots a stock single-player game, SAMP_TRACER_BOOT=1).
# Server 127.0.0.1:$E2E_PORT; TRACE_HOST overrides the host (the soak's proxy on 127.0.0.2).
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
    trace_log_cfg
    cd "$GTA_DIR" || return 1
    wine_launch samp-trace "$GTA_DIR/samp-rs-launcher.exe" \
        --gta "$(winepath -w "$GTA_DIR/gta_sa.exe")" "${dlls[@]}" \
        --name "$name" --host "${TRACE_HOST:-127.0.0.1}" --port "$E2E_PORT"
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
trace_wine_kill() { WINEPREFIX="$TRACE_DATA_ROOT/wine/prefix-trace" wineserver -k 2>/dev/null || true; }

# trace_server_start_fs PWN NAME [DEFINES...] — like e2e_server_start, plus the filterscript
# tools/trace/PWN compiled as filterscripts/NAME (pawncc sym=value DEFINES, e.g. IDLE_MS=300000).
trace_server_start_fs() {
    local pwn="$1" name="$2"; shift 2
    e2e_server_prepare
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$TRACE_ROOT/tools/trace/$pwn" "$@" \
        -iqawno/include -ofilterscripts/"$name".amx -d0 > /dev/null ) || return 1
    # TRACE_GAMEMODE=tracegm replaces the main script by tools/trace/tracegm.pwn (TRACE-6).
    # A bundled gamemode of the server copy (e.g. TRACE_GAMEMODE=derby, gamemodes/derby.amx) runs as is.
    if [ -n "${TRACE_GAMEMODE:-}" ] && [ -f "$TRACE_ROOT/tools/trace/$TRACE_GAMEMODE.pwn" ]; then
        ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$TRACE_ROOT/tools/trace/$TRACE_GAMEMODE.pwn" \
            -iqawno/include -ogamemodes/"$TRACE_GAMEMODE".amx -d0 > /dev/null ) || return 1
    fi
    # TRACE_LAGCOMP=0|1|2 sets game.lag_compensation_mode (InitGame lagcomp; TRACE-6).
    python3 -I - "$E2E_SERVER/config.json" "filterscripts/$name" "${TRACE_LAGCOMP:-}" "${TRACE_GAMEMODE:-}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["pawn"]["side_scripts"].append(sys.argv[2])
if sys.argv[3]:
    d["game"]["lag_compensation_mode"] = int(sys.argv[3])
if sys.argv[4]:
    d["pawn"]["main_scripts"] = [sys.argv[4] + " 1"]
json.dump(d, open(p, "w"), indent=4)
PY
    ( cd "$E2E_SERVER" && exec ./omp-server > "$E2E_LOG" 2>&1 ) &
    E2E_SERVER_PID=$!
    e2e_wait_log 'Legacy Network started' 20 || { echo "trace: server did not start" >&2; return 1; }
}

# trace_server_start_remote — tools/trace/remotefs.pwn (brings "RemoteBot" next to the
# traced player after it spawns; TRACE-2).
trace_server_start_remote() { trace_server_start_fs remotefs.pwn traceremote; }
