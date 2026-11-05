#!/usr/bin/env python3
"""tools/e2e/m10.sh helper: checks a session's loopback capture against the original's measured
connection-lifecycle timing (docs/phase3/trace-notes.md "Connection lifecycle", TRACE-8).

    m10_net.py SESSION OUTDIR SERVER_PORT

Reads OUTDIR/udp.txt (tshark: epoch, src port, dst port, UDP length). A "socket" is one client
source port (one connection attempt). Prints a summary; exit 1 on a mismatch.
"""
import sys

REQ_LEN = 12  # UDP length of the encrypted 3-byte open-connection request (measured)


def main():
    session, out, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
    rows = []
    for line in open(out + "/udp.txt"):
        p = line.split()
        if len(p) == 4:
            rows.append((float(p[0]), int(p[1]), int(p[2]), int(p[3])))
    socks = {}
    for t, s, d, n in rows:
        c = s if d == port else d
        e = socks.setdefault(c, {"first": t, "c2s": [], "s2c": []})
        (e["c2s"] if d == port else e["s2c"]).append((t, n))
    order = sorted(socks, key=lambda c: socks[c]["first"])
    for c in order:
        e = socks[c]
        reqs = [t for t, n in e["c2s"] if n == REQ_LEN]
        print("[m10-net] %s socket %d: first %.3f, %d C>S (%d requests), %d S>C" % (
            session, c, e["first"] - socks[order[0]]["first"], len(e["c2s"]), len(reqs), len(e["s2c"])))
    bad = []

    def requests(c):
        return [t for t, n in socks[c]["c2s"] if n == REQ_LEN]

    def unanswered(c):
        return not socks[c]["s2c"]

    def check_unanswered_attempts(cs):
        for c in cs:
            r = requests(c)
            gaps = [b - a for a, b in zip(r, r[1:])]
            if len(r) != 6 or any(not 0.95 <= g <= 1.1 for g in gaps):
                bad.append("socket %d: want 6 requests 1.0 s apart, got %d %s" % (c, len(r), ["%.2f" % g for g in gaps]))

    def cadence(cs, lo, hi, what):
        firsts = [socks[c]["first"] for c in cs]
        gaps = [b - a for a, b in zip(firsts, firsts[1:])]
        print("[m10-net] %s: %s cadence %s" % (session, what, ["%.2f" % g for g in gaps]))
        for g in gaps:
            if not lo <= g <= hi:
                bad.append("%s cadence %.2f s outside %.2f..%.2f" % (what, g, lo, hi))
        return gaps

    if session == "cfg":
        main_sock = order[-1] if order else None
        e = socks[main_sock]
        last_s2c = e["s2c"][-1][0]
        after = [t for t, n in e["c2s"] if t > last_s2c]
        print("[m10-net] kick: client datagrams after the server's last one: %s" % ["+%.1f ms" % ((t - last_s2c) * 1000) for t in after])
        if len(after) != 1 or after[0] - last_s2c > 0.05:
            bad.append("kick: want exactly one client datagram (the ack) within 50 ms after the server's last, got %d" % len(after))
        if len(order) != 1:
            bad.append("kick: %d sockets (a reconnect?)" % len(order))
    elif session == "lost":
        old = order[0]
        last_s2c = max(t for t, n in socks[old]["s2c"] if t < socks[order[1]]["first"]) if len(order) > 1 else None
        if last_s2c is None:
            bad.append("no new socket after the server stopped")
        else:
            dt = socks[order[1]]["first"] - last_s2c
            print("[m10-net] lost: first new attempt %.2f s after the server's last packet (original 12.0 s + reconnect at once)" % dt)
            if not 11.8 <= dt <= 12.8:
                bad.append("lost connection detected after %.2f s (want ~12.0)" % dt)
            retry = order[1:]
            cadence(retry, 5.95, 6.25, "no-answer retry")
            check_unanswered_attempts([c for c in retry if unanswered(c)])
            if unanswered(retry[-1]):
                bad.append("the last attempt was not answered (no resume?)")
    elif session == "full":
        ours = order[1:]  # order[0] is samp-bot
        full = ours[:-1]
        if len(full) < 2:
            bad.append("want at least 2 'full' attempts, got %d" % len(full))
        cadence(ours, 2.95, 3.2, "server-full retry")
        for c in full:
            if len(socks[c]["s2c"]) < 1:
                bad.append("full attempt %d got no answer" % c)
    elif session == "nosrv":
        pre = [c for c in order if unanswered(c)]
        if len(pre) < 2:
            bad.append("want at least 2 unanswered attempts, got %d" % len(pre))
        check_unanswered_attempts(pre)
        cadence(order, 5.95, 6.25, "no-answer retry")
        answered = [c for c in order if not unanswered(c)]
        if len(answered) != 1:
            bad.append("want exactly one answered attempt (wrong password, no retry), got %d" % len(answered))
        elif answered[0] != order[-1]:
            bad.append("an attempt after the wrong-password answer")
    for b in bad:
        print("[m10-net] FAIL %s: %s" % (session, b))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
