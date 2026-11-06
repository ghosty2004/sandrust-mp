#!/usr/bin/env python3
"""DIFF-3 (criterion F3): the original client and ours on ONE server at the
same time; each side's view of the other player is compared with that
player's own view of itself, both directions.

    xclient.py drive  --out DIR --log SERVER_LOG --port UDP --rcon-pass PASS
                      --a-port N --a-display D --a-name NAME --a-kind orig|ours
                      --b-port N --b-display D --b-name NAME --b-kind orig|ours
                      [--patches-only]
    xclient.py report --run DIR [--out target/diff]

Side A is the lane "diff" game (the original client + tracer, control port
47706), side B the lane "diff2" game (our client, 47705; or a second original
client with --calibrate). Both games run on their own isolated display; keys go
to one display only (xdotool, DISPLAY of that side).

What is compared — always through the same code on both sides, `peek` on the
control channel (gta_sa.exe 1.0 US memory; offsets from plugin-sdk
VALIDATE_OFFSET / *.cpp, named next to each constant):
  * the LOCAL ped of side S (CWorld::Players[0].m_pPed) and
  * the ped side O shows for S: from O's `remote` reply (ours: the line with
    S's name; tracer: the only cached remote ped, Players[slot >= 1]).
Fields: model (skin), position, heading, health, armour, move state, ducking /
in-the-air / standing flags, and the animation blend list (hierarchy hash,
group, id, blend amount) of the ped's RpClump.

Each step: pre samples -> action (keys on the actor's display or RCON to the
xclient filterscript) -> samples for the step window -> `state`/`remote` of
both sides and the server's view (`xc info`). DIR/samples.jsonl holds it all.
"""
import argparse
import concurrent.futures
import json
import math
import os
import re
import socket
import struct
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import drive  # noqa: E402  (ctl, rcon, Log from the DIFF-2 driver)
import patchrows  # noqa: E402

# --- gta_sa.exe 1.0 US (plugin-sdk) ------------------------------------------------
PLAYERS0_PED = 0xB7CD98            # CWorld::Players[0].m_pPed (CWorld.cpp)
CLUMP_OFFSET = 0xB5F878            # ClumpOffset (common.cpp): RpClump -> CAnimBlendClumpData*
E_MATRIX, E_RWOBJECT, E_MODEL = 0x14, 0x18, 0x22      # CPlaceable::m_matrix, CEntity::m_pRwClump, m_nModelIndex
E_PLACEMENT = 0x4                  # CPlaceable::m_placement (pos when there is no matrix)
PHYS_SPEED = 0x44                  # CPhysical::m_vecMoveSpeed
PED_FLAGS = 0x46C                  # CPed bitfield block after field_468 (CPed.h, pack 1)
PED_STATE, PED_MOVE = 0x530, 0x534  # m_nPedState, m_nMoveState
PED_HEALTH, PED_ARMOUR, PED_HEADING = 0x540, 0x548, 0x558
PED_WEAPONS, PED_WEAPON_SIZE, PED_SELECTED_SLOT = 0x5A0, 0x1C, 0x718  # m_aWeapons[13], m_nSelectedWepSlot (M7)
# CAnimBlendAssociation: m_link +4 (RwLLLink next,prev), m_nAnimGroup +0xE,
# m_pHierarchy +0x14, m_fBlendAmount +0x18, m_nAnimId +0x2C, m_nFlags +0x2E; size 0x3C.
# CAnimBlendHierarchy::m_hashKey +0. CAnimBlendClumpData::m_associationsList +0.
ASSOC_SIZE = 0x3C
PED_VEHICLE = 0x58C                # CPed::m_pVehicle; in-vehicle = bit 8 of the PED_FLAGS block (bInVehicle)
# CVehicle (plugin-sdk CVehicle.h VALIDATE_OFFSET): m_anUpgrades[15] +0x43A, m_pDriver +0x460,
# m_apPassengers[8] +0x464; CMatrix forward (+0x10) and pos (+0x30).
VEH_UPGRADES, VEH_DRIVER, VEH_PASSENGERS = 0x43A, 0x460, 0x464
MOVE_NAMES = {0: "none", 1: "still", 2: "turn_l", 3: "turn_r", 4: "walk", 5: "jog", 6: "run", 7: "sprint"}

# --- scenario -------------------------------------------------------------------------
# LSIA runway: open and flat. A stands 2.5 m north of the line, B 2.5 m south;
# both face west (SA:MP angle 90).
BASE = (1700.0, -2490.0, 13.55)
SPOT = {"A": (BASE[0], BASE[1] + 2.5, BASE[2], 90.0), "B": (BASE[0], BASE[1] - 2.5, BASE[2], 90.0)}
SKIN_STEP = {"A": 101, "B": 115}

# Check tolerances. Position/heading: the remote ped is placed from the 207
# packets. With a --calibrate run (original vs original) the report uses
# 1.5 x the original's own largest deviation (at least TOL_FLOOR); TOL are
# the defaults without one. See docs/phase3/diff-notes.md "DIFF-3".
TOL = {"pos": 0.5, "heading": 0.15, "health": 1.0, "armour": 1.0, "veh_pos": 1.0, "veh_heading": 0.15}
TOL_FLOOR = {"pos": 0.1, "heading": 0.03}

SAMPLE_PERIOD = 0.1

STATIC = ["visible", "pos", "heading", "skin", "health", "armour", "ducking", "anim_final"]


def actor_steps(a):
    return [
        {"name": f"walk_{a}", "actor": a, "events": [(0.0, "keydown", "w"), (2.0, "keyup", "w")], "window": 5.0,
         "checks": ["visible", "pos", "heading", "move", "anims", "anim_final"]},
        {"name": f"turn_{a}", "actor": a, "events": [(0.0, "keydown", "a"), (0.6, "keyup", "a")], "window": 3.5,
         "checks": ["visible", "pos", "heading", "anims"]},
        {"name": f"jump_{a}", "actor": a, "events": [(0.0, "key", "shift")], "window": 3.0,
         "checks": ["visible", "in_air", "anims", "pos"]},
        {"name": f"crouch_{a}", "actor": a, "events": [(0.0, "key", "c")], "window": 3.0,
         "checks": ["visible", "ducking", "anims", "anim_final", "pos", "heading"]},
        {"name": f"uncrouch_{a}", "actor": a, "events": [(0.0, "key", "c")], "window": 3.0,
         "checks": ["visible", "ducking", "anim_final", "pos"]},
    ]


STEPS = (
    [{"name": "spawn_idle", "actor": "both", "events": [], "window": 2.0, "checks": STATIC}]
    + [{"name": "health_armour", "actor": "both", "events": [(0.0, "rcon_each", "xc hpar {id} 64 37 ;")],
        "window": 3.0, "checks": ["visible", "health", "armour", "pos", "heading"]}]
    + [{"name": "tp_A", "actor": "both", "events": [(0.0, "tp", None)], "window": 3.0, "checks": STATIC}]
    + actor_steps("A")
    + [{"name": "tp_B", "actor": "both", "events": [(0.0, "tp", None)], "window": 3.0, "checks": STATIC}]
    + actor_steps("B")
    + [{"name": "handsup", "actor": "both", "events": [(0.0, "rcon_each", "xc sa {id} 10 ;")], "window": 3.0,
        "checks": ["visible", "anims", "anim_final", "pos"]},
       {"name": "handsup_off", "actor": "both", "events": [(0.0, "rcon_each", "xc sa {id} 0 ;")], "window": 3.0,
        "checks": ["visible", "anim_final", "pos"]},
       {"name": "skin", "actor": "both", "events": [(0.0, "rcon_each", "xc skin {id} {skin} ;")], "window": 4.0,
        "checks": ["visible", "skin", "pos", "heading"]}]
)

# M7 combat steps: weapons (RPC 22 → the other side sees the weapon through 207), aiming (203 + keys → the
# aim pose / anims on the other side), one shot into the air (206 miss relayed → shot anims), death by health 0
# (RPC 53 → 166 on the other side: the ped dies there) and the respawn.
def combat_steps(a):
    return [
        {"name": f"aim_{a}", "actor": a, "events": [(0.0, "mousedown", "3"), (2.5, "mouseup", "3")], "window": 3.5,
         "checks": ["visible", "weapon", "anims", "pos"]},
        {"name": f"fire_{a}", "actor": a,
         "events": [(0.0, "mousedown", "3"), (1.2, "mousedown", "1"), (1.4, "mouseup", "1"), (2.6, "mouseup", "3")],
         "window": 3.5, "checks": ["visible", "weapon", "anims", "pos"]},
    ]


STEPS = STEPS + (
    [{"name": "combat_weapon", "actor": "both", "events": [(0.0, "rcon_each", "xc weapon {id} 24 50 ;")],
      "window": 3.0, "checks": ["visible", "weapon", "pos", "heading"]}]
    + combat_steps("A") + combat_steps("B")
    + [{"name": "combat_death_A", "actor": "A", "events": [(0.0, "rcon_each", "xc hpar {id} 0 0 ;")], "window": 3.0,
        "checks": ["visible", "dead"]},
       # The respawn puts the player at the gamemode's spawn far away; the actor is brought back to its spot.
       # The ORIGINAL client sometimes never requests its spawn after this death (GTA restarts the ped at
       # 2488,-1666, samp does not follow; 2 of 9 runs, always lane A): `ensure_spawn` then has the server
       # spawn the player (SpawnPlayer) so the later steps do not cascade, and the step records it
       # (`self_spawn`: noted for an original sender, a DIFF for ours).
       {"name": "combat_respawn_A", "actor": "A", "events": [(2.0, "ensure_spawn", None), (3.5, "tp", None)],
        "window": 6.5, "checks": ["visible", "dead", "pos"]},
       {"name": "combat_death_B", "actor": "B", "events": [(0.0, "rcon_each", "xc hpar {id} 0 0 ;")], "window": 3.0,
        "checks": ["visible", "dead"]},
       {"name": "combat_respawn_B", "actor": "B", "events": [(2.0, "ensure_spawn", None), (3.5, "tp", None)],
        "window": 6.5, "checks": ["visible", "dead", "pos"]}]
)

# M5 vehicle steps: each player gets its own Sultan 2.5 m north (driver door towards it); the actor enters it with F
# (RPC 26 → the enter task on the other side), drives west for 2.5 s (200), stops and leaves it with F (RPC 154 →
# the leave task, no warp). Then the other player is put into A's car as a passenger (RPC 70 → 211 on the other
# side) and leaves it with F, and A is put back into its car and gets a component (RPC 96 relayed by the server).
VEH_CHECKS = ["visible", "vehicle", "pos"]


def vehicle_steps(a):
    return [
        {"name": f"veh_enter_{a}", "actor": a, "events": [(0.0, "key", "f")], "window": 7.0,
         "checks": VEH_CHECKS + ["anims"]},
        {"name": f"veh_drive_{a}", "actor": a, "events": [(0.0, "keydown", "w"), (2.5, "keyup", "w")], "window": 7.0,
         "checks": VEH_CHECKS + ["veh_pos", "veh_heading"]},
        {"name": f"veh_exit_{a}", "actor": a, "events": [(0.0, "key", "f")], "window": 6.0,
         "checks": VEH_CHECKS + ["anims", "heading"]},
    ]


STEPS = STEPS + (
    [{"name": "veh_create", "actor": "both",
      "events": [(0.0, "tp", None), (1.0, "rcon_each", "xc car {id} ;")], "window": 4.0,
      "checks": ["visible", "pos", "vehicle"]}]
    + vehicle_steps("A") + vehicle_steps("B")
    + [{"name": "veh_passenger_B", "actor": "B", "events": [(0.0, "rcon_each", "xc putin {id} {other} 1 ;")],
        "window": 4.0, "checks": VEH_CHECKS},
       {"name": "veh_passenger_exit_B", "actor": "B", "events": [(0.0, "key", "f")], "window": 6.0,
        "checks": VEH_CHECKS + ["anims"]},
       {"name": "veh_component_A", "actor": "A",
        "events": [(0.0, "rcon_each", "xc putin {id} {id} 0 ;"), (2.0, "rcon_each", "xc comp {id} 1010 ;")],
        "window": 5.0, "checks": VEH_CHECKS + ["veh_mods", "veh_pos"]}]
)

# Enabled when the milestone lands; never guessed before (PLAN §3b).
PENDING = [
    ("vehicle_unoccupied_trailer", "M5", "unoccupied (209) and trailer (210) sync"),
    ("combat_hit", "M7", "a 206 that hits the other client (needs an aimed-hit geometry for two real clients)"),
    ("objects", "M6", "same objects/attachments visible on both"),
    ("chat", "M4", "chat lines and bubbles of the other player"),
]


def say(*a):
    print("[xc]", *a, flush=True)


# --- control channel ------------------------------------------------------------------
class Conn:
    """Persistent control-channel connection; requests are pipelined (our
    client answers every complete line once per frame)."""

    def __init__(self, port, timeout=5.0):
        self.port = port
        self.s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        self.s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.f = self.s.makefile("rb")

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass

    def line(self):
        return self.f.readline().decode(errors="replace").rstrip("\r\n")

    def batch(self, lines):
        if not lines:
            return []
        self.s.sendall(("\n".join(lines) + "\n").encode())
        return [self.line() for _ in lines]

    def peeks(self, regs):
        """{(addr, len): bytes|None} for regions of any length (≤256 per peek, ≤64 per batch)."""
        pieces = patchrows.chunks(regs)
        got = {}
        for i in range(0, len(pieces), 64):
            part = pieces[i:i + 64]
            replies = self.batch([f"peek {a:#x} {k}" for a, k, _, _ in part])
            for (a, k, _reg, _o), r in zip(part, replies):
                parts = r.split()
                data = None
                if len(parts) >= 3 and parts[0] == "peek":
                    try:
                        data = bytes.fromhex("".join(parts[2:]))
                    except ValueError:
                        data = None
                    if data is not None and len(data) != k:
                        data = None
                got[(a, k)] = data
        out = {}
        for a, k, reg, o in pieces:
            d = got.get((a, k))
            cur = out.get(reg, b"")
            out[reg] = None if d is None or cur is None else cur + d
        return out

    def cmd_multi(self, cmd):
        """Header + `count=N` following lines (the `remote` reply format)."""
        self.s.sendall((cmd + "\n").encode())
        head = self.line()
        m = re.search(r"\bcount=(\d+)", head)
        n = int(m.group(1)) if m else 0
        return [head] + [self.line() for _ in range(n)]


def u32(b, o=0):
    return struct.unpack_from("<I", b, o)[0]


def f32(b, o=0):
    return struct.unpack_from("<f", b, o)[0]


def read_peds(conn, peds, clump_off):
    """Views of several peds through batched peeks (a few frames on our client)."""
    peds = [p for p in peds if p]
    views = {p: {"ped": f"{p:#x}"} for p in peds}
    if not peds:
        return views
    r1 = conn.peeks([r for p in peds for r in ((p, 0x60), (p + PED_FLAGS, 4), (p + PED_STATE, 0x30),
                                               (p + PED_SELECTED_SLOT, 1), (p + PED_VEHICLE, 4))])
    second = []
    for p in peds:
        h, fl, st = r1.get((p, 0x60)), r1.get((p + PED_FLAGS, 4)), r1.get((p + PED_STATE, 0x30))
        v = views[p]
        if h is None or fl is None or st is None:
            v["error"] = "unreadable"
            continue
        v["model"] = struct.unpack_from("<h", h, E_MODEL)[0]
        v["_matrix"] = u32(h, E_MATRIX)
        v["_clump"] = u32(h, E_RWOBJECT)
        v["speed"] = round(math.sqrt(sum(c * c for c in struct.unpack_from("<3f", h, PHYS_SPEED))), 4)
        v["standing"] = bool(fl[0] & 1)
        v["in_air"] = bool(fl[1] & 2)
        v["ducking"] = bool(fl[3] & 4)
        vb = r1.get((p + PED_VEHICLE, 4))
        v["in_vehicle"] = bool(fl[1] & 1)
        v["_veh"] = u32(vb) if vb and v["in_vehicle"] else 0
        if v["_veh"]:
            second += [(v["_veh"], 0x60), (v["_veh"] + VEH_UPGRADES, 30), (v["_veh"] + VEH_DRIVER, 36)]
        v["ped_state"] = u32(st, 0)
        v["move"] = u32(st, PED_MOVE - PED_STATE)
        v["health"] = round(f32(st, PED_HEALTH - PED_STATE), 2)
        v["armour"] = round(f32(st, PED_ARMOUR - PED_STATE), 2)
        v["heading"] = round(f32(st, PED_HEADING - PED_STATE), 4)
        slot = r1.get((p + PED_SELECTED_SLOT, 1))
        if slot is not None and slot[0] < 13:
            second.append((p + PED_WEAPONS + slot[0] * PED_WEAPON_SIZE, 4))
            v["_weapon_at"] = p + PED_WEAPONS + slot[0] * PED_WEAPON_SIZE
        pos_at = v["_matrix"] + 0x30 if v["_matrix"] else p + E_PLACEMENT
        v["_pos_at"] = pos_at
        second.append((pos_at, 12))
        if v["_clump"] and clump_off:
            second.append((v["_clump"] + clump_off, 4))
    r2 = conn.peeks(second)
    cds = {}
    for p in peds:
        v = views[p]
        if "error" in v:
            continue
        if "_weapon_at" in v:
            wb = r2.get((v["_weapon_at"], 4))
            v["weapon"] = u32(wb) if wb else None
        b = r2.get((v["_pos_at"], 12))
        v["pos"] = [round(c, 3) for c in struct.unpack("<3f", b)] if b else None
        if v["_veh"]:
            vh, up, occ = (r2.get((v["_veh"], 0x60)), r2.get((v["_veh"] + VEH_UPGRADES, 30)),
                           r2.get((v["_veh"] + VEH_DRIVER, 36)))
            if vh and up and occ:
                occupants = struct.unpack("<9I", occ)
                v["vehicle"] = {"model": struct.unpack_from("<h", vh, E_MODEL)[0],
                                "seat": occupants.index(p) if p in occupants else -1,
                                "speed": round(math.sqrt(sum(c * c for c in struct.unpack_from("<3f", vh, PHYS_SPEED))), 4),
                                "mods": sorted(m for m in struct.unpack("<15h", up) if m > 0),
                                "_matrix": u32(vh, E_MATRIX)}
        cdb = r2.get((v["_clump"] + clump_off, 4)) if v["_clump"] and clump_off else None
        if cdb and u32(cdb):
            cds[p] = u32(cdb)
    mats = conn.peeks([(v["vehicle"]["_matrix"], 0x40) for v in views.values()
                       if v.get("vehicle") and v["vehicle"]["_matrix"]])
    for v in views.values():
        vv = v.get("vehicle")
        if not vv:
            continue
        m = mats.get((vv["_matrix"], 0x40)) if vv["_matrix"] else None
        if m:
            fx, fy, _fz = struct.unpack_from("<3f", m, 0x10)
            vv["pos"] = [round(c, 3) for c in struct.unpack_from("<3f", m, 0x30)]
            vv["heading"] = round(math.atan2(-fx, fy), 4)
        del vv["_matrix"]
    # Association list: head at clumpdata+0 (RwLLLink next); node = assoc + 4.
    heads = conn.peeks([(cd, 4) for cd in cds.values()])
    cur = {p: u32(heads[(cd, 4)]) for p, cd in cds.items() if heads.get((cd, 4))}
    assocs = {p: [] for p in cds}
    for _ in range(16):
        live = {p: n for p, n in cur.items() if n and n != cds[p] and len(assocs[p]) < 16}
        if not live:
            break
        got = conn.peeks([(n - 4, ASSOC_SIZE) for n in live.values()])
        cur = {}
        for p, n in live.items():
            b = got.get((n - 4, ASSOC_SIZE))
            if not b:
                continue
            assocs[p].append({"group": struct.unpack_from("<h", b, 0xE)[0], "hier": u32(b, 0x14),
                              "blend": round(f32(b, 0x18), 3), "id": struct.unpack_from("<h", b, 0x2C)[0],
                              "flags": struct.unpack_from("<H", b, 0x2E)[0]})
            cur[p] = u32(b, 4)
    hiers = sorted({a["hier"] for lst in assocs.values() for a in lst if a["hier"]})
    hh = conn.peeks([(h, 4) for h in hiers])
    for p in peds:
        v = views[p]
        lst = []
        for a in assocs.get(p, []):
            hb = hh.get((a["hier"], 4))
            key = f"{u32(hb):08x}" if hb else "?"
            lst.append({"anim": f"{key}/{a['group']}:{a['id']}", "blend": a["blend"], "flags": a["flags"]})
        lst.sort(key=lambda x: -x["blend"])
        v["anims"] = lst
        v["dominant"] = lst[0]["anim"] if lst else None
        for k in [k for k in v if k.startswith("_")]:
            del v[k]
    return views


# --- sides ----------------------------------------------------------------------------
class Side:
    def __init__(self, key, port, display, name, kind, gamescope=""):
        self.key, self.port, self.display, self.name, self.kind = key, port, display, name, kind
        self.gamescope = gamescope
        self.conn = None
        self.clump_off = None
        self.pid = None  # server player id

    def connect(self):
        if self.conn is None:
            self.conn = Conn(self.port)
            b = self.conn.peeks([(CLUMP_OFFSET, 4)]).get((CLUMP_OFFSET, 4))
            self.clump_off = u32(b) if b else None
        return self.conn

    def xdo(self, *args):
        env = {**os.environ, "DISPLAY": self.display}
        env.pop("WAYLAND_DISPLAY", None)
        subprocess.run(["xdotool", *args], env=env, check=False)

    def remote_ped(self, other_name):
        """(ped, raw remote lines) of the other player as this side shows it."""
        lines = self.connect().cmd_multi("remote")
        ped = None
        if self.kind == "ours":
            for ln in lines[1:]:
                if re.search(rf"\bname={re.escape(other_name)}\b", ln):
                    m = re.search(r"\bped=(0x[0-9a-fA-F]+|0)\b", ln)
                    ped = int(m.group(1), 16) if m and m.group(1) != "0" else None
        else:
            peds = [int(m.group(1), 16) for ln in lines[1:] for m in [re.search(r"\bped=(0x[0-9a-fA-F]+)", ln)] if m]
            ped = peds[0] if len(peds) == 1 else None
            if len(peds) > 1:
                lines.append(f"# ambiguous: {len(peds)} remote peds")
        return ped, lines

    def sample(self, other_name):
        c = self.connect()
        t = time.time()
        lp = c.peeks([(PLAYERS0_PED, 4)]).get((PLAYERS0_PED, 4))
        local = u32(lp) if lp else 0
        rped, rlines = self.remote_ped(other_name)
        views = read_peds(c, [local, rped or 0], self.clump_off)
        return {"t": round(t, 3), "local": views.get(local) if local else None,
                "remote": views.get(rped) if rped else None, "remote_raw": rlines}

    def close(self):
        if self.conn:
            self.conn.close()
            self.conn = None


def shot(side, path):
    """Screenshot of one side's isolated gamescope display."""
    env = {**os.environ, "GAMESCOPE_WAYLAND_DISPLAY": side.gamescope}
    subprocess.run(["gamescopectl", "screenshot", path], env=env, check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class Driver:
    def __init__(self, a):
        self.a = a
        self.log = drive.Log(a.log)
        self.sides = {
            "A": Side("A", a.a_port, a.a_display, a.a_name, a.a_kind, a.a_gamescope),
            "B": Side("B", a.b_port, a.b_display, a.b_name, a.b_kind, a.b_gamescope),
        }
        os.makedirs(a.out, exist_ok=True)
        self.f = open(os.path.join(a.out, "samples.jsonl"), "w")
        self.pool = concurrent.futures.ThreadPoolExecutor(2)

    def other(self, k):
        return "B" if k == "A" else "A"

    def write(self, rec):
        self.f.write(json.dumps(rec) + "\n")
        self.f.flush()

    def rcon(self, cmd):
        drive.rcon(self.a.port, self.a.rcon_pass, cmd)

    def xc(self, cmd, wait=r"\[xc\] (ok|error)"):
        self.log.skip_to_end()
        self.rcon(cmd)
        lines = []
        m = self.log.wait(wait, 5, collect=lines)
        return m, lines

    def server_view(self, tag):
        _m, lines = self.xc(f"xc info {tag} ;", wait=rf"\[xc\] ok info {re.escape(tag)}\b")
        out = {}
        for ln in lines:
            m = re.search(r"\[xc\] info \S+ id=(\d+) name=(\S+) (.*)", ln)
            if m:
                out[m.group(2)] = dict(kv.split("=", 1) for kv in m.group(3).split())
        return out

    def sample_both(self):
        futs = {k: self.pool.submit(s.sample, self.sides[self.other(k)].name) for k, s in self.sides.items()}
        out = {}
        for k, f in futs.items():
            try:
                out[k] = f.result(timeout=20)
            except Exception as e:  # noqa: BLE001 — a failed sample is recorded
                out[k] = {"error": f"{type(e).__name__}: {e}"}
                self.sides[k].close()
        return out

    def side_cmd(self, side, line):
        try:
            return side.connect().batch([line])[0]
        except OSError as e:
            side.close()
            return f"error {e}"

    def do_event(self, kind, arg, actor):
        targets = ["A", "B"] if actor == "both" else [actor]
        if kind in ("key", "keydown", "keyup"):
            for k in targets:
                self.sides[k].xdo(kind, *(["--delay", "80"] if kind == "key" else []), arg)
        elif kind in ("mousedown", "mouseup"):
            for k in targets:
                self.sides[k].xdo(kind, arg)
        elif kind == "rcon_each":
            for k in targets:
                s = self.sides[k]
                self.xc(arg.format(id=s.pid, other=self.sides[self.other(k)].pid, skin=SKIN_STEP[k]))
        elif kind == "ensure_spawn":
            # Spawned since the previous step (the death) began? Else the server spawns the player.
            for k in targets:
                if self.spawn_count(k) <= self.step_spawns[-2][k]:
                    self.forced_spawn.append(k)
                    self.xc(f"xc spawn {self.sides[k].pid} ;")
        elif kind == "tp":
            for k in targets:
                x, y, z, ang = SPOT[k]
                self.xc(f"xc tp {self.sides[k].pid} {x:.3f} {y:.3f} {z:.3f} {ang:.1f} ;")

    def spawn_count(self, k):
        """Server-side spawns of side k's player so far (`[e2e] spawn id=N` lines of the server log)."""
        try:
            text = open(self.log.path, errors="replace").read()
        except OSError:
            return 0
        return len(re.findall(rf"\[e2e\] spawn id={self.sides[k].pid}\b", text))

    def run_step(self, st):
        rec = {"name": st["name"], "actor": st["actor"], "checks": st["checks"], "window": st["window"]}
        if not hasattr(self, "step_spawns"):
            self.step_spawns = []
        self.step_spawns.append({k: self.spawn_count(k) for k in self.sides})
        self.forced_spawn = []
        rec["pre"] = [self.sample_both() for _ in range(2)]
        t0 = time.time()
        events = sorted(st["events"], key=lambda e: e[0])
        samples = []
        while True:
            el = time.time() - t0
            while events and events[0][0] <= el:
                _, kind, arg = events.pop(0)
                self.do_event(kind, arg, st["actor"])
            if el >= st["window"] and not events:
                break
            ts = time.time()
            s = self.sample_both()
            s["dt"] = round(time.time() - t0, 3)
            samples.append(s)
            time.sleep(max(0.0, SAMPLE_PERIOD - (time.time() - ts)))  # at most 10 Hz on either client
        rec["samples"] = samples
        if self.forced_spawn:
            rec["forced_spawn"] = self.forced_spawn
            say(f"step {st['name']}: no spawn request from {self.forced_spawn} after the death; server spawned")
        # The tracer serves one connection at a time: reuse the persistent one.
        rec["state"] = {k: self.side_cmd(s, "state") for k, s in self.sides.items()}
        rec["server"] = self.server_view(st["name"])
        self.write(rec)
        say(f"step {st['name']:<14} samples={len(samples)}")

    def wait_join(self):
        names = {s.name: k for k, s in self.sides.items()}
        seen = {}
        end = time.time() + 240
        while len(seen) < 2 and time.time() < end:
            m = self.log.wait(r"\[e2e\] connect id=(\d+) name=(\S+)", max(1, end - time.time()))
            if m and m.group(2) in names:
                seen[names[m.group(2)]] = int(m.group(1))
        for k, pid in seen.items():
            self.sides[k].pid = pid
        return len(seen) == 2

    def spawn_all(self):
        """Shift on every side whose player has a class request but no spawn
        yet, every 4 s, for up to 120 s (the original sometimes ignores the
        first presses while its class selection is still settling)."""
        done = {k: False for k in self.sides}
        end = time.time() + 120
        while not all(done.values()) and time.time() < end:
            for k, s in self.sides.items():
                if done[k]:
                    continue
                self.log.cursor = 0
                if self.log.wait(rf"\[e2e\] spawn id={s.pid}\b", 0):
                    done[k] = True
                    continue
                self.log.cursor = 0
                if self.log.wait(rf"\[e2e\] class id={s.pid} class=", 0):
                    s.xdo("key", "--delay", "80", "shift")
            time.sleep(4)
            for k, s in self.sides.items():
                self.log.cursor = 0
                done[k] = done[k] or bool(self.log.wait(rf"\[e2e\] spawn id={s.pid}\b", 0))
        for k, s in self.sides.items():
            if not done[k] and s.gamescope:
                shot(s, os.path.join(self.a.out, f"spawn-failed-{k}.png"))
        return done

    def dump_patches(self, snapshots=5, gap=0.3):
        """Bytes at every CSV row region, `snapshots` times per side: samp
        toggles some rows (nop/restore around remote-ped processing), so one
        snapshot can land in either state. `regions` = first snapshot,
        `variants` = every distinct value seen."""
        regs = patchrows.regions()
        for k, s in self.sides.items():
            seen = {}
            first = None
            try:
                for i in range(snapshots):
                    got = s.connect().peeks(regs)
                    if first is None:
                        first = got
                    for reg, b in got.items():
                        v = seen.setdefault(f"{reg[0]:#x}:{reg[1]}", [])
                        h = b.hex() if b is not None else None
                        if h not in v:
                            v.append(h)
                    time.sleep(gap)
            except OSError as e:
                say(f"patch dump {k} failed: {e}")
                continue
            out = {f"{a:#x}:{n}": (b.hex() if b is not None else None) for (a, n), b in first.items()}
            with open(os.path.join(self.a.out, f"patch-bytes-{k}.json"), "w") as f:
                json.dump({"side": k, "kind": s.kind, "t": time.time(), "snapshots": snapshots,
                           "regions": out, "variants": seen}, f, indent=0)
            tog = sum(len(v) > 1 for v in seen.values())
            say(f"patch bytes {k} ({s.kind}): {sum(v is not None for v in out.values())}/{len(out)} regions read, "
                f"{tog} changed between {snapshots} snapshots")

    def watch_frame_local(self, frames=60, ms=2000):
        """Write watch on the frame-local rows (patchrows.FRAME_LOCAL_ROWS): one samp-tracer
        `efx watch` window per side (guard pages on the rows' data pages; every write with
        its value, writer and game frame goes to the side's efx.jsonl, copied to the run dir
        as {A,B}-efx.jsonl). Side A: the tracer of the original; side B: a tracer loaded next
        to our client in watch-only mode (SAMP_TRACER_MODE=watch, no game byte changed)."""
        regs = patchrows.frame_local_regions()
        spec = os.path.join(self.a.out, "frame-local.spec")
        with open(spec, "w") as f:
            for addr, n in regs:
                f.write(f"mem gta:{addr:#x} {n}\n")
        winpath = "Z:" + os.path.abspath(spec).replace("/", "\\")

        def one(k, s):
            port = getattr(self.a, f"{k.lower()}_tracer_port") or (s.port if s.kind == "orig" else 0)
            res = {"side": k, "kind": s.kind, "port": port, "frames": frames, "ms": ms,
                   "regions": [f"{a:#x}:{n}" for a, n in regs]}
            if not port:
                res["error"] = "no tracer on this side"
                return res
            if port == s.port:
                s.close()  # the tracer serves one connection at a time
            res["reply"] = drive.ctl(port, f"efx watch {winpath} frames={frames} ms={ms} faults=5000000", timeout=15)
            m = re.match(r"ok window (\d+)", res["reply"])
            if not m:
                res["error"] = "watch not started"
                return res
            end = time.time() + 300
            while time.time() < end:
                st = drive.ctl(port, "efx status", timeout=5)
                res["status"] = st
                d = re.match(r"idle done=(\d+)", st)
                if d and int(d.group(1)) >= int(m.group(1)):
                    res["window"] = int(m.group(1))
                    return res
                time.sleep(0.5)
            res["error"] = "window did not close"
            return res

        futs = {k: self.pool.submit(one, k, s) for k, s in self.sides.items()}
        for k, fu in futs.items():
            res = fu.result()
            with open(os.path.join(self.a.out, f"frame-watch-{k}.json"), "w") as f:
                json.dump(res, f, indent=1)
            say(f"frame-local watch {k} ({res['kind']}): {res.get('reply', '')} -> "
                f"{res.get('error') or res.get('status')}")

    def main(self):
        a = self.a
        for k, s in self.sides.items():
            for _ in range(240):
                if drive.ctl(s.port, "ping") == "pong":
                    break
                time.sleep(0.5)
            else:
                say(f"side {k} ({s.kind}) control channel did not answer")
                return 1
        say("both control channels up")
        if not self.wait_join():
            say("both players did not join")
            return 1
        say("joined: " + ", ".join(f"{k}={s.name}#{s.pid}" for k, s in self.sides.items()))
        time.sleep(3)
        ok = self.spawn_all()
        say(f"spawn: {ok}")
        meta = {"sides": {k: {"name": s.name, "kind": s.kind, "pid": s.pid, "port": s.port} for k, s in self.sides.items()},
                "spawned": ok, "tol": TOL, "pending": PENDING}
        with open(os.path.join(a.out, "meta.json"), "w") as f:
            json.dump(meta, f, indent=1)
        if not all(ok.values()):
            return 1
        time.sleep(3)
        self.do_event("tp", None, "both")
        time.sleep(5)
        self.dump_patches()
        self.watch_frame_local()
        if a.patches_only:
            return 0
        for st in STEPS:
            self.run_step(st)
        return 0


# --- report ---------------------------------------------------------------------------
def angdiff(a, b):
    d = (a - b + math.pi) % (2 * math.pi) - math.pi
    return abs(d)


def dist(p, q):
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(p, q)))


def series(rec, sender, observer):
    """[(sender local view, observer's view of sender)] over the step window."""
    out = []
    for s in rec["samples"]:
        sl = (s.get(sender) or {}).get("local")
        orv = (s.get(observer) or {}).get("remote")
        out.append((sl, orv))
    return out


def hkey(anim):
    """Animations are compared by hierarchy hash (what is drawn); the
    association group/id is shown but not compared."""
    return anim.split("/")[0] if anim else None


def anim_set(views, minblend):
    return {hkey(a["anim"]) for v in views if v for a in v.get("anims", []) if a["blend"] >= minblend}


def full_names(views, keys):
    names = {hkey(a["anim"]): a["anim"] for v in views if v for a in v.get("anims", [])}
    return sorted(names.get(k, k) for k in keys)


# Scalar checks: (sender value, observer value) at the end of the window or over it.
def scalar(chk, ser, last_s, last_o):
    if chk == "skin":
        return last_s["model"], last_o["model"]
    if chk in ("health", "armour"):
        return last_s[chk], last_o[chk]
    if chk == "weapon":
        return last_s.get("weapon"), last_o.get("weapon")
    if chk == "dead":
        # Dying / dead (ped state 54/55) at any time of the window, and at its end.
        def dead(v):
            return bool(v) and v.get("ped_state") in (54, 55)
        return ((int(any(dead(v) for v, _ in ser)), int(dead(last_s))),
                (int(any(dead(v) for _, v in ser)), int(dead(last_o))))
    if chk == "ducking":
        return int(last_s["ducking"]), int(last_o["ducking"])
    if chk == "in_air":
        return (int(any(v and v.get("in_air") for v, _ in ser)), int(any(v and v.get("in_air") for _, v in ser)))
    if chk == "move":
        return (max((v.get("move", 0) for v, _ in ser if v), default=0),
                max((v.get("move", 0) for _, v in ser if v), default=0))
    if chk == "anim_final":
        return hkey(last_s.get("dominant")), hkey(last_o.get("dominant"))
    if chk == "vehicle":
        # In a vehicle, which model, which seat (0 driver, k passenger k) at the end of the window.
        def vk(v):
            vv = v.get("vehicle")
            return (int(bool(v.get("in_vehicle"))), vv["model"], vv["seat"]) if vv else (int(bool(v.get("in_vehicle"))),)
        return vk(last_s), vk(last_o)
    if chk == "veh_mods":
        return ((last_s.get("vehicle") or {}).get("mods"), (last_o.get("vehicle") or {}).get("mods"))
    return None


def same(chk, a, b):
    if a is None or b is None:
        return False
    if chk in ("health", "armour"):
        return abs(a - b) <= TOL[chk]
    return a == b


def evaluate(rec, sender, observer, tol, calibs=None, min_hold=2):
    """calibs: the same step/direction evaluated on original-vs-original runs (a list; one per
    calibration run). Where the original itself does not keep a relation (e.g. a remote ped's GTA
    health is not the sender's), ours must show what the original observer showed.
    min_hold: samples (blend >= 0.5) a sender animation must last to be required on the observer."""
    if isinstance(calibs, dict):
        calibs = [calibs]
    calibs = calibs or []
    ser = series(rec, sender, observer)
    pre = [((s.get(sender) or {}).get("local")) for s in rec["pre"]]
    last_s, last_o = (ser[-1] if ser else (None, None))
    res = {}
    for chk in rec["checks"]:
        ok, val, extra = None, None, {}
        if chk == "visible":
            ok = last_o is not None and "error" not in last_o
            val = "remote ped present" if ok else "observer shows no ped for the sender"
        elif last_s is None or last_o is None or "error" in last_s or "error" in last_o:
            ok, val = False, "no data (sender local or observer remote ped missing)"
        elif chk == "pos":
            if last_s.get("pos") and last_o.get("pos"):
                d = dist(last_s["pos"], last_o["pos"])
                ok, val, extra = d <= tol["pos"], f"Δ={d:.3f} m (tol {tol['pos']:.2f})", {"delta": d}
            else:
                ok, val = False, "position unreadable"
        elif chk in ("veh_pos", "veh_heading"):
            vs, vo = last_s.get("vehicle") or {}, last_o.get("vehicle") or {}
            key = chk[4:]
            if vs.get(key) is None or vo.get(key) is None:
                ok, val = False, "vehicle not readable on one side"
            else:
                d = dist(vs["pos"], vo["pos"]) if key == "pos" else angdiff(vs["heading"], vo["heading"])
                t_ = tol.get(chk, TOL[chk])
                ok, val, extra = d <= t_, f"Δ={d:.3f} (tol {t_:.2f})", {"delta": d}
        elif chk == "heading":
            d = angdiff(last_s["heading"], last_o["heading"])
            ok, extra = d <= tol["heading"], {"delta": d}
            val = f"Δ={d:.3f} rad (tol {tol['heading']:.2f}) {last_s['heading']:.3f}/{last_o['heading']:.3f}"
        elif chk == "anims":
            # Animations the sender played in this step (blend >= 0.5 in at least `min_hold`
            # samples, not already playing before the action) must show up on the observer's ped at
            # some point of the window (blend > 0.1).
            counts = {}
            for v, _ in ser:
                for a_ in (v or {}).get("anims", []):
                    if a_["blend"] >= 0.5:
                        counts[hkey(a_["anim"])] = counts.get(hkey(a_["anim"]), 0) + 1
            before = anim_set(pre, 0.5)
            distinct = {k for k, n in counts.items() if n >= min_hold} - before
            short_ = {k for k, n in counts.items() if n < min_hold} - before
            seen = anim_set([v for _, v in ser], 0.1)
            missing = distinct - seen
            ok = not missing
            sv = [v for v, _ in ser]
            extra = {"distinct": sorted(distinct), "sender_seen": sorted(anim_set(sv, 0.1)),
                     "missing": sorted(missing), "hold": {k: counts[k] for k in distinct | short_}}
            val = f"sender {full_names(sv, distinct) or '-'}; missing on observer {full_names(sv, missing) or '-'}"
            if short_ - seen:
                val += f"; shorter than {min_hold} samples, not required: {full_names(sv, short_ - seen)}"
            cm = [set(c["checks"][chk].get("missing", [])) for c in calibs if chk in c["checks"]]
            if cm and any(cm):
                # The original observer does not show every animation of the original sender in this
                # step/direction (orig-vs-orig runs). Ours must reproduce that: miss what every
                # calibration run missed, and miss nothing no calibration run missed.
                must, may = set.intersection(*cm), set.union(*cm)
                ok = must <= missing <= may
                val += (f" (original observer missed {full_names(sv, must) or sorted(must)}"
                        + (f", sometimes {sorted(may - must)}" if may - must else "")
                        + f" in {len(cm)} original-vs-original run(s))")
        else:
            a_, b_ = scalar(chk, ser, last_s, last_o)
            ok = same(chk, a_, b_)
            val = f"sender {a_} / observer {b_}"
            extra = {"observer": b_, "sender": a_}
            cs = [c["checks"][chk] for c in calibs if chk in c["checks"]]
            notkept = [c for c in cs if not c["ok"] and "observer" in c]
            if notkept:
                # The original does not keep sender == observer here: compare the observer with what
                # the original's observer showed (any of the calibration runs; sender == observer also
                # passes when some calibration run kept the relation).
                obs = [c["observer"] for c in notkept]
                ok = any(same(chk, b_, o) for o in obs) or (ok and len(notkept) < len(cs))
                val += f" (original observer showed {obs[0] if len(set(map(str, obs))) == 1 else obs}: relation not kept by the original)"
        res[chk] = {"ok": ok, "val": val, **extra}
    if calibs:
        # The acting player's own ped must do what the original's did in the
        # same step (else "sender == observer" can hold trivially, e.g. when
        # neither side applied an RPC).
        bad = []
        for chk, r in res.items():
            cs = [c["checks"][chk] for c in calibs if chk in c["checks"]]
            if not cs:
                continue
            if chk == "anims" and "sender_seen" in r and all("distinct" in c for c in cs):
                # Played by the original sender in every calibration run.
                miss = sorted(set.intersection(*(set(c["distinct"]) for c in cs)) - set(r["sender_seen"]))
                if miss:
                    bad.append(f"anims {miss} not played")
            elif "sender" in r and all("sender" in c for c in cs) \
                    and not any(same(chk, r["sender"], c["sender"]) for c in cs):
                bad.append(f"{chk} {r['sender']} (original {cs[0]['sender']})")
        res["sender_vs_original"] = {"ok": not bad, "val": "; ".join(bad) or "same as the original sender"}
    judged = [r["ok"] for r in res.values() if r["ok"] is not None]
    verdict = "MATCH" if all(judged) else "DIFF"
    return {"verdict": verdict, "checks": res}


def learn_tol(calib_recs):
    """Tolerances: 1.5 x the largest deviation the original-vs-original run
    showed (floors TOL_FLOOR); the TOL defaults without a calibration run."""
    tol = dict(TOL)
    if not calib_recs:
        return tol
    mx = {"pos": 0.0, "heading": 0.0}
    for rec in calib_recs:
        for s, o in (("A", "B"), ("B", "A")):
            ev = evaluate(rec, s, o, {"pos": 1e9, "heading": 1e9})
            for k in mx:
                c = ev["checks"].get(k)
                if c and "delta" in c:
                    mx[k] = max(mx[k], c["delta"])
    for k in mx:
        tol[k] = max(TOL_FLOOR[k], round(1.5 * mx[k], 3))
    return tol


def directions(rec):
    if rec["actor"] == "both":
        return [("A", "B"), ("B", "A")]
    return [(rec["actor"], "B" if rec["actor"] == "A" else "A")]


CALIB_MAX = 3   # most recent original-vs-original runs the report uses


def learn_min_hold(evs):
    """Sub-sample tolerance from the calibration runs: an animation the original observer showed in
    one original-vs-original run and missed in another of the same step/direction was missed by
    sampling (both observers are the original). Sender animations held for no more samples than the
    longest such one are not required on the observer. At least 2 (one sample is never required)."""
    longest = 0
    for key, lst in evs.items():
        miss = [set(e["checks"].get("anims", {}).get("missing", [])) for e in lst]
        if len(miss) < 2 or not any(miss):
            continue
        transient = set.union(*miss) - set.intersection(*miss)
        for e in lst:
            hold = e["checks"].get("anims", {}).get("hold", {})
            longest = max([longest] + [hold.get(k, 0) for k in transient])
    return max(2, longest + 1)


def report(run, out, calib_runs=None):
    recs = [json.loads(ln) for ln in open(os.path.join(run, "samples.jsonl"))]
    meta = json.load(open(os.path.join(run, "meta.json")))
    sides = meta["sides"]
    ka, kb = sides["A"]["kind"], sides["B"]["kind"]
    label = {"A": ka, "B": kb if kb != ka else kb + "2"}
    if isinstance(calib_runs, str):
        calib_runs = [calib_runs]
    want = [st["name"] for st in STEPS]
    used, skipped = [], []
    for c in calib_runs or []:
        if os.path.abspath(c) == os.path.abspath(run) or len(used) >= CALIB_MAX:
            continue
        try:
            crecs = [json.loads(ln) for ln in open(os.path.join(c, "samples.jsonl"))]
        except OSError:
            continue
        # Only calibration runs of the current scenario (same steps) are used.
        if [r["name"] for r in crecs] != want:
            skipped.append(c)
            continue
        # A calibration run in which the scenario itself went wrong (the two originals disagree on a
        # position by more than the default tolerance, e.g. a respawn the observer never showed, after
        # which every later step is off) measures that failure, not the noise floor: not used.
        far = [(r["name"], k, round(e["checks"][k]["delta"], 1)) for r in crecs for so in directions(r)
               for e in [evaluate(r, so[0], so[1], TOL)] for k in ("pos", "veh_pos")
               if "delta" in e["checks"].get(k, {}) and e["checks"][k]["delta"] > TOL[k]]
        if far:
            skipped.append(c)
            print(f"[xc] calibration {os.path.relpath(c, patchrows.ROOT)} not used: positions apart beyond the "
                  f"default tolerance from {far[0][0]} on ({len(far)} checks)", file=sys.stderr)
            continue
        used.append((c, crecs))
    calib_recs = [r for _, crecs in used for r in crecs]
    tol = learn_tol(calib_recs)
    first = {}
    for _, crecs in used:
        for rec in crecs:
            for s, o in directions(rec):
                first.setdefault((rec["name"], s, o), []).append(evaluate(rec, s, o, tol, min_hold=1))
    min_hold = learn_min_hold(first)
    calib_ev = {}
    for _, crecs in used:
        for rec in crecs:
            for s, o in directions(rec):
                calib_ev.setdefault((rec["name"], s, o), []).append(evaluate(rec, s, o, tol, min_hold=min_hold))
    calib_run = ", ".join(os.path.relpath(c, patchrows.ROOT) for c, _ in used) or None
    rows = []
    for rec in recs:
        for s, o in directions(rec):
            ev = evaluate(rec, s, o, tol, calib_ev.get((rec["name"], s, o)), min_hold=min_hold)
            if s in rec.get("forced_spawn", []):
                # The sender did not request its spawn after the death; the server spawned it (ensure_spawn).
                if sides[s]["kind"] == "orig":
                    ev["checks"]["self_spawn"] = {"ok": None, "val": "the original did not request its spawn after "
                                                  "the death (its own flake, see diff-notes); server SpawnPlayer"}
                else:
                    ev["checks"]["self_spawn"] = {"ok": False, "val": "ours did not request its spawn after the death"}
                    ev["verdict"] = "DIFF"
            srv = rec.get("server", {}).get(sides[s]["name"], {})
            rows.append({"step": rec["name"], "direction": f"{label[s]}→{label[o]}",
                         "desc": f"{label[o]} sees {label[s]}", **ev,
                         "server": {k: srv.get(k) for k in ("anim", "sa", "skin", "hp", "ar")},
                         "samples": len(rec["samples"])})
    for name, ms, what in meta.get("pending", []):
        for d in (f"{label['A']}→{label['B']}", f"{label['B']}→{label['A']}"):
            rows.append({"step": name, "direction": d, "verdict": "PENDING", "checks": {}, "milestone": ms, "desc": what})
    enabled = [r for r in rows if r["verdict"] != "PENDING"]
    nmatch = sum(r["verdict"] == "MATCH" for r in enabled)
    calib_rel = calib_run
    summary = {"run": os.path.relpath(run, patchrows.ROOT), "calibration": calib_rel, "sides": sides,
               "enabled": len(enabled), "match": nmatch, "diff": len(enabled) - nmatch,
               "pending": len(rows) - len(enabled), "tolerances": tol, "anim_min_hold": min_hold,
               "calibration_skipped": [os.path.relpath(c, patchrows.ROOT) for c in skipped], "rows": rows,
               "pass": bool(enabled) and nmatch == len(enabled) and len(recs) == len(STEPS)}
    os.makedirs(out, exist_ok=True)
    name = "cross-client" if not (ka == kb == "orig") else "cross-client-calibration"
    with open(os.path.join(out, f"{name}.json"), "w") as f:
        json.dump(summary, f, indent=1)
    md = ["# DIFF-3 cross-client report (criterion F3)", "",
          f"Run `{summary['run']}`: A = {label['A']} ({sides['A']['name']}), B = {label['B']} ({sides['B']['name']}), "
          f"one server. `X→Y` = what Y shows for player X, compared with X's own local ped.", "",
          f"**{nmatch}/{len(enabled)} enabled step-directions MATCH**, {summary['diff']} DIFF, "
          f"{summary['pending']} pending (milestones not landed). Steps recorded: {len(recs)}/{len(STEPS)}.", "",
          f"Tolerances: position {tol['pos']} m, heading {tol['heading']} rad, health/armour {tol['health']}"
          + (f" (learned from the original-vs-original run(s) `{calib_rel}`)." if calib_rel else " (defaults; no calibration run)."),
          "", f"Animations: a sender animation is required on the observer when held (blend >= 0.5) for at least "
          f"{min_hold} samples (sub-sample tolerance learned from the original-vs-original runs: one more than the "
          "longest animation the original observer showed in one calibration run and missed in another).",
          "", "Animations are compared by hierarchy hash (`hash/group:id` shown). Checks the original itself does "
          "not keep (calibration) compare our observer with the original's observer.", "",
          "| step | direction | verdict | failed checks | details |", "|---|---|---|---|---|"]
    for r in rows:
        if r["verdict"] == "PENDING":
            md.append(f"| {r['step']} | {r['direction']} | PENDING ({r['milestone']}) | | {r['desc']} |")
            continue
        failed = [k for k, c in r["checks"].items() if c["ok"] is False]
        det = "; ".join(f"{k}: {c['val']}" for k, c in r["checks"].items()
                        if c["ok"] is not True or k in ("pos", "heading") or "original observer" in str(c["val"]))
        srv = r["server"]
        det += f"; server sees sender anim={srv.get('anim')} sa={srv.get('sa')}"
        md.append(f"| {r['step']} | {r['direction']} | {r['verdict']} | {', '.join(failed) or '-'} | {det} |")
    md.append("")
    with open(os.path.join(out, f"{name}.md"), "w") as f:
        f.write("\n".join(md))
    print("\n".join(md[:9]))
    return 0 if summary["pass"] else 1


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("drive")
    d.add_argument("--out", required=True)
    d.add_argument("--log", required=True)
    d.add_argument("--port", type=int, required=True)
    d.add_argument("--rcon-pass", required=True)
    for s in ("a", "b"):
        d.add_argument(f"--{s}-port", type=int, required=True)
        d.add_argument(f"--{s}-display", required=True)
        d.add_argument(f"--{s}-name", required=True)
        d.add_argument(f"--{s}-kind", choices=["orig", "ours"], required=True)
        d.add_argument(f"--{s}-gamescope", default="", help="gamescope wayland socket (screenshots)")
    d.add_argument("--patches-only", action="store_true")
    for s in ("a", "b"):
        d.add_argument(f"--{s}-tracer-port", type=int, default=0,
                       help="samp-tracer control port of the side (default: --X-port for an original client)")
    r = sub.add_parser("report")
    r.add_argument("--run", required=True)
    r.add_argument("--out", default=os.path.join(patchrows.ROOT, "target", "diff"))
    r.add_argument("--calib", action="append",
                   help="original-vs-original run (cross-client.sh --calibrate); repeat for several, newest first")
    a = ap.parse_args()
    if a.cmd == "report":
        return report(a.run, a.out, a.calib)
    drv = Driver(a)
    try:
        return drv.main()
    finally:
        drv.f.close()
        for s in drv.sides.values():
            s.close()
        drv.pool.shutdown(wait=False)


if __name__ == "__main__":
    sys.exit(main())
