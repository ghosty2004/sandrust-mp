# UI rendering and input (SA:MP 0.3.7-R4 client)

Clean-room spec of **how** the client draws and drives its UI: the hooks that run the UI, the
fonts it creates, layout and colours of each element, the frame draw order, key and mouse
handling, and which settings persist. Wire formats and the data model of chat, dialogs,
textdraws and menus are in `ui-chat-audio.md`. This file does not repeat them; it refers to
sections there as "UCA §n". Inputs for milestones M4 (chat, input, local commands, game text,
client messages, bubbles) and M8a (dialogs, textdraws incl. selection, menus, scoreboard,
nametags and bars, death window).

Conventions:
- **A** = 2021 build (primary), **B** = 2019 build. Addresses are samp.dll VAs unless they
  start with `0x5…`–`0xC…` (gta_sa.exe 1.0 US). GTA names come from `re/gta-reversed`.
- Labels: **[O]** observed in the code (decompiler output or disassembly), **[I]** inferred,
  **[U]** unknown or not traced.
- Colours are D3DCOLOR `0xAARRGGBB` unless stated.
- "Font renderer" = samp's text helper object (global A 0x1026ea14). "Small font" and
  "main font" are its two sizes (§3.2).
- Sources: decompiled A and B dumps, objdump of both DLLs, gta_sa.exe 1.0 US and the
  SA:MP install files in `wine/gta-sa/`. The game was not run.
- A and B: every function listed in §20 was compared after normalising addresses. They are
  identical except where they touch pool or netgame struct offsets, which moved between the
  builds (see `build-diff-r4.md`). The UI behaviour is the same in both builds. The global
  object pointers (A 0x1026e9b8–0x1026eab4) are at the **same** addresses in B.

---------------------------------------------------------------------------------------------

## 1. Architecture

The client draws UI in two passes per frame:

| pass | where it runs | what it draws | technique |
|---|---|---|---|
| GTA 2D pass | samp's per-frame entry, which replaces the call to `Render2dStuff` at 0x53EB12 in `Idle` | textdraws (text and sprites) | GTA `CFont`/`CSprite2d` (RenderWare) |
| D3D pass | samp's `IDirect3DDevice9::Present` wrapper | everything else: chat, input box, dialogs, scoreboard, nametags + bars, death window, bubbles, 3D labels, net stats, cursor-driven selection modes | D3DX fonts and sprites, DXUT-style GUI, raw `DrawPrimitiveUP` |

GTA keeps drawing game text (`CMessages`), menus (`CMenuSystem`), the HUD, the radar and the
clock itself. samp only feeds them data (UCA §73, §76–78, §30).

### 1.1 UI objects (all created once, on the first frame; A 0x100c47d0)

| global | size | ctor A / B | role | per-frame draw (A) |
|---|---|---|---|---|
| 0x1026e9b8 | — | — | the **real** `IDirect3DDevice9` (taken from 0xC97C28 before it is replaced) | — |
| 0x1026ea14 | 0x28 | 0x1006baf0 / 0x1006bab0 | font renderer (§3) | — |
| 0x1026e9f8 | 0x63ea | 0x10068380 / 0x10068340 | chat window (§7) | 0x10067e00 |
| 0x1026e9fc | 0x1afc | 0x100693d0 / 0x10069390 | chat input (§8) | 0x100696f0 |
| 0x1026ea00 | 0x157 | 0x1006a650 / 0x1006a610 | death window (§17) | 0x1006a2e0 |
| 0x1026e9c0 | 0x0c | 0x1006ccf0 / 0x1006ccb0 | nametag/bar painter: device, state block, sprite (§16) | via 0x10074270 / 0x10075390 |
| 0x1026e9c4 | 0x44 | 0x1006ea30 / 0x1006e9f0 | scoreboard (§15) | 0x1006f0b0 |
| 0x1026e9c8 | 0x29d | 0x1006f480 / 0x1006f450 | dialog (§13) | 0x1006f890 |
| 0x1026e9cc | 0x28 | 0x100716c0 / 0x10071690 | panel with three RGB-tinted sliders [U: activation not found] | 0x10071b20 |
| 0x1026e9d0 | 0x260 | 0x1006afd0 / 0x1006af90 | model download progress ("Updating models..", dormant in R4 [I]) | 0x1006a910 |
| 0x1026e9d4 | 0x08 | 0x1006bbb0 / 0x1006bb70 | 3D-anchored text painter: device + sprite (bubbles, labels, /dl) | 0x1006bc90 |
| 0x1026e9d8 | 0x117 | 0x10071bc0 / 0x10071b90 | object editor (out of scope) | 0x100737e0 |
| 0x1026e9dc | 0x06 | 0x1006da00 / 0x1006d9c0 | object selection (out of scope) | 0x1006dc00 |
| 0x1026e9e0 | 0x0a | inline | textdraw selection (§14.4) | 0x10071470 |
| 0x1026e9e4 | 0x18 | 0x10060d40 / 0x10060cf0 | net stats overlay, F5 (§18) | 0x10060d70 |
| 0x1026e9e8 | 0x18 | 0x10071280 / 0x10071250 | F10 object; its draw (A 0x100712c0) is an empty function | — |
| 0x1026e9ec | 0x04 | 0x1006bb20 / 0x1006bae0 | F1 help (§13.6) | 0x1006bb30 |
| 0x1026e9f0 | 0x44a20 | 0x10066de0 / 0x10066da0 | chat bubbles (§11) | 0x10066ed0 |
| 0x1026ea18 | 0x24 | 0x1006d9b0 / 0x1006d970 | number-plate texture painter (§18) | on demand |
| 0x1026ea98 | — | — | GUI resource manager (fonts + `sampgui.png`) (§4) | — |
| 0x1026ea9c / eaa0 / eaa4 / eaa8 / eaac / eab0 | 0x1ea each | 0x1008cbc0 | GUI dialogs: chat+input, scoreboard, class selection, dialog, slider panel, download progress (§4.2) | — |
| 0x1026eab4 | — | — | cursor surface loaded from `mouse.png` (§5) | — |

Objects from 0x1026ea00 to 0x1026e9f0 are only created when the client is a network client
(flag A 0x1026de64). The objects from 0x1026e9d4 on are always created.

---------------------------------------------------------------------------------------------

## 2. Hooks, frame order and device handling

### 2.1 Device proxy [O]
- On the first frame (A 0x100c47d0) the client copies the RenderWare device pointer
  `_RwD3DDevice` (0xC97C28) into A 0x1026e9b8 and stores in 0xC97C28 a pointer to a 4-byte
  object. That object's vtable is A 0x100ed8f0 (B 0x100ed908) and has 119 entries mirroring
  `IDirect3DDevice9`. Almost every slot is a thunk that forwards to the real device. The
  exceptions:

| slot | method | A handler (B) | behaviour |
|---|---|---|---|
| 16 | Reset | 0x10074630 (0x10074600) | calls "on lost device" (A 0x100c3f90), then the real Reset; on success calls "on reset device" (A 0x100c41f0, B 0x100c41c0) |
| 17 | Present | 0x10075790 (0x10075760) | draws the whole D3D UI pass (§2.3), then calls the real Present |
| 42 | EndScene | 0x10074890 (0x10074860) | real EndScene wrapped between two integrity checks (A 0x1009d910, anti-tamper, no visible effect) |
| 41 | BeginScene | 0x10074870 | plain forward |

- The UI is therefore drawn **after** RenderWare's `EndScene` and before the real Present.
  The Present body itself does not call BeginScene/EndScene [O]. D3DX sprites and fonts
  draw straight to the back buffer outside a scene. A reimplementation should wrap its own
  pass in a scene where the driver requires one [I].
- The draw functions call the real device directly (A 0x1026e9b8), never the proxy.

### 2.2 On lost / reset device (A 0x100c41f0) [O]
On lost device (A 0x100c3f90, before the real Reset): OnLost of the same objects, then an open
chat input is closed (A 0x10069580, cursor mode 0 immediate) and an open scoreboard too
(A 0x1006e9e0 with 1: cursor mode 0, delayed restore); the cursor surface is released and the
WM_SETCURSOR "once" flag is re-armed (§5). Measured (GAP-UI, docs/phase3/trace-notes.md "Audit
gaps"): with the chat input open a device loss hides the cursor and closes the input.
On reset device:
`SetCursor(NULL)`, then: GUI resource manager reset, nametag state block, font renderer
fonts (`OnResetDevice`), plate painter, 3D-text painter, death-window fonts and sprite,
object-editor fonts, chat render-to-texture rebuild (§7.6) and chat sprites. Then the cursor
surface is re-created from `mouse.png`, the scoreboard is re-centred, and, if the GTA menu is
not active, cursor mode 2 then 0 is applied with an immediate input restore (§5).

### 2.3 Present: per-frame draw order (A 0x10075790) [O]
1. **Screenshot**, if the F8 flag (A 0x1012de64) is set (§6.4).
2. Detect GTA menu enter/leave (A 0x100744e0). On a change, call A 0x100a04a0 [U: effect].
3. **GTA pause menu active** (byte 0xBA67A4 ≠ 0): only the map right-click handler runs.
   On a right-click while on the map page it looks for the waypoint blip and sends RPC 119
   (player-state.md). All UI below is skipped.
4. If the netgame exists:
   a. nametag **bars** pass (A 0x10074270), then nametag **names** pass (A 0x10075390), §16;
   b. audio-stream title line (A 0x10066ab0), §18;
   c. `/dl` vehicle labels, if toggled (flag A 0x1026ea30): every vehicle within 20 m gets
      `[id: %d, type: %d subtype: %d Health: %.1f preloaded: %u]…` through the 3D text painter.
5. **Scoreboard open** → HUD off, draw the scoreboard, **skip 6–12** (go to 13).
6. **F5 held** (`GetAsyncKeyState(0x74)`) and netgame state = 5 → HUD off, draw net stats,
   skip to 13.
7. **F10 held** → HUD off, call the empty F10 draw, skip to 13.
8. **F1 held** → open the help dialog (§13.6), if no dialog is open.
9. HUD visibility := (chat display mode ≠ 0) (§7.3). Then HUD off if the local ped field
   +0x2c9 > 5000 [U: meaning].
10. Chat bubbles (§11), 3D text labels (pool +0x18, A 0x10001350, world-entities scope).
11. Chat window (§7), chat input (§8, it only sets the cursor mode), death window (§17).
12. Dialog (§13), slider panel, download progress, then GUI dialog render for 0x1026ea9c
    (chat scrollbar + input edit box) and 0x1026eaa4 (class-selection buttons). Then object
    editor, object selection and textdraw-selection per-frame logic (§14.4).
13. Debug overlay if started with `-d` (A 0x1009e4f0), integrity check, **real Present**,
    integrity check.

"HUD off/on" is A 0x100a1df0(on). It sets 0xBA6769 := on (believed
`FrontEndMenuManager.m_bHudOn`) and `CHud::bScriptDontDisplayRadar` (0xBAA3FB) := !on [O].
Consequences: the HUD and the radar disappear while the scoreboard is shown, while F5 or
F10 is held, and whenever the chat is in mode 0 (F7).

### 2.4 Per-frame game entry (0x53EB12 call retarget → A 0x100c5030, B 0x100c5010) [O]
1. A 0x100c4fb0: first-frame init (§1.1), then fog render states (FOGENABLE, FOGTABLEMODE 0,
   FOGVERTEXMODE linear), netgame processing (A 0x1000b5b0), radio stop while a stream plays,
   game-object tick (A 0x100a2210).
2. The original `Render2dStuff` (0x53E230), unless a skip flag (A 0x1026ea50) is set.
   That flag is consumed (reset) here.
3. **Textdraws**: if the GTA menu is not active, the netgame and the textdraw pool exist,
   and the scoreboard is not open → draw every present textdraw in id order 0..2303
   (A 0x1001e830, §14).
4. Input restore countdown (A 0x100a0630, §5).

GTA's `Idle` continues after that call with `RenderMenus`, `DoFade`, `CHud::DrawAfterFade`,
`CMessages::Display(false)` and `CFont::DrawFonts` [O, gta-reversed]. Textdraw *strings* are
queued by `CFont::PrintString` and flushed by that later `DrawFonts`. They therefore appear
above the fade, while sprite textdraws and boxes are drawn at once and sit below it [I —
TRACE].

### 2.5 Other hooks that matter for UI
- The 0x53E981 hook (A 0x100a26a0) runs once per game frame and calls the chat
  render-to-texture update (A 0x10067ed0, §7.6) [O].
- The radar draw (0x58FC53 → A 0x100a2540 → `CHud::DrawRadar` 0x58A330) and the crosshair
  draw (0x58FBBF → A 0x100a2590 → `CHud::DrawCrossHairs` 0x58E020) apply the HUD scale fix
  (§9) around the call and restore it afterwards [O]. The crosshair fix only applies when the
  local camera mode is 0x35. Textdraws always see the stock constants 1/640 and 1/448.
- Window procedure subclass (§6).

---------------------------------------------------------------------------------------------

## 3. Fonts and the text renderer

### 3.1 Font assets [O]
- `gtaweap3.ttf` (family "GTAWEAPON3", weapon pictograms) and `sampaux3.ttf` (family
  "SampAux3", status icons) are loose files in the GTA directory, put there by the SA:MP
  installer. They are **not** in samp.saa. At DLL attach (A 0x100c5100, after samp.saa is
  opened) the client calls `AddFontResourceA("gtaweap3.ttf")` and
  `AddFontResourceA("sampaux3.ttf")`. The paths are relative, so the process working directory
  (the GTA folder) must hold them. They are requested by family name `"GTAWEAPON3"` and
  `"SAMPAUX3"` (GDI family matching ignores case) with `SYMBOL_CHARSET` (2).
- `mouse.png` (32×32 RGBA arrow, hotspot 0,0) is the cursor (§5). `sampgui.png` (512×512 RGBA)
  is the GUI skin texture (§4).
- Both PNGs are loaded by relative path with D3DX file loaders (also not through samp.saa).
  The Rust client must read all four files from the user's SA:MP install, in line with D-1.

### 3.2 Size and face settings [O]
`W` = `RsGlobal.maximumWidth` (0xC17044), the video mode width. `fs` = config `fontsize`
(default 0, range −3..5 via `/fontsize`).

| helper (A) | value |
|---|---|
| main height 0x100c52d0 | W < 1024 → 14; W < 1400 → 16; W < 1600 → 18; else 20. Then + 2·fs |
| aux height 0x100c5320 | (W < 1024 ? 12 : 14) + 2·fs, never below the base (12/14) |
| GUI height 0x100c5360 | 20 + 2·fs (independent of resolution) |
| weight 0x100c5380 | config `fontweight` = 1 → 400 (normal), anything else (also missing) → 700 (bold) |
| face 0x100c53b0 | config `fontface` if present, else `"Arial"` |

All D3DX fonts use `D3DXCreateFontA(device, Height, Width=0, Weight, MipLevels=1, Italic=0,
CharSet, OutputPrecision=0, Quality, PitchAndFamily=0, Face)`. A positive `Height` is the
character cell height in pixels:

| owner | slot | face | height | weight | charset | quality | used for |
|---|---|---|---|---|---|---|---|
| font renderer | main (+0x00, colour-code wrapper) | face | main | weight | 1 | 4 (antialiased) | chat lines, net stats |
| font renderer | main outline (+0x08, raw) | face | main | weight | 1 | 4 | black outline passes of main text |
| font renderer | small (+0x04, colour-code wrapper) | face | main − 2 | weight | 1 | 4 | nametags, bubbles, 3D labels, dialog body, scoreboard texts, audio title |
| font renderer | small outline (+0x0c, raw) | face | main − 2 | weight | 1 | 4 | outline passes of small text |
| font renderer | plate (+0x10) | "Arial" | 38, width 10 | 700 | 1 | 4 | number-plate texture (§18) |
| death window | names (+0x137) | face | aux | weight | 1 | 0 | killer/victim names |
| death window | icon (+0x13b) | GTAWEAPON3 | aux + 8 | 400 | 2 | 0 | weapon glyph |
| death window | icon backdrop (+0x13f) | GTAWEAPON3 | aux + 12 | 400 | 2 | 0 | black box glyph `G` behind the weapon |
| death window (lazy) | status 20 (+0x14f) | SAMPAUX3 | 20 | 400 | 2 | 0 | AFK icon foreground (§16.3) |
| death window (lazy) | status 22 (+0x153) | SAMPAUX3 | 22 | 400 | 2 | 0 | AFK icon backdrop |
| GUI resource manager | font 0 | face | GUI | weight | 1 | 0 | buttons, edit boxes, list boxes, captions |
| GUI resource manager | font 1 | face | GUI − 2 | weight | 1 | 0 | static texts, dialog lists (§4) |

The font renderer also owns one `ID3DXSprite` (+0x14), a 100 001-byte scratch buffer (+0x1c)
and two measured heights: +0x20 = height of `"Y"` in the main font and +0x24 = the same in the
small font (DT_CALCRECT, colour codes stripped) [O].

`/fontsize` (A 0x10068670) saves the key, then rebuilds the font renderer fonts, the chat
render target and line metrics, the chat scrollbar and the death-window fonts [O]. The GUI
fonts (font 0/1) are only built when the GUI skin is set up, at the first frame and on a
device reset [I]. Dialog controls therefore keep their old size until a reset, while the
dialog body text, which uses the font renderer, changes at once.

### 3.3 Embedded colour codes `{RRGGBB}` [O]
- A code is `{` + exactly six hex digits + `}` (A 0x100b5fd0, wide-char version). Anything
  else is literal text.
- Strip (A 0x100b6100) removes every valid 8-char code in place. It is used for measuring, for
  the outline passes, for dialog response text and for list cells.
- **Colouring** happens in the wrapper font (vtable A 0x100ea410, DrawTextA at slot 14 =
  A 0x10073c40). When a sprite is passed:
  1. Remember the caller colour as the current glyph colour. Convert the text to UTF-16
     (`MultiByteToWideChar(CP_ACP)`, max 20 000 wchar) and keep that original.
  2. Draw the **stripped** text with the real font, but through a proxy `ID3DXSprite`
     (object A 0x10102bf0, vtable A 0x100ea744). D3DX calls the proxy's `Draw` once per
     visible glyph (not for spaces, tabs, CR or LF).
  3. Before forwarding each glyph to the real sprite, the proxy's `Draw` (A 0x10073c20 →
     0x10073b40) moves a cursor through the original text. It skips white space (space, tab,
     LF, CR), consumes any run of colour codes (each sets the colour to `0xFF000000 | RRGGBB`)
     and then steps over the glyph. The glyph is drawn in the current colour.
  - So the caller's **alpha** applies only until the first code. After any `{RRGGBB}` the alpha
    is forced to FF. [O]
  - Without a sprite argument the wrapper forwards unchanged (codes would print literally).
    The callers always pass a sprite.

### 3.4 Outline ("shadow") patterns [O]
All outlines are black `0xFF000000`, drawn with the **outline** font of the same size on the
**stripped** text, before the main text. Offsets are (dx, dy) in pixels from the text origin:

| who | offsets |
|---|---|
| font renderer main/small draw (A 0x1006b360 / 0x1006b4e0), when its shadow flag is set | (0,−1), (0,+1), (−1,0), (+1,0) |
| chat lines, W ≤ 1280 (A 0x10067470) | (0,−1), (0,+1), (−1,0), (+1,0) |
| chat lines, W > 1280 | (0,−2), (−1,−1), (+1,−1), (0,+2), (−1,+1), (+1,+1), (−2,0), (+2,0) |
| death-window names (A 0x10069c40) | (0,−1), (0,+1), (−1,0), (+1,0) |

Chat outlines are drawn only in display mode 2 (§7.3). The format flags are the caller's
`DT_*` flags OR'd with `DT_NOCLIP` (0x100).

---------------------------------------------------------------------------------------------

## 4. GUI layer (DXUT-derived)

### 4.1 Resource manager and skin [O]
The dialogs, buttons, list boxes, edit boxes and scrollbars are a statically linked copy of
Microsoft's DXUT GUI (`CDXUTDialog`, `CDXUTDialogResourceManager`; the strings
`"CDXUTDialogResourceManager::CreateFont…"` and `"UI\\DXUTShared.fx"` are present). Changes
by samp:
- The skin texture is `sampgui.png` instead of `dxutcontrols.dds` (A 0x1008d7e0).
- Fonts are set from the settings above (A 0x1008d7a0).
- The element colours differ from stock DXUT. The element table below is samp's
  (A 0x1008d7e0). Control types follow DXUT numbering: 0 button, 1 static, 2 check box,
  3 radio, 4 combo box, 5 slider, 6 edit box, 7 IME edit box, 8 list box, 9 scrollbar.
  Texture rects are in `sampgui.png` pixels (left, top, right, bottom).

| ctrl,elem | texture rect | font, format | colours (state mapping per DXUT [I]) |
|---|---|---|---|
| caption | (17,269,241,287) | font 1, DT_VCENTER, 0xFFD2D2D2 | caption texture colour 0xC80A0A0A |
| 1,0 static | — | font 1, 0x44, 0xFFBEBEBE; disabled 0xC8C8C8C8 | — |
| 0,0 button face | (0,0,136,54) | font 0, centre, normal 0xFFC8C8C8, mouse-over 0xFFFFFFFF | tex normal 0xC8FFFFFF, mouse-over 0xFFB92228, pressed 0xFF95B0D0 |
| 0,1 button fill | (0,0,136,54) | — | default 0x00FFFFFF, focus 0xC8FFFFFF, mouse-over 0xFFB92228, pressed 0x3C505050 |
| 2,0 / 2,1 check box | (0,54,27,81) / (27,54,54,81) | font 0 | normal 0x96FFFFFF, focus 0xC8FFFFFF, pressed 0xFFFFFFFF |
| 3,0 / 3,1 radio | (54,54,81,81) / (81,54,108,81) | font 0 | same as check box |
| 4,0..4,3 combo box | (7,81,247,123), (272,0,325,49), (7,123,241,265), (7,266,241,289) | | (see A 0x1008d7e0) |
| 5,0 / 5,1 slider | (1,290,280,331) / (54,54,81,81) | | button: mouse-over 0xFFF0F0F0, pressed 0xFFB92228, focus 0xFFDCDCDC |
| 9,0..9,3 scrollbar | track (243,144,265,155); up (243,124,265,144); down (243,155,265,176); thumb (266,123,286,167) | | stock |
| 6,0..6,8 / 7,0..7,8 edit box | text area (14,90,241,113), borders from (8,82)–(246,121) | | — |
| 8,0 list box | (13,124,241,265) | font 1, DT_LEFT\|DT_TOP\|DT_EXPANDTABS (0x40) | — |
| 8,1 list selection | (17,269,241,287) | font 1, 0x40 | selection colours 0xFFB92228 / 0xC8808080 |

  Edit boxes in samp's own dialogs and in the chat input override their colours: text
  0xFFFFFFFF, frame/background 0xF0050505 with disabled 0xC8808080, and three edit-box colour
  slots set to 0xFF969696, 0xFFB92228 and 0xFF0A0A0F [O]. The slots are believed to be
  caret, selection background and selected text [U: exact mapping]. Both edit boxes are IME
  edit boxes. If config `ime` is set, IME input is enabled on them.
- Dialogs are drawn with `OnRender(10.0)` (elapsed-time argument 10 s, so DXUT colour blends
  finish at once) [O].

### 4.2 GUI dialogs created by samp (A 0x100c5660) [O]

| global | initial size | background (all 4 corners) | caption | hosts |
|---|---|---|---|---|
| 0x1026eaa0 | 640×480 | 0x960A0A0A | no | scoreboard list box (§15) |
| 0x1026ea9c | full video mode (W×H) | — (none) | no | chat scrollbar (§7.4) and chat input edit box (§8) |
| 0x1026eaa4 | 310×40 | 0x960A0A0A | no | class selection: buttons id 6 `<<` at (10,5), id 5 `>>` at (110,5), id 4 `Spawn` at (210,5), each 90×30 |
| 0x1026eaa8 | 600×300 | 0xDC050505 | yes (title) | dialog (§13) |
| 0x1026eaac | 500×160 | 0xDC050505 | no | slider panel |
| 0x1026eab0 | 500×160 | 0xDC050505 | no | download progress |

The class-selection panel is centred horizontally at `y = clientHeight − 40 − 50`. It is made visible
(+0x13 := 1, position taken from `GetClientRect`) by the class-selection step A 0x10006100 on its first
frame without an open dialog (dialog +0x28); that frame also sends the first RPC 128 (A 0x100058d0). While a
dialog is open the step does nothing, so a dialog that is open when the class selection starts keeps the
panel hidden until it closes; a dialog opened later does not hide it (measured). The panel is hidden again at
the spawn (A 0x10003c20). Present renders the GUI dialog every frame; DXUT skips it while invisible. Shift spawns
and Left/Right cycle classes, with a 250 ms repeat guard and a 2 s lock-out after a spawn
request (A 0x10006100, player-state scope).

---------------------------------------------------------------------------------------------

## 5. Cursor and input-lock modes [O]

- At start-up (A 0x10062970 (B 0x10062930), called from A 0x100c5221 (B 0x100c5201)) samp
  inline-hooks `user32!ShowCursor` with a stub (A 0x10062960 (B 0x10062920)) that never touches
  the display counter and answers 0 for "show" and −1 for "hide"; the original is kept at
  A/B 0x1012e350 and only the object editor calls it (capture / release). GTA itself calls
  `ShowCursor(FALSE)` in its window procedure on activation (0x747FE9), on every `WM_SETCURSOR`
  it receives (0x7481CD, with `SetCursor(NULL)`) and at shutdown (0x748EAF); without the stub
  the counter goes negative before samp's subclass exists and no cursor is ever visible
  (CURSOR-1, measured under Wine 11 + DXVK 3.1.1: the original makes no `NtUserShowCursor`
  call at all).
- On the first `WM_SETCURSOR` the client calls `SetCursor(NULL)` once and then always
  answers TRUE, so the Windows cursor is hidden. The "done once" flag (A 0x1012de68) is
  cleared again by the lost-device handler (A 0x100c3f90, store at A 0x100c419b).
- The visible cursor is the **D3D hardware cursor**: a 32×32 `A8R8G8B8` offscreen surface
  (system memory) loaded from `mouse.png`, set with `SetCursorProperties(0, 0, surface)`,
  and shown with `IDirect3DDevice9::ShowCursor`. It is re-created after every device reset.
- Cursor/input mode (game object A 0x1026ea24 +0x61, setter A 0x100a0750 (B 0x100a0720)):

| mode | used by | effects |
|---|---|---|
| 0 | default; leaving any UI | hide the D3D cursor. The GTA input patches are restored by A 0x100a0630 from the per-frame entry after a countdown: 10 frames when mode 0 was requested with "delay", 0 when requested with "immediate". |
| 1 | — | NOP the call at 0x541DF5 (keyboard to pad) only. No cursor. |
| 2 | chat input open, dialog open, class selection, textdraw selection | NOP 0x541DF5; mouse lock (patch `GetMouseState` use at 0x53F417 and a branch at 0x53F41F); zero mouse deltas (0xB73424/28); `CPad::ClearMouseHistory` and `CPad::UpdatePads`; `RsMouseSetPos` (0x6194A0) := `ret`; **show cursor** |
| 3 | scoreboard | as 2 but keyboard **not** cut off (0x541DF5 untouched) |
| 4 | object editing | like 3 with a different mouse redirect (A 0x100a0540), no cursor |

  The GTA patch rows are in `gta-patches.md` ("Input lock").
- Several UIs request mode 2 **every frame** while they are visible (input, dialog), so
  the last caller wins [O].

---------------------------------------------------------------------------------------------

## 6. Window procedure and keys

### 6.1 Install [O]
A 0x10061ec0 (B 0x10061e80) subclasses the GTA window (`HWND` at 0xC97C1C) with
`SetWindowLongA(GWL_WNDPROC)` → A 0x10061650 (B 0x10061610). The old procedure is kept at
A 0x1012de60 and messages not consumed go to it through `CallWindowProcA`. If the integrity
failure counter A 0x1026ea48 > 10, every message returns 0.

### 6.2 Routing order (A 0x10061650) [O]
1. `WM_ACTIVATEAPP(TRUE)` with cursor mode 0 and the game not in its menu → hide the D3D
   cursor.
2. The chat input IME handler (if input open). Then the dialog object's handler.
3. GUI dialog `MsgProc`s, each only while its UI is visible and the GTA menu is off:
   chat/input dialog (if input open; Enter/Return key messages are swallowed after it),
   scoreboard dialog, class-selection dialog, samp dialog (only if no scoreboard and no input
   open). A `WM_CHAR` ESC that the samp dialog leaves unconsumed answers it with button 2
   (response 0).
4. Object editor, object selection and textdraw selection handlers. Each also maps a
   `WM_CHAR` ESC to its cancel path.
5. Slider panel dialog.
6. Message switch below. Then the original window procedure.

### 6.3 Key and mouse table [O]
Unless stated, keys act on **key-up** (`WM_KEYUP` → A 0x10061360).

| input | condition | action |
|---|---|---|
| `T`, `t`, `` ` `` (`WM_CHAR`) | input exists and closed, no dialog open, GTA menu off, scoreboard closed | open chat input |
| F6 (0x75) | input closed / open | open / close input |
| ESC (`WM_CHAR` 0x1B) | input open → close it; else scoreboard open → close it; else dialog open → response 0 | — |
| ESC (key-up) | input open and IME not composing | close input |
| ESC (`WM_KEYDOWN`) | input, scoreboard, dialog, slider panel, class selection, object edit/select or textdraw selection active | message swallowed, so GTA does not open its pause menu |
| Enter (0x0D key-up) | dialog open | response 1 (button 1) |
| Enter in an edit box | — | handled by the GUI edit-box event (chat send UCA "Local chat input"; dialog response 1). samp's edit box raises that event on the Enter **key-up** (`WM_KEYUP` 0x0D, A 0x10089380) and consumes it; the typed CR (`WM_CHAR` 0x0D) and key-downs (auto-repeat included) do nothing. So the chat line is sent on the key-up and a dialog the server shows in reply, or from `OnDialogResponse`, arrives after that key-up and stays open (INPUT-1, measured) |
| PageUp (0x21) / PageDown (0x22) | netgame and chat exist, IME not composing | scroll chat one page up / down (§7.4) |
| Up (0x26) / Down (0x28) | input open, IME idle | history older / newer (§8) |
| Mouse wheel | input open and chat mode ≠ 0 | scroll chat by −(notches × `SPI_GETWHEELSCROLLLINES`) lines |
| F7 (0x76) | — | chat display mode 2 → 1 → 0 → 2 (§7.3) |
| F8 (0x77) | — | set the screenshot flag; it is taken at the next Present (§6.4) |
| F9 (0x78) | — | toggle death-window visibility |
| Numpad − (0x6D) | debug mode only | `/rs` with an empty comment |
| Tab (`WM_KEYUP` 9) | scoreboard dialog exists, GTA menu off | toggle scoreboard (opening needs the class selection to be inactive) |
| F1 / F5 / F10 | polled with `GetAsyncKeyState` in Present | help dialog / net stats while held / HUD hidden while held |
| Alt+Enter (`WM_SYSKEYUP` 0x0D) | — | toggle the windowed flag of the current video mode (list at 0xC97C48, index 0xC97C18) and call GTA `SetVideoMode` (0x745C70) |
| Left button down / up | scoreboard open and the click is outside its rect | close the scoreboard on button-up |
| Left button down / up | input open (no object edit/select) and the click is outside the input rect | close the input on button-up |
| `WM_KILLFOCUS` | — | close scoreboard and input. If the class selection is visible, re-apply cursor mode 0 with no delay. Run the input-restore step. |

F4 (class change) and G (enter as passenger) are not in this window procedure. They are
polled by the local-player code (player-state scope) [I].

### 6.4 Screenshot (A 0x10075620) [O]
At the start of the next Present (before this frame's UI is drawn):
1. Pick the file name (A 0x10062270): `<user files>\screens\sa-mp-NNN.png`, `%03i` [U: how the
   free index is chosen].
2. Create a desktop-sized `A8R8G8B8` system-memory surface and call `GetFrontBufferData`
   (what is on screen, i.e. the previous frame **with** its UI).
3. Save the window's client rect, converted to screen coordinates, as PNG
   (`D3DXSaveSurfaceToFileA`, format 3).
4. Print a type-4 chat line `Screenshot Taken - sa-mp-%03i.png`, or `Unable to save
   screenshot.` (type 8).

---------------------------------------------------------------------------------------------

## 7. Chat window (global A 0x1026e9f8)

### 7.1 State (own names; offsets in the object) [O]
`+0x00` page size (default 10) · `+0x08` display mode (default 2) · `+0x0c` timestamps (byte) ·
`+0x0d` logging on · `+0x11` chatlog path · `+0x11e` scrollbar control · `+0x122/+0x126/+0x12a`
default colours · `+0x12e` y of the bottom of the last drawn line · `+0x132` 100 entries ×
0xFC · `+0x63a2` font renderer · `+0x63a6/+0x63aa` sprites (direct / blit) · `+0x63ae` device ·
`+0x63b2..+0x63c2` render-to-texture objects and display mode · `+0x63d6` texture valid ·
`+0x63da` dirty · `+0x63de` scroll position at last texture update · `+0x63e2` line height ·
`+0x63e6` timestamp width.

Default colours [O]: player chat **text** 0xFFFFFFFF (`+0x122`; the name uses the player
colour), info lines (type 4, all local `printf`-style messages, e.g. command feedback)
0xFF88AA62, debug lines (type 8) 0xFFA9C4E4. Server messages (RPC 93) carry their own colour
(UCA §93).

Logging [O]: at construction `chatlog.txt` is opened with mode `"w"` (truncated each session)
and closed. If that succeeds, logging is on and each entry is appended later (UCA "chat
window model").

### 7.2 Layout (A 0x10067940 direct draw; A 0x10067120 metrics) [O]
- Line height `h` = height of `"Y"` in the main font (`DT_CALCRECT|DT_SINGLELINE`).
  Timestamp width `tw` = width of `"[19:58:34]"`.
- Visible lines: entries `[pos, pos + pageSize)` of the 100-entry array, where `pos` is the
  scrollbar position (minimum 1). At the bottom `pos = 100 − pageSize`, so the newest entry
  (index 99) is on the last line.
- Line *k* (0-based): rect top `y = 10 + k·(h + 1)`. The first line's bottom is 110; later
  bottoms are `top + h + 1`. Left edge 45 (0x2D), right 550 (unused because of `DT_NOCLIP`).
  Flags `DT_SINGLELINE|DT_NOCLIP` (0x120).
- After the last line, `+0x12e` := the bottom of the rect the loop has prepared for the
  line *after* the last one: `10 + (pageSize + 1)·(h + 1)` (110 when the page size is 0).
  The loop runs once per page line, filled or not. The input box hangs below it (§8).
  [O, corrected in M4: the loop advances `top += h + 1; bottom = top + h + 1` at the end
  of every iteration; confirmed by a screenshot of the original, box top 175 at page size
  10 with h = 14.]
- Per entry:
  - With timestamps on: `strftime("[%H:%M:%S]", localtime(entry.time))` at x = 45 in the
    entry's text colour. The content then starts at `x = tw + 50`.
  - Type 2 (player chat): if the name is non-empty, measure it with the main font (wrapper,
    `DT_CALCRECT`), draw it in the name colour, then the text at `x + nameWidth + 5` in the
    text colour. The `:` is part of the stored name (UCA).
  - Types 4 and 8: text only, at x.
- No background box is drawn behind the chat [O]. Messages have **no lifetime or fade**: lines
  stay until 100 newer entries push them out [O].

### 7.3 Display modes (F7, A 0x100612c0) [O]
| mode | chat | outline | HUD + radar |
|---|---|---|---|
| 2 (default) | shown | yes (§3.4) | shown |
| 1 | shown | none (plain text) | shown |
| 0 | hidden (also the scrollbar) | — | **hidden** (§2.3 step 9) |

Each switch marks the chat dirty. The mode is **not** persisted.

### 7.4 Scrollbar and paging [O]
- GUI scrollbar in dialog 0x1026ea9c at x = 10, y = 40, width 20,
  height `(h + 1)·pageSize − 60`. Track range 1..100, page size = chat page size
  (A 0x10067200; re-laid out on `/pagesize` and `/fontsize`).
- Visible only when mode ≠ 0 **and** (the view is not at the bottom **or** the input is open)
  (A 0x10067e00).
- PageUp: `pos := max(1, pos − pageSize)`. PageDown: `pos := pageSize` if `pos == 1`, else
  `pos + pageSize`, capped at 100. The scrollbar then clamps to `100 − pageSize`. Both are
  ignored while the GTA menu is on or the IME is composing.
- A new message does **not** move `pos`: the array shifts under the view. At the bottom the
  view keeps following new lines; when scrolled up, the visible lines move up by one per
  message.
- `/pagesize` (and the stored value at start-up) goes through A 0x100672a0, which accepts
  10..100, re-lays out the scrollbar and marks the chat dirty. The command itself only
  accepts 10..20.

### 7.5 Draw path (A 0x10067e00, from Present) [O]
If mode ≠ 0: when render-to-texture is available and the texture is valid, draw that texture
with the blit sprite at (0,0), colour 0xFFFFFFFF. Otherwise draw the entries directly (§7.2)
inside a sprite `Begin(D3DXSPRITE_ALPHABLEND)`/`End`.

### 7.6 Render-to-texture cache (A 0x100681d0 create, A 0x10067ed0 update) [O]
- Disabled by config `directmode` ≠ 0 ("ChatWindow: Using direct drawing mode.").
- Texture size: 1024×512 when the adapter display-mode width ≤ 1024, else 2048×1024. Format
  `A8R8G8B8`, render-target usage, a `D3DXCreateRenderToSurface` with D16 depth. On failure a
  debug chat line is printed and direct mode is used.
- Update (once per game frame, from the 0x53E981 hook): skipped and marked invalid while the
  GTA menu is on. Otherwise, if mode ≠ 0 and (dirty or the scroll position changed): clear to
  0x00000000, set `D3DRS_DESTBLEND` := 14, draw the entries, end, then mark valid and clean.
- So the chat in the screenshot equals the direct draw clipped to the texture size [I].

---------------------------------------------------------------------------------------------

## 8. Chat input (global A 0x1026e9fc) [O]

- Control: IME edit box id 1 in GUI dialog 0x1026ea9c (created at A 0x10069840, B 0x10069800),
  colours as in §4.1, hidden and disabled while closed.
- **Open** (A 0x10069480): width `min(800, round(0.6 · clientWidth))`, position
  `(40, chat+0x12e)` (right under the last chat line), height `14 + round(1.5 · h0)` where `h0`
  = GUI font 0 height. Show, enable, focus. The host dialog becomes
  `(width + 100) × (chat bottom + 50)`. Then "open" := 1. Opening the input does not open the
  scoreboard or change chat mode.
- While open, Present requests cursor mode 2 every frame.
- **Close** (A 0x10069580): focus out, disable, hide, cursor mode 0 with **immediate**
  restore, "open" := 0. It does **not** clear the text or reset the history position
  [O, corrected in M4: the original kept "not sent" after ESC and sent "not sent/q" when
  "/q" was typed after reopening]. Enter clears the text itself and resets the history
  position to the live line after pushing the line. Also closed by: ShowDialog (UCA §61), `WM_KILLFOCUS`, a click outside
  the box, ESC, F6.
- **History** (10 entries of 129 bytes; index −1 = the live line): Up saves the live text the
  first time, then shows older entries. Down goes newer and restores the live text at −1.
- Enter → the line processor and local commands (UCA "Local chat input"). The 128-character
  limit is applied there.
  When the input was open, Enter also sets the chat scroll position to 90 (A 0x10067390;
  the scrollbar then clamps it to `100 − pageSize`), i.e. the chat jumps to the newest
  lines.

---------------------------------------------------------------------------------------------

## 9. Local commands — UI effects and persistence

Command list, argument ranges and printed texts: UCA "Local chat input". The table adds what
each changes on screen and what persists. Persistence is in `sa-mp.cfg` in the user files
directory (writer A 0x10066080 rewrites the whole file as `key=value` lines). The Windows
registry (`Software\SAMP`) is not touched by these commands [O].

| command | persists (key → value) | visible effect |
|---|---|---|
| `/pagesize N` (10..20) | `pagesize` = N | lines on screen, scrollbar height |
| `/fontsize N` (−3..5) | `fontsize` = N | §3.2 sizes. Chat, nametags, bubbles, death window and dialog body change at once; GUI controls change after a reset |
| `/timestamp` | `timestamp` = 0/1 | `[HH:MM:SS]` column (§7.2). The stored value is read at start-up |
| `/fpslimit N` (20..90) | `fpslimit` = N (created as 48 if missing at start-up; values outside 20..90 are ignored) | frame limiter |
| `/headmove` | `disableheadmove` = 0/1 | ped head follows the camera or not |
| `/hudscalefix` | `nohudscalefix` = 0/1 (inverted) | radar (and the aiming crosshair) are scaled for aspect ≥ 1.6 (§2.5): x unit := 0.00222 / aspect, y unit := 0.00242, HUD floats 0x866B74/78 := 82/96 (otherwise 76/94) during the call. Default **on** |
| `/nametagstatus` | `nonametagstatus` = 1/0 | AFK icon on nametags (§16.3); prints `NameTag Player Status: ON/OFF` |
| `/audiomsg` | `audiomsgoff` | the `Audio stream: %s` line |
| `/logurls` | `logurls` | URL debug lines |
| `/dl` | — | vehicle debug labels (§2.3 4c) |
| `/ctd` | — | camera-target debug labels |
| `/interior`, `/mem` | — | one info line |
| `/save`, `/rs` | — (writes `savedpositions.txt` / `raw*.txt`) | one info line |
| `/q`, `/quit` | — | quits |
| `/togobjlight`, `/cmpstat`, `/testdw`, `/rcon` | — | UCA |

Other UI keys read from `sa-mp.cfg` (no command): `fontface` (string), `fontweight`
(1 = normal), `directmode` (chat render-to-texture off), `ime` (IME input), `multicore`,
`audioproxyoff`. The file format and start-up handling belong to RE-6.

---------------------------------------------------------------------------------------------

## 10. Client messages and player chat on screen [O]
- RPC 93 → type-4 entry with the server colour (alpha forced FF). `{RRGGBB}` codes inside the
  text recolour the rest of the line (§3.3). `~k~` is expanded before storing (UCA §93).
- RPC 101 → type-2 entry: name in the player colour, text in white, wrapping at 72 chars into a
  second entry (UCA).
- Local feedback lines use 0xFF88AA62 (type 4) or 0xFFA9C4E4 (type 8).

## 11. Chat bubbles (A 0x10066ed0) [O]
Per player slot (1004 slots, 0x118 bytes each): active, text (wrapped with `\n`), start tick,
duration, colour, distance, line count. Each Present:
- An expired bubble (`now > start + duration`) is deactivated.
- A bubble is drawn for a remote player who is streamed in with a ped, whose state ≠ 0, and
  whose distance (A 0x1009f120) is below the bubble distance.
- Anchor: head bone (id 8) position, raised by
  `0.2 + 0.02·(lines − 1) + (0.065 + 0.0125·(lines − 1)) · d`, where `d` = camera distance
  (A 0x1009f1f0).
- Projected with the current view/projection matrices (copied from GTA's camera into A
  0x10140be8/0x10140c28) and the device viewport. Drawn only if projected z ≤ 1.
- Text: small font, `DT_CENTER|DT_NOCLIP` (0x101), centred on the projected point (rect
  x..x+1, y..y+1), bubble colour, outline on. Bubbles do not fade.

## 12. Game text and menus
- **Game text** (RPC 73): handed to GTA as `CMessages::AddBigMessage` after clearing prints
  (UCA §73). GTA styles and `~x~` codes; GTA draws it in its own HUD/messages pass, inside the
  scene and before samp's Present UI. Only change on the samp side: `CFont::ProcessCurrentString`
  gets a larger stack line buffer (`gta-patches.md`), and the SP "Wasted" big message is
  removed. [O]
- **Menus** (RPC 76/77/78): GTA `CMenuSystem`, built through script opcodes. The text comes
  through samp's `CText::Get` hook keys (`SAMPHED`, `SAMP0rr`, `SAMP1rr`, `SAMPRW1/2`).
  Navigation, highlight and drawing are GTA's. Selection is polled per frame (UCA §132/§140).
  [O]

---------------------------------------------------------------------------------------------

## 13. Dialogs (global A 0x1026e9c8; GUI dialog 0x1026eaa8)

### 13.1 Controls (A 0x1006fa20) [O]
- List box (control at object +0x20): (10,10), size (600, 300 − 100) initially. Item colours
  0xC8FFFFFF / 0xC8808080. Column count 0, hidden.
- Button id 0x14 (button 1, label `BUTTON1` placeholder) at (10,5) and id 0x15 (button 2) at
  (110,5). Size `BW × BH` = 100 × 30 (object +0x14/+0x18).
- IME edit box id 0x13 at (10,175), 570×40, colours as in §4.1, hidden.
- Dialog background 0xDC050505. The caption text = title (first 64 chars). Caption height
  `C` = height of `"Y"` in the small font + 4, set only when the title is non-empty.

### 13.2 Geometry per style (A 0x10070010, then A 0x1006f500) [O]
Body size `(bw, bh)` = small-font `DT_CALCRECT|DT_EXPANDTABS` size of the body with colour
codes stripped. `h0`/`h1` = GUI font 0/1 heights. `W`/`H` are the dialog's outer width and
height.

| style | W | H | visible controls |
|---|---|---|---|
| 0 MSGBOX | bw + 40 | C + bh + 25 + BH | buttons |
| 1 INPUT, 3 PASSWORD | bw + 40 | C + bh + 25 + BH + round(1.6·h1 + 14) | edit box (style 3: masked), buttons |
| 2 LIST | clamp(maxRowWidth + 20 + 40, 400, 800) | C + BH + 15 + LH | list box at (5,5), size (W − 10, LH) |
| 4 TABLIST | max(400, colsTotal + 40) | C + BH + 15 + LH | list box at (5,5) |
| 5 TABLIST_HEADERS | as 4 | C + BH + 15 + LH + hdrH | list box at (5, hdrH + 5); headers drawn as text |

- `LH = clamp(sum of row heights + 2·listBorder, 200, 400)`. Row height = list font height.
- Rows are split on `\n` (lines up to 256 chars). Columns are split on `\t` (cells up to 128
  chars; runs of tabs count as one separator). Up to 4 columns. Column widths: for the first
  three columns, `wK = rawK + 20`, where `rawK` is the widest cell of column K (measured
  with GUI font 1, codes stripped). `colsTotal = raw0 + raw1 + raw2 + raw3 + 80`. If
  `colsTotal + 40 < 400`, W = 400 and each of the first `columnCount − 1` column widths gets
  `(360 − colsTotal) / (columnCount − 1)` extra (integer division) [O].
- Style 5: the first line gives the header cells (count = columns, list box gets count − 1
  extra columns). Header height `hdrH` = small-font height of `"Y"`.
- Minimum width for all styles: `2·BW + 30` = 230.
- **Placement**: centred in the GTA window client rect.
- **Buttons** (dialog-local y = `H − C − BH − 5`): with button 2 shown → button 1 at
  `W/2 − BW − 10`, button 2 at `W/2 + 10`. Otherwise button 1 alone at `W/2 − BW/2`. Button 2
  is hidden when its label is empty.
- **Edit box**: x = 5, width W − 10, height `14 + round(1.5·h0)`,
  y = `H − C − round(1.5·h0) − BH − 24`.
- Focus: edit box (1, 3) or list box with selection 0 (2, 4, 5).

### 13.3 Per-frame draw (A 0x1006f890) [O]
Only when open, the input is closed and the scoreboard is closed:
1. cursor mode 2;
2. GUI dialog `OnRender(10.0)`;
3. body text for styles 0/1/3: small font at `(X + 20, Y + C + 5)` with the dialog's
   right/bottom, `DT_NOCLIP|DT_EXPANDTABS` (0x140), default colour **0xFFA9C4E4**, outline
   on. `{RRGGBB}` recolours.
4. style 5 headers: small font at `x = X + 16 + Σ previous column widths`, `y = Y + C + 3`,
   `DT_SINGLELINE|DT_NOCLIP`, colour 0xFFA9C4E4, outline on.
List rows are drawn by the GUI list box with GUI font 1. Colour codes in rows are **not**
interpreted by the GUI list box [U: whether the list box uses the colour-code wrapper; TRACE].

### 13.4 Input [O]
Enter key (key-up) → response 1; button 1 click or list double-click/activate → 1; Enter in
the edit box (its key-up, §6.3) → 1; a typed CR or a held/auto-repeated Enter key-down → nothing; ESC (`WM_CHAR`) or button 2 → 0 (UCA §62). While open, ESC key-down never
reaches GTA. Opening a dialog closes the chat input.

### 13.5 Close
Cursor mode 0 with the 10-frame restore delay, GUI dialog hidden, "open" := 0 (A 0x1006f860).

### 13.6 Local dialogs [O]
F1 (held, polled in Present) opens, if no dialog is open, a **local** dialog: id 1, style 0,
title `SA-MP Keys:`, body = the colour-coded key list (A string 0x100ea470), button 1 `OK`,
no button 2, "to server" flag 0. So pressing a button sends nothing. This closes the open
question in UCA "Unknowns" for F1. Other local openers were not found [U].

---------------------------------------------------------------------------------------------

## 14. Textdraws

### 14.1 When [O]
In the per-frame game entry, after `Render2dStuff` (§2.4): all present textdraws in id order
0 → 2303. Global textdraws (0..2047) come before per-player ones (2048..2303). Skipped while
the GTA menu is on or the scoreboard is open. Every textdraw is drawn every frame; there is
no dirty tracking.

### 14.2 Text path (A 0x100b31e0, B 0x100b31b0) [O]
`W`,`H` = video-mode width/height (0xC17044/48). `ux` = GTA float 0x859520 (= 1/640) and
`uy` = 0x859524 (= 1/448), read at draw time:
1. Copy the text into a work buffer.
2. `CFont::SetScale(W·letterX·ux, H·letterY·uy·0.5)`.
3. `SetColor(hovered ? hoverColour : letterColour)`, `SetJustify(false)`.
4. `SetOrientation(right ? 2 : centre ? 0 : 1)`.
5. `SetWrapx(W·textSizeX·ux)`, `SetCentreSize(W·textSizeY·ux)`.
6. `SetBackground(box, false)`, `SetBackgroundColor(boxColour)`, `SetProportional`,
   `SetDropColor(backgroundColour)`.
7. `SetEdge(outline)` if outline ≠ 0, else `SetDropShadowPosition(shadow)`.
8. `SetFontStyle(font)`.
9. `InsertNumberInString` (0x69DE90, all slots −1). `InsertPlayerControlKeysInString`
   (0x69E160) if the text had `~k~`.
10. `PrintString(W − W·ux·(640 − x), H − H·uy·(448 − y))`, which equals `x·W/640`,
    `y·H/448` with these constants.
11. `SetEdge(0)`.
12. Store the screen-space hit rectangle and set "drawn". With box on: x from left to
    `left + wrap`. Otherwise the rect from the text extent and orientation (A 0x100b33e9..
    0x100b34a0 [O, formula details: TRACE]).
- Letter colours are stored as R,G,B,A bytes and passed to `CFont` as `CRGBA` (UCA §134).

### 14.3 Sprite / model path (A 0x100b30d0) [O]
Used when the texture slot (+0x9a3) ≠ −1 (font 4 sprite or font 5 preview). Rect =
`(x·W·ux, y·H·uy) – ((x + textSizeX)·W·ux, (y + textSizeY)·H·uy)`. Drawn at once with the
loaded texture (array A 0x1026b3e0) in letter or hover colour (A 0x100b2ce0, a `CSprite2d`-style
quad). The hit rect is the same rectangle.

### 14.3a Model previews (font 5) [O + measured, M8A-PREVIEW]
- Dispatch per textdraw (A 0x100b34c0): texture slot +0x9a3 ≠ −1 → sprite path, else the text path. A
  preview therefore goes through `CFont` (its text, usually `_`, font style 5) until its texture exists.
- Camera set, built once in the first-frame UI pass (A 0x100c4d97 → A 0x1006ccc0 → A 0x1006be40): ambient
  light `RpLightCreate(2)` colour (1,1,1,1); `RwRasterCreate(256,256,0,0x505)` + `RwTextureCreate` (never
  used afterwards); Z raster `RwRasterCreate(256,256,0,1)`; `RwCameraCreate`, `RwFrameCreate`, frame
  translated (0,0,50) then rotated 90° about X (pre-concat); camera raster/Z raster, frame, far 300, near
  0.01, view window (0.5,0.5), perspective, `RpWorldAddCamera(Scene.m_pRpWorld)`.
- Rendering: the 0x53E981 hook calls A 0x1001e870 after `CGame::Process` (A 0x100a27e6): for every textdraw
  of style 5 with slot −1 (A 0x100b34e0), by model id (signed 16-bit +0x9a8): ped model (model-info vtable
  0x85BDC0) → A 0x1006c140; 400..611 → A 0x1006c3c0 (569/570 → 537/538); 321..373 or ≥ 615 → A 0x1006c9b0
  (no model info → 18631); < 0 → A 0x1006c9b0 with 18631 (no custom-model table in R4); anything else is
  never rendered. Ids ≥ 20000 read past GTA's model table: id 20000 crashed the original (measured).
  A non-zero texture gets a slot (A 0x100b2b60, 200 slots shared with sprite textdraws).
- Every render: a NEW `RwRasterCreate(256,256,0,0x505)` + `RwTextureCreate`; camera raster := it, Z raster
  := the shared one; `CVisibilityPlugins::SetRenderWareCamera`; `RwCameraClear(cam, &bg, 3)` with the
  textdraw's background colour (+0x97f) as `RwRGBA`; `RwCameraBeginUpdate`; `RpWorldAddLight`; render states
  ZTEST 1, ZWRITE 1, SHADEMODE 2, (objects: ALPHATESTFUNCTION 0, CULLMODE 0), FOG 0; draw;
  `RwCameraEndUpdate`; `RpWorldRemoveLight`.
  - Ped: samp's ped constructor with player slot 0xD0, skin 0 at (0,0,0) (the remote-ped script sequence),
    then position (0, −2.25·zoom, 50.05) (skin 162: 50.15), skin := model, entity flags |= 0x80000002 then
    bit 0 cleared, matrix rotated (`RwMatrixRotate` pre-concat about X, Y, Z for each non-zero angle) and
    written back (Remove, rows, `CMatrix::UpdateRW`, `UpdateRwFrame`, Add); after the camera begin: added to
    the world when not in the moving list, `RpAnimBlendClumpUpdateAnimations(clump, 100, 1)`, `PreRender`,
    `CRenderer::RenderOneNonRoad`; destroyed; model marked no longer needed when its reference count is 0.
  - Vehicle: samp's vehicle constructor at (0,0,50) heading 0; flags as above; y = (−1 − 2r)·zoom with r =
    collision sphere radius (vtable 0x8721A0: −5.5 − 2.5r, zoom ignored); position (0, y, 50); colours 1/2
    written when both ≠ −1; rotation as for peds; `SetupVehicleVariables`, `PreRender`, `RenderOneNonRoad`
    with samp's flag A 0x1026ea58 set (its fog setter writes FOGENABLE 0); `DELETE_CAR`.
  - Object: models 1373, 3118, 3552, 3553 → 18631; loaded on demand (request, load-all, `Sleep(1)` until
    loaded); `CreateInstance()` (no instance → the uncleared texture is kept); the instance frame translated
    by (−cx, (−0.1 − 2.25r)·zoom, 50 − cz) (collision sphere centre / radius) and rotated pre-concat about
    X, Y, Z (18631: Z 180° only); atomic render callback or `RpClumpRender`; atomic and frame destroyed; the
    model marked no longer needed if it was requested here.
- The new texture is drawn by the sprite path in the same frame (measured: one `DrawPrimitive` triangle fan
  with the 256×256 render-target texture, the letter colour, the sprite rectangle).
- Freed: with the textdraw (A 0x100b2f90) and in the lost-device pass (A 0x100c41e5 → A 0x1001e8a0), so it is
  rendered again on the next game frame. A re-sent RPC 134 (e.g. after `TextDrawSetPreviewRot`) is a new
  textdraw and renders again.

### 14.4 Selection mode (A 0x10071470 per frame, 0x100715d0 mouse) [O]
UCA §83 has the protocol. Rendering: while active, cursor mode 2 (D3D cursor `mouse.png`). The
hovered textdraw (highest id under the cursor among selectable, drawn ones) uses the hover
colour on its next draw. Left-button **up** over a hovered textdraw sends its id. Selection
stays active. ESC (`WM_CHAR`) cancels and sends 0xFFFF.

---------------------------------------------------------------------------------------------

## 15. Scoreboard (global A 0x1026e9c4; GUI dialog 0x1026eaa0)

### 15.1 Geometry (A 0x1006ea30, 0x1006eab0) [O]
- Size `SW × SH` = 640 × 480, or 800 × 600 when W > 800 (and the game object exists). Scale
  `s` = 1.0.
- Placed at `X = W/2 − SW/2`, `Y = H/2 − SH/2` (video-mode size, re-centred every frame).
  Background 0x960A0A0A.
- List box at (0, 60), size (SW, SH − 60), 3 extra columns. Column widths: id
  `round(0.085·SW)`, name `round(0.4375·SW)`, score `round(0.265625·SW)`, ping = the rest.
  Item colours 0xC8FFFFFF / 0xC8808080.
- Texts (small font, `DT_SINGLELINE|DT_NOCLIP`, outline on):
  - server name (netgame +0x131) at (X+5, Y+5)–(X+SW, Y+60), colour 0xFFBEBEBE;
  - `Players: %d` (pool count + 1) with left edge `X + SW − width("Players: 10000")`, same
    colour;
  - headers `id`, `name`, `score`, `ping` in 0xFF95B0D0, top = `Y + 60 − smallH − 3`,
    left = X + 10, then + each column width.

### 15.2 Rows (A 0x1006ed30, rebuilt on open and whenever the player count changes) [O]
- Entries: the local player first, then every connected remote player except NPCs, in id
  order. Row = main text `"%u"` id (row colour = player colour, `(rgba >> 8) | 0xFF000000`),
  cells: name, `"%d"` score, `"%u"` ping. The previous selection and scroll position are kept.
- Sort mode field +0x40: 0 = none (default; nothing sets another value [O]), 1 = by name,
  2 = by score descending.

### 15.3 Behaviour [O]
Tab (key-up) toggles. Open: dialog visible, list enabled, rows built, **cursor mode 3**. Close:
ESC, Tab, click outside, focus loss. While open: the HUD is hidden, Present draws only the
scoreboard (no chat, dialogs, bubbles, death window), textdraws are not drawn, and RPC 155
(score/ping update request) is sent at most every 3000 ms (A 0x10008f10).

---------------------------------------------------------------------------------------------

## 16. Nametags and health/armour bars

### 16.1 Who gets one (A 0x10074270 bars, A 0x10075390 names) [O]
Both passes run only if the server enables nametags (netgame settings +0x38). They loop over
remote player ids 0..max. A player is drawn when the slot is active, the remote object has a
ped (+0x1dd), state (+0x10a) ≠ 0, "show nametag" (+0x04, set by RPC 80) ≠ 0, the ped is
valid, the distance (A 0x1009f1f0) < the server's nametag draw distance (settings +0x27,
float), and the player is not an NPC (+0x10c = 0).
- Anchor: bone 8 (head) of the ped, which must be valid and in the moving list (A 0x1009f430,
  `CPhysical` +0xB4 ≠ 0) — also for drivers of ordinary vehicles. Only for state 0x13 with a
  vehicle (remote +0x1e1) that passes A 0x100b7e40 — valid pool handle (GTA 0x4048E0) **and an
  RC model** (441, 464, 465, 501, 564, 594) — the anchor is the vehicle's matrix position
  (+0x30 of its matrix). Measured (vehtag, original): an Infernus driver's tag sits at samp's
  anchor global A 0x10140bdc = the head, ≈ 0.50 m above the seated ped's root and 0.30 m above
  the car's matrix position. If "nametag LOS" (+0x2f) is on — always in R4: the RPC 139 handler reads the bit into
  +0x2f and then writes 1 there (B 0x10010778), whatever the server sent — script opcode 0x06BD
  (line-of-sight clear: buildings and objects only) from the anchor to the camera position
  must pass.
- Screen point: project `(x, y, z + 0.0475·d + 0.2)` with the device viewport and samp's copy
  of GTA's view/projection (A 0x10140be8/0x10140c28). `d` = distance to the camera. Draw only
  if projected z ≤ 1. `X`, `Y` = rounded screen coordinates.

### 16.2 Bars (A 0x1006d0a0) [O]
Pre-transformed vertices (`D3DFVF_XYZRHW|D3DFVF_DIFFUSE`), triangle fans of 4 vertices.
The state block from A 0x1006d6b0 is applied: alpha blend, texture stage 0 diffuse only,
no texture, no Z [I from the set calls]. Health `hp` = remote +0x1b0, armour `ar` = remote
+0x1ac, each clamped to ≤ 100. Nothing is drawn when `hp < 0`.

| quad | x range | y range | colour |
|---|---|---|---|
| border | X−20 … X+21 | Y+18 … Y+24 | 0xFF000000 |
| health background | X−19 … X+19 | Y+19 … Y+23 | 0xFF4B0B14 |
| health fill | X−19 … X−19 + hp·0.3846154 | Y+19 … Y+23 | 0xFFB92228 |

- If `ar > 0`, the three health quads are moved **down 8 px** (Y+26…Y+32 border). The same
  three quads are then drawn again at the original height for armour: border black,
  background 0xFF282828, fill 0xFFC8C8C8 with width `ar·0.3846154`. Armour is on top, health
  below.

### 16.3 Name text (A 0x1006cda0) [O]
- Text `"%s (%d)"` = name and id. Small font, colour = player colour with alpha FF, outline on,
  flags 0x100. Rect: centred on X with width = measured width, bottom edge
  at `Y + 17` (top = Y + 17 − text height). So the text sits just above the bar.
- **AFK icon** (when `/nametagstatus` is on, netgame +0x234, and the remote "paused" state
  +0x1c5 = 2, which is set when no update arrived for ≥ 3000 ms and cleared when an update
  arrives within 1500 ms): SAMPAUX3 glyph `C` (22 px font) in black as backdrop, then glyph
  `E` (20 px font) in 0xFFE3E1E3 offset (−1, −2). Both are `DT_CENTER|DT_VCENTER|DT_NOCLIP`,
  placed left of the name: x ≈ nameLeft − 22 − glyphW/2, y ≈ Y + 17 + glyphH/2 [O for
  constants; exact rect arithmetic: TRACE].

---------------------------------------------------------------------------------------------

## 17. Death window (global A 0x1026ea00)

### 17.1 Data and geometry [O]
- 5 rows of 0x3B bytes (killer name [25], victim name [25], killer colour, victim colour,
  reason byte). A new entry is appended at the **bottom** and the others move up
  (A 0x1006a5d0). Rows stay until pushed out (no timeout). `/testdw` fills test rows. Visible
  flag +0 (default 1), toggled by F9. The flag is not persisted.
- Metrics: `nickW` = width of `"LONGESTNICKNICK_NICKNICK"` (names font, `DT_CALCRECT`).
  `iconW`, `iconH` = measured size of a GTAWEAPON3 glyph (+0x12f/+0x133).
- Origin: `x = round(0.75·W)`, `y = round(0.3·H)` (video-mode size). If
  `x + nickW·2 + iconW > W`, then `x = W − (nickW·2 + iconW)`. Row step `iconH + 5`.
- Per row (within a sprite `Begin/End`):
  - killer present: killer name right-aligned to end at `x + nickW` (outline, killer colour);
    weapon icon box at `left = x + nickW + 3` (35 px wide, top − 5): glyph `G` from the backdrop
    font in black, then the reason glyph centred (`DT_CENTER|DT_VCENTER`) in **white**
    0xFFFFFFFF; victim name at `x + nickW + 3 + iconW` (victim colour).
  - killer empty: the victim name in the killer position, then the icon. Reason 200 draws in
    0xFF1111AA and reason 201 in 0xFFAA1111 (connect/disconnect messages).
- Names: names font, `DT_NOCLIP`, 4-direction black outline (§3.4).

### 17.2 Reason → GTAWEAPON3 glyph (A 0x10069e50) [O]

| reason | glyph | reason | glyph | reason | glyph |
|---|---|---|---|---|---|
| 0 | `%` | 14 | `$` | 30 | `H` |
| 1 | `B` | 15 | `#` | 31 | `5` |
| 2 | `>` | 16 | `@` | 32 | `7` |
| 3 | `(` | 17 | `D` | 33 | `.` |
| 4 | `C` | 22 | `6` | 34 | `A` |
| 5 | `?` | 23 | `2` | 35 | `4` |
| 6 | `&` | 24 | `3` | 36 | `)` |
| 7 | `"` | 25 | `=` | 37 | `P` |
| 8 | `!` | 26 | `0` | 38 | `F` |
| 9 | `1` | 27 | `+` | 39 | `<` |
| 10–13 | `E` | 28 | `I` | 40 | `;` |
| 41 | `/` | 42 | `,` | 46 | `:` |
| 49 | `L` | 50 | `R` | 51 | `Q` |
| 54 | `K` | 200, 201 | `N` | any other | `J` |

---------------------------------------------------------------------------------------------

## 18. Smaller overlays [O]
- **Audio stream title** (A 0x10066ab0): `"<title> - <station>"` (both known), small font at
  (15, H − 20)–(W, H + 10), flags 0x100, 0x99FFFFFF, no outline.
- **Net stats** (F5 held, connected): title `Client Network Stats`, then lines of RakNet
  statistics plus `Process Mem`, `Streaming Mem`, send rates, pool counts and
  download/upload rates. Main font, white, outline, one draw per line. [U: exact origin;
  TRACE]
- **3D text labels** (pool, A 0x10001350) and bubbles share the 3D text painter (A 0x1006bc90):
  project, test z ≤ 1, optional LOS test, rect [x, y, x+1, y+1] (truncated), then the font renderer's
  small draw (B 0x1006b4a0) with 0x101: if the painter's outline flag is set, four black passes of the
  **stripped** text with the outline font, then the **raw** text through the colour wrapper (§3.3), so
  `{RRGGBB}` codes colour labels and bubbles per glyph. Outline flag: labels pass `alpha == FF`
  (A 0x100013bc; a translucent label has no outline), bubbles always 1 (A 0x1006702f); LOS flag:
  the label's, bubbles 0. [O, measured by the DIFF-4 elements `label3d` / `bubblecol`]
- **Number plates**: the plate painter renders the plate string into a 128×32 render target
  with the Arial 38/10/bold font, clear colour 0xFFBEB6A8 and text 0xEE444470 (A 0x1006d880),
  then copies it into a 64×32 texture [I]. Vehicles scope.
- **Class selection** panel: §4.2.
- **Download progress / slider panel**: dormant in normal play [I].

---------------------------------------------------------------------------------------------

## 19. Rendering checklist for the Rust client (derived)
1. Proxy or hook `Present` and `Reset` on the device in 0xC97C28. Draw §2.3 in that order;
   recreate fonts, sprites, state block, cursor and chat RT on reset.
2. Retarget the 0x53EB12 call: run the netgame tick, call `Render2dStuff`, then draw
   textdraws through `CFont` with the stock 1/640 and 1/448 constants.
3. Build the font set in §3.2 from `fontsize`/`fontface`/`fontweight` and the video-mode
   width. Register the two TTFs from the SA:MP install before creating fonts.
4. Reproduce the per-glyph `{RRGGBB}` colouring with alpha forced to FF after a code, and the
   outline patterns in §3.4.
5. The GUI (dialogs, scoreboard, input) can be a reimplementation, but it must match the
   `sampgui.png` element rects and colours in §4.1 and the geometry formulas.
6. Subclass the window procedure with the routing and key table in §6. Keys act on key-up.
7. HUD visibility rules of §2.3 (chat mode 0 hides HUD and radar).

---------------------------------------------------------------------------------------------

## 20. Address table (A / B)

| function | A | B |
|---|---|---|
| first-frame init (creates UI objects) | 0x100c47d0 | 0x100c47b0 |
| GUI dialog setup (`mouse.png`, dialogs) | 0x100c5660 | 0x100c5640 |
| on reset device | 0x100c41f0 | 0x100c41c0 |
| per-frame game entry / pre-step | 0x100c5030 / 0x100c4fb0 | 0x100c5010 / 0x100c4f90 |
| proxy vtable / Present / Reset / EndScene | 0x100ed8f0 / 0x10075790 / 0x10074630 / 0x10074890 | 0x100ed908 / 0x10075760 / 0x10074600 / 0x10074860 |
| screenshot | 0x10075620 | 0x100755f0 |
| HUD on/off | 0x100a1df0 | 0x100a1dc0 |
| cursor mode set / input restore | 0x100a0750 / 0x100a0630 | 0x100a0720 / 0x100a0600 |
| window proc / char handler / key-up handler / install | 0x10061650 / 0x10061590 / 0x10061360 / 0x10061ec0 | 0x10061610 / 0x10061550 / 0x10061320 / 0x10061e80 |
| Alt+Enter windowed toggle | 0x10061310 | 0x100612d0 |
| font heights / weight / face / GUI height | 0x100c52d0, 0x100c5320 / 0x100c5380 / 0x100c53b0 / 0x100c5360 | 0x100c52b0, 0x100c5300 / 0x100c5360 / 0x100c5390 / 0x100c5340 |
| font renderer ctor / (re)create fonts | 0x1006baf0 / 0x1006b8e0 | 0x1006bab0 / 0x1006b8a0 |
| main draw / small draw / measure main / measure small | 0x1006b360 / 0x1006b4e0 / 0x1006b200 / 0x1006b2b0 | 0x1006b320 / 0x1006b4a0 / 0x1006b1c0 / 0x1006b270 |
| colour wrapper DrawText / glyph colour step / code parse / strip | 0x10073c40 / 0x10073b40 / 0x100b5fd0 / 0x100b6100 | 0x10073c10 / 0x10073b10 / 0x100b5fa0 / 0x100b60d0 |
| GUI skin / GUI fonts | 0x1008d7e0 / 0x1008d7a0 | 0x1008d7b0 / 0x1008d770 |
| chat ctor / draw / entries / line draw / metrics | 0x10068380 / 0x10067e00 / 0x10067940 / 0x10067470 / 0x10067120 | 0x10068340 / 0x10067dc0 / 0x10067900 / 0x10067430 / 0x100670e0 |
| chat RT create / RT update / scrollbar layout / page size | 0x100681d0 / 0x10067ed0 / 0x10067200 / 0x100672a0 | 0x10068190 / 0x10067e90 / 0x100671c0 / 0x10067260 |
| chat PageUp / PageDown / wheel / F7 | 0x100672d0 / 0x10067330 / 0x100673c0 / 0x100612c0 | 0x10067290 / 0x100672f0 / 0x10067380 / 0x10061280 |
| input ctor / open / close / history up / down | 0x100693d0 / 0x10069480 / 0x10069580 / 0x10069630 / 0x100696a0 | 0x10069390 / 0x10069440 / 0x10069540 / 0x100695f0 / 0x10069660 |
| dialog ctor / controls / show / layout / draw / close | 0x1006f480 / 0x1006fa20 / 0x10070010 / 0x1006f500 / 0x1006f890 / 0x1006f860 | 0x1006f450 / 0x1006f9f0 / 0x1006ffe0 / 0x1006f4d0 / 0x1006f860 / 0x1006f830 |
| dialog list parse / tablist parse | 0x1006fbf0 / 0x1006fcf0 | 0x1006fbc0 / 0x1006fcc0 |
| help dialog (F1) | 0x1006bb30 | 0x1006baf0 |
| scoreboard ctor / controls / open / close / draw / rows | 0x1006ea30 / 0x1006eab0 / 0x1006f3d0 / 0x1006e9e0 / 0x1006f0b0 / 0x1006ed30 | 0x1006e9f0 / 0x1006ea70 / 0x1006f3a0 / 0x1006e9a0 / 0x1006f080 / 0x1006ecf0 |
| nametag bars pass / names pass / bar draw / name draw / state block | 0x10074270 / 0x10075390 / 0x1006d0a0 / 0x1006cda0 / 0x1006d6b0 | 0x10074240 / 0x10075360 / 0x1006d060 / 0x1006cd60 / 0x1006d670 |
| death window ctor / fonts / draw / add / name / icon / glyph map / SAMPAUX3 fonts | 0x1006a650 / 0x1006a150 / 0x1006a2e0 / 0x1006a5d0 / 0x10069c40 / 0x10069d40 / 0x10069e50 / 0x10069bb0 | 0x1006a610 / 0x1006a110 / 0x1006a2a0 / 0x1006a590 / 0x10069c00 / 0x10069d00 / 0x10069e10 / 0x10069b70 |
| bubbles draw / 3D text draw | 0x10066ed0 / 0x1006bc90 | 0x10066e90 / 0x1006bc50 |
| textdraw pool draw / text / sprite | 0x1001e830 / 0x100b31e0 / 0x100b30d0 | 0x1001e7e0 / 0x100b31b0 / 0x100b30a0 |
| HUD scale fix / radar hook / crosshair hook | 0x100a2440 / 0x100a2540 / 0x100a2590 | 0x100a2410 / 0x100a2510 / 0x100a2560 |
| audio title / net stats | 0x10066ab0 / 0x10060d70 | 0x10066a70 / 0x10060d20 |
| class-selection panel tick | 0x10006100 | 0x100060f0 |

---------------------------------------------------------------------------------------------

## 21. Unknowns and what TRACE should measure

Unknown or unverified:
1. **Exact pixels.** All geometry above is from code. Fonts under Wine and DXVK (Arial
   availability, D3DX glyph metrics, antialiasing quality 4 vs 0) decide the real line height
   `h`, text widths and glyph placement. Nothing was rendered.
2. GUI element colour→state mapping (§4.1) is inferred from the stock DXUT element layout. So
   are the three edit-box colour slots.
3. Whether GUI list-box rows (dialog styles 2/4/5, scoreboard) interpret `{RRGGBB}`.
4. Textdraw hit-rect formulas (§14.2 step 12), model-preview texture creation, and the text vs
   sprite order relative to `DoFade`.
5. AFK icon exact rect, net-stats origin, screenshot index rule.
6. The local ped field +0x2c9 (> 5000 hides the HUD), the menu enter/leave hook
   A 0x100a04a0, and how the slider panel (0x1026e9cc) is activated.
7. The render state values used for the chat render target (`D3DRS_DESTBLEND` = 14) and the
   nametag state block. They are taken from the call arguments; the visual outcome is not
   verified.

TRACE (runtime observation on the original client; trace lane, headless display, fixed
resolutions 800×600, 1280×720 and 1920×1080):
- Screenshots of: chat with 20 coloured lines (types 2/4/8, `{RRGGBB}` mid-line, alpha in
  the server colour) in F7 modes 2/1/0; `/timestamp` on; `/pagesize 20`; `/fontsize −3/0/5`;
  scrolled up by PageUp; input open with text. Measure line pitch, left margins, outline width
  and HUD/radar visibility per mode.
- All six dialog styles with known bodies (short and long, 1 vs 2 buttons, colour codes in
  list rows, 4-column tablist, headers). Measure W/H, centring, button and edit-box rects.
  Also F1 help.
- Scoreboard at 800×600 and 1920×1080 with ≥ 3 players (bots): position, column x's, header
  colours.
- Nametags at known distances with health 100/50/0 and armour 0/100. Measure the bar quad
  pixels and text baseline. Repeat with an AFK (paused) bot and `/nametagstatus` on/off.
- Death window with killer+victim rows, reasons 0/24/31/200/201, and a long nickname at 1280
  wide.
- Textdraws: box, right/centre alignment, outline/shadow, sprite and model preview, at two
  aspect ratios with `/hudscalefix` on/off. Capture hit rects through selection hover, and
  check the text/sprite order against a fade (`FadeCamera`-style script).
- A D3D9 call log (API-level hook in the tracer) of one Present while chat, a dialog and
  nametags are visible. It confirms the draw order of §2.3, the font creation parameters
  (`D3DXCreateFontA` arguments) and the render states.

---------------------------------------------------------------------------------------------

## 22. Corrections from M8a (static analysis of the A build + TRACE-3 D3D logs)

- **Textdraw set-text (A 0x100b2fa0):** texts of ≥ 800 chars leave the textdraw empty; while scanning,
  a `~` opens a code (a following `k`/`K` marks key expansion) and the next `~` closes it; a code
  longer than 24 characters or an unclosed code → `Error: unmatched tilde`; a key text longer than 255
  → `Error: can't use k-codes in long string`.
- **Textdraw hit rects (§14.2 step 12, §14.3):** with `X`,`Y` the PrintString position, `wrap` =
  W·textSizeX·ux, `cs` = W·textSizeY·ux, all truncated with `_ftol2`: left-aligned `[X, Y, wrap, Y+cs]`;
  right-aligned `[2X−wrap, Y, X, Y+cs]`; centred `[X−cs/2, Y, (X−cs/2)+cs, Y+wrap]` (the box flag is not
  used). Sprite: `[x·W·ux, y·H·uy, W·ux·tsx + left, H·uy·tsy + top]`.
- **Menus (§12):** build order 0x08D4 (title key, x, y, (w1+w2)/columns, columns, interactive, 1, 1),
  0x08DB + 0x09DB per column (both columns get w1 as width), 0x08D9 per row. `CText::Get` hook
  (CSV row 58) answers `SAMPHED`, `SAMPRW1/2`, `SAMP0rr`/`SAMP1rr` (rr 00–11) from the current menu.
- **Nametags (§16):** anchor = bone 8 (head) position; LOS test 0x06BD (bone 8 → camera, flags
  1,0,0,1,0) runs before the NPC check, once per pass; the projection uses samp's copies of the VIEW /
  PROJECTION matrices taken in its proxy `SetTransform` (slot 44); screen coordinates are truncated.
  Name rect `[X − w/2, Y + 17 − h, X + w/2, Y]`.
- **Death window (§17):** origin `ftol(0.75·W)`, `ftol(0.3·H)` (truncation); 5 fixed slots, the newest at
  the bottom, empty slots keep their space; killer rect `[x + nickW − w, y, x + 60, y + 30]`; icon box
  `[x + nickW + 3, y − 5, +35, y + 30]`: `G` drawn, then measured into the rect (`DT_CALCRECT|NOCLIP`),
  then the reason glyph centred; victim rect `[iconLeft + iconW, y, +35, y + 30]`; names use the death
  font for outline and main (no colour wrapper).
- **Outlines (§3.4):** the 4-way passes move only left/top of the rect; the 8-way chat passes (W > 1280)
  move the whole rect.
- **Dialogs (§13.2):** caption height = main-font "Y" height + 4; the edit-box term is
  `trunc(1.6·h1 + 14)`; placement rounds `(client − size)/2`; empty list lines make no row; the
  tablist column count is the first row's cell count while the widths use up to 4 cells of every row
  (header included); the focused control (list or edit box) is drawn last.
