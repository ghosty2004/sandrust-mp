#!/usr/bin/env bash
# Build and start the samp-rs in-game client (NOT the original samp.exe/samp.dll):
# samp-launcher.exe starts wine/gta-sa/gta_sa.exe suspended, injects
# samp_client.dll and resumes it, inside a Wine virtual desktop.
#   tools/run-client.sh [HOST:PORT] [NAME]       (default 127.0.0.1:7777 RustClient)
# GTA_RES=WxH sets the desktop size (default 1280x720). SKIP_BUILD=1 skips cargo.
# Logs: wine/gta-sa/samp-rs.log (DLL), wine/gta-sa/samp-rs-launcher.log.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/tools/wine-env.sh"
TARGET="${1:-127.0.0.1:7777}"
NAME="${2:-RustClient}"
HOST="${TARGET%:*}"
PORT="${TARGET##*:}"
[[ "$PORT" == "$TARGET" ]] && PORT=7777

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    (cd "$ROOT" && cargo build --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher)
fi
OUT="$ROOT/target/i686-pc-windows-gnu/release"
install -m 0755 "$OUT/samp-launcher.exe" "$GTA_DIR/samp-rs-launcher.exe"
install -m 0755 "$OUT/samp_client.dll" "$GTA_DIR/samp_client.dll"

# Clean up leftovers of a previous run (e.g. a killed virtual desktop).
wineserver -k 2>/dev/null || true
cleanup() { wineserver -k 2>/dev/null || true; }
trap cleanup EXIT

# The DLL reads the auth table (third-party data, gitignored) from this path.
AUTH_TABLE="${AUTH_TABLE:-${SAMP_RS_ROOT:-$ROOT}/data/auth_table.txt}"
if [[ -f "$AUTH_TABLE" ]]; then
    export SAMP_RS_AUTH_TABLE="$(winepath -w "$AUTH_TABLE")"
else
    echo "warning: $AUTH_TABLE missing (tools/extract_auth_table.py); the client cannot join" >&2
fi

cd "$GTA_DIR"
rc=0
wine_launch samp-rs "$GTA_DIR/samp-rs-launcher.exe" \
    --gta "$(winepath -w "$GTA_DIR/gta_sa.exe")" --dll "$(winepath -w "$GTA_DIR/samp_client.dll")" \
    --name "$NAME" --host "$HOST" --port "$PORT" || rc=$?
exit "$rc"
