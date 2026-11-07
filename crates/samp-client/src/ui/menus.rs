//! Server menus (ShowMenu), drawn by GTA's own menu system (Windows only).
//!
//! [`build`] creates the GTA menu with script commands in the same order as samp; [`pad_buttons`] reads the
//! pad state for the per-frame menu poll; [`install_gxt_hook`] hooks `CText::Get` so GTA's menu finds the
//! row and header texts under `SAMP…` keys. Spec: docs/re/subsystems/ui-chat-audio.md, "76 PlayerInitMenu"
//! to "132 OnPlayerSelectedMenuRow"; addresses in docs/re/addresses/ui-menus.md.

use samp_ui::menu::MenuBuild;

use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, Runner};

const OP_CREATE_MENU: u16 = 0x08D4;
const OP_SET_MENU_COLUMN: u16 = 0x08DB;
const OP_SET_MENU_COLUMN_WIDTH: u16 = 0x09DB;
const OP_ACTIVATE_MENU_ITEM: u16 = 0x08D9;
/// `CPad::GetPad(0)`; new state `ButtonCross` +0x20, `ButtonTriangle` +0x1E.
const PAD0: u32 = 0xB73458;

/// Runs the build sequence; returns the GTA menu handle. Mirrors samp.dll 0x100A7780.
pub fn build(r: &mut Runner, b: &MenuBuild) -> Option<i32> {
    r.run(
        OP_CREATE_MENU,
        &[
            Arg::Text(b.title),
            Arg::Float(b.x),
            Arg::Float(b.y),
            Arg::Float(b.width),
            Arg::Int(b.columns),
            Arg::Int(b.interactive),
            Arg::Int(1),
            Arg::Int(1),
            Arg::Local(0),
        ],
    );
    let h = r.local_i32(0);
    for c in &b.column_builds {
        let mut args = vec![Arg::Int(h), Arg::Int(i32::from(c.column)), Arg::Text(c.header)];
        args.extend(c.rows.iter().copied().map(Arg::Text));
        r.run(OP_SET_MENU_COLUMN, &args);
        r.run(OP_SET_MENU_COLUMN_WIDTH, &[Arg::Int(h), Arg::Int(i32::from(c.column)), Arg::Int(c.width)]);
    }
    for (row, &en) in b.rows_enabled.iter().enumerate() {
        r.run(OP_ACTIVATE_MENU_ITEM, &[Arg::Int(h), Arg::Int(row as i32), Arg::Int(en)]);
    }
    crate::log!("ui: menu built (handle {h}, {} column(s))", b.columns);
    Some(h)
}

/// (cross, triangle) of pad 0's new state, polled every frame while a menu is shown.
/// Mirrors samp.dll 0x10008200.
pub fn pad_buttons() -> (bool, bool) {
    let m = ProcessMemory;
    let w = |o: u32| m.read_u16(PAD0 + o).unwrap_or(0) != 0;
    (w(0x20), w(0x1E))
}

// ---- CText::Get hook (docs/re/subsystems/gta-patches.md, "HUD / menus / rendering") ----------

use std::sync::Mutex;
use std::sync::atomic::AtomicU32;

use samp_ui::menu::Menu;

/// `CText::Get` (thiscall, key on the stack); its first 9 bytes are replaced by
/// `jmp [0x6A0043]` and the handler address is stored at 0x6A0043 (like samp).
const CTEXT_GET: u32 = 0x6A0050;
const CTEXT_GET_SLOT: u32 = 0x6A0043;
const CTEXT_GET_ORIGINAL: [u8; 9] = [0x83, 0xEC, 0x20, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x2C];
/// Where the original continues after the replaced prologue.
const CTEXT_GET_CONTINUE: u32 = 0x6A0059;

/// NUL-terminated texts of the current menu by key (after `SAMP`).
type GxtEntry = ([u8; 8], Box<[u8; 40]>);
static GXT: Mutex<Vec<GxtEntry>> = Mutex::new(Vec::new());
static RESULT: AtomicU32 = AtomicU32::new(0);

/// Publishes the current menu's texts (called when a menu is shown / hidden).
pub fn set_gxt(menu: Option<&Menu>) {
    let mut g = GXT.lock().unwrap_or_else(|e| e.into_inner());
    g.clear();
    let Some(m) = menu else { return };
    let mut keys: Vec<&str> = vec!["HED", "RW1", "RW2"];
    for c in 0..2 {
        for k in samp_ui::menu::CELL_KEYS[c] {
            keys.push(&k[4..]);
        }
    }
    for k in keys {
        if let Some(t) = m.gxt(k.as_bytes()) {
            let mut key = [0u8; 8];
            key[..k.len()].copy_from_slice(k.as_bytes());
            let mut buf = Box::new([0u8; 40]);
            let n = t.len().min(39);
            buf[..n].copy_from_slice(&t[..n]);
            g.push((key, buf));
        }
    }
}

/// cdecl lookup for the handler: the text for a `SAMP…` key, or 0.
extern "C" fn gxt_lookup(key: *const u8) -> u32 {
    if key.is_null() {
        return 0;
    }
    let mut k = [0u8; 8];
    for (i, b) in k.iter_mut().enumerate() {
        // SAFETY: GTA passes a NUL-terminated key; reading stops at the NUL.
        *b = unsafe { *key.add(i) };
        if *b == 0 {
            break;
        }
    }
    if &k[..4] != b"SAMP" {
        return 0;
    }
    let rest = &k[4..];
    let n = rest.iter().position(|&b| b == 0).unwrap_or(4);
    let Ok(g) = GXT.try_lock() else { return 0 };
    g.iter().find(|(key, _)| key[..n] == rest[..n] && key[n] == 0).map_or(0, |(_, b)| b.as_ptr() as u32)
}

fn handler_code(lookup: u32, result: u32) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&CTEXT_GET_ORIGINAL); // the replaced prologue (edi = key)
    c.push(0x60); // pushad
    c.push(0x57); // push edi
    c.push(0xB8); // mov eax, lookup
    c.extend_from_slice(&lookup.to_le_bytes());
    c.extend_from_slice(&[0xFF, 0xD0]); // call eax
    c.extend_from_slice(&[0x83, 0xC4, 0x04]); // add esp, 4
    c.push(0xA3); // mov [result], eax
    c.extend_from_slice(&result.to_le_bytes());
    c.push(0x61); // popad
    c.push(0xA1); // mov eax, [result]
    c.extend_from_slice(&result.to_le_bytes());
    c.extend_from_slice(&[0x85, 0xC0]); // test eax, eax
    c.extend_from_slice(&[0x74, 0x08]); // jz +8
    c.extend_from_slice(&[0x5F, 0x5E, 0x83, 0xC4, 0x20, 0xC2, 0x04, 0x00]); // pop edi; pop esi; add esp,0x20; ret 4
    c.push(0xB8); // mov eax, continue
    c.extend_from_slice(&CTEXT_GET_CONTINUE.to_le_bytes());
    c.extend_from_slice(&[0xFF, 0xE0]); // jmp eax
    c
}

/// Installs the `CText::Get` hook once. Mirrors samp.dll 0x100A4070.
pub fn install_gxt_hook() -> Result<(), String> {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
    let mut mem = ProcessMemory;
    let mut cur = [0u8; 9];
    mem.read(CTEXT_GET, &mut cur).map_err(|e| e.to_string())?;
    if cur != CTEXT_GET_ORIGINAL {
        return Err(format!("CText::Get prologue differs: {cur:02x?}"));
    }
    let code = handler_code(gxt_lookup as *const () as usize as u32, RESULT.as_ptr() as u32);
    // SAFETY: fresh RWX allocation for the handler.
    let p = unsafe { VirtualAlloc(std::ptr::null(), code.len(), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    if p.is_null() {
        return Err("VirtualAlloc failed".into());
    }
    // SAFETY: `p` has room for the code.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, code.len()) };
    mem.write(CTEXT_GET_SLOT, &(p as u32).to_le_bytes()).map_err(|e| e.to_string())?;
    let mut jmp = vec![0xFF, 0x25];
    jmp.extend_from_slice(&CTEXT_GET_SLOT.to_le_bytes());
    jmp.extend_from_slice(&[0x90, 0x90, 0x90]);
    mem.write(CTEXT_GET, &jmp).map_err(|e| e.to_string())?;
    Ok(())
}
