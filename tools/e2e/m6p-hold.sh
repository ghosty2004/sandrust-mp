#!/usr/bin/env bash
# M6-P: interactive hold session of OUR client in lane veh (prefix-shot, gta-sa-shot, UDP 7820, control
# 47720/47722) with the trace tools of this worktree re-rooted at target/vehroot (like tools/e2e/m6parity.sh ours,
# but with this worktree's tools/trace and binaries). Prints "session: ID"; drive it with
#   SAMP_RS_TRACE_LOCK_HELD=1 TRACE_SESSION=ID target/vehroot/tools/trace/ui.sh …
# and end it with `ui.sh stop`. The session runs under flock of the main checkout's target/locks/veh.lock.
#   tools/e2e/m6p-hold.sh prepare   only re-root the tools (prints the root)
#   tools/e2e/m6p-hold.sh           prepare + start the session in the background (log: $VR/hold.log)
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
TGT="${CARGO_TARGET_DIR:-$ROOT/target}"
VR="$ROOT/target/vehroot"
BIN="$VR/target/i686-pc-windows-gnu/release"
mkdir -p "$BIN" "$VR/target/locks"
rm -rf "$VR/tools"; cp -r "$ROOT/tools" "$VR/tools"
for d in wine data server re; do ln -sfn "$DATA/$d" "$VR/$d"; done
ln -sfn "$ROOT/crates" "$VR/crates"
for f in samp_tracer.dll samp-launcher.exe samp_client.dll; do cp "$TGT/i686-pc-windows-gnu/release/$f" "$BIN/"; done
mkdir -p "$VR/target/debug"; cp "$TGT/debug/samp-bot" "$VR/target/debug/" 2>/dev/null || true
sha256sum "$BIN"/*.dll "$BIN"/*.exe > "$VR/binaries.sha256"
L="$VR/tools/trace/lib.sh"
sed -i -e 's#wine/prefix-trace#wine/prefix-shot#g' -e 's#wine/gta-sa-trace#wine/gta-sa-shot#g' \
    -e 's#E2E_TAG=trace E2E_PORT=7790 E2E_CTL_PORT=47701#E2E_TAG=veh E2E_PORT=7820 E2E_CTL_PORT=47720#' \
    -e 's#SAMP_TRACER_PORT=47702#SAMP_TRACER_PORT=47722#' "$L"
python3 -I - "$L" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"trace_build\(\) \{.*?\n\}", "trace_build() { true; }", s, count=1, flags=re.S)
open(p, "w").write(s)
PY
grep -q 'prefix-shot' "$L" && grep -q 'E2E_PORT=7820' "$L" && grep -q 'trace_build() { true; }' "$L" \
    || { echo "m6p-hold: could not re-root the trace tools" >&2; exit 1; }
[ "${1:-}" = prepare ] && { echo "$VR"; exit 0; }
LOCKS="$DATA/target/locks"; unset SAMP_RS_ROOT
nohup flock "$LOCKS/veh.lock" env CARGO_TARGET_DIR="$VR/target" "$VR/tools/trace/run.sh" "${M6P_CLIENT:-ours}" hold > "$VR/hold.log" 2>&1 &
for ((i = 0; i < 400; i++)); do
    sess="$(sed -n 's/.*session: \([^ ]*\).*/\1/p' "$VR/hold.log" | head -n 1)"
    [ -n "$sess" ] && { echo "session: $sess"; exit 0; }
    grep -q "scenario exit" "$VR/hold.log" && break
    sleep 1
done
cat "$VR/hold.log"; exit 1
