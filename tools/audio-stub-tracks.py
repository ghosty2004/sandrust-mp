#!/usr/bin/env python3
"""Steam 3.0 audio data under the 1.0 US gta_sa.exe: alias the zero-length stub tracks.

The Steam release replaced the removed licensed songs with stub tracks: a valid Ogg Vorbis
stream holding zero samples (4429 bytes, last granule position 0). The 1.0 US stream thread
starts a radio track at "play time modulo track length" (gta_sa.exe 0x4F1457..0x4F1469: the
decoder's length-in-ms virtual call, then `div ecx`), so a stub picked as the first track of a
station (entering a car mid-song) raises an integer division by zero on the audio stream thread
("wine: Unhandled division by zero at address 004F1464"). The original samp.dll does nothing
about it; it is a property of the mixed 1.0 exe + Steam data install (docs/phase3/soak-notes.md).

`patch` rewrites audio/CONFIG/TrakLkup.dat of a game copy so every stub entry points at the
nearest real track of the same stream pack (the previous one, else the next). The file is written
as a new inode (rename over), so other hard links to the shared original stay untouched; the
original is kept as TrakLkup.dat.steam (a hard link) for `restore`.

    audio-stub-tracks.py check   GTA_DIR    list the stub entries (exit 1 if the table has any)
    audio-stub-tracks.py patch   GTA_DIR    alias them (no-op when there are none)
    audio-stub-tracks.py restore GTA_DIR    put the original table back
"""
import os
import struct
import sys

# 1.0 stream pack XOR key (applied by file offset modulo 16).
KEY = bytes.fromhex("EA3AC4A19AA814F348B0D7239DE8FFF1")
# Per-track header (beat table + length/rate fields) before the Ogg data.
TRACK_HEADER = 8068
ENTRY = 12  # u8 pack, 3 pad, u32 offset, u32 Ogg size


def packs(gta):
    raw = open(os.path.join(gta, "audio/CONFIG/StrmPaks.dat"), "rb").read()
    return [raw[i:i + 16].split(b"\0")[0].decode() for i in range(0, len(raw), 16)]


def read_dec(f, off, n):
    f.seek(off)
    b = f.read(n)
    return bytes(x ^ KEY[(off + i) % 16] for i, x in enumerate(b))


def samples(f, off, size):
    """Last granule position of the track's Ogg stream (-1 when unreadable)."""
    n = min(size, 8192)
    tail = read_dec(f, off + TRACK_HEADER + size - n, n)
    k = tail.rfind(b"OggS", 0, max(0, len(tail) - 14))
    return struct.unpack_from("<q", tail, k + 6)[0] if k >= 0 else -1


def scan(gta, table):
    names = packs(gta)
    files = {}
    entries = []
    for i in range(len(table) // ENTRY):
        pk = table[i * ENTRY]
        off, size = struct.unpack_from("<II", table, i * ENTRY + 4)
        name = names[pk] if pk < len(names) else ""
        path = os.path.join(gta, "audio/streams", name)
        stub = False
        if name and os.path.isfile(path):
            f = files.setdefault(name, open(path, "rb"))
            stub = samples(f, off, size) == 0
        entries.append((pk, name, off, size, stub))
    for f in files.values():
        f.close()
    return entries


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("check", "patch", "restore"):
        print(__doc__.strip().splitlines()[-3:], file=sys.stderr)
        return 2
    cmd, gta = sys.argv[1], sys.argv[2]
    lk = os.path.join(gta, "audio/CONFIG/TrakLkup.dat")
    keep = lk + ".steam"
    if cmd == "restore":
        if os.path.exists(keep):
            os.replace(keep, lk)
            print("restored", lk)
        return 0
    if not os.path.isfile(lk):
        print("no", lk, file=sys.stderr)
        return 2
    table = bytearray(open(lk, "rb").read())
    entries = scan(gta, table)
    stubs = [i for i, e in enumerate(entries) if e[4]]
    if cmd == "check":
        for i in stubs:
            print("stub track %d: %s offset %d size %d" % (i, entries[i][1], entries[i][2], entries[i][3]))
        print("%d tracks, %d zero-length stubs" % (len(entries), len(stubs)))
        return 1 if stubs else 0
    if not stubs:
        print("%s: no zero-length stub tracks" % lk)
        return 0
    for i in stubs:
        pk = entries[i][0]
        alias = next((j for j in range(i - 1, -1, -1) if entries[j][0] == pk and not entries[j][4]), None)
        if alias is None:
            alias = next((j for j in range(i + 1, len(entries)) if entries[j][0] == pk and not entries[j][4]), None)
        if alias is None:
            print("track %d: no real track in pack %s to alias" % (i, entries[i][1]), file=sys.stderr)
            return 1
        table[i * ENTRY + 4:i * ENTRY + 12] = table[alias * ENTRY + 4:alias * ENTRY + 12]
    if not os.path.exists(keep):
        os.link(lk, keep)
    tmp = lk + ".tmp"
    with open(tmp, "wb") as f:
        f.write(table)
    os.replace(tmp, lk)
    print("%s: %d zero-length stub tracks aliased (original kept as %s)" % (lk, len(stubs), os.path.basename(keep)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
