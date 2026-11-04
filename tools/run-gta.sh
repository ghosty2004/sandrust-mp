#!/usr/bin/env bash
# Start GTA SA 1.0 US under Wine inside a virtual desktop window.
# GTA_RES=WxH overrides the window size (default 1280x720).
# Extra arguments are passed to gta_sa.exe.
set -euo pipefail
source "$(dirname "$0")/wine-env.sh"
# Clean up leftovers of a previous run (e.g. a killed virtual desktop).
wineserver -k 2>/dev/null || true
cd "$GTA_DIR"
wine_launch gta "$GTA_DIR/gta_sa.exe" "$@"
