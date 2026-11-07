//! The local player's remaining behaviour inside the game (Windows only): camera RPCs and the camera attachment,
//! spectating, world bounds, gravity, interiors, animations (RPC 86/87, the 207 animation field, remote apply),
//! drunk, fighting style, shop name, money/wanted/stunt/velocity, widescreen and ClientCheck.
//!
//! The game-independent rules live in [`crate::behaviour`] and [`crate::anims`]. Spec: docs/re/subsystems/
//! player-state.md, remote-players-sync.md ("212"), remote-control-onfoot.md §4.1, session-misc.md ("103").
//! samp.dll addresses here are build B where only B was read (table: docs/re/addresses/behaviourclient.md).

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

/// `TheCamera.m_mCameraMatrix` translation.
const CAMERA_MATRIX_POS: u32 = 0xB6F9CC;
/// Global gravity float (0.008 by default).
const GRAVITY: u32 = 0x863984;
/// Immediate operand in GTA's HUD code: the displayed wanted level (RPC 133).
const WANTED_HUD: u32 = 0x58DB60;
/// GTA's stunt-bonus enable dword (RPC 104).
const STUNT_BONUS: u32 = 0xA4A474;
/// `CShopping::LoadShop(const char*)` (cdecl; RPC 33).
const LOAD_SHOP: u32 = 0x49BBE0;
/// `CMessages::AddBigMessage(text, time, style)` (cdecl).
const ADD_BIG_MESSAGE: u32 = 0x69F2B0;
/// `CKeyGen::GetUppercaseKey(const char*)` (cdecl; samp hashes its animation table with it).
const GET_UPPERCASE_KEY: u32 = 0x53CF30;
/// `CAnimManager::GetAnimAssociation(group, id)` (cdecl).
const GET_ANIM_ASSOCIATION: u32 = 0x4D3A60;
/// `CPed` shop-name pointer.
const PED_SHOP_NAME: u32 = 0x78C;
/// `CPed::m_pVehicle`, `CVehicle::m_pDriver`, `CVehicle` primary/secondary colours.
const PED_VEHICLE: u32 = 0x58C;
const VEH_DRIVER: u32 = 0x460;
/// `CPhysical::m_vecTurnSpeed`.
const TURN_SPEED: u32 = 0x58;
/// `CEntity::m_pRwObject` (the clump the animation recorders compare).
const ENTITY_RW_OBJECT: u32 = 0x18;
/// `CPhysical::m_nPhysicalFlags`, the word samp reports for ClientCheck type 2.
const ENTITY_CHECK_WORD: u32 = 0x40;
const VTBL_CPLACEABLE: u32 = 0x863C40;
/// Train models samp moves with the mission-train command instead of `Teleport`.
const TRAIN_MODELS: [u16; 3] = [0x21A, 0x219, 0x1C1];
/// Task types samp's animation encoder accepts: 401 (named anim) and 934.
const TASK_NAMED_ANIM: u32 = 0x191;
const TASK_NAMED_ANIM_2: u32 = 0x3A6;
/// `CTaskSimpleRunNamedAnim` fields samp reads: anim name, group name, delta, time, flags.
const TASK_ANIM_NAME: u32 = 0x10;
const TASK_ANIM_GROUP: u32 = 0x28;
const TASK_ANIM_DELTA: u32 = 0x38;
const TASK_ANIM_TIME: u32 = 0x40;
const TASK_ANIM_FLAGS: u32 = 0x5C;
/// samp polls an animation block this many times, `Sleep(1)` apart, after requesting it.
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

/// samp's teleport wrapper: the entity's `Teleport` virtual (vtable +0x38) with `false`.
/// Mirrors samp.dll B 0x1009F070.
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

/// `CPed` is in a vehicle (bit 8 of the ped flags).
fn in_vehicle(ped: u32) -> bool {
    rd::<u32>(ped + off::PED_FLAGS) & (1 << 8) != 0
}

/// samp's "passenger" test: in a vehicle and not its driver, or in a train carriage (569/570).
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

/// A task's virtual, read only for vtables inside gta_sa.exe (0x800000..=0x900000), like samp.
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

/// First task of the primary chain (intelligence +0x10, then sub-tasks) whose type is in `types`. The encoder
/// accepts 401 and 934, the remote stop only 401.
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

/// A static text buffer handed to GTA (it keeps the pointer): the shop name (32 bytes) and the big-message text
/// (0x200 bytes), sized like samp's.
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

/// `CMessages::AddBigMessage` after `CLEAR_PRINTS` (only for styles ≤ 200). Mirrors samp.dll B 0x100A0D10.
pub(crate) fn big_message(runner: &mut Runner, text: &str, ms: u32, style: u32) {
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

// ---- state -------------------------------------------------------------------------------------------------------

pub struct Behaviour {
    pub bounds: Bounds,
    /// Local drunk level (RPC 35).
    pub drunk: i32,
    rand: bh::MsvcRand,
    /// GetTickCount of the last wobble decision (0 = never).
    wobble_tick: u32,
    pub spectate: Spectate,
    last_212: Option<Instant>,
    /// Camera attachment: the object id of RPC 81.
    pub attached: Option<u16>,
    /// The 207 animation field.
    pub anim: Latch,
    /// `CKeyGen::GetUppercaseKey` of every animation record's NAME part.
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

    /// Status line for the end-to-end tests' control channel: samp-side state plus the GTA values the RPCs change.
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

    /// Hashes the animation table (samp does it at start-up; we do it once per session).
    /// Mirrors samp.dll B 0x1009CF30.
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

    /// InitGame: bounds back to ±20000, then the stunt-bonus flag and the gravity applied.
    pub fn on_init_game(&mut self, gravity: f32, stunt_bonus: bool) {
        self.bounds = Bounds::default();
        set_stunt_bonus(stunt_bonus);
        set_gravity(gravity);
    }
}

fn tick_count() -> u32 {
    // SAFETY: plain Win32 query.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

/// samp's ms clock for ClientCheck 0x48: QueryPerformanceCounter · 1000 / frequency. Mirrors samp.dll B 0x1002E7A0.
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

mod animation;
mod bounds;
mod camera;
mod clientcheck;
mod rpcs;

pub(crate) use animation::play;
pub use animation::{install, remote_anim};
pub use bounds::{bounds_jump, publish_bounds};
pub use rpcs::{
    fighting_style, give_money, reset_money, set_gravity, set_interior, set_stunt_bonus, set_velocity, set_wanted,
    shop_name, widescreen,
};
