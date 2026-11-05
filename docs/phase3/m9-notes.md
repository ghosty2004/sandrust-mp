# Phase 3 / M9 — audio streams, radio calls, title line, PlayerPlaySound

Branch `audio-and-client` (git worktree `~/samp-rs-wt/m9`, lane "ui"). Acceptance: `tools/e2e/m9.sh`
(our client + samp-tracer with the BASS call log) and `tools/trace/bass_diff.py` against the original's
run of the same scenario (`tools/e2e/m9-orig.sh`, trace lane). Scenario: `tools/e2e/m9scen.sh`.
Spec: `docs/phase3/trace-notes.md` "Audio (TRACE-8)" (measured), `docs/re/subsystems/ui-chat-audio.md`
§41/§42 (static).

## Decision: the user's own bass.dll, imported statically (like samp.dll)

samp.dll imports 17 functions from `bass.dll` (BASS 2.4), which SA:MP ships in the GTA directory.
Our `samp_client.dll` imports the 12 it calls at runtime from the same `bass.dll` (Rust `raw-dylib`,
undecorated stdcall names; `i686-w64-mingw32-dlltool` builds the import stub). Nothing is
redistributed (consistent with D-4 = A: data and libraries come from the user's SA:MP install), and the
tracer's IAT redirect (`SAMP_TRACER_BASS=1`, `crates/samp-tracer/src/win/audio.rs`, which already looks
for `samp_client.dll`) logs our calls exactly like the original's, so the BASS call streams are diffed
1:1. Consequence: like samp.dll, our DLL does not load without `bass.dll` next to `gta_sa.exe` (every
lane's game copy has it). The five 3D functions are only referenced from samp's dead code
(B 0x10066B90) and are not imported.

## What is implemented (`crates/samp-client/src/audio.rs`)

| area | source |
|---|---|
| Boot (first frame with a D3D device, after the UI objects, `ui/glue.rs` `boot_settings`): `BASS_Free`, `BASS_Init(-1, 44100, 0, 0, 0)`, then only on success user agent `SA-MP/0.3`, `GVOL_STREAM` = trunc(radio volume × 7000), `NET_PLAYLIST` 1, `NET_TIMEOUT` 10000, `audioproxyoff` → `NET_PROXY` NULL, `SetEAXParameters(-1, 0, -1, -1)` | B 0x10066440, measured boot sequence |
| RPC 41 (main thread): stop with wait, stop a leftover handle, nothing while GTA's radio volume (0xB5FCC8) is not > 0, URL (256 bytes), position/distance/usePos, texts cleared, `Audio stream: <url>` type 4 unless `audiomsgoff`, worker thread | B 0x10066920 / 0x1001d370 |
| Worker: `StreamCreateURL(url, 0, 0x940000, 0, 0)`, `ChannelPlay(h, 0)`, station from ICY tags else HTTP tags (`icy-name:` / `icy-url:` case-insensitive, untrimmed), title from META `StreamTitle='…'` else OGG `artist=`/`title=`, syncs META + OGG_CHANGE (callback refreshes the title), then every 20 ms `StreamGetFilePosition(h, 4)` + `SetConfig(GVOL_STREAM, v)` until the stop flag, then `ChannelStop` | B 0x100666c0 / 0x10066580 |
| Volume: 0 in the GTA menu (0xBA67A4) or without focus (foreground ≠ 0xC97C1C); usePos: trunc(radio × 8000) × (1 − d/distance) for d < distance (d = local ped matrix position), else 0; otherwise trunc(radio × 7000) | B 0x10066823..0x100668e2; measured 7000 / 7945 @0.33 m / 4799 @20 m / 0 @60 m |
| RPC 42: stop without waiting (stop flag + `StreamFree` on the main thread); lost connection / server closed: with waiting (`lifecycle.rs`) | B 0x100180a0 → 0x10066520 |
| Per frame after the netgame tick while a worker runs: `CAudioEngine::StartRadio(-1, 0)` then `StopRadio(NULL, 0)` | B 0x10066a40 (called from B 0x100c4ffa) |
| Title line (Present step 4b, netgame exists): `"<title> - <station>"` when both are non-empty, small font, rect (15, H − 20)–(W, H + 10), `DT_NOCLIP`, 0x99FFFFFF, no outline passes | B 0x10066a70 → 0x1006b4a0(outline 0); UR §18 |
| PlayerPlaySound (RPC 16): 0 → (`0x040D(1)` if a mission sound was loaded) + `StopSpecialMissionAmbienceTrack`; 1 → flag only; < 1000 → `PlaySpecialMissionAmbienceTrack(id)`; < 2000 → `0x018C(x, y, z, id)`; else `0x040D(1)`, `0x03CF(1, id)`, `0x03D7(1, x, y, z)`, state 1; per frame in state 1 `0x03D0(1)` until true, then `0x03D1(1)` once (state 2) | B 0x100a22d0 / 0x100a21e0; measured |
| Control channel `audio`: ready, playing, handle, URL, usePos, distance, title, station, station URL, mission state | — |

## Parity (BASS call log, radio calls, sound commands)

`tools/trace/bass_diff.py ORIG OURS` cuts both traces at the scenario's marks and compares per step:
BASS calls in order (handles symbolic, URL host:port dropped, tag values null/list, durations and
repeat counts dropped, thread kept), the distinct stream volumes (±40), whether the per-frame radio pair
runs, and the sound script commands / ambience-track calls.

Result (original run `orig-20261105-020811` via `tools/e2e/m9-orig.sh`, ours `tools/e2e/m9.sh`):
`bass_diff: 0 difference(s) in 16 segment(s)` — boot sequence, global stream with ICY title,
replace (old worker `ChannelStop`, main `StreamFree(0)`), positional 7992 / 4799 / 0, Shoutcast
`ICY 200 OK` stream (no tags), menu 0 → 7000, stop (worker `ChannelStop` = 0, main `StreamFree` = 1),
unreachable URL (handle 0 throughout), radio pair while playing, sounds 1052 / 5 / 0 / 2000 (+ poll,
play) / 0 all identical. The title line was checked by screenshot (`target/m9/title.png`:
"Trace Artist - Title N -  TraceStation" at the bottom left; the double space is BASS's
`icy-name: …` header kept untrimmed, as samp does).

## Decisions

* No local ped matrix while a positional stream plays: samp's distance helper then leaves its output
  on uninitialised stack memory (B 0x1009eb40 null path); we use volume 0.
* A META tag without any `;` after `StreamTitle='`: samp writes before a null pointer (crash); we keep the
  whole rest as the title.
* `audioproxyoff` is created only after a successful `BASS_Init`, like samp (M10-P; m10-notes.md).

## Open (TODO-unknown, measurement requests)

* Whether GTA's radio resumes by itself after a stream stops (trace-notes: TODO-unknown) — not needed
  by our code (we make the same calls), but untested at audio level.
* The sound-id-1 flag (+4) has no traced reader; nothing uses it.
* samp's own class-selection sounds 1052/1053 (B 0x10006053 / 0x100060d6 through the same method) are
  not wired (class-selection owner).
