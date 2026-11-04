#!/usr/bin/env python3
"""TRACE-5: what happens between two marks of a trace.jsonl, minus the per-frame background.

  events.py TRACE FROM_MARK [TO_MARK] [--baseline N] [--keep REGEX] [--show REGEX]

FROM/TO are prefixes of mark texts. The baseline is the N frames before FROM (default 30):
every client script command (op + params) and hook call (fn + args) seen in the baseline is
treated as per-frame background and dropped. Per-frame record types (pos, watch, globals,
rped, rveh, vpc, pc, suppressed) are dropped unless --show matches them. --keep REGEX keeps
background lines that match.
"""
import argparse
import json
import re

NOISE = {"pos", "watch", "globals", "rped", "rveh", "vpc", "pc", "suppressed", "globals_base"}


def key(r):
    a = r.get("args", {})
    if r["fn"] == "scm":
        return "scm %s %s" % (a.get("op"), json.dumps(a.get("params"), separators=(",", ":")))
    return "%s %s" % (r["fn"], json.dumps(a, sort_keys=True, separators=(",", ":")))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("trace")
    ap.add_argument("start")
    ap.add_argument("end", nargs="?")
    ap.add_argument("--baseline", type=int, default=30)
    ap.add_argument("--keep")
    ap.add_argument("--show")
    a = ap.parse_args()
    recs = [json.loads(l) for l in open(a.trace) if l.strip()]
    i0 = next(i for i, r in enumerate(recs) if r["fn"] == "mark" and r["args"]["text"].startswith(a.start))
    f0 = recs[i0]["frame"]
    i1 = len(recs)
    if a.end:
        i1 = next((i for i, r in enumerate(recs) if i > i0 and r["fn"] == "mark" and r["args"]["text"].startswith(a.end)), len(recs))
    base = set()
    for r in recs[:i0]:
        if r.get("frame", 0) >= f0 - a.baseline and r["fn"] not in NOISE:
            base.add(re.sub(r"-?\d+\.\d+", "#", key(r)))
    for r in recs[i0:i1 + 1]:
        fn = r["fn"]
        if fn in NOISE and not (a.show and re.search(a.show, fn)):
            continue
        k = key(r)
        if fn != "mark" and re.sub(r"-?\d+\.\d+", "#", k) in base and not (a.keep and re.search(a.keep, k)):
            continue
        extra = ""
        if r.get("origin"):
            extra = "  <" + r["origin"].split(" < ")[0]
        elif r.get("caller") and not r["caller"].startswith("samp.dll+0xb2"):
            extra = "  @" + r["caller"]
        print("%6s %s%s" % (r.get("frame"), k[:260], extra))


if __name__ == "__main__":
    main()
