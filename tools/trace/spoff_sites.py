#!/usr/bin/env python3
"""TRACE-4: read the bytes at the single-player generator sites through the tracer's `peek`.

    spoff_sites.py PORT [OUTFILE]

Prints one line per site: address, what it is (CSV row of docs/re/data/gta-patches-r4-annotated.csv),
the stock bytes of gta_sa.exe 1.0 US and the live bytes, and whether they differ.
"""
import socket
import sys

# (address, length, stock bytes, description)
SITES = [
    (0x53C1C1, 5, "e8fa7fefff", "row 189: CGame::Process → CCarCtrl::GenerateRandomCars call"),
    (0x53C06A, 5, "e8d17e1b00", "row 181: CGame::Process → CTheCarGenerators::Process call"),
    (0x434272, 5, "e879902900", "row 182: GenerateRandomCars → CPlane::DoPlaneGenerationAndRemoval call"),
    (0x612710, 3, "90e9ad", "row 185: CPopulation::AddPed entry"),
    (0x613BA7, 5, "e8d4c60300", "row 186: AddPedInCar → CCarEnterExit::SetPedInCarDirect call"),
    (0x440833, 8, "528bcde875e0ffff", "row 164: CEntryExit::TransitionFinished → GenerateAmbientPeds call"),
    (0x4E7427, 5, "e8e46d0800", "row 55: police scanner AddAudioEvent → FindPlayerPed call"),
    (0x4E7529, 39, None, "row 361: AddAudioEvent block NOPed by PlayCrimeReport (RPC 112) only"),
    (0x53BF28, 5, "e8d39df9ff", "row 246: CGame::Process → CCutsceneMgr::Update call"),
    (0x53C090, 5, "e86b44f2ff", "row 163: CGame::Process → CReplay::Update call"),
    (0x58DB5F, 9, None, "row 202: CHud::DrawWanted level compare"),
    (0x8CDEE4, 4, None, "CWanted::MaximumWantedLevel (data)"),
    (0x8D2530, 4, None, "CPopulation::PedDensityMultiplier (data, float)"),
    (0x8A5B20, 4, None, "CCarCtrl::CarDensityMultiplier (data, float)"),
]


def main():
    port = int(sys.argv[1])
    out = open(sys.argv[2], "w") if len(sys.argv) > 2 else sys.stdout
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    f = s.makefile("rw")
    for addr, n, stock, what in SITES:
        f.write(f"peek {addr:#x} {n}\n")
        f.flush()
        reply = f.readline().split()
        live = "".join(reply[2:]) if len(reply) >= 3 and reply[0] == "peek" else "?"
        flag = "" if stock is None else ("  (stock)" if live == stock else "  CHANGED")
        print(f"{addr:#x} {live:<20s} {what}{flag}", file=out)


if __name__ == "__main__":
    main()
