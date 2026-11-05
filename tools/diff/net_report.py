#!/usr/bin/env python3
"""F2 network diff: original vs ours, per scenario window and at the RakNet level (NET-6).

    net_report.py ORIG_DIR OURS_DIR DECISIONS.tsv ROADMAP.md [--out report.md]

Each DIR holds net.pcapng (client leg only), marks.tsv (scenario windows "NAME start end",
input events "@NAME t") and loss.jsonl (datagrams the proxy dropped) from tools/diff/network.sh.
Exit 0 when every check matches or its mismatch is listed in DECISIONS.tsv
(scenario<TAB>check<TAB>key<TAB>decision id<TAB>note; fnmatch patterns) and the decision id
appears in ROADMAP.md.
"""
import fnmatch
import json
import os
import statistics
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import netcap  # noqa: E402

PERIODIC = {"sync %d" % i for i in range(200, 213)} | {"msg 0x09", "msg 0x17"}
# Tolerances.
COUNT_REL_TOL = 0.35      # periodic message counts per window
COUNT_ABS_TOL = 2
CADENCE_REL_TOL = 0.25    # median interval of periodic messages
CADENCE_ABS_TOL = 0.015   # s
TRIGGER_TOL = 0.25        # s, input event → first message
ACK_P50_TOL = 0.004       # s
ACK_P90_TOL = 0.010       # s
STANDALONE_TOL = 0.15     # share of ack datagrams without messages
PACKING_TOL = 0.25        # mean messages per datagram
RESEND_TOL = 0.030        # s, first transmission → resend


def detect_port(pcap):
    out = subprocess.run(["tshark", "-r", pcap, "-T", "fields", "-e", "udp.dstport"],
                         capture_output=True, text=True).stdout.split()
    for p in (7790, 7800, 7777, 7795, 7820):
        if str(p) in out:
            return p
    raise SystemExit("no SA:MP port in %s" % pcap)


class Session:
    def __init__(self, d):
        self.dir = d
        pcap = os.path.join(d, "net.pcapng")
        self.port = detect_port(pcap)
        self.recs = netcap.read(pcap, self.port)
        self.windows = {}
        self.events = []
        for line in open(os.path.join(d, "marks.tsv")):
            f = line.rstrip("\n").split("\t")
            if f[0].startswith("@"):
                self.events.append((f[0][1:], float(f[1])))
            elif len(f) >= 3 and f[0] != "fail":
                self.windows[f[0]] = (float(f[1]), float(f[2]))
        self.drops = []
        lp = os.path.join(d, "loss.jsonl")
        if os.path.exists(lp):
            for line in open(lp):
                j = json.loads(line)
                if "dir" in j:
                    self.drops.append(j)
        # The game's socket = the client port with the most datagrams.
        n = {}
        for r in self.recs:
            n[r["cport"]] = n.get(r["cport"], 0) + 1
        self.main = max(n, key=n.get) if n else None
        self.msgs = []      # C>S first transmissions: dict(t, key, kind, rel, ch, bits, num, dg)
        self.resends = []   # (num, first t, resend t)
        self.tx = {}        # reliable number -> every transmission time
        first = {}
        for i, r in enumerate(self.recs):
            if r["dir"] != "C>S" or r["cport"] != self.main:
                continue
            for m in r["msgs"]:
                if m["rel"] in (8, 9, 10):
                    self.tx.setdefault(m["num"], []).append(r["t"])
                    if m["num"] in first:
                        self.resends.append((m["num"], first[m["num"]], r["t"]))
                        continue
                    first[m["num"]] = r["t"]
                key = "%s rel%d" % (m["kind"], m["rel"]) + ("" if m["ch"] is None else " ch%d" % m["ch"])
                self.msgs.append({"t": r["t"], "key": key, "kind": m["kind"], "bits": m["bits"],
                                  "rpc_bits": m["rpc_bits"], "num": m["num"], "dg": i})

    def in_window(self, name):
        a, b = self.windows[name]
        return [m for m in self.msgs if a <= m["t"] <= b]

    def events_in(self, name):
        a, b = self.windows[name]
        return [(n, t) for n, t in self.events if a <= t <= b]


def is_event(key):
    return key.split(" rel")[0] not in PERIODIC and not key.startswith("msg 0x06")


def median_gap(ts):
    if len(ts) < 4:
        return None
    return statistics.median(b - a for a, b in zip(ts, ts[1:]))


def quantiles(ts, n=9):
    g = sorted(b - a for a, b in zip(ts, ts[1:]))
    return [g[min(len(g) - 1, int(i * len(g) / (n + 1)))] for i in range(1, n + 1)]


def same_cadence(ta, tb):
    """Interval distributions alike: the median gaps agree, or (for bimodal, event-driven
    sends where the median flips between modes) the deciles agree in the median."""
    ga, gb = median_gap(ta), median_gap(tb)
    tol = max(CADENCE_ABS_TOL, CADENCE_REL_TOL * ga)
    if abs(ga - gb) <= tol:
        return True
    qa, qb = quantiles(ta), quantiles(tb)
    return statistics.median(abs(x - y) for x, y in zip(qa, qb)) <= tol


def fmt(v):
    if isinstance(v, float):
        return "%.4f" % v
    return str(v)


class Report:
    def __init__(self, decisions, roadmap):
        self.rows = []
        self.decisions = []
        for line in open(decisions):
            if not line.strip() or line.startswith("#"):
                continue
            f = line.rstrip("\n").split("\t")
            self.decisions.append(f + [""] * (5 - len(f)))
        self.roadmap = open(roadmap).read()

    def check(self, scen, check, key, orig, ours, ok):
        status = "ok" if ok else "DIFF"
        dec = ""
        if not ok:
            for s, c, k, d, _note in self.decisions:
                if fnmatch.fnmatch(scen, s) and fnmatch.fnmatch(check, c) and fnmatch.fnmatch(key, k):
                    if d and d in self.roadmap:
                        status, dec = "accepted", d
                    else:
                        status, dec = "DIFF", "%s (not in ROADMAP)" % d
                    break
        self.rows.append((scen, check, key, fmt(orig), fmt(ours), status, dec))


def window_checks(rep, o, u, name):
    mo, mu = o.in_window(name), u.in_window(name)
    keys = sorted({m["key"] for m in mo} | {m["key"] for m in mu})
    for k in keys:
        a = [m for m in mo if m["key"] == k]
        b = [m for m in mu if m["key"] == k]
        if is_event(k):
            rep.check(name, "count", k, len(a), len(b), len(a) == len(b))
            if a and b:
                fa = sorted({m["rpc_bits"] if m["rpc_bits"] is not None else m["bits"] for m in a})
                fb = sorted({m["rpc_bits"] if m["rpc_bits"] is not None else m["bits"] for m in b})
                rep.check(name, "bits", k, fa, fb, fa == fb)
        else:
            tol = max(COUNT_ABS_TOL, COUNT_REL_TOL * max(len(a), len(b)))
            # Sent by one client only is a difference, whatever the count.
            # (rare periodic messages — 203 idle aim every ~10 s, pongs every 5 s — fall in or out of
            # a window by chance: there the session totals below decide).
            ok = abs(len(a) - len(b)) <= tol and not (min(len(a), len(b)) == 0 and max(len(a), len(b)) >= 3)
            rep.check(name, "count", k, len(a), len(b), ok)
            if a and b:
                ba = statistics.mode(m["bits"] for m in a)
                bb = statistics.mode(m["bits"] for m in b)
                rep.check(name, "bits", k, ba, bb, ba == bb)
            ga, gb = median_gap([m["t"] for m in a]), median_gap([m["t"] for m in b])
            # A cadence needs 4 messages on both sides; when one side has 4 and the other 3 (one rare
            # race-gated send, e.g. an unchanged 203 passing the shared 501 ms gate), the count check
            # above decides. Two runs of the original differ the same way.
            if (ga is None) != (gb is None) and max(len(a), len(b)) <= 4:
                ga = gb = None
            if ga is not None or gb is not None:
                ok = ga is not None and gb is not None and same_cadence([m["t"] for m in a], [m["t"] for m in b])
                rep.check(name, "cadence", k, ga if ga is not None else "-", gb if gb is not None else "-", ok)
    sa = [m["key"] for m in mo if is_event(m["key"])]
    sb = [m["key"] for m in mu if is_event(m["key"])]
    rep.check(name, "order", "events", " > ".join(sa) or "-", " > ".join(sb) or "-", sa == sb)
    # Trigger latency: input event → first event message of the window after it.
    eo, eu = dict(o.events_in(name)), dict(u.events_in(name))
    for ev in sorted(set(eo) & set(eu)):
        def first_after(ms, t):
            for m in ms:
                if m["t"] >= t and is_event(m["key"]):
                    return m
            return None
        fa, fb = first_after(mo, eo[ev]), first_after(mu, eu[ev])
        if fa is None and fb is None:
            continue
        la = (fa["key"], fa["t"] - eo[ev]) if fa else ("-", None)
        lb = (fb["key"], fb["t"] - eu[ev]) if fb else ("-", None)
        ok = fa is not None and fb is not None and la[0] == lb[0] and abs(la[1] - lb[1]) <= TRIGGER_TOL
        rep.check(name, "trigger", ev, "%s +%s" % (la[0], fmt(la[1])), "%s +%s" % (lb[0], fmt(lb[1])), ok)


def ack_stats(s):
    """Latency from each S>C datagram with reliable messages to the first C>S datagram acking one
    of them; share of ack datagrams without messages; mean messages per datagram."""
    lat = []
    pending = []
    standalone = with_msgs = 0
    per_dg = []
    sizes = []
    for r in s.recs:
        if r["cport"] != s.main or r["unconnected"] or r["bad"]:
            continue
        if r["dir"] == "S>C":
            nums = {m["num"] for m in r["msgs"] if m["rel"] in (8, 9, 10)}
            if nums:
                pending.append((r["t"], nums))
            continue
        sizes.append(r["len"])
        if r["msgs"]:
            per_dg.append(len(r["msgs"]))
        if not r["acks"]:
            continue
        if r["msgs"]:
            with_msgs += 1
        else:
            standalone += 1
        acked = set()
        for a, b in r["acks"]:
            n = a
            while True:
                acked.add(n)
                if n == b:
                    break
                n = (n + 1) & 0xFFFF
        keep = []
        for t, nums in pending:
            if nums & acked:
                lat.append(r["t"] - t)
            else:
                keep.append((t, nums))
        pending = keep
    lat.sort()
    q = lambda p: lat[min(len(lat) - 1, int(p * len(lat)))] if lat else None  # noqa: E731
    share = standalone / (standalone + with_msgs) if standalone + with_msgs else 0.0
    return {"p50": q(0.5), "p90": q(0.9), "n": len(lat), "standalone": share,
            "per_dg": statistics.mean(per_dg) if per_dg else 0.0, "max_len": max(sizes) if sizes else 0,
            "unacked": len(pending)}


def session_counts(rep, o, u):
    """Every C>S message key over the whole session: events exactly, periodic ones within
    max(2, 20 %), and never sent by one client only (when the other sent it ≥ 3 times)."""
    ca, cb = {}, {}
    for m in o.msgs:
        ca[m["key"]] = ca.get(m["key"], 0) + 1
    for m in u.msgs:
        cb[m["key"]] = cb.get(m["key"], 0) + 1
    for k in sorted(set(ca) | set(cb)):
        a, b = ca.get(k, 0), cb.get(k, 0)
        if is_event(k):
            ok = a == b
        else:
            # "Sent by one client only" needs ≥ 3 sends on the other side: a single rare physics- or
            # race-driven send (209 for the car left as a passenger: 0 or 1 per run in the original
            # itself) cannot tell a missing feature from chance.
            ok = abs(a - b) <= max(2, 0.2 * max(a, b)) and ((a == 0) == (b == 0) or max(a, b) < 3)
        rep.check("session", "total", k, a, b, ok)


def raknet_checks(rep, o, u):
    a, b = ack_stats(o), ack_stats(u)
    rep.check("session", "ack-latency p50", "S>C reliable → C>S ack", a["p50"], b["p50"],
              a["p50"] is not None and b["p50"] is not None and abs(a["p50"] - b["p50"]) <= ACK_P50_TOL)
    rep.check("session", "ack-latency p90", "S>C reliable → C>S ack", a["p90"], b["p90"],
              a["p90"] is not None and b["p90"] is not None and abs(a["p90"] - b["p90"]) <= ACK_P90_TOL)
    rep.check("session", "ack-standalone-share", "ack datagrams without messages", a["standalone"],
              b["standalone"], abs(a["standalone"] - b["standalone"]) <= STANDALONE_TOL)
    rep.check("session", "packing", "messages per C>S datagram", a["per_dg"], b["per_dg"],
              abs(a["per_dg"] - b["per_dg"]) <= PACKING_TOL)
    rep.check("session", "mtu", "largest C>S datagram ≤ 549 B", a["max_len"], b["max_len"],
              a["max_len"] <= 549 and b["max_len"] <= 549)
    rep.check("session", "unacked", "S>C reliable datagrams never acked", a["unacked"], b["unacked"],
              a["unacked"] == b["unacked"])
    # Internal pings: how many, and when relative to the first 0x1e.
    def pings(s):
        t1e = next((m["t"] for m in s.msgs if m["kind"] == "msg 0x1e"), None)
        return [round(m["t"] - t1e, 3) if t1e else m["t"] for m in s.msgs if m["kind"] == "msg 0x06"]
    pa, pb = pings(o), pings(u)
    rep.check("session", "ping", "C>S 0x06 count", len(pa), len(pb), len(pa) == len(pb))
    rep.check("session", "ping", "C>S 0x06 after 0x1e (s)", pa, pb,
              len(pa) == len(pb) and all(abs(x - y) <= 0.02 for x, y in zip(pa, pb)))
    # Datagram composition right after the connection is accepted.
    def first_multi(s):
        for r in s.recs:
            if r["dir"] == "C>S" and r["cport"] == s.main and any(m["kind"] == "msg 0x1e" for m in r["msgs"]):
                return [m["kind"] for m in r["msgs"]]
        return []
    fa, fb = first_multi(o), first_multi(u)
    rep.check("join", "datagram", "messages with 0x1e", fa, fb, fa == fb)
    # Resends: delay of each resend after the first transmission; transmissions per message.
    def resends(s):
        if "loss_cs" not in s.windows:
            return [], {}
        w0, w1 = s.windows["loss_cs"]
        ds = [b - a for _n, a, b in s.resends if w0 <= a <= w1]
        per = {}
        for n, a, _b in s.resends:
            if w0 <= a <= w1:
                per[n] = per.get(n, 0) + 1
        return ds, per
    ra, pa_ = resends(o)
    rb, pb_ = resends(u)
    ma = statistics.median(ra) if ra else None
    mb = statistics.median(rb) if rb else None
    rep.check("loss_cs", "resend-delay median", "first transmission → resend", ma, mb,
              ma is not None and mb is not None and abs(ma - mb) <= RESEND_TOL)
    ta = sorted(set(pa_.values())) if pa_ else []
    tb = sorted(set(pb_.values())) if pb_ else []
    rep.check("loss_cs", "resends per dropped message", "set", ta, tb, ta == tb)
    dra = len([d for d in o.drops if d["dir"] == "C>S"])
    drb = len([d for d in u.drops if d["dir"] == "C>S"])
    rep.check("loss_cs", "dropped datagrams", "C>S", dra, drb, dra == drb)
    # Repeated loss: the intervals between the transmissions of one message (backoff or not).
    def repeat(s):
        if "loss_repeat" not in s.windows:
            return []
        w0, w1 = s.windows["loss_repeat"]
        best = []
        for n, ts in s.tx.items():
            if w0 <= ts[0] <= w1 and len(ts) > len(best):
                best = ts
        return [round(b - a, 3) for a, b in zip(best, best[1:])]
    xa, xb = repeat(o), repeat(u)
    rep.check("loss_repeat", "transmissions", "of the dropped message", len(xa) + 1 if xa else 0,
              len(xb) + 1 if xb else 0, len(xa) == len(xb) and len(xa) > 0)
    rep.check("loss_repeat", "resend intervals", "s", xa, xb,
              len(xa) == len(xb) and all(abs(x - y) <= RESEND_TOL for x, y in zip(xa, xb)))
    # Keep-alive gaps in the idle window (0x17 after > 5 s without a reliable send).
    def ka(s):
        if "idle" not in s.windows:
            return []
        w0, w1 = s.windows["idle"]
        ts = [m["t"] for m in s.msgs if m["kind"] == "msg 0x17"]
        return [round(t - w0, 1) for t in ts if w0 <= t <= w1]
    ka_o, ka_u = ka(o), ka(u)
    rep.check("idle", "keep-alive", "0x17 count", len(ka_o), len(ka_u), len(ka_o) == len(ka_u))


def main():
    args = sys.argv[1:]
    out = None
    if "--out" in args:
        i = args.index("--out")
        out = args[i + 1]
        del args[i:i + 2]
    if len(args) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    o, u = Session(args[0]), Session(args[1])
    rep = Report(args[2], args[3])
    names = [n for n in o.windows if n in u.windows]
    for n in sorted(set(o.windows) ^ set(u.windows)):
        rep.check(n, "window", "present", n in o.windows, n in u.windows, False)
    for n in names:
        window_checks(rep, o, u, n)
    session_counts(rep, o, u)
    raknet_checks(rep, o, u)
    lines = ["# F2 network diff", "", "original: `%s` (port %d, socket %s)  " % (o.dir, o.port, o.main),
             "ours: `%s` (port %d, socket %s)" % (u.dir, u.port, u.main), "",
             "| scenario | check | key | original | ours | status | decision |",
             "|---|---|---|---|---|---|---|"]
    for r in rep.rows:
        lines.append("| " + " | ".join(str(x).replace("|", "/") for x in r) + " |")
    n_ok = sum(1 for r in rep.rows if r[5] == "ok")
    n_acc = sum(1 for r in rep.rows if r[5] == "accepted")
    bad = [r for r in rep.rows if r[5] == "DIFF"]
    lines += ["", "checks: %d, matching: %d, accepted decisions: %d, open differences: %d"
              % (len(rep.rows), n_ok, n_acc, len(bad))]
    text = "\n".join(lines) + "\n"
    if out:
        open(out, "w").write(text)
    for r in rep.rows:
        if r[5] != "ok":
            print("%-9s %-12s %-28s %-24s orig=%s ours=%s %s" % (r[5], r[0], r[1], r[2][:28], r[3][:60], r[4][:60], r[6]))
    print("checks: %d, matching: %d, accepted decisions: %d, open differences: %d"
          % (len(rep.rows), n_ok, n_acc, len(bad)))
    if out:
        print("report: %s" % out)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
