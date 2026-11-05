#!/usr/bin/env python3
"""tools/e2e/m10.sh helper: checks a session's loopback capture against the original's measured
connection-lifecycle timing (docs/phase3/trace-notes.md "Connection lifecycle", TRACE-8).

    m10_net.py SESSION OUTDIR SERVER_PORT

Reads OUTDIR/udp.txt (tshark: epoch, src port, dst port, UDP length) and OUTDIR/net-PORT.raw
(samp-dump --raw of one client socket, times relative to its first packet). A "socket" is one
client source port (one connection attempt). Prints a summary; exit 1 on a mismatch.
"""
import datetime
import json
import os
import re
import sys

REQ_LEN = 12  # UDP length of the encrypted 3-byte open-connection request (measured)
RAW = re.compile(r"^\s*([0-9.]+) (C>S|S>C) (.*)$")


def raw_lines(out, port, first):
    """(epoch, dir, text) of samp-dump --raw for one socket."""
    path = "%s/net-%d.raw" % (out, port)
    if not os.path.exists(path):
        return []
    rows = []
    for line in open(path):
        m = RAW.match(line)
        if m:
            rows.append((first + float(m.group(1)), m.group(2), m.group(3)))
    return rows


def reliable(text):
    """A message line with reliability 8/9/10 (RakNet RELIABLE, _ORDERED, _SEQUENCED)."""
    return re.match(r"#\d+ rel(8|9|10) ", text) is not None


def log_times(out, pattern):
    """Epoch times of samp-rs.log lines matching pattern."""
    ts = []
    try:
        for line in open(out + "/samp-rs.log", errors="replace"):
            if re.search(pattern, line):
                m = re.match(r"\[(\d+-\d+-\d+ \d+:\d+:\d+\.\d+) UTC", line)
                if m:
                    t = datetime.datetime.strptime(m.group(1), "%Y-%m-%d %H:%M:%S.%f")
                    ts.append(t.replace(tzinfo=datetime.timezone.utc).timestamp())
    except OSError:
        pass
    return ts


def cfg_keys(out):
    try:
        return [l.split("=", 1)[0] for l in open(out + "/sa-mp.cfg.end", errors="replace").read().replace("\r", "").split("\n") if "=" in l]
    except OSError:
        return None


# The original's replies to m10.sh's request set (run orig-20261105-035923; requests 3FFFFF, 856E01,
# offset 257 and count 1 got none). Where the answering client's own memory legitimately differs,
# only a reply is required, and the values are printed: "own" = 0x45 reads the client's own module
# (samp.dll there, our DLL here); "patch" = gta_sa.exe bytes both clients rewrite with a call into
# their own module (0x53E981, CGame::Process call); "heap" = ranges holding heap pointers
# (CVehicleModelInfo +0x1C RwObject…, CColModel +0x2C data pointer), different per process.
ORIG_CHECKS = [
    ("0x2", "0x10000212", "0x1"), ("0x5", "0x400000", "0x84"), ("0x5", "0x400000", "0x4C"),
    ("0x5", "0x401000", "0x40"), ("0x5", "0x53E981", "patch"), ("0x5", "0x856E00", "0x0"),
    ("0x45", "0x0", "0x84"), ("0x45", "0x1000", "own"), ("0x46", "0x19B", "0x88"), ("0x46", "0x190", "heap"),
    ("0x46", "0x0", "0x0"), ("0x46", "0x4E21", "0x0"), ("0x47", "0x19B", "0xC"), ("0x47", "0x190", "heap"),
    ("0x47", "0x539", "0x88"), ("0x48", "time", "0x0"),
]


def client_checks(out, bad):
    try:
        got = [l.split() for l in open(out + "/checks.txt")]
    except OSError:
        got = []
    got = [(g[1], g[3], g[5]) for g in got if len(g) >= 6]
    print("[m10-net] ClientCheck: %d replies (original 16)" % len(got))
    if len(got) != len(ORIG_CHECKS):
        bad.append("ClientCheck: %d replies, want %d: %s" % (len(got), len(ORIG_CHECKS), got))
        return
    for (t, a, r), (wt, wa, wr) in zip(got, ORIG_CHECKS):
        ok = t == wt and (a == wa or (wa == "time" and int(a, 16) & 0xF0000000 == 0x30000000)) and (r == wr or wr in ("own", "patch", "heap"))
        print("[m10-net]   type %s arg %s: ours %s, original %s%s" % (t, a, r, wr, "" if ok else "  <-- differs"))
        if not ok:
            bad.append("ClientCheck type %s arg %s: ours %s, original %s" % (t, a, r, wr))


def quit_checks(out, sock, socks, bad):
    """`/quit` (static A 0x100c3ec0 / 0x100c5030 / 0x10009880; capture orig-20261105-003749 net-42564):
    the disconnection notification UNRELIABLE (rel6) as the socket's last datagram, the netgame deleted
    more than 1000 ms later in the exit frame: the car's occupant warped out (0x0362), the car deleted."""
    if sock is None:
        bad.append("quit: no socket")
        return
    raw = raw_lines(out, sock, socks[sock]["first"])
    c2s = [(t, x) for t, d, x in raw if d == "C>S" and not x.startswith("acks")]
    last_c2s = max(t for t, n in socks[sock]["c2s"])
    notif = [(t, x) for t, x in c2s if "msg 0x20" in x]
    print("[m10-net] quit: %s" % notif)
    if len(notif) != 1 or not re.match(r"#\d+ rel6 msg 0x20", notif[0][1]):
        bad.append("quit: want one UNRELIABLE (rel6) 0x20, got %s" % notif)
        return
    if last_c2s > notif[0][0] + 0.0005:
        bad.append("quit: a client datagram %.3f s after the 0x20" % (last_c2s - notif[0][0]))
    q = log_times(out, r"client: /quit in netgame state 5")
    ex = log_times(out, r"\] exit$")
    if not q or not ex:
        bad.append("quit: log lines missing (quit %s, exit %s)" % (q, ex))
        return
    print("[m10-net] quit: 0x20 %+.1f ms after /quit, exit %.3f s after /quit" % ((notif[0][0] - q[0]) * 1000, ex[0] - q[0]))
    if not 0 <= notif[0][0] - q[0] <= 0.05:
        bad.append("quit: 0x20 not sent with the /quit")
    if not 1.0 <= ex[0] - q[0] <= 1.2:
        bad.append("quit: exit %.3f s after /quit (want > 1.0)" % (ex[0] - q[0]))
    # Exit frame (tracer): the car's occupant out (0x0362), then the car deleted (0x00a6).
    ops = []
    try:
        lines = open(out + "/trace.jsonl", errors="replace").read().splitlines()
    except OSError:
        lines = []
    recs = []
    for l in lines[-4000:]:
        try:
            recs.append(json.loads(l))
        except ValueError:
            pass
    frames = [r.get("frame") for r in recs if r.get("fn") == "scm" and r.get("args", {}).get("op") == "0x00a6"]
    if not frames:
        bad.append("quit: no DELETE_CAR in the trace's last records")
        return
    last = frames[-1]
    ops = [r["args"]["op"] for r in recs if r.get("frame") == last and r.get("fn") == "scm"
           and r["args"].get("op") in ("0x0362", "0x00a6", "0x0164")]
    print("[m10-net] quit: exit frame %s ops %s" % (last, ops))
    if "0x0362" not in ops or ops.index("0x0362") > len(ops) - 1 - ops[::-1].index("0x00a6"):
        bad.append("quit: want 0x0362 (occupant out) before 0x00a6 (car deleted) in the exit frame, got %s" % ops)


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
            print("[m10-net] lost: first new attempt %.2f s after the server's last packet (original 12.48: depends on the keep-alive phase)" % dt)
            # RakNet's ack timer (m10-notes "M10-P"): it starts with the first reliable message sent
            # while none awaits an ack; the connection is dead > 10 s later. Original
            # orig-20261105-010108: keep-alive at 112.508, new socket at 122.558 (10.05 s).
            raw = raw_lines(out, old, socks[old]["first"])
            unacked = [t for t, d, x in raw if d == "C>S" and t > last_s2c and reliable(x)]
            if not unacked:
                bad.append("lost: no reliable client message after the server stopped")
            else:
                dt = socks[order[1]]["first"] - unacked[0]
                print("[m10-net] lost: first new attempt %.3f s after the first unacked reliable message (original 10.050)" % dt)
                if not 10.0 <= dt <= 10.12:
                    bad.append("lost connection %.3f s after the first unacked reliable send (want 10.0..10.12)" % dt)
            # Keep-alives: > 5 s after the last reliable send, 15 s after a keep-alive (original 5.00 / 15.00).
            rel = [(t, "msg 0x17 17" in x) for t, d, x in raw if d == "C>S" and reliable(x)]
            gaps = []
            for (t0, k0), (t1, k1) in zip(rel, rel[1:]):
                # (the join sequence's own keep-alive follows the 0x1e at once, handshake.md §1)
                if k1 and t1 - t0 > 1.0:
                    gaps.append(t1 - t0)
                    want = (15.0, 15.08) if k0 else (5.0, 5.08)
                    if not want[0] <= t1 - t0 <= want[1]:
                        bad.append("keep-alive %.3f s after the previous reliable send (%s; want %.2f..%.2f)" % (
                            t1 - t0, "keep-alive" if k0 else "other", want[0], want[1]))
            print("[m10-net] lost: keep-alive gaps %s" % ["%.3f" % g for g in gaps])
            if not gaps:
                bad.append("no keep-alive seen")
            retry = order[1:]
            cadence(retry, 5.95, 6.25, "no-answer retry")
            check_unanswered_attempts([c for c in retry if unanswered(c)])
            if unanswered(retry[-1]):
                bad.append("the last attempt was not answered (no resume?)")
        # audioproxyoff is appended only after BASS started (A 0x10066480): last key, written after
        # the client logged "audio: BASS ready" (the failure branch has no e2e case: GTA itself does
        # not start under Wine without an audio device; unit test ur_9_startup_creates_…).
        keys = cfg_keys(out)
        ready = log_times(out, r"audio: BASS ready")
        print("[m10-net] lost: sa-mp.cfg keys at the end %s, BASS ready %s" % (keys, bool(ready)))
        if keys is None or keys[-1:] != ["audioproxyoff"] or not ready:
            bad.append("audioproxyoff not appended after BASS started: %s" % keys)
    elif session == "full":
        ours = order[1:]  # order[0] is samp-bot
        full = ours[:-1]
        if len(full) < 2:
            bad.append("want at least 2 'full' attempts, got %d" % len(full))
        # Original 3.03-3.04 (37.18 → 40.22 → 43.26 → 46.30 → 49.33): > 3000 ms after the previous
        # Connect returned + frame + Connect's own 25 ms. Single frame stalls of the loading world
        # show as outliers; the median must match.
        gaps = cadence(ours, 3.0, 3.6, "server-full retry")
        med = sorted(gaps)[len(gaps) // 2] if gaps else 0
        print("[m10-net] full: median %.3f (original 3.035)" % med)
        if not 3.02 <= med <= 3.06:
            bad.append("server-full retry median %.3f outside 3.02..3.06" % med)
        for c in full:
            if len(socks[c]["s2c"]) < 1:
                bad.append("full attempt %d got no answer" % c)
        quit_checks(out, ours[-1] if ours else None, socks, bad)
        client_checks(out, bad)
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
