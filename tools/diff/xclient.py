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
VEH_TRACTOR, VEH_TRAILER = 0x4C4, 0x4C8            # CVehicle::m_pTractor / m_pTrailer (CVehicle.h VALIDATE_OFFSET)
# CPools::ms_pObjectPool (CPools.cpp): CPool { objects, byteMap, size, … } (CPool.h, byte map bit 7 = empty);
# pool elements are CCutsceneObject, 0x19C bytes (CCutsceneObject.h VALIDATE_SIZE).
OBJECT_POOL, OBJECT_SIZE = 0xB7449C, 0x19C
MOVE_NAMES = {0: "none", 1: "still", 2: "turn_l", 3: "turn_r", 4: "walk", 5: "jog", 6: "run", 7: "sprint"}

# --- scenario -------------------------------------------------------------------------
# LSIA runway: open and flat. A stands 2.5 m north of the line, B 2.5 m south;
# both face west (SA:MP angle 90).
BASE = (1700.0, -2490.0, 13.55)
SPOT = {"A": (BASE[0], BASE[1] + 2.5, BASE[2], 90.0), "B": (BASE[0], BASE[1] - 2.5, BASE[2], 90.0)}
SKIN_STEP = {"A": 101, "B": 115}
# Combat hit: the shooter faces the victim 3 m away (A south, B north). The free-aim shot does not leave along the
# facing angle: it went 2.6 deg to the left of a ped turned left by 5.27 deg (original, xcalib dev run 2026-11-06 14:12),
# i.e. 2.67 deg right of the facing angle, and rises (~3 deg; it passed over the head at 5 m). The shooter therefore
# starts turned left by 2.67 deg, and the players stand 3 m apart. Each round has its own spot, more than 2 m east of
# where the players stood before: samp moves a remote ped by velocity nudges only within 2 m (x/y) of its synced position
# and teleports beyond (remote-players-sync 2.1), and on the original a standing ped nudged by 1 m stays where it was
# (calibration: 1.0 m off for the whole step). Round A at +5 m east, round B at +10 m.
HIT_AIM_TURN = 2.67
HIT_GAP = 1.5
HIT_EAST = {"A": 5.0, "B": 10.0}
SPOT_HIT = {r: {"A": (BASE[0] + HIT_EAST[r], BASE[1] + HIT_GAP, BASE[2], 180.0 + HIT_AIM_TURN),
                "B": (BASE[0] + HIT_EAST[r], BASE[1] - HIT_GAP, BASE[2], 0.0 + HIT_AIM_TURN)} for r in ("A", "B")}
# Unoccupied-car steps: where the bystander waits (60 m north of the lanes the cars roll along, within streaming range).
FAR_SPOT = (1560.0, BASE[1] + 60.0, BASE[2])
# Trailer steps: each player's Linerunner (+ trailer 10 m east) on its own lane, clear of the cars.
TRUCK_AT = {"A": (BASE[0], BASE[1] + 25.0, 14.0), "B": (BASE[0], BASE[1] - 25.0, 14.0)}
# Objects: one global object (gunbox), one attached to each player's car (cardboard box / sandbag), one attached
# to each player's head (SA:MP models from SAMP.img: motorcycle helmet / police cap).
OBJ_GLOBAL = (1271, BASE[0] - 8.0, BASE[1], BASE[2] + 0.5, 30.0)
OBJ_VEHICLE = {"A": 1220, "B": 2060}
OBJ_PLAYER = {"A": 18645, "B": 18636}
CHAT_TEXT = {"A": "xc chat from A", "B": "xc chat from B"}
# SetPlayerChatBubble(id, "bubble of <side>", 0xFFAA00FF, 50.0, 15000) (xclient.pwn `xc bubble`).
BUBBLE = {"colour": 0xFFAA00FF, "distance": 50.0, "duration": 15000}

# Check tolerances. Position/heading: the remote ped is placed from the 207
# packets. With a --calibrate run (original vs original) the report uses
# 1.5 x the original's own largest deviation (at least TOL_FLOOR); TOL are
# the defaults without one. See docs/phase3/diff-notes.md "DIFF-3".
TOL = {"pos": 0.5, "heading": 0.15, "health": 1.0, "armour": 1.0, "veh_pos": 1.0, "veh_heading": 0.15}
TOL_FLOOR = {"pos": 0.1, "heading": 0.03}
# Checks added with the M4–M7 steps: every tolerance is learned from the calibration runs (1.5 x the
# original's own largest deviation, floors as above: positions 0.1 m, angles 0.03 rad). The TOL value is only
# the placeholder of a report without calibration.
LEARNED = {"uveh_pos": "pos", "uveh_heading": "heading", "uveh_server": "pos",
           "trailer_pos": "pos", "trailer_heading": "heading", "trailer_server": "pos",
           "obj_global": "pos", "obj_global_rot": "heading", "obj_vehicle": "pos", "obj_vehicle_rot": "heading",
           "obj_player": "pos", "obj_player_rot": "heading"}
for _k, _kind in LEARNED.items():
    TOL[_k] = TOL["veh_pos"] if _kind == "pos" else TOL["veh_heading"]

SAMPLE_PERIOD = 0.1
# Run paths in reports are relative to the checkout that holds target/diff (SAMP_RS_ROOT for a worktree).
DATA_ROOT = os.environ.get("SAMP_RS_ROOT") or patchrows.ROOT

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

# M5 unoccupied (209) and trailer (210) steps. veh_unoccupied_X: the other player is first put 60 m north of the lane
# (still streamed), then the actor (driver of its own car) drives west for 1.5 s and leaves the car with F while it
# still rolls: the actor, the closest player throughout, reports the empty car with 209 and the other side applies it.
# With the bystander next to the car's path the 209 sender changes mid-roll and open.mp accepts the new sender only
# if it is closer than every other player to the server's last (stale) car position (Vehicle::updateFromUnoccupied):
# 4.2 vs 4.5 m decided whether the server's car stopped 21 m short (xclient-20261106-153610) — a race of where the
# peds stand, not of either client (the hand-over itself worked both ways in other runs: `[xc] unocc` lines). veh_trucks puts each player into its own new Linerunner and hitches a trailer
# (AttachTrailerToVehicle) — both at once: open.mp streams new vehicles to a player only on that player's sync packets
# (Vehicles component onPlayerUpdate), and an original client sitting in a car that has come to rest sends none, so a
# truck created later for the other player never reached it (server IsVehicleStreamedIn, dev runs 2026-11-06 14:40);
# veh_trailer_X drives it west for 3 s and waits until the rig is at rest (12 s window; with 8 s the truck still rolled
# and the compared positions depended on when the last 210 left, xclient-20261106-145939): the trailer follows the driver's
# 210 on the other side. "track": the
# actor's vehicle (sender: its local ped's car, observer: the car of its remote ped, both from the pre samples) is
# read on both sides in every sample (with the trailer hitched to it).
STEPS = STEPS + (
    [{"name": "veh_unoccupied_A", "actor": "A", "track": True,
      "events": [(0.0, "tp_far", None), (1.0, "keydown", "w"), (2.5, "keyup", "w"), (2.5, "key", "f")],
      "window": 9.0,
      "checks": ["visible", "vehicle", "pos", "uveh_pos", "uveh_heading", "uveh_server"]},
     {"name": "veh_reenter_B", "actor": "B", "events": [(0.0, "rcon_each", "xc putin {id} {id} 0 ;")], "window": 4.0,
      "checks": VEH_CHECKS},
     {"name": "veh_unoccupied_B", "actor": "B", "track": True,
      "events": [(0.0, "tp_far", None), (1.0, "keydown", "w"), (2.5, "keyup", "w"), (2.5, "key", "f")],
      "window": 9.0,
      "checks": ["visible", "vehicle", "pos", "uveh_pos", "uveh_heading", "uveh_server"]}]
    + [{"name": "veh_trucks", "actor": "both",
        "events": [(0.0, "truck", None), (1.5, "rcon_each", "xc tputin {id} ;"), (3.0, "rcon_each", "xc hitch {id} ;")],
        "window": 6.0, "checks": VEH_CHECKS + ["trailer"]}]
    + [{"name": f"veh_trailer_{a}", "actor": a, "track": True,
        "events": [(0.0, "keydown", "w"), (3.0, "keyup", "w")], "window": 12.0,
        "checks": VEH_CHECKS + ["trailer", "trailer_pos", "trailer_heading", "trailer_server"]} for a in ("A", "B")]
)

# M6 objects: both players leave their trucks and go back to their spots; then the server creates one global object,
# one object attached to each player's car and one attached to each player's head. Each client's GTA object pool is
# read after the window ("post"): for X→Y, the objects of X (its car's, its head's) and the global one as Y shows
# them are compared with X's own client.
STEPS = STEPS + (
    [{"name": "obj_setup", "actor": "both", "events": [(0.0, "rcon_each", "xc out {id} ;"), (3.5, "tp", None)],
      "window": 6.0, "checks": ["visible", "vehicle", "pos", "heading"]},
     {"name": "objects", "actor": "both",
      "events": [(0.0, "gobj", None), (0.0, "rcon_each", "xc vobj {id} {vmodel} ;"),
                 (0.0, "rcon_each", "xc pobj {id} {pmodel} ;")],
      "window": 4.0, "post": ["objects"], "checks": ["visible", "pos", "obj_global", "obj_vehicle", "obj_player"]}]
)

# M4 chat: the actor types a chat line (T, text, Enter); both chatlog.txt files (one per lane's private Documents)
# must get the same line. Then the server gives both players a chat bubble (SetPlayerChatBubble): the other client
# must hold it with the server's text, distance and duration (and the original's colour).
STEPS = STEPS + (
    [{"name": f"chat_{a}", "actor": a, "events": [(0.0, "type", None)], "window": 4.0, "post": ["chatlog"],
      "checks": ["visible", "chat"]} for a in ("A", "B")]
    + [{"name": "bubble", "actor": "both", "events": [(0.0, "rcon_each", "xc bubble {id} {side} ;")], "window": 3.0,
        "post": ["bubbles"], "checks": ["visible", "bubble", "bubble_colour"]}]
)

# M7 hits: the shooter (Desert Eagle) faces the victim 3 m away and fires one aimed shot (right mouse held, the
# camera pitched down by a relative mouse move of 40 px up — y is inverted in these lanes; without it the shot passes
# over the victim's head —, left click). Positions are not checked here: the hit reaction moves the peds and would
# widen the one position tolerance of every step (hit_setup_* checks them). The server log gives the shooter's 206 hit (OnPlayerWeaponShot), the damage both clients reported
# (OnPlayerGiveDamage / OnPlayerTakeDamage, RPC 115); the shooter's synced view of the victim's health is compared
# with the victim's own health ("post": `synced`).
STEPS = STEPS + [st for a in ("A", "B") for st in (
    {"name": f"hit_setup_{a}", "actor": "both",
     "events": [(0.0, "rcon_each", "xc hpar {id} 100 0 ;"), (0.2, "rcon_each", "xc weapon {id} 24 50 ;"),
                (0.5, "tp_hit", a)],
     "window": 3.5, "checks": ["visible", "pos", "heading", "weapon", "health"]},
    {"name": f"combat_hit_{a}", "actor": a,
     "events": [(0.0, "mousedown", "3"), (0.6, "mouserel", "0 -40"), (1.2, "mousedown", "1"), (1.4, "mouseup", "1"),
                (2.6, "mouseup", "3")],
     "window": 4.5, "post": ["synced"], "checks": ["visible", "weapon", "hit", "hit_damage", "victim_health"]})]

# Enabled when the milestone lands; never guessed before (PLAN §3b). Every milestone is enabled now.
PENDING = []


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
            second += [(v["_veh"], 0x60), (v["_veh"] + VEH_UPGRADES, 30), (v["_veh"] + VEH_DRIVER, 36),
                       (v["_veh"] + VEH_TRAILER, 4)]
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
                v["vehicle"] = {"ptr": f"{v['_veh']:#x}", "model": struct.unpack_from("<h", vh, E_MODEL)[0],
                                "seat": occupants.index(p) if p in occupants else -1,
                                "speed": round(math.sqrt(sum(c * c for c in struct.unpack_from("<3f", vh, PHYS_SPEED))), 4),
                                "mods": sorted(m for m in struct.unpack("<15h", up) if m > 0),
                                "_matrix": u32(vh, E_MATRIX)}
                tb = r2.get((v["_veh"] + VEH_TRAILER, 4))
                v["vehicle"]["_trailer"] = u32(tb) if tb else 0
        cdb = r2.get((v["_clump"] + clump_off, 4)) if v["_clump"] and clump_off else None
        if cdb and u32(cdb):
            cds[p] = u32(cdb)
    mats = conn.peeks([(v["vehicle"]["_matrix"], 0x40) for v in views.values()
                       if v.get("vehicle") and v["vehicle"]["_matrix"]]
                      + [(v["vehicle"]["_trailer"] + E_MODEL, 2) for v in views.values()
                         if v.get("vehicle") and v["vehicle"]["_trailer"]])
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
        tr = vv.pop("_trailer")
        tm = mats.get((tr + E_MODEL, 2)) if tr else None
        vv["trailer"] = struct.unpack("<h", tm)[0] if tm else (-1 if tr else 0)
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


def _placement(conn, ents):
    """{entity: (model, pos, forward, up)} from each entity's model index and matrix (CPlaceable::m_matrix,
    CMatrix right +0, forward +0x10, up +0x20, pos +0x30)."""
    ents = [e for e in ents if e]
    hdr = conn.peeks([(e, 0x24) for e in ents])
    mats = {}
    for e in ents:
        h = hdr.get((e, 0x24))
        if h and u32(h, E_MATRIX):
            mats[e] = (struct.unpack_from("<h", h, E_MODEL)[0], u32(h, E_MATRIX))
    mb = conn.peeks([(m, 0x40) for _, m in mats.values()])
    out = {}
    for e, (model, m) in mats.items():
        b = mb.get((m, 0x40))
        if b:
            out[e] = (model, [round(c, 3) for c in struct.unpack_from("<3f", b, 0x30)],
                      [round(c, 4) for c in struct.unpack_from("<3f", b, 0x10)],
                      [round(c, 4) for c in struct.unpack_from("<3f", b, 0x20)])
    return out


def read_tracked(conn, ptr):
    """One vehicle by pointer (the step's tracked vehicle) and the trailer hitched to it: model, position,
    heading (atan2 of the forward vector, as `vehicle` in read_peds), speed."""
    if not ptr:
        return None
    r = conn.peeks([(ptr, 0x60), (ptr + VEH_TRAILER, 4)])
    h, tb = r.get((ptr, 0x60)), r.get((ptr + VEH_TRAILER, 4))
    if not h:
        return {"ptr": f"{ptr:#x}", "error": "unreadable"}
    trailer = u32(tb) if tb else 0
    pl = _placement(conn, [ptr, trailer])

    def view(e, hb=None):
        if e not in pl:
            return None
        model, pos, fwd, _up = pl[e]
        v = {"ptr": f"{e:#x}", "model": model, "pos": pos, "heading": round(math.atan2(-fwd[0], fwd[1]), 4)}
        if hb:
            v["speed"] = round(math.sqrt(sum(c * c for c in struct.unpack_from("<3f", hb, PHYS_SPEED))), 4)
        return v

    v = view(ptr, h) or {"ptr": f"{ptr:#x}", "error": "no matrix"}
    v["trailer"] = view(trailer) if trailer else None
    return v


def read_objects(conn, models):
    """Every object of the GTA object pool whose model is in `models`: model, position, forward and up vectors."""
    hb = conn.peeks([(OBJECT_POOL, 4)]).get((OBJECT_POOL, 4))
    pool = u32(hb) if hb else 0
    if not pool:
        return {"error": "no object pool"}
    ph = conn.peeks([(pool, 12)]).get((pool, 12))
    if not ph:
        return {"error": "object pool unreadable"}
    objs, bmap, size = struct.unpack("<IIi", ph)
    size = max(0, min(size, 4096))
    flags = conn.peeks([(bmap, size)]).get((bmap, size)) or b""
    live = [objs + i * OBJECT_SIZE for i, f in enumerate(flags) if not f & 0x80]
    mb = conn.peeks([(e + E_MODEL, 2) for e in live])
    want = [e for e in live if mb.get((e + E_MODEL, 2)) and struct.unpack("<h", mb[(e + E_MODEL, 2)])[0] in models]
    pl = _placement(conn, want)
    return {"live": len(live), "objects": [{"ptr": f"{e:#x}", "model": m, "pos": p, "fwd": f, "up": u}
                                           for e, (m, p, f, u) in sorted(pl.items())]}


def parse_lines(lines, kind):
    """`<kind> k=v … text="…"` lines of a `bubbles` / `synced` reply → list of dicts (text kept escaped)."""
    out = []
    for ln in lines[1:]:
        if not ln.startswith(kind + " "):
            continue
        d = {}
        m = re.search(r' text="(.*)"$', ln)
        if m:
            d["text"] = m.group(1)
            ln = ln[:m.start()]
        for kv in ln.split()[1:]:
            if "=" in kv:
                k, v = kv.split("=", 1)
                d[k] = v
        out.append(d)
    return out


# --- sides ----------------------------------------------------------------------------
class Side:
    def __init__(self, key, port, display, name, kind, gamescope="", chatlog=""):
        self.key, self.port, self.display, self.name, self.kind = key, port, display, name, kind
        self.gamescope = gamescope
        self.chatlog = chatlog  # this lane's Documents/GTA San Andreas User Files/SAMP/chatlog.txt
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

    def sample(self, other_name, track=0):
        c = self.connect()
        t = time.time()
        lp = c.peeks([(PLAYERS0_PED, 4)]).get((PLAYERS0_PED, 4))
        local = u32(lp) if lp else 0
        rped, rlines = self.remote_ped(other_name)
        views = read_peds(c, [local, rped or 0], self.clump_off)
        out = {"t": round(t, 3), "local": views.get(local) if local else None,
               "remote": views.get(rped) if rped else None, "remote_raw": rlines}
        if track:
            out["track"] = read_tracked(c, track)
        return out

    def chatlog_size(self):
        try:
            return os.path.getsize(self.chatlog)
        except OSError:
            return 0

    def chatlog_since(self, off):
        """Lines appended to this lane's chatlog.txt since byte offset `off`."""
        try:
            with open(self.chatlog, "rb") as f:
                f.seek(off)
                data = f.read()
        except OSError as e:
            return {"error": str(e)}
        return {"lines": [ln for ln in data.decode("latin-1").replace("\r", "").split("\n") if ln]}

    def post(self, what):
        """End-of-step reads ("post" of a step)."""
        c = self.connect()
        if what == "objects":
            models = {OBJ_GLOBAL[0], *OBJ_VEHICLE.values(), *OBJ_PLAYER.values()}
            return read_objects(c, models)
        if what in ("bubbles", "synced"):
            lines = c.cmd_multi(what)
            return {"raw": lines, what: parse_lines(lines, what[:-1] if what == "bubbles" else what)}
        return {"error": f"unknown post read {what}"}

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
            "A": Side("A", a.a_port, a.a_display, a.a_name, a.a_kind, a.a_gamescope, a.a_chatlog),
            "B": Side("B", a.b_port, a.b_display, a.b_name, a.b_kind, a.b_gamescope, a.b_chatlog),
        }
        self.track = {}
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
        """({player name: info}, [vehicle info]) as the server sees them at the end of a step."""
        _m, lines = self.xc(f"xc info {tag} ;", wait=rf"\[xc\] ok info {re.escape(tag)}\b")
        out, veh = {}, []
        for ln in lines:
            m = re.search(r"\[xc\] info \S+ id=(\d+) name=(\S+) (.*)", ln)
            if m:
                out[m.group(2)] = dict(kv.split("=", 1) for kv in m.group(3).split())
            m = re.search(r"\[xc\] vinfo \S+ (.*)", ln)
            if m:
                veh.append(dict(kv.split("=", 1) for kv in m.group(1).split()))
        return out, veh

    def sample_both(self):
        futs = {k: self.pool.submit(s.sample, self.sides[self.other(k)].name, self.track.get(k, 0))
                for k, s in self.sides.items()}
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
        elif kind == "mouserel":
            for k in targets:
                self.sides[k].xdo("mousemove_relative", "--", *arg.split())
        elif kind == "rcon_each":
            for k in targets:
                s = self.sides[k]
                self.xc(arg.format(id=s.pid, other=self.sides[self.other(k)].pid, skin=SKIN_STEP[k], side=k,
                                   vmodel=OBJ_VEHICLE[k], pmodel=OBJ_PLAYER[k]))
        elif kind == "gobj":
            m, x, y, z, rz = OBJ_GLOBAL
            self.xc(f"xc gobj {m} {x:.3f} {y:.3f} {z:.3f} {rz:.1f} ;")
        elif kind == "truck":
            for k in targets:
                x, y, z = TRUCK_AT[k]
                self.xc(f"xc truck {self.sides[k].pid} {x:.3f} {y:.3f} {z:.3f} ;")
        elif kind == "type":
            # Chat input like a player: T opens it, the text, Enter sends it (as tools/diff/netscen.sh).
            for k in targets:
                s = self.sides[k]
                s.xdo("key", "--delay", "80", "t")
                time.sleep(0.6)
                s.xdo("type", "--delay", "60", "--", CHAT_TEXT[k])
                time.sleep(0.3)
                s.xdo("key", "--delay", "80", "Return")
        elif kind == "ensure_spawn":
            # Spawned since the previous step (the death) began? Else the server spawns the player.
            for k in targets:
                if self.spawn_count(k) <= self.step_spawns[-2][k]:
                    self.forced_spawn.append(k)
                    self.xc(f"xc spawn {self.sides[k].pid} ;")
        elif kind == "tp_far":
            # The players other than the actor: 60 m north of the unoccupied-car lane.
            for k in self.sides:
                if actor != "both" and k != actor:
                    self.xc(f"xc tp {self.sides[k].pid} {FAR_SPOT[0]:.3f} {FAR_SPOT[1]:.3f} {FAR_SPOT[2]:.3f} 90.0 ;")
        elif kind in ("tp", "tp_hit"):
            for k in targets:
                x, y, z, ang = (SPOT if kind == "tp" else SPOT_HIT[arg])[k]
                self.xc(f"xc tp {self.sides[k].pid} {x:.3f} {y:.3f} {z:.3f} {ang:.1f} ;")

    def spawn_count(self, k):
        """Server-side spawns of side k's player so far (`[e2e] spawn id=N` lines of the server log)."""
        try:
            text = open(self.log.path, errors="replace").read()
        except OSError:
            return 0
        return len(re.findall(rf"\[e2e\] spawn id={self.sides[k].pid}\b", text))

    def run_step(self, st):
        rec = {"name": st["name"], "actor": st["actor"], "checks": st["checks"], "window": st["window"],
               "pids": {k: s.pid for k, s in self.sides.items()}, "names": {k: s.name for k, s in self.sides.items()}}
        if not hasattr(self, "step_spawns"):
            self.step_spawns = []
        self.step_spawns.append({k: self.spawn_count(k) for k in self.sides})
        self.forced_spawn = []
        self.track = {}
        rec["pre"] = [self.sample_both() for _ in range(2)]
        if st.get("track"):
            # The actor's vehicle on both sides: its own local ped's car, the car of its remote ped on the other.
            a = st["actor"]
            last = rec["pre"][-1]
            for k in self.sides:
                v = ((last.get(k) or {}).get("local" if k == a else "remote") or {}).get("vehicle") or {}
                self.track[k] = int(v["ptr"], 16) if v.get("ptr") else 0
            rec["track"] = {k: f"{p:#x}" for k, p in self.track.items()}
        chat_off = {k: s.chatlog_size() for k, s in self.sides.items()} if "chatlog" in st.get("post", []) else {}
        self.log.skip_to_end()
        log_from = len(self.log.hist)
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
        self.track = {}
        if st.get("post"):
            time.sleep(0.5)  # the last chat line / RPC of the window reaches the files and pools
            rec["post"] = {}
            for k, s in self.sides.items():
                rec["post"][k] = {}
                for what in st["post"]:
                    try:
                        rec["post"][k][what] = (s.chatlog_since(chat_off.get(k, 0)) if what == "chatlog"
                                                else s.post(what))
                    except OSError as e:
                        s.close()
                        rec["post"][k][what] = {"error": f"{type(e).__name__}: {e}"}
        self.log.skip_to_end()
        rec["server_log"] = [ln for ln in self.log.hist[log_from:]
                             if re.search(r"\[xc\] (shot|takedamage|givedamage|unocc) ", ln)]
        if self.forced_spawn:
            rec["forced_spawn"] = self.forced_spawn
            say(f"step {st['name']}: no spawn request from {self.forced_spawn} after the death; server spawned")
        # The tracer serves one connection at a time: reuse the persistent one.
        rec["state"] = {k: self.side_cmd(s, "state") for k, s in self.sides.items()}
        rec["server"], rec["server_veh"] = self.server_view(st["name"])
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
        only = os.environ.get("XC_STEPS")  # development: a regex of step names (the report then refuses the run)
        for st in STEPS:
            if only and not re.search(only, st["name"]):
                continue
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


# --- checks of the M4–M7 steps ---------------------------------------------------------
# Delta checks (a position/angle difference against a learned tolerance) and scalar checks (sender value vs
# observer value, same calibration rules as `scalar`). "Sender" is always the acting player's own client (for the
# combat hits: the shooter), "observer" the other client (the victim).
EXT_DELTA = {"uveh_pos", "uveh_heading", "uveh_server", "trailer_pos", "trailer_heading", "trailer_server",
             "obj_global", "obj_vehicle", "obj_player"}
EXT_SCALAR = {"trailer", "chat", "bubble", "bubble_colour", "hit", "hit_damage", "victim_health"}


def _last_track(rec, k):
    for smp in reversed(rec["samples"]):
        t = (smp.get(k) or {}).get("track")
        if t:
            return t
    return None


def _server_veh(rec, owner_pid, kind):
    for v in rec.get("server_veh", []):
        if v.get("owner") == str(owner_pid) and v.get("kind") == kind:
            try:
                return [float(c) for c in v["pos"].split(",")]
            except (KeyError, ValueError):
                return None
    return None


def _angle(u, v):
    nu, nv = math.sqrt(sum(c * c for c in u)), math.sqrt(sum(c * c for c in v))
    if nu == 0 or nv == 0:
        return math.pi
    return math.acos(max(-1.0, min(1.0, sum(a * b for a, b in zip(u, v)) / (nu * nv))))


def ext_delta(chk, rec, sender, observer):
    """(delta or None, rdelta or None, text) of a delta check."""
    if chk.startswith("uveh_") or chk.startswith("trailer_"):
        ts, to = _last_track(rec, sender), _last_track(rec, observer)
        part = "trailer" if chk.startswith("trailer_") else None
        if part:
            ts, to = (ts or {}).get("trailer"), (to or {}).get("trailer")
        if chk.endswith("_server"):
            srv = _server_veh(rec, rec["pids"][sender], "trailer" if part else "car")
            if not ts or "pos" not in ts or srv is None:
                return None, None, "tracked vehicle or server vehicle missing"
            d = dist(ts["pos"], srv)
            return d, None, f"server Δ={d:.3f} m"
        if not ts or not to or "pos" not in ts or "pos" not in to:
            return None, None, f"{part or 'vehicle'} not tracked on both sides (sender {bool(ts)}, observer {bool(to)})"
        if chk.endswith("_pos"):
            d = dist(ts["pos"], to["pos"])
            return d, None, f"Δ={d:.3f} m"
        d = angdiff(ts["heading"], to["heading"])
        return d, None, f"Δ={d:.3f} rad"
    # Objects: obj_global (the server's object), obj_vehicle (the one on the sender's car), obj_player (the one on
    # the sender's head): the sender's client vs the observer's, matched pairwise by position.
    model = {"obj_global": OBJ_GLOBAL[0], "obj_vehicle": OBJ_VEHICLE[sender], "obj_player": OBJ_PLAYER[sender]}[chk]
    po = rec.get("post", {})
    ls = [o for o in (po.get(sender, {}).get("objects") or {}).get("objects", []) if o["model"] == model]
    lo = [o for o in (po.get(observer, {}).get("objects") or {}).get("objects", []) if o["model"] == model]
    if not ls or len(ls) != len(lo):
        return None, None, f"model {model}: sender shows {len(ls)}, observer {len(lo)}"
    d = r = 0.0
    rest = list(lo)
    for a in ls:
        b = min(rest, key=lambda o: dist(a["pos"], o["pos"]))
        rest.remove(b)
        d = max(d, dist(a["pos"], b["pos"]))
        r = max(r, _angle(a["fwd"], b["fwd"]), _angle(a["up"], b["up"]))
    return d, r, f"model {model} x{len(ls)}: Δ={d:.3f} m, rotation Δ={r:.3f} rad"


def _norm_chat(line, names):
    line = re.sub(r"^\[\d\d:\d\d:\d\d\] ", "", line)
    for k, n in sorted(names.items(), key=lambda kn: -len(kn[1])):  # XcOrig2 before XcOrig
        line = line.replace(n, "{" + k + "}")
    return line


def ext_scalar(chk, rec, sender, observer):
    """(sender value, observer value) of a scalar check."""
    po = rec.get("post", {})
    ps, pob = rec["pids"][sender], rec["pids"][observer]
    if chk == "trailer":
        # Model of the trailer hitched to the sender's car (0: none), on its own client and on the observer's.
        last = rec["samples"][-1] if rec["samples"] else {}
        vs = (((last.get(sender) or {}).get("local") or {}).get("vehicle") or {})
        vo = (((last.get(observer) or {}).get("remote") or {}).get("vehicle") or {})
        return vs.get("trailer"), vo.get("trailer")
    if chk == "chat":
        def lines(k):
            got = po.get(k, {}).get("chatlog") or {}
            if "error" in got:
                return None
            return [_norm_chat(ln, rec["names"]) for ln in got.get("lines", []) if CHAT_TEXT[sender] in ln]
        return lines(sender), lines(observer)
    if chk in ("bubble", "bubble_colour"):
        mine = [b for b in (po.get(observer, {}).get("bubbles") or {}).get("bubbles", []) if b.get("id") == str(ps)]
        if chk == "bubble":
            want = (f"bubble of {sender}", BUBBLE["distance"], BUBBLE["duration"])
            got = (mine[0].get("text"), float(mine[0]["distance"]), int(mine[0]["duration"])) if mine else None
            return want, got
        return BUBBLE["colour"], (int(mine[0]["colour"], 16) if mine else None)
    if chk in ("hit", "hit_damage"):
        log = rec.get("server_log", [])

        def kv(ln):
            return dict(x.split("=", 1) for x in ln.split() if "=" in x)
        shots = [kv(ln) for ln in log if "[xc] shot " in ln and f" id={ps} " in ln]
        give = [kv(ln) for ln in log if "[xc] givedamage " in ln and f" id={ps} " in ln and f" damaged={pob} " in ln]
        take = [kv(ln) for ln in log if "[xc] takedamage " in ln and f" id={pob} " in ln and f" issuer={ps} " in ln]
        if chk == "hit":
            hit = any(x.get("hittype") == "1" and x.get("hitid") == str(pob) for x in shots)
            base = "miss" if shots else "no shot"
            return ("hit" if hit else base), ("hit" if take else base)
        return (round(sum(float(x["amount"]) for x in give), 2), round(sum(float(x["amount"]) for x in take), 2))
    if chk == "victim_health":
        syn = [x for x in (po.get(sender, {}).get("synced") or {}).get("synced", []) if x.get("id") == str(pob)]
        last = rec["samples"][-1] if rec["samples"] else {}
        own = ((last.get(observer) or {}).get("local") or {}).get("health")
        return (float(syn[0]["health"]) if syn else None), own
    return None


def same(chk, a, b):
    if a is None or b is None:
        return False
    if chk in ("health", "armour"):
        return abs(a - b) <= TOL[chk]
    if chk == "victim_health":
        return abs(a - b) <= TOL["health"]
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
        elif chk in EXT_DELTA:
            d, r, txt = ext_delta(chk, rec, sender, observer)
            if d is None:
                ok, val = False, txt
            else:
                t_ = tol.get(chk, TOL[chk])
                ok, extra = d <= t_, {"delta": d}
                val = f"{txt} (tol {t_:.3f}"
                if r is not None:
                    tr = tol.get(chk + "_rot", TOL[chk + "_rot"])
                    ok = ok and r <= tr
                    extra["rdelta"] = r
                    val += f", rotation tol {tr:.3f}"
                val += ")"
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
            a_, b_ = ext_scalar(chk, rec, sender, observer) if chk in EXT_SCALAR else scalar(chk, ser, last_s, last_o)
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
    showed (floors TOL_FLOOR); the TOL defaults without a calibration run.
    Only the step-directions the report judges are measured (a one-actor step is judged actor → other only):
    the other direction of such a step is no relation of that step (e.g. a bystander's ped nudged by the
    actor's rolling car, 0.45 m on the original, veh_unoccupied_B)."""
    tol = dict(TOL)
    if not calib_recs:
        return tol
    kinds = {"pos": "pos", "heading": "heading", **LEARNED}
    mx = {k: None for k in kinds}
    for rec in calib_recs:
        for s, o in directions(rec):
            ev = evaluate(rec, s, o, {k: 1e9 for k in kinds})
            for k in kinds:
                c = ev["checks"].get(k[:-4] if k.endswith("_rot") else k)
                key = "rdelta" if k.endswith("_rot") else "delta"
                if c and key in c:
                    mx[k] = max(mx[k] or 0.0, c[key])
    for k, kind in kinds.items():
        if mx[k] is not None:
            tol[k] = max(TOL_FLOOR[kind], round(1.5 * mx[k], 3))
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
            print(f"[xc] calibration {os.path.relpath(c, DATA_ROOT)} not used: positions apart beyond the "
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
    calib_run = ", ".join(os.path.relpath(c, DATA_ROOT) for c, _ in used) or None
    rows = []
    for rec in recs:
        for s, o in directions(rec):
            ev = evaluate(rec, s, o, tol, calib_ev.get((rec["name"], s, o)), min_hold=min_hold)
            svo = ev["checks"].get("sender_vs_original")
            if svo and svo["ok"] is False and sides[s]["kind"] == "orig":
                # The sender IS the original client: a difference from the calibration senders is the original's
                # own run-to-run variance (e.g. a short walk animation on the way to the car door), not ours.
                svo["ok"] = None
                svo["val"] = f"not judged, the sender is the original ({svo['val']})"
                ev["verdict"] = "MATCH" if all(c["ok"] is not False for c in ev["checks"].values()) else "DIFF"
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
    summary = {"run": os.path.relpath(run, DATA_ROOT), "calibration": calib_rel, "sides": sides,
               "enabled": len(enabled), "match": nmatch, "diff": len(enabled) - nmatch,
               "pending": len(rows) - len(enabled), "tolerances": tol, "anim_min_hold": min_hold,
               "calibration_skipped": [os.path.relpath(c, DATA_ROOT) for c in skipped], "rows": rows,
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
        d.add_argument(f"--{s}-chatlog", default="", help="that lane's SAMP/chatlog.txt (chat steps)")
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
