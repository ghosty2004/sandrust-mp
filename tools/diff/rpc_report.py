#!/usr/bin/env python3
"""DIFF-2 report: per-step and per-RPC comparison of the client reaction,
original client vs ours (criterion F1).

    rpc_report.py --orig RUN [--orig RUN2 ...] [--ours RUN] [--out target/diff]

RUN = a directory written by tools/diff/rpc-coverage.sh (steps.jsonl,
step-list.json, trace.jsonl, shots/). With two or more --orig runs the report
first checks that the original side is deterministic; whatever differs between
original runs is treated as noise (ignored) in the original-vs-ours comparison.

Three components per step:
  trace  — the GTA calls the client made in the step's mark segment (normalised
           with tools/trace/diff.py, minus background: per-frame lines common
           to most steps, the idle steps, whatever ran in the gap right before
           the step or in both neighbouring steps), compared as ordered unique
           lines;
  state  — `state` + tools/diff/gtastate.py snapshot: keys that changed during
           the step on either side must end at the same value; entity lists
           compare added/removed items;
  visual — screenshot change mask (this step's shot vs the previous one,
           sharp pixel changes per cell of a 32x18 grid), compared between the two clients (relative bound, see
           VISUAL_MIN_CELLS / VISUAL_SHARE).
Verdicts: MATCH, DIFF, NOT-IMPLEMENTED (ours has no handler for the step's RPCs
or shows no reaction at all where the original reacts), NOT-FIRED (scenario
error). Per RPC: the worst verdict over the steps that cover it; RPC ids no
step covers are UNTRIGGERABLE (listed with the reason).
Writes OUT/rpc-coverage.md and OUT/rpc-coverage.json. Exit 0 only if ours ran
and every covered RPC is MATCH.
"""
import argparse
import glob
import json
import os
import re
import sys
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tools", "trace"))
import diff as tdiff  # noqa: E402  (tools/trace/diff.py: trace normaliser)

# The 141 S->C RPC ids the R4 client registers (docs/re/rpc-ids.md).
REGISTERED = [
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24, 26, 27, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
    42, 43, 44, 45, 46, 47, 48, 55, 56, 57, 58, 59, 60, 61, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76,
    77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98, 99, 101, 102, 103, 104,
    105, 106, 107, 108, 111, 112, 113, 116, 117, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 133, 134,
    135, 137, 138, 139, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161,
    162, 163, 164, 165, 166, 167, 169, 170, 171, 172, 173, 174, 175, 176, 178,
]
# Why the RPCs without a step cannot be fired from Pawn on open.mp 1.6.7.
UNTRIGGERABLE = {
    64: "no open.mp sender (client reads a u16 and does nothing)",
    92: "no open.mp sender (drunk visuals, opcode 0x052C)",
    98: "no open.mp sender (vehicle tyre status)",
    102: "no open.mp sender (server net-stats blob)",
    125: "no open.mp sender (no-op handler)",
    130: "connection rejected: only on a bad join (version/nickname/mod/slots), not from Pawn",
    150: "no open.mp sender (drunk handling, opcode 0x03FD)",
    151: "no open.mp sender (dropped-weapon pickups)",
    161: "no open.mp sender (old vehicle params: objective/doors)",
    169: "no open.mp sender (object camera collision is a CreateObject flag in open.mp)",
}

# Trace records that are diagnostics, not GTA calls made by the client.
NOISE_FNS = {
    "pos", "pc", "rped", "guard", "guard_start", "guard_summary", "players_mem", "players_mem_base",
    "players_mem_summary", "globals", "globals_base", "vtable_wrap", "players", "first_frame",
    "tracer_start", "module", "quit", "suppressed",
}
# GTA's own streaming decisions (evictions, dependency loads inside
# LOAD_ALL_MODELS_NOW) depend on memory state, not on the client.
GTA_STREAMING = re.compile(
    r"^CStreaming::RemoveModel .*<gta>|^CStreaming::RequestModel \{model=\d+,flags=(0|8)\}  <gta>")
POLL_MIN_STEPS = 3
ORDERED = re.compile(r"^scm |<client>|\[via scm ")
# Lines present in at least this share of all step segments are per-frame
# background (polling), not a reaction.
BACKGROUND_SHARE = 0.5
GRID = (32, 18)
# A grid cell counts as changed when more than CELL_FRACTION of its pixels
# changed by more than PIXEL_THRESHOLD (0..255 grey). Sharp, local changes (UI
# text, markers, objects) pass; soft scene animation (escalators, lighting,
# JPEG noise) mostly does not. Tuned on three original runs: each judged
# against the other two (held-out false positives) vs sensitivity to UI steps.
PIXEL_THRESHOLD = 60
CELL_FRACTION = 0.08
# Change masks match if they differ in at most max(VISUAL_MIN_CELLS,
# VISUAL_SHARE × cells changed on either side) cells; small UI elements
# (a textdraw, a chat line) change only a few cells, so the bound is relative.
VISUAL_MIN_CELLS = 4
VISUAL_SHARE = 0.4


class Opts:
    globals = False
    stock = False
    digits = 1


# ---------------------------------------------------------------- loading
def load_run(d):
    run = {"dir": d, "steps": OrderedDict(), "list": []}
    try:
        run["list"] = json.load(open(os.path.join(d, "step-list.json")))
    except (OSError, ValueError):
        pass
    try:
        for ln in open(os.path.join(d, "steps.jsonl")):
            r = json.loads(ln)
            run["steps"][r["name"]] = r
    except (OSError, ValueError):
        pass
    run["segments"] = load_segments(os.path.join(d, "trace.jsonl"))
    return run


PLAYER_MODEL = re.compile(r"model=-?\d+,player=True")


def split_watch(line):
    if not line.startswith("watch "):
        return [line]
    return ["watch " + kv for kv in line[6:].split(" ") if kv]


def load_segments(path):
    """{step name: [normalised reaction-candidate lines]} from step marks, plus
    {"gap:" + step name: lines between the previous step's end and this begin}
    (the driver's pre-step state collection: ongoing per-frame activity)."""
    segs = {}
    cur = None
    gap = []
    try:
        f = open(path, errors="replace")
    except OSError:
        return segs
    with f:
        for raw in f:
            try:
                rec = json.loads(raw)
            except ValueError:
                continue
            fn = rec.get("fn")
            if fn == "mark":
                m = re.match(r"step \d+ (\S+) (begin|end)$", rec.get("args", {}).get("text", ""))
                if m:
                    cur = m.group(1) if m.group(2) == "begin" else None
                    if cur:
                        segs[cur] = []
                        segs["gap:" + cur] = gap
                    gap = []
                continue
            if fn in NOISE_FNS:
                continue
            # Keep calls the client made (directly, from its stack, or as a script
            # command) and global-state changes; drop GTA's own background work.
            client = tdiff.caller_class(rec.get("caller")) == "client" or bool(rec.get("origin"))
            if fn not in ("scm", "watch") and not client and not str(rec.get("via", "")).startswith("scm"):
                continue
            n = tdiff.normalise(rec, Opts)
            if n:
                # The local ped's current model is state (judged by the state check
                # and the SetModelIndex calls); keeping it in every entity
                # descriptor would turn one missed skin change into many diffs.
                n = PLAYER_MODEL.sub("player=True", n)
                # A `los` record's gta_stack is a raw stack scan: with our client it picks up stale
                # return addresses (Rust frames), so every frame looks different. Diagnostic only.
                n = LOS_STACK.sub("", n)
                (segs[cur] if cur is not None else gap).extend(split_watch(n))
    return segs


NUM = re.compile(r"-?\d+\.\d+|-?\d+")
LOS_STACK = re.compile(r",gta_stack=[^}]*")
FLOAT_TOLERANCE = 1.5   # world coordinates/angles in trace lines (physics jitter)


class LineSet:
    """Set of normalised trace lines with fuzzy membership: same text with
    numbers masked, integers equal, decimals within FLOAT_TOLERANCE. Model
    ids, handles and opcodes are integers and must match exactly."""

    def __init__(self, lines=()):
        self.by = {}
        for ln in lines:
            self.add(ln)

    @staticmethod
    def key(ln):
        nums = NUM.findall(ln)
        return NUM.sub("#", ln), nums

    def add(self, ln):
        t, nums = self.key(ln)
        self.by.setdefault(t, []).append(nums)

    def __contains__(self, ln):
        t, nums = self.key(ln)
        for cand in self.by.get(t, ()):
            if all(a == b or ("." in a and "." in b and abs(float(a) - float(b)) <= FLOAT_TOLERANCE)
                   for a, b in zip(nums, cand)):
                return True
        return False


def unique(lines):
    seen, out = set(), []
    for ln in lines:
        if ln not in seen:
            seen.add(ln)
            out.append(ln)
    return out


def reactions(run):
    """Per step: ordered unique lines minus background: lines in at least
    BACKGROUND_SHARE of all steps, lines of the idle steps, and lines already
    present in the gap right before the step or in both neighbouring steps
    (polling that runs regardless of the RPC, e.g. samp's periodic commands
    for every existing vehicle)."""
    allsegs = run["segments"]
    segs = {k: unique(v) for k, v in allsegs.items() if not k.startswith("gap:")}
    cnt = Counter(ln for v in segs.values() for ln in v)
    n = max(1, len(segs))
    bg = {ln for ln, c in cnt.items() if c / n >= BACKGROUND_SHARE}
    for k, v in segs.items():
        if k.startswith("idle"):
            bg.update(v)
    names = list(segs)
    bgset = LineSet(bg)
    out = {}
    removed = Counter()
    for i, k in enumerate(names):
        before = LineSet(allsegs.get("gap:" + k, ()))
        # Periodic (not per-frame) polling that runs across this step: present
        # in both neighbouring steps as well.
        if 0 < i < len(names) - 1:
            nxt = LineSet(segs[names[i + 1]])
            for ln in segs[names[i - 1]]:
                if ln in nxt:
                    before.add(ln)
        out[k] = [ln for ln in segs[k] if ln not in bgset and ln not in before and not GTA_STREAMING.search(ln)]
        removed.update({LineSet.key(ln)[0] for ln in segs[k] if ln in before})
    # Templates (numbers masked) removed as running background in several
    # steps: samp's periodic polls. Their per-step presence and arguments
    # depend on timing, so they are tolerated as missing/extra (see compare_runs).
    run["poll_templates"] = {t for t, c in removed.items() if c >= POLL_MIN_STEPS}
    return out


# ---------------------------------------------------------------- state
def parse_state_line(s):
    out = {}
    if not s or not s.startswith("state "):
        return {"state": s or "none"}
    for k, v in re.findall(r"(\w+)=(\S+)", s):
        if k in ("ticks", "frame_hook"):
            continue
        if k == "ped":
            v = "none" if v == "none" else "present"
        elif k == "pos":
            try:
                v = ",".join(f"{round(float(c) * 2) / 2:.1f}" for c in v.split(","))
            except ValueError:
                pass
        elif k == "heading":
            v = f"{round(float(v) * 20) / 20:.2f}"
        elif k in ("health", "armour"):
            v = f"{round(float(v)):.0f}"
        out["state." + k] = v
    return out


def flat_state(rec):
    st = parse_state_line(rec.get("state", ""))
    for k, v in (rec.get("snap") or {}).items():
        st["snap." + k] = v
    return st


def item_id(it):
    """Entity list item without its position (model + attributes)."""
    return re.sub(r"@\S+", "", it)


def state_delta(step):
    pre, post = flat_state(step.get("pre", {})), flat_state(step.get("post", {}))
    step["_pre_flat"] = pre
    scal, lists = {}, {}
    for k in set(pre) | set(post):
        a, b = pre.get(k), post.get(k)
        if isinstance(a, list) or isinstance(b, list):
            ca, cb = Counter(map(item_id, a or [])), Counter(map(item_id, b or []))
            add, rem = cb - ca, ca - cb
            if add or rem:
                lists[k] = (sorted(add.elements()), sorted(rem.elements()))
        elif a != b:
            scal[k] = b
    return scal, lists, post


def pre_differs(sa, sb, k):
    a, b = sa.get("_pre_flat", {}).get(k), sb.get("_pre_flat", {}).get(k)
    if isinstance(a, list) or isinstance(b, list):
        return Counter(map(item_id, a or [])) != Counter(map(item_id, b or []))
    return a != b


def compare_state(sa, sb, ignore=frozenset(), drift=False):
    """Differences between two steps' state reactions -> list of text lines.
    drift=True (original vs ours): a key the original did not change in this
    step and that already differed before the step is a standing divergence
    (reported once under "Standing state differences"), not this step's."""
    sca, lia, posta = state_delta(sa)
    scb, lib, postb = state_delta(sb)
    out = []
    for k in sorted(set(sca) | set(scb)):
        if k in ignore or (drift and k not in sca and pre_differs(sa, sb, k)):
            continue
        if posta.get(k) != postb.get(k):
            out.append(f"{k}: {posta.get(k)!s} vs {postb.get(k)!s}")
    for k in sorted(set(lia) | set(lib)):
        if k in ignore or (drift and k not in lia and pre_differs(sa, sb, k)):
            continue
        a, b = lia.get(k, ([], [])), lib.get(k, ([], []))
        if a != b:
            out.append(f"{k}: +{a[0]} -{a[1]} vs +{b[0]} -{b[1]}")
    return out


def reacted_state(step):
    sc, li, _ = state_delta(step)
    return bool(sc or li)


# ---------------------------------------------------------------- visual
_img_cache = {}


def grid(run, step):
    if not step or not step.get("shot"):
        return None
    p = os.path.join(run["dir"], step["shot"])
    if p in _img_cache:
        return _img_cache[p]
    try:
        import numpy as np
        from PIL import Image

        with Image.open(p) as im:
            g = np.asarray(im.convert("L"), dtype=np.int16)
    except Exception:  # noqa: BLE001
        g = None
    _img_cache[p] = g
    return g


def change_mask(run, name):
    names = list(run["steps"])
    i = names.index(name)
    if i == 0:
        return None
    a, b = grid(run, run["steps"][names[i - 1]]), grid(run, run["steps"][name])
    if a is None or b is None or a.shape != b.shape:
        return None
    gx, gy = GRID
    h, w = a.shape
    d = abs(b - a)[: h // gy * gy, : w // gx * gx] > PIXEL_THRESHOLD
    return d.reshape(gy, h // gy, gx, w // gx).mean(axis=(1, 3)) > CELL_FRACTION


def dilate(m):
    d = m.copy()
    d[1:, :] |= m[:-1, :]
    d[:-1, :] |= m[1:, :]
    d[:, 1:] |= m[:, :-1]
    d[:, :-1] |= m[:, 1:]
    return d


def compare_visual(ra, na, rb, nb):
    """(match: bool|None, text)."""
    ma, mb = change_mask(ra, na), change_mask(rb, nb)
    if ma is None or mb is None:
        return None, "no screenshot"
    # A changed cell on one side counts as matched when the other side changed
    # it or a neighbouring cell (UI text shifted by a few pixels, cursor).
    xor = int((ma & ~dilate(mb)).sum() + (mb & ~dilate(ma)).sum())
    union = int((ma | mb).sum())
    ok = xor <= max(VISUAL_MIN_CELLS, VISUAL_SHARE * union)
    return ok, f"changed cells {int(ma.sum())} vs {int(mb.sum())}, differing {xor} of {union} changed"


def reacted_visual(run, name):
    m = change_mask(run, name)
    return m is not None and int(m.sum()) >= VISUAL_MIN_CELLS


# ---------------------------------------------------------------- static scan
def handled_rpcs():
    """RPC ids samp-client has a match arm for (names from samp-proto)."""
    names = {}
    for f in glob.glob(os.path.join(ROOT, "crates/samp-proto/src/rpcs/**/*.rs"), recursive=True):
        s = open(f).read()
        for m in re.finditer(r"(?m)^\s*([A-Z][A-Za-z0-9]+)\s*=\s*(\d+)\s*[;{]", s):
            names.setdefault(m.group(1), int(m.group(2)))
        for m in re.finditer(r"impl RpcPayload for (\w+)\s*\{\s*const ID: u8 = (\d+);", s):
            names.setdefault(m.group(1), int(m.group(2)))
    src = ""
    for f in glob.glob(os.path.join(ROOT, "crates/samp-client/src/**/*.rs"), recursive=True):
        src += open(f).read()
    out = set()
    for name, rid in names.items():
        # Also nested arms such as `Incoming::Ui(ui::Incoming::ShowDialog(d)) =>` (ui/core.rs).
        if re.search(r"::" + name + r"\s*(\([^()]*\))?\s*(\{[^{}]*\})?\s*\)*\s*(=>|\|)", src):
            out.add(rid)
    if re.search(r"Event::InitGame", src):
        out.add(139)
    if re.search(r"ConnectionRejected", src):
        out.add(130)
    return out


# ---------------------------------------------------------------- comparison
def ordered_ok(stable, union_, ours, polls=frozenset()):
    """ours must contain every stable line, nothing outside `union_`, and the
    stable lines in the same relative order. Lines of poll templates
    (`polls`) are not counted as missing or extra."""
    so = LineSet(ours)
    missing = [ln for ln in stable if ln not in so and LineSet.key(ln)[0] not in polls]
    extra = [ln for ln in ours if ln not in union_ and LineSet.key(ln)[0] not in polls]
    # Order: the client's own calls (script commands, direct calls) among the
    # stable lines present in ours must be a subsequence of ours. `watch` diffs
    # and GTA-internal follow-ups (tasks) land at frame boundaries and are
    # compared as a set only.
    want = [ln for ln in stable if ln in so and ORDERED.search(ln)]
    p = 0
    for o in ours:
        if p < len(want) and want[p] in LineSet([o]):
            p += 1
    return missing, extra, p != len(want)


def compare_runs(orig_runs, ours):
    """Per-step verdicts."""
    reac_o = [reactions(r) for r in orig_runs]
    reac_a = reactions(ours) if ours else None
    polls = set().union(*(r["poll_templates"] for r in orig_runs))
    handled = handled_rpcs() if ours else set()
    o0 = orig_runs[0]
    steps = []
    for name, st in o0["steps"].items():
        row = {"idx": st["idx"], "name": name, "rpcs": st["rpcs"], "action": st.get("action")}
        fired = [r["steps"].get(name, {}).get("fired") for r in orig_runs]
        row["fired_orig"] = all(fired)
        # --- determinism among original runs
        lines = [ro.get(name, []) for ro in reac_o]
        sets = [LineSet(x) for x in lines]
        stable = [ln for ln in lines[0] if all(ln in x for x in sets[1:])]
        union_ = LineSet([ln for x in lines for ln in x])
        det = {"trace": [], "state": [], "visual": None}
        unstable_keys = set()
        for i in range(1, len(orig_runs)):
            a, b = lines[0], lines[i]
            only_a = [x for x in a if x not in sets[i] and LineSet.key(x)[0] not in polls]
            only_b = [x for x in b if x not in sets[0] and LineSet.key(x)[0] not in polls]
            _, _, order_bad = ordered_ok([x for x in a if x in sets[i]], sets[i], b, polls)
            if only_a or only_b or order_bad:
                det["trace"].append({"only_run1": only_a, f"only_run{i + 1}": only_b, "order": order_bad})
            sd = compare_state(st, orig_runs[i]["steps"].get(name, {}))
            det["state"] += sd
            unstable_keys |= {d.split(":")[0] for d in sd}
            vm, vt = compare_visual(o0, name, orig_runs[i], name)
            if vm is False:
                det["visual"] = vt
        row["determinism"] = det
        row["orig_stable"] = not det["trace"] and not det["state"] and det["visual"] is None
        row["orig_reaction"] = stable
        if not ours:
            steps.append(row)
            continue
        sa = ours["steps"].get(name)
        row["fired_ours"] = bool(sa and sa.get("fired"))
        if sa and sa.get("post", {}).get("alive") is False:
            row["verdict"] = "DIFF"
            row["why"] = "our client is not running after this step (crash/exit)"
            steps.append(row)
            continue
        if not row["fired_orig"] or not row["fired_ours"]:
            row["verdict"] = "NOT-FIRED"
            row["why"] = (st.get("error") or (sa or {}).get("error") or "step missing in a run")
            steps.append(row)
            continue
        ra = reac_a.get(name, [])
        missing, extra, order_bad = ordered_ok(stable, union_, ra, polls)
        trace_ok = not missing and not extra and not order_bad
        state_diff = compare_state(st, sa, ignore=unstable_keys, drift=True)
        vis_ok, vis_text = compare_visual(o0, name, ours, name)
        if det["visual"] is not None:
            # Unstable on the original side (timing of chat lines, animated
            # markers …): only the cells that changed in every original run
            # are judged; ours must change at least half of them.
            masks = [change_mask(r, name) for r in orig_runs]
            mu = change_mask(ours, name)
            vis_ok = None
            if mu is not None and all(m is not None for m in masks):
                core = masks[0].copy()
                for m in masks[1:]:
                    core &= m
                n = int(core.sum())
                if n >= VISUAL_MIN_CELLS and int((core & mu).sum()) < n / 2:
                    vis_ok = False
                    vis_text += f"; ours changes {int((core & mu).sum())} of the {n} cells every original run changes"
        row["trace"] = {"ok": trace_ok, "missing_in_ours": missing, "extra_in_ours": extra, "order_differs": order_bad}
        row["state"] = {"ok": not state_diff, "diff": state_diff}
        row["visual"] = {"ok": vis_ok, "detail": vis_text}
        row["ours_rx"] = rx_rpcs(sa)
        ours_reacted = bool(ra) or reacted_state(sa) or reacted_visual(ours, name)
        orig_reacted = bool(stable) or reacted_state(st) or reacted_visual(o0, name)
        real = [r for r in st["rpcs"] if r]
        no_handler = [r for r in real if r not in handled]
        row["orig_observable"] = orig_reacted
        # Nothing judged at all (no trace/state reaction, screenshot unstable)
        # while the original did change the picture: fall back to the static
        # handler check instead of calling it a MATCH.
        unjudged = vis_ok is None and not stable and not reacted_state(st) and reacted_visual(o0, name)
        if unjudged and real and len(no_handler) == len(real):
            row["verdict"] = "NOT-IMPLEMENTED"
            row["why"] = "no handler in samp-client for " + ",".join(map(str, no_handler)) + \
                " (the original's only reaction is an unstable screenshot change)"
        elif trace_ok and not state_diff and vis_ok is not False:
            row["verdict"] = "MATCH"
            if not orig_reacted:
                row["why"] = "no observable reaction in the original (trace, state, screenshot)"
        elif (real and len(no_handler) == len(real)) or (orig_reacted and not ours_reacted):
            row["verdict"] = "NOT-IMPLEMENTED"
            row["why"] = ("no handler in samp-client for " + ",".join(map(str, no_handler))) if no_handler \
                else "no reaction in ours"
        else:
            row["verdict"] = "DIFF"
        steps.append(row)
    return steps


def rx_rpcs(step):
    """RPC ids our client received during the step (from `net` before/after)."""
    def parse(s):
        m = re.search(r"last_rpcs=\[([0-9,]*)\].* rpcs=(\d+)", s or "")
        if not m:
            return None, None
        return [int(x) for x in m.group(1).split(",") if x], int(m.group(2))
    _, n0 = parse(step.get("pre", {}).get("net"))
    last, n1 = parse(step.get("post", {}).get("net"))
    if n0 is None or n1 is None:
        return None
    k = n1 - n0
    if k <= 0:
        return []
    return last[-k:] if k <= len(last) else last + ["…"]


RANK = {"MATCH": 0, "NOT-IMPLEMENTED": 2, "DIFF": 3, "NOT-FIRED": 4}


def per_rpc(steps, step_list, have_ours):
    """`step_list`: every step the filterscript defines (coverage), `steps`:
    the ones compared in this report."""
    out = OrderedDict()
    for rid in REGISTERED:
        defined = [s["name"] for s in step_list if rid in s["rpcs"]]
        cov = [s for s in steps if rid in s["rpcs"]]
        if not defined and not cov:
            out[rid] = {"verdict": "UNTRIGGERABLE", "why": UNTRIGGERABLE.get(rid, "no step"), "steps": []}
            continue
        if not cov:
            out[rid] = {"verdict": "NOT-RUN", "steps": defined}
            continue
        if not have_ours:
            out[rid] = {"verdict": "ORIG-ONLY", "steps": [s["name"] for s in cov]}
            continue
        worst = max(cov, key=lambda s: RANK.get(s["verdict"], 5))
        out[rid] = {"verdict": worst["verdict"], "steps": [s["name"] for s in cov]}
        if worst["verdict"] == "MATCH" and not any(s.get("orig_observable") for s in cov):
            out[rid]["note"] = "no observable reaction in the original"
    return out


# ---------------------------------------------------------------- output
def short(lst, n=6):
    lst = list(lst)
    s = "; ".join(f"`{x}`" for x in lst[:n])
    return s + (f" … (+{len(lst) - n})" if len(lst) > n else "")


def standing_diffs(orig, ours, name="idle_b"):
    """Whole-state differences between the clients at an idle step (no RPC
    pending): divergences that are not caused by one particular step."""
    a, b = orig["steps"].get(name), ours["steps"].get(name)
    if not a or not b:
        return []
    fa, fb = flat_state(a.get("post", {})), flat_state(b.get("post", {}))
    out = []
    for k in sorted(set(fa) | set(fb)):
        va, vb = fa.get(k), fb.get(k)
        if isinstance(va, list) or isinstance(vb, list):
            ca, cb = Counter(map(item_id, va or [])), Counter(map(item_id, vb or []))
            if ca != cb:
                out.append(f"{k}: only orig {sorted((ca - cb).elements())}, only ours {sorted((cb - ca).elements())}")
        elif va != vb:
            out.append(f"{k}: {va} vs {vb}")
    return out


def write_report(out_dir, orig_runs, ours, steps, rpcs):
    os.makedirs(out_dir, exist_ok=True)
    have_ours = ours is not None
    totals = Counter(v["verdict"] for v in rpcs.values())
    det_bad = [s for s in steps if not s["orig_stable"]]
    md = ["# DIFF-2 RPC coverage (criterion F1)", ""]
    md.append("Original runs: " + ", ".join(f"`{os.path.relpath(r['dir'], ROOT)}`" for r in orig_runs))
    md.append("Ours: " + (f"`{os.path.relpath(ours['dir'], ROOT)}`" if have_ours else "not run"))
    md.append("")
    md.append(f"Steps: {len(steps)}; RPC ids covered by a step: "
              f"{sum(1 for v in rpcs.values() if v['verdict'] != 'UNTRIGGERABLE')} of {len(REGISTERED)}.")
    md.append("")
    md.append("## Summary")
    md.append("")
    for k in ("MATCH", "DIFF", "NOT-IMPLEMENTED", "NOT-FIRED", "NOT-RUN", "ORIG-ONLY", "UNTRIGGERABLE"):
        if totals.get(k):
            md.append(f"* {k}: {totals[k]}")
    md.append("")
    if len(orig_runs) > 1:
        md.append("## Determinism of the original side")
        md.append("")
        md.append(f"{len(steps) - len(det_bad)} of {len(steps)} steps identical across {len(orig_runs)} original runs "
                  "(trace reaction, state reaction, screenshot change mask).")
        md.append(f"Per component: trace {sum(1 for s in steps if not s['determinism']['trace'])}, "
                  f"state {sum(1 for s in steps if not s['determinism']['state'])}, "
                  f"visual {sum(1 for s in steps if s['determinism']['visual'] is None)} of {len(steps)} identical.")
        md.append("Unstable parts are excluded from the original-vs-ours comparison (noise lines, state keys,"
                  " the screenshot check of that step).")
        md.append("")
        for s in det_bad:
            d = s["determinism"]
            parts = []
            for t in d["trace"]:
                parts.append("trace " + "; ".join(f"{k}: {short(v, 4)}" if isinstance(v, list) else f"{k}={v}"
                                                  for k, v in t.items() if v))
            if d["state"]:
                parts.append("state " + short(d["state"], 4))
            if d["visual"]:
                parts.append("visual " + d["visual"])
            md.append(f"* `{s['name']}`: " + " | ".join(parts))
        md.append("")
    standing = standing_diffs(orig_runs[0], ours) if have_ours else []
    if have_ours:
        md.append("## Standing state differences (at idle_b, original vs ours)")
        md.append("")
        md.append("Not attributed to single steps; a key listed here is only judged in steps where the original"
                  " changes it.")
        md.append("")
        md += [f"* `{d}`" for d in standing] or ["* none"]
        md.append("")
    md.append("## Per RPC")
    md.append("")
    md.append("| RPC | verdict | steps / reason |")
    md.append("|---|---|---|")
    for rid, v in rpcs.items():
        note = f" ({v['note']})" if v.get("note") else ""
        md.append(f"| {rid} | {v['verdict']} | {', '.join(v['steps']) or v.get('why', '')}{note} |")
    md.append("")
    md.append("## Per step")
    md.append("")
    for s in steps:
        head = f"### {s['idx']} `{s['name']}` (RPC {','.join(map(str, s['rpcs']))})"
        md.append(head + (f" — **{s['verdict']}**" if "verdict" in s else ""))
        if s.get("why"):
            md.append(f"* why: {s['why']}")
        md.append(f"* original reaction ({len(s['orig_reaction'])} lines): {short(s['orig_reaction'])}")
        if "trace" in s:
            t = s["trace"]
            if not t["ok"]:
                if t["missing_in_ours"]:
                    md.append(f"* trace, missing in ours: {short(t['missing_in_ours'])}")
                if t["extra_in_ours"]:
                    md.append(f"* trace, extra in ours: {short(t['extra_in_ours'])}")
                if t["order_differs"]:
                    md.append("* trace: order differs")
            if s["state"]["diff"]:
                md.append(f"* state (orig vs ours): {short(s['state']['diff'])}")
            vok = s["visual"]["ok"]
            vtext = "ok" if vok else "not judged" if vok is None else "DIFF"
            md.append(f"* visual: {vtext} ({s['visual']['detail']})")
            if s.get("ours_rx") is not None:
                md.append(f"* RPCs received by ours: {s['ours_rx']}")
        md.append("")
    with open(os.path.join(out_dir, "rpc-coverage.md"), "w") as f:
        f.write("\n".join(md) + "\n")
    with open(os.path.join(out_dir, "rpc-coverage.json"), "w") as f:
        json.dump({
            "orig_runs": [r["dir"] for r in orig_runs], "ours_run": ours["dir"] if have_ours else None,
            "totals": totals, "determinism_unstable_steps": [s["name"] for s in det_bad],
            "standing_state_differences": standing,
            "rpcs": {str(k): v for k, v in rpcs.items()}, "steps": steps,
        }, f, indent=1, default=str)
    return totals, det_bad


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--orig", action="append", required=True)
    ap.add_argument("--ours")
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "diff"))
    a = ap.parse_args()
    orig_runs = [load_run(d) for d in a.orig]
    ours = load_run(a.ours) if a.ours else None
    steps = compare_runs(orig_runs, ours)
    rpcs = per_rpc(steps, orig_runs[0]["list"], ours is not None)
    totals, det_bad = write_report(a.out, orig_runs, ours, steps, rpcs)
    covered = sum(1 for v in rpcs.values() if v["verdict"] != "UNTRIGGERABLE")
    print(f"[diff] steps {len(steps)}, RPCs covered {covered}/{len(REGISTERED)}")
    if len(orig_runs) > 1:
        comp = (f" (trace {sum(1 for s in steps if not s['determinism']['trace'])}, "
                f"state {sum(1 for s in steps if not s['determinism']['state'])}, "
                f"visual {sum(1 for s in steps if s['determinism']['visual'] is None)})")
        print(f"[diff] original determinism: {len(steps) - len(det_bad)}/{len(steps)} steps identical{comp}"
              + (f"; unstable: {', '.join(s['name'] for s in det_bad)}" if det_bad else ""))
    print("[diff] per RPC: " + ", ".join(f"{k} {v}" for k, v in sorted(totals.items())))
    print(f"[diff] report: {os.path.join(a.out, 'rpc-coverage.md')}")
    if ours is None:
        return 1
    bad = sum(v for k, v in totals.items() if k in ("DIFF", "NOT-IMPLEMENTED", "NOT-FIRED", "NOT-RUN"))
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
