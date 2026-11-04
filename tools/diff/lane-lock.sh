#!/usr/bin/env bash
# Diff-lane lock (lanes diff + diff2: wine/prefix-diff*, wine/gta-sa-diff*, server UDP 7795).
# Source it and call `diff_lane_lock` before touching the lanes: it blocks until this process
# holds target/locks/diff.lock (fd 8, held until the caller and its children exit) and exports
# SAMP_RS_DIFF_LOCK_HELD=1 so nested harness scripts do not wait for themselves.
# Also a no-op when an ancestor process is `flock … diff.lock …` (the PLAN §4 way of wrapping
# a run), so `flock target/locks/diff.lock tools/diff/cross-client.sh` does not deadlock.
DIFF_LOCK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

_diff_lock_by_ancestor() {
    local pid=$$ cmd
    while [ "${pid:-0}" -gt 1 ]; do
        cmd="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null)"
        case "$cmd" in
            flock\ *diff.lock*|*/flock\ *diff.lock*) return 0 ;;
        esac
        pid="$(awk '/^PPid:/ {print $2}' "/proc/$pid/status" 2>/dev/null)"
    done
    return 1
}

diff_lane_lock() {
    [ "${SAMP_RS_DIFF_LOCK_HELD:-0}" = 1 ] && return 0
    if _diff_lock_by_ancestor; then export SAMP_RS_DIFF_LOCK_HELD=1; return 0; fi
    mkdir -p "$DIFF_LOCK_ROOT/target/locks"
    exec 8> "$DIFF_LOCK_ROOT/target/locks/diff.lock"
    if ! flock -n 8; then
        printf '[diff] waiting for the diff-lane lock (target/locks/diff.lock)\n' >&2
        flock 8
    fi
    export SAMP_RS_DIFF_LOCK_HELD=1
}
