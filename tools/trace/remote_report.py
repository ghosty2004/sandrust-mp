#!/usr/bin/env python3
"""TRACE-2: summarise a `run.sh orig|ours remote` trace (remote players).

    tools/trace/remote_report.py target/trace/orig-latest [--slot N]

Prints, from trace.jsonl:
  1. every stream-in (CREATE_PLAYER) frame: the client's script commands and
     the GTA calls made for the remote ped, in order;
  2. per-frame behaviour: which values change where (frame start / world /
     ProcessControl / after the world), the swapped game context inside the
     ped's ProcessControl, and the per-frame client calls;
  3. every destruction frame (CPed::~CPed of a player-slot ped);
  4. CWorld::Players[2..] memory: m_pPed changes and guard-page access sites by module.
Task vtables are named from re/gta-reversed (RH_ScopedVirtualClass) and opcodes from
eScriptCommands.h, when those files are present.
"""
import collections
import json
import math
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
GR = os.path.join(ROOT, "re/gta-reversed/source")
# Script commands samp issues every frame regardless of remote players (trace-notes.md).
BACKGROUND_OPS = {"0x00c0", "0x04ee", "0x04ed", "0x0248", "0x0247", "0x052c", "0x09ba", "0x077e"}


def opcode_names():
    names, i = {}, 0
    p = os.path.join(GR, "game_sa/Enums/eScriptCommands.h")
    if not os.path.exists(p):
        return names
    for line in open(p, errors="ignore"):
        m = re.match(r"\s*COMMAND_(\w+)\s*(?:=\s*(0x[0-9A-Fa-f]+|\d+))?\s*,", line)
        if m:
            if m.group(2):
                i = int(m.group(2), 0)
            names[i] = m.group(1)
            i += 1
    return names


def vtable_names():
    names = {}
    if not os.path.isdir(GR):
        return names
    for root, _, files in os.walk(GR):
        for f in files:
            if f.endswith(".cpp"):
                for line in open(os.path.join(root, f), errors="ignore"):
                    m = re.search(r"RH_ScopedVirtualClass\((\w+),\s*(0x[0-9A-Fa-f]+)", line)
                    if m:
                        names[int(m.group(2), 16)] = m.group(1)
    return names


OPS = opcode_names()
VTS = vtable_names()


def op_name(op):
    return f"{op} {OPS.get(int(op, 16), '?')}"


def short(r):
    a = r.get("args", {})
    if r["fn"] == "scm":
        out = f" -> {','.join(a['out'])}" if a.get("out") else ""
        return f"scm {op_name(a['op'])} {a.get('params')}{out}"
    a = dict(a)
    for k in ("task_vt",):
        if k in a and a[k]:
            a[k] = VTS.get(int(a[k], 16), a[k])
    for k, v in list(a.items()):
        if isinstance(v, dict) and "ptr" in v:
            a[k] = f"{v.get('type')}#{v.get('model')}" + (f"/slot{v['slot']}" if "slot" in v else "") + (
                "/local" if v.get("player") else "")
    via = f" [{r['via']}]" if r.get("via") else ""
    caller = f" <{r['caller']}>" if r.get("caller", "").startswith(("samp", "gta_sa.exe+0x20", "gta_sa.exe+0x6")) else ""
    return f"{r['fn']}{via}{caller} {json.dumps(a)}"


def main():
    d = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target/trace/orig-latest")
    slot = int(sys.argv[sys.argv.index("--slot") + 1]) if "--slot" in sys.argv else None
    rows = [json.loads(l) for l in open(os.path.join(d, "trace.jsonl"))]
    by_frame = collections.defaultdict(list)
    for r in rows:
        by_frame[r["frame"]].append(r)

    def frame_dump(f, title):
        print(f"--- frame {f}: {title}")
        for r in by_frame[f]:
            if r["fn"] in ("pc", "rped", "watch", "globals", "guard", "pos", "players_mem", "CStreaming::RemoveModel"):
                continue
            if r["fn"] == "scm" and r["args"].get("op") in BACKGROUND_OPS:
                continue
            print("   ", short(r))

    print("== marks")
    for r in rows:
        if r["fn"] == "mark":
            print(f"   frame {r['frame']}: {r['args']['text']}")

    print("\n== 1. stream-in")
    creates = [r["frame"] for r in rows if r["fn"] == "CPlayerPed::SetupPlayerPed" and r["args"].get("player", 0) >= 2]
    for f in creates:
        frame_dump(f, "CREATE_PLAYER")
        frame_dump(f + 1, "next frame")

    print("\n== 2. per frame")
    rped = {}
    pcs = {}
    for r in rows:
        a = r["args"] if isinstance(r.get("args"), dict) else {}
        if r["fn"] == "rped" and (slot is None or a["slot"] == slot):
            rped[r["frame"]] = a
        if r["fn"] == "pc" and "slot" in a.get("ped", {}) and (slot is None or a["ped"]["slot"] == slot):
            pcs[r["frame"]] = a
    print(f"   frames with a remote ped: {len(rped)}, ProcessControl records: {len(pcs)}")

    def changed(x, y, k):
        if not x or not y or k not in x or k not in y:
            return False
        if isinstance(x[k], list):
            return any(abs(x[k][i] - y[k][i]) > 1e-6 for i in range(3))
        return abs(x[k] - y[k]) > 1e-6

    where = collections.Counter()
    jumps = []
    for f, r in rped.items():
        p = pcs.get(f, {})
        stages = [("frame_start", r.get("frame_start")), ("world_in", r.get("world_in")),
                  ("pc_before_client", p.get("pre", {}).get("before")), ("pc_in", p.get("in")),
                  ("pc_out", p.get("out")), ("pc_after_client", p.get("pre", {}).get("after")),
                  ("world_out", r.get("world_out")), ("next_frame_start", rped.get(f + 1, {}).get("frame_start"))]
        stages = [s for s in stages if s[1]]
        for (n1, s1), (n2, s2) in zip(stages, stages[1:]):
            for k in ("pos", "speed", "heading", "heading_goal"):
                if changed(s1, s2, k):
                    where[(f"{n1} -> {n2}", k)] += 1
        a, b = r.get("world_out"), rped.get(f + 1, {}).get("frame_start")
        if a and b and math.dist(a["pos"], b["pos"]) > 1e-6:
            jumps.append((f, a["pos"], b["pos"]))
    print("   value changes by stage (count of frames):")
    for (st, k), n in sorted(where.items()):
        print(f"     {st:40s} {k:13s} {n}")
    for f, a, b in jumps:
        print(f"   position written between frames {f}->{f + 1}: {a} -> {b} ({math.dist(a, b):.3f} m)")
    ctx = collections.Counter()
    keys = ("pif", "pad", "cam_mode", "weapon_mode", "fov", "aspect", "skill0", "front", "upd_call", "aimrot_a", "aimrot_b")
    for a in pcs.values():
        for stage, s in (("before", a.get("pre", {}).get("before")), ("in", a.get("in")), ("after", a.get("pre", {}).get("after"))):
            if s:
                ctx[(stage, json.dumps({k: s.get(k) for k in keys}))] += 1
    print("   game context around the remote ped's ProcessControl (before client wrapper / inside GTA's / after):")
    for (stage, c), n in sorted(ctx.items()):
        print(f"     {stage:6s} x{n}: {c}")
    callers = collections.Counter((a.get("vtable_target"), a.get("orig_caller")) for a in pcs.values())
    print(f"   vtable target / caller of 0x60EA90: {dict(callers)}")
    calls = collections.Counter()
    for f in rped:
        for r in by_frame[f]:
            if r["fn"] == "scm":
                calls[op_name(r["args"]["op"])] += 1
            elif r["fn"] not in ("pc", "rped", "watch", "globals", "guard", "pos", "players", "players_mem", "mark"):
                s = json.dumps(r.get("args"))
                if '"slot"' in s:
                    calls[f"{r['fn']} (remote ped)"] += 1
    print("   client script commands / GTA calls on remote ped while it exists (total):")
    for k, n in calls.most_common(25):
        print(f"     {n:6d} {k}")

    print("\n== 3. destruction")
    for r in rows:
        if r["fn"] == "CPed::~CPed" and "slot" in r["args"]["this"]:
            print(f"   origin {r.get('origin')}")
            frame_dump(r["frame"], "destructor")

    print("\n== 4. CWorld::Players[2..] memory")
    for r in rows:
        if r["fn"] in ("players", "players_mem_base") or (r["fn"] == "players_mem" and any(k.endswith("+0x0") for k in r["args"])):
            print(f"   frame {r['frame']} {r['fn']}: {json.dumps(r['args'])[:300]}")
    gs = [r for r in rows if r["fn"] == "guard_summary"]
    if gs:
        a = gs[-1]["args"]
        print(f"   guard: {a['frames']} frames, {a['hits']} accesses, over-budget frames {a['over_budget_frames']}")
        mods = collections.Counter()
        for s in a["sites"]:
            mods[s.split("+")[0]] += int(s.rsplit("x", 1)[1])
            if not s.startswith("gta_sa.exe"):
                print(f"     non-GTA site: {s}")
        print(f"     accesses by module: {dict(mods)}")
        slots = collections.Counter(re.search(r"slot(\d+)", s).group(1) for s in a["sites"])
        print(f"     distinct sites per slot: {dict(slots)}")
    else:
        print("   (no guard records: run with SAMP_TRACER_GUARD=<budget>)")


if __name__ == "__main__":
    main()
