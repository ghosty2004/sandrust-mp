#!/usr/bin/env python3
"""FEAT-SWEEP: samp.dll functions never executed by any scenario, with a static trigger hint.

samp_never.py FEAT.functions.tsv OUT.tsv
FEAT.functions.tsv comes from samp_features.py; the RPC handler addresses (build B column) are
read from docs/re/rpc-ids.md. Library, runtime, dormant and dead (unreferenced) functions are left out.
Trigger hint = the RPC handlers among the function's ancestors (callers, up to 6 levels), else
the nearest documented ancestor, plus the strings the function itself uses.
"""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from samp_graph import Graph  # noqa: E402

SKIP = ("RakNet", "DXUT", "EH unwind", "MSVC CRT", "detours", "big-int", "SHA-1", "(dormant)")


def main():
    feat, out = sys.argv[1:3]
    g = Graph()
    rpc = {}
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    for l in open(os.path.join(root, "docs", "re", "rpc-ids.md"), encoding="utf-8"):
        c = [x.strip() for x in l.split("|")]
        if len(c) > 6 and c[1].isdigit() and c[4].startswith("0x1"):
            rpc[int(c[4], 16)] = f"RPC {c[1]} {c[6].replace('open.mp: ', '')[:28]}"
    rows = []
    info = {}
    for l in open(feat):
        if l.startswith("va_B"):
            continue
        va, size, lab, ex, dead, docs, src = l.rstrip("\n").split("\t")
        info[int(va, 16)] = (lab, ex, dead, docs)
    # Spec section that mentions the function (entry address, build A or B): its nearest heading.
    import json
    import re
    a2b_path = os.path.join(root, "target", "featsweep", "a2b.json")
    b2a = {}
    if os.path.exists(a2b_path):
        b2a = {int(v, 16): int(k, 16) for k, v in json.load(open(a2b_path)).items()}
    sect = {}
    for d in ("re/subsystems", "re", "phase3"):
        base = os.path.join(root, "docs", d)
        for fn in sorted(os.listdir(base)):
            if not fn.endswith(".md"):
                continue
            head = ""
            for line in open(os.path.join(base, fn), encoding="utf-8"):
                if line.startswith("#"):
                    head = line.strip("# \n")[:60]
                for m in re.finditer(r"0x1[0-9a-fA-F]{7}", line):
                    sect.setdefault(int(m.group(0), 16), f"{fn[:-3]}: {head}")
    n = collections.Counter()
    with open(out, "w") as w:
        w.write("va_B\tsize\tfeature\tdocs\ttrigger_hint\tstrings\tspec_section\n")
        for f in sorted(info):
            lab, ex, dead, docs = info[f]
            if ex or dead == "1" or g.funcs[f]["thunk"] or any(s in lab for s in SKIP):
                continue
            # ancestors
            seen, fr, hits = {f}, [f], []
            for _ in range(6):
                nx = []
                for x in fr:
                    if x in rpc:
                        hits.append(rpc[x])
                    for c in g.callers.get(x, ()):
                        if c not in seen:
                            seen.add(c)
                            nx.append(c)
                fr = nx
            hint = "; ".join(sorted(set(hits))[:4])
            if not hint:
                anc = [x for x in seen if x != f and info.get(x, ("", "", "", ""))[3]]
                hint = "via " + ",".join(hex(x) for x in sorted(anc)[:3]) if anc else (
                    "no callers (indirect/vtable)" if not g.callers.get(f) else "")
            strs = " | ".join(t[:40] for _, t in g.strs.get(f, []))[:120]
            lab = lab.replace("near:", "")
            n[lab] += 1
            sec = sect.get(b2a.get(f, -1)) or sect.get(f, "")
            w.write(f"{f:#x}\t{g.funcs[f]['size']}\t{lab}\t{docs}\t{hint}\t{strs}\t{sec}\n")
    for k, v in n.most_common():
        print(f"{v:5d} {k}")


if __name__ == "__main__":
    main()
