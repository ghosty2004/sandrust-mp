#!/usr/bin/env python3
"""COV-1: merge the Ghidra function export with known names and decide which entries the
tracer's coverage mode may arm with a one-shot int3.

    names.py GHIDRA_TSV GTA_SA_EXE OUT_TSV

Writes OUT_TSV (entry, size, name, name_src, category, ghidra_name, orig, arm, reason) and
next to it arm.txt ("0xADDR BYTE" per armable entry), the list samp-tracer loads
(SAMP_TRACER_COV).

Names: re/gta-reversed RH_Scoped*Install macros (with the RH_ScopedCategory of the file as
the category), then plugin-sdk plugin_sa ("Converted from ... NAME(...) 0xADDR" and
META_BEGIN/id), else Ghidra's own name.

An entry is NOT armed (reason column) when:
  * section: outside the game's main .text / _rwcseg (the other code sections hold the
    exe's protection layer: obfuscated stubs that read and write game code as data);
  * data_read: an instruction anywhere in the image reads or writes memory covering the entry
    byte (absolute memory operand; e.g. the stub at 0x403794 reads 0x42B7D3), or Ghidra
    reports a READ/WRITE reference to the entry;
  * samp_site: a samp.dll patch/read site (annotated CSV rows, model-pointer table, auth read)
    covers the entry: samp checks or copies those bytes;
  * client_site: a samp-client patch site covers the entry (it checks the original bytes);
  * unreferenced: unnamed, and no rel32 call/jmp and no 32-bit value in the image points at
    the entry (likely not code);
  * not_code: the entry byte is 0xCC / 0x00 padding.
The tracer additionally skips, at runtime, entries inside its own hook prologues and any
entry whose live byte differs from the file (patched by a client).
"""
import os
import re
import struct
import sys
from bisect import bisect_right

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
TEXT = (0x401000, 0x401000 + 0x455E00)
RWC = (0x857000, 0x857600)


class Pe:
    def __init__(self, path):
        self.data = open(path, "rb").read()
        d = self.data
        e = struct.unpack_from("<I", d, 0x3C)[0]
        n = struct.unpack_from("<H", d, e + 6)[0]
        opt = struct.unpack_from("<H", d, e + 20)[0]
        self.base = struct.unpack_from("<I", d, e + 24 + 28)[0]
        self.secs = []
        for i in range(n):
            o = e + 24 + opt + 40 * i
            name = d[o:o + 8].rstrip(b"\0").decode("latin1")
            vsize, va, rsize, roff, _, _, _, _, chars = struct.unpack_from("<IIIIIIHHI", d, o + 8)
            self.secs.append((name, self.base + va, rsize, roff, chars))

    def byte(self, va):
        for _, sva, size, off, _ in self.secs:
            if sva <= va < sva + size:
                return self.data[off + va - sva]
        return None


def scan_image(pe, entries):
    """(abs memory operands covering an entry, rel32 targets, 32-bit values) over all sections."""
    covered = set()
    targets = set()
    values = set()
    srt = sorted(entries)
    ent = set(entries)
    # ModRM opcodes whose r/m operand is memory (one-byte opcode map, common ALU/mov forms).
    modrm_ops = set(range(0x00, 0x40)) - {0x0F, 0x26, 0x27, 0x2E, 0x2F, 0x36, 0x37, 0x3E, 0x3F,
                                           0x04, 0x05, 0x0C, 0x0D, 0x14, 0x15, 0x1C, 0x1D, 0x24, 0x25,
                                           0x2C, 0x2D, 0x34, 0x35, 0x3C, 0x3D, 0x06, 0x07, 0x0E, 0x16,
                                           0x17, 0x1E, 0x1F}
    modrm_ops |= {0x80, 0x81, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0xC6, 0xC7,
                  0xF6, 0xF7, 0xFE, 0xFF, 0xD0, 0xD1, 0xD2, 0xD3, 0xD8, 0xD9, 0xDA, 0xDB, 0xDC,
                  0xDD, 0xDE, 0xDF, 0x69, 0x6B, 0x8F}
    twobyte = {0xB6, 0xB7, 0xBE, 0xBF, 0xAF, 0xA3, 0xAB, 0xB3, 0xBB, 0xBA, 0x10, 0x11, 0x28, 0x29,
               0x6F, 0x7F, 0x6E, 0x7E}

    def cover(v, width=4):
        # entries E with v <= E < v + width
        i = bisect_right(srt, v - 1)
        while i < len(srt) and srt[i] < v + width:
            covered.add(srt[i])
            i += 1

    for name, sva, size, off, chars in pe.secs:
        blob = pe.data[off:off + size]
        is_code = bool(chars & 0x20000000) or bool(chars & 0x20)
        n = len(blob)
        for i in range(n - 3):
            v = struct.unpack_from("<I", blob, i)[0]
            if v in ent:
                values.add(v)
        if not is_code:
            continue
        for i in range(n - 5):
            b = blob[i]
            if b in (0xE8, 0xE9):
                rel = struct.unpack_from("<i", blob, i + 1)[0]
                targets.add((sva + i + 5 + rel) & 0xFFFFFFFF)
            elif b == 0x0F and 0x80 <= blob[i + 1] <= 0x8F and i + 6 <= n:
                rel = struct.unpack_from("<i", blob, i + 2)[0]
                targets.add((sva + i + 6 + rel) & 0xFFFFFFFF)
            if b in (0xA0, 0xA1, 0xA2, 0xA3):
                cover(struct.unpack_from("<I", blob, i + 1)[0])
            if b in modrm_ops and i + 6 <= n and (blob[i + 1] & 0xC7) == 0x05:
                cover(struct.unpack_from("<I", blob, i + 2)[0])
            if b == 0x0F and i + 7 <= n and blob[i + 1] in twobyte and (blob[i + 2] & 0xC7) == 0x05:
                cover(struct.unpack_from("<I", blob, i + 3)[0], 16)
            if b in (0x66, 0xF2, 0xF3) and i + 8 <= n and blob[i + 1] == 0x0F and (blob[i + 3] & 0xC7) == 0x05:
                cover(struct.unpack_from("<I", blob, i + 4)[0], 16)
    return covered, targets, values


def reversed_names():
    out = {}
    src = os.path.join(ROOT, "re/gta-reversed/source")
    inst = re.compile(r"^\s*RH_Scoped(\w*)Install\s*\(\s*([^,]+?)\s*,(.*)$")
    for dp, _, fs in os.walk(src):
        for f in fs:
            if not f.endswith((".cpp", ".h", ".hpp")):
                continue
            cls = None
            cat = ""
            try:
                lines = open(os.path.join(dp, f), encoding="utf-8", errors="replace").read().splitlines()
            except OSError:
                continue
            for line in lines:
                s = line.strip()
                if s.startswith("//"):
                    continue
                m = re.match(r"RH_Scoped(?:Virtual|Named)?Class\s*\(\s*([\w:<>]+)", s)
                if m:
                    cls = m.group(1)
                    continue
                m = re.match(r"RH_ScopedNamespace(?:Name)?\s*\(\s*\"?([\w:]+)", s)
                if m:
                    cls = m.group(1)
                    continue
                m = re.match(r"RH_ScopedCategory\s*\(\s*\"([^\"]*)\"", s)
                if m:
                    cat = m.group(1)
                    continue
                if s.startswith("RH_ScopedCategoryGlobal"):
                    cat = "Global"
                    continue
                m = inst.match(s)
                if not m:
                    continue
                kind, nm, rest = m.groups()
                addrs = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{6,8})", rest)]
                addrs = [a for a in addrs if TEXT[0] <= a < 0x1600000]
                if not addrs:
                    continue
                nm = nm.strip().strip('"')
                ov = re.match(r'\s*"([^"]*)"', rest)
                if "Overloaded" in kind and ov and ov.group(1):
                    nm = f"{nm}<{ov.group(1)}>"
                full = f"{cls}::{nm}" if cls and "Global" not in kind else nm
                if "Global" in kind and cls:
                    full = f"{cls}::{nm}"
                out.setdefault(addrs[0], (full, cat or os.path.relpath(dp, src)))
    return out


def plugin_names():
    out = {}
    src = os.path.join(ROOT, "re/plugin-sdk/plugin_sa")
    conv = re.compile(r"Converted from \w+ .*?([\w:~<>]+(?:::[\w~<>=!+\-*/\[\] ]+)?)\s*\(.*\)\s*0x([0-9A-Fa-f]{6})\b")
    meta = re.compile(r"META_BEGIN(?:_OVERLOADED)?\s*\(\s*([\w:~]+)")
    mid = re.compile(r"static const int id = 0x([0-9A-Fa-f]{6})")
    for dp, _, fs in os.walk(src):
        for f in fs:
            if not f.endswith((".cpp", ".h")):
                continue
            pending = None
            for line in open(os.path.join(dp, f), encoding="utf-8", errors="replace"):
                m = conv.search(line)
                if m:
                    out.setdefault(int(m.group(2), 16), m.group(1).strip())
                    continue
                m = meta.search(line)
                if m:
                    pending = m.group(1)
                    continue
                m = mid.search(line)
                if m and pending:
                    out.setdefault(int(m.group(1), 16), pending)
                    pending = None
    return out


def samp_sites():
    sites = []
    csv = os.path.join(ROOT, "docs/re/data/gta-patches-r4-annotated.csv")
    for line in open(csv).read().splitlines()[1:]:
        if line.startswith('"'):
            first = line[1:].split('"', 1)[0]
            rest = line.split('"', 2)[2].lstrip(",")
            size = rest.split(",", 1)[0]
        else:
            first, size = line.split(",")[:2]
        try:
            size = max(int(size), 1)
        except ValueError:
            size = 4
        for a in re.findall(r"0x([0-9a-fA-F]+)", first):
            sites.append((int(a, 16), size))
    sites.append((0x4D3AA0, 20))  # auth handler reads 20 bytes (gta-patches.md §4)
    # Model-pointer table: 707 x {u32 code address, u8 expected opcode} (samp.dll A 0x10114B10).
    p = os.path.join(ROOT, "re/samp-dll/0.3.7-R4_fv0.3.7.4_2021-05-07/samp.dll")
    if os.path.exists(p):
        pe = Pe(p)
        for name, sva, size, off, _ in pe.secs:
            if sva <= 0x10114B10 < sva + size:
                t = pe.data[off + 0x10114B10 - sva: off + 0x10114B10 - sva + 707 * 5]
                for k in range(707):
                    a = struct.unpack_from("<I", t, 5 * k)[0]
                    sites.append((a, 8))
    return sites


def client_sites(entries):
    """Patch sites of samp-client: table entries (addr + original bytes) and constants on
    patch lines."""
    sites = []
    src = os.path.join(ROOT, "crates/samp-client/src")
    patchy = re.compile(r"write|nop|hook_|retarget|SITE|_CALL|addr:|original|swap_pointer")
    for dp, _, fs in os.walk(src):
        for f in fs:
            if not f.endswith(".rs"):
                continue
            text = open(os.path.join(dp, f), encoding="utf-8").read()
            for m in re.finditer(r"addr:\s*(0x[0-9A-Fa-f_]+),(.{0,600}?)original:\s*&\[([^\]]*)\]", text, re.S):
                a = int(m.group(1).replace("_", ""), 16)
                n = len([x for x in m.group(3).split(",") if x.strip()])
                sites.append((a, max(n, 1)))
            for line in text.splitlines():
                for h in re.findall(r"\b0x([0-9A-Fa-f_]{6,9})\b", line):
                    a = int(h.replace("_", ""), 16)
                    if not (TEXT[0] <= a < 0x1600000):
                        continue
                    if a in entries and not patchy.search(line):
                        continue  # a function the client calls, not a patch site
                    sites.append((a, 8 if a not in entries else 5))
    sites.append((0x156CDAE, 6))
    return sites


def main():
    raw, exe, out = sys.argv[1:4]
    pe = Pe(exe)
    rows = []
    with open(raw) as fh:
        next(fh)
        for line in fh:
            p = line.rstrip("\n").split("\t")
            while len(p) < 5:
                p.append("")
            rows.append((int(p[0], 16), int(p[1]), p[2], p[3] == "1", p[4]))
    entries = {r[0] for r in rows}
    covered, targets, values = scan_image(pe, entries)
    rev = reversed_names()
    plg = plugin_names()
    srt = sorted(entries)

    def covered_by(sites):
        hit = set()
        for a, n in sites:
            i = bisect_right(srt, a - 1)
            while i < len(srt) and srt[i] < a + n:
                hit.add(srt[i])
                i += 1
        return hit

    samp = covered_by(samp_sites())
    client = covered_by(client_sites(entries))
    n_arm = 0
    with open(out, "w") as w, open(os.path.join(os.path.dirname(out), "arm.txt"), "w") as aw:
        w.write("entry\tsize\tname\tname_src\tcategory\tghidra_name\torig\tarm\treason\n")
        aw.write("# samp-tracer coverage arm list (tools/cov/names.py): ADDR ORIGINAL_BYTE\n")
        for e, size, gname, thunk, drefs in sorted(rows):
            if e in rev:
                name, src, cat = rev[e][0], "gta-reversed", rev[e][1]
            elif e in plg:
                name, src, cat = plg[e], "plugin-sdk", ""
            else:
                name, src, cat = gname, "ghidra", ""
            b = pe.byte(e)
            reason = ""
            gh_rw = any(x.split("+")[1].startswith("0:") and (":READ" in x or ":WRITE" in x)
                        for x in drefs.split(",") if "+" in x)
            if not (TEXT[0] <= e < TEXT[1] or RWC[0] <= e < RWC[1]):
                reason = "section"
            elif b is None or b in (0xCC, 0x00):
                reason = "not_code"
            elif e in covered or gh_rw:
                reason = "data_read"
            elif e in samp:
                reason = "samp_site"
            elif e in client:
                reason = "client_site"
            elif e not in targets and e not in values and src == "ghidra":
                reason = "unreferenced"  # named functions are known code (reached via stubs)
            arm = 0 if reason else 1
            n_arm += arm
            if arm:
                aw.write(f"0x{e:08x} {b:02x}\n")
            w.write(f"0x{e:08x}\t{size}\t{name}\t{src}\t{cat}\t{gname}\t{'' if b is None else f'{b:02x}'}\t{arm}\t{reason}\n")
    from collections import Counter
    cnt = Counter()
    with open(out) as fh:
        next(fh)
        for line in fh:
            cnt[line.rstrip("\n").split("\t")[8] or "armed"] += 1
    named = sum(1 for e in entries if e in rev or e in plg)
    print(f"{len(rows)} functions, {named} named ({sum(1 for e in entries if e in rev)} gta-reversed), "
          f"{n_arm} armable; " + ", ".join(f"{k} {v}" for k, v in sorted(cnt.items())))


if __name__ == "__main__":
    main()
