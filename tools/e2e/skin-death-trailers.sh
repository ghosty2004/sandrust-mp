#!/usr/bin/env bash
# Reproduction of the F3 crash of 2026-11-06 (cross-client run xclient-20261106-161806) and of the
# trailer timing cases around it, on one client in the trace lane (tools/trace/run.sh CLIENT hold).
#
#   tools/e2e/skin-death-trailers.sh [ours|orig] [ROUNDS]     (default: ours, 3 rounds)
#
# Each round, on the local player (tools/trace/uifs.pwn commands):
#  1. SetPlayerSpecialAction 10 (hands up), then 0: the hands-up task stays the primary task (samp does not stop
#     it, measured on the original) and holds its animation association;
#  2. SetPlayerSkin (alternating 60 / 61): the clump and its association list are deleted;
#  3. probe (control channel `peek`): primary task, its +8 association, whether that is still in the clump list
#     and its vtable — the original keeps the old pointer (no longer listed) with an intact block;
#  4. SetPlayerHealth 0: aborting the task writes into that association (+0x1C, +0x30..0x38);
#  5. trailers: a Linerunner next to the player and a trailer 3 km away (not streamed in) → AttachTrailerToVehicle
#     (RPC 148 for a trailer we do not have), then the trailer next to the truck → attach, destroy the truck right
#     after the attach, destroy the trailer;
#  6. vehicle model churn (12 models created and destroyed next to the player) so GTA's streaming evicts models:
#     a heap block corrupted by step 4 faulted there in RpMaterialDestroy (0x74DA25).
# Pass (ours): the game is alive at the end, no guarded fault / crash in samp-rs.log, and every skin change while
# the task ran logged "pedanims: kept". Pass (orig): the game is alive. Probe lines are printed for both.
# SAMP_RS_ROOT: checkout with wine/, server/, data/ (worktrees).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CLIENT="${1:-ours}"; ROUNDS="${2:-3}"
DATA="${SAMP_RS_ROOT:-$ROOT}"
UI="$ROOT/tools/trace/ui.sh"
OUT="$ROOT/target/skin-death-trailers"; mkdir -p "$OUT"
HOLD_LOG="$OUT/hold-$CLIENT.log"
CLIENT_LOG="$DATA/wine/gta-sa-trace/samp-rs.log"
say() { printf '[sdt] %s\n' "$*"; }

TRACE_GAMEMODE=tracegm "$ROOT/tools/trace/run.sh" "$CLIENT" hold > "$HOLD_LOG" 2>&1 &
RUN_PID=$!
for _ in $(seq 1 200); do grep -q "holding;" "$HOLD_LOG" && break; kill -0 "$RUN_PID" 2>/dev/null || break; sleep 3; done
grep -q "holding;" "$HOLD_LOG" || { say "FAIL: hold session did not start ($HOLD_LOG)"; exit 1; }
export TRACE_SESSION SAMP_RS_TRACE_LOCK_HELD=1
TRACE_SESSION="$(grep -o 'session: [^ ]*' "$HOLD_LOG" | head -1 | cut -d' ' -f2)"
say "session $TRACE_SESSION"
fs() { "$UI" fs "$1" > /dev/null || say "fs '$1' failed"; }
fsid() { "$UI" fs "$1" | sed -n 's/.*veh-created \([0-9]*\).*/\1/p' | head -1; }
peek_u32() {   # peek_u32 ADDR → decimal
    local h; h=$("$UI" ctl "peek $(printf '0x%x' "$1") 4" | awk '{print $6$5$4$3}')
    [ -n "$h" ] && echo $((16#$h)) || echo 0
}
probe() {
    local ped intel task vt anim avt clump plugin data link listed=no n=0
    ped=$(peek_u32 $((0xB7CD98))); intel=$(peek_u32 $((ped + 0x47C))); task=$(peek_u32 $((intel + 0x10)))
    vt=0; anim=0; avt=0
    if [ "$task" != 0 ]; then vt=$(peek_u32 "$task"); anim=$(peek_u32 $((task + 8))); fi
    [ "$anim" != 0 ] && avt=$(peek_u32 "$anim")
    clump=$(peek_u32 $((ped + 0x18))); plugin=$(peek_u32 $((0xB5F878))); data=$(peek_u32 $((clump + plugin)))
    link=$(peek_u32 "$data")
    while [ "$link" != 0 ] && [ $n -lt 32 ]; do
        [ $((link - 4)) = "$anim" ] && listed=yes
        link=$(peek_u32 "$link"); n=$((n + 1))
    done
    printf '[sdt] %s: primary task vt 0x%x, its association 0x%x (vt 0x%x), still in the clump list: %s\n' \
        "$1" "$vt" "$anim" "$avt" "$listed"
}

for r in $(seq 1 "$ROUNDS"); do
    skin=$((60 + r % 2))
    fs "pos|L|1700|-2600|13.6|0"; sleep 1
    fs "special|L|10"; sleep 2.5
    fs "special|L|0"; sleep 1
    probe "round $r hands up off"
    fs "pskin|L|$skin"; sleep 2
    probe "round $r after skin $skin"
    fs "kill|L"; sleep 8
    # Trailers: attach before the trailer streams in, then attach + destroy in quick succession.
    truck=$(fsid "veh|403|1705|-2600|14|90|1|1")
    trailer=$(fsid "veh|435|4700|-2600|14|90|1|1")
    sleep 2
    fs "attachtrailer|$trailer|$truck"; sleep 1
    fs "vdel|$trailer"
    trailer=$(fsid "veh|435|1715|-2600|14|90|1|1"); sleep 2
    fs "attachtrailer|$trailer|$truck"; fs "vdel|$truck"; sleep 1
    fs "vdel|$trailer"; sleep 1
    # Model churn next to the player.
    for m in 400 401 404 405 410 412 418 419 420 421 422 426; do
        v=$(fsid "veh|$m|1690|-2610|14|0|1|1"); sleep 1.2; fs "vdel|$v"
    done
    say "round $r: $("$UI" ctl state | cut -c1-90)"
done
STATE=$("$UI" ctl state)
"$UI" stop > /dev/null 2>&1
wait "$RUN_PID" 2>/dev/null
cp "$CLIENT_LOG" "$OUT/samp-rs-$CLIENT.log" 2>/dev/null
case "$STATE" in state*game_state=9*) ;; *) say "FAIL: game not alive at the end ($STATE)"; exit 1 ;; esac
if [ "$CLIENT" = ours ]; then
    faults=$(grep -cE "faulted|crash: exception" "$OUT/samp-rs-ours.log")
    kept=$(grep -c "pedanims: .* kept" "$OUT/samp-rs-ours.log")
    say "ours: $kept skin change(s) kept a task-held association, $faults guarded fault(s)"
    [ "$faults" = 0 ] || { grep -E "faulted|crash: exception" "$OUT/samp-rs-ours.log" | head; say "FAIL: faults"; exit 1; }
    [ "$kept" -ge "$ROUNDS" ] || { say "FAIL: want ≥ $ROUNDS kept associations"; exit 1; }
fi
say "PASS ($CLIENT, $ROUNDS rounds)"
