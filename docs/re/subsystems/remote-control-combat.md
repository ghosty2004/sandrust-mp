# Combat: aiming, firing, weapons, damage, death (SA:MP 0.3.7-R4 client, build A) — clean-room spec

Scope: how samp.dll (A = 0.3.7-R4 2021) makes GTA render and simulate combat for remote players, and how local combat
events turn into network traffic. Remote on-foot movement and vehicles belong to other files. Wire layouts of 203/204/205/206/207 and of RPCs 53/115/166 are in
`remote-players-sync.md` and `player-state.md`. They are not repeated here, only corrected where needed (§10).

Labels: **Observed** = read from samp.dll A code (objdump / decompilation index) or gta_sa.exe 1.0 US bytes.
**Inferred** = reasoning beyond the instructions. **Unknown** = not resolved. GTA names come from
`re/gta-reversed` or `re/plugin-sdk` ("named via …"). Anything else is described by behaviour only.

## TL;DR

1. **Remote players are real `CPlayerPed`s with their own `CWorld::Players[]` slot (2..209).** samp makes GTA
   believe that each remote ped is "the player" while that ped is processed. It swaps in a per-slot copy of
   `CPad::Pads[0]`, the active camera's mode, the front/source/up vectors, FOV, aspect ratio, `m_PlayerWeaponMode`, the weapon-skill
   stats and `CWorld::PlayerInFocus`. GTA's own player-control and gun tasks then aim and shoot for the remote ped
   exactly as they would for the local player. Aim sync (203) only fills these per-slot tables.
2. **Remote bullets come only from bullet sync (206).** samp stops instant-hit/sniper shots by any ped other than the local
   one inside `CWeapon::Fire`. The remote ped's injected FIRE key still produces the animation, muzzle flash, sound and
   projectiles. When a 206 arrives, samp fires the shot itself through `CWeapon::FireInstantHit` / `FireSniper`
   with that shooter's camera swapped in. A `CWorld::ProcessLineOfSight` hook bends the ray onto the reported entity
   (lag-comp modes) and keeps bullets that reported no hit from touching the local ped.
3. **Damage is decided locally and reported.** A hook at the entry of
   `CPedDamageResponseCalculator::ComputeDamageResponse` sends RPC 115 (damage × 0.33) or 177. It cancels the damage
   for same-team pairs, NPC victims, actors hit by others and surf cases. Otherwise GTA applies it. The local ped's health is
   simulated by the client. Every remote ped's health is reset to 1000 each frame, so GTA never kills one locally.
   Remote deaths come only from RPC 166.

---------------------------------------------------------------------------------------------------

## 1. Foundation: player slots and the per-slot context swap

### 1.1 Remote peds occupy `CWorld::Players[slot]`

- **Observed.** Ped creation (A 0x100a0170) picks the first free game slot in 2..209 (table A 0x101501b8, 210
  entries). The game-side ped object (A 0x100b0d20) runs script opcode **0x0053** (create player, with that slot as
  the player index) and then **0x01F5** (get player char). The slot is stored in the ped object at +0x2b0. Slot 0 is the
  local player. A 0x100b43a0 maps a `CPed*` to its slot through the table A 0x1026bd88 (0 if not found).
- **Observed.** `CPlayerPed::CPlayerPed(playerId, …)` sets `m_pPlayerData = &CWorld::Players[playerId].m_PlayerData`
  and uses ped type `PED_TYPE_PLAYER1` (named via gta-reversed `PlayerPed.cpp`). Every remote ped is therefore
  `IsPlayer()`. This matters for damage: players can survive headshots (`ComputeWillForceDeath`, named via gta-reversed).
- **Observed.** While a remote ped is processed, samp writes the slot into `CWorld::PlayerInFocus` (0xB7CD74, named via
  gta-reversed). `FindPlayerPed()` then resolves to `Players[slot].m_pPed`.
- **Inferred, strongly.** `CWorld::Players` is at 0xB7CD98 with 0x190-byte entries and two entries in GTA.
  `Players[2]` starts at exactly 0xB7D0B8 = `CWorld::ms_aSectors`, which samp relocates into its own buffer
  (`gta-patches.md` §4.3). 210 entries end at 0xB915B8, inside the old sector array (which ends at 0xB992B8). The sector relocation is
  what frees the memory for the extended player array. `gta-patches.md` says remote players have no `Players` slot.
  That is wrong; see §10.

### 1.2 Per-slot tables (A .data, 210 slots each; init A 0x1009c930, A 0x100a72e0)

| table | A address | per slot | default | swapped into (GTA) |
|---|---|---|---|---|
| emulated pad | 0x10152778 | 0x134 bytes (one `CPad`) | zero | `CPad::Pads[0]` 0xB73458 (named via gta-reversed) |
| camera mode | 0x10143e50 | u8 | 4 (follow-ped) | `TheCamera.m_aCams[0].m_nMode` 0xB6F1A8 (u8 write) |
| ″ (same byte, 4→0) | ″ | | | `TheCamera.m_PlayerWeaponMode.m_nMode` 0xB6F858 (u16 write) |
| aim block | 0x101442a0 | 12 floats | zero | `m_aCams[0]` +0x190..+0x1BF = `m_vecFront`, `m_vecSource`, `m_vecSourceBeforeLookBehind`, `m_vecUp` (0xB6F32C, offsets named via plugin-sdk `CCam.h`) |
| zoom | 0x10143f28 | f32 0..1 | 1.0 | `m_aCams[0].m_fFOV` 0xB6F250 := zoom × 35 + 35 |
| aspect | 0x10146a30 | f32 | 0.3333 | `CDraw::ms_fAspectRatio` 0xC3EFA4 := value + 1 (named via gta-reversed) |
| weapon skills | 0x10146d78 | 11 floats | 999.0 | `CStats::StatTypesFloat[69..79]` (0xB79380 + 0x114..0x13C; named via gta-reversed) |

The emulated pad also drives `m_aCams[0].m_bLookingLeft/Right` (0xB6F1A4/0xB6F1A5) from the slot's LeftShoulder2 /
RightShoulder2 (A 0x100a73a0). Restore helpers: A 0x100a7340 (pad), 0x1009c840 (FOV/aspect), 0x1009c9c0 (aim block),
0x1009cc20 (skills). Saved copies of the local values: pad A 0x10152640, aim block A 0x10144270, skills A 0x10146a04.

Writers: aim sync store (§2), RPC 34 SetPlayerSkillLevel for a remote (A 0x100ae490 → 0x1009ca70), key apply
A 0x100af380 (pad, §3.1).

### 1.3 Where the swap is applied (Observed)

| site (A) | GTA function wrapped | swaps |
|---|---|---|
| 0x100a29a0 | `CPlayerPed::ProcessControl` (vtable 0x86D190 → orig 0x60EA90) | everything + PlayerInFocus; also NOPs the `CPed::UpdatePosition` call 0x5E92F4 for the duration |
| 0x100a2e20 | `CTaskSimpleUseGun::SetPedPosition` (vtable 0x86D744 → orig 0x624ED0) | everything + PlayerInFocus |
| 0x100a3140 | vehicle `ProcessControl` (10 classes), remote driver | pad, cam/weapon mode, aim block, PlayerInFocus (no FOV/skills; vehicles scope) |
| 0x100afab0 | samp's "fire one shot for this ped" (§3.3) | cam/weapon mode, FOV/aspect, aim block, skills (no pad, no PlayerInFocus) |
| 0x100afcb0 | aim-target helper (calls `CCamera::Find3rdPersonCamTargetVector` 0x514970, range 300) | same as 0x100afab0 |
| 0x100a2d80 | `CPed::GetWeaponSkill(type)` call in `CVisibilityPlugins::RenderWeaponPedsForPC` (0x7330A2) | skills only |
| 0x100ac0a0 / 0x100af7d0 | set-armed-weapon / give-weapon (§4) | skills + PlayerInFocus |

The swap only happens when the ped's slot is non-zero **and** `PlayerInFocus` is currently 0, so it never nests. The local branch
of the ProcessControl wrapper does not swap. It applies three local pad masks (local-player +0x2f6/+0x2f7/+0x2f8 clear
Square/ShockL/Cross; unknown setter), a world-bounds push, and it restores the two NOPed `m_fAimingRotation` stores
(0x6884C4/0x688200) around the original call. Those stores stay NOPed while remote peds are processed (Observed).
The heading of a remote ped therefore comes from the sync slerp, not from the player-control task.

---------------------------------------------------------------------------------------------------

## 2. Remote aiming (answer to Q1)

**Store (Observed, A 0x10015760, called from the 203 handler A 0x10009c40; needs the remote ped to exist):**
1. camMode → per-slot camera-mode byte.
2. front vector (bytes 1..12) and position (13..24) → per-slot aim block as {front, pos, pos, up}. `up` is computed
   (A 0x100b6940) as the cross product of front with the horizontal right vector derived from atan2(front.x, front.y) − π/2.
3. aimZ (bytes 25..28) → `ped->m_pPlayerData->m_fLookPitch` (CPed +0x480 → +0x54, named via plugin-sdk), only if not NaN
   and within [−100, 100].
4. zoom = (byte29 & 0x3F)/63 and aspect = byte30/255 → per-slot tables.
5. weaponState = byte29 >> 6 applied to the ped's current `CWeapon` (`m_aWeapons[m_nSelectedWepSlot]`, CPed
   +0x5A0 + 0x1C·slot; CWeapon +4 `m_nState`, +8 `m_nAmmoInClip`, named via plugin-sdk): 3 → `m_nState` := 2
   (reloading); 2 → clip raised to ≥ 2; 0/1 → clip := value.

**Render (Inferred from the swap set plus GTA names).** Nothing is written into the ped's aim directly. On the next
`CPlayerPed::ProcessControl` of that ped, the player on-foot task reads the emulated pad (AIM = RightShoulder1, from the
synced keys) and the swapped camera, just as it would for the local player. It therefore enters gun-aim and points the weapon
along the remote's camera front. `CTaskSimpleUseGun::SetPedPosition` runs later (from
`CPedIntelligence::ProcessAfterProcCol`, named via gta-reversed) and needs the same swap, which is why it is wrapped
separately. Supporting facts:
- Per frame (remote-players-sync §2.1, Observed): keys → emulated pad; with AIM held and weapon 34/35/36 the ped state is
  forced to 12, otherwise 12 → 1. A head-look task points the head 20 units along the last front vector (not for NPCs).
- `CTaskSimpleUseGun::ControlGunMove` uses a constant 2.0 instead of `CTimer::ms_fTimeStep` (gta-patches row 0x61E0C2).
  Inferred purpose: the arm reaches the target pose quickly whatever the frame rate.
- Lock-on is disabled (`FindWeaponLockOnTarget` / `FindNextWeaponLockOnTarget` RET, set by InitGame), and `ClearPlayerWeaponMode` is
  NOPed in `ClearWeaponTarget` (0x609CB4). Inferred: one ped clearing the shared camera weapon mode must not affect another.
- If no 203 has arrived yet, the slot keeps zero aim vectors and camera mode 4 (Observed defaults). See pitfall P6.

---------------------------------------------------------------------------------------------------

## 3. Remote firing (answer to Q2)

### 3.1 Key injection
**Observed** (A 0x100af380): synced keys are written into the slot's emulated `CPad`, in both NewState and OldState. Old is set
to 0 when the button was not already down, so GTA sees a fresh press edge, and to 0xFF while it is held. FIRE (bit 2) → Circle,
AIM (bit 7) → RightShoulder1, SECONDARY_ATTACK → Triangle (suppressed while the ped is on foot with
`CTaskSimpleJetPack` as active task; vtable 0x8705C4 named via gta-reversed), CROUCH → ShockL only while in a vehicle
(bInVehicle = CPed+0x46C bit 8). This is the *only* path for melee, throwables, rockets, satchel/detonator, spraycan,
extinguisher, flamethrower and camera. GTA simulates all of them locally for remote peds (Inferred from the absence of
any other hook in those paths; `CWeapon::FireProjectile` uses `TheCamera`'s active-cam front and mode for player peds,
named via gta-reversed, so the camera swap aims them too).

### 3.2 Instant-hit / sniper shots of remote peds are suppressed inside `CWeapon::Fire` (Observed, set by InitGame)
- `FireInstantHit` call sites 0x742495, 0x7424EC, 0x742548 in `CWeapon::Fire` → A 0x100a5820. It **replaces** the
  call and returns 1 ("fired"), because the stub returns the 8th argument and all three sites push 1. If the firing entity is not
  `Players[0].m_pPed` (the local ped), it does nothing. Otherwise it fires the local shot itself (§3.4).
- `FireSniper` call site 0x7424CB → A 0x100a58e0. Same rule: only the local ped fires.
- Result: a remote ped holding FIRE still plays the gun animation and gets `CWeapon::Fire`'s post-fire effects (muzzle
  flash, sound, shell; ammo decrement; named via gta-reversed `Fire`), but no bullet trace, impact or damage. Those come
  from 206.

### 3.3 Applying bullet sync (Observed, A 0x10016370, called directly from the 206 handler — i.e. at packet receive)
1. Needs the shooter's ped. If hitType ≠ 0, every offset component must be within ±300, else the packet is dropped.
2. Target entity: hitType 1 → the local ped if hitId is the local id; ignored if hitId is the shooter; else that remote's
   ped. hitType 2 → that vehicle's game entity. hitType 3 (object) and 0 → no entity. Only the GTA entity pointer is
   kept (+0x28 of the internal 44-byte shot record).
3. Weapon: if the shooter's current weapon ≠ weaponId, set it (A 0x100ac0a0). If it still differs, give it with 9999 ammo
   (A 0x100af7d0) and set it again.
4. Record stored on the shooter's ped object (A 0x100af2c0: +0x2fd record, +0x329 valid). For slot 0 this same function
   also sends 206.
5. Fire (A 0x100afab0): swap the shooter's camera context (§1.3) and publish the record (A 0x10151648) and the shooter
   (A 0x10151644) for the LOS hook. Then:
   - if the current weapon (opcode 0x0470) is 34 → GTA `CWeapon::FireSniper(ped, 0, 0)` (0x73AAC0);
   - else compute origin and muzzle (A 0x100afa00): origin = `CPed::GetBonePosition` (0x5E4280) of bone 24 (inferred
     right hand), then z += weapon `m_vecFireOffset.z` (CWeaponInfo +0x24, named via plugin-sdk) + 0.15; muzzle =
     `GetTransformedBonePosition` (0x5E01C0) of the fire offset. Then GTA `CWeapon::FireInstantHit(ped, &origin,
     &muzzle, null, null, null, false, true)` (0x73FB10) is called on the ped's current weapon, bypassing the hooked call sites.
   - clear the shooter/record globals, restore the camera context.
6. **The 206 origin and hitPos are not consumed on receive** (Observed: no reader found. The LOS hook reads only offset
   + entity). The ray is built by GTA from the swapped camera. **Inferred:** `FireInstantHit` calls
   `CCamera::Find3rdPersonCamTargetVector` (call at 0x7403BE; named via gta-reversed). That function uses the active cam's
   front, up, source, FOV, `CDraw::ms_fAspectRatio` and the 3rd-person crosshair multipliers. This is exactly the set
   samp swaps, and it explains why zoom and aspect are synced at all.

### 3.4 `CWorld::ProcessLineOfSight` hook (Observed; sites 0x740721, 0x740B69 in `FireInstantHit`, 0x736247 in `CBulletInfo::Update`; A 0x100a5640)
Lag-comp selector S (A 0x10118990; InitGame lagcomp 1 → S = 0, 2 → S = 1, else S = 2):
- **Redirect (S ≠ 2, shooter set and ≠ local, record has a live entity — not the `CPlaceable` vtable 0x863C40, named via
  gta-reversed — with a matrix):** P = entity matrix × offset (S = 0) or entity position + offset (S = 1). The ray end
  becomes P + (P − start), so the ray passes through P and continues.
- Call the original `ProcessLineOfSight`.
- **Local-hit suppression (S ≠ 2, remote shooter, record without entity):** if the ray hit the local ped, or the local
  ped's vehicle while the ped is in it, the result entity and hit point are zeroed. A remote "miss" can therefore never hurt
  the local player. With S = 2 (lag comp off) remote shots hit whatever the local simulation hits, including the local ped.
- **Local capture (shooter == local ped):** A 0x100a5450 builds the record (origin = ray start, hitPos = col point,
  offset per S, entity = hit entity) → A 0x100af2c0 → send 206. Before a local instant-hit shot, A 0x10013c90 forces
  `bUsesCollision` (CEntity +0x1C bit 0) on for every streamed, on-foot remote ped. A 0x10013d10 restores it afterwards.
  Inferred: remote peds whose collision samp switched off (vehicle entry, surfing) can still be shot.

Sniper: `CBulletInfo::AddBullet` inside `FireSniper` (0x73AC4B) → A 0x100a5930. It multiplies the velocity by 50 and
calls `CBulletInfo::Update` (0x7360D0) at once, so the sniper bullet resolves while the shooter/record globals are still set
(Observed). The sniper camera shake (`CamShakeNoPos` call 0x73ACE2) → A 0x100a3da0, a bare `ret` (Observed).

Inferred: one shotgun blast calls LOS once per pellet, so it produces several 206 packets. When a record has an entity, all
remote pellets converge on P.

### 3.5 Other weapon hooks (Observed)
- Satchels: in `CProjectileInfo::Update` (0x738F3A → A 0x100a3fc0) a type-39 projectile whose owner is null is removed
  (`RemoveIfThisIsAProjectile` 0x739A40). In `RemoveDetonatorProjectiles` (0x738877 → A 0x100a4000) only satchels owned
  by the detonating ped are blown. Inferred: a remote's detonator (key-injected) sets off only that remote's charges.
- `CWeapon::TakePhotograph` (0x73C252 → A 0x100a3db0): the photo flags are set only for the local photographer.
- Crosshair: `CHud::DrawCrossHairs` call (0x58FBBF → A 0x100a2590). While the local camera mode is 53, the crosshair
  multipliers (0x859520/0x859524) are corrected for the screen aspect, then restored.
- `ComputeGunAimedAtResponse` NOP (0x4C3A5B): peds ignore guns aimed at them.

---------------------------------------------------------------------------------------------------

## 4. Weapons of remote peds (answer to Q3)

- 204 is client→server only. A remote ped's weapon comes from the **weapon byte of 207/200/211** (low 6 bits),
  applied once per new packet, and from the **206 weaponId** (Observed, remote-players-sync §2.1 and §3.3 above). The server
  never sends a remote inventory; RPCs 21/22/67/145 act on the local ped only.
- Set armed weapon (A 0x100ac0a0): PlayerInFocus := slot and skills swapped. On foot: opcode 0x01B9; in a vehicle (or
  forced): GTA `CPed::SetCurrentWeapon(type)` 0x5E6280 (named via gta-reversed). Then PlayerInFocus := 0
  (**not** restored — callers must not be inside a context).
- Give weapon (A 0x100af7d0): weapon → model table A 0x100b3e60 (1→331, 2→333 … 9→341, 10–15→321–326, 16→342,
  17→343, 18→344, 22–29→346–353, 30→355, 31→356, 32→372, 33–45→357–369, 46→371; 0, 19–21 and others → none, so nothing
  is given). If the model is not loaded: opcodes 0x0247 request, 0x038B load-all, then **busy-wait `Sleep(1)` on 0x0248
  until loaded**, on the game thread. Then `CPed::GiveWeapon(type, ammo, true)` 0x5E6080 inside the slot/skill context,
  then set armed. Ammo given to remotes is always 9999. Old weapons stay in the inventory (Inferred: no clear path for
  remotes was found).
- Clip and reload state come from 203 (§2). Weapon skills (dual wield, skill-dependent animations and rendering) come from
  the per-slot skill table, filled by RPC 34 (fallback 999).

---------------------------------------------------------------------------------------------------

## 5. Local firing (for completeness)
In the local ped's own `ProcessControl`, `CWeapon::Fire` → hooked site → A 0x100a5820 → (remote collisions on) →
A 0x100afab0 on the local wrapper (no swap; origin from the hand bone as in §3.3, GTA's own origin arguments are ignored) →
GTA `FireInstantHit`/`FireSniper` → LOS hook captures the hit → 206 sent at once (remote-players-sync §206 for the
layout and the "sniper with no hit is not sent" rule). Projectiles and melee are not sent; other clients simulate them from
keys and aim.

---------------------------------------------------------------------------------------------------

## 6. Damage (answer to Q4)

### 6.1 `ComputeDamageResponse` hook (Observed; entry 0x4B5AC0 → indirect jmp → A 0x100a3d50 → decision A 0x100a3bf0)
Inputs: `this` = `CPedDamageResponseCalculator` (+0 damager, +4 damage, +8 body part, +0xC weapon; named via
gta-reversed), arg0 = victim ped. Decision returns 1 = **cancel** (stub returns `ret 0xC`, so GTA computes no response),
0 = run the original. "Local" = `Players[0].m_pPed`.

| # | condition (in order) | network | local result |
|---|---|---|---|
| 0 | no netgame | — | original |
| 1 | damager ≠ local and victim is a `CCivilianPed` (vtable 0x86C0A8; samp actors) | — | cancel |
| 2 | neither victim nor damager is local | — | original |
| 3 | victim local, the local surf state has a surface (+0x2da: the samp wrapper of a pool vehicle **or** object) and the damager is its GTA entity (wrapper +0x40) | — | cancel |
| 4 | victim local, surface set (as in 3), weapon 54 | — | cancel |
| 5 | victim local | RPC 115 taking=1, attacker id (0xFFFF when not a player) | cancel if same team (6.3), else original |
| 6 | damager local, victim maps to a remote player | RPC 115 taking=0, victim id | cancel if NPC (remote +0x10c ≠ 0) or same team, else original |
| 7 | damager local, victim is an actor | RPC 177 (damage **not** scaled) | cancel |
| 8 | damager local, other victim | — | original |

Entity → player id (A 0x100a3b20): a ped is matched against the remote peds (A 0x100138c0; the local ped never matches). A
vehicle is mapped to the first remote player whose vehicle id (remote +0x1e7) equals it. Damage in RPC 115 is
GTA's damage × 0.33 (A 0x100068a0 / 0x100069b0); weapon and body part are passed through unchanged.

### 6.2 `CEventDamage::AffectsPed` hook (Observed; entry 0x4B35A0 → A 0x100a53e0)
Returns true at once when the tested ped is the local ped **and** `PlayerInFocus` ≠ 0 (a remote context is active);
otherwise the original runs. Inferred: while a remote ped is processed, `FindPlayerPed()` is that remote, so the
original's player-specific checks (`m_bCanBeDamaged`, fire-proof, gang-grenade rule; named via gta-reversed) would test
the wrong ped. Damage events that a remote (for example its melee) puts on the local ped are always accepted.

### 6.3 Same-team rule (Observed)
Active when Settings+0x22 ≠ 0 (set to 1 by the NetGame constructor; player-state.md), local team (local player +0x10c)
≠ 255, the other id is a player, and remote team (+0x109) == local team.

### 6.4 What remote peds take locally
- Every frame, for state ≠ 32 (wasted), samp sets the remote ped's health to 1000.0 (A 0x100166b0 tail, Observed).
  Local damage to a remote ped still runs (hit reactions, knock-downs) but cannot kill it, and players survive critical
  headshots (§1.1). Remote health/armour bars come from sync.
- NPC remote players (+0x10c, set by PlayerJoin isNPC) never receive locally caused damage (case 6), but it is still reported.

### 6.5 Vehicles
`CVehicle::InflictDamage` entry (0x6D7C90 → A 0x100a5370, InitGame): cancelled (returns without the original) only when
the vehicle is the one the local player is **driving** and the damager maps to a same-team player (A 0x100a5280).
Otherwise the original runs (Observed). The resulting vehicle health is reported and overwritten through driver sync
(vehicles scope).

### 6.6 Local health is client-simulated
Nothing restores the local ped's health. GTA applies damage per 6.1, and the result goes out in the 207/200/211 health
byte. The server can override it with RPC 14. Out of scope: the server-side use of RPC 115.

---------------------------------------------------------------------------------------------------

## 7. Death and respawn (answer to Q5)

**Local (Observed, local tick A 0x100074c0):**
- Detection: "dead" (+0xf4) is clear and the ped state is 54 (die), or the ped reports dead (A 0x100abe50). Then special
  actions, jetpack, phone and carry stop, drunk level := 0, the player becomes controllable, a driver sends one
  last 200 (a passenger records the vehicle id), the ped leaves the vehicle (A 0x100ad130), **RPC 53** is sent
  (A 0x10005810), dead := 1 and spawned := 0. When spectating, it requests a spawn instead.
- Killer and reason (A 0x100ac890), from `CPed::m_nLastWeaponDamage` (+0x760) and `m_pLastEntityDamage` (+0x764) (named
  via plugin-sdk):

| last weapon | killer | reason |
|---|---|---|
| < 43, 50, 51 | remote whose ped is the damager; else the remote driver (`CVehicle` +0x460, inferred `m_pDriver`) of the damaging vehicle | weapon (255/0xFFFF when nobody is found) |
| 49 (rammed) | driver of the damaging vehicle | 49 (else 255/0xFFFF) |
| 53 (drowned) | 0xFFFF | 53 |
| 54 (collision/fall) | driver of the damaging vehicle; 0xFFFF if the damager is not a vehicle | 54 (255/0xFFFF when it is a vehicle without a player driver, or there is no damager) |
| other | 0xFFFF | 255 |

- GTA's own wasted sequence runs (the "Wasted" big message NOPed at 0x56E5AD). Once the ped state is no longer 54/55,
  samp calls Spawn (A 0x10003c20) when class info is valid, no forced class selection is pending, the netgame is connected and
  health > 0. Otherwise it enters class selection. F4 sets "return to class selection after next death".
- The client ignores RPC 166 for its own id.

**Remote (Observed, RPC 166 → A 0x10017570):** pad keys cleared, then the ped helper A 0x100b0010: jetpack removed if active,
the ped leaves the vehicle, is teleported to its own position, health := 0, then under PlayerInFocus = slot opcode **0x0321** (kill char) runs.
Remote state := 32, so the 1000-health forcing stops, and the sync caches are reset. GTA's death task plays the fall
animation. A remote comes back through PlayerStreamIn / respawn sync (other file).

---------------------------------------------------------------------------------------------------

## 8. Per-frame order (answer to Q6; GTA order from `Idle` 0x53E920 / `CGame::Process` 0x53BEE0 call sites)

1. `Idle` → `CGame::Process` call (0x53E981) → samp wrapper A 0x100a26a0 (game/pool pre-steps), then the original:
   1. `CPad::UpdatePads` (0x541DD0): local pad only.
   2. `CWeapon::UpdateWeapons` (0x73A360, call 0x53C05C): projectiles, satchel hooks, sniper bullet updates.
   3. `CWorld::Process` (0x5684A0, call 0x53C095): each ped's `ProcessControl`. For remote peds that is the full slot swap
      (§1.3): the player task aims and fires from the emulated pad and camera. `CWeapon::Fire` hooks: the local ped's shots
      go through samp (LOS hook → 206), remote instant-hit shots are dropped. Melee and projectiles are simulated. Damage events → AffectsPed /
      ComputeDamageResponse hooks → RPC 115/177. Then collision; `ProcessAfterProcCol` → `CTaskSimpleUseGun::SetPedPosition` hook
      (second swap). Vehicle `ProcessControl` swaps aim for remote drivers.
   4. `CCamera::Process` (call 0x53C104 → A 0x100a2670): local camera; it overwrites `m_aCams` with local values (the
      remote aim data lives only in samp's tables).
2. `CWorld::ProcessPedsAfterPreRender` (0x53EA03 → A 0x100a2640): attached objects and weapon positions per player.
3. Rendering: `CPed::Render` vtable hooks, `RenderWeaponPedsForPC` skill swap, crosshair hook.
4. `Render2dStuff` call (0x53EB12) retargeted to samp's per-frame entry A 0x100c5030 → netgame A 0x1000b5b0:
   - packets: 203 stores aim; **206 fires its synthetic shot immediately** (§3.3), i.e. outside `CWorld::Process`. Its
     damage events go through the same hooks, with `PlayerInFocus` = 0. 207/200/211 are stored.
   - player pool (A 0x100137c0): each remote A 0x100166b0 (weapon switch/give, ped state 12, keys → emulated pad,
     health := 1000) and the local tick A 0x100074c0 (death → RPC 53; sends 207/203/204/205).

---------------------------------------------------------------------------------------------------

## 9. Design implications for the Rust DLL (answer to Q7)

### 9.1 Minimal hook set for combat parity
Prerequisites (other scopes): `Players[]` extension through the sector relocation and pool sizes, remote `CPlayerPed`
creation (opcode 0x0053 or a direct `CPlayerPed` constructor call with the slot), and the `GetPlayerInfoForThisPlayerPed`
safety NOPs from `gta-patches.md`.

| # | site | why |
|---|---|---|
| H1 | `CPlayerPed` vtable ProcessControl (0x86D190) | per-slot context swap for remote peds; local masks; toggle `m_fAimingRotation` NOPs (0x6884C4/0x688200) and the `UpdatePosition` call (0x5E92F4) |
| H2 | `CTaskSimpleUseGun` vtable SetPedPosition (0x86D744) | same swap for the aiming pose |
| H3 | `FireInstantHit` call sites 0x742495/0x7424EC/0x742548 + `FireSniper` 0x7424CB | drop remote shots; fire local shots with the hand-bone origin |
| H4 | `ProcessLineOfSight` calls 0x740721/0x740B69/0x736247 | 206 capture; lag-comp redirect; local-hit suppression |
| H5 | `AddBullet` call 0x73AC4B | ×50 velocity + immediate `CBulletInfo::Update` |
| H6 | `ComputeDamageResponse` entry 0x4B5AC0 | RPC 115/177 and the cancel table §6.1 |
| H7 | `CEventDamage::AffectsPed` entry 0x4B35A0 | accept damage events on the local ped during a remote context |
| H8 | `CVehicle::InflictDamage` entry 0x6D7C90 | same-team cancel for the local vehicle |
| H9 | `GetWeaponSkill` call 0x7330A2 | per-slot skills while rendering |
| opt | 0x73ACE2 (no shake), 0x738877/0x738F3A (satchels), 0x58FBBF (crosshair), 0x73C252 (photo), lock-on RETs, 0x61E0C2, 0x609CB4, 0x4C3A5B, bullet-bound constants 0x7361B2.. | parity details listed in `gta-patches.md` |

Per frame without hooks: keys → emulated pad; weapon set/give from sync; weaponState → clip/state; remote health := 1000
unless wasted; aim store from 203.

### 9.2 Pitfalls
- **P1 Non-reentrant globals.** Every samp stub keeps its arguments in fixed globals. The ComputeDamageResponse stub also
  overwrites `ebx` before `pusha` and then jumps back into the original (Observed: `mov ebx,[esp+4]` precedes `pusha`).
  Write proper naked trampolines that preserve callee-saved registers. Do not reproduce this.
- **P2 PlayerInFocus discipline.** The set-weapon/give-weapon helpers write 0 instead of restoring. Call them only outside any
  remote context, as samp does from its end-of-frame tick. Any GTA code that calls `FindPlayerPed()` during a swap sees the
  remote ped. Keep each swap and its restore inside one wrapper and guard against nesting (samp: slot ≠ 0 and focus == 0).
- **P3 Blocking model load.** Give-weapon spins `Sleep(1)` until the weapon model is loaded. Prefer requesting the model and
  retrying on a later frame.
- **P4 Health forcing has a one-frame gap.** Remote health is reset only once per frame, at the end. Enough damage inside
  one `CWorld::Process` (more than 1000) could kill a remote ped locally. A more robust option is to clamp in H6 for remote victims, but
  then hit reactions must still run (keep returning "original").
- **P5 Camera index.** samp writes `m_aCams[0]` only, while GTA's aim code reads `m_aCams[m_nActiveCam]`. This is harmless
  while the active cam is 0 (normal play; Inferred). Replicate it, but be careful during camera transitions.
- **P6 Defaults.** A slot with no 203 yet has a zero front vector. Initialise it to the ped's forward vector.
- **P7 Exact field reuse.** The weapon-mode word receives the camera-mode byte (only 4 maps to 0). Keep this so that `Fire`'s
  `m_PlayerWeaponMode == MODE_SNIPER` test (named via gta-reversed) behaves like samp.
- **P8 Scaling.** Scale 115 damage by 0.33; send 177 unscaled.
- **P9 Lag-comp modes.** With lagcomp off (S = 2), remote shots are not redirected and can hit the local ped from local
  simulation alone. Do not "fix" this, because servers depend on the behaviour.
- **P10 Order.** 206 shots are fired from the network tick, after rendering. Firing them inside `CWorld::Process` would change
  which peds count as "local" for the hooks (PlayerInFocus would be non-zero).

---------------------------------------------------------------------------------------------------

## 10. Corrections to other docs
- `player-state.md` RPC 53: weapon 53 (drowning) gives killer 0xFFFF and reason 53. Weapon **49** gives the vehicle driver with reason 49.
  For weapon 54 with a vehicle damager that has no player driver, the result is 255/0xFFFF.
- `player-state.md` RPC 115: the two surf cases also **cancel** the damage locally (not just skip the report). A local
  victim is reported even when the attacker is not a player (id 0xFFFF). Actors can only be damaged by the local player.
- `gta-patches.md` ("Player / ped control"): remote players **do** use `CWorld::Players[2..209]`, through the relocated
  sector memory (§1.1). Only the local-player-only *lookups* are patched.
- `gta-patches.md` / CSV row for 0x858EF0 (0.95 → 2.0 around `CTaskSimpleUseGun::ProcessPed` 0x62A380): that code is in
  A 0x100a2fa0, which has **no reference** (dead code), not in the live hook 0x100a2e20. Also unreferenced: A 0x100a3000
  (wrapper of 0x6857E0, `CTaskSimplePlayerOnFoot::MakeAbortable`), A 0x100a30b0 (0x688810 `…::ProcessPed`) and
  A 0x100a4d80/0x100a4e00/0x100a4e80 (pass-throughs for `Find3rdPersonCamTargetVector`, `Fire` and `FireInstantHit`). All found by an
  immediate-value scan of the whole DLL.
- `remote-players-sync.md` §206 apply: the synthetic shot is **not** fired "from origin toward hitPos". See §3.3 item 6.

## 11. Unknown
- The internal branches of GTA `FireInstantHit` (not reversed in gta-reversed): which player-ped condition selects the
  camera-based ray for a remote ped while `PlayerInFocus` = 0.
- Who sets local-player +0x2f6/+0x2f7/+0x2f8 (pad masks) and why the out-of-bounds branch forces a Square press.
- Whether the vehicle → player mapping (remote +0x1e7) also matches passengers.
- Whether InitGame changes Settings+0x22 (same-team block).
- How many 206 packets a shotgun blast produces at runtime (needs a live test).
- Anti-cheat: none was seen in these combat hooks. The `CCheat::AddToCheatString` hook and the auth read of
  `AddAnimation` bytes exist (gta-patches.md) and were not analysed.

---

## RE-10 additions (static, 2026-11-04)

Static analysis of A for milestone M7. Answers to several §11 unknowns.

### 1. Local pad masks (§1.3 / §11 "who sets +0x2f6/+0x2f7/+0x2f8") — Observed
The three bytes belong to the **local ped object** (Game +8, reached through A 0x10001010), not the local player object.
In the local branch of the ProcessControl wrapper: +0x2F6 clears Square (jump), +0x2F7 clears ShockL (crouch), +0x2F8
clears Cross (sprint), in both NewState and OldState of `Pads[0]`.
* A 0x100ACE90 (carry flag, ped object +0x2E2) sets or clears all three: while carrying, no jump, crouch or sprint.
* A 0x100ACF40 (cuff flag, ped object +0x2DE) sets +0x2F8: cuffed players cannot sprint. While cuffed, every frame
  (A 0x100ACF60): if the object editor is active the sprint mask is cleared; GET_CURRENT_CHAR_WEAPON (0x0470) and, if the
  weapon is not 0, the armed weapon is reset to fists.
* Callers: the local special-action setter A 0x100030F0 (RPC 88, special action 0 clears all) and the remote special-
  action apply A 0x10014950.

### 2. Same-team rule (§11 "whether InitGame changes Settings+0x22") — Observed
Settings +0x22 is written only by the NetGame constructor (A 0x1000B930, := 1). The InitGame handler (A 0x10010660)
reads its fields into other offsets and never touches +0x22. The friendly-fire filter of §6.1/§6.3/§6.5 is therefore
always enabled; it depends only on the team values.

### 3. Vehicle → player mapping (§11 "does remote +0x1e7 match passengers") — Observed
Remote +0x1E7 (vehicle id) is written by both the driver store (A 0x1001737D) and the passenger store (A 0x10017475). The
mapping in §6.1 and in the RPC 53 killer derivation picks the first remote (pool order) whose +0x1E7 equals the vehicle, so
it can resolve to a passenger.

### 4. Weapon skills — Observed
* First frame: GTA's `CCheat::WeaponSkillsCheat` (0x439940) sets all 11 weapon-skill stats to 1000, then
  `CStats::SetStatValue` becomes RET (CSV row 356; the same call also maxes the driving skills). This explains the
  TRACE-4 observation "local pistol skill is 1000 after spawn".
* RPC 40 / session reset (A 0x1000A540 → A 0x1009CCF0): all 11 stat floats := 999.0 and the local saved copy is refreshed.
* RPC 34 (local) overwrites one entry. Remote slots default to 999.0 (§1.2) and are swapped in around remote processing.

### 5. Remote death opcode (§7) — correction
Opcode 0x0321 is `EXPLODE_CHAR_HEAD` (gta-reversed): a damage event with no damager, weapon 34, head, 1000 damage, applied
through `ComputeDamageResponseIfAffectsPed`. It passes samp's damage hook (rule 2: neither side local → original).
Whether the remote `CPlayerPed` loses its head or just dies (players get special head-shot handling in GTA) is not
settled statically; it is a measurement request (M7).

### 6. Drive-by (remote passengers) and 204 targets
See `remote-players-sync.md` RE-10 additions (TASK_DRIVE_BY parameters, task type 0x3FE, deleting-destructor stop; 204
target = `CPlayerPed` +0x79C).

---

## M7 implementation notes (static, samp.dll A, 2026-11-05)

Corrections and details found while implementing M7 (`docs/phase3/m7-notes.md`):
* **InitGame installs the bullet hooks only for lag-comp 1 and 2** (A 0x10010909): lag-comp 1 → S 0 and the installer
  A 0x100A7050 (rows 98–106) plus the lock-on RETs A 0x100A1C10 (rows 4/5); 2 → S 1, the same; anything else → S 2 and
  **neither** installer runs. So with lag-comp off GTA's own `FireInstantHit` / `FireSniper` / LOS run for every ped.
* §3.2: the stubs return their 8th argument (`FireInstantHit` sites) / their 1st argument (`FireSniper` site) through
  `popa`; the local shot ignores GTA's origin arguments (§5).
* §3.3 step 5: the fire offset comes from `CWeaponInfo::GetWeaponInfo(type, 1)` +0x24 (skill 1 = standard).
* §3.4: when the local-hit suppression clears the result, the hook returns 0 (no hit), not the original result.
* §7 and RPC 166: the ped helper A 0x100AD130 (local death) and its use in A 0x100B0010 (remote death) **extinguish the
  ped's fire and its vehicle's fire** (`CFire::Extinguish` 0x5393F0 on CPed +0x730 and CVehicle +0x490); they do not take
  the ped out of the vehicle. The remote helper also removes an active jetpack task (deleting destructor 0x6801D0) and
  teleports through the entity's `Teleport` virtual.
* §7 detection: `(dead == 0 && ped state == 54) || health ≤ 0` (A 0x100ABE50 is "health ≤ 0.0 or no ped"). On death
  zone names go off (A 0x100A1560(0)). Respawn branch: ped state not 54/55 → cuffs off (A 0x100ACF40(0)), then Spawn
  when class info is valid, no forced reselection, connected and health > 0; otherwise class selection.
* Row 45 (`TakePhotograph`): the stub's `[esp+8]` at 0x73C252 is the saved EDI of `CWeapon::Fire` = its owner entity.
* Row 8 (crosshair): the correction (A 0x100A2440) runs only with the flag A 0x10117470, which start-up clears unless
  sa-mp.cfg `nohudscalefix` is non-zero (A 0x100C49DD; the config getter returns the int value, 0 when missing).
