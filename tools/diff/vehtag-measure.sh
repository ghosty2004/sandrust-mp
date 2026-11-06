#!/usr/bin/env bash
# Measurement scenario for the remote driver's nametag (sourced by tools/diff/visual.sh through
# VISUAL_SCENARIO_FILE, with VISUAL_DIR set to its own output root). A samp-bot drives (speed 0)
# an Infernus; the local player watches it on foot through fixed cameras (SetPlayerCameraPos /
# LookAt, 4 / 8 / 16 m east of the car) and then as the passenger of that car. Every capture
# (`cap NAME all 1`) is bracketed by `vt_probe NAME`: remote peds and their vehicles' matrix
# positions, the last device VIEW / PROJECTION / viewport (tracer `xform`), GTA's camera
# position (0xB6F9CC) and, on the original, samp's own nametag anchor (A 0x10140bdc) and its
# VIEW / PROJECTION copies (A 0x10140be8 / 0x10140c28). Evaluate with tools/diff/vehtag_eval.py.

vt_probe() {
    remote_probe "$1"
    {
        tctl xform
        tctl "peek 0xb6f9cc 12"
        # The original's samp.dll base (tracer `module` record); our side has no samp.dll.
        local base=""
        [[ "$SIDE" == orig ]] && base="$(grep -m1 -oE '"name":"samp.dll","base_ptr":"0x[0-9a-f]+"' "$GTA_DIR/trace.jsonl" 2>/dev/null \
            | grep -oE '0x[0-9a-f]+')"
        if [[ -n "$base" ]]; then
            printf 'samp_anchor '; tctl "peek $(printf '%#x' $((base + 0x140bdc))) 12"
            printf 'samp_view '; tctl "peek $(printf '%#x' $((base + 0x140be8))) 64"
            printf 'samp_proj '; tctl "peek $(printf '%#x' $((base + 0x140c28))) 64"
        fi
    } >> "$OUT/$1-remote.txt"
}

vt_cap() { vt_probe "$1"; cap "$1" all 1; vt_probe "$1"; }

visual_scenario() {
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    fs 'veh|411|1696.0|-2592.0|13.5|0.0|1|1'
    local vid
    vid="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    bot Bot2 --mode drive --speed 0.0 --seconds 300
    waitlog 'spawn [0-9]+' 30
    sleep 2
    fs 'pos|B1|1694.0|-2592.0|13.5|0.0'
    waitlog 'state [0-9]+ 1->2' 30
    sleep 8
    local d
    for d in 4 8 16; do
        fs "campos|L|$((1696 + d)).0|-2592.0|14.5"
        fs 'camlook|L|1696.0|-2592.0|13.8'
        sleep 2
        vt_cap "vt-foot$d"
    done
    fs 'camera|L'
    fs "putin|L|${vid:-1}|1"
    sleep 8
    vt_cap vt-pass
    sleep 2
    vt_cap vt-pass2
    killbots
}
