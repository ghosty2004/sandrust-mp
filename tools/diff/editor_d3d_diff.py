#!/usr/bin/env python3
"""M6-P: D3D call-stream diff of samp's object editor / object selection pass (Present step 12).

  editor_d3d_diff.py LEFT.jsonl RIGHT.jsonl [--frame-left K] [--frame-right K] [--dump]
  editor_d3d_diff.py LEFT.jsonl RIGHT.jsonl --plate     (the number-plate painter block)

Takes one logged frame of each d3d.jsonl (`ui.sh d3d ui N NAME`; default the second frame, the first one may start
mid-frame), cuts it from the editor / selection pass start (the first `dev.ShowCursor` after the chat GUI, else the
glyph probe or the first line draw) to `dev.Present`, normalises it with tools/trace/d3d_diff.py's rules plus:
  * caller modules samp.dll / samp_client.dll → "client";
  * D3DX's dynamic vertex/index buffer positions (DrawIndexedPrimitive min_index / start_index) dropped;
  * the glyph quads D3DX draws for a DrawText (atlas position depends on what the font cached before);
  * vertex dumps of DrawPrimitive (the tracer's VB shadow of ID3DXLine's internal buffer is stale on both sides);
  * the glyph probe's rect and its result position (samp passes an uninitialised RECT; only the size is used:
    decided, docs/ROADMAP.md);
and prints a unified diff (exit 1) or "MATCH" (exit 0).
"""
import argparse
import difflib
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "trace"))
import d3d_diff  # noqa: E402


def frames(path):
    out, cur = [], []
    with open(path) as f:
        for line in f:
            try:
                r = json.loads(line)
            except json.JSONDecodeError:
                continue
            cur.append(r)
            if r.get("fn") == "dev.Present":
                out.append(cur)
                cur = []
    return out


def cut(frame):
    start = None
    for i, r in enumerate(frame):
        fn = r.get("fn", "")
        if fn == "dev.ShowCursor":
            start = i
            break
    if start is None:
        for i, r in enumerate(frame):
            a = r.get("args", {})
            if r.get("fn") == "font.DrawTextA" and "SAMPAUX3 h28" in str(r.get("font", "")) or (
                r.get("fn") == "dev.DrawPrimitive" and a.get("type") == 3
            ):
                start = i
                break
    return frame[start:] if start is not None else []


def cut_plate(path):
    """The plate painter block (D3DXCreateTexture 64x32 … the render-to-surface EndScene + its device EndScene)."""
    out, on = [], False
    with open(path) as f:
        for line in f:
            try:
                r = json.loads(line)
            except json.JSONDecodeError:
                continue
            a = r.get("args", {})
            if not on and r.get("fn") == "D3DXCreateTexture" and a.get("w") == 64 and a.get("h") == 32:
                on = True
            if on:
                out.append(r)
                if r.get("fn") == "dev.EndScene" and r.get("sc") == "d3dx":
                    return out
    return out


def norm(recs):
    out = []
    probe = False
    sprites = {}
    for r in recs:
        n = d3d_diff.norm(r, sprites)
        if n.get("by") in ("samp.dll", "samp_client.dll"):
            n["by"] = "client"
        a = n.get("args", {})
        fn = n.get("fn")
        if fn == "dev.DrawIndexedPrimitive":
            a.pop("min_index", None)
            a.pop("start_index", None)
        if fn == "sprite.Draw" and n.get("by") == "d3dx9_25.dll":
            # Glyphs of a DrawText: their place in the font's glyph atlas depends on which glyphs that font cached
            # earlier in the session (D3DX internal state), not on the caller.
            a.pop("src", None)
            a.pop("center", None)
            a.pop("pos", None)
        if fn == "dev.DrawIndexedPrimitive" and n.get("by") == "d3dx9_25.dll":
            n.pop("vertices", None)
        if fn == "dev.DrawPrimitive":
            n.pop("vertices", None)
            n.pop("v", None)
        if fn == "font.DrawTextA" and a.get("fmt") == "0x505":
            a.pop("rect", None)
            probe = True
        elif fn == "font.DrawTextA.ret" and probe:
            rc = n.pop("rect", None)
            n.pop("h", None)
            if rc:
                n["size"] = [rc[2] - rc[0], rc[3] - rc[1]]
            probe = False
        n.pop("seq", None)
        out.append(json.dumps(n, sort_keys=True))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("left")
    ap.add_argument("right")
    ap.add_argument("--frame-left", type=int, default=1)
    ap.add_argument("--frame-right", type=int, default=1)
    ap.add_argument("--dump", action="store_true")
    ap.add_argument("--plate", action="store_true", help="compare the first plate painter block instead")
    a = ap.parse_args()
    if a.plate:
        l, r = norm(cut_plate(a.left)), norm(cut_plate(a.right))
        d = list(difflib.unified_diff(l, r, a.left, a.right, lineterm="", n=2))
        print("\n".join(d) if d else f"MATCH ({len(l)} calls)")
        return 1 if d else 0
    lf, rf = frames(a.left), frames(a.right)
    if a.frame_left >= len(lf) or a.frame_right >= len(rf):
        print(f"not enough frames ({len(lf)} / {len(rf)})")
        return 2
    l, r = norm(cut(lf[a.frame_left])), norm(cut(rf[a.frame_right]))
    if a.dump:
        print("\n".join(l))
        print("=====")
        print("\n".join(r))
    if not l:
        print("left: no editor/selection pass in that frame")
        return 2
    d = list(difflib.unified_diff(l, r, a.left, a.right, lineterm="", n=2))
    if not d:
        print(f"MATCH ({len(l)} calls)")
        return 0
    print("\n".join(d))
    return 1


if __name__ == "__main__":
    sys.exit(main())
