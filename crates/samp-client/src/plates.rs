//! Number plates (RPC 123 SetVehiclePlate): each vehicle's plate is a samp-owned D3D texture.
//!
//! GTA's plate-texture creation gets one shared raster for every car, which is never destroyed while the device
//! is alive. [`tick`] paints the default plate `XYZSR998` and then each vehicle's own text into textures;
//! [`substitute`], called from the device proxy's `SetTexture`, swaps the shared raster's texture for the plate
//! of the vehicle being rendered. [`on_lost`] / [`on_reset`] follow D3D device loss.
//! Spec: docs/re/subsystems/vehicles.md, "Number plates".

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::patch::{PatchError, Patcher, ProcessMemory};
use crate::worldhooks::rd;

/// `RwRasterCreate` (1.0 US) and the shared raster's width, height, depth and flags.
const RW_RASTER_CREATE: u32 = 0x7FB230;
const PLATE_RASTER: (i32, i32, i32, i32) = (64, 16, 32, 0x604);
/// The `RwRasterCreate` call in `CCustomCarPlateMgr::CreatePlateTexture`.
const CREATE_PLATE_CALL: u32 = 0x6FDED6;
/// `RwRasterDestroy` entry (hooked).
const RASTER_DESTROY: u32 = 0x7FB020;
/// Pointer slot for the hook's `jmp [slot]`, in padding inside gta_sa.exe (samp's form of the hook).
const RASTER_DESTROY_SLOT: u32 = 0x59C721;
/// Where `RwRasterDestroy` continues after the overwritten prologue.
const RASTER_DESTROY_RESUME: u32 = 0x7FB026;
/// The prologue bytes the hook overwrites.
const RASTER_DESTROY_ORIGINAL: [u8; 6] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x56];
/// `RwD3D9Raster` extension: offset of the D3D texture in a raster.
const RASTER_D3D_TEXTURE: u32 = 0x34;
/// `RwEngineInstance` and its `RwRenderStateSet` slot (+0x20); TEXTUREFILTER (9) := LINEAR (2).
const RW_ENGINE: u32 = 0xC97B24;
const RW_RENDER_STATE_SET: u32 = 0x20;
/// Default plate text.
pub const DEFAULT_TEXT: &[u8] = b"XYZSR998";

static SHARED_RASTER: AtomicU32 = AtomicU32::new(0);
/// While set, `RwRasterDestroy` keeps the shared raster; cleared during a device loss.
static GUARD: AtomicBool = AtomicBool::new(true);

#[derive(Default)]
struct State {
    /// The device is usable (samp keeps this flag in its vehicle pool); off between loss and reset.
    device_ok: bool,
    default_tex: u32,
    /// (vehicle id, CVehicle*) → texture.
    by_vehicle: BTreeMap<(u16, u32), u32>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { device_ok: true, ..State::default() }))
}

fn release(tex: u32) {
    if tex != 0 {
        // SAFETY: a texture this module owns one reference of (IUnknown::Release, slot 2).
        unsafe {
            let vt: u32 = rd(tex);
            let f: extern "system" fn(u32) -> u32 = std::mem::transmute(rd::<u32>(vt + 8) as usize);
            f(tex);
        }
    }
}

/// Replaces the `RwRasterCreate(w, h, depth, flags)` call of plate creation: returns the shared raster, created
/// on the first call.
extern "C" fn create_plate_raster(_w: i32, _h: i32, _d: i32, _flags: i32) -> u32 {
    let r = SHARED_RASTER.load(Ordering::SeqCst);
    if r != 0 {
        return r;
    }
    let (w, h, d, f) = PLATE_RASTER;
    // SAFETY: RenderWare's cdecl RwRasterCreate.
    let r = unsafe {
        let create: extern "C" fn(i32, i32, i32, i32) -> u32 = std::mem::transmute(RW_RASTER_CREATE as usize);
        create(w, h, d, f)
    };
    SHARED_RASTER.store(r, Ordering::SeqCst);
    r
}

extern "C" fn keep_raster(raster: u32) -> u32 {
    u32::from(GUARD.load(Ordering::SeqCst) && raster != 0 && raster == SHARED_RASTER.load(Ordering::SeqCst))
}

/// `RwRasterDestroy(raster)` hook: returns TRUE without destroying the shared raster while the guard is set,
/// else runs the overwritten prologue and continues.
#[unsafe(naked)]
unsafe extern "C" fn raster_destroy_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x24]",
        "call {keep}",
        "add esp, 4",
        "test eax, eax",
        "popad",
        "jz 2f",
        "mov eax, 1",
        "ret",
        "2:",
        "push esi",
        "mov esi, [esp+8]",
        "push esi",
        "push {resume}",
        "ret",
        keep = sym keep_raster,
        resume = const RASTER_DESTROY_RESUME,
    )
}

/// Installs the shared-raster create and destroy hooks; returns (row in docs/re/data/gta-patches-r4.csv, result) for
/// each.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let create = create_plate_raster as extern "C" fn(i32, i32, i32, i32) -> u32 as usize as u32;
    let destroy = raster_destroy_stub as unsafe extern "C" fn() as usize as u32;
    vec![
        (
            59,
            patcher
                .retarget_branch(
                    mem,
                    "CreatePlateTexture RwRasterCreate",
                    CREATE_PLATE_CALL,
                    create,
                    Some(RW_RASTER_CREATE),
                )
                .map(|_| ()),
        ),
        (
            60,
            crate::worldhooks::jmp_indirect(
                patcher,
                mem,
                "RwRasterDestroy plate raster",
                RASTER_DESTROY,
                RASTER_DESTROY_SLOT,
                destroy,
                &RASTER_DESTROY_ORIGINAL,
            ),
        ),
    ]
}

/// Per-frame pass before the game step: default plate first, then each vehicle (pool order) without a texture
/// and with a text. `paint` is the plate painter; vehicles no longer in `list` lose their texture.
/// Mirrors samp.dll 0x1001ED20 (build B).
pub fn tick(list: &[(u16, u32, Vec<u8>)], paint: &mut dyn FnMut(&[u8]) -> u32) {
    with(|s| {
        let gone: Vec<(u16, u32)> =
            s.by_vehicle.keys().filter(|k| !list.iter().any(|(id, p, _)| (*id, *p) == **k)).copied().collect();
        for k in gone {
            if let Some(t) = s.by_vehicle.remove(&k) {
                release(t);
            }
        }
        if !s.device_ok {
            return;
        }
        if s.default_tex == 0 {
            s.default_tex = paint(DEFAULT_TEXT);
        }
        for (id, ptr, text) in list {
            if *ptr == 0 || text.is_empty() || s.by_vehicle.contains_key(&(*id, *ptr)) {
                continue;
            }
            let t = paint(text);
            if t != 0 {
                s.by_vehicle.insert((*id, *ptr), t);
            }
        }
    });
}

/// Device loss: guard and device flag off, every plate texture released (the default plate too, which samp's
/// painter releases with its own objects).
pub fn on_lost() {
    GUARD.store(false, Ordering::SeqCst);
    with(|s| {
        s.device_ok = false;
        for (_, t) in std::mem::take(&mut s.by_vehicle) {
            release(t);
        }
        release(std::mem::take(&mut s.default_tex));
    });
}

/// After the device reset: guard and device flag on again; the next pass repaints.
pub fn on_reset() {
    GUARD.store(true, Ordering::SeqCst);
    with(|s| s.device_ok = true);
}

/// `SetTexture` substitution: for the shared raster's D3D texture, returns the plate of the vehicle being
/// rendered (recorded by the `CVehicle::Render` hook), else the default plate, after setting
/// `RwRenderStateSet(TEXTUREFILTER = 9, LINEAR = 2)`.
pub fn substitute(tex: u32) -> Option<u32> {
    let raster = SHARED_RASTER.load(Ordering::Relaxed);
    if raster == 0 || tex == 0 || rd::<u32>(raster + RASTER_D3D_TEXTURE) != tex {
        return None;
    }
    let veh = crate::vehhooks::render_vehicle();
    let id = crate::vehhooks::try_with_shared(|s| s.by_ptr.get(&veh).map(|i| i.id)).flatten();
    let own = id.and_then(|id| {
        STATE.try_lock().ok().and_then(|g| g.as_ref().and_then(|s| s.by_vehicle.get(&(id, veh)).copied()))
    });
    let t =
        own.or_else(|| STATE.try_lock().ok().and_then(|g| g.as_ref().map(|s| s.default_tex)).filter(|t| *t != 0))?;
    // SAFETY: RenderWare's RwRenderStateSet through the engine instance (cdecl).
    unsafe {
        let engine: u32 = rd(RW_ENGINE);
        let f: extern "C" fn(u32, u32) -> i32 = std::mem::transmute(rd::<u32>(engine + RW_RENDER_STATE_SET) as usize);
        f(9, 2);
    }
    Some(t)
}

/// Diagnostics for the control channel.
pub fn status() -> String {
    with(|s| {
        format!(
            "plates raster={:#x} default={:#x} vehicles={} device_ok={}",
            SHARED_RASTER.load(Ordering::Relaxed),
            s.default_tex,
            s.by_vehicle.len(),
            s.device_ok
        )
    })
}
