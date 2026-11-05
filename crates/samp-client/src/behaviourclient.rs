//! Remaining local behaviour inside the game (M8b, Windows only): camera RPCs and the camera attachment, spectating,
//! world bounds, gravity, interiors, animations (RPC 86/87, the 207 animation field, remote apply), drunk, fighting
//! style, shop name, money/wanted/stunt/velocity, widescreen, ClientCheck.
//!
//! Spec: player-state.md, remote-players-sync.md §1/§2 "212", remote-control-onfoot.md §4.1, session-misc.md "103",
//! docs/phase3/trace-notes.md "Remaining behaviour (TRACE-8)"; rules in [`crate::behaviour`] / [`crate::anims`].
//! samp.dll build B addresses are cited where a detail was read statically.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::player::ApplyPlayerAnimation;
use samp_proto::rpcs::session::{ClientCheckRequest, ClientCheckResponse};
use samp_proto::sync::player::SpectatorSync;
use samp_proto::types::Vec3;

use crate::anims::{self, Latch, RemoteAction, RemoteState};
use crate::behaviour::{self as bh, Bounds, SpecTarget, Spectate};
use crate::game::{self, off};
use crate::netclient::NetClient;
use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};
use crate::script::{Arg, Runner, op};

/// `TheCamera.m_mCameraMatrix` translation (samp's camera object +4 points at 0xB6F99C; B 0x1009FF92).
const CAMERA_MATRIX_POS: u32 = 0xB6F9CC;
/// Global gravity float (B 0x100A1410; 0.008 by default).
const GRAVITY: u32 = 0x863984;
/// Immediate operand in GTA's HUD code: the displayed wanted level (player-state.md 133).
const WANTED_HUD: u32 = 0x58DB60;
/// GTA's stunt-bonus enable dword (player-state.md 104).
const STUNT_BONUS: u32 = 0xA4A474;
/// `CShopping::LoadShop(const char*)` (cdecl; player-state.md 33, TRACE-8).
const LOAD_SHOP: u32 = 0x49BBE0;
/// `CMessages::AddBigMessage(text, time, style)` (cdecl; B 0x100A0D55).
const ADD_BIG_MESSAGE: u32 = 0x69F2B0;
/// `CKeyGen::GetUppercaseKey(const char*)` (cdecl; samp's table hashing B 0x1009CF30 → 0x100B4490).
const GET_UPPERCASE_KEY: u32 = 0x53CF30;
/// `CAnimManager::GetAnimAssociation(group, id)` (cdecl; B 0x100B4440).
const GET_ANIM_ASSOCIATION: u32 = 0x4D3A60;
/// `CPed` shop-name pointer (B 0x100AE43E).
const PED_SHOP_NAME: u32 = 0x78C;
/// `CPed::m_pVehicle`, `CVehicle::m_pDriver`, `CVehicle` primary/secondary colours.
const PED_VEHICLE: u32 = 0x58C;
const VEH_DRIVER: u32 = 0x460;
/// `CPhysical::m_vecTurnSpeed`.
const TURN_SPEED: u32 = 0x58;
/// `CEntity::m_pRwObject` (the clump the anim hooks compare, B 0x100A4220).
const ENTITY_RW_OBJECT: u32 = 0x18;
/// `CPhysical::m_nPhysicalFlags`-word samp reports for ClientCheck type 2 (entity +0x40, B 0x10011BB7).
const ENTITY_CHECK_WORD: u32 = 0x40;
const VTBL_CPLACEABLE: u32 = 0x863C40;
/// Train models samp moves with the mission-train command instead of `Teleport` (B 0x1009F086).
const TRAIN_MODELS: [u16; 3] = [0x21A, 0x219, 0x1C1];
/// Task types samp's anim encoder accepts (B 0x100AE711): 401 named anim, 934.
const TASK_NAMED_ANIM: u32 = 0x191;
const TASK_NAMED_ANIM_2: u32 = 0x3A6;
/// `CTaskSimpleRunNamedAnim` fields read by B 0x100AE600: anim name, group name, delta, time, flags.
const TASK_ANIM_NAME: u32 = 0x10;
const TASK_ANIM_GROUP: u32 = 0x28;
const TASK_ANIM_DELTA: u32 = 0x38;
const TASK_ANIM_TIME: u32 = 0x40;
const TASK_ANIM_FLAGS: u32 = 0x5C;
/// samp's model-anim block poll: 15 × Sleep(1) after the request (B 0x100AD2B3).
const ANIM_LOAD_POLLS: u32 = 15;

fn rd<T: Copy + Default>(a: u32) -> T {
    if a == 0 {
        return T::default();
    }
    let mut buf = [0u8; 16];
    let n = std::mem::size_of::<T>();
    if ProcessMemory.read(a, &mut buf[..n]).is_err() {
        return T::default();
    }
    // SAFETY: `buf` holds `n` initialised bytes of a plain-data type.
    unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) }
}

fn poke<T: Copy>(a: u32, v: T) {
    if a != 0 {
        // SAFETY: game data (heap objects, .data/.bss) of live objects, main thread.
        unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) };
    }
}

fn v3(a: u32) -> [f32; 3] {
    [rd(a), rd(a + 4), rd(a + 8)]
}

fn local_ped() -> u32 {
    rd(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED)
}

fn matrix_pos(entity: u32) -> Option<[f32; 3]> {
    let m: u32 = rd(entity + off::PLACEABLE_MATRIX);
    (m != 0).then(|| v3(m + off::MATRIX_POS))
}

fn entity_live(e: u32) -> bool {
    e != 0 && rd::<u32>(e) != VTBL_CPLACEABLE
}

/// samp's teleport wrapper (B 0x1009F070): the entity's `Teleport` virtual (vtable +0x38) with `false`.
pub(crate) fn teleport_entity(e: u32, p: [f32; 3]) {
    if !entity_live(e) || TRAIN_MODELS.contains(&rd::<u16>(e + off::ENTITY_MODEL)) {
        return;
    }
    let target: u32 = rd(rd::<u32>(e) + off::VT_TELEPORT);
    if target == 0 {
        return;
    }
    // SAFETY: the entity's own virtual (thiscall: CVector by value, bool), main thread.
    unsafe {
        let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
        f(e, p[0], p[1], p[2], 0);
    }
}

/// `CPed` is in a vehicle (flag bit 8 of +0x46C, B 0x100ABE80).
fn in_vehicle(ped: u32) -> bool {
    rd::<u32>(ped + off::PED_FLAGS) & (1 << 8) != 0
}

/// samp's "passenger" test (B 0x100ABFD0): in a vehicle and not its driver, or in a train carriage (569/570).
fn is_passenger(ped: u32) -> bool {
    let veh: u32 = rd(ped + PED_VEHICLE);
    if veh == 0 || !in_vehicle(ped) {
        return false;
    }
    if rd::<u32>(veh + VEH_DRIVER) != ped {
        return true;
    }
    matches!(rd::<u16>(veh + off::ENTITY_MODEL), 0x23A | 0x239)
}

/// A task's virtual (B 0x100B4060 / 0x100B40B0): only for vtables in 0x800000..=0x900000.
fn task_virtual(task: u32, slot: u32) -> u32 {
    if task == 0 {
        return 0;
    }
    let vt: u32 = rd(task);
    if !(0x80_0000..=0x90_0000).contains(&vt) {
        return 0;
    }
    let f: u32 = rd(vt + slot);
    // SAFETY: the task's own GetSubTask (+0x8) / GetTaskType (+0x10) virtual (thiscall), main thread.
    unsafe {
        let g: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(f as usize);
        g(task)
    }
}

/// First task of the primary chain (intelligence +0x10, then sub-tasks) whose type is in `types`
/// (B 0x100AE6F0 for the encoder, B 0x100ADA50 with 401 only for the remote stop).
fn find_task(ped: u32, types: &[u32]) -> u32 {
    let intel: u32 = rd(ped + off::PED_INTELLIGENCE);
    if intel == 0 {
        return 0;
    }
    let mut t: u32 = rd(intel + off::INTELLIGENCE_PRIMARY_TASK);
    while t != 0 {
        if types.contains(&task_virtual(t, 0x10)) {
            return t;
        }
        t = task_virtual(t, 0x8);
    }
    0
}

fn c_str_at(a: u32, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..max as u32 {
        let b: u8 = rd(a + i);
        if b == 0 {
            break;
        }
        out.push(b);
    }
    out
}

/// A static text buffer handed to GTA (it keeps the pointer): the shop name (B 0x1026B260, 32 bytes) and the
/// big-message text (B 0x101501AC → 0x200 bytes).
struct StaticText<const N: usize>(UnsafeCell<[u8; N]>);
// SAFETY: only touched on the game's main thread.
unsafe impl<const N: usize> Sync for StaticText<N> {}
impl<const N: usize> StaticText<N> {
    const fn new() -> Self {
        Self(UnsafeCell::new([0; N]))
    }
    /// Copies `text` (NUL-terminated, at most N − 1 bytes) and returns the buffer address.
    fn set(&self, text: &[u8]) -> u32 {
        // SAFETY: main thread only; GTA reads the buffer between our calls.
        let b = unsafe { &mut *self.0.get() };
        b.fill(0);
        let n = text.len().min(N - 1);
        b[..n].copy_from_slice(&text[..n]);
        b.as_ptr() as u32
    }
}

static SHOP_NAME: StaticText<32> = StaticText::new();
static BIG_MESSAGE: StaticText<0x200> = StaticText::new();

/// `CMessages::AddBigMessage` after `CLEAR_PRINTS` (B 0x100A0D10; only for styles ≤ 200).
fn big_message(runner: &mut Runner, text: &str, ms: u32, style: u32) {
    if style > 200 {
        return;
    }
    runner.run(op::CLEAR_PRINTS, &[]);
    let p = BIG_MESSAGE.set(text.as_bytes());
    // SAFETY: GTA's cdecl message queue entry, main thread, text in a static buffer.
    unsafe {
        let f: extern "C" fn(u32, u32, u32) = std::mem::transmute(ADD_BIG_MESSAGE as usize);
        f(p, ms, style);
    }
}

// ---- the AddAnimation / BlendAnimation recorders (CSV rows 17 / 18) ---------------------------------------------

/// Last recorded call on the local clump (B globals 0x101514D0/D4/D8): group, id, blend delta (samp also keeps
/// the kind, 1 add / 2 blend, at 0x101514CC; the encoder does not read it).
static REC_GROUP: AtomicU32 = AtomicU32::new(0);
static REC_ID: AtomicU32 = AtomicU32::new(0);
static REC_DELTA: AtomicU32 = AtomicU32::new(0);

fn record(clump: u32, group: u32, id: u32, delta: Option<u32>) {
    let ped = local_ped();
    let local_clump: u32 = if ped != 0 { rd(ped + ENTITY_RW_OBJECT) } else { 0 };
    if clump != local_clump || (group == 0 && id == 0xA0) {
        return;
    }
    REC_GROUP.store(group, Ordering::Relaxed);
    REC_ID.store(id, Ordering::Relaxed);
    if let Some(d) = delta {
        REC_DELTA.store(d, Ordering::Relaxed);
    }
}

extern "C" fn record_add(clump: u32, group: u32, id: u32) {
    record(clump, group, id, None);
}

extern "C" fn record_blend(clump: u32, group: u32, id: u32, delta: u32) {
    record(clump, group, id, Some(delta));
}

/// `CAnimManager::AddAnimation(clump, group, id)` entry: record, then the two displaced instructions and the rest.
#[unsafe(naked)]
unsafe extern "C" fn add_animation_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x2C]",
        "push dword ptr [esp+0x2C]",
        "push dword ptr [esp+0x2C]",
        "call {rec}",
        "add esp, 12",
        "popad",
        "mov eax, [esp+0xC]",
        "mov edx, dword ptr [0xB4EA34]",
        "push {cont}",
        "ret",
        rec = sym record_add,
        cont = const 0x4D3AAAu32,
    )
}

/// `CAnimManager::BlendAnimation(clump, group, id, delta)` entry.
#[unsafe(naked)]
unsafe extern "C" fn blend_animation_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "call {rec}",
        "add esp, 16",
        "popad",
        "sub esp, 0x14",
        "mov ecx, [esp+0x18]",
        "push {cont}",
        "ret",
        rec = sym record_blend,
        cont = const 0x4D4617u32,
    )
}

/// CSV rows 198 (below) and 17 / 18: `jmp dword [slot]` at the entries, the handler addresses in the NOP padding before them.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    // CSV row 198: POINT_CAMERA_AT_CHAR always takes the non-player branch (spectating remote CPlayerPeds).
    out.push((198, patcher.write(mem, "POINT_CAMERA_AT_CHAR ped mode", 0x47C477, &[0xEB], Some(&[0x74]))));
    type Row = (u32, u32, u32, &'static [u8], u32, &'static str);
    let rows: [Row; 2] = [
        (
            17,
            0x4D3AA0,
            0x4D3934,
            &[0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x15, 0x34, 0xEA, 0xB4, 0x00],
            add_animation_stub as unsafe extern "C" fn() as usize as u32,
            "CAnimManager::AddAnimation recorder",
        ),
        (
            18,
            0x4D4610,
            0x4D4609,
            &[0x83, 0xEC, 0x14, 0x8B, 0x4C, 0x24, 0x18],
            blend_animation_stub as unsafe extern "C" fn() as usize as u32,
            "CAnimManager::BlendAnimation recorder",
        ),
    ];
    for (row, site, slot, orig, handler, label) in rows {
        let r = patcher.write(mem, label, slot, &handler.to_le_bytes(), Some(&[0x90; 4])).and_then(|()| {
            let mut jmp = vec![0xFF, 0x25];
            jmp.extend_from_slice(&slot.to_le_bytes());
            jmp.resize(orig.len(), 0x90);
            patcher.write(mem, label, site, &jmp, Some(orig))
        });
        out.push((row, r));
    }
    out
}

// ---- state -------------------------------------------------------------------------------------------------------

pub struct Behaviour {
    pub bounds: Bounds,
    /// Local drunk level (local ped object +0x2C9).
    pub drunk: i32,
    rand: bh::MsvcRand,
    /// GetTickCount of the last wobble decision (B 0x1026B284; 0 = never).
    wobble_tick: u32,
    pub spectate: Spectate,
    last_212: Option<Instant>,
    /// Camera attachment (samp's camera object +0): the object id of RPC 81.
    pub attached: Option<u16>,
    /// The 207 animation field (LocalPlayer +0xFC / +0x100).
    pub anim: Latch,
    /// `CKeyGen::GetUppercaseKey` of every record's NAME part (B 0x10149190).
    hashes: Vec<u32>,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl Behaviour {
    pub fn new() -> Self {
        Self {
            bounds: Bounds::default(),
            drunk: 0,
            rand: bh::MsvcRand::default(),
            wobble_tick: 0,
            spectate: Spectate::default(),
            last_212: None,
            attached: None,
            anim: Latch::default(),
            hashes: Vec::new(),
        }
    }

    /// Control-channel line (M8b e2e): samp-side state plus the GTA values the RPCs change.
    pub fn status_line(&self, spawned: bool) -> String {
        const CAM: u32 = 0xB6F028;
        let active = u32::from(rd::<u8>(CAM + 0x59).min(2));
        let ped = local_ped();
        let target = match self.spectate.target {
            SpecTarget::None => "none".to_string(),
            SpecTarget::Player(i) => format!("player:{i}"),
            SpecTarget::Vehicle(i) => format!("vehicle:{i}"),
        };
        let b = &self.bounds;
        let pos = if ped != 0 { matrix_pos(ped).unwrap_or_default() } else { [0.0; 3] };
        let speed = if ped != 0 { v3(ped + off::MOVE_SPEED) } else { [0.0; 3] };
        let cam = v3(CAMERA_MATRIX_POS);
        format!(
            "behaviour spawned={} spectate={} target={target} applied={} mode={} attached={} bounds={},{},{},{} \
             drunk={} anim={:#x} anim_sent={} gravity={} cam_mode={} widescreen={} area={} ped_area={} wanted={} \
             stunt={} money={} shop={:#x} cam={:.2},{:.2},{:.2} pos={:.2},{:.2},{:.2} speed={:.3},{:.3},{:.3} health={:.1} hashes={}",
            u8::from(spawned),
            u8::from(self.spectate.on),
            u8::from(self.spectate.applied),
            self.spectate.mode,
            self.attached.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
            b.max_x,
            b.min_x,
            b.max_y,
            b.min_y,
            self.drunk,
            self.anim.value,
            u8::from(self.anim.sent),
            rd::<f32>(GRAVITY),
            rd::<u16>(CAM + 0x174 + active * 0x238 + 0xC),
            rd::<u8>(CAM + 0x3D),
            rd::<i32>(0xB72914),
            if ped != 0 { rd::<u8>(ped + 0x2F) } else { 0 },
            rd::<u8>(WANTED_HUD),
            rd::<u32>(STUNT_BONUS),
            rd::<i32>(game::PLAYER_MONEY),
            if ped != 0 { rd::<u32>(ped + PED_SHOP_NAME) } else { 0 },
            cam[0],
            cam[1],
            cam[2],
            pos[0],
            pos[1],
            pos[2],
            speed[0],
            speed[1],
            speed[2],
            if ped != 0 { rd::<f32>(ped + off::PED_HEALTH) } else { 0.0 },
            self.hashes.len(),
        )
    }

    /// samp hashes the table at start-up (B 0x1009CF30, called from B 0x100A091F); here once per session.
    pub fn hash_table(&mut self) {
        if !self.hashes.is_empty() {
            return;
        }
        self.hashes = (0..anims::COUNT)
            .map(|i| {
                let Some(name) = anims::name_part(i) else { return 0 };
                let c = CString::new(name).unwrap_or_default();
                // SAFETY: GTA's cdecl string hash on a NUL-terminated string, main thread.
                unsafe {
                    let f: extern "C" fn(*const u8) -> u32 = std::mem::transmute(GET_UPPERCASE_KEY as usize);
                    f(c.as_ptr() as *const u8)
                }
            })
            .collect();
    }

    // ---- InitGame, world state ---------------------------------------------------------------------------------

    /// InitGame (B 0x100109E9 → 0x100088E0, then 0x100A1410): bounds back to ±20000, the gravity applied.
    pub fn on_init_game(&mut self, gravity: f32) {
        self.bounds = Bounds::default();
        set_gravity(gravity);
    }

    // ---- camera ------------------------------------------------------------------------------------------------

    /// RPC 82 (B 0x1001A0A0 → 0x1009D5C0 / 0x1009D630): time ≤ 0 → ignored; the camera counts as "applied" for the
    /// spectator tick (LocalPlayer +0x314).
    pub fn interpolate(&mut self, runner: &mut Runner, position: bool, from: [f32; 3], to: [f32; 3], ms: i32, cut: u8) {
        if ms <= 0 {
            return;
        }
        self.spectate.applied = true;
        let cut = i32::from(bh::camera_cut(cut));
        if position {
            vector_move(runner, from, to, ms, cut);
        } else {
            runner.run(op::CAMERA_PERSIST_TRACK, &[Arg::Int(1)]);
            let mut a: Vec<Arg> = from.iter().chain(to.iter()).map(|&v| Arg::Float(v)).collect();
            a.extend([Arg::Int(ms), Arg::Int(cut)]);
            runner.run(op::CAMERA_SET_VECTOR_TRACK, &a);
        }
    }

    /// RPC 81 (B 0x10019FA0 → 0x1009D690): ids ≤ 1000 of existing objects; snap with a 100 ms move to itself.
    pub fn attach_camera(&mut self, runner: &mut Runner, objects: &crate::objects::Objects, id: u16) {
        if id > 1000 {
            return;
        }
        let Some(ptr) = objects.gta_ptr(id) else { return };
        self.spectate.applied = true;
        self.attached = Some(id);
        self.follow_entity(runner, ptr);
    }

    /// Every frame before `CCamera::Process` (CSV row 9, B 0x100A2640 → 0x1009D6E0).
    pub fn camera_follow(&mut self, runner: &mut Runner, objects: &crate::objects::Objects) {
        let Some(id) = self.attached else { return };
        if let Some(ptr) = objects.gta_ptr(id) {
            self.follow_entity(runner, ptr);
        }
    }

    fn follow_entity(&mut self, runner: &mut Runner, ptr: u32) {
        if ptr == 0 {
            return;
        }
        if let Some(p) = matrix_pos(ptr)
            && bh::valid_position(p)
        {
            vector_move(runner, p, p, 100, 1);
        }
    }

    // ---- spectating (B 0x100041A0, 0x10004260, 0x10004210, tick 0x10006530) ------------------------------------

    /// RPC 124. Returns true when the spawn routine has to run (leaving spectator mode while spectating).
    pub fn toggle_spectating(&mut self, enable: bool) -> bool {
        let spawn = self.spectate.on && !enable;
        self.spectate = Spectate { on: enable, target: SpecTarget::None, applied: false, mode: self.spectate.mode };
        spawn
    }

    /// RPC 126: id < 1004, slot in use, state neither 0 nor 32 (B 0x10004260).
    pub fn spectate_player(&mut self, players: &crate::players::Players, id: u16, mode: u8) {
        self.spectate.mode = mode;
        if id >= 1004 {
            return;
        }
        if players.get(id).is_some_and(|p| p.state != 0 && p.state != crate::remote::state::WASTED) {
            self.spectate.target = SpecTarget::Player(id);
            self.spectate.applied = false;
        }
    }

    /// RPC 127: id < 2000 and the vehicle exists (B 0x10004210).
    pub fn spectate_vehicle(&mut self, vehicles: &crate::vehicles::Vehicles, id: u16, mode: u8) {
        self.spectate.mode = mode;
        if id < 2000 && vehicles.script_handle(id).is_some() {
            self.spectate.target = SpecTarget::Vehicle(id);
            self.spectate.applied = false;
        }
    }

    /// One spectator tick (B 0x10006530): 212 at the on-foot interval (then `after_212`, the 203 attempt), HUD off, the ped parked 20 m above the camera
    /// with 100 health, then the view (once per target).
    #[allow(clippy::too_many_arguments)]
    pub fn spectator_tick(
        &mut self,
        runner: &mut Runner,
        net: &mut NetClient,
        players: &crate::players::Players,
        vehicles: &crate::vehicles::Vehicles,
        interval: Duration,
        now: Instant,
        after_212: impl FnOnce(&mut NetClient),
    ) {
        let ped = local_ped();
        if ped == 0 {
            return;
        }
        let cam = v3(CAMERA_MATRIX_POS);
        if let Ok(s) = game::read_local_sample(&ProcessMemory, ped) {
            let keys = crate::footsync::key_word(&s.pad, s.key_ctx);
            if self.last_212.is_none_or(|t| now.duration_since(t) > interval) {
                self.last_212 = Some(now);
                net.send_sync(&SpectatorSync {
                    left_right: s.pad.left_stick_x,
                    up_down: s.pad.left_stick_y,
                    keys,
                    position: Vec3 { x: cam[0], y: cam[1], z: cam[2] },
                });
                // The spectator 203 is tried only right after a 212 (A 0x10006620).
                after_212(net);
            }
        }
        crate::client::set_hud(false);
        poke(ped + off::PED_HEALTH, bh::SPEC_PED_HEALTH);
        teleport_entity(ped, [cam[0], cam[1], cam[2] + bh::SPEC_PED_ABOVE]);
        if let SpecTarget::Player(id) = self.spectate.target {
            let ok =
                players.get(id).is_some_and(|p| p.ped != 0 && p.state != 0 && p.state != crate::remote::state::WASTED);
            if !ok {
                crate::log!("behaviour: spectated player {id} gone (not streamed in, state 0 or wasted)");
                self.spectate.target = SpecTarget::None;
                self.spectate.applied = false;
            }
        }
        if self.spectate.applied {
            return;
        }
        match self.spectate.target {
            SpecTarget::None => {
                if in_vehicle(ped)
                    && let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten()
                {
                    let w = bh::SPEC_WARP_OUT;
                    runner.run(
                        op::WARP_CHAR_FROM_CAR_TO_COORD,
                        &[Arg::Int(h), Arg::Float(w[0]), Arg::Float(w[1]), Arg::Float(w[2])],
                    );
                }
                self.camera_position(runner, bh::SPEC_NO_TARGET_POS);
                self.camera_look_at(runner, bh::SPEC_NO_TARGET_LOOK, 2);
                self.spectate.applied = true;
            }
            SpecTarget::Player(id) => {
                let Some(p) = players.get(id).filter(|p| p.ped != 0) else { return };
                let Some(h) = game::ped_handle(&ProcessMemory, p.ped).ok().flatten() else { return };
                runner.run(
                    op::POINT_CAMERA_AT_CHAR,
                    &[Arg::Int(h), Arg::Int(i32::from(self.spectate.mode)), Arg::Int(2)],
                );
                self.spectate.applied = true;
                crate::log!("behaviour: spectating player {id} (mode {})", self.spectate.mode);
            }
            SpecTarget::Vehicle(id) => {
                let Some(h) = vehicles.script_handle(id) else { return };
                runner
                    .run(op::POINT_CAMERA_AT_CAR, &[Arg::Int(h), Arg::Int(i32::from(self.spectate.mode)), Arg::Int(2)]);
                self.spectate.applied = true;
                crate::log!("behaviour: spectating vehicle {id} (mode {})", self.spectate.mode);
            }
        }
    }

    /// RPC 157 / samp's camera method B 0x1009D7B0: 0x0925, attachment off, 0x015F.
    pub fn camera_position(&mut self, runner: &mut Runner, p: [f32; 3]) {
        runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.attached = None;
        runner.run(op::SET_FIXED_CAMERA_POSITION, &[p[0], p[1], p[2], 0.0, 0.0, 0.0].map(Arg::Float));
    }

    /// RPC 158 / B 0x1009D810: 0x0925, attachment off, 0x0160 with the cut.
    pub fn camera_look_at(&mut self, runner: &mut Runner, p: [f32; 3], cut: u8) {
        runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.attached = None;
        runner.run(
            op::POINT_CAMERA_AT_POINT,
            &[Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Int(i32::from(bh::camera_cut(cut)))],
        );
    }

    // ---- world bounds (B 0x10003F40 on foot, 0x10003FB0 driving) -----------------------------------------------

    /// On foot, every frame before the 207 (B 0x10007C2E); only outside interiors (GET_AREA_VISIBLE = 0).
    pub fn bounds_on_foot(&mut self, runner: &mut Runner, ped: u32) {
        if area_visible(runner) != 0 {
            return;
        }
        if push_back_entity(ped, &self.bounds) {
            if let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten()
                && !in_vehicle(ped)
            {
                runner.run(op::SET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Int(0)]);
            }
            big_message(runner, bh::BOUNDS_TEXT, bh::BOUNDS_TEXT_MS, bh::BOUNDS_TEXT_STYLE);
        }
    }

    /// Driving (B 0x1000793D): the vehicle of the pool the driver sits in.
    pub fn bounds_driver(&mut self, runner: &mut Runner, vehicles: &crate::vehicles::Vehicles, ped: u32) {
        let veh: u32 = rd(ped + PED_VEHICLE);
        if is_passenger(ped) || veh == 0 {
            return;
        }
        // samp reads the area first, then needs a pool vehicle (B 0x10003FD3).
        if area_visible(runner) != 0 || vehicles.id_of(veh).is_none() {
            return;
        }
        if push_back_entity(veh, &self.bounds) {
            big_message(runner, bh::BOUNDS_TEXT, bh::BOUNDS_TEXT_MS, bh::BOUNDS_TEXT_STYLE);
        }
    }

    // ---- drunk (B 0x100ADE00) ----------------------------------------------------------------------------------

    /// Every alive frame of the local tick (B 0x100077B0).
    pub fn drunk_tick(&mut self, runner: &mut Runner, ped: u32) {
        if self.drunk > bh::DRUNK_THRESHOLD {
            let v = bh::drunk_value(self.drunk);
            runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(v)]);
            if ped != 0 && in_vehicle(ped) && !is_passenger(ped) {
                let veh: u32 = rd(ped + PED_VEHICLE);
                if veh != 0 {
                    let tick = tick_count();
                    if self.wobble_tick == 0 || tick.wrapping_sub(self.wobble_tick) > bh::DRUNK_WOBBLE_MS {
                        let w = bh::drunk_wobble(self.rand.next_rand(), self.drunk);
                        if bh::drunk_moving(v3(veh + off::MOVE_SPEED)) {
                            let t: f32 = rd(veh + TURN_SPEED + 8);
                            poke(veh + TURN_SPEED + 8, t + w);
                        }
                        self.wobble_tick = tick_count();
                    }
                }
            }
            self.drunk -= 1;
        } else {
            if self.drunk > 0 {
                self.drunk -= 1;
            }
            runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(0)]);
        }
    }

    /// Not spawned (B 0x10007A70): a positive level is reset to 0 and the routine runs once (drunkenness 0).
    pub fn drunk_not_spawned(&mut self, runner: &mut Runner) {
        if self.drunk > 0 {
            self.drunk = 0;
            self.drunk_tick(runner, 0);
        }
    }

    // ---- animations ------------------------------------------------------------------------------------------

    /// Every alive frame (B 0x100077EF): the local encoder into the 207 latch.
    pub fn anim_tick(&mut self, ped: u32) {
        let v = self.encode_local(ped);
        self.anim.update(v);
    }

    /// B 0x100AE7A0.
    fn encode_local(&mut self, ped: u32) -> u32 {
        if !entity_live(ped) || rd::<u32>(ped + ENTITY_RW_OBJECT) == 0 {
            return 0;
        }
        let task = find_task(ped, &[TASK_NAMED_ANIM, TASK_NAMED_ANIM_2]);
        if task != 0 {
            let lib = c_str_at(task + TASK_ANIM_GROUP, 16);
            let name = c_str_at(task + TASK_ANIM_NAME, 24);
            let index = anims::index_of(&lib, &name);
            return anims::encode_task(
                index,
                rd(task + TASK_ANIM_DELTA),
                rd(task + TASK_ANIM_FLAGS),
                rd(task + TASK_ANIM_TIME),
            );
        }
        self.hash_table();
        let (group, id) = (REC_GROUP.load(Ordering::Relaxed), REC_ID.load(Ordering::Relaxed));
        // SAFETY: GTA's cdecl static-association lookup, main thread.
        let assoc = unsafe {
            let f: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(GET_ANIM_ASSOCIATION as usize);
            f(group, id)
        };
        let hash = if assoc != 0 { rd::<u32>(rd::<u32>(assoc + 0x10)) } else { 0 };
        let index = anims::index_of_hash(&self.hashes, hash);
        anims::encode_plain(index, f32::from_bits(REC_DELTA.load(Ordering::Relaxed)))
    }

    /// RPC 86 (B 0x1001A900): the local ped, or a streamed-in remote ped (marked as animated by the server).
    pub fn apply_animation(
        &mut self,
        runner: &mut Runner,
        players: &crate::players::Players,
        local_id: Option<u16>,
        a: &ApplyPlayerAnimation,
    ) {
        let ped = if Some(a.player_id) == local_id {
            local_ped()
        } else {
            if a.player_id >= 1004 {
                return;
            }
            let Some(p) = players.get(a.player_id) else { return };
            remote_states(|m| m.entry(a.player_id).or_default().by_rpc = true);
            p.ped
        };
        if ped == 0 {
            return;
        }
        play(runner, ped, &a.name, &a.lib, a.delta, [a.looping, a.lock_x, a.lock_y, a.freeze], a.time as i32);
    }

    /// RPC 87 (B 0x10018530): the ped re-set to its own position (flushes its tasks); a remote loses the RPC mark.
    pub fn clear_animations(&mut self, players: &crate::players::Players, local_id: Option<u16>, player_id: u16) {
        let ped = if Some(player_id) == local_id {
            local_ped()
        } else {
            if player_id >= 1004 {
                return;
            }
            let Some(p) = players.get(player_id) else { return };
            remote_states(|m| m.entry(player_id).or_default().by_rpc = false);
            p.ped
        };
        if ped != 0
            && let Some(p) = matrix_pos(ped)
        {
            teleport_entity(ped, p);
        }
    }

    /// A remote player streamed in or out: its animation state starts over (remote constructor B 0x100165E0).
    pub fn forget_remote(&mut self, id: u16) {
        remote_states(|m| m.remove(&id));
    }

    // ---- ClientCheck (RPC 103, B 0x10011A40) -------------------------------------------------------------------

    /// ClientCheck (RPC 103, A 0x10011A60). Every reply is computed from this client's own state
    /// and memory, with samp's algorithm on the real data: type 2 the entity flags word; 5 the
    /// checksum of gta_sa.exe memory as it is in this process (our patches included); 0x45 the
    /// checksum of this client's own module image (samp_client.dll, the module that answers; samp
    /// reads its own image here); 0x46 / 0x47 of GTA's model info / collision data; 0x48 the clock.
    /// Nothing is replayed or substituted: memory we changed answers as changed.
    pub fn client_check(&mut self, net: &mut NetClient, runner: &mut Runner, r: &ClientCheckRequest) {
        crate::log!(
            "behaviour: ClientCheck request type {:#x} arg {:#x} offset {} count {}",
            r.check_type,
            r.arg,
            r.offset,
            r.count
        );
        if !bh::client_check_in_limits(r.offset, r.count) {
            return;
        }
        let at = |base: u32| base.wrapping_add(u32::from(r.offset));
        let reply = match r.check_type {
            0x02 => {
                let ped = local_ped();
                let (mut arg, mut result) = (r.arg, 0u8);
                if ped != 0 {
                    let veh: u32 = rd(ped + PED_VEHICLE);
                    if in_vehicle(ped) && !is_passenger(ped) {
                        if veh != 0 {
                            arg = rd(veh + ENTITY_CHECK_WORD);
                            result = 2;
                        }
                    } else {
                        arg = rd(ped + ENTITY_CHECK_WORD);
                        result = 1;
                    }
                }
                Some((arg, result))
            }
            0x05 if bh::client_check_exe_range(r.arg) => checksum(at(r.arg), r.count).map(|c| (r.arg, c)),
            0x45 if r.arg <= bh::CLIENT_CHECK_MODULE_LIMIT => {
                checksum(at(crate::dll::module_base().wrapping_add(r.arg)), r.count).map(|c| (r.arg, c))
            }
            0x46 => {
                let mi = model_info(r.arg);
                let c = if mi != 0 { checksum(at(mi), r.count) } else { Some(0) };
                c.map(|c| (r.arg, c))
            }
            0x47 => {
                let mi = model_info(r.arg);
                let c = if mi != 0 { collision_checksum(runner, r.arg as i32, mi, r.offset, r.count) } else { Some(0) };
                c.map(|c| (r.arg, c))
            }
            0x48 => Some((bh::client_check_time_arg(qpc_ms()), 0)),
            _ => None,
        };
        if let Some((arg, result)) = reply {
            crate::log!("behaviour: ClientCheck reply type {:#x} arg {arg:#x} result {result:#04x}", r.check_type);
            net.send_rpc(&ClientCheckResponse { check_type: r.check_type, arg, result }, Reliability::ReliableOrdered);
        }
    }
}

/// `CModelInfo::ms_modelInfoPtrs[model]` for 0 ≤ model ≤ 20000 (samp A 0x100A7A80), else 0.
fn model_info(model: u32) -> u32 {
    if model > game::MODEL_INFO_COUNT - 1 {
        return 0;
    }
    rd(game::MODEL_INFO_PTRS + 4 * model)
}

/// The checksum of `count` bytes at `addr` of this process; `None` (no reply) when they are not
/// readable (samp would fault there).
fn checksum(addr: u32, count: u16) -> Option<u8> {
    if addr == 0 || !crate::dll::readable(addr, usize::from(count)) {
        crate::log!("behaviour: ClientCheck: {addr:#x}+{count} not readable; no reply");
        return None;
    }
    let mut buf = vec![0u8; usize::from(count)];
    ProcessMemory.read(addr, &mut buf).ok()?;
    Some(bh::client_check_sum(&buf))
}

/// Type 0x47 (A 0x10011B4C): the model is requested and loaded when it is not (HAS_MODEL_LOADED polled
/// up to 1000 times, `Sleep(1)` apart), the checksum taken over its collision model (`CBaseModelInfo`
/// +0x14), and the model marked as no longer needed when nothing uses it (+8 reference count 0).
fn collision_checksum(runner: &mut Runner, model: i32, mi: u32, offset: u16, count: u16) -> Option<u8> {
    const MI_COL_MODEL: u32 = 0x14;
    const MI_REF_COUNT: u32 = 8;
    if !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
        runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
        let mut polls = 0;
        while !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            polls += 1;
            if polls >= bh::CLIENT_CHECK_LOAD_POLLS {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut result = 0;
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        let col: u32 = rd(mi + MI_COL_MODEL);
        if col != 0 {
            result = checksum(col.wrapping_add(u32::from(offset)), count)?;
        }
        if rd::<u16>(mi + MI_REF_COUNT) == 0 && runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
        }
    }
    Some(result)
}

/// One stored 207 of a remote player (B 0x100167E4); called from the one-shot apply in players.rs.
pub fn remote_anim(runner: &mut Runner, id: u16, ped: u32, anim: u32) {
    let actions = remote_states(|m| m.entry(id).or_default().step(anim, || find_task(ped, &[TASK_NAMED_ANIM]) != 0));
    for a in actions {
        match a {
            RemoteAction::Play(d) => {
                let Some((lib, name)) = anims::record(i32::from(d.index)).and_then(anims::split) else { continue };
                play(
                    runner,
                    ped,
                    name.as_bytes(),
                    lib.as_bytes(),
                    d.delta,
                    [d.looping, d.lock_x, d.lock_y, d.freeze],
                    d.time,
                );
                // B 0x100AE7E0 → 0x100AD630: the parachute object follows PARACHUTE anims (FEAT-B).
                if lib == "PARACHUTE" {
                    crate::specials::remote_para_anim(runner, id, ped, name);
                }
            }
            RemoteAction::VendEat => play(runner, ped, b"VEND_EAT_P", b"VENDING", 4.0, [false; 4], -1),
            RemoteAction::Stop => play(runner, ped, b"crry_prtial", b"CARRY", 4.0, [false; 4], 0),
        }
    }
}

/// Remote animation state per player (remote +0x1B4 / +0x1C1); shared with the one-shot apply in players.rs.
static REMOTE_ANIMS: std::sync::Mutex<Option<HashMap<u16, RemoteState>>> = std::sync::Mutex::new(None);

fn remote_states<R>(f: impl FnOnce(&mut HashMap<u16, RemoteState>) -> R) -> R {
    let mut g = REMOTE_ANIMS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// `CAMERA_RESET_NEW_SCRIPTABLES`, `CAMERA_PERSIST_POS(1)`, `CAMERA_SET_VECTOR_MOVE` (B 0x1009D5C0).
fn vector_move(runner: &mut Runner, from: [f32; 3], to: [f32; 3], ms: i32, cut: i32) {
    runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
    runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(1)]);
    let mut a: Vec<Arg> = from.iter().chain(to.iter()).map(|&v| Arg::Float(v)).collect();
    a.extend([Arg::Int(ms), Arg::Int(cut)]);
    runner.run(op::CAMERA_SET_VECTOR_MOVE, &a);
}

fn area_visible(runner: &mut Runner) -> i32 {
    runner.run(op::GET_AREA_VISIBLE, &[Arg::Local(0)]);
    runner.local_i32(0) & 0xFF
}

/// The world bounds while the local player is spawned (samp: local player +0xF0), published by the client tick
/// for the `CPlayerPed::ProcessControl` hook ([`bounds_jump`]).
static LOCAL_BOUNDS: std::sync::Mutex<Option<Bounds>> = std::sync::Mutex::new(None);

pub fn publish_bounds(b: Option<Bounds>) {
    *LOCAL_BOUNDS.lock().unwrap_or_else(|e| e.into_inner()) = b;
}

/// samp's local branch of the `CPlayerPed::ProcessControl` hook (B 0x100A2AF9, measured with the effect tracer:
/// `Pads[0]` Square written by samp B+0xA2B70 one frame before `CTaskComplexJump` is created from
/// `PlayerControlZelda`): while the local player is spawned, the local ped is outside the bounds (same strict
/// compares as the push-back, B 0x1009F5F0), the area is 0 (GET_AREA_VISIBLE, B 0x100A0EF0) and the ped is not in a
/// vehicle and stands (flags +0x46C: bit 8 clear, bit 0 or 1 set), the pad gets a fresh jump press
/// (NewState.Square := 0xFF, OldState.Square := 0) before GTA processes the ped. Runs before the special-action
/// masks, like samp.
pub fn bounds_jump(ped: u32) {
    let Some(b) = *LOCAL_BOUNDS.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    let Some(p) = matrix_pos(ped) else { return };
    if !bh::outside(p, &b) {
        return;
    }
    thread_local! {
        static RUNNER: std::cell::RefCell<Runner> = std::cell::RefCell::new(Runner::new());
    }
    if RUNNER.with(|r| area_visible(&mut r.borrow_mut())) != 0 {
        return;
    }
    let flags: u32 = rd(ped + off::PED_FLAGS);
    if flags & 0x100 != 0 || flags & 0x3 == 0 {
        return;
    }
    use crate::combat::pad;
    poke(game::PAD0 + pad::SQUARE as u32, 0xFFi16);
    poke(game::PAD0 + (pad::OLD_STATE + pad::SQUARE) as u32, 0i16);
}

/// B 0x1009F450 on a live entity: speed, then the matrix rows written back with z + 0.04.
fn push_back_entity(e: u32, b: &Bounds) -> bool {
    let m: u32 = rd(e + off::PLACEABLE_MATRIX);
    if !entity_live(e) || m == 0 {
        return false;
    }
    let Some(p) = bh::push_back(v3(m + off::MATRIX_POS), v3(e + off::MOVE_SPEED), b) else { return false };
    for (i, s) in p.speed.iter().enumerate() {
        poke(e + off::MOVE_SPEED + 4 * i as u32, *s);
    }
    poke(m + off::MATRIX_POS + 8, p.z);
    true
}

/// samp's animation player (B 0x100AD240): "SEX" refused; an unloaded library is requested and polled with
/// `Sleep(1)` 15 times, then given up; else `TASK_PLAY_ANIM_NON_INTERRUPTABLE`.
pub(crate) fn play(runner: &mut Runner, ped: u32, name: &[u8], lib: &[u8], delta: f32, flags: [bool; 4], time: i32) {
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
    if anims::refused_library(lib) {
        return;
    }
    if !runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]) {
        runner.run(op::REQUEST_ANIMATION, &[Arg::Bytes(lib)]);
        let mut loaded = runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]);
        let mut polls = 0;
        while !loaded {
            std::thread::sleep(Duration::from_millis(1));
            polls += 1;
            if polls == ANIM_LOAD_POLLS {
                return;
            }
            loaded = runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]);
        }
    }
    let f = |b: bool| Arg::Int(i32::from(b));
    runner.run(
        op::TASK_PLAY_ANIM_NON_INTERRUPTABLE,
        &[
            Arg::Int(h),
            Arg::Bytes(name),
            Arg::Bytes(lib),
            Arg::Float(delta),
            f(flags[0]),
            f(flags[1]),
            f(flags[2]),
            f(flags[3]),
            Arg::Int(time),
        ],
    );
}

// ---- small RPCs ------------------------------------------------------------------------------------------------

/// RPC 146 and InitGame (B 0x100A1410): the global gravity float.
pub fn set_gravity(g: f32) {
    let _ = ProcessMemory.write(GRAVITY, &g.to_le_bytes());
}

/// RPC 133: the HUD's wanted-level immediate.
pub fn set_wanted(level: u8) {
    let _ = ProcessMemory.write(WANTED_HUD, &[level]);
}

/// RPC 104: GTA's stunt-bonus dword.
pub fn set_stunt_bonus(on: bool) {
    let _ = ProcessMemory.write(STUNT_BONUS, &u32::from(on).to_le_bytes());
}

/// RPC 18: `ADD_SCORE(0, amount)`; RPC 20: the negated current money (nothing when it is 0).
pub fn give_money(runner: &mut Runner, amount: i32) {
    runner.run(op::ADD_SCORE, &[Arg::Int(0), Arg::Int(amount)]);
}

pub fn reset_money(runner: &mut Runner) {
    let m: i32 = rd(game::PLAYER_MONEY);
    if m != 0 {
        give_money(runner, m.wrapping_neg());
    }
}

/// RPC 111 (B 0x10018A70): the debug chat line, then `SWITCH_WIDESCREEN`.
pub fn widescreen(runner: &mut Runner, on: u8) {
    crate::ui::print_debug(&format!("Widescreen = {on}"));
    runner.run(op::SWITCH_WIDESCREEN, &[Arg::Int(i32::from(on))]);
}

/// RPC 89 (B 0x100AE0E0): `GIVE_MELEE_ATTACK_TO_CHAR(ped, style, 6)`.
pub fn fighting_style(runner: &mut Runner, ped: u32, style: u8) {
    if let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() {
        runner.run(op::GIVE_MELEE_ATTACK_TO_CHAR, &[Arg::Int(h), Arg::Int(i32::from(style)), Arg::Int(6)]);
    }
}

/// RPC 33 (B 0x10017E00 → 0x100AE390 / 0x100AE310): the ped's shop-name pointer (an empty name clears only its low
/// byte, like samp), then `CShopping::LoadShop` for names of ≤ 8 characters (the empty one included).
pub fn shop_name(name: &[u8]) {
    let ped = local_ped();
    if ped == 0 || game::ped_handle(&ProcessMemory, ped).ok().flatten().is_none() {
        return;
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let name = &name[..end];
    if name.is_empty() {
        poke(ped + PED_SHOP_NAME, 0u8);
    } else if name.len() <= 31 {
        poke(ped + PED_SHOP_NAME, SHOP_NAME.set(name));
    }
    if name.len() <= 8 {
        let c = CString::new(name).unwrap_or_default();
        // SAFETY: GTA's cdecl shop loader with a NUL-terminated name, main thread.
        unsafe {
            let f: extern "C" fn(*const u8) = std::mem::transmute(LOAD_SHOP as usize);
            f(c.as_ptr() as *const u8);
        }
    }
}

/// RPC 90 (B 0x10018800): ground bits (low two of +0x46C) flipped when set, then the move speed written.
pub fn set_velocity(v: [f32; 3]) {
    let ped = local_ped();
    if ped == 0 {
        return;
    }
    let flags: u32 = rd(ped + off::PED_FLAGS);
    if flags & 3 != 0 {
        poke(ped + off::PED_FLAGS, flags ^ 3);
    }
    for (i, s) in v.iter().enumerate() {
        poke(ped + off::MOVE_SPEED + 4 * i as u32, *s);
    }
}

/// RPC 156 (B 0x100199B0 → 0x100AD350 with refresh): `SET_AREA_VISIBLE`, `SET_CHAR_AREA_VISIBLE`, then
/// `REQUEST_COLLISION` at the ped's x, y.
pub fn set_interior(runner: &mut Runner, interior: u8) {
    let ped = local_ped();
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
    let i = i32::from(interior);
    runner.run(op::SET_AREA_VISIBLE, &[Arg::Int(i)]);
    runner.run(op::SET_CHAR_AREA_VISIBLE, &[Arg::Int(h), Arg::Int(i)]);
    if let Some(p) = matrix_pos(ped) {
        runner.run(op::REQUEST_COLLISION, &[Arg::Float(p[0]), Arg::Float(p[1])]);
    }
}

fn tick_count() -> u32 {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

/// samp's ms clock for ClientCheck 0x48 (B 0x1002E7A0): QueryPerformanceCounter · 1000 / frequency.
fn qpc_ms() -> u64 {
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: plain Win32 queries into locals.
    unsafe {
        QueryPerformanceFrequency(&mut f);
        QueryPerformanceCounter(&mut c);
    }
    if f <= 0 { 0 } else { (c as i128 * 1000 / f as i128) as u64 }
}
