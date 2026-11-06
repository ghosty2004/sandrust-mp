#!/usr/bin/env bash
# Remote driver nametag parity: runs tools/diff/vehtag-measure.sh through tools/diff/visual.sh
# (original in the trace lane, ours in lane ui; each under its lock) into target/vehtag/, then
# tools/diff/vehtag_eval.py --check: every capture (fixed cameras 4 / 8 / 16 m from an
# Infernus driven by a samp-bot, and the local player as that car's passenger) must show the
# driver's tag on both clients, anchored at the same height relative to the driver (the
# original: bone 8, ~0.5 m above the seated ped's root; the vehicle position would be 0.3 m
# lower). The car and the in-car camera drift a little, so screen points are not compared
# 1:1; the anchor height recovered from the bar Y with each side's own VIEW / PROJECTION is.
#   tools/diff/vehtag.sh            ours + check (the original first if target/vehtag/orig
#                                   has no complete capture)
#   tools/diff/vehtag.sh --orig     re-capture the original too
#   tools/diff/vehtag.sh --check    only the check of the existing captures
set -uo pipefail
ROOT="$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/../.." && pwd)"
export VISUAL_SCENARIO_FILE="$ROOT/tools/diff/vehtag-measure.sh" VISUAL_DIR="$ROOT/target/vehtag" VISUAL_ONLY=none
case "${1:-}" in
    --check) ;;
    --orig) "$ROOT/tools/diff/visual.sh" --orig > /dev/null || exit 1 ;;
    *)
        if [[ ! -f "$VISUAL_DIR/orig/complete" ]]; then
            "$ROOT/tools/diff/visual.sh" --orig > /dev/null || exit 1
        else
            "$ROOT/tools/diff/visual.sh" --ours-only || exit 1
        fi
        ;;
esac
python3 -I "$ROOT/tools/diff/vehtag_eval.py" "$VISUAL_DIR/orig" "$VISUAL_DIR/ours"
python3 -I "$ROOT/tools/diff/vehtag_eval.py" --check "$VISUAL_DIR/orig" "$VISUAL_DIR/ours"
