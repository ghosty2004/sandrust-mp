//! Player state, world settings, camera, class selection, spawn and death RPCs.
//!
//! Spec: docs/re/subsystems/player-state.md (section headers are "### <id> <name>").
//! Struct names follow open.mp's NetCode names. Every payload here has a
//! fixed, uncompressed layout; nothing in this module uses compressed strings.
//!
//! Decoders are faithful to the wire. Client-side clamps and limits quoted in
//! the doc comments (e.g. cut type 0/>2 → 2, name length checks) are *not*
//! applied by `read`; callers that emulate the client apply them.

use crate::bitstream::{BitReader, BitWriter};
use crate::rpcs::RpcPayload;
use crate::rpcs::wire::Wire;
use crate::types::{Vec3, read_str8, write_str8};

/// Player ID meaning "no player" (killer/attacker/target fields).
pub const INVALID_PLAYER_ID: u16 = 0xFFFF;

/// Defines a payload struct whose fields are read/written in declaration order.
/// Field types must implement [`Wire`] (`bool` = one bit).
macro_rules! payload {
    ($(#[$m:meta])* $name:ident = $id:literal;) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $name;
        impl RpcPayload for $name {
            const ID: u8 = $id;
            fn read(_r: &mut BitReader) -> Option<Self> {
                Some(Self)
            }
            fn write(&self, _w: &mut BitWriter) {}
        }
    };
    ($(#[$m:meta])* $name:ident = $id:literal { $( $(#[$fm:meta])* $f:ident : $t:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct $name {
            $( $(#[$fm])* pub $f: $t, )*
        }
        impl RpcPayload for $name {
            const ID: u8 = $id;
            fn read(r: &mut BitReader) -> Option<Self> {
                // Struct-expression fields are evaluated in source order.
                Some(Self { $( $f: Wire::get(r)?, )* })
            }
            fn write(&self, w: &mut BitWriter) {
                $( Wire::put(&self.$f, w); )*
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Spawn, class selection, death
// ---------------------------------------------------------------------------

/// Spawn information block (46 bytes), shared by RPC 68 and RPC 128 S→C.
///
/// Spec: player-state.md "68 SetSpawnInfo" (byte offsets 0..46). The client
/// reads it as one raw 46-byte block; field-by-field reads are bit-identical.
/// The client accepts a block only if −20000 < x < 20000, −20000 < y < 20000
/// and −10000 < z < 100000 (not enforced here). open.mp's DL-only
/// `CustomModelID` is never sent to 0.3.7 clients and is not modelled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpawnInfo {
    pub team: u8,
    /// Ped model.
    pub skin: u32,
    /// open.mp `Unknown1`; unused by the client.
    pub unused: u8,
    pub pos: Vec3,
    /// Degrees.
    pub rotation: f32,
    /// Weapon IDs; [`SpawnInfo::NO_WEAPON`] = none. The client gives slot 3,
    /// then 2, then 1 on spawn.
    pub weapons: [u32; 3],
    pub ammo: [u32; 3],
}

impl SpawnInfo {
    /// Size of the block on the wire, in bytes.
    pub const SIZE: usize = 46;
    /// Weapon ID meaning "no weapon in this slot".
    pub const NO_WEAPON: u32 = u32::MAX;

    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            team: Wire::get(r)?,
            skin: Wire::get(r)?,
            unused: Wire::get(r)?,
            pos: Wire::get(r)?,
            rotation: Wire::get(r)?,
            weapons: Wire::get(r)?,
            ammo: Wire::get(r)?,
        })
    }

    pub fn write(&self, w: &mut BitWriter) {
        self.team.put(w);
        self.skin.put(w);
        self.unused.put(w);
        self.pos.put(w);
        self.rotation.put(w);
        self.weapons.put(w);
        self.ammo.put(w);
    }
}

payload! {
    /// RPC 128 PlayerRequestClass, C→S. Reliability RELIABLE.
    ///
    /// Spec: player-state.md "128 PlayerRequestClass (C→S)". Sent on the first
    /// class-selection tick and on each left/right arrow press (wrapping at
    /// 0 / classCount−1). **Width note:** the client writes a u32; open.mp
    /// reads only a u16 (the low half, wire-compatible). We follow the client.
    PlayerRequestClass = 128 {
        /// Index into the server's class list.
        class_index: u32,
    }
}

/// RPC 128 PlayerRequestClassResponse, S→C.
///
/// Spec: player-state.md "128 PlayerRequestClassResponse (S→C)". Layout:
/// `selectable` u8, then the 46-byte [`SpawnInfo`]. If `selectable` ≠ 0 the
/// client stores the spawn info (as RPC 68), applies the class skin and
/// re-enables the class-selection keys; if 0 nothing changes. open.mp adds a
/// `CustomModelID` only for DL clients, so the 0.3.7 body is 47 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerRequestClassResponse {
    /// 0 = the server refused the class.
    pub selectable: u8,
    pub spawn: SpawnInfo,
}

impl RpcPayload for PlayerRequestClassResponse {
    const ID: u8 = 128;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { selectable: r.read_u8()?, spawn: SpawnInfo::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.selectable);
        self.spawn.write(w);
    }
}

/// RPC 68 SetSpawnInfo, S→C.
///
/// Spec: player-state.md "68 SetSpawnInfo (S→C)". Body is exactly one
/// 46-byte [`SpawnInfo`]. The client only stores it (after the position
/// bounds check); nothing spawns until RPC 129 or a respawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetSpawnInfo {
    pub spawn: SpawnInfo,
}

impl RpcPayload for SetSpawnInfo {
    const ID: u8 = 68;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { spawn: SpawnInfo::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        self.spawn.write(w);
    }
}

payload! {
    /// RPC 129 PlayerRequestSpawn, C→S. Reliability RELIABLE. **Empty payload**
    /// (the RPC frame carries a second compressed 0, see `rpc::encode`).
    ///
    /// Spec: player-state.md "129 PlayerRequestSpawn (C→S)". Sent on Shift in
    /// class selection, only once class info was received, no request is
    /// pending and ≥ 2000 ms have passed since class selection began.
    PlayerRequestSpawn = 129;
}

/// RPC 129 PlayerRequestSpawnResponse / ImmediatelySpawnPlayer, S→C.
///
/// Spec: player-state.md "129 PlayerRequestSpawnResponse / ImmediatelySpawnPlayer
/// (S→C)". **Width note:** open.mp (and its `ImmediatelySpawnPlayer`) writes
/// the value as a u32; the client reads only the first 8 bits (the low byte,
/// since values are little-endian) and ignores the rest. `read` therefore
/// consumes exactly 8 bits and leaves any trailing bits unread; `write`
/// emits the u32 that servers send.
///
/// Client behaviour: response 2, or any non-zero response while a spawn
/// request is pending, runs the local spawn routine (which sends RPC 52);
/// otherwise the pending flag is cleared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerRequestSpawnResponse {
    /// 0 = refused, 1 = allowed, 2 = force spawn.
    pub response: u8,
}

impl PlayerRequestSpawnResponse {
    pub const REFUSED: u8 = 0;
    pub const ALLOWED: u8 = 1;
    pub const FORCE: u8 = 2;

    /// open.mp's `ImmediatelySpawnPlayer` (response = 2).
    pub fn immediate() -> Self {
        Self { response: Self::FORCE }
    }
}

impl RpcPayload for PlayerRequestSpawnResponse {
    const ID: u8 = 129;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { response: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.response.into());
    }
}

payload! {
    /// RPC 52 PlayerSpawn, C→S. Reliability RELIABLE_ORDERED. **Empty payload.**
    ///
    /// Spec: player-state.md "Local Spawn routine and 52 PlayerSpawn (C→S)".
    /// Sent at the end of the local spawn routine (after teleporting to the
    /// spawn position + 0.5 z and setting "spawned").
    PlayerSpawn = 52;
}

payload! {
    /// RPC 74 ForcePlayerClassSelection, S→C. **No payload.**
    ///
    /// Spec: player-state.md "74 ForcePlayerClassSelection (S→C)". Only sets a
    /// flag; class selection opens at the next death instead of respawning.
    ForcePlayerClassSelection = 74;
}

payload! {
    /// RPC 53 OnPlayerDeath, C→S. Reliability RELIABLE_ORDERED.
    ///
    /// Spec: player-state.md "53 OnPlayerDeath (C→S)". Sent once when the local
    /// ped dies. When no killer player can be derived the client sends
    /// reason 255 and killer [`INVALID_PLAYER_ID`].
    OnPlayerDeath = 53 {
        /// Weapon / death reason.
        reason: u8,
        /// [`INVALID_PLAYER_ID`] = none.
        killer_id: u16,
    }
}

payload! {
    /// RPC 166 PlayerDeath, S→C.
    ///
    /// Spec: player-state.md "166 PlayerDeath (S→C)". Kills a remote player's
    /// ped; the client ignores the local ID (no remote entry).
    PlayerDeath = 166 {
        player_id: u16,
    }
}

payload! {
    /// RPC 55 SendDeathMessage, S→C.
    ///
    /// Spec: player-state.md "55 SendDeathMessage (S→C)". Adds a kill-list
    /// entry. The client drops the message when `victim_id` is 0xFFFF.
    SendDeathMessage = 55 {
        /// [`INVALID_PLAYER_ID`] = no killer.
        killer_id: u16,
        victim_id: u16,
        /// Weapon / reason icon.
        reason: u8,
    }
}

// ---------------------------------------------------------------------------
// Position, physics, health
// ---------------------------------------------------------------------------

payload! {
    /// RPC 12 SetPlayerPosition, S→C.
    ///
    /// Spec: player-state.md "12 SetPlayerPosition (S→C)". Teleports the local
    /// ped and clears surfing/attachment state (train models use the
    /// mission-train opcode instead).
    SetPlayerPosition = 12 {
        pos: Vec3,
    }
}

payload! {
    /// RPC 13 SetPlayerPositionFindZ, S→C.
    ///
    /// Spec: player-state.md "13 SetPlayerPositionFindZ (S→C)". The client
    /// replaces z with the ground height at (x, y, z) **+ 1.5**, then acts as
    /// RPC 12.
    SetPlayerPositionFindZ = 13 {
        pos: Vec3,
    }
}

payload! {
    /// RPC 19 SetPlayerFacingAngle, S→C.
    ///
    /// Spec: player-state.md "19 SetPlayerFacingAngle (S→C)".
    SetPlayerFacingAngle = 19 {
        /// Degrees.
        angle: f32,
    }
}

payload! {
    /// RPC 90 SetPlayerVelocity, S→C.
    ///
    /// Spec: player-state.md "90 SetPlayerVelocity (S→C)". Written unscaled
    /// into the ped's move-speed vector (GTA move-speed units).
    SetPlayerVelocity = 90 {
        velocity: Vec3,
    }
}

payload! {
    /// RPC 14 SetPlayerHealth, S→C.
    ///
    /// Spec: player-state.md "14 SetPlayerHealth (S→C)". Written into the local
    /// ped's health without clamping.
    SetPlayerHealth = 14 {
        health: f32,
    }
}

payload! {
    /// RPC 66 SetPlayerArmour, S→C.
    ///
    /// Spec: player-state.md "66 SetPlayerArmour (S→C)".
    SetPlayerArmour = 66 {
        armour: f32,
    }
}

payload! {
    /// RPC 15 TogglePlayerControllable, S→C.
    ///
    /// Spec: player-state.md "15 TogglePlayerControllable (S→C)". 0 freezes the
    /// player; any non-zero value unfreezes.
    TogglePlayerControllable = 15 {
        enable: u8,
    }
}

payload! {
    /// RPC 87 ClearPlayerTasks, S→C.
    ///
    /// Spec: player-state.md "87 ClearPlayerTasks (S→C)". Targets the local
    /// ped or a streamed-in remote ped.
    ClearPlayerTasks = 87 {
        player_id: u16,
    }
}

// ---------------------------------------------------------------------------
// Appearance and identity
// ---------------------------------------------------------------------------

payload! {
    /// RPC 153 SetPlayerSkin, S→C (0.3.7 / non-DL layout).
    ///
    /// Spec: player-state.md "153 SetPlayerSkin (S→C)". The player ID is a full
    /// **u32** in the 0.3.7 layout (open.mp's u16+u32+u32 form is DL-only).
    /// Invalid ped models are rejected by the client with a log warning.
    SetPlayerSkin = 153 {
        player_id: u32,
        /// Ped model ID.
        skin: u32,
    }
}

payload! {
    /// RPC 69 SetPlayerTeam, S→C.
    ///
    /// Spec: player-state.md "69 SetPlayerTeam (S→C)". Team 255 = no team; the
    /// client cancels same-team damage locally.
    SetPlayerTeam = 69 {
        player_id: u16,
        team: u8,
    }
}

payload! {
    /// RPC 72 SetPlayerColor, S→C.
    ///
    /// Spec: player-state.md "72 SetPlayerColor (S→C)". Colour is RGBA as
    /// written by open.mp.
    SetPlayerColor = 72 {
        player_id: u16,
        colour: u32,
    }
}

payload! {
    /// RPC 34 SetPlayerSkillLevel, S→C.
    ///
    /// Spec: player-state.md "34 SetPlayerSkillLevel (S→C)". The client ignores
    /// skill types ≥ 11 and stores the level as a float.
    SetPlayerSkillLevel = 34 {
        player_id: u16,
        /// GTA weapon-skill index (0..11).
        skill_type: u32,
        level: u16,
    }
}

payload! {
    /// RPC 89 SetPlayerFightingStyle, S→C.
    ///
    /// Spec: player-state.md "89 SetPlayerFightingStyle (S→C)".
    SetPlayerFightingStyle = 89 {
        player_id: u16,
        style: u8,
    }
}

/// RPC 33 SetPlayerShopName, S→C.
///
/// Spec: player-state.md "33 SetPlayerShopName (S→C)". A fixed 32-byte,
/// NUL-padded name (raw, not length-prefixed). The client treats an empty
/// name as "clear" and uses at most 31 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetPlayerShopName {
    pub name: [u8; 32],
}

impl SetPlayerShopName {
    /// Build from a name; truncated to 31 bytes so a NUL always follows.
    pub fn from_name(name: &[u8]) -> Self {
        let mut buf = [0u8; 32];
        let n = name.len().min(31);
        buf[..n].copy_from_slice(&name[..n]);
        Self { name: buf }
    }

    /// The name up to (not including) the first NUL.
    pub fn name_bytes(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(self.name.len());
        &self.name[..end]
    }
}

impl RpcPayload for SetPlayerShopName {
    const ID: u8 = 33;
    fn read(r: &mut BitReader) -> Option<Self> {
        let mut name = [0u8; 32];
        r.read_bytes_into(&mut name)?;
        Some(Self { name })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_bytes(&self.name);
    }
}

payload! {
    /// RPC 35 SetPlayerDrunkLevel, S→C.
    ///
    /// Spec: player-state.md "35 SetPlayerDrunkLevel (S→C)". Above 2000 the
    /// client applies drunk effects; the level then decays by 1 per tick.
    SetPlayerDrunkLevel = 35 {
        level: i32,
    }
}

payload! {
    /// RPC 88 SetPlayerSpecialAction, S→C.
    ///
    /// Spec: player-state.md "88 SetPlayerSpecialAction (S→C)". Values match the
    /// public SPECIAL_ACTION_* constants; the client ignores unknown values.
    SetPlayerSpecialAction = 88 {
        action: u8,
    }
}

/// RPC 86 ApplyPlayerAnimation, S→C.
///
/// Spec: player-state.md "86 ApplyPlayerAnimation (S→C)". Layout: u16 player,
/// u8-prefixed library, u8-prefixed name, f32 delta, four single bits (loop,
/// lockX, lockY, freeze), u32 time. The client ignores the library "SEX"
/// (case-insensitive) and gives up if the animation block does not load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyPlayerAnimation {
    pub player_id: u16,
    pub lib: Vec<u8>,
    pub name: Vec<u8>,
    pub delta: f32,
    pub looping: bool,
    pub lock_x: bool,
    pub lock_y: bool,
    pub freeze: bool,
    /// Milliseconds.
    pub time: u32,
}

impl RpcPayload for ApplyPlayerAnimation {
    const ID: u8 = 86;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            player_id: r.read_u16()?,
            lib: read_str8(r)?,
            name: read_str8(r)?,
            delta: r.read_f32()?,
            looping: r.read_bit()?,
            lock_x: r.read_bit()?,
            lock_y: r.read_bit()?,
            freeze: r.read_bit()?,
            time: r.read_u32()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        write_str8(w, &self.lib);
        write_str8(w, &self.name);
        w.write_f32(self.delta);
        w.write_bit(self.looping);
        w.write_bit(self.lock_x);
        w.write_bit(self.lock_y);
        w.write_bit(self.freeze);
        w.write_u32(self.time);
    }
}

// ---------------------------------------------------------------------------
// Weapons and money
// ---------------------------------------------------------------------------

payload! {
    /// RPC 22 GivePlayerWeapon, S→C.
    ///
    /// Spec: player-state.md "22 GivePlayerWeapon (S→C)". The client loads the
    /// weapon model if needed, gives the weapon and makes it current.
    GivePlayerWeapon = 22 {
        weapon: u32,
        ammo: u32,
    }
}

payload! {
    /// RPC 21 ResetPlayerWeapons, S→C. **No payload.**
    ///
    /// Spec: player-state.md "21 ResetPlayerWeapons (S→C)".
    ResetPlayerWeapons = 21;
}

payload! {
    /// RPC 67 SetPlayerArmedWeapon, S→C.
    ///
    /// Spec: player-state.md "67 SetPlayerArmedWeapon (S→C)". The client
    /// ignores values outside 0..=46.
    SetPlayerArmedWeapon = 67 {
        weapon: u32,
    }
}

payload! {
    /// RPC 145 SetPlayerAmmo, S→C.
    ///
    /// Spec: player-state.md "145 SetPlayerAmmo (S→C)". Ignored by the client
    /// if the weapon is not carried.
    SetPlayerAmmo = 145 {
        weapon: u8,
        ammo: u16,
    }
}

payload! {
    /// RPC 18 GivePlayerMoney, S→C.
    ///
    /// Spec: player-state.md "18 GivePlayerMoney (S→C)". Signed delta.
    GivePlayerMoney = 18 {
        amount: i32,
    }
}

payload! {
    /// RPC 20 ResetPlayerMoney, S→C. **No payload.**
    ///
    /// Spec: player-state.md "20 ResetPlayerMoney (S→C)".
    ResetPlayerMoney = 20;
}

// ---------------------------------------------------------------------------
// World state
// ---------------------------------------------------------------------------

payload! {
    /// RPC 29 SetPlayerTime, S→C.
    ///
    /// Spec: player-state.md "29 SetPlayerTime (S→C)". Sets the GTA clock.
    SetPlayerTime = 29 {
        hour: u8,
        minute: u8,
    }
}

payload! {
    /// RPC 94 SetPlayerWorldTime, S→C.
    ///
    /// Spec: player-state.md "94 SetPlayerWorldTime (S→C)". The client only
    /// stores the hour in its server settings.
    SetPlayerWorldTime = 94 {
        hour: u8,
    }
}

payload! {
    /// RPC 30 TogglePlayerClock, S→C.
    ///
    /// Spec: player-state.md "30 TogglePlayerClock (S→C)". Non-zero enables the
    /// HUD clock and running time; 0 freezes the time at its current value.
    TogglePlayerClock = 30 {
        enable: u8,
    }
}

payload! {
    /// RPC 60 SendGameTimeUpdate, S→C.
    ///
    /// Spec: player-state.md "60 SendGameTimeUpdate (S→C)". Used by the client
    /// only while the HUD clock is disabled (`time & 0x3FFFFFFF`).
    SendGameTimeUpdate = 60 {
        time: i32,
    }
}

payload! {
    /// RPC 152 SetPlayerWeather, S→C.
    ///
    /// Spec: player-state.md "152 SetPlayerWeather (S→C)". Immediate when the
    /// clock is disabled, blended otherwise.
    SetPlayerWeather = 152 {
        weather: u8,
    }
}

payload! {
    /// RPC 146 SetPlayerGravity, S→C.
    ///
    /// Spec: player-state.md "146 SetPlayerGravity (S→C)".
    SetPlayerGravity = 146 {
        gravity: f32,
    }
}

payload! {
    /// RPC 17 SetWorldBounds, S→C.
    ///
    /// Spec: player-state.md "17 SetWorldBounds (S→C)". Field order is
    /// maxX, minX, maxY, minY (open.mp Vector4 order). ±20000 = no bounds.
    SetWorldBounds = 17 {
        max_x: f32,
        min_x: f32,
        max_y: f32,
        min_y: f32,
    }
}

payload! {
    /// RPC 133 SetPlayerWantedLevel, S→C.
    ///
    /// Spec: player-state.md "133 SetPlayerWantedLevel (S→C)". Display-only in
    /// the client (HUD patch).
    SetPlayerWantedLevel = 133 {
        level: u8,
    }
}

payload! {
    /// RPC 48 SetPlayerVirtualWorld, S→C.
    ///
    /// Spec: player-state.md "48 SetPlayerVirtualWorld (S→C)".
    SetPlayerVirtualWorld = 48 {
        world_id: i32,
    }
}

payload! {
    /// RPC 104 EnableStuntBonusForPlayer, S→C. One bit.
    ///
    /// Spec: player-state.md "104 EnableStuntBonusForPlayer (S→C)".
    EnableStuntBonusForPlayer = 104 {
        enable: bool,
    }
}

payload! {
    /// RPC 111 ToggleWidescreen, S→C.
    ///
    /// Spec: player-state.md "111 ToggleWidescreen (S→C)".
    ToggleWidescreen = 111 {
        enable: u8,
    }
}

payload! {
    /// RPC 16 PlayerPlaySound, S→C.
    ///
    /// Spec: player-state.md "16 PlayerPlaySound (S→C)". 0 stops, 1 sets a flag,
    /// 2..999 frontend tracks, 1000..1999 one-off positional sounds, ≥ 2000
    /// mission audio at the position.
    PlayerPlaySound = 16 {
        sound_id: u32,
        pos: Vec3,
    }
}

payload! {
    /// RPC 79 CreateExplosion, S→C.
    ///
    /// Spec: player-state.md "79 CreateExplosion (S→C)".
    CreateExplosion = 79 {
        pos: Vec3,
        kind: u32,
        radius: f32,
    }
}

payload! {
    /// RPC 43 RemoveBuildingForPlayer, S→C.
    ///
    /// Spec: player-state.md "43 RemoveBuildingForPlayer (S→C)". Model −1
    /// (0xFFFFFFFF) = all models. The original client keeps up to 1000
    /// removals without a bounds check; a reimplementation should cap at 1000.
    RemoveBuildingForPlayer = 43 {
        model_id: u32,
        pos: Vec3,
        radius: f32,
    }
}

payload! {
    /// RPC 112 PlayCrimeReport, S→C.
    ///
    /// Spec: player-state.md "112 PlayCrimeReport (S→C)". `suspect_id` is read
    /// but unused by the client; the vehicle fields are used only when
    /// `in_vehicle` ≠ 0 and the model is 400..=611.
    PlayCrimeReport = 112 {
        suspect_id: u16,
        in_vehicle: u32,
        vehicle_model: u32,
        vehicle_colour: u32,
        crime_id: u32,
        pos: Vec3,
    }
}

payload! {
    /// RPC 167 DisableRemoteVehicleCollisions, S→C. One bit.
    ///
    /// Spec: player-state.md "167 DisableRemoteVehicleCollisions (S→C)".
    DisableRemoteVehicleCollisions = 167 {
        disable: bool,
    }
}

// ---------------------------------------------------------------------------
// Interior
// ---------------------------------------------------------------------------

payload! {
    /// RPC 156 SetPlayerInterior, S→C.
    ///
    /// Spec: player-state.md "156 SetPlayerInterior (S→C)". The resulting
    /// interior change is reported back with RPC 118.
    SetPlayerInterior = 156 {
        interior: u8,
    }
}

payload! {
    /// RPC 118 OnPlayerInteriorChange, C→S. Reliability RELIABLE.
    ///
    /// Spec: player-state.md "118 OnPlayerInteriorChange (C→S)". Sent whenever
    /// GTA's current area differs from the last reported one, including
    /// changes caused by RPC 156.
    OnPlayerInteriorChange = 118 {
        interior: u8,
    }
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

payload! {
    /// RPC 157 SetPlayerCameraPosition, S→C.
    ///
    /// Spec: player-state.md "157 SetPlayerCameraPosition (S→C)".
    SetPlayerCameraPosition = 157 {
        pos: Vec3,
    }
}

payload! {
    /// RPC 158 SetPlayerCameraLookAt, S→C.
    ///
    /// Spec: player-state.md "158 SetPlayerCameraLookAt (S→C)". The client
    /// replaces a cut of 0 or > 2 with 2 (not applied here).
    SetPlayerCameraLookAt = 158 {
        pos: Vec3,
        cut: u8,
    }
}

payload! {
    /// RPC 162 SetPlayerCameraBehindPlayer, S→C. **No payload.**
    ///
    /// Spec: player-state.md "162 SetPlayerCameraBehindPlayer (S→C)".
    SetPlayerCameraBehindPlayer = 162;
}

payload! {
    /// RPC 82 InterpolateCamera, S→C.
    ///
    /// Spec: player-state.md "82 InterpolateCamera (S→C)". Starts with a single
    /// bit, so every later field is unaligned. The client ignores the RPC when
    /// `time` ≤ 0 and replaces a cut of 0 or > 2 with 2 (not applied here).
    /// open.mp writes `time` as u32; the client reads it signed (same bits).
    InterpolateCamera = 82 {
        /// true = interpolate the camera position, false = the look-at point.
        is_position: bool,
        from: Vec3,
        to: Vec3,
        /// Milliseconds.
        time: i32,
        cut: u8,
    }
}

payload! {
    /// RPC 81 AttachCameraToObject, S→C.
    ///
    /// Spec: player-state.md "81 AttachCameraToObject (S→C)". Only object IDs
    /// < 1001 that exist are honoured by the client.
    AttachCameraToObject = 81 {
        object_id: u16,
    }
}

payload! {
    /// RPC 170 SetPlayerCameraTargeting, S→C. One bit.
    ///
    /// Spec: player-state.md "170 SetPlayerCameraTargeting (S→C)". Enables or
    /// disables the client's RPC 168 (OnPlayerCameraTarget) reports.
    SetPlayerCameraTargeting = 170 {
        enabled: bool,
    }
}

// ---------------------------------------------------------------------------
// Damage
// ---------------------------------------------------------------------------

payload! {
    /// RPC 115 OnPlayerGiveTakeDamage, C→S. Reliability RELIABLE_ORDERED.
    ///
    /// Spec: player-state.md "115 OnPlayerGiveTakeDamage (C→S)". Starts with a
    /// single bit, so later fields are unaligned. The client sends
    /// `damage` = GTA damage value × 0.33 (scaling is the caller's job, not
    /// this encoder's).
    OnPlayerGiveTakeDamage = 115 {
        /// true = the local player was damaged; false = it damaged `player_id`.
        taking: bool,
        /// The other player; [`INVALID_PLAYER_ID`] if the attacker is not a player.
        player_id: u16,
        damage: f32,
        weapon_id: u32,
        bodypart: u32,
    }
}

// ---------------------------------------------------------------------------
// Incoming dispatch
// ---------------------------------------------------------------------------

macro_rules! incoming {
    ($($name:ident),* $(,)?) => {
        /// S→C payloads of this module.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Incoming {
            $( $name($name), )*
        }

        /// RPC ids decoded by [`decode`].
        pub const INCOMING_IDS: &[u8] = &[$( $name::ID, )*];

        /// Decode an S→C payload of this module; `None` for other ids or
        /// truncated data.
        pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
            match id {
                $( <$name as RpcPayload>::ID => $name::read(r).map(Incoming::$name), )*
                _ => None,
            }
        }

        impl Incoming {
            /// RPC id of the payload.
            pub fn id(&self) -> u8 {
                match self {
                    $( Incoming::$name(_) => $name::ID, )*
                }
            }

            /// Encode the payload body.
            pub fn write(&self, w: &mut BitWriter) {
                match self {
                    $( Incoming::$name(p) => p.write(w), )*
                }
            }
        }
    };
}

incoming! {
    SetPlayerPosition,
    SetPlayerPositionFindZ,
    SetPlayerHealth,
    TogglePlayerControllable,
    PlayerPlaySound,
    SetWorldBounds,
    GivePlayerMoney,
    SetPlayerFacingAngle,
    ResetPlayerMoney,
    ResetPlayerWeapons,
    GivePlayerWeapon,
    SetPlayerTime,
    TogglePlayerClock,
    SetPlayerShopName,
    SetPlayerSkillLevel,
    SetPlayerDrunkLevel,
    RemoveBuildingForPlayer,
    SetPlayerVirtualWorld,
    SendDeathMessage,
    SendGameTimeUpdate,
    SetPlayerArmour,
    SetPlayerArmedWeapon,
    SetSpawnInfo,
    SetPlayerTeam,
    SetPlayerColor,
    ForcePlayerClassSelection,
    CreateExplosion,
    AttachCameraToObject,
    InterpolateCamera,
    ApplyPlayerAnimation,
    ClearPlayerTasks,
    SetPlayerSpecialAction,
    SetPlayerFightingStyle,
    SetPlayerVelocity,
    SetPlayerWorldTime,
    EnableStuntBonusForPlayer,
    ToggleWidescreen,
    PlayCrimeReport,
    PlayerRequestClassResponse,
    PlayerRequestSpawnResponse,
    SetPlayerWantedLevel,
    SetPlayerAmmo,
    SetPlayerGravity,
    SetPlayerWeather,
    SetPlayerSkin,
    SetPlayerInterior,
    SetPlayerCameraPosition,
    SetPlayerCameraLookAt,
    SetPlayerCameraBehindPlayer,
    PlayerDeath,
    DisableRemoteVehicleCollisions,
    SetPlayerCameraTargeting,
}

// ---------------------------------------------------------------------------
// Outgoing (C→S) send parameters
// ---------------------------------------------------------------------------

outgoing! {
    Player;
    /// RPC 128: reliability 8 (RELIABLE), priority 1, channel 0
    /// (player-state.md "128 PlayerRequestClass (C→S)"). Mirrors samp.dll
    /// 0x100058d0.
    PlayerRequestClass => Reliable;
    /// RPC 129: reliability 8 (RELIABLE), priority 1, channel 0
    /// (player-state.md "129 PlayerRequestSpawn (C→S)"). Mirrors samp.dll
    /// 0x10003b60.
    PlayerRequestSpawn => Reliable;
    /// RPC 52: sent at the end of the local spawn routine, reliability 9
    /// (RELIABLE_ORDERED), priority 1, channel 0 (player-state.md "Local Spawn
    /// routine and 52 PlayerSpawn (C→S)"). Mirrors samp.dll 0x10003c20.
    PlayerSpawn => ReliableOrdered;
    /// RPC 53: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (player-state.md "53 OnPlayerDeath (C→S)"). Mirrors samp.dll 0x10005810.
    OnPlayerDeath => ReliableOrdered;
    /// RPC 118: reliability 8 (RELIABLE), priority 1, channel 0
    /// (player-state.md "118 OnPlayerInteriorChange (C→S)"). Mirrors samp.dll
    /// 0x10005970.
    OnPlayerInteriorChange => Reliable;
    /// RPC 115: sent from two sites (damage given, damage taken), both
    /// reliability 9 (RELIABLE_ORDERED), priority 1, channel 0 (player-state.md
    /// "115 OnPlayerGiveTakeDamage (C→S)"; both sites in
    /// docs/re/addresses/samp-proto-rpcs.md).
    OnPlayerGiveTakeDamage => ReliableOrdered;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;

    // f32 little-endian byte patterns used in hand-built vectors.
    const F_1: [u8; 4] = [0x00, 0x00, 0x80, 0x3F]; // 1.0
    const F_2: [u8; 4] = [0x00, 0x00, 0x00, 0x40]; // 2.0
    const F_M1: [u8; 4] = [0x00, 0x00, 0x80, 0xBF]; // -1.0
    const F_HALF: [u8; 4] = [0x00, 0x00, 0x00, 0x3F]; // 0.5
    const F_4: [u8; 4] = [0x00, 0x00, 0x80, 0x40]; // 4.0
    const F_100: [u8; 4] = [0x00, 0x00, 0xC8, 0x42]; // 100.0
    const F_20K: [u8; 4] = [0x00, 0x40, 0x9C, 0x46]; // 20000.0
    const F_M20K: [u8; 4] = [0x00, 0x40, 0x9C, 0xC6]; // -20000.0

    fn v3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// Bits of `bytes`, MSB-first. Independent of `BitWriter`.
    fn bits_of(bytes: &[u8]) -> Vec<bool> {
        bytes.iter().flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1 == 1)).collect()
    }

    /// Hand-pack a bit sequence into bytes (last byte zero-padded).
    fn pack(seq: &[bool]) -> (Vec<u8>, usize) {
        let mut out = vec![0u8; seq.len().div_ceil(8)];
        for (i, &bit) in seq.iter().enumerate() {
            if bit {
                out[i / 8] |= 0x80 >> (i % 8);
            }
        }
        (out, seq.len())
    }

    fn encode<T: RpcPayload>(p: &T) -> (Vec<u8>, usize) {
        let mut w = BitWriter::new();
        p.write(&mut w);
        let n = w.bit_len();
        (w.into_bytes(), n)
    }

    /// Writer output equals `bytes`/`bits`, and reading those bytes gives `p`
    /// back with nothing left over.
    fn assert_wire<T: RpcPayload + PartialEq + Debug>(p: &T, bytes: &[u8], bits: usize) {
        assert_eq!(encode(p), (bytes.to_vec(), bits), "writer output");
        let mut r = BitReader::with_bit_len(bytes, bits);
        assert_eq!(T::read(&mut r).as_ref(), Some(p), "reader output");
        assert_eq!(r.remaining(), 0, "unread bits");
    }

    fn assert_bytes<T: RpcPayload + PartialEq + Debug>(p: &T, bytes: &[u8]) {
        assert_wire(p, bytes, bytes.len() * 8);
    }

    /// Round trip through write/read, misaligned by one leading bit; a read
    /// truncated by one bit fails.
    fn roundtrip<T: RpcPayload + PartialEq + Debug>(p: T) {
        let mut w = BitWriter::new();
        w.write_bit(true);
        p.write(&mut w);
        let n = w.bit_len();
        let mut r = BitReader::with_bit_len(w.as_bytes(), n);
        r.read_bit();
        assert_eq!(T::read(&mut r).as_ref(), Some(&p));
        assert_eq!(r.remaining(), 0);
        if n > 1 {
            let mut r = BitReader::with_bit_len(w.as_bytes(), n - 1);
            r.read_bit();
            assert_eq!(T::read(&mut r), None, "truncated read must fail");
        }
    }

    fn sample_spawn() -> SpawnInfo {
        SpawnInfo {
            team: 3,
            skin: 0x0102_0304,
            unused: 0xAA,
            pos: v3(1.0, 2.0, -1.0),
            rotation: 0.5,
            weapons: [24, SpawnInfo::NO_WEAPON, 31],
            ammo: [100, 0, 0x0001_0000],
        }
    }

    /// The 46-byte spawn block from the spec table, built by hand.
    fn sample_spawn_bytes() -> Vec<u8> {
        cat(&[
            &[0x03],                   // 0 team
            &[0x04, 0x03, 0x02, 0x01], // 1 skin
            &[0xAA],                   // 5 unused
            &F_1,
            &F_2,
            &F_M1,   // 6 x, y, z
            &F_HALF, // 18 rotation
            &[0x18, 0, 0, 0],
            &[0xFF, 0xFF, 0xFF, 0xFF],
            &[0x1F, 0, 0, 0], // 22 weapons
            &[0x64, 0, 0, 0],
            &[0, 0, 0, 0],
            &[0, 0, 0x01, 0], // 34 ammo
        ])
    }

    // ---- spawn / class / death -------------------------------------------

    #[test]
    fn spawn_info_layout_is_46_bytes() {
        let b = sample_spawn_bytes();
        assert_eq!(b.len(), SpawnInfo::SIZE);
        assert_bytes(&SetSpawnInfo { spawn: sample_spawn() }, &b);
    }

    #[test]
    fn request_class_response_bytes() {
        let b = cat(&[&[0x01], &sample_spawn_bytes()]);
        assert_eq!(b.len(), 47);
        assert_bytes(&PlayerRequestClassResponse { selectable: 1, spawn: sample_spawn() }, &b);
    }

    #[test]
    fn request_class_is_u32() {
        // Client writes 32 bits (open.mp reads only the low u16).
        assert_bytes(&PlayerRequestClass { class_index: 0x0001_0002 }, &[0x02, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn request_spawn_response_reads_low_byte_of_u32() {
        // What open.mp's ImmediatelySpawnPlayer sends: u32 2.
        let wire = [0x02, 0x00, 0x00, 0x00];
        let mut r = BitReader::new(&wire);
        assert_eq!(PlayerRequestSpawnResponse::read(&mut r), Some(PlayerRequestSpawnResponse::immediate()));
        assert_eq!(r.remaining(), 24, "only 8 bits are consumed");
        // A server sending a bare u8 also decodes.
        let mut r = BitReader::new(&[0x01]);
        assert_eq!(PlayerRequestSpawnResponse::read(&mut r), Some(PlayerRequestSpawnResponse { response: 1 }));
        // Our writer emits the u32 servers send.
        assert_eq!(encode(&PlayerRequestSpawnResponse { response: 1 }), (vec![1, 0, 0, 0], 32));
        // Through the module decoder.
        let mut r = BitReader::new(&wire);
        assert_eq!(
            decode(129, &mut r),
            Some(Incoming::PlayerRequestSpawnResponse(PlayerRequestSpawnResponse { response: 2 }))
        );
    }

    #[test]
    fn empty_payloads() {
        assert_wire(&PlayerRequestSpawn, &[], 0);
        assert_wire(&PlayerSpawn, &[], 0);
        assert_wire(&ForcePlayerClassSelection, &[], 0);
        assert_wire(&ResetPlayerWeapons, &[], 0);
        assert_wire(&ResetPlayerMoney, &[], 0);
        assert_wire(&SetPlayerCameraBehindPlayer, &[], 0);
        // Full RPC frame for an empty C→S payload: 0x14, id, compressed 0, compressed 0.
        // compressed u32 0 = 1,1,1, 1 + nibble 0000 = 8 bits 0xF0 each.
        let w = PlayerSpawn.to_rpc();
        assert_eq!(w.as_bytes(), [0x14, 52, 0xF0, 0xF0]);
        assert_eq!(w.bit_len(), 32);
    }

    #[test]
    fn on_player_death_bytes() {
        assert_bytes(&OnPlayerDeath { reason: 255, killer_id: INVALID_PLAYER_ID }, &[0xFF, 0xFF, 0xFF]);
        assert_bytes(&OnPlayerDeath { reason: 24, killer_id: 0x0102 }, &[0x18, 0x02, 0x01]);
    }

    #[test]
    fn death_message_bytes() {
        assert_bytes(&SendDeathMessage { killer_id: 1, victim_id: 2, reason: 31 }, &[0x01, 0x00, 0x02, 0x00, 0x1F]);
    }

    #[test]
    fn player_death_bytes() {
        assert_bytes(&PlayerDeath { player_id: 0x03E8 }, &[0xE8, 0x03]);
    }

    // ---- position / physics ------------------------------------------------

    #[test]
    fn position_family_bytes() {
        let b = cat(&[&F_1, &F_2, &F_M1]);
        let p = v3(1.0, 2.0, -1.0);
        assert_bytes(&SetPlayerPosition { pos: p }, &b);
        assert_bytes(&SetPlayerPositionFindZ { pos: p }, &b);
        assert_bytes(&SetPlayerVelocity { velocity: p }, &b);
        assert_bytes(&SetPlayerCameraPosition { pos: p }, &b);
    }

    #[test]
    fn single_float_bytes() {
        assert_bytes(&SetPlayerHealth { health: 100.0 }, &F_100);
        assert_bytes(&SetPlayerArmour { armour: 100.0 }, &F_100);
        assert_bytes(&SetPlayerFacingAngle { angle: 0.5 }, &F_HALF);
        assert_bytes(&SetPlayerGravity { gravity: 4.0 }, &F_4);
    }

    #[test]
    fn single_byte_bytes() {
        assert_bytes(&TogglePlayerControllable { enable: 1 }, &[0x01]);
        assert_bytes(&TogglePlayerClock { enable: 0 }, &[0x00]);
        assert_bytes(&ToggleWidescreen { enable: 1 }, &[0x01]);
        assert_bytes(&SetPlayerSpecialAction { action: 68 }, &[0x44]);
        assert_bytes(&SetPlayerWorldTime { hour: 23 }, &[0x17]);
        assert_bytes(&SetPlayerWeather { weather: 9 }, &[0x09]);
        assert_bytes(&SetPlayerWantedLevel { level: 6 }, &[0x06]);
        assert_bytes(&SetPlayerInterior { interior: 3 }, &[0x03]);
        assert_bytes(&OnPlayerInteriorChange { interior: 18 }, &[0x12]);
    }

    #[test]
    fn single_bit_payloads() {
        assert_wire(&EnableStuntBonusForPlayer { enable: true }, &[0x80], 1);
        assert_wire(&EnableStuntBonusForPlayer { enable: false }, &[0x00], 1);
        assert_wire(&DisableRemoteVehicleCollisions { disable: true }, &[0x80], 1);
        assert_wire(&SetPlayerCameraTargeting { enabled: true }, &[0x80], 1);
    }

    #[test]
    fn clear_tasks_and_camera_attach_bytes() {
        assert_bytes(&ClearPlayerTasks { player_id: 7 }, &[0x07, 0x00]);
        assert_bytes(&AttachCameraToObject { object_id: 0x0102 }, &[0x02, 0x01]);
    }

    // ---- appearance ----------------------------------------------------------

    #[test]
    fn set_player_skin_is_u32_u32() {
        assert_bytes(&SetPlayerSkin { player_id: 5, skin: 294 }, &[0x05, 0, 0, 0, 0x26, 0x01, 0, 0]);
    }

    #[test]
    fn team_colour_style_bytes() {
        assert_bytes(&SetPlayerTeam { player_id: 0x0102, team: 255 }, &[0x02, 0x01, 0xFF]);
        assert_bytes(&SetPlayerColor { player_id: 1, colour: 0xFF00_00AA }, &[0x01, 0x00, 0xAA, 0x00, 0x00, 0xFF]);
        assert_bytes(&SetPlayerFightingStyle { player_id: 2, style: 6 }, &[0x02, 0x00, 0x06]);
    }

    #[test]
    fn skill_level_bytes() {
        assert_bytes(
            &SetPlayerSkillLevel { player_id: 2, skill_type: 5, level: 999 },
            &[0x02, 0x00, 0x05, 0x00, 0x00, 0x00, 0xE7, 0x03],
        );
    }

    #[test]
    fn shop_name_is_fixed_32_bytes() {
        let mut b = [0u8; 32];
        b[..6].copy_from_slice(b"FDPIZA");
        let p = SetPlayerShopName::from_name(b"FDPIZA");
        assert_bytes(&p, &b);
        assert_eq!(p.name_bytes(), b"FDPIZA");
        let long = SetPlayerShopName::from_name(&[b'x'; 40]);
        assert_eq!(long.name_bytes().len(), 31);
        assert_eq!(long.name[31], 0);
        let full = SetPlayerShopName { name: [b'y'; 32] };
        assert_eq!(full.name_bytes().len(), 32);
        assert_eq!(SetPlayerShopName::default().name_bytes(), b"");
    }

    #[test]
    fn drunk_level_is_signed() {
        assert_bytes(&SetPlayerDrunkLevel { level: 50_000 }, &[0x50, 0xC3, 0x00, 0x00]);
        assert_bytes(&SetPlayerDrunkLevel { level: -1 }, &[0xFF; 4]);
    }

    #[test]
    fn apply_animation_bits() {
        // u16 id, str8 "PED", str8 "WALK", f32 4.0, bits 1,0,1,1, u32 1000.
        let head = cat(&[&[0x07, 0x00], &[3], b"PED", &[4], b"WALK", &F_4]);
        let seq = [bits_of(&head), vec![true, false, true, true], bits_of(&1000u32.to_le_bytes())].concat();
        let (bytes, bits) = pack(&seq);
        assert_eq!(bits, (2 + 1 + 3 + 1 + 4 + 4) * 8 + 4 + 32);
        let p = ApplyPlayerAnimation {
            player_id: 7,
            lib: b"PED".to_vec(),
            name: b"WALK".to_vec(),
            delta: 4.0,
            looping: true,
            lock_x: false,
            lock_y: true,
            freeze: true,
            time: 1000,
        };
        assert_wire(&p, &bytes, bits);
    }

    // ---- weapons / money -----------------------------------------------------

    #[test]
    fn weapon_and_money_bytes() {
        assert_bytes(&GivePlayerWeapon { weapon: 24, ammo: 500 }, &[0x18, 0, 0, 0, 0xF4, 0x01, 0, 0]);
        assert_bytes(&SetPlayerArmedWeapon { weapon: 46 }, &[0x2E, 0, 0, 0]);
        assert_bytes(&SetPlayerAmmo { weapon: 31, ammo: 0x0102 }, &[0x1F, 0x02, 0x01]);
        assert_bytes(&GivePlayerMoney { amount: -1 }, &[0xFF; 4]);
        assert_bytes(&GivePlayerMoney { amount: 1000 }, &[0xE8, 0x03, 0, 0]);
    }

    // ---- world ---------------------------------------------------------------

    #[test]
    fn time_bytes() {
        assert_bytes(&SetPlayerTime { hour: 12, minute: 30 }, &[0x0C, 0x1E]);
        assert_bytes(&SendGameTimeUpdate { time: 0x0102_0304 }, &[0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn world_bounds_order() {
        assert_bytes(
            &SetWorldBounds { max_x: 20000.0, min_x: -20000.0, max_y: 1.0, min_y: -1.0 },
            &cat(&[&F_20K, &F_M20K, &F_1, &F_M1]),
        );
    }

    #[test]
    fn virtual_world_bytes() {
        assert_bytes(&SetPlayerVirtualWorld { world_id: 0x0A0B }, &[0x0B, 0x0A, 0, 0]);
    }

    #[test]
    fn play_sound_bytes() {
        assert_bytes(
            &PlayerPlaySound { sound_id: 1052, pos: v3(1.0, 2.0, -1.0) },
            &cat(&[&[0x1C, 0x04, 0, 0], &F_1, &F_2, &F_M1]),
        );
    }

    #[test]
    fn explosion_bytes() {
        assert_bytes(
            &CreateExplosion { pos: v3(1.0, 2.0, -1.0), kind: 7, radius: 4.0 },
            &cat(&[&F_1, &F_2, &F_M1, &[7, 0, 0, 0], &F_4]),
        );
    }

    #[test]
    fn remove_building_bytes() {
        assert_bytes(
            &RemoveBuildingForPlayer { model_id: u32::MAX, pos: v3(1.0, 2.0, -1.0), radius: 100.0 },
            &cat(&[&[0xFF; 4], &F_1, &F_2, &F_M1, &F_100]),
        );
    }

    #[test]
    fn crime_report_bytes() {
        assert_bytes(
            &PlayCrimeReport {
                suspect_id: 0x0102,
                in_vehicle: 1,
                vehicle_model: 411,
                vehicle_colour: 3,
                crime_id: 16,
                pos: v3(1.0, 2.0, -1.0),
            },
            &cat(&[
                &[0x02, 0x01],
                &[1, 0, 0, 0],
                &[0x9B, 0x01, 0, 0],
                &[3, 0, 0, 0],
                &[0x10, 0, 0, 0],
                &F_1,
                &F_2,
                &F_M1,
            ]),
        );
    }

    // ---- camera --------------------------------------------------------------

    #[test]
    fn camera_look_at_bytes() {
        assert_bytes(&SetPlayerCameraLookAt { pos: v3(1.0, 2.0, -1.0), cut: 2 }, &cat(&[&F_1, &F_2, &F_M1, &[2]]));
    }

    #[test]
    fn interpolate_camera_bits() {
        let body = cat(&[&F_1, &F_2, &F_M1, &F_HALF, &F_4, &F_100, &[0xE8, 0x03, 0, 0], &[1]]);
        let (bytes, bits) = pack(&[vec![true], bits_of(&body)].concat());
        assert_eq!(bits, 1 + 6 * 32 + 32 + 8);
        assert_wire(
            &InterpolateCamera {
                is_position: true,
                from: v3(1.0, 2.0, -1.0),
                to: v3(0.5, 4.0, 100.0),
                time: 1000,
                cut: 1,
            },
            &bytes,
            bits,
        );
        // Leading bit 0 = look-at interpolation.
        let (bytes, bits) = pack(&[vec![false], bits_of(&body)].concat());
        let mut r = BitReader::with_bit_len(&bytes, bits);
        assert!(!InterpolateCamera::read(&mut r).unwrap().is_position);
    }

    // ---- damage --------------------------------------------------------------

    #[test]
    fn give_take_damage_bits() {
        let body = cat(&[&[0x02, 0x01], &F_HALF, &[24, 0, 0, 0], &[3, 0, 0, 0]]);
        let (bytes, bits) = pack(&[vec![true], bits_of(&body)].concat());
        assert_eq!(bits, 1 + 16 + 32 + 32 + 32);
        assert_wire(
            &OnPlayerGiveTakeDamage { taking: true, player_id: 0x0102, damage: 0.5, weapon_id: 24, bodypart: 3 },
            &bytes,
            bits,
        );
    }

    // ---- round trips -----------------------------------------------------------

    #[test]
    fn roundtrip_all() {
        let p = v3(-1.5, 2.25, 1e5);
        roundtrip(SetPlayerPosition { pos: p });
        roundtrip(SetPlayerPositionFindZ { pos: p });
        roundtrip(SetPlayerHealth { health: 55.5 });
        roundtrip(TogglePlayerControllable { enable: 0 });
        roundtrip(PlayerPlaySound { sound_id: 2000, pos: p });
        roundtrip(SetWorldBounds { max_x: 1.0, min_x: -2.0, max_y: 3.0, min_y: -4.0 });
        roundtrip(GivePlayerMoney { amount: i32::MIN });
        roundtrip(SetPlayerFacingAngle { angle: 270.0 });
        roundtrip(ResetPlayerMoney);
        roundtrip(ResetPlayerWeapons);
        roundtrip(GivePlayerWeapon { weapon: 38, ammo: u32::MAX });
        roundtrip(SetPlayerTime { hour: 23, minute: 59 });
        roundtrip(TogglePlayerClock { enable: 1 });
        roundtrip(SetPlayerShopName::from_name(b"AMMUN1"));
        roundtrip(SetPlayerSkillLevel { player_id: 999, skill_type: 10, level: 1000 });
        roundtrip(SetPlayerDrunkLevel { level: 2001 });
        roundtrip(RemoveBuildingForPlayer { model_id: 1302, pos: p, radius: 0.25 });
        roundtrip(SetPlayerVirtualWorld { world_id: -5 });
        roundtrip(PlayerSpawn);
        roundtrip(OnPlayerDeath { reason: 49, killer_id: 3 });
        roundtrip(SendDeathMessage { killer_id: INVALID_PLAYER_ID, victim_id: 4, reason: 54 });
        roundtrip(SendGameTimeUpdate { time: -42 });
        roundtrip(SetPlayerArmour { armour: 0.0 });
        roundtrip(SetPlayerArmedWeapon { weapon: 0 });
        roundtrip(SetSpawnInfo { spawn: sample_spawn() });
        roundtrip(SetPlayerTeam { player_id: 1, team: 2 });
        roundtrip(SetPlayerColor { player_id: 1003, colour: 0x1234_5678 });
        roundtrip(ForcePlayerClassSelection);
        roundtrip(CreateExplosion { pos: p, kind: 12, radius: 10.0 });
        roundtrip(AttachCameraToObject { object_id: 1000 });
        roundtrip(InterpolateCamera { is_position: false, from: p, to: p, time: -1, cut: 0 });
        roundtrip(ApplyPlayerAnimation {
            player_id: 0,
            lib: b"DANCING".to_vec(),
            name: Vec::new(),
            delta: 4.1,
            looping: false,
            lock_x: true,
            lock_y: false,
            freeze: false,
            time: 0,
        });
        roundtrip(ClearPlayerTasks { player_id: 12 });
        roundtrip(SetPlayerSpecialAction { action: 2 });
        roundtrip(SetPlayerFightingStyle { player_id: 3, style: 15 });
        roundtrip(SetPlayerVelocity { velocity: p });
        roundtrip(SetPlayerWorldTime { hour: 6 });
        roundtrip(EnableStuntBonusForPlayer { enable: true });
        roundtrip(ToggleWidescreen { enable: 0 });
        roundtrip(PlayCrimeReport { suspect_id: 9, crime_id: 4, pos: p, ..Default::default() });
        roundtrip(OnPlayerGiveTakeDamage { taking: false, player_id: 8, damage: 15.18, weapon_id: 0, bodypart: 9 });
        roundtrip(OnPlayerInteriorChange { interior: 0 });
        roundtrip(PlayerRequestClass { class_index: 311 });
        roundtrip(PlayerRequestClassResponse { selectable: 0, spawn: SpawnInfo::default() });
        roundtrip(PlayerRequestSpawn);
        roundtrip(SetPlayerWantedLevel { level: 0 });
        roundtrip(SetPlayerAmmo { weapon: 22, ammo: 65535 });
        roundtrip(SetPlayerGravity { gravity: 0.008 });
        roundtrip(SetPlayerWeather { weather: 255 });
        roundtrip(SetPlayerSkin { player_id: 0xFFFF_FFFF, skin: 0 });
        roundtrip(SetPlayerInterior { interior: 255 });
        roundtrip(SetPlayerCameraPosition { pos: p });
        roundtrip(SetPlayerCameraLookAt { pos: p, cut: 3 });
        roundtrip(SetPlayerCameraBehindPlayer);
        roundtrip(PlayerDeath { player_id: 1004 });
        roundtrip(DisableRemoteVehicleCollisions { disable: false });
        roundtrip(SetPlayerCameraTargeting { enabled: false });
    }

    #[test]
    fn request_spawn_response_roundtrip() {
        // The writer emits 32 bits, the reader consumes 8 (client behaviour).
        for v in [0u8, 1, 2, 255] {
            let p = PlayerRequestSpawnResponse { response: v };
            let (bytes, bits) = encode(&p);
            let mut r = BitReader::with_bit_len(&bytes, bits);
            assert_eq!(PlayerRequestSpawnResponse::read(&mut r), Some(p));
            assert_eq!(r.remaining(), 24);
        }
    }

    // ---- module dispatch -------------------------------------------------------

    #[test]
    fn incoming_ids_match_scope() {
        let mut ids = INCOMING_IDS.to_vec();
        ids.sort_unstable();
        let expected = [
            12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 29, 30, 33, 34, 35, 43, 48, 55, 60, 66, 67, 68, 69, 72, 74, 79,
            81, 82, 86, 87, 88, 89, 90, 94, 104, 111, 112, 128, 129, 133, 145, 146, 152, 153, 156, 157, 158, 162, 166,
            167, 170,
        ];
        assert_eq!(ids, expected);
        // C→S-only ids are not decoded as incoming.
        for id in [52u8, 53, 115, 118, 11, 23, 96, 119, 168] {
            assert!(decode(id, &mut BitReader::new(&[0; 64])).is_none(), "id {id}");
        }
    }

    #[test]
    fn decode_dispatch_roundtrip() {
        let samples = vec![
            Incoming::SetPlayerPosition(SetPlayerPosition { pos: v3(1.0, 2.0, 3.0) }),
            Incoming::SetSpawnInfo(SetSpawnInfo { spawn: sample_spawn() }),
            Incoming::PlayerRequestClassResponse(PlayerRequestClassResponse { selectable: 1, spawn: sample_spawn() }),
            Incoming::ApplyPlayerAnimation(ApplyPlayerAnimation {
                player_id: 1,
                lib: b"PED".to_vec(),
                name: b"SEAT_DOWN".to_vec(),
                ..Default::default()
            }),
            Incoming::InterpolateCamera(InterpolateCamera { is_position: true, time: 5, ..Default::default() }),
            Incoming::SetPlayerCameraBehindPlayer(SetPlayerCameraBehindPlayer),
            Incoming::SetPlayerShopName(SetPlayerShopName::from_name(b"X")),
            Incoming::EnableStuntBonusForPlayer(EnableStuntBonusForPlayer { enable: true }),
        ];
        for m in samples {
            let mut w = BitWriter::new();
            m.write(&mut w);
            let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
            assert_eq!(decode(m.id(), &mut r), Some(m.clone()));
            assert_eq!(r.remaining(), 0);
        }
    }

    #[test]
    fn decode_through_rpc_frame() {
        let w = SetPlayerHealth { health: 100.0 }.to_rpc();
        let rpc = crate::rpc::decode(w.as_bytes(), w.bit_len()).unwrap();
        assert_eq!(rpc.id, 14);
        assert_eq!(
            decode(rpc.id, &mut rpc.reader()),
            Some(Incoming::SetPlayerHealth(SetPlayerHealth { health: 100.0 }))
        );
    }

    #[test]
    fn truncated_is_none() {
        assert!(decode(68, &mut BitReader::new(&[0; 45])).is_none());
        assert!(decode(128, &mut BitReader::new(&[0; 46])).is_none());
        assert!(decode(104, &mut BitReader::new(&[])).is_none());
        assert!(decode(162, &mut BitReader::new(&[])).is_some());
        // Animation with a library length larger than the data.
        assert!(decode(86, &mut BitReader::new(&[0, 0, 10, b'a'])).is_none());
    }
}
