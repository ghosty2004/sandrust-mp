#!/usr/bin/env python3
"""Datagram timeline of the soak's netloss cases (docs/phase3/reconnect-notes.md).

    netloss_timeline.py OUTDIR [--port P] [--dump target/debug/samp-dump] [--json]

OUTDIR is one soak gamemode directory run with SOAK_CAPTURE=1 (capture.pcapng, events.jsonl,
proxy.log). Only the client <-> proxy leg (the proxy address, default 127.0.0.2) is read. For every
netloss case it prints, relative to the moment the proxy started dropping (`drop on`), per client
socket (source port): its first datagram, the open-connection requests with the cookie they carry
(0 = none), the server's cookie / accept / refusal replies, the connection request (0x0b), the
server's 0x22, PlayerConnect (RPC 25), InitGame (139) and a rejection (RPC 130), plus the chat send
(RPC 101) on the old socket and the old socket's last datagram. Packets sent while the proxy drops
are in the capture (the client's leg) but never reached the server.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

RAW = re.compile(r"^\s*([0-9.]+) (C>S|S>C) (.*)$")


def tshark_rows(cap, host, port):
    out = subprocess.run(["tshark", "-r", cap, "-Y", "ip.addr==%s && udp.port==%d" % (host, port), "-T", "fields",
                          "-e", "frame.time_epoch", "-e", "udp.srcport", "-e", "udp.dstport"],
                         capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        p = line.split("\t")
        if len(p) == 3:
            t, sp, dp = float(p[0]), int(p[1]), int(p[2])
            rows.append((t, sp if dp == port else dp, "C>S" if dp == port else "S>C"))
    return rows


def socket_lines(cap, host, port, cport, dump, tmp):
    """samp-dump --raw of one client socket, as (epoch, dir, text)."""
    sub = os.path.join(tmp, "s-%d.pcapng" % cport)
    subprocess.run(["tshark", "-r", cap, "-Y", "ip.addr==%s && udp.port==%d && udp.port==%d" % (host, port, cport),
                    "-w", sub], capture_output=True)
    first = subprocess.run(["tshark", "-r", sub, "-c", "1", "-T", "fields", "-e", "frame.time_epoch"],
                           capture_output=True, text=True).stdout.strip()
    if not first:
        return []
    t0 = float(first)
    out = subprocess.run([dump, sub, "--port", str(port), "--raw"], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = RAW.match(line)
        if m:
            rows.append((t0 + float(m.group(1)), m.group(2), m.group(3)))
    return rows


def classify(d, text):
    if "open-connection" in text:
        return text.replace("open-connection-", "")
    m = re.match(r"#\d+ rel\d+ (?:ch\d+:\d+ )?(.*)$", text)
    body = m.group(1) if m else text
    if body.startswith("msg 0x0b"):
        return "connection-request 0x0b"
    if body.startswith("msg 0x22"):
        return "accepted 0x22"
    if body.startswith("msg 0x20"):
        return "disconnect 0x20"
    m = re.match(r"rpc +(\d+) ", body)
    if m and (d, int(m.group(1))) in ((("C>S", 25)), ("C>S", 101), ("S>C", 139), ("S>C", 130)):
        return {25: "PlayerConnect rpc 25", 101: "chat rpc 101", 139: "InitGame rpc 139",
                130: "rejected rpc 130"}[int(m.group(1))] + ("" if int(m.group(1)) != 130 else " " + body[8:60])
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--host", default=os.environ.get("SOAK_PROXY_HOST", "127.0.0.2"))
    ap.add_argument("--dump", default=None)
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    dump = a.dump or os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(root, "target")), "debug", "samp-dump")
    cap = os.path.join(a.out, "capture.pcapng")
    port = a.port
    if port is None:
        m = re.search(r"listening on \S+:(\d+)", open(os.path.join(a.out, "proxy.log")).read())
        port = int(m.group(1))
    events = [json.loads(l) for l in open(os.path.join(a.out, "events.jsonl"))]
    cases = [e for e in events if e.get("case") == "netloss" and e.get("kind") == "case-end" and "drop_on" in e]
    if not cases:
        print("no netloss case with drop times in events.jsonl")
        return 1
    rows = tshark_rows(cap, a.host, port)
    result = []
    with tempfile.TemporaryDirectory() as tmp:
        cache = {}
        for c in cases:
            t0, t1 = c["drop_on"], c["drop_off"]
            lo, hi = t0 - 5, t1 + 40
            ports = []
            for t, cp, _ in rows:
                if lo <= t <= hi and cp not in ports:
                    ports.append(cp)
            case = {"drop_on": t0, "drop_off_rel": round(t1 - t0, 3), "chat_typed_rel": round(c["chat_sent"] - t0, 3)
                    if isinstance(c.get("chat_sent"), (int, float)) else None, "sockets": []}
            for cp in ports:
                if cp not in cache:
                    cache[cp] = socket_lines(cap, a.host, port, cp, dump, tmp)
                lines = cache[cp]
                first = min(t for t, p, _ in rows if p == cp)
                last_c = max((t for t, p, d in rows if p == cp and d == "C>S" and t <= hi), default=None)
                sock = {"port": cp, "first_rel": round(first - t0, 3),
                        "last_c2s_rel": round(last_c - t0, 3) if last_c else None, "events": []}
                for t, d, text in lines:
                    if not lo <= t <= hi:
                        continue
                    k = classify(d, text)
                    if d == "C>S" and first < t0 <= t < t1 and "first_reliable_rel" not in sock \
                            and re.match(r"#\d+ rel(8|9|10) ", text):
                        # the first reliable send into the gap starts RakNet's ack timer
                        sock["first_reliable_rel"] = round(t - t0, 3)
                        if not k:
                            k = "first reliable in the gap: " + text[:60]
                        else:
                            k += " (first reliable in the gap)"
                    if k:
                        sock["events"].append((round(t - t0, 3), d, k))
                sock["events"].sort()
                case["sockets"].append(sock)
            old = next((s for s in case["sockets"] if s["first_rel"] < 0), None)
            new = [s for s in case["sockets"] if s["first_rel"] >= 0]
            if old and new and "first_reliable_rel" in old:
                case["lost_after_first_reliable"] = round(new[0]["first_rel"] - old["first_reliable_rel"], 3)
            starts = [s["first_rel"] for s in new]
            case["attempt_intervals"] = [round(b - a, 3) for a, b in zip(starts, starts[1:])]
            result.append(case)
    if a.json:
        print(json.dumps(result, indent=1))
        return 0
    for c in result:
        print("netloss: drop on at 0.000, chat typed (Enter) at %s, drop off at %.3f" % (c["chat_typed_rel"], c["drop_off_rel"]))
        for s in c["sockets"]:
            print("  socket %d: first datagram %+.3f, last C>S %s" % (s["port"], s["first_rel"], s["last_c2s_rel"]))
            for t, d, k in s["events"]:
                print("    %+9.3f %s %s" % (t, d, k))
        print("  new sockets: %d; first new socket - first reliable in the gap: %s s; attempt intervals: %s"
              % (len([s for s in c["sockets"] if s["first_rel"] >= 0]), c.get("lost_after_first_reliable"),
                 c["attempt_intervals"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
