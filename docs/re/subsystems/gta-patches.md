# GTA:SA memory patches applied by samp.dll 0.3.7-R4 (clean-room notes)

Targets: A = samp.dll fv0.3.7.4 (2021), B = samp.dll fv0.3.7.3 (2019); game = gta_sa.exe 1.0 US (EU variants noted where the DLL branches).
Machine-readable list: `docs/re/data/gta-patches-r4.csv` (one row per write event; the tables below are generated from it, with
table-driven pointer redirects grouped into one row).

Everything here is a description in our own words of *what* bytes end up in game memory and *when*. Purposes marked
"inferred" come from public knowledge of the GTA:SA 1.0 US layout (not from this DLL) and are not verified; "unknown" means
no evidence was collected. The tables below were written without `gta_sa.exe`. Original bytes and the GTA-side
purposes were added later from `re/gta/1.0-US/gta_sa.exe` (see "Purpose analysis" at the end and
`data/gta-patches-r4-annotated.csv`, which take precedence over the `purpose` column where they differ).

## 1. Method and coverage

* A linear-sweep constant-propagating scan over `objdump -d` of both DLLs (scratch script, not part of the repo) recorded
  every store whose resolved target lies in 0x00400000–0x015FFFFF: absolute stores, stores through a register that holds a
  constant, `rep stos`/`rep movs` with known edi/ecx, every `VirtualProtect` call with a game-range address, and every call to
  the four patch helpers below with a game-range first argument. Register state is reset at function boundaries
  (taken from the decompile index), so code that Ghidra did not index (hook bodies, a few small functions) was still
  scanned but attributed manually.
* Loops that walk address tables stored in samp.dll `.data` were resolved by reading the tables from the file
  (section 4.3). Writes whose value is only known at runtime are listed with a placeholder.
* Results were cross-checked against the decompiled functions of A for the main patch routines (0x100aa750, 0x100aaef0,
  0x100a6320 and their callees).
* **Not covered / limits:** writes through pointers computed at runtime (e.g. game objects reached through pools) are
  normal game-state access, not patches, and are out of scope; game data variables that samp writes at runtime are only listed
  where they are paired with code patches or are clearly global settings (gravity, clock format, stunt bonus, …). Anti-cheat /
  integrity logic was not analysed; the one place where samp reads back patched code (the auth handler) is only noted.

## 2. When patches are applied (A addresses; B = same flow)

1. **DLL attach** (A 0x100c5100): constructs the game wrapper object (stored at 0x1026ea24) and starts a thread at A 0x100c3ac0
   (`_beginthread`).
2. **Init thread, early** (A 0x100c3ac0 → 0x100a08f0 → **0x100aa750**, "init-early"):
   * Version probe: polls the byte at game 0x747483 every 10 ms (max 6000 tries ≈ 60 s; aborts early if game byte 0xBA6831 == 1):
     0x89 → 1.0 US (internal flag 0x1026e9a0 := 1), 0xC8 → 1.0 EU (flag := 2). Failure shows a message box
     ("…only supports GTA:SA v1.0 USA/EU") and exits the process. The flag selects US/EU addresses in later patches.
   * Applies the early patch block (startup/intro skips, streaming memory budget from physical RAM, a few NOPs).
     Two sites are written only after the byte at the site equals an expected value (helper 0x100aa4b0, which spins with
     `Sleep(1)`), i.e. after the packed game code has been decoded.
3. Init thread then **waits until game byte 0xC8D4C0 == 7** (5 ms sleeps; inferred: game state "frontend idle"), calls
   0x1009ddb0, then **0x100a0940** ("init-main"):
   * 0x100aaef0: sub-routines 0x100aaac0 (sector-array relocation), 0x100aaa00 (ped-model store), 0x100aada0 (colour table),
     0x100aaa50 (atomic-model store), 0x100aa5d0 (pool sizes); then ~95 inline byte/NOP/pointer patches; then
     0x10002100 (two pointer redirects into samp).
   * 0x100a6320: first 0x100a6130 (per-frame / HUD / ped-render hooks), then all remaining hooks.
   * 0x100b2590, 0x1009cf60 (not analysed here), then writes game state 0xC8D4C0 := 8 and three menu flags.
   * The thread then exits.
4. **Per frame**: the call retarget at 0x53EB12 enters A 0x100c5030 → 0x100c4fb0 → 0x100c47d0. The first time through,
   0x100c47d0 creates config/chat-log paths, the netgame, replaces the game pointer at 0xC97C28 with a samp wrapper object,
   and calls a few runtime patchers (0x100a0c40, 0x100a0b60(0), 0x100a14a0(0), 0x100a0c10).
5. **InitGame RPC 139** (A 0x10010660): 0x100a1c10 (two functions disabled), 0x100a7050 (8 call hooks in 0x736xxx–0x742xxx),
   0x100a7200 (one jmp hook at 0x6D7C90), gravity/stunt-bonus writes, and 0x100088f0 → 0x100aac20 (vehicle-light code
   rewrite, only if the "manual vehicle engine/lights" setting at netgame settings +0x2b is set; guarded to run once).
6. **Lazily, once**: A 0x1000bd60 (model-cache directory setup, reached from two unindexed handlers) → 0x100a7a40: relocates
   the 20000-entry model-info pointer array and patches 707 code references (section 4.4), then 0x100a7030 adds one more
   indirect-jmp hook (0x4087EA).
7. **Runtime toggles** (section 5): input lock/unlock, HUD clock, gravity, stunt bonus, wanted level, crime report, spawn,
   skin change, plus temporary patch/unpatch pairs inside hook bodies.

## 3. Patch helper functions

| A | B | signature (inferred) | behaviour |
|---|---|---|---|
| 0x100aa500 | 0x100aa4d0 | `unprotect(addr, len)` | VirtualProtect(addr, len, PAGE_EXECUTE_READWRITE); old protection is **not** restored. Caller then stores the bytes inline. 47 call sites in A. |
| 0x100aa4b0 | 0x100aa480 | `wait_then_unprotect(addr, len, expected_byte)` | VirtualProtect RWX; if `*addr != expected` sleeps 1 ms in a loop until it is, then VirtualProtect RWX again. Used for 0x551024 (0x68), 0x561872 (0x85), 0x4083C0 (0xB8): waits for the packed executable to be decoded. |
| 0x100aa160 | 0x100aa130 | `patch_copy(addr, src, len)` | VirtualProtect RWX, copy `len` bytes from `src`, restore old protection. Only caller is dead code (0x100aa1b0). |
| 0x100a6080 | 0x100a6050 | `install_indirect_jmp(site, handler, slot, stub, stub_len)` | Stores the samp `handler` address into the 4-byte game location `slot` (protect/restore), then copies `stub` (from samp `.data`) over `site` (protect/restore). Every stub is `FF 25 <slot>` (= `jmp dword [slot]`) followed by 0–3 NOPs, so the stub already encodes the slot address. 18 sites. |
| 0x100de180 / 0x100de200 | (B not checked) | scoped protection guard (ctor/dtor) | ctor: FlushInstructionCache + VirtualProtect RWX, saving old protection; dtor: FlushInstructionCache + restore. Used by 0x100de010 / 0x100de2f0; no game-range constant addresses reach it in the scan (not a GTA patch site as far as observed). |
| (inline) | (inline) | call/jmp hook | Pattern repeated inline in 0x100a6130, 0x100a6320, 0x100a7050, 0x100a7200: VirtualProtect(site, 5, RWX); `site[0] = 0xE8` (call) or `0xE9` (jmp); `site[1..4] = handler − (site + 5)`; restore protection. |
| (inline) | (inline) | pointer redirect | VirtualProtect(site, 4, RWX); `*(u32*)site = samp address`; restore (in 0x100a6130/0x100a6320) or leave RWX (0x100aaef0 and its callees). Used for vtable slots, operand pointers in instructions, and float-constant operand pointers. |
| (inline) | (inline) | byte/NOP patch | VirtualProtect(site, n, RWX) without restore, then stores (NOP runs are written as dword/word/byte stores or `rep stosd`). |

Patch kinds used in the table: `nop` (all 0x90), `byte-patch` (other constant bytes), `hook-call`/`hook-jmp` (5-byte rel32
to a samp handler; handler addresses given for A and B), `hook-jmp-indirect` (helper 0x100a6080), `call-retarget` (only the
rel32 of an existing call changed), `pointer-redirect` (4-byte absolute pointer into samp code or data),
`data-write` / `data-fill` (game data, not code).

## 4. Notable structured patches

### 4.1 Pool capacities (A 0x100aa5d0, init-main; inferred CPools::Initialise at 0x550F10)
Pool-constructor arguments inside the game's pool-initialisation routine are rewritten: 0x550F46 := 100000,
0x550F82 := 8000, 0x550FBA := 5000, 0x550FF2 := 240 (byte), 0x551024 := `push 0; push 710` (7 bytes from samp .data
0x101166d8), 0x55105F := 20000, 0x551097 := 3000, 0x5510CF := 4000, 0x551106 := `push 32511` (5 bytes from .data 0x101166e0),
0x551140 := 5, 0x551178 := 1, 0x551283 := 240 (byte); plus 0x54F3A1 := 6000. Which pool each value belongs to is
inferred only (column `purpose`).

### 4.2 Model-info stores (init-main)
* 0x100aaa50: a samp array at 0x101cea50 = { u32 count; 20000 × 32-byte entries } is initialised with vtable 0x85BBF0 in
  each entry and zeroed fields; 14 code references (list at samp .data 0x10116720) are pointed at it (inferred atomic model
  info store enlarged).
* 0x100aaa00: samp array at 0x101c9590 = { u32 count; 319 × 0x44-byte entries } with vtable 0x85BDC0; reference 0x4C67AD
  plus 4 more from 0x100aaac0's list (0x5634A6, 0x5638DF, 0x56420F, 0x564283) point at it (inferred ped model info store).

### 4.3 Sector array relocation (0x100aaac0, init-main)
A 160000-byte samp buffer at 0x101a2490 is zeroed. Code references are redirected to it from three .data tables
(US / EU variants for the first two): 14 absolute pointers (list 0x10116758 US / 0x10116790 EU), 56 imm32 operands at
instruction+3 (0x101167c8 / 0x101168a8) → base; 11 imm32 operands at instruction+3 (0x10116988) → base+4;
0x40936A → base+4; 0x564DC7 → base+115200. The original 115200-byte game array at 0xB7D0B8 is then zeroed
(inferred CWorld sector grid: 14400 × 8-byte entries). The EU tables differ from US only in 0x408259→0x408261,
0x84E9C2→0x84EA02, 0x85652D→0x85656D and 0x1566855→0x1566845.

### 4.4 Model-info pointer array (0x100a7a40 + 0x100a79b0, once)
Zeroes a 65535-entry samp array (0x10162428…0x101a2424), copies 20000 pointers from game 0xA9B0C8 (inferred
CModelInfo pointer table) to the samp array starting at 0x10182424 (pointer kept at .data 0x10114b08), so indices
−32767…32767 are addressable. Then walks 707 records of 5 bytes {u32 code address, u8 expected first opcode byte} at
.data 0x10114b10 (US) / 0x101158e0 (EU). A record is patched only if the opcode in memory matches: 0x8B/0x89/0x39 → imm32 at
+3, 0xBE/0xBF → imm32 at +1, written with the new base. Afterwards sets samp flag 0x101a2424 := 1. The tables are byte-identical
in A and B (same .data addresses). The individual 707 sites are not repeated in the CSV (one aggregated row).

### 4.5 Vehicle colour table (0x100aada0, init-main)
1024 bytes (256 × 4) are copied from samp .data 0x101169c8 to 0x1026ae50 and 7 operand pointers are redirected
(0x44B1C1, 0x4C8390, 0x4C8399 (+1), 0x4C83A3 (+2), 0x5817CC, 0x582176, 0x6A6FFA). Inferred: vehicle colour palette
enlarged to 256 entries; the 1024-byte table is interop data the Rust client must ship (read it from the DLL's .data).

### 4.6 Streaming memory (0x100aa750)
Physical RAM in MB from GlobalMemoryStatusEx: > 4000 → 0x40000000; > 2000 → 0x20000000; > 1000 → 0x10000000;
> 500 → 0x08000000; else 0x06000000; written as imm32 at 0x5B8E6A.

### 4.7 Hooks overview
41 `call` hooks, 4 `jmp` hooks, 18 indirect-jmp stub rows (17 per game version: the 0x7FB020/0x7FB060 pair is US/EU),
1 call retarget, and 39 vtable/pointer redirects into samp code (37 from 0x100a6130/0x100a6320, 2 from 0x10002100). Handler addresses in B are A − 0x30 throughout this code region. Vtable redirects follow a clear
pattern: one handler for slot +0x28 (index 10, inferred ProcessControl) of 10 vehicle classes; per-class handlers for slot +0x58
(index 22) of the vehicle classes that override it; one handler for slot +0x48 (index 18, inferred Render) of five ped classes;
slot +0x28 of the player-ped and one other ped class. Class identities are inferred from public vtable addresses.

Two sites are patched twice during init (order matters): 0x609A4E is NOPed (6) by 0x100aaef0 and then receives a call hook in
its first 5 bytes from 0x100a6320; 0x6F8CF8 receives NOP×5 + `mov eax,0x6F8F89; jmp eax` from 0x100aaef0 and then a call hook
over its first 5 bytes. 0x6884C4 / 0x688200 are NOPed at init and toggled at runtime by hook bodies.

VirtualProtect-only (no init write) sites in 0x100aaef0, written later at runtime: 0x541DF5 (5), 0x858B90 (4),
0x859520 (8), 0xC17044 (8, read as screen width/height), 0x47BF54 (4).

## 5. Runtime patches (summary)

* **Input lock** (A 0x100a0750(mode), unlock in per-frame 0x100a0630, also 0x100c4570): NOPs the call at 0x541DF5,
  NOPs/rewrites mouse-update code at 0x53F417/0x53F41F, redirects three mouse-state operand pointers (0x53F47A/0x53F49A/0x53F4B3)
  to a samp dummy (mode 4), zeroes the game's mouse state, calls game 0x541BD0/0x541DD0 directly, and makes 0x6194A0 a `ret`.
  Unlock restores the original bytes (constants in the DLL: `E8 46 F3 FE FF` at 0x541DF5; at 0x53F417 `E8 B4 7A 20 00` US or
  `E8 04 7B 20 00` EU; `85 C0 0F 8C` at 0x53F41F; 0xE9 at 0x6194A0; game pointers 0xB73410/0xB73414/0xB7340C). Requesting
  mode 0 only arms a countdown (10 frames when the second argument is 0, otherwise 0); the per-frame routine performs the
  restore when it reaches zero.
* **HUD clock** (TogglePlayerClock RPC 30 and init paths, 0x100a14a0 / 0x100a0b60): 0x52CF10 = 0x56 (on) / 0xC3 (off) and the
  string at 0x859A6C = "%02d:%02d" (on) / zeros (off).
* **Gravity** (InitGame, SetPlayerGravity RPC 146): float at 0x863984. **Stunt bonus** (InitGame, RPC 104): 0xA4A474.
  **Wanted level** (RPC 133): byte 0x58DB60 (immediate of the init patch at 0x58DB5F).
* **Crime report** (RPC 112): NOP 39 bytes at 0x4E7529 then call game 0x4E71E0.
* **Spawn** (0x10003c20): 0x50AC20 := `C2 08 00`; A additionally writes the restart override block (0xA4326C/0xA43318/0xA432E8/0xA432C0),
  where B instead invokes a game script command with the same position+heading.
* **Temporary patches inside hook bodies** (applied, original called, restored): 0x6884C4/0x688200 (x87 store NOPed),
  0x5E92F4 (call NOPed), 0x858EF0 := 2.0, 0x858BA4 := 40.0 & 0x858F84 := 50.0, 0x4096AA (3-byte call NOPed),
  0x71791F (call NOPed), 0x5543C0/0x5543DC (24 bytes NOPed), 0x46977F jge↔jne, 0x41B1D0 ret↔0x64.
* **Aspect ratio** (0x100a2440, from hooks at 0x53E930/0x58FC53/0x58FBBF, when a samp option is enabled and screen
  width/height ≥ 1.6): rewrites two screen-scale floats at 0x859520/0x859524 (second = bit pattern 0x3B1E98DD, first =
  0.00222 / aspect) and two HUD floats at 0x866B74/0x866B78 (82.0, 96.0; otherwise 76.0, 94.0).
* The auth-challenge handler (A 0x1000a100 → 0x100576f0) reads 20 bytes of game code at 0x4D3AA0 (an indirect-jmp hook site);
  see handshake.md §4. Not analysed further here.

## 6. Dead code (present in both builds, never referenced)
0x100aa1b0 (allocates a buffer and patches 0x40619B/0x405C3D plus 4-byte cells in 0x156xxxx), 0x100aa2c0 (alternate store
allocator, 8 sites), 0x100aa520 (pool-size changer: 0x411458 and 7 imm32 operands from .data 0x101166f8), 0x100aa580 (duplicate of
the fifteen `:= 3` writes at 0x71A2xx), 0x100aae70 (reduced init: two NOPs + the same sub-routines). They have no call, jump or
data reference in A (checked by disassembly grep and raw pointer search) and are not needed for compatibility.

## 7. A vs B
* The scan finds the **same set of game addresses, sizes and constant bytes** in both builds (822 vs 818 merged records;
  after normalising rel32 and samp pointers, the only difference is the A-only spawn helper 0x100ac790 above).
* Hook/pointer targets differ only by the samp-side relocation (B = A − 0x30 in 0x100a2xxx–0x100a6xxx; samp data targets
  such as 0x101a2490, 0x101c9590, 0x101cea50, 0x1026ae50, 0x10182424, 0x10116718… are at identical addresses).
* All game-address tables in samp .data (0x10114b10, 0x101158e0, 0x101166f8, 0x10116720…0x101169c4) and the colour table at
  0x101169c8 are byte-identical in A and B.
* Helper addresses: B = A − 0x30 (0x100aa4d0, 0x100aa480, 0x100aa130, 0x100a6050).

## 8. Implications for the Rust client (inferred)
* Reproduce the init-early block once the game code is decoded (poll the version byte, honour the three expected-byte waits),
  then wait for game state 7 before the init-main block, apply in the same order (sub-routines, inline patches, then hooks —
  two sites depend on that order), and finally set state 8.
* Data the client must carry: the 707-record and sector/model reference tables, the 256-entry colour table, the
  indirect-jmp stub bytes, and the original bytes used for restore operations — all are interop constants readable from the
  DLL's .data at the addresses given.
* Most hook purposes are unknown; the Rust implementation needs each hook's behaviour analysed separately before it can
  replace the samp handler.

## 9. Unknowns
* ~~Purpose of most NOP/byte patches and hooks~~ and ~~meaning of the pool indices and the 0x54F3A1, 0x731F60,
  0x53E94C values~~ are resolved against gta_sa.exe. See "Purpose analysis" below and
  `data/gta-patches-r4-annotated.csv`. The GTA side of every row is now described. What remains open is listed under
  "Remaining unknowns" at the end of that section (mainly the samp-side hook handler bodies).
* What 0x100b2590 and 0x1009cf60 (called in init-main after the hooks) and 0x1009ddb0 do; they were not in this scope.
* Callers of 0x1000bd60 (0x1000eba0, 0x10011eb0) are not directly referenced; when exactly the model-pointer relocation happens
  in a session was not pinned down.

## 10. Full table

#### applied_when = init-early*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0xc8d4c0 | 1 | byte-patch | `05` | 0x100aa808 | 0x100aa7d8 | game-state variable set to 5 (inferred gGameState; skips startup logo/intro states) (medium) |
| 0x747483 | 6 | nop | `909090909090` | 0x100aa80f | 0x100aa7df | 6-byte NOP in startup code; byte here (0x89 US / 0xC8 EU) is also the GTA version probe (EU: 0x7474D3) (medium) |
| 0x866cd8 | 6 | byte-patch | `7469746c6500` | 0x100aa83f | 0x100aa80f | string patched to "title" (inferred: startup movie filename -> skip logo/intro movies) (medium) |
| 0x866ccc | 6 | byte-patch | `7469746c6500` | 0x100aa856 | 0x100aa826 | string patched to "title" (same as above, second movie name) (medium) |
| 0x745b87 | 68 | nop | `90 x68` | 0x100aa882 | 0x100aa852 | 68-byte NOP in startup code (unknown; inferred WinMain/device-setup region) (low) |
| 0x7459e1 | 2 | nop | `9090` | 0x100aa8a5 | 0x100aa875 | 2-byte NOP in startup code (low) |
| 0x561872 | 29 | byte-patch | `33c0909090909090909090909090909090909090…` | 0x100aa8af | 0x100aa87f | xor eax,eax + 27 NOP (waits until byte==0x85 first; inferred: removes a focus/pause-on-inactive check) (low) |
| 0x5b8e6a | 4 | byte-patch | `<RAM-dependent>` | 0x100aa97f | 0x100aa94f | imm32 = streaming memory budget chosen from physical RAM (>4000MB:0x40000000, >2000:0x20000000, >1000:0x10000000, >500:0x8000000, else 0x6000000) (inferred CStreaming memory limit) (high) |
| 0x4083c0 | 1 | byte-patch | `c3` | 0x100aa99b | 0x100aa96b | ret (waits until byte==0xB8 first): function disabled (low) |
| 0x590099 | 5 | nop | `9090909090` | 0x100aa9b7 | 0x100aa987 | 5-byte NOP (call removed) (low) |
| 0x53e94c | 1 | byte-patch | `02` | 0x100aa9d1 | 0x100aa9a1 | byte := 2 (low) |
| 0x731f60 | 4 | byte-patch | `204e0000` | 0x100aa9dd | 0x100aa9ad | dword := 20000 (low) |

#### applied_when = init-main*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x47bf54 | 4 | pointer-redirect(A:0x10002090 B:0x10002090) | `<samp ptr>` | 0x10002100 | 0x10002100 | pointer -> samp 0x10002090 (A) (VirtualProtect done in 0x100aaef0 just before) (medium) |
| 0x47bf88 | 4 | pointer-redirect(A:0x10001c30 B:0x10001c30) | `<samp ptr>` | 0x1000210a | 0x1000210a | pointer -> samp 0x10001c30 (A) (no VirtualProtect seen; target page must already be writable) (medium) |
| 0x53eb13 | 4 | call-retarget(A:0x100c5030 B:0x100c5010) | `<rel32>` | 0x100a6163 | 0x100a6133 | rel32 of existing CALL at 0x53EB12 retargeted to samp per-frame entry (A 0x100c5030, via var 0x1026e9b4); inferred Idle() late-frame call (high) |
| 0x58fc53 | 5 | hook-call(A:0x100a2540 B:0x100a2510) | `e8<rel32>` | 0x100a6182 | 0x100a6152 | hook (inferred HUD draw); body runs aspect-ratio constant update (medium) |
| 0x58fbbf | 5 | hook-call(A:0x100a2590 B:0x100a2560) | `e8<rel32>` | 0x100a61b5 | 0x100a6185 | hook (inferred HUD draw); body runs aspect-ratio constant update (medium) |
| 0x53c104 | 5 | hook-call(A:0x100a2670 B:0x100a2640) | `e8<rel32>` | 0x100a61e8 | 0x100a61b8 | unknown (-) |
| 0x53e981 | 5 | hook-call(A:0x100a26a0 B:0x100a2670) | `e8<rel32>` | 0x100a6219 | 0x100a61e9 | per-frame hook (inferred Idle() -> game-process call); body also toggles 0x6884C4/0x688200 and 0x5E92F4 (medium) |
| 0x86d1b0 | 4 | pointer-redirect(A:0x100a3e70 B:0x100a3e40) | `<samp ptr>` | 0x100a6245 | 0x100a6215 | vtable slot +0x48 (index 18, inferred Render) of player-ped class (medium) |
| 0x86c0f0, 0x86c168, 0x86c248 | 4 | pointer-redirect(A:0x100a3e70 B:0x100a3e40) | `<samp ptr>` | 0x100a626b (+2 more) | 0x100a623b (+2 more) | vtable slot +0x48 (inferred Render) of a ped class (medium) |
| 0x86c3a0 | 4 | pointer-redirect(A:0x100a3e70 B:0x100a3e40) | `<samp ptr>` | 0x100a62dd | 0x100a62ad | vtable slot +0x48 (inferred Render) of base ped class (medium) |
| 0x53e930 | 5 | hook-call(A:0x100a2420 B:0x100a23f0) | `e8<rel32>` | 0x100a630d | 0x100a62dd | hook; body runs aspect-ratio constant update (0x100a2440) (medium) |
| 0x4d3aa0 | 10 | hook-jmp-indirect(A:0x100a4210 B:0x100a41e0) | `ff2534394d0090909090` | 0x100a6340 | 0x100a6310 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4d3934.  () |
| 0x4d4610 | 7 | hook-jmp-indirect(A:0x100a42c0 B:0x100a4290) | `ff2509464d0090` | 0x100a635b | 0x100a632b | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4d4609.  () |
| 0x86d190 | 4 | pointer-redirect(A:0x100a29a0 B:0x100a2970) | `<samp ptr>` | 0x100a6387 | 0x100a6357 | vtable slot +0x28 (index 10, inferred ProcessControl) of player-ped class (medium) |
| 0x86c0d0 | 4 | pointer-redirect(A:0x100a2ca0 B:0x100a2c70) | `<samp ptr>` | 0x100a63ad | 0x100a637d | vtable slot +0x28 (inferred ProcessControl) of a ped class (medium) |
| 0x86d744 | 4 | pointer-redirect(A:0x100a2e20 B:0x100a2df0) | `<samp ptr>` | 0x100a63d3 | 0x100a63a3 | unknown (-) |
| 0x7330a2 | 5 | hook-call(A:0x100a2d80 B:0x100a2d50) | `e8<rel32>` | 0x100a6404 | 0x100a63d4 | unknown (-) |
| 0x86d194 | 4 | pointer-redirect(A:0x100a4130 B:0x100a4100) | `<samp ptr>` | 0x100a642d | 0x100a63fd | vtable slot +0x2C of player-ped class; body makes 0x6D64F0 a ret (low) |
| 0x5689fd | 5 | hook-call(A:0x100a4990 B:0x100a4960) | `e8<rel32>` | 0x100a645e | 0x100a642e | unknown (-) |
| 0x53ea03 | 5 | hook-call(A:0x100a2640 B:0x100a2610) | `e8<rel32>` | 0x100a6491 | 0x100a6461 | unknown (-) |
| 0x871148 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a64b9 | 0x100a6489 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871120 (inferred CAutomobile) (medium) |
| 0x8721c8 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a64df | 0x100a64af | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x8721A0 (inferred CBoat) (medium) |
| 0x871388 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a6505 | 0x100a64d5 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871360 (inferred CBike) (medium) |
| 0x871970 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a652b | 0x100a64fb | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871948 (inferred CPlane) (medium) |
| 0x8716a8 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a6551 | 0x100a6521 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871680 (inferred CHeli) (medium) |
| 0x871550 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a6577 | 0x100a6547 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871528 (inferred CBmx) (medium) |
| 0x871800 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a659d | 0x100a656d | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x8717D8 (inferred CMonsterTruck) (medium) |
| 0x871b10 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a65c3 | 0x100a6593 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871AE8 (inferred CQuadBike) (medium) |
| 0x872398 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a65e9 | 0x100a65b9 | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x872370 (inferred CTrain) (medium) |
| 0x871c50 | 4 | pointer-redirect(A:0x100a3140 B:0x100a3110) | `<samp ptr>` | 0x100a660f | 0x100a65df | vtable slot +0x28 (index 10, inferred ProcessControl) of vehicle class @0x871C28 (inferred CTrailer) (medium) |
| 0x501b1d | 5 | hook-call(A:0x100a3690 B:0x100a3660) | `e8<rel32>` | 0x100a6640 | 0x100a6610 | unknown (-) |
| 0x501b42 | 5 | hook-call(A:0x100a3690 B:0x100a3660) | `e8<rel32>` | 0x100a6674 | 0x100a6644 | unknown (-) |
| 0x501fc2 | 5 | hook-call(A:0x100a3690 B:0x100a3660) | `e8<rel32>` | 0x100a66a7 | 0x100a6677 | unknown (-) |
| 0x502067 | 5 | hook-call(A:0x100a3690 B:0x100a3660) | `e8<rel32>` | 0x100a66da | 0x100a66aa | unknown (-) |
| 0x5021ae | 5 | hook-call(A:0x100a3690 B:0x100a3660) | `e8<rel32>` | 0x100a670e | 0x100a66de | unknown (-) |
| 0x5869bf | 5 | hook-call(A:0x100a3f40 B:0x100a3f10) | `e8<rel32>` | 0x100a6741 | 0x100a6711 | unknown (-) |
| 0x5759e4 | 5 | hook-call(A:0x100a3f40 B:0x100a3f10) | `e8<rel32>` | 0x100a6770 | 0x100a6740 | unknown (-) |
| 0x609a4e | 5 | hook-call(A:0x100a3f60 B:0x100a3f30) | `e8<rel32>` | 0x100a67a8 | 0x100a6778 | unknown (-) |
| 0x4579c6 | 5 | hook-jmp(A:0x100a3f80 B:0x100a3f50) | `e9<rel32>` | 0x100a67db | 0x100a67ab | unknown (-) |
| 0x73c252 | 5 | hook-jmp(A:0x100a3db0 B:0x100a3d80) | `e9<rel32>` | 0x100a680e | 0x100a67de | unknown (-) |
| 0x73ace2 | 5 | hook-call(A:0x100a3da0 B:0x100a3d70) | `e8<rel32>` | 0x100a6842 | 0x100a6812 | unknown (-) |
| 0x6f8cf8 | 5 | hook-call(A:0x100a3de0 B:0x100a3db0) | `e8<rel32>` | 0x100a6875 | 0x100a6845 | unknown (-) |
| 0x6402f0 | 7 | hook-jmp-indirect(A:0x100a3860 B:0x100a3830) | `ff25bb19690090` | 0x100a6895 | 0x100a6865 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x6919bb.  () |
| 0x63b8c0 | 7 | hook-jmp-indirect(A:0x100a38c0 B:0x100a3890) | `ff25bab8630090` | 0x100a68b0 | 0x100a6880 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x63b8ba.  () |
| 0x438576 | 7 | hook-jmp-indirect(A:0x100a3aa0 B:0x100a3a70) | `ff25aa85430090` | 0x100a68cb | 0x100a689b | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4385aa.  () |
| 0x584770 | 7 | hook-jmp-indirect(A:0x100a3a80 B:0x100a3a50) | `ff25794a580090` | 0x100a68e6 | 0x100a68b6 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x584a79.  () |
| 0x53c900 | 7 | hook-jmp-indirect(A:0x100a3ae0 B:0x100a3ab0) | `ff25f1c8530090` | 0x100a6904 | 0x100a68d4 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x53c8f1.  () |
| 0x4b5ac0 | 6 | hook-jmp-indirect(A:0x100a3d50 B:0x100a3d20) | `ff25bc5a4b00` | 0x100a691f | 0x100a68ef | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4b5abc.  () |
| 0x4d41c0 | 5 | hook-jmp(A:0x100a3e00 B:0x100a3dd0) | `e9<rel32>` | 0x100a6950 | 0x100a6920 | unknown (-) |
| 0x4e7427 | 5 | hook-call(A:0x100a41a0 B:0x100a4170) | `e8<rel32>` | 0x100a697b | 0x100a694b | unknown (-) |
| 0x738f3a | 6 | hook-jmp-indirect(A:0x100a3fc0 B:0x100a3f90) | `ff251b8b7300` | 0x100a69a5 | 0x100a6975 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x738b1b.  () |
| 0x738877 | 6 | hook-jmp-indirect(A:0x100a4000 B:0x100a3fd0) | `ff255b887300` | 0x100a69c0 | 0x100a6990 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x73885b.  () |
| 0x6a0050 | 9 | hook-jmp-indirect(A:0x100a4070 B:0x100a4040) | `ff2543006a00909090` | 0x100a69db | 0x100a69ab | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x6a0043.  () |
| 0x6fded6 | 5 | hook-call(A:0x100a4560 B:0x100a4530) | `e8<rel32>` | 0x100a6a0b | 0x100a69db | unknown (-) |
| 0x7fb020 | 6 | hook-jmp-indirect(A:0x100a4470 B:0x100a4440) | `ff2521c75900` | 0x100a6a34 | 0x100a6a04 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x59c721.  () |
| 0x7fb060 | 6 | hook-jmp-indirect(A:0x100a4470 B:0x100a4440) | `ff2521c75900` | 0x100a6a4a | 0x100a6a1a | EU-only variant of the 0x7FB020 stub (same hook, same slot 0x59C721, same 6 bytes) (high) |
| 0x6d0e7e | 5 | hook-call(A:0x100a45a0 B:0x100a4570) | `e8<rel32>` | 0x100a6a85 | 0x100a6a55 | unknown (-) |
| 0x866fa8 | 4 | pointer-redirect(A:0x100a45b0 B:0x100a4580) | `<samp ptr>` | 0x100a6aae | 0x100a6a7e | unknown (-) |
| 0x586c0a | 5 | hook-call(A:0x100a4a50 B:0x100a4a20) | `e8<rel32>` | 0x100a6adf | 0x100a6aaf | unknown (-) |
| 0x538090 | 7 | hook-jmp-indirect(A:0x100a4ab0 B:0x100a4a80) | `ff258480530090` | 0x100a6b00 | 0x100a6ad0 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x538084.  () |
| 0x718599 | 5 | hook-call(A:0x100a4c40 B:0x100a4c10) | `e8<rel32>` | 0x100a6b30 | 0x100a6b00 | unknown (-) |
| 0x866f7c | 4 | pointer-redirect(A:0x100a4b30 B:0x100a4b00) | `<samp ptr>` | 0x100a6b58 | 0x100a6b28 | unknown (-) |
| 0x866f80 | 4 | pointer-redirect(A:0x100a4ba0 B:0x100a4b70) | `<samp ptr>` | 0x100a6b7e | 0x100a6b4e | unknown (-) |
| 0x8585e8 | 4 | pointer-redirect(A:0x100a4ba0 B:0x100a4b70) | `<samp ptr>` | 0x100a6ba4 | 0x100a6b74 | unknown (-) |
| 0x871218 | 4 | pointer-redirect(A:0x100a4cb0 B:0x100a4c80) | `<samp ptr>` | 0x100a6bca | 0x100a6b9a | vtable slot +0xF8 (index 62, candidate BreakTowLink (low)) of vehicle class @0x871120 (inferred CAutomobile) (low) |
| 0x871778 | 4 | pointer-redirect(A:0x100a4cb0 B:0x100a4c80) | `<samp ptr>` | 0x100a6bf0 | 0x100a6bc0 | vtable slot +0xF8 (index 62, candidate BreakTowLink (low)) of vehicle class @0x871680 (inferred CHeli) (low) |
| 0x8718d0 | 4 | pointer-redirect(A:0x100a4cb0 B:0x100a4c80) | `<samp ptr>` | 0x100a6c0d | 0x100a6bdd | vtable slot +0xF8 (index 62, candidate BreakTowLink (low)) of vehicle class @0x8717D8 (inferred CMonsterTruck) (low) |
| 0x871a40 | 4 | pointer-redirect(A:0x100a4cb0 B:0x100a4c80) | `<samp ptr>` | 0x100a6c3c | 0x100a6c0c | vtable slot +0xF8 (index 62, candidate BreakTowLink (low)) of vehicle class @0x871948 (inferred CPlane) (low) |
| 0x871be0 | 4 | pointer-redirect(A:0x100a4cb0 B:0x100a4c80) | `<samp ptr>` | 0x100a6c62 | 0x100a6c32 | vtable slot +0xF8 (index 62, candidate BreakTowLink (low)) of vehicle class @0x871AE8 (inferred CQuadBike) (low) |
| 0x5648d3 | 5 | hook-call(A:0x100a4f10 B:0x100a4ee0) | `e8<rel32>` | 0x100a6c93 | 0x100a6c63 | unknown (-) |
| 0x53dfdd | 5 | hook-call(A:0x100a50d0 B:0x100a50a0) | `e8<rel32>` | 0x100a6cc7 | 0x100a6c97 | unknown (-) |
| 0x53e019 | 5 | hook-call(A:0x100a5110 B:0x100a50e0) | `e8<rel32>` | 0x100a6ce8 | 0x100a6cb8 | unknown (-) |
| 0x5534b0 | 9 | hook-jmp-indirect(A:0x100a5150 B:0x100a5120) | `ff25a6345500909090` | 0x100a6d1a | 0x100a6cea | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x5534a6.  () |
| 0x5342f9 | 5 | hook-call(A:0x100a5430 B:0x100a5400) | `e8<rel32>` | 0x100a6d4b | 0x100a6d1b | unknown (-) |
| 0x4b35a0 | 6 | hook-jmp-indirect(A:0x100a53e0 B:0x100a53b0) | `ff2533344b00` | 0x100a6d6c | 0x100a6d3c | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4b3433.  () |
| 0x41b02e | 5 | hook-call(A:0x100a5990 B:0x100a5960) | `e8<rel32>` | 0x100a6d9d | 0x100a6d6d | unknown (-) |
| 0x41af80 | 5 | hook-call(A:0x100a5b00 B:0x100a5ad0) | `e8<rel32>` | 0x100a6dd0 | 0x100a6da0 | unknown (-) |
| 0x41ab78 | 5 | hook-call(A:0x100a5b30 B:0x100a5b00) | `e8<rel32>` | 0x100a6e03 | 0x100a6dd3 | unknown (-) |
| 0x871178 | 4 | pointer-redirect(A:0x100a5b50 B:0x100a5b20) | `<samp ptr>` | 0x100a6e2c | 0x100a6dfc | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871120 (inferred CAutomobile) (medium) |
| 0x8716d8 | 4 | pointer-redirect(A:0x100a5b50 B:0x100a5b20) | `<samp ptr>` | 0x100a6e52 | 0x100a6e22 | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871680 (inferred CHeli) (medium) |
| 0x8719a0 | 4 | pointer-redirect(A:0x100a5b50 B:0x100a5b20) | `<samp ptr>` | 0x100a6e78 | 0x100a6e48 | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871948 (inferred CPlane) (medium) |
| 0x871b40 | 4 | pointer-redirect(A:0x100a5b50 B:0x100a5b20) | `<samp ptr>` | 0x100a6e9e | 0x100a6e6e | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871AE8 (inferred CQuadBike) (medium) |
| 0x8713b8 | 4 | pointer-redirect(A:0x100a5bf0 B:0x100a5bc0) | `<samp ptr>` | 0x100a6eb7 | 0x100a6e87 | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871360 (inferred CBike) (medium) |
| 0x871580 | 4 | pointer-redirect(A:0x100a5bf0 B:0x100a5bc0) | `<samp ptr>` | 0x100a6eea | 0x100a6eba | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x871528 (inferred CBmx) (medium) |
| 0x871830 | 4 | pointer-redirect(A:0x100a5c90 B:0x100a5c60) | `<samp ptr>` | 0x100a6f10 | 0x100a6ee0 | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x8717D8 (inferred CMonsterTruck) (medium) |
| 0x8721f8 | 4 | pointer-redirect(A:0x100a5d30 B:0x100a5d00) | `<samp ptr>` | 0x100a6f36 | 0x100a6f06 | vtable slot +0x58 (index 22, inferred ProcessEntityCollision) of vehicle class @0x8721A0 (inferred CBoat) (medium) |
| 0x6e0954 | 5 | hook-call(A:0x100a5dd0 B:0x100a5da0) | `e8<rel32>` | 0x100a6f67 | 0x100a6f37 | unknown (-) |
| 0x6b2bcb | 5 | hook-call(A:0x100a5dd0 B:0x100a5da0) | `e8<rel32>` | 0x100a6f8d | 0x100a6f5d | unknown (-) |
| 0x4f77da | 5 | hook-call(A:0x100a5dd0 B:0x100a5da0) | `e8<rel32>` | 0x100a6fcd | 0x100a6f9d | unknown (-) |
| 0x872a74 | 4 | pointer-redirect(A:0x100a5e70 B:0x100a5e40) | `<samp ptr>` | 0x100a6ff6 | 0x100a6fc6 | unknown (-) |
| 0x5674e0 | 6 | hook-jmp-indirect(A:0x100a6000 B:0x100a5fd0) | `ff25db745600` | 0x100a7014 | 0x100a6fe4 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x5674db.  () |
| 0x551024 | 7 | byte-patch | `6a0068c6020000` | 0x100aa5f1 | 0x100aa5c1 | 7 bytes "push 0; push 0x2C6": pool capacity 710 (inferred CPools::Initialise vehicle pool); waits until byte==0x68 first (medium) |
| 0x55105f | 4 | byte-patch | `204e0000` | 0x100aa628 | 0x100aa5f8 | pool capacity := 20000 (inferred CPools::Initialise building pool) (medium) |
| 0x5510cf | 4 | byte-patch | `a00f0000` | 0x100aa642 | 0x100aa612 | pool capacity := 4000 (inferred CPools::Initialise) (medium) |
| 0x550f46 | 4 | byte-patch | `a0860100` | 0x100aa65c | 0x100aa62c | pool capacity := 100000 (inferred ptr-node-single pool) (medium) |
| 0x550f82 | 4 | byte-patch | `401f0000` | 0x100aa676 | 0x100aa646 | pool capacity := 8000 (inferred ptr-node-double pool) (medium) |
| 0x550fba | 4 | byte-patch | `88130000` | 0x100aa690 | 0x100aa660 | pool capacity := 5000 (inferred entry-info-node pool) (medium) |
| 0x551097 | 4 | byte-patch | `b80b0000` | 0x100aa6aa | 0x100aa67a | pool capacity := 3000 (inferred CPools::Initialise) (medium) |
| 0x550ff2 | 1 | byte-patch | `f0` | 0x100aa6c4 | 0x100aa694 | pool capacity byte := 240 (inferred ped pool) (medium) |
| 0x551283 | 1 | byte-patch | `f0` | 0x100aa6db | 0x100aa6ab | pool capacity byte := 240 (inferred ped-intelligence pool, matches ped pool) (medium) |
| 0x551140 | 1 | byte-patch | `05` | 0x100aa6eb | 0x100aa6bb | pool capacity byte := 5 (low) |
| 0x551178 | 1 | byte-patch | `01` | 0x100aa709 | 0x100aa6d9 | pool capacity byte := 1 (low) |
| 0x54f3a1 | 4 | byte-patch | `70170000` | 0x100aa720 | 0x100aa6f0 | dword := 6000 (low) |
| 0x551106 | 5 | byte-patch | `68ff7e0000` | 0x100aa736 | 0x100aa706 | 5 bytes "push 0x7EFF": pool capacity 32511 (inferred CPools::Initialise col-model pool) (medium) |
| 0x4c67ad | 4 | pointer-redirect(A:0x101c9590 B:0x101c9590) | `<samp ptr>` | 0x100aaa41 | 0x100aaa11 | ped-model-info store relocated to samp buffer: 319 x 0x44-byte entries (vtable 0x85BDC0 written per entry) (inferred CPedModelInfo store enlarged) (medium) |
| 0x4c63f2 | 4 | pointer-redirect(A:0x101cea50 B:0x101cea50) | `<samp ptr>` | 0x100aaaa9 | 0x100aaa79 | unknown (-) |
| 0x5dc7aa | 4 | pointer-redirect(A:0x101a2490 B:0x101a2490) | `<samp ptr>` | 0x100aab21 | 0x100aaaf1 | unknown (-) |
| 0x40d68f | 4 | pointer-redirect(A:0x101a2490 B:0x101a2490) | `<samp ptr>` | 0x100aab71 | 0x100aab41 | unknown (-) |
| 0x4091c8 | 4 | pointer-redirect(A:0x101a2494 B:0x101a2494) | `<samp ptr>` | 0x100aaba9 | 0x100aab79 | unknown (-) |
| 0x5634a6 | 4 | pointer-redirect(A:0x101c9590 B:0x101c9590) | `<samp ptr>` | 0x100aabcc | 0x100aab9c | unknown (-) |
| 0x564dc7 | 4 | pointer-redirect(A:0x101be690 B:0x101be690) | `<samp ptr>` | 0x100aabf2 | 0x100aabc2 | pointer -> samp 0x101be690 (= relocated sector buffer + 115200, i.e. end of original-size grid) (medium) |
| 0x40936a | 4 | pointer-redirect(A:0x101a2494 B:0x101a2494) | `<samp ptr>` | 0x100aabfe | 0x100aabce | pointer -> samp 0x101a2494 (relocated sector buffer +4) (medium) |
| 0x44b1c1 | 4 | pointer-redirect(A:0x1026ae50 B:0x1026ae50) | `<samp ptr>` | 0x100aaddb | 0x100aadab | 256-entry x 4-byte table copied from samp .data 0x101169c8 to 0x1026ae50; 7 references redirected (inferred vehicle colour palette 128->256) (medium) |
| 0x4c8390 | 4 | pointer-redirect(A:0x1026ae50 B:0x1026ae50) | `<samp ptr>` | 0x100aadf1 | 0x100aadc1 | colour table redirect (byte 0) (medium) |
| 0x4c8399 | 4 | pointer-redirect(A:0x1026ae51 B:0x1026ae51) | `<samp ptr>` | 0x100aae07 | 0x100aadd7 | colour table redirect (byte 1) (medium) |
| 0x4c83a3 | 4 | pointer-redirect(A:0x1026ae52 B:0x1026ae52) | `<samp ptr>` | 0x100aae21 | 0x100aadf1 | colour table redirect (byte 2) (medium) |
| 0x5817cc | 4 | pointer-redirect(A:0x1026ae50 B:0x1026ae50) | `<samp ptr>` | 0x100aae3b | 0x100aae0b | colour table redirect (medium) |
| 0x582176 | 4 | pointer-redirect(A:0x1026ae50 B:0x1026ae50) | `<samp ptr>` | 0x100aae51 | 0x100aae21 | colour table redirect (medium) |
| 0x6a6ffa | 4 | pointer-redirect(A:0x1026ae50 B:0x1026ae50) | `<samp ptr>` | 0x100aae5b | 0x100aae2b | colour table redirect (medium) |
| 0x4df7e2 | 5 | nop | `9090909090` | 0x100aaf69 | 0x100aaf39 | unknown (-) |
| 0x5b8fe4 | 1 | byte-patch | `7f` | 0x100aaf83 | 0x100aaf53 | byte := 0x7F (low) |
| 0x5720a5 | 5 | nop | `9090909090` | 0x100aaf9f | 0x100aaf6f | unknown (-) |
| 0x53c159 | 5 | nop | `9090909090` | 0x100aafbe | 0x100aaf8e | unknown (-) |
| 0x53c136 | 5 | nop | `9090909090` | 0x100aafd4 | 0x100aafa4 | unknown (-) |
| 0x748063 | 5 | nop | `9090909090` | 0x100ab013 | 0x100aafe3 | 5-byte NOP (call removed) (EU: 0x7480B3) () |
| 0x58fbe9 | 5 | nop | `9090909090` | 0x100ab032 | 0x100ab002 | unknown (-) |
| 0x86d1ec | 1 | byte-patch | `00` | 0x100ab04c | 0x100ab01c | byte := 0x00 (inside .rdata/vtable area) (low) |
| 0x53c090 | 5 | nop | `9090909090` | 0x100ab068 | 0x100ab038 | unknown (-) |
| 0x440833 | 8 | nop | `9090909090909090` | 0x100ab087 | 0x100ab057 | unknown (-) |
| 0x53ea08 | 10 | nop | `90 x10` | 0x100ab0a4 | 0x100ab074 | unknown (-) |
| 0x71162c | 1 | byte-patch | `50` | 0x100ab0c4 | 0x100ab094 | byte := 0x50 (low) |
| 0x609c08 | 39 | nop | `90 x39` | 0x100ab0dc | 0x100ab0ac | unknown (-) |
| 0x56e0fa | 18 | nop | `90 x18` | 0x100ab0fb | 0x100ab0cb | unknown (-) |
| 0x6bc9eb | 2 | nop | `9090` | 0x100ab131 | 0x100ab101 | unknown (-) |
| 0x4bc6c1 | 3 | byte-patch | `b00090` | 0x100ab13f | 0x100ab10f | "mov al,0; nop": forces a bool result false (medium) |
| 0x44ac75 | 5 | byte-patch | `b001909090` | 0x100ab16e | 0x100ab13e | "mov al,1; 3xNOP": forces a bool result true (medium) |
| 0x5b47b0 | 1 | byte-patch | `c3` | 0x100ab19d | 0x100ab16d | ret: function disabled (medium) |
| 0x5e63a6 | 19 | nop | `90 x19` | 0x100ab1ac | 0x100ab17c | unknown (-) |
| 0x621aea | 12 | nop | `90 x12` | 0x100ab1df | 0x100ab1af | unknown (-) |
| 0x62d331 | 11 | nop | `90 x11` | 0x100ab1fa | 0x100ab1ca | unknown (-) |
| 0x741fff | 27 | nop | `90 x27` | 0x100ab220 | 0x100ab1f0 | unknown (-) |
| 0x704e8a | 5 | nop | `9090909090` | 0x100ab25b | 0x100ab22b | unknown (-) |
| 0x4090a0 | 1 | byte-patch | `c3` | 0x100ab275 | 0x100ab245 | ret: function disabled (medium) |
| 0x441482 | 5 | nop | `9090909090` | 0x100ab291 | 0x100ab261 | unknown (-) |
| 0x588be0 | 1 | byte-patch | `c3` | 0x100ab2ab | 0x100ab27b | ret: function disabled (inferred CHud::SetHelpMessage -> no SP help boxes) (low) |
| 0x53c06a | 5 | nop | `9090909090` | 0x100ab2c7 | 0x100ab297 | unknown (-) |
| 0x434272 | 5 | nop | `9090909090` | 0x100ab2e6 | 0x100ab2b6 | unknown (-) |
| 0x60d64e | 1 | byte-patch | `84` | 0x100ab300 | 0x100ab2d0 | jcc opcode byte := 0x84 (condition inverted/changed; VirtualProtect covers 0x60D64D..) (medium) |
| 0x542485 | 11 | nop | `90 x11` | 0x100ab318 | 0x100ab2e8 | unknown (-) |
| 0x612710 | 3 | byte-patch | `33c0c3` | 0x100ab335 | 0x100ab305 | "xor eax,eax; ret": function returns 0 (high) |
| 0x613ba7 | 5 | nop | `9090909090` | 0x100ab366 | 0x100ab336 | unknown (-) |
| 0x609a4e | 6 | nop | `909090909090` | 0x100ab385 | 0x100ab355 | 6-byte NOP; later 0x100a6320 writes a call hook over the first 5 bytes (high) |
| 0x6f8cf8 | 12 | byte-patch | `9090909090b8898f6f00ffe0` | 0x100ab39c | 0x100ab36c | NOP5 + "mov eax,0x6F8F89; jmp eax" (samp .data 0x101166e8); later 0x100a6320 overwrites the first 5 bytes with a call hook (high) |
| 0x53c1c1 | 5 | nop | `9090909090` | 0x100ab3e4 | 0x100ab3b4 | unknown (-) |
| 0x540040 | 9 | byte-patch | `90909090909085c974` | 0x100ab3ff | 0x100ab3cf | 6xNOP + "test ecx,ecx; je": condition rewritten (medium) |
| 0x56e5ad | 5 | nop | `9090909090` | 0x100ab438 | 0x100ab408 | unknown (-) |
| 0x609cb4 | 5 | nop | `9090909090` | 0x100ab467 | 0x100ab437 | unknown (-) |
| 0x60f2c4 | 25 | nop | `90 x25` | 0x100ab47d | 0x100ab44d | unknown (-) |
| 0x6b18f1 | 5 | nop | `9090909090` | 0x100ab4b4 | 0x100ab484 | unknown (-) |
| 0x6b9298 | 5 | nop | `9090909090` | 0x100ab4d3 | 0x100ab4a3 | unknown (-) |
| 0x6f1793 | 5 | nop | `9090909090` | 0x100ab4f2 | 0x100ab4c2 | unknown (-) |
| 0x6f86b6 | 5 | nop | `9090909090` | 0x100ab511 | 0x100ab4e1 | unknown (-) |
| 0x47c477 | 1 | byte-patch | `eb` | 0x100ab51d | 0x100ab4ed | 0xEB: conditional jump made unconditional (medium) |
| 0x53a984 | 2 | byte-patch | `eb77` | 0x100ab53d | 0x100ab50d | "jmp short +0x77": skip block (medium) |
| 0x60f289 | 8 | nop | `9090909090909090` | 0x100ab565 | 0x100ab535 | unknown (-) |
| 0x60f29d | 19 | nop | `90 x19` | 0x100ab57b | 0x100ab54b | unknown (-) |
| 0x58db5f | 9 | byte-patch | `bd0000000090909090` | 0x100ab59d | 0x100ab56d | "mov ebp, imm32; 4xNOP"; imm32 low byte is rewritten at runtime by 0x100a1460 (SetPlayerWantedLevel RPC 133 path) -> HUD wanted level comes from samp (high) |
| 0x575b0e | 5 | nop | `9090909090` | 0x100ab5ff | 0x100ab5cf | unknown (-) |
| 0x63adc8 | 6 | nop | `909090909090` | 0x100ab61e | 0x100ab5ee | unknown (-) |
| 0x58fc2e | 30 | nop | `90 x30` | 0x100ab635 | 0x100ab605 | unknown (-) |
| 0x5e1e70 | 2 | byte-patch | `33c0` | 0x100ab666 | 0x100ab636 | "xor eax,eax" (medium) |
| 0x6a8500 | 119 | byte-patch | `c3 x119` | 0x100ab68a | 0x100ab65a | 119 bytes of 0xC3 (ret) - block of functions disabled (medium) |
| 0x593d7c | 5 | nop | `9090909090` | 0x100ab6a9 | 0x100ab679 | unknown (-) |
| 0x7469af | 2 | byte-patch | `33c0` | 0x100ab6c7 | 0x100ab697 | xor eax,eax (EU: 0x7469FF) (medium) |
| 0x61e0c2 | 4 | pointer-redirect(A:0x10116718 B:0x10116718) | `<samp ptr>` | 0x100ab713 | 0x100ab6e3 | float operand pointer -> samp const 2.0 (.data 0x10116718) (medium) |
| 0x454cc9 | 4 | pointer-redirect(A:0x1011671c B:0x1011671c) | `<samp ptr>` | 0x100ab72d | 0x100ab6fd | float operand pointer -> samp const 10000.0 (.data 0x1011671c) (medium) |
| 0x5952a6 | 5 | nop | `9090909090` | 0x100ab74c | 0x100ab71c | unknown (-) |
| 0x6884c4 | 6 | nop | `909090909090` | 0x100ab772 | 0x100ab742 | unknown (-) |
| 0x688200 | 6 | nop | `909090909090` | 0x100ab788 | 0x100ab758 | unknown (-) |
| 0x570535 | 15 | nop | `90 x15` | 0x100ab79f | 0x100ab76f | unknown (-) |
| 0x5e7847 | 17 | nop | `90 x17` | 0x100ab7c8 | 0x100ab798 | unknown (-) |
| 0x570546 | 175 | nop | `90 x175` | 0x100ab7f8 | 0x100ab7c8 | unknown (-) |
| 0x6d1874 | 24 | byte-patch | `d5176d00d5176d00d5176d00d5176d00d5176d00…` | 0x100ab812 | 0x100ab7e2 | six jump-table entries all pointed at 0x6D17D5 (switch cases collapsed) (high) |
| 0x64db49 | 9 | nop | `90 x9` | 0x100ab851 | 0x100ab821 | unknown (-) |
| 0x64bc9f | 9 | nop | `90 x9` | 0x100ab873 | 0x100ab843 | unknown (-) |
| 0x64b872 | 9 | nop | `90 x9` | 0x100ab895 | 0x100ab865 | unknown (-) |
| 0x5e7d62 | 5 | byte-patch | `5e5dc21c00` | 0x100ab8ab | 0x100ab87b | "pop esi; pop ebp; ret 0x1C": early return (high) |
| 0x6348f6 | 4 | byte-patch | `ffffffff` | 0x100ab8ea | 0x100ab8ba | dword := 0xFFFFFFFF (low) |
| 0x634de2 | 4 | byte-patch | `ffffffff` | 0x100ab903 | 0x100ab8d3 | dword := 0xFFFFFFFF (low) |
| 0x71a223 | 1 | byte-patch | `03` | 0x100ab90b | 0x100ab8db | unknown (-) |
| 0x71a5d5 | 1 | byte-patch | `03` | 0x100ab911 | 0x100ab8e1 | unknown (-) |
| 0x71a247 | 1 | byte-patch | `03` | 0x100ab917 | 0x100ab8e7 | unknown (-) |
| 0x71a282 | 1 | byte-patch | `03` | 0x100ab91d | 0x100ab8ed | unknown (-) |
| 0x71a26a | 1 | byte-patch | `03` | 0x100ab923 | 0x100ab8f3 | unknown (-) |
| 0x71a2e5 | 1 | byte-patch | `03` | 0x100ab929 | 0x100ab8f9 | unknown (-) |
| 0x71a3ac | 1 | byte-patch | `03` | 0x100ab92f | 0x100ab8ff | unknown (-) |
| 0x71a4b9 | 1 | byte-patch | `03` | 0x100ab935 | 0x100ab905 | unknown (-) |
| 0x71a53c | 1 | byte-patch | `03` | 0x100ab93b | 0x100ab90b | unknown (-) |
| 0x71a37f | 1 | byte-patch | `03` | 0x100ab941 | 0x100ab911 | unknown (-) |
| 0x71a518 | 1 | byte-patch | `03` | 0x100ab94c | 0x100ab91c | unknown (-) |
| 0x71a525 | 1 | byte-patch | `03` | 0x100ab954 | 0x100ab924 | unknown (-) |
| 0x71a54b | 1 | byte-patch | `03` | 0x100ab95a | 0x100ab92a | unknown (-) |
| 0x71a5b6 | 1 | byte-patch | `03` | 0x100ab962 | 0x100ab932 | unknown (-) |
| 0x71a3bb | 1 | byte-patch | `03` | 0x100ab96d | 0x100ab93d | unknown (-) |
| 0x7361b2 | 4 | pointer-redirect(A:0x101166f4 B:0x101166f4) | `<samp ptr>` | 0x100ab988 | 0x100ab958 | float operand pointer -> samp const -20000.0 (.data 0x101166f4) (inferred bullet/world bound) (medium) |
| 0x7361cb | 4 | pointer-redirect(A:0x101166f0 B:0x101166f0) | `<samp ptr>` | 0x100ab9a3 | 0x100ab973 | float operand pointer -> samp const 20000.0 (.data 0x101166f0) (medium) |
| 0x7361e0 | 4 | pointer-redirect(A:0x101166f4 B:0x101166f4) | `<samp ptr>` | 0x100ab9b9 | 0x100ab989 | float operand pointer -> samp const -20000.0 (medium) |
| 0x7361f5 | 4 | pointer-redirect(A:0x101166f0 B:0x101166f0) | `<samp ptr>` | 0x100ab9cf | 0x100ab99f | float operand pointer -> samp const 20000.0 (medium) |
| 0x524582 | 5 | nop | `9090909090` | 0x100ab9ea | 0x100ab9ba | unknown (-) |
| 0x712025 | 1 | byte-patch | `01` | 0x100aba04 | 0x100ab9d4 | byte := 0x01 (low) |
| 0x53bf28 | 5 | nop | `9090909090` | 0x100aba2e | 0x100ab9fe | unknown (-) |
| 0x4c3a5b | 10 | nop | `90 x10` | 0x100aba6d | 0x100aba3d | unknown (-) |
| 0x4c63f2, 0x4c662d, 0x4c6822, 0x4c6829, 0x4c6877, 0x4c6881, 0x4c6890, 0x4c68a5, 0x4c68f3, 0x4c6932, 0x4c6971, 0x4c69b0, 0x4c69ef, 0x4c6a2e | 4 | pointer-redirect(A:0x101cea50 B:0x101cea50) | `<samp ptr>` | 0x100aaaa9 | 0x100aaa79 | atomic-model-info store relocated to samp buffer of 20000 x 32-byte entries (vtable 0x85BBF0 written per entry) (inferred CModelInfo atomic store; default smaller) (medium) |
| 0x5dc7aa, 0x41a85d, 0x41a864, 0x408259, 0x711b32, 0x699cf8, 0x4092ec, 0x40914e, 0x408702, 0x564220, 0x564172, 0x563845, 0x84e9c2, 0x85652d | 4 | pointer-redirect(A:0x101a2490 B:0x101a2490) | `<samp ptr>` | 0x100aab21 | 0x100aaaf1 | sector-array base relocated to samp buffer (160000 bytes, zeroed); original 115200-byte array at 0xB7D0B8 zeroed (inferred CWorld sector grid). EU list differs at 3 entries (medium) |
| 0x40d68f, 0x5664da, 0x566589, 0x408709, 0x56b3b4, 0x56ad94, 0x56a862, 0x5675fd, 0x56cd87, 0x56cc7c, 0x56cb54, 0x56ca4d, 0x56c667, 0x56c56c, 0x56c448, 0x56c344, 0x56bd49, 0x56bc56, 0x56be59, 0x56a943, 0x567738, 0x54673b, 0x54bb26, 0x6e31ad, 0x40dc2c, 0x534a0c, 0x534d6e, 0x564b5c, 0x564dac, 0x67ff60, 0x568cbc, 0x568efe, 0x569f5a, 0x56953a, 0x56912a, 0x56b4b8, 0x56b597, 0x56b2c6, 0x56af77, 0x56ae98, 0x56bf52, 0x56aca6, 0x56a769, 0x56a688, 0x70b9bd, 0x5647a0, 0x70acb5, 0x6063ca, 0x699d01, 0x41a864, 0x40e064, 0x40df61, 0x40ddd1, 0x40db11, 0x40d98f, 0x1566858 | 4 | pointer-redirect(A:0x101a2490 B:0x101a2490) | `<samp ptr>` | 0x100aab71 | 0x100aab41 | same relocation, imm32 at instruction+3 (EU list differs only in last entry: 0x1566845 vs 0x1566855) (medium) |
| 0x4091c8, 0x40936a, 0x40d9c8, 0x40db4a, 0x40dc64, 0x40de0a, 0x40df9a, 0x40e09d, 0x534a9b, 0x534dfd, 0x71cdb3 | 4 | pointer-redirect(A:0x101a2494 B:0x101a2494) | `<samp ptr>` | 0x100aaba9 | 0x100aab79 | same relocation, +4 field, imm32 at instruction+3 (medium) |
| 0x5634a6, 0x5638df, 0x56420f, 0x564283 | 4 | pointer-redirect(A:0x101c9590 B:0x101c9590) | `<samp ptr>` | 0x100aabcc | 0x100aab9c | ped-model-info store relocated (see 0x4C67AD) (medium) |
| 0xb7d0b8 | 115200 | data-fill | `00 x115200` | 0x100aac10 | 0x100aabe0 | zero original array after relocation (inferred CWorld sector grid) (medium) |
| 0xc8d4c0 | 4 | data-write | `08000000` | 0x100a0954 | 0x100a0924 | game-state variable := 8 (inferred: leave frontend and start loading the game) (medium) |
| 0xba6831,0xba67a4,0xba677b | 1 | data-write | `01 / 00 / 00` | 0x100a0960..0x100a096c | 0x100a0930..0x100a093c | frontend/menu flags (inferred menu manager fields) (low) |

#### applied_when = once*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x4087ea | 6 | hook-jmp-indirect(A:0x100a5ed0 B:0x100a5ea0) | `ff25d7874000` | 0x100a7046 | 0x100a7016 | writes "jmp dword [slot]" stub at site and stores the samp handler address into GTA slot 0x4087d7.  () |
| (707 sites) | 4 each | pointer-redirect(A:0x10182424 B:0x10182424) | `<samp ptr>` | 0x100a79ff/0x100a7a14 | 0x100a79cf/0x100a79e4 | model-info pointer array (20000 ptrs copied from 0xA9B0C8) relocated into a 65535-entry samp array whose element 0 is at 0x10182424; each site from table A .data 0x10114b10 (US) / 0x101158e0 (EU), 5-byte records {u32 addr; u8 expected opcode}; patched only if opcode matches: 8B/89/39 -> imm32 at +3, BE/BF -> imm32 at +1 (inferred CModelInfo::ms_modelInfoPtrs) (medium) |

#### applied_when = rpc139-InitGame*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x60dc50 | 1 | byte-patch | `c3` | 0x100a1c23 | 0x100a1bf3 | unknown (-) |
| 0x60e530 | 3 | byte-patch | `c20800` | 0x100a1c2f | 0x100a1bff | unknown (-) |
| 0x742495 | 5 | hook-call(A:0x100a5820 B:0x100a57f0) | `e8<rel32>` | 0x100a7084 | 0x100a7054 | unknown (-) |
| 0x7424ec | 5 | hook-call(A:0x100a5820 B:0x100a57f0) | `e8<rel32>` | 0x100a70b8 | 0x100a7088 | unknown (-) |
| 0x742548 | 5 | hook-call(A:0x100a5820 B:0x100a57f0) | `e8<rel32>` | 0x100a70eb | 0x100a70bb | unknown (-) |
| 0x740721 | 5 | hook-call(A:0x100a5640 B:0x100a5610) | `e8<rel32>` | 0x100a711e | 0x100a70ee | unknown (-) |
| 0x740b69 | 5 | hook-call(A:0x100a5640 B:0x100a5610) | `e8<rel32>` | 0x100a7152 | 0x100a7122 | unknown (-) |
| 0x736247 | 5 | hook-call(A:0x100a5640 B:0x100a5610) | `e8<rel32>` | 0x100a7185 | 0x100a7155 | unknown (-) |
| 0x7424cb | 5 | hook-call(A:0x100a58e0 B:0x100a58b0) | `e8<rel32>` | 0x100a71b8 | 0x100a7188 | unknown (-) |
| 0x73ac4b | 5 | hook-call(A:0x100a5930 B:0x100a5900) | `e8<rel32>` | 0x100a71ec | 0x100a71bc | unknown (-) |
| 0x6d7c90 | 5 | hook-jmp(A:0x100a5370 B:0x100a5340) | `e9<rel32>` | 0x100a7234 | 0x100a7204 | unknown (-) |
| 0x6e1be3 | 37 | nop | `90 x37` | 0x100aac51 | 0x100aac21 | unknown (-) |
| 0x6e1c38 | 8 | nop | `9090909090909090` | 0x100aac91 | 0x100aac61 | unknown (-) |
| 0x6e1d98 | 15 | nop | `90 x15` | 0x100aaca7 | 0x100aac77 | unknown (-) |
| 0x6e1dbc | 8 | nop | `9090909090909090` | 0x100aacd9 | 0x100aaca9 | unknown (-) |
| 0x6e1ba0 | 6 | nop | `909090909090` | 0x100aacf8 | 0x100aacc8 | unknown (-) |
| 0x6e1bb1 | 6 | nop | `909090909090` | 0x100aad18 | 0x100aace8 | unknown (-) |
| 0x6e1bd2 | 7 | nop | `90909090909090` | 0x100aad2f | 0x100aacff | unknown (-) |
| 0x6e1be5 | 10 | byte-patch | `8a8628040000a8407428` | 0x100aad5a | 0x100aad2a | rewritten inside the NOPed range: "mov al,[esi+0x428]; test al,0x40; jz +0x28" (vehicle flag test) (high) |

#### applied_when = runtime:*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x541df5 | 5 | nop / restore | `9090909090 \| restore e846f3feff (call 0x…` | 0x100a0775,0x100a07f6,0x100c4598 / 0x100a0663 | 0x100a0745,0x100a07c6,0x100c4578 / 0x100a0633 | call inside the pad-update routine (samp itself calls 0x541DD0 and 0x541BD0 around the toggle; inferred CPad::UpdatePads / ClearMouseHistory). VirtualProtect for it is already done in 0x100aaef0 (medium) |
| 0x53f417 | 5 | nop / restore | `9090909090 \| restore e8b47a2000 (US, cal…` | 0x100a05f6 / 0x100a0691,0x100a06ab | 0x100a05c6 / 0x100a0661,0x100a067b | call in mouse-update code removed while input is locked (inferred) (medium) |
| 0x53f41f | 4 | byte-patch / restore | `33c00f84 \| restore 85c00f8c` | 0x100a0607 / 0x100a06ca | 0x100a05d7 / 0x100a069a | "xor eax,eax; jz" replaces "test eax,eax; jl" (mouse-update branch forced) (medium) |
| 0x53f47a,0x53f49a,0x53f4b3 | 4 | pointer-redirect / restore | `-> samp 0x101501a8 (dummy) \| restore 0xB…` | 0x100a0578.. / 0x100a05b4.. | 0x100a0548.. / 0x100a0584.. | mouse-state operand pointers redirected to a samp dummy (and GTA mouse state 0xB7340C..0xB73417 zeroed) (medium) |
| 0x6194a0 | 1 | byte-patch / restore | `c3 \| restore e9` | 0x100a07ad,0x100a0846,0x100a089f,0x100c45b8 / 0x100a0725 | 0x100a077d,0x100a0816,0x100a086f,0x100c4598 / 0x100a06f5 | function disabled while samp owns the cursor (inferred mouse-position setter) (low) |
| 0x52cf10 | 1 | byte-patch toggle | `c3 (off) \| 56 (on)` | 0x100a0b7a,0x100a1503,0x100a1533 | 0x100a0b4a,0x100a14d3,0x100a1503 | on/off of a GTA function (paired with clock format string at 0x859A6C) -> HUD clock display (inferred) (medium) |
| 0x859a6c | 10 | data-write toggle | `"%02d:%02d" \| zeros` | 0x100a1516,0x100a154b | 0x100a14e6,0x100a151b | clock text format string (high) |
| 0x55a070 | 1 | byte-patch | `c3` | 0x100a0c5d | 0x100a0c2d | function disabled (low) |
| 0xc1704c | 4 | data-write | `c8000000` | 0x100a0c26 | 0x100a0bf6 | dword := 200 (low) |
| 0x863984 | 4 | data-write | `<float gravity>` | 0x100a1453 | 0x100a1423 | gravity constant (default 0.008) - unprotected then written (high) |
| 0xa4a474 | 4 | data-write | `<u8 zero-extended>` | 0x100a1594 | 0x100a1564 | stunt-bonus enable flag (high) |
| 0x58db60 | 1 | data-write | `<wanted level>` | 0x100a1464 | 0x100a1434 | imm of the "mov ebp, imm32" patched at 0x58DB5F by init (high) |
| 0x4e7529 | 39 | nop | `90 x39` | 0x100a1816 | 0x100a17e6 | 39-byte NOP before playing a crime/police report (inferred police-scanner audio) (medium) |
| 0x46977f | 1 | byte-patch temporary | `75 during call, then 7d` | 0x100a929b | 0x100a926b | jge->jne swap around a GTA vehicle-create call, restored after (medium) |
| 0x5a82c0 | 1 | byte-patch | `c3` | 0x100affd3 | 0x100affa3 | function disabled (once set, never restored) (low) |
| 0x41b1d0 | 1 | byte-patch temporary | `c3 during call, then 64` | 0x100b1f7a,0x100b202c | 0x100b1f4a,0x100b1ffc | function disabled around a samp-driven load, restored to 0x64 (low) |
| 0x50ac20 | 3 | byte-patch | `c208 + 00` | 0x10003df3 | 0x10003de3 | "ret 8": function disabled at each spawn (low) |
| 0x5e92f4 | 5 | nop / restore | `9090909090 \| restore e81788ffff (call 0x…` | 0x100a2976 / 0x100a298b,0x100a2ac9 | 0x100a2946 / 0x100a295b,0x100a2a99 | call toggled per frame around local/remote ped processing (low) |
| 0x6884c4,0x688200 | 6 | nop / restore (temporary) | `909090909090 \| restore from samp .data 0…` | 0x100a290a.. 0x100a2c4e.. | 0x100a28da.. 0x100a2c1e.. | float store suppressed while samp processes remote players (medium) |
| 0x859520 | 8 | data-write | `<two floats>` | 0x100a24bd,0x100a24d9 | 0x100a248d,0x100a24a9 | screen-scale constants: second := 0x3B1E98DD, first := 0.00222/aspect; restored to saved values in the hooks (VirtualProtect done in 0x100aaef0; reads 0xC17044/0xC17048 = screen width/height) (medium) |
| 0x866b74 | 8 | data-write | `00009842 0000bc42 (76.0,94.0) or 0000a44…` | 0x100a24ac,0x100a24ed,0x100a2525.. | 0x100a247c.. | two HUD float constants chosen by aspect ratio (pointers held in samp .data 0x10113a88/8c) (medium) |
| 0x858ef0 | 4 | data-write temporary | `00000040 (2.0) then restore` | 0x100a2fc8,0x100a2ff0 | 0x100a2f98,0x100a2fc0 | float constant forced to 2.0 for one call (medium) |
| 0x858ba4,0x858f84 | 4 | data-write temporary | `00002042 (40.0), 00004842 (50.0) then re…` | 0x100a4c6f,0x100a4c7c,0x100a4c95,0x100a4ca0 | 0x100a4c3f,.. | two float constants overridden for one call (medium) |
| 0x4096aa | 3 | nop / restore (temporary) | `909090 \| restore ff5220` | 0x100a4b4f,0x100a4b6c | 0x100a4b1f,0x100a4b3c | "call [edx+0x20]" suppressed for one call (medium) |
| 0x71791f | 5 | nop / restore (temporary) | `9090909090 \| restore e83cffffff` | 0x100a4c0d,0x100a4c26 | 0x100a4bdd,0x100a4bf6 | call suppressed for one call (medium) |
| 0x5543c0 | 30 | nop / restore (temporary) | `NOP 22 @0x5543C0 + NOP 2 @0x5543DC \| res…` | 0x100a501f,0x100a503b,0x100a5081,0x100a50b8 | 0x100a4fef,.. | code block suppressed for one call, original bytes rewritten from constants afterwards (medium) |
| 0x6d64f0 | 1 | byte-patch | `c3` | 0x100a4171 | 0x100a4141 | function disabled (not restored) (low) |
| 0xc97c28 | 4 | pointer-redirect (data) | `<samp object, vtable A 0x100ed8f0>` | 0x100c48a3 | 0x100c4883 | GTA global object pointer replaced by a samp wrapper object (inferred render-device pointer) (low) |
| 0xa4326c(u16),0xa43318(3xf32),0xa432e8(f32),0xa432c0(u32) | 22 | data-write (A only; B uses a script command instead) | `0 / spawn pos / heading / 0` | 0x100ac796..0x100ac7c2 | (B 0x100ac760 runs GTA script command descriptor 0x100ece20 with pos+heading instead) | respawn/restart override set to the spawn point (inferred CRestart override). Build difference in mechanism only (medium) |

#### applied_when = auth*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x4d3aa0 | 20 | read (not a patch) | `-` | 0x100578d5 | 0x10057885 | connection-auth related read of patched code (see handshake.md section 4); recorded for completeness only (high) |

#### applied_when = never*

| GTA addr | size | kind | new bytes | A site | B site | purpose (conf.) |
|---|---|---|---|---|---|---|
| 0x40619b,0x405c3d | 4 | pointer-redirect (DEAD CODE) | `<malloc buffer>` | 0x100aa1b0 | 0x100aa180 | also writes 4-byte values into 0x1569777/0x15649FA/0x1561160 (US) or 0x1569717/0x156495A/0x156115C (EU) via patch-copy helper 0x100aa160 (high) |
| 0x4c66b1,0x4c66c2,0x84bc51,0x856261,0x4c683b,0x4c6464,0x4c66bd,0x4c58a5 | 4 | pointer-redirect (DEAD CODE) | `<malloc buffer>` | 0x100aa2c0 | 0x100aa290 | alternate store allocator: N x 0x24-byte entries (vtable 0x85BCB0 bytes written per entry), plus "push N" at 0x4C58A5 (high) |
| 0x411458 + 7 sites from .data 0x101166f8 (+2) | 4 | data-write (DEAD CODE) | `N and N*0x2C` | 0x100aa520 | 0x100aa4f0 | pool-size changer (count N, size N*44) (high) |
| 0x71a223.. (15 bytes) | 1 | byte-patch (DEAD COPY) | `03` | 0x100aa580 | 0x100aa550 | duplicate (high) |
| 0x704e8a,0x609a4e | 5 | nop (DEAD CODE) | `90..` | 0x100aae70 | 0x100aae40 | alternate reduced init (high) |

## Purpose analysis (gta_sa.exe 1.0 US)

Per-row results are in `docs/re/data/gta-patches-r4-annotated.csv` (original columns plus `gta_function`,
`original_bytes_hex`, `effect`, `purpose_source`). Method: original bytes were read from
`re/gta/1.0-US/gta_sa.exe`; function boundaries come from Ghidra (REA) plus function starts known from
gta-reversed / plugin-sdk (shallow clones in `re/gta-reversed/`, `re/plugin-sdk/`, used only to map addresses to
names); every code site was disassembled. `purpose_source`: **named** = the purpose follows from the GTA
function/variable name (named via gta-reversed and/or plugin-sdk, recorded in `gta_function`); **observed** = derived
from instruction/data semantics alone; **inferred** = reasoning beyond that. Counts over the 384 rows: named 285,
observed 76, inferred 23, unknown 0 (but see "Remaining unknowns": many hook *handlers* are still unanalysed).

**About this executable.** The reference exe is the 14 MB build with a `.HOODLUM` section. Many function entries
are relocated stubs (`push -1; jmp 0x40xxxx`, `nop; jmp ...`), and samp's sector table contains one address inside
`.HOODLUM` (0x1566858, a sector-lookup helper; the EU table has 0x1566845). samp therefore targets this exact
layout, and the Rust client must target it as well. The three "wait until byte == X" sites (0x551024 = 0x68, 0x561872 =
0x85, 0x4083C0 = 0xB8) already hold those bytes in this file, so the waits only matter for a packed retail exe.

### Startup / intro
* `gGameState` (0xC8D4C0) := 5 (FRONTEND_LOADING) at attach, and the `mov [gGameState],esi` (esi = 0) in
  `psInitialize` (0x747483) is NOPed, so the logo/intro states are skipped. The first byte of that instruction
  (0x89) is also samp's version probe.
* Splash texture names are changed: "title_pc_US" → "title" (0x866CD8) and "loadsc%d" → "title" (0x866CCC). Every loading slide
  uses one texture called `title`. A `CSprite2d::DrawRect` in `CLoadingScreen::RenderSplash` (0x590099) is removed.
* `GetVideoModeList` (0x745B87, 68 bytes) and `FillAvailableVMs` (0x7459E1) no longer filter modes by aspect
  ratio.
* `CTimer::Initialise` (0x561872) always takes the millisecond-timer path and never uses QueryPerformanceCounter.
  The frame-limiter busy-wait in `Idle` waits for 2 ms instead of 14 ms (0x53E94C).
* `CStreaming::InitImageList` becomes RET (0x4083C0). The game does not register GTA3.IMG/GTA_INT.IMG
  itself (inferred: samp builds the image list, including its own archive).
* `diMouseInit` always uses cooperative level 6 (non-exclusive, foreground) (0x7469AF). The call to `CGame::InitAfterLostFocus` in
  `__MainWndProc` is removed (0x748063).
* At the end of init-main: `gGameState` := 8 (LOADING_STARTED). The FrontEndMenuManager fields at +0xE9 (`m_bMainMenuSwitch`),
  +0x5C (`m_bMenuActive`) and +0x33 (`m_bActivateMenuNextFrame`) are set to 1/0/0. The game then loads straight
  into the world without the main menu. At runtime `RsGlobal.frameLimit` (0xC1704C) := 200, and `_RwD3DDevice` (0xC97C28) is
  replaced by a samp wrapper object.

### Pools & limits (CPools::Initialise 0x550F10 and others; pool names read from the push arguments)
| pool | site | original | samp |
|---|---|---|---|
| PtrNode Single | 0x550F46 | 70000 | 100000 |
| PtrNode Double | 0x550F82 | 3200 | 8000 |
| EntryInfoNode | 0x550FBA | 500 | 5000 |
| Peds | 0x550FF2 | 140 | 240 |
| Vehicles | 0x551024 | 110 | 710 (name argument becomes NULL) |
| Buildings | 0x55105F | 13000 | 20000 |
| Objects | 0x551097 | 350 | 3000 |
| Dummys | 0x5510CF | 2500 | 4000 |
| ColModel | 0x551106 | 10150 | 32511 |
| Task | 0x551140 | 500 | 1524 (second byte 0x01→0x05) |
| Event | 0x551178 | 200 | 456 (second byte 0x00→0x01) |
| PedIntelligence | 0x551283 | 140 | 240 |
| TexDictionary (`CTxdStore::Initialise`) | 0x731F60 | 5000 | 20000 |
| VehicleStruct (`CVehicleModelInfo::SetupCommonData`) | 0x5B8FE4 | 50 | 127 |
| matrices (`CPlaceable::InitMatrixArray`) | 0x54F3A1 | 900 | 6000 |

The Task and Event values are produced by single-byte writes into the imm32. The earlier reading of these two as
"capacity 5 / 1" was wrong.

### Streaming, memory and world tables
* `CStreaming::ms_memoryAvailable` initial value is 50 MB in the file (imm32 at 0x5B8E6A). samp replaces it with 96 MB–1 GB
  depending on RAM. `CStreaming::DeleteAllRwObjects` becomes RET (0x4090A0), and `DeleteRwObjectsAfterDeath` is no longer called from
  `CGameLogic::SortOutStreamingAndMemory` (0x441482).
* The sector grid is relocated: every listed reference to `CWorld::ms_aSectors` (0xB7D0B8, 120×120×8 bytes) or
  +4 now points into samp's 160000-byte buffer. **Correction to §4.2/§4.3:** 0x5634A6, 0x5638DF, 0x56420F and 0x564283
  are *not* ped-model-store references. In the exe they are loop bounds `cmp reg, 0xB992B8` (the end of the sector array
  = `ms_aRepeatSectors`), and samp sets them to 0x101C9590, the end of its 160000-byte buffer (which happens to be where the
  ped store begins). 0x564DC7 (`lea esi,[esi*4+ms_aRepeatSectors]` in `FindObjectsOfTypeInRange`) is moved to buffer+115200. The other
  repeat-sector references stay at 0xB992B8. 0x40914E is listed in samp's absolute-pointer table, but the dword there is
  the start of an instruction, not its displacement, so samp's write corrupts that instruction. This is harmless because the
  containing function (`DeleteAllRwObjects`) is made RET.
* `CModelInfo::ms_pedModelInfoStore` (0x4C67AD, AddPedModel) and `ms_atomicModelInfoStore` (14 references) are
  moved to larger samp stores. `ms_modelInfoPtrs` (707 references) is moved to a ±32767-index table. The
  vehicle colour table (`CVehicleModelInfo::ms_vehicleColourTable`, 7 references) is moved to a 256-entry copy.
* `CFont::ProcessCurrentString`: 15 single-byte patches raise the stack frame from 0x12C to 0x32C bytes and shift
  every stack access. The likely purpose is a larger line buffer for long chat lines.

### Player / ped control
* Vtable redirects: ProcessControl of `CPlayerPed` and `CCivilianPed`; Render of `CPlayerPed`, `CCivilianPed`,
  `CCopPed`, `CPed` and the ped vtable at 0x86C200 (inferred `CEmergencyPed`); `CPlayerPed` slot +0x2C
  (`ProcessCollision`); `CTaskSimpleUseGun::SetPedPosition`.
* The local-player-only lookups are removed: `GetPlayerInfoForThisPlayerPed` / `m_bCanDoDriveBy` in
  `RemoveWeaponWhenEnteringVehicle`, `DoDriveByShootings` and two kill-ped tasks;
  `FindPlayerSlotWithPedPointer`/`m_pRemoteVehicle` in `CPlayerPed::ProcessControl`; the remote-vehicle branch of
  `FindPlayerVehicle`; the `Players[0]` driver test in `CBike::ProcessAI`. Together these show that samp's remote
  players are `CPlayerPed` instances without a `CWorld::Players` slot. The client must apply the same patches, or the
  game indexes the player array with -1.
* Animation: `ProcessAnimGroups` no longer sets the move-anim group (a samp call hook replaces the store) and no
  longer releases anim blocks. The "playidles" block name is blanked (0x86D1EC). `AddAnimation`/`BlendAnimation` are
  redirected through indirect jmps, and `UncompressAnimation` through a direct jmp.
* `m_fAimingRotation` stores in `PlayerControlZelda`/`PlayerControlDucked` are NOPed and toggled at runtime.
  `CPed::UpdatePosition` ignores `m_standingOnEntity`, and the call to it from `CPed::ProcessControl` is toggled per
  frame.
* `CPed::AttachPedToEntity` returns before it gives the weapon, sets the camera mode or changes the ped state. `CPad::GetHandBrake`
  ignores DisablePlayerControls. Lock-on is disabled (`FindWeaponLockOnTarget`, `FindNextWeaponLockOnTarget`;
  InitGame). Car-entry tasks keep the ped's weapon. The `CPlayerPed` constructor's ped-group setup condition is inverted
  (0x60D64E). The cell-phone model (330) is never added (0x634DE2).
* Damage: `ComputeDamageResponse` and `CEventDamage::AffectsPed` are redirected. Vehicle enter/leave task constructors are
  hooked.

### Vehicles
* ProcessControl of all ten vehicle classes, ProcessEntityCollision (8 classes) and BreakTowLink (5 classes) are
  redirected to samp. The game's own `CAEVehicleAudioEntity::Service` calls in Automobile/Bike/Boat/Train
  ProcessControl are NOPed. The siren/horn audio and `UsesSiren` call sites are hooked.
* `CVehicle::SetDriver` "freebies" are removed (ambulance health, taxi cash, Enforcer armour, Caddy club, Hotdog calories,
  cop-car shotgun). `CAutomobile::ProcessFlyingCarStuff` is filled with RET. The train passenger block in
  `CTrain::ProcessControl` is skipped. `CVehicle::InflictDamage` is jmp-hooked (in InitGame). Number plates
  (`RwRasterCreate` in `CreatePlateTexture`, `RwRasterDestroy`) and `SetDirtTextures` are hooked.
* Manual engine/lights mode (InitGame, once): `CVehicle::DoVehicleLights` loses its own on/off logic, and the
  lights follow bit 0x40 of the byte at +0x428.

### Weapons / bullets
* InitGame hooks: `FireInstantHit` (3 call sites in `CWeapon::Fire`), `ProcessLineOfSight` (2 in `FireInstantHit`, 1 in
  `CBulletInfo::Update`), `FireSniper`, and `CBulletInfo::AddBullet`. These form the bullet-sync entry points.
* The world bounds for bullets are widened from ±3000 to ±20000. Gun-aim movement uses a constant 2.0 instead of
  `CTimer::ms_fTimeStep`. The "fire failed" sniper/rocket sounds are removed. Detonator (projectile type 39) checks are
  hooked. The sniper camera shake goes to samp.

### Camera / input
* `CCamera::Process` and three camera-collision helpers are hooked. `COMMAND_POINT_CAMERA_AT_CHAR` always uses
  the ped (non-player) mode, which is needed to spectate remote `CPlayerPed`s. `CIdleCam::Process` is removed. `ClearPlayerWeaponMode` is removed
  from `ClearWeaponTarget`. `CCamera::Fade` becomes `ret 8` at spawn.
* Input lock (runtime): keyboard→pad (`AffectPadFromKeyBoard`), mouse (`GetMouseState`, branch and state
  pointers) and `RsMouseSetPos` are switched off and later restored.

### HUD / menus / rendering
* `CHud::DrawRadar`, `DrawCrossHairs`, the radar/map gang overlay, `GetRadarTraceColour` and `CText::Get` are hooked.
  Vital stats, vehicle name, zone name (`CPlaceName::Process`), `SetHelpMessage`, the "Wasted" big message, a map
  `DrawRect` and SpeedFX are removed. The wanted level comes from samp (`CHud::DrawWanted` imm). The clock
  (`CClock::Update` RET plus the format string) follows TogglePlayerClock. Aspect-ratio constants are set at runtime.
* Rendering: hooks around `RenderEverythingBarRoads`, `RenderFadingInEntities`, `AddEntityToRenderList`,
  `RenderRoadsignAtomic`, `CObject::Render/CreateRwObject/DeleteRwObject` and `CBuilding::DeleteRwObject`. Real-time
  shadows are removed (update and return calls). Stencil-shadow alpha goes from 50 to 80. Sun/moon coronas are removed.

### Single-player features disabled
Ambient population (`CPopulation::AddPed` returns NULL, `AddPedInCar` does not seat), random traffic, car
generators,
planes, IPL and interior pickups, interior ambient peds, cutscene and replay updates, plant manager, ambient gunfire
audio, stats changes (`SetStatValue` RET), Pay'n'Spray restrictions (always sprayable), BMX-fire player ignition,
event facial expressions, peds reacting to aimed guns, one `CPlayerInfo::Process` task block (see unknowns) and CJ
clothes rebuild on skin change. Cheat activation is intercepted in `CCheat::AddToCheatString` (anti-cheat related;
noted only).

### Other
* Two unimplemented script opcodes, 0x09C6 and 0x09D3, are pointed into samp code through the jump table of
  `ProcessCommands2500To2599`. During samp's own `COMMAND_CREATE_OBJECT` (0x0107) call the model-id sign test accepts
  negative ids (matching the ±32767 model table). This corrects the earlier "vehicle-create" label.
* Gravity (`GAME_GRAVITY`), the stunt-bonus SCM global (ScriptSpace+0xB14) and the hospital-restart override are written.
  Escalator floats are raised around a samp hook. The auth handler reads `AddAnimation`'s patched entry (integrity;
  noted only).

### What the Rust client must do at startup (summary)
1. Target exactly this exe layout (US compact/HOODLUM addresses, including the .HOODLUM sector helper).
2. init-early: game state 5, NOP the psInitialize reset, splash names, video-mode filters, QPC timer
   bypass, frame-limiter 2 ms, InitImageList RET (then register the image archives yourself), streaming budget.
3. Wait for game state 7. Then apply the pool sizes, the store/sector/colour relocations, the ~95 inline patches (including the
   CFont stack-frame rewrite and every remote-`CPlayerPed` safety NOP), then the hooks/vtable redirects in the documented
   order. Set game state 8 and the three menu flags.
4. Provide replacements for every redirected function. Many handlers must call the original and add sync
   logic, and several temporarily patch and restore other sites (see runtime rows).

### Remaining unknowns
* The task built by the NOPed block at 0x570546 in `CPlayerInfo::Process` (constructor 0x6919C0, argument 0x51) is not
  identified.
* Names of the phone-model functions 0x6348F0 / 0x634A40, of 0x621960 and of the vtable owner at 0x86C200 (inferred
  `CEmergencyPed`).
* What samp's handlers for the custom opcodes 0x09C6/0x09D3 do, and the behaviour of every hook handler in general
  (only their GTA side is described here).
* Whether the partial repeat-sector relocation (one site moved, the rest left at 0xB992B8) is intentional.
* Why the `CPlayerPed` ProcessCollision hook permanently turns `CVehicle::SetupRender` into RET.
* The 0x4DF7E2 NOP leaves `mov bh,0` and stores an unset `eax` into 0xB61364. The effect is believed harmless but
  this is not verified.
* Which caller passes `true` to the `CPlayerPed` constructor after the 0x60D64E inversion.
