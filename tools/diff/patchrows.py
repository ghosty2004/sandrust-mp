"""Rows of docs/re/data/gta-patches-r4-annotated.csv as memory regions (F5).

Row numbers are line numbers of the CSV file (header = line 1), the convention
of samp-client's `csv_row` fields and of docs/phase3/re10-gaps.md.

    rows()            -> [Row]  (row, addr field, size, kind, regions, partial, ...)
    regions()         -> sorted unique (addr, len) of every row's regions
    exe_bytes(a, n)   -> bytes of the unmodified gta_sa.exe 1.0 US at VA a
                         (re/gta/1.0-US/gta_sa.exe; .bss = zeros), None if absent
"""
import csv
import os
import re
import struct

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
CSV = os.path.join(ROOT, "docs", "re", "data", "gta-patches-r4-annotated.csv")
EXE = os.path.join(ROOT, "re", "gta", "1.0-US", "gta_sa.exe")
# gta_sa.exe 1.0 US image: ImageBase 0x400000, SizeOfImage 0x1177000 (PE header).
IMAGE_LO, IMAGE_HI = 0x400000, 0x400000 + 0x1177000

TYPE_SIZE = {"u8": 1, "u16": 2, "u32": 4, "f32": 4}


class Row:
    def __init__(self, row, d):
        self.row = row
        self.d = d
        self.addr_field = d["gta_addr"]
        self.size_field = d["size"]
        self.kind = d["kind"]
        self.applied_when = d["applied_when"]
        self.function = d["gta_function"]
        self.regions, self.partial = parse_regions(self.addr_field, self.size_field)

    def short(self):
        return f"{self.addr_field} ({self.kind.split('(')[0].strip()})"


def parse_regions(addr_field, size_field):
    """[(addr, len)] inside the GTA image, and whether the field had more than
    plain addresses (e.g. "0x411458 + 7 sites from .data …", "(707 sites)")."""
    size = int(size_field) if size_field.strip().isdigit() else None
    out = []
    for m in re.finditer(r"0x([0-9a-fA-F]+)(?:\(([^)]*)\))?", addr_field):
        a = int(m.group(1), 16)
        if not IMAGE_LO <= a < IMAGE_HI:
            continue
        n = size
        if m.group(2):
            t = m.group(2)
            mm = re.fullmatch(r"(\d+)x(\w+)", t)
            n = int(mm.group(1)) * TYPE_SIZE[mm.group(2)] if mm else TYPE_SIZE.get(t, size)
        if n:
            out.append((a, n))
    rest = re.sub(r"0x[0-9a-fA-F]+(\([^)]*\))?", "", addr_field)
    partial = bool(re.sub(r"[\s,]", "", rest)) or size is None and not out
    return out, partial


def rows():
    with open(CSV, newline="") as f:
        return [Row(i + 2, d) for i, d in enumerate(csv.DictReader(f))]


def regions():
    return sorted({r for row in rows() for r in row.regions})


class Exe:
    def __init__(self, path=EXE):
        self.data = open(path, "rb").read()
        d = self.data
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        nsec = struct.unpack_from("<H", d, pe + 6)[0]
        opt = struct.unpack_from("<H", d, pe + 20)[0]
        self.base = struct.unpack_from("<I", d, pe + 24 + 28)[0]
        self.secs = []
        for i in range(nsec):
            s = pe + 24 + opt + 40 * i
            vsize, vaddr, rsize, roff = struct.unpack_from("<IIII", d, s + 8)
            self.secs.append((vaddr, max(vsize, rsize), rsize, roff))

    def read(self, va, n):
        out = bytearray()
        for k in range(n):
            rva = va + k - self.base
            b = None
            for vaddr, vlen, rsize, roff in self.secs:
                if vaddr <= rva < vaddr + vlen:
                    o = rva - vaddr
                    b = self.data[roff + o] if o < rsize else 0
                    break
            if b is None:
                return None
            out.append(b)
        return bytes(out)


def exe():
    try:
        return Exe()
    except OSError:
        return None


def chunks(regs, maxlen=256):
    """Split regions into peek-sized pieces: [(addr, len, (region, offset))]."""
    out = []
    for a, n in regs:
        o = 0
        while o < n:
            k = min(maxlen, n - o)
            out.append((a + o, k, (a, n), o))
            o += k
    return out
