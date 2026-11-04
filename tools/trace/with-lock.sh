#!/usr/bin/env bash
# tools/trace/with-lock.sh CMD... — runs CMD while holding the trace-lane lock
# (target/locks/trace.lock), with SAMP_RS_TRACE_LOCK_HELD=1 so tools/trace/run.sh inside does
# not wait for itself. Blocks until the lane is free.
set -uo pipefail
ROOT="${SAMP_RS_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
mkdir -p "$ROOT/target/locks"
exec 9> "$ROOT/target/locks/trace.lock"
if ! flock -n 9; then
    echo "[trace] waiting for the trace-lane lock (target/locks/trace.lock)" >&2
    flock 9
fi
SAMP_RS_TRACE_LOCK_HELD=1 exec "$@"
