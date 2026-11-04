#!/usr/bin/env bash
# M6 world-entity scenario, shared by the acceptance test (tools/e2e/m6.sh, our client) and the parity runs
# (tools/e2e/m6parity.sh: the ORIGINAL client in the trace lane and ours in lane veh, both traced).
# Source it and define before calling m6_scenario:
#   fs 'CMD|ARG|…'   run one tools/trace/uifs.pwn command and wait for "[ui-fs] SEQ done"
#   mark NAME        tracer mark (no-op without a tracer)
#   shot NAME        isolated-display screenshot
#   check STEP       acceptance assertions after STEP (no-op for parity runs)
#   fire MS          hold the left mouse button MS ms on the isolated display
# The player stands at the LS airport apron (1700, -2600) like TRACE-6's runs; all ids are the first ones the
# server hands out (objects 1…5, pickups 0…2, gang zones 0…2, actor 0, labels 0…1).

M6_X=1700; M6_Y=-2600; M6_Z=13.55

m6_scenario() {
    local only="${M6_ONLY:-all}"
    fs "pos|L|$M6_X|$M6_Y|$M6_Z|0"; sleep 2
    fs "time|12|0"; fs "weather|1"
    # ---- objects: draw distance 0 (GTA list), 500 (samp pass), draw-last 500, material text, material colour
    if [[ $only == all || $only == *objects* ]]; then
        mark objs-create
        fs "obj|1271|1695|-2590|13.9|0|0|0|0"
        fs "obj|1271|1705|-2590|13.9|0|0|0|500"
        fs "obj|19325|1700|-2585|14.5|0|0|0|500"
        fs "obj|19353|1690|-2585|14.5|0|0|0|0"
        fs "obj|1271|1710|-2585|13.9|0|0|0|0"
        sleep 1
        fs "objmat|5|0|-1|none|none|FFFF0000"
        fs "objtext|4|0|M6 TEXT|90|Arial|24|1|FFFFFFFF|FF0000FF|1"
        sleep 3
        shot objects; check objects
        mark move1;  fs "objmove|1|1695|-2590|23.9|2"; sleep 6; check move1
        mark move2rot; fs "objmove|1|1695|-2590|13.9|2|0|0|90"; sleep 6; check move2rot
        mark move3stop; fs "objmove|1|1695|-2590|23.9|2"; sleep 2; fs "objstop|1"; sleep 2; check move3stop
        mark move-done
        mark attach
        fs "att|L|0|1210|6|0.3|0.1|0|0|-90|0|1|1|1"; sleep 2; shot attached; check attach
        fs "attdel|L|0"; sleep 1
        mark rmb; fs "rmb|L|-1|1680|-2640|13.5|0.25"; sleep 1; check rmb
    fi
    # ---- pickups: type 2 and 19 by collision, type 14 polled
    if [[ $only == all || $only == *pickups* ]]; then
        mark pickups
        fs "pickup|1240|2|1700|-2594|13.55"
        fs "pickup|1242|14|1704|-2594|13.55"
        fs "pickup|1212|19|1708|-2594|13.55"
        sleep 2; shot pickups; check pickups-created
        mark walk1; fs "pos|L|1700|-2594|$M6_Z|0"; sleep 3
        mark walk14; fs "pos|L|1704|-2594|$M6_Z|0"; sleep 3
        mark walk19; fs "pos|L|1708|-2594|$M6_Z|0"; sleep 3
        check pickups
        fs "pos|L|$M6_X|$M6_Y|$M6_Z|0"; sleep 1
    fi
    # ---- checkpoints and race checkpoints
    if [[ $only == all || $only == *cps* ]]; then
        mark cp; fs "cp|L|1700|-2590|13.55|3"; sleep 2; shot checkpoint
        mark cp-walk; fs "pos|L|1700|-2590|$M6_Z|0"; sleep 3; check cp
        mark cpoff; fs "cpoff|L"; sleep 1; fs "pos|L|$M6_X|$M6_Y|$M6_Z|0"; sleep 1
        mark rcp0; fs "rcp|L|0|1700|-2580|13.55|1720|-2560|13.55|5"; sleep 2
        mark rcp1; fs "rcp|L|1|1700|-2580|13.55|1720|-2560|13.55|5"; sleep 2
        mark rcp-walk; fs "pos|L|1700|-2580|$M6_Z|0"; sleep 3; shot race-checkpoint; check rcp
        mark rcpoff; fs "rcpoff|L"; sleep 1; fs "pos|L|$M6_X|$M6_Y|$M6_Z|0"; sleep 1
    fi
    # ---- gang zones and map icons
    if [[ $only == all || $only == *zones* ]]; then
        mark gz
        fs "gz|1650|-2650|1750|-2550"; fs "gz|1600|-2700|1640|-2660"; fs "gz|1760|-2540|1800|-2500"
        fs "gzshow|L|0|FF000080"; fs "gzshow|L|1|00FF0080"; fs "gzshow|L|2|0000FF80"
        sleep 2; shot zones; check zones
        mark gzflash; fs "gzflash|L|0|FFFF00C0"; sleep 3; check zones-flash
        mark gzstop; fs "gzstop|L|0"; sleep 1
        fs "gzhide|L|0"; fs "gzhide|L|1"; fs "gzhide|L|2"; sleep 1
        mark icons
        fs "icon|L|0|1720|-2600|13|0|FF0000FF|0"
        fs "icon|L|1|1680|-2600|13|52|0|1"
        fs "icon|L|2|1700|-2620|13|19|0|2"
        fs "icon|L|3|1700|-2580|13|0|#5|3"
        sleep 2; shot icons; check icons
        mark icons-del; fs "icondel|L|0"; fs "icondel|L|1"; fs "icondel|L|2"; fs "icondel|L|3"; sleep 1
    fi
    # ---- actors: create, animation, health 0, invulnerable, damage by the local player
    if [[ $only == all || $only == *actors* ]]; then
        mark actor; fs "actor|101|1700|-2606|13.55|180"; sleep 2; check actor
        mark actanim; fs "actanim|0|PED|IDLE_CHAT|4.1|1|0|0|0|0"; sleep 2
        mark actclear; fs "actclear|0"; sleep 1
        fs "actdel|0"; sleep 1
        mark shoot-actor
        fs "actor|101|1700|-2606|13.55|180"; fs "actinv|0|0"; sleep 1
        fs "pos|L|1700|-2611.5|$M6_Z|0"; fs "weapon|L|22|50"; sleep 2
        fire 300; sleep 1; fire 300; sleep 1; fire 300; sleep 2
        check actor-damage
        mark shoot-actor-done
        mark acthp0; fs "acthp|0|0"; sleep 3; shot actor-dead; check acthp0
        fs "actdel|0"; sleep 1
        fs "pos|L|$M6_X|$M6_Y|$M6_Z|0"; sleep 1
    fi
    # ---- 3D text labels
    if [[ $only == all || $only == *labels* ]]; then
        mark labels
        fs "label|M6 {FF0000}LABEL|FFFF00FF|1700|-2594|14.5|50|0"
        fs "label|M6 LOS|00FFFFFF|1700|-2590|14.5|50|1"
        sleep 2; shot labels; check labels
    fi
    # ---- blank material texture (material model 0, TRACE-7: SAMP\blanktex.txd)
    if [[ $only == all || $only == *blanktex* ]]; then
        mark blanktex
        fs "obj|1271|1695|-2595|13.9|0|0|0|0"; sleep 1
        fs "objmat|6|0|0|none|none|FF00FF00"; sleep 3
        shot blanktex; check blanktex
    fi
    mark scenario-end
}
