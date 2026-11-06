#!/usr/bin/env bash
# Parity check: bullet hit on an object (206 hit type 3), the remote driver of an RC vehicle (taken out of the
# world), F while surfing a player-driven vehicle (refused), the local passenger enter of a 570 carriage (warp).
# Runs tools/diff/combatveh-measure.sh through tools/diff/visual.sh's side drivers on the ORIGINAL client (trace
# lane, target/locks/trace.lock) and on OUR client (lane ui, target/locks/ui.lock), one after the other, each
# only once at least COMBATVEH_MIN_GB (default 6) GB of memory are available, then compares both sides with
# tools/diff/combatveh_eval.py.
#   tools/diff/combatveh.sh [--orig-only|--ours-only|--eval-only]
# Output: target/combatveh/{orig,ours}/cv-*.txt (+ screenshots). Exit 0 = both sides agree.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/wine" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/wine" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
export VISUAL_SCENARIO_FILE="$ROOT/tools/diff/combatveh-measure.sh" VISUAL_DIR="$ROOT/target/combatveh"
export COMBATVEH_MIN_GB="${COMBATVEH_MIN_GB:-6}"
mkdir -p "$SAMP_RS_ROOT/target/locks"
memwait() {
    local a
    while :; do
        a=$(free -g | awk '/^Mem:/ {print $7}')
        (( a >= COMBATVEH_MIN_GB )) && break
        echo "[combatveh] waiting for memory (available $a GB)"; sleep 30
    done
}
export -f memwait
side() {   # side orig|ours
    case "$1" in
        orig) flock "$SAMP_RS_ROOT/target/locks/trace.lock" bash -c "memwait; export SAMP_RS_TRACE_LOCK_HELD=1
                  source '$ROOT/tools/diff/visual.sh' --functions; run_orig" ;;
        ours) flock "$SAMP_RS_ROOT/target/locks/ui.lock" bash -c "memwait; export SAMP_RS_UI_LOCK_HELD=1
                  source '$ROOT/tools/diff/visual.sh' --functions; run_ours" ;;
    esac
}
case "${1:-}" in
    --orig-only) side orig || exit 1 ;;
    --ours-only) side ours || exit 1 ;;
    --eval-only) ;;
    *) side orig || exit 1; side ours || exit 1 ;;
esac
python3 -I "$ROOT/tools/diff/combatveh_eval.py" "$VISUAL_DIR" | tee "$VISUAL_DIR/report.txt"
exit "${PIPESTATUS[0]}"
