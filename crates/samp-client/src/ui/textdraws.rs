//! Textdraws drawn through GTA's `CFont` right after `Render2dStuff` (UR §2.4 step 3,
//! §14.2; the call sequence of samp's text path A 0x100b31e0 — samp calls the same
//! GTA functions through thin thunks, all checked: SetScale 0x719380 … PrintString
//! 0x71A700). Windows only.
//!
//! Sprite textdraws (font 4, `txd:name`) follow samp's loader (A 0x100b2d10 / 0x100b2bd0 /
//! 0x100b2ca0, static analysis): the TXD is found by name or added and loaded from
//! `models\txd\<txd>.txd` (`hud` used as is, `samaps` → `SAMP\samaps.txd`, `vehicleprev` →
//! `SAMP\vehicles128.txd`), the texture is taken with `CSprite2d::SetTexture` inside
//! Push/Set/PopCurrentTxd, and drawn with `RwRenderStateSet(TEXTUREFILTER, LINEAR)` +
//! `CSprite2d::Draw(rect, colour)`. `mdlNNN` names take the generic TXD path (no custom-model
//! store in R4). Model previews (font 5): `preview.rs` renders their textures; with a texture they
//! take the sprite path, without one (the frame their RPC 134 arrived, or a model samp never
//! renders) the text path, like samp's dispatch A 0x100b34c0.

use samp_ui::textdraw::{self, FontParams, TextDraws};

use crate::patch::{Memory, ProcessMemory};

/// `CFont` (gta_sa.exe 1.0 US).
mod cfont {
    pub const SET_SCALE: usize = 0x719380;
    pub const SET_COLOR: usize = 0x719430;
    pub const SET_JUSTIFY: usize = 0x719600;
    pub const SET_ORIENTATION: usize = 0x719610;
    pub const SET_WRAPX: usize = 0x7194D0;
    pub const SET_CENTRE_SIZE: usize = 0x7194E0;
    pub const SET_BACKGROUND: usize = 0x7195C0;
    pub const SET_BACKGROUND_COLOR: usize = 0x7195E0;
    pub const SET_PROPORTIONAL: usize = 0x7195B0;
    pub const SET_DROP_COLOR: usize = 0x719510;
    pub const SET_EDGE: usize = 0x719590;
    pub const SET_DROP_SHADOW_POSITION: usize = 0x719570;
    pub const SET_FONT_STYLE: usize = 0x719490;
    pub const PRINT_STRING: usize = 0x71A700;
    /// `CMessages::InsertNumberInString` / `InsertPlayerControlKeysInString`.
    pub const INSERT_NUMBERS: usize = 0x69DE90;
    pub const INSERT_KEYS: usize = 0x69E160;
}

/// Textures of sprite textdraws by textdraw id: (text it was loaded for, RwTexture*).
static SPRITES: std::sync::Mutex<Vec<(u16, Vec<u8>, u32)>> = std::sync::Mutex::new(Vec::new());

mod txd {
    pub const FIND_SLOT: usize = 0x731850;
    pub const ADD_SLOT: usize = 0x731C80;
    pub const LOAD: usize = 0x7320B0;
    pub const ADD_REF: usize = 0x731A00;
    pub const PUSH_CURRENT: usize = 0x7316A0;
    pub const POP_CURRENT: usize = 0x7316B0;
    pub const SET_CURRENT: usize = 0x7319C0;
    pub const SPRITE_SET_TEXTURE: usize = 0x727270;
    pub const SPRITE_DRAW: usize = 0x728350;
    pub const SPRITE_DELETE: usize = 0x7281E0;
    /// `RwEngineInstance`; `+0x20` = `fpRenderStateSet`.
    pub const RW_ENGINE: u32 = 0xC97B24;
}

fn cstr(s: &[u8]) -> Vec<u8> {
    let mut v = s.to_vec();
    v.push(0);
    v
}

fn find_slot(name: &[u8]) -> i32 {
    // SAFETY: GTA's cdecl CTxdStore::FindTxdSlot(const char*).
    let f: extern "C" fn(*const u8) -> i32 = unsafe { std::mem::transmute(txd::FIND_SLOT) };
    f(cstr(name).as_ptr())
}

/// samp's "ensure loaded" (A 0x100b2bd0): add + load + AddRef when not present.
fn ensure_txd(key: &[u8], file: &[u8], samp_dir: bool) -> bool {
    if find_slot(key) != -1 {
        return true;
    }
    let dir: &[u8] = if samp_dir { b"SAMP" } else { b"models\\txd" };
    let mut path = dir.to_vec();
    path.push(b'\\');
    path.extend_from_slice(file);
    path.extend_from_slice(b".txd");
    let exists = std::path::Path::new(&String::from_utf8_lossy(&path).replace('\\', "/")).exists();
    // SAFETY: GTA's cdecl CTxdStore functions on the main thread.
    unsafe {
        let add: extern "C" fn(*const u8) -> i32 = std::mem::transmute(txd::ADD_SLOT);
        let slot = add(cstr(key).as_ptr());
        if !exists {
            return false;
        }
        let load: extern "C" fn(i32, *const u8) -> u8 = std::mem::transmute(txd::LOAD);
        load(slot, cstr(&path).as_ptr());
        let addref: extern "C" fn(i32) = std::mem::transmute(txd::ADD_REF);
        addref(slot);
    }
    true
}

/// Texture of `txd:name` (A 0x100b2d10 + 0x100b2ca0), 0 when unavailable.
fn load_sprite(text: &[u8]) -> u32 {
    let Some(i) = text.iter().position(|&b| b == b':') else { return 0 };
    if text.len() > 0x3F || text.contains(&b'\\') || text.contains(&b'/') {
        return 0;
    }
    let (t, name) = (&text[..i], &text[i + 1..]);
    let key: Vec<u8> = match t {
        b"hud" => b"hud".to_vec(),
        b"samaps" => {
            if !ensure_txd(b"samaps", b"samaps", true) {
                return 0;
            }
            b"samaps".to_vec()
        }
        b"vehicleprev" => {
            if !ensure_txd(b"vehicleprev", b"vehicles128", true) {
                return 0;
            }
            b"vehicleprev".to_vec()
        }
        // `mdlNNN` names (A 0x100b2d10): samp only treats them specially when its custom-model
        // store (A 0x1026ea10) exists, and only the dormant RPC 179 handler creates it; in R4 they
        // take the generic TXD path below, like any other dictionary name.
        _ => {
            let k = t[..t.len().min(10)].to_vec();
            if !ensure_txd(&k, t, false) {
                return 0;
            }
            k
        }
    };
    let slot = find_slot(&key);
    if slot == -1 {
        return 0;
    }
    let mut sprite: u32 = 0;
    // SAFETY: GTA's TXD stack and CSprite2d::SetTexture (thiscall on a one-pointer sprite).
    unsafe {
        let push: extern "C" fn() = std::mem::transmute(txd::PUSH_CURRENT);
        let set: extern "C" fn(i32) = std::mem::transmute(txd::SET_CURRENT);
        let pop: extern "C" fn() = std::mem::transmute(txd::POP_CURRENT);
        let tex: extern "thiscall" fn(*mut u32, *const u8) = std::mem::transmute(txd::SPRITE_SET_TEXTURE);
        push();
        set(slot);
        tex(&mut sprite, cstr(name).as_ptr());
        pop();
    }
    sprite
}

fn free_sprite(tex: u32) {
    if tex != 0 {
        let mut s = tex;
        // SAFETY: CSprite2d destructor on a temporary holding the texture (A 0x100b39a0).
        let f: extern "thiscall" fn(*mut u32) = unsafe { std::mem::transmute(txd::SPRITE_DELETE) };
        f(&mut s);
    }
}

/// Drops the textures of textdraws that are gone or changed.
fn sync_sprites(pool: &TextDraws) {
    let mut g = SPRITES.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(id, text, tex)| {
        let keep = pool.get(*id).is_some_and(|t| t.data.style == textdraw::FONT_SPRITE && t.text == *text);
        if !keep {
            free_sprite(*tex);
        }
        keep
    });
}

/// Number of sprite textdraw textures held (they share samp's 200 texture slots with previews).
pub fn sprite_count() -> usize {
    SPRITES.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// The texture of sprite textdraw `id`, loaded on first use.
fn sprite_texture(id: u16, t: &samp_ui::textdraw::TextDraw) -> u32 {
    let mut g = SPRITES.lock().unwrap_or_else(|e| e.into_inner());
    match g.iter().find(|(i, _, _)| *i == id) {
        Some((_, _, tex)) => *tex,
        None => {
            let tex = load_sprite(&t.text);
            g.push((id, t.text.clone(), tex));
            tex
        }
    }
}

/// The sprite path (A 0x100b30d0): textured quad, then the hit rect.
fn draw_sprite(tex: u32, t: &mut samp_ui::textdraw::TextDraw, hover: u32, w: i32, h: i32, ux: f32, uy: f32) {
    let r = textdraw::sprite_rect(&t.data, w, h, ux, uy);
    if tex != 0 {
        // CRect as samp builds it: left, bottom, right, top.
        let rect = [r[0], r[3], r[2], r[1]];
        let colour = if t.hovered { hover } else { t.data.letter_colour };
        let mut sprite = tex;
        // SAFETY: RwRenderStateSet through RwEngineInstance and CSprite2d::Draw (thiscall).
        unsafe {
            let eng = std::ptr::read_volatile(txd::RW_ENGINE as usize as *const u32);
            if eng != 0 {
                let rs: extern "C" fn(u32, u32) -> i32 = std::mem::transmute(*((eng + 0x20) as usize as *const u32) as usize);
                rs(9, 2);
            }
            let draw: extern "thiscall" fn(*mut u32, *const [f32; 4], *const u32) = std::mem::transmute(txd::SPRITE_DRAW);
            draw(&mut sprite, &rect, &colour);
        }
    }
    t.hit = Some(textdraw::sprite_hit_rect(&t.data, w, h, ux, uy));
}

/// Unit constants read at draw time (1/640, 1/448).
const UNIT_X: u32 = 0x859520;
const UNIT_Y: u32 = 0x859524;

/// samp's work buffer is 0x321+ bytes inside the textdraw; GTA's insertions grow the
/// text, so ours is larger.
const WORK: usize = 2048;

fn call1(f: usize, a: u32) {
    // SAFETY: GTA's cdecl one-argument CFont setters (by-value 32-bit argument).
    let f: extern "C" fn(u32) = unsafe { std::mem::transmute(f) };
    f(a);
}

fn call_f(f: usize, a: f32) {
    // SAFETY: GTA's cdecl float setters.
    let f: extern "C" fn(f32) = unsafe { std::mem::transmute(f) };
    f(a);
}

/// The CFont calls of one text textdraw (A 0x100b31e0 order).
fn print(p: &FontParams, buf: &mut [u8; WORK], has_keys: bool) {
    // SAFETY: GTA functions on the main thread; `buf` is NUL-terminated with room to grow.
    unsafe {
        let scale: extern "C" fn(f32, f32) = std::mem::transmute(cfont::SET_SCALE);
        scale(p.scale.0, p.scale.1);
        call1(cfont::SET_COLOR, p.colour);
        call1(cfont::SET_JUSTIFY, 0);
        call1(cfont::SET_ORIENTATION, u32::from(p.orientation));
        call_f(cfont::SET_WRAPX, p.wrap_x);
        call_f(cfont::SET_CENTRE_SIZE, p.centre_size);
        let bg: extern "C" fn(u32, u32) = std::mem::transmute(cfont::SET_BACKGROUND);
        bg(u32::from(p.background), 0);
        call1(cfont::SET_BACKGROUND_COLOR, p.background_colour);
        call1(cfont::SET_PROPORTIONAL, u32::from(p.proportional));
        call1(cfont::SET_DROP_COLOR, p.drop_colour);
        if p.outline != 0 {
            call1(cfont::SET_EDGE, u32::from(p.outline));
        } else {
            call1(cfont::SET_DROP_SHADOW_POSITION, u32::from(p.shadow));
        }
        call1(cfont::SET_FONT_STYLE, u32::from(p.font));
        let ins: extern "C" fn(*mut u8, i32, i32, i32, i32, i32, i32, *mut u8) =
            std::mem::transmute(cfont::INSERT_NUMBERS);
        let b = buf.as_mut_ptr();
        ins(b, -1, -1, -1, -1, -1, -1, b);
        if has_keys {
            let k: extern "C" fn(*mut u8) = std::mem::transmute(cfont::INSERT_KEYS);
            k(b);
        }
        let ps: extern "C" fn(f32, f32, *const u8) = std::mem::transmute(cfont::PRINT_STRING);
        ps(p.x, p.y, b);
        call1(cfont::SET_EDGE, 0);
    }
}

/// Draws every present textdraw in id order and stores the hit rectangles.
pub fn draw_all(pool: &mut TextDraws) {
    let mem = ProcessMemory;
    let w = super::render::video_width();
    let h = super::render::video_height();
    let ux = mem.read_f32(UNIT_X).unwrap_or(1.0 / 640.0);
    let uy = mem.read_f32(UNIT_Y).unwrap_or(1.0 / 448.0);
    let hover = pool.selection.hover_colour;
    let ids = pool.ids();
    sync_sprites(pool);
    let mut buf = Box::new([0u8; WORK]);
    for id in ids {
        let Some(t) = pool.get_mut(id) else { continue };
        if t.data.style == textdraw::FONT_SPRITE {
            let tex = sprite_texture(id, t);
            draw_sprite(tex, t, hover, w, h, ux, uy);
            continue;
        }
        if t.data.style == textdraw::FONT_PREVIEW
            && let Some(tex) = super::preview::texture(id, t.serial)
        {
            draw_sprite(tex, t, hover, w, h, ux, uy);
            continue;
        }
        if t.text.is_empty() {
            continue;
        }
        let n = t.text.len().min(WORK / 2);
        buf[..n].copy_from_slice(&t.text[..n]);
        buf[n..].fill(0);
        let p = textdraw::font_params(&t.data, t.hovered.then_some(hover), w, h, ux, uy);
        print(&p, &mut buf, t.has_keys);
        t.hit = Some(textdraw::hit_rect(&p));
    }
}
