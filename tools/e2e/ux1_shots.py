#!/usr/bin/env python3
"""UX-1: same-moment screenshot comparison of the boot (sanity check next to the D3D diff).

  ux1_shots.py OURS_DIR ORIG_DIR [--max-diff N]

Each directory comes from tools/diff/visual.sh: boot/shot-NNN.png with a `mark shot-NNN` record in
d3d-full.jsonl before each shot, and class.png (the class-selection screen). Moments compared:
  * loading screen: the first shot taken while gGameState is 8 and the loading bar is shown
    (more than 400 Presents logged); the bar band (y 640–690 at 720 lines) is left out because
    its length depends on loading speed (see tools/diff/boot_diff.py);
  * class selection: class.png.
The score is the mean absolute difference per channel (0–255). Exit 0 when every moment is at or
below --max-diff (default 6).
"""
import argparse
import json
import os
import sys

from PIL import Image, ImageChops, ImageStat


def shot_states(d):
    """shot number -> (game state, Presents so far) from the marks in d3d-full.jsonl."""
    out, gs, pr = {}, None, 0
    with open(os.path.join(d, "d3d-full.jsonl")) as f:
        for line in f:
            try:
                r = json.loads(line)
            except ValueError:
                continue
            pr = r.get("pr", pr)
            if r.get("fn") == "game_state":
                gs = r.get("gs")
            elif r.get("fn") == "mark" and str(r.get("text", "")).startswith("shot-"):
                out[r["text"][5:]] = (gs, pr)
    return out


def loading_shot(d):
    for n, (gs, pr) in sorted(shot_states(d).items()):
        p = os.path.join(d, "boot", "shot-%s.png" % n)
        if gs == 8 and pr > 400 and os.path.exists(p):
            return p
    return None


def score(a, b, mask_band=None):
    ia, ib = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
    if ia.size != ib.size:
        return 255.0
    if mask_band:
        h = ia.size[1]
        y0, y1 = int(mask_band[0] * h / 720), int(mask_band[1] * h / 720)
        for im in (ia, ib):
            im.paste((0, 0, 0), (0, y0, im.size[0], y1))
    st = ImageStat.Stat(ImageChops.difference(ia, ib))
    return sum(st.mean) / 3


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("orig")
    ap.add_argument("--max-diff", type=float, default=6.0)
    a = ap.parse_args()
    bad = 0
    pairs = [("loading screen", loading_shot(a.ours), loading_shot(a.orig), (640, 690)),
             ("class selection", os.path.join(a.ours, "class.png"), os.path.join(a.orig, "class.png"), None)]
    for name, po, pg, band in pairs:
        if not (po and pg and os.path.exists(po) and os.path.exists(pg)):
            print("%-16s MISSING (ours %s, original %s)" % (name, po, pg))
            bad += 1
            continue
        s = score(po, pg, band)
        ok = s <= a.max_diff
        bad += 0 if ok else 1
        print("%-16s %s mean abs diff %.2f (ours %s, original %s)" % (name, "OK  " if ok else "DIFF", s,
                                                                      os.path.relpath(po), os.path.relpath(pg)))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
