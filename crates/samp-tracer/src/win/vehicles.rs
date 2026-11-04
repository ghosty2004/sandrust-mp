//! TRACE-4: vehicles.
//!
//! * `CAutomobile::ProcessControl` is wrapped twice, like `CPlayerPed::ProcessControl` for
//!   remote players: the inline hook at 0x6B1880 (inner) sees the game context the client set
//!   up for the car; once a client module has redirected the CAutomobile vtable slot
//!   (0x871148), an outer wrapper in front of the client's function samples the context before
//!   the client changes it and after it restores it. One `vpc` record per call on a mission
//!   car (samp's cars are mission cars): `pre.before` / `pre.after` (around the client's
//!   function), `in` / `out` (around GTA's), or `orig_called: false` when the client skipped
//!   GTA's function.
//! * `rveh` record per mission car per frame: position, move/turn speed, forward vector,
//!   driver, status, health, flags at the frame start and around `CWorld::Process`.
//!   Records are written only while a client module is loaded.

use super::*;

/// `CAutomobile` vtable 0x871120 slot +0x28 (`ProcessControl`, gta-reversed Automobile.cpp).
const VPC_VTABLE_SLOT: u32 = 0x871148;
const VPC_ORIGINAL: u32 = 0x6B1880;
/// `CPools::ms_pVehiclePool`; element size = sizeof(CHeli) (gta-reversed Heli.h).
const VEHICLE_POOL: u32 = 0xB74494;
const VEHICLE_SIZE: u32 = 0xA18;
/// Camera context samp swaps for remote drivers (remote-control-vehicles.md §2.3).
/// `m_aCams[0].m_bLookingLeft` (0xB6F1A4) and `m_bLookingRight` (0xB6F1A5), read as 2 bytes.
const CAM0_LOOK_LR: u32 = 0xB6F1A4;

/// CVehicle / CPhysical offsets (gta-reversed Vehicle.h / Physical.h, layout counted from
/// the known +0x460 driver and +0x4C0 health).
mod voff {
    pub const PHYS_FLAGS: u32 = 0x40;
    pub const MOVE_SPEED: u32 = 0x44;
    pub const TURN_SPEED: u32 = 0x50;
    pub const VEH_FLAGS: u32 = 0x428;
    pub const DRIVER: u32 = 0x460;
    pub const NUM_PASSENGERS: u32 = 0x484;
    pub const STEER: u32 = 0x494;
    pub const GAS: u32 = 0x49C;
    pub const BRAKE: u32 = 0x4A0;
    pub const CREATED_BY: u32 = 0x4A4;
    pub const HEALTH: u32 = 0x4C0;
    pub const DOOR_LOCK: u32 = 0x4F8;
    pub const PED_TYPE: u32 = 0x598;
}
const CREATED_BY_MISSION: u8 = 2;

fn hex(m: &ProcMem, a: u32, n: usize) -> Val {
    let mut b = vec![0u8; n];
    if m.read(a, &mut b) { Val::Str(b.iter().map(|x| format!("{x:02x}")).collect()) } else { Val::Null }
}

fn vec3(m: &ProcMem, a: u32) -> Val {
    match (m.f32(a), m.f32(a + 4), m.f32(a + 8)) {
        (Some(x), Some(y), Some(z)) => Val::List(vec![x.into(), y.into(), z.into()]),
        _ => Val::Null,
    }
}

fn client_loaded() -> bool {
    !client_ranges().is_empty()
}

fn is_mission_car(m: &ProcMem, v: u32) -> bool {
    m.u8(v + voff::CREATED_BY) == Some(CREATED_BY_MISSION)
}

/// The car's own state.
fn car_state(m: &ProcMem, v: u32) -> Val {
    let mut s = Val::obj();
    if let Some(p) = gta::entity_pos(m, v) {
        s.put("pos", Val::List(p.iter().map(|&f| f.into()).collect()));
    }
    let mat = m.u32(v + gta::off::PLACEABLE_MATRIX).unwrap_or(0);
    if mat != 0 {
        s.put("fwd", vec3(m, mat + 0x10));
    }
    s.put("speed", vec3(m, v + voff::MOVE_SPEED));
    s.put("turn", vec3(m, v + voff::TURN_SPEED));
    s.put("status", m.u8(v + gta::off::ENTITY_TYPE).map(|b| u32::from(b >> 3)));
    s.put("health", m.f32(v + voff::HEALTH));
    s.put("steer", m.f32(v + voff::STEER));
    s.put("gas", m.f32(v + voff::GAS));
    s.put("brake", m.f32(v + voff::BRAKE));
    s.put("phys_flags", m.u32(v + voff::PHYS_FLAGS).map(Val::Hex).unwrap_or(Val::Null));
    s.put("veh_flags", hex(m, v + voff::VEH_FLAGS, 8));
    s.put("door_lock", m.u32(v + voff::DOOR_LOCK));
    s.put("passengers", m.u8(v + voff::NUM_PASSENGERS).map(u32::from));
    let d = m.u32(v + voff::DRIVER).unwrap_or(0);
    if d != 0 {
        s.put("driver", gta::describe_entity(m, d));
        s.put("driver_ped_type", m.u32(d + voff::PED_TYPE));
    }
    s
}

/// Game context (what samp swaps for a remote driver) + the car's state.
fn sample(v: u32) -> Val {
    let m = ProcMem;
    let mut s = Val::obj()
        .with("pif", m.i32(gta::PLAYER_IN_FOCUS))
        .with("pad", super::remote::pad_state(&m))
        .with("cam_mode", m.u16(gta::CAM0_MODE).map(u32::from))
        .with("weapon_mode", m.u16(gta::CAM_WEAPON_MODE).map(u32::from))
        .with("look_lr", hex(&m, CAM0_LOOK_LR, 2))
        .with("front", vec3(&m, gta::CAM0_FRONT));
    s.put("car", car_state(&m, v));
    s
}

// ------------------------------------------------------------- ProcessControl -----

static VPC_CALLER: AtomicU32 = AtomicU32::new(0);
static CLIENT_VPC: AtomicU32 = AtomicU32::new(0);

struct VpcRec {
    veh: u32,
    pre: Option<Val>,
    inner: Option<(String, Val, Val)>,
}

thread_local! {
    static OUTER: std::cell::RefCell<Vec<VpcRec>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn on_vpc_call(r: &Regs) {
    VPC_CALLER.store(ProcMem.u32(r.esp).unwrap_or(0), Ordering::Relaxed);
}

fn wanted(v: u32) -> bool {
    client_loaded() && is_mission_car(&ProcMem, v)
}

fn emit_vpc(rec: VpcRec, outer: Option<String>) {
    let m = ProcMem;
    let mut a = Val::obj().with("veh", gta::describe_entity(&m, rec.veh));
    if let Some(c) = outer {
        a.put("vtable_target", c);
    }
    if let Some(p) = rec.pre {
        a.put("pre", p);
    }
    match rec.inner {
        Some((caller, i, o)) => {
            a.put("orig_caller", caller);
            a.put("in", i);
            a.put("out", o);
        }
        None => a.put("orig_called", false),
    }
    emit(record("vpc", None, None, None, a));
}

/// Continuation of the inline hook at 0x6B1880 (thiscall, no arguments).
pub extern "thiscall" fn vpc_inner_wrap(this: u32) {
    let caller = VPC_CALLER.load(Ordering::Relaxed);
    let in_outer = OUTER.with(|o| o.borrow().last().is_some_and(|r| r.veh == this));
    let want = in_outer || catch_unwind(AssertUnwindSafe(|| wanted(this))).unwrap_or(false);
    let pre = want.then(|| catch_unwind(AssertUnwindSafe(|| sample(this))).ok()).flatten();
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(tramp_of(Special::VehProcessControl) as usize) };
    f(this);
    let Some(pre) = pre else { return };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let inner = (fmt_loc(caller), pre, sample(this));
        if in_outer {
            OUTER.with(|o| {
                if let Some(r) = o.borrow_mut().last_mut() {
                    r.inner = Some(inner);
                }
            });
        } else {
            emit_vpc(VpcRec { veh: this, pre: None, inner: Some(inner) }, None);
        }
    }));
}

/// Our function in the CAutomobile vtable, in front of the client's.
extern "thiscall" fn vpc_outer_wrap(this: u32) {
    let target = CLIENT_VPC.load(Ordering::Relaxed);
    let want = catch_unwind(AssertUnwindSafe(|| wanted(this))).unwrap_or(false);
    if want {
        let pre = catch_unwind(AssertUnwindSafe(|| sample(this))).ok();
        OUTER.with(|o| o.borrow_mut().push(VpcRec { veh: this, pre, inner: None }));
    }
    // SAFETY: the client's ProcessControl replacement, same signature as GTA's.
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(target as usize) };
    f(this);
    if want {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let post = sample(this);
            if let Some(mut r) = OUTER.with(|o| o.borrow_mut().pop()) {
                let p = r.pre.take().unwrap_or(Val::Null);
                r.pre = Some(Val::obj().with("before", p).with("after", post));
                emit_vpc(r, Some(fmt_loc(target)));
            }
        }));
    }
}

fn install_outer() {
    if CLIENT_VPC.load(Ordering::Relaxed) != 0 {
        return;
    }
    let cur = ProcMem.u32(VPC_VTABLE_SLOT).unwrap_or(0);
    let ours = vpc_outer_wrap as extern "thiscall" fn(u32) as usize as u32;
    if cur == 0 || cur == VPC_ORIGINAL || cur == ours || !module_of(cur).is_some_and(|m| m.client) {
        return;
    }
    CLIENT_VPC.store(cur, Ordering::SeqCst);
    let mut old = 0u32;
    // SAFETY: one pointer in the game's vtable (.rdata); main thread at the frame boundary.
    unsafe {
        let p = VPC_VTABLE_SLOT as usize as *mut c_void;
        VirtualProtect(p, 4, PAGE_EXECUTE_READWRITE, &mut old);
        std::ptr::write_volatile(p as *mut u32, ours);
        VirtualProtect(p, 4, old, &mut old);
    }
    emit(record(
        "vtable_wrap",
        None,
        None,
        None,
        Val::obj().with("slot_ptr", Val::Hex(VPC_VTABLE_SLOT)).with("client_target", fmt_loc(cur)),
    ));
    tlog(&format!("CAutomobile::ProcessControl vtable slot: client target {}", fmt_loc(cur)));
}

// ------------------------------------------------------------- per-frame car samples

fn mission_cars(m: &ProcMem) -> Vec<u32> {
    let Some(pool) = m.u32(VEHICLE_POOL).filter(|&p| p != 0) else { return Vec::new() };
    let (Some(objs), Some(map), Some(size)) = (m.u32(pool), m.u32(pool + 4), m.i32(pool + 8)) else {
        return Vec::new();
    };
    let size = size.clamp(0, 0x1000) as u32;
    let mut flags = vec![0u8; size as usize];
    if !m.read(map, &mut flags) {
        return Vec::new();
    }
    flags
        .iter()
        .enumerate()
        .filter(|(_, f)| **f & 0x80 == 0)
        .map(|(i, _)| objs + i as u32 * VEHICLE_SIZE)
        .filter(|&v| is_mission_car(m, v))
        .collect()
}

static RVEH: Mutex<Vec<(u32, Val)>> = Mutex::new(Vec::new());

pub fn sample_stage(at: &str) {
    if !client_loaded() {
        return;
    }
    let m = ProcMem;
    let cars = mission_cars(&m);
    let mut g = RVEH.lock().unwrap_or_else(|e| e.into_inner());
    for v in cars {
        let s = car_state(&m, v);
        match g.iter_mut().find(|(p, _)| *p == v) {
            Some((_, x)) => x.put(at, s),
            None => g.push((v, Val::obj().with(at, s))),
        }
    }
}

pub fn emit_stage() {
    let rows = std::mem::take(&mut *RVEH.lock().unwrap_or_else(|e| e.into_inner()));
    let m = ProcMem;
    for (v, x) in rows {
        let mut a = Val::obj().with("veh", gta::describe_entity(&m, v));
        if let Val::Obj(f) = x {
            for (k, y) in f {
                a.put(&k, y);
            }
        }
        emit(record("rveh", None, None, None, a));
    }
}

/// Called from `on_frame` (main thread, CGame::Process entry).
pub fn frame() {
    install_outer();
    sample_stage("frame_start");
}
