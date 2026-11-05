#!/usr/bin/env python3
"""FEAT-SWEEP: samp.dll arm list for the tracer's module coverage mode.

samp_arm.py FUNCTIONS.tsv SAMP.DLL OUT.txt
Writes `0xRVA BYTE` for every non-thunk function entry inside an executable section, with
the entry byte read from the file (cross-checked against Ghidra's). Entries whose byte is a
relocation target are left out (the live byte would differ after a rebase).
"""
import struct
import sys


def sections(pe):
    e = struct.unpack_from("<I", pe, 0x3C)[0]
    nsec = struct.unpack_from("<H", pe, e + 6)[0]
    opt = struct.unpack_from("<H", pe, e + 20)[0]
    base = struct.unpack_from("<I", pe, e + 24 + 28)[0]
    reloc_rva, reloc_size = struct.unpack_from("<II", pe, e + 24 + 96 + 5 * 8)
    out = []
    for i in range(nsec):
        o = e + 24 + opt + i * 40
        name = pe[o:o + 8].rstrip(b"\0").decode(errors="replace")
        vsz, va, rsz, raw = struct.unpack_from("<IIII", pe, o + 8)
        ch = struct.unpack_from("<I", pe, o + 36)[0]
        out.append((name, va, max(vsz, rsz), raw, rsz, ch))
    return base, out, (reloc_rva, reloc_size)


def rva_to_off(secs, rva):
    for _, va, sz, raw, rsz, _ in secs:
        if va <= rva < va + sz and rva - va < rsz:
            return raw + rva - va
    return None


def reloc_bytes(pe, secs, rr):
    rva, size = rr
    off = rva_to_off(secs, rva) if rva else None
    s = set()
    if off is None:
        return s
    end = off + size
    while off + 8 <= end:
        page, blk = struct.unpack_from("<II", pe, off)
        if blk < 8:
            break
        for i in range((blk - 8) // 2):
            w = struct.unpack_from("<H", pe, off + 8 + 2 * i)[0]
            if w >> 12 == 3:
                a = page + (w & 0xFFF)
                s.update(range(a, a + 4))
        off += blk
    return s


def main():
    fn, dll, out = sys.argv[1:4]
    pe = open(dll, "rb").read()
    base, secs, rr = sections(pe)
    rel = reloc_bytes(pe, secs, rr)
    exe = [(va, va + sz) for _, va, sz, _, _, ch in secs if ch & 0x20000000]
    lines, skipped = [], {"thunk": 0, "section": 0, "reloc": 0, "mismatch": 0}
    for row in open(fn, encoding="utf-8"):
        if row.startswith("entry\t"):
            continue
        f = row.rstrip("\n").split("\t")
        addr = int(f[0], 16)
        rva = addr - base
        if f[3] == "1":
            skipped["thunk"] += 1
            continue
        if not any(a <= rva < b for a, b in exe):
            skipped["section"] += 1
            continue
        if rva in rel:
            skipped["reloc"] += 1
            continue
        off = rva_to_off(secs, rva)
        b = pe[off]
        if f[4] != "-" and int(f[4], 16) != b:
            skipped["mismatch"] += 1
            continue
        lines.append(f"{rva:#010x} {b:02x}")
    with open(out, "w") as w:
        w.write(f"# samp.dll arm list (RVA, entry byte); image base {base:#x}; {dll}\n")
        w.write("\n".join(lines) + "\n")
    print(f"samp_arm: {len(lines)} armable, skipped {skipped} -> {out}")


if __name__ == "__main__":
    main()
