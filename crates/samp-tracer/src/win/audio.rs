//! BASS calls of the client (audio streams, RPC 41/42), opt-in with `SAMP_TRACER_BASS=1`.
//! The client module's import-table slots for `bass.dll` are pointed at logging wrappers (neither the
//! client's code nor bass.dll is modified), once, from the first hooked call or frame that sees the
//! client module, i.e. before the client's own boot code runs.
//! Each call becomes a `bass` record: function, decoded arguments (`crate::bass`), return value,
//! `dur_ms` when ≥ 1 ms, and `thread` (`main` or `worker`). A call identical to the previous call of the
//! same function is folded into the next record's `repeats_before`, so a polling loop (e.g. samp's
//! volume updates) shows up as one record per change.

use std::sync::atomic::AtomicU32;

use super::*;
use crate::bass::{self, BASS_FNS};

static ENABLED: AtomicBool = AtomicBool::new(false);
static CHECKED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static MAIN_TID: AtomicU32 = AtomicU32::new(0);
static ORIG: [AtomicU32; 17] = [const { AtomicU32::new(0) }; 17];

struct Last {
    key: String,
    repeats: u32,
    first_t: u64,
    last_t: u64,
}

static LAST: Mutex<Vec<Option<Last>>> = Mutex::new(Vec::new());

pub fn init() {
    ENABLED.store(std::env::var("SAMP_TRACER_BASS").is_ok_and(|v| v == "1"), Ordering::Relaxed);
}

/// Called from every hooked call and every frame; cheap after the first success.
pub fn poll() {
    if INSTALLED.load(Ordering::Relaxed) || !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if CHECKED.swap(true, Ordering::AcqRel) {
        // Another thread is installing or a previous attempt found no client yet: retry later.
        if !INSTALLED.load(Ordering::Acquire) {
            CHECKED.store(false, Ordering::Release);
        }
        return;
    }
    install();
}

pub fn on_frame() {
    if MAIN_TID.load(Ordering::Relaxed) == 0 {
        MAIN_TID.store(tid(), Ordering::Relaxed);
    }
    poll();
}

fn client_base() -> Option<u32> {
    for n in ["samp.dll", "samp_client.dll"] {
        let w: Vec<u16> = n.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated wide string.
        let h = unsafe { GetModuleHandleW(w.as_ptr()) };
        if !h.is_null() {
            return Some(h as usize as u32);
        }
    }
    None
}

fn install() {
    let Some(base) = client_base() else {
        CHECKED.store(false, Ordering::Release);
        return;
    };
    let m = ProcMem;
    let mut patched = Vec::new();
    let res = (|| -> Option<()> {
        let pe = base + m.u32(base + 0x3C)?;
        let imp_rva = m.u32(pe + 0x80)?;
        let mut d = base + imp_rva;
        loop {
            let oft = m.u32(d)?;
            let name_rva = m.u32(d + 12)?;
            let ft = m.u32(d + 16)?;
            if name_rva == 0 {
                break;
            }
            let dll = m.cstr(base + name_rva, 64)?;
            if dll.eq_ignore_ascii_case("bass.dll") {
                let names = if oft != 0 { oft } else { ft };
                for i in 0.. {
                    let ent = m.u32(base + names + 4 * i)?;
                    if ent == 0 {
                        break;
                    }
                    if ent & 0x8000_0000 != 0 {
                        continue;
                    }
                    let fname = m.cstr(base + ent + 2, 64)?;
                    let Some(idx) = bass::index_of(&fname) else { continue };
                    let slot = base + ft + 4 * i;
                    let orig = m.u32(slot)?;
                    ORIG[idx].store(orig, Ordering::SeqCst);
                    let mut old = 0u32;
                    // SAFETY: slot is a 4-byte IAT entry of a loaded module; we restore the protection.
                    unsafe {
                        let p = slot as usize as *const c_void;
                        if VirtualProtect(p, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                            continue;
                        }
                        std::ptr::write_volatile(slot as usize as *mut u32, wrappers()[idx] as u32);
                        VirtualProtect(p, 4, old, &mut old);
                    }
                    patched.push(Val::Str(fname));
                }
            }
            d += 20;
        }
        Some(())
    })();
    {
        let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        g.clear();
        g.resize_with(BASS_FNS.len(), || None);
    }
    INSTALLED.store(true, Ordering::Release);
    tlog(&format!("bass: {} import slots redirected (client base {base:#x}, parse {})", patched.len(), res.is_some()));
    emit(record(
        "bass_install",
        None,
        None,
        None,
        Val::obj().with("client_ptr", Val::Hex(base)).with("patched", Val::List(patched)),
    ));
}

fn after(idx: usize, args: &[u32], ret: u64, t0: u64) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let m = ProcMem;
        let t = ms();
        let mut v = bass::render(&m, idx, args, ret);
        let key = v.to_json();
        let main = tid() == MAIN_TID.load(Ordering::Relaxed);
        let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Some(l)) = g.get_mut(idx)
            && l.key == key
        {
            if l.repeats == 0 {
                l.first_t = t;
            }
            l.repeats += 1;
            l.last_t = t;
            return;
        }
        if let Some(Some(l)) = g.get(idx)
            && l.repeats > 0
        {
            v.put("repeats_before", l.repeats);
            v.put("repeat_span_ms", Val::List(vec![Val::from(l.first_t), Val::from(l.last_t)]));
        }
        if let Some(slot) = g.get_mut(idx) {
            *slot = Some(Last { key, repeats: 0, first_t: 0, last_t: 0 });
        }
        drop(g);
        if t > t0 {
            v.put("dur_ms", t - t0);
        }
        v.put("thread", if main { "main" } else { "worker" });
        emit(record("bass", None, None, None, Val::obj().with("call", BASS_FNS[idx].name).with("a", v)));
    }));
}

macro_rules! wrap {
    ($fn:ident, $idx:expr, $r:ty $(, $a:ident)*) => {
        extern "system" fn $fn($($a: u32),*) -> $r {
            let p = ORIG[$idx].load(Ordering::Relaxed) as usize;
            // SAFETY: p is the original bass.dll export taken from this import slot; the wrapper has
            // the same stdcall signature (all arguments are 32-bit stack slots).
            let f: extern "system" fn($(wrap!(@t $a)),*) -> $r = unsafe { std::mem::transmute(p) };
            let t0 = ms();
            let r = f($($a),*);
            after($idx, &[$($a),*], u64::from(r), t0);
            r
        }
    };
    (@t $a:ident) => { u32 };
}

wrap!(w_set_eax, 0, u32, a0, a1, a2, a3);
wrap!(w_set_config, 1, u32, a0, a1);
wrap!(w_set_config_ptr, 2, u32, a0, a1);
wrap!(w_init, 3, u32, a0, a1, a2, a3, a4);
wrap!(w_free, 4, u32);
wrap!(w_stream_free, 5, u32, a0);
wrap!(w_get_tags, 6, u32, a0, a1);
wrap!(w_get_file_pos, 7, u64, a0, a1);
wrap!(w_set_sync, 8, u32, a0, a1, a2, a3, a4, a5);
wrap!(w_play, 9, u32, a0, a1);
wrap!(w_create_url, 10, u32, a0, a1, a2, a3, a4);
wrap!(w_stop, 11, u32, a0);
wrap!(w_apply3d, 12, u32);
wrap!(w_ch_set3d_pos, 13, u32, a0, a1, a2, a3);
wrap!(w_ch_set3d_attr, 14, u32, a0, a1, a2, a3, a4, a5, a6);
wrap!(w_set3d_pos, 15, u32, a0, a1, a2, a3);
wrap!(w_set3d_factors, 16, u32, a0, a1, a2);

/// In `BASS_FNS` order.
fn wrappers() -> [usize; 17] {
    [
    w_set_eax as *const () as usize,
    w_set_config as *const () as usize,
    w_set_config_ptr as *const () as usize,
    w_init as *const () as usize,
    w_free as *const () as usize,
    w_stream_free as *const () as usize,
    w_get_tags as *const () as usize,
    w_get_file_pos as *const () as usize,
    w_set_sync as *const () as usize,
    w_play as *const () as usize,
    w_create_url as *const () as usize,
    w_stop as *const () as usize,
    w_apply3d as *const () as usize,
    w_ch_set3d_pos as *const () as usize,
    w_ch_set3d_attr as *const () as usize,
    w_set3d_pos as *const () as usize,
    w_set3d_factors as *const () as usize,
    ]
}
