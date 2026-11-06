#!/usr/bin/env bash
# F7 soak (PLAN.md): our in-game client on each bundled gamemode (gungame, derby, simpletdm) for
# SOAK_SECS (default 7200 = 2 h) with several samp-bot players (walk, idle with a special action,
# shoot at us, shoot at a bot, drive, passenger), plus the scripted cases, each at least once:
#   gmx         server restart via RPC 40 (SendRconCommand gmx)
#   kick        Kick → "Server closed the connection.", no automatic reconnect (as samp), the game
#               stays up; then quit the game and start it again (rejoin)
#   netloss     the client's UDP path dropped for SOAK_NETDROP_SECS (default 15; a list like "15 40 6"
#               is used by the netloss cases in turn) by a userspace proxy
#               (tools/e2e/soak_udpproxy.py; the client always connects through it) → lost connection
#               → reconnect
#   srvkill     server process killed (SIGKILL) and started again → lost connection → reconnect
#   srvrestart  server process stopped (SIGTERM) and started again; rejoin by itself, or after
#               "Server closed the connection." by starting the game again
# Every SOAK_SAMPLE_SECS (default 5) tools/e2e/soak_mon.py takes a paired sample: the server's view
# (tools/e2e/probe/soakfs.pwn dump: every player and vehicle) and the client's (control channel
# state / net / remote / vehicles), plus the frame counter, the game's RSS and crash markers.
# Pass per gamemode (thresholds and their reasons: docs/phase3/soak-notes.md): no client crash, no
# panic, no stuck or frozen state, every case recovered, desync within tolerance in >= 99% of the
# steady-state comparisons of every category and no persistent desync, fps stable, memory slope
# bounded. Exit 0 only when every gamemode passes.
#
# Lane main by default: run as `flock target/locks/main.lock tools/e2e/soak.sh`.
#   SOAK_SECS=600 tools/e2e/soak.sh          short trial (the cases are spread over the run)
#   SOAK_GAMEMODES="derby" …                  a subset
#   SOAK_SKIP_BUILD=1                         use the binaries already built
#   SOAK_CAPTURE=1                            loopback capture of the server port (capture.pcapng)
#   SOAK_CASES="gmx kick"                     the scripted cases to run (default all; "" = none)
#   SOAK_CLIENT=orig                          the ORIGINAL samp.dll (+ samp-tracer.dll) instead of our client,
#                                             in the trace lane (its lock is taken; output target/soak-orig/);
#                                             the same cases (chat read from samp's chatlog.txt, chat typed on
#                                             the game window; net / remote / vehicles are read from samp.dll's
#                                             memory, soak_mon.py OrigView)
#   SOAK_BOTS="SkDrive SkPass"                only these bots (default all of the gamemode)
#   SOAK_BOT_SECS="40-90"                     session length range of each bot run (default 240-720)
# Output: target/soak/<gamemode>/ (summary.json, samples.jsonl, events.jsonl, client-N/, bots/).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/server" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/server" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
export SOAK_CLIENT="${SOAK_CLIENT:-ours}"
if [[ "$SOAK_CLIENT" == orig ]]; then
    # trace lane: prefix-trace, gta-sa-trace, server 7790, tracer control 47701; blocking lane lock
    source "$ROOT/tools/trace/lib.sh"
    trace_lock
fi
export E2E_TAG="${E2E_TAG:-main}" E2E_PORT="${E2E_PORT:-7777}" E2E_CTL_PORT="${E2E_CTL_PORT:-47700}"
export SAMP_RS_CTL_PORT="$E2E_CTL_PORT"
# The proxy listens on the server port of a second loopback address (the packet cipher is keyed with
# the port the client dials); the server is bound to 127.0.0.1.
export SOAK_PROXY_HOST="${SOAK_PROXY_HOST:-127.0.0.2}"
export SOAK_SECS="${SOAK_SECS:-7200}" SOAK_SAMPLE_SECS="${SOAK_SAMPLE_SECS:-5}"
export SOAK_EVENT_PERIOD="${SOAK_EVENT_PERIOD:-900}" SOAK_NETDROP_SECS="${SOAK_NETDROP_SECS:-15}"
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
OUTROOT="$ROOT/target/soak"
[[ "$SOAK_CLIENT" == orig ]] && OUTROOT="$ROOT/target/soak-orig"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
GAMEMODES="${SOAK_GAMEMODES:-gungame derby simpletdm}"
say() { printf '[soak] %s\n' "$*"; }

finish() {
    e2e_headless_stop
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true  # this lane's WINEPREFIX only
}
trap finish EXIT

mkdir -p "$OUTROOT"
if [[ "$SOAK_CLIENT" == orig ]]; then
    say "building (tracer, launcher; samp-bot) for the original client"
    { trace_build && (cd "$ROOT" && cargo build -p samp-bot); } > "$OUTROOT/build.out" 2>&1 \
        || { tail -n 30 "$OUTROOT/build.out"; say "build failed"; exit 1; }
    trace_install orig
    [[ -f "$GTA_DIR/samp.dll" ]] || { say "no samp.dll in $GTA_DIR"; exit 1; }
elif [[ "${SOAK_SKIP_BUILD:-0}" != 1 ]]; then
    say "building (i686-pc-windows-gnu release: client, launcher; samp-bot)"
    { (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) &&
      (cd "$ROOT" && cargo build -p samp-bot); } > "$OUTROOT/build.out" 2>&1 \
        || { tail -n 30 "$OUTROOT/build.out"; say "build failed"; exit 1; }
fi
[[ -f "$SAMP_RS_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
[[ -x "$TB/debug/samp-bot" ]] || { say "samp-bot not built"; exit 1; }
if [[ -n "$(e2e_ctl ping)" ]]; then say "something already answers on control port $E2E_CTL_PORT; refusing to run"; exit 1; fi
if ss -uln 2>/dev/null | grep -qE "[:.]$E2E_PORT\b"; then
    say "UDP port $E2E_PORT already in use; refusing to run"; exit 1
fi

# soak_prepare GAMEMODE — isolated server copy running GAMEMODE with the probe and soak filterscripts.
soak_prepare() {
    e2e_server_prepare || return 1
    ( cd "$E2E_SERVER" && LD_LIBRARY_PATH=qawno ./qawno/pawncc "$ROOT/tools/e2e/probe/soakfs.pwn" \
        -iqawno/include -ofilterscripts/soakfs.amx -d0 > /dev/null ) || return 1
    [[ -f "$E2E_SERVER/gamemodes/$1.amx" ]] || { say "gamemode $1 not in the server copy"; return 1; }
    mkdir -p "$E2E_SERVER/scriptfiles"
    python3 -I - "$E2E_SERVER/config.json" "$1" <<'PY'
import json, sys
p, gm = sys.argv[1], sys.argv[2]
d = json.load(open(p))
d["pawn"]["main_scripts"] = [gm + " 1"]
d["pawn"]["side_scripts"] = ["filterscripts/e2eprobe", "filterscripts/soakfs"]
d["max_players"] = 50
d["network"]["bind"] = "127.0.0.1"
json.dump(d, open(p, "w"), indent=4)
PY
}

rc=0
results=()
for gm in $GAMEMODES; do
    out="$OUTROOT/$gm"
    rm -rf "$out"; mkdir -p "$out"
    say "---- $gm: $SOAK_SECS s (lane $E2E_TAG, server $E2E_PORT, proxy $SOAK_PROXY_HOST:$E2E_PORT, control $E2E_CTL_PORT)"
    soak_prepare "$gm" || { say "$gm: server copy not prepared"; rc=1; results+=("$gm:FAIL(prepare)"); continue; }
    r=0
    SOAK_ROOT="$ROOT" SOAK_DATA_ROOT="$SAMP_RS_ROOT" SOAK_OUT="$out" SOAK_GAMEMODE="$gm" SOAK_BOT="$TB/debug/samp-bot" \
        E2E_SERVER="$E2E_SERVER" e2e_headless_run python3 -I "$ROOT/tools/e2e/soak_mon.py" run 2>&1 | tee "$out/soak.out"
    r=${PIPESTATUS[0]}
    e2e_server_stop
    ( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
    if (( r == 0 )); then results+=("$gm:PASS"); else results+=("$gm:FAIL"); rc=1; fi
done
say "results: ${results[*]}"
(( rc == 0 )) && say "PASS" || say "FAIL"
exit "$rc"
