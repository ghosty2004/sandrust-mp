# samp-rs

Clean-room Rust reimplementation of the SA:MP client, protocol-compatible with
existing servers (samp-server 0.3.7 and open.mp with `allow_037_clients`).
Multiple client versions must be supported: one shared protocol core plus a
small per-version profile (join version string, RPC differences, game addresses).

## Progress tracking
- `docs/PLAN.md` — master plan: definition of done (1:1), phases, milestone acceptance, autonomous loop.
- `docs/ROADMAP.md` — task board (status per task, verification, decisions waiting for the user). Read it first; update it after every step.
- `docs/WORKLOG.md` — append one entry per work step (what, how verified, cost).

## Rules
- **Clean room.** `docs/re/` holds specs written in our own words (layouts,
  constants, state machines). Never paste or line-by-line translate decompiled
  code into specs or source. Functional interop constants (tables, magic
  numbers) are fine, with their source address recorded.
- No cheats, no bypassing server-side anti-cheat. Passing the connection-level
  checks a server requires of any compatible client is in scope.
- Never commit third-party binaries (`re/samp-dll/`, `re/snapshots/`, `server/`
  are gitignored). Don't add Claude attribution lines to commits.

## Layout
- `crates/samp-proto` — version-independent wire protocol (cipher, unconnected
  handshake; RakNet layer, RPCs to come).
- `crates/samp-bot` — headless test client. `cargo run -p samp-bot -- 127.0.0.1:7777`
- `docs/re/` — protocol specs; start at `docs/re/README.md` (index + ID→file
  coverage + open questions). `handshake.md` (§1 connect, §2 cipher, §3 RakNet,
  §4 auth, §5 join, §6 RPC), `rpc-ids.md`, `build-diff-r4.md`,
  `subsystems/*.md` (every RPC and sync packet, by subsystem; `gta-patches.md`
  for samp.dll's memory patches to gta_sa.exe), `data/` tables
  (`gta-patches-r4.csv`).
- `re/openmp-src/` — shallow clone of open.mp (MPL-2.0), reference for wire layouts.
  `re/omp-raknet/`, `re/omp-network/` — open.mp's RakNet fork / network lib (reference only;
  RakNet 2.52 licence is GPL/CC-BY-NC/commercial — never copy code from it).
- `re/gta/1.0-US/gta_sa.exe` — game reference binary (hashes in `re/gta/VERSIONS.md`).
- `tools/` — helper scripts; `data/` — locally generated third-party data (gitignored).
- `re/samp-dll/<version>_fv<FileVersion>_<PE date>/samp.dll` — reference builds,
  hashes in `re/samp-dll/VERSIONS.md`. Currently two 0.3.7-R4 builds
  (2019, 2021), protocol-identical.
- `re/snapshots/` — REA/Ghidra analysis snapshots of those builds.
- `server/` — local open.mp 1.6.7 server (32-bit Linux, has debug info), UDP 7777.
  Run `cd server && ./omp-server`.

## Wine environment (Linux)
`tools/run-gta.sh` starts GTA SA 1.0 US under Wine 11 in a virtual desktop
(`GTA_RES=WxH`, default 1280x720). Prefix `wine/prefix` (d3dx9 + DXVK 3.1.1),
game copy `wine/gta-sa` (Steam 3.0 data + 1.0 US exe, intro movies moved to
`movies.disabled/`). Verified: single-player runs. `wine/` is gitignored.
Never screenshot the user's desktop. Visual checks only on the isolated headless
gamescope display via `tools/e2e/lib.sh` (`e2e_headless_start`, `e2e_shot`, `e2e_key`).
E2E helpers: `tools/e2e/lib.sh` (isolated server copy in `target/e2e-server` with
the `tools/e2e/probe/e2eprobe.pwn` filterscript, control-channel client, cleanup).

## Reverse-engineering tooling
REA MCP server (`rea`, Ghidra provider) — open a DLL with `open_binary`
(+ `snapshot_path` from `re/snapshots/` to skip re-analysis).

## Status / roadmap
1. Headless network client — **done for on-foot**: cipher, cookie exchange,
   RakNet reliability (`connection.rs`, sans-IO), auth key, `0x22`,
   PlayerConnect (25), InitGame (139), class selection (128), spawn (129→52)
   and on-foot sync (207). Verified live: the server sees the bot spawned,
   state ONFOOT, walking a circle (probe filterscript logging GetPlayerPos).
   - `samp-proto::rpcs::{player,remote,vehicle,world,ui,session}`: typed
     codecs for all 160 RPC ids; `rpcs::decode` dispatches S→C ids.
     `samp-proto::sync`: all sync packets (200–212) + `policy` consts.
     `samp-proto::strcomp`: Huffman strings (verified against live open.mp).
   - Heading ↔ quaternion: h ↔ (cos h/2, 0, 0, −sin h/2) (verified live).
   - Next: vehicle driving in the bot, remaining C→S behaviour, then phase 3.
   - Auth: servers check the `0x0c` answer against a fixed 256-entry table
     (open.mp `SAMPRakNet.cpp`). It's third-party data: generate
     `data/auth_table.txt` with `tools/extract_auth_table.py` (gitignored), never commit it.
   - Our server copy has `announce: false` so tests don't hit the public list.
2. Full sync packets / RPC coverage.
3. i686 Windows DLL hooking gta_sa.exe 1.0 US (planned on Windows). Specs:
   `docs/re/subsystems/gta-patches.md` (+ annotated CSV) and
   `remote-control-{onfoot,vehicles,combat}.md`. Open decision: SA:MP's own
   assets (`SAMP/` folder: SAMP.img etc.) can't be redistributed — load them
   from an existing SA:MP install or another legal source.
