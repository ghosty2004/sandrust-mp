#!/usr/bin/env bash
# FEAT-B scenario (player-visible features of the completeness audit), shared by both clients
# (tools/e2e/featb.sh: the ORIGINAL in the trace lane, ours in lane ui; both with samp-tracer).
# Source it and define before calling featb_scenario:
#   fs 'CMD|ARG|…'   run one tools/trace/uifs.pwn command (prints its output lines)
#   mark NAME        tracer mark            shot NAME     isolated-display screenshot
#   key KEY…         key presses            fire MS       left button held MS ms
#   chat TEXT        chat input: t, TEXT, Return
#   ctl CMD          tracer control line (peek, poke, window, d3d …)
#   d3d NAME         log 3 Presents of the UI draw stream to d3d-NAME.jsonl
#   bot NAME ARGS…   start samp-bot NAME on this lane's server (waits for its spawn; sets FB_BOT to its id)
#   check STEP       acceptance assertions after STEP (no-op on the original)
# FEATB_ONLY=items,remote,para,guard,dl,crash runs a subset ("window" always runs).

FB_X=1700; FB_Y=-2600; FB_Z=13.55
# Fault injection (FEAT-11 / FEAT-12): `mov eax, [0]` written over a GTA function entry, so the next call
# faults at that entry. CVehicle::AddVehicleUpgrade (0x6E3290) runs only from ADD_VEHICLE_MOD here,
# CTaskComplexUseMobilePhone's constructor (0x6348A0) only from TASK_USE_MOBILE_PHONE, CWeather::Update
# (0x72B850) every frame outside any client code.
FB_FAULT=a100000000

featb_want() { [[ "${FEATB_ONLY:-all}" == all || ",${FEATB_ONLY}," == *",$1,"* ]]; }

# fault_poke ADDR — saves the 5 original bytes in FB_SAVED and writes the faulting instruction.
fault_poke() {
    FB_SAVED="$(ctl "peek $1 5" | sed -nE 's/^peek [^ ]+ //p' | tr -d ' ')"
    [[ ${#FB_SAVED} == 10 ]] || { echo "featb: cannot read $1"; return 1; }
    ctl "poke $1 $FB_FAULT" > /dev/null
}
fault_restore() { [[ -n "${FB_SAVED:-}" ]] && ctl "poke $1 $FB_SAVED" > /dev/null; FB_SAVED=; }

featb_scenario() {
    local a v
    fs "pos|L|$FB_X|$FB_Y|$FB_Z|0" > /dev/null; sleep 2
    fs "time|12|0" > /dev/null; fs "weather|1" > /dev/null
    # ---- FEAT-10: window title and class style
    mark window; ctl window > "$FB_OUT/window.txt"; check window
    # ---- FEAT-5: held items 20 beer, 22 wine, 23 sprunk, 21 cigar (local)
    if featb_want items; then
        for a in 20 22 23 21; do
            mark "sa${a}"; fs "special|L|$a" > /dev/null; sleep 1.5; shot "sa${a}_held"
            fs "pstate|L"
            mark "sa${a}_fire"; fire 200; sleep 1; shot "sa${a}_use"; sleep 4
            fs "pstate|L"
            mark "sa${a}_fire2"; fire 200; sleep 4
            fs "pstate|L"; check "sa${a}"
            mark "sa${a}_drop"; key f; sleep 1.5
            fs "pstate|L"
        done
        # Special action 0 drops a held item.
        mark sa20_zero_on; fs "special|L|20" > /dev/null; sleep 1.5
        mark sa20_zero; fs "special|L|0" > /dev/null; sleep 1.5; fs "pstate|L"
        # 68: piss (anim + particle); the enter/exit key ends it, as does 0.
        mark sa68; fs "special|L|68" > /dev/null; sleep 2; shot sa68; fs "pstate|L"; check sa68
        mark sa68_stop; key f; sleep 1.5; fs "pstate|L"
        mark sa68b; fs "special|L|68" > /dev/null; sleep 2
        mark sa68_zero; fs "special|L|0" > /dev/null; sleep 1.5; fs "pstate|L"
        mark items_done
    fi
    # ---- FEAT-5 remote: a bot holding a beer, then a pissing bot
    if featb_want remote; then
        mark remote20; bot BotItem --mode idle --seconds 120 --special-action 20
        fs "pos|$FB_BOT|$((FB_X + 2))|$((FB_Y + 3))|$FB_Z|180" > /dev/null; sleep 5; shot remote20; check remote20
        mark remote20_off; fs "kick|$FB_BOT" > /dev/null; sleep 3
        mark remote68; bot BotPiss --mode idle --seconds 120 --special-action 68
        fs "pos|$FB_BOT|$((FB_X + 2))|$((FB_Y + 3))|$FB_Z|180" > /dev/null; sleep 5; shot remote68; check remote68
        mark remote68_off; fs "kick|$FB_BOT" > /dev/null; sleep 3
        mark remote_done
    fi
    # ---- FEAT-7: remote parachute (weapon 46 + PARACHUTE anims from the 207 animation field)
    if featb_want para; then
        # 3 × 0, then OPEN, FLOAT, STEERL, STEERR, DECEL, LAND_WATER (delta 4, loop), then 0.
        mark para; bot BotPara --mode idle --seconds 120 --anim-ms 2500 \
            --anim-seq 0,0,0,0x10403cb,0x10403c5,0x10403d0,0x10403d2,0x10403c3,0x10403c9,0,0,0
        fs "pos|$FB_BOT|$((FB_X + 3))|$((FB_Y + 3))|33.55|180" > /dev/null
        fs "weapon|$FB_BOT|46|1" > /dev/null
        sleep 9; shot para_open; sleep 5; shot para_steer; sleep 7; shot para_land
        check para
        mark para_resetw; fs "resetw|$FB_BOT" > /dev/null; sleep 4; shot para_gone; check para_gone
        mark para_off; fs "kick|$FB_BOT" > /dev/null; sleep 2
        mark para_done
    fi
    # ---- FEAT-11: guarded opcodes (add_car_component variant, then the generic one)
    if featb_want guard; then
        v="$(fs "veh|560|$((FB_X + 6))|$FB_Y|$FB_Z|0|1|1" | sed -nE 's/.*veh-created ([0-9]+).*/\1/p' | head -n 1)"
        sleep 2
        mark guard_vmod; fault_poke 0x6E3290 && { fs "vmod|${v:-1}|1010" > /dev/null; sleep 1.5; fault_restore 0x6E3290; }
        mark guard_phone; fault_poke 0x6348A0 && { fs "special|L|11" > /dev/null; sleep 1.5; fault_restore 0x6348A0; }
        fs "special|L|13" > /dev/null; sleep 1
        shot guard; check guard
        mark guard_done
    fi
    # ---- FEAT-8: /dl labels (vehicle near) and /testdw rows
    if featb_want dl; then
        [[ -n "${v:-}" ]] || { fs "veh|560|$((FB_X + 6))|$FB_Y|$FB_Z|0|1|1" > /dev/null; sleep 2; }
        mark dl_on; chat "/dl"; sleep 1.5; d3d dl; shot dl; check dl
        mark dl_off; chat "/dl"; sleep 1
        mark testdw; chat "/testdw"; sleep 1; d3d testdw; shot testdw; check testdw
        mark dl_done
    fi
    # ---- FEAT-12: an unhandled fault on the main thread → crash dialog (Close only; never "Report")
    if featb_want crash; then
        mark crash; ctl "poke 0x72B850 $FB_FAULT" > /dev/null
        sleep 6
        ctl window > "$FB_OUT/crash-window.txt"; shot crash; check crash
        ctl "window click 1001" > /dev/null
        sleep 6
    fi
}
