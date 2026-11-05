#!/usr/bin/env python3
"""Datagram-level decoder for SA:MP 0.3.7 captures (NET-6 / F2).

Reads a pcap/pcapng with tshark, removes the client→server cipher and decodes every RakNet
datagram: acks, messages (number, reliability, ordering channel/index, split info, bit length,
kind). Used by tools/diff/net_report.py and as a stand-alone dumper:

    netcap.py CAPTURE --port 7777 [--client-port N] [--jsonl]

Spec: docs/re/handshake.md §2 (cipher), §3 (RakNet framing). The substitution table is read
from crates/samp-proto/src/sbox.rs (single source of the interop constant).
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _load_sbox():
    src = open(os.path.join(ROOT, "crates/samp-proto/src/sbox.rs")).read()
    body = src[src.index("["):]
    vals = [int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", body)]
    vals = vals[:256]
    assert len(vals) == 256
    inv = [0] * 256
    for i, v in enumerate(vals):
        inv[v] = i
    return inv


SBOX_INV = _load_sbox()


def decrypt(wire, port):
    """Client→server de-obfuscation; returns plaintext or None on a checksum mismatch."""
    if not wire:
        return None
    key = (port ^ 0xCCCC) & 0xFF
    out = bytearray()
    for i, b in enumerate(wire[1:]):
        out.append(SBOX_INV[b ^ key if i % 2 == 1 else b])
    s = 0
    for b in out:
        s ^= b & 0xAA
    return bytes(out) if s == wire[0] else None


class Bits:
    def __init__(self, data, nbits=None):
        self.d = data
        self.n = len(data) * 8 if nbits is None else nbits
        self.p = 0

    def remaining(self):
        return self.n - self.p

    def bit(self):
        if self.p >= self.n:
            raise EOFError
        v = (self.d[self.p >> 3] >> (7 - (self.p & 7))) & 1
        self.p += 1
        return v

    def bits(self, k):
        v = 0
        for _ in range(k):
            v = (v << 1) | self.bit()
        return v

    def byte(self):
        return self.bits(8)

    def u16(self):
        a = self.byte()
        b = self.byte()
        return a | (b << 8)

    def u32(self):
        return self.u16() | (self.u16() << 16)

    def compressed(self, size):
        out = [0] * size
        for k in range(size - 1, 0, -1):
            if not self.bit():
                for i in range(k + 1):
                    out[i] = self.byte()
                return int.from_bytes(bytes(out), "little")
        out[0] = self.bits(4) if self.bit() else self.bits(8)
        return int.from_bytes(bytes(out), "little")

    def align(self):
        self.p = (self.p + 7) & ~7

    def aligned_bytes(self, k):
        self.align()
        if self.p + 8 * k > self.n:
            raise EOFError
        s = self.p >> 3
        self.p += 8 * k
        return self.d[s:s + k]


def kind_of(data, bits):
    """Short description of a message payload: 'rpc N', 'sync N', 'msg 0xNN'."""
    body = data
    if len(body) >= 5 and body[0] == 0x28:
        body = body[5:]
    if not body:
        return "empty", None
    first = body[0]
    if first == 0x14:
        try:
            r = Bits(body)
            r.byte()
            rid = r.byte()
            nb = r.compressed(4)
            return "rpc %d" % rid, nb
        except EOFError:
            return "rpc ?", None
    if 0xC8 <= first <= 0xD4:
        return "sync %d" % first, None
    return "msg 0x%02x" % first, None


def decode_datagram(body):
    r = Bits(body)
    acks = []
    msgs = []
    if len(body) < 2:
        raise ValueError("short")
    if r.bit():
        cnt = r.compressed(2)
        for _ in range(cnt):
            single = r.bit()
            mn = r.u16()
            mx = mn if single else r.u16()
            acks.append((mn, mx))
    while r.remaining() >= 16:
        num = r.u16()
        rel = r.bits(4)
        if rel < 6 or rel > 10:
            raise ValueError("reliability %d" % rel)
        ch = idx = None
        if rel in (7, 9, 10):
            ch = r.bits(5)
            idx = r.u16()
        split = None
        if r.bit():
            split = (r.u16(), r.compressed(4), r.compressed(4))
        nbits = r.compressed(2)
        data = r.aligned_bytes((nbits + 7) // 8)
        kind, rpc_bits = kind_of(data, nbits) if split is None or split[1] == 0 else ("split", None)
        msgs.append({
            "num": num, "rel": rel, "ch": ch, "idx": idx, "split": split,
            "bits": nbits, "kind": kind, "rpc_bits": rpc_bits, "data": data.hex(),
        })
    return acks, msgs


UNCONNECTED = {
    (0, 0x18, 3): "open-connection-request",
    (1, 0x1A, 3): "open-connection-cookie",
}


def read(path, port, client_port=None):
    """Datagram records (dicts) in capture order: t (s, epoch), dir ('C>S'/'S>C'), cport, len,
    acks, msgs, unconnected (name or None), bad (str or None)."""
    out = subprocess.run(
        ["tshark", "-r", path, "-T", "fields", "-E", "separator=\t",
         "-e", "frame.time_epoch", "-e", "udp.srcport", "-e", "udp.dstport", "-e", "udp.payload",
         "-Y", "udp.port==%d" % port],
        capture_output=True, text=True, check=True).stdout
    recs = []
    for line in out.splitlines():
        f = line.split("\t")
        if len(f) < 4 or not f[3]:
            continue
        t, sp, dp, hx = float(f[0]), int(f[1]), int(f[2]), f[3].replace(":", "")
        wire = bytes.fromhex(hx)
        if dp == port:
            d, cport = "C>S", sp
        elif sp == port:
            d, cport = "S>C", dp
        else:
            continue
        if client_port is not None and cport != client_port:
            continue
        rec = {"t": t, "dir": d, "cport": cport, "len": len(wire), "acks": [], "msgs": [],
               "unconnected": None, "bad": None}
        body = wire
        if d == "C>S":
            body = decrypt(wire, port)
            if body is None:
                rec["bad"] = "checksum"
                recs.append(rec)
                continue
        rec["plain_len"] = len(body)
        first = body[0] if body else None
        if d == "C>S" and first == 0x18 and len(body) == 3:
            rec["unconnected"] = "open-connection-request"
        elif d == "S>C" and first == 0x1A and len(body) == 3:
            rec["unconnected"] = "open-connection-cookie"
        elif d == "S>C" and first in (0x19, 0x1D, 0x1F, 0x24) and len(body) <= 2:
            rec["unconnected"] = "open-connection-reply 0x%02x" % first
        else:
            try:
                rec["acks"], rec["msgs"] = decode_datagram(body)
            except (EOFError, ValueError) as e:
                rec["bad"] = "undecodable: %s" % e
        recs.append(rec)
    return recs


def client_ports(recs):
    seen = []
    for r in recs:
        if r["cport"] not in seen:
            seen.append(r["cport"])
    return seen


def main():
    args = sys.argv[1:]
    port = 7777
    cport = None
    jsonl = False
    path = None
    while args:
        a = args.pop(0)
        if a == "--port":
            port = int(args.pop(0))
        elif a == "--client-port":
            cport = int(args.pop(0))
        elif a == "--jsonl":
            jsonl = True
        else:
            path = a
    if not path:
        print(__doc__, file=sys.stderr)
        return 2
    recs = read(path, port, cport)
    t0 = recs[0]["t"] if recs else 0
    for r in recs:
        if jsonl:
            print(json.dumps(r))
            continue
        head = "%9.4f %s %5d %4dB" % (r["t"] - t0, r["dir"], r["cport"], r["len"])
        if r["unconnected"] or r["bad"]:
            print(head, r["unconnected"] or r["bad"])
            continue
        parts = []
        if r["acks"]:
            parts.append("acks " + ",".join("%d" % a if a == b else "%d-%d" % (a, b) for a, b in r["acks"]))
        for m in r["msgs"]:
            s = "#%d r%d" % (m["num"], m["rel"])
            if m["ch"] is not None:
                s += " c%d:%d" % (m["ch"], m["idx"])
            if m["split"]:
                s += " split%d/%d/%d" % m["split"]
            s += " %s %db" % (m["kind"], m["bits"])
            parts.append(s)
        print(head, " | ".join(parts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
