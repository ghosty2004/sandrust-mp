#!/usr/bin/env python3
"""M7: combat sends per combat2.sh part, for one trace run (original or ours).

    combat_report.py RUN_DIR [--client-port N]   (run with python3 -I)

Decodes the client→server lines of RUN_DIR/net.pcapng (samp-dump --raw | c2s_decode) for the
combat ids (206, 115, 53, 52, 204, 203, 128) of the traced client (the per-port capture that starts first), places them between the `combat2.sh` marks
("<part> wall=<epoch>") of RUN_DIR/trace.jsonl and prints one block per part with a normalised
line per packet (values rounded, timestamps relative to the part's mark), so that the original's
and our blocks can be compared with diff(1).
"""
import json
import os
import re
import subprocess
import sys
from datetime import datetime

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
IDS = "206,115,53,52,204,203,128,211"


def capture_start(pcap):
    out = subprocess.run(["capinfos", "-a", "-T", "-r", pcap], capture_output=True, text=True).stdout.strip()
    # "-T -r": tab-separated "file<TAB>start" without a header.
    field = out.split("\t")[-1].strip()
    return datetime.strptime(field[:26], "%Y-%m-%d %H:%M:%S.%f").timestamp()


def marks(run):
    out = []
    for ln in open(os.path.join(run, "trace.jsonl"), errors="replace"):
        if '"mark"' not in ln:
            continue
        try:
            text = json.loads(ln)["args"]["text"]
        except (ValueError, KeyError):
            continue
        m = re.match(r"(\S+) wall=([0-9.]+)", text)
        if m:
            out.append((float(m.group(2)), m.group(1)))
    return sorted(out)


def norm(body):
    """Round floats to 2 decimals so tiny position differences do not dominate the diff."""
    return re.sub(r"-?\d+\.\d+(e-?\d+)?", lambda m: f"{float(m.group(0)):.2f}", body)


def main():
    run = sys.argv[1]
    # run.sh splits the capture per client port (net-<port>.pcapng); the traced client joined first.
    per_port = [os.path.join(run, f) for f in os.listdir(run) if re.fullmatch(r"net-\d+\.pcapng", f)]
    pcap = min(per_port, key=capture_start) if per_port else os.path.join(run, "net.pcapng")
    t0 = capture_start(pcap)
    dump = subprocess.run([os.path.join(BIN, "debug", "samp-dump"), pcap, "--port", "7790", "--raw"],
                          capture_output=True, text=True).stdout
    dec = subprocess.run([os.path.join(BIN, "debug", "examples", "c2s_decode"), "--c2s", "--only", IDS],
                         input=dump, capture_output=True, text=True).stdout
    ms = marks(run)
    parts = {}
    for ln in dec.splitlines():
        m = re.match(r"\s*([0-9.]+) C>S #\d+ (rel\d+)(?: ch\d+:\d+)? (rpc|sync)\s+(\d+) \d+ (?:bits \S+ )?(.*)", ln)
        if not m:
            continue
        t = t0 + float(m.group(1))
        cur = None
        for wall, name in ms:
            if wall <= t:
                cur = (wall, name)
        if cur is None:
            continue
        rel = t - cur[0]
        parts.setdefault(cur[1], []).append(f"+{rel:6.2f}s {m.group(2)} {m.group(3)} {m.group(4)} {norm(m.group(5))}")
    for _, name in ms:
        lines = parts.get(name, [])
        print(f"== {name} ({len(lines)} packets)")
        for x in lines:
            print("  " + x)


if __name__ == "__main__":
    main()
