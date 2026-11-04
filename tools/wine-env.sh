#!/usr/bin/env bash
# Shared environment for running GTA SA (1.0 US) and SA:MP under Wine.
# Usage: source tools/wine-env.sh
_samp_rs_repo="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
ROOT="${ROOT:-$_samp_rs_repo}"
# SAMP_RS_ROOT: checkout that holds the gitignored wine/, server/ and data/
# (set it when running from a git worktree); defaults to this checkout.
SAMP_RS_DATA_ROOT="${SAMP_RS_ROOT:-$_samp_rs_repo}"
# Override WINEPREFIX / GTA_DIR to run a second, independent lane (e.g. tracing).
export WINEPREFIX="${WINEPREFIX:-$SAMP_RS_DATA_ROOT/wine/prefix}"
export GTA_DIR="${GTA_DIR:-$SAMP_RS_DATA_ROOT/wine/gta-sa}"
export WINEDEBUG="${WINEDEBUG:--all}"
export DXVK_LOG_LEVEL="${DXVK_LOG_LEVEL:-warn}"

# wine_launch NAME EXE ARGS... — run a Windows program in a Wine virtual desktop
# window (default, so it doesn't take over the user's monitors), or directly when
# E2E_HEADLESS=1 (inside headless gamescope, which already is an isolated screen).
wine_launch() {
    local name="$1" exe="$2"; shift 2
    if [ "${E2E_HEADLESS:-0}" = 1 ]; then
        wine "$(winepath -w "$exe")" "$@"
    else
        wine explorer "/desktop=$name,${GTA_RES:-1280x720}" "$(winepath -w "$exe")" "$@"
    fi
}

# Test runs must never be audible on the user's speakers: route Wine audio to a
# private null sink (created on demand). Set SAMP_RS_AUDIBLE=1 to hear a run.
if [ "${SAMP_RS_AUDIBLE:-0}" != 1 ] && command -v pactl > /dev/null; then
    if ! pactl list short sinks 2>/dev/null | grep -q samp_rs_null; then
        # PipeWire may make a new sink the default and move other apps' streams
        # to it (it did once: Spotify went silent). Keep the user's default.
        _samp_rs_default_sink="$(pactl get-default-sink 2>/dev/null)"
        pactl load-module module-null-sink sink_name=samp_rs_null \
            sink_properties="device.description=samp-rs-tests priority.session=1 priority.driver=1" > /dev/null 2>&1
        if [ -n "$_samp_rs_default_sink" ] && [ "$(pactl get-default-sink 2>/dev/null)" != "$_samp_rs_default_sink" ]; then
            pactl set-default-sink "$_samp_rs_default_sink"
        fi
    fi
    export PULSE_SINK=samp_rs_null
elif command -v pactl > /dev/null; then
    # Audible run: pin the user's default sink (PipeWire remembers per-app routing).
    PULSE_SINK="$(pactl get-default-sink 2>/dev/null)"; export PULSE_SINK
fi
