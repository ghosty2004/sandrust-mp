#!/usr/bin/env python3
"""FEAT-SWEEP: merge samp.dll module-coverage dumps (samp-tracer SAMP_TRACER_COV_MOD).

samp_cov.py OUT.tsv LABEL=DUMP [LABEL=DUMP ...]
Each dump's `# module samp.dll base=0x…` line rebases its hits to build-B addresses
(0x10000000 + RVA). Output: one line per armed samp.dll function (target/cov/samp-arm.txt):
  va_B  n_scenarios  scenarios(comma)  windows(first scenario:window list, truncated)
and a summary on stdout.
"""
import collections
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
IMAGE = 0x10000000


def load(path):
    base = None
    hits = collections.defaultdict(set)
    for line in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"# module (\S+) base=(0x[0-9a-f]+)", line)
        if m:
            base = int(m.group(2), 16)
            continue
        if line.startswith("hit\t"):
            f = line.split("\t")
            hits[int(f[2], 16)].add(f[1])
    if base is None:
        return {}
    out = {}
    for a, ws in hits.items():
        if base <= a < base + 0x200000:
            out[IMAGE + a - base] = ws
    return out


def main():
    out = sys.argv[1]
    arm = []
    for l in open(os.path.join(ROOT, "target", "cov", "samp-arm.txt")):
        if l.startswith("#") or not l.strip():
            continue
        arm.append(IMAGE + int(l.split()[0], 16))
    per = collections.defaultdict(dict)
    labels = []
    for spec in sys.argv[2:]:
        label, path = spec.split("=", 1)
        labels.append(label)
        for a, ws in load(path).items():
            per[a].setdefault(label, set()).update(ws)
    with open(out, "w") as w:
        w.write("va_B\tn\tscenarios\twindows\n")
        for a in arm:
            d = per.get(a, {})
            win = ";".join(f"{k}:{','.join(sorted(v))}" for k, v in sorted(d.items()))[:200]
            w.write(f"{a:#x}\t{len(d)}\t{','.join(sorted(d))}\t{win}\n")
    hit = sum(1 for a in arm if per.get(a))
    print(f"armed {len(arm)}; executed in any scenario {hit}; never {len(arm) - hit}")
    for l in labels:
        print(f"  {l}: {sum(1 for a in arm if l in per.get(a, {}))}")


if __name__ == "__main__":
    main()
