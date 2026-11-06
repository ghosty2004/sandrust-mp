# Phase 3 / UX-1 + DIFF-4 + M8a leftovers — loading screen, visual parity harness, F-keys

Branch `ui-loading` (git worktree `~/samp-rs-wt/ui2`). Acceptance: `tools/e2e/ux1.sh` and
`tools/diff/visual.sh` (lane "ui" for our client, trace lane for the original).

## Tooling (new)

* **Boot D3D logging** (`samp-tracer`, `win/d3d.rs`): `SAMP_TRACER_D3D_BOOT="MODE FRAMES"` arms
  a helper thread that waits for a device in `_RwD3DDevice` and points its vtable's Present
  slot at a one-shot bootstrap. On the game thread the bootstrap puts the slot back, installs the
  normal hooks and starts logging from the first Present (the loading screen), long before the
  first `CGame::Process`. With `all`, logging drops to `ui` when `gGameState` reaches 9. Every
  change of `gGameState` is written as a `game_state` record. When a client loads `d3dx9_NN.dll`
  after the first Present, the D3DX hooks are installed late (every 16th Present).
* `tools/trace/lib.sh` / `with-lock.sh`: `SAMP_RS_ROOT` names the checkout that holds `wine/`,
  `data/` and the lane locks, so a worktree runs its own tracer build in the shared trace lane
  under the same lock.
* **`tools/diff/visual.sh`** (DIFF-4, F4; generalises `tools/e2e/m8a_d3d_diff.py`): one scenario
  (`tools/diff/visual-scenario.sh`) driven on both clients with the same server filterscript
  (`m8afs.pwn`) and the same player name. Primitives (`fs`, `key`, `kdown`, `move`, `click`,
  `typ`, `clip`, `cap NAME MODE`, `bot`) act on whichever client runs. Boot (D3D log + screenshot
  timeline up to the class request, `mark boot-end`), class selection (`cap class all`), then 23
  UI elements. `tools/diff/visual_diff.py` diffs each element (`ui`: from samp's first sprite
  call; `all2d`: XYZRHW draws + D3DX calls; `geo`: positions/digits dropped; `boot`:
  `boot_diff.py`). Lines covered by `tools/diff/visual-decisions.tsv` (each row names its ROADMAP
  decision) are reported but not counted. Exit 0 only when every element is clean.
* `tools/diff/boot_diff.py`: phase 1 (gGameState < 9) and phase 2 (UI pass) compared as runs of
  identical Presents (run lengths are timing). Normalised: the loading bar's x coordinates (its
  denominator depends on loading speed, measured 110 vs 109 steps in two runs of one build), DT_CALCRECT
  results reduced to their size, glyph-atlas texture coordinates, GTA's own out-of-scene calls
  (streaming texture creation, render-target/camera setup), the number-plate painter (decision
  below), and a final run cut by the end of the logged window.
* `tools/e2e/ux1.sh` + `ux1_shots.py`: boot diff plus same-moment screenshots (loading screen
  during the bar, excluding the bar band; class selection), mean absolute difference ≤ 6.
* `tools/e2e/ux1-orig.sh`, `tools/e2e/m8a-orig3.sh`: original-side measurement sessions.

## Measured on the original (trace lane, sessions orig-20261104-235122, orig-20261104-235538, visual/orig)

* **Loading screen:** with the archive's `loadscs.txd` and rows 122/123/129 (already applied in
  archive mode by SAA-2) the original and our client produce the same 582 Presents up to
  gGameState 9 (gs 5 → 7 at Present 52, 8 at 56, 9 at 583, in both). samp draws nothing of its
  own during loading. **UX-1's loading screen was already 1:1**; the differences were after it.
* **First UI frame** (Present 614, game frame 33, inside GTA's scene): samp's construction order
  (A 0x100c47d0): ShowCursor(0); font renderer (4 fonts, sprite, Arial 38/10 plate font, "Y"
  measures with count −1); chat window (2 sprites, 2048×1024 RT, "Y" and `[19:58:34]` measures);
  chat input → GUI resource manager sprite + its `OnResetDevice` + `CreateStateBlock(D3DSBT_ALL)`;
  death window (sprite, names font + sample measure, GTAWEAPON3 22/26 + two "G" measures with
  0x504 on the 26 font); the `logo.png` texture from samp.saa (hash 0xBADDEA6B) with a recorded
  state block, twice (B 0x10070c20); nametag and 3D-text sprites; object-editor SAMPAUX3 22/28;
  plate render target 128×32 X8R8G8B8; two more sprites; `sampgui.png` (D3DX_DEFAULT mip levels)
  and the GUI fonts 20/18; the `mouse.png` cursor. The input-lock restore's ShowCursor(0) closes
  the frame. samp's netgame object is created on the next game frame, so bubbles/3D labels start
  one frame later; the nametag passes start after InitGame (the show-nametags setting is 0 until
  then; the nametag painter creates its state block on its first pass, measured Present 765).
* **State blocks:** samp brackets its DONOTSAVESTATE sprite passes with D3DSBT_ALL state blocks
  (GUI resource manager: Capture before / Apply after every dialog render; nametag painter
  A 0x1006d6b0 / 0x1006cd50). Without them the sprite's PROJECTION, ZENABLE 0 etc. stay set while
  RenderWare's state cache assumes its own values: the world was drawn with the ortho matrix
  (black world, only RHW clouds — the class-selection bug; probably also the "row 165" black
  world in ROADMAP, worth re-testing).
* **Input lock (setter A 0x100a0750):** idle e8/e8/85/e9 (keyboard call, mouse call, branch,
  RsMouseSetPos); dialog, SelectTextDraw, F1 help, class selection: mode 2 (90/90/33/c3); ESC
  restores in the same check. **EditObject and SelectObject use mode 3** (keyboard call intact,
  mouse locked); holding Space while editing switches to mode 0 (flag 1) and back. The mouse-store
  operands 0x53F47A/9A/B3 never change. Static: modes 1 and 4 have no caller in either R4 build
  (every `call 0x100a0750` passes 0, 2 or 3). Modelled anyway (`InputLock::set_mode`), plus the
  restore's operand rewrite (A 0x100a0590) and the editor start's immediate restore
  (A 0x10072480 → `restore_now`). API for the world-entity flow: `ui::set_input_mode(mode, flag)`,
  `ui::restore_input_now(flag)`.
* **F1:** `SA-MP Keys:` local dialog through the normal show path (closes the chat input); Enter
  sends nothing. **F5:** `Client Network Stats` in 0xFF8888EE at (10,10)–(W−150,10+h), then the
  `\n`-separated lines in white, each with its own sprite Begin/End and 4-way outline; the text is
  measured whole (CALCRECT) and "Y" first. Labels in order: rates, the pool block (Vehicles,
  Vehicle Models, Vehicle Models Loaded, RWObjects, PedSlotsUsed, ObjectSlotsUsed, ShotCompMode,
  FreeIDEs), OnFoot/InCar Send Rate, Streaming Mem, Process Mem, then RakNet's statistics block
  (Messages in Send buffer … Duplicate acks received). **F10:** HUD off, nothing drawn.
  **F8:** next Present: desktop-sized A8R8G8B8 scratch surface, GetFrontBufferData, client rect
  in screen coordinates saved as PNG to `<user files>\SAMP\screens\sa-mp-%03i.png` (first free
  index; measured 000 then 001), chat line `Screenshot Taken - sa-mp-NNN.png` (type 4); the
  surface is never released (samp). `SAMP\screens` is created at start-up when missing.
* **Clipboard:** Ctrl+V pastes the clipboard into the chat input; Shift+Home then Ctrl+C copies
  the selection (measured with the X clipboard on the headless display). DXUT semantics:
  copy only with a selection, CF_UNICODETEXT, Shift+Insert / Ctrl+Insert too.
* **`mdlNNN:` sprites:** samp only special-cases them when its custom-model store
  (A 0x1026ea10) exists, which only the dormant RPC 179 handler creates; in R4 they take the
  generic TXD path (measured: identical `mdl` element).
* **Remote driver nametag** (MR-M8A-5): `samp-bot --mode drive --speed 0` placed next to a car
  becomes the driver (state 1→2); the original draws its tag. A driver (samp state 0x13) with a
  vehicle is anchored at the vehicle's position and skips the moving-list test (A 0x10074270).
  F4 (2026-11-06): the 248-line `vehtag` difference was timing, not the anchor: the killed Bot1
  timed out on the server (~14 s) during the capture window, so it vanished between frames 1 and 2
  on the original and only at frame 3 on ours. `killbots` now waits for the server's `[part]` line
  and the capture waits 8 s after state 1→2 (the get-in animation). Measured with `remote_probe`
  (`vehtag-remote.txt`): the remote vehicle stands at 1696.000,−2591.13,13.274 on both clients
  before and after the capture, and the call streams match (tag, bars, LOS, per frame).

## Implemented (samp-client)

`ui/render.rs` (first-frame construction in samp's order, run from the game frame; state
blocks; F5 draw; frame gating for nametag/bubble passes), `ui/glue.rs` (game-frame construction,
class-selection lock from the game frame, F1/F5/F10 polling as samp (`GetAsyncKeyState != 0`, F5
only when connected), F8 screenshot, clipboard, input-lock API, driver anchor),
`ui/d3d.rs` (state blocks, recorded state block, texture from memory, screenshot, NUL-count
measures, `RenderTarget` with format), `ui/dxut.rs` (resource-manager state block),
`ui/core.rs` (screenshot naming, F1, net-stats text, clipboard requests), `ui/inputlock.rs`
(all setter modes), `ui/textdraws.rs` (`mdl` names), `filehook.rs` (archive read by hash),
`dll.rs` (d3dx9_25 + fonts loaded first thing in the init thread, before the first Present),
`samp-ui` (`TextMeasure::timestamp_width`).

## Open / decisions

* Number plates (rows 59/60; samp's vehicle creation renders `XYZSR998`/the server plate into its
  128×32 target and copies it into a 64×32 texture): not reproduced; the boot diff leaves the
  plate painter's calls out (decision logged). Belongs with the vehicle owner.
* Model-preview textdraws (font 5): samp creates a GTA ped/vehicle/object, renders it with an RW
  camera into a 256×256 raster (A 0x100b34e0 → 0x1006c140/0x1006c3c0/0x1006c9b0) — not
  implemented in this step (decision logged; follow-up task).
* F5 values (A 0x10060d20, measured 2026-11-06): rates = byte deltas of the connection totals
  sampled while the overlay draws, once > 1000 ms passed, / 1024; pool block = vehicles in the
  pool, distinct models among those with a GTA vehicle, GTA vehicle models 400…610 with a
  non-zero streaming load state, GTA's RW object counter (0xB71804), remote ped slots in use
  (2…209), objects in the pool, the lag-comp selector (InitGame 1 → 0, 2 → 1, else 2), null
  model-info slots among ids 1000…19999; network block = RakNet's statistics, from our
  reliability layer (`samp_proto::connection::Stats`). Labels, layout and colours match; DIFF-4
  compares the element without digits.
* D3DX glyph-atlas batch splits (two DIPs vs one) depend on each font's glyph-cache history; the
  glyph draws themselves match — covered rows in `visual-decisions.tsv`.
* IME composition is not modelled (no IME on the test display).
