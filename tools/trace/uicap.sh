#!/usr/bin/env bash
# TRACE-3 helpers on top of ui.sh (source it): cap NAME FRAMES MODE -- ACTION...
# starts a d3d log of FRAMES frames, runs ACTION (a ui.sh sub-command line) while it runs,
# then takes a screenshot NAME.png.
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/ui.sh"
cap() {
    local name="$1" frames="$2" mode="$3"; shift 3
    [ "${1:-}" = -- ] && shift
    "$T" d3d-start "$mode" "$frames" "$name" || return 1
    [ $# -gt 0 ] && "$T" "$@" > /dev/null
    "$T" d3d-collect "$name" > /dev/null
    "$T" shot "$name" > /dev/null
}
