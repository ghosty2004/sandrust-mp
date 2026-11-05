# M6-P — world and vehicle parity follow-ups (implementation notes)

Branch `world-leftovers` (worktree `samp-rs-wt/m6p`). Static details come from samp.dll **B** (2019 build, the one
the lanes run), read with objdump / REA; every behaviour was measured on the original first.

Measurement runs:
* original: `target/trace/orig-20261105-042300` (trace lane, `run.sh orig hold`, driven with `ui.sh`; marks
  `objedit*`, `drag-begin`, `rotdrag`, `save`, `attedit`, `attdrag*`, `attsave`, `objsel`, `selclick`, `cam81*`,
  `plate`, `plate2`, `fenter`, `trainexit`);
* ours: `target/vehroot/target/trace/ours-20261105-044503`, `…-045448`, `…-045944` (lane veh through
  `tools/e2e/m6p-hold.sh`, the same commands).

## 1. Object editor, attached-object editor, selection (RPC 27/28/116/117)

Files: `objedit.rs` (state, layout, input rules, unit tests), `objects.rs` (RPC flows, drag apply, 117/116/27 sends,
pick ray test), `ui/render.rs` (`editor_gizmo`, `select_highlight`, D3DX line), `ui/glue.rs` (Present step 12
`editor_pass`, `editor_message`), `ui/wndproc.rs` (mouse messages and ESC `WM_CHAR` first), `ui/d3d.rs`
(`ID3DXLine`, WORLD transform copy).

Static (B): editor object 0x1026E9D8 (ctor 0x10071B90: `D3DXCreateLine`, SAMPAUX3 22 and 28), per-frame
0x100737B0, gizmo 0x10072C60, handle 0x10072570, hover 0x100720D0, messages 0x10073580, drag 0x100734B0, object
apply 0x10073240 (MoveObject 0x100A9BE0 at speed 1 / rotation through 0x100A80B0), attached apply 0x10072240, send
0x100728E0 / 0x10072AF0, stop 0x10072BF0. Selection 0x1026E9DC: per-frame 0x1006DBC0, pick ray 0x10073F70, pool ray
test 0x10012E40, highlight 0x1006DA20 (cross 0x10071D50, 3D-text painter 0x1006BC50 with outline on and LOS off),
click 0x1006DE00 → 27 sender 0x1006DC90. Details in `objedit.rs`.

* Gizmo: per axis a 2-point antialiased `ID3DXLine` from +s to −s along the object's axis (s = camera distance ×
  0.125; colours X 0xFF9999CC, Y 0xFFCC9999, Z 0xFF99CC99), handle at the + end: background glyph `C` (SAMPAUX3 28;
  0x99222222, hovered 0xFFAAAAAA, dragged / active mode 0xFF774444) + axis glyph (SAMPAUX3 22; translate `6 7 8`,
  rotate `3 4 5`, scale `9 A B`). Buttons at the − end of Z, right to left: save `H`, [scale `1` for attached
  objects], rotate `0`, translate `2`, 2 px apart. Any end point behind the camera stops the drawing.
* Attached objects in translate mode use the bone's axes (B 0x10073930), else the object's.
* Input: cursor mode 3 with flag 1 every frame (0 while Space/sprint is held), the real device's `ShowCursor(TRUE)`;
  while dragging `ShowCursor(FALSE)`. LMB down on an axis handle starts a drag (SetCapture, user32 ShowCursor(FALSE),
  ClipCursor(NULL)); each mouse move moves by (pixels × 0.01) along x or y of the axis' screen direction (swapped in
  rotate mode) and re-centres the cursor; LMB up restores the cursor on the handle. Buttons act on LMB up. ESC as
  `WM_CHAR` cancels (the `WM_KEYDOWN` still reaches GTA: the original opens the pause menu too).
* Drag apply: translate → MoveObject(speed 1) to the edit matrix moved along its own axis; rotate → rotation copy
  ± d × 10 (wrapped to ±180), SET_OBJECT_ROTATION; then 117 response 2 if ≥ 250 ms since the last 117 (any
  response). Attached: offset ± d × 0.1, rotation ± d × 10, scale ± d × 0.1 (not below 0); no 116 until the end.
* Start (117 / 116 S→C): selection and textdraw selection off, editor on (mode translate), cursor mode 0 with an
  immediate restore. Stop: cursor mode 0 (immediate), chat input closed, final 117 / 116.
* Selection: cursor mode 3 / flag 0 (0 / 1 while sprint is held), the pick ray through the cursor (inverse of
  WORLD × VIEW, unnormalised direction, 150 m), the nearest hit object closer than 150 m to the ray start: sphere
  cast (radius-0.25 spheres, samp's own collision cache) when the object uses collision and has collision data, else
  a frustum-checked line-vs-bounding-sphere test. The hovered object gets its axis cross (length 1, 0xFFCC9999) and
  the label `[id: %d, type: %d]\nDistance: %.2fm\n` (0xFFD47B75, outlined small font). LMB up sends 27 (type 0).

Results (same inputs on both clients):
* `tools/diff/editor_d3d_diff.py ORIG ours` — **MATCH** for `objedit2` (154 calls), `hoverx`, `dragx` (154),
  `rotmode` (154), `selhover2` (179). `attedit`: only the Y handle differs (634,345 vs 644,343) — the attached object
  sits on bone 2 (head) and only that bone row differs between the two clients' poses; the gizmo code path is the
  same (X / Z handles identical). Not an editor difference (ped head pose).
* 117 response 2 while dragging the X handle: original 21 sends 1700.000 → 1702.041, ours 20 sends 1700.000 →
  1702.072 (first one at the unchanged position on both, 250–290 ms apart); rotate drag: −0.299, −1.199, −2.099,
  −2.999 on both; save: response 1 with the same values. Attached drag: no 116 during the drag on both; save →
  offset 0.060 on both. Selection click: 27 → `select-object type 1 obj 1 model 1271` on both.

Decisions: the glyph probe gets a zeroed RECT (samp passes an uninitialised one; only the size is used).

## 2. CSV row 81 — camera collision with client objects / vehicles

Static (B 0x100A5960): after `BuildCacheOfCameraCollision` (0x41AC40) found nothing, samp tests its own objects only
when the camera sphere is **outside** ±3000 in x or y (the trace notes' "inside" was a misreading of the
`fcomp/test ah` pairs): object pool (B 0x10012DB0: alive, no-camera-collision byte clear, camera distance < 200, not
`CWorld::pIgnoreEntity`, collision radius > 0, `CCollision::SphereCastVsEntity`), the second object pool (only the
`-d` DEBUGSCRIPT pool, absent online) and vehicles (B 0x1001EDD0: in the moving list, < 300, skipped while RPC 167
disabled collisions). Implemented in `worldhooks.rs` (`build_cache_of_camera_collision`).

Measured (mark `cam81`): player in the water at (3100, −2600), wall 19353 behind him: camera y −2603.33 → −2600.57
with the wall; `SetObjectNoCameraCol` → −2603.33 (so GTA's own pass does not see the object there); removed →
−2603.32. Ours: −2603.33 / −2600.57 / −2603.33 / −2603.33.

## 3. Vehicles

### Number plates (rows 59/60, RPC 123)
`plates.rs` + `ui/render.rs::paint_plate` + `ui/d3d.rs` SetTexture substitution + `vehhooks::render_vehicle`. See the
module header. Painter D3D stream (from `D3DXCreateTexture 64×32` to the RTS `EndScene`, glyph quads excluded):
**MATCH** (32 calls) after drawing every pass through the font renderer's colour proxy (measured: the DrawTexts
carry the proxy, not the sprite). Screenshot `p-plate3.png` identical to the original's (RUST 42 on GTA's plate).

### Train exit
Measured: F in a stopped train → in the constructor hook WARP_CHAR_FROM_CAR_TO_COORD(train + (2.5, 2.5, 0)) two
frames after the key, then GTA's SetTask of the leave task; no camera call, no RPC 154. Ours warped from the next
client tick and reset the camera; now the warp runs inside `vehhooks::leave_filter` and nothing else — same order
and arguments (frames F+2 on both).

### F-enter (DIFF-3 veh_enter_B)
Per-frame poll of the local ped (move state +0x534, primary task chain; `fpoll`-style peeks through the control
port) in the same scenario on both clients (car 2.5 m north, heading 90, F): identical — task at +0.36 s, move
state 1 → 4 → 6 at +0.41 s, the go-to-door subtask (move state 6) until +0.66 s, then 4. The 118:5 approach reported
by FIX-B does not reproduce: DIFF-3 run `target/diff/runs/xclient-20261105-050140` (`tools/diff/cross-client.sh
--skip-build`, report against the new calibration `xcalib-20261105-050900`) — **45/45 step-directions MATCH**,
`veh_enter_B` MATCH (our sender plays `a38efa5f/118:0` like the original; the observer's missing `0:63` is unstable
in the original-vs-original run and not judged).

### Trailer receive (210) and aircraft remote z — measured, no change needed
`tools/e2e/m6pvehscen.sh` (samp-bot gained `--trailer ID`: trailer id in 200 + a 210 every in-car tick 9 m behind
the cab): TowBot drives a Linerunner with a trailer (AttachTrailerToVehicle), FlyBot a Dodo. Original
`target/trace/orig-20261105-051538`, ours `target/vehroot/target/trace/ours-20261105-051729`:
* trailer: 41 vs 42 × (`0x0893 ATTACH_TRAILER_TO_CAB` + `CPlaceable::SetPosn`) in the tow window; trailer positions
  frame by frame within 0.1–0.5 m (bot timing), same z;
* Dodo: z held at 26.90 vs 26.91 while the bot flies it; tracks within ~3 m (bot timing).
* Found on the way: the original runs the nametag LOS test (`0x06BD`, bars and names pass) although open.mp sends
  `DisableNameTagLOS = 0`: samp's RPC 139 handler stores 1 into settings +0x2F after reading the bit
  (B 0x10010778). Ours honoured the bit and never tested LOS; now always on (`ui/core.rs`).

### Remote passenger drive-by
The start (weapon + `0x0713 TASK_DRIVE_BY`) is implemented since M7 and matches TRACE-7; firing stays unmeasured
(samp-bot cannot fire as a passenger).

## Tests and tools
* `tools/e2e/m6.sh` (+ `m6scen.sh` blocks `editor`, `cam81`, `plates`, `train`): editor drag/rotate/save (117
  updates ≥ 5, saved position/rotation), attached editor (116 only at the end, offset 0.060), selection (27), camera
  outside ±3000 with / without the wall / with no camera collision, plate textures, train exit warp.
  `tools/e2e/m5.sh`: plate textures of car A (m5helper sets `M5 RUST`).
* `tools/diff/editor_d3d_diff.py`: editor / selection D3D pass diff (normalisation in its header).
* `tools/e2e/m6p-hold.sh`: interactive hold session of our client in lane veh with this worktree's trace tools.
* `tools/trace/uifs.pwn`: `plate|VEH|text`.

## Verification (lane veh, this branch)
* e2e: m1, saa, m2, m3, goal, m4, m5 (+ plates), m6 (+ editor / cam81 / plates / train), m7, m8a, m9 (with the
  original BASS reference copied from main's target/m9-orig), m10 exit 0 (m10 failed once on its cfg/kick
  timing, passed on the rerun). m8b: the world-bounds walk step fails intermittently on this base — with
  `M8B_ONLY=bounds` our build passed and the unmodified base commit 6dbbfb3 failed (walked through maxX 1720); main
  has since merged samp's world-bound jump (102d456), which this branch does not contain.
* 569 tests; clippy host (all targets) and i686 0 warnings.
