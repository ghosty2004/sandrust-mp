#!/usr/bin/env python3
"""Compares the two sides of tools/diff/combatveh-measure.sh (DIR/orig and DIR/ours).

  python3 -I tools/diff/combatveh_eval.py DIR

obj   every OnPlayerWeaponShot: hit type, hit id (relative to the scenario's object id) and offset (±0.05)
rc    the remote RC driver's world links: whether CPhysical +0xB0 / +0xB4 were zero in every sample
surf  surf ids, enter requests and state changes after F: on the roof of a player-driven car, beside it, on a
      moving platform object, and next to a parked car while cuffed / carrying / free
modshop area / control state of the local passenger before, inside and after RPC 96 type 4, interior changes
train the server seat, PLAYER_STATE_PASSENGER within 1 s of G (warp) or later (enter animation), the
      local ped's passenger slot
Exit 0 = both sides agree on every item.
"""
import re
import sys
from pathlib import Path


def read(p):
    try:
        return Path(p).read_text()
    except OSError:
        return ""


def obj(side):
    t = read(side / "cv-obj.txt")
    m = re.search(r"^object (\d+)", t, re.M)
    oid = int(m.group(1)) if m else None
    shots = []
    for s in re.finditer(r"shot \d+ weapon (\d+) hittype (\d+) hitid (\d+) offset (\S+) (\S+) (\S+)", t):
        w, ht, hid = int(s.group(1)), int(s.group(2)), int(s.group(3))
        off = tuple(float(s.group(i)) for i in (4, 5, 6))
        rel = "object" if oid is not None and ht == 3 and hid == oid else hid
        shots.append((w, ht, rel, off))
    return oid, shots


def rc(side):
    t = read(side / "cv-rc.txt")
    samples = re.findall(r"entry_list (\d+) moving_list (\d+) vehicle (\d+) flags (\d+)", t)
    in_veh = all(int(f) & 0x100 for _, _, _, f in samples) if samples else None
    entry_zero = all(e == "0" for e, _, _, _ in samples) if samples else None
    moving_zero = all(m == "0" for _, m, _, _ in samples) if samples else None
    return {"samples": len(samples), "in_vehicle": in_veh, "entry_list_all_zero": entry_zero,
            "moving_list_all_zero": moving_zero}


def train(side):
    t = read(side / "cv-train.txt")
    ev = {}
    for line in read(side / "cv-events.txt").splitlines():
        ts, name = line.split(" ", 1)
        ev[name.strip()] = float(ts)
    where = re.search(r"where \d+ pos \S+ \S+ \S+ angle \S+ veh (\d+) seat (-?\d+) state (\d+)", t)
    lp = re.search(r"local ped \S+ vehicle (\S+)", t)
    slot = None
    if lp:
        ped = re.search(r"local ped (\S+)", t).group(1)
        for m in re.finditer(r"^passenger (\d) (\S+)", t, re.M):
            if m.group(2) == ped:
                slot = int(m.group(1))
    model = re.search(r"^driver \S+ model (\d+)", t, re.M)
    dt = ev["train-passenger"] - ev["train-key"] if "train-passenger" in ev and "train-key" in ev else None
    return {
        "seat": int(where.group(2)) if where else None,
        "state": int(where.group(3)) if where else None,
        "model": int(model.group(1)) if model else None,
        "passenger_slot": slot,
        "warped": None if dt is None else dt < 1.0,
    }


def surf(side):
    t = read(side / "cv-surf.txt")
    if "-- surfing" not in t:
        return None
    out = {}
    for part in t.split("-- ")[1:]:
        name = part.split("\n", 1)[0].strip()
        out[name] = {
            "enter_requests": re.findall(r"enter-request \d+ veh \d+ passenger (\d)", part),
            "state_changes": re.findall(r"\[ui-fs\] state \d+ (\d+->\d+)", part),
            "surfing": re.findall(r"surf \d+ vehicle (\d+) object (\d+)", part),
        }
    return out


def modshop(side):
    t = read(side / "cv-modshop.txt")
    if not t:
        return None
    areas = re.findall(r"^(\w+) curr_area (\S+) ped_area (\S+) veh_area (\S+)", t, re.M)
    pads = re.findall(r"^(\w+) pad peek \S+ (.*)$", t, re.M)
    return {
        "areas": areas,
        # Pad 0 is swapped with remote slots' pads while they are processed: a phase counts as "controls disabled"
        # when any sample has DisablePlayerControls bit 0x20.
        "controls_disabled": sorted({ph for ph, b in pads if int(b.split()[0], 16) & 0x20}),
        "interior_changes": re.findall(r"\[ui-fs\] interior \d+ (\d+->\d+)", t),
        "bot_events": re.findall(r"arg1 (\d) arg2 (\d+)", t),
    }


def main():
    d = Path(sys.argv[1])
    orig, ours = d / "orig", d / "ours"
    ok = True

    def cmp(name, a, b):
        nonlocal ok
        same = a == b
        ok &= same
        print(f"{'OK  ' if same else 'DIFF'} {name}\n      orig: {a}\n      ours: {b}")

    (oa, sa), (ob, sb) = obj(orig), obj(ours)
    print(f"obj: object id orig {oa} ours {ob}")
    key = lambda s: [(w, ht, rel) for w, ht, rel, _ in s]
    cmp("obj shots (weapon, hittype, hitid)", key(sa), key(sb))
    offs_ok = len(sa) == len(sb) and all(
        all(abs(x - y) <= 0.05 for x, y in zip(a[3], b[3])) for a, b in zip(sa, sb))
    ok &= offs_ok
    print(f"{'OK  ' if offs_ok else 'DIFF'} obj offsets ±0.05\n      orig: {[s[3] for s in sa]}\n      ours: {[s[3] for s in sb]}")
    if not sa:
        ok = False
        print("DIFF obj: no shots recorded")
    ra, rb = rc(orig), rc(ours)
    cmp("rc driver world links", {k: v for k, v in ra.items() if k != "samples"}, {k: v for k, v in rb.items() if k != "samples"})
    if not ra["samples"] or not rb["samples"]:
        ok = False
        print("DIFF rc: no samples")
    cmp("driver-enter gate: F surfing a car / beside it / surfing a platform / cuffed / carrying / free", surf(orig), surf(ours))
    cmp("mod-shop follow (RPC 96 type 4)", modshop(orig), modshop(ours))
    cmp("train passenger enter (570)", train(orig), train(ours))
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
