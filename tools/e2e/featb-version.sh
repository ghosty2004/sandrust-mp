#!/usr/bin/env bash
# FEAT-10 "Version Error": both clients started on a gta_sa.exe whose version probe byte (VA 0x747483, 0x89 on
# 1.0 US) is 0x8B (`mov esi, [gGameState]` instead of the store: harmless, but neither 1.0 US nor EU). samp's
# start-up thread polls the probe and then shows the "Version Error" box and exits with code 1; ours must too.
#   tools/e2e/featb-version.sh [orig|ours|both]   lane ui (prefix-ui, gta-sa-ui; no server needed); takes
#                                                target/locks/ui.lock unless SAMP_RS_UI_LOCK_HELD=1
# The modified executable is a new top-level file of the lane's game dir (gta_sa_featbver.exe; the hard-linked
# original is never touched) and is removed afterwards. Output: target/featb/version-{orig,ours}.txt + .png.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA="${SAMP_RS_ROOT:-/home/ghosty2004/samp-rs}"
export SAMP_RS_ROOT="$DATA"
WHICH="${1:-both}"
if [ "${SAMP_RS_UI_LOCK_HELD:-0}" != 1 ]; then
    mkdir -p "$DATA/target/locks"
    SAMP_RS_UI_LOCK_HELD=1 exec flock "$DATA/target/locks/ui.lock" "$0" "$@"
fi
export WINEPREFIX="$DATA/wine/prefix-ui" GTA_DIR="$DATA/wine/gta-sa-ui" E2E_TAG=ui E2E_PORT=7800 E2E_CTL_PORT=47710
export SAMP_RS_CTL_PORT=47710
source "$ROOT/tools/wine-env.sh"
source "$ROOT/tools/e2e/lib.sh"
OUT="$ROOT/target/featb"
mkdir -p "$OUT"
TB="${CARGO_TARGET_DIR:-$ROOT/target}/i686-pc-windows-gnu/release"
(cd "$ROOT" && cargo build -q --release --target i686-pc-windows-gnu -p samp-client -p samp-launcher) || exit 1
EXE="gta_sa_featbver.exe"
python3 -I - "$GTA_DIR/gta_sa.exe" "$GTA_DIR/$EXE.tmp" <<'PY'
import subprocess, sys
src, dst = sys.argv[1], sys.argv[2]
d = bytearray(open(src, "rb").read())
for l in subprocess.run(["objdump", "-h", src], capture_output=True, text=True).stdout.splitlines():
    p = l.split()
    if len(p) >= 7 and p[0].isdigit():
        va, sz, fo = int(p[3], 16), int(p[2], 16), int(p[5], 16)
        if va <= 0x747483 < va + sz:
            off = fo + 0x747483 - va
            assert d[off] == 0x89, hex(d[off])
            d[off] = 0x8B
open(dst, "wb").write(d)
PY
rm -f "$GTA_DIR/$EXE"; mv "$GTA_DIR/$EXE.tmp" "$GTA_DIR/$EXE"
rm -f "$GTA_DIR/samp-rs-launcher.exe"; cp "$TB/samp-launcher.exe" "$GTA_DIR/samp-rs-launcher.exe"
rm -f "$GTA_DIR/samp_client.dll"; cp "$TB/samp_client.dll" "$GTA_DIR/samp_client.dll"

one() {
    local side="$1" dll i found="" rc=""
    case "$side" in orig) dll=samp.dll ;; ours) dll=samp_client.dll ;; esac
    wineserver -k 2>/dev/null
    ( cd "$GTA_DIR" && wine "$(winepath -w "$GTA_DIR/samp-rs-launcher.exe")" --gta "$(winepath -w "$GTA_DIR/$EXE")" \
        --dll "$(winepath -w "$GTA_DIR/$dll")" --name FbVersion --host 127.0.0.1 --port 7899 > "$OUT/version-$side.launch" 2>&1 ) &
    local lp=$!
    for ((i = 0; i < 150; i++)); do
        found="$(DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool search --name '^Version Error$' 2>/dev/null | head -n 1)"
        [ -n "$found" ] && break
        sleep 1
    done
    {
        echo "side=$side waited=${i}s window=${found:-none}"
        [ -n "$found" ] && echo "title=$(DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool getwindowname "$found")"
    } > "$OUT/version-$side.txt"
    if [ -n "$found" ]; then
        e2e_shot "$OUT/version-$side.png"
        # The box's OK through a mouse click on its button area is not reliable under gamescope; focus the box and
        # press Return (MB_OK), then Escape.
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool windowfocus "$found" 2>/dev/null; sleep 0.5
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool key Return 2>/dev/null; sleep 1
        DISPLAY="$E2E_HEADLESS_DISPLAY" xdotool key Escape 2>/dev/null
        # The launcher returns when the game process ends (ExitProcess(1) after the box).
        for ((i = 0; i < 20; i++)); do kill -0 "$lp" 2>/dev/null || break; sleep 1; done
        kill -0 "$lp" 2>/dev/null && echo "game still running 20 s after OK" >> "$OUT/version-$side.txt" \
            || echo "game exited after OK: $(grep -io 'exit[^,;]*' "$OUT/version-$side.launch" | tail -n 1)" >> "$OUT/version-$side.txt"
    fi
    wineserver -k 2>/dev/null
    wait "$lp" 2>/dev/null
    cat "$OUT/version-$side.txt"
}
export -f one
export OUT EXE
rc=0
for side in orig ours; do
    [ "$WHICH" = both ] || [ "$WHICH" = "$side" ] || continue
    e2e_headless_run bash -c "source '$ROOT/tools/e2e/lib.sh'; one $side" || rc=1
    grep -q "window=none" "$OUT/version-$side.txt" && rc=1
done
rm -f "$GTA_DIR/$EXE"
exit "$rc"
