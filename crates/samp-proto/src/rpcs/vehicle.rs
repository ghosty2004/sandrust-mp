//! Vehicle RPCs (stream in/out, params, damage, mods, trailers, enter/exit).
//!
//! Spec: docs/re/subsystems/vehicles.md. Every field is raw little-endian with
//! no compression or alignment. The spec found no wire-layout disagreement
//! with open.mp for any RPC in this module.
//!
//! Several ids carry different payloads per direction:
//! - 26: [`EnterVehicle`] (S→C) / [`OnPlayerEnterVehicle`] (C→S);
//! - 96: [`ScmEvent`] (S→C) / [`OnPlayerScmEvent`] (C→S), with a different field order;
//! - 154: [`ExitVehicle`] (S→C) / [`OnPlayerExitVehicle`] (C→S);
//! - 106: [`SetVehicleDamageStatus`] has the same layout both ways.
//!
//! Easy-to-miss layout points (spec "Summary of open.mp vs client"): 164 puts
//! door damage before panel damage while 106 puts panels first; 148 carries
//! the trailer id before the cab id; 164 encodes mods as `component − 999`
//! and the paintjob as `paintjob + 1`, whereas 57 / 96 carry raw values.

use super::RpcPayload;
use crate::bitstream::{BitReader, BitWriter};
use crate::types::{Vec3, read_str8, write_str8};

/// Size of the client's vehicle pool (valid ids 0..1999; the client ignores
/// RPCs naming other ids). Spec: vehicles.md "Overview".
pub const MAX_VEHICLES: u16 = 2000;

/// Number of mod slots carried by [`StreamInVehicle`].
pub const STREAM_IN_MOD_SLOTS: usize = 14;

/// Longest plate text the client accepts in [`SetVehiclePlate`].
pub const MAX_PLATE_LEN: usize = 32;

// ---------------------------------------------------------------------------
// 164 / 165: streaming
// ---------------------------------------------------------------------------

/// RPC 164 StreamInVehicle (S→C): creates a vehicle in the client's pool.
///
/// Spec: vehicles.md "164 StreamInVehicle". 63 bytes, read by the client as
/// two raw blocks (40 + 23 bytes).
///
/// Client behaviour worth knowing:
/// - `model` outside 400..=611 makes the client ignore the whole RPC.
/// - An occupied slot is deleted first (with a "was not deleted" warning).
/// - Locomotives 537 / 538 also create carriages (569 / 570) at `vehicle_id + 1..=+3`;
///   the server must keep those ids reserved.
/// - ⚠ `door_damage` precedes `panel_damage` here (opposite of RPC 106).
/// - `mods` / `paintjob` are only applied to automobiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamInVehicle {
    pub vehicle_id: u16,
    /// Vehicle model, 400..=611 accepted by the client.
    pub model: u32,
    pub position: Vec3,
    /// Z rotation in degrees.
    pub angle: f32,
    /// 0xFF = keep the model's default colour.
    pub colour1: u8,
    /// 0xFF = keep the model's default colour.
    pub colour2: u8,
    pub health: f32,
    /// 0 = not linked to an interior.
    pub interior: u8,
    pub door_damage: u32,
    pub panel_damage: u32,
    pub light_damage: u8,
    /// Tyre bits as in [`SetVehicleDamageStatus::tyres`].
    pub tyre_damage: u8,
    /// Non-zero = vehicle has a siren.
    pub siren: u8,
    /// Wire-encoded mods: 0 = empty, otherwise `component_id − 999`.
    /// See [`StreamInVehicle::component`].
    pub mods: [u8; STREAM_IN_MOD_SLOTS],
    /// Wire-encoded paintjob: 0 = none, otherwise `paintjob + 1`.
    /// See [`StreamInVehicle::paintjob`].
    pub paintjob: u8,
    /// 0xFFFF_FFFF = unset; otherwise the low byte is the colour index.
    pub body_colour1: u32,
    /// 0xFFFF_FFFF = unset.
    pub body_colour2: u32,
}

impl StreamInVehicle {
    /// Body-colour value meaning "not set".
    pub const BODY_COLOUR_UNSET: u32 = 0xFFFF_FFFF;
    /// `colour1` / `colour2` value meaning "model default".
    pub const COLOUR_DEFAULT: u8 = 0xFF;

    /// Component model id stored in mod slot `slot` (byte + 999), or `None`
    /// for an empty or out-of-range slot.
    pub fn component(&self, slot: usize) -> Option<u16> {
        match *self.mods.get(slot)? {
            0 => None,
            b => Some(u16::from(b) + 999),
        }
    }

    /// Encode a component model id into its mod-slot byte (0 = empty).
    /// Returns `None` if the id is not representable (outside 1000..=1254).
    pub fn encode_component(component: Option<u16>) -> Option<u8> {
        match component {
            None => Some(0),
            Some(c) => c.checked_sub(999).filter(|&b| b != 0).and_then(|b| u8::try_from(b).ok()),
        }
    }

    /// Decoded paintjob (`byte − 1`), or `None` when the byte is 0.
    pub fn paintjob(&self) -> Option<u8> {
        self.paintjob.checked_sub(1)
    }

    /// Wire byte for a paintjob (`None` → 0, `Some(p)` → `p + 1`, saturating).
    pub fn encode_paintjob(paintjob: Option<u8>) -> u8 {
        paintjob.map_or(0, |p| p.saturating_add(1))
    }
}

impl RpcPayload for StreamInVehicle {
    const ID: u8 = 164;
    fn read(r: &mut BitReader) -> Option<Self> {
        let vehicle_id = r.read_u16()?;
        let model = r.read_u32()?;
        let position = Vec3::read(r)?;
        let angle = r.read_f32()?;
        let colour1 = r.read_u8()?;
        let colour2 = r.read_u8()?;
        let health = r.read_f32()?;
        let interior = r.read_u8()?;
        let door_damage = r.read_u32()?;
        let panel_damage = r.read_u32()?;
        let light_damage = r.read_u8()?;
        let tyre_damage = r.read_u8()?;
        let siren = r.read_u8()?;
        let mut mods = [0u8; STREAM_IN_MOD_SLOTS];
        r.read_bytes_into(&mut mods)?;
        let paintjob = r.read_u8()?;
        let body_colour1 = r.read_u32()?;
        let body_colour2 = r.read_u32()?;
        Some(Self {
            vehicle_id,
            model,
            position,
            angle,
            colour1,
            colour2,
            health,
            interior,
            door_damage,
            panel_damage,
            light_damage,
            tyre_damage,
            siren,
            mods,
            paintjob,
            body_colour1,
            body_colour2,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u32(self.model);
        self.position.write(w);
        w.write_f32(self.angle);
        w.write_u8(self.colour1);
        w.write_u8(self.colour2);
        w.write_f32(self.health);
        w.write_u8(self.interior);
        w.write_u32(self.door_damage);
        w.write_u32(self.panel_damage);
        w.write_u8(self.light_damage);
        w.write_u8(self.tyre_damage);
        w.write_u8(self.siren);
        w.write_bytes(&self.mods);
        w.write_u8(self.paintjob);
        w.write_u32(self.body_colour1);
        w.write_u32(self.body_colour2);
    }
}

/// RPC 165 StreamOutVehicle (S→C): removes a vehicle from the client's pool.
///
/// Spec: vehicles.md "165 StreamOutVehicle". If the local ped is inside it,
/// the client teleports the ped to the vehicle's position; occupants are
/// removed, trailers pointing at it are detached, and for models 537 / 538
/// the carriages at `vehicle_id + 1..=+3` are deleted too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamOutVehicle {
    pub vehicle_id: u16,
}

impl RpcPayload for StreamOutVehicle {
    const ID: u8 = 165;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
    }
}

// ---------------------------------------------------------------------------
// 24: params
// ---------------------------------------------------------------------------

/// The 16 one-byte vehicle parameters of [`SetVehicleParams`], in wire order.
///
/// Spec: vehicles.md "24 SetVehicleParams". Each byte is acted on by the
/// client only when it is 0 or 1; any other value (open.mp uses 0xFF for
/// "unset", see [`VehicleParams::UNSET`]) is ignored. Bonnet, boot and door
/// open/close only work on automobiles. For windows, 0 = open (inferred).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VehicleParams {
    pub engine: u8,
    pub lights: u8,
    pub alarm: u8,
    /// 1 = doors locked.
    pub doors: u8,
    pub bonnet: u8,
    pub boot: u8,
    /// 1 = show the objective radar marker for this vehicle.
    pub objective: u8,
    pub siren: u8,
    pub door_driver: u8,
    pub door_passenger: u8,
    pub door_back_left: u8,
    pub door_back_right: u8,
    pub window_driver: u8,
    pub window_passenger: u8,
    pub window_back_left: u8,
    pub window_back_right: u8,
}

impl VehicleParams {
    /// Conventional "leave unchanged" value (anything other than 0/1 works).
    pub const UNSET: u8 = 0xFF;

    /// All parameters set to [`VehicleParams::UNSET`].
    pub const fn unset() -> Self {
        Self::from_bytes([Self::UNSET; 16])
    }

    /// Parameters in wire order.
    pub const fn to_bytes(&self) -> [u8; 16] {
        [
            self.engine,
            self.lights,
            self.alarm,
            self.doors,
            self.bonnet,
            self.boot,
            self.objective,
            self.siren,
            self.door_driver,
            self.door_passenger,
            self.door_back_left,
            self.door_back_right,
            self.window_driver,
            self.window_passenger,
            self.window_back_left,
            self.window_back_right,
        ]
    }

    /// Parameters from wire-order bytes.
    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self {
            engine: b[0],
            lights: b[1],
            alarm: b[2],
            doors: b[3],
            bonnet: b[4],
            boot: b[5],
            objective: b[6],
            siren: b[7],
            door_driver: b[8],
            door_passenger: b[9],
            door_back_left: b[10],
            door_back_right: b[11],
            window_driver: b[12],
            window_passenger: b[13],
            window_back_left: b[14],
            window_back_right: b[15],
        }
    }
}

impl Default for VehicleParams {
    fn default() -> Self {
        Self::unset()
    }
}

/// RPC 24 SetVehicleParams (S→C): engine, lights, alarm, locks, doors, windows…
///
/// Spec: vehicles.md "24 SetVehicleParams". Wire: u16 vehicle id, then a
/// 16-byte block ([`VehicleParams`]). Engine/lights are applied by the pool's
/// per-frame process and depend on InitGame's "manual engine and lights" flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetVehicleParams {
    pub vehicle_id: u16,
    pub params: VehicleParams,
}

impl RpcPayload for SetVehicleParams {
    const ID: u8 = 24;
    fn read(r: &mut BitReader) -> Option<Self> {
        let vehicle_id = r.read_u16()?;
        let mut b = [0u8; 16];
        r.read_bytes_into(&mut b)?;
        Some(Self { vehicle_id, params: VehicleParams::from_bytes(b) })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_bytes(&self.params.to_bytes());
    }
}

// ---------------------------------------------------------------------------
// 26 / 154 / 70 / 71: entering and leaving
// ---------------------------------------------------------------------------

/// RPC 26 EnterVehicle (S→C): a remote player starts entering a vehicle.
///
/// Spec: vehicles.md "26 EnterVehicle (S→C)". The client acts only if
/// `player_id < 1005`, the remote player has a ped, and it is within 200
/// units; the ped then gets a 3000 ms walk-in task (driver or any passenger seat).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnterVehicle {
    pub player_id: u16,
    pub vehicle_id: u16,
    /// Non-zero = enters as passenger.
    pub passenger: u8,
}

impl RpcPayload for EnterVehicle {
    const ID: u8 = 26;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, vehicle_id: r.read_u16()?, passenger: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u16(self.vehicle_id);
        w.write_u8(self.passenger);
    }
}

/// RPC 26 OnPlayerEnterVehicle (C→S): the local player starts entering a vehicle.
///
/// Spec: vehicles.md "26 OnPlayerEnterVehicle (C→S)". Sent RELIABLE_SEQUENCED
/// (10), channel 0. The client sends it from the driver enter-task hook
/// (passenger = 0; refused for locked doors, model 570, unmapped vehicles…)
/// or from the passenger-key poll when a vehicle is closer than 8.0 (passenger = 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerEnterVehicle {
    pub vehicle_id: u16,
    /// 0 = driver, 1 = passenger.
    pub passenger: u8,
}

impl RpcPayload for OnPlayerEnterVehicle {
    const ID: u8 = 26;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, passenger: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.passenger);
    }
}

/// RPC 154 ExitVehicle (S→C): a remote player leaves a vehicle.
///
/// Spec: vehicles.md "154 ExitVehicle (S→C)". Same distance / id checks as
/// [`EnterVehicle`]. ⚠ The client reads `vehicle_id` but does not use it: the
/// remote ped leaves whatever vehicle it is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExitVehicle {
    pub player_id: u16,
    pub vehicle_id: u16,
}

impl RpcPayload for ExitVehicle {
    const ID: u8 = 154;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { player_id: r.read_u16()?, vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u16(self.vehicle_id);
    }
}

/// RPC 154 OnPlayerExitVehicle (C→S): the local player leaves a vehicle.
///
/// Spec: vehicles.md "154 OnPlayerExitVehicle (C→S)". Sent RELIABLE_SEQUENCED
/// (10), channel 0, only for a player-input exit (not after RPC 71), never for
/// trains or RC models. Leaving the driver seat also records the vehicle as
/// "last driven", which gates RPC 136.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerExitVehicle {
    pub vehicle_id: u16,
}

impl RpcPayload for OnPlayerExitVehicle {
    const ID: u8 = 154;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
    }
}

/// RPC 70 PutPlayerInVehicle (S→C): warp the local player into a seat.
///
/// Spec: vehicles.md "70 PutPlayerInVehicle". Seat 0 = driver (refused if
/// already occupied), n > 0 = passenger seat n − 1. Refused for a dead
/// vehicle or a seat beyond the vehicle's capacity. No reply RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PutPlayerInVehicle {
    pub vehicle_id: u16,
    pub seat: u8,
}

impl RpcPayload for PutPlayerInVehicle {
    const ID: u8 = 70;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, seat: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.seat);
    }
}

/// RPC 71 RemovePlayerFromVehicle (S→C): the local player leaves its vehicle.
///
/// Spec: vehicles.md "71 RemovePlayerFromVehicle". Empty payload. Ignored in
/// models 538 / 570. The resulting exit does not send RPC 154.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemovePlayerFromVehicle;

impl RpcPayload for RemovePlayerFromVehicle {
    const ID: u8 = 71;
    fn read(_r: &mut BitReader) -> Option<Self> {
        Some(Self)
    }
    fn write(&self, _w: &mut BitWriter) {}
}

// ---------------------------------------------------------------------------
// 106 / 136 / 147: damage and health
// ---------------------------------------------------------------------------

/// RPC 106 SetVehicleDamageStatus (both directions, same layout).
///
/// Spec: vehicles.md "106 SetVehicleDamageStatus". ⚠ Panels precede doors
/// here (164 has doors first).
///
/// - S→C: panels/doors/lights are applied to automobiles and planes only
///   (all three 0 on a damaged vehicle = repair); tyres to automobiles and bikes.
/// - C→S: sent RELIABLE (8), channel 0, by the driver whenever the cached
///   damage of the same vehicle changes (no send on vehicle change).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetVehicleDamageStatus {
    pub vehicle_id: u16,
    pub panels: u32,
    pub doors: u32,
    pub lights: u8,
    /// Automobile: bit0..bit3 = wheels (bit0 = GTA wheel byte +0x5A8 … bit3 =
    /// +0x5A5). Bike: bit0 = rear, bit1 = front.
    pub tyres: u8,
}

impl RpcPayload for SetVehicleDamageStatus {
    const ID: u8 = 106;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            vehicle_id: r.read_u16()?,
            panels: r.read_u32()?,
            doors: r.read_u32()?,
            lights: r.read_u8()?,
            tyres: r.read_u8()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u32(self.panels);
        w.write_u32(self.doors);
        w.write_u8(self.lights);
        w.write_u8(self.tyres);
    }
}

/// RPC 136 VehicleDeath (C→S): a vehicle was destroyed or sank.
///
/// Spec: vehicles.md "136 VehicleDeath". Sent RELIABLE_SEQUENCED (10),
/// channel 0, only by the vehicle's last driver and at most once per death
/// (the "last driven vehicle" is reset to 0xFFFF after sending).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VehicleDeath {
    pub vehicle_id: u16,
}

impl RpcPayload for VehicleDeath {
    const ID: u8 = 136;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
    }
}

/// RPC 147 SetVehicleHealth (S→C).
///
/// Spec: vehicles.md "147 SetVehicleHealth". Written straight to the GTA
/// vehicle's health; the client does not clamp it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetVehicleHealth {
    pub vehicle_id: u16,
    pub health: f32,
}

impl RpcPayload for SetVehicleHealth {
    const ID: u8 = 147;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, health: r.read_f32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_f32(self.health);
    }
}

// ---------------------------------------------------------------------------
// 57 / 96 / 123: mods, colours, plate
// ---------------------------------------------------------------------------

/// RPC 57 RemoveVehicleComponent (S→C).
///
/// Spec: vehicles.md "57 RemoveVehicleComponent". `component` is the full
/// component model id (1000..=1193). open.mp also sends this to veto a
/// client's mod event (96 type 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoveVehicleComponent {
    pub vehicle_id: u16,
    pub component: u16,
}

impl RpcPayload for RemoveVehicleComponent {
    const ID: u8 = 57;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, component: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u16(self.component);
    }
}

/// SCM event types carried by RPC 96 (spec: vehicles.md "96 SCMEvent").
pub mod scm_event_type {
    /// arg1 = paintjob 0..=3 (applied to automobiles only).
    pub const PAINTJOB: u32 = 1;
    /// arg1 = component model id (automobiles only; may be dropped if the
    /// model does not load within ~20 ms).
    pub const ADD_COMPONENT: u32 = 2;
    /// arg1, arg2 = body colours.
    pub const COLOUR: u32 = 3;
    /// arg1 = non-zero entering / 0 leaving, arg2 = interior.
    pub const MOD_SHOP: u32 = 4;
}

/// RPC 96 SCMEvent (S→C): a mod-shop event (paintjob, component, colours,
/// mod-shop enter/exit) to apply locally.
///
/// Spec: vehicles.md "96 SCMEvent (S→C)". Wire order: u16 player, u32 type,
/// u32 vehicle, u32 arg1, u32 arg2 — ⚠ differs from the C→S form
/// ([`OnPlayerScmEvent`]). The client ignores it until the local player has
/// spawned once. Types in [`scm_event_type`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScmEvent {
    /// Originating player (INVALID_PLAYER_ID for server-initiated events).
    pub player_id: u16,
    pub event_type: u32,
    pub vehicle_id: u32,
    pub arg1: u32,
    pub arg2: u32,
}

impl RpcPayload for ScmEvent {
    const ID: u8 = 96;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            player_id: r.read_u16()?,
            event_type: r.read_u32()?,
            vehicle_id: r.read_u32()?,
            arg1: r.read_u32()?,
            arg2: r.read_u32()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u32(self.event_type);
        w.write_u32(self.vehicle_id);
        w.write_u32(self.arg1);
        w.write_u32(self.arg2);
    }
}

/// RPC 96 SCMEvent (C→S): the local game script raised a mod-shop event.
///
/// Spec: vehicles.md "96 SCMEvent (C→S)". Wire order: u32 vehicle, u32 arg1,
/// u32 arg2, u32 type (no player id) — ⚠ differs from the S→C form
/// ([`ScmEvent`]). Sent RELIABLE_SEQUENCED (10), channel 0, only for types
/// 1..=4 and for vehicles that map to a SA-MP id (widened to 32 bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnPlayerScmEvent {
    pub vehicle_id: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub event_type: u32,
}

impl RpcPayload for OnPlayerScmEvent {
    const ID: u8 = 96;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u32()?, arg1: r.read_u32()?, arg2: r.read_u32()?, event_type: r.read_u32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.vehicle_id);
        w.write_u32(self.arg1);
        w.write_u32(self.arg2);
        w.write_u32(self.event_type);
    }
}

/// RPC 123 SetVehiclePlate (S→C).
///
/// Spec: vehicles.md "123 SetVehiclePlate". Wire: u16 vehicle id, u8 length,
/// text (not NUL-terminated). The client ignores the RPC if the length
/// exceeds [`MAX_PLATE_LEN`] (32); the decoder still returns such payloads.
/// The plate texture is built once, so in practice it only takes effect when
/// sent right after stream-in (inferred).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetVehiclePlate {
    pub vehicle_id: u16,
    pub plate: Vec<u8>,
}

impl RpcPayload for SetVehiclePlate {
    const ID: u8 = 123;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, plate: read_str8(r)? })
    }
    /// Plates longer than 255 bytes are truncated by the u8 length prefix.
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        write_str8(w, &self.plate);
    }
}

// ---------------------------------------------------------------------------
// 65 / 91 / 148 / 149 / 159 / 160: placement and physics
// ---------------------------------------------------------------------------

/// RPC 65 LinkVehicleToInterior (S→C).
///
/// Spec: vehicles.md "65 LinkVehicleToInterior". The pool's stored stream-in
/// interior is not changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkVehicleToInterior {
    pub vehicle_id: u16,
    pub interior: u8,
}

impl RpcPayload for LinkVehicleToInterior {
    const ID: u8 = 65;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, interior: r.read_u8()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_u8(self.interior);
    }
}

/// RPC 91 SetVehicleVelocity (S→C): set the speed of the vehicle the local
/// player is in.
///
/// Spec: vehicles.md "91 SetVehicleVelocity". ⚠ No vehicle id on the wire.
/// `kind` 0 ([`Self::LINEAR`]) = move speed, 1 ([`Self::ANGULAR`]) = turn
/// speed, anything else is ignored by the client. GTA units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetVehicleVelocity {
    pub kind: u8,
    pub velocity: Vec3,
}

impl SetVehicleVelocity {
    pub const LINEAR: u8 = 0;
    pub const ANGULAR: u8 = 1;
}

impl RpcPayload for SetVehicleVelocity {
    const ID: u8 = 91;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { kind: r.read_u8()?, velocity: Vec3::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.kind);
        self.velocity.write(w);
    }
}

/// RPC 148 AttachTrailer (S→C).
///
/// Spec: vehicles.md "148 AttachTrailer". ⚠ Trailer id first, cab second.
/// Both vehicles must be streamed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachTrailer {
    pub trailer_id: u16,
    /// The cab / tractor.
    pub vehicle_id: u16,
}

impl RpcPayload for AttachTrailer {
    const ID: u8 = 148;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { trailer_id: r.read_u16()?, vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.trailer_id);
        w.write_u16(self.vehicle_id);
    }
}

/// RPC 149 DetachTrailer (S→C).
///
/// Spec: vehicles.md "149 DetachTrailer". `vehicle_id` is the **cab**; the
/// client detaches whatever trailer it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetachTrailer {
    pub vehicle_id: u16,
}

impl RpcPayload for DetachTrailer {
    const ID: u8 = 149;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
    }
}

/// RPC 159 SetVehiclePosition (S→C).
///
/// Spec: vehicles.md "159 SetVehiclePosition". Trains (537, 538, 449) use the
/// train-position script command, other vehicles a plain teleport.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetVehiclePosition {
    pub vehicle_id: u16,
    pub position: Vec3,
}

impl RpcPayload for SetVehiclePosition {
    const ID: u8 = 159;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, position: Vec3::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        self.position.write(w);
    }
}

/// RPC 160 SetVehicleZAngle (S→C).
///
/// Spec: vehicles.md "160 SetVehicleZAngle". Angle in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetVehicleZAngle {
    pub vehicle_id: u16,
    pub angle: f32,
}

impl RpcPayload for SetVehicleZAngle {
    const ID: u8 = 160;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { vehicle_id: r.read_u16()?, angle: r.read_f32()? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.vehicle_id);
        w.write_f32(self.angle);
    }
}

// ---------------------------------------------------------------------------
// Incoming dispatch
// ---------------------------------------------------------------------------

/// S→C vehicle RPCs.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    SetVehicleParams(SetVehicleParams),
    EnterVehicle(EnterVehicle),
    RemoveVehicleComponent(RemoveVehicleComponent),
    LinkVehicleToInterior(LinkVehicleToInterior),
    PutPlayerInVehicle(PutPlayerInVehicle),
    RemovePlayerFromVehicle(RemovePlayerFromVehicle),
    SetVehicleVelocity(SetVehicleVelocity),
    ScmEvent(ScmEvent),
    SetVehicleDamageStatus(SetVehicleDamageStatus),
    SetVehiclePlate(SetVehiclePlate),
    SetVehicleHealth(SetVehicleHealth),
    AttachTrailer(AttachTrailer),
    DetachTrailer(DetachTrailer),
    ExitVehicle(ExitVehicle),
    SetVehiclePosition(SetVehiclePosition),
    SetVehicleZAngle(SetVehicleZAngle),
    StreamInVehicle(StreamInVehicle),
    StreamOutVehicle(StreamOutVehicle),
}

/// Ids handled by [`decode`].
pub const INCOMING_IDS: &[u8] = &[24, 26, 57, 65, 70, 71, 91, 96, 106, 123, 147, 148, 149, 154, 159, 160, 164, 165];

/// Decode an S→C vehicle RPC payload; `None` for other ids or truncated data.
pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
    use Incoming as I;
    Some(match id {
        SetVehicleParams::ID => I::SetVehicleParams(SetVehicleParams::read(r)?),
        EnterVehicle::ID => I::EnterVehicle(EnterVehicle::read(r)?),
        RemoveVehicleComponent::ID => I::RemoveVehicleComponent(RemoveVehicleComponent::read(r)?),
        LinkVehicleToInterior::ID => I::LinkVehicleToInterior(LinkVehicleToInterior::read(r)?),
        PutPlayerInVehicle::ID => I::PutPlayerInVehicle(PutPlayerInVehicle::read(r)?),
        RemovePlayerFromVehicle::ID => I::RemovePlayerFromVehicle(RemovePlayerFromVehicle::read(r)?),
        SetVehicleVelocity::ID => I::SetVehicleVelocity(SetVehicleVelocity::read(r)?),
        ScmEvent::ID => I::ScmEvent(ScmEvent::read(r)?),
        SetVehicleDamageStatus::ID => I::SetVehicleDamageStatus(SetVehicleDamageStatus::read(r)?),
        SetVehiclePlate::ID => I::SetVehiclePlate(SetVehiclePlate::read(r)?),
        SetVehicleHealth::ID => I::SetVehicleHealth(SetVehicleHealth::read(r)?),
        AttachTrailer::ID => I::AttachTrailer(AttachTrailer::read(r)?),
        DetachTrailer::ID => I::DetachTrailer(DetachTrailer::read(r)?),
        ExitVehicle::ID => I::ExitVehicle(ExitVehicle::read(r)?),
        SetVehiclePosition::ID => I::SetVehiclePosition(SetVehiclePosition::read(r)?),
        SetVehicleZAngle::ID => I::SetVehicleZAngle(SetVehicleZAngle::read(r)?),
        StreamInVehicle::ID => I::StreamInVehicle(StreamInVehicle::read(r)?),
        StreamOutVehicle::ID => I::StreamOutVehicle(StreamOutVehicle::read(r)?),
        _ => return None,
    })
}

impl Incoming {
    /// RPC id of the payload.
    pub fn id(&self) -> u8 {
        match self {
            Self::SetVehicleParams(_) => SetVehicleParams::ID,
            Self::EnterVehicle(_) => EnterVehicle::ID,
            Self::RemoveVehicleComponent(_) => RemoveVehicleComponent::ID,
            Self::LinkVehicleToInterior(_) => LinkVehicleToInterior::ID,
            Self::PutPlayerInVehicle(_) => PutPlayerInVehicle::ID,
            Self::RemovePlayerFromVehicle(_) => RemovePlayerFromVehicle::ID,
            Self::SetVehicleVelocity(_) => SetVehicleVelocity::ID,
            Self::ScmEvent(_) => ScmEvent::ID,
            Self::SetVehicleDamageStatus(_) => SetVehicleDamageStatus::ID,
            Self::SetVehiclePlate(_) => SetVehiclePlate::ID,
            Self::SetVehicleHealth(_) => SetVehicleHealth::ID,
            Self::AttachTrailer(_) => AttachTrailer::ID,
            Self::DetachTrailer(_) => DetachTrailer::ID,
            Self::ExitVehicle(_) => ExitVehicle::ID,
            Self::SetVehiclePosition(_) => SetVehiclePosition::ID,
            Self::SetVehicleZAngle(_) => SetVehicleZAngle::ID,
            Self::StreamInVehicle(_) => StreamInVehicle::ID,
            Self::StreamOutVehicle(_) => StreamOutVehicle::ID,
        }
    }

    /// Encode the payload body (without RPC framing).
    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::SetVehicleParams(p) => p.write(w),
            Self::EnterVehicle(p) => p.write(w),
            Self::RemoveVehicleComponent(p) => p.write(w),
            Self::LinkVehicleToInterior(p) => p.write(w),
            Self::PutPlayerInVehicle(p) => p.write(w),
            Self::RemovePlayerFromVehicle(p) => p.write(w),
            Self::SetVehicleVelocity(p) => p.write(w),
            Self::ScmEvent(p) => p.write(w),
            Self::SetVehicleDamageStatus(p) => p.write(w),
            Self::SetVehiclePlate(p) => p.write(w),
            Self::SetVehicleHealth(p) => p.write(w),
            Self::AttachTrailer(p) => p.write(w),
            Self::DetachTrailer(p) => p.write(w),
            Self::ExitVehicle(p) => p.write(w),
            Self::SetVehiclePosition(p) => p.write(w),
            Self::SetVehicleZAngle(p) => p.write(w),
            Self::StreamInVehicle(p) => p.write(w),
            Self::StreamOutVehicle(p) => p.write(w),
        }
    }
}

// ---------------------------------------------------------------------------
// Outgoing (C→S) send parameters
// ---------------------------------------------------------------------------

outgoing! {
    Vehicle;
    /// RPC 26: reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0
    /// (vehicles.md "26 OnPlayerEnterVehicle (C→S)"). Mirrors samp.dll
    /// 0x10005AD0.
    OnPlayerEnterVehicle => ReliableSequenced;
    /// RPC 154: reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0
    /// (vehicles.md "154 OnPlayerExitVehicle (C→S)"). Mirrors samp.dll
    /// 0x10005BF0.
    OnPlayerExitVehicle => ReliableSequenced;
    /// RPC 106 (C→S direction): reliability 8 (RELIABLE), priority 1, channel 0
    /// (vehicles.md "106 SetVehicleDamageStatus"). Mirrors samp.dll 0x10005DE0.
    SetVehicleDamageStatus => Reliable;
    /// RPC 136: reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0
    /// (vehicles.md "136 VehicleDeath"). Mirrors samp.dll 0x1001F230.
    VehicleDeath => ReliableSequenced;
    /// RPC 96: reliability 10 (RELIABLE_SEQUENCED), priority 1, channel 0
    /// (vehicles.md "96 SCMEvent (C→S)", player-state.md). Mirrors samp.dll
    /// 0x10001CE0.
    OnPlayerScmEvent => ReliableSequenced;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of<T: RpcPayload>(p: &T) -> (usize, Vec<u8>) {
        let mut w = BitWriter::new();
        p.write(&mut w);
        (w.bit_len(), w.into_bytes())
    }

    /// Write, then read back both directly and at a 3-bit misalignment.
    fn roundtrip<T: RpcPayload + PartialEq + std::fmt::Debug>(p: &T) {
        let (bits, bytes) = bytes_of(p);
        let mut r = BitReader::with_bit_len(&bytes, bits);
        assert_eq!(T::read(&mut r).as_ref(), Some(p));
        assert_eq!(r.remaining(), 0);

        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        p.write(&mut w);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        r.read_bits(3);
        assert_eq!(T::read(&mut r).as_ref(), Some(p));
        assert_eq!(r.remaining(), 0);

        // Every strict prefix must fail to decode.
        if bits > 0 {
            let mut r = BitReader::with_bit_len(&bytes, bits - 1);
            assert_eq!(T::read(&mut r), None);
        }
    }

    /// Decode `bytes` (hand-built from the spec table) and check `expected`;
    /// also check that our writer produces the exact same bytes.
    fn exact<T: RpcPayload + PartialEq + std::fmt::Debug>(bytes: &[u8], expected: &T) {
        let mut r = BitReader::new(bytes);
        assert_eq!(T::read(&mut r).as_ref(), Some(expected));
        assert_eq!(r.remaining(), 0, "spec bytes not fully consumed");
        let (bits, written) = bytes_of(expected);
        assert_eq!(bits, bytes.len() * 8);
        assert_eq!(written, bytes);
    }

    fn sample_stream_in() -> StreamInVehicle {
        StreamInVehicle {
            vehicle_id: 0x0102,
            model: 411,
            position: Vec3 { x: 1.0, y: -2.0, z: 0.5 },
            angle: 90.0,
            colour1: 3,
            colour2: StreamInVehicle::COLOUR_DEFAULT,
            health: 1000.0,
            interior: 0,
            door_damage: 0x0403_0201,
            panel_damage: 0x0807_0605,
            light_damage: 5,
            tyre_damage: 0b1010,
            siren: 1,
            mods: [0, 81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7],
            paintjob: 2,
            body_colour1: StreamInVehicle::BODY_COLOUR_UNSET,
            body_colour2: 6,
        }
    }

    #[test]
    fn roundtrips() {
        roundtrip(&sample_stream_in());
        roundtrip(&StreamOutVehicle { vehicle_id: 1999 });
        roundtrip(&SetVehicleParams {
            vehicle_id: 7,
            params: VehicleParams { engine: 1, doors: 0, window_back_right: 1, ..Default::default() },
        });
        roundtrip(&EnterVehicle { player_id: 3, vehicle_id: 4, passenger: 1 });
        roundtrip(&OnPlayerEnterVehicle { vehicle_id: 4, passenger: 1 });
        roundtrip(&ExitVehicle { player_id: 1004, vehicle_id: 9 });
        roundtrip(&OnPlayerExitVehicle { vehicle_id: 9 });
        roundtrip(&PutPlayerInVehicle { vehicle_id: 12, seat: 2 });
        roundtrip(&RemovePlayerFromVehicle);
        roundtrip(&SetVehicleDamageStatus {
            vehicle_id: 1,
            panels: 0x1234_5678,
            doors: 0x0202_0202,
            lights: 0x45,
            tyres: 0x0F,
        });
        roundtrip(&VehicleDeath { vehicle_id: 55 });
        roundtrip(&SetVehicleHealth { vehicle_id: 2, health: 249.5 });
        roundtrip(&RemoveVehicleComponent { vehicle_id: 2, component: 1010 });
        roundtrip(&ScmEvent {
            player_id: 0xFFFF,
            event_type: scm_event_type::ADD_COMPONENT,
            vehicle_id: 3,
            arg1: 1080,
            arg2: 0,
        });
        roundtrip(&OnPlayerScmEvent { vehicle_id: 3, arg1: 1, arg2: 2, event_type: scm_event_type::COLOUR });
        roundtrip(&SetVehiclePlate { vehicle_id: 8, plate: b"SAMP 037".to_vec() });
        roundtrip(&SetVehiclePlate { vehicle_id: 8, plate: Vec::new() });
        roundtrip(&LinkVehicleToInterior { vehicle_id: 8, interior: 15 });
        roundtrip(&SetVehicleVelocity {
            kind: SetVehicleVelocity::ANGULAR,
            velocity: Vec3 { x: 0.1, y: 0.2, z: -0.3 },
        });
        roundtrip(&AttachTrailer { trailer_id: 20, vehicle_id: 21 });
        roundtrip(&DetachTrailer { vehicle_id: 21 });
        roundtrip(&SetVehiclePosition { vehicle_id: 5, position: Vec3 { x: 2000.0, y: -1500.25, z: 13.5 } });
        roundtrip(&SetVehicleZAngle { vehicle_id: 5, angle: 270.0 });
    }

    #[test]
    fn stream_in_exact() {
        // Hand-built from the vehicles.md 164 table (63 bytes).
        let mut b = Vec::new();
        b.extend_from_slice(&[0x02, 0x01]); // 0 vehicle_id 0x0102
        b.extend_from_slice(&[0x9B, 0x01, 0x00, 0x00]); // 2 model 411
        b.extend_from_slice(&[0x00, 0x00, 0x80, 0x3F]); // 6 x 1.0
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0xC0]); // 10 y -2.0
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0x3F]); // 14 z 0.5
        b.extend_from_slice(&[0x00, 0x00, 0xB4, 0x42]); // 18 angle 90.0
        b.extend_from_slice(&[0x03, 0xFF]); // 22, 23 colours
        b.extend_from_slice(&[0x00, 0x00, 0x7A, 0x44]); // 24 health 1000.0
        b.push(0x00); // 28 interior
        b.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // 29 doors (before panels)
        b.extend_from_slice(&[0x05, 0x06, 0x07, 0x08]); // 33 panels
        b.extend_from_slice(&[0x05, 0x0A, 0x01]); // 37 lights, 38 tyres, 39 siren
        b.extend_from_slice(&[0, 81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]); // 40 mods
        b.push(0x02); // 54 paintjob (+1-encoded)
        b.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]); // 55 body colour 1 unset
        b.extend_from_slice(&[0x06, 0x00, 0x00, 0x00]); // 59 body colour 2
        assert_eq!(b.len(), 63);
        let v = sample_stream_in();
        exact(&b, &v);
        assert_eq!(v.component(1), Some(1080));
        assert_eq!(v.component(13), Some(1006));
        assert_eq!(v.component(0), None);
        assert_eq!(v.component(14), None);
        assert_eq!(v.paintjob(), Some(1));
    }

    #[test]
    fn stream_in_helpers() {
        assert_eq!(StreamInVehicle::encode_component(None), Some(0));
        assert_eq!(StreamInVehicle::encode_component(Some(1000)), Some(1));
        assert_eq!(StreamInVehicle::encode_component(Some(1193)), Some(194));
        assert_eq!(StreamInVehicle::encode_component(Some(999)), None);
        assert_eq!(StreamInVehicle::encode_component(Some(1255)), None);
        assert_eq!(StreamInVehicle::encode_paintjob(None), 0);
        assert_eq!(StreamInVehicle::encode_paintjob(Some(3)), 4);
        let v = StreamInVehicle { paintjob: 0, ..Default::default() };
        assert_eq!(v.paintjob(), None);
    }

    #[test]
    fn params_exact() {
        let mut b = vec![0x07, 0x00];
        b.extend_from_slice(&[1, 0, 0xFF, 1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1]);
        let p = SetVehicleParams {
            vehicle_id: 7,
            params: VehicleParams {
                engine: 1,
                lights: 0,
                alarm: 0xFF,
                doors: 1,
                bonnet: 0,
                boot: 1,
                objective: 0,
                siren: 0,
                door_driver: 1,
                door_passenger: 0,
                door_back_left: 0,
                door_back_right: 0,
                window_driver: 0,
                window_passenger: 0,
                window_back_left: 0,
                window_back_right: 1,
            },
        };
        exact(&b, &p);
        assert_eq!(VehicleParams::default().to_bytes(), [0xFF; 16]);
    }

    #[test]
    fn damage_status_exact_panels_first() {
        let b = [0x2A, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x09, 0x03];
        exact(
            &b,
            &SetVehicleDamageStatus { vehicle_id: 42, panels: 0x4433_2211, doors: 0x8877_6655, lights: 9, tyres: 3 },
        );
    }

    #[test]
    fn scm_event_both_orders_exact() {
        // S→C: u16 player, u32 type, u32 vehicle, u32 arg1, u32 arg2.
        let s2c = [
            0x05, 0x00, 0x02, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        exact(&s2c, &ScmEvent { player_id: 5, event_type: 2, vehicle_id: 10, arg1: 1080, arg2: 0 });
        // C→S: u32 vehicle, u32 arg1, u32 arg2, u32 type.
        let c2s = [0x0A, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00];
        exact(&c2s, &OnPlayerScmEvent { vehicle_id: 10, arg1: 1, arg2: 2, event_type: 3 });
    }

    #[test]
    fn small_layouts_exact() {
        exact(&[0x03, 0x00, 0x04, 0x00, 0x01], &EnterVehicle { player_id: 3, vehicle_id: 4, passenger: 1 });
        exact(&[0x04, 0x00, 0x00], &OnPlayerEnterVehicle { vehicle_id: 4, passenger: 0 });
        exact(&[0xEC, 0x03, 0x09, 0x00], &ExitVehicle { player_id: 1004, vehicle_id: 9 });
        exact(&[0x09, 0x00], &OnPlayerExitVehicle { vehicle_id: 9 });
        exact(&[0x0C, 0x00, 0x02], &PutPlayerInVehicle { vehicle_id: 12, seat: 2 });
        exact(&[], &RemovePlayerFromVehicle);
        exact(&[0x37, 0x00], &VehicleDeath { vehicle_id: 55 });
        exact(&[0x02, 0x00, 0x00, 0x00, 0x7A, 0x44], &SetVehicleHealth { vehicle_id: 2, health: 1000.0 });
        exact(&[0x02, 0x00, 0xF2, 0x03], &RemoveVehicleComponent { vehicle_id: 2, component: 1010 });
        exact(&[0x08, 0x00, 0x0F], &LinkVehicleToInterior { vehicle_id: 8, interior: 15 });
        // 148: trailer first, then cab.
        exact(&[0x14, 0x00, 0x15, 0x00], &AttachTrailer { trailer_id: 20, vehicle_id: 21 });
        exact(&[0x15, 0x00], &DetachTrailer { vehicle_id: 21 });
        exact(&[0xCF, 0x07], &StreamOutVehicle { vehicle_id: 1999 });
        exact(&[0x05, 0x00, 0x00, 0x00, 0x87, 0x43], &SetVehicleZAngle { vehicle_id: 5, angle: 270.0 });
    }

    #[test]
    fn vec3_layouts_exact() {
        // 91: no vehicle id; u8 type then 3×f32.
        exact(
            &[0x00, 0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0],
            &SetVehicleVelocity { kind: SetVehicleVelocity::LINEAR, velocity: Vec3 { x: 1.0, y: 0.0, z: -2.0 } },
        );
        exact(
            &[0x05, 0x00, 0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40, 0x40],
            &SetVehiclePosition { vehicle_id: 5, position: Vec3 { x: 1.0, y: 2.0, z: 3.0 } },
        );
    }

    #[test]
    fn plate_exact() {
        exact(&[0x08, 0x00, 0x03, b'A', b'B', b'C'], &SetVehiclePlate { vehicle_id: 8, plate: b"ABC".to_vec() });
        // Length byte larger than the remaining data → truncated.
        let mut r = BitReader::new(&[0x08, 0x00, 0x05, b'A']);
        assert_eq!(SetVehiclePlate::read(&mut r), None);
        // Over-long plates are still decoded faithfully (the client ignores them).
        let mut b = vec![0x01, 0x00, 40];
        b.extend_from_slice(&[b'X'; 40]);
        let mut r = BitReader::new(&b);
        assert_eq!(SetVehiclePlate::read(&mut r).unwrap().plate.len(), 40);
    }

    #[test]
    fn decode_dispatch() {
        let samples: Vec<(u8, BitWriter, Incoming)> = {
            fn enc<T: RpcPayload>(p: &T) -> BitWriter {
                let mut w = BitWriter::new();
                p.write(&mut w);
                w
            }
            let si = sample_stream_in();
            let dmg = SetVehicleDamageStatus { vehicle_id: 1, panels: 2, doors: 3, lights: 4, tyres: 5 };
            let scm = ScmEvent { player_id: 1, event_type: 4, vehicle_id: 2, arg1: 1, arg2: 3 };
            let plate = SetVehiclePlate { vehicle_id: 1, plate: b"X".to_vec() };
            let params = SetVehicleParams::default();
            let vel = SetVehicleVelocity::default();
            let pos = SetVehiclePosition::default();
            vec![
                (24, enc(&params), Incoming::SetVehicleParams(params)),
                (26, enc(&EnterVehicle::default()), Incoming::EnterVehicle(EnterVehicle::default())),
                (
                    57,
                    enc(&RemoveVehicleComponent::default()),
                    Incoming::RemoveVehicleComponent(RemoveVehicleComponent::default()),
                ),
                (
                    65,
                    enc(&LinkVehicleToInterior::default()),
                    Incoming::LinkVehicleToInterior(LinkVehicleToInterior::default()),
                ),
                (70, enc(&PutPlayerInVehicle::default()), Incoming::PutPlayerInVehicle(PutPlayerInVehicle::default())),
                (71, enc(&RemovePlayerFromVehicle), Incoming::RemovePlayerFromVehicle(RemovePlayerFromVehicle)),
                (91, enc(&vel), Incoming::SetVehicleVelocity(vel)),
                (96, enc(&scm), Incoming::ScmEvent(scm)),
                (106, enc(&dmg), Incoming::SetVehicleDamageStatus(dmg)),
                (123, enc(&plate), Incoming::SetVehiclePlate(plate.clone())),
                (147, enc(&SetVehicleHealth::default()), Incoming::SetVehicleHealth(SetVehicleHealth::default())),
                (148, enc(&AttachTrailer::default()), Incoming::AttachTrailer(AttachTrailer::default())),
                (149, enc(&DetachTrailer::default()), Incoming::DetachTrailer(DetachTrailer::default())),
                (154, enc(&ExitVehicle::default()), Incoming::ExitVehicle(ExitVehicle::default())),
                (159, enc(&pos), Incoming::SetVehiclePosition(pos)),
                (160, enc(&SetVehicleZAngle::default()), Incoming::SetVehicleZAngle(SetVehicleZAngle::default())),
                (164, enc(&si), Incoming::StreamInVehicle(si.clone())),
                (165, enc(&StreamOutVehicle::default()), Incoming::StreamOutVehicle(StreamOutVehicle::default())),
            ]
        };
        let ids: Vec<u8> = samples.iter().map(|s| s.0).collect();
        assert_eq!(ids, INCOMING_IDS);
        for (id, w, expected) in samples {
            let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
            assert_eq!(decode(id, &mut r), Some(expected), "id {id}");
        }
        // C→S-only id and unrelated ids are not decoded.
        assert_eq!(decode(VehicleDeath::ID, &mut BitReader::new(&[0, 0])), None);
        assert_eq!(decode(93, &mut BitReader::new(&[0; 64])), None);
        // Truncated payload.
        assert_eq!(decode(164, &mut BitReader::new(&[0; 62])), None);
    }

    #[test]
    fn through_rpc_envelope() {
        let p = StreamInVehicle { vehicle_id: 42, ..sample_stream_in() };
        let w = p.to_rpc();
        let rpc = crate::rpc::decode(w.as_bytes(), w.bit_len()).unwrap();
        assert_eq!(rpc.id, 164);
        assert_eq!(rpc.bit_len, 63 * 8);
        assert_eq!(decode(rpc.id, &mut rpc.reader()), Some(Incoming::StreamInVehicle(p)));

        let w = RemovePlayerFromVehicle.to_rpc();
        let rpc = crate::rpc::decode(w.as_bytes(), w.bit_len()).unwrap();
        assert_eq!((rpc.id, rpc.bit_len), (71, 0));
        assert_eq!(decode(rpc.id, &mut rpc.reader()), Some(Incoming::RemovePlayerFromVehicle(RemovePlayerFromVehicle)));
    }
}
