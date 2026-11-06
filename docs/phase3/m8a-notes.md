# Phase 3 / M8a + M4-P — dialogs, textdraws, menus, scoreboard, nametags, death window, bubbles; UI parity

Branch `ui-dialogs` (git worktree `~/samp-rs-wt/m8a`). Acceptance: `tools/e2e/m8a.sh` (lane "ui").
D3D parity: `M8A_D3D=1 tools/e2e/m8a.sh` + `tools/e2e/m8a_d3d_diff.py` against the original's
TRACE-3 logs; original-side session for the elements TRACE-3 did not log: `tools/e2e/m8a-orig.sh`.

## What is implemented

| area | where | source |
|---|---|---|
| Dialogs, all 6 styles: layout once per open, DXUT-style render (caption, buttons, list box with tablist columns, edit box focused last), body/headers through the font renderer; keys (ESC, Enter, list keys, edit keys, typing), mouse (buttons, rows; second click on the selected row activates); RPC 62 | `samp_ui::dialog`, `ui/render.rs` `dialog`, `ui/dxut.rs`, `ui/core.rs` | UR §13, UCA §61/62, TRACE-3 |
| Textdraws: pool 2304, set-text rules, `CFont` call sequence after `Render2dStuff`, hit rects, selection (hover colour, click → RPC 83 id, ESC/RPC 83 enable 0 → 0xFFFF, cursor mode 2) | `samp_ui::textdraw`, `ui/textdraws.rs` | UR §14, UCA §83/105/134/135, samp A 0x100b31e0/0x100b2fa0 (static) |
| Menus: RPC 76/77/78, GTA menu via 0x08D4/0x08DB/0x09DB/0x08D9, `CText::Get` hook (CSV row 58: `jmp [0x6A0043]`) answering `SAMPHED`/`SAMPRW1/2`/`SAMP0rr`/`SAMP1rr`, per-frame pad poll → RPC 132/140, local hide one frame later | `samp_ui::menu`, `ui/menus.rs` | UCA §76–78/132/140, samp A 0x100a7780/0x100a4070/0x10008060 (static) |
| Scoreboard: Tab, cursor mode 3, HUD off, DXUT list (selection on row 0, selected row white), texts with outline, RPC 155 on open + every 3000 ms | `samp_ui::scoreboard`, `ui/render.rs` `scoreboard` | UR §15, TRACE-3 |
| Nametags: server flags (show, distance, LOS 0x06BD twice per tag), bone-8 anchor + `0.0475·d + 0.2`, D3DXVec3Project with the VIEW/PROJECTION recorded by the device proxy's SetTransform (samp's proxy slot 44), `_ftol2` truncation, bars (FVF 0x44 fans), name (measure, 4 outlines, main), AFK icon (SAMPAUX3 20/22, lazily created) | `samp_ui::nametags`, `ui/glue.rs` `nametags`, `ui/render.rs` | UR §16, TRACE-3, samp A 0x10074270/0x1006d0a0 (static) |
| Death window: RPC 55 names/colours, 5 slots newest at the bottom, `ftol(0.75·W)`, `ftol(0.3·H)`, nick/icon metrics, killer rect `[x+nickW−w, y, x+60, y+30]`, icon `G` + CALCRECT + glyph, victim at `+iconW` | `ui/render.rs` `death_window` | UR §17, samp A 0x1006a2e0/0x10069d40 (static), TRACE-3 |
| Chat bubbles: RPC 59, wrap, head bone anchor, distance to the local ped, small font `DT_CENTER` | `samp_ui::bubbles`, `ui/glue.rs` `bubbles` | UR §11 |
| Chat render-to-texture (M4-P): `D3DXCreateTexture(2048,1024,RT)` + `D3DXCreateRenderToSurface(…, D16)`, updated from the `CGame::Process` hook only when dirty/scrolled (`Clear(3)`, sprite `Begin(0x10)`, `DESTBLEND 14`, lines, End), blitted in Present (`Begin(0x10)`, `Draw(NULL…)`) | `ui/render.rs` `update_chat_rt`/`chat`, `dll.rs` | UR §7.6, TRACE-3 |
| Frame order (M4-P): netgame tick + UI pre-step → `Render2dStuff` → textdraws → input restore; HUD flags written every Present (`!scoreboard && mode ≠ 0`) | `dll.rs` `frame_hook`, `ui/glue.rs` | UR §2.3/§2.4, TRACE-3 U-HUD-STEP9 |
| Connect timing (M4-P): "Connecting to" 3.0 s after "Started" and the network starts then; retry line + new "Connecting to" every 6.05 s while unanswered | `ui/core.rs` `connect_tick`, `client.rs` | TRACE-3 U-CONNECT-TIMING |
| Input/edit (M4-P): password mask U+25CF (DrawTextW), DXUT first-visible scrolling (`PlaceCaret`, GDI widths), key consumption (no key-down/char reaches GTA while typing; every key-up does), U-LOCK-INIT restore at start-up, `/fontsize` rebuilds GUI fonts at once | `samp_ui::edit`, `samp_ui::keys`, `ui/inputlock.rs`, `ui/render.rs` | TRACE-3 |
| D3D call shape: per-painter sprites (font renderer, chat, blit, GUI, death, tags, 3D text), colour proxy `Draw` that tail-jumps into the real `Draw` (D3DX stays the caller), 4-way outlines move only left/top (8-way whole rect), DXUT `DrawSprite`/`DrawText`/`DrawRect` sequences, proxy `EndScene` wrapper, d3dx9_25 + `AddFontResourceA` at start-up | `ui/d3d.rs`, `ui/dxut.rs` | TRACE-3 D3D logs |
| Class-selection panel (GUI dialog 0x1026eaa4): 310×40 at `y = H − 90`, `<<` `>>` `Spawn` buttons (clicks act like Left/Right/Shift), cursor mode 2 while shown, drawn after the chat GUI dialog | `ui/render.rs` `class_panel`, `ui/glue.rs`, `client.rs` | UR §4.2, TRACE-3 class.png |
| Control: `ui` (dialog, scoreboard, textdraws, selection, menu, deaths, bubbles, cursor mode, remotes) | `control.rs`, `ui/core.rs` | — |

Changes outside `ui/`: `dll.rs` (frame order, game-frame hook calls the RT update, `ui` command, preload),
`client.rs` (connect gate, remotes/class-selection to the UI, `on_init_game(&InitGame)`), `players.rs`
(`ui_snapshot`), `control.rs`, `samp-proto::session::InitGame` derives `Default`, `tools/run-client.sh`
(`SAMP_RS_PRE_DLL`).

## Static analysis used (§3b step 1; samp.dll A build, objdump)

* Textdraw set-text rules (A 0x100b2fa0) and hit rects (A 0x100b31e0 tail, A 0x100b30d0): left = x .. wrap,
  height = centre size; centred = x − cs/2 .. +cs, height = wrap (the known swap); right = 2x − wrap .. x.
* Menu build order, keys and the `CText::Get` hook; both menu columns get col1W as width (samp passes +0x384
  twice: reproduced).
* Death window draw (A 0x1006a2e0): truncation, 5 fixed slots, rect values (killer right edge `x + 60`).
* Nametag projection (A 0x1006d0a0): D3DXVec3Project with samp's copies of VIEW/PROJECTION from proxy
  slot 44; z ≤ 1; `_ftol2`; LOS 0x06BD from bone 8 to the camera with flags (1,0,0,1,0); distance helper
  A 0x1009f1f0 = ped position − camera (0xB6F9CC).

## Measured on the original (this work, trace lane, `tools/e2e/m8a-orig.sh`, session orig-20261104-230225)

* MR-M8A-3 list mouse: a click selects; a second single click 1.5 s later does nothing; a double click
  answers with that row (`listitem 2 'Row 2 green'`). Implemented with the system double-click time.
* MR-M8A-4 glyph boxes: the original is deterministic (the `a` of "Cancel" has the same box in TRACE-3 and
  in this run); ours differs by 1 px for that glyph → an open residual, not noise (see below).
* Textdraw selection: HUD flags are 0/1 while selecting (samp's per-frame A 0x10071470 = cursor mode 2 +
  HUD off) — implemented.
* Same-scenario D3D comparison (ours `M8A_D3D=1` vs this run): select, death window, nametags+bubble+AFK
  measure, scoreboard, menu: identical call structure (`m8a_d3d_diff.py --same`: 0 lines); textdraw and
  menu 2D draws (box quads, glyph batches, positions, colours) identical; menu script commands
  (0x08D4/08DB/09DB/08D9/08D7/08DA/00BE) identical in `trace.jsonl`.
* Part 2 (`tools/e2e/m8a-orig2.sh`, session orig-20261104-231716): MR-M8A-1 sprite textdraw
  `LD_SPAC:white` 100×60 at (500,20): one textured quad (8×8 texture) 1000,32.1–1200,128.6 white — ours
  identical; its selection frame diffs to 0 lines and the click is reported. MR-M8A-2 model preview
  (model 411, 100×100 at (300,200)): the original draws a 256×256 render-target texture (A8R8G8B8,
  RT usage) as one white quad 600,321.4–800,482.1 — not implemented (needs the off-screen model render,
  TODO). MR-M8A-5 (remote driver nametag): not measured — samp-bot did not become a driver in that
  session; still open (belongs with M5's remote vehicles).
* Static: A 0x1009f430 is not an on-screen test but "ped valid and in the moving list" (+0xB4 ≠ 0).

## Results

* `tools/e2e/m8a.sh`: PASS (dialogs 0–5 incl. mouse, textdraws + selection, menu select/exit, death window,
  bot nametag + bubble, scoreboard, game text; capture: RPC 62/83/132/140/155 as specified).
  RPC 62 payloads are byte-identical to the original's TRACE-3 capture for all six dialogs.
* D3D diff (`m8a_d3d_diff.py`, frame 1, normalised): input 0 lines; dlg2/dlg3 0; dlg0/1/4/5 4 lines each =
  one glyph (`a` of "Cancel"/"OK" row) whose D3DX glyph box is 1 px taller in our process (atlas
  rasterisation; not caused by our calls). Scoreboard / nametags / death window: same call structure,
  differences are scenario content (names, ids, number of bots) — see the final report.
* m1 (headless), m2, m3, m4 pass in lane ui.

## Mouse cursor (CURSOR-1, 2026-11-06)

* User report (lane main): no mouse cursor with the chat input, a dialog or the scoreboard open, although
  both clients issue the same D3D cursor calls (one `SetCursorProperties`, the same `ShowCursor(1)` count).
* Measurement: gamescope screenshots do not contain the cursor. `tools/e2e/xcursor.py` reads the X cursor
  of the isolated headless display (XFixes). Trace lane hold sessions, mouse moved with xdotool:
  original — input / dialog / scoreboard: 32×32 arrow (438 opaque px, sha a918be417195), hidden (1×1)
  when closed; ours before the fix — 1×1 (hidden) in every state.
* DXVK 3.1.1 turns a ≤ 32×32 cursor into a Win32 cursor (`CreateIconIndirect`, `SetCursor`).
  `WINEDEBUG=+cursor`: ours made `NtUserShowCursor(0)` twice before our window subclass existed
  (count −2: GTA's window procedure on activation, 0x747FE9, and on `WM_SETCURSOR`, 0x7481CD), so every
  later `SetCursor(mouse cursor)` stayed invisible; the original made none. Cause: samp inline-hooks
  `user32!ShowCursor` at start-up with a counter-neutral stub (UR §5; seen live: the export starts with a
  jump into samp.dll+0x62920, B build).
* Fix: `crates/samp-client/src/cursorhook.rs` redirects gta_sa.exe's `ShowCursor` import to the same stub at
  attach (deviation: import slot instead of export hook, ROADMAP "Decisions"). After the fix ours shows the
  identical 32×32 image in the same three states, hidden when closed, and makes no `NtUserShowCursor` call.
  The scoreboard opens on the Tab key-up in both clients.
* Lane main (Wine virtual desktop through tools/run-client.sh) has the same cause: GTA's activation runs
  before any subclass in every setup.

## Deviations / TODO-unknown

* Sprite textdraws (font 4) follow samp's loader and match the original's draw; `mdlNNN` sprites and
  model previews (font 5, A 0x100b34e0: 256×256 RT quad measured) are not drawn (TODO).
* Nametags of drivers: resolved 2026-11-06, see "Remote driver nametag (MR-M8A-5)" below.
* Resolved: the 1-px glyph difference of GUI font 0 (`a` of "Cancel") came from the missing
  class-selection panel — the original caches `a` first through its `Spawn` button; with the panel
  drawn, dlg0–dlg5 diff to 0 lines.
* F1 help, F5 net stats, F8 screenshot, F10, IME/clipboard, `directmode` config are not done here.
* Slider panel / download progress / class-selection GUI dialogs (DXUT) are not drawn.
* The object-editor SAMPAUX3 22/28 fonts created at start are not created (no visible effect).
* The original passes an uninitialised rect / stale colour to its DT_CALCRECT calls; ours pass zeros (the
  diff ignores those arguments).

## Remote driver nametag (MR-M8A-5, 2026-11-06)

User report: as the passenger of a car, the driver's nametag sat low ("below the vehicle"). Ours anchored
every remote in samp state 0x13 (driver) at the vehicle's matrix position.

* Static (A = 2021 build): A 0x10074270 (bars) and A 0x10075390 (names) take the vehicle path only when
  state is 0x13, the remote's vehicle (+0x1e1) is set and A 0x100b7e40 passes. A 0x100b7e40 = valid pool
  handle (GTA `CPool::GetAt` 0x4048E0) **and** model 441/464/465/501/564/594 — RC vehicles only. Every other
  driver goes through A 0x1009f430 (ped valid and `CPhysical` +0xB4 ≠ 0) and bone 8 (head), like a
  pedestrian. The vehicle path reads the matrix position (A 0x1009eb70 copies the matrix; +0x30); no extra
  offset; the raise is `0.0475·d + 0.2` with d = ped-to-camera (0xB6F9CC) for every path.
* Dynamic (`tools/diff/vehtag.sh` = `vehtag-measure.sh` through visual.sh; samp-bot drives an Infernus at
  speed 0; fixed cameras 4/8/16 m east of the car via new m8afs `campos`/`camlook`; then the local player
  as that car's passenger; viewport 1280×720). On the original, samp's anchor global A 0x10140bdc read
  after the frame was (1695.517, y, 13.569) = the driver's head (ped root 13.065, car matrix 13.274), and
  projecting it with samp's own VIEW/PROJ copies (A 0x10140be8/0x10140c28) reproduces the bars (e.g.
  passenger 580,328 vs bars 581,327). Bar origins (X, Y) — original / ours after the fix:
  foot4 780,317 / 792,318; foot8 715,325 / 732,325; foot16 695,327 / 717,327; passenger 581,327 / 582,327
  (twice). The anchor height recovered from Y with each side's VIEW/PROJ (tracer `xform`): original
  +0.43…+0.52 m above the ped root, ours +0.50…+0.52 m (tolerance 0.08 m; the vehicle anchor would be
  0.21 m). X wanders with the car (the remote car rocks ±0.3–0.5 m laterally between the probes; the
  original alone gave foot4 X 804 and 780 in two runs), so X is not compared 1:1.
* Ours before the fix (same scenario, measured, target/vehtag-before): passenger 639,352 (twice) instead
  of 581,327 — 25 px lower and 58 px towards the car's centre; foot4 839,376 (59 px lower), foot8 774,352,
  foot16 690,340; recovered anchor = the car's matrix position (−0.10…+0.02 m), +0.11…+0.23 m above the
  ped root. `vehtag_eval.py --check` FAILs all 5 captures on that build and PASSes all 5 after the fix.
* Fix: `vehicle::tag_at_vehicle(state, model)` (RC models only), used by `ui/glue.rs` `nametags`.
* Permanent check: `tools/diff/vehtag.sh` (exit 0 = PASS; `--check` re-evaluates existing captures).
  The old `vehtag` element of visual-scenario.sh is a geo element whose tag is off-screen (X ≈ 1700 at
  W 1280) — it never compared a position.
* Not measured: an RC driver's tag (vehicle path) — from static analysis only.
