#!/usr/bin/env bash
# Start the ORIGINAL SA:MP 0.3.7-R4 client (installed into wine/gta-sa) and
# connect to HOST:PORT (default 127.0.0.1:7777). NAME via SAMP_NAME.
set -euo pipefail
source "$(dirname "$0")/wine-env.sh"
# Clean up leftovers of a previous run (e.g. a killed virtual desktop).
wineserver -k 2>/dev/null || true
TARGET="${1:-127.0.0.1:7777}"
wine reg add 'HKCU\Software\SAMP' /v PlayerName /t REG_SZ /d "${SAMP_NAME:-OrigClient}" /f >/dev/null 2>&1
cd "$GTA_DIR"
wine_launch gta "$GTA_DIR/samp.exe" "$TARGET"
