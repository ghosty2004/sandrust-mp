# Phase 3 / M1 — launcher, client DLL skeleton, boot into gameplay

Status: `tools/e2e/m1.sh` passes (Wine 11, gta_sa.exe 1.0 US HOODLUM build in `wine/gta-sa`).
Typical run: state 8 after ~1 s, state 9 (IDLE) with the player ped at Grove Street
(2488.6, -1666.9, 13.3) after ~12 s, first frame tick ~2 s later, ~60 fps.

## Pieces
* `crates/samp-launcher` — `CreateProcessW(CREATE_SUSPENDED)` of gta_sa.exe with
  `-c -n NAME -h HOST -p PORT`, current dir = game dir; writes the DLL path into the child
  (`VirtualAllocEx`/`WriteProcessMemory`), runs `LoadLibraryW` there with
  `CreateRemoteThread`, waits for it (60 s), checks the returned module, then resumes the
  main thread and waits for the game to exit (`--no-wait` to return at once). Log:
  `samp-rs-launcher.log` next to gta_sa.exe. The original `samp.exe` is packed; its import
  table only showed `LoadLibraryA` by name, so the remote-thread method is the standard one,
  not derived from its code.
* `crates/samp-client` (cdylib, `samp_client.dll`):
  * `DllMain` (under loader lock, main thread still suspended): log file `samp-rs.log`,
    panic hook, version probe (0x747483 == 0x89 else refuse and patch nothing), early patch
    block, start `samp-init` thread.
  * `samp-init`: polls `gGameState` (5 ms) until 7, applies the main block, retargets the
    `Render2dStuff` call at 0x53EB12 to `frame_hook`, binds the control channel, writes the
    three FrontEndMenuManager flags and `gGameState := 8`; then services the control channel
    whenever the game has not ticked for 1 s (loading, menus).
  * `frame_hook` (cdecl, no args): calls the original `Render2dStuff`, then `tick()` inside
    `catch_unwind`: tick counter, one log line per second with fps and player position,
    control-channel servicing. First tick: `RsGlobal.frameLimit := 200` (CSV row 357).
  * Control channel `127.0.0.1:47700`: `ping`, `state`, `quit`, `peek ADDR LEN`.
    `state` = `gGameState`, tick count, `CWorld::Players[0].m_pPed` position (matrix pos or
    placement), heading (`+0x558`), health (`+0x540`), armour (`+0x548`), and whether the
    frame hook is still in place. `quit` sets `RsGlobal.quit` (0xC17050); the game exits
    cleanly (exit code 0); a watchdog terminates the process if it is still alive after 8 s.
  * Patching module (`patch.rs`): rel32 math, call/jmp hooks, branch retarget, pointer
    swap, byte/NOP writes with expected-original checks and restore; host unit tests use a
    fake address space. `patches.rs` tests every table entry against the annotated CSV and,
    when present, against `re/gta/1.0-US/gta_sa.exe`.

## Patches applied (CSV = line in `docs/re/data/gta-patches-r4-annotated.csv`)
Early (at attach): row 120 `gGameState := 5`; 121 NOP psInitialize state reset; 124/125
video-mode aspect filters; 126 CTimer millisecond path; 127 streaming budget from RAM; 129
RenderSplash DrawRect NOP; 130 Idle frame limiter 2 ms.
Main (at state 7): 160 no `CGame::InitAfterLostFocus`; 209 non-exclusive mouse; 6 the
per-frame call retarget; 378 menu flags; 377 `gGameState := 8`. Runtime: 357 frameLimit.

## Deviations from the spec / samp
* Not applied on purpose: row 128 `CStreaming::InitImageList := RET` (samp then registers the
  image archives itself, including SAMP.img; we don't yet, so the game must keep doing it);
  rows 122/123 splash texture names → "title" (the stock txds have no `title` texture; samp
  supplies its own splash); row 131 TxdStore pool size (pool resizing is a later milestone);
  everything else in init-main (pools, relocations, hooks, single-player removals).
* No polling for decoded code: this exe is not packed (the three "wait until byte" sites
  already hold their values in the file, see gta-patches.md), so the probe is read once.
* **Extra fix, not in samp:** lost-wakeup race in the game's streaming code. About 1 boot in
  6–8 froze in state 8: main thread in `CdStreamSync` (0x406460 → body 0x156CD80) waiting
  `INFINITE` on the channel semaphore, `CdStreamThread` (0x406560) idle (winedbg backtraces).
  The reader clears "sectors to read", then releases the semaphore only if "locked" is set;
  the sync side tests "sectors to read" and only afterwards sets "locked" → lost release.
  We replace the `call [WaitForSingleObject]` at 0x156CDAE (`FF 15 88 80 85 00`) with a call
  to our wait that waits in 5 ms slices and returns once the channel is idle
  (`nSectorsToRead == 0 && !bInUse`), consuming a late release with a zero-timeout wait.
  A rescue is logged as `cdstream: lost wake-up …`. After the fix: 8/8 + 3/3 e2e boots OK
  (the race was not hit in those runs, so the rescue path itself is still unobserved).

## Observations
* New game runs the stock `main.scm`: the player starts at Grove Street and a few seconds later
  the intro script moves him to ~(2239.7, -1261.9, 23.9). samp's way to keep the story script
  from running is not identified yet (candidates: the unanalysed init-main calls 0x100b2590 /
  0x1009cf60, or script handling in the first-frame init).
* `state` may report `ticks=0` for ~2 s after `game_state=9`: `gGameState` becomes 9 before the
  first `Idle` frame reaches 0x53EB12. The hook stays in place (`peek 0x53EB12 5` →
  `e8 <rel32 to samp_client>`), and ticks start right after.
* One early run took 55 s to reach the player position (cause not captured; not reproduced in
  11 later runs).

## For M2 (network join + local player sync from inside the game)
* Run the samp-proto connection (sans-IO) from `tick()` with a non-blocking UDP socket; the name,
  host and port are already parsed from the command line (`cmdline.rs`, logged at attach).
* Keep the story script from running and the player from being controlled by it; implement
  spawn (RPC 129 → 52) by placing the ped (spawn path rows 365/385) and class selection.
* Build on-foot sync (207) from the ped: position/quaternion/velocity/keys (CPad), health and
  armour.
* Image list (row 128) + SAMP.img/SAMP.ide loading once samp models are needed; then the pool,
  sector and model-store relocations and the remote-player safety NOPs before any remote ped.
