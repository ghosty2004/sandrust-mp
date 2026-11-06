//! TRACE-6: world entities and combat.
//!
//! * `robj` record per client object (samp marks its objects with entity flag bit 31) whenever
//!   its sampled state changes between two frames (position, rotation, move/turn speed, RW
//!   object present, area, flags, attachment): the per-frame trajectory of a moving object,
//!   stream-in/out of its RW object and building removal can be read from it. `robj_gone` when
//!   it leaves the pool.
//! * `orender` record: which objects `CObject::Render` drew in the last frame and from which
//!   GTA code path (first GTA return addresses on the stack), written when that set changes.
//!   GTA's render list goes through `CRenderer`; the client's own passes return straight into
//!   `RenderScene`.
//! * `hp` record when the health/armour/state/weapon of the local ped, a remote player's ped
//!   or another (non-player) ped changes (actors).
//! * `los` record for every `CWorld::ProcessLineOfSight` call made by a client module, with
//!   its result.

use std::collections::BTreeMap;

use super::*;

/// `CPools::ms_pObjectPool`; element size = sizeof(CObject) (gta-reversed Object.h).
const OBJECT_POOL: u32 = 0xB7449C;
const OBJECT_SIZE: u32 = 0x19C;
/// `CPools::ms_pPedPool`; element size = sizeof(CCopPed) (gta-reversed Pools.cpp).
const PED_POOL: u32 = 0xB74490;
const PED_SIZE: u32 = 0x7C4;
/// samp's marker on its own objects (world-entities.md RE-10 §2).
const CLIENT_OBJECT_FLAG: u32 = 0x8000_0000;

mod ooff {
    pub const RW_OBJECT: u32 = 0x18;
    pub const FLAGS: u32 = 0x1C;
    pub const MOVE_SPEED: u32 = 0x44;
    pub const TURN_SPEED: u32 = 0x50;
    /// `CPhysical::m_pAttachedTo` (gta-reversed Physical.h).
    pub const ATTACHED_TO: u32 = 0xFC;
}

/// `CPed::m_aWeapons` (+0x5A0, 13 × 0x1C bytes; gta-reversed Ped.h).
const PED_WEAPONS: u32 = 0x5A0;
const WEAPON_SIZE: u32 = 0x1C;

fn pool_items(m: &ProcMem, pool_ptr: u32, elem: u32) -> Vec<u32> {
    let Some(pool) = m.u32(pool_ptr).filter(|&p| p != 0) else { return Vec::new() };
    let (Some(objs), Some(map), Some(size)) = (m.u32(pool), m.u32(pool + 4), m.i32(pool + 8)) else {
        return Vec::new();
    };
    let size = size.clamp(0, 0x4000) as u32;
    let mut flags = vec![0u8; size as usize];
    if !m.read(map, &mut flags) {
        return Vec::new();
    }
    flags.iter().enumerate().filter(|(_, f)| **f & 0x80 == 0).map(|(i, _)| objs + i as u32 * elem).collect()
}

fn r3(m: &ProcMem, a: u32, scale: f32) -> String {
    let f = |x: Option<f32>| x.map(|v| (v * scale).round() / scale).unwrap_or(f32::NAN);
    format!("{},{},{}", f(m.f32(a)), f(m.f32(a + 4)), f(m.f32(a + 8)))
}

fn object_state(m: &ProcMem, o: u32) -> Option<(String, Val)> {
    let flags = m.u32(o + ooff::FLAGS)?;
    if flags & CLIENT_OBJECT_FLAG == 0 {
        return None;
    }
    let model = m.i16(o + gta::off::ENTITY_MODEL)?;
    let mat = m.u32(o + gta::off::PLACEABLE_MATRIX).unwrap_or(0);
    let (pos, right, fwd, up) = if mat != 0 {
        (r3(m, mat + 0x30, 1000.0), r3(m, mat, 10000.0), r3(m, mat + 0x10, 10000.0), r3(m, mat + 0x20, 10000.0))
    } else {
        (r3(m, o + gta::off::PLACEABLE_PLACEMENT, 1000.0), String::new(), String::new(), String::new())
    };
    let rw = m.u32(o + ooff::RW_OBJECT).unwrap_or(0) != 0;
    let area = m.u8(o + gta::off::ENTITY_AREA).unwrap_or(0);
    let mv = r3(m, o + ooff::MOVE_SPEED, 100_000.0);
    let tn = r3(m, o + ooff::TURN_SPEED, 100_000.0);
    let att = m.u32(o + ooff::ATTACHED_TO).unwrap_or(0);
    let key = format!("{model}|{pos}|{right}|{fwd}|{up}|{rw}|{area}|{flags:x}|{mv}|{tn}|{att:x}");
    let v = Val::obj()
        .with("obj", Val::Hex(o))
        .with("model", i32::from(model))
        .with("pos", pos)
        .with("right", right)
        .with("fwd", fwd)
        .with("up", up)
        .with("rw", rw)
        .with("area", u32::from(area))
        .with("flags", Val::Hex(flags))
        .with("move", mv)
        .with("turn", tn)
        .with("attached", gta::describe_entity(m, att))
        .with("timestep", m.f32(gta::TIME_STEP));
    Some((key, v))
}

static OBJ_PREV: Mutex<BTreeMap<u32, String>> = Mutex::new(BTreeMap::new());

fn sample_objects(m: &ProcMem) {
    let mut prev = OBJ_PREV.lock().unwrap_or_else(|e| e.into_inner());
    let mut cur = BTreeMap::new();
    for o in pool_items(m, OBJECT_POOL, OBJECT_SIZE) {
        if let Some((key, v)) = object_state(m, o) {
            if prev.get(&o) != Some(&key) {
                emit(record("robj", None, None, None, v));
            }
            cur.insert(o, key);
        }
    }
    for (o, k) in prev.iter() {
        if !cur.contains_key(o) {
            let model = k.split('|').next().unwrap_or("");
            emit(record("robj_gone", None, None, None, Val::obj().with("obj", Val::Hex(*o)).with("model", model)));
        }
    }
    *prev = cur;
}

// ------------------------------------------------------------- object render paths -----

/// `hideped on|off` (control channel): `CPed::Render` returns without drawing for the local player's ped.
pub static HIDE_LOCAL_PED: AtomicBool = AtomicBool::new(false);

/// Continuation of the inline hook on `CPed::Render` (thiscall, no arguments).
pub extern "thiscall" fn ped_render_wrap(this: u32) {
    if HIDE_LOCAL_PED.load(Ordering::Relaxed) && this != 0 && this == gta::player_ped(&ProcMem) {
        return;
    }
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(tramp_of(Special::PedRender) as usize) };
    f(this);
}

static RENDERS: Mutex<BTreeMap<(u32, &'static str, String), u32>> = Mutex::new(BTreeMap::new());
static RENDER_PREV: Mutex<(String, u64)> = Mutex::new((String::new(), 0));

pub fn on_obj_render(spec: &HookSpec, r: &Regs) {
    let m = ProcMem;
    let (ent, tag) = match spec.addr {
        0x553260 => (m.u32(r.esp + 4).unwrap_or(0), "nonroad"),
        0x534310 => (r.ecx, "entity"),
        _ => (r.ecx, "object"),
    };
    // Objects only (entity type 4).
    if ent == 0 || m.u8(ent + gta::off::ENTITY_TYPE).map(|t| t & 7) != Some(4) {
        return;
    }
    // Return addresses above the hooked function: which code asked for the draw (the
    // first one may be a client module's, then GTA's).
    let mut path = String::new();
    let ret = m.u32(r.esp).unwrap_or(0);
    if let Some(c) = module_of(ret).filter(|c| c.client) {
        path = format!("{}+{:#x} < ", c.name, ret - c.base);
    }
    path.push_str(&gta_stack(r.esp + 4, 2).unwrap_or_default());
    let mut g = RENDERS.lock().unwrap_or_else(|e| e.into_inner());
    *g.entry((ent, tag, path)).or_insert(0) += 1;
}

fn emit_renders(m: &ProcMem, n: u64) {
    let rows = std::mem::take(&mut *RENDERS.lock().unwrap_or_else(|e| e.into_inner()));
    let mut list = Vec::new();
    let mut key = String::new();
    for ((o, tag, path), count) in rows {
        let flags = m.u32(o + ooff::FLAGS).unwrap_or(0);
        let model = m.i16(o + gta::off::ENTITY_MODEL).unwrap_or(-1);
        let client = flags & CLIENT_OBJECT_FLAG != 0;
        key.push_str(&format!("{o:x}/{tag}/{model}/{path}/{count};"));
        list.push(
            Val::obj()
                .with("obj", Val::Hex(o))
                .with("model", i32::from(model))
                .with("client_obj", client)
                .with("fn", tag)
                .with("path", path)
                .with("count", count),
        );
    }
    let mut prev = RENDER_PREV.lock().unwrap_or_else(|e| e.into_inner());
    if prev.0 != key || n >= prev.1 + 500 {
        prev.0 = key;
        prev.1 = n;
        emit(record("orender", None, None, None, Val::obj().with("draws", Val::List(list))));
    }
}

// ------------------------------------------------------------- health / weapons -------

static HP_PREV: Mutex<BTreeMap<u32, String>> = Mutex::new(BTreeMap::new());

fn ped_state(m: &ProcMem, p: u32) -> (String, Val) {
    let hp = m.f32(p + gta::off::PED_HEALTH).unwrap_or(f32::NAN);
    let ar = m.f32(p + gta::off::PED_ARMOUR).unwrap_or(f32::NAN);
    let st = m.u32(p + gta::off::PED_STATE).unwrap_or(0);
    let slot = u32::from(m.u8(p + gta::off::PED_WEAPON_SLOT).unwrap_or(0));
    let w = p + PED_WEAPONS + slot.min(12) * WEAPON_SIZE;
    let (wt, ws, clip, ammo) = (m.i32(w), m.i32(w + 4), m.i32(w + 8), m.i32(w + 12));
    let key = format!("{:.1}|{:.1}|{st}|{slot}|{wt:?}|{ws:?}|{clip:?}|{ammo:?}", hp, ar);
    let v = Val::obj()
        .with("ped", gta::describe_entity(m, p))
        .with("health", (hp * 1000.0).round() / 1000.0)
        .with("armour", (ar * 1000.0).round() / 1000.0)
        .with("state", st)
        .with("slot", slot)
        .with("wtype", wt)
        .with("wstate", ws)
        .with("clip", clip)
        .with("ammo", ammo)
        .with(
            "pos",
            gta::entity_pos(m, p)
                .map(|q| Val::List(q.iter().map(|&f| ((f * 100.0).round() / 100.0).into()).collect()))
                .unwrap_or(Val::Null),
        );
    (key, v)
}

fn sample_peds(m: &ProcMem) {
    let local = gta::player_ped(m);
    let mut peds: Vec<u32> = Vec::new();
    if local != 0 {
        peds.push(local);
    }
    for (_, p) in gta::scan_cached_remote() {
        if p != 0 && !peds.contains(&p) {
            peds.push(p);
        }
    }
    // Other peds (actors): every live ped in the pool that is not a player.
    for p in pool_items(m, PED_POOL, PED_SIZE) {
        if !peds.contains(&p) {
            peds.push(p);
        }
    }
    let mut prev = HP_PREV.lock().unwrap_or_else(|e| e.into_inner());
    let mut cur = BTreeMap::new();
    for p in peds {
        let (key, v) = ped_state(m, p);
        if prev.get(&p) != Some(&key) {
            emit(record("hp", None, None, None, v));
        }
        cur.insert(p, key);
    }
    *prev = cur;
}

// ------------------------------------------------------------- line of sight ----------

struct LosCall {
    caller: String,
    stack: Option<String>,
    pif: Option<u8>,
}

thread_local! {
    static LOS: std::cell::RefCell<Option<LosCall>> = const { std::cell::RefCell::new(None) };
}

pub fn on_los_call(r: &Regs) {
    let m = ProcMem;
    let ret = m.u32(r.esp).unwrap_or(0);
    let pending = module_of(ret).filter(|c| c.client).map(|c| LosCall {
        caller: format!("{}+{:#x}", c.name, ret - c.base),
        stack: gta_stack(r.esp + 4, 3),
        pif: m.u8(gta::PLAYER_IN_FOCUS),
    });
    LOS.with(|l| *l.borrow_mut() = pending);
}

type LosFn = extern "C" fn(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32;
pub type LosWrapFn = LosFn;

/// Continuation of the inline hook at 0x56BA00 (cdecl, 12 arguments, bool result in AL).
#[allow(clippy::too_many_arguments)]
pub extern "C" fn los_wrap(
    start: u32,
    end: u32,
    colpoint: u32,
    out_entity: u32,
    a4: u32,
    a5: u32,
    a6: u32,
    a7: u32,
    a8: u32,
    a9: u32,
    a10: u32,
    a11: u32,
) -> u32 {
    let pending = LOS.with(|l| l.borrow_mut().take());
    let m = ProcMem;
    let pre = pending.as_ref().map(|_| (hooks::vec3(&m, start), hooks::vec3(&m, end)));
    // SAFETY: the trampoline runs the original prologue and jumps back into the function.
    let f: LosFn = unsafe { std::mem::transmute(tramp_of(Special::Los) as usize) };
    let res = f(start, end, colpoint, out_entity, a4, a5, a6, a7, a8, a9, a10, a11);
    if let (Some(c), Some((s, e))) = (pending, pre) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let hit = res & 0xFF != 0;
            let flags: String =
                [a4, a5, a6, a7, a8, a9, a10, a11].iter().map(|v| if v & 0xFF != 0 { '1' } else { '0' }).collect();
            let ent = m.u32(out_entity).unwrap_or(0);
            let mut a = Val::obj()
                .with("start", s)
                .with("end", e)
                .with("flags", flags)
                .with("hit", hit)
                .with("pif", c.pif.map(u32::from));
            if hit {
                a.put("entity", gta::describe_entity(&m, ent));
                a.put("col", hooks::colpoint(&m, colpoint));
            }
            if let Some(st) = c.stack {
                a.put("gta_stack", st);
            }
            emit(record("los", Some(c.caller), None, None, a));
        }));
    }
    res
}

// ------------------------------------------------------------- ped bones (opt-in) ------
//
// `SAMP_TRACER_BONES=2,6,…` (`ePedNode` ids, = SA:MP bone ids 1..18): per frame, a `bones`
// record per player ped with the world LTM of those bones (the RpHAnim hierarchy matrix of
// `CPed::m_apBones[id]`'s node), to check attached-object placement (MR-6.4).

/// `GetAnimHierarchyFromSkinClump(RpClump*)` (gta-reversed AnimManager / plugin-sdk).
const GET_ANIM_HIERARCHY_FROM_SKIN_CLUMP: u32 = 0x734A40;
/// `CPed::m_apBones` (19 `AnimBlendFrameData*`; gta-reversed Ped.h).
const PED_BONES: u32 = 0x488;

static BONE_IDS: OnceLock<Vec<u32>> = OnceLock::new();

fn bone_ids() -> &'static [u32] {
    BONE_IDS.get_or_init(|| {
        std::env::var("SAMP_TRACER_BONES")
            .ok()
            .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).filter(|&x| (1..19).contains(&x)).collect())
            .unwrap_or_default()
    })
}

fn matrix(m: &ProcMem, a: u32) -> Val {
    let row = |o: u32| {
        Val::List((0..3).map(|j| m.f32(a + o + 4 * j).map(|f| (f * 10000.0).round() / 10000.0).into()).collect())
    };
    Val::obj().with("r0", row(0)).with("r1", row(0x10)).with("r2", row(0x20)).with("pos", row(0x30))
}

fn sample_bones(m: &ProcMem) {
    let ids = bone_ids();
    if ids.is_empty() {
        return;
    }
    let mut peds = vec![gta::player_ped(m)];
    peds.extend(gta::scan_cached_remote().into_iter().map(|(_, p)| p));
    for p in peds.into_iter().filter(|&p| p != 0) {
        let clump = m.u32(p + 0x18).unwrap_or(0);
        if clump == 0 {
            continue;
        }
        // SAFETY: GTA's own cdecl helper, called on the game thread at the frame boundary with
        // the ped's live clump.
        let f: extern "C" fn(u32) -> u32 = unsafe { std::mem::transmute(GET_ANIM_HIERARCHY_FROM_SKIN_CLUMP as usize) };
        let hier = f(clump);
        if hier == 0 {
            continue;
        }
        let (n, mats, info) = (
            m.i32(hier + 4).unwrap_or(0).clamp(0, 64) as u32,
            m.u32(hier + 8).unwrap_or(0),
            m.u32(hier + 0x10).unwrap_or(0),
        );
        let mut out = Val::obj()
            .with("ped", gta::describe_entity(m, p))
            .with("ped_matrix", matrix(m, m.u32(p + 0x14).unwrap_or(0)));
        for &id in ids {
            let fd = m.u32(p + PED_BONES + 4 * id).unwrap_or(0);
            let node = m.u32(fd + 0x14).unwrap_or(u32::MAX);
            // RpHAnimIDGetIndex: position of the node id in pNodeInfo (16-byte records, id first).
            let idx = (0..n).find(|&i| m.u32(info + 16 * i) == Some(node));
            if let Some(i) = idx {
                out.put(&format!("b{id}"), matrix(m, mats + 64 * i));
            }
        }
        emit(record("bones", None, None, None, out));
    }
}

// ------------------------------------------------------------- watched models (opt-in) --
//
// `SAMP_TRACER_WATCH_MODELS=4019,…`: every 25 frames the building (0xB74498), dummy (0xB744A0)
// and object pools are scanned for entities of those models; a `wmodel` record is written when
// one appears, disappears or changes area / position (RemoveBuildingForPlayer, MR-6.5).

const BUILDING_POOL: u32 = 0xB74498;
const DUMMY_POOL: u32 = 0xB744A0;
const BUILDING_SIZE: u32 = 0x38;

static WATCH_MODELS: OnceLock<Vec<i16>> = OnceLock::new();
static WATCH_PREV: Mutex<BTreeMap<u32, String>> = Mutex::new(BTreeMap::new());

fn watch_models() -> &'static [i16] {
    WATCH_MODELS.get_or_init(|| {
        std::env::var("SAMP_TRACER_WATCH_MODELS")
            .ok()
            .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    })
}

fn sample_watched(m: &ProcMem, n: u64) {
    let models = watch_models();
    if models.is_empty() || !n.is_multiple_of(25) {
        return;
    }
    let mut cur = BTreeMap::new();
    for (pool, size, kind) in [
        (BUILDING_POOL, BUILDING_SIZE, "building"),
        (DUMMY_POOL, BUILDING_SIZE, "dummy"),
        (OBJECT_POOL, OBJECT_SIZE, "object"),
    ] {
        for e in pool_items(m, pool, size) {
            let Some(model) = m.i16(e + gta::off::ENTITY_MODEL) else { continue };
            if !models.contains(&model) {
                continue;
            }
            let mat = m.u32(e + gta::off::PLACEABLE_MATRIX).unwrap_or(0);
            let pos = if mat != 0 { r3(m, mat + 0x30, 100.0) } else { String::new() };
            let placement = r3(m, e + gta::off::PLACEABLE_PLACEMENT, 100.0);
            let area = m.u8(e + gta::off::ENTITY_AREA).unwrap_or(0);
            let rw = m.u32(e + 0x18).unwrap_or(0) != 0;
            cur.insert(e, format!("{kind}|{model}|{area}|{placement}|{pos}|{rw}"));
        }
    }
    let mut prev = WATCH_PREV.lock().unwrap_or_else(|e| e.into_inner());
    for (e, k) in &cur {
        if prev.get(e) != Some(k) {
            emit(record("wmodel", None, None, None, Val::obj().with("ent", Val::Hex(*e)).with("state", k.as_str())));
        }
    }
    for (e, k) in prev.iter() {
        if !cur.contains_key(e) {
            emit(record(
                "wmodel_gone",
                None,
                None,
                None,
                Val::obj().with("ent", Val::Hex(*e)).with("state", k.as_str()),
            ));
        }
    }
    *prev = cur;
}

/// Called from `on_frame` (main thread, CGame::Process entry).
pub fn frame(n: u64) {
    if !vehicles_client_loaded() {
        return;
    }
    let m = ProcMem;
    sample_objects(&m);
    emit_renders(&m, n);
    sample_peds(&m);
    sample_bones(&m);
    sample_watched(&m, n);
}

fn vehicles_client_loaded() -> bool {
    MODULES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|m| m.client)
}
