#!/usr/bin/env bash
# Inner part of tools/efx/run.sh (runs inside e2e_headless_run, isolated display).
#   scenario.sh CLIENT NAME OUTDIR SPECDIR
# For each step in EFX_STEPS (default "pos hp skin putin") the same RPCs are fired at both
# clients: capture 1 = first value (orig: single-step capture of the samp.dll handler, `efx arm
# rpc:N`; ours: window capture with the spec made from the original's capture 1), capture 2 =
# second value (window capture for both clients, same spec).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/lib.sh"
CLIENT="$1"; NAME="$2"; OUT="$3"; SPECDIR="${4:-}"
trace_ports "$CLIENT"
EFX="$GTA_DIR/efx.jsonl"
WIN_OPTS="${EFX_WIN_OPTS:-frames=20 ms=2500}"
ARM_OPTS="${EFX_ARM_OPTS:-}"

( trace_launch "$CLIENT" "$NAME" > "$OUT/launch.out" 2>&1 ) &
LPID=$!
end_game() {
    trace_mark end
    e2e_ctl quit > /dev/null
    trace_ctl quit > /dev/null
    local i
    for ((i = 0; i < 30; i++)); do kill -0 "$LPID" 2>/dev/null || break; sleep 1; done
    kill -0 "$LPID" 2>/dev/null && { efx_say "game still running; killing lane wine"; trace_wine_kill; }
    wait "$LPID" 2>/dev/null
}
fail() { efx_say "FAIL: $*"; e2e_shot "$OUT/fail.png"; end_game; exit 1; }

trace_wait_ctl 90 || fail "tracer control channel did not answer"
trace_mark boot
e2e_wait_log "\[e2e\] connect id=[0-9]+ name=$NAME" 120 || fail "no join"
e2e_wait_log "\[e2e\] class id=[0-9]+ class=" 60 || fail "no class request"
sleep 3
spawned=0
for i in 1 2 3 4 5 6; do
    e2e_key shift
    if e2e_wait_log "\[e2e\] spawn id=" 4; then spawned=1; break; fi
done
[ "$spawned" = 1 ] || fail "no spawn after Shift"
trace_mark spawned
sleep 4
efx_fs where > "$OUT/where-spawn.txt"

# segment NAME — copies the efx.jsonl records written since `seg_start`.
seg_start() { SEG0=$(wc -l < "$EFX" 2>/dev/null || echo 0); }
seg_save() { sleep 0.5; tail -n +"$((SEG0 + 1))" "$EFX" > "$OUT/$1.jsonl"; efx_say "$1: $(wc -l < "$OUT/$1.jsonl") records"; }

# window_spec STEP — the spec of this step (made from the original's capture 1).
window_spec() {
    local spec="$SPECDIR/$1.spec"
    [ "$CLIENT" = orig ] && spec="$OUT/$1.spec"
    [ -s "$spec" ] || { efx_say "$1: no spec $spec"; return 1; }
    cp "$spec" "$GTA_DIR/efx-$1.spec"
}

# baseline STEP IDX — the same window without firing anything: what the game and the client
# write to the watched pages anyway (efx_diff.py subtracts it).
baseline() {
    local step="$1" idx="$2" n r
    window_spec "$step" || return 1
    n=$(( $(efx_done_count) + 1 ))
    trace_mark "efx $step-$idx-base"
    seg_start
    # shellcheck disable=SC2086
    r=$(efx_ctl watch "efx-$step.spec" $WIN_OPTS)
    case "$r" in ok*) ;; *) efx_say "$step-$idx-base: $r"; return 1 ;; esac
    efx_wait_done "$n" 180 || efx_ctl disarm > /dev/null
    seg_save "$step-$idx-base"
}

# capture STEP IDX RPC FSCMD
capture() {
    local step="$1" idx="$2" rpc="$3" cmd="$4" n r
    if [ "$CLIENT" = ours ] || [ "$idx" != 1 ]; then baseline "$step" "$idx"; fi
    n=$(( $(efx_done_count) + 1 ))
    trace_mark "efx $step-$idx"
    seg_start
    if [ "$CLIENT" = orig ] && [ "$idx" = 1 ]; then
        # shellcheck disable=SC2086
        r=$(efx_ctl arm "rpc:$rpc" $ARM_OPTS)
    else
        window_spec "$step" || return 1
        # shellcheck disable=SC2086
        r=$(efx_ctl watch "efx-$step.spec" $WIN_OPTS)
    fi
    efx_say "$step-$idx: $r"
    case "$r" in ok*) ;; *) return 1 ;; esac
    efx_fs "$cmd" > /dev/null
    efx_wait_done "$n" 180 || { efx_say "$step-$idx: capture $n did not finish ($(efx_ctl status))"; efx_ctl disarm > /dev/null; }
    seg_save "$step-$idx"
    if [ "$CLIENT" = orig ] && [ "$idx" = 1 ]; then
        python3 -I "$HERE/efx_diff.py" spec "$OUT/$step-1.jsonl" > "$OUT/$step.spec"
    fi
    efx_fs where > "$OUT/where-$step-$idx.txt"
    sleep 1
}

for step in ${EFX_STEPS:-pos hp skin putin}; do
    case "$step" in
        pos) capture pos 1 12 "pos|1510.0|-1660.0|13.55"; capture pos 2 12 "pos|1530.0|-1670.0|13.55" ;;
        hp) capture hp 1 14 "hp|57.5"; capture hp 2 14 "hp|33.0" ;;
        skin) capture skin 1 153 "skin|29"; sleep 2; capture skin 2 153 "skin|101" ;;
        putin)
            # Next to the player (the tracer names the nearest car `near_veh` in both clients).
            efx_fs "pos|1510.0|-1660.0|13.55" > /dev/null
            sleep 3
            v=$(efx_fs "veh|560|1515.0|-1665.0|13.6|90.0" | sed -n 's/.* veh \([0-9]*\).*/\1/p')
            [ -n "$v" ] || { efx_say "putin: no vehicle"; continue; }
            sleep 4
            capture putin 1 70 "putin|$v|0"
            ;;
        *) efx_say "unknown step $step" ;;
    esac
    e2e_shot "$OUT/$step.png"
done
end_game
exit 0
