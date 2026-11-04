# Phase 3 / M4-core — `crates/samp-ui` (renderer-independent UI logic)

Status: crate added to the workspace. 83 unit tests; `cargo clippy --workspace --all-targets` reports no warnings.
It has no Windows or D3D dependency and only depends on `samp-proto` (for the RPC 50/61/62/73/101 payload
types). Sources:
- `docs/re/subsystems/ui-rendering.md` (RE-5, primary, cited as "UR §n" in code and test names);
- `docs/re/subsystems/ui-chat-audio.md` (UCA);
- user-visible strings and format strings from the samp.dll 0.3.7-R4 string table (2021 build, file offsets
  ≈ 0xe9b80–0xea6f4). These are functional constants such as command feedback, the chatlog, `/save` and
  `sa-mp.cfg` formats, and the F1 help body.

Nothing in this crate was taken from decompiled code.

## Modules
| module | contents |
|---|---|
| `colour` | `{RRGGBB}` parse/strip; per-glyph colouring with the original cursor walk (alpha forced to FF after a code, blanks skipped); colour runs; RGBA→ARGB; default colours |
| `text` | `strlen`, control chars → spaces, 72/12-style split and wrap, `~k~` hook (`KeyNameExpander`, <400 in / ≤255 out), `atoi`, `%d` scanning |
| `fonts` | `FontId`, `TextMeasure` trait (pluggable pixel metrics) + `MonoMeasure`, font heights/weight/face (UR §3.2), outline patterns (UR §3.4) |
| `chat` | 100-entry buffer, entry types 2/4/8, prefix/text limits, RPC 93/101 rules, paging (PageUp/PageDown quirk, wheel, scroll min 1), page size setter vs command ranges, F7 modes, scrollbar visibility/rect, line layout, chatlog lines |
| `edit` | DXUT-style single-line edit box (caret, selection, word moves, overwrite, clipboard, password mask) |
| `input` | chat input: open/close, 128-char cap, 10-entry history with live line, Enter → `Submit::{Chat(RPC 101), Command(RPC 50), Local, UnknownCommand, …}`, box geometry |
| `commands` | local command table (21 normal + 6 debug-only names), case-insensitive lookup, word/args split, `execute` → typed `Effect`s, all printed strings, `/save` and `/rs` line formats |
| `config` | `sa-mp.cfg` parse/serialise (typed int/string/float with the writer's formats), start-up defaults, `UiSettings` |
| `gametext` | RPC 73 acceptance (1..200 chars, style < 201, hardened to 0..6), GTA big-message style table |
| `bubbles` | 1004 slots, RPC 59 limits, wrap, expiry, distance test, anchor height formula |
| `dialog` | all six styles, row/cell parsing, list keyboard, responses as `OnPlayerDialogResponse`, geometry (UR §13.2), F1 help dialog |
| `scoreboard` | rows (local first, no NPCs), sort modes, toggle, RPC 155 timing, geometry (UR §15.1) |
| `deathwindow` | 5 rows, reason→glyph table (UR §17.2), glyph colours, origin and row layout |
| `keys` | window-message routing per UR §6.3 → typed `Action`s; Present frame plan per UR §2.3 |

## How samp-client uses it (M4/M8a)
- Implement `TextMeasure` on top of the D3DX fonts. Strip colour codes before measuring, as the layouts do.
  Draw `TextItem`/`DialogText` through `colour::glyph_colours`.
- Translate window messages into `keys::Event`, call `keys::route`, then apply the `Action`s to `ChatInput`,
  `ChatWindow`, `Dialogs`, `Scoreboard` and `DeathWindow`.
- `ChatInput::submit` returns typed RPC payloads. Local commands go through `commands::execute`, whose
  `Effect`s the host carries out: rewrite the config, append to a file, quit, send packet 201, rebuild fonts.

## Unknown / assumed behaviour (for TRACE-3 to confirm)
Every item below is a named constant or a commented code path tagged with the same id.

| id | where | assumption |
|---|---|---|
| U-ROUND | `lib.rs round_i` | layout `round()` = half away from zero |
| U-HEXCASE | `colour::parse_code` | hex digits in `{RRGGBB}` accepted in both cases |
| U-STRIP | `colour::strip_codes` | in-place removal; scan resumes at the same index (a code formed by a removal there is removed too) |
| U-KEXP | `text::KEY_EXPANSION_MAX_OUTPUT` | expanded `~k~` text kept if `len <= 255` |
| U-PAGESIZE-SCROLL | `chat::PAGESIZE_CHANGE_SCROLLS_TO_BOTTOM` | a page-size change scrolls to the bottom (alternative: only clamps) |
| U-CHATBOTTOM | `chat::ChatWindow::bottom` | chat bottom (+0x12e) counts every page line, filled or not |
| U-CHATLOG-NAME | `chat::ChatEntry::chatlog_line` | chatlog shows `<name>` without the stored `:` |
| U-CHATLOG-MODE | `chat::CHATLOG_LINE_END` | lines end in `\r\n` (format string); if the file is opened in text mode the disk has CR CR LF |
| U-CHAT-LONGNAME | `chat::add_player_chat` | wrapped halves are added independently (a >24-char name drops only line 1) |
| U-EDITKEYS | `edit` | stock DXUT edit-box keys; a word stop is a non-blank character after a blank |
| U-MASK | `edit::PASSWORD_MASK` | password mask `*` |
| U-INPUT-CAP | `input::EDIT_CAPACITY` | chat edit box stops at 128 chars (alternative: truncation only on Enter; same bytes sent) |
| U-HISTORY-DUP | `input::HISTORY_SKIPS_DUPLICATES` | repeated lines are pushed again |
| U-HISTORY-RESET | `input::ChatInput::close` | closing resets the history position to the live line |
| U-F6 | `keys::key_up` | F6 opens/closes with no condition besides the input existing |
| U-CFG-READ | `config::SampCfg::parse` | `key=value` lines, trimmed, case-sensitive keys, `"…"` = string, number with `.` = float, other numbers = int, a later duplicate wins |
| U-CFG-CRLF | `config::LINE_END` | writer output uses CR LF (mode `"w"` is text mode) |
| U-CFG-STARTUP | `config::apply_startup_defaults` | only `fpslimit=48` and `audioproxyoff=0` are created; new keys are appended (RE-6) |
| U-FONTSIZE-STARTUP | `config::UiSettings::from_cfg` | stored `fontsize` not range-checked at start-up |
| U-TOGOBJLIGHT | `config::UiSettings::object_light_flag` | flag starts false; visible effect unknown |
| U-RS-ERROR | `commands::msg::RAW_OPEN_FAILED` | the single raw-save error string is used for both raw files |
| U-SAVE-NOPED | `commands::execute` | `/save`/`/rs` without a local ped do nothing |
| U-DEBUGCMDS | `commands::execute` | debug commands: usage text when the number is missing, `/set_time` range check after reading two ints |
| U-BUBBLE-WRAP | `bubbles::WRAP_BACKTRACK` | helper args (36, 12) read as (line width, space look-back), applied repeatedly; UCA believed 12 = max lines |
| U-GAMETEXT-DURATION | `gametext::Duration` | style durations/animations as in GTA's `CHud` (GTA-owned; confirm visually) |
| U-DW-SLOTS | `deathwindow::layout` | with fewer than 5 rows, the stored rows fill from the top |
| U-SB-FIRST-REQUEST | `scoreboard::REQUEST_ON_OPEN` | RPC 155 goes out on the first open frame |
| U-SB-SORT | `scoreboard::rebuild` | sort modes (unused in R4) sort all rows, stable |
| U-DLG-STYLE | `dialog::Style::Other` | style > 5 opens like a message box with no body drawn |
| U-DLG-EDITCAP | `dialog::EDIT_CAPACITY` | dialog edit box accepts 128 chars |
| U-DLG-CAPTION | `dialog::layout` | control coordinates are relative to the area below the caption (DXUT) |
| U-LIST-BORDER | `dialog::LIST_BORDER` | DXUT list border 6 in `LH` |
| U-LIST-TRAILING | `dialog::SKIP_TRAILING_EMPTY_ROW` | a trailing `\n` adds no empty row; inner empty lines are rows |
| U-LIST-CODES | `dialog::LIST_ROWS_INTERPRET_COLOUR_CODES` | list rows interpret `{RRGGBB}`. **Differs from RE-5's tentative "not interpreted"** (UR §13.3/§21.3); chosen because servers colour rows and the response strips codes |
| U-LIST-KEYS | `dialog::Dialog::list_key` | DXUT list keys: Up/Down ±1, PageUp/PageDown ±(page − 1), Home/End, clamped |
| U-TAB-EDGES | `dialog::split_cells` | leading/trailing tabs give no cell; cells beyond 4 are dropped |
| U-TAB-COLUMNS | `dialog::Dialog::new` | style 4 column count = widest row |
| U-TAB-HEADERW | `dialog::layout` | style 5 header cells count towards column widths |
| U-TAB-LASTCOL | `dialog::layout` | the 4th column width is the remaining list width |

Also open from RE-5 (not modelled here, renderer side): exact pixel metrics under Wine/DXVK, GUI element
colour-to-state mapping, the textdraw hit rect, the AFK icon rect, the net-stats origin and the screenshot index
rule.

## Out of scope for M4-core
Textdraws (M8a, including the tilde validation rules), GTA menus (GTA draws them), nametags, net stats, audio
stream, and the `/testdw` row contents. For `/testdw` the crate only emits `Effect::FillTestDeathWindow`; the
test rows use the strings `PooperPooperPooper0001`, `Pooper` and `Pooper333`, and their pattern is not specified.
