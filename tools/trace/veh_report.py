#!/usr/bin/env python3
"""TRACE-4: summarise a `run.sh orig|ours vehicles` trace.

    tools/trace/veh_report.py target/trace/orig-latest

Prints, from trace.jsonl:
  1. marks;
  2. every vehicle creation frame (CVehicle::CVehicle): the client's script commands and GTA
     calls in that frame (per-frame background commands dropped), plus model-load polls;
  3. per-frame script commands on cars (counts per phase between marks);
  4. CAutomobile::ProcessControl context (`vpc`) by driver kind and stage;
  5. per-frame car state changes by stage (`rveh`) for cars with a remote driver, position jumps;
  6. enter / exit: player-ped task changes and seat opcodes, in order;
  7. vehicle removal frames (DELETE_CAR and friends).
"""
import collections
import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from remote_report import BACKGROUND_OPS, VTS, op_name, short  # noqa: E402

CAR_OPS = {"0x00a5", "0x00a6", "0x0175", "0x09c4", "0x07ff", "0x053f", "0x02ac", "0x036a", "0x0430", "0x0362",
           "0x05cb", "0x05ca", "0x05cd", "0x0840", "0x06e7", "0x06ed", "0x0164", "0x07ac", "0x0224", "0x0229",
           "0x0508", "0x03a2", "0x03f3", "0x020a", "0x0186", "0x0396", "0x0397", "0x0399"}
SKIP_FNS = {"pc", "rped", "rveh", "vpc", "watch", "globals", "guard", "pos", "players_mem", "suppressed"}


def phase_of(marks, f):
    cur = "start"
    for mf, t in marks:
        if mf <= f:
            cur = t.split(" pid=")[0]
    return cur


def main():
    d = sys.argv[1] if len(sys.argv) > 1 else "target/trace/orig-latest"
    rows = [json.loads(l) for l in open(os.path.join(d, "trace.jsonl"))]
    by_frame = collections.defaultdict(list)
    for r in rows:
        by_frame[r["frame"]].append(r)
    marks = [(r["frame"], r["args"]["text"]) for r in rows if r["fn"] == "mark"]
    print("== 1. marks")
    for f, t in marks:
        print(f"   frame {f}: {t}")

    print("\n== 2. vehicle creation frames")
    for f in sorted({r["frame"] for r in rows if r["fn"] == "CVehicle::CVehicle"}):
        print(f"--- frame {f} ({phase_of(marks, f)})")
        polls = collections.Counter()
        for r in by_frame[f]:
            if r["fn"] in SKIP_FNS:
                continue
            if r["fn"] == "scm" and r["args"].get("op") in BACKGROUND_OPS:
                if r["args"]["op"] in ("0x0248", "0x0247") and r["args"].get("params", [0])[0] in range(400, 612):
                    polls[f"{op_name(r['args']['op'])} {r['args']['params']}"] += 1
                continue
            print("   ", short(r)[:260])
        for k, n in polls.items():
            print(f"    (model poll) {n} x {k}")

    print("\n== 3. car script commands per phase (count; frames)")
    per = collections.defaultdict(lambda: collections.Counter())
    frames = collections.defaultdict(set)
    for r in rows:
        if r["fn"] == "scm" and r["args"].get("op") in CAR_OPS:
            ph = phase_of(marks, r["frame"])
            k = f"{op_name(r['args']['op'])} {r['args'].get('params')}"
            per[ph][k] += 1
            frames[(ph, k)].add(r["frame"])
    for ph in [t.split(" pid=")[0] for _, t in marks]:
        if ph in per:
            print(f"   [{ph}]")
            for k, n in per[ph].most_common(12):
                print(f"     {n:6d} in {len(frames[(ph, k)]):5d} frames  {k}")
            per.pop(ph)

    print("\n== 4. CAutomobile::ProcessControl context (vpc)")
    keys = ("pif", "pad", "cam_mode", "weapon_mode", "look_lr")
    ctx = collections.Counter()
    zfix = collections.Counter()
    for r in rows:
        if r["fn"] != "vpc":
            continue
        a = r["args"]
        pre = a.get("pre", {}) or {}
        b = pre.get("before") or a.get("in") or {}
        drv = (b.get("car") or {}).get("driver")
        kind = "none" if not drv else ("local" if drv.get("player") else (f"remote slot{drv.get('slot')}" if "slot" in drv else "ped"))
        ph = phase_of(marks, r["frame"])
        for st, s in (("before", pre.get("before")), ("in", a.get("in")), ("out", a.get("out")), ("after", pre.get("after"))):
            if s:
                c = {k: s.get(k) for k in keys}
                c["status"] = (s.get("car") or {}).get("status")
                ctx[(kind, st, json.dumps(c))] += 1
        if "orig_called" in a:
            ctx[(kind, "orig_called", "false")] += 1
        o, af = a.get("out"), pre.get("after")
        if o and af and o.get("car", {}).get("pos") and af.get("car", {}).get("pos"):
            dz = af["car"]["pos"][2] - o["car"]["pos"][2]
            dvz = (af["car"]["speed"] or [0, 0, 0])[2] - (o["car"]["speed"] or [0, 0, 0])[2]
            zfix[(kind, ph, abs(dz) > 1e-5 or abs(dvz) > 1e-6)] += 1
    for (kind, st, c), n in sorted(ctx.items()):
        print(f"   {kind:14s} {st:11s} x{n:5d} {c}")
    print("   z / move-speed z changed by the client after GTA's ProcessControl (kind, phase, changed): count")
    for k, n in sorted(zfix.items()):
        print(f"     {k}: {n}")

    print("\n== 5. per-frame car state (rveh), cars with a remote driver")
    rv = collections.defaultdict(dict)
    for r in rows:
        if r["fn"] == "rveh":
            rv[r["args"]["veh"]["ptr"]][r["frame"]] = r["args"]
    for ptr, fr in rv.items():
        where = collections.Counter()
        jumps = []
        nremote = 0
        for f in sorted(fr):
            a, nxt = fr[f], fr.get(f + 1)
            drv = (a.get("frame_start") or {}).get("driver") or {}
            if "slot" not in drv:
                continue
            nremote += 1
            st = [("frame_start", a.get("frame_start")), ("world_in", a.get("world_in")),
                  ("world_out", a.get("world_out")), ("next_start", (nxt or {}).get("frame_start"))]
            st = [s for s in st if s[1]]
            for (n1, s1), (n2, s2) in zip(st, st[1:]):
                for k in ("pos", "speed", "turn", "fwd"):
                    x, y = s1.get(k), s2.get(k)
                    if x and y and any(abs(x[i] - y[i]) > 1e-6 for i in range(3)):
                        where[(f"{n1} -> {n2}", k)] += 1
                if n1 == "world_out" and s1.get("pos") and s2.get("pos") and math.dist(s1["pos"], s2["pos"]) > 0.5:
                    jumps.append((f, s1["pos"], s2["pos"]))
        if not nremote:
            continue
        print(f"   car {ptr}: {nremote} frames with a remote driver")
        for (st, k), n in sorted(where.items()):
            print(f"     {st:28s} {k:6s} {n}")
        for f, a, b in jumps[:10]:
            print(f"     jump between frames {f}->{f + 1}: {a} -> {b} ({math.dist(a, b):.2f} m)")

    print("\n== 6. enter / exit (player peds' tasks, seat opcodes)")
    for r in rows:
        if r["fn"] in ("CTaskManager::SetTask", "CTaskManager::SetTaskSecondary"):
            p = r["args"].get("ped") or {}
            vt = r["args"].get("task_vt")
            name = VTS.get(int(vt, 16), vt) if vt else None
            if name and any(w in str(name) for w in ("Car", "Vehicle", "Drive", "Seat", "Door", "Jack")):
                who = "local" if p.get("player") else f"slot{p.get('slot')}"
                print(f"   frame {r['frame']} ({phase_of(marks, r['frame'])}) {who} {r['fn'].split('::')[1]} {name} idx {r['args'].get('index')} <{r.get('caller')}>{' via ' + r['via'] if r.get('via') else ''}")
        elif r["fn"] == "scm" and r["args"].get("op") in ("0x036a", "0x0430", "0x0362", "0x05cb", "0x05ca", "0x05cd"):
            print(f"   frame {r['frame']} ({phase_of(marks, r['frame'])}) {short(r)[:200]}")

    print("\n== 7. removal frames")
    for f in sorted({r["frame"] for r in rows if r["fn"] == "scm" and r["args"].get("op") in ("0x00a6", "0x0362")}):
        print(f"--- frame {f} ({phase_of(marks, f)})")
        for r in by_frame[f]:
            if r["fn"] in SKIP_FNS or (r["fn"] == "scm" and r["args"].get("op") in BACKGROUND_OPS):
                continue
            print("   ", short(r)[:220])


if __name__ == "__main__":
    main()
