#!/usr/bin/env bash
# F5: every row of docs/re/data/gta-patches-r4-annotated.csv is applied by our client
# (same live bytes as the original client) or justified in tools/diff/patch-justifications.tsv.
#
#   tools/diff/patch-coverage.sh [--skip-build]   boot both clients on one server (DIFF-3 lanes,
#                                                  tools/diff/cross-client.sh --patches-only), dump the
#                                                  bytes at every row's address after spawn, report
#   tools/diff/patch-coverage.sh --from RUN_DIR    report from an existing run (cross-client runs also
#                                                  dump patch-bytes-{A,B}.json)
#
# Output: target/diff/patch-coverage.{md,json}. Categories: applied/matching, applied (static only:
# the original had not applied the row at the snapshot, ours lists it in its tables),
# applied/different, missing (with owning milestone from docs/phase3/re10-gaps.md), justified.
# Frame-local rows (tools/diff/patchrows.py FRAME_LOCAL_ROWS, e.g. 368/369: written before a hooked
# draw and restored after it) are judged from a samp-tracer `efx watch` of the writes themselves on
# both clients (60 frames each), not from snapshots; a failed watch is "unverified" (FAIL).
# Negative test: XC_B_SAMP_CFG=<sa-mp.cfg with nohudscalefix=1> gives our client a config
# without the HUD scale fix -> rows 368/369 must be reported missing.
# Exit 0 only when no row is missing, different or unverified.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
RUN=""; BUILD_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --from) RUN="$2"; shift 2 ;;
        --skip-build) BUILD_ARGS+=(--skip-build); shift ;;
        *) echo "usage: $0 [--skip-build] | --from RUN_DIR" >&2; exit 2 ;;
    esac
done
if [ -z "$RUN" ]; then
    "$HERE/cross-client.sh" --patches-only "${BUILD_ARGS[@]}" || { echo "[f5] boot/dump run failed"; exit 1; }
    RUN="$(readlink -f "$ROOT/target/diff/xpatch-latest")"
fi
for s in A B; do
    [ -f "$RUN/patch-bytes-$s.json" ] || { echo "[f5] $RUN/patch-bytes-$s.json missing"; exit 1; }
done
exec python3 -I "$HERE/patch_report.py" --run "$RUN" --out "$ROOT/target/diff"
