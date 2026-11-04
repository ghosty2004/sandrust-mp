#!/usr/bin/env python3
"""Dump game memory through a control channel's `peek` command.
   peek_dump.py PORT ADDR LEN OUTFILE   (ADDR/LEN accept 0x… hex)"""
import socket
import sys

port, addr, length, out = int(sys.argv[1]), int(sys.argv[2], 0), int(sys.argv[3], 0), sys.argv[4]
s = socket.create_connection(("127.0.0.1", port), timeout=5)
f = s.makefile("rw")
data = bytearray()
a = addr
while a < addr + length:
    n = min(256, addr + length - a)
    f.write(f"peek {a:#x} {n}\n")
    f.flush()
    reply = f.readline().split()
    if len(reply) < 3 or reply[0] != "peek":
        sys.exit(f"peek {a:#x} failed: {' '.join(reply)}")
    data += bytes.fromhex("".join(reply[2:]))
    a += n
open(out, "wb").write(data)
print(f"{len(data)} bytes from {addr:#x} -> {out}")
