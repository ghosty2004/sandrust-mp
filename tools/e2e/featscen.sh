#!/usr/bin/env bash
# FEAT-A scenario (docs/phase3/feata-notes.md), shared by the acceptance test (tools/e2e/feata.sh, our client) and
# the parity runs (tools/e2e/featparity.sh: the ORIGINAL client and ours, both traced). Gamemode: tools/trace/tracegm.pwn
# (one class at the LS airport, no death handling), filterscript tools/trace/uifs.pwn. Display 1280x720.
# Source it and define before calling feat_scenario:
#   fs 'CMD|ARG|…'          run one uifs.pwn command, wait for "[ui-fs] SEQ done", print its output lines
#   mark NAME               tracer mark (no-op without a tracer)
#   shot NAME               isolated-display screenshot
#   check STEP              acceptance assertions after STEP (no-op for parity runs)
#   key KEY… / keydown KEY / keyup KEY / mouse ARGS…   xdotool on the isolated display
#   chat TEXT               open the chat input, type TEXT, Enter
#   bot NAME ARGS…          start samp-bot NAME (name must contain "Bot") on this lane's server
#   waitlog REGEX SECONDS   wait for a server log line
#   d3dcap NAME             D3D call-stream capture of 2 frames (no-op where unsupported)
# and set FEAT_LOG (the server log).
# FEAT_ONLY=f4,camtarget,scoreboard,map,crime,jack,modshop,vw runs a subset.
FEAT_X=1700; FEAT_Y=-2600; FEAT_Z=13.55

feat_want() { [[ "${FEAT_ONLY:-all}" == all || ",${FEAT_ONLY}," == *",$1,"* ]]; }
feat_id() { sed -nE "s/.* $1 ([0-9]+).*/\1/p" | head -n 1; }
feat_spawns() { grep -cE "\[e2e\] spawn id=" "$FEAT_LOG" 2>/dev/null; }
feat_bot1() { [[ -n "${FEAT_BOT1:-}" ]] && return 0; bot TgtBot --mode idle --seconds 900; FEAT_BOT1=1; sleep 6; }

feat_scenario() {
    local v o a
    fs "pos|L|$FEAT_X|$FEAT_Y|$FEAT_Z|0" > /dev/null; fs "camera|L" > /dev/null; sleep 2
    fs "time|12|0" > /dev/null; fs "weather|1" > /dev/null
    # ---- FEAT-3: F4 twice → one line; the next death → class selection (RPC 128, not 52); the second press is
    # still pending in GetAsyncKeyState's "pressed since" bit and re-arms the flag once it is consumed.
    if feat_want f4; then
        mark f4; key F4; sleep 0.7; key F4; sleep 1; shot f4; check f4_line
        local n0 i; n0="$(feat_spawns)"
        mark f4_death; fs "kill|L" > /dev/null
        sleep 12; check f4_class; shot f4_class
        for i in 1 2 3; do key shift; sleep 4; (( $(feat_spawns) > n0 )) && break; done
        sleep 2; check f4_rearm
        fs "pos|L|$FEAT_X|$FEAT_Y|$FEAT_Z|0" > /dev/null; fs "camera|L" > /dev/null; sleep 2
    fi
    # ---- FEAT-2: camera target (RPC 170 → 168) and /ctd
    if feat_want camtarget; then
        v="$(fs "veh|416|1700|-2590|13.6|90|1|1" | feat_id veh-created)"; FEAT_VEH="$v"
        mark camtarget_on; fs "camtarget|L|1" > /dev/null; sleep 2
        CAMTGT="$(fs "camtgt|L")"; check camtgt_vehicle
        mark ctd_on; chat "/ctd"; sleep 1.5; shot ctd_vehicle; d3dcap ctd_vehicle
        fs "vdel|$v" > /dev/null
        o="$(fs "obj|1337|1700|-2592|13.55|0|0|0|100" | feat_id object)"; FEAT_OBJ="$o"; sleep 1.5
        CAMTGT="$(fs "camtgt|L")"; check camtgt_object; shot ctd_object
        fs "objdel|$o" > /dev/null
        a="$(fs "actor|0|1700|-2594|13.55|180" | feat_id actor)"; FEAT_ACTOR="$a"; sleep 1.5
        CAMTGT="$(fs "camtgt|L")"; check camtgt_actor; shot ctd_actor
        fs "actdel|$a" > /dev/null
        feat_bot1
        fs "pos|B1|1700|-2594|13.55|180" > /dev/null; sleep 2
        mark camtgt_player; shot ctd_player; check camtgt_player
        mark ctd_off; chat "/ctd"; sleep 1
        mark camtarget_off; fs "camtarget|L|0" > /dev/null; sleep 1
        fs "pos|B1|1690|-2600|13.55|180" > /dev/null; sleep 1
    fi
    # ---- FEAT-4: scoreboard double click (RPC 23), pause-menu map right click (RPC 119)
    if feat_want scoreboard; then
        feat_bot1
        mark scoreboard; key Tab; sleep 2; shot scoreboard
        # 800x600 scoreboard at 1280x720: rows at y 134 (local player), 152 (the bot).
        mouse mousemove 500 152; sleep 0.3; mouse click 1; sleep 1.5
        mark scoreboard_dbl; mouse click --repeat 2 --delay 120 1; sleep 1.5
        check click_player; shot scoreboard_after
    fi
    if feat_want map; then
        mark map_menu; key Escape; sleep 2.5; key Down Down; sleep 0.5; key Return; sleep 2.5; shot map
        mark map_click; mouse mousemove 700 300; sleep 0.5; mouse mousedown 3; sleep 0.3; mouse mouseup 3; sleep 1.5
        shot map_waypoint; check click_map
        key Escape; sleep 1.5; key Escape; sleep 2.5
        fs "camera|L" > /dev/null; sleep 1
    fi
    # ---- FEAT-9: crime report with an on-foot suspect
    if feat_want crime; then
        feat_bot1
        mark crime_foot; fs "crime|L|B1|14" > /dev/null; sleep 3; check crime_foot
    fi
    # ---- FEAT-6: Car Jacked (a bot enters the car the local player drives), then a crime report with the bot
    # driving (the in-vehicle path)
    if feat_want jack; then
        feat_bot1
        v="$(fs "veh|416|1705|-2590|13.6|90|1|1" | feat_id veh-created)"; FEAT_JACK_VEH="$v"; sleep 1
        fs "putin|L|$v|0" > /dev/null; sleep 3
        mark jack; bot JackBot --mode drive --vehicle "$v" --seconds 900
        waitlog "connect [0-9]+ JackBot" 20; sleep 5
        fs "pos|B2|1708|-2590|13.55|90" > /dev/null
        local k; for k in $(seq 30); do grep -qE "\[ui-fs\] state [0-9]+ [0-9]+->2 veh $v " "$FEAT_LOG" && break; sleep 0.5; done
        sleep 0.4; shot jack_text; sleep 3; check jack
        mark crime_vehicle; fs "crime|L|B2|14" > /dev/null; sleep 3; check crime_vehicle
    fi
    # ---- FEAT-1: Wheel Arch Angels (SF) with a Sultan: enter (type 4), paint job preview (1 + 3), exit (type 4)
    if feat_want modshop; then
        fs "money|L|50000" > /dev/null
        v="$(fs "veh|560|-2706|217.5|4.4|90|1|1" | feat_id veh-created)"; FEAT_MOD_VEH="$v"; sleep 2
        fs "pos|L|-2700|213|4.3|90" > /dev/null; sleep 3
        local t; for t in 1 2 3 4; do fs "putin|L|$v|0" > /dev/null; sleep 3; fs "where|L" | grep -q " veh $v " && break; done
        # Drive in by short throttle taps until the car is inside the fitting bay (x < -2722), then handbrake taps
        # until the garage closes (it needs the car standing still inside; a brake tap would reverse).
        mark modshop_drive
        local x
        for t in $(seq 16); do
            waitlog "modshop [0-9]+ enterexit 1" 1 && break
            x="$(fs "where|L" | sed -nE 's/.* pos (-?[0-9.]+) .*/\1/p' | head -n 1)"
            if awk -v x="${x:-0}" 'BEGIN { exit !(x > -2722) }'; then
                keydown w; sleep 0.35; keyup w; sleep 1.2
            else
                keydown space; sleep 0.5; keyup space; sleep 2.5
            fi
        done
        sleep 3; shot modshop_in; check modshop_enter
        # Space: open "Paint Jobs" (no event), Space: preview paint job 1 (96 types 1 and 3), Space: buy it.
        mark modshop_paint; key space; sleep 2; key space; sleep 3; shot modshop_paint; check modshop_paint
        mark modshop_buy; key space; sleep 3
        # Enter leaves the shop (96 type 4, arg1 0).
        mark modshop_exit; key Return; sleep 5; check modshop_exit; shot modshop_out
    fi
    # ---- FEAT-9: SetPlayerVirtualWorld (no effect in R4)
    if feat_want vw; then
        mark vw; fs "vw|L|5" > /dev/null; sleep 1; fs "vw|L|0" > /dev/null; sleep 1; check vw
    fi
}
