#!/usr/bin/env python3
"""FEAT-SWEEP: load the samp.dll Ghidra export (tools/cov/samp-inventory.sh) as a graph.

Library module: functions, call edges, string references (Ghidra's defined strings plus data
references whose target bytes form a printable NUL-terminated ASCII/UTF-16 string), data
pointers to entries (vtables / tables). `python3 samp_graph.py strings` prints function→strings.
"""
import collections
import os
import struct
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
PRE = os.path.join(ROOT, "target", "cov", "samp")
DLL = os.path.join(ROOT, "re", "samp-dll", "0.3.7-R4_fv0.3.7.3_2019-11-21", "samp.dll")


class Pe:
    def __init__(self, path):
        self.b = open(path, "rb").read()
        e = struct.unpack_from("<I", self.b, 0x3C)[0]
        n = struct.unpack_from("<H", self.b, e + 6)[0]
        opt = struct.unpack_from("<H", self.b, e + 20)[0]
        self.base = struct.unpack_from("<I", self.b, e + 24 + 28)[0]
        self.secs = []
        for i in range(n):
            o = e + 24 + opt + i * 40
            name = self.b[o:o + 8].rstrip(b"\0").decode()
            vsz, va, rsz, raw = struct.unpack_from("<IIII", self.b, o + 8)
            self.secs.append((name, self.base + va, max(vsz, rsz), raw, rsz))

    def off(self, va):
        for _, a, sz, raw, rsz in self.secs:
            if a <= va < a + sz and va - a < rsz:
                return raw + va - a
        return None

    def u32(self, va):
        o = self.off(va)
        return None if o is None else struct.unpack_from("<I", self.b, o)[0]

    def cstr(self, va, maxlen=400):
        o = self.off(va)
        if o is None:
            return None
        end = self.b.find(b"\0", o, o + maxlen)
        if end < 0:
            return None
        s = self.b[o:end]
        if len(s) >= 2 and all(32 <= c < 127 or c in (9, 10, 13) for c in s):
            return s.decode()
        # UTF-16LE
        w = []
        for i in range(o, o + maxlen, 2):
            c = struct.unpack_from("<H", self.b, i)[0]
            if c == 0:
                break
            if not (32 <= c < 127):
                return None
            w.append(chr(c))
        return "".join(w) if len(w) >= 3 else None


class Graph:
    def __init__(self, pre=PRE, dll=DLL):
        self.pe = Pe(dll)
        self.funcs = {}
        for row in open(pre + ".functions.tsv", encoding="utf-8"):
            if row.startswith("entry\t"):
                continue
            f = row.rstrip("\n").split("\t")
            self.funcs[int(f[0], 16)] = {"size": int(f[1]), "name": f[2], "thunk": f[3] == "1"}
        self.imports = collections.defaultdict(set)
        self.callees = collections.defaultdict(set)
        self.callers = collections.defaultdict(set)
        for row in open(pre + ".edges.tsv", encoding="utf-8"):
            if row.startswith("from\t"):
                continue
            a, b, _ = row.split("\t")
            a = int(a, 16)
            if b.startswith("EXT:"):
                self.imports[a].add(b[4:])
                continue
            b = int(b, 16)
            self.callees[a].add(b)
            self.callers[b].add(a)
        self.strs = collections.defaultdict(list)
        self.datarefs = collections.defaultdict(set)
        self.fnrefs = collections.defaultdict(set)
        self.ptrs = collections.defaultdict(set)
        for row in open(pre + ".refs.tsv", encoding="utf-8"):
            if row.startswith("function\t"):
                continue
            fn, site, kind, tgt, text = row.rstrip("\n").split("\t", 4)
            tgt = int(tgt, 16)
            if kind == "ptr":
                if site.startswith("0x") and ":" not in site and " " not in site:
                    self.ptrs[tgt].add(int(site, 16))
                continue
            fn = int(fn, 16)
            if kind == "str":
                self.strs[fn].append((tgt, text))
            elif kind == "fnref":
                self.fnrefs[fn].add(tgt)
            else:
                s = self.pe.cstr(tgt)
                if s is not None:
                    self.strs[fn].append((tgt, s.replace("\n", "\\n").replace("\t", "\\t")))
                else:
                    self.datarefs[fn].add(tgt)
        # data pointer → table: group ptr sites into tables (consecutive dwords)
        self.func_of = sorted(self.funcs)

    def containing(self, a):
        import bisect
        i = bisect.bisect_right(self.func_of, a) - 1
        if i < 0:
            return None
        f = self.func_of[i]
        return f if a < f + max(self.funcs[f]["size"], 1) + 0x400 else None


if __name__ == "__main__":
    g = Graph()
    if len(sys.argv) > 1 and sys.argv[1] == "strings":
        for f in sorted(g.strs):
            for a, s in g.strs[f]:
                print(f"{f:#x}\t{a:#x}\t{s}")
