#!/usr/bin/env python3
"""tools/e2e/soak.sh driver for one gamemode (PLAN F7): runs the server, a UDP proxy in front
of it, our in-game client (through the proxy) and a set of samp-bot players, samples the
client and the server side by side, injects the scripted cases and writes the verdict.

    soak_mon.py run      (inside the headless display; configuration from the environment)
    soak_mon.py report OUTDIR

Environment: SOAK_ROOT (checkout with tools/), SOAK_DATA_ROOT (checkout with data/),
E2E_SERVER (prepared server copy), E2E_PORT, SOAK_PROXY_HOST, E2E_CTL_PORT, GTA_DIR,
WINEPREFIX, E2E_HEADLESS_DISPLAY, SOAK_OUT, SOAK_GAMEMODE, SOAK_SECS, SOAK_SAMPLE_SECS,
SOAK_EVENT_PERIOD, SOAK_NETDROP_SECS, SOAK_BOT (samp-bot binary).

Every process this script starts is tracked by PID and only those are stopped
(the client through its own Wine prefix: `wineserver -k` with this lane's WINEPREFIX).

Output in SOAK_OUT: samples.jsonl (one line per paired sample), events.jsonl (scripted
cases, crashes, recoveries), summary.json (verdict), client-N/ (logs of each client session),
bots/, server.log, proxy.log.
"""
import json
import math
import os
import random
import re
import signal
import socket
import statistics
import struct
import subprocess
import sys
import time

NAME = "RustClient"

# ---- tolerances --------------------------------------------------------------------------------
# Position: the acceptance values of the remote-player tests (m3.sh on foot 4 m, m5.sh vehicles 6 m),
# plus the distance covered at the entity's server-side speed in LAG_S (the client shows a remote
# entity one sync interval + network delay behind the server's copy; the two snapshots are taken
# up to ~0.15 s apart). Velocity units are GTA's (1 unit per frame = 50 m/s).
TOL_FOOT_M = 4.0
TOL_VEH_M = 6.0
LAG_S = 0.6
TOL_HP = 1.0          # local player health (the 207/200 sync carries it as a byte)
# A remote player's health arrives relayed by the server packed with the armour into one byte,
# 4 bits each in steps of 7 HP (a 43 HP bot shows as 49 = 7 x 7): one step is not a desync.
TOL_REMOTE_HP = 7.0
TOL_VEH_HP = 10.0     # vehicle health (float in 200; client physics may chip a parked car)
PERSIST = 3           # the same entity off in this many consecutive samples = persistent desync
RATE_MIN = 0.99       # steady-state comparisons within tolerance, per category
STUCK_S = 150         # unhealthy for longer than this = stuck
FREEZE_SAMPLES = 2    # ticks not advancing over this many samples while alive = frozen
MEM_SLOPE_MAX_MB_H = 40.0
# Samples are at most ~25 s apart even across a case (recover() keeps sampling); a longer gap means the
# whole run (monitor, server, game, bots) was stopped from outside, e.g. a SIGSTOP of its process group.
PAUSE_S = 60.0
# State the client takes over in steps from the server or the driver's sync: join/quit, stream in/out,
# seat, driver (RPCs) and vehicle health (a step per received sync; the server's copy of our own car's
# health likewise steps with each of our driver syncs). The client is queried after the server dump; a
# step on either side in between shows the client already in the next state. These categories are
# therefore judged against two server dumps, one before and one after the client query, and fail only
# when the client matches neither. Measured for veh_hp (derby 2 h, docs/phase3/soak-notes.md "F7
# settled"): 14 of the original's 15 and 29 of our 34 misses showed exactly the second dump's value.
BRACKETED = ("remote_known", "remote_ghost", "remote_stream", "remote_vehicle", "veh_stream", "veh_driver", "veh_hp")
WARMUP_S = 300        # memory slope: ignore the first part of each client session (and of each segment, below)
# A fresh game process keeps streaming in for longer: in derby its RSS still rose by 3-26 MB between 300 s
# and ~900 s after the launch and was flat (<= 10 MB/h) after that, on the original as on ours (2 h derby,
# docs/phase3/soak-notes.md "F7 settled"). The judged slope skips that part of a session's first segment.
WARMUP_FRESH_S = 900

# Wine's messages for a fatal exception (with WINEDEBUG=-all,err+seh) and the DLL's panic hook.
CRASH_PATTERNS = [r"wine: Unhandled", r"starting debugger", r"Unhandled exception", r"Unhandled page fault", r"err:seh", r"wine: Assertion", r"EXCEPTION_ACCESS_VIOLATION"]
PANIC_PATTERNS = [r"PANIC: "]
# Logged for the report only (not a failure by themselves).
NOTE_PATTERNS = [r"(?i)\berror\b(?!: \[\])", r"(?i)\bfailed\b(?!: \[\])"]


def now():
    return time.time()


def env(k, d=None):
    v = os.environ.get(k, d)
    if v is None:
        sys.exit("soak_mon: %s not set" % k)
    return v


class Log:
    """Incremental reader of the (appended) server log."""

    def __init__(self, path):
        self.path = path
        self.off = 0
        self.lines = []

    def poll(self):
        try:
            with open(self.path, "rb") as f:
                f.seek(self.off)
                data = f.read()
        except OSError:
            return []
        if not data:
            return []
        cut = data.rfind(b"\n")
        if cut < 0:
            return []
        self.off += cut + 1
        new = data[: cut + 1].decode("utf-8", "replace").splitlines()
        self.lines.extend(new)
        return new


class OrigView:
    """SOAK_CLIENT=orig: the original samp.dll (0.3.7-R4, 2021 build) with samp-tracer.dll in the trace
    lane. The tracer answers `state` in our client's format and `quit`; this class builds the replies
    of our client's `net`, `remote` and `vehicles` commands from the original's memory through the
    tracer's `peek`, so both clients are judged by the same compare(). The trace lane runs the 2019 build
    (B, fv0.3.7.3); addresses are for samp.dll at its preferred base 0x10000000 and are rebased on the
    load address the launcher logs in the session's run.out (Wine loads it elsewhere). Layout of B, read
    from its code: player pool loop B 0x10013790 (used flags +0xFDE, local player +0x2A), remote getter
    B 0x100010F0 (entries +0x2E, entry +0x10 = remote player), vehicle pool tick B 0x1001F370 (same
    arrays as A, remote-control-vehicles.md §1.1); pools struct: player pool at +8, vehicle pool at +0xC
    (measured: the local name in the one, the highest vehicle id in the other). Remote player +0x1DD ped
    object, ped object +0x2A4 CPed* as in player-state.md "Object map". The local player's spawned /
    class-selection flags are not read: the server's state stands in (Soak.sample). GTA fields from
    tools/diff/xclient.py."""

    NETGAME = 0x1026EA0C
    NG_STATE, NG_POOLS = 0x3CD, 0x3DE                  # netgame state (5 = connected), pools struct
    POOLS_PLAYER, POOLS_VEHICLE = 0x8, 0xC
    PP_LOCAL_ID, PP_LOCAL, PP_USED, PP_ENTRY = 0xC, 0x2A, 0xFDE, 0x2E
    E_REMOTE, E_NAME = 0x10, 0x14                      # player pool entry: RemotePlayer*, name string
    RP_PED = 0x1DD                                     # RemotePlayer: ped object (0 = not streamed in)
    PEDOBJ_CPED = 0x2A4                                # ped object: GTA CPed*
    LP_SPAWNED, LP_CLASS = 0xF0, 0x306                 # LocalPlayer: spawned, in class selection
    VP_MAX, VP_USED, VP_CVEH = 0x0, 0x3074, 0x4FB4     # vehicle pool
    PLAYERS0_PED = 0xB7CD98                            # CWorld::Players[0].m_pPed
    E_MATRIX, E_PLACEMENT, MATRIX_POS = 0x14, 0x4, 0x30
    PED_HEALTH, PED_VEHICLE, PED_FLAGS = 0x540, 0x58C, 0x46C
    VEH_HEALTH, VEH_DRIVER, VEH_PASSENGERS = 0x4C0, 0x460, 0x464
    MAX_ID = 64                                        # the soak server has max_players 50

    def __init__(self, port):
        self.port = port
        self.base = None          # samp.dll load address of the current game session
        self.run_out = None

    def session(self, run_out):
        self.run_out, self.base = run_out, None

    def _find_base(self):
        try:
            for l in open(self.run_out, errors="replace"):
                m = re.search(r"client DLL loaded at (0x[0-9a-fA-F]+) \(.*\\samp\.dll\)", l)
                if m:
                    return int(m.group(1), 16)
        except (OSError, TypeError):
            pass
        return None

    def _conn(self):
        s = socket.create_connection(("127.0.0.1", self.port), timeout=3.0)
        s.settimeout(3.0)
        return s, s.makefile("rw", errors="replace")

    def _peek(self, f, addr, n):
        out = b""
        while n > 0:
            k = min(256, n)
            f.write("peek %#x %d\n" % (addr, k))
            f.flush()
            r = f.readline().split()
            if len(r) < 3 or r[0] != "peek":
                return None
            out += bytes.fromhex("".join(r[2:]))
            addr += k
            n -= k
        return out

    def _u32(self, f, a):
        b = self._peek(f, a, 4)
        return struct.unpack("<I", b)[0] if b else 0

    def _pos(self, f, e):
        m = self._u32(f, e + self.E_MATRIX)
        b = self._peek(f, (m + self.MATRIX_POS) if m else (e + self.E_PLACEMENT), 12)
        return struct.unpack("<3f", b) if b else None

    def _string(self, f, a):
        """MSVC std::string: buffer (16 bytes inline, else a pointer), size, capacity; the allocator
        word may precede them depending on the runtime, so both placements are tried."""
        b = self._peek(f, a, 0x1C)
        if not b:
            return None
        for off in (0, 4):
            size, cap = struct.unpack_from("<II", b, off + 0x10)
            if 0 < size <= 24 and size <= cap < 0x1000:
                if cap < 16:
                    raw = b[off:off + size]
                else:
                    raw = self._peek(f, struct.unpack_from("<I", b, off)[0], size) or b""
                txt = raw.decode("latin-1")
                if txt.isprintable():
                    return txt
        return None

    def raw(self):
        """Debug: hex of the netgame, pools, player pool head and local player (offset checks)."""
        try:
            s, f = self._conn()
        except OSError:
            return None
        try:
            if self.base is None:
                self.base = self._find_base()
            if self.base is None:
                return None
            out = ["base=%#x" % self.base]
            ng = self._u32(f, self.NETGAME - 0x10000000 + self.base)
            out.append("netgame=%#x %s" % (ng, (self._peek(f, ng, 0x400) or b"").hex()))
            pools = self._u32(f, ng + self.NG_POOLS) if ng else 0
            out.append("pools=%#x %s" % (pools, (self._peek(f, pools, 0x20) or b"").hex()))
            for i in range(4):
                pp = self._u32(f, pools + 4 * i) if pools else 0
                out.append("pool%d=%#x %s" % (i, pp, (self._peek(f, pp, 0x200) or b"").hex()))
            pp = self._u32(f, pools + self.POOLS_PLAYER) if pools else 0
            if pp:
                out.append("pp1f8a %s" % (self._peek(f, pp + self.PP_ENTRY, 0x40) or b"").hex())
                lp = self._u32(f, pp + self.PP_LOCAL)
                out.append("lp=%#x %s" % (lp, (self._peek(f, lp, 0x340) or b"").hex() if lp else ""))
            return "\n".join(out)
        except (OSError, struct.error, ValueError):
            return None
        finally:
            s.close()

    def snapshot(self):
        try:
            s, f = self._conn()
        except OSError:
            return None
        try:
            return self._snapshot(f)
        except (OSError, struct.error, ValueError):
            return None
        finally:
            s.close()

    def _snapshot(self, f):
        snap = {"state": None, "local_id": None, "spawned": False, "class": False, "players": {}, "vehicles": {}}
        if self.base is None:
            self.base = self._find_base()
            if self.base is None:
                return snap
        ng = self._u32(f, self.NETGAME - 0x10000000 + self.base)
        if not ng:
            return snap
        b = self._peek(f, ng + self.NG_STATE, 4)
        snap["state"] = struct.unpack("<i", b)[0] if b else None
        pools = self._u32(f, ng + self.NG_POOLS)
        if not pools:
            return snap
        vpool, ppool = self._u32(f, pools + self.POOLS_VEHICLE), self._u32(f, pools + self.POOLS_PLAYER)
        local_ped = self._u32(f, self.PLAYERS0_PED)
        ped_ids = {}
        if ppool:
            b = self._peek(f, ppool + self.PP_LOCAL_ID, 2)
            snap["local_id"] = struct.unpack("<H", b)[0] if b else None
            lp = self._u32(f, ppool + self.PP_LOCAL)
            if lp:
                snap["spawned"] = (self._peek(f, lp + self.LP_SPAWNED, 1) or b"\0")[0] != 0
                snap["class"] = (self._peek(f, lp + self.LP_CLASS, 1) or b"\0")[0] != 0
            used = self._peek(f, ppool + self.PP_USED, 4 * self.MAX_ID) or b""
            entries = self._peek(f, ppool + self.PP_ENTRY, 4 * self.MAX_ID) or b""
            for pid in range(min(len(used), len(entries)) // 4):
                if not struct.unpack_from("<I", used, 4 * pid)[0]:
                    continue
                e = struct.unpack_from("<I", entries, 4 * pid)[0]
                if not e:
                    continue
                rp = self._u32(f, e + self.E_REMOTE)
                name = self._string(f, e + self.E_NAME)
                p = {"id": pid, "name": name or "?", "ped": 0}
                if rp:
                    po = self._u32(f, rp + self.RP_PED)
                    if po:
                        p["ped"] = self._u32(f, po + self.PEDOBJ_CPED)
                if p["ped"]:
                    ped_ids[p["ped"]] = pid
                    p["pos"] = self._pos(f, p["ped"])
                    hb = self._peek(f, p["ped"] + self.PED_HEALTH, 4)
                    p["health"] = struct.unpack("<f", hb)[0] if hb else None
                    p["cveh"] = self._u32(f, p["ped"] + self.PED_VEHICLE)
                    fl = self._peek(f, p["ped"] + self.PED_FLAGS, 4) or b"\0\0\0\0"
                    p["in_vehicle"] = bool(struct.unpack("<I", fl)[0] & (1 << 8))
                snap["players"][pid] = p
        veh_ids = {}
        if vpool:
            b = self._peek(f, vpool + self.VP_MAX, 4)
            top = min(struct.unpack("<i", b)[0] if b else 0, 1999)
            if top > 0:
                used = self._peek(f, vpool + self.VP_USED, 4 * (top + 1)) or b""
                cvs = self._peek(f, vpool + self.VP_CVEH, 4 * (top + 1)) or b""
                for vid in range(1, min(len(used), len(cvs)) // 4):
                    if not struct.unpack_from("<I", used, 4 * vid)[0]:
                        continue
                    cv = struct.unpack_from("<I", cvs, 4 * vid)[0]
                    v = {"id": vid, "ptr": cv}
                    if cv:
                        veh_ids[cv] = vid
                        v["pos"] = self._pos(f, cv)
                        hb = self._peek(f, cv + self.VEH_HEALTH, 4)
                        v["health"] = struct.unpack("<f", hb)[0] if hb else None
                        drv = self._u32(f, cv + self.VEH_DRIVER)
                        v["driver"] = "-" if not drv else ("local" if drv == local_ped else str(ped_ids.get(drv, "?")))
                        pas = self._peek(f, cv + self.VEH_PASSENGERS, 32) or b""
                        v["passengers"] = [x for x in struct.unpack_from("<8I", pas)] if len(pas) == 32 else []
                    snap["vehicles"][vid] = v
        for p in snap["players"].values():
            cv = p.get("cveh") if p.get("in_vehicle") else 0
            p["vehicle"] = veh_ids.get(cv) if cv else None
            p["seat"] = None
            if p["vehicle"] is not None:
                v = snap["vehicles"][p["vehicle"]]
                p["seat"] = 0 if self._u32(f, cv + self.VEH_DRIVER) == p["ped"] else (
                    1 + v["passengers"].index(p["ped"]) if p["ped"] in v.get("passengers", []) else None)
        return snap

    @staticmethod
    def fmt(v, n=2):
        return "-" if v is None else ",".join("%.*f" % (n, x) for x in v)

    def replies(self):
        """(net, remote lines, vehicles lines) in our client's control-channel format, or Nones."""
        sn = self.snapshot()
        if sn is None:
            return None, None, None
        net = "net phase=%s player_id=%s class_selection=%s spawned=%s orig_state=%s" % (
            "connected" if sn["state"] == 5 else "other", sn["local_id"], str(sn["class"]).lower(),
            str(sn["spawned"]).lower(), sn["state"])
        rem = ["remote count=%d" % len(sn["players"])]
        for p in sn["players"].values():
            # health: samp keeps a remote ped's CPed health at its own value (1000 measured) and the synced
            # health in its remote-player object (offset not identified in B): not compared (remote_hp)
            rem.append("remote id=%d name=%s ped=%#x pos=%s health=- cped_health=%s vehicle=%s seat=%s" % (
                p["id"], p["name"], p["ped"], self.fmt(p.get("pos")),
                "-" if p.get("health") is None else "%.1f" % p["health"],
                "-" if p.get("vehicle") is None else p["vehicle"], "-" if p.get("seat") is None else p["seat"]))
        veh = ["vehicles count=%d" % len(sn["vehicles"])]
        for v in sn["vehicles"].values():
            veh.append("vehicle id=%d ptr=%#x pos=%s health=%s driver=%s" % (
                v["id"], v["ptr"], self.fmt(v.get("pos")),
                "-" if v.get("health") is None else "%.1f" % v["health"], v.get("driver", "-")))
        return net, rem, veh


class Soak:
    def __init__(self):
        self.root = env("SOAK_ROOT")
        self.data_root = env("SOAK_DATA_ROOT")
        self.server_dir = env("E2E_SERVER")
        self.port = int(env("E2E_PORT"))
        self.proxy_host = env("SOAK_PROXY_HOST", "127.0.0.2")
        self.ctl_port = int(env("E2E_CTL_PORT"))
        self.gta_dir = env("GTA_DIR")
        self.prefix = env("WINEPREFIX")
        self.display = env("E2E_HEADLESS_DISPLAY")
        self.out = env("SOAK_OUT")
        self.gm = env("SOAK_GAMEMODE")
        self.secs = float(env("SOAK_SECS", "7200"))
        self.sample_s = float(env("SOAK_SAMPLE_SECS", "5"))
        self.event_period = float(env("SOAK_EVENT_PERIOD", "900"))
        self.netdrop_s = float(env("SOAK_NETDROP_SECS", "15"))
        self.bot_bin = env("SOAK_BOT")
        # ours: our client (tools/run-client.sh); orig: the original samp.dll + samp-tracer.dll (trace lane)
        self.kind = env("SOAK_CLIENT", "ours")
        self.orig = OrigView(self.ctl_port) if self.kind == "orig" else None
        cases = os.environ.get("SOAK_CASES")
        self.cases = list(self.CASES) if cases is None else cases.split()
        os.makedirs(self.out, exist_ok=True)
        os.makedirs(self.out + "/bots", exist_ok=True)
        self.server_log_path = self.out + "/server.log"
        self.log = Log(self.server_log_path)
        self.samples_f = open(self.out + "/samples.jsonl", "w")
        self.events_f = open(self.out + "/events.jsonl", "w")
        self.seq = 0
        self.server = None
        self.proxy = None
        self.client = None
        self.client_session = 0
        self.client_quitting = False
        self.bots = {}
        self.t0 = now()
        self.phase = "start"           # steady | event:<name> | start
        self.unhealthy_since = None
        self.last_healthy = None
        self.crashes = 0
        self.stuck = 0
        self.prev_ticks = None
        self.frozen = 0
        self.seat_transition = False   # the previous sample was a local seat transition (see compare)
        self.persist = {}              # (category, entity) -> consecutive bad samples
        self.persistent = []
        self.server_crashes = 0
        self.last_move = 0.0
        self.spectating_since = None
        self.dead_since = None
        self.class_since = None
        self.events_done = []
        self.keys_log = []
        self.scan_off = {}
        self.notes = {}
        self.panics = 0
        self.errors = 0
        self.client_started = now()
        self.last_sample_at = None
        self.client_started_wall = time.time()

    # ---- bookkeeping ---------------------------------------------------------------------------
    def t(self):
        return round(now() - self.t0, 2)

    def say(self, msg):
        print("[soak %s %7.1fs] %s" % (self.gm, now() - self.t0, msg), flush=True)

    def event(self, kind, **kw):
        kw.update(t=self.t(), kind=kind, client_session=self.client_session)
        self.events_f.write(json.dumps(kw) + "\n")
        self.events_f.flush()
        self.say("event %s %s" % (kind, {k: v for k, v in kw.items() if k not in ("t", "kind")}))

    # ---- server --------------------------------------------------------------------------------
    def server_start(self):
        self.log.poll()
        mark = len(self.log.lines)
        logf = open(self.server_log_path, "ab")
        self.server = subprocess.Popen(["./omp-server"], cwd=self.server_dir, stdout=logf, stderr=subprocess.STDOUT)
        logf.close()
        end = now() + 30
        net = fs = False
        while now() < end:
            self.log.poll()
            new = self.log.lines[mark:]
            net = net or any("Legacy Network started" in l for l in new)
            fs = fs or any("[soak] fs loaded" in l for l in new)
            if net and fs:
                self.event("server-start", pid=self.server.pid)
                return True
            time.sleep(0.25)
        self.event("server-start-failed", network=net, fs=fs)
        return False

    def server_stop(self, sig=signal.SIGTERM, timeout=15):
        if not self.server:
            return
        try:
            self.server.send_signal(sig)
            self.server.wait(timeout)
        except subprocess.TimeoutExpired:
            self.server.kill()
            self.server.wait()
        except ProcessLookupError:
            pass
        self.server = None

    def fs(self, cmd, wait=5.0):
        """Run one soakfs command; returns its output lines (without the '[soak] SEQ ' prefix)."""
        self.seq += 1
        seq = self.seq
        path = self.server_dir + "/scriptfiles/soakcmd.txt"
        try:
            with open(path) as f:
                old = f.read().splitlines()[-4:]
        except OSError:
            old = []
        tmp = path + ".tmp"
        with open(tmp, "w") as f:
            f.write("\n".join(old + ["%d %s" % (seq, cmd)]) + "\n")
        os.replace(tmp, path)
        if wait <= 0:
            return []
        rx = re.compile(r"\[soak\] %d (.*)$" % seq)
        start = len(self.log.lines)
        end = now() + wait
        out = []
        while now() < end:
            self.log.poll()
            for l in self.log.lines[start:]:
                m = rx.search(l)
                if m:
                    out.append(m.group(1))
            start = len(self.log.lines)
            if out and out[-1].startswith("done"):
                return out
            time.sleep(0.03)
        return None

    def server_dump(self):
        lines = self.fs("dump|" + NAME, wait=3.0)
        if lines is None:
            return None
        d = {"players": {}, "vehicles": {}, "local": None}
        for l in lines:
            if l.startswith("dump begin"):
                m = re.search(r"local=(\d+)", l)
                d["local"] = int(m.group(1)) if m and int(m.group(1)) < 1000 else None
            elif l.startswith("p "):
                f = dict(re.findall(r"(\w+)=(\S+)", l))
                p = {
                    "id": int(f["id"]), "name": f["name"], "st": int(f["st"]),
                    "pos": [float(x) for x in f["pos"].split(",")],
                    "speed": 50.0 * math.hypot(*[float(x) for x in f["vel"].split(",")]),
                    "hp": float(f["hp"]), "ar": float(f["ar"]), "veh": int(f["veh"]), "seat": int(f["seat"]),
                    "streamed": int(f["streamed"]),
                }
                d["players"][p["id"]] = p
            elif l.startswith("v "):
                f = dict(re.findall(r"(\w+)=(\S+)", l))
                v = {
                    "id": int(f["id"]), "model": int(f["model"]),
                    "pos": [float(x) for x in f["pos"].split(",")],
                    "speed": 50.0 * math.hypot(*[float(x) for x in f["vel"].split(",")]),
                    "hp": float(f["hp"]), "streamed": int(f["streamed"]),
                }
                d["vehicles"][v["id"]] = v
        return d

    # ---- proxy ---------------------------------------------------------------------------------
    def proxy_start(self):
        logf = open(self.out + "/proxy.log", "ab")
        self.proxy = subprocess.Popen(
            [sys.executable, "-I", self.root + "/tools/e2e/soak_udpproxy.py", "%s:%d" % (self.proxy_host, self.port), str(self.port)],
            stdout=logf, stderr=subprocess.STDOUT)
        logf.close()
        time.sleep(0.5)

    # ---- bots ----------------------------------------------------------------------------------
    def bot_specs(self):
        spec = {
            "SkWalk": ["--mode", "walk"],
            "SkIdle": ["--mode", "idle", "--special-action", "1"],
            "SkShootC": ["--mode", "shoot", "--target", NAME, "--fire-ms", "2500"],
            "SkShootB": ["--mode", "shoot", "--target", "SkWalk", "--fire-ms", "1500"],
        }
        if self.gm == "derby":
            # The gamemode puts every spawned player into its own car (RPC 70): all of them drive.
            spec["SkDrive"] = ["--mode", "drive", "--speed", "8", "--radius", "15"]
            spec["SkDriveL"] = ["--mode", "drive", "--path", "line", "--speed", "6"]
        elif self.gm == "gungame":
            spec["SkDrive"] = ["--mode", "drive", "--speed", "5", "--radius", "10"]
            spec["SkPass"] = ["--mode", "passenger"]
        else:
            spec["SkWalk2"] = ["--mode", "walk"]
        return spec

    def bots_tick(self):
        for name, args in self.bot_specs().items():
            b = self.bots.setdefault(name, {"p": None, "next": now() + random.uniform(1, 8), "runs": 0, "bad": 0})
            if b["p"] is not None:
                rc = b["p"].poll()
                if rc is None:
                    continue
                if rc != 0:
                    b["bad"] += 1
                b["p"] = None
                b["next"] = now() + random.uniform(5, 15)
            if now() < b["next"]:
                continue
            secs = random.randint(240, 720)
            b["runs"] += 1
            logf = open("%s/bots/%s-%d.log" % (self.out, name, b["runs"]), "wb")
            b["p"] = subprocess.Popen(
                [self.bot_bin, "127.0.0.1:%d" % self.port, "--name", name, "--seconds", str(secs)] + args,
                cwd=self.data_root, stdout=logf, stderr=subprocess.STDOUT)
            logf.close()

    def bots_stop(self):
        for b in self.bots.values():
            if b["p"] is not None and b["p"].poll() is None:
                b["p"].terminate()
        for b in self.bots.values():
            if b["p"] is not None:
                try:
                    b["p"].wait(5)
                except subprocess.TimeoutExpired:
                    b["p"].kill()

    # ---- client --------------------------------------------------------------------------------
    def ctl(self, cmd, multi=False, timeout=3.0):
        try:
            s = socket.create_connection(("127.0.0.1", self.ctl_port), timeout=timeout)
            s.settimeout(timeout)
            s.sendall((cmd + "\n").encode())
            f = s.makefile(errors="replace")
            head = f.readline().strip()
            lines = [head]
            if multi:
                m = re.search(r"count=(\d+)", head)
                for _ in range(int(m.group(1)) if m else 0):
                    lines.append(f.readline().strip())
            s.close()
            return lines if multi else head
        except OSError:
            return None

    def gta_pid(self):
        want = os.path.basename(self.gta_dir) + "\\gta_sa.exe"
        for p in os.listdir("/proc"):
            if not p.isdigit():
                continue
            try:
                cmd0 = open("/proc/%s/cmdline" % p, "rb").read().split(b"\0")[0].decode("utf-8", "replace")
                if not cmd0.endswith(want):
                    continue
                if ("WINEPREFIX=" + self.prefix).encode() in open("/proc/%s/environ" % p, "rb").read().split(b"\0"):
                    return int(p)
            except OSError:
                continue
        return None

    def winedbg_running(self):
        for p in os.listdir("/proc"):
            if not p.isdigit():
                continue
            try:
                if b"winedbg" not in open("/proc/%s/cmdline" % p, "rb").read():
                    continue
                if ("WINEPREFIX=" + self.prefix).encode() in open("/proc/%s/environ" % p, "rb").read().split(b"\0"):
                    return int(p)
            except OSError:
                continue
        return None

    @staticmethod
    def rss_kb(pid):
        try:
            for l in open("/proc/%d/status" % pid):
                if l.startswith("VmRSS:"):
                    return int(l.split()[1])
        except (OSError, TypeError):
            pass
        return None

    def session_dir(self):
        return "%s/client-%d" % (self.out, self.client_session)

    def client_launch(self):
        self.client_session += 1
        d = self.session_dir()
        os.makedirs(d, exist_ok=True)
        for f in ("samp-rs.log", "samp-rs-launcher.log"):
            try:
                os.remove(self.gta_dir + "/" + f)
            except OSError:
                pass
        e = dict(os.environ, SKIP_BUILD="1", E2E_HEADLESS="1", DISPLAY=self.display, SAMP_RS_CTL_PORT=str(self.ctl_port),
                 WINEDEBUG=os.environ.get("SOAK_WINEDEBUG", "-all,err+seh"))
        e.pop("WAYLAND_DISPLAY", None)
        e.pop("SAMP_RS_PRE_DLL", None)
        logf = open(d + "/run.out", "wb")
        self.client_started_wall = time.time()
        if self.orig is not None:
            # tools/trace/lib.sh trace_launch: the launcher loads samp-tracer.dll, then samp.dll
            e["TRACE_HOST"] = self.proxy_host
            cmd = ["bash", "-c", 'source "$0/tools/trace/lib.sh" && trace_launch orig "$1"', self.root, NAME]
        else:
            cmd = [self.root + "/tools/run-client.sh", "%s:%d" % (self.proxy_host, self.port), NAME]
        self.client = subprocess.Popen(cmd, stdout=logf, stderr=subprocess.STDOUT, env=e, start_new_session=True)
        if self.orig is not None:
            self.orig.session(d + "/run.out")
        logf.close()
        self.client_quitting = False
        self.client_started = now()
        self.prev_ticks = None
        self.event("client-launch")

    def client_archive(self):
        d = self.session_dir()
        for f in ("samp-rs.log", "samp-rs-launcher.log"):
            try:
                with open(self.gta_dir + "/" + f, "rb") as a, open(d + "/" + f, "wb") as b:
                    b.write(a.read())
            except OSError:
                pass

    def client_kill(self):
        if self.client is None:
            return
        try:
            os.killpg(self.client.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        subprocess.run(["wineserver", "-k"], env=dict(os.environ, WINEPREFIX=self.prefix), stderr=subprocess.DEVNULL)
        try:
            self.client.wait(15)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.client.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.client.wait()
        self.client = None

    def client_quit(self, reason):
        """Ask the game to exit (control `quit`), as a player closing the game would."""
        self.client_quitting = True
        r = self.ctl("quit")
        ok = False
        end = now() + 25
        while now() < end:
            if self.client.poll() is not None and self.gta_pid() is None:
                ok = True
                break
            time.sleep(0.5)
        self.client_archive()
        if not ok:
            self.event("client-quit-hung", reason=reason, reply=r)
        self.client_kill()
        return ok

    def scan_client_logs(self):
        """New crash / panic / note lines in the session's Wine output and DLL log."""
        found = {"crash": [], "panic": [], "note": []}
        for f in (self.session_dir() + "/run.out", self.gta_dir + "/samp-rs.log"):
            try:
                with open(f, "rb") as fh:
                    key = (self.client_session, f)
                    fh.seek(self.scan_off.get(key, 0))
                    data = fh.read()
            except OSError:
                continue
            cut = data.rfind(b"\n")
            if cut < 0:
                continue
            self.scan_off[key] = self.scan_off.get(key, 0) + cut + 1
            for line in data[: cut + 1].decode("utf-8", "replace").splitlines():
                for kind, pats in (("crash", CRASH_PATTERNS), ("panic", PANIC_PATTERNS), ("note", NOTE_PATTERNS)):
                    if any(re.search(p, line) for p in pats):
                        found[kind].append("%s: %s" % (os.path.basename(f), line[:300]))
                        break
        return found

    def check_crash(self, scan=True):
        """True (and the client relaunched) when the client crashed."""
        if self.client is None or self.client_quitting:
            return False
        dead = self.client.poll() is not None or (now() - self.client_started > 40 and self.gta_pid() is None)
        dbg = self.winedbg_running() if (scan or dead) else None
        hits = self.scan_client_logs() if (scan or dead) else {"crash": [], "panic": [], "note": []}
        for l in hits["note"]:
            self.notes[l.split("] ", 2)[-1][:160]] = self.notes.get(l.split("] ", 2)[-1][:160], 0) + 1
        if hits["panic"]:
            self.panics += len(hits["panic"])
            self.event("client-panic", lines=hits["panic"][:20])
        if not dead and dbg is None and not hits["crash"]:
            return False
        if not dead and dbg is None:
            self.errors += 1
            self.event("client-crash-marker", markers=hits["crash"][:20])
            return False
        self.crashes += 1
        self.client_archive()
        tail = []
        try:
            tail = open(self.gta_dir + "/samp-rs.log", errors="replace").read().splitlines()[-25:]
        except OSError:
            pass
        self.event("client-crash", process_gone=dead, winedbg=dbg, markers=hits["crash"][:20], log_tail=tail,
                   rc=self.client.poll())
        self.client_kill()
        time.sleep(3)
        self.client_launch()
        return True

    # ---- player input ---------------------------------------------------------------------------
    def key(self, *keys):
        subprocess.run(["xdotool", "key", "--delay", "80"] + list(keys), env=dict(os.environ, DISPLAY=self.display),
                       stderr=subprocess.DEVNULL)

    def hold(self, key, secs):
        e = dict(os.environ, DISPLAY=self.display)
        subprocess.run(["xdotool", "keydown", key], env=e, stderr=subprocess.DEVNULL)
        time.sleep(secs)
        subprocess.run(["xdotool", "keyup", key], env=e, stderr=subprocess.DEVNULL)

    def assist(self, net, sp):
        """What a player would do: spawn from class selection, respawn when spectating after a death
        (gungame waits for the secondary-attack key), move around now and then."""
        t = now()
        cls = net is not None and "class_selection=true" in net
        if cls:
            self.class_since = self.class_since or t
            if t - self.class_since > 4:
                self.key("shift")
                self.class_since = t
        else:
            self.class_since = None
        st = sp["st"] if sp else None
        if st == 9 and not cls:
            self.spectating_since = self.spectating_since or t
            if t - self.spectating_since > 8:
                self.key("f")
                self.spectating_since = t
        else:
            self.spectating_since = None
        if st in (1, 2) and t - self.last_move > 40:
            self.last_move = t
            self.hold("w", 2.0)

    # ---- sample --------------------------------------------------------------------------------
    def sample(self):
        ts = now()
        if self.last_sample_at is not None and ts - self.last_sample_at > PAUSE_S:
            self.event("paused", gap_s=round(ts - self.last_sample_at, 1), since=round(self.last_sample_at - self.t0, 1))
        self.last_sample_at = ts
        dump = self.server_dump()
        tdump = now()
        state = self.ctl("state")
        if self.orig is not None:
            net, remote, vehicles = self.orig.replies() if state is not None else (None, None, None)
            if net is not None and os.environ.get("SOAK_ORIG_RAW") and self.t() < float(os.environ["SOAK_ORIG_RAW"]):
                with open(self.session_dir() + "/orig-raw.log", "a") as lf:
                    lf.write("t=%.1f server_state=%s\n%s\n" % (self.t(), None, self.orig.raw()))
            if net is not None:
                with open(self.session_dir() + "/orig-view.log", "a") as lf:
                    lf.write("t=%.1f %s\n%s\n%s\n" % (self.t(), net, "\n".join(remote or []), "\n".join(vehicles or [])))
        else:
            net = self.ctl("net")
            remote = self.ctl("remote", multi=True)
            vehicles = self.ctl("vehicles", multi=True)
        tc = now()
        # the closing bracket of the client query (see BRACKETED)
        dump2 = self.server_dump() if dump is not None else None
        pid = self.gta_pid()
        s = {"t": self.t(), "phase": self.phase, "session": self.client_session, "dump_ms": round((tdump - ts) * 1000),
             "client_ms": round((tc - tdump) * 1000), "dump2_ms": round((now() - tc) * 1000), "alive": state is not None, "gta_pid": pid,
             "rss_kb": self.rss_kb(pid) if pid else None, "net": net, "server": dump is not None}
        # ticks / fps
        ticks = None
        if state:
            m = re.search(r"ticks=(\d+)", state)
            ticks = int(m.group(1)) if m else None
        s["ticks"] = ticks
        if ticks is not None and self.prev_ticks and self.prev_ticks[2] == self.client_session:
            dt = tc - self.prev_ticks[1]
            s["fps"] = round((ticks - self.prev_ticks[0]) / dt, 1) if dt > 0 else None
            if ticks == self.prev_ticks[0] and ticks > 0:
                self.frozen += 1
            else:
                self.frozen = 0
        if ticks is not None:
            self.prev_ticks = (ticks, tc, self.client_session)
        comps = []   # (category, entity, ok, detail)
        sp = None
        if dump is not None:
            sp = next((p for p in dump["players"].values() if p["name"] == NAME), None)
        s["server_state"] = sp["st"] if sp else None
        if self.orig is not None and net is not None and sp is not None and "spawned=false" in net and sp["st"] == 0:
            # the original's class-selection flag is not read (see OrigView); the server's state NONE of a
            # connected player is class selection
            net = net.replace("class_selection=false", "class_selection=true")
            s["net"] = net
        if self.orig is not None and net is not None and sp is not None and sp["st"] in (1, 2, 3):
            net = net.replace("spawned=false", "spawned=true")   # likewise for the spawned flag
            s["net"] = net
        connected = net is not None and "phase=connected" in net
        spawned = net is not None and "spawned=true" in net
        healthy = state is not None and connected and sp is not None and (ticks is not None)
        s["healthy"] = healthy
        if healthy and dump is not None:
            comps = self.compare(dump, sp, state, remote, vehicles, spawned)
            sp2 = next((p for p in dump2["players"].values() if p["name"] == NAME), None) if dump2 else None
            if sp2 is not None:
                later = {(c[0], c[1]): c[2] for c in self.compare(dump2, sp2, state, remote, vehicles, spawned, commit=False)
                         if c[0] in BRACKETED}
                for i, c in enumerate(comps):
                    # bad against the first dump, but the server changed it before the second (the
                    # comparison is gone or holds there): the client shows the later server state
                    if not c[2] and c[0] in BRACKETED and later.get((c[0], c[1]), True):
                        comps[i] = (c[0], c[1], True, c[3] + " [matches the dump after the client query]")
        s["n_comp"] = len(comps)
        s["bad"] = [c for c in comps if not c[2]]
        s["by_cat"] = {}
        for c in comps:
            k = s["by_cat"].setdefault(c[0], [0, 0])
            k[0] += 1
            k[1] += 0 if c[2] else 1
        # persistence (steady phase only)
        if self.phase == "steady":
            seen = set()
            for c in comps:
                key = (c[0], c[1])
                seen.add(key)
                if c[2]:
                    self.persist.pop(key, None)
                else:
                    self.persist[key] = self.persist.get(key, 0) + 1
                    if self.persist[key] == PERSIST:
                        self.persistent.append({"t": self.t(), "cat": c[0], "entity": c[1], "detail": c[3]})
                        self.event("persistent-desync", cat=c[0], entity=c[1], detail=c[3])
            for key in list(self.persist):
                if key not in seen:
                    self.persist.pop(key)
        self.samples_f.write(json.dumps(s) + "\n")
        self.samples_f.flush()
        # health of the whole setup
        if healthy:
            self.unhealthy_since = None
            self.last_healthy = now()
        else:
            self.unhealthy_since = self.unhealthy_since or now()
        if self.frozen == FREEZE_SAMPLES:
            self.event("client-frozen", ticks=ticks)
        if state is not None:
            self.assist(net, sp)
        return s

    def compare(self, dump, sp, state, remote, vehicles, spawned, commit=True):
        """Comparisons (category, entity, ok, detail) of the client's state with one server dump;
        commit=False for the bracket's second dump (leaves the seat-transition memory alone)."""
        comps = []
        # GetPlayerVelocity is 0 for a seated player: use the vehicle's velocity.
        for p in dump["players"].values():
            v = dump["vehicles"].get(p["veh"]) if p["veh"] > 0 else None
            if v is not None and p["st"] in (2, 3):
                p["speed"] = max(p["speed"], v["speed"])
        st = sp["st"]
        # Local seat transition in flight: the client sits as driver while the server still has the player on foot
        # (or the reverse), e.g. derby's OnPlayerSpawn → PutPlayerInVehicle, applied by the client at once and by
        # the server only with the client's next driver sync. The local position / driver of that one sample are
        # compared against the old seat; skipped once, judged again from the next sample on (a real mismatch persists).
        client_drives = any(re.search(r"\bdriver=local\b", l) for l in (vehicles or [])[1:])
        transition = spawned and st in (1, 2) and client_drives != (st == 2)
        skip_local = transition and not self.seat_transition
        if commit:
            self.seat_transition = transition
        # local player
        m = re.search(r"pos=(-?[\d.]+),(-?[\d.]+),(-?[\d.]+) heading=\S+ health=(-?[\d.]+)", state or "")
        if m and spawned and st in (1, 2, 3) and not skip_local:
            cpos = [float(m.group(i)) for i in (1, 2, 3)]
            chp = float(m.group(4))
            tol = (TOL_FOOT_M if st == 1 else TOL_VEH_M) + sp["speed"] * LAG_S
            d = math.dist(cpos, sp["pos"])
            comps.append(("local_pos", "local", d <= tol, "d=%.2f tol=%.2f st=%d client=%s server=%s" % (d, tol, st, cpos, sp["pos"])))
            comps.append(("local_hp", "local", abs(chp - sp["hp"]) <= TOL_HP, "client=%.1f server=%.1f" % (chp, sp["hp"])))
        # remote players
        rl = {}
        for l in (remote or [])[1:]:
            f = dict(re.findall(r"(\w+)=(\S+)", l))
            if "id" in f:
                rl[int(f["id"])] = f
        local_id = dump["local"]
        vl = {}
        hdr_ok = vehicles is not None
        for l in (vehicles or [])[1:]:
            f = dict(re.findall(r"(\w+)=(\S+)", l))
            if "id" in f:
                vl[int(f["id"])] = f
        for pid_, p in dump["players"].items():
            if pid_ == local_id:
                continue
            r = rl.get(pid_)
            # the client knows every connected player (RPC 137), streamed or not
            comps.append(("remote_known", p["name"], r is not None and r.get("name") == p["name"],
                          "server has %s(%d), client %s" % (p["name"], pid_, r.get("name") if r else None)))
            if r is None or p["streamed"] != 1 or p["st"] not in (1, 2, 3):
                continue
            has_ped = r.get("ped", "0x0") != "0x0" and r.get("pos", "-") != "-"
            comps.append(("remote_stream", p["name"], has_ped, "server: streamed in, client ped=%s pos=%s" % (r.get("ped"), r.get("pos"))))
            if not has_ped:
                continue
            cpos = [float(x) for x in r["pos"].split(",")]
            # A seated player's server position is its vehicle's (driver and passenger sync carry the vehicle
            # position); the ped itself sits at the model's seat, up to several metres from the car's origin
            # (Sandking 0.5 m, Dune 2.1 m, Bus more: measured on both clients). Compare the client's car then.
            via = ""
            cv = vl.get(int(r["vehicle"])) if p["st"] in (2, 3) and r.get("vehicle", "-").isdigit() else None
            if cv is not None and cv.get("pos", "-") != "-":
                vpos = [float(x) for x in cv["pos"].split(",")]
                via = " via=veh%s seat_offset=%.2f" % (r["vehicle"], math.dist(cpos, vpos))
                cpos = vpos
            tol = (TOL_FOOT_M if p["st"] == 1 else TOL_VEH_M) + p["speed"] * LAG_S
            d = math.dist(cpos, p["pos"])
            comps.append(("remote_pos", p["name"], d <= tol, "d=%.2f tol=%.2f st=%d client=%s server=%s%s | %s" % (
                d, tol, p["st"], cpos, p["pos"], via, " ".join("%s=%s" % (k, r.get(k)) for k in (
                    "id", "slot", "state", "sync_age_ms", "sync_pos", "teleports", "vehicle", "veh_sync")))))
            try:
                chp = float(r["health"])
                comps.append(("remote_hp", p["name"], abs(chp - p["hp"]) <= TOL_REMOTE_HP, "client=%.0f server=%.1f" % (chp, p["hp"])))
            except (KeyError, ValueError):
                pass
            if p["st"] in (2, 3):
                cveh = r.get("vehicle", "-")
                comps.append(("remote_vehicle", p["name"], cveh == str(p["veh"]),
                              "server veh %d seat %d, client vehicle=%s seat=%s" % (p["veh"], p["seat"], cveh, r.get("seat"))))
        # every player the client lists is one comparison (a ghost is the failing case)
        for cid, r in rl.items():
            if cid == local_id:
                continue
            comps.append(("remote_ghost", r.get("name", str(cid)), cid in dump["players"],
                          "client lists id %d (%s), server does not" % (cid, r.get("name"))))
        # vehicles
        drivers = {p["veh"]: p for p in dump["players"].values() if p["st"] == 2 and p["veh"] > 0}
        for vid, v in dump["vehicles"].items():
            if v["streamed"] != 1 or not hdr_ok:
                continue
            c = vl.get(vid)
            ok = c is not None and c.get("ptr", "0x0") != "0x0" and c.get("pos", "-") != "-"
            comps.append(("veh_stream", "veh%d" % vid, ok, "server: streamed in, client %s" % (c and {k: c.get(k) for k in ("ptr", "pos")})))
            if not ok:
                continue
            cpos = [float(x) for x in c["pos"].split(",")]
            tol = TOL_VEH_M + v["speed"] * LAG_S
            d = math.dist(cpos, v["pos"])
            comps.append(("veh_pos", "veh%d" % vid, d <= tol, "d=%.2f tol=%.2f client=%s server=%s driver=%s" % (
                d, tol, cpos, v["pos"], c.get("driver"))))
            try:
                comps.append(("veh_hp", "veh%d" % vid, abs(float(c["health"]) - v["hp"]) <= TOL_VEH_HP,
                              "client=%s server=%.1f" % (c["health"], v["hp"])))
            except (KeyError, ValueError):
                pass
            dp = drivers.get(vid)
            want = "-" if dp is None else ("local" if dp["id"] == local_id else str(dp["id"]))
            if skip_local and (dp is None or dp["id"] == local_id or c.get("driver") == "local"):
                continue
            if dp is None or dp["id"] == local_id or dp["streamed"] == 1:
                comps.append(("veh_driver", "veh%d" % vid, c.get("driver") == want, "client driver=%s server=%s" % (c.get("driver"), want)))
        return comps

    # ---- recovery ------------------------------------------------------------------------------
    def chatlog_path(self):
        """The original's chat log (samp.dll appends every chat entry to chatlog.txt in the user files
        dir, docs/re/subsystems/ui-chat-audio.md "Logging"); the tracer serves no `chat` command."""
        return self.prefix + "/drive_c/users/%s/Documents/GTA San Andreas User Files/SAMP/chatlog.txt" % os.environ.get("USER", "")

    def chat_lines(self, n):
        if self.orig is None:
            return self.ctl("chat %d" % n, multi=True) or []
        # the chat window opens the file with mode "w" (truncated at every game start,
        # docs/re/subsystems/ui-rendering.md): a file not written since this session's launch is the last one's
        try:
            p = self.chatlog_path()
            if os.path.getmtime(p) < self.client_started_wall:
                return []
            with open(p, "rb") as f:
                return f.read().decode("cp1252", "replace").splitlines()[-n:]
        except OSError:
            return []

    def chat_has(self, text, n=60):
        return any(text in l for l in self.chat_lines(n))

    def say(self, text):
        """A chat line from the player: our client's control `say`; the original's chat input typed on the
        game window (T, the text, Enter), as a player would."""
        if self.orig is None:
            return self.ctl("say " + text)
        self.key("t")
        time.sleep(0.3)
        subprocess.run(["xdotool", "type", "--delay", "40", text], env=dict(os.environ, DISPLAY=self.display),
                       stderr=subprocess.DEVNULL)
        self.key("Return")
        return "typed"

    def recover(self, name, deadline=STUCK_S, relaunch_on=None):
        """Sample until the client is healthy and spawned again; relaunch_on: chat text after which a
        player would restart the game (no automatic reconnect in that path)."""
        start = now()
        relaunched = False
        while now() - start < deadline:
            self.bots_tick()
            if self.check_crash():
                relaunched = True
            s = self.sample()
            if relaunch_on and not relaunched and self.client and self.chat_has(relaunch_on):
                self.event("relaunch-after", case=name, chat=relaunch_on)
                time.sleep(5)
                self.client_quit(name)
                time.sleep(2)
                self.client_launch()
                relaunched = True
            if s["healthy"] and s["server_state"] in (1, 2, 3):
                return round(now() - start, 1)
            time.sleep(self.sample_s / 2)
        return None

    # ---- scripted cases --------------------------------------------------------------------------
    def case_gmx(self):
        self.fs("rcon|gmx", wait=3)
        restarting = False
        for _ in range(20):
            if self.chat_has("The server is restarting.."):
                restarting = True
                break
            time.sleep(0.5)
        net = self.ctl("net")
        rec = self.recover("gmx")
        return {"restart_line": restarting, "net_after": net, "recovered_s": rec, "expect_ok": restarting}

    def case_kick(self):
        self.fs("kick|" + NAME, wait=3)
        closed = False
        for _ in range(20):
            if self.chat_has("Server closed the connection."):
                closed = True
                break
            time.sleep(0.5)
        time.sleep(10)
        alive = self.ctl("ping") == "pong"
        n_conn = len([l for l in self.log.lines if re.search(r"\[e2e\] connect id=\d+ name=%s\b" % NAME, l)])
        time.sleep(5)
        self.log.poll()
        n_conn2 = len([l for l in self.log.lines if re.search(r"\[e2e\] connect id=\d+ name=%s\b" % NAME, l)])
        quit_ok = self.client_quit("kick")
        time.sleep(2)
        self.client_launch()
        rec = self.recover("kick", deadline=STUCK_S + 60)
        return {"closed_line": closed, "alive_after_kick": alive, "auto_reconnect": n_conn2 > n_conn,
                "quit_clean": quit_ok, "recovered_s": rec,
                "expect_ok": closed and alive and n_conn2 == n_conn and quit_ok}

    def case_netloss(self):
        """The client's path is cut for netdrop_s. A chat line typed 1 s in is a reliable send that
        stays unacknowledged, so the client must declare the connection lost (RakNet's ack timer:
        about 10 s after the first unacked reliable send, docs/phase3/trace-notes.md TRACE-8); an
        idle client sends only unreliable syncs and would ride out the gap. When the server still
        holds the old player at the reconnect, the nick is rejected and (as with samp) nothing
        retries: then the game is started again."""
        os.kill(self.proxy.pid, signal.SIGUSR1)
        t = now()
        time.sleep(1)
        said = self.say("soak netloss probe")
        while now() - t < self.netdrop_s:
            time.sleep(0.5)
        os.kill(self.proxy.pid, signal.SIGUSR2)
        lost_during = self.chat_has("Lost connection to the server. Reconnecting..", 100)
        net_mid = self.ctl("net")
        rec = self.recover("netloss", deadline=STUCK_S + 60, relaunch_on="CONNECTION REJECTED: Unacceptable NickName")
        lost = lost_during or self.chat_has("Lost connection to the server. Reconnecting..", 100)
        return {"drop_s": self.netdrop_s, "say": said, "lost_line": lost, "lost_within_drop": lost_during,
                "net_after_drop": net_mid, "recovered_s": rec, "expect_ok": bool(lost)}

    def case_srvkill(self):
        self.server_stop(signal.SIGKILL)
        time.sleep(5)
        ok = self.server_start()
        if ok:
            self.fs_setup()
        rec = self.recover("srvkill", deadline=STUCK_S + 30)
        lost = self.chat_has("Lost connection to the server. Reconnecting..", 100)
        return {"server_up": ok, "lost_line": lost, "recovered_s": rec, "expect_ok": ok and lost}

    def case_srvrestart(self):
        self.server_stop(signal.SIGTERM)
        time.sleep(3)
        closed = self.chat_has("Server closed the connection.", 100)
        ok = self.server_start()
        if ok:
            self.fs_setup()
        rec = self.recover("srvrestart", deadline=STUCK_S + 60, relaunch_on="Server closed the connection.")
        return {"server_up": ok, "closed_line": closed, "recovered_s": rec, "expect_ok": ok}

    # Cases that keep the game process first, so the first client session is long (memory slope).
    CASES = ["gmx", "netloss", "srvkill", "kick", "srvrestart"]

    def fs_setup(self):
        if self.gm == "gungame":
            self.fs("vehspot|400|-1300.0|2506.0|88.0|0.0|3")

    # ---- main loop -------------------------------------------------------------------------------
    def capture_start(self):
        """SOAK_CAPTURE=1: loopback capture of the server port (both the proxy's and the bots' traffic)
        into capture.pcapng, for samp-dump."""
        self.capture = None
        if os.environ.get("SOAK_CAPTURE") != "1":
            return
        self.capture = subprocess.Popen(
            ["dumpcap", "-q", "-i", "lo", "-f", "udp port %d" % self.port, "-w", self.out + "/capture.pcapng"],
            stdout=subprocess.DEVNULL, stderr=open(self.out + "/dumpcap.out", "wb"))
        time.sleep(1)

    def run(self):
        self.say("gamemode %s, %ds, sample every %ss" % (self.gm, self.secs, self.sample_s))
        self.capture_start()
        self.proxy_start()
        if not self.server_start():
            return self.finish(fatal="server did not start")
        self.fs_setup()
        self.client_launch()
        rec = self.recover("join", deadline=240)
        self.event("joined", recovered_s=rec)
        if rec is None:
            return self.finish(fatal="client never joined and spawned")
        self.phase = "steady"
        n_cases = len(self.cases)
        period = min(self.event_period, max(60.0, (self.secs - (now() - self.t0)) / (n_cases + 1)))
        next_case = now() + period
        ci = 0
        last_sample = 0
        while True:
            elapsed = now() - self.t0
            if elapsed >= self.secs and ci >= n_cases:
                break
            self.bots_tick()
            if self.server is not None and self.server.poll() is not None:
                self.server_crashes += 1
                self.event("server-exited", rc=self.server.returncode)
                self.server = None
                self.server_start()
                self.fs_setup()
            self.check_crash(scan=False)
            if n_cases and now() >= next_case and (elapsed < self.secs or ci < n_cases):
                name = self.cases[ci % n_cases]
                ci += 1
                self.phase = "event:" + name
                self.event("case-start", case=name)
                res = getattr(self, "case_" + name)()
                ok = res.get("recovered_s") is not None and res.get("expect_ok", True)
                self.events_done.append({"case": name, "ok": ok, **res})
                self.event("case-end", case=name, ok=ok, **res)
                if res.get("recovered_s") is None:
                    self.stuck += 1
                    # get going again for the rest of the soak
                    self.client_quit("stuck-" + name)
                    self.client_launch()
                    self.recover("restart-after-stuck", deadline=240)
                self.phase = "steady"
                # a grace period after each case before steady samples count again
                grace_end = now() + 10
                while now() < grace_end:
                    self.phase = "grace"
                    self.sample()
                    time.sleep(self.sample_s)
                self.phase = "steady"
                remaining = self.secs - (now() - self.t0)
                left = n_cases - ci
                next_case = now() + (period if left <= 0 or remaining <= 0 else min(period, max(30.0, remaining / (left + 1))))
                if remaining <= 0 and left > 0:
                    next_case = now() + 20
                continue
            if now() - last_sample >= self.sample_s:
                last_sample = now()
                self.check_crash()
                s = self.sample()
                if self.unhealthy_since and now() - self.unhealthy_since > STUCK_S:
                    self.stuck += 1
                    self.event("stuck", since=round(self.unhealthy_since - self.t0, 1), net=s.get("net"),
                               server_state=s.get("server_state"))
                    self.client_quit("stuck")
                    self.client_launch()
                    self.unhealthy_since = None
                    self.recover("restart-after-stuck", deadline=240)
                elapsed_i = int(now() - self.t0)
                if elapsed_i % 60 < self.sample_s:
                    self.say("alive=%s healthy=%s fps=%s rss=%s comps=%d bad=%d" % (
                        s["alive"], s["healthy"], s.get("fps"), s.get("rss_kb"), s["n_comp"], len(s["bad"])))
            time.sleep(0.5)
        return self.finish()

    def cleanup(self):
        try:
            if self.client is not None:
                self.client_quit("end")
        except Exception as e:  # noqa: BLE001 - keep tearing down
            self.say("client quit failed: %s" % e)
            self.client_kill()
        self.bots_stop()
        self.server_stop()
        if getattr(self, "capture", None) is not None:
            self.capture.terminate()
            self.capture.wait()
        if self.proxy is not None:
            self.proxy.terminate()
            try:
                self.proxy.wait(5)
            except subprocess.TimeoutExpired:
                self.proxy.kill()

    def finish(self, fatal=None):
        self.cleanup()
        self.samples_f.close()
        self.events_f.close()
        summ = report(self.out, extra={
            "fatal": fatal, "crashes": self.crashes, "panics": self.panics, "crash_markers": self.errors,
            "log_notes": dict(sorted(self.notes.items(), key=lambda kv: -kv[1])[:40]), "stuck": self.stuck, "server_crashes": self.server_crashes,
            "cases": self.events_done, "bots": {k: {"runs": v["runs"], "nonzero_exits": v["bad"]} for k, v in self.bots.items()},
            "gamemode": self.gm, "soak_secs": self.secs, "client": self.kind, "cases_planned": self.cases, "wall_secs": round(now() - self.t0, 1),
        })
        return 0 if summ["pass"] else 1


# ---- report ----------------------------------------------------------------------------------------
def linreg(xs, ys):
    n = len(xs)
    if n < 3:
        return None
    mx, my = sum(xs) / n, sum(ys) / n
    sxx = sum((x - mx) ** 2 for x in xs)
    if sxx == 0:
        return None
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sxx


def report(out, extra=None):
    samples = [json.loads(l) for l in open(out + "/samples.jsonl")]
    events = [json.loads(l) for l in open(out + "/events.jsonl")]
    sp = out + "/summary.json"
    prev = {}
    if extra is None and os.path.exists(sp):
        prev = json.load(open(sp))
    extra = extra if extra is not None else {k: prev.get(k) for k in ("fatal", "crashes", "panics", "crash_markers", "log_notes", "stuck", "server_crashes", "cases", "bots", "gamemode", "soak_secs", "wall_secs", "client", "cases_planned")}
    steady = [s for s in samples if s["phase"] == "steady"]
    cats = {}
    for s in steady:
        for c, (n, bad) in s.get("by_cat", {}).items():
            k = cats.setdefault(c, [0, 0])
            k[0] += n
            k[1] += bad
    rates = {c: {"n": n, "bad": b, "ok_rate": round(1 - b / n, 5) if n else None} for c, (n, b) in sorted(cats.items())}
    total_n = sum(n for n, _ in cats.values())
    total_bad = sum(b for _, b in cats.values())
    overall = round(1 - total_bad / total_n, 5) if total_n else None
    healthy_steady = [s for s in steady if s.get("healthy")]
    # fps (steady, healthy)
    fps = [s["fps"] for s in healthy_steady if s.get("fps")]
    fps_stats = None
    if fps:
        fs = sorted(fps)
        fps_stats = {"median": statistics.median(fs), "p5": fs[int(0.05 * (len(fs) - 1))], "min": fs[0], "max": fs[-1], "n": len(fs)}
    # pauses: the whole run stopped from outside (see PAUSE_S). Nothing ran during one, so the time
    # axis of the memory slope skips it (otherwise a stopped session looks like hours of uptime with
    # its warm-up RSS growth crammed into the last seconds).
    pauses = [{"at": a["t"], "gap_s": round(b["t"] - a["t"], 1)} for a, b in zip(samples, samples[1:]) if b["t"] - a["t"] > PAUSE_S]

    def active(t):
        return t - sum(p["gap_s"] for p in pauses if p["at"] < t)

    # memory: per client session, after warm-up
    mem = {}
    for s in samples:
        if s.get("rss_kb") and s.get("session"):
            mem.setdefault(s["session"], []).append((active(s["t"]), s["rss_kb"] / 1024.0))
    mem_sessions = []
    for sess, pts in sorted(mem.items()):
        t0 = pts[0][0]
        warm = [(t, m) for t, m in pts if t - t0 >= WARMUP_S]
        slope = linreg([t for t, _ in warm], [m for _, m in warm])
        mem_sessions.append({"session": sess, "span_s": round(pts[-1][0] - t0), "first_mb": round(pts[0][1]),
                             "last_mb": round(pts[-1][1]), "max_mb": round(max(m for _, m in pts)),
                             "slope_mb_per_h": round(slope * 3600, 1) if slope is not None else None,
                             "warm_span_s": round(warm[-1][0] - warm[0][0]) if len(warm) > 1 else 0})
    # steady-phase slope (the RSS between the cases) and the RSS step across each case that kept
    # the game process (in-process reconnects: gmx, netloss without relaunch, srvkill)
    for m in mem_sessions:
        pts = [(active(s["t"]), s["rss_kb"] / 1024.0) for s in samples
               if s.get("session") == m["session"] and s.get("rss_kb") and s["phase"] == "steady"]
        if pts:
            t0 = min(active(s["t"]) for s in samples if s.get("session") == m["session"])
            pts = [(t, v) for t, v in pts if t - t0 >= WARMUP_S]
        sl = linreg([t for t, _ in pts], [v for _, v in pts]) if pts else None
        m["steady_slope_mb_per_h"] = round(sl * 3600, 1) if sl is not None else None
        m["steady_span_s"] = round(pts[-1][0] - pts[0][0]) if len(pts) > 1 else 0
    steps = []
    starts = [e for e in events if e["kind"] == "case-start"]
    ends = [e for e in events if e["kind"] == "case-end"]
    for st, en in zip(starts, ends):
        before = [s for s in samples if s["t"] <= st["t"] and s.get("rss_kb") and s.get("session") == st["client_session"]]
        after = [s for s in samples if s["t"] >= en["t"] and s.get("rss_kb") and s.get("session") == st["client_session"]][:3]
        if before and after:
            steps.append({"case": st["case"], "rss_step_mb": round((after[-1]["rss_kb"] - before[-1]["rss_kb"]) / 1024.0, 1)})
    # The judged slope. An in-process reconnect (gmx, netloss, srvkill: the reset routine deletes every
    # remote entity and the new join streams the world in again) is a warm-up of its own: the RSS climbs
    # for minutes after it, on the original as on ours. The slope is therefore fitted within segments
    # (session start / case end + WARMUP_S ... next case start), pooled (one common slope, an intercept
    # per segment), so neither the re-warm-ups nor the steps across the cases (reported separately in
    # memory_case_steps) count as a trend. Judged when the segments cover >= 600 s.
    # The first segment of a session starts WARMUP_FRESH_S after the game launch (late streaming warm-up).
    case_spans = [(st["t"], en["t"]) for st, en in zip(starts, ends)]
    for m in mem_sessions:
        ss = [s for s in samples if s.get("session") == m["session"] and s.get("rss_kb")]
        bounds = [ss[0]["t"]] + [en for st, en in case_spans if ss[0]["t"] < st < ss[-1]["t"]]
        segs = {}
        for s in ss:
            if s["phase"] != "steady":
                continue
            b = max(x for x in bounds if x <= s["t"])
            if active(s["t"]) - active(b) >= (WARMUP_FRESH_S if b == bounds[0] else WARMUP_S):
                segs.setdefault(b, []).append((active(s["t"]), s["rss_kb"] / 1024.0))
        sxy = sxx = span = 0.0
        m["segments"] = []
        for b, v in sorted(segs.items()):
            if len(v) < 3 or v[-1][0] - v[0][0] < 60:
                continue
            mx, my = sum(t for t, _ in v) / len(v), sum(y for _, y in v) / len(v)
            sxy += sum((t - mx) * (y - my) for t, y in v)
            sxx += sum((t - mx) ** 2 for t, _ in v)
            span += v[-1][0] - v[0][0]
            sl = linreg([t for t, _ in v], [y for _, y in v])
            m["segments"].append({"from": round(v[0][0]), "to": round(v[-1][0]), "fresh": b == bounds[0],
                                  "slope_mb_per_h": round(sl * 3600, 1) if sl is not None else None,
                                  "rise_mb": round(v[-1][1] - v[0][1], 1)})
        m["judged_slope_mb_per_h"] = round(sxy / sxx * 3600, 1) if sxx else None
        m["judged_span_s"] = round(span)
    longest = max(mem_sessions, key=lambda m: m["judged_span_s"], default=None)
    frozen = sum(1 for e in events if e["kind"] == "client-frozen")
    persistent = [e for e in events if e["kind"] == "persistent-desync"]
    crash_events = [e for e in events if e["kind"] == "client-crash"]
    reasons = []
    if extra.get("fatal"):
        reasons.append("fatal: %s" % extra["fatal"])
    if crash_events:
        reasons.append("%d client crash(es)" % len(crash_events))
    if extra.get("panics"):
        reasons.append("%d caught panic(s) in the DLL log" % extra["panics"])
    if extra.get("crash_markers"):
        reasons.append("%d Wine exception marker(s) with the game still running" % extra["crash_markers"])
    if extra.get("stuck"):
        reasons.append("%d stuck state(s)" % extra["stuck"])
    if frozen:
        reasons.append("%d frozen frame counter(s)" % frozen)
    planned = extra.get("cases_planned")
    missing = [c for c in (Soak.CASES if planned is None else planned) if not any(d["case"] == c for d in (extra.get("cases") or []))]
    if missing:
        reasons.append("cases not run: %s" % missing)
    for d in extra.get("cases") or []:
        if not d.get("ok"):
            reasons.append("case %s failed: %s" % (d["case"], {k: v for k, v in d.items() if k not in ("case", "net_after", "net_after_drop")}))
    for c, r in rates.items():
        if r["ok_rate"] is not None and r["ok_rate"] < RATE_MIN:
            reasons.append("desync %s: %.2f%% within tolerance (< %.1f%%)" % (c, 100 * r["ok_rate"], 100 * RATE_MIN))
    if persistent:
        reasons.append("%d persistent desync(s) (>= %d consecutive samples)" % (len(persistent), PERSIST))
    if fps_stats and (fps_stats["p5"] < 0.5 * fps_stats["median"] or fps_stats["median"] < 20):
        reasons.append("fps unstable: %s" % fps_stats)
    if longest and longest["judged_slope_mb_per_h"] is not None and longest["judged_span_s"] >= 600 \
            and longest["judged_slope_mb_per_h"] > MEM_SLOPE_MAX_MB_H:
        reasons.append("memory slope %.1f MB/h > %.0f (session %d, %d s judged)" % (
            longest["judged_slope_mb_per_h"], MEM_SLOPE_MAX_MB_H, longest["session"], longest["judged_span_s"]))
    if not steady:
        reasons.append("no steady samples")
    if pauses:
        reasons.append("run stopped from outside for %s s (at t=%s): it did not cover its span, rerun" % (
            "+".join(str(round(p["gap_s"])) for p in pauses), ",".join(str(round(p["at"])) for p in pauses)))
    summ = dict(extra)
    summ.update({
        "pass": not reasons, "reasons": reasons, "samples": len(samples), "steady_samples": len(steady),
        "steady_healthy": len(healthy_steady), "desync": rates, "desync_overall_ok_rate": overall,
        "comparisons": total_n, "fps": fps_stats, "memory": mem_sessions,
        "memory_slope_mb_per_h": longest["judged_slope_mb_per_h"] if longest else None,
        "memory_case_steps": steps, "pauses": pauses,
        "persistent_desyncs": persistent[:50], "crash_events": crash_events,
    })
    json.dump(summ, open(sp, "w"), indent=1)
    print("[soak] %s: %s" % (summ.get("gamemode"), "PASS" if summ["pass"] else "FAIL: " + "; ".join(reasons)))
    print("[soak]   samples %d (steady %d, healthy %d), comparisons %d, overall within tolerance %s" % (
        len(samples), len(steady), len(healthy_steady), total_n, "%.3f%%" % (100 * overall) if overall is not None else "-"))
    for c, r in rates.items():
        print("[soak]   %-15s n=%-6d bad=%-5d ok=%s" % (c, r["n"], r["bad"], "%.3f%%" % (100 * r["ok_rate"]) if r["ok_rate"] is not None else "-"))
    print("[soak]   fps %s" % fps_stats)
    for m in mem_sessions:
        print("[soak]   memory session %d: %s" % (m["session"], m))
    print("[soak]   memory steps across in-process cases: %s" % steps)
    for c in extra.get("cases") or []:
        print("[soak]   case %s" % c)
    return summ


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "run":
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
        soak = Soak()
        try:
            rc = soak.run()
        except BaseException:
            soak.cleanup()
            raise
        sys.exit(rc)
    elif len(sys.argv) >= 3 and sys.argv[1] == "report":
        sys.exit(0 if report(sys.argv[2])["pass"] else 1)
    else:
        sys.exit(__doc__)
