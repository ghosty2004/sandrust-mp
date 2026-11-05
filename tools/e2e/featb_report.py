#!/usr/bin/env python3
"""FEAT-B report: per-feature assertions on a featb run (tools/e2e/featb.sh) and, with --orig, the parity diff of
the script commands both clients issued per scenario step.

  featb_report.py RUN [--orig ORIG_RUN]… [--decisions TSV] [--only items,remote,…]

RUN / ORIG_RUN are run directories (trace.jsonl, server.log, chatlog.txt, window.txt, crash-window.txt,
d3d-*.jsonl). Exit 0 when every assertion holds on RUN and every diff line is empty or covered by a decision
(TSV: step<TAB>pattern<TAB>reason; a diff line containing pattern in that step is accepted).
"""
import argparse
import json
import os
import re
import sys
from collections import Counter

# Script commands of the FEAT-B features (held items, piss, remote parachute) compared per step.
OPS = {
    0x0107: "CREATE_OBJECT", 0x070A: "TASK_PICK_UP_OBJECT", 0x0812: "TASK_PLAY_ANIM_NON_INTERRUPTABLE",
    0x066A: "CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION", 0x064C: "PLAY_FX_SYSTEM", 0x0650: "KILL_FX_SYSTEM",
    0x069B: "ATTACH_OBJECT_TO_CHAR", 0x0750: "SET_OBJECT_VISIBLE", 0x08D2: "SET_OBJECT_SCALE",
    0x075A: "PLAY_OBJECT_ANIM", 0x018C: "ADD_ONE_OFF_SOUND", 0x0682: "DETACH_OBJECT", 0x09A2: "REMOVE_OBJECT_ELEGANTLY",
    0x06E7: "ADD_VEHICLE_MOD", 0x0729: "TASK_USE_MOBILE_PHONE",
}


def load_trace(run):
    """[(mark or None, record)] in file order; marks as ('mark', text)."""
    out = []
    p = os.path.join(run, "trace.jsonl")
    if not os.path.exists(p):
        return out
    with open(p, errors="replace") as f:
        for line in f:
            try:
                r = json.loads(line)
            except ValueError:
                continue
            fn = r.get("fn")
            if fn == "mark":
                out.append(("mark", r.get("args", {}).get("text", "")))
            elif fn == "scm":
                caller = r.get("caller", "")
                if "samp.dll" not in caller and "samp_client" not in caller:
                    continue
                a = r.get("args", {})
                try:
                    op = int(a.get("op", "0"), 16)
                except ValueError:
                    continue
                if op in OPS:
                    out.append(("scm", (op, a.get("params", []))))
    return out


# Parameter positions holding handles (objects, peds, effects, vehicles) or positions that differ between runs.
HANDLES = {
    0x0107: {4}, 0x070A: {0, 1}, 0x0812: {0}, 0x066A: {1, 9}, 0x064C: {0}, 0x0650: {0}, 0x069B: {0, 1},
    0x0750: {0}, 0x08D2: {0}, 0x075A: {0}, 0x0682: {0}, 0x09A2: {0}, 0x06E7: {0, 2}, 0x0729: {0},
}
POSITIONS = {0x0107: {1, 2, 3}, 0x018C: {0, 1, 2}}


def norm(op, i, v):
    if i in HANDLES.get(op, ()):
        return "H"
    if i in POSITIONS.get(op, ()):
        return "pos"
    if isinstance(v, float):
        return round(v, 3)
    return v


def segments(trace):
    """{step: Counter of normalised (op, params)} keyed by the preceding mark."""
    segs, cur = {}, None
    for kind, val in trace:
        if kind == "mark":
            cur = val
            segs.setdefault(cur, Counter())
        elif cur is not None:
            op, params = val
            segs[cur][(OPS[op],) + tuple(norm(op, i, p) for i, p in enumerate(params))] += 1
    return segs


def read(run, name):
    p = os.path.join(run, name)
    if not os.path.exists(p):
        return ""
    with open(p, errors="replace") as f:
        return f.read()


def pstates(server_log):
    """[(drunk, special, anim, weapon)] in order."""
    return [tuple(int(x) for x in m) for m in
            re.findall(r"pstate \d+ drunk (-?\d+) special (\d+) anim (\d+) weapon (\d+)", server_log)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--orig", action="append", help="original run(s); a step is taken from the last run that has it")
    ap.add_argument("--decisions")
    ap.add_argument("--only")
    a = ap.parse_args()
    only = set(a.only.split(",")) if a.only else None
    want = lambda k: only is None or k in only
    ok = True

    def check(name, cond, detail=""):
        nonlocal ok
        print(f"{'PASS' if cond else 'FAIL'} {name}{(': ' + detail) if detail and not cond else ''}")
        ok &= bool(cond)

    run = a.run
    trace = load_trace(run)
    segs = segments(trace)
    log = read(run, "server.log")
    chat = read(run, "chatlog.txt") + read(run, "chat.txt")
    print(f"# run {os.path.realpath(run)}: {len(trace)} records, {len(segs)} steps")

    win = read(run, "window.txt")
    check("FEAT-10 window title GTA:SA:MP", 'title="GTA:SA:MP"' in win, win[:200])
    m = re.search(r'title="GTA:SA:MP" style=0x[0-9a-f]+ cstyle=(0x[0-9a-f]+)', win)
    check("FEAT-10 class style CS_DBLCLKS", m is not None and int(m.group(1), 16) & 0x8, win[:200])

    def has(step, opname, *vals):
        for key in segs.get(step, {}):
            if key[0] == opname and all(v in key for v in vals):
                return True
        return False

    if want("items") and "sa20" in segs:
        ps = pstates(log)
        check("FEAT-5 beer object + pick-up", has("sa20", "CREATE_OBJECT", 1543) and has("sa20", "TASK_PICK_UP_OBJECT"))
        check("FEAT-5 drink anim on fire", has("sa20_fire", "TASK_PLAY_ANIM_NON_INTERRUPTABLE", "dnk_stndM_loop", "BAR"))
        check("FEAT-5 cigar anim on fire", has("sa21_fire", "TASK_PLAY_ANIM_NON_INTERRUPTABLE", "smkcig_prtl", "GANGS"))
        check("FEAT-5 wine/sprunk/cigar objects", has("sa22", "CREATE_OBJECT", 1486) and has("sa23", "CREATE_OBJECT", 1546)
              and has("sa21", "CREATE_OBJECT", 1485))
        check("FEAT-5 drop on enter/exit key", has("sa20_drop", "TASK_PICK_UP_OBJECT", 0))
        specials = [p[1] for p in ps]
        check("FEAT-5 207 reports 20/22/23/21", all(s in specials for s in (20, 22, 23, 21)), str(specials))
        drunk = [p[0] for p in ps]
        check("FEAT-5 drinks raise the drunk level", len(drunk) > 2 and max(drunk) >= 1250, str(drunk))
        check("FEAT-5 piss fx", has("sa68", "CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION", "PETROLCAN")
              and has("sa68", "TASK_PLAY_ANIM_NON_INTERRUPTABLE", "PISS_LOOP", "PAULNMAC"))
        check("FEAT-5 piss stops on enter/exit key", has("sa68_stop", "KILL_FX_SYSTEM"))
        check("FEAT-5 piss reported as 68", 68 in specials, str(specials))
    if want("remote") and "remote20" in segs:
        check("FEAT-5 remote beer object", has("remote20", "CREATE_OBJECT", 1543))
        check("FEAT-5 remote piss fx", has("remote68", "CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION", "PETROLCAN"))
    if want("para") and "para" in segs:
        check("FEAT-7 remote parachute object", has("para", "CREATE_OBJECT", 3131) and has("para", "ATTACH_OBJECT_TO_CHAR"))
        check("FEAT-7 PARA_OPEN sound", has("para", "ADD_ONE_OFF_SOUND", 1039))
        check("FEAT-7 object anims", has("para", "PLAY_OBJECT_ANIM", "PARA_FLOAT_O")
              and has("para", "PLAY_OBJECT_ANIM", "PARA_LAND_WATER_O"))
        check("FEAT-7 removed without weapon 46", has("para_resetw", "REMOVE_OBJECT_ELEGANTLY"))
    if want("guard") and "guard_vmod" in segs:
        check("FEAT-11 add_car_component warning",
              re.search(r"Warning\(add_car_component 1010\): Exception 0xC0000005 at 0x6E3290", chat) is not None)
        check("FEAT-11 opcode warning",
              re.search(r"Warning\(opcode 0x729\): Exception 0xC0000005 at 0x6348A0", chat) is not None)
    if want("dl") and "dl_on" in segs:
        dl = read(run, "d3d-dl.jsonl")
        check("FEAT-8 /dl label text", "[id: " in dl and "PassengerSeats: " in dl and "sPos: " in dl)
        dw = read(run, "d3d-testdw.jsonl")
        check("FEAT-8 /testdw rows", "Pooper" in dw and "PooperPooperPooper0001" in dw)
    if want("crash") and os.path.exists(os.path.join(run, "crash-window.txt")):
        cw = read(run, "crash-window.txt")
        check("FEAT-12 crash dialog", 'title="SA:MP An exception occured"' in cw and '"Close"' in cw
              and "Exception At Address: 0x0072B850" in cw, cw[:300])
        if "samp-rs " in cw:
            check("FEAT-12 no report button", "Report Crash" not in cw)
            check("FEAT-12 local report file", os.path.exists(os.path.join(run, "samp-rs-crash.txt")))

    if a.orig:
        dec = []
        if a.decisions and os.path.exists(a.decisions):
            for line in open(a.decisions):
                if line.strip() and not line.startswith("#"):
                    p = line.rstrip("\n").split("\t")
                    if len(p) >= 2:
                        dec.append((p[0], p[1]))
        osegs = {}
        for o in a.orig:
            osegs.update(segments(load_trace(o)))
        print(f"# parity diff against {', '.join(os.path.realpath(o) for o in a.orig)} (per step, feature script commands)")
        undecided = 0
        for step in [s for s in osegs if s in segs]:
            o, u = osegs[step], segs[step]
            lines = []
            for k in sorted(set(o) | set(u), key=str):
                # Presence per step (counts depend on the frame rate for per-frame commands).
                if (k in o) != (k in u):
                    lines.append(f"{'orig' if k in o else 'ours'} only: {k}")
            for l in lines:
                covered = any(s == step and pat in l for s, pat in dec)
                print(f"  {step}: {l}{'  [decision]' if covered else ''}")
                undecided += 0 if covered else 1
        check("parity diff empty or decided", undecided == 0, f"{undecided} undecided line(s)")
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
