#!/usr/bin/env python3
"""M8a: Direct3D call-stream diff of our client against the original (TRACE-3 logs).

  m8a_d3d_diff.py OURS_DIR ORIG_TRACE_DIR [--frame K] [--context N] [--max N] [--only NAME]

OURS_DIR holds d3d-NAME.jsonl from `M8A_D3D=1 tools/e2e/m8a.sh`; ORIG_TRACE_DIR is the
main checkout's target/trace (the original client's TRACE-3 runs). Pairs (ours → original):
input → orig-20261104-210626/d3d-input-hello, dlg0..dlg5 → orig-20261104-211839/d3d-dlgN,
tags → orig-20261104-213110/d3d-tags, scoreboard → orig-20261104-213110/d3d-score.

Normalisation on top of tools/trace/d3d_diff.py `norm` (pointers, callers, sprite labels in
order of use, floats to 2 decimals):
- the caller module samp_client.dll is named samp.dll (our DLL takes samp's place);
- only the part from samp's first sprite call on is compared (before it GTA draws
  location-dependent render targets);
- dynamic vertex-buffer offsets and glyph-atlas texel rects (D3DX glyph-cache order)
  are dropped;
- DT_CALCRECT records keep text, font and format only: the original passes an
  uninitialised rect and a stale colour there;
- `--geometry-free` (always on for tags/scoreboard): vertex and rect coordinates are dropped,
  because those scenes depend on where the players stand and on names/pings.
Exit 0 when every pair matches.
"""
import argparse
import difflib
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

PAIRS = [
    ("input", "orig-20261104-210626/d3d-input-hello.jsonl", False),
    ("dlg0", "orig-20261104-211839/d3d-dlg0.jsonl", False),
    ("dlg1", "orig-20261104-211839/d3d-dlg1.jsonl", False),
    ("dlg2", "orig-20261104-211839/d3d-dlg2.jsonl", False),
    ("dlg3", "orig-20261104-211839/d3d-dlg3.jsonl", False),
    ("dlg4", "orig-20261104-211839/d3d-dlg4.jsonl", False),
    ("dlg5", "orig-20261104-211839/d3d-dlg5.jsonl", False),
    ("tags", "orig-20261104-213110/d3d-tags.jsonl", True),
    ("scoreboard", "orig-20261104-213110/d3d-score.jsonl", True),
    ("dw", "orig-20261104-213110/d3d-dw.jsonl", True),
]


# Same-scenario pairs from tools/e2e/m8a-orig.sh (ORIG_DIR = target/m8a-orig): same names.
SAME = [("select", False), ("spritesel", False), ("dw", True), ("tags", True), ("scoreboard", True), ("menu", True), ("textdraws", True)]


def load_tool(root):
    p = os.path.join(root, "tools", "trace", "d3d_diff.py")
    spec = importlib.util.spec_from_file_location("d3d_diff", p)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def normalise(tool, frame, geometry_free):
    sprites = {}
    out = []
    # samp's pass starts with the first sprite call after GTA's scene; what GTA draws
    # before (location-dependent render targets) is not compared.
    start = next((i for i, r in enumerate(frame) if r.get("fn", "").startswith("sprite.")), 0)
    for r in frame[start:]:
        n = tool.norm(r, sprites)
        if n.get("by") == "samp_client.dll":
            n["by"] = "samp.dll"
        a = n.get("args") or {}
        # Dynamic vertex-buffer offsets.
        for k in ("min_index", "start_index", "base"):
            a.pop(k, None)
        # Glyph atlas rects depend on D3DX's glyph-cache fill order (first use).
        if n["fn"] == "sprite.Draw" and "file:" not in str(a.get("tex")) and "256x256" in str(a.get("tex")):
            a.pop("src", None)
        if n["fn"] == "dev.DrawIndexedPrimitive" and "file:" not in str(n.get("tex0")) and "256x256" in str(n.get("tex0")):
            # Glyph batches: positions and colours only (texels come from the atlas).
            vv = n.get("vertices") or {}
            # (The original's logs keep only a hash for some batches: compare the count.)
            n["vertices"] = {"n": vv.get("n")}
        fmt = a.get("fmt")
        if isinstance(fmt, str) and int(fmt, 16) & 0x400:
            a.pop("rect", None)
            a.pop("color", None)
            n.pop("rect", None)
        if geometry_free:
            for k in ("rect", "pos", "vp"):
                a.pop(k, None)
            n.pop("vertices", None)
            n.pop("rect", None)
            if "text" in a:
                a["text"] = re.sub(r"[0-9]+", "#", a["text"].replace("TraceOrig", "RustClient"))
            a.pop("count", None)
            # Glyphs differ with the names (TraceOrig/RustClient): structure only.
            if n["fn"] == "sprite.Draw" and "file:" not in str(a.get("tex")):
                continue
            if n["fn"] == "dev.DrawIndexedPrimitive" and "file:" not in str(n.get("tex0")):
                n.pop("vertices", None)
                for k in ("num_vertices", "prims"):
                    a.pop(k, None)
            # Stage-0 texture inherited from GTA's last draw (scene-dependent).
            if n.get("fvf") == "0x44":
                n.pop("tex0", None)
        if n["fn"].endswith(".ret"):
            n.pop("h", None)
        out.append(json.dumps(n, sort_keys=True))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("orig")
    ap.add_argument("--frame", type=int, default=1)
    ap.add_argument("--context", type=int, default=1)
    ap.add_argument("--max", type=int, default=40)
    ap.add_argument("--only")
    ap.add_argument("--same", action="store_true", help="ORIG is target/m8a-orig (same scenario, same names)")
    ap.add_argument("--root", default=os.environ.get("SAMP_RS_ROOT", os.path.dirname(os.path.dirname(HERE))))
    args = ap.parse_args()
    tool = load_tool(args.root)
    bad = 0
    pairs = [(n, "d3d-%s.jsonl" % n, g) for n, g in SAME] if args.same else PAIRS
    for name, orig, geo in pairs:
        if args.only and args.only != name:
            continue
        po = os.path.join(args.ours, "d3d-%s.jsonl" % name)
        pg = os.path.join(args.orig, orig)
        if not (os.path.exists(po) and os.path.exists(pg)):
            print("== %-10s SKIP (missing %s)" % (name, po if not os.path.exists(po) else pg))
            continue
        fo = tool.frames(tool.load(po))
        fg = tool.frames(tool.load(pg))
        k = min(args.frame, len(fo) - 1, len(fg) - 1)
        if k < 0:
            print("== %-10s SKIP (no logged frames)" % name)
            continue
        lo = normalise(tool, fo[k], geo)
        lg = normalise(tool, fg[k], geo)
        diff = list(difflib.unified_diff(lg, lo, "original", "ours", n=args.context, lineterm=""))
        changes = sum(1 for d in diff if d[:1] in "+-" and not d.startswith(("+++", "---")))
        print("== %-10s frame %d: original %d calls, ours %d calls, %d differing lines%s" % (
            name, k, len(lg), len(lo), changes, " (geometry-free)" if geo else ""))
        for d in diff[: args.max]:
            print("   " + d[:260])
        if changes:
            bad += 1
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
