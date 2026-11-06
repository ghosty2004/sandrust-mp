#!/usr/bin/env python3
"""Remote driver nametag: where the bars are and which world point they belong to.

  vehtag_eval.py DIR [DIR ...]          describe every capture
  vehtag_eval.py --check ORIG OURS      exit 0 when every capture's tag sits at the same height
                                        relative to the driver on both clients

DIR holds the output of tools/diff/vehtag-measure.sh for one client (NAME-remote.txt probes and
d3d-NAME.jsonl captures). For every capture: the bar origin (X, Y) of each nametag fan group
(border quad = [X-20, Y+18 ...]), the remote's vehicle matrix position and ped position, the
device VIEW / PROJECTION / viewport (tracer `xform`) and GTA's camera position; then the screen
point predicted for the vehicle anchor `(vx, vy, vz + 0.0475*d + 0.2)` (d = ped-to-camera
distance, A 0x1006d060) and the anchor height implied by the measured Y on the vertical line
through the vehicle position.
"""
import json
import math
import os
import re
import struct
import sys


def parse_probes(path):
    """The probe blocks of a capture (one before, one after it), each a dict."""
    if not os.path.exists(path):
        return []
    blocks = []
    for line in open(path):
        if line.startswith("remote count="):
            blocks.append([])
        if blocks:
            blocks[-1].append(line)
    return [parse_block(b) for b in blocks]


def parse_block(lines):
    out = {}
    for line in lines:
        line = line.strip()
        m = re.match(r"remote slot=\d+ ped=(\S+) .*pos=([-\d.]+),([-\d.]+),([-\d.]+)", line)
        if m and "ped" not in out:
            out["ped"] = tuple(float(m.group(i)) for i in (2, 3, 4))
        m = re.match(r"vehicle=\S+ pos=([-\d.]+),([-\d.]+),([-\d.]+)", line)
        if m and "veh" not in out:
            out["veh"] = tuple(float(m.group(i)) for i in (1, 2, 3))
        if line.startswith("xform ") and "view" not in out:
            kv = dict(p.split("=", 1) for p in line.split()[1:])
            vp = kv["vp"].split(",")
            out["vp"] = [float(x) for x in vp]
            out["view"] = [float(x) for x in kv["view"].split(",")]
            out["proj"] = [float(x) for x in kv["proj"].split(",")]
        m = re.match(r"(samp_\w+ )?peek (0x[0-9a-f]+) (.*)", line)
        if m:
            b = bytes(int(x, 16) for x in m.group(3).split())
            key = (m.group(1) or "").strip() or {0xB6F9CC: "cam"}.get(int(m.group(2), 16))
            if key and key not in out:
                out[key] = struct.unpack("<%df" % (len(b) // 4), b)
    return out


def bars(path):
    """Per frame: list of (X, Y) bar origins (one per border quad with colour FF000000)."""
    frames = {}
    if not os.path.exists(path):
        return frames
    for line in open(path):
        if '"fvf":"0x44"' not in line or "DrawPrimitiveUP" not in line:
            continue
        d = json.loads(line)
        v = d["vertices"]["v"]
        if not v[0].endswith("#FF000000"):
            continue
        x0, y0 = (float(t) for t in v[0].split()[0].split(",")[:2])
        frames.setdefault(d["f"], []).append((x0 + 20, y0 - 18))
    return frames


def mul(v, m):
    return [sum(v[i] * m[4 * i + j] for i in range(4)) for j in range(4)]


def project(p, view, proj, vp):
    c = mul(mul([p[0], p[1], p[2], 1.0], view), proj)
    x, y, z = c[0] / c[3], c[1] / c[3], c[2] / c[3]
    return vp[0] + (1 + x) * vp[2] / 2, vp[1] + (1 - y) * vp[3] / 2, z


def implied_z(veh, y_screen, view, proj, vp):
    lo, hi = veh[2] - 5, veh[2] + 10
    f = lambda z: project((veh[0], veh[1], z), view, proj, vp)[1] - y_screen
    if f(lo) * f(hi) > 0:
        return None
    for _ in range(60):
        mid = (lo + hi) / 2
        if f(lo) * f(mid) <= 0:
            hi = mid
        else:
            lo = mid
    return (lo + hi) / 2


def evaluate(d, n):
    """One capture: dict with the bar origin, the vehicle-anchor prediction and the implied
    anchor height above the ped's root (on the vertical line through the ped), from the probe
    before the capture (`h_ped`, `h_veh`) and from both probes (`h_all`; the in-car camera and
    the car move a little between the probes and the captured frame)."""
    probes = parse_probes(os.path.join(d, n + "-remote.txt"))
    fr = bars(os.path.join(d, "d3d-%s.jsonl" % n))
    pts = [pt for f in sorted(fr) for pt in fr[f]]
    p = probes[0] if probes else {}
    r = {"name": n, "bars": pts[0] if pts else None, "probe": p, "h_all": []}
    for k, q in enumerate(probes):
        if not all(key in q for key in ("veh", "ped", "cam", "view")):
            continue
        dist = math.dist(q["ped"], q["cam"])
        raise_ = 0.0475 * dist + 0.2
        if k == 0:
            r["dist"] = dist
            sx, sy, _ = project((q["veh"][0], q["veh"][1], q["veh"][2] + raise_), q["view"], q["proj"], q["vp"])
            r["pred_vehicle"] = (int(sx), int(sy))
        z = implied_z(q["ped"], pts[0][1], q["view"], q["proj"], q["vp"]) if pts else None
        if z is None:
            continue
        r["h_all"].append(z - raise_ - q["ped"][2])
        if k == 0:
            r["h_ped"] = z - raise_ - q["ped"][2]
            r["h_veh"] = z - raise_ - q["veh"][2]
    return r


def describe(r):
    p = r["probe"]
    line = "%-10s bars=%s" % (r["name"], r["bars"])
    if "dist" in r:
        line += " veh=%.3f,%.3f,%.3f ped=%.3f,%.3f,%.3f d=%.2f vehicle-anchor would be at %s" % (
            p["veh"] + p["ped"] + (r["dist"], r["pred_vehicle"]))
    if "h_ped" in r:
        line += " implied anchor: %+.3f m above the ped root, %+.3f m above the vehicle position" % (
            r["h_ped"], r["h_veh"])
    if "samp_anchor" in p:
        line += " samp_anchor=%.3f,%.3f,%.3f" % p["samp_anchor"]
        if all(k in p for k in ("samp_view", "samp_proj", "vp", "cam", "ped")):
            a = p["samp_anchor"]
            raised = (a[0], a[1], a[2] + 0.0475 * math.dist(p["ped"], p["cam"]) + 0.2)
            sx, sy, _ = project(raised, p["samp_view"], p["samp_proj"], p["vp"])
            line += " samp_pred=(%d,%d)" % (int(sx), int(sy))
    return line


def captures(d):
    return sorted(n[4:-6] for n in os.listdir(d) if n.startswith("d3d-vt-") and n.endswith(".jsonl"))


# Largest allowed difference of the implied anchor height between the clients (m). The car
# drifts a few cm between the probes and the captured frame; head vs vehicle anchor is 0.3 m.
TOLERANCE = 0.08


def check(orig, ours):
    bad = 0
    for n in captures(orig):
        a, b = evaluate(orig, n), evaluate(ours, n) if os.path.exists(
            os.path.join(ours, "d3d-%s.jsonl" % n)) else {"name": n}
        ha, hb = a.get("h_all") or [], b.get("h_all") or []
        if a.get("bars") is None and b.get("bars") is None:
            print("%-10s OK: no tag on either client" % n)
            continue
        if not ha or not hb:
            print("%-10s FAIL: tag on one client only (orig %s, ours %s)" % (n, a.get("bars"), b.get("bars")))
            bad += 1
            continue
        gap = min(abs(x - y) for x in ha for y in hb)
        ok = gap <= TOLERANCE
        bad += not ok
        fmt = lambda hs: "/".join("%+.3f" % h for h in hs)
        print("%-10s %s: anchor %s m above the ped root (orig) vs %s m (ours), bars %s vs %s" % (
            n, "OK" if ok else "FAIL", fmt(ha), fmt(hb), a["bars"], b["bars"]))
    return bad


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--check":
        bad = check(sys.argv[2], sys.argv[3])
        print("vehtag: %s" % ("PASS" if not bad else "FAIL (%d)" % bad))
        sys.exit(1 if bad else 0)
    for d in sys.argv[1:]:
        print("==", d)
        for n in captures(d):
            print(describe(evaluate(d, n)))


if __name__ == "__main__":
    main()
