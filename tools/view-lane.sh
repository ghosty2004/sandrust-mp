#!/usr/bin/env bash
# tools/view-lane.sh [LANE] [SECONDS] — watch a test lane's isolated headless display without
# touching the test: takes a gamescope screenshot every SECONDS (default 2) into
# target/view-LANE.png and opens it once in an image viewer that reloads changed files (Loupe).
# LANE is the e2e tag: main, ui, veh, trace, diff, diff2 (default veh). Ctrl+C stops the loop.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LANE="${1:-veh}" EVERY="${2:-2}"
ENV="$ROOT/target/e2e-headless-$LANE.env"
OUT="$ROOT/target/view-$LANE.png"
TMP="$ROOT/target/.view-$LANE.tmp.png"
opened=0
while true; do
    sock="$(sed -n 's/^GAMESCOPE_WAYLAND_DISPLAY=//p' "$ENV" 2>/dev/null)"
    if [ -n "$sock" ] && GAMESCOPE_WAYLAND_DISPLAY="$sock" gamescopectl screenshot "$TMP" > /dev/null 2>&1; then
        sleep 0.5
        [ -s "$TMP" ] && mv -f "$TMP" "$OUT"
        if [ "$opened" = 0 ] && [ -s "$OUT" ]; then
            (loupe "$OUT" > /dev/null 2>&1 &)
            opened=1
        fi
    else
        echo "lane $LANE: no headless display running right now (waiting)" >&2
    fi
    sleep "$EVERY"
done
