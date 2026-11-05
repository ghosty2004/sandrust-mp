//! Number plates (CSV rows 59/60, RPC 123; vehicles.md "Number plates"): one samp-owned D3D texture per vehicle.
//!
//! Static source (samp.dll B): row 59 (B 0x100A4530) replaces the `RwRasterCreate` call in
//! `CCustomCarPlateMgr::CreatePlateTexture` (0x6FDED6) and hands every car one shared raster (64×16, depth 32,
//! flags 0x604, created once); row 60 (B 0x100A4440, `jmp [0x59C721]` at `RwRasterDestroy`) refuses to destroy that
//! raster while the guard (B 0x10113AD8) is set — it is cleared during a device loss. The pre-step pool pass
//! (B 0x1001ED20, only while the pool's device flag +0x17894 is set) paints the default plate `XYZSR998` once and
//! then, per vehicle without a texture and with a non-empty text, its own plate (B 0x100B81B0 → painter
//! B 0x1006D840). The device proxy's `SetTexture` (B 0x10074B80) swaps the shared raster's texture for the texture
//! of the vehicle `CVehicle::Render` is drawing (row 62 records it), else the default one, after
//! `RwRenderStateSet(TEXTUREFILTER, LINEAR)`.
//!
//! Measured (run `orig-20261105-042300`, marks `plate` / `plate2`): the painter's D3D calls (D3DXCreateTexture
//! 64×32 X8R8G8B8 default pool, RTS BeginScene, Clear 0xFFBEB6A8, four outline DrawTexts 0xB4F0EEE4 at
//! (0,−2)/(+2,0)/(0,+2)/(−2,0) around (0, 3) then the text 0xEE444470, rect right/bottom 128/32, format 0x105,
//! EndScene, D3DXLoadSurfaceFromSurface) on the frame after the stream-in, and "RUST 42" on the car.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::patch::{PatchError, Patcher, ProcessMemory};
use crate::worldhooks::rd;

/// `RwRasterCreate` (1.0 US) and the shared raster's size / depth / flags (B 0x100A4504…).
const RW_RASTER_CREATE: u32 = 0x7FB230;
const PLATE_RASTER: (i32, i32, i32, i32) = (64, 16, 32, 0x604);
/// Row 59 call site and row 60 site / slot / resume.
const CREATE_PLATE_CALL: u32 = 0x6FDED6;
const RASTER_DESTROY: u32 = 0x7FB020;
const RASTER_DESTROY_SLOT: u32 = 0x59C721;
const RASTER_DESTROY_RESUME: u32 = 0x7FB026;
const RASTER_DESTROY_ORIGINAL: [u8; 6] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x56];
/// `RwD3D9Raster` extension: the D3D texture at raster + 0x34 (B 0x100A4480).
const RASTER_D3D_TEXTURE: u32 = 0x34;
/// `RwEngineInstance` and its `RwRenderStateSet` slot (+0x20); TEXTUREFILTER (9) := LINEAR (2).
const RW_ENGINE: u32 = 0xC97B24;
const RW_RENDER_STATE_SET: u32 = 0x20;
/// Default plate text (B 0x100E645C).
pub const DEFAULT_TEXT: &[u8] = b"XYZSR998";

static SHARED_RASTER: AtomicU32 = AtomicU32::new(0);
/// B 0x10113AD8: 1 except during a device loss.
static GUARD: AtomicBool = AtomicBool::new(true);

#[derive(Default)]
struct State {
    /// The pool's device flag (+0x17894).
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

/// Row 59: `RwRasterCreate(w, h, depth, flags)` → the shared raster (created on the first call).
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

/// Row 60: `RwRasterDestroy(raster)` returns TRUE without destroying the shared raster while the guard is set,
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

/// The pre-step pool pass (B 0x1001ED20): default plate first, then each vehicle (pool order) without a texture
/// and with a text. `paint` is the plate painter (B 0x1006D840); vehicles no longer in `list` lose their texture.
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

/// Device loss (B 0x100C4171…): guard and pool flag off, every vehicle's texture released (B 0x1001ED90); the
/// painter releases the default plate with its own objects (B 0x1006D770).
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

/// After the reset (B 0x100C440D…): guard and pool flag on again; the next pass repaints.
pub fn on_reset() {
    GUARD.store(true, Ordering::SeqCst);
    with(|s| s.device_ok = true);
}

/// `SetTexture` substitution (B 0x10074BC3…): for the shared raster's D3D texture, the texture of the vehicle being
/// rendered (row 62), else the default plate, after `RwRenderStateSet(9, 2)`.
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
