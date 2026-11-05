#!/usr/bin/env python3
"""F5 patch coverage: every row of gta-patches-r4-annotated.csv against what our
client applies, from LIVE bytes (tools/diff/patch-coverage.sh).

    patch_report.py --run DIR [--out target/diff] [--justify tools/diff/patch-justifications.tsv]

DIR holds patch-bytes-A.json (original client, read through the tracer `peek`)
and patch-bytes-B.json (our client, its own `peek`), dumped in the same
scenario after both joined, spawned and stood still (tools/diff/xclient.py).
The unmodified bytes come from re/gta/1.0-US/gta_sa.exe.

Per region (address, size) of a row, with o = original live, u = ours live,
e = unmodified exe (each side is read 5 times ~0.3 s apart; a region whose
value changes between snapshots — samp's nop/restore toggles — must show only
values the original showed, at least one of them applied):
  o != e (samp applied it):  u == o -> match;  u equivalent to o -> match
                             (pointer / call / jmp both leaving gta_sa.exe);
                             u == e -> missing;  else -> different
  o == e (not applied at this point: runtime/temporary/conditional/dead):
                             u == e -> inactive;  else -> different
Per row: justified (tools/diff/patch-justifications.tsv) wins; else different >
missing > match; a row whose regions are all inactive or that cannot be read
live (no plain address, e.g. "(707 sites)") is judged by the static table of
samp-client (`csv_row` entries in patches.rs/sectors.rs/stores.rs, "CSV row N"
comments elsewhere): referenced -> applied (static only), otherwise missing.
Owning milestone of missing rows: the "CSV rows Mx must apply" tables of
docs/phase3/re10-gaps.md.

Frame-local rows (patchrows.FRAME_LOCAL_ROWS: samp writes the data right before a
hooked GTA call and restores it right after) are judged from the writes themselves,
not from snapshots: DIR/{A,B}-efx.jsonl hold one samp-tracer `efx watch` window per
side (guard pages on the rows' pages for >= 60 game frames; every write with its
value and game frame), DIR/frame-watch-{A,B}.json the window's start/close. Per
region: the original wrote, ours did not -> missing; ours wrote, the original did
not -> different; both wrote -> match only if the written values ((address, bytes)
set), the per-frame pattern (set of writes in a writing frame, most common one),
the share of frames with writes (within 0.25) and the end-of-frame restore are the
same; neither wrote -> the snapshot rule below. A side whose window failed, stopped
on its fault budget or did not guard the region's page -> unverified (FAIL).
Exit 0 only when nothing is missing or different.
"""
import argparse
import glob
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import patchrows  # noqa: E402

ROOT = patchrows.ROOT
CLIENT_SRC = os.path.join(ROOT, "crates", "samp-client", "src")
TRACER_SRC = os.path.join(ROOT, "crates", "samp-tracer", "src")
RE10 = os.path.join(ROOT, "docs", "phase3", "re10-gaps.md")
JUSTIFY = os.path.join(HERE, "patch-justifications.tsv")


def static_rows():
    """{row: [where]} referenced by samp-client source."""
    refs = {}

    def add(n, where):
        refs.setdefault(int(n), []).append(where)

    # Recursive: ui/ (input lock rows 349–353) and other submodules apply rows too.
    for path in sorted(glob.glob(os.path.join(CLIENT_SRC, "**", "*.rs"), recursive=True)):
        name = os.path.relpath(path, CLIENT_SRC)
        src = open(path).read()
        for m in re.finditer(r"csv_row:\s*(\d+)", src):
            add(m.group(1), f"{name} csv_row")
        if name == "sectors.rs":
            for m in re.finditer(r"\br\(0x[0-9a-fA-F]+,\s*0x[0-9a-fA-F]+,\s*\w+,\s*(\d+)\)", src):
                add(m.group(1), "sectors.rs SECTOR_REFS")
            for m in re.finditer(r"ZERO_FILL_CSV_ROW: u32 = (\d+)", src):
                add(m.group(1), "sectors.rs zero fill")
            continue  # its doc comments name whole ranges incl. rows it does not apply
        if name == "stores.rs":
            for m in re.finditer(r"\(0x[0-9a-fA-F]+,\s*(\d+)\)", src):
                add(m.group(1), "stores.rs code refs")
        if name == "patches.rs":
            continue  # table entries only (comments repeat them)
        # Installers that return (row, result) tuples (worldhooks.rs).
        for m in re.finditer(r"out\.push\(\(\s*(\d+)\s*,", src):
            add(m.group(1), f"{name} install")
        pats = [r"CSV rows? ([0-9][0-9 ,/–\-+and]*)",
                # comment lines that start with "Row N" / "rows N/M + K" (worldhooks.rs, ui/inputlock.rs)
                r"(?m)^\s*//[/!]?\s*(?:-+\s*)?[Rr]ows? ([0-9][0-9 ,/–\-+and]*)"]
        for m in (m for pat in pats for m in re.finditer(pat, src)):
            for part in re.split(r",|/|\+| and ", m.group(1)):
                part = part.strip()
                r = re.fullmatch(r"(\d+)\s*[–-]\s*(\d+)", part)
                if r:
                    for n in range(int(r.group(1)), int(r.group(2)) + 1):
                        add(n, f"{name} comment")
                elif part.isdigit():
                    add(part, f"{name} comment")
    return {k: sorted(set(v)) for k, v in refs.items()}


_CLIENT_SRC_TEXT = None


def addr_referenced(row):
    """True when every region address of a runtime row appears as a hex constant in samp-client."""
    global _CLIENT_SRC_TEXT
    if _CLIENT_SRC_TEXT is None:
        _CLIENT_SRC_TEXT = "".join(open(p).read() for p in glob.glob(os.path.join(CLIENT_SRC, "**", "*.rs"),
                                                                      recursive=True)).lower().replace("_", "")
    # Code addresses only: data constants (e.g. the HUD scale floats) are often just read.
    return bool(row.regions) and all(reg[0] < 0x856000 and f"{reg[0]:#x}" in _CLIENT_SRC_TEXT for reg in row.regions)


def milestones():
    """{row: [M5, …]} from re10-gaps.md 'CSV rows Mx must apply' tables."""
    out = {}
    cur = None
    for ln in open(RE10):
        m = re.match(r"### CSV rows (M\d+) must apply", ln)
        if m:
            cur = m.group(1)
            continue
        if ln.startswith("#"):
            cur = None
        if cur:
            m = re.match(r"\|\s*(\d+)\s*\|", ln)
            if m:
                out.setdefault(int(m.group(1)), []).append(cur)
    return out


def justifications(path):
    out = {}
    if not os.path.exists(path):
        return out
    for ln in open(path):
        if not ln.strip() or ln.startswith("#"):
            continue
        parts = ln.rstrip("\n").split("\t")
        if parts[0].strip() == "row":
            continue
        out[int(parts[0])] = {"reason": parts[1] if len(parts) > 1 else "", "decision": parts[2] if len(parts) > 2 else ""}
    return out


def tracer_sites():
    """[(lo, hi, what)] GTA bytes the tracer itself rewrites in the original's lane."""
    out = []
    src = open(os.path.join(TRACER_SRC, "hooks.rs")).read()
    for m in re.finditer(r"addr:\s*(0x[0-9A-Fa-f]+),\s*prologue:\s*&\[([^\]]*)\]", src):
        n = len([x for x in m.group(2).split(",") if x.strip()])
        out.append((int(m.group(1), 16), int(m.group(1), 16) + n, "tracer inline hook"))
    for m in re.finditer(r"\bh\(\s*\"[^\"]*\",\s*(0x[0-9A-Fa-f]+),\s*&\[([^\]]*)\]", src):
        n = len([x for x in m.group(2).split(",") if x.strip()])
        out.append((int(m.group(1), 16), int(m.group(1), 16) + n, "tracer inline hook"))
    for path in glob.glob(os.path.join(TRACER_SRC, "win", "*.rs")):
        for m in re.finditer(r"const \w*VTABLE_SLOT: u32 = (0x[0-9A-Fa-f]+)", open(path).read()):
            a = int(m.group(1), 16)
            out.append((a, a + 4, "tracer outer wrapper (vtable slot)"))
    return out


def load_dump(path):
    """{region: [bytes, …]}: every distinct value seen over the snapshots
    (older dumps: the single snapshot); [None] if unreadable."""
    d = json.load(open(path))
    src = d.get("variants") or {k: [v] for k, v in d["regions"].items()}
    out = {}
    for k, vals in src.items():
        a, n = k.split(":")
        bs = [bytes.fromhex(v) for v in vals if v is not None]
        out[(int(a, 16), int(n))] = bs or [None]
    return d, out


def load_watch(run, side):
    """The side's frame-local write watch; None when the run has none (older runs)."""
    mp = os.path.join(run, f"frame-watch-{side}.json")
    if not os.path.exists(mp):
        return None
    meta = json.load(open(mp))
    w = {"error": meta.get("error"), "window": meta.get("window"), "pages": set(), "writes": [], "frames": 0}
    begin = end = stop = None
    ep = os.path.join(run, f"{side}-efx.jsonl")
    if not w["error"] and not os.path.exists(ep):
        w["error"] = f"{side}-efx.jsonl missing"
    if not w["error"]:
        for ln in open(ep):
            try:
                r = json.loads(ln)
            except ValueError:
                continue
            if r.get("cap") != w["window"]:
                continue
            f = r.get("fn")
            if f == "efx_begin":
                begin = r
            elif f == "efx_end":
                end = r
            elif f == "efx_stop":
                stop = r
            elif f == "efx_write":
                w["writes"].append(r)
        if begin is None or end is None:
            w["error"] = "window records missing"
        elif stop is not None:
            w["error"] = f"window stopped ({stop.get('why')})"
        else:
            w["pages"] = {int(str(p), 0) if not isinstance(p, int) else p for p in begin.get("pages", [])}
            w["frames"] = int(end.get("game_frames") or 0)
            w["faults"] = end.get("faults")
    return w


def watch_summary(w, reg, e):
    """Writes of one watch window that touch region reg (e = unmodified bytes)."""
    a0, n = reg
    ws = [x for x in w["writes"] if int(str(x["addr"]), 0) < a0 + n and a0 < int(str(x["addr"]), 0) + int(x["size"])]
    per = {}
    for x in ws:
        per.setdefault(int(x.get("game_frame", 0)), []).append(x)
    values = {(int(str(x["addr"]), 0), x.get("new")) for x in ws}
    pats = {}
    restored = 0
    for fr, xs in per.items():
        key = tuple(sorted({(int(str(x["addr"]), 0), x.get("new")) for x in xs}))
        pats[key] = pats.get(key, 0) + 1
        last = {}
        for x in xs:
            last[int(str(x["addr"]), 0)] = x
        ok = True
        for ad, x in last.items():
            off = ad - a0
            want = e[off:off + int(x["size"])] if 0 <= off < n else None
            if want is None or x.get("new") != want.hex():
                ok = False
        restored += ok
    top = max(pats.items(), key=lambda kv: kv[1])[0] if pats else ()
    frames = max(w["frames"], 1)
    return {"writes": len(ws), "frames_with_writes": len(per), "frames": w["frames"],
            "share": len(per) / frames, "values": sorted(values, key=str), "pattern": list(top),
            "restored_frames": restored, "per_frame": round(len(ws) / max(len(per), 1), 2),
            "by": sorted({str(x.get("by")) for x in ws}), "at": sorted({str(x.get("at")) for x in ws})[:4]}


def classify_watch(reg, wo, wu, e):
    """(status, info) from the write watch, or None when it has no say (no watch / no writes)."""
    if wo is None and wu is None:
        return None
    for name, w in (("orig", wo), ("ours", wu)):
        if w is None or w["error"]:
            return "unverified", {"why": f"{name}: {w['error'] if w else 'no watch'}"}
        pages = {reg[0] & ~0xFFF, (reg[0] + reg[1] - 1) & ~0xFFF}
        if not pages <= w["pages"]:
            return "unverified", {"why": f"{name}: page {min(pages - w['pages']):#x} not guarded"}
    so, su = watch_summary(wo, reg, e), watch_summary(wu, reg, e)
    info = {"orig": so, "ours": su}
    if so["writes"] == 0 and su["writes"] == 0:
        return None
    if su["writes"] == 0:
        return "missing", info
    if so["writes"] == 0:
        return "different", info
    same = (so["values"] == su["values"] and so["pattern"] == su["pattern"]
            and abs(so["share"] - su["share"]) <= 0.25
            and (so["restored_frames"] == so["frames_with_writes"]) == (su["restored_frames"] == su["frames_with_writes"]))
    return ("match-watched" if same else "different"), info


def watch_note(addr, info):
    if "why" in info:
        return f"{addr}: write watch unverified ({info['why']})"
    o, u = info["orig"], info["ours"]

    def side(x):
        vals = "/".join(f"{a:#x}={v}" for a, v in x["values"])[:90]
        return (f"{x['writes']} writes in {x['frames_with_writes']}/{x['frames']} frames, {x['per_frame']}/frame, "
                f"restored {x['restored_frames']}, values {vals or '-'}")
    return f"{addr}: write watch (orig {side(o)}; ours {side(u)})"


def outside(v):
    return not patchrows.IMAGE_LO <= v < patchrows.IMAGE_HI


def equivalent(kind, addr, o, u, e):
    """Same effect although the bytes differ: both redirect out of gta_sa.exe."""
    k = kind.lower()
    if len(o) == 4 and ("pointer" in k or "<samp ptr>" in k):
        ov, uv, ev = (int.from_bytes(x, "little") for x in (o, u, e))
        return ov != ev and uv != ev and outside(ov) and outside(uv)
    if len(o) == 4 and "call-retarget" in k:
        # rel32 operand only (the row's address is call+1)
        to = (addr + 4 + int.from_bytes(o, "little", signed=True)) & 0xFFFFFFFF
        tu = (addr + 4 + int.from_bytes(u, "little", signed=True)) & 0xFFFFFFFF
        return o != e and u != e and outside(to) and outside(tu)
    if len(o) >= 5 and o[0] in (0xE8, 0xE9) and u[0] == o[0]:
        to = (addr + 5 + int.from_bytes(o[1:5], "little", signed=True)) & 0xFFFFFFFF
        tu = (addr + 5 + int.from_bytes(u[1:5], "little", signed=True)) & 0xFFFFFFFF
        return outside(to) and outside(tu) and o[5:] == u[5:]
    return False


def classify_region(row, reg, os_, us_, e):
    """os_/us_: values seen on the original / ours over the snapshots."""
    if None in os_ or None in us_ or e is None:
        return "unreadable"
    if len(os_) == 1 and len(us_) == 1:
        return classify_one(row, reg, os_[0], us_[0], e)
    # Toggling: ours must apply it (some value != exe) and every value ours
    # shows must be one the original showed (or equivalent to one).
    if all(o == e for o in os_):
        return "inactive" if all(u == e for u in us_) else "different"
    if all(u == e for u in us_):
        return "missing"
    for u in us_:
        if not any(u == o or (o != e and classify_one(row, reg, o, u, e) == "match-equivalent") for o in os_):
            return "different"
    return "match-toggling"


def classify_one(row, reg, o, u, e):
    if row.kind.startswith("data-fill"):
        # The filled memory is reused at runtime (row 347: the old sector grid
        # holds Players[2..209]); its live content says nothing about the fill.
        return "not-live-checkable"
    if o != e:
        if u == o:
            return "match"
        if equivalent(row.kind + " " + row.d.get("new_bytes_hex", ""), reg[0], o, u, e):
            return "match-equivalent"
        if u == e:
            return "missing"
        return "different"
    return "inactive" if u == e else "different"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--run", required=True)
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "diff"))
    ap.add_argument("--justify", default=JUSTIFY)
    a = ap.parse_args()
    meta_a, dump_o = load_dump(os.path.join(a.run, "patch-bytes-A.json"))
    meta_b, dump_u = load_dump(os.path.join(a.run, "patch-bytes-B.json"))
    if meta_a["kind"] != "orig" or meta_b["kind"] != "ours":
        print(f"expected A=orig, B=ours; got A={meta_a['kind']} B={meta_b['kind']}")
        return 2
    exe = patchrows.exe()
    if exe is None:
        print("re/gta/1.0-US/gta_sa.exe missing: cannot tell applied from unmodified bytes")
        return 2
    static = static_rows()
    owners = milestones()
    just = justifications(a.justify)
    tsites = tracer_sites()
    wo, wu = load_watch(a.run, "A"), load_watch(a.run, "B")

    results = []
    for row in patchrows.rows():
        regs = []
        for reg in row.regions:
            os_, us_, e = dump_o.get(reg, [None]), dump_u.get(reg, [None]), exe.read(*reg)
            st = classify_region(row, reg, os_, us_, e)
            watch = None
            if row.row in patchrows.FRAME_LOCAL_ROWS and e is not None:
                cw = classify_watch(reg, wo, wu, e)
                if cw is not None:
                    st, watch = cw
            o, u = os_[0], us_[0]
            overl = sorted({w for lo, hi, w in tsites if lo < reg[0] + reg[1] and reg[0] < hi})
            regs.append({"addr": f"{reg[0]:#x}", "len": reg[1], "status": st, "orig": o.hex() if o else None,
                         "ours": u.hex() if u else None, "exe": e.hex() if e else None, "tracer_overlap": overl,
                         "orig_variants": [x.hex() for x in os_ if x], "ours_variants": [x.hex() for x in us_ if x],
                         "watch": watch})
        sts = {r["status"] for r in regs}
        live = None
        if "unverified" in sts:
            live = "unverified"
        elif "different" in sts:
            live = "different"
        elif "missing" in sts:
            live = "missing"
        elif sts & {"match", "match-equivalent", "match-toggling", "match-watched"}:
            live = "match"
        elif sts == {"inactive"}:
            live = "inactive"
        elif "unreadable" in sts:
            live = "unreadable"
        elif "not-live-checkable" in sts:
            live = "not-live-checkable"
        if row.partial and live in ("match", "inactive", None):
            live = (live or "") + "+not-live-checkable" if live else "not-live-checkable"
        if row.row in just:
            cat = "justified"
        elif live == "unverified":
            cat = "unverified"
        elif live == "different":
            cat = "applied/different"
        elif live == "missing":
            cat = "missing"
        elif live == "match":
            cat = "applied/matching"
        elif row.row in static:
            cat = "applied (static only)"
        elif live and "inactive" in live and addr_referenced(row):
            # Runtime rows (e.g. the input lock, 349–353) whose code names the patch address but no row.
            static.setdefault(row.row, []).append("address constant in samp-client")
            cat = "applied (static only)"
        else:
            cat = "missing"
        results.append({"row": row.row, "addr": row.addr_field, "size": row.size_field, "kind": row.kind,
                        "applied_when": row.applied_when, "function": row.function, "live": live,
                        "category": cat, "static_refs": static.get(row.row, []),
                        "milestones": owners.get(row.row, []), "justification": just.get(row.row),
                        "regions": regs})

    cats = ["applied/matching", "applied (static only)", "applied/different", "missing", "unverified", "justified"]
    totals = {c: sum(r["category"] == c for r in results) for c in cats}
    ok = totals["missing"] == 0 and totals["applied/different"] == 0 and totals["unverified"] == 0
    os.makedirs(a.out, exist_ok=True)
    summary = {"run": os.path.relpath(a.run, ROOT), "rows": len(results), "totals": totals, "pass": ok,
               "results": results}
    json.dump(summary, open(os.path.join(a.out, "patch-coverage.json"), "w"), indent=1)

    def owner(r):
        return "/".join(r["milestones"]) or "not M5–M7 (re10: start-up/SP-OFF/M3/UI/M8b/M9)"

    md = ["# F5 patch coverage", "",
          f"Live bytes from run `{summary['run']}`: original client (tracer `peek`) vs ours (`peek`), "
          "after join + spawn + teleport, compared with re/gta/1.0-US/gta_sa.exe. "
          f"{len(results)} CSV rows (row = CSV line number).", "",
          "| category | rows |", "|---|---|"]
    md += [f"| {c} | {totals[c]} |" for c in cats]
    md += ["", f"**{'PASS' if ok else 'FAIL'}**: {totals['missing']} missing, {totals['applied/different']} different, "
           f"{totals['unverified']} unverified.", ""]
    wl = []
    for r in results:
        for g in r["regions"]:
            if r["row"] in patchrows.FRAME_LOCAL_ROWS:
                wl.append(f"* row {r['row']} {watch_note(g['addr'], g['watch']) if g['watch'] else g['addr'] + ': no writes on either client in the watch window (snapshot rule)'}"
                          f" -> {g['status']}")
    if wo is not None or wu is not None:
        md += ["Frame-local rows (write watch, both clients):", ""] + wl + [""]
    by_ms = {}
    for r in results:
        if r["category"] == "missing":
            by_ms[owner(r)] = by_ms.get(owner(r), 0) + 1
    md += ["Missing rows by owning milestone (re10-gaps.md):", ""]
    md += [f"* {k}: {v}" for k, v in sorted(by_ms.items(), key=lambda x: -x[1])]
    md += ["", "Live status of rows that the original had NOT applied at the snapshot (`inactive`: runtime, "
           "temporary, conditional or dead rows) is judged by the static table only.", ""]
    for c in ["applied/different", "missing", "unverified", "applied (static only)", "justified", "applied/matching"]:
        sel = [r for r in results if r["category"] == c]
        md += [f"## {c} ({len(sel)})", "", "| row | address | kind | function | live | owner | notes |", "|---|---|---|---|---|---|---|"]
        for r in sel:
            notes = []
            for g in r["regions"]:
                if g["watch"]:
                    notes.append(watch_note(g["addr"], g["watch"]))
                elif g["status"] in ("different", "missing"):
                    notes.append(f"{g['addr']}: orig {(g['orig'] or '?')[:24]} ours {(g['ours'] or '?')[:24]} exe {(g['exe'] or '?')[:24]}")
                if len(g["orig_variants"]) > 1 or len(g["ours_variants"]) > 1:
                    notes.append(f"{g['addr']}: toggles (orig {'/'.join(v[:12] for v in g['orig_variants'])}; "
                                 f"ours {'/'.join(v[:12] for v in g['ours_variants'])})")
                if g["tracer_overlap"]:
                    notes.append(f"{g['addr']}: {', '.join(g['tracer_overlap'])} in the original's lane")
            if r["justification"]:
                notes.append(f"{r['justification']['reason']} ({r['justification']['decision']})")
            if r["static_refs"] and c != "applied/matching":
                notes.append("static: " + ", ".join(r["static_refs"]))
            md.append(f"| {r['row']} | {r['addr'][:40]} | {r['kind'].split('(')[0].strip()} | {r['function'][:44]} | "
                      f"{r['live']} | {owner(r) if c in ('missing', 'applied/different') else ''} | {'; '.join(notes)} |")
        md.append("")
    open(os.path.join(a.out, "patch-coverage.md"), "w").write("\n".join(md))
    head = md.index("Missing rows by owning milestone (re10-gaps.md):") if "Missing rows by owning milestone (re10-gaps.md):" in md else 14
    print("\n".join(md[:head]))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
