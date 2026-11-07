#!/usr/bin/env python3
"""Compare the original client and ours on the server-password cases of tools/e2e/password.sh.

    password_report.py OUTROOT CASE...

Reads OUTROOT/{orig,ours}-CASE/{net.log,chatlog.txt,server.log} and prints a Markdown report. Per case it
compares the `0x0b` connection request(s), the server's reaction (`0x25` refusal / join), the open-connection
requests the client sent after the refusal (a retry) and the chat lines (timestamps stripped; for a join, up
to the "Connected to" line). Exit 0 when both clients match in every case and behave as expected
(right: joins; wrong / missing: refused, no retry).
"""
import os
import re
import sys

LINE = re.compile(r"^\s*([0-9.]+) (C>S|S>C) (.*)$")


def parse_net(path):
    out = {"requests": [], "refused_at": None, "opens": [], "joined": False}
    if not os.path.exists(path):
        return None
    for raw in open(path, encoding="utf-8", errors="replace"):
        m = LINE.match(raw)
        if not m:
            continue
        t, d, rest = float(m.group(1)), m.group(2), m.group(3)
        if d == "C>S" and rest.startswith("open-connection-request"):
            out["opens"].append(t)
        mm = re.match(r"#\d+ rel(\d+) msg 0x0b ([0-9a-f]+)", rest)
        if d == "C>S" and mm:
            out["requests"].append((int(mm.group(1)), mm.group(2)))
        if d == "S>C" and re.match(r"#\d+ rel\d+ msg 0x25\b", rest) and out["refused_at"] is None:
            out["refused_at"] = t
        if d == "S>C" and re.search(r"rpc 139 ", rest):
            out["joined"] = True
    return out


def parse_chat(path, joined):
    if not os.path.exists(path):
        return None
    lines = []
    for raw in open(path, "rb").read().decode("latin-1").splitlines():
        s = re.sub(r"^\[\d\d:\d\d:\d\d\] ", "", raw.strip("\r"))
        if not s:
            continue
        lines.append(s)
        if joined and s.startswith("Connected to"):
            break
    return lines


def summary(root, client, case):
    d = os.path.join(root, f"{client}-{case}")
    net = parse_net(os.path.join(d, "net.log"))
    if net is None:
        return None
    srv = os.path.join(d, "server.log")
    joined_srv = os.path.exists(srv) and re.search(r"\[e2e\] connect id=\d+ name=Pw", open(srv, errors="replace").read())
    net["joined"] = bool(net["joined"] or joined_srv)
    r = net["refused_at"]
    net["retries"] = len([t for t in net["opens"] if r is not None and t > r])
    # Distinct requests (a resend repeats the same bytes).
    net["request_set"] = sorted(set(net["requests"]))
    net["chat"] = parse_chat(os.path.join(d, "chatlog.txt"), net["joined"])
    return net


def main():
    root, cases = sys.argv[1], sys.argv[2:]
    ok = True
    print("# Server password: original vs ours\n")
    for case in cases:
        o, u = summary(root, "orig", case), summary(root, "ours", case)
        print(f"## {case}\n")
        if o is None or u is None:
            print(f"- missing run: orig={'yes' if o else 'no'} ours={'yes' if u else 'no'}\n")
            ok = False
            continue
        rows = [
            ("0x0b request (rel, hex)", o["request_set"], u["request_set"]),
            ("0x25 refusal", o["refused_at"] is not None, u["refused_at"] is not None),
            ("joined", o["joined"], u["joined"]),
            ("open requests after refusal", o["retries"], u["retries"]),
            ("chat lines", o["chat"], u["chat"]),
        ]
        print("| check | original | ours | equal |\n|---|---|---|---|")
        for name, a, b in rows:
            eq = a == b
            ok &= eq
            print(f"| {name} | `{a}` | `{b}` | {'yes' if eq else '**NO**'} |")
        expect_join = case == "right"
        for who, s in (("original", o), ("ours", u)):
            good = s["joined"] if expect_join else (s["refused_at"] is not None and s["retries"] == 0 and not s["joined"])
            if not good:
                print(f"\n- {who}: unexpected outcome for {case}")
                ok = False
        print()
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
