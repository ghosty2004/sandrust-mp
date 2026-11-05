#!/usr/bin/env python3
"""FEAT-SWEEP: cluster samp.dll (build B) functions into features and join coverage + citations.

samp_features.py COVERAGE.tsv DOCREFS.tsv UNREF.txt OUT_PREFIX
Label per function, first rule that applies:
 1. docs/phase3/data/samp-feature-ranges.tsv (libraries, dormant and undocumented clusters);
 2. the spec (docs/re/subsystems/*.md, else other docs) that cites it most specifically;
 3. the label of its callers (breadth-first, majority), then of its callees;
 4. the nearest labelled neighbour by address (MSVC keeps each source file contiguous).
Writes OUT_PREFIX.functions.tsv (va, label, executed scenarios, dead, docs, src) and
OUT_PREFIX.features.tsv (per label: functions, live, executed, never-executed live, cited by src).
"""
import bisect
import collections
import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.dirname(__file__))
from samp_graph import Graph  # noqa: E402

PREF = ["remote-control-onfoot", "remote-control-vehicles", "remote-control-combat", "remote-players-sync",
        "player-state", "vehicles", "world-entities", "ui-rendering", "ui-chat-audio", "session-misc",
        "gta-patches", "samp-saa", "handshake", "rpc-ids", "build-diff-r4"]


def main():
    cov, docrefs, unref, out = sys.argv[1:5]
    g = Graph()
    fs = sorted(g.funcs)
    ranges = []
    for l in open(os.path.join(ROOT, "docs", "phase3", "data", "samp-feature-ranges.tsv")):
        if l.startswith(("#", "start")) or not l.strip():
            continue
        a, b, n = l.rstrip("\n").split("\t")
        ranges.append((int(a, 16), int(b, 16), n))
    ex = {}
    for l in open(cov):
        if l.startswith("va_B"):
            continue
        a, n, sc, _ = l.rstrip("\n").split("\t", 3)
        ex[int(a, 16)] = sc
    dead = {int(x, 16) for x in open(unref).read().split()}
    docs, src = {}, {}
    for l in open(docrefs):
        if l.startswith("va_B"):
            continue
        a, d, s = l.rstrip("\n").split("\t")
        docs[int(a, 16)] = [x for x in d.split(",") if x]
        src[int(a, 16)] = [x for x in s.split(",") if x]
    lab = {}
    for f in fs:
        for a, b, n in ranges:
            if a <= f < b:
                lab[f] = n
                break
        else:
            names = [os.path.basename(p)[:-3] for p in docs.get(f, []) if p.endswith(".md")]
            for p in PREF:
                if p in names:
                    lab[f] = "spec:" + p
                    break
            else:
                notes = [n for n in names if n not in ("README", "ROADMAP", "WORKLOG")]
                if notes:
                    lab[f] = "notes:" + sorted(notes)[0]
    for direction in (g.callers, g.callees):
        for _ in range(4):
            new = {}
            for f in fs:
                if f in lab:
                    continue
                c = collections.Counter(lab[x] for x in direction.get(f, ()) if x in lab)
                if c:
                    new[f] = c.most_common(1)[0][0]
            if not new:
                break
            lab.update(new)
    known = sorted(f for f in fs if f in lab)
    for f in fs:
        if f in lab:
            continue
        i = bisect.bisect_left(known, f)
        cands = [known[j] for j in (i - 1, i) if 0 <= j < len(known)]
        lab[f] = "near:" + lab[min(cands, key=lambda k: abs(k - f))] if cands else "unknown"
    with open(out + ".functions.tsv", "w") as w:
        w.write("va_B\tsize\tlabel\texecuted_in\tdead\tdocs\tsrc\n")
        for f in fs:
            w.write(f"{f:#x}\t{g.funcs[f]['size']}\t{lab[f]}\t{ex.get(f, '')}\t{int(f in dead)}\t"
                    f"{','.join(os.path.basename(x) for x in docs.get(f, []))}\t"
                    f"{','.join(os.path.basename(x) for x in src.get(f, []))}\n")
    agg = collections.defaultdict(lambda: [0, 0, 0, 0, 0])
    for f in fs:
        k = lab[f].replace("near:", "")
        a = agg[k]
        a[0] += 1
        live = f not in dead and not g.funcs[f]["thunk"]
        a[1] += live
        a[2] += bool(ex.get(f))
        a[3] += live and not ex.get(f)
        a[4] += bool(src.get(f))
    with open(out + ".features.tsv", "w") as w:
        w.write("feature\tfunctions\tlive\texecuted\tnever_executed_live\tcited_by_our_sources\n")
        for k, a in sorted(agg.items(), key=lambda kv: -kv[1][0]):
            w.write(f"{k}\t" + "\t".join(map(str, a)) + "\n")
    print(open(out + ".features.tsv").read())


if __name__ == "__main__":
    main()
