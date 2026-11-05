#!/usr/bin/env bash
# M8b remaining-behaviour scenario, shared by the acceptance test (tools/e2e/m8b.sh, our client) and the parity runs
# (tools/e2e/m8bparity.sh: the ORIGINAL client in the trace lane and ours in lane veh, both traced).
# Source it and define before calling m8b_scenario:
#   fs 'CMD|ARG|…'   run one tools/trace/uifs.pwn command, wait for "[ui-fs] SEQ done", print its output lines
#   mark NAME        tracer mark (no-op without a tracer)
#   shot NAME        isolated-display screenshot
#   check STEP       acceptance assertions after STEP (no-op for parity runs)
#   keydown KEY / keyup KEY   hold / release a key on the isolated display
#   bot NAME         start samp-bot NAME on this lane's server (walks its default circle)
# The player stands on the LS airport apron (1700, -2600) like TRACE-8's run. M8B_ONLY=camera,bounds,… runs a subset.

M8B_X=1700; M8B_Y=-2600; M8B_Z=13.55

m8b_want() { [[ "${M8B_ONLY:-all}" == all || ",${M8B_ONLY}," == *",$1,"* ]]; }

m8b_scenario() {
    local obj v
    fs "pos|L|$M8B_X|$M8B_Y|$M8B_Z|0" > /dev/null; sleep 2
    fs "time|12|0" > /dev/null; fs "weather|1" > /dev/null
    # ---- widescreen (111)
    if m8b_want wide; then
        mark wide_on; fs "wide|L|1" > /dev/null; sleep 1.5; shot wide_on; check wide_on
        mark wide_off; fs "wide|L|0" > /dev/null; sleep 1.5; check wide_off
    fi
    # ---- camera RPCs (157, 158, 162, 82, 81)
    if m8b_want camera; then
        mark campos; fs "campos|L|1700|-2625|25" > /dev/null; sleep 1
        mark camlook_cut2; fs "camlook|L|1700|-2600|14|2" > /dev/null; sleep 1.5; shot camlook; check camlook
        mark camlook_cut1; fs "camlook|L|1710|-2600|14|1" > /dev/null; sleep 1.5
        mark camlook_cut0; fs "camlook|L|1690|-2600|14|0" > /dev/null; sleep 1.5
        mark cambehind; fs "camera|L" > /dev/null; sleep 1.5; check cambehind
        mark ipos; fs "ipos|L|1700|-2625|25|1720|-2625|25|3000|1" > /dev/null; sleep 3.5; check ipos
        mark ilook; fs "ilook|L|1700|-2600|14|1720|-2600|14|3000|1" > /dev/null; sleep 3.5
        mark ipos_cut2; fs "ipos|L|1720|-2625|25|1700|-2625|25|2000|2" > /dev/null; sleep 2.5
        mark ipos_zero; fs "ipos|L|1700|-2625|25|1690|-2625|25|0|1" > /dev/null; sleep 1
        mark cambehind2; fs "camera|L" > /dev/null; sleep 1.5
        mark camobj
        obj="$(fs "obj|1271|1700|-2590|13.9|0|0|0|0" | sed -nE 's/.* object ([0-9]+).*/\1/p' | head -n 1)"
        fs "camobj|L|$obj" > /dev/null; sleep 1.5; check camobj
        mark objmove; fs "objmove|$obj|1700|-2590|23.9|2" > /dev/null; sleep 6; check camobj_moved
        mark cambehind3; fs "camera|L" > /dev/null; sleep 1; check cambehind3
        fs "objdel|$obj" > /dev/null; sleep 1
        mark cam_done
    fi
    # ---- world bounds (17)
    if m8b_want bounds; then
        # uifs `pos` resets the bounds to ±20000 first, so every bounds step sets them after the teleport.
        mark bounds_set; fs "pos|L|1716|-2600|$M8B_Z|270" > /dev/null; sleep 1
        fs "bounds|L|1720|1680|-2580|-2620" > /dev/null; sleep 1
        mark bounds_walk; keydown w; sleep 3; keyup w; sleep 0.5; shot bounds_walk; check bounds_walk
        mark bounds_tp_out; fs "pos|L|$M8B_X|$M8B_Y|$M8B_Z|0" > /dev/null; sleep 1
        fs "bounds|L|1690|1680|-2580|-2620" > /dev/null; sleep 2; check bounds_out
        mark bounds_interior; fs "interior|L|1" > /dev/null; sleep 1.5; check bounds_interior
        mark bounds_int0; fs "interior|L|0" > /dev/null; sleep 1.5
        mark bounds_reset; fs "pos|L|$M8B_X|$M8B_Y|$M8B_Z|0" > /dev/null; sleep 2; check bounds_reset
        mark bounds_done
    fi
    # ---- gravity (146), drunk (35), fighting style (89), shop name (33)
    if m8b_want misc; then
        mark grav_player; fs "gravity|L|0.002" > /dev/null; sleep 1; check grav_player
        mark grav_global; fs "ggravity|0.005" > /dev/null; sleep 1
        mark grav_reset; fs "gravity|L|0.008" > /dev/null; sleep 1; check grav_reset
        mark drunk_5000; fs "drunk|L|5000" > /dev/null; sleep 2; check drunk_5000
        mark drunk_2100; fs "drunk|L|2100" > /dev/null; sleep 3; check drunk_2100
        mark drunk_0; fs "drunk|L|0" > /dev/null; sleep 1
        mark fstyle; fs "fstyle|L|6" > /dev/null; sleep 1
        mark fstyle4; fs "fstyle|L|4" > /dev/null; sleep 1
        mark shop; fs "shop|L|FDPIZA" > /dev/null; sleep 1; check shop
        mark shop_long; fs "shop|L|LONGSHOPNAM" > /dev/null; sleep 1
        mark shop_empty; fs "shop|L|" > /dev/null; sleep 1
        mark wanted3; fs "wanted|L|3" > /dev/null; sleep 1; check wanted3
        mark wanted0; fs "wanted|L|0" > /dev/null; sleep 1
        mark stunt1; fs "stunt|L|1" > /dev/null; sleep 1; check stunt1
        mark stunt0; fs "stunt|L|0" > /dev/null; sleep 1
        mark money; fs "money|L|500" > /dev/null; sleep 1; check money
        mark vel; fs "vel|L|0|0|0.5" > /dev/null; sleep 2.5
        mark ctl_off; fs "togglecontrol|L|0" > /dev/null; sleep 1
        mark ctl_on; fs "togglecontrol|L|1" > /dev/null; sleep 1
        mark interior1; fs "interior|L|1" > /dev/null; sleep 1.5; check interior1
        mark interior0; fs "interior|L|0" > /dev/null; sleep 1.5
        fs "pos|L|$M8B_X|$M8B_Y|$M8B_Z|0" > /dev/null; sleep 1
        mark misc_done
    fi
    # ---- animations (86, 87, the 207 animation field)
    if m8b_want anims; then
        mark anim_ped; fs "anim|L|PED|WALK_DRUNK|4.1|1|1|1|0|0|1" > /dev/null; sleep 2; check anim_walk
        mark anim_clear; fs "clranim|L|1" > /dev/null; sleep 1.5
        mark anim_dance; fs "anim|L|DANCING|DNCE_M_A|4.1|1|0|0|0|0|1" > /dev/null; sleep 2
        mark anim_dance2; fs "anim|L|DANCING|DNCE_M_A|4.1|1|0|0|0|0|1" > /dev/null; sleep 2; check anim_dance
        mark anim_clear2; fs "clranim|L|1" > /dev/null; sleep 1.5
        mark anim_sex; fs "anim|L|SEX|SEX_1_CUM_P|4.1|0|0|0|0|0|1" > /dev/null; sleep 1.5
        mark anim_timed; fs "anim|L|PED|IDLE_CHAT|4.1|0|0|0|1|1500|1" > /dev/null; sleep 2.5
        mark anim_freeze; fs "anim|L|PED|KO_skid_front|4.1|0|1|1|1|0|1" > /dev/null; sleep 2
        mark anim_clear3; fs "clranim|L|1" > /dev/null; sleep 1.5
        mark anim_done
    fi
    # ---- SCM events (96) on a car next to the player, ClientCheck (103; uifs takes type and address in hex)
    if m8b_want scm; then
        mark scm_veh
        v="$(fs "veh|560|1706|-2600|13.6|0|1|1" | sed -nE 's/.* veh-created ([0-9]+).*/\1/p' | head -n 1)"
        sleep 3
        mark scm_paint; fs "paint|$v|1" > /dev/null; sleep 1.5
        mark scm_mod; fs "vmod|$v|1010" > /dev/null; sleep 1.5
        mark scm_mod2; fs "vmod|$v|1080" > /dev/null; sleep 1.5
        mark scm_color; fs "vcolor|$v|3|6" > /dev/null; sleep 1.5
        mark check_2; fs "check|L|2|0|0|4" > /dev/null; sleep 1
        mark check_5; fs "check|L|5|400000|0|4" > /dev/null; sleep 1
        mark check_0x45; fs "check|L|45|0|0|4" > /dev/null; sleep 1
        mark check_0x46; fs "check|L|46|19B|0|4" > /dev/null; sleep 1
        mark check_0x47; fs "check|L|47|19B|0|4" > /dev/null; sleep 1
        mark check_0x48; fs "check|L|48|0|0|4" > /dev/null; sleep 1
        mark check_0x01; fs "check|L|1|0|0|4" > /dev/null; sleep 1
        mark check_limits; fs "check|L|2|0|300|4" > /dev/null; fs "check|L|2|0|0|300" > /dev/null; sleep 1
        check checks
        fs "vdel|$v" > /dev/null; sleep 1
        mark scm_done
    fi
    # ---- spectating (124, 126, 127) with a walking bot and a police car
    if m8b_want spectate; then
        bot SpecBot
        sleep 8
        fs "pos|B1|1695|-2595|$M8B_Z|0" > /dev/null; sleep 2
        v="$(fs "veh|596|1690|-2600|13.6|0|1|1" | sed -nE 's/.* veh-created ([0-9]+).*/\1/p' | head -n 1)"
        sleep 3
        mark spec_on; fs "specon|L" > /dev/null; sleep 2; shot spec_none; check spec_on
        mark spec_pl_normal; fs "specpl|L|B1|1" > /dev/null; sleep 2; shot spec_pl; check spec_pl
        mark spec_pl_fixed; fs "specpl|L|B1|2" > /dev/null; sleep 2
        mark spec_pl_side; fs "specpl|L|B1|3" > /dev/null; sleep 2
        mark spec_veh; fs "specveh|L|$v|1" > /dev/null; sleep 2; shot spec_veh; check spec_veh
        mark spec_veh_fixed; fs "specveh|L|$v|2" > /dev/null; sleep 2
        mark spec_off; fs "specoff|L" > /dev/null; sleep 3; shot spec_off; check spec_off
        fs "vdel|$v" > /dev/null
        mark spec_done
    fi
    mark scenario-end
}
