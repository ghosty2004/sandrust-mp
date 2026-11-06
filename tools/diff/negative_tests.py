#!/usr/bin/env python3
"""Negative tests for the F1 (rpc_report.py) and F3 (xclient.py report) criteria: a recorded run of
our client is changed the way a broken client would behave, and the report must then FAIL that step.

    negative_tests.py f1 --orig RUN [--orig RUN ...] --ours RUN [--work DIR]
    negative_tests.py f3 --run XCLIENT_RUN --calib XCALIB_RUN [--calib ...] [--work DIR]

F1 cases (on a copy of the ours run; unchanged files are symlinked):
  no_stopflash   gang zone keeps flashing after RPC 85: the gangzone_stopflash segment's per-frame
                 CRadar::DrawAreaOnRadar colours alternate like in gangzone_flash (27-frame phases)
  no_flash       RPC 121 ignored: the gangzone_flash segment draws the zone colour only
  wrong_flash    flash colour wrong: the flash colour in gangzone_flash replaced by another colour
  slow_flash     flash phase twice as long (54 frames)
  no_hide        RPC 120 ignored: the zone is still drawn through the end of gangzone_hide
  no_textdraw    a static UI element not drawn: ours' screenshot of ptextdraw_show is the previous shot
  wrong_pos      SetPlayerPos (pos_offset) lands 1 m off: the quantised state positions keep a 1-quantum
                 tolerance (rounding boundary), two quanta must still fail
F3 cases (on a copy of samples.jsonl):
  observer_no_enter    our observer does not play the door/get-in animations of the original sender
                       (veh_enter_A: group 88 removed from B's view of A)
  sender_no_enter      our sender does not play them (veh_enter_B: group 88 removed from B's own ped and
                       from A's view of B)
  observer_sit_pro     the original observer shows CAR_sit_pro for our sender in veh_enter_B, which it
                       never does for an original sender (calibration): what we send differs
  ours_no_respawn      our client did not request its spawn after the death (the driver had the server
                       spawn it: combat_respawn_B marked forced_spawn B)
  obs_unoccupied_off   our observer keeps A's rolling empty car 1 m off (veh_unoccupied_A, uveh_pos)
  obs_no_trailer       our observer shows A's truck without the trailer (veh_trailer_A)
  obs_no_head_object   our client lacks the object attached to A's head (objects, obj_player)
  obs_no_chat          A's chat line is missing from our chatlog (chat_A)
  obs_no_bubble        our client holds no chat bubble for A (bubble)
  ours_shot_misses     our aimed shot misses the original: no player hit, no damage (combat_hit_B)
Exit 0 when every case fails as expected (and the unchanged run still passes those steps).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
RADAR = "CRadar::DrawAreaOnRadar"


def copy_run(src, dst, keep=()):
    """dst: symlinks to src's files (shots per file), except the names in `keep`."""
    shutil.rmtree(dst, ignore_errors=True)
    os.makedirs(os.path.join(dst, "shots"))
    for f in os.listdir(src):
        if f in keep or f == "shots":
            continue
        os.symlink(os.path.realpath(os.path.join(src, f)), os.path.join(dst, f))
    for f in os.listdir(os.path.join(src, "shots")):
        os.symlink(os.path.realpath(os.path.join(src, "shots", f)), os.path.join(dst, "shots", f))


def rewrite_trace(src, dst, step, fn):
    """Copy trace.jsonl; fn(rec, nth) rewrites RADAR records inside `step` (nth = index within the
    segment) and may return None to drop the record. Returns the number of records changed."""
    cur, n, changed = None, 0, 0
    with open(os.path.join(src, "trace.jsonl"), errors="replace") as f, open(os.path.join(dst, "trace.jsonl"), "w") as g:
        for raw in f:
            if '"mark"' in raw:
                m = re.search(r'"text":"step \d+ (\S+) (begin|end)"', raw)
                if m:
                    cur = m.group(1) if m.group(2) == "begin" else None
                    n = 0
            if cur == step and RADAR in raw:
                rec = json.loads(raw)
                out = fn(rec, n)
                n += 1
                changed += 1
                if out is None:
                    continue
                raw = json.dumps(out, separators=(",", ":")) + "\n"
            g.write(raw)
    return changed


def zone_colours(run, step):
    cols = []
    cur = None
    for raw in open(os.path.join(run, "trace.jsonl"), errors="replace"):
        if '"mark"' in raw:
            m = re.search(r'"text":"step \d+ (\S+) (begin|end)"', raw)
            if m:
                cur = m.group(1) if m.group(2) == "begin" else None
        elif cur == step and RADAR in raw:
            c = json.loads(raw)["args"]["rgba"]
            if c not in cols:
                cols.append(c)
    return cols


def last_frame(run, step):
    """(first, last) frame of a step segment."""
    first = last = None
    for raw in open(os.path.join(run, "trace.jsonl"), errors="replace"):
        if '"mark"' in raw:
            m = re.search(r'"text":"step \d+ (\S+) (begin|end)"', raw)
            if m and m.group(1) == step:
                rec = json.loads(raw)
                if m.group(2) == "begin":
                    first = rec["frame"]
                else:
                    last = rec["frame"]
    return first, last


def run_f1_report(origs, ours, out):
    args = [sys.executable, "-I", os.path.join(HERE, "rpc_report.py"), "--out", out, "--ours", ours]
    for o in origs:
        args += ["--orig", o]
    subprocess.run(args, stdout=subprocess.DEVNULL, check=False)
    return {s["name"]: s for s in json.load(open(os.path.join(out, "rpc-coverage.json")))["steps"]}


def f1(a):
    work = a.work
    os.makedirs(work, exist_ok=True)
    ours = os.path.realpath(a.ours)
    base, flash = zone_colours(ours, "gangzone_show"), zone_colours(ours, "gangzone_flash")
    if not base or len(flash) < 2:
        print("ours run has no gang zone flash to work on")
        return 1
    zone = base[0]
    other = [c for c in flash if c != zone][0]
    cases = {}

    def alternate(period):
        def fn(rec, n):
            rec["args"]["rgba"] = zone if (n // period) % 2 else other
            return rec
        return fn

    def const(c):
        def fn(rec, n):
            rec["args"]["rgba"] = c
            return rec
        return fn

    def swap_flash(rec, n):
        if rec["args"]["rgba"] == other:
            rec["args"]["rgba"] = "0x8000ff00"
        return rec

    trace_cases = {
        "no_stopflash": ("gangzone_stopflash", alternate(27), "gangzone_stopflash"),
        "no_flash": ("gangzone_flash", const(zone), "gangzone_flash"),
        "wrong_flash": ("gangzone_flash", swap_flash, "gangzone_flash"),
        "slow_flash": ("gangzone_flash", alternate(54), "gangzone_flash"),
    }
    for name, (step, fn, judged) in trace_cases.items():
        d = os.path.join(work, "f1-" + name)
        copy_run(ours, d, keep=("trace.jsonl",))
        n = rewrite_trace(ours, d, step, fn)
        cases[name] = (d, judged, f"{n} radar records of {step} rewritten")
    # no_hide: the zone keeps being drawn every frame through gangzone_hide (records synthesised from the
    # previous step's last draw, one per frame of the hide segment).
    d = os.path.join(work, "f1-no_hide")
    copy_run(ours, d, keep=("trace.jsonl",))
    first, last = last_frame(ours, "gangzone_hide")
    proto = None
    cur = None
    with open(os.path.join(ours, "trace.jsonl"), errors="replace") as f, open(os.path.join(d, "trace.jsonl"), "w") as g:
        for raw in f:
            if '"mark"' in raw:
                m = re.search(r'"text":"step \d+ (\S+) (begin|end)"', raw)
                if m:
                    if m.group(1) == "gangzone_hide" and m.group(2) == "end" and proto:
                        for fr in range(first, last + 1):
                            r = dict(proto, frame=fr, t=proto["t"] + 19 * (fr - proto["frame"]))
                            g.write(json.dumps(r, separators=(",", ":")) + "\n")
                    cur = m.group(1) if m.group(2) == "begin" else None
            if cur == "gangzone_stopflash" and RADAR in raw:
                proto = json.loads(raw)
            if cur == "gangzone_hide" and RADAR in raw:
                continue
            g.write(raw)
    cases["no_hide"] = (d, "gangzone_hide", f"zone drawn in every frame {first}..{last} of gangzone_hide")
    # no_textdraw: ours' screenshot of ptextdraw_show = its previous shot (nothing appeared).
    d = os.path.join(work, "f1-no_textdraw")
    copy_run(ours, d)
    steps = [json.loads(ln) for ln in open(os.path.join(ours, "steps.jsonl"))]
    names = [s["name"] for s in steps]
    i = names.index("ptextdraw_show")
    shot, prev = steps[i]["shot"], steps[i - 1]["shot"]
    os.unlink(os.path.join(d, shot))
    shutil.copy(os.path.join(ours, prev), os.path.join(d, shot))
    cases["no_textdraw"] = (d, "ptextdraw_show", f"{shot} replaced by {prev}")
    # wrong_pos: SetPlayerPos applied 1 m off in x (two quanta of the recorded ped position).
    d = os.path.join(work, "f1-wrong_pos")
    copy_run(ours, d, keep=("steps.jsonl",))
    with open(os.path.join(d, "steps.jsonl"), "w") as g:
        for st in steps:
            if st["name"] == "pos_offset":
                post = st["post"]
                post["state"] = re.sub(r"pos=(-?[\d.]+)", lambda m: f"pos={float(m.group(1)) + 1.0:.3f}", post["state"])
                x, rest = post["snap"]["ped_pos"].split(",", 1)
                post["snap"]["ped_pos"] = f"{float(x) + 1.0:.1f},{rest}"
            g.write(json.dumps(st) + "\n")
    cases["wrong_pos"] = (d, "pos_offset", "ped position after RPC 12 moved +1.0 m in x")

    rc = 0
    base_rep = run_f1_report(a.orig, ours, os.path.join(work, "f1-report-unchanged"))
    for name, (d, step, what) in cases.items():
        rep = run_f1_report(a.orig, d, os.path.join(work, f"f1-report-{name}"))
        s = rep[step]
        before = base_rep[step]["verdict"]
        failed = s["verdict"] != "MATCH"
        r_ = s.get("radar") or {}
        detail = ("radar: " + r_["detail"]) if r_.get("ok") is False else "visual: " + (s.get("visual") or {}).get("detail", "")
        print(f"[neg] F1 {name:<13} {what}: {step} {before} -> {s['verdict']} "
              f"({'expected FAIL' if failed else 'NOT DETECTED'}) {detail}")
        rc |= 0 if (failed and before == "MATCH") else 1
    return rc


def f3(a):
    work = a.work
    os.makedirs(work, exist_ok=True)
    run = os.path.realpath(a.run)
    recs = [json.loads(ln) for ln in open(os.path.join(run, "samples.jsonl"))]
    sit_pro = "1ccd46ef"

    def strip88(view):
        if view and view.get("anims"):
            view["anims"] = [x for x in view["anims"] if "/88:" not in x["anim"]]

    def case_observer_no_enter(rs):
        for r in rs:
            if r["name"] == "veh_enter_A":
                for s in r["samples"]:
                    strip88((s.get("B") or {}).get("remote"))
        return "veh_enter_A", "orig→ours"

    def case_sender_no_enter(rs):
        for r in rs:
            if r["name"] == "veh_enter_B":
                for s in r["samples"]:
                    strip88((s.get("B") or {}).get("local"))
                    strip88((s.get("A") or {}).get("remote"))
        return "veh_enter_B", "ours→orig"

    def case_observer_sit_pro(rs):
        for r in rs:
            if r["name"] == "veh_enter_B":
                for s in r["samples"]:
                    v = (s.get("A") or {}).get("remote")
                    for x in (v or {}).get("anims", []):
                        if x["anim"].endswith("/0:60"):
                            x["anim"] = sit_pro + "/0:63"
        return "veh_enter_B", "ours→orig"

    def case_ours_no_respawn(rs):
        for r in rs:
            if r["name"] == "combat_respawn_B":
                r["forced_spawn"] = ["B"]
        return "combat_respawn_B", "ours→orig"

    def case_observer_unoccupied_off(rs):
        # Our observer keeps A's empty car 1 m from where A's client (and its 209s) put it.
        for r in rs:
            if r["name"] == "veh_unoccupied_A":
                for s in r["samples"]:
                    t = (s.get("B") or {}).get("track")
                    if t and t.get("pos"):
                        t["pos"][0] += 1.0
        return "veh_unoccupied_A", "orig→ours"

    def case_observer_no_trailer(rs):
        # Our observer shows A's truck without its trailer.
        for r in rs:
            if r["name"] == "veh_trailer_A":
                for s in r["samples"]:
                    v = ((s.get("B") or {}).get("remote") or {}).get("vehicle")
                    if v:
                        v["trailer"] = 0
                    t = (s.get("B") or {}).get("track")
                    if t:
                        t["trailer"] = None
        return "veh_trailer_A", "orig→ours"

    def case_observer_no_head_object(rs):
        # Our client does not show the object attached to A's head.
        for r in rs:
            if r["name"] == "objects":
                o = r["post"]["B"]["objects"]
                o["objects"] = [x for x in o.get("objects", []) if x["model"] != 18645]
        return "objects", "orig→ours"

    def case_observer_no_chat(rs):
        # A's chat line never reaches our chat window / chatlog.
        for r in rs:
            if r["name"] == "chat_A":
                r["post"]["B"]["chatlog"]["lines"] = []
        return "chat_A", "orig→ours"

    def case_observer_no_bubble(rs):
        # Our client holds no bubble for A.
        for r in rs:
            if r["name"] == "bubble":
                b = r["post"]["B"]["bubbles"]
                b["bubbles"] = [x for x in b.get("bubbles", []) if x.get("id") != str(r["pids"]["A"])]
        return "bubble", "orig→ours"

    def case_ours_shot_misses(rs):
        # Our shot does not hit the original (206 without the player hit, no damage on either side).
        for r in rs:
            if r["name"] == "combat_hit_B":
                r["server_log"] = [ln.replace("hittype=1", "hittype=0") for ln in r["server_log"]
                                   if "damage " not in ln]
                for k in r["post"]["B"]["synced"]["synced"]:
                    k["health"] = "100.0"
                for s in r["samples"]:
                    v = (s.get("A") or {}).get("local")
                    if v:
                        v["health"] = 100.0
        return "combat_hit_B", "ours→orig"

    rc = 0
    unchanged = report_f3(run, a.calib, os.path.join(work, "f3-report-unchanged"))
    for name, fn in (("observer_no_enter", case_observer_no_enter), ("sender_no_enter", case_sender_no_enter),
                     ("observer_sit_pro", case_observer_sit_pro), ("ours_no_respawn", case_ours_no_respawn),
                     ("obs_unoccupied_off", case_observer_unoccupied_off),
                     ("obs_no_trailer", case_observer_no_trailer),
                     ("obs_no_head_object", case_observer_no_head_object), ("obs_no_chat", case_observer_no_chat),
                     ("obs_no_bubble", case_observer_no_bubble), ("ours_shot_misses", case_ours_shot_misses)):
        d = os.path.join(work, "f3-" + name)
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
        rs = json.loads(json.dumps(recs))
        step, direction = fn(rs)
        with open(os.path.join(d, "samples.jsonl"), "w") as f:
            for r in rs:
                f.write(json.dumps(r) + "\n")
        os.symlink(os.path.join(run, "meta.json"), os.path.join(d, "meta.json"))
        rows = report_f3(d, a.calib, os.path.join(work, f"f3-report-{name}"))
        r = rows[(step, direction)]
        before = unchanged[(step, direction)]["verdict"]
        bad = [f"{k}: {c['val']}" for k, c in r["checks"].items() if c["ok"] is False]
        failed = r["verdict"] != "MATCH"
        print(f"[neg] F3 {name:<18} {step} {direction}: {before} -> {r['verdict']} "
              f"({'expected FAIL' if failed else 'NOT DETECTED'}) {'; '.join(bad)[:300]}")
        rc |= 0 if (failed and before == "MATCH") else 1
    return rc


def report_f3(run, calibs, out):
    args = [sys.executable, "-I", os.path.join(HERE, "xclient.py"), "report", "--run", run, "--out", out]
    for c in calibs:
        args += ["--calib", c]
    subprocess.run(args, stdout=subprocess.DEVNULL, check=False)
    name = "cross-client"
    rows = json.load(open(os.path.join(out, f"{name}.json")))["rows"]
    return {(r["step"], r["direction"]): r for r in rows}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("f1")
    p.add_argument("--orig", action="append", required=True)
    p.add_argument("--ours", required=True)
    p.add_argument("--work", default=os.path.join(ROOT, "target", "diff", "negative"))
    p = sub.add_parser("f3")
    p.add_argument("--run", required=True)
    p.add_argument("--calib", action="append", required=True)
    p.add_argument("--work", default=os.path.join(ROOT, "target", "diff", "negative"))
    a = ap.parse_args()
    return f1(a) if a.cmd == "f1" else f3(a)


if __name__ == "__main__":
    sys.exit(main())
