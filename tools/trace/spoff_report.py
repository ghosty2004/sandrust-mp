#!/usr/bin/env python3
"""TRACE-4: summarise single-player population / police activity in a trace (SP-OFF).

    tools/trace/spoff_report.py target/trace/orig-latest [--from MARK] [--to MARK]

For a `run.sh orig|ours idle` (or `run.sh bare N`) trace, prints:
  1. per-frame generator functions (Count hooks: CPopulation::Update, CCarCtrl::GenerateRandomCars,
     CTheCarGenerators::Process, …): in how many frames of the window each one ran;
  2. ped / vehicle creations (CPed::CPed by caller, CCopPed::CCopPed, CVehicle::CVehicle by
     created_by), with the GTA call stack of the first few;
  3. wanted level / crime / police scanner calls;
  4. population state (`watch`): first, min, max and last value of every counter in the window;
  5. ambient interior peds (CEntryExit::GenerateAmbientPeds, CPopulation::PopulateInterior).
The window defaults to the whole trace; --from/--to take mark prefixes (e.g. --from grove).
"""
import collections
import json
import os
import sys

COUNTERS = ("ped_pool_used", "vehicle_pool_used", "pop_total_peds", "pop_cops", "cars_random", "cars_law",
            "cars_parked", "cars_mission", "max_wanted", "ped_density", "car_density", "wanted_level",
            "wanted_chaos", "cur_area", "player_area")
CREATION = ("CPed::CPed", "CCopPed::CCopPed", "CVehicle::CVehicle", "CPopulation::AddPedInCar",
            "CPopulation::AddPedAtAttractor", "CPopulation::CreateWaitingCoppers")
POLICE = ("CWanted::SetWantedLevel", "CWanted::SetWantedLevelNoDrop", "CWanted::RegisterCrime",
          "CWanted::RegisterCrime_Immediately", "CWanted::UpdateWantedLevel", "CWanted::SetMaximumWantedLevel",
          "CAEPoliceScannerAudioEntity::AddAudioEvent", "CAEPoliceScannerAudioEntity::PlayPoliceScannerDialogue")
INTERIOR = ("CEntryExit::GenerateAmbientPeds", "CPopulation::PopulateInterior")
# GTA's CVehicle::m_nCreatedBy (eVehicleCreatedBy).
CREATED_BY = {1: "RANDOM", 2: "MISSION", 3: "PARKED", 4: "PERMANENT"}


def arg(argv, name):
    return argv[argv.index(name) + 1] if name in argv else None


def main():
    argv = sys.argv[1:]
    d = argv[0] if argv and not argv[0].startswith("--") else "target/trace/orig-latest"
    path = d if d.endswith(".jsonl") else os.path.join(d, "trace.jsonl")
    rows = [json.loads(l) for l in open(path)]
    marks = [(r["frame"], r["args"]["text"]) for r in rows if r["fn"] == "mark"]
    f0, f1 = 0, max(r["frame"] for r in rows)
    if arg(argv, "--from"):
        f0 = next(f for f, t in marks if t.startswith(arg(argv, "--from")))
    if arg(argv, "--to"):
        f1 = next(f for f, t in marks if t.startswith(arg(argv, "--to")))
    print(f"== {path}: frames {f0}..{f1} ({f1 - f0} frames)")
    for f, t in marks:
        print(f"   mark frame {f}: {t}")

    # Reconstruct the per-frame watch state (watch records carry changes only).
    state, on_frames, values = {}, collections.Counter(), collections.defaultdict(list)
    by_frame = collections.defaultdict(list)
    for r in rows:
        by_frame[r["frame"]].append(r)
    for f in range(0, f1 + 1):
        for r in by_frame.get(f, ()):
            if r["fn"] == "watch":
                for k, v in r["args"].items():
                    state[k] = v
        if f < f0:
            continue
        for k, v in state.items():
            if v == "on":
                on_frames[k] += 1
            if k in COUNTERS and v not in (None, ""):
                values[k].append(v)
    gens = sorted(k for k, v in state.items() if v in ("on", "off"))
    print("\n== 1. per-frame generators (frames in which the function ran)")
    for k in gens:
        if "::" in k:
            print(f"   {on_frames[k]:7d} / {f1 - f0 + 1}  {k}")

    win = [r for r in rows if f0 <= r["frame"] <= f1]
    print("\n== 2. creations")
    c = collections.Counter()
    first = {}
    for r in win:
        if r["fn"] not in CREATION:
            continue
        a = r["args"]
        if r["fn"] == "CPed::CPed":
            key = (r["fn"], f"ped_type {a.get('ped_type')}", r.get("caller"))
        elif r["fn"] == "CVehicle::CVehicle":
            cb = (a.get("created_by") or 0) & 0xFF
            key = (r["fn"], f"created_by {CREATED_BY.get(cb, cb)}", r.get("caller"))
        else:
            key = (r["fn"], json.dumps({k: v for k, v in a.items() if k != "vehicle"}), r.get("caller"))
        c[key] += 1
        first.setdefault(key, r)
    if not c:
        print("   none")
    for k, n in c.most_common():
        r = first[k]
        print(f"   {n:5d} x {k[0]} {k[1]} <{k[2]}> first frame {r['frame']}"
              f"{' via ' + r['via'] if r.get('via') else ''}{' stack ' + r['gta_stack'] if r.get('gta_stack') else ''}")
    models = collections.Counter(r["args"].get("model") for r in win if r["fn"] == "CPed::SetModelIndex"
                                 and not r["args"].get("this", {}).get("player"))
    if models:
        print(f"   CPed::SetModelIndex on non-player peds, by model: {dict(models.most_common(20))}")

    print("\n== 3. wanted level / crimes / police scanner")
    p = collections.Counter()
    firstp = {}
    for r in win:
        if r["fn"] in POLICE:
            k = (r["fn"], json.dumps(r["args"]), r.get("caller"), r.get("via"))
            p[k] += 1
            firstp.setdefault(k, r["frame"])
    if not p:
        print("   none")
    for k, n in sorted(p.items(), key=lambda kv: firstp[kv[0]]):
        print(f"   {n:5d} x {k[0]} {k[1]} <{k[2]}>{' via ' + k[3] if k[3] else ''} first frame {firstp[k]}")

    print("\n== 4. population state in the window (first / min / max / last)")
    for k in COUNTERS:
        vs = values.get(k)
        if not vs:
            continue
        try:
            nums = [float(v) for v in vs]
            print(f"   {k:18s} {vs[0]:>6s} {min(nums):>8g} {max(nums):>8g} {vs[-1]:>6s}")
        except ValueError:
            print(f"   {k:18s} {vs[0]} … {vs[-1]}")

    print("\n== 5. ambient interior peds")
    hits = [r for r in win if r["fn"] in INTERIOR]
    for r in hits:
        print(f"   frame {r['frame']} {r['fn']} {json.dumps(r['args'])} <{r.get('caller')}>")
    if not hits:
        print("   none")


if __name__ == "__main__":
    main()
