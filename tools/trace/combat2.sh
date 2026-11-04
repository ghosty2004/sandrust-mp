#!/usr/bin/env bash
# TRACE-7: combat / world measurements in a running hold session
# (TRACE_GAMEMODE=tracegm [TRACE_LAGCOMP=N] [SAMP_TRACER_WATCH_MODELS=4019,19300] tools/trace/run.sh orig hold).
#   combat2.sh PART...   with TRACE_SESSION=<id> SAMP_RS_TRACE_LOCK_HELD=1
# Parts (each starts and removes its own bots; marks "<part>…" in trace.jsonl):
#   cross     remote 206 hit type 0 whose camera ray crosses the local ped: ShootBot aims at TgtBot, which
#             stands below ground behind the local player, so that the crosshair ray (4.57° above / 2.56° right
#             of the synced camera front, measured in TRACE-6) passes through the local ped's torso
#   ht1       remote 206 hit type 1 at the local player (Deagle, bot 10 m ahead)
#   proj      local projectiles: grenade, molotov, satchel + detonator, RPG, heat seeker
#   satchel   one satchel thrown, then the detonator
#   rproj     remote projectiles from keys: grenade, RPG, satchel (bot holds FIRE while it has ammo)
#   gap       explosions at an idle bot (single, then 3 in one server tick): remote health mid-frame
#   masks     jump / sprint / crouch with no special action, carry (25) and cuffs (24; + Deagle given)
#   skills    weapon-skill stats (peek) after spawn and after RPC 34; remote Colt at skill 999 / 1000 (screenshots)
#   rskill    remote Colt at pistol skill 0 / 1000 (screenshots)
#   driveby   local passenger and driver drive-by (Uzi), then a remote passenger drive-by bot
#   rmb       RemoveBuildingForPlayer(4019) then 3.3 km away and back (watch 4019,19300)
#   respawn   death after ForceClassSelection, then death while spectating
# COMBAT2_TARGET=TraceOurs when driving our client (bots aim at that name).
# Decode sends: samp-dump OUT/net.pcapng --port 7790 --raw | target/debug/examples/c2s_decode --only 206,115,…
set -uo pipefail
U="$(cd "$(dirname "$0")" && pwd)/ui.sh"
OUT="$(cd "$(dirname "$0")/../.." && pwd)/target/trace/${TRACE_SESSION%%.*}"
mark() { "$U" mark "$1 wall=$(date +%s.%N)" > /dev/null; }
fs() { "$U" fs "$1" > /dev/null; }
ndisc() { "$U" log 100000 | grep -c "\[ui-fs\] disconnect"; }
# Kill this script's bots by PID and wait until the server has dropped them (so "B1" is the next bot).
killbots() {
    local n before i
    n=$(grep -c . "$OUT/bots.pids" 2>/dev/null || echo 0)
    [ "$n" -gt 0 ] || return 0
    before=$(ndisc)
    while read -r p; do kill "$p" 2>/dev/null; done < "$OUT/bots.pids"
    : > "$OUT/bots.pids"
    for ((i = 0; i < 60; i++)); do
        [ "$(ndisc)" -ge $((before + n)) ] && return 0
        sleep 0.5
    done
    echo "combat2: bots did not leave" >&2
}
bot() { "$U" bot "$@" > /dev/null; }
reset_local() { fs 'hp|L|100'; fs 'resetw|L'; fs 'pos|L|1700|-2600|13.55|0'; }

part_cross() {
    reset_local
    bot TgtBot --mode idle --seconds 3000
    sleep 9
    fs 'pos|B1|1701.3|-2608|11.34|0'
    bot ShootBot --mode shoot --target TgtBot --weapon 24 --hit-type 0 --fire-ms 1500 --seconds 3000
    sleep 9
    fs 'pos|B2|1700|-2590|13.55|180'
    sleep 1
    mark cross-ht0
    fs 'weapon|B2|24|500'
    sleep 7
    killbots
    mark cross-ht0-done
}

part_ht1() {
    reset_local
    bot ShootBot1 --mode shoot --target "${COMBAT2_TARGET:-TraceOrig}" --weapon 24 --hit-type 1 --fire-ms 1500 --seconds 3000
    sleep 9
    fs 'pos|B1|1700|-2590|13.55|180'
    sleep 1
    mark ht1
    fs 'weapon|B1|24|500'
    sleep 5
    killbots
    mark ht1-done
}

part_proj() {
    reset_local
    sleep 1
    for w in 16 18; do
        fs "weapon|L|$w|3"
        sleep 1.5
        mark "proj$w"
        "$U" fire 700
        sleep 5
        fs 'hp|L|100'
    done
    # Satchel: throw, then the detonator.
    fs 'weapon|L|39|2'
    sleep 1.5
    mark proj39
    "$U" fire 400
    sleep 3
    mark proj40
    "$U" fire 150
    sleep 4
    fs 'hp|L|100'
    for w in 35 36; do
        fs "weapon|L|$w|3"
        sleep 1.5
        "$U" mouse mousedown 3
        sleep 1.5
        mark "proj$w"
        "$U" fire 150
        sleep 4
        "$U" mouse mouseup 3
        fs 'hp|L|100'
    done
    fs 'resetw|L'
    mark proj-done
}

part_satchel() {
    reset_local
    fs 'weapon|L|39|1'
    sleep 1.5
    mark satchel-throw
    "$U" fire 400
    sleep 3
    mark satchel-detonate
    "$U" fire 150
    sleep 4
    fs 'hp|L|100'
    mark satchel-done
}

part_rproj() {
    reset_local
    for w in 16 35 39; do
        bot "ProjBot$w" --mode shoot --target "${COMBAT2_TARGET:-TraceOrig}" --weapon "$w" --hit-type 0 --fire-ms 900 --seconds 3000
        sleep 9
        fs 'pos|B1|1700|-2580|13.55|180'
        sleep 1
        mark "rproj$w"
        fs "weapon|B1|$w|2"
        sleep 8
        fs 'hp|L|100'
        killbots
        mark "rproj$w-done"
        fs 'hp|L|100'
    done
}

part_gap() {
    reset_local
    fs 'pos|L|1700|-2550|13.55|180'
    bot GapBot --mode idle --seconds 3000
    sleep 9
    fs 'pos|B1|1700|-2600|13.55|0'
    sleep 2
    # Remote peds (CWorld::Players[2..3].m_pPed): entity flags (+0x1C, bit 0 uses-collision) and physical
    # flags (+0x40, bit 23 explosion-proof).
    local sl ped
    for sl in 2 3; do
        ped=$("$U" ctl "peek $(printf '0x%x' $((0xB7CD98 + sl * 0x190))) 4" | awk '{printf "0x%s%s%s%s", $6, $5, $4, $3}')
        [ "$ped" = 0x00000000 ] || [ -z "$ped" ] && continue
        echo "gap slot $sl ped $ped flags1c $("$U" ctl "peek $(printf '0x%x' $((ped + 0x1C))) 4") flags40 $("$U" ctl "peek $(printf '0x%x' $((ped + 0x40))) 4")" >> "$OUT/gap.txt"
    done
    mark gap1
    fs 'explode|1700|-2600|13.55|7|10'
    sleep 3
    mark gap3
    fs 'explodeN|1700|-2600|13.55|7|10|3'
    sleep 3
    mark gap6
    fs 'explodeN|1700.5|-2600|13.0|6|10|6'
    sleep 3
    killbots
    mark gap-done
}

masks_run() {
    local tag="$1"
    mark "mask-$tag"
    "$U" down w; sleep 1.2
    mark "mask-$tag-sprint"; "$U" down space; sleep 1.5; "$U" up space; sleep 0.5
    mark "mask-$tag-jump"; "$U" key shift; sleep 1.5
    mark "mask-$tag-crouch"; "$U" up w; sleep 0.5; "$U" key c; sleep 1.5; "$U" key c; sleep 1
}

part_masks() {
    reset_local
    sleep 1
    masks_run none
    fs 'pos|L|1700|-2600|13.55|0'
    fs 'special|L|25'; sleep 1
    masks_run carry
    fs 'special|L|0'; sleep 1
    fs 'pos|L|1700|-2600|13.55|0'
    fs 'special|L|24'; sleep 1
    masks_run cuffs
    mark mask-cuffs-weapon
    fs 'weapon|L|24|50'; sleep 2
    fs 'special|L|0'; sleep 1
    mark mask-cuffs-off
    fs 'weapon|L|24|50'; sleep 2
    fs 'resetw|L'
    mark masks-done
}

peek_skills() { echo "$1 $("$U" ctl 'peek 0xB79494 44')" >> "$OUT/skills.txt"; }

part_skills() {
    reset_local
    peek_skills spawn
    fs 'skill|L|0|500'; sleep 1; peek_skills after-34-pistol-500
    fs 'skill|L|1|0'; sleep 1; peek_skills after-34-silenced-0
    bot SkillBot --mode shoot --target "${COMBAT2_TARGET:-TraceOrig}" --weapon 22 --hit-type 0 --fire-ms 100000 --seconds 3000
    sleep 9
    fs 'pos|B1|1702.5|-2595|13.55|180'
    fs 'weapon|B1|22|500'
    sleep 1
    fs 'skill|B1|0|999'; sleep 2
    mark skills-999
    "$U" shot s-remote-colt-999 > /dev/null
    fs 'skill|B1|0|1000'; sleep 2
    mark skills-1000
    "$U" shot s-remote-colt-1000 > /dev/null
    killbots
    mark skills-done
}

# Remote Colt at pistol skill 0 and 1000 (single vs dual), same placement as `skills`.
part_rskill() {
    reset_local
    bot SkillBot --mode shoot --target "${COMBAT2_TARGET:-TraceOrig}" --weapon 22 --hit-type 0 --fire-ms 100000 --seconds 3000
    sleep 9
    fs 'pos|B1|1702.5|-2595|13.55|180'
    fs 'weapon|B1|22|500'
    sleep 1
    fs 'skill|B1|0|0'; sleep 2
    mark rskill-0
    "$U" shot s-remote-colt-0 > /dev/null
    fs 'skill|B1|0|1000'; sleep 2
    mark rskill-1000
    "$U" shot s-remote-colt-1000b > /dev/null
    killbots
    mark rskill-done
}

veh_id() { "$U" fs "$1" | sed -n 's/.*veh-created \([0-9]*\).*/\1/p'; }

part_driveby() {
    reset_local
    local v
    # Police car: a Sultan crashed the original client in GTA's radio stream thread (div by zero at 0x4F1464)
    # when the player entered it as a passenger; emergency vehicles start no radio station.
    v=$(veh_id 'veh|596|1704|-2595|13.55|0|1|1')
    sleep 2
    fs 'weapon|L|28|500'
    fs "putin|L|$v|1"
    sleep 3
    mark db-pass
    "$U" shot d-pass-0 > /dev/null
    "$U" fire 150; sleep 1.5
    "$U" shot d-pass-lmb > /dev/null
    # Static (B 0x10007b76): samp starts the passenger drive-by on the in-car ShockButtonL (horn key).
    mark db-pass-horn
    "$U" key h; sleep 1.5
    "$U" shot d-pass-h > /dev/null
    mark db-pass-fire
    "$U" fire 600; sleep 1
    for i in 1 2 3 4 5; do "$U" mouse mousemove_relative -- 30 0; sleep 0.1; done
    "$U" fire 600; sleep 1
    "$U" shot d-pass-aim > /dev/null
    "$U" mouse mousedown 3; sleep 1; "$U" shot d-pass-rmb > /dev/null; "$U" fire 400; "$U" mouse mouseup 3
    sleep 1.5
    mark db-driver
    fs "putin|L|$v|0"
    sleep 3
    "$U" down q; sleep 0.8; "$U" shot d-drv-q > /dev/null; "$U" fire 600; "$U" up q
    sleep 1
    "$U" down e; sleep 0.8; "$U" down Control_L; sleep 0.6; "$U" up Control_L; "$U" up e
    sleep 1.5
    mark db-remote
    fs 'pos|L|1700|-2610|13.55|0'
    sleep 2
    bot DbBot --mode passenger --vehicle "$v" --seat 1 --drive-by --seconds 3000
    sleep 9
    fs 'weapon|B1|28|500'
    sleep 1
    fs "putin|B1|$v|1"
    sleep 8
    "$U" shot d-remote > /dev/null
    killbots
    mark db-done
    fs "vdel|$v"
}

part_rmb() {
    reset_local
    fs 'pos|L|1777.8|-1760|13.5|180'
    sleep 6
    mark rmb
    fs 'rmb|L|4019|1777.8|-1773.9|12.5|0.25'
    sleep 3
    mark rmb-away
    fs 'pos|L|1500|1500|10.9|0'
    sleep 12
    mark rmb-back
    fs 'pos|L|1777.8|-1760|13.5|180'
    sleep 10
    "$U" shot r-back > /dev/null
    mark rmb-done
}

part_respawn() {
    reset_local
    sleep 1
    mark respawn-forceclass
    fs 'forceclass|L'
    fs 'kill|L'
    sleep 9
    "$U" shot rs-class > /dev/null
    mark respawn-forceclass-spawn
    "$U" key shift
    sleep 5
    bot SpecBot --mode idle --seconds 3000
    sleep 9
    fs 'pos|B1|1705|-2600|13.55|0'
    mark respawn-spectate
    fs 'spectate|L|B1'
    sleep 3
    mark respawn-spectate-kill
    fs 'kill|L'
    sleep 8
    "$U" shot rs-spec > /dev/null
    mark respawn-specoff
    fs 'specoff|L'
    sleep 8
    killbots
    mark respawn-done
}

for p in "$@"; do
    case "$p" in
        cross|ht1|proj|satchel|rproj|gap|masks|skills|rskill|driveby|rmb|respawn) "part_$p" ;;
        *) echo "combat2: unknown part $p" >&2; exit 2 ;;
    esac
done
"$U" log 200 | grep -E "shot|damage|death|spawn|state|edit|class" | tail -60 | cut -c30-
