# World entities — objects, pickups, actors, checkpoints, gang zones, map icons, 3D labels (clean-room spec)

Overview. This file covers the RPCs that create and manipulate script-placed world entities
in the SA:MP 0.3.7-R4 client. Each layout below was taken from open.mp's `Shared/NetCode`
writer/reader and then checked against the client's handler (A = 2021 build) by following its
bitstream read calls in order and their bit widths, using the decompiled dump plus `objdump` where
the decompiler lost arguments. B (2019 build) was spot-checked on every non-trivial handler and
sender: the read and write sequences are identical (the only differences are struct offsets, for
example the object pool sits at pools+0x14 in B and at pools+0xC in A). All payload layouts
match open.mp. The client-side quirks to keep in mind are: SetObjectPos reads a 4th float; the
object-select reply always sends type 0; the gang zone ID is not bounds-checked; and the material
index check is off by one. Most GTA-side work is done by running single SCM opcodes through
the client's script-command helper. Opcode numbers are listed so the Rust client can do the same
or call the matching GTA functions. The custom-model ("artwork") RPCs exist in R4 only as dormant
code: they are never registered (see the end of this file and `session-misc.md`).

All addresses are A unless marked B. "rel" means the reliability enum value passed to the RPC send
call (8 = RELIABLE, 9 = RELIABLE_ORDERED, 10 = RELIABLE_SEQUENCED, see handshake.md §3). All
client sends use priority 1 and ordering channel 0.

## Index

| ID | open.mp name | dir | A handler / send site | B | confidence |
|---|---|---|---|---|---|
| 44 | CreateObject | S→C | 0x1001b340 (+ material reader 0x1001b070) | 0x1001b2f0 | high |
| 45 | SetObjectPosition | S→C | 0x1001ba10 | 0x1001b9c0 | high |
| 46 | SetObjectRotation | S→C | 0x1001bb20 | 0x1001bad0 | high |
| 47 | DestroyObject | S→C | 0x1001bc20 | 0x1001bbd0 | high |
| 99 | MoveObject | S→C | 0x1001bf00 | 0x1001beb0 | high |
| 122 | StopObject | S→C | 0x1001c0b0 | 0x1001c060 | high |
| 75 | AttachObjectToPlayer | S→C | 0x1001c6a0 | 0x1001c650 | high |
| 84 | SetPlayerObjectMaterial | S→C | 0x1001b6a0 | 0x1001b650 | high |
| 169 | (none in open.mp) object no-camera-collision | S→C | 0x1001c170 | 0x1001c120 | high (also in session-misc.md) |
| 113 | SetPlayerAttachedObject | S→C | 0x10018f00 | 0x10018eb0 | high |
| 27 | PlayerBeginObjectSelect | S→C | 0x1000e8b0 | 0x1000e880 | high |
| 27 | OnPlayerSelectObject | C→S | 0x1006dcd0 (not in rpc-ids.md send table) | 0x1006dc90 | high |
| 28 | PlayerCancelObjectEdit | S→C | 0x1000e970 | 0x1000e940 | high |
| 117 | PlayerBeginObjectEdit | S→C | 0x100118a0 | 0x10011880 | high |
| 117 | OnPlayerEditObject | C→S | 0x10072910 | 0x100728e0 | high |
| 116 | PlayerBeginAttachedObjectEdit | S→C | 0x100117e0 | 0x100117c0 | high |
| 116 | OnPlayerEditAttachedObject | C→S | 0x10072b20 | 0x10072af0 | high |
| 95 | PlayerCreatePickup | S→C | 0x1000f080 | 0x1000f050 | high |
| 63 | PlayerDestroyPickup | S→C | 0x1000f140 | 0x1000f110 | high |
| 131 | OnPlayerPickUpPickup | C→S | 0x10013440 (hook path), 0x10013520 (poll path) | 0x10013410 / 0x100134f0 | high |
| 171 | ShowActorForPlayer | S→C | 0x1000eab0 | 0x1000ea80 | high |
| 172 | HideActorForPlayer | S→C | 0x10011e00 (not a decompiled function; read from disassembly) | 0x10011de0 | high |
| 173 | ApplyActorAnimationForPlayer | S→C | 0x1001d750 | 0x1001d700 | high |
| 174 | ClearActorAnimationsForPlayer | S→C | 0x1001d930 | 0x1001d8e0 | high |
| 175 | SetActorFacingAngleForPlayer | S→C | 0x1001d9f0 | 0x1001d9a0 | high |
| 176 | SetActorPosForPlayer | S→C | 0x1001dad0 | 0x1001da80 | high |
| 178 | SetActorHealthForPlayer | S→C | 0x1001dbe0 | 0x1001db90 | high |
| 177 | OnPlayerDamageActor | C→S | 0x10006ac0 | 0x10006ab0 | high |
| 107 | SetCheckpoint | S→C | 0x10010170 | 0x10010150 | high |
| 37 | DisableCheckpoint | S→C | 0x1000e5a0 | 0x1000e570 | high |
| 38 | SetRaceCheckpoint | S→C | 0x10010280 | 0x10010260 | high |
| 39 | DisableRaceCheckpoint | S→C | 0x1000e5b0 | 0x1000e580 | high |
| 108 | ShowGangZone | S→C | 0x1001d080 | 0x1001d030 | high |
| 120 | HideGangZone | S→C | 0x1001d1a0 | 0x1001d150 | high |
| 121 | FlashGangZone | S→C | 0x1001d250 | 0x1001d200 | high |
| 85 | StopFlashGangZone | S→C | 0x1001d310 | 0x1001d2c0 | high |
| 56 | SetPlayerMapIcon | S→C | 0x1001a790 | 0x1001a740 | high |
| 144 | RemovePlayerMapIcon | S→C | 0x1001a8b0 | 0x1001a860 | high |
| 36 | PlayerShowTextLabel | S→C | 0x1000f720 | 0x1000f6f0 | high |
| 58 | PlayerHideTextLabel | S→C | 0x1000f8d0 | 0x1000f8a0 | high |
| 179 | ModelRequest | S→C | 0x10011eb0, **unregistered (dormant)** | 0x10011e90 | high (dormant) |
| 183 | ModelUrl | S→C | 0x100120b0, unregistered | — | medium (ID by shape) |
| 185 | DownloadCompleted | S→C | 0x1000ec30 candidate, unregistered | — | low |
| 181 / 182 | RequestDFF / RequestTXD | C→S | 0x1000d800 (variable-ID site), never reached | 0x1000d7f0 | high (dormant) |
| 184 | FinishDownload | C→S | 0x1000d920, never reached | 0x1000d910 | high (dormant) |

Not covered here, but related: 43 RemoveBuildingForPlayer and 81 AttachCameraToObject (see
`player-state.md`), and 97 dropped-weapon pickup (see `session-misc.md`).

## Shared conventions

### Bitstream primitives used by the handlers (A)
| A address | role |
|---|---|
| 0x1001f840 | build a read stream over the RPC payload (`bits/8 + 1` bytes, no copy) |
| 0x1001f9b0 | read N bits (right-aligned) — every `uN`/`iN`/`f32` below |
| 0x1001f920, 0x10008420 | read 1 bit |
| 0x1001fea0 | read N **bytes** (bit-unaligned allowed; used for raw struct blocks and strings) |
| 0x1001fa70 | read compressed integer |
| 0x1001ffe0 / 0x10020090 | write N bits / write N bytes (writer); 0x1001ff30 / 0x1001ff10 write bit 1 / 0 |
| 0x10053c30 + 0x100542d0 | string-compressor instance + "decode compressed string" |

"dynstr8" = u8 length followed by that many raw bytes. The client copies them into a zeroed
256-byte buffer, so the result is always NUL-terminated.

"cstr" = RakNet **compressed (Huffman) string**, decoded by 0x100542d0: a *compressed u16*
bit count, then that many Huffman-coded bits, decoded into a buffer of at most 2048 bytes (output is
truncated and NUL-terminated at max-1). If fewer bits remain than the count says, the decode fails and
the buffer stays empty. The Huffman frequency table was not traced here.

Floats are IEEE-754 f32, little-endian, like all multi-byte fields. `vec3` = 3 × f32 (x, y, z).

### Client entity pools (reached via netgame +0x3DE → pools struct, A offsets)
| pool | pools+ | capacity / ID check | notes |
|---|---|---|---|
| players | +0x04 | local id compared first; remote ≤ 1003/1004 depending on handler | |
| pickups | +0x08 | 4096 (`0 ≤ id < 0x1000`), also a total-count < 4096 check | |
| objects | +0x0C (B: +0x14) | arrays of 1000; handlers accept `id < 1001` (off by one, see below) | per id: "exists" flag at +4+4·id, wrapper pointer at +0xFA4+4·id |
| actors | +0x10 | `id < 1000` | |
| gang zones | +0x14 | 1024 slots, **no bounds check on the wire id** | |
| 3D labels | +0x18 | `id < 2048` (0x800) | |
| map icons | in the local-player/netgame object, +0x23D+4·id | `id < 100` | |

Object-ID bound: every object handler tests `id < 0x3E9` (≤ 1000), but the per-ID arrays have
1000 entries. So id 1000 indexes one past the flag array, into the first wrapper pointer. Treat
**0…999** as the valid range (open.mp's 0.3.7 object pool size is also 1000).

### SCM helper
Most game actions go through the client's script-command executor (A 0x100b2350). It takes a
descriptor (u16 opcode + format string such as `"ifff"`) followed by the arguments, packs them like
GTA's script engine does, and runs a single command through GTA's interpreter (a function pointer at
A 0x101173b8). Below, "SCM 0xNNNN" means one such call. Opcode names are given only where
confident and are marked "believed" otherwise.

---------------------------------------------------------------------------------------------------

## Objects

### 44 CreateObject (S→C)
- A 0x1001b340, B 0x1001b2f0. Material sub-reader A 0x1001b070 (B 0x1001b020), identical sequence.

| # | field | type | notes |
|---|---|---|---|
| 1 | object id | u16 | |
| 2 | model | i32 | negative = custom model (inactive in R4, see below) |
| 3 | position | vec3 | |
| 4 | rotation | vec3 | degrees |
| 5 | draw distance | f32 | 0 = keep model default |
| 6 | no camera collision | u8 | non-zero → object ignored by camera-collision test (same flag as RPC 169) |
| 7 | attached vehicle id | u16 | 0xFFFF = none |
| 8 | attached object id | u16 | 0xFFFF = none |
| 9 | *if 7 ≠ 0xFFFF or 8 ≠ 0xFFFF:* attach offset | vec3 | |
| 10 | … attach rotation | vec3 | |
| 11 | … sync rotation | u8 | used only for object-to-object attachment |
| 12 | material count | u8 | number of material records that follow |
| 13 | material records × count | see "material record" | |

**Material record** (shared with RPC 84 after its object id):

| field | type | notes |
|---|---|---|
| kind | u8 | 1 = texture material, 2 = material text; any other value → nothing else read for this record (and the following records will be misparsed) |
| slot | u8 | material index |
| *kind 1:* model | u16 | 0xFFFF → −1 (colour only); in RPC 44 also any value > 20000 → −1 (RPC 84 has only the 0xFFFF rule) |
| … txd name | dynstr8 | |
| … texture name | dynstr8 | |
| … colour | u32 | open.mp writes it as ABGR (= R,G,B,A bytes in memory) |
| *kind 2:* material size | u8 | |
| … font name | dynstr8 | |
| … font size | u8 | |
| … bold | u8 | |
| … font colour | u32 | open.mp writes ARGB |
| … background colour | u32 | open.mp writes ARGB |
| … alignment | u8 | |
| … text | cstr (max 2048) | |

Client behaviour (observed):
1. Reads every field before acting. Then calls the pool create routine (0x100128d0). If the slot is
   already occupied, the old object is destroyed first.
2. Object construction (0x100a0390 → 0x100a9150, 0x119C-byte wrapper): if the model has no
   model info, model 18631 (0x48C7, the "?" model) is used. Then SCM 0x0107 (believed
   create_object: model, x, y, z → handle) runs, followed by SCM 0x0453 (set object rotation)
   with the rotation. The rotation is kept at wrapper +0xAD/+0xB1/+0xB5 (the editor reads it
   later). If draw distance ≠ 0 and the model info is of a suitable type (checked against GTA
   model-info vtables 0x85BBF0…0x85BCF0), the model's draw distance is overridden (0x100b4590,
   inferred). Up to 16 material slots are cleared.
3. Attachment is stored, not applied at once: a vehicle id has priority (0x100a8150 stores the
   vehicle id, offset and rotation). Otherwise an object id is stored (0x100a81e0, also stores
   the sync-rotation byte). Both are applied later by the object's per-frame processing through
   SCM 0x0681 (attach object to car) / SCM 0x069A (attach object to object).
4. Materials are applied only if the object was created. Name strings must be < 32 chars,
   otherwise the record is skipped. A material text record whose text decodes to an empty string
   is also skipped.
   - kind 1 (0x100a8570): releases any texture already in that slot. Model 0 → the special
     "blanktex" texture. Model > 0 → the model is loaded (waits at most ~250 × 2 ms) and the
     texture `txd/texture` is looked up. Model −1 → no texture, only the colour. The colour is
     stored per slot.
   - kind 2 (A 0x100a86f0, B 0x100a86c0; analysed 2026-11-06, MATTEXT-1): releases the slot's texture
     (a kind-1 texture or an earlier text texture; its remembered D3D texture +0x110B is cleared too), an
     earlier render target (+0x114B, "created" flag +0x31B := 0) and the stored text. Then it creates a
     **2×2 RenderWare texture** (`RwRasterCreate(2, 2, 0, 0)` 0x7FB230 + `RwTextureCreate` 0x7F37C0, then
     `filterAddressing` (+0x50) += 1: filter NEAREST → LINEAR, measured as MIN/MAGFILTER 2 on the text draws) as the slot texture (+0x217), copies the text, and stores per slot (record of 0xD7 bytes at +0x3E4):
     slot, material size, font name, font size, bold (as 0/1), font colour, background colour, alignment.
     Slot colour +0x257 := 0 (so no colour modulation), slot kind +0x2DB := 2, "has materials" +0x118B := 1,
     and +0x110B := the 2×2 texture's D3D texture (raster +0x34). A kind-1 record on the same slot later
     releases the 2×2 texture and clears +0x110B, so the render target is never shown again (it stays allocated).
   - Render-target fill (object pool pass B 0x100A9EF0, once per slot with kind 2, a slot texture and
     +0x31B = 0; renderer B 0x100708E0, size table B 0x100A8C90):
     * size: 10 → 32×32, 20 → 64×32, 30 → 64×64, 40 → 128×32, 50 → 128×64, 60 → 128×128, 70 → 256×32,
       80 → 256×64, 90 → 256×128, 100 → 256×256, 110 → 512×64, 120 → 512×128, 130 → 512×256, 140 → 512×512;
       any other value → no render target. The render target has **exactly** that size (no scaling).
     * `GetDisplayMode` (result unused); `D3DXCreateTexture(w, h, 1, RENDERTARGET, fmt, DEFAULT)` with fmt
       **A8R8G8B8 (21) when the background colour is 0**, else X8R8G8B8 (22); `GetSurfaceLevel(0)`, `GetDesc`;
       `D3DXCreateRenderToSurface(desc w, desc h, desc format, depth 1, D16)`;
       `D3DXCreateFontA(face, height = font size, width 0, weight bold ? 700 : 400, mip 1, italic 0,
       charset 1, precision 0, quality 0, pitch 0)` (failure → debug line "SignText: Can't create font %s").
     * `rts.BeginScene(surface)`, `Clear(0, TARGET|ZBUFFER, background ARGB, z 0.0, 0)`, `sprite.Begin(0x10)`;
       **background 0 → `SetRenderState(DESTBLEND, BLENDFACTOR)`**; `DrawTextA(text, -1, [0, 0, w, h], fmt,
       font colour)` with fmt = alignment 1 → 0x105 (centre, vcentre, noclip), 2 → 0x102 (right, top, noclip),
       anything else → 0x100 (left, top, noclip); colour codes per glyph through samp's sprite wrapper;
       `sprite.End`, `rts.EndScene(0)`.
     * While the object renders, the device proxy's `SetTexture` swaps the 2×2 texture (+0x110B) for the
       render target (§4 RE-10), so the text covers exactly the RW materials of that slot's index.
   - Slot check is `slot < 17` against 16-entry arrays (off by one). A Rust client should accept
     only 0…15.
5. If the no-camera-collision byte is non-zero, it sets wrapper +0x53 = 1. That flag makes the
   camera-collision routine 0x10012de0 skip the object; that routine only tests objects within
   200 units.
- Negative model IDs are first looked up in the custom-model manager (A 0x1026ea10). In R4 that
  manager is never created, so the lookup is skipped and the model-validity check replaces the
  value with 18631.
- open.mp: identical order and widths. open.mp writes `!CameraCollision`, which matches the client's
  "non-zero = no collision" meaning. **No disagreement.**

### 45 SetObjectPosition (S→C)
- A 0x1001ba10, B 0x1001b9c0.

| field | type | notes |
|---|---|---|
| object id | u16 | |
| position | vec3 | |
| (extra) | f32 | **the client attempts a 4th f32 read and never uses it** |

- Behaviour: if the object exists, it calls the generic entity "set position" (0x1009f0a0). That
  function uses the GTA entity's virtual Teleport (vtable +0x38) and skips entities whose vtable
  is the GTA placeholder 0x863C40. Train models (449/537/538) go through SCM 0x07C7 instead, but
  that branch matters only for vehicles.
- Disagreement (harmless): open.mp sends only id + vec3. The extra read runs off the end of
  the payload, fails without effect, and the value is discarded. A Rust client should read
  id + vec3 and ignore any trailing data.

### 46 SetObjectRotation (S→C)
- A 0x1001bb20, B 0x1001bad0. Fields: object id u16, rotation vec3 (degrees).
- Behaviour: if the object and its GTA entity exist, SCM 0x0453 (set object rotation) runs and the
  rotation is stored at wrapper +0xAD.. (0x100a80e0). Matches open.mp.

### 47 DestroyObject (S→C)
- A 0x1001bc20, B 0x1001bbd0. Fields: object id u16.
- Behaviour: if the object exists, it first checks the local player's "contact/surf object"
  reference (local player wrapper +0x2DA, paired field +0x2E2). If that reference points to this
  object, both fields are cleared. Then the pool delete (0x10012850) runs, which destroys the
  wrapper (destructor uses SCM 0x0108, believed delete_object). Matches open.mp.

### 99 MoveObject (S→C)
- A 0x1001bf00, B 0x1001beb0.

| field | type | notes |
|---|---|---|
| object id | u16 | |
| current position | vec3 | **read but not used by R4** |
| target position | vec3 | |
| speed | f32 | units/s |
| target rotation | vec3 | each component ≤ −999 means "do not rotate" (open.mp sends −1000 for "no rotation") |

- Behaviour: ignored if the object is currently open in the object editor (editor active and its
  edited object id equals this id). Otherwise 0x100a9c10 runs. If a move is already in progress, it
  is stopped first and the object snaps to its previous target. Then it records the start time,
  target position and speed, and sets the "moving" flag (wrapper +0x1AF bit 0). If all three rotation
  components are > −999, it also computes a rotation target and per-axis deltas. It computes the
  remaining distance (0x1009f310), stored at +0x15B. The actual motion happens in the object's
  per-frame processing (not analysed in detail).
- open.mp: same layout. The client does **not** teleport to "current position". A Rust client
  that wants exact parity should also ignore it.

### 122 StopObject (S→C)
- A 0x1001c0b0, B 0x1001c060. Fields: object id u16.
- Behaviour: 0x100a8070 zeroes the GTA entity's move speed and turn speed and clears the moving
  flag. The object stays where it is. Matches open.mp.

### 75 AttachObjectToPlayer (S→C)
- A 0x1001c6a0, B 0x1001c650. The decompiler lost the arguments; widths come from the disassembly.

| field | type |
|---|---|
| object id | u16 |
| player id | u16 |
| offset | vec3 |
| rotation | vec3 |

- Behaviour: the object must exist. If player id = local player id, the target is the local ped.
  Otherwise the remote player (id ≤ 1004) must exist and have a ped. Then SCM 0x069B (attach
  object to char: object handle, ped handle, offset xyz, rotation xyz) runs immediately.
  Unknown player → nothing. Matches open.mp.

### 84 SetPlayerObjectMaterial (S→C)
- A 0x1001b6a0, B 0x1001b650. Fields: object id u16, then **one** material record (see 44).
- Behaviour: as in 44, step 4. The record is still read when the object id is invalid, but
  nothing is applied. Differences from the 44 path: there is no "> 20000 → −1" model rule, only 0xFFFF → −1.
- open.mp: same layout (type, slot, …). Matches.

### 169 object no-camera-collision (S→C, no open.mp struct)
- A 0x1001c170, B 0x1001c120. Fields: object id u16.
- Sets wrapper +0x53 = 1, the same as CreateObject field 6. It can never be cleared again. open.mp
  never sends it, because it puts the flag into CreateObject. Also documented in `session-misc.md`.

### 113 SetPlayerAttachedObject (S→C)
- A 0x10018f00, B 0x10018eb0.

| field | type | notes |
|---|---|---|
| player id | u16 | local id, or remote ≤ 1003 with an existing slot |
| slot index | u32 | 0…9 |
| create | bit | |
| *if create:* slot data | 52 raw bytes (read-bytes) | model i32, bone i32, offset vec3, rotation vec3, scale vec3, colour1 u32, colour2 u32 |

- Behaviour: the target is the player's ped wrapper (local: via local player +0x26 → +0x104;
  remote: the player entry's ped at +0x1DD).
  - create = 0 → 0x100aee90 removes slot `index` (if 0…9 and used): destroys the attached
    object and zeroes the 52-byte slot record.
  - create = 1 → 0x100b0b50. Requires the ped's GTA entity to be alive (not vtable 0x863C40).
    Index must be 0…9 and **bone must be 1…18**, otherwise nothing happens. An existing slot is
    replaced. The 52 bytes are copied into the slot record (ped wrapper +0x74 + 52·index).
    Negative models go through the dormant custom-model lookup. A standalone object wrapper
    is created at the ped position with that model, draw distance 200, and this rotation. Non-zero
    colour1 / colour2 are applied as the colour of material slot 0 / 1 (model −1 = colour only).
- open.mp: same layout. open.mp writes the colours ABGR and reads them back (RPC 116) as ARGB. The
  client simply echoes the stored bytes. That is an open.mp-internal inconsistency, not a wire
  conflict.

### 27 PlayerBeginObjectSelect (S→C)
- A 0x1000e8b0, B 0x1000e880. Empty payload.
- Behaviour: leaves textdraw-select mode if it is active (global A 0x1026e9e0 → 0x10071580). Stops the
  object editor if it is active (0x10072c20(0), which sends a "cancel" 117/116, see below). Then it
  enables object-selection mode (A 0x1026e9dc → 0x1006db80(1): shows the cursor, clears a camera/UI field).
- In selection mode, the per-frame routine 0x1006dc00 picks the object under the mouse
  (0x1006da10). It accepts it only if the object exists and is closer than about 150 units, and
  highlights it. A left-button-up window message (0x202) then sends 27 C→S.

### 27 OnPlayerSelectObject (C→S)
- Send site A 0x1006dcd0 (B 0x1006dc90), rel 9. This site is **missing from rpc-ids.md's send
  table** (the ID cell A 0x100e5e9c is shared with the S→C registration).

| field | type | notes |
|---|---|---|
| select type | u32 | **always 0 in R4** (constant) |
| object id | u16 | the client object id (global and player objects share one client ID space) |
| model | u32 | object's model (0x1009f090) |
| position | vec3 | object's current matrix position |

- Selection mode is not left automatically after a click. The server ends it with 28.
- open.mp names the first field SelectType but ignores it. It resolves the object by id and model,
  first as a global object and then as a player object. No functional conflict, but do not expect 1/2 values
  from this client.

### 28 PlayerCancelObjectEdit (S→C)
- A 0x1000e970, B 0x1000e940. Empty payload.
- Behaviour: if the object editor is active → 0x10072c20(0) (ends editing and sends a final
  117 or 116 with response 0 = cancel). If selection mode is active → 0x1006db80(0) (hides the cursor and
  leaves selection mode).

### 117 PlayerBeginObjectEdit (S→C)
- A 0x100118a0, B 0x10011880.

| field | type |
|---|---|
| player object | bit |
| object id | u16 |

- Behaviour: leaves selection mode and textdraw-select mode. Then the editor (global A 0x1026e9d8)
  starts in mode 1 = object (0x10072480): stores the id, the player-object bit (+0x8E) and the
  object's current rotation (from wrapper +0xAD..), and shows the cursor. The gizmo UI is described in
  "Object editor and selection UI" below (M6-P).

### 117 OnPlayerEditObject (C→S)
- Send site A 0x10072910 (B 0x100728e0), rel 9. Called with response 0 (cancel, from
  0x10072c20(0)), 1 (final, from the editor's save action 0x10072c20(1)) or 2 (update, from the
  per-frame editor 0x10073270).

| field | type | notes |
|---|---|---|
| player object | bit | echo of the bit from S→C 117 |
| object id | u16 | |
| response | u32 | 0 cancel, 1 final, 2 update |
| position | vec3 | object matrix position |
| rotation | vec3 | editor's rotation copy |

- Sent only if the object still exists. Response 2 is rate-limited: at most one every 250 ms
  (GetTickCount). Matches open.mp's reader.

### 116 PlayerBeginAttachedObjectEdit (S→C)
- A 0x100117e0, B 0x100117c0. Fields: slot index u32.
- Behaviour: leaves selection mode. The editor starts in mode 2 = attached object (0x10072540):
  stores the index, sets the edited object id to 0xFFFF, and shows the cursor. The index is not
  validated here.

### 116 OnPlayerEditAttachedObject (C→S)
- Send site A 0x10072b20 (B 0x10072af0), rel 9. Only for final/cancel (0x10072c20). No periodic
  updates in this mode.

| field | type | notes |
|---|---|---|
| response | u32 | 0 cancel, 1 final |
| slot index | u32 | |
| slot data | 52 raw bytes | the local ped's slot record: model, bone, offset, rotation, scale, colour1, colour2 |

- If the index is outside 0…9 or the slot is unused, the 52-byte block is **omitted** (null
  source → write-bytes writes nothing). The payload is then 8 bytes, and open.mp's reader fails.
- open.mp: same layout when the slot exists.

### Object editor and selection UI (M6-P; samp.dll B addresses, measured in run orig-20261105-042300)

- Editor object (global 0x1026E9D8): made at start-up with one `ID3DXLine` and two SAMPAUX3 fonts (22 and 28).
  State: active, kind (1 object / 2 attached slot), mode (0 translate, 1 rotate, 2 scale; objects never get 2),
  dragging flag, hovered handle (−1, 0–2 axes, 3–5 mode buttons, 10 save), a "drawn" flag, the size of the
  background glyph, seven hit rects, per-axis screen-direction flags (mostly vertical? inverted?), the cursor at the
  press, a rotation copy, an edit-matrix copy and the tick of the last 117.
- Start (117 or 116 S→C): kind/mode set, cursor mode 0 with an immediate restore, run the restore at once; the
  scale button is shown only for attached slots. Stop (cancel, save or RPC 28): cursor mode 0 (immediate), the chat
  input is closed, then the final 117 (only if the object exists) or 116.
- Per Present (UI step 12, before the selection): when not dragging, cursor mode 3 with flag 1 and the real device's
  cursor shown, or cursor mode 0 while the sprint key is held; while dragging the device cursor is hidden. Then the
  gizmo is drawn around the edited object's matrix (attached slots in translate mode: the bone's axes, the object's
  position), half length = camera distance × 0.125:
  - probe: background glyph `C` measured with DT_CALCRECT (only its size is kept);
  - for X, Y, Z: both ends (±half length along the axis) projected; any end with z > 1 ends the whole drawing (not
    "drawn"); an antialiased 2-point line (X 0xFF9999CC, Y 0xFFCC9999, Z 0xFF99CC99); unless dragging, the axis'
    screen direction is stored (mostly vertical or horizontal, and whether + points left/up); a handle at the + end;
  - buttons at the − end of Z, right to left 2 px + glyph width apart: save, [scale], rotate, translate.
  - Handle = background `C` (SAMPAUX3 28, count 1, rect = the point + 1, format 0x105; 0x99222222, hovered
    0xFFAAAAAA, the dragged axis / the active mode 0xFF774444) and its glyph (SAMPAUX3 22): axes `6 7 8`
    (translate), `3 4 5` (rotate), `9 A B` (scale) in the axis colours; buttons `2` / `0` / `1` / `H` in 0xFF999999.
    The hit rect is the glyph size centred on the point.
  - Unless dragging: hover test of the client cursor against the rects (only when drawn and the cursor mode ≠ 0).
- Window messages (checked before the chat input): LMB down on an axis handle starts a drag (capture, Windows
  cursor hidden, clip off, the object matrix copied as the edit matrix); every mouse move during the drag takes the
  cursor delta on x when the axis is mostly horizontal (on y otherwise; swapped in rotate mode), negated when
  inverted, × 0.01, applies it and puts the cursor back to the client centre; LMB up ends the drag (capture off,
  cursor shown at the handle's rect corner, hover test). LMB up on a mode button sets the mode, on save stops with
  response 1. `WM_CHAR` ESC stops with response 0 (the key-down still reaches GTA).
- Drag apply, objects: clear physical flag bit 3; translate moves the edit matrix' position along its own axis and
  starts MoveObject at speed 1 towards it (no rotation change), so the object follows with a lag; rotate adds d × 10°
  to the rotation copy (wrapped once by ±360 outside (−180, 180]) and sets it; then 117 response 2 when ≥ 250 ms
  (GetTickCount) passed since the last 117. Attached slots: offset ± d × 0.1, rotation ± d × 10 (wrapped), scale
  ± d × 0.1 (negative → 0); nothing is sent until the end.
- Selection (global 0x1026E9DC): per Present cursor mode 3 / flag 0 with the device cursor (0 / 1 while sprint is
  held); with a visible cursor a ray through the cursor (inverse of WORLD × VIEW from the device proxy's copies,
  unnormalised direction, 150 m) picks the nearest pool object (by distance to the ray start, < 150) the segment
  hits: a sphere cast with 0.25-radius spheres (with samp's own collision cache swapped in) when the object uses
  collision and its model has collision data, otherwise a frustum-checked line test against the bounding sphere
  (radius < 0.5 → 0.75, < 1 → × 1.5). The hovered object (< 150 m from the camera) gets its axis cross (length 1,
  0xFFCC9999) and the label `[id: %d, type: %d]\nDistance: %.2fm\n` (outlined small font, 0xFFD47B75, no LOS test).
  LMB up sends 27 for the hovered object; ESC (`WM_CHAR`) leaves the mode.

---------------------------------------------------------------------------------------------------

## Pickups

### 95 PlayerCreatePickup (S→C)
- A 0x1000f080, B 0x1000f050.

| field | type | notes |
|---|---|---|
| pickup id | i32 | 0…4095 |
| model | i32 | read as part of a 20-byte raw block |
| type | i32 | |
| position | vec3 | |

(The handler reads the id with read-bits(32), then the last 20 bytes as one read-bytes block. The bit
layout is the same as 5 separate 32-bit fields.)
- Behaviour (0x10013270, only if the pickup pool exists): requires total count < 4096 and the id in
  range. An existing pickup in that slot is removed (SCM 0x0215, believed remove_pickup). Stores
  the 20-byte record and clears the "dropped-weapon" marker. Then 0x100a1230: invalid model →
  18631. Models 0…20000 are requested and loaded (SCM 0x0247/0x038B/0x0248, polling). Then
  SCM 0x0213 (believed create_pickup: model, type, x, y, z → handle). The GTA pickup array index
  is derived from the handle (low 16 bits) and stored for reverse lookup. Count is incremented.
- Matches open.mp.

### 63 PlayerDestroyPickup (S→C)
- A 0x1000f140, B 0x1000f110. Fields: pickup id i32.
- Behaviour (0x10013320): if count > 0, the id is in range and the slot is live → SCM 0x0215. Clears the slot
  (GTA index := −1) and decrements the count.

### 131 OnPlayerPickUpPickup (C→S)
- Fields: pickup id i32.
- Two send paths:
  1. **Collision hook** (rel 9): GTA code at **0x4579C6** is patched with a 5-byte jump to A
     0x100a3f80. That stub computes the GTA pickup index as `(CPickup* − 0x9788C0) / 32` (GTA's pickup array).
     It calls 0x10013440, which maps the GTA index back to the SA:MP id. It sends only if the slot is live,
     its cooldown is 0 and it is not a dropped-weapon slot. Then the cooldown is set to 15.
  2. **Poll** (0x10013520, rel 10, run from the netgame tick 0x10008b20 every ~6 ticks): for each
     live slot that is type 14 (or a dropped-weapon slot), SCM 0x0214 (believed
     has_pickup_been_collected) runs. If it returns true, 131 is sent with the id (or 97 for dropped weapons).
     For every other slot this path only counts the cooldown down by 1.
- Matches open.mp's reader (single i32).

---------------------------------------------------------------------------------------------------

## Actors

### 171 ShowActorForPlayer (S→C)
- A 0x1000eab0, B 0x1000ea80. One 27-byte raw read:

| field | type |
|---|---|
| actor id | u16 |
| skin | u32 |
| position | vec3 |
| facing angle | f32 (degrees) |
| health | f32 |
| invulnerable | u8 |

- Behaviour (actor pool 0x10001900, only if the pool exists and the skin has valid model info): if the slot
  is occupied, it logs "Warning: actor %u was not deleted" and deletes the old actor. The new actor
  (0x1009c310) loads the skin model (busy-wait), then runs SCM 0x009A (believed create_char) with
  ped type 5, skin, x, y, **z − 1.0**. Then SCM 0x0173 (set heading), SCM 0x0446 (handle, 0) and
  SCM 0x060B. Then health is applied (see 178) and invulnerability: SCM 0x02AB (believed set_char_proofs)
  with (1,1,1,1,1) if invulnerable, else (0,0,0,1,0).
- open.mp: same, for non-DL clients (DL adds a custom-skin u32, which the R4 client must never
  receive). Matches.

### 172 HideActorForPlayer (S→C)
- A 0x10011e00, B 0x10011de0. Fields: actor id u16.
- Behaviour: 0x100016f0. If id < 1000 and live: destroys the actor (virtual destructor) and
  clears the slot.

### 173 ApplyActorAnimationForPlayer (S→C)
- A 0x1001d750, B 0x1001d700.

| field | type |
|---|---|
| actor id | u16 |
| anim library | dynstr8 |
| anim name | dynstr8 |
| delta | f32 |
| loop | bit |
| lock X | bit |
| lock Y | bit |
| freeze | bit |
| time | u32 |

- Behaviour (0x1009c4c0): requires the actor and its ped. **Library "SEX" (case-insensitive) is
  refused.** The library is loaded if needed (SCM 0x04ED / poll 0x04EE, giving up after ~15 ms). Then
  SCM 0x0812 (ped, name, library, delta, loop, lockX, lockY, freeze, time). Matches open.mp.

### 174 ClearActorAnimationsForPlayer (S→C)
- A 0x1001d930. Fields: actor id u16.
- Behaviour: calls GTA 0x601640 with argument 1 on the actor's ped (via a small stub that loads the
  ped from a global). Inferred: flushes the ped's tasks immediately.

### 175 SetActorFacingAngleForPlayer (S→C)
- A 0x1001d9f0. Fields: actor id u16, angle f32 (degrees).
- Behaviour: converts to radians (0x100b59b0) and writes the ped's float at +0x55C (inferred:
  target heading). The current heading at +0x558 is not written here.

### 176 SetActorPosForPlayer (S→C)
- A 0x1001dad0. Fields: actor id u16, position vec3. The generic entity "set position" (0x1009f0a0, as in 45) teleports the ped.

### 178 SetActorHealthForPlayer (S→C)
- A 0x1001dbe0. Fields: actor id u16, health f32.
- Behaviour (0x1009c630): writes CPed health (+0x540). If the result is ≤ 0, it runs SCM 0x0321 on the
  ped (inferred: kill).

### 177 OnPlayerDamageActor (C→S)
- Send site A 0x10006ac0 (B 0x10006ab0), rel 9.

| field | type | notes |
|---|---|---|
| (unknown) | bit | always 0 |
| actor id | u16 | |
| damage | f32 | |
| weapon | u32 | |
| body part | u32 | |

- Trigger: GTA damage-response code is patched near **0x4B5AC0** (the stub returns to 0x4B5AC6) and
  calls A 0x100a3bf0. When the attacker is the local ped and the victim is not a player ped but maps
  to an actor id (0x100a3bc0), 177 is sent with fields from GTA's damage record. (The same
  hook sends 115 for player damage, which is outside this file.)
- Matches open.mp (`Unknown` bit, then id, damage, weapon, body part).

---------------------------------------------------------------------------------------------------

## Checkpoints

The game-wide object (A 0x1026ea24) holds one normal checkpoint (active flag +0x4D, pos +0x35,
extent +0x41, blip +0x51) and one race checkpoint (active +0x29, pos +0x0C, next +0x18, size
+0x24, type +0x28, blip +0x2D, GTA race-CP handle +0x31). The client never reports
entering or leaving a checkpoint. The server detects that from position sync.

### 107 SetCheckpoint (S→C)
- A 0x10010170, B 0x10010150. Fields: position vec3, size f32.
- Behaviour: stores the position and sets the extent to (size, size, size) (0x100a1e20). If a blip exists, it
  is recreated at the new position. Sets active = 1.
- Per frame (0x100a1f60, called from 0x100074c0): while active, it runs SCM 0x00FE (believed
  locate_char_any_means_3d) for the local ped with pos, extent and "show sphere" = 1. That draws the
  checkpoint marker. A radar blip is created if missing: SCM 0x04CE (short-range coord blip,
  sprite 0) + SCM 0x0165 (handle, 0x3ED) + SCM 0x0168 (handle, 3). When not active, the blip is
  removed (SCM 0x0164).

### 37 DisableCheckpoint (S→C)
- A 0x1000e5a0. Empty payload. It only clears the active flag. The blip goes away on the next frame.

### 38 SetRaceCheckpoint (S→C)
- A 0x10010280, B 0x10010260.

| field | type |
|---|---|
| type | u8 |
| position | vec3 |
| next position | vec3 |
| size | f32 |

- Behaviour (0x100a2140): stores the fields and recreates the blip if one exists. Deletes any existing GTA race
  checkpoint (SCM 0x06D6), then SCM 0x06D5 (believed create_racing_checkpoint: type, pos, next,
  size → handle). Sets active = 1. A blip is added per frame like the normal checkpoint.

### 39 DisableRaceCheckpoint (S→C)
- A 0x1000e5b0. Empty payload. Clears active. The next frame removes the blip and deletes the GTA race
  checkpoint (SCM 0x06D6).

---------------------------------------------------------------------------------------------------

## Gang zones

Pool at pools+0x14: 1024 pointer slots + 1024 "used" flags. Each zone is a 24-byte heap record:
`[minX, maxY, maxX, minY, colour, current/flash colour]`, laid out as the GTA rectangle the
radar function expects.

### 108 ShowGangZone (S→C)
- A 0x1001d080, B 0x1001d030.

| field | type | notes |
|---|---|---|
| zone id | u16 | **not bounds-checked by the client**; values ≥ 1024 corrupt memory. A Rust client must reject them. |
| min X | f32 | |
| min Y | f32 | |
| max X | f32 | |
| max Y | f32 | |
| colour | u32 | open.mp writes ABGR (memory bytes R,G,B,A = GTA CRGBA) |

- Behaviour (0x10002180): frees an existing record, allocates a new one, and sets flash colour := colour.
- Drawing: GTA radar code at **0x5869BF** gets a `call` to A 0x100a3f40. That calls 0x10002270, which
  toggles a global phase every 500 ms and draws every used zone with GTA **0x5853D0**
  (believed CRadar::DrawAreaOnRadar(rect, colour, flag), flag from GTA 0xBA67A4) using colour or
  flash colour depending on the phase.

### 120 HideGangZone (S→C)
- A 0x1001d1a0. Fields: zone id u16 (unchecked). Frees the record and clears the slot.

### 121 FlashGangZone (S→C)
- A 0x1001d250. Fields: zone id u16, flash colour u32 (ABGR). Sets record[5] (flash colour). No
  effect if the slot is empty.

### 85 StopFlashGangZone (S→C)
- A 0x1001d310. Fields: zone id u16. Sets flash colour := colour.

All four match open.mp (open.mp writes the id as a signed 16-bit value; the bits are the same).

---------------------------------------------------------------------------------------------------

## Map icons

### 56 SetPlayerMapIcon (S→C)
- A 0x1001a790, B 0x1001a740.

| field | type | notes |
|---|---|---|
| icon id | u8 | < 100, otherwise ignored |
| position | vec3 | |
| type | u8 | radar sprite id (0 = plain coloured square) |
| colour | u32 | open.mp writes RGBA; used only when type = 0 |
| style | u8 | selects the blip kind, see below |

- Behaviour (0x1000a300 → 0x100a0dd0): removes the existing blip in that slot (SCM 0x0164), then creates
  a coord blip with sprite = type. The opcode depends on style: 0 → SCM 0x04CE (short-range),
  1 → SCM 0x02A8, 2 → SCM 0x0570, 3 → SCM 0x02A7, anything else → 0x04CE. (Believed: 1 = global, 2 =
  short-range with checkpoint, 3 = global with checkpoint, matching the public MAPICON_* styles.)
  If type = 0: SCM 0x0165 (handle, colour), then SCM 0x0168 (handle, 2) if colour < 1004, else
  (handle, 3).
- Matches open.mp.

### 144 RemovePlayerMapIcon (S→C)
- A 0x1001a8b0. Fields: icon id u8. If < 100: SCM 0x0164 on the stored handle (even if 0), then the slot is cleared.

---------------------------------------------------------------------------------------------------

## 3D text labels

### 36 PlayerShowTextLabel (S→C)
- A 0x1000f720, B 0x1000f6f0.

| field | type | notes |
|---|---|---|
| label id | u16 | < 2048. open.mp sends player labels as 1024 + id |
| colour | u32 | open.mp writes RGBA. The client rotates it right by 8 bits into ARGB |
| position | vec3 | absolute, or an offset when attached |
| draw distance | f32 | |
| test LOS | u8 | |
| attached player | u16 | 0xFFFF = none |
| attached vehicle | u16 | 0xFFFF = none |
| text | cstr (max 2048) | |

- Behaviour: requires the label pool. Fields are read and the text decoded before the id check. Then
  0x100011d0 replaces any existing label in the slot and keeps a heap copy of the text. If the text is
  shorter than 400 chars, it is post-processed by 0x100b6140 with limit 399. [O] That helper is the `~k~`
  key-name filter, not a colour pass: it acts only when the text is shorter than 400 chars and contains
  `~k~`; it runs GTA's `CMessages::InsertPlayerControlKeysInString` (0x69E160) on a 2 KiB copy and writes
  the result back only if its length is ≤ the limit (signed `>` test). Colour codes are stored unchanged.
- Rendering (0x10001350, per frame, entries of 0x1D = 29 bytes):
  - Player attachment is checked first. The player (id < 1004) must exist with a ped. The position is
    head-bone (bone 8) position + offset, and z gets an extra + 0.05 × camera distance.
  - Otherwise vehicle attachment: the vehicle must exist. The offset is transformed by the vehicle matrix.
  - Otherwise the label is drawn at the absolute position.
  - It is drawn only when camera distance ≤ draw distance, via 0x1006bc90 with (pos, text, colour,
    "alpha = 0xFF" flag, test-LOS flag). [O] The alpha flag is the painter's **outline** flag: a label whose
    colour alpha is not FF gets no black outline passes. The painter hands the raw text (codes included) to
    the font renderer's small draw (B 0x1006b4a0), whose main pass goes through the colour-code wrapper
    (ui-rendering.md §3.3), so `{RRGGBB}` codes colour the following glyphs (alpha forced to FF after the
    first code). Measured: tools/diff/visual.sh elements `label3d` / `bubblecol` (WORKLOG 2026-11-06).
- Matches open.mp.

### 58 PlayerHideTextLabel (S→C)
- A 0x1000f8d0, B 0x1000f8a0. Fields: label id u16. If < 2048 and used → 0x100012e0 frees it.

---------------------------------------------------------------------------------------------------

## Custom models (custommodels.hpp) — not registered in R4

The R4 DLL contains a complete custom-model download client, but **none of its S→C handlers is
registered**. The two registration functions (A 0x10012450 / 0x1001e130) never reference them,
and a raw search of both DLLs finds no pointer to them. The manager (global A 0x1026ea10) is
created only inside the dormant 179 handler, so it stays NULL. The netgame tick (A 0x1000b604)
therefore never runs the request loop, and 181/182/184 are never sent. A Rust R4 profile must
**not** register 179/183/185. Layouts found (all agree with open.mp):

- **179 ModelRequest**, dormant A 0x10011eb0 / B 0x10011e90: u32 pool id, u32 count, u8 type,
  i32 world, i32 base id, i32 new id, u32 DFF crc, u32 TXD crc, u32 DFF size, u32 TXD size,
  u8 time on, u8 time off.
- **183 ModelUrl**, dormant A 0x100120b0 (ID inferred from shape): the first u8 is a sub-type. Sub-type 6 =
  u8 file type, u32 crc, dynstr8 URL (this is what open.mp sends). Other sub-types (0, 1, 2, 5)
  exist. See `session-misc.md`.
- **185 DownloadCompleted**: unregistered. Best candidate is **A 0x1000ec30**: empty payload, sets
  the manager's "done" byte +0x221, which is the flag that stops the download tick and the 184
  sender, then calls 0x1000c0c0. That fits open.mp sending 185 in reply to 184. `session-misc.md`
  instead suggests A 0x1000eba0 (re-initialises the cache). Both are dormant; low confidence either way.
- **181 / 182 RequestDFF / RequestTXD** (C→S): variable-ID site A 0x1000d800 (cells 0x100e5d90 =
  181, 0x100e5d8c = 182), payload u32 crc, rel 9.
- **184 FinishDownload** (C→S): A 0x1000d920 (cell 0x100e5d94), payload is a single 1-bit, rel 9.
  open.mp reads nothing, so the bit is harmless.

---------------------------------------------------------------------------------------------------

## Disagreements and notes for the Rust client

1. **45 SetObjectPosition**: the client reads an extra f32 after the position, but open.mp
   does not send it. The read fails and is ignored. Do not require it.
2. **99 MoveObject**: "current position" is read but unused by R4.
3. **27 OnPlayerSelectObject**: R4 always writes select type = 0. open.mp ignores the field.
4. **116 OnPlayerEditAttachedObject**: for an empty or invalid slot the 52-byte block is omitted
   (8-byte payload). open.mp's reader then fails.
5. **113 / 116 colours**: open.mp writes ABGR and reads ARGB. The client echoes bytes verbatim.
6. Client robustness bugs a Rust client should not copy: gang zone id unchecked (108/120/121/85);
   object id accepted up to 1000 against 1000-entry arrays; material slot accepted up to 16
   against 16-entry arrays.
7. rpc-ids.md gap: the C→S send of RPC 27 (A 0x1006dcd0 / B 0x1006dc90, rel 9) is not in its
   send table.

---

## RE-10 additions (static, 2026-11-04)

Static analysis of A for milestone M6. Tags: **Observed** (read from samp.dll A / gta_sa.exe 1.0 US),
**Inferred**. GTA and SCM names via gta-reversed (`Enums/eScriptCommands.h`) / plugin-sdk.

### 1. Opcode names (replaces "believed" above) — Observed
0x0107 CREATE_OBJECT, 0x0108 DELETE_OBJECT, 0x0453 SET_OBJECT_ROTATION, 0x0681 ATTACH_OBJECT_TO_CAR,
0x069A ATTACH_OBJECT_TO_OBJECT, 0x069B ATTACH_OBJECT_TO_CHAR, 0x0682 DETACH_OBJECT, 0x0685 IS_OBJECT_ATTACHED,
0x0213 CREATE_PICKUP, 0x0214 HAS_PICKUP_BEEN_COLLECTED, 0x0215 REMOVE_PICKUP, 0x009A CREATE_CHAR,
0x0173 SET_CHAR_HEADING, 0x0446 SET_CHAR_SUFFERS_CRITICAL_HITS, 0x060B SET_CHAR_DECISION_MAKER, 0x02AB SET_CHAR_PROOFS,
0x04ED REQUEST_ANIMATION, 0x04EE HAS_ANIMATION_LOADED, 0x0812 TASK_PLAY_ANIM_NON_INTERRUPTABLE,
0x00FE LOCATE_CHAR_ANY_MEANS_3D, 0x06D5 CREATE_CHECKPOINT, 0x06D6 DELETE_CHECKPOINT,
0x04CE ADD_SHORT_RANGE_SPRITE_BLIP_FOR_COORD, 0x02A8 ADD_SPRITE_BLIP_FOR_COORD,
0x0570 ADD_SHORT_RANGE_SPRITE_BLIP_FOR_CONTACT_POINT, 0x02A7 ADD_SPRITE_BLIP_FOR_CONTACT_POINT, 0x0165 CHANGE_BLIP_COLOUR,
0x0168 CHANGE_BLIP_SCALE, 0x0164 REMOVE_BLIP, 0x0161 ADD_BLIP_FOR_CAR_OLD. GTA functions: 0x601640
`CPedIntelligence::FlushImmediately` (RPC 174), 0x5853D0 `CRadar::DrawAreaOnRadar` (gang zones).
* **Correction (178 / 166):** 0x0321 is `EXPLODE_CHAR_HEAD`, not a plain "kill". In gta-reversed it builds a damage event
  (no damager, weapon 34 sniper, head, 1000 damage) and runs `ComputeDamageResponseIfAffectsPed`, i.e. it goes through
  samp's `ComputeDamageResponse` hook (0x4B5AC0). For an actor victim the damager is not the local ped, so rule 1 of the
  combat cancel table ("damager ≠ local and victim is a CCivilianPed → cancel") applies. What the actor then does (dies
  from health ≤ 0 alone, or not; head dismembered or not) is a runtime question (measurement request M6).

### 2. Object streaming and the two render paths — Observed
* samp marks every object it creates with entity flag bit 31 (+0x1C, A 0x1009F780, from the object ctor A 0x100A9150).
* `CRenderer::AddEntityToRenderList` hook (row 78, A 0x100A5150): a marked `CObject` (vtable 0x866F60) whose samp
  object has materials (+0x118B), a draw distance > 300.0 (constant A 0x100E6170) or the "timed" flag (+0x1199) is **not**
  added to GTA's render list. Everything else goes to the original.
* Those objects are streamed and drawn by samp itself:
  * per frame (object process, §3) A 0x100A95A0: distance = camera position (`TheCamera` matrix pos 0xB6F9CC) to the
    object, stored at +0x68. Out of range (> draw distance +0x54, or timed object outside its hours): after the RW object
    has existed for 1000 consecutive out-of-range frames, it is destroyed and the model released if unused. In range:
    request the model (priority 2) until loaded, create the RW object, mark +0x49 = "drawn by samp", and clear the
    model's "pre-rendered" flag.
  * `RenderScene` hooks: after `CRenderer::RenderEverythingBarRoads` (row 76, A 0x100A50D0) samp draws its opaque objects
    (+0x49 set, model not draw-last); after `CRenderer::RenderFadingInEntities` (row 77, A 0x100A5110) it draws the
    draw-last ones (+0x6C = model flag bit 1), sorted far → near.
  * Each manual draw (A 0x100A9FD0): skipped by a distance/visibility test on stored bounds (+0x58/+0x5C, not analysed).
    Draw-last models: RW alpha-test reference (render state 30) := 100 unless 0xB72914 is set or the model has flag bit 3
    (no z-write), else 0. Cull mode (render state 20) := back-face (2) if the model has flag bit 6, else none (1). Then
    materials (§4), lighting for distances < 600, render the RW object, restore.
* Ordinary objects (no materials, draw distance ≤ 300, not timed): if the RW object is missing and the camera is closer
  than draw distance − 30, the model is requested (priority 2); GTA streams and renders them as usual.
* CObject `CreateRwObject` hook (row 67, A 0x100A4B30): NOPs the 3-byte `call [edx+0x20]` at 0x4096AA in
  `CStreaming::AddEntity` while `CEntity::CreateRwObject` (0x533D30) runs, then restores it (row 372). Inferred: creating
  an RW object must not make the streamer delete another entity's RW object.
* CObject/CBuilding `DeleteRwObject` hook (rows 68/69, A 0x100A4BA0): for models 3586, 3743, 8979, 8980 (escalators) it NOPs
  the `CEscalator::SwitchOff` call (0x71791F, row 373) around `CEntity::DeleteRwObject` (0x534030), so escalators keep
  running when their RW object is dropped.

### 3. Per-frame object process (MoveObject, attachments) — Observed
Runs in the CGame::Process pre-step (`remote-control-vehicles.md` RE-10 A), A 0x10012A50 → A 0x100A9690(dt), with dt in
seconds from samp's millisecond clock (A 0x1002E7F0) since the previous pass.
1. Streaming as in §2.
2. Attached to a vehicle (+0xBE ≠ 0xFFFF): the vehicle must exist with a live entity; if IS_OBJECT_ATTACHED (0x0685) is
   false → ATTACH_OBJECT_TO_CAR (0x0681) with the stored offset (+0xC2..) and rotation (+0xCE..). Re-done whenever GTA
   reports it detached (e.g. after the car is re-created). Nothing else runs for attached objects.
3. Attached to an object (+0xC0 ≠ 0xFFFF): same with ATTACH_OBJECT_TO_OBJECT (0x069A).
4. Moving (+0x1AF bit 0):
   * step = dt × speed (+0x1B0); remaining = |target (+0x10B..) − current position|; expected travel =
     (now − start time +0x15F) × 0.001 × speed; travelled = total distance (+0x15B) − remaining.
   * remaining ≤ step → arrival: move and turn speed := 0, position := target, rotation := target rotation if a rotation
     was requested, matrix written (entity removed, matrix set, re-added), moving flag cleared.
   * otherwise (dt > 0): next = current + (target − current) × step / remaining; velocity = (next − current) / dt × 0.02
     (GTA speed units per 1/50 s). If |expected − travelled| > 0.1 the velocity is scaled by 1 + (expected − travelled) × 0.1
     (behind schedule → faster, ahead → slower). Move speed := velocity, and samp itself advances the matrix by
     move speed × `CTimer::ms_fTimeStep` (A 0x1009FDC0). With rotation: turn speed z := clamp(0.01 × angle difference,
     ±0.001), the rotation is the slerp of the start quaternion (+0x177) toward the target quaternion (+0x187) by
     progress = 1 − remaining / total, and the matrix is rewritten with the kept position.
* MoveObject setup (A 0x100A9C10): a move in progress is stopped and the object snaps to its old target (and target
  rotation); then start time, target, speed and the moving bit are set; rotation is used only if all three components
  are > −999 (stored flag +0x1B5).

### 4. Materials (RPC 44/84) at render time — Observed
* The CObject Render hook (row 63, A 0x100A45B0) finds the samp object for the entity (object pool, else a player's
  attached-object slot, A 0x10013B70), runs the material pre-pass, calls `CObject::Render` (0x59F180) and the post-pass.
* Pre-pass (A 0x100A8C40): the first render after the RW object changes records the original material textures and
  colours (atomic callback); later renders apply, for material index i < 16 of each atomic's geometry: slot texture
  (+0x217 + 4i) → material texture; non-zero slot colour (+0x257 + 4i) → geometry flags |= MODULATEMATERIALCOLOR (0x40) and
  &= ~PRELIT (0x08), surface properties (ambient 1, specular 0, diffuse 1), material colour := slot colour. The material
  "slot" is therefore the RW material index in geometry order. Objects with the lighting flag (+0x1193) get entity bit 28
  and `SetupLighting` / `RemoveLighting` around the render. The post-pass restores the recorded state.
* Gate (A 0x100AA082 samp pass, A 0x100A9E70 CObject::Render hook): the pre-pass runs only for objects with
  materials (+0x118B) whose "drawn" flag +0x118F is set; +0x118F := 1 at the end of every draw (samp pass, after
  the visibility test) and in the CObject::Render hook before the original render; the record path (RW object ≠
  +0x1D3) sets +0x118F := 0. So a new / re-created RW object is drawn twice unmodified before the first apply. The
  post-pass (A 0x100A9EB0 → 0x100A8B90) runs only if a texture (+0x1CB) or colour (+0x1CF) was applied and the RW
  object is still +0x1D3; it restores textures / colours and the geometry flags recorded at the record step, then
  clears +0x1CB / +0x1CF. (CRASH-1: applying from the first draw made RenderWare re-instance a prelight-locked
  geometry with PRELIT off and crash at 0x7F5A3A.)
* Device lost (B 0x100C3F60 → object pool loop 0x10012B20 → 0x100A8EE0 per object with materials): every material
  text render target (+0x114B + 4i) is released and its "created" flag cleared; the object pass re-creates it.
* Material **text** slots are substituted at Direct3D level: while such an object renders, samp's device proxy
  `SetTexture` (A 0x10074BB0) replaces the slot's 2×2 texture (+0x110B + 4i; set as the slot texture by the
  kind-2 record, not the model's original texture) with the text render target (+0x114B + 4i).

### 5. Attached objects on players (RPC 113), per frame — Observed
`CWorld::ProcessPedsAfterPreRender` is called through A 0x100A2640 (row 25). After the original, for the local ped and
every streamed remote ped (A 0x10013960): held special-action objects (A 0x100B12B0), then each used slot 0..9 with an
object entity (A 0x100B05C0): if the object entity is invalid it is moved to (0,0,0); otherwise `CEntity::UpdateRpHAnim`
(0x532B20) once per ped, the object is removed from the world, its matrix = LTM of `CPed::m_apBones[bone]` (+0x488; the
SA:MP bone id 1..18 equals GTA's `ePedNode`) × offset, then rotated by rx, ry, rz degrees (X, then Y, then Z; zero
components skipped), scaled, written and re-added.

### 6. Actors — Observed
* `CCivilianPed::ProcessControl` wrapper (row 20, A 0x100A2CA0): when the ped is bullet-, explosion- and melee-proof
  (physical flags +0x40 bits 0x40000, 0x800000, 0x200000 — i.e. an invulnerable actor, SET_CHAR_PROOFS 1,1,1,1,1) the
  move speed x/y is zeroed and the matrix saved before the original (0x5DDBF0); afterwards the saved position is restored
  and move speed x/y zeroed again. Invulnerable actors cannot be pushed.
* All ped classes' Render (rows 11–15, A 0x100A3E70): Direct3D fog is switched off (FOGENABLE 0, FOGTABLEMODE 0,
  FOGVERTEXMODE 3) for the ped draw and FOGENABLE is restored from samp's fog flag (A 0x1026EA54) afterwards.

### 7. RemoveBuildingForPlayer (RPC 43) internals (resolves player-state "internals not analysed") — Observed
* A 0x1009D1E0: (1) every map occluder (`COcclusion::Occluders` 0xC73FA0, 0x12-byte records, count
  `NumOccludersOnMap` 0xC73F98) whose centre is within 500 units (<) of the point gets its centre and size (the six
  i16 at +0..+0xB, helper A 0x1009D0F0) set to 0. The rotation bytes and the 15-bit next-occluder link + far-away bit
  at +0x10 are **not** touched: GTA walks its nearby / far-away lists through that link every frame
  (`COcclusion::ProcessBeforeRendering` 0x7201C0, unbounded `while (link != -1)`); clearing it makes the occluder
  point at occluder 0 and can close a cycle, freezing the main thread (HANG-1); (2) the Building pool (20000), Dummy pool (4000) and Object pool (3000) are
  scanned: every live entity whose model equals the id (or id = −1) and whose position is within the radius (≤) gets
  area code (+0x2F) := 1 and is lowered by 2000 units (placement z and matrix z). Nothing is deleted.
* Later IPL loads: `CFileLoader::LoadObjectInstance` hook (row 65, A 0x100A4AB0) replaces the model id of any instance
  matching a stored removal (same rule) with **19300** before the original runs.
* An unreferenced routine (A 0x1009D3A0) would hide every building and turn all future instances into 19300 (flag
  A 0x1015159C); it is dead code.

### 8. Blip colours (map icons, checkpoints) — Observed
`CRadar::GetRadarTraceColour` is replaced (row 51): 0x3ED (checkpoint) → RGBA 0xAA0000FF; 0x3EC / 0x3EE are vehicle blips
(`vehicles.md` RE-10 additions); any other value < 4000 is a player id whose current colour is used (1000-entry table);
values ≥ 4000 are RGBA. RPC 56 with type 0 therefore shows the server colour as RGBA, except values below 4000.

### 9. SA:MP models (SAMP.img, 18631, 19300) — pointer
The archives are registered by the `default.dat` / `gta.dat` overrides served from samp.saa (IMG lines for
SAMP\SAMP.IMG, SAMP\CUSTOM.IMG, SAMP\SAMPCOL.IMG and samp.ide/custom.ide; `docs/phase3/trace-notes.md`
"samp.saa data overrides", CSV row 128). The DLL itself contains no `SAMP.img` string. M6 therefore depends on SAA-2.

### 10. Two more object hooks — Observed
* Row 75 (A 0x100A4F10): the `CPhysical::PositionAttachedEntity` (0x546FF0) call inside
  `CWorld::ProcessAttachedEntities`. After GTA positions an attached entity, samp looks up its object; for an object
  attached to another object (+0xC0 ≠ 0xFFFF) whose **sync-rotation** byte (+0xDA, default 1, set from CreateObject
  field 11) is 0, it rewrites the rotation part of the matrix from the object's stored rotation (A 0x100A7E60), so the
  child keeps its own orientation instead of following the parent. This is where field 11 of RPC 44 takes effect.
* Row 79 (A 0x100A5430): the `CCustomRoadsignMgr::RenderRoadsignAtomic` (0x6FF350) call in `CEntity::RenderEffects` is
  only made when its atomic argument is non-null (crash guard). **Correction**: the CSV purpose "object material text
  rendering" is wrong for this row; material text goes through the Direct3D proxy (§4).
