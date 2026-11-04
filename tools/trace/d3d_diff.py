#!/usr/bin/env python3
"""TRACE-3 / DIFF-4: Direct3D9 call-stream logs (d3d.jsonl from samp-tracer) — dump and diff.

  d3d_diff.py dump FILE [--frame K] [--raw] [--only REGEX]
      readable listing (render/texture-stage/sampler states named); --frame K = K-th logged
      frame (0-based) of the file, default all; --raw keeps pointers and callers.
  d3d_diff.py diff LEFT RIGHT [--frame-left K] [--frame-right K] [--only REGEX] [--context N]
      aligned diff of the normalised streams of one logged frame each (default: the first).
  d3d_diff.py texts FILE      every DrawText (font, text, rect, flags, colour) with its frame
  d3d_diff.py quads FILE [--frame K]  decoded draws as screen rectangles: x0,y0-x1,y1, texel rect
      (texture pixels when the texture size is known), colour; one line per quad

Normalisation (diff): drops sequence numbers, times, frame/present counters, pointers
(`*_ptr`), caller offsets and client origins (keeps the caller module), numbers sprite labels
in order of first use, rounds floats to 2 decimals.
"""
import argparse
import difflib
import json
import re
import sys

RS = {7: "ZENABLE", 8: "FILLMODE", 9: "SHADEMODE", 14: "ZWRITEENABLE", 15: "ALPHATESTENABLE",
      16: "LASTPIXEL", 19: "SRCBLEND", 20: "DESTBLEND", 22: "CULLMODE", 23: "ZFUNC", 24: "ALPHAREF",
      25: "ALPHAFUNC", 26: "DITHERENABLE", 27: "ALPHABLENDENABLE", 28: "FOGENABLE",
      29: "SPECULARENABLE", 34: "FOGCOLOR", 35: "FOGTABLEMODE", 36: "FOGSTART", 37: "FOGEND",
      48: "RANGEFOGENABLE", 52: "STENCILENABLE", 60: "TEXTUREFACTOR", 128: "WRAP0", 136: "LIGHTING",
      137: "LIGHTING?", 139: "AMBIENT", 140: "FOGVERTEXMODE", 141: "COLORVERTEX",
      143: "NORMALIZENORMALS", 145: "DIFFUSEMATERIALSOURCE", 146: "SPECULARMATERIALSOURCE",
      147: "AMBIENTMATERIALSOURCE", 148: "EMISSIVEMATERIALSOURCE", 151: "VERTEXBLEND",
      152: "CLIPPLANEENABLE", 154: "POINTSIZE", 161: "MULTISAMPLEANTIALIAS", 167: "INDEXEDVERTEXBLENDENABLE",
      168: "COLORWRITEENABLE", 171: "BLENDOP", 174: "SCISSORTESTENABLE", 206: "SEPARATEALPHABLENDENABLE",
      207: "SRCBLENDALPHA", 208: "DESTBLENDALPHA", 209: "BLENDOPALPHA", 194: "SRGBWRITEENABLE"}
TSS = {1: "COLOROP", 2: "COLORARG1", 3: "COLORARG2", 4: "ALPHAOP", 5: "ALPHAARG1", 6: "ALPHAARG2",
       11: "TEXCOORDINDEX", 24: "TEXTURETRANSFORMFLAGS", 26: "COLORARG0", 27: "ALPHAARG0", 28: "RESULTARG"}
SS = {1: "ADDRESSU", 2: "ADDRESSV", 3: "ADDRESSW", 4: "BORDERCOLOR", 5: "MAGFILTER", 6: "MINFILTER",
      7: "MIPFILTER", 8: "MIPMAPLODBIAS", 9: "MAXMIPLEVEL", 10: "MAXANISOTROPY"}
TS = {2: "VIEW", 3: "PROJECTION", 16: "TEXTURE0", 256: "WORLD"}


def load(path):
    out = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    return out


def frames(recs):
    """Splits records into logged frames (each ends with dev.Present)."""
    res, cur, on = [], [], False
    for r in recs:
        fn = r.get("fn", "")
        if fn == "d3d_logging":
            on = r.get("mode") != "off"
            continue
        if not on:
            continue
        cur.append(r)
        if fn == "dev.Present":
            res.append(cur)
            cur = []
    if cur:
        res.append(cur)
    return res


def rnd(v):
    if isinstance(v, float):
        return round(v, 2)
    if isinstance(v, list):
        return [rnd(x) for x in v]
    if isinstance(v, dict):
        return {k: rnd(x) for k, x in v.items()}
    return v


def named_args(fn, a):
    a = dict(a)
    if fn == "dev.SetRenderState" and "rs" in a:
        a["rs"] = RS.get(a["rs"], a["rs"])
    elif fn == "dev.SetTextureStageState" and "tss" in a:
        a["tss"] = TSS.get(a["tss"], a["tss"])
    elif fn == "dev.SetSamplerState" and "ss" in a:
        a["ss"] = SS.get(a["ss"], a["ss"])
    elif fn == "dev.SetTransform" and "state" in a:
        a["state"] = TS.get(a["state"], a["state"])
    return a


def norm(r, sprites):
    fn = r.get("fn", "")
    out = {"fn": fn, "sc": r.get("sc")}
    by = r.get("by")
    if by:
        out["by"] = by.split("+")[0]
    for k, v in r.items():
        if k in ("n", "t", "f", "pr", "by", "org", "fn", "sc", "obj_ptr", "sprite_ptr") or k.endswith("_ptr"):
            continue
        if k == "args":
            v = {kk: vv for kk, vv in v.items() if not kk.endswith("_ptr")}
            v = named_args(fn, v)
        if k == "sprite" and isinstance(v, str):
            v = sprites.setdefault(v, "sprite%d" % (len(sprites) + 1))
        out[k] = rnd(v)
    return out


def fmt(r, raw=False):
    fn = r.get("fn", "")
    a = named_args(fn, r.get("args", {}))
    if not raw:
        a = {k: v for k, v in a.items() if not k.endswith("_ptr")}
    parts = [fn]
    if a:
        parts.append(" ".join("%s=%s" % (k, json.dumps(rnd(v), separators=(",", ":"))) for k, v in a.items()))
    for k in ("font", "sprite", "fvf", "tex0", "h", "rect", "hud", "result", "text", "info", "mode"):
        if k in r and r[k] is not None:
            parts.append("%s=%s" % (k, json.dumps(rnd(r[k]), separators=(",", ":"))))
    if "vertices" in r:
        vv = r["vertices"]
        if "v" in vv:
            parts.append("v[%d]: %s" % (vv["n"], " | ".join(vv["v"])))
        else:
            parts.append("v[%s] hash=%s %s" % (vv.get("n"), vv.get("hash", ""), vv.get("note", "")))
    if "indices" in r:
        parts.append("idx=%s" % r["indices"][:24])
    line = "  ".join(parts)
    if raw:
        line = "%s %s %s  [%s%s]" % (r.get("n"), r.get("sc", ""), line, r.get("by", ""),
                                    (" < " + r["org"]) if r.get("org") else "")
    else:
        line = "%-5s %s  (%s)" % (r.get("sc", ""), line, (r.get("by") or "").split("+")[0])
    return line


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("dump")
    d.add_argument("file")
    d.add_argument("--frame", type=int)
    d.add_argument("--raw", action="store_true")
    d.add_argument("--only")
    d.add_argument("--all-records", action="store_true", help="also records outside logged frames")
    x = sub.add_parser("diff")
    x.add_argument("left")
    x.add_argument("right")
    x.add_argument("--frame-left", type=int, default=0)
    x.add_argument("--frame-right", type=int, default=0)
    x.add_argument("--only")
    x.add_argument("--context", type=int, default=3)
    t = sub.add_parser("texts")
    t.add_argument("file")
    q = sub.add_parser("quads")
    q.add_argument("file")
    q.add_argument("--frame", type=int, default=0)
    args = ap.parse_args()

    if args.cmd == "dump":
        recs = load(args.file)
        if args.all_records:
            groups = [recs]
        else:
            groups = frames(recs)
            if args.frame is not None:
                groups = [groups[args.frame]]
        for i, g in enumerate(groups):
            print("=== frame %d (%d calls)" % (i if args.frame is None else args.frame, len(g)))
            for r in g:
                line = fmt(r, args.raw)
                if args.only and not re.search(args.only, line):
                    continue
                print(line)
    elif args.cmd == "quads":
        for r in frames(load(args.file))[args.frame]:
            vv = r.get("vertices") or {}
            if "v" not in vv:
                continue
            m = re.search(r"tex (\d+)x(\d+)", r.get("tex0") or "")
            tw, th = (int(m.group(1)), int(m.group(2))) if m else (1, 1)
            label = (r.get("tex0") or "").split(" ")[-1] if r.get("tex0") else "-"
            pts = []
            for v in vv["v"]:
                parts = v.split()
                xyz = [float(x) for x in parts[0].split(",")]
                col = next((p for p in parts if p.startswith("#")), "")
                uv = [float(x) for x in parts[-1].split(",")] if len(parts) > 2 or not col else [0, 0]
                pts.append((xyz, col, uv))
            step = 4
            print("%s %s prims=%s tex=%s" % (r["fn"], r.get("args", {}).get("type"), r.get("args", {}).get("prims"), label))
            for i in range(0, len(pts) - step + 1, step):
                q4 = pts[i:i + step]
                xs = [p[0][0] for p in q4]; ys = [p[0][1] for p in q4]
                us = [p[2][0] * tw for p in q4]; vs = [p[2][1] * th for p in q4]
                cols = sorted(set(p[1] for p in q4))
                print("   %g,%g-%g,%g  uv %g,%g-%g,%g  %s" % (min(xs), min(ys), max(xs), max(ys), round(min(us), 1),
                                                         round(min(vs), 1), round(max(us), 1), round(max(vs), 1), "/".join(cols)))
    elif args.cmd == "texts":
        for r in load(args.file):
            if r.get("fn", "").startswith("font.DrawText") and not r["fn"].endswith(".ret"):
                a = r.get("args", {})
                print("pr=%s f=%s %s rect=%s fmt=%s color=%s  %r" % (r.get("pr"), r.get("f"), r.get("font"), a.get("rect"),
                                                                 a.get("fmt"), a.get("color"), a.get("text")))
    else:
        sides = []
        for path, k in ((args.left, args.frame_left), (args.right, args.frame_right)):
            g = frames(load(path))
            if k >= len(g):
                sys.exit("%s: only %d logged frames" % (path, len(g)))
            sprites = {}
            lines = [json.dumps(norm(r, sprites), sort_keys=True) for r in g[k]]
            if args.only:
                lines = [l for l in lines if re.search(args.only, l)]
            sides.append(lines)
        diff = list(difflib.unified_diff(sides[0], sides[1], args.left, args.right, n=args.context, lineterm=""))
        for l in diff:
            print(l)
        sys.exit(1 if diff else 0)


if __name__ == "__main__":
    main()
