#!/usr/bin/env python3
"""Sets the video mode index stored in a gta_sa.set (CMenuManager::m_nDisplayVideoMode).

  setmode.py GTA_SA_SET INDEX

The index refers to RenderWare's mode list, which samp-tracer writes to samp-tracer.log
("d3d: installed: … modes [i:WxHxDEPTH …]"). The field is found from the end of the file:
the settings block ends with the '_' constant, and the layout (gta-reversed
CMenuManager::LoadSettings) puts the int32 m_nDisplayVideoMode right after the
'T' constant, the draw-distance float and three bools.
"""
import struct
import sys

path, idx = sys.argv[1], int(sys.argv[2])
d = bytearray(open(path, "rb").read())
t = d.rfind(b"T", 0, len(d) - 16)
# 'T' (m_unk) + float draw distance + 3 bools → video mode
off = t + 1 + 4 + 3
old = struct.unpack_from("<i", d, off)[0]
assert 0 <= old < 256, "unexpected layout (old value %d at %#x)" % (old, off)
struct.pack_into("<i", d, off, idx)
open(path, "wb").write(d)
print("video mode %d -> %d at %#x" % (old, idx, off))
