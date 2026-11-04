"""Extended GTA state snapshot for DIFF-2, read through the control channel's
`peek ADDR LEN` (same protocol on samp-client and samp-tracer, DIFF-1).

The `state` line only has the local ped's position/heading/health/armour; RPC
effects reach far more game state (money, wanted level, clock, weather,
weapons, vehicles, objects, pickups, radar blips, camera …). Everything here is
read from gta_sa.exe 1.0 US globals; addresses/offsets are named via
plugin-sdk (re/plugin-sdk/plugin_sa/game_sa, VALIDATE_OFFSET / *.cpp globals).

    snap = snapshot(Peeker(port))   # dict: key -> normalised value (str / list)

Values are normalised for comparison (floats rounded, pointers dropped);
entity lists are sorted so pool-slot order does not matter.
"""
import math
import socket
import struct

# Globals (plugin-sdk *.cpp)
G_GAME_STATE = 0xC8D4C0          # gGameState
CUR_AREA = 0xB72914              # CGame::currArea
PLAYERS = 0xB7CD98               # CWorld::Players[0]
CLOCK_MIN, CLOCK_HOUR = 0xB70152, 0xB70153   # CClock::ms_nGameClockMinutes/Hours
CLOCK_MS_PER_MIN = 0xB7015C      # CClock::ms_nMillisecondsPerGameMinute
WEATHER_FORCED, WEATHER_NEW, WEATHER_OLD = 0xC81318, 0xC8131C, 0xC81320  # CWeather
GRAVITY = 0x863984               # GAME_GRAVITY (common.cpp)
PAD0_DISABLE = 0xB73458 + 0x10E  # CPad::Pads[0].DisablePlayerControls
CAMERA = 0xB6F028                # TheCamera
CAM_WIDESCREEN, CAM_FADING, CAM_ACTIVE = 0x3D, 0x51, 0x59
CAM_CAMS, CAM_SIZE, CAM_MODE = 0x174, 0x238, 0xC
HUD_WANTS_TO_DRAW = 0xBAA45D     # CHud::m_Wants_To_Draw_Hud
HUD_NO_RADAR = 0xBAA3FB          # CHud::bScriptDontDisplayRadar
SCRIPTS_DISPLAY_HUD = 0xA444A0   # CTheScripts::bDisplayHud
MENU_ACTIVE = 0xBA6748 + 0x5C    # FrontEndMenuManager.m_bMenuActive
STATS_FLOAT = 0xB79380           # CStats::StatTypesFloat
POOL_PED, POOL_VEH, POOL_OBJ = 0xB74490, 0xB74494, 0xB7449C   # CPools::ms_p*Pool
STRIDE_PED, STRIDE_VEH, STRIDE_OBJ = 0x7C4, 0xA18, 0x19C      # sizeof(CCopPed/CHeli/CCutsceneObject)
PICKUPS, PICKUP_COUNT, PICKUP_SIZE = 0x9788C0, 620, 0x20      # CPickups::aPickUps
RADAR, RADAR_COUNT, RADAR_SIZE = 0xBA86F0, 175, 0x28          # CRadar::ms_RadarTrace

# Entity / ped / vehicle offsets (plugin-sdk VALIDATE_OFFSET)
E_MATRIX, E_PLACEMENT, E_MODEL, E_AREA, E_TYPE = 0x14, 0x4, 0x22, 0x2F, 0x36
PHYS_SPEED = 0x44
PED_PLAYERDATA, PED_STATE, PED_HEALTH, PED_ARMOUR = 0x480, 0x530, 0x540, 0x548
PED_HEADING, PED_VEHICLE, PED_WEAPONS, PED_WEAPON_SLOT, PED_FIGHT = 0x558, 0x58C, 0x5A0, 0x718, 0x72D
WEAPON_SIZE = 0x1C               # CWeapon: type +0, state +4, clip +8, total +0xC
PD_WANTED, PD_DRUNK = 0x0, 0x40  # CPlayerData
WANTED_LEVEL = 0x2C              # CWanted::m_nWantedLevel
VEH_COL1, VEH_HEALTH, VEH_LOCK = 0x434, 0x4C0, 0x4F8


class Peeker:
    """One persistent connection to a control channel; `peek` in ≤256-byte chunks."""

    def __init__(self, port, host="127.0.0.1", timeout=3.0):
        self.s = socket.create_connection((host, port), timeout=timeout)
        self.f = self.s.makefile("rb")
        self.cache = {}

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass

    def cmd(self, line):
        self.s.sendall((line + "\n").encode())
        return self.f.readline().decode(errors="replace").strip()

    def read(self, addr, n):
        out = b""
        while n > 0:
            k = min(n, 256)
            r = self.cmd(f"peek {addr:#x} {k}")
            # samp-client prints the bytes as one hex run, the tracer space-separated.
            parts = r.split()
            if len(parts) < 3 or parts[0] != "peek":
                return None
            data = bytes.fromhex("".join(parts[2:]))
            if len(data) != k:
                return None
            out += data
            addr += k
            n -= k
        return out

    def u8(self, a):
        b = self.read(a, 1)
        return b[0] if b else None

    def u16(self, a):
        b = self.read(a, 2)
        return struct.unpack("<H", b)[0] if b else None

    def i16(self, a):
        b = self.read(a, 2)
        return struct.unpack("<h", b)[0] if b else None

    def u32(self, a):
        b = self.read(a, 4)
        return struct.unpack("<I", b)[0] if b else None

    def i32(self, a):
        b = self.read(a, 4)
        return struct.unpack("<i", b)[0] if b else None

    def f32(self, a):
        b = self.read(a, 4)
        return struct.unpack("<f", b)[0] if b else None


def rnd(v, step):
    """Round to a multiple of `step` (string, -0 folded to 0)."""
    if v is None or (isinstance(v, float) and (math.isnan(v) or math.isinf(v))):
        return "?"
    r = round(v / step) * step
    if abs(r) < step / 2:
        r = 0.0
    digits = max(0, -int(math.floor(math.log10(step)))) if step < 1 else 0
    return f"{r:.{digits}f}"


def entity_pos(p, e):
    m = p.u32(e + E_MATRIX)
    a = (m + 0x30) if m else (e + E_PLACEMENT)
    b = p.read(a, 12)
    return struct.unpack("<3f", b) if b else None


def pool_entries(p, pool_ptr_addr, stride):
    """Addresses of the used slots of a CPool (byte map bit 0x80 = empty)."""
    pool = p.u32(pool_ptr_addr)
    if not pool:
        return []
    hdr = p.read(pool, 12)
    if not hdr:
        return []
    objs, bmap, size = struct.unpack("<IIi", hdr)
    if not objs or not bmap or not (0 < size <= 4096):
        return []
    flags = p.read(bmap, size) or b""
    return [objs + i * stride for i, f in enumerate(flags) if not f & 0x80]


def fmt_pos(pos, step=0.5):
    return ",".join(rnd(c, step) for c in pos) if pos else "?"


def near(a, b, r):
    return a and b and (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 <= r * r


def snapshot(p, radius=120.0):
    """Normalised dict of game state. `radius` limits entity lists to the
    area around the local ped (or the camera when there is no ped)."""
    s = {}
    gs = p.i32(G_GAME_STATE)
    s["game_state"] = str(gs)
    s["cur_area"] = str(p.i32(CUR_AREA))
    s["menu_active"] = str(p.u8(MENU_ACTIVE))
    s["clock_hour"] = str(p.u8(CLOCK_HOUR))
    s["clock_ms_per_min"] = str(p.u32(CLOCK_MS_PER_MIN))
    s["weather"] = f"{p.i16(WEATHER_OLD)}/{p.i16(WEATHER_NEW)}/forced {p.i16(WEATHER_FORCED)}"
    s["gravity"] = rnd(p.f32(GRAVITY), 0.0001)
    s["controls_disabled"] = str(p.u16(PAD0_DISABLE))
    s["widescreen"] = str(p.u8(CAMERA + CAM_WIDESCREEN))
    act = min(p.u8(CAMERA + CAM_ACTIVE) or 0, 2)
    s["cam_mode"] = str(p.u16(CAMERA + CAM_CAMS + act * CAM_SIZE + CAM_MODE))
    cam_pos = entity_pos(p, CAMERA)
    s["cam_pos"] = fmt_pos(cam_pos, 1.0)
    s["hud"] = f"draw={p.u8(HUD_WANTS_TO_DRAW)} script_hud={p.u8(SCRIPTS_DISPLAY_HUD)} no_radar={p.u8(HUD_NO_RADAR)}"
    s["money"] = str(p.i32(PLAYERS + 0xB8))
    skills = [p.f32(STATS_FLOAT + 4 * i) for i in range(69, 80)]
    s["weapon_skills"] = ",".join(rnd(v, 1.0) for v in skills)

    ped = p.u32(PLAYERS) if gs is not None and gs >= 9 else 0
    center = cam_pos
    if ped:
        pos = entity_pos(p, ped)
        center = pos or center
        s["ped_model"] = str(p.i16(ped + E_MODEL))
        s["ped_pos"] = fmt_pos(pos, 0.5)
        s["ped_heading"] = rnd(p.f32(ped + PED_HEADING), 0.05)
        s["ped_health"] = rnd(p.f32(ped + PED_HEALTH), 1.0)
        s["ped_armour"] = rnd(p.f32(ped + PED_ARMOUR), 1.0)
        s["ped_state"] = str(p.u32(ped + PED_STATE))
        s["ped_area"] = str(p.u8(ped + E_AREA))
        sp = p.read(ped + PHYS_SPEED, 12)
        s["ped_moving"] = str(bool(sp) and max(abs(c) for c in struct.unpack("<3f", sp)) > 0.01)
        veh = p.u32(ped + PED_VEHICLE) or 0
        s["ped_vehicle"] = str(p.i16(veh + E_MODEL)) if veh else "none"
        s["ped_weapon_slot"] = str(p.u8(ped + PED_WEAPON_SLOT))
        s["ped_fight_style"] = str(p.u8(ped + PED_FIGHT))
        wb = p.read(ped + PED_WEAPONS, 13 * WEAPON_SIZE) or b""
        weapons = []
        for i in range(len(wb) // WEAPON_SIZE):
            wtype, _st, clip, total = struct.unpack_from("<IIII", wb, i * WEAPON_SIZE)
            if wtype:
                weapons.append(f"{i}:{wtype}x{total}")
        s["ped_weapons"] = " ".join(weapons) or "none"
        pd = p.u32(ped + PED_PLAYERDATA) or 0
        if pd:
            s["drunk"] = str(p.u8(pd + PD_DRUNK))
            w = p.u32(pd + PD_WANTED) or 0
            s["wanted"] = str(p.u32(w + WANTED_LEVEL)) if w else "?"
    else:
        s["ped_model"] = "none"

    # Entities near the player (sorted; pointers and slot order dropped).
    peds = []
    for e in pool_entries(p, POOL_PED, STRIDE_PED):
        if e == ped:
            continue
        pos = entity_pos(p, e)
        if near(pos, center, radius):
            peds.append(f"{p.i16(e + E_MODEL)}@{fmt_pos(pos, 1.0)}")
    s["peds"] = sorted(peds)
    vehs = []
    for e in pool_entries(p, POOL_VEH, STRIDE_VEH):
        pos = entity_pos(p, e)
        if near(pos, center, radius):
            b = p.read(e + VEH_COL1, 2) or b"\0\0"
            vehs.append(
                f"{p.i16(e + E_MODEL)}@{fmt_pos(pos, 1.0)} hp={rnd(p.f32(e + VEH_HEALTH), 10.0)}"
                f" col={b[0]}/{b[1]} lock={p.u32(e + VEH_LOCK)} area={p.u8(e + E_AREA)}"
            )
    s["vehicles"] = sorted(vehs)
    objs = []
    for e in pool_entries(p, POOL_OBJ, STRIDE_OBJ):
        pos = entity_pos(p, e)
        if near(pos, center, radius):
            objs.append(f"{p.i16(e + E_MODEL)}@{fmt_pos(pos, 1.0)}")
    s["objects"] = sorted(objs)

    pk = p.read(PICKUPS, PICKUP_COUNT * PICKUP_SIZE) or b""
    pickups = []
    for i in range(len(pk) // PICKUP_SIZE):
        o = i * PICKUP_SIZE
        ptype = pk[o + 0x1C]
        if ptype == 0:
            continue
        x, y, z = (c / 8.0 for c in struct.unpack_from("<3h", pk, o + 0x10))
        model = struct.unpack_from("<h", pk, o + 0x18)[0]
        pickups.append(f"{model} t{ptype}@{fmt_pos((x, y, z), 1.0)}")
    s["pickups"] = sorted(pickups)

    rb = p.read(RADAR, RADAR_COUNT * RADAR_SIZE) or b""
    blips = []
    for i in range(len(rb) // RADAR_SIZE):
        o = i * RADAR_SIZE
        if not rb[o + 0x25] & 0x02:  # m_bInUse
            continue
        colour = struct.unpack_from("<I", rb, o)[0]
        x, y, z = struct.unpack_from("<3f", rb, o + 8)
        sprite = rb[o + 0x24]
        btype = rb[o + 0x26] >> 4
        size = struct.unpack_from("<H", rb, o + 0x1C)[0]
        # eBlipType: 1 car, 2 char, 3 object, 7 pickup follow an entity; 4/5/6/8 are coordinates.
        where = fmt_pos((x, y, z), 1.0) if btype in (4, 5, 6, 8) else "entity"
        blips.append(f"type{btype} sprite{sprite} col{colour} size{size}@{where}")
    s["blips"] = sorted(blips)
    return s


def take(port):
    """Snapshot through a fresh connection; {"error": …} if unreachable."""
    try:
        p = Peeker(port)
    except OSError as e:
        return {"error": str(e)}
    try:
        return snapshot(p)
    except (OSError, struct.error, ValueError) as e:
        return {"error": f"{type(e).__name__}: {e}"}
    finally:
        p.close()


if __name__ == "__main__":
    import json
    import sys

    print(json.dumps(take(int(sys.argv[1])), indent=1))
