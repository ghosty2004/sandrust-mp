#!/usr/bin/env python3
"""Userspace UDP proxy with controlled loss for RakNet resend measurements (NET-6).

    lossproxy.py LISTEN_HOST PORT SERVER_HOST CONTROL_FILE LOG_FILE

Listens on LISTEN_HOST:PORT (the address the client connects to) and forwards every datagram to
SERVER_HOST:PORT (the server bound to another loopback address with the same port, so the
port-keyed client cipher stays valid), one upstream socket per client source port.

CONTROL_FILE holds one rule line, re-read when it changes:
    off                     forward everything (default)
    drop-cs-reliable N      drop every N-th client→server datagram that carries a reliable
                            message transmitted for the first time (resends pass)
    drop-sc-reliable N      the same for server→client datagrams
    drop-cs-repeat K        drop client→server datagrams carrying a reliable message while it
                            has been transmitted at most K times (first send + K−1 resends)
Every drop is logged to LOG_FILE as JSON: {"t": epoch, "dir": "C>S", "nums": [...], "len": n}.
No root, no tc: plain sockets.
"""
import json
import os
import select
import socket
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import netcap  # noqa: E402


def reliable_new(body, seen):
    """Message numbers of reliable messages in this datagram not seen before."""
    try:
        _, msgs = netcap.decode_datagram(body)
    except (EOFError, ValueError):
        return []
    out = []
    for m in msgs:
        if m["rel"] in (8, 9, 10) and m["num"] not in seen:
            seen.add(m["num"])
            out.append(m["num"])
    return out


def reliable_nums(body):
    try:
        _, msgs = netcap.decode_datagram(body)
    except (EOFError, ValueError):
        return []
    return [m["num"] for m in msgs if m["rel"] in (8, 9, 10)]


def main():
    if len(sys.argv) != 6:
        print(__doc__, file=sys.stderr)
        return 2
    lhost, port, shost, ctl, logf = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
    lsock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    lsock.bind((lhost, port))
    up = {}      # client addr -> upstream socket
    back = {}    # upstream fileno -> client addr
    seen = {}    # (client addr, dir) -> set of reliable numbers
    count = {}   # (client addr, dir) -> matching datagrams so far
    txs = {}     # client addr -> {reliable number: transmissions seen} (drop-cs-repeat)
    rule, rule_mtime = ("off", 0), None
    log = open(logf, "a", buffering=1)
    last_ctl = 0.0
    print("lossproxy %s:%d -> %s:%d" % (lhost, port, shost, port), flush=True)
    while True:
        now = time.time()
        if now - last_ctl > 0.05:
            last_ctl = now
            try:
                m = os.stat(ctl).st_mtime
                if m != rule_mtime:
                    rule_mtime = m
                    parts = open(ctl).read().split()
                    rule = (parts[0], int(parts[1])) if len(parts) >= 2 else ("off", 0)
                    log.write(json.dumps({"t": now, "rule": rule}) + "\n")
                    count.clear()
                    txs.clear()
            except (OSError, ValueError):
                pass
        socks = [lsock] + list(up.values())
        r, _, _ = select.select(socks, [], [], 0.05)
        for s in r:
            data, addr = s.recvfrom(4096)
            if s is lsock:
                d, client = "C>S", addr
                body = netcap.decrypt(data, port)
            else:
                d, client = "S>C", back[s.fileno()]
                body = data
            key = (client, d)
            want = {"C>S": "drop-cs-reliable", "S>C": "drop-sc-reliable"}[d]
            if body is not None and len(body) >= 2 and rule[0] == "drop-cs-repeat" and d == "C>S":
                tx = txs.setdefault(client, {})
                nums = reliable_nums(body)
                for n in nums:
                    tx[n] = tx.get(n, 0) + 1
                if nums and any(tx[n] <= rule[1] for n in nums):
                    log.write(json.dumps({"t": time.time(), "dir": d, "cport": client[1], "nums": nums,
                                          "len": len(data), "rule": rule[0]}) + "\n")
                    continue
            if body is not None and len(body) >= 2:
                nums = reliable_new(body, seen.setdefault(key, set()))
                if nums and rule[0] == want and rule[1] > 0:
                    count[key] = count.get(key, 0) + 1
                    if count[key] % rule[1] == 0:
                        log.write(json.dumps({"t": time.time(), "dir": d, "cport": client[1],
                                              "nums": nums, "len": len(data)}) + "\n")
                        continue
            if s is lsock:
                u = up.get(client)
                if u is None:
                    u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                    u.bind((lhost, 0))
                    u.connect((shost, port))
                    up[client] = u
                    back[u.fileno()] = client
                u.send(data)
            else:
                lsock.sendto(data, client)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(0)
