#!/usr/bin/env bash
# Inner part of tools/trace/run.sh: runs inside e2e_headless_run (isolated display).
#   scenario.sh CLIENT NAME STEPS OUTDIR
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/lib.sh"
CLIENT="$1"; NAME="$2"; STEPS="$3"; OUT="$4"
trace_ports "$CLIENT"
# TRACE_VIDEO_MODE=N overrides the GTA video mode index (tools/wine-env.sh sets 32 = 1280x720 for
# a 1280x720 display; other display sizes have other mode lists, printed by samp-tracer).
if [ -n "${TRACE_VIDEO_MODE:-}" ]; then
    python3 -I "$HERE/setmode.py" "$WINEPREFIX/drive_c/users/$USER/Documents/GTA San Andreas User Files/gta_sa.set" "$TRACE_VIDEO_MODE"
fi

( trace_launch "$CLIENT" "$NAME" > "$OUT/launch.out" 2>&1 ) &
LPID=$!

BOTPID=
end_game() {
    # Only our own bot, by PID (the main lane may run its own samp-bot).
    [ -n "$BOTPID" ] && kill "$BOTPID" 2>/dev/null
    trace_mark end
    e2e_ctl state > "$OUT/state-end.txt"
    e2e_ctl quit > /dev/null
    trace_ctl quit > /dev/null
    local i
    for ((i = 0; i < 30; i++)); do kill -0 "$LPID" 2>/dev/null || break; sleep 1; done
    kill -0 "$LPID" 2>/dev/null && { trace_say "game still running; killing lane wine"; trace_wine_kill; }
    wait "$LPID" 2>/dev/null
}
fail() { trace_say "FAIL: $*"; e2e_shot "$OUT/fail.png"; end_game; exit 1; }

trace_wait_ctl 90 || fail "tracer control channel did not answer"
trace_mark boot
trace_say "tracer up: $(trace_ctl state)"
if [ "$CLIENT" = bare ]; then
    # Stock single-player baseline: no server; just let main.scm run.
    secs="${STEPS//[!0-9]/}"; secs="${secs:-40}"
    for ((i = 0; i < secs; i += 5)); do
        sleep 5
        trace_say "t+$((i + 5))s: $(e2e_ctl state)"
    done
    e2e_shot "$OUT/bare.png"
    python3 -I "$HERE/spoff_sites.py" "$SAMP_TRACER_PORT" "$OUT/spoff-sites.txt" 2>/dev/null || true
    python3 -I "$HERE/peek_dump.py" "$SAMP_TRACER_PORT" 0xA49960 200000 "$OUT/scriptspace.bin" > /dev/null 2>&1 || true
    end_game
    exit 0
fi
if [ "$STEPS" = hold ] || [ "$STEPS" = holdjoin ]; then
    # TRACE-3/5: interactive session driven by tools/trace/ui.sh (keys, screenshots, d3d logs,
    # filterscript commands, bots) until $OUT/stop exists or TRACE_HOLD_SECS (default 3600) pass.
    # The session is private (TRACE-6): its id (run name + random token) is written only to this run's
    # own $OUT/hold.env and printed on run.sh's output; ui.sh needs TRACE_SESSION=<id> and
    # SAMP_RS_TRACE_LOCK_HELD=1 from the session owner. Nothing is written to a shared path.
    SESSION="$(basename "$OUT").$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')"
    ( umask 077
      {
        echo "SESSION=$SESSION"
        echo "OUT=$OUT"
        echo "E2E_HEADLESS_DISPLAY=$E2E_HEADLESS_DISPLAY"
        echo "E2E_GAMESCOPE=$E2E_GAMESCOPE"
        echo "SAMP_TRACER_PORT=$SAMP_TRACER_PORT"
        echo "E2E_CTL_PORT=$E2E_CTL_PORT"
      } > "$OUT/hold.env" )
    trace_say "session: $SESSION (drive it with TRACE_SESSION=$SESSION SAMP_RS_TRACE_LOCK_HELD=1 tools/trace/ui.sh …)"
    if [ "$STEPS" = hold ]; then
        e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no join"
        trace_mark joined
        e2e_wait_log "\[e2e\] class id=[0-9]+ class=" 60 || fail "no class request"
        trace_mark class_selection
        sleep 3
        spawned=0
        for i in 1 2 3 4 5 6; do
            e2e_key shift
            if e2e_wait_log "\[e2e\] spawn id=" 4; then spawned=1; break; fi
        done
        [ "$spawned" = 1 ] || fail "no spawn after Shift"
        trace_mark spawned
    fi
    trace_say "holding; tools/trace/ui.sh drives the session (ui.sh stop ends it)"
    echo ready > "$OUT/ready"
    for ((i = 0; i < ${TRACE_HOLD_SECS:-3600}; i++)); do
        [ -e "$OUT/stop" ] && break
        kill -0 "$LPID" 2>/dev/null || { trace_say "game exited"; break; }
        sleep 1
    done
    rm -f "$OUT/hold.env"
    end_game
    exit 0
fi
e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no join"
trace_mark joined
e2e_wait_log "\[e2e\] class id=[0-9]+ class=" 60 || fail "no class request"
trace_mark class_selection
sleep 3
e2e_shot "$OUT/class.png"
e2e_ctl state > "$OUT/state-class.txt"
[ "$SAMP_TRACER_PORT" != "$E2E_CTL_PORT" ] && trace_ctl state > "$OUT/state-class.tracer.txt"
# main.scm as loaded in memory (CTheScripts::ScriptSpace, main part = 200000 bytes).
python3 -I "$HERE/peek_dump.py" "$SAMP_TRACER_PORT" 0xA49960 200000 "$OUT/scriptspace.bin" > /dev/null 2>&1 || true
if [ "$STEPS" = join ]; then end_game; exit 0; fi

spawned=0
for i in 1 2 3 4 5 6; do
    trace_mark "shift $i"
    e2e_key shift
    if e2e_wait_log "\[e2e\] spawn id=" 4; then spawned=1; break; fi
done
[ "$spawned" = 1 ] || fail "no spawn after Shift"
trace_mark spawned
sleep 2
e2e_ctl state > "$OUT/state-spawn.txt"
[ "$SAMP_TRACER_PORT" != "$E2E_CTL_PORT" ] && trace_ctl state > "$OUT/state-spawn.tracer.txt"
e2e_shot "$OUT/spawn.png"
if [ "$STEPS" = vehicles ]; then
    # TRACE-4: local enter/drive/exit, then a remote driver (drive_bot) — vehfs.pwn drives the rest.
    e2e_wait_log "\[trace-fs\] veh ready" 20 && trace_mark veh_ready || fail "vehicle setup missing"
    python3 -I "$HERE/spoff_sites.py" "$SAMP_TRACER_PORT" "$OUT/spoff-sites.txt" 2>/dev/null || true
    sleep 5
    e2e_shot "$OUT/veh-0.png"
    trace_mark enter_press
    # Held for 0.3 s: a bare press/release can fall between two pad updates.
    e2e_keydown f; sleep 0.3; e2e_keyup f
    e2e_wait_log "\[trace-fs\] state [0-9]+ 1->2" 10 && trace_mark local_driver || trace_say "local player did not become driver"
    sleep 1
    trace_mark drive_start
    e2e_keydown w
    sleep 4
    e2e_keyup w
    trace_mark drive_end
    e2e_shot "$OUT/veh-1.png"
    sleep 5
    trace_mark exit_press
    e2e_keydown f; sleep 0.3; e2e_keyup f
    e2e_wait_log "\[trace-fs\] state [0-9]+ 2->1" 10 && trace_mark local_onfoot || trace_say "local player did not leave the car"
    sleep 3
    ( cd "$TRACE_ROOT" && exec target/debug/examples/drive_bot "127.0.0.1:$E2E_PORT" --name DriveBot --seconds 62 ) \
        > "$OUT/bot.out" 2>&1 &
    BOTPID=$!
    trace_mark "bot_started pid=$BOTPID"
    e2e_wait_log "\[trace-fs\] bot in" 30 && trace_mark bot_in
    sleep 5; e2e_shot "$OUT/veh-remote-1.png"
    e2e_wait_log "\[trace-fs\] bot out" 25 && trace_mark bot_out
    sleep 3; e2e_shot "$OUT/veh-remote-2.png"
    e2e_wait_log "\[trace-fs\] local away" 15 && trace_mark local_away
    e2e_wait_log "\[trace-fs\] local back" 15 && trace_mark local_back
    sleep 4; e2e_shot "$OUT/veh-remote-3.png"
    e2e_wait_log "\[trace-fs\] carB [0-9]+ destroyed" 15 && trace_mark carB_destroyed
    sleep 3; e2e_shot "$OUT/veh-remote-4.png"
    for ((i = 0; i < 30; i++)); do kill -0 "$BOTPID" 2>/dev/null || break; sleep 1; done
    wait "$BOTPID" 2>/dev/null
    BOTPID=
    trace_mark bot_exited
    sleep 3
    end_game
    exit 0
fi
if [ "$STEPS" = idle ]; then
    # TRACE-4: SP-OFF baseline (spoff.pwn moves the player; we only watch and take pictures).
    e2e_wait_log "\[trace-fs\] grove" 20 && trace_mark grove || trace_say "player was not moved"
    secs="${TRACE_IDLE_SECS:-300}"
    for ((i = 0; i < secs; i += 30)); do
        sleep 30
        e2e_ctl state > "$OUT/state-idle-$((i + 30)).txt"
        e2e_shot "$OUT/idle-$((i + 30)).png"
        trace_mark "idle $((i + 30))s"
    done
    python3 -I "$HERE/spoff_sites.py" "$SAMP_TRACER_PORT" "$OUT/spoff-sites.txt" 2>/dev/null || true
    e2e_wait_log "\[trace-fs\] enex" 30 && trace_mark enex_outside || trace_say "no enex move"
    sleep 2
    trace_mark enex_walk
    e2e_keydown w
    sleep 2.5
    e2e_keyup w
    sleep 6
    e2e_shot "$OUT/enex-1.png"
    trace_mark enex_inside
    sleep 24
    e2e_shot "$OUT/enex-2.png"
    e2e_ctl state > "$OUT/state-enex.txt"
    end_game
    exit 0
fi
if [ "$STEPS" = remote ]; then
    # TRACE-2: a second player (samp-bot) streams in next to us, walks, is sent
    # away (stream-out) and brought back (stream-in again) by remotefs.pwn, leaves.
    ( cd "$TRACE_ROOT" && exec target/debug/samp-bot "127.0.0.1:$E2E_PORT" --name RemoteBot --seconds 40 ) \
        > "$OUT/bot.out" 2>&1 &
    BOTPID=$!
    trace_mark "bot_started pid=$BOTPID"
    e2e_wait_log "\[trace-fs\] bot" 30 && trace_mark bot_moved || trace_say "bot was not moved"
    e2e_wait_log "\[e2e\] streamin id=" 20 && trace_mark bot_streamed_in || trace_say "no streamin logged"
    for k in 1 2; do sleep 5; e2e_shot "$OUT/remote-$k.png"; trace_mark "remote_shot $k"; done
    e2e_wait_log "\[trace-fs\] bot [0-9]+ sent away" 20 && trace_mark bot_sent_away
    e2e_wait_log "\[e2e\] streamout id=" 10 && trace_mark bot_streamed_out
    sleep 9
    trace_mark bot_brought_back
    sleep 3
    e2e_shot "$OUT/remote-3.png"
    for ((i = 0; i < 60; i++)); do kill -0 "$BOTPID" 2>/dev/null || break; sleep 1; done
    wait "$BOTPID" 2>/dev/null
    BOTPID=
    trace_mark bot_exited
    e2e_wait_log "\[e2e\] disconnect id=" 30 && trace_mark bot_disconnected
    sleep 4
    e2e_ctl state > "$OUT/state-after-bot.txt"
    e2e_shot "$OUT/after-bot.png"
    end_game
    exit 0
fi
trace_mark walk_start
e2e_keydown w
sleep 3
e2e_keyup w
trace_mark walk_end
sleep 1
e2e_ctl state > "$OUT/state-walk.txt"
e2e_shot "$OUT/walk.png"
end_game
exit 0
