#!/usr/bin/env bash
# UX-1 acceptance: the SA:MP loading screen and everything drawn from boot until the class
# selection, ours vs the original.
#  1. original side (only when target/visual/orig has no complete capture): tools/diff/visual.sh's
#     original run (trace lane, under its lock);
#  2. our side: our client in lane "ui" (visual.sh takes target/locks/ui.lock itself; a caller that
#     already holds it must export SAMP_RS_UI_LOCK_HELD=1) with samp-tracer logging the
#     Direct3D call stream from the first Present (SAMP_TRACER_D3D_BOOT) and a screenshot
#     timeline of the isolated headless display, up to the class selection (no further scenario);
#  3. tools/diff/boot_diff.py: loading screen frame by frame (runs of identical Presents), then
#     the client's UI pass (first-frame construction order, connecting screen, chat updates);
#  4. tools/e2e/ux1_shots.py: same-moment screenshots (loading screen, class selection).
# Exit 0 = both diffs pass. Output: target/visual/{orig,ours}/, target/ux1/report.txt.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${SAMP_RS_ROOT:-}" && ! -d "$ROOT/wine" ]]; then
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    [[ -n "$common" && -d "$(dirname "$common")/wine" ]] && SAMP_RS_ROOT="$(dirname "$common")"
fi
export SAMP_RS_ROOT="${SAMP_RS_ROOT:-$ROOT}"
V="$ROOT/target/visual"
mkdir -p "$ROOT/target/ux1"
say() { printf '[ux1] %s\n' "$*"; }
if [[ ! -f "$V/orig/complete" ]]; then
    say "no original capture yet: running tools/diff/visual.sh (original + ours, full scenario)"
    "$ROOT/tools/diff/visual.sh" > "$ROOT/target/ux1/visual.out" 2>&1
else
    say "our side (boot only)"
    VISUAL_SCENARIO=0 "$ROOT/tools/diff/visual.sh" --ours-only > "$ROOT/target/ux1/visual.out" 2>&1 \
        || { cat "$ROOT/target/ux1/visual.out"; say "FAIL: our run failed"; exit 1; }
fi
{
    python3 -I "$ROOT/tools/diff/boot_diff.py" "$V/ours/d3d-full.jsonl" "$V/orig/d3d-full.jsonl"
    d=$?
    python3 -I "$ROOT/tools/e2e/ux1_shots.py" "$V/ours" "$V/orig"
    s=$?
    echo "boot_diff exit $d, screenshots exit $s"
    exit $(( d || s ))
} 2>&1 | tee "$ROOT/target/ux1/report.txt"
rc=${PIPESTATUS[0]}
(( rc == 0 )) && say "PASS" || say "FAIL"
exit "$rc"
