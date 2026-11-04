#!/usr/bin/env bash
# EFX-1 scenario runner: fires single RPCs at a client in the trace lane and captures their
# effect on the game with samp-tracer's effect tracer (docs/phase3/efx-notes.md).
#   tools/efx/run.sh orig        original samp.dll: single-step capture of each RPC handler
#   tools/efx/run.sh ours [DIR]  our samp_client.dll: window captures with the specs of an
#                                original run (DIR, default target/efx/orig-latest)
#   tools/efx/run.sh both        orig, then ours with its specs, then the diffs
# EFX_STEPS="pos hp skin putin" (RPCs 12, 14, 153, 70), EFX_ARM_OPTS / EFX_WIN_OPTS pass options
# to `efx arm` / `efx watch`. Output: target/efx/<client>-<stamp>/ (+ <client>-latest).
# Takes the trace-lane lock (target/locks/trace.lock of the lane checkout) for the whole run.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/lib.sh" || exit 1
trace_lock
CLIENT="${1:-orig}"
# Interrupted runs must not leave the lane's game, server or display (which hold the lock) behind.
cleanup() { e2e_server_stop; trace_wine_kill; e2e_headless_stop; }
trap 'cleanup; exit 130' INT TERM

one() {
    local client="$1" specdir="${2:-}" name out rc
    case "$client" in orig) name=EfxOrig ;; ours) name=EfxOurs ;; *) echo "client must be orig|ours|both" >&2; return 2 ;; esac
    out="$EFX_OUT_ROOT/$client-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$out"
    trace_ports "$client"
    efx_say "building ($client)"
    trace_build "$client" > "$out/build.out" 2>&1 || { cat "$out/build.out"; return 1; }
    trace_install "$client"
    for p in "$E2E_CTL_PORT" "$SAMP_TRACER_PORT"; do
        if [ "$(E2E_CTL_PORT=$p e2e_ctl ping)" = pong ]; then efx_say "port $p already answers; refusing"; return 1; fi
    done
    trace_wine_kill
    rm -f "$GTA_DIR/trace.jsonl" "$GTA_DIR/efx.jsonl" "$GTA_DIR/samp-tracer.log" "$GTA_DIR"/efx-*.spec
    efx_server_start || return 1
    efx_say "server on UDP $E2E_PORT (+ efxfs filterscript)"
    e2e_headless_run "$HERE/scenario.sh" "$client" "$name" "$out" "$specdir"
    rc=$?
    e2e_server_stop
    trace_wine_kill
    for f in trace.jsonl efx.jsonl samp-tracer.log samp-rs-launcher.log samp-rs.log; do
        [ -f "$GTA_DIR/$f" ] && cp "$GTA_DIR/$f" "$out/"
    done
    cp "$E2E_LOG" "$out/server.log" 2>/dev/null
    ln -sfn "$(basename "$out")" "$EFX_OUT_ROOT/$client-latest"
    efx_say "$client: scenario exit $rc, output $out"
    LAST_OUT="$out"
    return "$rc"
}

case "$CLIENT" in
    orig) one orig ;;
    ours) one ours "${2:-$EFX_OUT_ROOT/orig-latest}" ;;
    both)
        one orig || exit 1
        o="$LAST_OUT"
        one ours "$o"
        u="$LAST_OUT"
        for f in "$o"/*-[12].jsonl; do
            b="$(basename "$f")"
            [ -f "$u/$b" ] || continue
            python3 -I "$HERE/efx_diff.py" diff "$f" "$u/$b" > "$u/diff-${b%.jsonl}.txt"
            efx_say "diff ${b%.jsonl}: $u/diff-${b%.jsonl}.txt"
        done
        ;;
    *) sed -n '2,12p' "$0"; exit 2 ;;
esac
