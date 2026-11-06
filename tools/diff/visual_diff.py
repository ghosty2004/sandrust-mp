#!/usr/bin/env python3
"""DIFF-4: per-element Direct3D call-stream diff of our client against the original.

  visual_diff.py OURS_DIR ORIG_DIR [--decisions TSV] [--only a,b] [--max N] [--frame K]

Both directories come from tools/diff/visual.sh (same scenario, same player name): d3d-NAME.jsonl
per element plus d3d-full.jsonl (the boot log, UX-1). Normalisation (on top of
tools/trace/d3d_diff.py `norm`: pointers, sequence numbers, times, caller offsets dropped, sprite
labels numbered in order of use, floats to 2 decimals):
- caller module samp_client.dll is named samp.dll (our DLL takes samp's place);
- `ui` elements: from samp's first sprite call on (GTA's scene before it is not samp's);
  dynamic vertex-buffer offsets and D3DX glyph-atlas texel rects dropped; DT_CALCRECT keeps
  text/font/format (the original passes an uninitialised rect and a stale colour);
- `all2d` elements (textdraws, sprites, menus: drawn by GTA inside its scene): only the
  pre-transformed (XYZRHW) draws, with texture, vertices and FVF, plus D3DX sprite/font calls;
- `geo` elements depend on where players stand / pings: coordinates and digits dropped;
- `boot` (UX-1): tools/diff/boot_diff.py on d3d-full.jsonl.
Lines matching a row of the decisions TSV (element, regex, ROADMAP decision) are reported as
covered. Exit 0 when every element has no uncovered difference and every element exists on
both sides.
"""
import argparse
import difflib
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))

# element -> (kind, options)
ELEMENTS = {
    "boot": ("boot", {}),
    "class": ("ui", {}),
    "dlg0": ("ui", {}), "dlg1": ("ui", {}), "dlg2": ("ui", {}), "dlg3": ("ui", {}),
    "dlg4": ("ui", {}), "dlg5": ("ui", {}), "help": ("ui", {}),
    "sprite": ("all2d", {}), "spritesel": ("ui", {}), "textdraws": ("all2d", {}),
    "select": ("ui", {}), "mdl": ("all2d", {}), "menu": ("all2d", {}),
    "dw": ("ui", {}), "f10": ("ui", {}), "netstats": ("ui", {"geo": True}),
    "f8": ("ui", {}), "after-f8": ("ui", {}), "paste": ("ui", {}),
    "tags": ("ui", {"geo": True}), "scoreboard": ("ui", {"geo": True}),
    "vehtag": ("ui", {"geo": True}), "input": ("ui", {}),
    # FEAT-B (end of tools/diff/visual-scenario.sh): /dl labels, /dl off, /testdw rows.
    "dl": ("ui", {"geo": True}), "dloff": ("ui", {}), "testdw": ("ui", {}),
}


def load_mod(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def common(tool, r, sprites):
    n = tool.norm(r, sprites)
    if n.get("by") == "samp_client.dll":
        n["by"] = "samp.dll"
    a = n.get("args") or {}
    for k in ("min_index", "start_index", "base"):
        a.pop(k, None)
    if n["fn"] == "sprite.Draw" and "file:" not in str(a.get("tex")) and "256x256" in str(a.get("tex")):
        a.pop("src", None)
    if n["fn"] == "dev.DrawIndexedPrimitive" and "file:" not in str(n.get("tex0")) and "256x256" in str(n.get("tex0")):
        n["vertices"] = {"n": (n.get("vertices") or {}).get("n")}
    fmt = a.get("fmt")
    if isinstance(fmt, str) and int(fmt, 16) & 0x400:
        a.pop("rect", None)
        a.pop("color", None)
        n.pop("rect", None)
    if n["fn"].endswith(".ret"):
        n.pop("h", None)
    return n


def calc_ret(n, calc):
    """DT_CALCRECT results keep only the size: the original passes uninitialised / stale input
    rects (the result is offset by them). Returns whether the next .ret belongs to a CALCRECT."""
    fn = n.get("fn", "")
    if fn.startswith("font.DrawText") and not fn.endswith(".ret"):
        fmt = (n.get("args") or {}).get("fmt")
        return isinstance(fmt, str) and bool(int(fmt, 16) & 0x400)
    if fn.endswith(".ret") and calc and isinstance(n.get("rect"), list) and len(n["rect"]) == 4:
        r = n["rect"]
        n["rect"] = ["w", r[2] - r[0], "h", r[3] - r[1]]
    return False


def degeo(n):
    a = n.get("args") or {}
    for k in ("rect", "pos", "vp", "count"):
        a.pop(k, None)
    n.pop("vertices", None)
    n.pop("rect", None)
    if "text" in a:
        a["text"] = re.sub(r"[0-9]+", "#", a["text"])
    if n["fn"] == "dev.DrawIndexedPrimitive":
        for k in ("num_vertices", "prims"):
            a.pop(k, None)
    if n.get("fvf") == "0x44":
        n.pop("tex0", None)
    return n


def normalise(tool, kind, opts, frame):
    sprites = {}
    out = []
    if kind == "ui":
        start = next((i for i, r in enumerate(frame) if r.get("fn", "").startswith("sprite.")), 0)
        frame = frame[start:]
    calc = False
    for r in frame:
        fn = r.get("fn", "")
        if kind == "all2d":
            is2d = fn.startswith(("sprite.", "font.")) or (
                fn.startswith("dev.Draw") and int(str(r.get("fvf") or "0"), 16) & 0x4)
            if not is2d:
                continue
        n = common(tool, r, sprites)
        calc = calc_ret(n, calc)
        if opts.get("geo"):
            if n["fn"] == "sprite.Draw" and "file:" not in str((n.get("args") or {}).get("tex")):
                continue
            n = degeo(n)
        if atlas_draw(n) and out and atlas_draw(json.loads(out[-1])):
            # D3DX flushes a glyph batch when its 256x256 atlas texture fills up mid-string, so
            # the same text can arrive as one draw or as two consecutive draws of the same atlas
            # with the same state (depends on what was drawn before). Merge consecutive atlas
            # draws: identical totals mean identical geometry on screen.
            prev = json.loads(out[-1])
            if same_but_counts(prev, n):
                for k in ("num_vertices", "prims"):
                    prev["args"][k] = (prev["args"].get(k) or 0) + (n["args"].get(k) or 0)
                if "vertices" in prev and "vertices" in n:
                    prev["vertices"]["n"] = (prev["vertices"].get("n") or 0) + (n["vertices"].get("n") or 0)
                out[-1] = json.dumps(prev, sort_keys=True)
                continue
        out.append(json.dumps(n, sort_keys=True))
    return out


def atlas_draw(n):
    """A DrawIndexedPrimitive of D3DX's dynamic 256x256 glyph atlas (not a file texture)."""
    return (n.get("fn") == "dev.DrawIndexedPrimitive" and "256x256" in str(n.get("tex0"))
            and "file:" not in str(n.get("tex0")))


def same_but_counts(a, b):
    """Two atlas draws that differ only in their vertex / primitive counts."""
    def strip(n):
        n = json.loads(json.dumps(n))
        for k in ("num_vertices", "prims"):
            (n.get("args") or {}).pop(k, None)
        n.pop("vertices", None)
        return n
    return strip(a) == strip(b)


def load_decisions(path):
    rows = []
    if path and os.path.exists(path):
        for line in open(path):
            if line.startswith("#") or not line.strip():
                continue
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 3:
                rows.append((parts[0], re.compile(parts[1]), parts[2]))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("orig")
    ap.add_argument("--decisions")
    ap.add_argument("--only")
    ap.add_argument("--max", type=int, default=30)
    ap.add_argument("--frame", type=int, default=1)
    args = ap.parse_args()
    tool = load_mod("d3d_diff", os.path.join(ROOT, "tools", "trace", "d3d_diff.py"))
    decisions = load_decisions(args.decisions)
    only = set(args.only.split(",")) if args.only else None
    bad = 0
    rows = []
    for name, (kind, opts) in ELEMENTS.items():
        if only and name not in only:
            continue
        if kind == "boot":
            boot = load_mod("boot_diff", os.path.join(HERE, "boot_diff.py"))
            changes, lines = boot.compare(os.path.join(args.ours, "d3d-full.jsonl"), os.path.join(args.orig, "d3d-full.jsonl"))
        else:
            po = os.path.join(args.ours, "d3d-%s.jsonl" % name)
            pg = os.path.join(args.orig, "d3d-%s.jsonl" % name)
            if not (os.path.exists(po) and os.path.exists(pg)):
                rows.append((name, "MISSING", 0, 0))
                print("== %-10s MISSING (%s)" % (name, po if not os.path.exists(po) else pg))
                bad += 1
                continue
            fo, fg = tool.frames(tool.load(po)), tool.frames(tool.load(pg))
            k = min(args.frame, len(fo) - 1, len(fg) - 1)
            if k < 0:
                rows.append((name, "NO FRAMES", 0, 0))
                print("== %-10s no logged frames" % name)
                bad += 1
                continue
            lo, lg = normalise(tool, kind, opts, fo[k]), normalise(tool, kind, opts, fg[k])
            diff = list(difflib.unified_diff(lg, lo, "original", "ours", n=1, lineterm=""))
            lines = [d for d in diff if d[:1] in "+-" and not d.startswith(("+++", "---"))]
            changes = len(lines)
            lines = diff
        changed = [d for d in lines if d[:1] in "+-" and not d.startswith(("+++", "---"))]
        covered = [d for d in changed if any(e == name and rx.search(d) for e, rx, _ in decisions)]
        open_ = len(changed) - len(covered)
        status = "OK" if open_ == 0 else "DIFF"
        rows.append((name, status, open_, len(covered)))
        print("== %-10s %s: %d differing lines (%d covered by decisions)" % (name, status, open_, len(covered)))
        shown = 0
        for d in lines:
            if d in covered:
                continue
            print("   " + d[:260])
            shown += 1
            if shown >= args.max:
                print("   …")
                break
        if open_:
            bad += 1
    print("\n| element | result | open lines | covered |")
    print("|---|---|---|---|")
    for r in rows:
        print("| %s | %s | %d | %d |" % r)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
