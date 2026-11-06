#!/usr/bin/env python3
"""Print the X server's current cursor image (XFixesGetCursorImage) on $DISPLAY.

CURSOR-1: gamescope screenshots (e2e_shot) do not contain the cursor, so the visible cursor of a
client on an isolated headless display is measured here instead, e.g.
    DISPLAY=$E2E_HEADLESS_DISPLAY python3 -I tools/e2e/xcursor.py [-v]
Wine shows a hidden cursor as a 1x1 transparent image; samp's mouse.png arrow is 32x32 with 438
non-transparent pixels (sha a918be417195 under Wine 11 + DXVK 3.1.1). -v prints the alpha mask.
"""
import ctypes, ctypes.util, hashlib, sys
x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
xf = ctypes.CDLL(ctypes.util.find_library("Xfixes"))
class Img(ctypes.Structure):
    _fields_ = [("x", ctypes.c_short), ("y", ctypes.c_short), ("width", ctypes.c_ushort), ("height", ctypes.c_ushort),
                ("xhot", ctypes.c_ushort), ("yhot", ctypes.c_ushort), ("serial", ctypes.c_ulong),
                ("pixels", ctypes.POINTER(ctypes.c_ulong)), ("atom", ctypes.c_ulong), ("name", ctypes.c_char_p)]
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
xf.XFixesGetCursorImage.restype = ctypes.POINTER(Img)
xf.XFixesGetCursorImage.argtypes = [ctypes.c_void_p]
d = x11.XOpenDisplay(None)
if not d:
    sys.exit("no display")
p = xf.XFixesGetCursorImage(d)
if not p:
    sys.exit("no image")
i = p.contents
n = i.width * i.height
px = [i.pixels[k] & 0xFFFFFFFF for k in range(n)]
opaque = sum(1 for v in px if v >> 24)
h = hashlib.sha1(b"".join(v.to_bytes(4, "little") for v in px)).hexdigest()[:12]
print(f"pos=({i.x},{i.y}) size={i.width}x{i.height} hot=({i.xhot},{i.yhot}) serial={i.serial} "
      f"name={i.name!r} opaque_px={opaque} sha={h}")
if "-v" in sys.argv:
    for y in range(i.height):
        print("".join("#" if (px[y * i.width + x] >> 24) > 128 else ("+" if px[y * i.width + x] >> 24 else ".")
                      for x in range(i.width)))
