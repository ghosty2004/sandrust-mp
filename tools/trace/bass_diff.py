#!/usr/bin/env python3
"""M9 parity: audio behaviour of two samp-tracer traces of tools/e2e/m9scen.sh (original vs ours).

    tools/trace/bass_diff.py ORIG.jsonl OURS.jsonl

Both traces need the BASS call log (SAMP_TRACER_BASS=1). The traces are cut at the scenario's
`mark` records (m9_*); "boot" is everything before the first mark. Per segment it compares:

* the BASS calls in order (consecutive repeats collapsed; the tracer already folds identical
  calls). Handles become H1, H2, … in order of appearance (0 stays 0); the stream URL's host:port
  is dropped; tag values are reduced to null / list; `dur_ms`, timestamps and repeat counts are
  dropped; the thread (main / worker) is kept. META tag reads after the stream's syncs were set
  (BASS's metadata callback, timing-dependent) are only checked to happen somewhere in the run;
* stream volumes (`SetConfig(GVOL_STREAM)` values) as a sequence of values that differ by more than
  40 from the previous one, equal within ±40 (the positional volume depends on the ped's exact position);
* whether the per-frame `CAudioEngine::StartRadio(-1, 0)` / `StopRadio(NULL, 0)` pair runs;
* the sound script commands (0x018C, 0x03CF, 0x03D0, 0x03D1, 0x03D7, 0x040D; 0x03D0 repeats
  collapsed) and the ambience-track calls.

Exit 0 when every segment matches.
"""
import json
import re
import sys

SOUND_OPS = {"0x018c", "0x03cf", "0x03d0", "0x03d1", "0x03d7", "0x040d"}
VOL_TOL = 40


def load(path):
    recs = []
    with open(path, errors="replace") as f:
        for line in f:
            try:
                recs.append(json.loads(line))
            except ValueError:
                pass
    return recs


def segments(recs):
    seg, name = {}, "boot"
    seg[name] = []
    for r in recs:
        if r.get("fn") == "mark":
            text = str(r.get("args", {}).get("text", r.get("args", "")))
            m = re.search(r"(m9_\w+)", text)
            if m:
                name = m.group(1)
                seg.setdefault(name, [])
                continue
        seg[name].append(r)
    return seg


class Norm:
    def __init__(self):
        self.handles = {}

    def h(self, v):
        if v in (None, "0x0", 0, "0"):
            return "0"
        if v not in self.handles:
            self.handles[v] = "H%d" % (len(self.handles) + 1)
        return self.handles[v]

    def bass(self, r):
        a = dict(r["args"]["a"])
        call = r["args"]["call"].replace("BASS_", "")
        thread = a.pop("thread", "?")
        for k in ("dur_ms", "repeats_before", "repeat_span_ms"):
            a.pop(k, None)
        for k in list(a):
            if k.endswith("handle_ptr") or k == "ret_ptr":
                a[k] = self.h(a[k])
        if "url" in a:
            a["url"] = re.sub(r"^(\w+://)[^/]+", r"\1HOST", str(a["url"]))
        if "proc_ptr" in a:
            a["proc_ptr"] = "proc" if a["proc_ptr"] not in ("0x0", None) else "0"
        if "value" in a and call == "ChannelGetTags":
            a["value"] = "null" if a["value"] is None else "list"
        if call == "StreamGetFilePosition" and "ret" in a:
            a["ret"] = "ok" if isinstance(a["ret"], int) and 0 <= a["ret"] < 2**62 else "fail"
        return call, a, thread


def summarize(recs):
    n = Norm()
    calls, vols, radio, sounds = [], [], set(), []
    meta_callbacks = 0
    synced = False
    for r in recs:
        fn = r.get("fn")
        if fn == "bass":
            call, a, thread = n.bass(r)
            if call == "SetConfig" and a.get("name") == "GVOL_STREAM":
                # Jitter of a few units (the ped's exact position) is not a change.
                if not vols or abs(vols[-1] - a["value"]) > VOL_TOL:
                    vols.append(a["value"])
                if not synced:
                    line = "%s(GVOL_STREAM) [%s]" % (call, thread)
                    if not calls or calls[-1] != line:
                        calls.append(line)
                continue
            if call == "ChannelSetSync":
                synced = a.get("type_name") == "OGG_CHANGE" or synced
            elif call == "ChannelGetTags" and a.get("tag_name") == "META" and synced:
                meta_callbacks += 1
                continue
            if call == "StreamGetFilePosition":
                line = "%s(mode %s, %s) [%s]" % (call, a.get("mode"), a.get("ret"), thread)
            else:
                line = "%s(%s) [%s]" % (call, ", ".join("%s=%s" % (k, a[k]) for k in sorted(a)), thread)
            if call in ("ChannelPlay", "StreamCreateURL"):
                synced = False
            if not calls or calls[-1] != line:
                calls.append(line)
        elif fn in ("CAudioEngine::StartRadio", "CAudioEngine::StopRadio"):
            radio.add("%s %s" % (fn, json.dumps(r.get("args"), sort_keys=True)))
        elif fn == "scm" and str(r.get("args", {}).get("op", "")).lower() in SOUND_OPS:
            p = r["args"].get("params", [])
            line = "scm %s %s" % (r["args"]["op"].lower(), [round(x, 1) if isinstance(x, float) else x for x in p])
            if not (sounds and sounds[-1] == line and r["args"]["op"].lower() == "0x03d0"):
                sounds.append(line)
        elif fn and "AmbienceTrack" in fn:
            sounds.append("%s %s" % (fn, json.dumps(r.get("args"), sort_keys=True)))
    return calls, vols, sorted(radio), sounds, meta_callbacks > 0


def vols_equal(a, b):
    return len(a) == len(b) and all(abs(x - y) <= VOL_TOL for x, y in zip(a, b))


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    orig, ours = segments(load(sys.argv[1])), segments(load(sys.argv[2]))
    names = [k for k in orig if k in ours]
    missing = [k for k in orig if k not in ours] + [k for k in ours if k not in orig]
    bad = 0
    for k in names:
        o, u = summarize(orig[k]), summarize(ours[k])
        diffs = []
        if o[0] != u[0]:
            diffs.append(("BASS calls", o[0], u[0]))
        if not vols_equal(o[1], u[1]):
            diffs.append(("stream volumes", o[1], u[1]))
        if o[2] != u[2]:
            diffs.append(("radio calls", o[2], u[2]))
        if o[3] != u[3]:
            diffs.append(("sound commands", o[3], u[3]))
        status = "ok" if not diffs else "DIFF"
        print("== %-18s %s  (%d BASS lines, volumes %s, radio %s, sounds %d)" % (k, status, len(u[0]), u[1], "yes" if u[2] else "no", len(u[3])))
        for what, a, b in diffs:
            bad += 1
            print("   %s:" % what)
            for line in a:
                print("     orig: %s" % line)
            for line in b:
                print("     ours: %s" % line)
    # The META callback's tag reads are folded by the tracer while the title is unchanged, so they
    # show up only when the title changes (every 10 s on icyserve.py): compared over the whole run.
    cb_o = any(summarize(orig[k])[4] for k in names)
    cb_u = any(summarize(ours[k])[4] for k in names)
    print("== META sync callback reads after the syncs were set: orig %s, ours %s" % (cb_o, cb_u))
    if cb_o != cb_u:
        bad += 1
    if missing:
        print("segments only in one trace: %s" % missing)
        bad += 1
    print("bass_diff: %d difference(s) in %d segment(s)" % (bad, len(names)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
