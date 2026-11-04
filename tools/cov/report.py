#!/usr/bin/env python3
"""COV-1 report: per scenario window, the gta_sa.exe functions hit ONLY with our client and
ONLY with the original, grouped by GTA subsystem.

    report.py --orig RUN/cov-A.tsv --ours RUN/cov-B.tsv [--calib RUN2/cov-A.tsv --calib RUN2/cov-B.tsv]
              [--inv target/cov/gta-functions.tsv] [--out target/cov/coverage-diff.md]

--calib (repeatable): more ORIGINAL runs (run.sh calib gives two); a function counts as
`only ours` only if no original run hit it, as `only orig` only if every original run hit it;
hits that differ between original runs are noise.
Only functions armed in every compared run's window count. Writes the markdown report and
a TSV with every difference next to it (coverage-diff.tsv).
"""
import argparse
import os
import re
from bisect import bisect_right
from collections import defaultdict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# Subsystem by class/function-name prefix (first match wins), then by gta-reversed category.
SUBSYS = [
    ("population", r"^(CPopulation|CPopCycle|CPedPlacement|CDummyPed|CCivilianPed|CEmergencyPed|CPedType|CLoadedCarGroup|CStreamedScripts)"),
    ("car generators", r"^(CCarGenerator|CTheCarGenerators|CSpecialPlateHandler)"),
    ("traffic / car AI", r"^(CCarCtrl|CCarAI|CAutoPilot|CCarPathLink|CPathFind|CPathNode|CCarEnterExit|CNodeAddress|CPathIntersectionInfo|CJunctions)"),
    ("wanted / police / crime", r"^(CWanted|CCopPed|CPolice|CRoadBlocks|CCrime|CSwat|CFireManager|CFire\b|CScriptedBrainTaskStore|CEmergency)"),
    ("gangs", r"^(CGangWars|CGangs|CGangInfo|CPedGroup|CPedGroups|CPedGroupMembership|CPedGroupIntelligence)"),
    ("audio / ambient", r"^(CAE|cAE|CAudio|cAudio|CAESound|CAEAudio|CAEStream|cTransmission|CAEWeather|cDMAudio|CAEScript|CAERadio|CAEPolice|CAEGlobal|CAEPed|CAEVehicle|CAEFront|CAECollision|CAEDoor|CAEFire|CAEExplosion|CAEPedSpeech|CAEMP3|CAEStaticChannel|CAEStreamThread|CAEUser|CAETwin|CAECutscene|CAEMFX|CAEPoliceScanner|CAEScriptAudio|OALBase|CAEDataStream|CAEWaterCannon|CAEWeapon|CAEAmbience|CAEBass)"),
    ("stats / progress", r"^(CStats|CPlayerInfo::|CGameLogic|CRestart|CPedStats|CStatsLib)"),
    ("garages", r"^(CGarage|CGarages|CStoredCar)"),
    ("pickups", r"^(CPickup|CPickups|CCollectable)"),
    ("script", r"^(CRunningScript|CTheScripts|CScript|CMissionCleanup|CUpsideDownCarCheck|CStuckCarCheck|CScriptResource|CScripted|tScript|CTheZones|CZone|CStuntJump|COnscreen|CUserDisplay|CPlaceName|CCurrentVehicle|CScriptsForBrains)"),
    ("streaming / loading", r"^(CStreaming|CColStore|CIplStore|CTxdStore|CCdStream|CdStream|CFileLoader|CFileMgr|CModelInfo|CBaseModelInfo|CPedModelInfo|CVehicleModelInfo|CClumpModelInfo|CAtomicModelInfo|CDamageAtomicModelInfo|CWeaponModelInfo|CTimeModelInfo|CLodAtomicModelInfo|CStore|CKeyGen|CModelInfoAccelerator|CLoadingScreen|CGenericGameStorage|C_PcSave|CPedModelInfo|CCustomCarPlateMgr|CAnimManager::Load|CClothesBuilder|CTimeCycle::Initialise)"),
    ("camera", r"^(CCam\b|CCam::|CCamera|CCamPathSplines|CSpline|CCutsceneMgr|CCullZone|CCullZones)"),
    ("HUD / menu / text", r"^(CHud|CRadar|CMenuManager|CFont|CSprite2d|CSprite|CMessages|CText|CDraw|CWidget|CHudColours|CPlayerSkin|CMenuSystem|CControllerConfig|CBlip|CTouch|CGxt|CKeyArray|CData::|CMissionText|CPad|CMousePointer|CIntroScreen|CTitleScreen|CCredits|CSprite|CTextLine)"),
    ("weather / time", r"^(CWeather|CClock|CTimeCycle|CRainDrop|CClouds|CSkidmarks|CWaterLevel|CWaterCannon|CMovingThings|CRope|CHeliPilot|CPlaneTrail)"),
    ("render / fx", r"^(CRenderer|CVisibilityPlugins|CShadows|CCoronas|CBrightLights|CSpecialFX|CPostEffects|CMirrors|CFx|Fx|CParticle|C3dMarker|CCheckpoint|CBulletTraces|CPointLights|CGlass|CEntity::Render|CEntity::PreRender|CClothes|CCustomBuilding|CCustomRoad|CCarFXRenderer|CCustomCarEnvMapPipeline|CTagManager|CLines|CScene|RenderBuffer|CGrassRenderer|CPlantMgr|CPlant|CBirds|CShinyTexts|CWaterCannons|CProcObjectMan|ProcObject|CDecal|CMotionBlur|CWindModifiers|Rw|Rp|Rt|_rw|_rp|CBreakable|BreakManager|CBouncingPanel|CLensFlare|CLight|CTexDictionary|CCustomBuildingDNPipeline)"),
    ("vehicles", r"^(CVehicle|CAutomobile|CBike|CBmx|CQuadBike|CHeli|CPlane|CTrain|CBoat|CTrailer|CMonsterTruck|CDamageManager|cHandlingDataMgr|CTransmission|CCarEnterExit|CVehicleRecording|CRemote|CHandling|CTrailer|CDoor|CBouncingPanel|CPedHandling|CPlaneTrails|CTrainLink)"),
    ("peds / AI tasks / events", r"^(CPed\b|CPed::|CPlayerPed|CTask|CEvent|CPedIntelligence|CPedAttractor|CPedScriptedTaskRecord|CPedDamageResponse|CPedGeometryAnalyser|CPedList|CPedClothesDesc|CAcquaintance|CTaskManager|CTaskTimer|CDecisionMaker|CDecision|CPedEventDecision|CPointRoute|CPedTaskPair|CAttractor|CScriptedEffectPairs|CInformGroupEvent|CInformFriendsEvent|CPedGroupDefaultTask|CAnimBlend|CAnimManager|CAnim|CCutsceneObject|CIKChain|IKChain|BoneNode|CPedIK|CPedAcquaintance|CFormation|CPlayerData|CTaskComplex|CTaskSimple|CPedDebugVisualiser)"),
    ("entry-exits / interiors", r"^(CEntryExit|CEntryExitManager|Interior|InteriorManager|InteriorGroup|FurnitureManager|Furniture)"),
    ("world / physics / collision", r"^(CWorld|CPhysical|CCollision|CColModel|CColTriangle|CColSphere|CColBox|CColLine|CColPoint|CColTrianglePlane|CEntity|CPlaceable|CMatrix|CSimpleTransform|CObject|CDummy|CBuilding|CTreadable|CPtrList|CPtrNode|CEntryInfo|CSector|CRepeatSector|CQuadTree|CRect|CBox|CVector|CQuaternion|CGeneral|CMaths|CEntityScanner|CPedScanner|CVehicleScanner|CObjectScanner|CCollisionData|CLinkList|CDoubleLink|CRegisteredMotionBlur|CCover|CExplosion|CProjectile|CProjectileInfo|CWeapon|CWeaponInfo|CShotInfo|CBulletInfo|CCreepingFire|CFire|CHeli|CAccident|CAccidentManager|CEscalator|CShopping|CRope|CRopes|CConversations|CPedToPlayerConversations|CGridRef|CDarkel|CObjectData|CCustomRoadsignMgr)"),
    ("game loop / pools / misc", r"^(CGame|CTimer|CPool|CPools|CMemoryMgr|CIniFile|CGlobal|CDebug|CReplay|CTheZones|CCredits|CGameVariables|CSystem|CRandom|CPedType|CRegistry|CMemInfo|CFrontEnd|CPostEffects|operator new|operator delete|_GLOBAL|psInitialize|Rs|_psSelect|WinMain|MainWndProc)"),
]
SUBSYS = [(n, re.compile(p)) for n, p in SUBSYS]
CATEGORY_MAP = [
    ("Audio", "audio / ambient"), ("Entity/Ped", "peds / AI tasks / events"), ("Tasks", "peds / AI tasks / events"),
    ("Events", "peds / AI tasks / events"), ("Entity/Vehicle", "vehicles"), ("Scripts", "script"),
    ("Streaming", "streaming / loading"), ("Fx", "render / fx"), ("Plugins", "render / fx"),
    ("Animation", "peds / AI tasks / events"), ("Collision", "world / physics / collision"),
    ("Camera", "camera"), ("Interior", "entry-exits / interiors"), ("Pickups", "pickups"),
    ("Hud", "HUD / menu / text"), ("Frontend", "HUD / menu / text"), ("Weapons", "world / physics / collision"),
    ("Models", "streaming / loading"), ("Global", ""),
]


def subsystem(name, cat):
    for n, rx in SUBSYS:
        if rx.search(name):
            return n
    for c, n in CATEGORY_MAP:
        if cat.startswith(c) and n:
            return n
    return "other / unnamed"


class Inventory:
    def __init__(self, path):
        self.rows = {}
        for line in list(open(path))[1:]:
            p = line.rstrip("\n").split("\t")
            self.rows[int(p[0], 16)] = {"size": int(p[1]), "name": p[2], "src": p[3], "cat": p[4]}
        self.addrs = sorted(self.rows)
        self.named = [a for a in self.addrs if self.rows[a]["src"] != "ghidra"]

    def label(self, a):
        r = self.rows.get(a)
        if not r:
            return f"{a:#x}", "other / unnamed"
        if r["src"] != "ghidra":
            return r["name"], subsystem(r["name"], r["cat"])
        # Unnamed: the named neighbours (same compilation unit when both agree on the class).
        i = bisect_right(self.named, a)
        prev = self.rows[self.named[i - 1]] if i > 0 else None
        nxt = self.rows[self.named[i]] if i < len(self.named) else None
        cls = lambda r: r["name"].split("::")[0] if r else None  # noqa: E731
        near = cls(prev) if prev and (cls(prev) == cls(nxt) or not nxt) else None
        if near:
            return f"{r['name']} (in {near})", subsystem(prev["name"], prev["cat"])
        return r["name"], "other / unnamed"

    def containing(self, a):
        i = bisect_right(self.addrs, a)
        if i == 0:
            return None
        e = self.addrs[i - 1]
        return e if a < e + max(self.rows[e]["size"], 1) + 64 else None


def load_dump(path):
    wins = {}
    order = []
    for line in open(path):
        p = line.rstrip("\n").split("\t")
        if p[0] == "window":
            meta = dict(x.split("=", 1) for x in p[2:])
            wins[p[1]] = {"meta": meta, "skip": set(), "hits": {}}
            order.append(p[1])
        elif p[0] == "skip":
            wins[p[1]]["skip"].add(int(p[2], 16))
        elif p[0] == "hit":
            wins[p[1]]["hits"][int(p[2], 16)] = {"frame": int(p[3]), "ms": int(p[4]), "tid": int(p[5]), "caller": p[6]}
    return order, wins


def caller_label(inv, c):
    if c.startswith("0x"):
        a = int(c, 16)
        e = inv.containing(a)
        return f"{inv.label(e)[0]}+{a - e:#x}" if e else c
    return c


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--orig", required=True)
    ap.add_argument("--ours", required=True)
    ap.add_argument("--calib", action="append", default=[],
                    help="another ORIGINAL run's dump (repeatable): noise filter")
    ap.add_argument("--inv", default=os.path.join(ROOT, "target/cov/gta-functions.tsv"))
    ap.add_argument("--out", default=os.path.join(ROOT, "target/cov/coverage-diff.md"))
    ap.add_argument("--max-list", type=int, default=40)
    a = ap.parse_args()
    inv = Inventory(a.inv)
    armed_all = {e for e in open(os.path.join(os.path.dirname(a.inv), "arm.txt")).read().split() if e.startswith("0x")}
    armed_all = {int(x, 16) for x in armed_all}
    o_order, o = load_dump(a.orig)
    _, u = load_dump(a.ours)
    cal = [load_dump(p)[1] for p in a.calib]
    origs = [o] + cal
    windows = [w for w in o_order if w in u and all(w in c for c in cal)]
    md = ["# COV-1: gta_sa.exe function coverage, original client vs ours", "",
          f"original: `{a.orig}`  ", f"ours: `{a.ours}`  "]
    md += [f"calibration (another original run): `{p}`  " for p in a.calib] or ["calibration: none (no noise filter)  "]
    md += [f"inventory: {len(inv.rows)} functions, {len(armed_all)} armable", "",
           f"Per window, over the functions armed in every run ({len(origs)} original run(s)):",
           "`only ours` = hit with our client and in NO original run; `only orig` = hit in EVERY original run",
           "and not with ours; `noise` = hit in some original runs but not all (excluded from both).", "",
           "| window | armed | hit orig (run 1) | hit ours | common | only ours | only orig | noise (orig vs orig) |",
           "|---|---|---|---|---|---|---|---|"]
    tsv = ["window\tside\taddr\tname\tsubsystem\tframe_rel\tcaller\tcaller_raw"]
    detail = []
    never = {"ours": defaultdict(list), "orig": defaultdict(list)}
    # Over all windows: any original run hit it / every original run hit it somewhere.
    any_u = set()
    per_run_any = [set() for _ in origs]
    for w in windows:
        any_u |= set(u[w]["hits"])
        for k, r in enumerate(origs):
            per_run_any[k] |= set(r[w]["hits"])
    any_o = set().union(*per_run_any)
    all_o = set.intersection(*per_run_any)
    for w in windows:
        skip = u[w]["skip"].union(*(r[w]["skip"] for r in origs))
        armed = armed_all - skip
        hits = [set(r[w]["hits"]) & armed for r in origs]
        union, inter = set().union(*hits), set.intersection(*hits)
        hu = set(u[w]["hits"]) & armed
        noise = union - inter
        only_u = hu - union
        only_o = inter - hu
        md.append(f"| {w} | {len(armed)} | {len(hits[0])} | {len(hu)} | {len(hits[0] & hu)} | {len(only_u)} | {len(only_o)} | {len(noise) if cal else '-'} |")
        for side, s, src in (("ours", only_u, u), ("orig", only_o, o)):
            groups = defaultdict(list)
            f0 = int(src[w]["meta"].get("start_frame", "0"))
            for e in s:
                name, sub = inv.label(e)
                h = src[w]["hits"][e]
                groups[sub].append((h["frame"] - f0, name, e, caller_label(inv, h["caller"])))
                tsv.append(f"{w}\t{side}\t{e:#x}\t{name}\t{sub}\t{h['frame'] - f0}\t{caller_label(inv, h['caller'])}\t{h['caller']}")
                if (side == "ours" and e not in any_o) or (side == "orig" and e in all_o and e not in any_u):
                    never[side][sub].append((name, e, w))
            detail.append((w, side, groups))
    # Entries the original's client patched (skipped there) but that ran with ours: samp
    # rewrote the function's first bytes (e.g. a RET stub) and we do not.
    patched = defaultdict(set)
    for w in windows:
        po = o[w]["skip"] - u[w]["skip"]
        for e in po & set(u[w]["hits"]):
            patched[e].add(w)
    md += ["", "## Patched at the entry by the original client, executed with ours", "",
           "Functions whose first byte samp.dll had changed (so the original never armed them) and that our",
           "client let GTA execute.", "", "| function | subsystem | windows |", "|---|---|---|"]
    for e in sorted(patched):
        name, sub = inv.label(e)
        md.append(f"| `{name}` ({e:#x}) | {sub} | {', '.join(w for w in windows if w in patched[e])} |")
        tsv.append(f"*\tpatched-orig\t{e:#x}\t{name}\t{sub}\t-\t-\t-")
    md += ["", "## Never hit by the other client in ANY window", "",
           "The strongest signal: functions one client runs somewhere in the scenario and the other never does.", ""]
    for side, title in (("ours", "Only ours (GTA does this with our client, never with the original)"),
                        ("orig", "Only the original")):
        tot = len({e for v in never[side].values() for _n, e, _w in v})
        md += [f"### {title}: {tot} functions", "", "| subsystem | count | functions (first window) |", "|---|---|---|"]
        for sub, lst in sorted(never[side].items(), key=lambda kv: -len(kv[1])):
            seen = {}
            for name, e, w in sorted(lst, key=lambda x: x[1]):
                seen.setdefault(e, (name, w))
            names = ", ".join(f"`{n}` ({w})" for n, w in list(seen.values())[:a.max_list])
            more = f" … +{len(seen) - a.max_list}" if len(seen) > a.max_list else ""
            md.append(f"| {sub} | {len(seen)} | {names}{more} |")
        md.append("")
    md += ["## Per window", ""]
    for w, side, groups in detail:
        n = sum(len(v) for v in groups.values())
        if not n:
            continue
        md += [f"### {w}: only {side} ({n})", "", "| subsystem | count | functions (frame after window start; first caller) |", "|---|---|---|"]
        for sub, lst in sorted(groups.items(), key=lambda kv: -len(kv[1])):
            lst.sort()
            names = ", ".join(f"`{nm}` (f{fr}; {cl})" for fr, nm, _e, cl in lst[:a.max_list])
            more = f" … +{len(lst) - a.max_list}" if len(lst) > a.max_list else ""
            md.append(f"| {sub} | {len(lst)} | {names}{more} |")
        md.append("")
    os.makedirs(os.path.dirname(a.out), exist_ok=True)
    open(a.out, "w").write("\n".join(md) + "\n")
    open(os.path.splitext(a.out)[0] + ".tsv", "w").write("\n".join(tsv) + "\n")
    print("\n".join(md[:14 + len(windows)]))
    print(f"-> {a.out}")


if __name__ == "__main__":
    main()
