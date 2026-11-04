#!/usr/bin/env python3
"""COV-1 scenario driver (run by tools/cov/run.sh inside the diff lanes).

Drives one or two games (A = original samp.dll, B = our samp_client.dll; both with
samp-tracer in coverage mode) through the same windows, each side in its own thread,
starting a coverage window (`cov start NAME`) on the side's tracer before each step:

  boot (from attach) -> connecting (game state 9) -> class_selection ([e2e] class)
  -> spawn (Shift) -> grove_idle (Grove Street, COV_IDLE_SECS, default 300)
  -> walk (W 5 s) -> interior_enter (Alhambra entry-exit, W into the marker, 20 s inside)
  -> interior_exit (S back out) -> vehicle (Sultan: F enter, W 3 s, F exit)
  -> idle_end (COV_END_SECS, default 60) -> `cov dump`, quit.

    drive.py --out DIR --log SERVER_LOG --port UDP --rcon-pass PASS
             --side "KEY|NAME|TRACER_PORT|DISPLAY|DUMP_WINPATH" [--side ...]
"""
import argparse
import json
import os
import re
import socket
import struct
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "diff"))
import drive as ddrive  # noqa: E402  (rcon, Log)

AREA = 0xB72914  # CGame::currArea
IDLE = int(os.environ.get("COV_IDLE_SECS", "300"))
END = int(os.environ.get("COV_END_SECS", "60"))
LOCK = threading.Lock()


def ctl(port, line, timeout=10.0):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
            s.sendall((line + "\n").encode())
            return s.makefile().readline().strip()
    except OSError:
        return ""


class Side:
    def __init__(self, spec, args, log_path):
        key, name, port, display, dump = spec.split("|", 4)
        self.key, self.name, self.port, self.display, self.dump = key, name, int(port), display, dump
        self.args = args
        self.log = ddrive.Log(log_path)
        self.pid = None
        self.events = []
        self.ok = False
        self.t0 = time.time()

    def say(self, *a):
        with LOCK:
            print(f"[cov {self.key} +{time.time() - self.t0:6.1f}s]", *a, flush=True)

    def ev(self, what, **kw):
        rec = {"t": round(time.time() - self.t0, 2), "what": what, **kw}
        self.events.append(rec)
        self.say(what, json.dumps(kw) if kw else "")

    def xdo(self, *a):
        env = {**os.environ, "DISPLAY": self.display}
        env.pop("WAYLAND_DISPLAY", None)
        subprocess.run(["xdotool", *a], env=env, check=False)

    def hold(self, key, secs):
        self.xdo("keydown", key)
        time.sleep(secs)
        self.xdo("keyup", key)

    def window(self, name):
        r = ctl(self.port, f"cov start {name}")
        ctl(self.port, f"mark cov {name}")
        self.ev("window", name=name, reply=r)
        if not r.startswith("ok"):
            raise RuntimeError(f"cov start {name}: {r!r}")

    def rcon(self, cmd, pat, secs=10):
        ddrive.rcon(self.args.port, self.args.rcon_pass, f"cov {cmd} {self.pid} ;")
        return self.log.wait(pat, secs)

    def state(self):
        return ctl(self.port, "state")

    def area(self):
        r = ctl(self.port, f"peek {AREA:#x} 4").split()
        try:
            return struct.unpack("<i", bytes.fromhex("".join(r[2:6])))[0]
        except (ValueError, IndexError):
            return None

    def pos(self):
        m = re.search(r"pos=([-\d.]+),([-\d.]+),([-\d.]+)", self.state())
        return tuple(float(x) for x in m.groups()) if m else None

    def wait_area(self, want_inside, secs):
        end = time.time() + secs
        while time.time() < end:
            a = self.area()
            if a is not None and (a != 0) == want_inside:
                return a
            time.sleep(0.5)
        return None

    def logwait(self, pat, secs):
        self.log.cursor = 0
        return self.log.wait(pat, secs)

    def run(self):
        try:
            self._run()
            self.ok = True
        except Exception as e:  # noqa: BLE001 (report and dump what we have)
            self.ev("error", msg=str(e))
        finally:
            r = ctl(self.port, f"cov dump {self.dump}", timeout=60)
            self.ev("dump", reply=r)

    def _run(self):
        # connecting: the game is running (state 9) and the client starts its connection.
        end = time.time() + 180
        while time.time() < end and "game_state=9" not in self.state():
            time.sleep(0.25)
        if "game_state=9" not in self.state():
            raise RuntimeError("game never reached state 9")
        self.window("connecting")
        m = self.logwait(rf"\[e2e\] connect id=(\d+) name={self.name}\b", 180)
        if not m:
            raise RuntimeError("no join")
        self.pid = int(m.group(1))
        self.ev("joined", pid=self.pid)
        if not self.logwait(rf"\[e2e\] class id={self.pid} class=", 90):
            raise RuntimeError("no class request")
        self.window("class_selection")
        time.sleep(15)
        self.window("spawn")
        spawned = False
        for _ in range(8):
            self.xdo("key", "--delay", "80", "shift")
            if self.logwait(rf"\[cov\] spawned id={self.pid}\b", 5):
                spawned = True
                break
        if not spawned:
            raise RuntimeError("no spawn after Shift")
        time.sleep(10)
        self.window("grove_idle")
        if not self.rcon("grove", rf"\[cov\] ok grove {self.pid}\b"):
            raise RuntimeError("grove teleport not confirmed")
        for k in range(0, IDLE, 30):
            time.sleep(min(30, IDLE - k))
            self.ev("idle", secs=k + 30, state=self.state())
        self.window("walk")
        self.hold("w", 5)
        time.sleep(3)
        self.ev("walked", state=self.state())
        self.window("interior_enter")
        if not self.rcon("enex", rf"\[cov\] ok enex {self.pid}\b"):
            raise RuntimeError("enex teleport not confirmed")
        time.sleep(4)
        self.hold("w", 2.5)
        a = self.wait_area(True, 15)
        self.ev("inside", area=a, state=self.state())
        if a is None:
            raise RuntimeError("did not enter the interior")
        time.sleep(20)
        self.window("interior_exit")
        self.hold("s", 2.5)
        a = self.wait_area(False, 15)
        if a is None:
            # The exit marker is behind the entry point; try once more, longer.
            self.hold("s", 4)
            a = self.wait_area(False, 15)
        self.ev("outside", area=a, state=self.state())
        if a is None:
            raise RuntimeError("did not leave the interior")
        time.sleep(8)
        self.window("vehicle")
        if not self.rcon("car", rf"\[cov\] ok car {self.pid} veh=\d+"):
            raise RuntimeError("car not created")
        time.sleep(4)
        self.hold("f", 0.3)
        if not self.logwait(rf"\[cov\] state id={self.pid} 1->2\b", 12):
            self.hold("f", 0.3)
            if not self.logwait(rf"\[cov\] state id={self.pid} 1->2\b", 12):
                raise RuntimeError("did not become driver")
        self.ev("driver")
        time.sleep(1)
        self.hold("w", 3)
        time.sleep(3)
        self.hold("f", 0.3)
        if not self.logwait(rf"\[cov\] state id={self.pid} 2->1\b", 12):
            self.hold("f", 0.3)
            if not self.logwait(rf"\[cov\] state id={self.pid} 2->1\b", 12):
                raise RuntimeError("did not leave the car")
        self.ev("onfoot")
        time.sleep(3)
        self.window("idle_end")
        time.sleep(END)
        self.ev("end", state=self.state())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--log", required=True)
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--rcon-pass", required=True)
    ap.add_argument("--side", action="append", required=True)
    a = ap.parse_args()
    sides = [Side(s, a, a.log) for s in a.side]
    th = [threading.Thread(target=s.run, name=s.key) for s in sides]
    for t in th:
        t.start()
    for t in th:
        t.join()
    with open(os.path.join(a.out, "events.json"), "w") as f:
        json.dump({s.key: {"ok": s.ok, "name": s.name, "events": s.events} for s in sides}, f, indent=1)
    sys.exit(0 if all(s.ok for s in sides) else 1)


if __name__ == "__main__":
    main()
