# Remote players on foot inside GTA: ped lifecycle, input injection, movement, animations (SA:MP 0.3.7-R4, build A)

Clean-room notes. They describe how samp.dll makes a remote player exist as a GTA ped and how it moves that ped on foot.
Wire layouts and the sync-store rules are not repeated here. See `remote-players-sync.md` (§207 layout, §2.1 store/apply
constants) and `gta-patches.md` plus `data/gta-patches-r4-annotated.csv` (patch list). This file adds the GTA-side
mechanics: which GTA objects and calls are involved, the per-player "context switch" around GTA's own player code, the
order of operations in a frame, and what a Rust DLL must reproduce. Vehicles (driver/passenger, enter/exit) are in
`remote-control-vehicles.md`. Aiming and firing belong to the combat spec. Both are only touched here where they change
on-foot movement or rotation.

A = samp.dll 0.3.7-R4 2021 build (base 0x10000000). GTA = gta_sa.exe 1.0 US (base 0x400000). GTA names come from
`re/gta-reversed` / `re/plugin-sdk` ("named via …"). SCM opcode names come from `gta-reversed/.../eScriptCommands.h`, and
each opcode number was read from the first u16 of the descriptor that samp passes to its script-command helper
A 0x100B2350. Tags: **Observed** = read from disassembly or decompilation. **Inferred** = reasoning beyond that.
**Unknown** = open. Build B was not re-checked here (hook handlers in B are A − 0x30, per `gta-patches.md` §7).

---------------------------------------------------------------------------------------------------

## 0. Mechanism in a nutshell

* A remote player is a real GTA **`CPlayerPed`**. samp creates it with the script command `CREATE_PLAYER` using a
  **player index 2…209** (its "ped slot"). GTA therefore indexes `CWorld::Players[slot]` far past the 2-entry array.
  samp makes that memory safe by relocating the sector grid that follows the array (Inferred, §1.3), and it NOPs the
  GTA code paths that cannot cope with a player that has no `Players[0/1]` entry.
* GTA's own player logic runs every remote ped. samp hooks `CPlayerPed::ProcessControl` (vtable slot), and for a remote
  ped it **swaps in that player's shadow state** before calling the original: `CPad::Pads[0]` (built from the synced keys
  and sticks), `CWorld::PlayerInFocus := slot`, camera mode/vectors/FOV (from aim sync), and the weapon-skill stats. It
  also NOPs `CPed::UpdatePosition` for the duration of the call. Everything is restored afterwards (§2).
* Movement does not come from GTA's animation-driven velocity. samp writes the ped's **move speed** directly (the synced
  velocity, nudged by 0.1 × position error per sync). Large errors are teleported. samp writes the **matrix rotation
  every frame** (slerp toward the synced quaternion, 25 % per frame) and mirrors it into the heading fields (§3).
* GTA tasks running under the swapped pad produce walk/run/sprint/jump/crouch animations. samp adds explicit tasks and
  script commands for special actions and for the synced "animation" field (§4).
* All sync application happens in samp's per-frame entry, which replaces GTA's `Render2dStuff` call late in `Idle`. That
  is after GTA has processed and rendered the world, so values written there take effect in the next frame's
  `CGame::Process` (§6).

---------------------------------------------------------------------------------------------------

## 1. Remote ped lifecycle in GTA

### 1.1 Slot tables (Observed)

| A address | entries | meaning |
|---|---|---|
| 0x101501B8 | 210 × u32 | ped-slot **in use** flag (1/0). Allocation scans slots **2…209**. Slot 0 is the local player (`Players[0]`). Slot 1 (`Players[1]`, GTA's second player) is never used. Max 208 remote peds. Counter A 0x100A0150 counts used slots 2…209 (used by the send-rate formula). |
| 0x1026BD88 | 210 × ptr | slot → GTA `CPed*` (set by A 0x100B4360, which also zeroes a 16-byte per-slot record at 0x1026C0D0). Reverse lookup ped → slot: A 0x100B43A0 (linear scan; returns 0 if not found). |
| 0x1026BA30 | 210 × u32 | slot → "is NPC" flag (A 0x100B4500). |
| 0x10152778 | 210 × 0x134 | per-slot **shadow `CPad`** (same size and layout as GTA `CPad`). Getter A 0x100A7410, setter A 0x100A7380, zeroed at create/destroy by A 0x100A7420. |
| 0x10143E50 | 210 × u8 | per-slot camera mode (from aim sync; setter A 0x1009C8F0). |
| 0x101442A0 | 210 × 48 B | per-slot camera vectors Front/Source/SourceBeforeLookBehind/Up (setter A 0x1009C9F0, filled from aim sync by A 0x10015760). |
| 0x10143F28 / 0x10146A30 | 210 × f32 each | per-slot camera zoom and aspect (setter A 0x1009C860). |
| 0x10146D78 | 210 × 11 × f32 | per-slot weapon-skill stats (setter A 0x1009CA70, from PlayerStreamIn skill levels). |

samp wraps each GTA ped in its own object (constructed by A 0x100B0D20, 0x32D bytes, vtable A 0x100ED140). That object
holds the `CPed*` (+0x2A4, also +0x40 as the generic entity pointer), the SCM ped handle (+0x44) and the slot byte
(+0x2B0). The remote-player object holds the wrapper at +0x1DD (see `remote-players-sync.md` §2.1).

### 1.2 Creation (PlayerStreamIn → spawn A 0x10017130 → A 0x100A0170 → A 0x100B0D20) (Observed)

1. Spawn validates the skin model (A 0x100A0990). If a ped already exists, it is deleted first (§1.5).
2. A 0x100A0170 finds the first free slot in 2…209 and constructs the wrapper. It marks the slot used only if a
   `CPed*` resulted, then records the NPC flag.
3. Wrapper constructor, in order, all through samp's script-command helper (A 0x100B2350):
   * `CREATE_PLAYER` (0x0053) with **player index = slot** and the stream-in position. GTA (named via gta-reversed,
     `PlayerCommands.cpp`) loads the special "player" model if needed and calls `CPlayerPed::SetupPlayerPed(slot)`, which
     does `new CPlayerPed(slot, false)` and stores it in `CWorld::Players[slot].m_pPed`. It then marks the ped as a
     mission char, positions it at the ground-adjusted position, clears space and gives it the default task
     **`CTaskSimplePlayerOnFoot`**. The ped type is `PED_TYPE_PLAYER1`, so `CPlayerPed::GetPadFromPlayer` returns
     `CPad::Pads[0]` for every remote ped.
   * `GET_PLAYER_CHAR` (0x01F5) → ped pool handle. The `CPed*` comes from the pool (A 0x100B3BB0) and is stored in the
     slot → ped table.
   * `SET_CHAR_DROPS_WEAPONS_WHEN_DEAD` (0x087E) (handle, 1); `SET_CHAR_PROOFS` (0x02AB) (handle, 0,0,1,0,0);
     `SET_CHAR_SUFFERS_CRITICAL_HITS` (0x0446) (handle, 0).
   * If the netgame exists: `SET_CHAR_MONEY` (0x03FE) twice, first 0 and then the InitGame "death-drop money" value
     (netgame settings +0x01).
   * Set the skin (A 0x100AFF90, §1.4).
   * Heading: `CPed::m_fCurrentRotation` (+0x558) and `m_fAimingRotation` (+0x55C; plugin-sdk calls them
     `m_fHeadingCurrent`/`m_fHeadingGoal`) := the stream-in angle. The angle goes through degrees→radians A 0x100B59B0:
     0…180° → 0…π, (180°, 360°] → (−π, 0), out of range → 0.
   * Shadow pad zeroed. Position re-written as the matrix translation with z + 0.15 (A 0x1009EB70 get matrix /
     A 0x1009EC20 set matrix).
4. Back in spawn: player colour, marker blip (if marker mode ≠ 0), synced health := 100, pad cleared through the
   key-mapping routine (A 0x100AF380 with zeros), fighting style if ≠ 4, remote state := 33.

### 1.3 Why a player index above 1 does not corrupt memory (Inferred, strong)

* `CPlayerPed::CPlayerPed(id, …)` sets `m_pPlayerData = &CWorld::Players[id].m_PlayerData` and allocates its data.
  `CWorld::Players` (0xB7CD98) has 2 entries of 0x190 bytes. Its end, 0xB7D0B8, is exactly the start of
  `CWorld::ms_aSectors` (120 × 120 × 8 = 115200 bytes, to 0xB992B8).
* At init-main samp moves every listed `ms_aSectors` reference to its own 160000-byte buffer and then **zeroes the
  original array** (`gta-patches.md` §4.3). Indices 2…209 of `Players[]` reach up to 0xB7CD98 + 210 × 0x190 = 0xB915B8 (corrected 2026-11-04 from 0xB91638, confirmed by TRACE-2 guard pages).
  That is inside the freed and zeroed former sector grid. samp never references `Players[2+]` itself (no samp code
  addresses that range). It relies on GTA's own indexing through `CREATE_PLAYER`, `FindPlayerPed()`,
  `FindPlayerInfo()` and `Players[PlayerInFocus]`.
* Consequence for a reimplementation: the sector relocation is **mandatory** for remote players, not just a
  "bigger world" tweak. The former sector memory must be zero before the first `CREATE_PLAYER` with index ≥ 2. The
  `CPlayerInfo` constructors never run for those slots.
* GTA's own per-player loop in `CWorld::Process` (`for i < MAX_PLAYERS(2): Players[i].Process()`, named via
  gta-reversed) never visits slots ≥ 2. So `CPlayerInfo::Process` (wanted level, money, and similar) does not run for
  remote players (Inferred: samp does not patch that loop).

### 1.4 Skin / model change (A 0x100AFF90 → A 0x1009EFB0) (Observed)

Invalid ped models become 0. `CClothes::RebuildPlayer` (0x5A82C0) is made RET (first byte := 0xC3). A ped
intelligence sub-object at +0x2C is deleted (via its vtable, GTA 0x639330) and cleared. The model is loaded if needed:
request, load-all, then poll with `Sleep(1)` up to **200 ms**, after which it prints "Warning: Model %u wouldn't load in
time!" and gives up. Then the entity's `DeleteRwObject` (vtable +0x20) runs, `m_nModelIndex` (+0x22) is written and
`SetModelIndex` (vtable +0x14) is called. Finally `CAEPedSpeechAudioEntity::Initialise(ped)` (0x4E68D0, named via
gta-reversed) re-initialises the voice.

### 1.5 Destruction (stream-out / quit / re-spawn → A 0x100A0270 → wrapper dtor A 0x100B0FE0) (Observed)

Shadow pad zeroed, slot → ped entry cleared. If the ped is still valid (vtable ≠ `CPlaceable` 0x863C40):
* a samp-attached object (+0x2B9) is detached and removed (`DETACH_OBJECT` 0x0682, `REMOVE_OBJECT_ELEGANTLY` 0x09A2);
* the 10 per-player attached-object slots are freed;
* the special-action carry object (+0x2C1) is removed;
* if the ped is in a vehicle it is warped out to (100, 100, 10) (`WARP_CHAR_FROM_CAR_TO_COORD` 0x0362);
* **`m_pPlayerData + 0x4C` := 0**. This is the anim-block reference that the patched `ProcessAnimGroups` no longer
  maintains (patch 0x609C08). Inferred: it prevents a bad `RemoveAnimBlockRef` in the destructor.
* The ped is deleted through its scalar deleting destructor (vtable slot 0). `CPed::~CPed` does `CWorld::Remove`
  itself (named via gta-reversed).

The slot's in-use flag is then cleared. `CWorld::Players[slot].m_pPed` is **not** cleared (Observed: no samp write).
It dangles until the slot is reused (pitfall, §7.3).

### 1.6 GTA patches that exist for remote `CPlayerPed`s (from the annotated CSV; on-foot relevant)

| GTA addr | effect | why (for remote peds) |
|---|---|---|
| sector relocation (§4.3 of gta-patches) | `ms_aSectors` moved, original zeroed | frees memory for `Players[2…209]` (§1.3, Inferred) |
| 0x60D64E | `CPlayerPed` ctor: group set-up condition inverted, so `SetupPlayerPed` (arg false) creates no ped group | `CPedGroups` has only 8 groups (0xC09920). 208 player peds would exhaust them (Inferred) |
| 0x60F2C4 (25 NOP) | removes `FindPlayerSlotWithPedPointer` + `Players[slot].m_pRemoteVehicle` from `CPlayerPed::ProcessControl` | the lookup scans only `Players[0..1]` and returns −1 for a remote ped, which would then index `Players[-1]` |
| 0x5E63A6, 0x621AEA, 0x62D331, 0x741FFF | remove `GetPlayerInfoForThisPlayerPed()->+0x153` uses | that lookup returns NULL for slots ≥ 2 (it scans 0xB7CD98…0xB7D0B8) |
| 0x56E0FA | `FindPlayerVehicle` remote-vehicle branch removed | same reason (vehicles spec) |
| 0x6BC9EB | `CBike::ProcessAI` `Players[0]` driver test removed | remote bikers take the player path (vehicles spec) |
| 0x6884C4, 0x688200 | `fst [esi+0x55C]` (aiming rotation / heading goal) in `CTaskSimplePlayerOnFoot::PlayerControlZelda` / `PlayerControlDucked` NOPed; restored only around the **local** player's ProcessControl | the remote's stick input must not steer its heading (§3.5) |
| 0x5E92F4 | `CPed::ProcessControl` → `CPed::UpdatePosition` call, NOPed only during a remote ped's ProcessControl | samp owns velocity and heading (§3.5) |
| 0x5E1E70 | `CPed::UpdatePosition` ignores `m_standingOnEntity` (always) | samp does surfing itself (§3.7) |
| 0x86D1EC | "playidles" anim block name blanked | no random idle anims on player peds |
| 0x609C08, 0x609A4E + hook | `ProcessAnimGroups`: no anim-block ref juggling; move-anim-group store gated by "CJ walk" | §4.4 |
| 0x47C477 | `POINT_CAMERA_AT_CHAR` always uses ped mode | spectating a remote `CPlayerPed` |
| 0x5E7D62, 0x64DB49/0x64BC9F/0x64B872 | attach and car-entry paths keep weapons / camera | vehicles / combat |
| 0x86D194 → A 0x100A4130 | `CPlayerPed` `ProcessCollision` hook: returns immediately when entity flag bit 31 is set | samp's "no collision" marker (§3.6) |

---------------------------------------------------------------------------------------------------

## 2. Input injection: the per-player context switch

### 2.1 Keys → shadow pad (A 0x100AF380, called every frame from the remote update) (Observed)

The routine writes `CControllerState` words (named via gta-reversed `ControllerState.h`) in the slot's shadow `CPad`.
It writes NewState (+0x00) and OldState (+0x30) together.
Edge rule for every button and for the left stick: if the input is active now **and was active in the previous shadow
NewState**, OldState := NewState (held). If it just became active, OldState := 0, so GTA sees a "just pressed" edge
for one frame. If it is inactive, both are 0. GTA therefore never sees a "just released" edge (Observed).

| sync input | pad field (word index) | value | note |
|---|---|---|---|
| leftRight | LeftStickX (0) | raw i16 | |
| upDown | LeftStickY (1) | raw i16 | |
| ANALOG_LEFT/RIGHT (bits 13/14) | RightStickX (2) | −128 / +128 | |
| ANALOG_UP/DOWN (bits 11/12) | RightStickY (3) | −128 / +128 | |
| bit 0 (1) | LeftShoulder1 (4) | 0xFF | |
| bit 8 (256) | LeftShoulder2 (5) | 0xFF | |
| bit 7 (128) AIM | RightShoulder1 (6) | 0xFF | aim / handbrake |
| bit 6 (64) | RightShoulder2 (7) | 0xFF | |
| bit 5 (32) JUMP | ButtonSquare (0xE) | 0xFF | |
| bit 4 (16) SECONDARY_ATTACK | ButtonTriangle (0xF) | 0xFF | **suppressed** on foot while the ped's primary task slot (intelligence +0x10) holds `CTaskSimpleJetPack` (vtable 0x8705C4) |
| bit 3 (8) SPRINT | ButtonCross (0x10) | 0xFF | |
| bit 2 (4) FIRE | ButtonCircle (0x11) | 0xFF | |
| bit 1 (2) CROUCH | ShockButtonL (0x12) | 0xFF | **only while in a vehicle** (horn). On foot, crouch comes from special action 1 (§4.2) |
| bit 9 (512) | ShockButtonR (0x13) | 0xFF | |
| bit 10 (1024) WALK | m_bPedWalk (0x15) | 0xFF | |

All other `CPad` bytes in the shadow copy stay zero (Mode, DisablePlayerControls, …) (Inferred from the zero-init and
the absence of other writers).

### 2.2 `CPlayerPed::ProcessControl` hook (vtable 0x86D190 → A 0x100A29A0) (Observed)

Entered with the ped in ECX. It looks up the ped's slot (A 0x100B43A0) and reads `CWorld::PlayerInFocus` (0xB7CD74).

**Remote branch** (slot ≠ 0 and `PlayerInFocus` == 0), in order:
1. Save `CPad::Pads[0]` (0xB73458, 0x134 bytes) and the two bytes `TheCamera.Cams[0].m_bLookingLeft/Right`
   (0xB6F1A4/0xB6F1A5) (A 0x100A7300).
2. Copy the slot's shadow pad over `Pads[0]`. Set those two camera bytes from the shadow pad's
   NewState.LeftShoulder2 / RightShoulder2 being non-zero (A 0x100A73A0).
3. Save and replace `TheCamera.Cams[0].m_nMode` (0xB6F1A8, written as a byte) and `TheCamera.m_PlayerWeaponMode.m_nMode`
   (0xB6F858, word) with the slot's camera mode. Weapon mode 4 is written as 0.
4. Save and replace `Cams[0].m_fFOV` (0xB6F250) := zoom × 35 + 35 and `CDraw::ms_fAspectRatio` (0xC3EFA4) :=
   aspect + 1.0.
5. Save and replace `Cams[0]` Front/Source/SourceBeforeLookBehind/Up (0xB6F32C, 48 bytes) with the slot's vectors.
6. Save and replace the 11 weapon-skill floats `CStats::StatTypesFloat[69..79]` (0xB79380 + 0x114…0x13C).
7. `CWorld::PlayerInFocus := slot`. From here GTA's `FindPlayerPed()`, `FindPlayerInfo()`, `CPad::GetPad(0)` and
   the camera all refer to this remote player.
8. Remember `m_fHealth` (+0x540) in a global (A 0x101508A0; no reader found here).
9. NOP the `CPed::UpdatePosition` call (0x5E92F4, 5 bytes).
10. Call GTA `CPlayerPed::ProcessControl` (0x60EA90) inside an SEH frame (A 0x100A2800). If it faults while
    `PlayerInFocus` ≠ 0, samp finds the remote player id for the ped, streams that player out (A 0x10017530) and prints
    "Removed player %u due to error.".
11. Restore the `UpdatePosition` call bytes (`E8 17 88 FF FF`, kept at A 0x10113AB0). Then restore the skills,
    `Cams[0].m_nMode`, the weapon mode, FOV/aspect, `PlayerInFocus := 0`, `Pads[0]` and the two camera bytes, and the
    camera vectors.

**Local branch** (everything else): optional pad overrides for the local player (outside-world-bounds forced jump,
and jump/sprint/crouch blocked by the local cuffed/carry flags). Then the `m_fAimingRotation` stores at 0x6884C4 and
0x688200 are restored, the original runs, and they are NOPed again. Remote peds therefore always run with those stores
NOPed.

### 2.3 Other hooks doing the same switch (Observed)

* `CTaskSimpleUseGun::SetPedPosition` (vtable 0x86D744 → A 0x100A2E20) performs the identical remote swap (steps 1–7)
  around the original 0x624ED0. That task method runs outside `ProcessControl` (from the post-collision task pass), so
  the aiming ped also needs the remote's pad and camera there. The local path additionally forces float 0x858EF0 := 2.0
  (combat spec).
* `CVisibilityPlugins::RenderWeaponPedsForPC` → `CPed::GetWeaponSkill` call (0x7330A2 → A 0x100A2D80) swaps only the
  weapon-skill stats (for dual-wield rendering).
* Weapon switching for a remote (A 0x100AC0A0) also sets `PlayerInFocus := slot` and the slot's skills around
  `SET_CURRENT_CHAR_WEAPON`. Giving a jetpack (§4.2) sets `PlayerInFocus` around `CCheat::JetpackCheat`.

### 2.4 What GTA then does with it (Inferred from GTA names)

`CPlayerPed::ProcessControl` runs the ped's task tree. The default `CTaskSimplePlayerOnFoot` (PlayerControlZelda /
PlayerControlDucked) reads `Pads[0]`: the stick magnitude chooses walk/run, Cross chooses sprint, Square chooses jump,
Circle chooses fire, RightShoulder1 chooses aim, and Triangle chooses enter vehicle. GTA then blends the matching
animations. The heading the stick would produce is discarded (NOPed store), and `UpdatePosition` is skipped. Animations
therefore play, but the ped's displacement and facing come only from samp (§3).

---------------------------------------------------------------------------------------------------

## 3. Movement application (on foot)

Constants and the store rules are in `remote-players-sync.md` §2.1. The GTA-side detail follows.

### 3.1 One-shot apply when a new 207 has been stored (A 0x100166B0, state 17, update type 16) (Observed)

Skipped if the ped is in a vehicle (`m_nPedFlags` bit 8 `bInVehicle`, +0x46C).
1. Target position and target velocity are stored in the remote object (+0x17C / +0x188). **`CPhysical::m_vecMoveSpeed`
   (+0x44) := synced velocity** (A 0x100150D0 → A 0x1009ED70).
2. If not surfing (§3.7): position correction A 0x10014C40:
   * If the ped is not a live entity in the moving list (A 0x1009F430: pointer non-null, vtable ≠ `CPlaceable`,
     `m_pMovingList` +0xB4 non-null) → teleport. **Correction to remote-players-sync §2.1**: the test is "not in the
     moving list", not "has no RW object".
   * Otherwise per-axis |target − current|. If all three are < 1e-5, nothing happens. If Δx ≤ 2, Δy ≤ 2, Δz ≤ 1:
     `moveSpeed_i += (target_i − current_i) × 0.1` (constant A 0x100FE4B0) for each axis with Δ > 1e-5, so this adds to
     the synced velocity written in step 1. Otherwise teleport.
   * **Teleport** = write the matrix translation (`m_matrix` at +0x14, +0x30…0x38) directly via A 0x1009EC20. No
     `CWorld::Remove/Add`, no `Teleport()` virtual, no RW update in that call (Observed). The RW frame and sector
     lists catch up during GTA's own processing (Inferred).
3. Weapon sync, sniper/rocket ped state, and the animation field (§4).

### 3.2 Every frame, on foot (Observed)

Order inside A 0x100166B0 for state 17 when the ped is not in a vehicle:
1. **Rotation** A 0x10014FF0: current matrix → quaternion (A 0x100B52F0). Then `q := D3DXQuaternionSlerp(synced,
   current, 0.75)` (25 % toward the synced value per frame). The result → matrix rotation (A 0x100B6AC0, position kept)
   is written with A 0x1009EC20. Then heading = atan2(−forward.x, forward.y) in degrees, normalised to [0, 360), is
   converted with the degree→radian rule above and written to **both** `m_fCurrentRotation` (+0x558) and
   `m_fAimingRotation` (+0x55C) (A 0x100ABF50).
2. Head-look (§5.2).
3. Special actions (§4.2).
4. Keys and sticks → shadow pad (§2.1).
5. Collision gating (§3.6).
6. If the synced velocity is exactly zero, move speed := 0 (A 0x1009ED70). This stops drift when the remote stands
   still. AFK flag handling as in §2.1 of the sync spec.
7. If more than 3000 ms have passed since the last sync: sticks zeroed, move speed := 0, matrix translation := last
   synced position.
8. Common tail (all states except 32): `m_fHealth` (+0x540) := **1000.0**. Damage is server-authoritative, and the
   displayed bar uses synced health.

The per-frame rotation write and the one-shot velocity write are the only places that touch the ped's physical
state. GTA integrates position from `m_vecMoveSpeed` during its own world processing in the next frame. Because
`UpdatePosition` is skipped for remote peds, the velocity persists between syncs instead of being overwritten from the
animation (Inferred from the 0x5E92F4 toggle and GTA's `UpdatePosition` writing +0x44/+0x48/+0x558/+0x55C, Observed
in gta_sa.exe 0x5E1B10).

### 3.3 Quaternion ↔ GTA matrix convention (Observed; interop formula)

Sync quaternions are stored (w, x, y, z). The receiver builds the GTA axis vectors as follows (A 0x100B4F50). The
right, forward and up vectors are the matrix rows at +0x00, +0x10 and +0x20.

| vector | x | y | z |
|---|---|---|---|
| right | w²+x²−y²−z² | 2(xy − wz) | 2(xz + wy) |
| forward | 2(xy + wz) | w²−x²+y²−z² | 2(yz − wx) |
| up | 2(xz − wy) | 2(yz + wx) | w²−x²−y²+z² |

The sender (A 0x100B4DB0) is the inverse: w = ½√(1+m00+m11+m22). The magnitudes of x, y, z come from the analogous
diagonal sums, with their signs from (up.y − fwd.z), (right.z − up.x) and (fwd.x − right.y). **Consequence:** a ped
with GTA heading h (radians, `forward = (−sin h, cos h, 0)`) has q = (cos(h/2), 0, 0, **−sin(h/2)**). A positive
rotation about +z in the textbook convention is therefore a negative GTA heading. This is very likely the sign
mismatch noted for the headless bot in `CLAUDE.md` (Inferred).

### 3.4 Interpolation summary (constants for a reimplementation)

| item | value | where |
|---|---|---|
| position nudge gain | 0.1 per received packet (not per frame, not time-scaled) | A 0x100FE4B0 |
| teleport if | Δx > 2 or Δy > 2 or Δz > 1 (or ped not in the moving list) | A 0x10014C40 |
| dead zone | 1e-5 per axis | same |
| rotation | slerp 25 % per **frame** toward the synced quaternion (frame-rate dependent) | 0.75 at A 0x10014FF0 |
| stale | > 3000 ms → stop and snap to the last synced position | A 0x100166B0 |

### 3.5 Why GTA does not fight these writes (Observed patches, Inferred reasoning)

* `CPed::UpdatePosition` NOPed during a remote's ProcessControl. Without this, GTA would replace the velocity with the
  animation's root motion and turn the ped toward `m_fAimingRotation`.
* The `m_fAimingRotation` stores in the on-foot player task are NOPed for remotes. The remote's left stick (raw
  leftRight/upDown, relative to the remote's own camera) would otherwise set a heading goal relative to *our* camera.
* samp writes both heading fields each frame, so any GTA code that does run (task heading, aiming) starts from the
  synced facing.

### 3.6 Collision gating (Observed)

samp uses entity flag bit 31 (`CEntity::m_nFlags` +0x1C; GTA's `m_bTunnelTransition`, unused on peds) as its own
"skip collision" marker:
* A 0x1009F6E0(on): on → clear bits 31 and 1 (`m_bCollisionProcessed`); off → set both.
* A 0x1009F690(on): sets/clears bit 0 `m_bUsesCollision`.
* The `CPlayerPed` `ProcessCollision` hook (A 0x100A4130) returns without calling `CPhysical::ProcessCollision`
  (0x54DFB0) when bit 31 is set.

Per frame on foot: not surfing and special action 3 (entering a vehicle) → collision off (both helpers with 0).
Not surfing otherwise → on. Surfing a vehicle (id < 2000) → off. Surfing an object → unchanged.

### 3.7 Surfing (Observed)

* Surfing = state 17 and surfId ∉ {0, 0xFFFF} and surfId < 3000 (A 0x100145F0). While surfing, the one-shot apply sets
  only the velocity (no position correction).
* Position is applied **inside `CWorld::Process`**. The call to `CWorld::ProcessAttachedEntities` (0x5689FD) is
  replaced by A 0x100A4990. That routine calls the original, then the local player's handler (A 0x10003570), then for
  every remote with a ped and state ≠ 0 calls A 0x10014650.
* A 0x10014650: vehicle (surfId 1…1999, must exist in the vehicle pool) or object (surfId − 2000 ≤ 1000, must exist in
  the object pool). Each offset component must be within [−100, 100]. New position = surface matrix × offset (full
  affine transform). It is written with entity `Remove()` (vtable +0xC), matrix write, `CMatrix::UpdateRwMatrix`
  (0x59AD70) + `CEntity::UpdateRwFrame` (0x532B00), then `Add()` (vtable +8). This relinks sectors correctly (unlike
  §3.1 teleport). Rotation is not touched (the per-frame slerp handles it).
* GTA's own "standing on entity" handling in `UpdatePosition` is disabled for every ped (0x5E1E70).

### 3.8 Ground and vertical-line queries (Observed, general)

`CWorld::ProcessVerticalLine` (0x5674E0) is hooked (A 0x100A6000). For every caller except return address 0x59E8F9, if
the "buildings" argument is set, the "objects" argument is forced on. Ground probes therefore also hit samp-created
objects. This matters for peds standing on custom map objects (Inferred).

---------------------------------------------------------------------------------------------------

## 4. Animations and special actions

### 4.1 The sync "animation" field (one-shot, A 0x100166B0 → A 0x100AE810) (Observed)

Applied only when the stored value differs from the last applied one (remote +0x1B4). The store turns negative values
(flag bit 15) into 0.

| bits | meaning |
|---|---|
| 0–15 | anim index into samp's table at A 0x101039C4 (+ index × 36 bytes, "LIBRARY:NAME", valid 1…0x714) |
| 16–23 | frame delta (used as a float, e.g. 4.0) |
| 24 | loop |
| 25 | lockX |
| 26 | lockY |
| 27 | freeze |
| 28–29 | time: 2 → 1, 1 → −1, otherwise 0 |

Play (A 0x100AD270): library "SEX" is refused. If the anim block is not loaded it is requested (`REQUEST_ANIMATION`)
and polled (`HAS_ANIMATION_LOADED`) with `Sleep(1)` up to 15 times, which blocks the game thread. Then
`TASK_PLAY_ANIM_NON_INTERRUPTABLE` (0x0812) is issued with (name, library, delta, loop, lockX, lockY, freeze, time).
For library "PARACHUTE" an extra parachute helper runs (A 0x100AD660, not analysed).

Special cases: anim index 0 after 0x1804067C ("VENDING:VEND_USE", index 1660) → play "VENDING:VEND_EAT_P" once.
Anim 0 while carrying (special action 25) → "CARRY:crry_prtial". Anim 0 otherwise does not stop the previous anim
explicitly (Inferred: it ends on its own or is replaced by the player task).

### 4.2 Special actions (every frame, A 0x10014950) (Observed)

Each action is edge-driven: it starts when the synced value matches and the ped is not yet in that state, and it
stops when the value no longer matches.

| value | start | stop |
|---|---|---|
| 1 duck | if not in a vehicle and not ducking: `CPedIntelligence::SetTaskDuckSecondary(0)` (0x601230) | clear `bIsDucking` (m_nPedFlags bit 26) |
| 2 jetpack | `PlayerInFocus := slot`, `CCheat::JetpackCheat` (0x439600), `PlayerInFocus := 0` | if the primary task slot holds `CTaskSimpleJetPack`: delete it (scalar dtor 0x6801D0) and clear the slot |
| 10 hands up | `TASK_HANDS_UP` (0x05C4, time −2) | re-set the position to itself through the `Teleport` virtual (vtable +0x38), which clears the task (Inferred) |
| 11 phone | `TASK_USE_MOBILE_PHONE` (0x0729, 1) | same opcode with 0 |
| 20 beer / 21 cigar / 22 wine / 23 sprunk | internal carry type 1 / 4 / 2 / 3. `CREATE_OBJECT` (0x0107) with model 1543 / 1485 / 1486 / 1546 at the ped position, then `TASK_PICK_UP_OBJECT` (0x070A) with fixed hand offsets | remove the object (A 0x100ADC40). While an object is held, the drink/smoke loop anims are triggered from the ped's pad (A 0x100ADCE0, Inferred) |
| 24 cuffed | samp flags only (+0x2DE, +0x2F8) | cleared unless the action is 24, 3 or 4 |
| 25 carry | samp flags only (+0x2E2, +0x2F6/7/8) | cleared unless the action is 25, 3 or 4 |
| 68 piss | `CREATE_FX_SYSTEM_ON_CHAR_WITH_DIRECTION` (0x066A, "PETROLCAN", fixed offsets) + `PLAY_FX_SYSTEM` | `KILL_FX_SYSTEM` + position self-set |
| 3 / 4 | vehicle enter/exit (collision gating §3.6, vehicles spec) | |

A final step (A 0x100AD480) removes a samp-attached object (+0x2B9) once the current weapon is no longer 46
(parachute).

**Crouch** on foot is purely special action 1. The CROUCH key bit is not injected on foot (§2.1).
**Cuffed / carry** poses are not tasks. Every frame after `CWorld::ProcessPedsAfterPreRender` (hook 0x53EA03 →
A 0x100A2640 → A 0x10013960 → A 0x100B12B0) samp overwrites bone rotations in the ped's `RpHAnimHierarchy` (via
`GetAnimHierarchyFromSkinClump` 0x734A40) from a fixed table at A 0x101172C0 (bone id + quaternion entries). The same
pass positions per-player attached objects (A 0x100B05C0).

### 4.3 Weapon and ped state (one-shot) (Observed)

If the current weapon differs from the synced one: `SET_CURRENT_CHAR_WEAPON` under the context switch. If it still
differs: give the weapon with 9999 ammo and switch again. If keys contain AIM and the weapon is 34/35/36, then
`m_nPedState` (+0x530) := 12 `PEDSTATE_SNIPER_MODE`. Otherwise a ped in state 12 is set back to 1 `PEDSTATE_IDLE`.

### 4.4 Animation groups, idles, anim-manager hooks (Observed)

* `CPlayerPed::ProcessAnimGroups`: the store of the move-anim group (`[esi+0x4D4]`) is replaced by a samp call
  (A 0x100A3F60) that performs the store **only if InitGame "CJ walk" (netgame settings +0x00) is set**. Otherwise each
  player ped keeps its skin's own walk style. The anim-block release / `playerData+0x4C` store (0x609C08) is NOPed.
* `"playidles"` blanked: player peds never load idle animations.
* `CAnimManager::AddAnimation` / `BlendAnimation` indirect-jmp hooks (A 0x100A4210 / 0x100A42C0) only record calls whose
  target clump is the **local** player's (group/id/blend, ignoring group 0 anim 0xA0) and then run the original. They are
  not needed for remotes. `UncompressAnimation` (A 0x100A3E00) returns early on a NULL argument.
* The ped `Render` vtable redirect (A 0x100A3E70) only toggles D3D fog render states around `CPed::Render`. No movement
  effect.

---------------------------------------------------------------------------------------------------

## 5. Remote camera and aim, as far as on-foot rotation is concerned

### 5.1 Camera shadow state (Observed, applied by the switch in §2.2)

Aim sync (A 0x10015760) stores per slot: camera mode, Front := aim front, Source and SourceBeforeLookBehind := aim
position, Up derived from Front (A 0x100B6940), zoom and aspect. During the remote's ProcessControl and UseGun
SetPedPosition these values replace the local camera. When the remote aims, GTA's aiming code therefore turns the ped
toward the remote's real aim direction. The per-frame slerp (§3.2) pulls the matrix back toward the synced quaternion,
which already contains the sender's aim rotation (Inferred: the two agree in steady state). Aim-sync decoding and fire
logic: combat spec.

### 5.2 Head look (A 0x100143A0, every frame on foot and in vehicles) (Observed)

Skipped for NPCs (+0x10C), when a samp game option (game wrapper +0x59, Inferred "head movement" setting) is off, and
while holding a drink/smoke object. Every 1000 ms the look offset is refreshed as 20 × the slot's camera Front.
Every 200 ms `IKChainManager_c::LookAt` (0x618970, `g_ikChainMan` 0xC15448) is called with ("FollowPedSA", ped, no
entity, 1000 ms, bone −1, ped position + offset, useTorso 0, speed 0.1, blend 500, priority 3). This moves only the
head and neck and has no effect on the ped's facing.

---------------------------------------------------------------------------------------------------

## 6. Per-frame order for a remote on-foot player (one `Idle` 0x53E920 frame) (Observed call sites)

1. `Idle` → `CGame::Process` call at 0x53E981 → samp A 0x100A26A0: frame-rate/timer adjustment (A 0x100A1C50), object
   and other pool ticks, then the original `CGame::Process` (0x53BEE0), then one more pool tick. **No player-pool work
   here.** (Correction: the CSV says this hook toggles 0x6884C4/0x688200/0x5E92F4. It does not. Those toggles are in
   A 0x100A29A0, and the stand-alone toggle helpers A 0x100A2900/2920/2960/2980 have no callers.)
2. Inside `CGame::Process` → `CWorld::Process`:
   a. Each ped's `ProcessControl`. For remote `CPlayerPed`s → A 0x100A29A0 context switch → GTA `CPlayerPed::
      ProcessControl` (tasks read the shadow pad, animations blend, gravity; no `UpdatePosition`).
   b. Collision/movement: `ProcessCollision` (skipped when bit 31 is set) integrates `m_vecMoveSpeed`.
   c. Post-collision task pass: `CTaskSimpleUseGun::SetPedPosition` → A 0x100A2E20 context switch.
   d. `ProcessAttachedEntities` → A 0x100A4990: **surf positions** for local and remote players.
   e. `CCamera::Process` (hook 0x53C104, local camera only).
3. `Idle` → `CWorld::ProcessPedsAfterPreRender` (0x53EA03 → A 0x100A2640): original, then per-player bone overrides
   (cuffs/carry) and attached-object placement.
4. Rendering (ped Render hook, weapon render skill swap).
5. `Idle` → `Render2dStuff` call at 0x53EB12 → samp A 0x100C5030 → A 0x100C4FB0 → netgame A 0x1000B5B0:
   a. receive and dispatch packets (A 0x1000B260). Sync 207 is **stored** only.
   b. A 0x10008B20 → player pool A 0x100137C0 → for every remote **A 0x100166B0** (one-shot apply if new data, then the
      per-frame rotation, head look, special actions, pad, collision, stale/health). Then the local player tick and send
      (A 0x100074C0).
   c. Other pools, chat/UI, then the original `Render2dStuff`.
6. Next frame: step 2 uses the velocity, matrix, pad and flags written in 5b.

Latency consequence (Inferred): a received foot sync affects physics one frame later. Rotation and position writes made
in 5b are drawn in the following frame (after one more physics step).

---------------------------------------------------------------------------------------------------

## 7. Design implications for the Rust DLL

### 7.1 Minimal hook and patch set for remote on-foot players

1. **Per-frame tick after the world update.** Retarget 0x53EB12 (or any late `Idle` point) to run network receive and
   apply. The `CGame::Process` wrapper is not needed for players.
2. **`CPlayerPed` vtable slot +0x28 (0x86D190)** with the full context switch (§2.2): `Pads[0]` + the two camera look
   bytes, `PlayerInFocus`, camera mode/weapon mode, FOV/aspect, camera vectors, weapon-skill stats. Bracket it with the
   0x5E92F4 NOP/restore and fault containment.
3. **`CTaskSimpleUseGun::SetPedPosition` (0x86D744)** with the same switch, needed as soon as remotes aim (shared with
   the combat spec).
4. **`CPlayerPed` `ProcessCollision` slot +0x2C (0x86D194)** gate on a private flag (or use `m_bUsesCollision` alone,
   at the cost of matching samp less exactly).
5. **`CWorld::ProcessAttachedEntities` call at 0x5689FD** for surfing (or any point after collision inside
   `CWorld::Process`).
6. **`CWorld::ProcessPedsAfterPreRender` call at 0x53EA03**: only if cuffed/carry poses or attached objects are wanted.
7. Static patches: the sector relocation (memory for `Players[2…209]`), 0x60D64E, the `Players[-1]`/NULL-player-info
   NOPs (0x60F2C4, 0x5E63A6, 0x621AEA, 0x62D331, 0x741FFF), 0x6884C4/0x688200 (NOP, restored around the local
   ProcessControl), 0x5E1E70, "playidles", 0x609C08 + the 0x609A4E anim-group gate.
8. Optional fidelity: the `ProcessVerticalLine` objects flag, the weapon-skill swap in `RenderWeaponPedsForPC`, and the
   ped Render fog toggle.

Ped creation can reuse GTA's script commands (call the opcode handlers or the underlying functions directly):
`CREATE_PLAYER(slot, pos)` → `GET_PLAYER_CHAR`, then the proofs/money/critical-hit settings, the skin via
`SetModelIndex`, and the heading fields.

### 7.2 Data a reimplementation must carry

The 1812-entry "LIBRARY:NAME" animation table (A 0x101039C4, 36-byte records, interop data, read it from the DLL like
the other tables). The cuffed/carry bone table (A 0x101172C0…0x10117378). Special-action object models and hand offsets
(§4.2). The original bytes for the 0x5E92F4 and 0x6884C4/0x688200 toggles.

### 7.3 Pitfalls

1. **Context-switch completeness.** Any GTA global that the player code reads via `PlayerInFocus` or `Pads[0]` and that
   is not swapped leaks local state into remotes (for example mouse state, `TheCamera.m_nActiveCam` ≠ 0, or
   `CPad::DisablePlayerControls`). samp swaps only what §2.2 lists. Anything GTA writes into `Pads[0]` during the
   remote call is discarded on restore. The swap must be exception-safe. In samp the fault is caught inside the call
   wrapper A 0x100A2800, which then returns normally, so the hook body's restore code still runs (Observed). Rust
   cannot catch SEH natively, so use a vectored handler or a small C/asm shim, and keep the restore outside it.
2. **`Players[]` overflow.** Slots 2…209 live in the former sector memory. It must stay zeroed and unused. The
   `CPlayerInfo` constructors never ran there. `Players[slot].m_pPed` dangles after delete. Never let GTA iterate
   players beyond index 1 (it does not today).
3. **Frame-rate dependence.** The 25 %-per-frame slerp and the 0.1 nudge per packet are not time-scaled. A Rust client
   that runs at another frame rate or tick rate will turn and converge differently. Matching samp exactly means
   applying them at the same points (per frame / per packet).
4. **Main-thread stalls.** samp blocks for up to 200 ms (skin load) and 15 ms (anim block) with `Sleep(1)` polling. A
   reimplementation should queue instead.
5. **Teleport via raw matrix write** does not relink sectors. Surfing uses Remove/Add. Prefer the latter (or
   `CPed::Teleport`) for teleports too, unless exact samp behaviour is required.
6. **Quaternion sign convention** (§3.3): heading h ↔ q = (cos h/2, 0, 0, −sin h/2).
7. Special-action "stop" for hands-up and piss relies on re-setting the position through the `Teleport` virtual, which
   has side effects (task clearing). Copy that deliberately or use an explicit task clear.

---------------------------------------------------------------------------------------------------

## 8. Corrections to other specs

* `remote-players-sync.md` §2.1 step 2: "no model/RW object → teleport" should read "ped not a live entity in GTA's
  moving list (`m_pMovingList` +0xB4) → teleport". The teleport writes only the matrix translation.
* `gta-patches-r4-annotated.csv` row 0x53E981: the Idle → `CGame::Process` hook does not toggle 0x6884C4/0x688200/
  0x5E92F4 (see §6 step 1). The rows for 0x6884C4/0x688200 and 0x5E92F4 should name only A 0x100A29A0 (plus the unreferenced
  helpers).
* CSV row 0x86D194: the `ProcessCollision` handler A 0x100A4130 only gates on entity flag bit 31. The `CVehicle::
  SetupRender` RET write sits in the adjacent routine A 0x100A4150. That routine also calls `RpMatFXMaterialSetEffects
  (material, 0)` (0x811C80) and then continues at GTA 0x4C8848, inside an unnamed material callback at 0x4C8840 in the
  `CVehicleModelInfo` code range. It is a vehicle-material hook body that was attributed to the wrong function because
  of function boundaries (its installer was not identified here). This also answers the "Remaining unknowns" item in
  gta-patches.md.
* gta-patches.md "Remaining unknowns", 0x60D64E: no samp caller passes `true`. `SetupPlayerPed` always passes
  `false`, so after the inversion no `CPlayerPed` creates a ped group (Inferred purpose: 8-group limit).
* `remote-players-sync.md` §0.3: the task vtable 0x8705C4 that masks SECONDARY_ATTACK is `CTaskSimpleJetPack` (named
  via gta-reversed).

---------------------------------------------------------------------------------------------------

## 9. Unknown / not verified

* Whether GTA state half-modified by a faulting `ProcessControl` (tasks, world lists) stays consistent after samp
  streams the player out.
* The meaning of the three `CPhysical` flag bits (0x40000, 0x800000, 0x200000 at +0x40) that the `CCivilianPed`
  ProcessControl hook (A 0x100A2CA0) uses to freeze actors (save the position, run the original, restore it and zero
  XY speed). This affects actors, not players.
* The game-wrapper option at +0x59 gating head-look. The global written at A 0x101508A0 (saved remote health) has no
  reader found.
* The parachute helper A 0x100AD660 and the local-player surf handler A 0x10003570.
* Exact GTA behaviour of `CTaskSimplePlayerOnFoot` under a swapped pad with NOPed heading stores (for example how
  strafing while aiming resolves). It was reasoned from names, not traced.
* The frame-rate/timer adjustment in A 0x100A1C50 (writes `CTimer::ms_fTimeStep` 0xB7CB5C and 0xB7CB58) was not
  analysed. It may affect how far remote peds move per frame.
* Build B not re-verified for the bodies described here.
