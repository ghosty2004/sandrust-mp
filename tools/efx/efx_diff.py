#!/usr/bin/env python3
"""EFX-1: effect logs of samp-tracer's effect tracer (efx.jsonl segments).

  efx_diff.py dump LOG                 normalised effects of one capture
  efx_diff.py spec LOG                 window spec (mem/fn lines) from a step capture
  efx_diff.py diff LEFT RIGHT [--all]  aligned diff of two captures (LEFT = original)

Normalisation: game objects are already symbolic in the log (player_ped+0x540, gta:0xb7cd98);
pool indexes of non-player objects (task#17) and raw heap addresses become `#?` / `heap`;
4-byte values that look like pointers become `ptr` (or the object they point to); timing is
dropped. GTA function and global names are read from re/gta-reversed if present (named via
gta-reversed `RH_Scoped*Install` / `StaticRef` lines).
"""
import difflib
import json
import os
import re
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def lane_roots():
    yield ROOT
    if os.environ.get("EFX_LANE_ROOT"):
        yield os.environ["EFX_LANE_ROOT"]
    yield os.path.join(os.path.expanduser("~"), "samp-rs")


_NAMES = None


def names():
    """(functions {addr: name}, globals sorted [(addr, name)]) from gta-reversed."""
    global _NAMES
    if _NAMES is not None:
        return _NAMES
    fns, gl = {}, {}
    src = next((os.path.join(r, "re/gta-reversed/source") for r in lane_roots()
                if os.path.isdir(os.path.join(r, "re/gta-reversed/source"))), None)
    if src:
        cls_re = re.compile(r"RH_Scoped(?:Virtual)?(?:Class|Namespace)\((\w+)")
        named_cls_re = re.compile(r'RH_ScopedNamedClass\((\w+),\s*"(\w+)"\)')
        inst_re = re.compile(r"RH_Scoped(?:Virtual|VMT|Overloaded|Global|)Install\((\w+),\s*(?:\"[^\"]*\",\s*)?(0x[0-9A-Fa-f]+)")
        sref_re = re.compile(r"(\w[\w:]*)\s*=\s*StaticRef<[^>]*?(?:,\s*(0x[0-9A-Fa-f]+))?>\s*\(\s*(0x[0-9A-Fa-f]+)?")
        for dp, _, fs in os.walk(src):
            for f in fs:
                if not f.endswith((".cpp", ".h")):
                    continue
                try:
                    text = open(os.path.join(dp, f), errors="replace").read()
                except OSError:
                    continue
                cls = None
                for line in text.splitlines():
                    m = named_cls_re.search(line) or cls_re.search(line)
                    if m:
                        cls = m.group(1)
                        continue
                    m = inst_re.search(line)
                    if m:
                        a = int(m.group(2), 16)
                        n = m.group(1)
                        fns.setdefault(a, f"{cls}::{n}" if cls and "Global" not in line else n)
                        continue
                    m = sref_re.search(line)
                    if m and (m.group(2) or m.group(3)):
                        gl.setdefault(int(m.group(2) or m.group(3), 16), m.group(1))
    _NAMES = (fns, sorted(gl.items()))
    return _NAMES


def fn_name(addr):
    return names()[0].get(addr, f"sub_{addr:x}")


_SCM = None


def scm_info():
    """({handler address}, {opcode: name}): the 27 command-handler pointers at 0x8A6168 in
    re/gta/1.0-US/gta_sa.exe (CRunningScript::ProcessCommandsNNNToMMM) and opcode names from
    gta-reversed eScriptCommands.h."""
    global _SCM
    if _SCM is not None:
        return _SCM
    handlers, ops = set(), {}
    for r in lane_roots():
        exe = os.path.join(r, "re/gta/1.0-US/gta_sa.exe")
        if not os.path.exists(exe):
            continue
        d = open(exe, "rb").read()
        e = struct.unpack_from("<I", d, 0x3C)[0]
        nsec = struct.unpack_from("<H", d, e + 6)[0]
        opt = struct.unpack_from("<H", d, e + 20)[0]
        base = struct.unpack_from("<I", d, e + 52)[0]
        for i in range(nsec):
            sh = e + 24 + opt + 40 * i
            va, raw_sz, raw = struct.unpack_from("<I", d, sh + 12)[0], struct.unpack_from("<I", d, sh + 16)[0], struct.unpack_from("<I", d, sh + 20)[0]
            rva = 0x8A6168 - base
            if va <= rva < va + raw_sz:
                handlers = set(struct.unpack_from("<27I", d, raw + rva - va))
        break
    for r in lane_roots():
        enum = os.path.join(r, "re/gta-reversed/source/game_sa/Enums/eScriptCommands.h")
        if os.path.exists(enum):
            idx = 0
            for n, v in re.findall(r"^\s*(COMMAND_[A-Z0-9_]+)\s*(?:=\s*(0x[0-9A-Fa-f]+|\d+))?\s*,", open(enum).read(), re.M):
                if v:
                    idx = int(v, 0)
                ops[idx] = n[len("COMMAND_"):]
                idx += 1
            break
    _SCM = (handlers, ops)
    return _SCM


_FN_SORTED = None

def containing_fn(addr):
    """Name of the known function whose start is the nearest one at or below addr."""
    global _FN_SORTED
    if _FN_SORTED is None:
        _FN_SORTED = sorted(names()[0].items())
    lo, hi = 0, len(_FN_SORTED)
    while lo < hi:
        mid = (lo + hi) // 2
        if _FN_SORTED[mid][0] <= addr:
            lo = mid + 1
        else:
            hi = mid
    return _FN_SORTED[lo - 1][1] if lo else None


# Game functions both clients call from their frame hooks to run the normal game frame: window
# effects whose outermost game frame lies in them are game processing, not the RPC's effect.
FRAME_FNS = ("CGame::Process", "Render2dStuff", "Idle", "CPlayerPed::ProcessControl")
CLIENT_MODULES = ("samp.dll", "samp_client.dll")


def background(r, base):
    """Window record that is not an effect of the fired RPC: seen in the baseline window, run
    by the game frame the client hook drives, or with no client frame on the stack at all."""
    if r.get("fn") == "efx_write" and base is not None and ("w", norm_sym(r.get("sym")), r.get("at"), r.get("via")) in base:
        return True
    if r.get("fn") == "efx_call" and base is not None and ("c", r.get("to"), r.get("via")) in base:
        return True
    if r.get("by") == "client":
        return False
    if not r.get("via"):
        return True
    o = r.get("gta_outer")
    return bool(o) and containing_fn(int(o, 16)) in FRAME_FNS


def base_keys(recs):
    """Keys of what a baseline window (same spec, nothing fired) saw: writes (sym, writer)
    and calls (function, via)."""
    keys = set()
    for r in recs:
        if r.get("fn") == "efx_write":
            keys.add(("w", norm_sym(r.get("sym")), r.get("at"), r.get("via")))
        elif r.get("fn") == "efx_call":
            keys.add(("c", r.get("to"), r.get("via")))
    return keys


def base_path(path):
    p = path[: -len(".jsonl")] + "-base.jsonl"
    return p if os.path.exists(p) else None


def global_name(addr):
    gl = names()[1]
    lo, hi = 0, len(gl)
    while lo < hi:
        mid = (lo + hi) // 2
        if gl[mid][0] <= addr:
            lo = mid + 1
        else:
            hi = mid
    if lo and addr - gl[lo - 1][0] < 0x1000:
        a, n = gl[lo - 1]
        return n if a == addr else f"{n}+{addr - a:#x}"
    return None


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


def norm_sym(s):
    if s is None:
        return "?"
    if s.startswith("heap:"):
        return "heap"
    s = re.sub(r"^(task|event|obj|ped|veh|intel|taskalloc|pointroute|noderoute|matrix)#\d+", r"\1#?", s)
    m = re.match(r"gta:(0x[0-9a-f]+)$", s)
    if m:
        g = global_name(int(m.group(1), 16))
        if g:
            return f"{s} ({g})"
    return s


def objects_of(recs):
    """{address: name} of the objects the capture listed in efx_begin."""
    objs = {}
    for r in recs:
        if r.get("fn") == "efx_begin":
            for o in r.get("objects", []):
                n, _, a = o.partition("=")
                objs[int(a, 16)] = n
    return objs


def fmt_val(hexs, objs):
    """Readable, comparable rendering of written bytes."""
    if hexs is None:
        return "?"
    b = bytes.fromhex(hexs)
    if len(b) == 4:
        v = struct.unpack("<I", b)[0]
        f = struct.unpack("<f", b)[0]
        if v in objs:
            return f"&{objs[v]}"
        if v == 0:
            return "0"
        if 0.0001 <= abs(f) <= 1e7:
            return f"{f:.4g}f"
        if v < 0x10000 or v > 0xFFFF0000:
            return str(v if v < 0x80000000 else v - (1 << 32))
        if 0x400000 <= v < 0x1577000:
            return f"gta:{v:#x}"
        return "ptr"
    if len(b) in (1, 2):
        return str(int.from_bytes(b, "little"))
    if len(b) == 12:
        return "(" + ",".join(f"{x:.4g}" for x in struct.unpack("<3f", b)) + ")"
    return hexs if len(hexs) <= 32 else hexs[:32] + "…"


def rename_target_car(recs, objs):
    """The car the player ends up in (player_ped+0x58c, CPed::m_pVehicle) is the RPC's target:
    name its pool slot like the tracer's `near_veh` so both clients' captures line up."""
    car = None
    for r in recs:
        if r.get("fn") == "efx_write" and r.get("sym") == "player_ped+0x58c" and r.get("new"):
            car = struct.unpack("<I", bytes.fromhex(r["new"]))[0] or car
    slot = None
    if car:
        for r in recs:
            m = re.match(r"(veh#\d+)\+(0x[0-9a-f]+)$", r.get("sym") or "")
            if m and r.get("addr") and int(r["addr"], 16) - int(m.group(2), 16) == car:
                slot = m.group(1)
                break
        if slot:
            objs.setdefault(car, "near_veh")
            for r in recs:
                for k in ("sym", "this", "eax_sym"):
                    if isinstance(r.get(k), str) and r[k].startswith(slot + "+"):
                        r[k] = "near_veh" + r[k][len(slot):]
                if isinstance(r.get("args_sym"), list):
                    r["args_sym"] = [("near_veh" + x[len(slot):]) if isinstance(x, str) and x.startswith(slot + "+") else x
                                     for x in r["args_sym"]]


def effects(recs, base=None):
    """Effects of one capture: (writes {sym: [old, new, n, at, by]}, calls [str], meta).
    Window captures: records whose key also occurs in the baseline window `base` (or, without
    a baseline, that no client frame is on the stack for) are background and dropped."""
    objs = objects_of(recs)
    rename_target_car(recs, objs)
    writes, calls, meta = {}, [], {}
    ids = {}
    window = any(r.get("mode") == "window" and r.get("fn") == "efx_begin" for r in recs)
    for r in recs:
        k = r.get("fn")
        if k == "efx_begin":
            meta["mode"] = r.get("mode")
            meta["target"] = r.get("target")
            if r.get("pages"):
                meta["pages"] = {int(x, 16) for x in r["pages"]}
                meta["objects"] = {n: a for a, n in objs.items()}
        elif k == "efx_end":
            meta.update({x: r.get(x) for x in ("reason", "steps", "ms", "faults", "writes", "frames", "truncated", "stepped_over")})
        elif k == "efx_write":
            if window and background(r, base):
                meta["background"] = meta.get("background", 0) + 1
                continue
            s = norm_sym(r.get("sym"))
            e = writes.get(s)
            if e is None:
                writes[s] = [fmt_val(r.get("old"), objs), fmt_val(r.get("new"), objs), 1, r.get("at"), r.get("by")]
            else:
                e[1] = fmt_val(r.get("new"), objs)
                e[2] += 1
        elif k == "efx_branch":
            meta.setdefault("branch_reads", []).append(
                f"{norm_sym(r.get('sym'))}={fmt_val(r.get('val'), objs)} {'taken' if r.get('taken') else 'not taken'} at {r.get('at')}")
        elif k == "efx_net":
            s = norm_sym(r.get("sym"))
            writes.setdefault(s, [fmt_val(r.get("old"), objs), fmt_val(r.get("new"), objs), 0, "net", "net"])
        elif k == "efx_call":
            if window and background(r, base):
                continue
            to = int(r["to"], 16)
            d = r.get("d") or (1 if r.get("from", "").startswith(CLIENT_MODULES) else 2)
            args = []
            for a, s in zip(r.get("args", []), r.get("args_sym", [])):
                args.append(norm_sym(s) if s else fmt_val(struct.pack("<I", int(a, 16)).hex(), objs))
            this = r.get("this")
            ids[r.get("id")] = len(calls)
            handlers, ops = scm_info()
            if to in handlers and r.get("args"):
                op = int(r["args"][0], 16) & 0x7FFF
                calls.append([d, f"scm {op:#06x} {ops.get(op, '?')}", ""])
                continue
            if fn_name(to) == "CRunningScript::ProcessOneCommand":
                # thiscall without arguments (the stack words are the caller's locals).
                calls.append([d, "CRunningScript::ProcessOneCommand", ""])
                continue
            calls.append([d, f"{fn_name(to)}" + (f" this={norm_sym(this)}" if this else "") + "(" + ", ".join(args[:4]) + ")", ""])
        elif k == "efx_ret" and r.get("id") in ids:
            ret = norm_sym(r["eax_sym"]) if r.get("eax_sym") else fmt_val(struct.pack("<I", int(r["eax"], 16)).hex(), objs)
            calls[ids[r["id"]]][2] = f" -> {ret}" + (f" st0={r['st0']}" if "st0" in r else "")
    return writes, calls, meta


def effects_of(path):
    b = base_path(path)
    return effects(load(path), base_keys(load(b)) if b else None)


def cmd_dump(path, all_depths):
    w, c, meta = effects_of(path)
    print(f"# {path}: { {k: v for k, v in meta.items() if k != 'branch_reads'} }")
    print("## writes (sym: old -> new  [n] at by)")
    for s in sorted(w):
        o, n, cnt, at, by = w[s]
        where = ""
        if at and at.startswith("gta_sa.exe+") and int(at.split("+")[1], 16) < 0x457000:
            where = f" ({containing_fn(0x400000 + int(at.split('+')[1], 16))}?)"
        print(f"  {s}: {o} -> {n}  [{cnt}] {at}{where} {by}")
    for b in meta.get("branch_reads", []):
        print(f"## branch read: {b}")
    print("## game calls")
    for d, sig, ret in c:
        if all_depths or d <= 2:
            print(f"  {'  ' * (d - 1)}{sig}{ret}")


def cmd_spec(path):
    recs = load(path)
    rename_target_car(recs, objects_of(recs))
    syms, fns = {}, {}
    for r in recs:
        if r.get("fn") == "efx_write":
            s = r.get("sym", "")
            if s.startswith(("player_", "near_veh", "gta:")):
                syms[s] = max(syms.get(s, 0), min(int(r.get("size", 4)), 64))
        elif r.get("fn") == "efx_call" and r.get("d", 1) <= 2:
            a = int(r["to"], 16)
            if 0x401000 <= a < 0x857000:
                fns[a] = fns.get(a, 0) + 1
    print(f"# window spec from {os.path.basename(path)}")
    for s in sorted(syms):
        print(f"mem {s} {syms[s]}")
    for a in sorted(fns):
        print(f"fn {a:#x} {fn_name(a)}")


def collapse(lines):
    """Consecutive identical lines → one line with ×N."""
    out = []
    for line in lines:
        if out and out[-1][0] == line:
            out[-1][1] += 1
        else:
            out.append([line, 1])
    return [l if n == 1 else f"{l}  x{n}" for l, n in out]


def observable(sym, meta):
    """Whether a window capture (meta of its efx_begin) watched the page of this symbol."""
    pages = meta.get("pages")
    if pages is None:
        return True
    m = re.match(r"(gta:)?(0x[0-9a-f]+)", sym)
    if m and m.group(1):
        addr = int(m.group(2), 16)
    else:
        m = re.match(r"([a-z_.]+)\+(0x[0-9a-f]+)$", sym)
        if not m or m.group(1) not in meta.get("objects", {}):
            return False
        addr = meta["objects"][m.group(1)] + int(m.group(2), 16)
    return addr & ~0xFFF in pages


def cmd_diff(left, right, all_depths):
    lw, lc, lm = effects_of(left)
    rw, rc, rm = effects_of(right)
    for tag, path, m in (("LEFT ", left, lm), ("RIGHT", right, rm)):
        print(f"# {tag} {path}: { {k: v for k, v in m.items() if k not in ('branch_reads', 'pages', 'objects') and v is not None} }")
        for b in m.get("branch_reads", []):
            print(f"#   branch read: {b}")
    print("## writes: sym | left old -> new [n] | right old -> new [n] | verdict")
    same = unseen = 0
    for s in sorted(set(lw) | set(rw)):
        a, b = lw.get(s), rw.get(s)
        if a and not b and not observable(s, rm):
            unseen += 1
            if not all_depths:
                continue
        if a and b:
            v = "same" if a[1] == b[1] else "DIFFERENT VALUE"
        elif a:
            v = "MISSING in right"
        else:
            v = "EXTRA in right"
        if v == "same":
            same += 1
            if not all_depths:
                continue
        fa = f"{a[0]} -> {a[1]} [{a[2]}]" if a else "-"
        fb = f"{b[0]} -> {b[1]} [{b[2]}]" if b else "-"
        print(f"  {s} | {fa} | {fb} | {v}")
    print(f"  ({same} written addresses end with the same value; {unseen} left-only writes are outside the right"
          f" window's watched pages{'' if all_depths else '; --all lists both'})")
    print("## game calls (depth <= 2 unless --all; ' ' both, '-' left only, '+' right only)")
    sel = lambda c: collapse([f"{'  ' * (d - 1)}{sig}" for d, sig, _ in c if all_depths or d <= 2])
    lines = list(difflib.unified_diff(sel(lc), sel(rc), "left", "right", n=10000, lineterm=""))[3:]
    if not lines:
        lines = [" " + x for x in sel(lc)] + (["  (identical)"] if lc else ["  (none)"])
    for line in lines:
        print("  " + line)


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    all_depths = "--all" in argv
    args = [a for a in argv[1:] if not a.startswith("--")]
    if args[0] == "dump":
        cmd_dump(args[1], all_depths)
    elif args[0] == "spec":
        cmd_spec(args[1])
    elif args[0] == "diff" and len(args) >= 3:
        cmd_diff(args[1], args[2], all_depths)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
