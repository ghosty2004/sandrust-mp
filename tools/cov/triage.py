#!/usr/bin/env python3
"""COV-1 triage helper: for every function hit only with our client, the samp.dll patch
rows (docs/re/data/gta-patches-r4-annotated.csv; row = CSV line number) that could be what
stops it in the original, found statically:

  * call-site: a row whose bytes cover the call instruction before the first caller's
    return address (samp NOPs or redirects that call);
  * in-caller: a row inside the first caller's function body;
  * entry: a row inside the function itself;
and, from target/diff/patch-coverage.json (F5), whether our client applies that row.

    triage.py [--diff target/cov/coverage-diff.tsv] [--never-only] [--out target/cov/triage.tsv]
"""
import argparse
import csv
import json
import os
import re
from bisect import bisect_right
from collections import defaultdict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--diff", default=os.path.join(ROOT, "target/cov/coverage-diff.tsv"))
    ap.add_argument("--inv", default=os.path.join(ROOT, "target/cov/gta-functions.tsv"))
    ap.add_argument("--out", default=os.path.join(ROOT, "target/cov/triage.tsv"))
    a = ap.parse_args()
    inv = {}
    for line in list(open(a.inv))[1:]:
        p = line.rstrip("\n").split("\t")
        inv[int(p[0], 16)] = (int(p[1]), p[2])
    addrs = sorted(inv)

    def containing(x):
        i = bisect_right(addrs, x)
        if i == 0:
            return None
        e = addrs[i - 1]
        return e if x < e + max(inv[e][0], 1) + 64 else None

    rows = []
    texts = {}  # row -> all its text (the effect column names the original call target)
    with open(os.path.join(ROOT, "docs/re/data/gta-patches-r4-annotated.csv")) as f:
        for n, r in enumerate(csv.reader(f), start=1):
            if n == 1:
                continue
            try:
                size = int(r[1])
            except ValueError:
                size = 4
            for t in re.findall(r"0x([0-9a-fA-F]+)", r[0]):
                rows.append((int(t, 16), max(size, 1), n, r[2], r[7][:90]))
            texts[n] = " ".join(r).lower()
    status = {}
    pc = os.path.join(ROOT, "target/diff/patch-coverage.json")
    if os.path.exists(pc):
        for r in json.load(open(pc))["results"]:
            status[r["row"]] = r["category"]

    def rows_in(lo, hi):
        return [r for r in rows if r[0] < hi and r[0] + r[1] > lo]

    out = ["window\taddr\tname\tcaller\thow\trow\trow_addr\tkind\tours\tpurpose"]
    seen = defaultdict(set)
    for line in list(open(a.diff))[1:]:
        p = line.rstrip("\n").split("\t")
        if len(p) < 7 or p[1] != "ours":
            continue
        w, e, name, caller = p[0], int(p[2], 16), p[3], p[6]
        cands = []
        for r in rows_in(e, e + max(inv.get(e, (1, ""))[0], 1)):
            cands.append(("entry", r))
        tag = f"{e:#x}"
        bare = name.split(" (in ")[0]
        for rn, txt in texts.items():
            if f"<{bare.lower()}>" in txt or f"call {tag} " in txt or f"call {tag}<" in txt:
                r = next(x for x in rows if x[2] == rn)
                cands.append(("mentions", r))
        raw = p[7] if len(p) > 7 else ""
        if raw.startswith("0x"):
            ret = int(raw, 16)
            for r in rows_in(ret - 6, ret):
                cands.append(("call-site", r))
            ce = containing(ret)
            if ce is not None:
                for r in rows_in(ce, ce + inv[ce][0]):
                    if ("call-site", r) not in cands:
                        cands.append(("in-caller", r))
        if not cands:
            cands = [("none", None)]
        for how, r in cands:
            key = (e, how, r[2] if r else 0)
            if key in seen[w]:
                continue
            seen[w].add(key)
            if r:
                out.append(f"{w}\t{e:#x}\t{name}\t{caller}\t{how}\t{r[2]}\t{r[0]:#x}\t{r[3]}\t{status.get(r[2], '?')}\t{r[4]}")
            else:
                out.append(f"{w}\t{e:#x}\t{name}\t{caller}\tnone\t-\t-\t-\t-\t-")
    open(a.out, "w").write("\n".join(out) + "\n")
    print(f"{len(out) - 1} lines -> {a.out}")


if __name__ == "__main__":
    main()
