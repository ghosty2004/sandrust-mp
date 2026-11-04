#!/usr/bin/env bash
# UX-1 parity loop, original side: boots the ORIGINAL client (samp.dll 0.3.7-R4 + samp-tracer) in
# the trace lane with Direct3D logging from the first Present (SAMP_TRACER_D3D_BOOT: every call
# while gGameState < 9, then the client's UI pass) and a screenshot timeline of the headless
# display (a `mark shot-N` in d3d.jsonl before each shot), from the loading screen through the
# connecting screen to class selection.
#   tools/e2e/ux1-orig.sh            (takes the trace-lane lock itself)
# Output: target/ux1-orig/ (d3d.jsonl, shots/shot-NNN.png, shots.tsv, run.out, trace dir link).
# UX1_SECS (default 45) = how long the timeline runs after the tracer answers.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/wine" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/wine" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
T="$ROOT/tools/trace"
if [ "${SAMP_RS_TRACE_LOCK_HELD:-0}" != 1 ]; then
    exec "$T/with-lock.sh" "$0" "$@"
fi
OUT="$ROOT/target/ux1-orig"
rm -rf "$OUT"; mkdir -p "$OUT/shots"
say() { printf '[ux1-orig] %s\n' "$*"; }

export SAMP_TRACER_D3D_BOOT="${SAMP_TRACER_D3D_BOOT:-all 20000}"
TRACE_HOLD_SECS=$(( ${UX1_SECS:-45} + 60 )) "$T/run.sh" orig holdjoin > "$OUT/run.out" 2>&1 &
RUN=$!
SESSION=""
for _ in $(seq 1 600); do
    SESSION="$(grep -oE 'session: [^ ]+' "$OUT/run.out" | head -n 1 | cut -d' ' -f2)"
    [ -n "$SESSION" ] && break
    kill -0 "$RUN" 2>/dev/null || break
    sleep 0.1
done
[ -n "$SESSION" ] || { say "no session id"; cat "$OUT/run.out"; exit 1; }
export TRACE_SESSION="$SESSION"
TDIR="$ROOT/target/trace/${SESSION%%.*}"
ln -sfn "$TDIR" "$OUT/trace"
E2E_GAMESCOPE="$(sed -n 's/^E2E_GAMESCOPE=//p' "$TDIR/hold.env")"
say "session $SESSION"
start=$(date +%s.%N)
i=0
end=$(( $(date +%s) + ${UX1_SECS:-45} ))
while (( $(date +%s) < end )); do
    i=$((i + 1))
    n=$(printf '%03d' "$i")
    "$T/ui.sh" mark "shot-$n" > /dev/null
    now=$(date +%s.%N)
    GAMESCOPE_WAYLAND_DISPLAY="$E2E_GAMESCOPE" gamescopectl screenshot "$OUT/shots/shot-$n.png" > /dev/null 2>&1
    printf '%s\t%.2f\n' "$n" "$(echo "$now - $start" | bc)" >> "$OUT/shots.tsv"
    sleep 0.4
done
"$T/ui.sh" stop > /dev/null
wait "$RUN"
cp -f "$TDIR/d3d.jsonl" "$OUT/" 2>/dev/null
cp -f "$TDIR/samp-tracer.log" "$OUT/" 2>/dev/null
say "done: $i shots, $(wc -l < "$OUT/d3d.jsonl" 2>/dev/null || echo 0) d3d records in $OUT"
