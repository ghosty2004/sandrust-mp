#!/usr/bin/env python3
"""FEAT-SWEEP: which samp.dll functions do our specs (docs/) and our sources (crates/, tools/)
cite? Addresses are build A unless written as `B 0x…` (docs convention); a bare address that
is a build-B entry but not a build-A one counts as B.

samp_docrefs.py A2B.json OUT.tsv
A2B.json maps build-A function entries to build-B entries (normalised-pseudocode alignment).
Output: va_B  docs_files  crates_files
"""
import bisect
import collections
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.dirname(__file__))
from samp_graph import Graph  # noqa: E402


def main():
    a2b = {int(k, 16): int(v, 16) for k, v in json.load(open(sys.argv[1])).items()}
    out = sys.argv[2]
    g = Graph()
    bs = sorted(g.funcs)
    as_ = sorted(a2b)

    def cont(lst, a, sizes=None):
        i = bisect.bisect_right(lst, a) - 1
        if i < 0:
            return None
        f = lst[i]
        return f if a - f < 0x3000 else None

    pat = re.compile(r"(?:\b([AB])\s*(?:=\s*)?)?\b(0x1[0-9a-fA-F]{7})\b")
    refs = {"docs": collections.defaultdict(set), "src": collections.defaultdict(set)}
    for kind, top in (("docs", "docs"), ("src", "crates"), ("src", "tools")):
        for d, _, files in os.walk(os.path.join(ROOT, top)):
            if "target" in d.split(os.sep) or "__pycache__" in d:
                continue
            for fn in files:
                if not fn.endswith((".md", ".rs", ".csv", ".tsv", ".py", ".sh", ".pwn")):
                    continue
                p = os.path.join(d, fn)
                rel = os.path.relpath(p, ROOT)
                try:
                    text = open(p, encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                for m in pat.finditer(text):
                    tag, a = m.group(1), int(m.group(2), 16)
                    if not (0x10001000 <= a < 0x100e5000):
                        continue
                    b = None
                    if tag == "B" or (tag is None and a in g.funcs and a not in a2b):
                        b = cont(bs, a)
                    else:
                        fa = cont(as_, a)
                        if fa is not None:
                            b = a2b[fa]
                            # keep the same offset inside the function, then re-resolve
                            b = cont(bs, b + (a - fa))
                    if b is not None:
                        refs[kind][b].add(rel)
    with open(out, "w") as w:
        w.write("va_B\tdocs\tsrc\n")
        for b in bs:
            w.write(f"{b:#x}\t{','.join(sorted(refs['docs'].get(b, ())))}\t{','.join(sorted(refs['src'].get(b, ())))}\n")
    print(f"functions cited: docs {len(refs['docs'])}, sources {len(refs['src'])}, "
          f"either {len(set(refs['docs']) | set(refs['src']))}")


if __name__ == "__main__":
    main()
