#!/usr/bin/env python3
"""DIFF-2 scenario driver: runs inside e2e_headless_run (isolated display),
while the game (tracer + client) and the lane's server are already started.

    drive.py --client orig|ours --out DIR --log SERVER_LOG --port UDP
             --rcon-pass PASS --client-port N --tracer-port N [--only REGEX]

Steps (from the rpcfire filterscript's `diff list`):
  boot -> join (flow_join) -> class selection -> Shift until spawned (flow_spawn)
  -> for every step: pre state -> mark "step <i> <name> begin" -> fire (RCON
  `diff fire <name>` or a key press) -> wait settle -> post state + screenshot
  -> mark "step <i> <name> end".
After flow_spawn the tracer hides the local ped from rendering (`hideped on`; DIFF_SHOW_PED=1 keeps it).
Output: DIR/steps.jsonl (one record per step), DIR/shots/<idx>-<name>.jpg.
Exit 0 when every step ran (an error in single steps is recorded, not fatal).
"""
import argparse
import json
import os
import re
import socket
import struct
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gtastate  # noqa: E402

NAME = "DiffPlayer"


def say(*a):
    print("[diff]", *a, flush=True)


def ctl(port, line, timeout=3.0):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
            s.sendall((line + "\n").encode())
            return s.makefile().readline().strip()
    except OSError:
        return ""


def rcon(port, password, cmd):
    """SA:MP query-protocol RCON packet ('x'); open.mp needs rcon.enable."""
    pkt = b"SAMP" + bytes([127, 0, 0, 1]) + struct.pack("<H", port) + b"x"
    pkt += struct.pack("<H", len(password)) + password.encode()
    pkt += struct.pack("<H", len(cmd)) + cmd.encode()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.sendto(pkt, ("127.0.0.1", port))


class Log:
    """Incremental reader of the server log. Lines are kept; `wait` scans
    forward from a cursor, so lines read in the same batch are never lost."""

    def __init__(self, path):
        self.path = path
        self.pos = 0
        self.hist = []
        self.cursor = 0

    def _read(self):
        try:
            with open(self.path, "rb") as f:
                f.seek(self.pos)
                data = f.read()
        except OSError:
            return
        cut = data.rfind(b"\n") + 1  # only complete lines
        self.pos += cut
        self.hist += data[:cut].decode(errors="replace").splitlines()

    def wait(self, pattern, secs, collect=None):
        """First line at/after the cursor matching `pattern`; the cursor moves
        past it. `collect` receives every line scanned."""
        rx = re.compile(pattern)
        end = time.time() + secs
        while True:
            self._read()
            while self.cursor < len(self.hist):
                ln = self.hist[self.cursor]
                self.cursor += 1
                if collect is not None:
                    collect.append(ln)
                m = rx.search(ln)
                if m:
                    return m
            if time.time() >= end:
                return None
            time.sleep(0.1)

    def skip_to_end(self):
        self._read()
        self.cursor = len(self.hist)


class Driver:
    def __init__(self, a):
        self.a = a
        self.log = Log(a.log)
        self.out = a.out
        os.makedirs(os.path.join(self.out, "shots"), exist_ok=True)
        self.steps_f = open(os.path.join(self.out, "steps.jsonl"), "w")
        self.ours = a.client == "ours"
        self.env_x = {**os.environ, "DISPLAY": os.environ.get("E2E_HEADLESS_DISPLAY", "")}

    def mark(self, text):
        ctl(self.a.tracer_port, f"mark {text}")

    def key(self, k):
        subprocess.run(["xdotool", "key", "--delay", "80", k], env=self.env_x, check=False)

    def shot(self, name):
        """Screenshot of the isolated gamescope display, stored as 640x360 JPEG."""
        raw = os.path.join(self.out, "shots", f".{name}.png")
        dst = os.path.join(self.out, "shots", f"{name}.jpg")
        try:
            os.unlink(raw)
        except OSError:
            pass
        env = {**os.environ, "GAMESCOPE_WAYLAND_DISPLAY": os.environ.get("E2E_GAMESCOPE", "")}
        subprocess.run(["gamescopectl", "screenshot", raw], env=env, check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        last = -1
        for _ in range(40):
            time.sleep(0.1)
            try:
                sz = os.path.getsize(raw)
            except OSError:
                continue
            if sz > 0 and sz == last:
                break
            last = sz
        try:
            from PIL import Image

            with Image.open(raw) as im:
                im.convert("RGB").resize((640, 360)).save(dst, quality=90)
            os.unlink(raw)
            return os.path.relpath(dst, self.out)
        except Exception as e:  # noqa: BLE001 — a missing shot is recorded, not fatal
            say(f"screenshot {name} failed: {e}")
            return None

    def collect(self):
        """state (client port), extended snapshot (tracer peek), net (ours)."""
        rec = {"t": round(time.time(), 3), "alive": ctl(self.a.tracer_port, "ping") == "pong"}
        rec["state"] = ctl(self.a.client_port, "state")
        rec["snap"] = gtastate.take(self.a.tracer_port)
        if self.ours:
            rec["net"] = ctl(self.a.client_port, "net")
        return rec

    def write(self, rec):
        self.steps_f.write(json.dumps(rec) + "\n")
        self.steps_f.flush()

    def run_step(self, st, fire):
        tag = f"{st['idx']:03d}-{st['name']}"
        rec = dict(st)
        rec["pre"] = self.collect()
        self.mark(f"step {st['idx']} {st['name']} begin")
        rec["t_fire"] = round(time.time(), 3)
        rec["fired"], rec["error"] = fire()
        if rec["fired"]:
            time.sleep(st["settle_ms"] / 1000.0)
        rec["post"] = self.collect()
        rec["shot"] = self.shot(tag)
        self.mark(f"step {st['idx']} {st['name']} end")
        self.write(rec)
        state = "ok" if rec["fired"] else f"NOT FIRED ({rec['error']})"
        say(f"step {st['idx']:3d} {st['name']:<30} rpcs={st['rpcs']} {state}")
        return rec

    def fire_fs(self, st):
        def go():
            self.log.skip_to_end()
            rcon(self.a.port, self.a.rcon_pass, f"diff fire {st['name']} ;")
            m = self.log.wait(rf"\[diff\] (fire \d+ \S+ step={st['idx']}\b|error .*)", 5)
            if not m:
                return False, "no [diff] fire line within 5 s"
            if m.group(1).startswith("error"):
                return False, m.group(1)
            return True, None

        return go

    def fire_key(self, k):
        def go():
            self.key(k)
            return True, None

        return go

    def main(self):
        a = self.a
        for _ in range(180):
            if ctl(a.tracer_port, "ping") == "pong":
                break
            time.sleep(0.5)
        else:
            say("tracer control channel did not answer")
            return 1
        self.mark("boot")
        say("tracer up:", ctl(a.tracer_port, "state"))

        # Step list from the filterscript.
        lines = []
        rcon(a.port, a.rcon_pass, "diff list ;")
        ok = self.log.wait(r"\[diff\] list end \d+", 10, collect=lines)
        self.log.cursor = 0  # join/class lines may predate the list
        if not ok:
            say("filterscript did not answer `diff list` over RCON")
            return 1
        steps = []
        for ln in lines:
            m = re.search(r"\[diff\] step (\d+) (\S+) (\S+) (\d+) (\S+)", ln)
            if m:
                steps.append({
                    "idx": int(m.group(1)), "rpcs": [int(x) for x in m.group(2).split(",")],
                    "name": m.group(3), "settle_ms": int(m.group(4)), "action": m.group(5),
                })
        say(f"{len(steps)} steps")
        with open(os.path.join(self.out, "step-list.json"), "w") as f:
            json.dump(steps, f, indent=1)
        flow = {s["name"]: s for s in steps if s["action"] == "flow"}

        # flow_join: from boot to class selection.
        st = flow["flow_join"]
        rec = dict(st, pre=self.collect(), t_fire=round(time.time(), 3))
        self.mark(f"step {st['idx']} {st['name']} begin")
        ok = bool(self.log.wait(rf"\[e2e\] connect id=\d+ name={NAME}\b", 150))
        ok = ok and bool(self.log.wait(r"\[e2e\] class id=\d+ class=", 60))
        rec["fired"], rec["error"] = ok, (None if ok else "no join/class request")
        if ok:
            time.sleep(4)
        rec["post"] = self.collect()
        rec["shot"] = self.shot(f"{st['idx']:03d}-{st['name']}")
        self.mark(f"step {st['idx']} {st['name']} end")
        self.write(rec)
        say(f"join: {'ok' if ok else 'FAILED'}")
        if not ok:
            return 1

        # flow_spawn: Shift until the server sees the spawn.
        st = flow["flow_spawn"]
        rec = dict(st, pre=self.collect(), t_fire=round(time.time(), 3))
        self.mark(f"step {st['idx']} {st['name']} begin")
        ok = False
        for _ in range(6):
            self.key("shift")
            if self.log.wait(r"\[e2e\] spawn id=", 4):
                ok = True
                break
        rec["fired"], rec["error"] = ok, (None if ok else "no spawn after Shift")
        if ok:
            time.sleep(4)
        rec["post"] = self.collect()
        rec["shot"] = self.shot(f"{st['idx']:03d}-{st['name']}")
        self.mark(f"step {st['idx']} {st['name']} end")
        self.write(rec)
        say(f"spawn: {'ok' if ok else 'FAILED'}")
        if not ok:
            return 1

        # The local ped's idle animation (breathing, head movement) runs on its own clock: its pose at a
        # screenshot is a sampling accident, not a client reaction, and it covers the middle of the picture.
        # From here on the tracer leaves the local ped out of rendering on both clients (`hideped`, a
        # render filter: game state untouched); what the ped does is judged through the trace and state.
        if os.environ.get("DIFF_SHOW_PED") != "1":
            say("hideped:", ctl(a.tracer_port, "hideped on"))

        only = re.compile(a.only) if a.only else None
        dead = 0
        for st in steps:
            if st["action"] == "flow":
                continue
            if only and not (st["name"].startswith("idle") or only.search(st["name"])):
                continue
            if dead >= 3:
                # The game is gone: record the remaining steps without firing them.
                self.write(dict(st, fired=False, error="game not running", pre={}, post={}, shot=None))
                continue
            if st["action"] == "fs":
                fire = self.fire_fs(st)
            elif st["action"].startswith("key:"):
                fire = self.fire_key(st["action"][4:])
            else:
                say(f"unknown action {st['action']}")
                continue
            rec = self.run_step(st, fire)
            dead = 0 if rec["post"]["alive"] else dead + 1
            if dead == 3:
                say("game not running for 3 steps; skipping the rest")
        self.mark("end")
        return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--client", required=True, choices=["orig", "ours"])
    ap.add_argument("--out", required=True)
    ap.add_argument("--log", required=True)
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--rcon-pass", required=True)
    ap.add_argument("--client-port", type=int, required=True)
    ap.add_argument("--tracer-port", type=int, required=True)
    ap.add_argument("--only")
    a = ap.parse_args()
    d = Driver(a)
    try:
        return d.main()
    finally:
        d.steps_f.close()


if __name__ == "__main__":
    sys.exit(main())
