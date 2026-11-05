#!/usr/bin/env python3
"""F6 data-parity report: which files gta_sa.exe opens under each client, where each is served
from (samp.saa archive handle vs disk) and a SHA-1 of the bytes the game actually read.

Input: two trace output dirs of `tools/trace/run.sh orig|ours spawn` run with
SAMP_TRACER_FILEIO=1 (samp-tracer `fileio_*` records, crates/samp-tracer/src/win/fileio.rs).
Only digests are compared and printed; no file content is read or written here.

Checks (each OK or DIFF):
  armed         both tracers wrapped all 5 import slots
  opens-set     the multiset of (normalised path, class) of every CreateFileA is equal
  opens-order   the ordered list of opens is equal (difflib on (path, class))
  archive-order the ordered list of archive-served opens is equal
  sizes         per open of the same path (k-th open each), the size the client reported
                (GetFileSize) and the extent read are equal
  hashes        per archive open (k-th open of a path), SHA-1 of the reassembled bytes is equal;
                read-only disk files that were closed: equal too
  ours-log      our tracer's archive opens equal the `files: CreateFileA(...) -> archive handle`
                lines of samp-rs.log, and each archive size equals the entry size the client listed
A DIFF line may be accepted by a row of tools/diff/files-decisions.tsv whose decision id appears in
docs/ROADMAP.md "Decisions made autonomously". Exit 0 = no unaccepted difference.

Usage: files_report.py ORIG_DIR OURS_DIR [--decisions TSV] [--roadmap MD] [--summary OUT.txt]
"""
import argparse
import collections
import difflib
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def norm(p):
    p = p.strip().replace("/", "\\").lower()
    p = re.sub(r"\\+", r"\\", p)
    m = re.search(r"\\gta san andreas user files\\(.*)$", p)
    if m:
        return "<userfiles>\\" + m.group(1)
    p = re.sub(r"^[a-z]:\\.*?\\gta-sa(-[a-z0-9]+)?\\", "", p)
    while p.startswith(".\\"):
        p = p[2:]
    return p


def load(d):
    recs = []
    path = os.path.join(d, "trace.jsonl")
    with open(path, "rb") as f:
        for raw in f:
            if b'"fn":"fileio_' not in raw:
                continue
            try:
                recs.append(json.loads(raw))
            except ValueError:
                pass
    armed = [r["args"] for r in recs if r["fn"] == "fileio_armed"]
    opens = [r["args"] for r in recs if r["fn"] == "fileio_create"]
    summ = {r["args"]["n"]: dict(r["args"], fn=r["fn"]) for r in recs if r["fn"] in ("fileio_close", "fileio_end")}
    done = any(r["fn"] == "fileio_done" for r in recs)
    for o in opens:
        o["np"] = norm(o["path"])
        o["sum"] = summ.get(o["n"])
    return {"armed": armed, "opens": opens, "done": done}


def kth(opens):
    """(path, k) -> open, k = occurrence index of that path."""
    seen = collections.Counter()
    out = {}
    for o in opens:
        out[(o["np"], seen[o["np"]])] = o
        seen[o["np"]] += 1
    return out


def load_decisions(tsv, roadmap):
    ids = set()
    try:
        text = open(roadmap, encoding="utf-8").read()
        sec = text.split("## Decisions made autonomously", 1)[1] if "## Decisions made autonomously" in text else ""
        sec = sec.split("\n## ", 1)[0]
        ids = set(re.findall(r"\b([A-Z][A-Z0-9]*-[A-Z]*\d+)\b", sec))
    except OSError:
        pass
    rows = []
    if os.path.exists(tsv):
        for line in open(tsv, encoding="utf-8"):
            if not line.strip() or line.startswith("#"):
                continue
            cols = line.rstrip("\n").split("\t")
            if len(cols) < 3:
                continue
            check, rx, did = cols[0], cols[1], cols[2]
            if did not in ids:
                print(f"[files] decision row ignored ({did} not in ROADMAP decisions): {line.strip()}")
                continue
            rows.append((check, re.compile(rx), did))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("orig")
    ap.add_argument("ours")
    ap.add_argument("--decisions", default=os.path.join(ROOT, "tools/diff/files-decisions.tsv"))
    ap.add_argument("--roadmap", default=os.path.join(ROOT, "docs/ROADMAP.md"))
    ap.add_argument("--summary")
    a = ap.parse_args()
    O, U = load(a.orig), load(a.ours)
    decisions = load_decisions(a.decisions, a.roadmap)
    lines = []
    diffs = collections.defaultdict(list)  # check -> [detail]

    def say(s):
        lines.append(s)
        print(s)

    def diff(check, detail):
        diffs[check].append(detail)

    # armed
    for name, X in (("orig", O), ("ours", U)):
        if not X["armed"] or X["armed"][0].get("slots") != 5:
            diff("armed", f"{name}: {X['armed'] or 'no fileio_armed record (SAMP_TRACER_FILEIO=1 missing?)'}")
        elif X["armed"][0].get("from") != "WinMain":
            diff("armed", f"{name}: armed from {X['armed'][0].get('from')} (not WinMain)")
        if not X["done"]:
            diff("armed", f"{name}: no fileio_done record (still-open handles not summarised)")

    def key(o):
        return (o["np"], o["class"])

    # opens-set
    co, cu = collections.Counter(map(key, O["opens"])), collections.Counter(map(key, U["opens"]))
    for k in sorted(set(co) | set(cu)):
        if co[k] != cu[k]:
            diff("opens-set", f"{k[0]} [{k[1]}]: orig x{co[k]}, ours x{cu[k]}")

    def seqdiff(check, a_, b_):
        sm = difflib.SequenceMatcher(a=a_, b=b_, autojunk=False)
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag == "equal":
                continue
            left = "; ".join(f"#{i} {p} [{c}]" for i, (p, c) in enumerate(a_[i1:i2], i1 + 1))
            right = "; ".join(f"#{j} {p} [{c}]" for j, (p, c) in enumerate(b_[j1:j2], j1 + 1))
            diff(check, f"{tag}: orig {{{left or '-'}}} vs ours {{{right or '-'}}}")

    seqdiff("opens-order", [key(o) for o in O["opens"]], [key(o) for o in U["opens"]])
    arch = lambda X: [key(o) for o in X["opens"] if o["class"] == "archive"]  # noqa: E731
    seqdiff("archive-order", arch(O), arch(U))

    # sizes + hashes, per k-th open of a path
    ko, ku = kth(O["opens"]), kth(U["opens"])
    n_hash = n_hash_disk = 0
    for k in sorted(set(ko) & set(ku)):
        o, u = ko[k], ku[k]
        so, su = o["sum"], u["sum"]
        tag = f"{k[0]} (open {k[1] + 1})"
        if o["class"] != u["class"]:
            continue  # reported by opens-set
        if so is None or su is None:
            if (so is None) != (su is None):
                diff("sizes", f"{tag}: summary orig={'yes' if so else 'no'} ours={'yes' if su else 'no'}")
            continue
        if so.get("size") != su.get("size"):
            diff("sizes", f"{tag}: GetFileSize orig={so.get('size')} ours={su.get('size')}")
        if so.get("extent") != su.get("extent") and (o["class"] == "archive" or so["fn"] == "fileio_close"):
            diff("sizes", f"{tag}: bytes read up to orig={so.get('extent')} ours={su.get('extent')}")
        if o["class"] == "archive":
            n_hash += 1
            if so.get("sha1") != su.get("sha1"):
                diff("hashes", f"{tag} [archive]: sha1 orig={so.get('sha1')} ours={su.get('sha1')}"
                     f" (reads {so.get('reads')}/{su.get('reads')}, bytes {so.get('bytes')}/{su.get('bytes')})")
        elif so["fn"] == "fileio_close" and su["fn"] == "fileio_close" and not so.get("write") and so.get("sha1"):
            n_hash_disk += 1
            if so.get("sha1") != su.get("sha1"):
                diff("hashes", f"{tag} [disk]: sha1 orig={so.get('sha1')} ours={su.get('sha1')}")

    # ours-log: our client's own account of what it served
    log = os.path.join(a.ours, "samp-rs.log")
    served, sizes = [], {}
    if os.path.exists(log):
        for line in open(log, encoding="utf-8", errors="replace"):
            m = re.search(r"files: CreateFileA\((.*)\) -> archive handle", line)
            if m:
                served.append(norm(m.group(1)))
            m = re.search(r"files:\s+slot\s+\d+ hash 0x[0-9a-f]+ size\s+(\d+) (\S+)$", line.rstrip())
            if m:
                sizes[m.group(2).lower()] = int(m.group(1))
        traced = [o["np"] for o in U["opens"] if o["class"] == "archive"]
        if served != traced:
            seqdiff("ours-log", [(p, "log") for p in served], [(p, "log") for p in traced])
        for o in U["opens"]:
            if o["class"] != "archive" or not o["sum"]:
                continue
            base = o["np"].rsplit("\\", 1)[-1]
            want = sizes.get(base)
            got = o["sum"].get("size")
            if got is not None and want is not None and got != want:
                diff("ours-log", f"{o['np']}: GetFileSize {got}, archive entry {want}")
    else:
        diff("ours-log", f"{log} missing")

    # summary
    no, nu = len(O["opens"]), len(U["opens"])
    na_o = sum(o["class"] == "archive" for o in O["opens"])
    na_u = sum(o["class"] == "archive" for o in U["opens"])
    dist = len({o["np"] for o in O["opens"] if o["class"] == "archive"})
    say(f"[files] orig: {no} opens ({na_o} from samp.saa, {dist} distinct), "
        f"ours: {nu} opens ({na_u} from samp.saa); {n_hash} archive + {n_hash_disk} disk content hashes compared")
    unaccepted = 0
    for check in ("armed", "opens-set", "opens-order", "archive-order", "sizes", "hashes", "ours-log"):
        ds = diffs.get(check, [])
        if not ds:
            say(f"[files] {check}: OK")
            continue
        open_ = []
        for dline in ds:
            acc = next((did for c, rx, did in decisions if c in (check, "*") and rx.search(dline)), None)
            if acc:
                say(f"[files] {check}: accepted ({acc}): {dline}")
            else:
                open_.append(dline)
        unaccepted += len(open_)
        if open_:
            say(f"[files] {check}: DIFF ({len(open_)})")
            for dline in open_[:40]:
                say(f"    {dline}")
            if len(open_) > 40:
                say(f"    … {len(open_) - 40} more")
        else:
            say(f"[files] {check}: OK (all differences accepted)")
    say(f"[files] {'IDENTICAL' if unaccepted == 0 else f'{unaccepted} unaccepted difference(s)'}")
    if a.summary:
        with open(a.summary, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    return 0 if unaccepted == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
