# Phase 3 / M4 — in-game UI, first part (chat, input, local commands, game text)

Branch `ui-render` (git worktree). Acceptance: `tools/e2e/m4.sh` (lane "ui").

## What is implemented

| area | where | spec |
|---|---|---|
| Device proxy: `_RwD3DDevice` (0xC97C28) replaced on the first frame by a 4-byte object with a 119-slot forwarding vtable; Present (17) runs the UI pass then the real Present, Reset (16) does lost/reset around the real Reset | `ui/d3d.rs` (generated thunks: `mov eax,[real]; mov [esp+4],eax; mov eax,[eax]; jmp [eax+4·slot]`) | UR §2.1 |
| D3DX from `d3dx9_25.dll` (LoadLibrary/GetProcAddress): `D3DXCreateFontA`, `D3DXCreateSprite`, `D3DXCreateTextureFromFileExA`, `D3DXLoadSurfaceFromFileA` | `ui/d3d.rs` | UR §3.2 |
| Fonts: main / main-outline / small / small-outline (face, height per W and `fontsize`, weight, charset 1, quality 4) and GUI font 0 (quality 0) with samp's `D3DXCreateFontA` arguments; rebuilt on `/fontsize` and on a video-mode width change | `ui/render.rs` | UR §3.2, TRACE-3 |
| `{RRGGBB}` colouring: the stripped text is drawn through a proxy `ID3DXSprite` whose `Draw` gives the i-th glyph the i-th colour of `samp_ui::colour::glyph_colours` | `ui/d3d.rs` `proxy_draw` | UR §3.3 |
| Outline passes (4 or 8 offsets, black, outline font, stripped text, before the main pass), mode 2 only | `ui/render.rs` | UR §3.4 |
| Chat window: layout from `samp_ui::chat`, one `Begin(ALPHABLEND)`/`End` sprite batch per Present, wrapped in our own BeginScene/EndScene | `ui/render.rs` | UR §7 |
| Scrollbar (sampgui.png track/up/down/thumb), visible when not at the bottom or the input is open | `ui/render.rs` | UR §7.4, TRACE-3 |
| Chat input edit box: 9 sampgui pieces (border 5, centre 0xF0050505, edges white), text in GUI font 0 at inset 4 with `DT_VCENTER`, caret 2 px 0xFF969696 (no blink), selection 0xFFB92228 + selected text 0xFF0A0A0F | `ui/render.rs` | UR §8, §4.1, TRACE-3 |
| D3D hardware cursor from `mouse.png` (`CreateOffscreenPlainSurface` 32×32 A8R8G8B8 SYSTEMMEM, `D3DXLoadSurfaceFromFileA` filter 1, `SetCursorProperties(0,0,…)`), re-created on Reset | `ui/d3d.rs` | UR §5, TRACE-3 |
| Cursor/input-lock modes 2 (input open, re-applied every frame) and 0 (immediate restore at the next frame entry); byte values of 0x541DF5, 0x53F417, 0x53F41F, 0x6194A0, the mouse-delta zeroing and the two `CPad` calls | `ui/inputlock.rs` (host-tested) | UR §5, gta-patches §5 |
| Window procedure subclass (`SetWindowLongA`), `WM_SETCURSOR` (hide once, answer TRUE), messages → `samp_ui::keys::route` → actions; edit keys on key-down | `ui/wndproc.rs` | UR §6 |
| RPC 93 (type 4, server colour, `~k~` via GTA 0x69E160), RPC 101 (type 2, name + colour table, connected only), RPC 137/138/11/72 for names and colours | `ui/core.rs` | UCA §93/§101 |
| Player colour table: 1000 entries from samp's 100-entry default table (A 0x10113b48, period 100), 4000 slots | `ui/colours.rs` | player-state §72 |
| Client-local lines (all type 8): "SA-MP 0.3.7-R4 Started", "Connecting to %s:%d...", "Connected. Joining the game...", "Connected to {B9C9BF}%.64s", "Server closed the connection." | `ui/core.rs` `msg` | strings A 0x100e596c… |
| RPC 73: opcode 0x00BE, text copied into a 512-byte static buffer, `CMessages::AddBigMessage(buf, time, style)` (0x69F2B0); styles 0..6 (hardened) | `ui/glue.rs` | UCA §73, UR §12 |
| Local commands through `samp_ui::commands`: `/q` `/quit` (quit via `dll::request_quit`), `/pagesize`, `/fontsize`, `/timestamp`, `/save`, `/rs`, `/rcon` (packet 201), `/mem`, `/interior`, feedback lines; `sa-mp.cfg` rewritten on every change; start-up keys | `ui/core.rs` | UCA local commands, UR §9 |
| Enter: RPC 101 for text, RPC 50 for non-local `/` lines (reliable), chat scroll to 90 (clamped) | `ui/core.rs`, `ui/glue.rs` `net_tick` | UCA §50/§101 |
| `chatlog.txt` truncated at start, one line per entry, written in C text mode (CR CR LF) | `ui/core.rs` | UR §7.1 |
| Control channel: `chat [N]`, `input`, `say TEXT` | `control.rs`, `ui/glue.rs` | — |

Hooks into existing files (kept small for the merge): `client.rs` (5 calls: `on_netgame_start`,
`on_init_game`, `on_closed`, `on_rpc`, `net_tick`), `dll.rs` (`frame_entry` per frame, the three
control commands, `request_quit` made `pub(crate)`), `control.rs` (commands), `lib.rs` (`pub mod ui`).

## Measured on the original (this lane, samp.exe 0.3.7-R4, same scenario)

A scratch run of the original client through the m4 scenario (screenshots, capture, files) gave:
- **RPC 101 / 50 wire format identical**: `C>S rel8 rpc 101 96 bits 0b68656c6c6f20776f726c64` and
  `C>S rel8 rpc 50 88 bits 070000002f6d3470696e67` from both clients.
- **Input close keeps the text.** ESC closed the input; T reopened it with "not sent" still in it,
  and the next line went out as "not sent/q". The close routine (A 0x10069580) only does focus out,
  disable, hide, cursor mode 0 and "open" := 0; Enter clears the text (A 0x100699d0) and resets the
  history position. RE-5 §8 said "clear the text": corrected (RE-5 and `samp_ui::input`).
- **Chat bottom** (+0x12e) is `10 + (pageSize + 1)·(h + 1)` (box top 175 at page 10, h 14 at W 864;
  197 at W 1280 per TRACE-3). The draw loop stores the bottom of the rect prepared for the line after
  the last one. Corrected in `samp_ui::chat::bottom` and RE-5 §7.2.
- **Startup keys**: the original wrote `fpslimit=48`, `multicore=1`, `audioproxyoff=0` (CR LF).
  `multicore=1` is created when missing (A 0x100c4e96); fixed in `samp_ui::config`.
- **Feedback line types**: which `/command` strings use the type-4 vs type-8 helper was read from the
  call after each string push; `samp_ui::commands` printed several type-8 lines as type 4 (pagesize /
  fontsize usage, /mem, /interior, /rs, debug usages). Fixed, `AppendFile` carries the kind.
- **Files**: chatlog.txt lines end in CR CR LF, savedpositions.txt in CR LF (text mode). `/save` on
  foot wrote `AddPlayerClass(0,…,355.3697,0,0,0,0,0,0); // m4` (heading in degrees, consistent with
  the ped heading).
- "Connecting to" appears ≈ 3 s after "Started" in the original (chatlog timestamps); ours prints it at
  once (see deviations).

TRACE-3 batch 1 (D3D call log of the original at 1280×720, relayed by the main session) settled
the font arguments, chat line rects/flags/outline order, the per-glyph sprite draws, the edit-box
pieces/colours/insets, caret and selection, the scrollbar geometry and thumb colour, and the cursor
calls. Our 1280×720 screenshot matches those numbers to the pixel (box 40,197–808,241; scrollbar
10,40–30,150; thumb rows 122–129; caret 2 px).

## Deviations (known, deliberate or pending)

- **Chat render-to-texture cache** (UR §7.6, 2048×1024 RT + `D3DXCreateRenderToSurface`) is not
  used: we draw directly every Present (equivalent to `directmode=1`). The visible result is the same
  except for the clip at the RT size.
- **HUD and chat mode** (UR §2.3 step 9): samp sets "HUD on := mode ≠ 0" every Present. Our frame
  hook runs `Render2dStuff` *before* the session tick (samp runs the netgame first), so doing it
  literally would show the HUD during class selection. We force the HUD off every frame in mode 0
  and switch it back on once when leaving mode 0. Revisit when the frame order is aligned.
- **"Connecting to"** is printed when the session starts, not by a 3 s connect tick (the network
  session connects immediately; not changed in this milestone).
- `KillFocus` during class selection does not re-apply cursor mode 0 (no samp class-selection GUI yet).
- No IME, clipboard (Ctrl+C/V), F8 screenshot, F9, Tab, dialogs, scoreboard, bubbles (later milestones).
- Text is drawn with `DrawTextA`; the original's edit box uses `DrawTextW` (DXUT). Same glyphs for
  the ANSI code page.
- `/fpslimit`, `/headmove`, `/hudscalefix`, `/nametagstatus`, `/audiomsg`, `/logurls`, `/togobjlight`,
  `/dl`, `/ctd` persist and print like the original but have no game effect yet. `multicore=0`
  (affinity mask 1) is not applied.

## Unknown (TODO-unknown in code; measurement requests sent to the main session)

| id | where | open question |
|---|---|---|
| U-EDIT-SCROLL | `ui/render.rs` | which characters are visible when the input text is wider than the box |
| U-SAVE-HEADING | `ui/glue.rs` `LiveGame` | heading source of `/save` in a vehicle (ours: forward vector) and the colour fields used |
| U-LOCK-INIT | `ui/inputlock.rs` | initial restore countdown (ours −1: no restore before the first mode change) |
| U-EDITKEY-CONSUME | `ui/wndproc.rs` | whether edit keys (arrows, Home/End, Delete, Insert) are passed on to GTA while the input is open |
| U-CONNECT-TIMING | `ui/core.rs` | exact timing/repeat of "Connecting to" (≈ 3 s after Started in the original) |
| U-HUD-STEP9 | `ui/glue.rs` | HUD flags per frame during class selection in the original |

## Lane setup (ui)

- `wine/prefix-ui` linked `Documents` to the user's `~/Documents` (Wine default), shared by every lane
  and by the user. Our client and the original write `GTA San Andreas User Files/SAMP/{sa-mp.cfg,
  chatlog.txt,savedpositions.txt}` there, so the lane got its own `Documents` folder (copy of
  `gta_sa.set`, fresh `SAMP/`). **The other lanes still share `~/Documents`** (the trace lane's
  original client was writing chatlog.txt there during this work); worth isolating them too.
- The lane's `gta_sa.set` was at video mode 27 (864×486); set to mode 32 (1280×720) with
  `tools/trace/setmode.py` to compare with TRACE-3 (backup `gta_sa.set.bak-864`).
- `tools/wine-env.sh` no longer overrides the caller's `ROOT` with `SAMP_RS_ROOT`; `SAMP_RS_ROOT`
  now only locates `wine/`, `server/`, `data/` (needed from a git worktree). `lib.sh` takes
  `server/` from there, `run-client.sh` the auth table; `m1.sh`–`m3.sh` accept `E2E_TAG/E2E_PORT/
  E2E_CTL_PORT` overrides (defaults unchanged) and export `SAMP_RS_CTL_PORT`.

## Test

`tools/e2e/m4.sh` (lane ui, port 7800, control 47710): connect lines; three client messages with
expected colour runs; game text accepted; T + "hello world" + Enter → server log and echoed type-2
line; `/pagesize 15` local, applied and saved; `/m4ping` round trip (RPC 50); `/save m4` (file
content, CR LF); ESC keeps text; Up/Down history; BackSpace; F7 2→1→0→2; PageUp/PageDown; `/q`
quits; capture decoded with samp-dump (RPC 101/50 layouts, nothing sent for local commands).
Screenshots `target/m4-{chat,typed,input,pagesize,mode1,mode0}.png`.
