#!/usr/bin/env bash
# Parity scenario: bullet hit on an object, the remote driver of an RC vehicle, F while surfing, the local
# passenger enter of a train carriage (model 570). Sourced by tools/diff/visual.sh through VISUAL_SCENARIO_FILE
# (VISUAL_DIR = its own output root, VISUAL_ONLY=none); run on both clients, evaluate with
# tools/diff/combatveh_eval.py VISUAL_DIR.
#   obj   The local player faces a container (object, server id from `obj`) 8 m ahead with a Deagle and
#         fires twice un-aimed. Server: OnPlayerWeaponShot hittype / hitid / offset (`[ui-fs] shot`).
#   rc    samp-bot "RcBot" drives (speed 0) an RC Bandit (441) next to the local player. The remote ped's
#         world links are sampled through the tracer: CPhysical +0xB0 (sector entry list) and +0xB4 (moving
#         list node); `rc-*.png` screenshots from a fixed camera.
#   surf  samp-bot "SurfBot" drives (speed 0) an Infernus; the local player is dropped onto its roof (surfing a
#         player-driven vehicle) and presses F; then, standing beside the car, F again; then F while standing on a
#         slowly moving platform object next to a parked car (surfing it). Then F next to a parked car while cuffed
#         (special action 24), carrying (25) and free (0) — the driver-enter gate's refusal rules. Server: surf
#         ids, enter requests / state changes.
#   modshop samp-bot "ShopBot" drives (speed 0) a car with the local player as passenger and sends RPC 96 type 4
#         ("entered mod shop", interior 1, then "left"); the server relays it to the passenger. Local area / freeze /
#         control state from GTA memory before, inside, after; OnPlayerInteriorChange on the server.
#   train The local player walks up to the middle carriage (570) of the static Brown Streak at Unity
#         Station and presses G. Server: the state change to PASSENGER (timing) and the seat (`where`);
#         the local ped's vehicle and passenger slot from GTA memory.
# Output per side: $OUT/cv-*.txt (+ screenshots).

cv_peek() { tctl "peek $1 $2"; }
cv_u32() {   # cv_u32 ADDR → decimal (empty on failure)
    python3 -I - "$(cv_peek "$(printf '%#x' "$1")" 4)" <<'PY'
import sys
w = sys.argv[1].split()
if len(w) >= 6 and w[0] == "peek":
    print(int.from_bytes(bytes(int(b, 16) for b in w[2:6]), "little"))
PY
}
# GTA vehicle pool (CPools::ms_pVehiclePool 0xB74494; CVehicle size 0xA18): "ptr model x y z" per live vehicle
# with MODEL (or every vehicle when empty).
cv_vehicles() {
    python3 -I - "$TPORT" "${1:-}" <<'PY'
import socket, struct, sys
port, want = int(sys.argv[1]), sys.argv[2]
def peek(addr, n):
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        s.sendall(("peek %#x %d\n" % (addr, n)).encode())
        w = s.makefile().readline().split()
    except OSError:
        return None
    if len(w) < 2 + n or w[0] != "peek":
        return None
    return bytes(int(b, 16) for b in w[2:2 + n])
pool = peek(0xB74494, 4)
pool = struct.unpack("<I", pool)[0] if pool else 0
hdr = peek(pool, 12) if pool else None
if hdr:
    objs, flags, size = struct.unpack("<IIi", hdr)
    fl = b""
    while len(fl) < size:  # the tracer's peek is limited (MAX_PEEK)
        part = peek(flags + len(fl), min(128, size - len(fl)))
        if not part:
            break
        fl += part
    for i, f in enumerate(fl):
        if f & 0x80:
            continue
        v = objs + i * 0xA18
        model = struct.unpack("<H", peek(v + 0x22, 2) or b"\0\0")[0]
        if want and str(model) != want:
            continue
        m = struct.unpack("<I", peek(v + 0x14, 4) or b"\0" * 4)[0]
        pos = struct.unpack("<3f", peek(m + 0x30, 12)) if m else (0.0, 0.0, 0.0)
        print("%#x %d %.2f %.2f %.2f" % (v, model, *pos))
PY
}
# Local interior state: GTA's current area (CGame::currArea 0xB72914), the local ped's and its vehicle's area code
# (CEntity +0x2F), the ped's frozen-position flag word (CPed +0x46C..+0x47B) and Pad 0's player-control bytes
# (CPad +0x10E..+0x111).
cv_area() {
    local lp lv
    lp="$(cv_u32 0xB7CD98)"
    lv="$(cv_u32 $((lp + 0x58C)))"
    {
        printf '%s curr_area %s ped_area %s veh_area %s\n' "$1" "$(cv_u32 0xB72914)" \
            "$(( $(cv_u32 $((lp + 0x2C))) >> 24 ))" "$( [[ -n "$lv" && "$lv" != 0 ]] && echo $(( $(cv_u32 $((lv + 0x2C))) >> 24 )) || echo - )"
        printf '%s pedflags %s\n' "$1" "$(cv_peek "$(printf '%#x' $((lp + 0x46C)))" 16)"
        # Pad 0 is swapped with the remote slots' pads during their processing: several samples.
        local n
        for n in 1 2 3 4 5; do
            printf '%s pad %s\n' "$1" "$(cv_peek "$(printf '%#x' $((0xB73458 + 0x10E)))" 2)"
        done
    } >> "$OUT/cv-modshop.txt"
}
cv_fire() {
    DISPLAY="$DISPLAY" xdotool mousedown 1
    sleep 0.15
    DISPLAY="$DISPLAY" xdotool mouseup 1
}
cv_log() { printf '%s %s\n' "$(date +%s.%N)" "$*" >> "$OUT/cv-events.txt"; }

visual_scenario() {
    # ---- obj: bullet hit on an object ---------------------------------------------------------------
    fs 'pos|L|1700.0|-2600.0|13.55|0.0'
    sleep 2
    fs 'obj|2932|1700.0|-2592.0|13.55|0.0|0.0|90.0|300.0'
    grep -oE "\[ui-fs\] $SEQ object [0-9]+" "$E2E_LOG" | awk '{print "object", $4}' > "$OUT/cv-obj.txt"
    fs 'weapon|L|24|200'
    sleep 2
    mark_log
    cv_log obj-fire1; cv_fire; sleep 1.5
    cv_log obj-fire2; cv_fire; sleep 2
    tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] shot ' >> "$OUT/cv-obj.txt"
    shot cv-obj
    fs "objdel|$(awk '/^object/ {print $2}' "$OUT/cv-obj.txt" | head -n 1)"
    fs 'resetw|L'

    # ---- rc: the remote driver of an RC Bandit ---------------------------------------------------------
    fs 'veh|441|1696.0|-2594.0|13.0|0.0|1|1'
    local rc
    rc="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    echo "rc vehicle ${rc:-?}" > "$OUT/cv-rc.txt"
    bot RcBot --mode drive --speed 0.0 --seconds 300 --vehicle "${rc:-1}"
    waitlog 'spawn [0-9]+' 30
    sleep 2
    fs 'pos|B1|1694.0|-2594.0|13.5|0.0'
    waitlog 'state [0-9]+ 1->2' 30 && cv_log rc-driving
    sleep 6
    fs 'campos|L|1700.0|-2594.0|15.0'
    fs 'camlook|L|1696.0|-2594.0|13.2'
    sleep 2
    local i ped
    for i in 1 2 3 4 5 6; do
        ped="$(python3 -I - "$TPORT" <<'PY'
import socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=5)
s.sendall(b"remote\n")
f = s.makefile()
head = f.readline().strip()
n = int(head.split("count=")[1].split()[0]) if "count=" in head else 0
for _ in range(n):
    l = f.readline()
    if "ped=" in l:
        print(l.split("ped=")[1].split()[0]); break
PY
)"
        if [[ -n "$ped" ]]; then
            echo "sample $i ped $ped entry_list $(cv_u32 $((ped + 0xB0))) moving_list $(cv_u32 $((ped + 0xB4))) vehicle $(cv_u32 $((ped + 0x58C))) flags $(cv_u32 $((ped + 0x46C)))" >> "$OUT/cv-rc.txt"
        else
            echo "sample $i no remote ped" >> "$OUT/cv-rc.txt"
        fi
        sleep 0.5
    done
    shot cv-rc
    fs 'camera|L'
    killbots
    fs "vdel|${rc:-0}"

    # ---- surf: F on the roof of a player-driven car ----------------------------------------------------------
    fs 'veh|411|1700.0|-2560.0|13.5|0.0|1|1'
    local car
    car="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    echo "car ${car:-?}" > "$OUT/cv-surf.txt"
    bot SurfBot --mode drive --speed 0.0 --seconds 300 --vehicle "${car:-1}"
    waitlog 'spawn [0-9]+' 30
    sleep 2
    fs 'pos|B2|1698.0|-2560.0|13.5|0.0'
    waitlog 'state [0-9]+ 1->2' 30 && cv_log surf-car-driven
    sleep 4
    # Dropped onto the roof: a player-driven pool vehicle under the ped latches the surf state.
    fs 'pos|L|1700.0|-2560.3|15.6|0.0'
    sleep 4
    mark_log
    fs 'surf|L'
    cv_log surf-key; key f
    sleep 5
    echo "-- surfing" >> "$OUT/cv-surf.txt"
    tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (state|enter-request|[0-9]+ surf) ' >> "$OUT/cv-surf.txt"
    fs 'where|L'
    shot cv-surf
    # Control: beside the car on the ground (the teleport detaches the surf state), F enters (jacks) it.
    fs 'pos|L|1702.5|-2560.0|13.6|90.0'
    sleep 4
    mark_log
    fs 'surf|L'
    cv_log surf-control-key; key f
    sleep 6
    echo "-- ground" >> "$OUT/cv-surf.txt"
    tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (state|enter-request|[0-9]+ surf) ' >> "$OUT/cv-surf.txt"
    fs 'pos|L|1700.0|-2575.0|13.6|0.0'
    sleep 2
    killbots
    fs "vdel|${car:-0}"
    # Moving platform object next to a parked car: F while standing on it.
    fs 'obj|3095|1700.0|-2560.0|12.55|0.0|0.0|0.0|300.0'
    local plat
    plat="$(grep -oE "\[ui-fs\] $SEQ object [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    fs 'veh|411|1706.5|-2560.0|13.5|0.0|1|1'
    car="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    fs 'pos|L|1703.5|-2560.0|14.2|90.0'
    sleep 3
    fs "objmove|${plat:-0}|1700.0|-2555.0|12.55|0.05"
    sleep 3
    echo "-- platform" >> "$OUT/cv-surf.txt"
    local k
    for k in 1 2 3; do
        fs 'surf|L'
        grep -E "\[ui-fs\] $SEQ surf " "$E2E_LOG" >> "$OUT/cv-surf.txt"
        sleep 0.5
    done
    fs 'where|L'
    grep -E "\[ui-fs\] $SEQ where " "$E2E_LOG" >> "$OUT/cv-surf.txt"
    mark_log
    cv_log surf-platform-key; key f
    sleep 5
    tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (state|enter-request) ' >> "$OUT/cv-surf.txt"
    shot cv-surf-platform
    fs 'pos|L|1700.0|-2575.0|13.6|0.0'
    sleep 2
    fs "vdel|${car:-0}"
    fs "objdel|${plat:-0}"

    # ---- restrained: F next to a parked car while cuffed (special action 24) / carrying (25) / free --------
    fs 'veh|411|1706.5|-2545.0|13.5|0.0|1|1'
    car="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    local sa
    for sa in 24 25 0; do
        fs 'pos|L|1703.8|-2545.0|13.6|90.0'
        sleep 2
        fs "special|L|$sa"
        sleep 2
        mark_log
        cv_log "restrained-$sa-key"; key f
        sleep 5
        echo "-- special$sa" >> "$OUT/cv-surf.txt"
        tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (state|enter-request) ' >> "$OUT/cv-surf.txt"
        fs 'special|L|0'
        fs 'pos|L|1700.0|-2575.0|13.6|0.0'
        sleep 3
    done
    fs "vdel|${car:-0}"

    # ---- modshop: RPC 96 type 4 relayed to the local passenger --------------------------------------------
    fs 'veh|411|1690.0|-2570.0|13.5|0.0|1|1'
    local shop
    shop="$(grep -oE "\[ui-fs\] $SEQ veh-created [0-9]+" "$E2E_LOG" | awk '{print $4}' | tail -n 1)"
    echo "shop vehicle ${shop:-?}" > "$OUT/cv-modshop.txt"
    bot ShopBot --mode drive --speed 0.0 --seconds 300 --vehicle "${shop:-1}" --modshop 12,10,1
    waitlog 'spawn [0-9]+' 30
    sleep 2
    fs 'pos|B3|1688.0|-2570.0|13.5|0.0'
    waitlog 'state [0-9]+ 1->2' 30 && cv_log modshop-driving
    mark_log
    fs "putin|L|${shop:-1}|1"
    sleep 6
    cv_area before
    # The bot sends "entered" 12 s and "left" 22 s after sitting down.
    sleep 10
    cv_area inside
    sleep 12
    cv_area after
    tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (interior|state) ' >> "$OUT/cv-modshop.txt"
    grep -E 'mod-shop' "$OUT/bot-ShopBot.out" >> "$OUT/cv-modshop.txt"
    shot cv-modshop
    fs "pos|L|1690.0|-2575.0|13.6|0.0"
    killbots
    fs "vdel|${shop:-0}"
    fs 'interior|L|0'

    # ---- train: local passenger enter of a 570 carriage -------------------------------------------------
    fs 'pos|L|1712.0|-1945.0|13.6|90.0'
    sleep 6
    for i in 1 2 3 4 5; do
        cv_vehicles > "$OUT/cv-train-pool.txt"
        grep -q ' 570 ' "$OUT/cv-train-pool.txt" && break
        sleep 2
    done
    # The middle carriage (570) by distance from the engine (538).
    local tgt
    tgt="$(python3 -I - "$OUT/cv-train-pool.txt" <<'PY'
import math, sys
rows = [l.split() for l in open(sys.argv[1]) if l.strip()]
eng = [r for r in rows if r[1] == "538"]
cars = [r for r in rows if r[1] == "570"]
if eng and cars:
    e = [float(v) for v in eng[0][2:5]]
    cars.sort(key=lambda r: math.dist(e, [float(v) for v in r[2:5]]))
    c = cars[len(cars) // 2]
    print(c[0], c[2], c[3], c[4])
PY
)"
    echo "target $tgt" > "$OUT/cv-train.txt"
    if [[ -n "$tgt" ]]; then
        local cx cy cz
        read -r _ cx cy cz <<< "$tgt"
        # 3.5 m east of the carriage (the line runs roughly north-south here), facing it.
        fs "pos|L|$(awk -v x="$cx" 'BEGIN{printf "%.2f", x + 3.5}')|$cy|$(awk -v z="$cz" 'BEGIN{printf "%.2f", z + 0.2}')|90.0"
        sleep 3
        mark_log
        cv_log train-key; key g
        waitlog 'state [0-9]+ 1->3' 15 && cv_log train-passenger
        sleep 4
        fs 'where|L'
        tail -n +"$((LOGMARK + 1))" "$E2E_LOG" | grep -E '\[ui-fs\] (state|enter-request|[0-9]+ where)' >> "$OUT/cv-train.txt"
        local lp lv
        lp="$(cv_u32 0xB7CD98)"
        lv="$(cv_u32 $((lp + 0x58C)))"
        echo "local ped $(printf '%#x' "${lp:-0}") vehicle $(printf '%#x' "${lv:-0}") flags $(cv_u32 $((lp + 0x46C)))" >> "$OUT/cv-train.txt"
        if [[ -n "$lv" && "$lv" != 0 ]]; then
            for i in 0 1 2 3 4 5 6 7; do
                echo "passenger $i $(printf '%#x' "$(cv_u32 $((lv + 0x464 + 4 * i)))")" >> "$OUT/cv-train.txt"
            done
            echo "driver $(printf '%#x' "$(cv_u32 $((lv + 0x460)))") model $(( $(cv_u32 $((lv + 0x22))) & 0xFFFF ))" >> "$OUT/cv-train.txt"
        fi
        shot cv-train
    fi
}
