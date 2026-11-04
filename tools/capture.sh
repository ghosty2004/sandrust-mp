#!/usr/bin/env bash
# Run the local test server + a packet capture while a client command runs.
#   tools/capture.sh NAME SECONDS COMMAND [ARGS...]
# Writes captures/NAME.pcapng (loopback, UDP 7777) and captures/NAME.server.log,
# then decodes it to captures/NAME.log with samp-dump.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="$1"; SECS="$2"; shift 2
OUT="$ROOT/captures"; mkdir -p "$OUT"
( cd "$ROOT/server" && timeout "$((SECS + 6))" ./omp-server > "$OUT/$NAME.server.log" 2>&1 ) &
timeout "$((SECS + 4))" dumpcap -q -i lo -f "udp port 7777" -w "$OUT/$NAME.pcapng" > /dev/null 2>&1 &
sleep 3
timeout "$SECS" "$@" || true
# Stop any Wine processes the client left behind.
( source "$ROOT/tools/wine-env.sh" && wineserver -k 2>/dev/null ) || true
sleep 4
cargo run -q --manifest-path "$ROOT/Cargo.toml" -p samp-dump -- "$OUT/$NAME.pcapng" --port 7777 > "$OUT/$NAME.log"
echo "decoded: $OUT/$NAME.log ($(wc -l < "$OUT/$NAME.log") messages)"
