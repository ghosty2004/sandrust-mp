#!/usr/bin/env python3
"""Aligned diff of two samp-tracer traces (TRACE-1).

    tools/trace/diff.py LEFT RIGHT [options]

LEFT/RIGHT: a trace.jsonl or a run directory (target/trace/orig-latest, …).
Each record is normalised to one line: function name + key arguments; pointers,
timing, sequence/frame numbers and caller offsets are dropped (the caller is
reduced to gta/client), floats are rounded. Consecutive identical lines are
collapsed ("×N"). Script opcodes are annotated with names from
re/gta-reversed/source/game_sa/Enums/eScriptCommands.h when present.

Options:
  --from MARK / --to MARK   only the part between two `mark` records (prefix match)
  --only REGEX              keep only lines matching REGEX
  --skip REGEX              drop lines matching REGEX
  --globals                 keep `globals` records (dropped by default)
  --stock                   keep calls made inside stock scripts (dropped by default)
  --digits N                float rounding (default 1)
  --context N               unchanged lines shown around changes (default 3; -1 = all)
  --dump                    print the normalised LEFT only (no diff)
"""
import argparse
import difflib
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
ENUM = os.path.join(ROOT, "re/gta-reversed/source/game_sa/Enums/eScriptCommands.h")
DROP_FNS = {"pos", "globals_base", "suppressed", "tracer_start", "module", "quit", "first_frame"}
DROP_WATCH = {"script_ips", "stock_script_cmds"}


def opcode_names():
    names = {}
    try:
        src = open(ENUM).read()
    except OSError:
        return names
    idx = 0
    for name, val in re.findall(r"^\s*(COMMAND_[A-Z0-9_]+)\s*(?:=\s*(0x[0-9A-Fa-f]+|\d+))?\s*,", src, re.M):
        if val:
            idx = int(val, 0)
        names[idx] = name[len("COMMAND_"):]
        idx += 1
    return names


OPS = opcode_names()


def op_label(op):
    try:
        n = OPS.get(int(op, 16))
    except (TypeError, ValueError):
        n = None
    return f"{op} {n}" if n else str(op)


def norm_val(v, digits):
    if isinstance(v, float):
        r = round(v, digits)
        return 0.0 if r == 0 else r
    if isinstance(v, dict):
        return {k: norm_val(x, digits) for k, x in v.items() if not k.endswith("ptr")}
    if isinstance(v, list):
        return [norm_val(x, digits) for x in v]
    return v


def fmt(v):
    if isinstance(v, dict):
        return "{" + ",".join(f"{k}={fmt(x)}" for k, x in v.items()) + "}"
    if isinstance(v, list):
        return "[" + ",".join(fmt(x) for x in v) + "]"
    if isinstance(v, str) and re.fullmatch(r"0x[0-9a-fA-F]+", v):
        return "<ptr>"
    return str(v)


def caller_class(c):
    if not c:
        return ""
    m = c.split("+")[0].lower()
    if m in ("samp.dll", "samp_client.dll"):
        return "client"
    if m == "gta_sa.exe":
        return "gta"
    return "other"


def via_label(v):
    if not v:
        return ""
    m = re.match(r"(scm|stock \S+) (!?)(0x[0-9a-f]+)$", v)
    if m:
        return f"{m.group(1)} {m.group(2)}{op_label(m.group(3))}"
    return v


def normalise(rec, opts):
    fn = rec.get("fn", "?")
    a = rec.get("args", {}) or {}
    if fn in DROP_FNS:
        return None
    if fn == "mark":
        return f"== mark {a.get('text', '')}"
    if fn == "globals" and not opts.globals:
        return None
    via = rec.get("via", "")
    if via.startswith("stock") and not opts.stock:
        return None
    if fn == "watch":
        a = {k: v for k, v in a.items() if k not in DROP_WATCH}
        if not a:
            return None
        return "watch " + " ".join(f"{k}={v}" for k, v in a.items())
    if fn == "scm":
        params = fmt(norm_val(a.get("params", []), opts.digits))
        out = f" out={fmt(a['out'])}" if "out" in a else ""
        neg = "!" if a.get("not") else ""
        return f"scm {neg}{op_label(a.get('op'))} {params}{out}"
    if fn == "stock_cmd":
        return f"stock_cmd {a.get('script')} {op_label(a.get('op'))}"
    line = f"{fn} {fmt(norm_val(a, opts.digits))}"
    cc = caller_class(rec.get("caller"))
    if cc:
        line += f"  <{cc}>"
    vl = via_label(via)
    if vl:
        line += f"  [via {vl}]"
    return line


def load(path, opts):
    if os.path.isdir(path):
        path = os.path.join(path, "trace.jsonl")
    lines, frames = [], []
    active = opts.from_mark is None
    with open(path) as f:
        for raw in f:
            try:
                rec = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if rec.get("fn") == "mark":
                text = rec.get("args", {}).get("text", "")
                if opts.from_mark is not None and text.startswith(opts.from_mark):
                    active = True
                elif opts.to_mark is not None and active and text.startswith(opts.to_mark):
                    lines.append(f"== mark {text}")
                    frames.append(rec.get("frame", 0))
                    break
            if not active:
                continue
            n = normalise(rec, opts)
            if n is None:
                continue
            if opts.only and not re.search(opts.only, n):
                continue
            if opts.skip and re.search(opts.skip, n):
                continue
            lines.append(n)
            frames.append(rec.get("frame", 0))
    # Collapse frames whose whole block equals the previous frame's block
    # (per-frame polling), then runs of identical lines.
    blocks = []
    for ln, f in zip(lines, frames):
        if blocks and blocks[-1][0] == f and not ln.startswith("== mark"):
            blocks[-1][1].append(ln)
        else:
            blocks.append([f, [ln], 1])
    merged = []
    for f, blk, _ in blocks:
        if merged and merged[-1][1] == blk and not blk[0].startswith("== mark"):
            merged[-1][2] += 1
        else:
            merged.append([f, blk, 1])
    lines, frames = [], []
    for f, blk, n in merged:
        lines += blk
        frames += [f] * len(blk)
        if n > 1:
            lines.append(f"   (frame block above repeated {n - 1} more times)")
            frames.append(f)
    out, fr = [], []
    for ln, f in zip(lines, frames):
        if out and re.sub(r"  ×\d+$", "", out[-1]) == ln:
            m = re.search(r"  ×(\d+)$", out[-1])
            n = int(m.group(1)) + 1 if m else 2
            out[-1] = f"{ln}  ×{n}"
        else:
            out.append(ln)
            fr.append(f)
    return out, fr


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("left")
    ap.add_argument("right", nargs="?")
    ap.add_argument("--from", dest="from_mark")
    ap.add_argument("--to", dest="to_mark")
    ap.add_argument("--only")
    ap.add_argument("--skip")
    ap.add_argument("--globals", action="store_true")
    ap.add_argument("--stock", action="store_true")
    ap.add_argument("--digits", type=int, default=1)
    ap.add_argument("--context", type=int, default=3)
    ap.add_argument("--dump", action="store_true")
    opts = ap.parse_args()

    left, lf = load(opts.left, opts)
    if opts.dump or not opts.right:
        try:
            for ln, f in zip(left, lf):
                print(f"{f:>6}  {ln}")
        except BrokenPipeError:
            pass
        return 0
    right, rf = load(opts.right, opts)
    # Compare without the repeat counts so "×3" vs "×5" still aligns (shown as a change).
    key = lambda l: re.sub(r"  ×\d+$", "", l)  # noqa: E731
    sm = difflib.SequenceMatcher(a=[key(x) for x in left], b=[key(x) for x in right], autojunk=False)
    changes = 0
    print(f"--- {opts.left} ({len(left)} lines)\n+++ {opts.right} ({len(right)} lines)")
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            n = i2 - i1
            ctx = opts.context
            if ctx < 0 or n <= 2 * ctx:
                for k in range(n):
                    print(f"  {lf[i1 + k]:>6} {rf[j1 + k]:>6}  {left[i1 + k]}")
            else:
                for k in range(ctx):
                    print(f"  {lf[i1 + k]:>6} {rf[j1 + k]:>6}  {left[i1 + k]}")
                print(f"  {'':>6} {'':>6}  … {n - 2 * ctx} equal lines …")
                for k in range(n - ctx, n):
                    print(f"  {lf[i1 + k]:>6} {rf[j1 + k]:>6}  {left[i1 + k]}")
            continue
        changes += 1
        for k in range(i1, i2):
            print(f"- {lf[k]:>6} {'':>6}  {left[k]}")
        for k in range(j1, j2):
            print(f"+ {'':>6} {rf[k]:>6}  {right[k]}")
    print(f"# {changes} differing blocks")
    return 1 if changes else 0


if __name__ == "__main__":
    sys.exit(main())
