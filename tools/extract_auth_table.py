#!/usr/bin/env python3
"""Extract the auth-key challenge/response table from open.mp's RakNet fork.

The table is third-party data and is NOT committed. Usage:
    git clone --depth 1 https://github.com/openmultiplayer/RakNet re/omp-raknet
    python3 tools/extract_auth_table.py re/omp-raknet/SAMPRakNet.cpp data/auth_table.txt
Output: one "CHALLENGE RESPONSE" pair per line (format read by samp_proto::auth::AuthTable).
"""
import re
import sys

src, dst = sys.argv[1], sys.argv[2]
text = open(src, encoding="utf-8", errors="replace").read()
start = text.index("AuthTable[")
pairs = re.findall(r'\{\s*"([0-9A-Fa-f]+)"\s*,\s*"([0-9A-Fa-f]+)"\s*\}', text[start:])
if len(pairs) != 256:
    sys.exit(f"expected 256 pairs, found {len(pairs)}")
with open(dst, "w") as f:
    f.write(f"# extracted from {src}; third-party data, do not commit\n")
    for c, r in pairs:
        f.write(f"{c} {r}\n")
print(f"wrote {len(pairs)} pairs to {dst}")
