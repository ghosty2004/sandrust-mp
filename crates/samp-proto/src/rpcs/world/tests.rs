//! Tests for `rpcs::world`. Expected byte streams are built by hand from the
//! spec tables (docs/re/subsystems/world-entities.md), not from the writers.

use super::*;
use std::fmt::Debug;

// --- helpers ----------------------------------------------------------------

/// Hand-built expected bit stream: whole bytes and single bits, MSB-first.
#[derive(Default)]
struct Bits(Vec<bool>);

impl Bits {
    fn bytes(mut self, bytes: &[u8]) -> Self {
        for &b in bytes {
            for i in (0..8).rev() {
                self.0.push((b >> i) & 1 == 1);
            }
        }
        self
    }
    fn bit(mut self, b: bool) -> Self {
        self.0.push(b);
        self
    }
    fn f32(self, v: f32) -> Self {
        self.bytes(&v.to_le_bytes())
    }
    fn vec3(self, x: f32, y: f32, z: f32) -> Self {
        self.f32(x).f32(y).f32(z)
    }
    fn u16(self, v: u16) -> Self {
        self.bytes(&v.to_le_bytes())
    }
    fn u32(self, v: u32) -> Self {
        self.bytes(&v.to_le_bytes())
    }
    fn i32(self, v: i32) -> Self {
        self.bytes(&v.to_le_bytes())
    }
    /// Packed bytes (last byte zero-padded) and bit length.
    fn finish(&self) -> (Vec<u8>, usize) {
        let mut out = vec![0u8; self.0.len().div_ceil(8)];
        for (i, &b) in self.0.iter().enumerate() {
            if b {
                out[i / 8] |= 0x80 >> (i % 8);
            }
        }
        (out, self.0.len())
    }
}

fn encode<T: RpcPayload>(v: &T) -> (Vec<u8>, usize) {
    let mut w = BitWriter::new();
    v.write(&mut w);
    let n = w.bit_len();
    (w.into_bytes(), n)
}

/// Write, read back, compare, and check the reader consumed everything.
fn roundtrip<T: RpcPayload + PartialEq + Debug>(v: &T) {
    let (bytes, bits) = encode(v);
    let mut r = BitReader::with_bit_len(&bytes, bits);
    let back = T::read(&mut r).expect("decode");
    assert_eq!(&back, v);
    assert_eq!(r.remaining(), 0, "{} left over", r.remaining());
}

/// Writer output equals the hand-built stream, and the reader decodes it to `v`.
fn exact<T: RpcPayload + PartialEq + Debug>(v: &T, expected: Bits) {
    let (exp, exp_bits) = expected.finish();
    assert_eq!(encode(v), (exp.clone(), exp_bits), "writer");
    let mut r = BitReader::with_bit_len(&exp, exp_bits);
    assert_eq!(T::read(&mut r).as_ref(), Some(v), "reader");
    assert_eq!(r.remaining(), 0);
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn attached_data() -> AttachedObjectData {
    AttachedObjectData {
        model: 19_006,
        bone: 2,
        offset: v3(0.1, 0.2, 0.3),
        rotation: v3(10.0, 20.0, 30.0),
        scale: v3(1.0, 1.0, 1.5),
        colour1: 0xFF00_00FF,
        colour2: 0,
    }
}

fn attached_bits(b: Bits) -> Bits {
    b.i32(19_006).i32(2).vec3(0.1, 0.2, 0.3).vec3(10.0, 20.0, 30.0).vec3(1.0, 1.0, 1.5).u32(0xFF00_00FF).u32(0)
}

fn texture_material() -> ObjectMaterial {
    ObjectMaterial::Texture {
        slot: 3,
        model: 18_200,
        txd: b"abc".to_vec(),
        texture: b"t".to_vec(),
        colour: 0xFF00_FF00,
    }
}

fn text_material() -> ObjectMaterial {
    ObjectMaterial::Text {
        slot: 0,
        material_size: 90,
        font: b"Arial".to_vec(),
        font_size: 24,
        bold: 1,
        font_colour: 0xFFFF_FFFF,
        background_colour: 0,
        alignment: 1,
        text: b"Hello".to_vec(),
    }
}

// --- objects -----------------------------------------------------------------

#[test]
fn create_object_unattached_with_texture_bytes() {
    let o = CreateObject {
        object_id: 0x0102,
        model: 19_000,
        position: v3(1.0, 2.0, 3.0),
        rotation: v3(0.0, 0.0, 90.0),
        draw_distance: 300.0,
        no_camera_collision: 1,
        materials: vec![texture_material()],
        ..CreateObject::default()
    };
    let exp = Bits::default()
        .bytes(&[0x02, 0x01]) // id
        .bytes(&[0x38, 0x4A, 0x00, 0x00]) // model 19000
        .vec3(1.0, 2.0, 3.0)
        .vec3(0.0, 0.0, 90.0)
        .f32(300.0)
        .bytes(&[1]) // no camera collision
        .bytes(&[0xFF, 0xFF, 0xFF, 0xFF]) // no vehicle, no object → no attach block
        .bytes(&[1]) // material count
        .bytes(&[1, 3]) // kind texture, slot 3
        .bytes(&[0x18, 0x47]) // model 18200
        .bytes(&[3, b'a', b'b', b'c', 1, b't'])
        .bytes(&[0x00, 0xFF, 0x00, 0xFF]); // colour
    exact(&o, exp);
}

#[test]
fn create_object_attached_to_vehicle_bytes() {
    let o = CreateObject {
        object_id: 7,
        model: -5,
        attached_vehicle_id: 5,
        attach_offset: v3(0.0, 1.0, 0.5),
        attach_rotation: v3(0.0, 0.0, 180.0),
        sync_rotation: 1,
        ..CreateObject::default()
    };
    assert!(o.is_attached());
    let exp = Bits::default()
        .u16(7)
        .bytes(&[0xFB, 0xFF, 0xFF, 0xFF]) // model -5
        .vec3(0.0, 0.0, 0.0)
        .vec3(0.0, 0.0, 0.0)
        .f32(0.0)
        .bytes(&[0])
        .u16(5)
        .u16(0xFFFF)
        .vec3(0.0, 1.0, 0.5)
        .vec3(0.0, 0.0, 180.0)
        .bytes(&[1]) // sync rotation
        .bytes(&[0]); // no materials
    exact(&o, exp);
}

#[test]
fn create_object_attached_to_object_roundtrip() {
    roundtrip(&CreateObject {
        object_id: 999,
        model: 1337,
        attached_object_id: 12,
        attach_offset: v3(1.0, 2.0, 3.0),
        sync_rotation: 0,
        materials: vec![texture_material(), ObjectMaterial::Unknown { kind: 9 }],
        ..CreateObject::default()
    });
}

#[test]
fn create_object_unknown_material_kind_reads_only_kind() {
    // count 2: an unknown kind (only its kind byte is consumed), then a texture.
    let (bytes, bits) = Bits::default()
        .u16(1)
        .i32(1000)
        .vec3(0.0, 0.0, 0.0)
        .vec3(0.0, 0.0, 0.0)
        .f32(0.0)
        .bytes(&[0, 0xFF, 0xFF, 0xFF, 0xFF])
        .bytes(&[2])
        .bytes(&[7])
        .bytes(&[1, 3, 0x18, 0x47, 3, b'a', b'b', b'c', 1, b't', 0x00, 0xFF, 0x00, 0xFF])
        .finish();
    let mut r = BitReader::with_bit_len(&bytes, bits);
    let o = CreateObject::read(&mut r).unwrap();
    assert_eq!(o.materials, vec![ObjectMaterial::Unknown { kind: 7 }, texture_material()]);
    assert_eq!(r.remaining(), 0);
}

#[test]
fn create_object_truncated_is_none() {
    let (bytes, _) = encode(&CreateObject { materials: vec![texture_material()], ..Default::default() });
    let mut r = BitReader::new(&bytes[..bytes.len() - 1]);
    assert_eq!(CreateObject::read(&mut r), None);
}

#[test]
fn create_object_with_text_material_roundtrip() {
    roundtrip(&CreateObject {
        object_id: 3,
        model: 19_477,
        materials: vec![text_material(), texture_material()],
        ..CreateObject::default()
    });
}

#[test]
fn set_object_position_bytes_and_extra_float_ignored() {
    let o = SetObjectPosition { object_id: 0x10, position: v3(-1.0, 2.5, 100.0) };
    exact(&o, Bits::default().bytes(&[0x10, 0x00]).vec3(-1.0, 2.5, 100.0));
    // The R4 client tries to read a 4th float; a sender that adds one must still decode.
    let (bytes, bits) = Bits::default().u16(0x10).vec3(-1.0, 2.5, 100.0).f32(9.0).finish();
    let mut r = BitReader::with_bit_len(&bytes, bits);
    assert_eq!(SetObjectPosition::read(&mut r), Some(o));
}

#[test]
fn simple_object_payloads_roundtrip() {
    roundtrip(&SetObjectRotation { object_id: 4, rotation: v3(1.0, 2.0, 3.0) });
    roundtrip(&DestroyObject { object_id: 999 });
    roundtrip(&StopObject { object_id: 1 });
    roundtrip(&SetObjectNoCameraCollision { object_id: 42 });
    roundtrip(&AttachObjectToPlayer {
        object_id: 5,
        player_id: 3,
        offset: v3(0.0, 0.0, 1.0),
        rotation: v3(0.0, 90.0, 0.0),
    });
    roundtrip(&PlayerBeginObjectSelect);
    roundtrip(&PlayerCancelObjectEdit);
}

#[test]
fn set_object_rotation_and_attach_bytes() {
    exact(
        &SetObjectRotation { object_id: 0xABCD, rotation: v3(0.0, 45.0, 0.0) },
        Bits::default().bytes(&[0xCD, 0xAB]).vec3(0.0, 45.0, 0.0),
    );
    exact(
        &AttachObjectToPlayer { object_id: 1, player_id: 2, offset: v3(1.0, 0.0, 0.0), rotation: v3(0.0, 0.0, 1.0) },
        Bits::default().bytes(&[1, 0, 2, 0]).vec3(1.0, 0.0, 0.0).vec3(0.0, 0.0, 1.0),
    );
    exact(&SetObjectNoCameraCollision { object_id: 0x0203 }, Bits::default().bytes(&[0x03, 0x02]));
    exact(&PlayerBeginObjectSelect, Bits::default());
}

#[test]
fn move_object_bytes() {
    let m = MoveObject {
        object_id: 9,
        current_position: v3(1.0, 1.0, 1.0),
        target_position: v3(2.0, 2.0, 2.0),
        speed: 3.5,
        target_rotation: v3(MoveObject::NO_ROTATION, MoveObject::NO_ROTATION, MoveObject::NO_ROTATION),
    };
    let exp = Bits::default()
        .u16(9)
        .vec3(1.0, 1.0, 1.0)
        .vec3(2.0, 2.0, 2.0)
        .f32(3.5)
        .bytes(&[0x00, 0x00, 0x7A, 0xC4]) // -1000.0
        .bytes(&[0x00, 0x00, 0x7A, 0xC4])
        .bytes(&[0x00, 0x00, 0x7A, 0xC4]);
    exact(&m, exp);
}

#[test]
fn set_player_object_material_texture_bytes() {
    let m = SetPlayerObjectMaterial {
        object_id: 0x0301,
        material: ObjectMaterial::Texture {
            slot: 0,
            model: 0xFFFF,
            txd: b"none".to_vec(),
            texture: b"none".to_vec(),
            colour: 0x1122_3344,
        },
    };
    let exp = Bits::default()
        .bytes(&[0x01, 0x03, 1, 0, 0xFF, 0xFF])
        .bytes(&[4, b'n', b'o', b'n', b'e', 4, b'n', b'o', b'n', b'e'])
        .bytes(&[0x44, 0x33, 0x22, 0x11]);
    exact(&m, exp);
    roundtrip(&SetPlayerObjectMaterial { object_id: 1, material: ObjectMaterial::Unknown { kind: 0 } });
}

#[test]
fn set_player_object_material_text_roundtrip() {
    roundtrip(&SetPlayerObjectMaterial { object_id: 77, material: text_material() });
}

#[test]
fn set_player_attached_object_create_bits() {
    let a = SetPlayerAttachedObject { player_id: 0x0102, index: 9, data: Some(attached_data()) };
    // 6 aligned bytes, the create bit, then the 52-byte record shifted by one bit.
    let exp = attached_bits(Bits::default().bytes(&[0x02, 0x01, 9, 0, 0, 0]).bit(true));
    let (bytes, bits) = encode(&a);
    assert_eq!(bits, 6 * 8 + 1 + AttachedObjectData::SIZE * 8);
    assert_eq!((bytes, bits), exp.finish());
    exact(&a, attached_bits(Bits::default().u16(0x0102).u32(9).bit(true)));
}

#[test]
fn set_player_attached_object_remove_bits() {
    let a = SetPlayerAttachedObject { player_id: 4, index: 2, data: None };
    // 4,0 | 2,0,0,0 | 0 → 49 bits.
    exact(&a, Bits::default().bytes(&[4, 0, 2, 0, 0, 0]).bit(false));
    assert_eq!(encode(&a).1, 49);
}

#[test]
fn on_player_select_object_bytes() {
    let s = OnPlayerSelectObject { select_type: 0, object_id: 0x0010, model: 1225, position: v3(5.0, 6.0, 7.0) };
    exact(&s, Bits::default().bytes(&[0, 0, 0, 0, 0x10, 0x00, 0xC9, 0x04, 0, 0]).vec3(5.0, 6.0, 7.0));
}

#[test]
fn begin_object_edit_bit_exact() {
    // 1 | 0x34 | 0x12 → 1001_1010 0000_1001 0000_0000 (17 bits).
    let b = PlayerBeginObjectEdit { player_object: true, object_id: 0x1234 };
    assert_eq!(encode(&b), (vec![0x9A, 0x09, 0x00], 17));
    let mut r = BitReader::with_bit_len(&[0x9A, 0x09, 0x00], 17);
    assert_eq!(PlayerBeginObjectEdit::read(&mut r), Some(b));
}

#[test]
fn on_player_edit_object_bits() {
    let e = OnPlayerEditObject {
        player_object: false,
        object_id: 300,
        response: OnPlayerEditObject::RESPONSE_UPDATE,
        position: v3(1.0, 2.0, 3.0),
        rotation: v3(4.0, 5.0, 6.0),
    };
    exact(&e, Bits::default().bit(false).u16(300).u32(2).vec3(1.0, 2.0, 3.0).vec3(4.0, 5.0, 6.0));
    assert_eq!(encode(&e).1, 1 + 16 + 32 + 6 * 32);
}

#[test]
fn begin_attached_object_edit_bytes() {
    exact(&PlayerBeginAttachedObjectEdit { index: 3 }, Bits::default().bytes(&[3, 0, 0, 0]));
}

#[test]
fn on_player_edit_attached_object_full_and_short() {
    let full = OnPlayerEditAttachedObject { response: 1, index: 4, data: Some(attached_data()) };
    exact(&full, attached_bits(Bits::default().bytes(&[1, 0, 0, 0, 4, 0, 0, 0])));
    // Empty/invalid slot: the client omits the record → 8-byte payload.
    let short = OnPlayerEditAttachedObject { response: 0, index: 12, data: None };
    exact(&short, Bits::default().bytes(&[0, 0, 0, 0, 12, 0, 0, 0]));
    // A partial record is malformed.
    let mut r = BitReader::new(&[0, 0, 0, 0, 1, 0, 0, 0, 0xAA]);
    assert_eq!(OnPlayerEditAttachedObject::read(&mut r), None);
}

// --- pickups -------------------------------------------------------------------

#[test]
fn pickup_bytes() {
    exact(
        &PlayerCreatePickup { pickup_id: 4095, model: 1239, pickup_type: 14, position: v3(10.0, -20.0, 3.0) },
        Bits::default().bytes(&[0xFF, 0x0F, 0, 0, 0xD7, 0x04, 0, 0, 14, 0, 0, 0]).vec3(10.0, -20.0, 3.0),
    );
    exact(&PlayerDestroyPickup { pickup_id: 1 }, Bits::default().bytes(&[1, 0, 0, 0]));
    exact(&OnPlayerPickUpPickup { pickup_id: 0x0102_0304 }, Bits::default().bytes(&[4, 3, 2, 1]));
    roundtrip(&OnPlayerPickUpPickup { pickup_id: -1 });
}

// --- actors ----------------------------------------------------------------------

#[test]
fn show_actor_is_27_bytes() {
    let a = ShowActorForPlayer {
        actor_id: 999,
        skin: 280,
        position: v3(1.0, 2.0, 3.0),
        angle: 90.0,
        health: 100.0,
        invulnerable: 1,
    };
    let exp =
        Bits::default().bytes(&[0xE7, 0x03, 0x18, 0x01, 0, 0]).vec3(1.0, 2.0, 3.0).f32(90.0).f32(100.0).bytes(&[1]);
    exact(&a, exp);
    assert_eq!(encode(&a).1, 27 * 8);
}

#[test]
fn apply_actor_animation_bits() {
    let a = ApplyActorAnimationForPlayer {
        actor_id: 2,
        library: b"PED".to_vec(),
        name: b"WALK_civi".to_vec(),
        delta: 4.1,
        loop_anim: true,
        lock_x: false,
        lock_y: true,
        freeze: false,
        time: 1000,
    };
    let exp = Bits::default()
        .bytes(&[2, 0, 3, b'P', b'E', b'D', 9])
        .bytes(b"WALK_civi")
        .f32(4.1)
        .bit(true)
        .bit(false)
        .bit(true)
        .bit(false)
        .bytes(&[0xE8, 0x03, 0, 0]);
    exact(&a, exp);
}

#[test]
fn actor_small_payloads() {
    exact(&HideActorForPlayer { actor_id: 5 }, Bits::default().bytes(&[5, 0]));
    exact(&ClearActorAnimationsForPlayer { actor_id: 6 }, Bits::default().bytes(&[6, 0]));
    exact(&SetActorFacingAngleForPlayer { actor_id: 1, angle: 180.0 }, Bits::default().u16(1).f32(180.0));
    exact(
        &SetActorPosForPlayer { actor_id: 1, position: v3(1.0, 2.0, 3.0) },
        Bits::default().u16(1).vec3(1.0, 2.0, 3.0),
    );
    exact(&SetActorHealthForPlayer { actor_id: 1, health: 0.0 }, Bits::default().u16(1).f32(0.0));
}

#[test]
fn on_player_damage_actor_bits() {
    let d = OnPlayerDamageActor { unknown: false, actor_id: 7, damage: 46.2, weapon: 24, body_part: 3 };
    exact(&d, Bits::default().bit(false).bytes(&[7, 0]).f32(46.2).bytes(&[24, 0, 0, 0, 3, 0, 0, 0]));
    assert_eq!(encode(&d).1, 1 + 16 + 32 * 3);
}

// --- checkpoints, gang zones, map icons ------------------------------------------

#[test]
fn checkpoint_bytes() {
    exact(&SetCheckpoint { position: v3(1.0, 2.0, 3.0), size: 5.0 }, Bits::default().vec3(1.0, 2.0, 3.0).f32(5.0));
    exact(
        &SetRaceCheckpoint {
            checkpoint_type: 1,
            position: v3(1.0, 2.0, 3.0),
            next_position: v3(4.0, 5.0, 6.0),
            size: 8.0,
        },
        Bits::default().bytes(&[1]).vec3(1.0, 2.0, 3.0).vec3(4.0, 5.0, 6.0).f32(8.0),
    );
    exact(&DisableCheckpoint, Bits::default());
    exact(&DisableRaceCheckpoint, Bits::default());
}

#[test]
fn gang_zone_bytes() {
    exact(
        &ShowGangZone {
            zone_id: 1023,
            min: Vec2 { x: -100.0, y: -50.0 },
            max: Vec2 { x: 100.0, y: 50.0 },
            colour: 0xAA00_00FF,
        },
        Bits::default()
            .bytes(&[0xFF, 0x03])
            .f32(-100.0)
            .f32(-50.0)
            .f32(100.0)
            .f32(50.0)
            .bytes(&[0xFF, 0x00, 0x00, 0xAA]),
    );
    exact(&HideGangZone { zone_id: 2 }, Bits::default().bytes(&[2, 0]));
    exact(&FlashGangZone { zone_id: 2, colour: 0x0102_0304 }, Bits::default().bytes(&[2, 0, 4, 3, 2, 1]));
    exact(&StopFlashGangZone { zone_id: 0xFFFF }, Bits::default().bytes(&[0xFF, 0xFF]));
}

#[test]
fn map_icon_bytes() {
    exact(
        &SetPlayerMapIcon { icon_id: 99, position: v3(1.0, 2.0, 3.0), icon_type: 0, colour: 0xFF00_00FF, style: 3 },
        Bits::default().bytes(&[99]).vec3(1.0, 2.0, 3.0).bytes(&[0, 0xFF, 0, 0, 0xFF, 3]),
    );
    exact(&RemovePlayerMapIcon { icon_id: 12 }, Bits::default().bytes(&[12]));
}

// --- 3D text labels ------------------------------------------------------------------

#[test]
fn hide_text_label_bytes() {
    exact(&PlayerHideTextLabel { label_id: 1024 + 5 }, Bits::default().bytes(&[0x05, 0x04]));
}

#[test]
fn show_text_label_roundtrip() {
    let l = PlayerShowTextLabel {
        label_id: 1024 + 3,
        colour: 0xFFFF_00FF,
        position: v3(0.0, 0.0, 0.5),
        draw_distance: 20.0,
        test_los: 1,
        attached_player_id: 4,
        attached_vehicle_id: INVALID_ID,
        text: b"{FF0000}Hi there".to_vec(),
    };
    roundtrip(&l);
    // Fixed prefix is byte-exact; the compressed text follows.
    let (bytes, _) = encode(&l);
    let (prefix, _) = Bits::default()
        .bytes(&[0x03, 0x04, 0xFF, 0x00, 0xFF, 0xFF])
        .vec3(0.0, 0.0, 0.5)
        .f32(20.0)
        .bytes(&[1, 4, 0, 0xFF, 0xFF])
        .finish();
    assert_eq!(&bytes[..prefix.len()], &prefix[..]);
}

// --- custom models ---------------------------------------------------------------------

#[test]
fn model_request_bytes() {
    let m = ModelRequest {
        pool_id: 1,
        count: 2,
        model_type: 0,
        virtual_world: -1,
        base_id: 19_379,
        new_id: -1000,
        dff_checksum: 0xDEAD_BEEF,
        txd_checksum: 0x0102_0304,
        dff_size: 100,
        txd_size: 200,
        time_on: 0,
        time_off: 24,
    };
    let exp = Bits::default()
        .bytes(&[1, 0, 0, 0, 2, 0, 0, 0, 0])
        .bytes(&[0xFF, 0xFF, 0xFF, 0xFF])
        .bytes(&[0xB3, 0x4B, 0, 0])
        .bytes(&[0x18, 0xFC, 0xFF, 0xFF])
        .bytes(&[0xEF, 0xBE, 0xAD, 0xDE, 4, 3, 2, 1])
        .bytes(&[100, 0, 0, 0, 200, 0, 0, 0, 0, 24]);
    exact(&m, exp);
}

#[test]
fn model_url_redirect_bytes() {
    let m = ModelUrl::Redirect { file_type: 1, checksum: 0x1122_3344, url: b"http://x/a.dff".to_vec() };
    let mut exp = Bits::default().bytes(&[6, 1, 0x44, 0x33, 0x22, 0x11, 14]);
    exp = exp.bytes(b"http://x/a.dff");
    exact(&m, exp);
}

#[test]
fn model_url_other_subtypes() {
    exact(
        &ModelUrl::FileChunk { file_type: 2, checksum: 7, data: vec![0xAA, 0xBB, 0xCC] },
        Bits::default().bytes(&[1, 2, 7, 0, 0, 0, 3, 0, 0, 0, 0xAA, 0xBB, 0xCC]),
    );
    exact(&ModelUrl::NoDownloadInfo { file_type: 1, checksum: 9 }, Bits::default().bytes(&[5, 1, 9, 0, 0, 0]));
    exact(&ModelUrl::Reset, Bits::default().bytes(&[0]));
    exact(&ModelUrl::SubType2, Bits::default().bytes(&[2]));
    exact(&ModelUrl::Other(3), Bits::default().bytes(&[3]));
    // Chunk length outside 0…4096 is malformed.
    let mut r = BitReader::new(&[1, 2, 7, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(ModelUrl::read(&mut r), None);
    let mut r = BitReader::new(&[1, 2, 7, 0, 0, 0, 0x01, 0x10, 0, 0]);
    assert_eq!(ModelUrl::read(&mut r), None);
}

#[test]
fn download_c2s_bytes() {
    exact(&RequestDff { checksum: 0xCAFE_BABE }, Bits::default().bytes(&[0xBE, 0xBA, 0xFE, 0xCA]));
    exact(&RequestTxd { checksum: 1 }, Bits::default().bytes(&[1, 0, 0, 0]));
    exact(&DownloadCompleted, Bits::default());
    // The client writes a single 1 bit; open.mp's empty form must also decode.
    exact(&FinishDownload, Bits::default().bit(true));
    assert_eq!(FinishDownload::read(&mut BitReader::new(&[])), Some(FinishDownload));
}

// --- dispatch ------------------------------------------------------------------------------

fn incoming_samples() -> Vec<Incoming> {
    use Incoming as I;
    vec![
        I::PlayerBeginObjectSelect(PlayerBeginObjectSelect),
        I::PlayerCancelObjectEdit(PlayerCancelObjectEdit),
        I::DisableCheckpoint(DisableCheckpoint),
        I::SetRaceCheckpoint(SetRaceCheckpoint { checkpoint_type: 2, size: 3.0, ..Default::default() }),
        I::DisableRaceCheckpoint(DisableRaceCheckpoint),
        I::CreateObject(CreateObject {
            object_id: 1,
            model: 1337,
            materials: vec![texture_material()],
            ..Default::default()
        }),
        I::SetObjectPosition(SetObjectPosition { object_id: 2, position: v3(1.0, 2.0, 3.0) }),
        I::SetObjectRotation(SetObjectRotation { object_id: 3, rotation: v3(1.0, 0.0, 0.0) }),
        I::DestroyObject(DestroyObject { object_id: 4 }),
        I::SetPlayerMapIcon(SetPlayerMapIcon { icon_id: 1, ..Default::default() }),
        I::PlayerHideTextLabel(PlayerHideTextLabel { label_id: 9 }),
        I::PlayerDestroyPickup(PlayerDestroyPickup { pickup_id: 8 }),
        I::AttachObjectToPlayer(AttachObjectToPlayer { object_id: 1, player_id: 2, ..Default::default() }),
        I::SetPlayerObjectMaterial(SetPlayerObjectMaterial { object_id: 1, material: texture_material() }),
        I::StopFlashGangZone(StopFlashGangZone { zone_id: 3 }),
        I::PlayerCreatePickup(PlayerCreatePickup { pickup_id: 1, model: 1240, pickup_type: 2, ..Default::default() }),
        I::MoveObject(MoveObject { object_id: 1, speed: 2.0, ..Default::default() }),
        I::SetCheckpoint(SetCheckpoint { position: v3(1.0, 1.0, 1.0), size: 2.0 }),
        I::ShowGangZone(ShowGangZone { zone_id: 1, colour: 5, ..Default::default() }),
        I::SetPlayerAttachedObject(SetPlayerAttachedObject { player_id: 1, index: 0, data: Some(attached_data()) }),
        I::PlayerBeginAttachedObjectEdit(PlayerBeginAttachedObjectEdit { index: 1 }),
        I::PlayerBeginObjectEdit(PlayerBeginObjectEdit { player_object: true, object_id: 5 }),
        I::HideGangZone(HideGangZone { zone_id: 1 }),
        I::FlashGangZone(FlashGangZone { zone_id: 1, colour: 2 }),
        I::StopObject(StopObject { object_id: 1 }),
        I::RemovePlayerMapIcon(RemovePlayerMapIcon { icon_id: 3 }),
        I::SetObjectNoCameraCollision(SetObjectNoCameraCollision { object_id: 6 }),
        I::ShowActorForPlayer(ShowActorForPlayer { actor_id: 1, skin: 2, health: 100.0, ..Default::default() }),
        I::HideActorForPlayer(HideActorForPlayer { actor_id: 1 }),
        I::ApplyActorAnimationForPlayer(ApplyActorAnimationForPlayer {
            actor_id: 1,
            library: b"PED".to_vec(),
            name: b"IDLE".to_vec(),
            freeze: true,
            ..Default::default()
        }),
        I::ClearActorAnimationsForPlayer(ClearActorAnimationsForPlayer { actor_id: 1 }),
        I::SetActorFacingAngleForPlayer(SetActorFacingAngleForPlayer { actor_id: 1, angle: 1.0 }),
        I::SetActorPosForPlayer(SetActorPosForPlayer { actor_id: 1, position: v3(0.0, 1.0, 2.0) }),
        I::SetActorHealthForPlayer(SetActorHealthForPlayer { actor_id: 1, health: 50.0 }),
        I::ModelRequest(ModelRequest { pool_id: 1, count: 1, virtual_world: -1, ..Default::default() }),
        I::ModelUrl(ModelUrl::Redirect { file_type: 1, checksum: 2, url: b"u".to_vec() }),
        I::DownloadCompleted(DownloadCompleted),
        I::PlayerShowTextLabel(PlayerShowTextLabel {
            label_id: 1024 + 1,
            colour: 0xFF88_00FF,
            position: v3(1.0, 2.0, 3.0),
            draw_distance: 10.0,
            test_los: 0,
            attached_player_id: INVALID_ID,
            attached_vehicle_id: INVALID_ID,
            text: b"label\ntext".to_vec(),
        }),
    ]
}

#[test]
fn decode_every_incoming_variant() {
    let samples = incoming_samples();
    for msg in &samples {
        let mut w = BitWriter::new();
        msg.write(&mut w);
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        assert_eq!(decode(msg.id(), &mut r).as_ref(), Some(msg), "id {}", msg.id());
        assert_eq!(r.remaining(), 0, "id {}", msg.id());
    }
    let mut covered: Vec<u8> = samples.iter().map(Incoming::id).collect();
    covered.sort_unstable();
    let mut ids = INCOMING_IDS.to_vec();
    ids.sort_unstable();
    assert_eq!(covered, ids);
}

#[test]
fn incoming_ids_are_unique_and_scoped() {
    let mut ids = INCOMING_IDS.to_vec();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), INCOMING_IDS.len());
    for id in DORMANT_IN_R4 {
        assert!(INCOMING_IDS.contains(id));
    }
    // C→S-only ids are not incoming.
    for id in [131u8, 177, 181, 182, 184] {
        assert!(!INCOMING_IDS.contains(&id));
        assert_eq!(decode(id, &mut BitReader::new(&[0; 64])), None);
    }
    assert_eq!(decode(200, &mut BitReader::new(&[0; 64])), None);
}

#[test]
fn decode_malformed_is_none() {
    assert_eq!(decode(DestroyObject::ID, &mut BitReader::new(&[1])), None);
    assert_eq!(decode(ShowActorForPlayer::ID, &mut BitReader::new(&[0; 26])), None);
    assert_eq!(
        decode(SetPlayerAttachedObject::ID, &mut BitReader::with_bit_len(&[0, 0, 0, 0, 0, 0, 0x80, 0], 64)),
        None
    );
}

#[test]
fn to_rpc_frames_with_id() {
    let w = DestroyObject { object_id: 0x0102 }.to_rpc();
    let rpc = crate::rpc::decode(w.as_bytes(), w.bit_len()).unwrap();
    assert_eq!((rpc.id, rpc.bit_len), (47, 16));
    assert_eq!(decode(rpc.id, &mut rpc.reader()), Some(Incoming::DestroyObject(DestroyObject { object_id: 0x0102 })));
}

#[test]
fn wire_i32_is_twos_complement_le() {
    let mut w = BitWriter::new();
    (-2i32).put(&mut w);
    assert_eq!(w.as_bytes(), [0xFE, 0xFF, 0xFF, 0xFF]);
    assert_eq!(i32::get(&mut BitReader::new(w.as_bytes())), Some(-2));
}
