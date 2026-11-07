//! World entities: objects (incl. select/edit), player-attached objects, pickups,
//! actors, checkpoints, gang zones, map icons, 3D text labels and the
//! custom-model download RPCs.
//!
//! Spec: docs/re/subsystems/world-entities.md (custom-model sub-messages also in
//! docs/re/subsystems/session-misc.md "Custom-model download machinery").
//! Colours are kept as the raw `u32` that is on the wire; the byte order open.mp
//! uses for each one is noted on the field.

use super::RpcPayload;
use super::wire::Wire;
use crate::bitstream::{BitReader, BitWriter};
use crate::strcomp::{read_compressed_string, write_compressed_string};
use crate::types::{Vec2, Vec3, read_str8, write_str8};

/// "No entity" value for 16-bit vehicle/object/player ids (attachments).
pub const INVALID_ID: u16 = 0xFFFF;
/// Receive-buffer limit of compressed strings (material text, 3D label text).
pub const CSTR_MAX_LEN: usize = 2048;

/// Declares a payload struct whose fields are all fixed-layout [`Wire`] fields,
/// encoded in declaration order. The unit form declares an empty payload.
macro_rules! payload {
    ($(#[$m:meta])* $name:ident = $id:expr;) => {
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
    ($(#[$m:meta])* $name:ident = $id:expr; { $($(#[$fm:meta])* $f:ident : $t:ty),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $($(#[$fm])* pub $f: $t,)+
        }
        impl RpcPayload for $name {
            const ID: u8 = $id;
            fn read(r: &mut BitReader) -> Option<Self> {
                Some(Self { $($f: <$t as Wire>::get(r)?,)+ })
            }
            fn write(&self, w: &mut BitWriter) {
                $(Wire::put(&self.$f, w);)+
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

/// One material record (shared by RPC 44 and RPC 84).
///
/// Spec: world-entities.md §44 "Material record". Wire: `u8 kind`, then for the
/// known kinds `u8 slot` and the kind's fields. For any other kind the client
/// reads nothing more for that record (later records then misparse), which is
/// what [`ObjectMaterial::Unknown`] models.
///
/// Client behaviour (not applied by the decoder): `txd`, `texture` and `font`
/// longer than 31 bytes make the client skip the record; a text record whose
/// text decodes empty is skipped; the client accepts slot < 17 against 16-entry
/// arrays (off by one) — a Rust client should accept only 0…15.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectMaterial {
    /// kind 1: texture replacement.
    Texture {
        slot: u8,
        /// Model whose TXD holds the texture. 0xFFFF → −1 (colour only); in
        /// RPC 44 the client also maps any value > 20000 to −1 (RPC 84 does
        /// not). 0 selects the built-in blank texture.
        model: u16,
        /// TXD name (dynstr8).
        txd: Vec<u8>,
        /// Texture name (dynstr8).
        texture: Vec<u8>,
        /// open.mp writes ABGR (memory bytes R, G, B, A).
        colour: u32,
    },
    /// kind 2: material text.
    Text {
        slot: u8,
        material_size: u8,
        /// Font name (dynstr8).
        font: Vec<u8>,
        font_size: u8,
        bold: u8,
        /// open.mp writes ARGB.
        font_colour: u32,
        /// open.mp writes ARGB.
        background_colour: u32,
        alignment: u8,
        /// Compressed string, receive limit [`CSTR_MAX_LEN`].
        text: Vec<u8>,
    },
    /// Any other kind byte: no further fields were read.
    Unknown { kind: u8 },
}

impl ObjectMaterial {
    pub const KIND_TEXTURE: u8 = 1;
    pub const KIND_TEXT: u8 = 2;

    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(match r.read_u8()? {
            Self::KIND_TEXTURE => Self::Texture {
                slot: r.read_u8()?,
                model: r.read_u16()?,
                txd: read_str8(r)?,
                texture: read_str8(r)?,
                colour: r.read_u32()?,
            },
            Self::KIND_TEXT => Self::Text {
                slot: r.read_u8()?,
                material_size: r.read_u8()?,
                font: read_str8(r)?,
                font_size: r.read_u8()?,
                bold: r.read_u8()?,
                font_colour: r.read_u32()?,
                background_colour: r.read_u32()?,
                alignment: r.read_u8()?,
                text: read_compressed_string(r, CSTR_MAX_LEN)?,
            },
            kind => Self::Unknown { kind },
        })
    }

    pub fn write(&self, w: &mut BitWriter) {
        match self {
            Self::Texture { slot, model, txd, texture, colour } => {
                w.write_u8(Self::KIND_TEXTURE);
                w.write_u8(*slot);
                w.write_u16(*model);
                write_str8(w, txd);
                write_str8(w, texture);
                w.write_u32(*colour);
            }
            Self::Text {
                slot,
                material_size,
                font,
                font_size,
                bold,
                font_colour,
                background_colour,
                alignment,
                text,
            } => {
                w.write_u8(Self::KIND_TEXT);
                w.write_u8(*slot);
                w.write_u8(*material_size);
                write_str8(w, font);
                w.write_u8(*font_size);
                w.write_u8(*bold);
                w.write_u32(*font_colour);
                w.write_u32(*background_colour);
                w.write_u8(*alignment);
                write_compressed_string(w, text);
            }
            Self::Unknown { kind } => w.write_u8(*kind),
        }
    }
}

/// RPC 44 CreateObject (S→C).
///
/// Spec: world-entities.md §44. Layout: `u16 id, i32 model, vec3 position,
/// vec3 rotation (degrees), f32 draw distance, u8 no-camera-collision,
/// u16 attached vehicle, u16 attached object, [vec3 offset, vec3 rotation,
/// u8 sync rotation] (only if either attachment id ≠ 0xFFFF), u8 material
/// count, material records`.
///
/// Client behaviour: negative models are custom models (inactive in R4, become
/// 18631); a model without model info also becomes 18631; draw distance 0 keeps
/// the model default; a vehicle attachment wins over an object attachment and
/// both are applied later per frame; materials apply only if the object was
/// created. Valid object ids are 0…999 (the client accepts 1000, off by one).
/// Matches open.mp, which writes `!CameraCollision` into `no_camera_collision`.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateObject {
    pub object_id: u16,
    pub model: i32,
    pub position: Vec3,
    /// Degrees.
    pub rotation: Vec3,
    /// 0 = keep the model's default.
    pub draw_distance: f32,
    /// Non-zero → the camera-collision test ignores the object (as RPC 169).
    pub no_camera_collision: u8,
    /// [`INVALID_ID`] = none.
    pub attached_vehicle_id: u16,
    /// [`INVALID_ID`] = none.
    pub attached_object_id: u16,
    /// Only on the wire when attached (see [`CreateObject::is_attached`]).
    pub attach_offset: Vec3,
    /// Only on the wire when attached.
    pub attach_rotation: Vec3,
    /// Only on the wire when attached; used for object-to-object attachment.
    pub sync_rotation: u8,
    /// Written with a u8 count; at most 255 records are encoded.
    pub materials: Vec<ObjectMaterial>,
}

impl Default for CreateObject {
    fn default() -> Self {
        Self {
            object_id: 0,
            model: 0,
            position: Vec3::default(),
            rotation: Vec3::default(),
            draw_distance: 0.0,
            no_camera_collision: 0,
            attached_vehicle_id: INVALID_ID,
            attached_object_id: INVALID_ID,
            attach_offset: Vec3::default(),
            attach_rotation: Vec3::default(),
            sync_rotation: 0,
            materials: Vec::new(),
        }
    }
}

impl CreateObject {
    /// Whether the attachment block is present on the wire.
    pub fn is_attached(&self) -> bool {
        self.attached_vehicle_id != INVALID_ID || self.attached_object_id != INVALID_ID
    }
}

impl RpcPayload for CreateObject {
    const ID: u8 = 44;
    fn read(r: &mut BitReader) -> Option<Self> {
        let mut o = Self {
            object_id: r.read_u16()?,
            model: i32::get(r)?,
            position: Vec3::read(r)?,
            rotation: Vec3::read(r)?,
            draw_distance: r.read_f32()?,
            no_camera_collision: r.read_u8()?,
            attached_vehicle_id: r.read_u16()?,
            attached_object_id: r.read_u16()?,
            ..Self::default()
        };
        if o.is_attached() {
            o.attach_offset = Vec3::read(r)?;
            o.attach_rotation = Vec3::read(r)?;
            o.sync_rotation = r.read_u8()?;
        }
        let count = r.read_u8()?;
        o.materials = (0..count).map(|_| ObjectMaterial::read(r)).collect::<Option<_>>()?;
        Some(o)
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.object_id);
        self.model.put(w);
        self.position.write(w);
        self.rotation.write(w);
        w.write_f32(self.draw_distance);
        w.write_u8(self.no_camera_collision);
        w.write_u16(self.attached_vehicle_id);
        w.write_u16(self.attached_object_id);
        if self.is_attached() {
            self.attach_offset.write(w);
            self.attach_rotation.write(w);
            w.write_u8(self.sync_rotation);
        }
        let materials = &self.materials[..self.materials.len().min(u8::MAX.into())];
        w.write_u8(materials.len() as u8);
        for m in materials {
            m.write(w);
        }
    }
}

payload! {
    /// RPC 45 SetObjectPosition (S→C).
    ///
    /// Spec: world-entities.md §45. Layout: `u16 id, vec3 position`.
    ///
    /// Disagreement (harmless): the R4 client attempts to read a 4th f32 after
    /// the position and never uses it; open.mp does not send it. This decoder
    /// reads id + vec3 and ignores trailing data; the writer emits what open.mp
    /// sends.
    SetObjectPosition = 45; {
        object_id: u16,
        position: Vec3,
    }
}

payload! {
    /// RPC 46 SetObjectRotation (S→C).
    ///
    /// Spec: world-entities.md §46. Layout: `u16 id, vec3 rotation (degrees)`.
    SetObjectRotation = 46; {
        object_id: u16,
        /// Degrees.
        rotation: Vec3,
    }
}

payload! {
    /// RPC 47 DestroyObject (S→C).
    ///
    /// Spec: world-entities.md §47. Layout: `u16 id`. The client also clears its
    /// local "surfing/contact object" reference if it points to this object.
    DestroyObject = 47; {
        object_id: u16,
    }
}

payload! {
    /// RPC 99 MoveObject (S→C).
    ///
    /// Spec: world-entities.md §99. Layout: `u16 id, vec3 current position,
    /// vec3 target position, f32 speed, vec3 target rotation`.
    ///
    /// Client behaviour: ignored while the object is open in the object editor;
    /// `current_position` is read but not used by R4 (no teleport); a target
    /// rotation component ≤ −999 means "do not rotate" (open.mp sends
    /// [`MoveObject::NO_ROTATION`]); rotation is applied only if all three
    /// components are > −999.
    MoveObject = 99; {
        object_id: u16,
        /// Unused by the R4 client.
        current_position: Vec3,
        target_position: Vec3,
        /// Units per second.
        speed: f32,
        target_rotation: Vec3,
    }
}

impl MoveObject {
    /// Rotation component value open.mp sends for "do not rotate".
    pub const NO_ROTATION: f32 = -1000.0;
}

payload! {
    /// RPC 122 StopObject (S→C).
    ///
    /// Spec: world-entities.md §122. Layout: `u16 id`. Stops the motion; the
    /// object stays where it is.
    StopObject = 122; {
        object_id: u16,
    }
}

payload! {
    /// RPC 75 AttachObjectToPlayer (S→C).
    ///
    /// Spec: world-entities.md §75. Layout: `u16 object id, u16 player id,
    /// vec3 offset, vec3 rotation`. Applied immediately; unknown object or
    /// player → no effect.
    AttachObjectToPlayer = 75; {
        object_id: u16,
        player_id: u16,
        offset: Vec3,
        rotation: Vec3,
    }
}

/// RPC 84 SetPlayerObjectMaterial (S→C).
///
/// Spec: world-entities.md §84. Layout: `u16 object id`, then exactly one
/// material record (see [`ObjectMaterial`]). The client reads the record even
/// when the object id is invalid. Unlike RPC 44 there is no "model > 20000 →
/// −1" rule, only 0xFFFF → −1. Matches open.mp.
#[derive(Clone, Debug, PartialEq)]
pub struct SetPlayerObjectMaterial {
    pub object_id: u16,
    pub material: ObjectMaterial,
}

impl RpcPayload for SetPlayerObjectMaterial {
    const ID: u8 = 84;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self { object_id: r.read_u16()?, material: ObjectMaterial::read(r)? })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.object_id);
        self.material.write(w);
    }
}

payload! {
    /// RPC 169 object no-camera-collision (S→C; no open.mp struct).
    ///
    /// Spec: world-entities.md §169 and session-misc.md §169. Layout: `u16 id`.
    /// Sets the same flag as [`CreateObject::no_camera_collision`]; it can never
    /// be cleared. open.mp never sends it (it uses the CreateObject byte).
    SetObjectNoCameraCollision = 169; {
        object_id: u16,
    }
}

/// The 52-byte attached-object slot record used by RPC 113 and RPC 116 (C→S).
///
/// Spec: world-entities.md §113. Layout: `i32 model, i32 bone, vec3 offset,
/// vec3 rotation, vec3 scale, u32 colour1, u32 colour2` (the client moves it as
/// one raw 52-byte block; the bit layout is identical to the separate fields).
///
/// Colours: open.mp writes ABGR in RPC 113 and reads ARGB in RPC 116; the
/// client echoes the stored bytes verbatim (open.mp-internal inconsistency).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AttachedObjectData {
    pub model: i32,
    /// The client accepts bones 1…18 only.
    pub bone: i32,
    pub offset: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    /// Non-zero → colour of material slot 0.
    pub colour1: u32,
    /// Non-zero → colour of material slot 1.
    pub colour2: u32,
}

impl AttachedObjectData {
    /// Size of the record on the wire, in bytes.
    pub const SIZE: usize = 52;

    pub fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            model: i32::get(r)?,
            bone: i32::get(r)?,
            offset: Vec3::read(r)?,
            rotation: Vec3::read(r)?,
            scale: Vec3::read(r)?,
            colour1: r.read_u32()?,
            colour2: r.read_u32()?,
        })
    }
    pub fn write(&self, w: &mut BitWriter) {
        self.model.put(w);
        self.bone.put(w);
        self.offset.write(w);
        self.rotation.write(w);
        self.scale.write(w);
        w.write_u32(self.colour1);
        w.write_u32(self.colour2);
    }
}

/// RPC 113 SetPlayerAttachedObject (S→C).
///
/// Spec: world-entities.md §113. Layout: `u16 player id, u32 slot index,
/// bit create, [52-byte slot record if create]`. `data` is `Some` exactly when
/// the create bit is set.
///
/// Client behaviour: the player is the local player or a remote player ≤ 1003
/// with a ped. create = 0 removes slot `index` (if 0…9 and used); create = 1
/// requires index 0…9 and bone 1…18 (otherwise ignored) and replaces any
/// existing slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetPlayerAttachedObject {
    pub player_id: u16,
    /// 0…9.
    pub index: u32,
    pub data: Option<AttachedObjectData>,
}

impl RpcPayload for SetPlayerAttachedObject {
    const ID: u8 = 113;
    fn read(r: &mut BitReader) -> Option<Self> {
        let player_id = r.read_u16()?;
        let index = r.read_u32()?;
        let data = if r.read_bit()? { Some(AttachedObjectData::read(r)?) } else { None };
        Some(Self { player_id, index, data })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.player_id);
        w.write_u32(self.index);
        w.write_bit(self.data.is_some());
        if let Some(d) = &self.data {
            d.write(w);
        }
    }
}

payload! {
    /// RPC 27 PlayerBeginObjectSelect (S→C). Empty payload.
    ///
    /// Spec: world-entities.md §27. Leaves textdraw selection, cancels the object
    /// editor (which sends a cancel 117/116) and enters object-selection mode.
    /// Clicking an object (closer than ~150 units) sends [`OnPlayerSelectObject`];
    /// the mode only ends with RPC 28.
    PlayerBeginObjectSelect = 27;
}

payload! {
    /// RPC 27 OnPlayerSelectObject (C→S), reliability RELIABLE_ORDERED.
    ///
    /// Spec: world-entities.md §27 (C→S). Layout: `u32 select type, u16 object
    /// id, u32 model, vec3 position`.
    ///
    /// The R4 client always writes `select_type` = 0 (open.mp names the field
    /// SelectType but ignores it and resolves the object by id and model, global
    /// first, then per-player). Global and player objects share one id space.
    OnPlayerSelectObject = 27; {
        /// Always 0 from the R4 client.
        select_type: u32,
        object_id: u16,
        model: u32,
        /// The object's current matrix position.
        position: Vec3,
    }
}

payload! {
    /// RPC 28 PlayerCancelObjectEdit (S→C). Empty payload.
    ///
    /// Spec: world-entities.md §28. Ends the object editor (sending a final
    /// 117/116 with response 0 = cancel) and/or leaves object-selection mode.
    PlayerCancelObjectEdit = 28;
}

payload! {
    /// RPC 117 PlayerBeginObjectEdit (S→C).
    ///
    /// Spec: world-entities.md §117. Layout: `bit player object, u16 object id`.
    /// Leaves selection modes and opens the editor on the object; the bit is
    /// echoed back in [`OnPlayerEditObject`].
    PlayerBeginObjectEdit = 117; {
        player_object: bool,
        object_id: u16,
    }
}

payload! {
    /// RPC 117 OnPlayerEditObject (C→S), reliability RELIABLE_ORDERED.
    ///
    /// Spec: world-entities.md §117 (C→S). Layout: `bit player object, u16
    /// object id, u32 response, vec3 position, vec3 rotation`.
    ///
    /// Responses: [`OnPlayerEditObject::RESPONSE_CANCEL`] (0),
    /// [`OnPlayerEditObject::RESPONSE_FINAL`] (1),
    /// [`OnPlayerEditObject::RESPONSE_UPDATE`] (2). The client sends it only
    /// while the object exists, and rate-limits updates to one per 250 ms.
    OnPlayerEditObject = 117; {
        /// Echo of the bit from [`PlayerBeginObjectEdit`].
        player_object: bool,
        object_id: u16,
        response: u32,
        /// Object matrix position.
        position: Vec3,
        /// Editor's rotation copy (degrees).
        rotation: Vec3,
    }
}

impl OnPlayerEditObject {
    pub const RESPONSE_CANCEL: u32 = 0;
    pub const RESPONSE_FINAL: u32 = 1;
    pub const RESPONSE_UPDATE: u32 = 2;
}

payload! {
    /// RPC 116 PlayerBeginAttachedObjectEdit (S→C).
    ///
    /// Spec: world-entities.md §116. Layout: `u32 slot index` (not validated by
    /// the client here). Opens the editor on the local player's attached slot.
    PlayerBeginAttachedObjectEdit = 116; {
        index: u32,
    }
}

/// RPC 116 OnPlayerEditAttachedObject (C→S), reliability RELIABLE_ORDERED.
///
/// Spec: world-entities.md §116 (C→S). Layout: `u32 response, u32 slot index,
/// [52-byte slot record]`. Only final (1) or cancel (0) are sent; no periodic
/// updates in this mode.
///
/// Disagreement: when the index is outside 0…9 or the slot is unused, the R4
/// client omits the 52-byte block (8-byte payload), and open.mp's reader then
/// fails. This type follows the client: `data` = `None` writes the short form,
/// and the reader accepts both forms (`None` when nothing follows the index).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnPlayerEditAttachedObject {
    /// 0 cancel, 1 final.
    pub response: u32,
    pub index: u32,
    pub data: Option<AttachedObjectData>,
}

impl OnPlayerEditAttachedObject {
    pub const RESPONSE_CANCEL: u32 = 0;
    pub const RESPONSE_FINAL: u32 = 1;
}

impl RpcPayload for OnPlayerEditAttachedObject {
    const ID: u8 = 116;
    fn read(r: &mut BitReader) -> Option<Self> {
        let response = r.read_u32()?;
        let index = r.read_u32()?;
        let data = if r.remaining() == 0 { None } else { Some(AttachedObjectData::read(r)?) };
        Some(Self { response, index, data })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u32(self.response);
        w.write_u32(self.index);
        if let Some(d) = &self.data {
            d.write(w);
        }
    }
}

// ---------------------------------------------------------------------------
// Pickups
// ---------------------------------------------------------------------------

payload! {
    /// RPC 95 PlayerCreatePickup (S→C).
    ///
    /// Spec: world-entities.md §95. Layout: `i32 id, i32 model, i32 type,
    /// vec3 position` (the client reads the last 20 bytes as one raw block; same
    /// bits). Client: id 0…4095, total count < 4096; an existing pickup in the
    /// slot is replaced; an invalid model becomes 18631.
    PlayerCreatePickup = 95; {
        pickup_id: i32,
        model: i32,
        pickup_type: i32,
        position: Vec3,
    }
}

payload! {
    /// RPC 63 PlayerDestroyPickup (S→C).
    ///
    /// Spec: world-entities.md §63. Layout: `i32 id`.
    PlayerDestroyPickup = 63; {
        pickup_id: i32,
    }
}

payload! {
    /// RPC 131 OnPlayerPickUpPickup (C→S).
    ///
    /// Spec: world-entities.md §131. Layout: `i32 id`. Sent from a GTA collision
    /// hook (RELIABLE_ORDERED, with a 15-tick per-pickup cooldown) or from a
    /// poll of type-14 pickups (RELIABLE_SEQUENCED).
    OnPlayerPickUpPickup = 131; {
        pickup_id: i32,
    }
}

// ---------------------------------------------------------------------------
// Actors
// ---------------------------------------------------------------------------

payload! {
    /// RPC 171 ShowActorForPlayer (S→C).
    ///
    /// Spec: world-entities.md §171. Layout (27 bytes, read as one block):
    /// `u16 id, u32 skin, vec3 position, f32 angle (degrees), f32 health,
    /// u8 invulnerable`. Client: id < 1000; an existing actor is replaced; the
    /// ped is created at z − 1.0. open.mp's DL variant adds a custom-skin u32
    /// that an R4 client never receives.
    ShowActorForPlayer = 171; {
        actor_id: u16,
        skin: u32,
        position: Vec3,
        /// Degrees.
        angle: f32,
        health: f32,
        invulnerable: u8,
    }
}

payload! {
    /// RPC 172 HideActorForPlayer (S→C).
    ///
    /// Spec: world-entities.md §172. Layout: `u16 id` (client: < 1000).
    HideActorForPlayer = 172; {
        actor_id: u16,
    }
}

/// RPC 173 ApplyActorAnimationForPlayer (S→C).
///
/// Spec: world-entities.md §173. Layout: `u16 id, dynstr8 library, dynstr8
/// name, f32 delta, bit loop, bit lock X, bit lock Y, bit freeze, u32 time`.
/// Client: the library "SEX" (case-insensitive) is refused; the library is
/// loaded on demand (giving up after ~15 ms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyActorAnimationForPlayer {
    pub actor_id: u16,
    pub library: Vec<u8>,
    pub name: Vec<u8>,
    pub delta: f32,
    pub loop_anim: bool,
    pub lock_x: bool,
    pub lock_y: bool,
    pub freeze: bool,
    pub time: u32,
}

impl RpcPayload for ApplyActorAnimationForPlayer {
    const ID: u8 = 173;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            actor_id: r.read_u16()?,
            library: read_str8(r)?,
            name: read_str8(r)?,
            delta: r.read_f32()?,
            loop_anim: r.read_bit()?,
            lock_x: r.read_bit()?,
            lock_y: r.read_bit()?,
            freeze: r.read_bit()?,
            time: r.read_u32()?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.actor_id);
        write_str8(w, &self.library);
        write_str8(w, &self.name);
        w.write_f32(self.delta);
        w.write_bit(self.loop_anim);
        w.write_bit(self.lock_x);
        w.write_bit(self.lock_y);
        w.write_bit(self.freeze);
        w.write_u32(self.time);
    }
}

payload! {
    /// RPC 174 ClearActorAnimationsForPlayer (S→C).
    ///
    /// Spec: world-entities.md §174. Layout: `u16 id`.
    ClearActorAnimationsForPlayer = 174; {
        actor_id: u16,
    }
}

payload! {
    /// RPC 175 SetActorFacingAngleForPlayer (S→C).
    ///
    /// Spec: world-entities.md §175. Layout: `u16 id, f32 angle (degrees)`.
    /// The client sets the ped's target heading only.
    SetActorFacingAngleForPlayer = 175; {
        actor_id: u16,
        /// Degrees.
        angle: f32,
    }
}

payload! {
    /// RPC 176 SetActorPosForPlayer (S→C).
    ///
    /// Spec: world-entities.md §176. Layout: `u16 id, vec3 position`.
    SetActorPosForPlayer = 176; {
        actor_id: u16,
        position: Vec3,
    }
}

payload! {
    /// RPC 178 SetActorHealthForPlayer (S→C).
    ///
    /// Spec: world-entities.md §178. Layout: `u16 id, f32 health`. Health ≤ 0
    /// kills the actor ped.
    SetActorHealthForPlayer = 178; {
        actor_id: u16,
        health: f32,
    }
}

payload! {
    /// RPC 177 OnPlayerDamageActor (C→S), reliability RELIABLE_ORDERED.
    ///
    /// Spec: world-entities.md §177. Layout: `bit unknown, u16 actor id,
    /// f32 damage, u32 weapon, u32 body part`. The R4 client always writes the
    /// leading bit as 0. Sent from a GTA damage hook when the local ped damages
    /// an actor.
    OnPlayerDamageActor = 177; {
        /// Always `false` from the R4 client.
        unknown: bool,
        actor_id: u16,
        damage: f32,
        weapon: u32,
        body_part: u32,
    }
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

payload! {
    /// RPC 107 SetCheckpoint (S→C).
    ///
    /// Spec: world-entities.md §107. Layout: `vec3 position, f32 size`. The
    /// client never reports entering/leaving; the server derives it from sync.
    SetCheckpoint = 107; {
        position: Vec3,
        size: f32,
    }
}

payload! {
    /// RPC 37 DisableCheckpoint (S→C). Empty payload.
    ///
    /// Spec: world-entities.md §37.
    DisableCheckpoint = 37;
}

payload! {
    /// RPC 38 SetRaceCheckpoint (S→C).
    ///
    /// Spec: world-entities.md §38. Layout: `u8 type, vec3 position, vec3 next
    /// position, f32 size`.
    SetRaceCheckpoint = 38; {
        checkpoint_type: u8,
        position: Vec3,
        next_position: Vec3,
        size: f32,
    }
}

payload! {
    /// RPC 39 DisableRaceCheckpoint (S→C). Empty payload.
    ///
    /// Spec: world-entities.md §39.
    DisableRaceCheckpoint = 39;
}

// ---------------------------------------------------------------------------
// Gang zones
// ---------------------------------------------------------------------------

payload! {
    /// RPC 108 ShowGangZone (S→C).
    ///
    /// Spec: world-entities.md §108. Layout: `u16 id, f32 min X, f32 min Y,
    /// f32 max X, f32 max Y, u32 colour`. open.mp writes the id as i16 (same
    /// bits) and the colour as ABGR.
    ///
    /// The R4 client does not bounds-check the id (≥ 1024 corrupts memory); a
    /// Rust client must reject ids ≥ 1024. Same for 120/121/85.
    ShowGangZone = 108; {
        zone_id: u16,
        min: Vec2,
        max: Vec2,
        /// ABGR (memory bytes R, G, B, A).
        colour: u32,
    }
}

payload! {
    /// RPC 120 HideGangZone (S→C).
    ///
    /// Spec: world-entities.md §120. Layout: `u16 id` (unchecked by the client).
    HideGangZone = 120; {
        zone_id: u16,
    }
}

payload! {
    /// RPC 121 FlashGangZone (S→C).
    ///
    /// Spec: world-entities.md §121. Layout: `u16 id, u32 flash colour (ABGR)`.
    /// No effect on an empty slot.
    FlashGangZone = 121; {
        zone_id: u16,
        /// ABGR.
        colour: u32,
    }
}

payload! {
    /// RPC 85 StopFlashGangZone (S→C).
    ///
    /// Spec: world-entities.md §85. Layout: `u16 id`.
    StopFlashGangZone = 85; {
        zone_id: u16,
    }
}

// ---------------------------------------------------------------------------
// Map icons
// ---------------------------------------------------------------------------

payload! {
    /// RPC 56 SetPlayerMapIcon (S→C).
    ///
    /// Spec: world-entities.md §56. Layout: `u8 id, vec3 position, u8 type,
    /// u32 colour (RGBA), u8 style`. Client: id < 100 else ignored; the colour
    /// is used only for type 0; style 0…3 selects the blip kind (others behave
    /// as 0).
    SetPlayerMapIcon = 56; {
        icon_id: u8,
        position: Vec3,
        /// Radar sprite id (0 = plain coloured square).
        icon_type: u8,
        /// RGBA.
        colour: u32,
        style: u8,
    }
}

payload! {
    /// RPC 144 RemovePlayerMapIcon (S→C).
    ///
    /// Spec: world-entities.md §144. Layout: `u8 id` (client: < 100).
    RemovePlayerMapIcon = 144; {
        icon_id: u8,
    }
}

// ---------------------------------------------------------------------------
// 3D text labels
// ---------------------------------------------------------------------------

/// RPC 36 PlayerShowTextLabel (S→C).
///
/// Spec: world-entities.md §36. Layout: `u16 id, u32 colour (RGBA), vec3
/// position, f32 draw distance, u8 test LOS, u16 attached player, u16 attached
/// vehicle, cstr text (limit 2048)`.
///
/// Client: id < 2048 (open.mp sends per-player labels as 1024 + id); the colour
/// is rotated into ARGB; the position is an offset when attached (player head
/// bone, or the vehicle matrix); player attachment wins over vehicle.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerShowTextLabel {
    pub label_id: u16,
    /// RGBA.
    pub colour: u32,
    pub position: Vec3,
    pub draw_distance: f32,
    pub test_los: u8,
    /// [`INVALID_ID`] = none.
    pub attached_player_id: u16,
    /// [`INVALID_ID`] = none.
    pub attached_vehicle_id: u16,
    pub text: Vec<u8>,
}

impl Default for PlayerShowTextLabel {
    fn default() -> Self {
        Self {
            label_id: 0,
            colour: 0,
            position: Vec3::default(),
            draw_distance: 0.0,
            test_los: 0,
            attached_player_id: INVALID_ID,
            attached_vehicle_id: INVALID_ID,
            text: Vec::new(),
        }
    }
}

impl RpcPayload for PlayerShowTextLabel {
    const ID: u8 = 36;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(Self {
            label_id: r.read_u16()?,
            colour: r.read_u32()?,
            position: Vec3::read(r)?,
            draw_distance: r.read_f32()?,
            test_los: r.read_u8()?,
            attached_player_id: r.read_u16()?,
            attached_vehicle_id: r.read_u16()?,
            text: read_compressed_string(r, CSTR_MAX_LEN)?,
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u16(self.label_id);
        w.write_u32(self.colour);
        self.position.write(w);
        w.write_f32(self.draw_distance);
        w.write_u8(self.test_los);
        w.write_u16(self.attached_player_id);
        w.write_u16(self.attached_vehicle_id);
        write_compressed_string(w, &self.text);
    }
}

payload! {
    /// RPC 58 PlayerHideTextLabel (S→C).
    ///
    /// Spec: world-entities.md §58. Layout: `u16 id` (client: < 2048).
    PlayerHideTextLabel = 58; {
        label_id: u16,
    }
}

// ---------------------------------------------------------------------------
// Custom models (dormant in 0.3.7-R4)
// ---------------------------------------------------------------------------

payload! {
    /// RPC 179 ModelRequest (S→C) — dormant in R4.
    ///
    /// Spec: world-entities.md "Custom models", session-misc.md §179. Layout:
    /// `u32 pool id, u32 total count, u8 type, i32 virtual world, i32 base id,
    /// i32 new id, u32 DFF checksum, u32 TXD checksum, u32 DFF size, u32 TXD
    /// size, u8 time on, u8 time off`. Matches open.mp.
    ///
    /// The R4 client has a handler but never registers it (see
    /// [`DORMANT_IN_R4`]); later client versions use it.
    ModelRequest = 179; {
        pool_id: u32,
        /// Total number of entries the server will send.
        count: u32,
        model_type: u8,
        /// −1 = all worlds.
        virtual_world: i32,
        base_id: i32,
        new_id: i32,
        dff_checksum: u32,
        txd_checksum: u32,
        dff_size: u32,
        txd_size: u32,
        time_on: u8,
        time_off: u8,
    }
}

/// RPC 183 ModelUrl and related download sub-messages (S→C) — dormant in R4.
///
/// Spec: world-entities.md "Custom models", session-misc.md §183. The payload
/// starts with a `u8` sub-type; open.mp only sends sub-type 6
/// ([`ModelUrl::Redirect`]). The meanings of the other sub-types are partly
/// inferred (see the variants). Unknown sub-types carry no further fields (the
/// dormant R4 handler reads nothing more for them).
#[derive(Clone, Debug, PartialEq)]
pub enum ModelUrl {
    /// Sub-type 0: reset the download state (no fields).
    Reset,
    /// Sub-type 1: `u8, u32, i32 length (0…4096), bytes` — an in-band file chunk
    /// (inferred). The client reads the bytes only when the length is in
    /// 0…4096; this decoder treats any other length as malformed. The writer
    /// encodes at most 4096 bytes.
    FileChunk { file_type: u8, checksum: u32, data: Vec<u8> },
    /// Sub-type 2: no fields (meaning not analysed).
    SubType2,
    /// Sub-type 5: `u8 file type, u32 checksum` — "can't fill download info
    /// for CRC".
    NoDownloadInfo { file_type: u8, checksum: u32 },
    /// Sub-type 6: `u8 file type, u32 checksum, dynstr8 URL` — download
    /// redirect. This is what open.mp's ModelUrl sends.
    Redirect { file_type: u8, checksum: u32, url: Vec<u8> },
    /// Any other sub-type (no fields).
    Other(u8),
}

impl ModelUrl {
    pub const SUB_RESET: u8 = 0;
    pub const SUB_FILE_CHUNK: u8 = 1;
    pub const SUB_TYPE_2: u8 = 2;
    pub const SUB_NO_DOWNLOAD_INFO: u8 = 5;
    pub const SUB_REDIRECT: u8 = 6;
    /// Largest file chunk the client accepts in sub-type 1.
    pub const MAX_CHUNK: usize = 4096;

    /// The sub-type byte of this message.
    pub fn sub_type(&self) -> u8 {
        match self {
            Self::Reset => Self::SUB_RESET,
            Self::FileChunk { .. } => Self::SUB_FILE_CHUNK,
            Self::SubType2 => Self::SUB_TYPE_2,
            Self::NoDownloadInfo { .. } => Self::SUB_NO_DOWNLOAD_INFO,
            Self::Redirect { .. } => Self::SUB_REDIRECT,
            Self::Other(s) => *s,
        }
    }
}

impl RpcPayload for ModelUrl {
    const ID: u8 = 183;
    fn read(r: &mut BitReader) -> Option<Self> {
        Some(match r.read_u8()? {
            Self::SUB_RESET => Self::Reset,
            Self::SUB_FILE_CHUNK => {
                let file_type = r.read_u8()?;
                let checksum = r.read_u32()?;
                let len = usize::try_from(i32::get(r)?).ok().filter(|&n| n <= Self::MAX_CHUNK)?;
                Self::FileChunk { file_type, checksum, data: r.read_bytes(len)? }
            }
            Self::SUB_TYPE_2 => Self::SubType2,
            Self::SUB_NO_DOWNLOAD_INFO => Self::NoDownloadInfo { file_type: r.read_u8()?, checksum: r.read_u32()? },
            Self::SUB_REDIRECT => {
                Self::Redirect { file_type: r.read_u8()?, checksum: r.read_u32()?, url: read_str8(r)? }
            }
            s => Self::Other(s),
        })
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_u8(self.sub_type());
        match self {
            Self::Reset | Self::SubType2 | Self::Other(_) => {}
            Self::FileChunk { file_type, checksum, data } => {
                let data = &data[..data.len().min(Self::MAX_CHUNK)];
                w.write_u8(*file_type);
                w.write_u32(*checksum);
                w.write_u32(data.len() as u32);
                w.write_bytes(data);
            }
            Self::NoDownloadInfo { file_type, checksum } => {
                w.write_u8(*file_type);
                w.write_u32(*checksum);
            }
            Self::Redirect { file_type, checksum, url } => {
                w.write_u8(*file_type);
                w.write_u32(*checksum);
                write_str8(w, url);
            }
        }
    }
}

payload! {
    /// RPC 185 DownloadCompleted (S→C) — dormant in R4. Empty payload.
    ///
    /// Spec: world-entities.md "Custom models" (handler mapping is low
    /// confidence). open.mp sends it in reply to [`FinishDownload`].
    DownloadCompleted = 185;
}

payload! {
    /// RPC 181 RequestDFF (C→S) — dormant in R4 (never sent).
    ///
    /// Spec: world-entities.md "Custom models", session-misc.md §181. Layout:
    /// `u32 checksum` of the DFF file wanted. Reliability RELIABLE_ORDERED.
    RequestDff = 181; {
        checksum: u32,
    }
}

payload! {
    /// RPC 182 RequestTXD (C→S) — dormant in R4 (never sent).
    ///
    /// Spec: world-entities.md "Custom models", session-misc.md §182. Layout:
    /// `u32 checksum` of the TXD file wanted. Reliability RELIABLE_ORDERED.
    RequestTxd = 182; {
        checksum: u32,
    }
}

/// RPC 184 FinishDownload (C→S) — dormant in R4 (never sent).
///
/// Spec: world-entities.md "Custom models" (single 1 bit). The client's send
/// site writes one `1` bit (session-misc.md §184 calls the payload empty;
/// the bit is confirmed at the send site). open.mp reads nothing, so the bit is
/// harmless. Following the client, [`FinishDownload::write`] emits the bit; the
/// reader accepts the payload with or without it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinishDownload;

impl RpcPayload for FinishDownload {
    const ID: u8 = 184;
    fn read(r: &mut BitReader) -> Option<Self> {
        let _ = r.read_bit();
        Some(Self)
    }
    fn write(&self, w: &mut BitWriter) {
        w.write_bit(true);
    }
}

// ---------------------------------------------------------------------------
// Incoming dispatch
// ---------------------------------------------------------------------------

/// S→C RPC ids the R4 client has handlers for but never registers (custom
/// models). An R4 profile should not dispatch these; later versions do.
pub const DORMANT_IN_R4: &[u8] = &[179, 183, 185];

/// All S→C payloads of this module.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    PlayerBeginObjectSelect(PlayerBeginObjectSelect),
    PlayerCancelObjectEdit(PlayerCancelObjectEdit),
    PlayerShowTextLabel(PlayerShowTextLabel),
    DisableCheckpoint(DisableCheckpoint),
    SetRaceCheckpoint(SetRaceCheckpoint),
    DisableRaceCheckpoint(DisableRaceCheckpoint),
    CreateObject(CreateObject),
    SetObjectPosition(SetObjectPosition),
    SetObjectRotation(SetObjectRotation),
    DestroyObject(DestroyObject),
    SetPlayerMapIcon(SetPlayerMapIcon),
    PlayerHideTextLabel(PlayerHideTextLabel),
    PlayerDestroyPickup(PlayerDestroyPickup),
    AttachObjectToPlayer(AttachObjectToPlayer),
    SetPlayerObjectMaterial(SetPlayerObjectMaterial),
    StopFlashGangZone(StopFlashGangZone),
    PlayerCreatePickup(PlayerCreatePickup),
    MoveObject(MoveObject),
    SetCheckpoint(SetCheckpoint),
    ShowGangZone(ShowGangZone),
    SetPlayerAttachedObject(SetPlayerAttachedObject),
    PlayerBeginAttachedObjectEdit(PlayerBeginAttachedObjectEdit),
    PlayerBeginObjectEdit(PlayerBeginObjectEdit),
    HideGangZone(HideGangZone),
    FlashGangZone(FlashGangZone),
    StopObject(StopObject),
    RemovePlayerMapIcon(RemovePlayerMapIcon),
    SetObjectNoCameraCollision(SetObjectNoCameraCollision),
    ShowActorForPlayer(ShowActorForPlayer),
    HideActorForPlayer(HideActorForPlayer),
    ApplyActorAnimationForPlayer(ApplyActorAnimationForPlayer),
    ClearActorAnimationsForPlayer(ClearActorAnimationsForPlayer),
    SetActorFacingAngleForPlayer(SetActorFacingAngleForPlayer),
    SetActorPosForPlayer(SetActorPosForPlayer),
    SetActorHealthForPlayer(SetActorHealthForPlayer),
    ModelRequest(ModelRequest),
    ModelUrl(ModelUrl),
    DownloadCompleted(DownloadCompleted),
}

/// Generates `INCOMING_IDS`, [`decode`] and [`Incoming::id`] from one list so
/// they cannot drift apart.
macro_rules! incoming {
    ($($ty:ident),+ $(,)?) => {
        /// S→C RPC ids decoded by [`decode`] (includes [`DORMANT_IN_R4`]).
        pub const INCOMING_IDS: &[u8] = &[$(<$ty as RpcPayload>::ID),+];

        /// Decode an S→C RPC of this module; `None` for other ids or malformed data.
        pub fn decode(id: u8, r: &mut BitReader) -> Option<Incoming> {
            $(
                if id == <$ty as RpcPayload>::ID {
                    return <$ty as RpcPayload>::read(r).map(Incoming::$ty);
                }
            )+
            None
        }

        impl Incoming {
            /// RPC id of the payload.
            pub fn id(&self) -> u8 {
                match self {
                    $(Self::$ty(_) => <$ty as RpcPayload>::ID,)+
                }
            }

            /// Encode the payload (without RPC framing).
            pub fn write(&self, w: &mut BitWriter) {
                match self {
                    $(Self::$ty(p) => p.write(w),)+
                }
            }
        }
    };
}

incoming!(
    PlayerBeginObjectSelect,
    PlayerCancelObjectEdit,
    PlayerShowTextLabel,
    DisableCheckpoint,
    SetRaceCheckpoint,
    DisableRaceCheckpoint,
    CreateObject,
    SetObjectPosition,
    SetObjectRotation,
    DestroyObject,
    SetPlayerMapIcon,
    PlayerHideTextLabel,
    PlayerDestroyPickup,
    AttachObjectToPlayer,
    SetPlayerObjectMaterial,
    StopFlashGangZone,
    PlayerCreatePickup,
    MoveObject,
    SetCheckpoint,
    ShowGangZone,
    SetPlayerAttachedObject,
    PlayerBeginAttachedObjectEdit,
    PlayerBeginObjectEdit,
    HideGangZone,
    FlashGangZone,
    StopObject,
    RemovePlayerMapIcon,
    SetObjectNoCameraCollision,
    ShowActorForPlayer,
    HideActorForPlayer,
    ApplyActorAnimationForPlayer,
    ClearActorAnimationsForPlayer,
    SetActorFacingAngleForPlayer,
    SetActorPosForPlayer,
    SetActorHealthForPlayer,
    ModelRequest,
    ModelUrl,
    DownloadCompleted,
);

// ---------------------------------------------------------------------------
// Outgoing (C→S) send parameters
// ---------------------------------------------------------------------------

outgoing! {
    World;
    /// RPC 27: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (world-entities.md §27; missing from rpc-ids.md's send table, see
    /// docs/re/README.md "Gaps"). Mirrors samp.dll 0x1006dcd0.
    OnPlayerSelectObject => ReliableOrdered;
    /// RPC 117: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (world-entities.md §117). Mirrors samp.dll 0x10072910.
    OnPlayerEditObject => ReliableOrdered;
    /// RPC 116: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (world-entities.md §116). Mirrors samp.dll 0x10072b20.
    OnPlayerEditAttachedObject => ReliableOrdered;
    /// RPC 131, collision-hook path: reliability 9 (RELIABLE_ORDERED),
    /// priority 1, channel 0 (world-entities.md §131). The poll of type-14
    /// pickups sends the same payload with reliability 10, see
    /// [`OnPlayerPickUpPickup::POLL_SEND_PARAMS`]. Mirrors samp.dll 0x10013440.
    OnPlayerPickUpPickup => ReliableOrdered;
    /// RPC 177: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (world-entities.md §177). Mirrors samp.dll 0x10006ac0.
    OnPlayerDamageActor => ReliableOrdered;
    /// RPC 181: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (session-misc.md §181/182). Sent from a shared send site that takes the
    /// RPC id as a variable (kind 1); dormant in R4, so the original client
    /// never sends it. Mirrors samp.dll 0x1000d800.
    RequestDff => ReliableOrdered;
    /// RPC 182: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (session-misc.md §181/182). Same shared send site as RPC 181 (kind 2);
    /// dormant in R4. Mirrors samp.dll 0x1000d800.
    RequestTxd => ReliableOrdered;
    /// RPC 184: reliability 9 (RELIABLE_ORDERED), priority 1, channel 0
    /// (world-entities.md "Custom models", session-misc.md §184). Dormant in
    /// R4. Mirrors samp.dll 0x1000d920.
    FinishDownload => ReliableOrdered;
}

impl OnPlayerPickUpPickup {
    /// Send parameters of RPC 131's second send site, the poll of type-14
    /// pickups (run every ~6 netgame ticks): reliability 10
    /// (RELIABLE_SEQUENCED), priority 1, channel 0 (world-entities.md §131).
    /// [`crate::rpcs::OutgoingRpc::SEND_PARAMS`] is the collision-hook path.
    /// Mirrors samp.dll 0x10013520.
    pub const POLL_SEND_PARAMS: crate::rpcs::SendParams = crate::rpcs::SendParams {
        priority: crate::rpcs::RPC_PRIORITY,
        reliability: crate::raknet::Reliability::ReliableSequenced,
        channel: crate::rpcs::RPC_CHANNEL,
    };
}

#[cfg(test)]
mod tests;
