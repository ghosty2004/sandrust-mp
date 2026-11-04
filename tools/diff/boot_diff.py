#!/usr/bin/env python3
"""UX-1: boot Direct3D call-stream diff (loading screen → connecting screen → class selection).

  boot_diff.py OURS_D3D ORIG_D3D [--max N] [--frames]

Both logs come from samp-tracer with SAMP_TRACER_D3D_BOOT (every call from the first Present
while gGameState < 9, then the client's UI pass). The stream is cut into Presents; each Present
is normalised (tools/trace/d3d_diff.py `norm`; samp_client.dll named samp.dll) and labelled with
the game state. Phase 1 (gGameState < 9, loading screen): the frames are compared one to one
(same number of Presents in both: the loading screen is driven by the game's own loading
steps). Phase 2 (gGameState 9, the client's UI pass until the first capture command or the end
of boot logging): consecutive identical frames are collapsed into runs and the run sequences
are compared without their lengths (the connecting-screen timing is wall-clock based and the
logger slows both games down). Exit 0 when both phases match.
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


def tool():
    spec = importlib.util.spec_from_file_location("d3d_diff", os.path.join(ROOT, "tools", "trace", "d3d_diff.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def mask_bar(n):
    """Loading bar (untextured quads drawn by CLoadingScreen): its length is
    chunks_loaded_since_bar / (total − chunk at which the bar appeared), and the bar appears
    after a fixed time, so the denominator depends on how fast the machine loads (measured: 110
    vs 109 steps in two runs of the same build). The x coordinates are masked; the colours, y
    band and the number of quads are kept."""
    if n["fn"] == "dev.DrawPrimitive" and n.get("tex0") is None and isinstance(n.get("vertices"), dict):
        vv = dict(n["vertices"])
        vv.pop("hash", None)
        vv["v"] = [re.sub(r"^[-0-9.]+,", "x,", v) for v in vv.get("v", [])]
        n["vertices"] = vv
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


def norm_frame(t, recs, loading=False):
    sprites = {}
    out = []
    calc = False
    for r in recs:
        n = t.norm(r, sprites)
        if loading:
            n = mask_bar(n)
        calc = calc_ret(n, calc)
        n = glyph_batch(n)
        if n.get("by") == "samp_client.dll":
            n["by"] = "samp.dll"
        a = n.get("args") or {}
        for k in ("min_index", "start_index", "base"):
            a.pop(k, None)
        # Glyph-atlas texel rects depend on D3DX's glyph-cache fill order.
        if n["fn"] == "sprite.Draw" and "file:" not in str(a.get("tex")) and "256x256" in str(a.get("tex")):
            a.pop("src", None)
        fmt = a.get("fmt")
        if isinstance(fmt, str) and int(fmt, 16) & 0x400:
            a.pop("rect", None)
            a.pop("color", None)
            n.pop("rect", None)
        if n["fn"].endswith(".ret"):
            n.pop("h", None)
        if "text" in a:
            # Ports / addresses / times in the connect lines are run-specific.
            a["text"] = re.sub(r"[0-9]+", "#", a["text"])
        out.append(json.dumps(n, sort_keys=True))
    return out


# Number-plate painter (samp B 0x1006d8xx–0x1006d9xx, called from samp's vehicle creation):
# plate textures are not reproduced yet (ROADMAP decision "UX-1 plates"), so its calls after
# the start-up frame are left out on both sides.
PLATE_PATH = re.compile(r"samp(_client)?\.dll\+0x6d[89][0-9a-f]{2}\b")


def plate_call(r):
    return bool(PLATE_PATH.search(str(r.get("by", "")) + " " + str(r.get("org", ""))))


def game_side(r):
    """GTA's own calls outside its scene in the UI-pass log (texture/buffer creation while
    streaming, render-target and camera setup): game work whose timing depends on streaming,
    not the client's UI pass."""
    by = str(r.get("by", ""))
    fn = r.get("fn", "")
    if by.startswith("gta_sa.exe") and fn.startswith(("dev.Create", "dev.SetRenderTarget", "dev.SetDepthStencilSurface")):
        return True
    # GTA's SetTransform reaches the device through the client's proxy thunk.
    if fn == "dev.SetTransform" and r.get("sc") == "none" and by.startswith(("samp.dll", "samp_client.dll")) \
            and (r.get("args") or {}).get("state") in (2, 3) and not r.get("org"):
        return True
    return False


def glyph_batch(n):
    """D3DX glyph batches: positions and colours kept, atlas texture coordinates dropped (they
    depend on the glyph-cache fill order and differ for different digits, e.g. the port in the
    connect line)."""
    if n["fn"] == "dev.DrawIndexedPrimitive" and "file:" not in str(n.get("tex0")) and "256x256" in str(n.get("tex0")):
        vv = n.get("vertices")
        if isinstance(vv, dict):
            vv = dict(vv)
            vv.pop("hash", None)
            vv["v"] = [" ".join(x.split()[:-1]) if len(x.split()) > 2 else x for x in vv.get("v", [])]
            n["vertices"] = vv
    return n


def boot_frames(path, keep_plates=False):
    """[(game_state, [records])] per Present while boot logging is on, until the mark `boot-end`
    (visual.sh: the server saw the class request) or the first scenario capture."""
    gs = None
    on = False
    frames, cur = [], []
    g9 = 0
    in_plate = False
    with open(path) as f:
        for line in f:
            try:
                r = json.loads(line)
            except ValueError:
                continue
            fn = r.get("fn", "")
            if fn == "mark" and str(r.get("text", "")).startswith(("d3d-begin", "boot-end")):
                break
            if fn == "game_state":
                gs = r.get("gs")
                continue
            if fn == "d3d_logging":
                on = r.get("mode") != "off"
                continue
            if fn in ("mark", "d3d_installed") or not on:
                continue
            if not keep_plates and gs == 9 and g9 > 0:
                # The plate render: a render-to-surface scope on a 128x32 surface, plus the
                # painter's own calls (texture creation, copies).
                if fn == "rts.BeginScene" and "128x32" in str((r.get("args") or {}).get("surface")):
                    in_plate = True
                creation = fn.startswith("D3DXCreate") and "128" in json.dumps(r.get("args"))
                if in_plate or (plate_call(r) and not creation):
                    if fn == "rts.EndScene":
                        in_plate = False
                    continue
            if gs == 9 and game_side(r):
                continue
            cur.append(r)
            if fn == "dev.Present":
                frames.append((gs, cur))
                cur = []
                if gs == 9:
                    g9 += 1
    return frames


def runs(lines_per_frame):
    out = []
    for lines in lines_per_frame:
        if out and out[-1][0] == lines:
            out[-1][1] += 1
        else:
            out.append([lines, 1])
    return out


def diff_runs(rg, ro, report, max_lines, tail_ok=False):
    """Differing lines between two run sequences (run lengths ignored)."""
    kg = [json.dumps(f) for f, _ in rg]
    ko = [json.dumps(f) for f, _ in ro]
    if ko == kg:
        return 0
    changes = 0
    sm = difflib.SequenceMatcher(a=kg, b=ko, autojunk=False)
    for op, a0, a1, b0, b1 in sm.get_opcodes():
        if op == "equal":
            continue
        if tail_ok and op in ("delete", "insert") and a1 == len(kg) and b1 == len(ko):
            # The logged window ends at a mark set from outside (the class request seen by the
            # server): a run that one side reached just before the cut is not a difference.
            report.append("  runs %s at the end of the window: original %d..%d, ours %d..%d (cut, not counted)" % (
                op, a0, a1, b0, b1))
            continue
        report.append("  runs %s: original %d..%d, ours %d..%d" % (op, a0, a1, b0, b1))
        ga = [x for f, _ in rg[a0:a1] for x in f]
        gb = [x for f, _ in ro[b0:b1] for x in f]
        d = [x for x in difflib.unified_diff(ga, gb, "original", "ours", n=0, lineterm="")
             if x[:1] in "+-" and not x.startswith(("+++", "---"))]
        changes += max(1, len(d))
        report.extend(x[:240] for x in d[:max_lines])
    return changes


def compare(ours, orig, max_lines=40, verbose=False):
    t = tool()
    fo, fg = boot_frames(ours), boot_frames(orig)
    report = []
    changes = 0
    # Phase 1: loading screen. Frames are collapsed into runs of identical frames (the loading
    # screen repeats a frame while a step loads) and the run sequences are compared; the game
    # state is part of each frame.
    lo = [[json.dumps({"gs": g})] + norm_frame(t, r, True) for g, r in fo if g is not None and g < 9]
    lg = [[json.dumps({"gs": g})] + norm_frame(t, r, True) for g, r in fg if g is not None and g < 9]
    ro, rg = runs(lo), runs(lg)
    report.append("phase 1 (gGameState < 9): original %d Presents in %d runs, ours %d Presents in %d runs" % (
        len(lg), len(rg), len(lo), len(ro)))
    changes += diff_runs(rg, ro, report, max_lines)
    if verbose:
        report.append("original runs: " + " ".join(str(n) for _, n in rg))
        report.append("ours runs:     " + " ".join(str(n) for _, n in ro))
    so = runs([f for g, f in [(g, norm_frame(t, r)) for g, r in fo if g == 9]])
    sg = runs([f for g, f in [(g, norm_frame(t, r)) for g, r in fg if g == 9]])
    report.append("phase 2 (gGameState 9, UI pass; plate painter calls left out): original %d frames in %d runs, ours %d frames in %d runs" % (
        sum(n for _, n in sg), len(sg), sum(n for _, n in so), len(so)))
    changes += diff_runs(sg, so, report, max_lines, tail_ok=True)
    if verbose:
        report.append("original runs: " + " ".join(str(n) for _, n in sg))
        report.append("ours runs:     " + " ".join(str(n) for _, n in so))
    return changes, report


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("orig")
    ap.add_argument("--max", type=int, default=40)
    ap.add_argument("--frames", action="store_true", help="print the run lengths")
    a = ap.parse_args()
    changes, report = compare(a.ours, a.orig, a.max, a.frames)
    for l in report:
        print(l)
    print("boot: %d differing lines" % changes)
    sys.exit(1 if changes else 0)


if __name__ == "__main__":
    main()
