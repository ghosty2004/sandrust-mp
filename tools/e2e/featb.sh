#!/usr/bin/env bash
# FEAT-B acceptance + parity runs (held items, remote parachute, /dl, /testdw, window setup, guarded opcodes,
# crash dialog): the scenario tools/e2e/featbscen.sh traced on the ORIGINAL client and on ours.
#   tools/e2e/featb.sh orig   original samp.dll in the trace lane (its tools/trace/run.sh orig hold, under the
#                             trace-lane lock); prints the run directory
#   tools/e2e/featb.sh ours   our samp_client.dll in lane ui (prefix-ui, gta-sa-ui, UDP 7800, control 47710,
#                             tracer 47711) through a private copy of the trace tools rooted at target/uiroot
#                             (wrap it in `flock SAMP_RS_ROOT/target/locks/ui.lock` with SAMP_RS_UI_LOCK_HELD=1,
#                             or let the script take the lock)
#   tools/e2e/featb.sh        ours, then tools/e2e/featb_report.py on ours against the original run(s)
#                             (FEATB_ORIG="DIR …", default target/featb/orig-latest; measured first when missing). Exit 0 = every feature
#                             asserted and every diff empty or covered by a logged decision.
# FEATB_ONLY selects scenario blocks (see featbscen.sh). Runs land in target/trace/… (orig) and
# target/uiroot/target/trace/… (ours); target/featb/{orig,ours}-latest point at them.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
export SAMP_RS_ROOT="$DATA"
TB="${CARGO_TARGET_DIR:-$ROOT/target}"
FB="$ROOT/target/featb"
mkdir -p "$FB"
say() { printf '[featb] %s\n' "$*"; }
CLIENT="${1:-all}"

user_files() { echo "$1/drive_c/users/$USER/Documents/GTA San Andreas User Files/SAMP"; }

if [ "$CLIENT" = all ]; then
    if [ ! -e "$FB/orig-latest/complete" ]; then
        say "no complete original run yet: measuring it first (trace lane)"
        "$0" orig || exit 1
    fi
    "$0" ours || { say "our run failed"; }
    # FEATB_ORIG: the original runs to diff against (a scenario block is taken from the last run that has it),
    # default the newest one.
    origs=()
    for o in ${FEATB_ORIG:-$FB/orig-latest}; do origs+=(--orig "$o"); done
    python3 -I "$ROOT/tools/e2e/featb_report.py" "$FB/ours-latest" "${origs[@]}" \
        --decisions "$ROOT/tools/e2e/featb-decisions.tsv" | tee "$FB/report.txt"
    exit "${PIPESTATUS[0]}"
fi

if [ "$CLIENT" = ours ]; then
    if [ "${SAMP_RS_UI_LOCK_HELD:-0}" != 1 ]; then
        mkdir -p "$DATA/target/locks"
        SAMP_RS_UI_LOCK_HELD=1 exec flock "$DATA/target/locks/ui.lock" "$0" ours
    fi
    say "building (client, launcher, tracer, bot)"
    { (cd "$ROOT" && cargo build -q --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher -p samp-tracer) &&
      (cd "$ROOT" && cargo build -q -p samp-bot); } > "$FB/build-ours.out" 2>&1 || { cat "$FB/build-ours.out"; exit 1; }
    unset SAMP_RS_ROOT
    VR="$ROOT/target/uiroot"
    BIN="$VR/target/i686-pc-windows-gnu/release"
    mkdir -p "$BIN" "$VR/target/locks"
    rm -rf "$VR/tools"; cp -r "$ROOT/tools" "$VR/tools"
    for d in wine data server re; do ln -sfn "$DATA/$d" "$VR/$d"; done
    ln -sfn "$TB/debug" "$VR/target/debug"
    ln -sfn "$ROOT/crates" "$VR/crates"
    for f in samp_tracer.dll samp-launcher.exe samp_client.dll; do cp "$TB/i686-pc-windows-gnu/release/$f" "$BIN/"; done
    L="$VR/tools/trace/lib.sh"
    sed -i -e 's#wine/prefix-trace#wine/prefix-ui#g' -e 's#wine/gta-sa-trace#wine/gta-sa-ui#g' \
        -e 's#E2E_TAG=trace E2E_PORT=7790 E2E_CTL_PORT=47701#E2E_TAG=ui E2E_PORT=7800 E2E_CTL_PORT=47710#' \
        -e 's#SAMP_TRACER_PORT=47702#SAMP_TRACER_PORT=47711#' "$L"
    python3 -I - "$L" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"trace_build\(\) \{.*?\n\}", "trace_build() { true; }", s, count=1, flags=re.S)
open(p, "w").write(s)
PY
    grep -q 'prefix-ui' "$L" && grep -q 'E2E_PORT=7800' "$L" && grep -q 'trace_build() { true; }' "$L" \
        || { say "could not re-root the trace tools"; exit 1; }
    TR="$VR/tools/trace"
    PREFIX="$DATA/wine/prefix-ui"
    GAMEDIR="$DATA/wine/gta-sa-ui"
else
    TR="$ROOT/tools/trace"
    PREFIX="$DATA/wine/prefix-trace"
    GAMEDIR="$DATA/wine/gta-sa-trace"
fi

# Everything below runs under the lane lock (with-lock exports SAMP_RS_TRACE_LOCK_HELD=1; the private root
# has its own trace.lock).
inner() {
    local log sess out pid i uf
    log="$(mktemp)"
    uf="$(user_files "$PREFIX")"
    mkdir -p "$uf"; rm -f "$uf/chatlog.txt" "$GAMEDIR/samp-rs-crash.txt"
    "$TR/run.sh" "$CLIENT" hold > "$log" 2>&1 &
    pid=$!
    for ((i = 0; i < 400; i++)); do
        sess="$(sed -n 's/.*session: \([^ ]*\).*/\1/p' "$log" | head -n 1)"
        [ -n "$sess" ] && break
        kill -0 "$pid" 2>/dev/null || { cat "$log"; say "run.sh ended early"; return 1; }
        sleep 1
    done
    [ -n "$sess" ] || { cat "$log"; say "no session id"; return 1; }
    export TRACE_SESSION="$sess"
    out="$(cd "$TR/../.." && pwd)/target/trace/${sess%%.*}"
    for ((i = 0; i < 200; i++)); do [ -e "$out/ready" ] && break; sleep 1; done
    [ -e "$out/ready" ] || { cat "$log"; say "session not ready"; return 1; }
    say "session $sess ($out)"
    FB_OUT="$out"
    fs() { "$TR/ui.sh" fs "$1" || say "fs failed: $1"; }
    mark() { "$TR/ui.sh" mark "$1" > /dev/null; }
    shot() { "$TR/ui.sh" shot "fb-$1" > /dev/null; }
    key() { "$TR/ui.sh" key "$@" > /dev/null; }
    fire() { "$TR/ui.sh" fire "$1" > /dev/null; }
    ctl() { "$TR/ui.sh" ctl "$*"; }
    d3d() { "$TR/ui.sh" d3d ui 3 "$1" > /dev/null; }
    say() { printf '[featb] %s\n' "$*"; }
    chat() { "$TR/ui.sh" key t > /dev/null; sleep 0.4; "$TR/ui.sh" type "$1" > /dev/null; sleep 0.3; "$TR/ui.sh" key Return > /dev/null; }
    bot() {
        local n="$1"; shift
        "$TR/ui.sh" bot "$n" "$@" > /dev/null
        "$TR/ui.sh" wait "\\[ui-fs\\] connect [0-9]+ $n" 25 > /dev/null || say "bot $n did not join"
        # Wait for its spawn (the gamemode teleports a player on spawn; the scenario moves the bot afterwards).
        FB_BOT="$("$TR/ui.sh" log 400 | sed -nE "s/.*\\[ui-fs\\] connect ([0-9]+) $n\$/\\1/p" | tail -n 1)"
        # Ids are reused: wait for a spawn line after this bot's connect line.
        local i
        [ -n "$FB_BOT" ] || { say "bot $n: no id"; return; }
        for ((i = 0; i < 50; i++)); do
            "$TR/ui.sh" log 400 | sed -n "/\\[ui-fs\\] connect $FB_BOT $n\$/,\$p" | grep -q "\\[ui-fs\\] spawn $FB_BOT\$" && break
            sleep 0.5
        done
        [ "$i" -lt 50 ] || say "bot $n did not spawn"
        sleep 1
    }
    check() { :; }
    source "$FB_WT/tools/e2e/featbscen.sh"
    sleep 2
    # In a subshell: a scenario error must never leave the hold session (and the lane lock) running.
    ( featb_scenario ) || say "scenario ended with status $?"
    sleep 2
    "$TR/ui.sh" stop > /dev/null 2>&1
    wait "$pid"
    tail -n 3 "$log"
    cp -f "$uf/chatlog.txt" "$out/chatlog.txt" 2>/dev/null
    cp -f "$GAMEDIR/samp-rs-crash.txt" "$out/" 2>/dev/null
    touch "$out/complete"
    ln -sfn "$out" "$FB_DIR/$CLIENT-latest"
    say "run: $(readlink -f "$out")"
}
export -f inner user_files say
FB_WT="$ROOT"; FB_DIR="$FB"
export TR CLIENT FB_WT FB_DIR PREFIX GAMEDIR
exec "$TR/with-lock.sh" bash -c inner
