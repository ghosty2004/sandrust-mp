//! The D3D UI pass (Windows only): the fonts, sprites and render targets samp creates on its first frame, and
//! the per-Present draw of nametags, 3D labels, chat, dialogs, scoreboard and death window in samp's order and
//! with its D3D call pattern (so the D3D call-stream diff against the original stays empty).
//! Entry points: [`Renderer::new`], [`Renderer::update_chat_rt`] (game frame) and [`Renderer::frame`] (Present).
//! Spec: docs/re/subsystems/ui-rendering.md; measured call order in docs/phase3/m8a-notes.md.

use std::ffi::c_void;

use samp_ui::chat::ChatWindow;
use samp_ui::deathwindow::{self, DeathWindow};
use samp_ui::dialog::{Dialog, DialogLayout};
use samp_ui::fonts;
use samp_ui::input::ChatInput;
use samp_ui::scoreboard::Row;

use super::d3d::{self, D3dx, Font, FontArgs, Line, Rect, RenderTarget, Sprite, dt};
use super::dxut;
use crate::patch::{Memory, ProcessMemory};

mod chat;
mod editor;
mod fontset;
mod panels;
mod text;
mod world;

pub use fontset::Fonts;
use fontset::{Sprites, build_fonts};

/// `RsGlobal.maximumWidth`: the video mode width (the `W` the font sizes scale with).
const RS_MAX_WIDTH: u32 = 0xC17044;
/// `RsGlobal.maximumHeight`: the video mode height.
const RS_MAX_HEIGHT: u32 = 0xC17048;
/// GTA main window handle.
const GTA_HWND: u32 = 0xC97C1C;
/// Scoreboard dialog background (also used by the class-selection panel).
const SB_BACKGROUND: u32 = 0x960A_0A0A;

/// Video mode width in pixels.
pub fn video_width() -> i32 {
    ProcessMemory.read_u32(RS_MAX_WIDTH).unwrap_or(0) as i32
}

/// Video mode height in pixels.
pub fn video_height() -> i32 {
    ProcessMemory.read_u32(RS_MAX_HEIGHT).unwrap_or(0) as i32
}

/// GTA's main window.
pub fn hwnd() -> *mut c_void {
    ProcessMemory.read_u32(GTA_HWND).unwrap_or(0) as usize as *mut c_void
}

/// Client rect of the GTA window.
pub fn client_size() -> (i32, i32) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let h = hwnd();
    let mut r: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: Win32 query on the game window.
    if h.is_null() || unsafe { GetClientRect(h, &mut r) } == 0 {
        return (video_width(), video_height());
    }
    (r.right - r.left, r.bottom - r.top)
}

/// The cursor position in client coordinates.
pub fn cursor_pos() -> (i32, i32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: -1, y: -1 };
    // SAFETY: Win32 queries.
    unsafe {
        GetCursorPos(&mut p);
        ScreenToClient(hwnd(), &mut p);
    }
    (p.x, p.y)
}

/// Text width with the font's GDI DC, which is what DXUT's Uniscribe buffer measures. Not a D3DX call, so it
/// does not show in the D3D call stream.
fn gdi_width(font: &Font, text: &[u16]) -> i32 {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::GetTextExtentPoint32W;
    if text.is_empty() {
        return 0;
    }
    // SAFETY: ID3DXFont::GetDC (slot 8) returns the font's memory DC.
    let get_dc: extern "system" fn(*mut c_void) -> *mut c_void = unsafe { d3d::vfn(font.ptr, 8) };
    let dc = get_dc(font.ptr);
    let mut s = SIZE { cx: 0, cy: 0 };
    // SAFETY: valid DC and UTF-16 buffer.
    unsafe { GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut s) };
    s.cx
}

/// ANSI → UTF-16 (`MultiByteToWideChar(CP_ACP)`).
pub fn to_wide(b: &[u8]) -> Vec<u16> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
    if b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u16; b.len() * 2 + 1];
    // SAFETY: buffers sized for the conversion.
    let n = unsafe { MultiByteToWideChar(0, 0, b.as_ptr(), b.len() as i32, out.as_mut_ptr(), out.len() as i32) };
    out.truncate(n.max(0) as usize);
    out
}

/// One nametag to consider this frame.
#[derive(Clone, Debug)]
pub struct TagDraw {
    /// Head bone position (bone 8), world coordinates.
    pub anchor: [f32; 3],
    /// Distance to the camera.
    pub distance: f32,
    pub name: Vec<u8>,
    /// ARGB, alpha forced FF.
    pub colour: u32,
    pub health: f32,
    pub armour: f32,
    pub afk: bool,
}

/// One chat bubble to draw (anchor already raised).
#[derive(Clone, Debug)]
pub struct BubbleDraw {
    pub anchor: [f32; 3],
    pub text: Vec<u8>,
    pub colour: u32,
}

/// Scoreboard contents.
pub struct ScoreboardDraw<'a> {
    pub rows: &'a [Row],
    pub server_name: &'a [u8],
    pub players: Vec<u8>,
    pub selected: Option<usize>,
    pub scroll: usize,
}

/// Cursor-mode requests made by the pass (executed by the caller at that point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorReq {
    Mode2,
}

/// Everything one Present draws.
pub struct Frame<'a> {
    pub chat: &'a ChatWindow,
    pub input: &'a ChatInput,
    pub input_first_visible: usize,
    pub tags: &'a [TagDraw],
    pub nametag_status: bool,
    pub bubbles: &'a [BubbleDraw],
    pub scoreboard: Option<ScoreboardDraw<'a>>,
    pub death: Option<&'a DeathWindow>,
    pub dialog: Option<(&'a Dialog, &'a DialogLayout)>,
    pub cursor: (i32, i32),
    /// F5 / F10 held: the passes after the nametags are skipped; F5 draws its net stats text instead.
    pub overlay: Option<samp_ui::keys::Overlay>,
    /// F5 text (`net_stats_text`) when the net stats overlay is shown.
    pub net_stats: Option<&'a [u8]>,
    /// samp's netgame object exists (created on the game frame after the UI objects): the
    /// bubble and 3D-label passes run.
    pub netgame_object: bool,
    /// Netgame exists and the server's show-nametags setting is on (the guards of samp's bars and names
    /// passes): both nametag passes run, even with nobody to draw.
    pub name_tag_passes: bool,
    /// Audio stream title line, when the netgame exists and a title is known.
    pub audio_title: Option<&'a [u8]>,
}

/// The renderer: the D3DX objects samp creates on the first game frame.
pub struct Renderer {
    /// Nametag painter's state block (created on its first bars pass).
    tags_state_block: *mut c_void,
    pub d3dx: D3dx,
    sprites: Sprites,
    pub fonts: Fonts,
    /// SAMPAUX3 20 / 22 (AFK icon), created lazily on the first frame with a nametag, like samp.
    aux: Option<(Font, Font)>,
    gui: *mut c_void,
    cursor: *mut c_void,
    rt: Option<RenderTarget>,
    rt_valid: bool,
    /// `directmode`: the chat render target is never created.
    direct_mode: bool,
    rt_scroll: u32,
    built_for: (i32, i32),
    /// Size of the chat GUI dialog: the video mode, then `(w + 100) × (bottom + 50)` once the input was opened
    /// (ui-rendering.md, "Chat input").
    host_size: (i32, i32),
    /// Start-up objects outside the main per-frame passes (plate painter, object editor, logo, spare sprites).
    unused: Unused,
}

/// Start-up objects outside the main per-frame passes. The plate font and target and the editor line and fonts
/// are drawn with by [`Renderer::paint_plate`], [`Renderer::editor_gizmo`] and friends; the logo and the two
/// extra sprites are only created, reset and released, to keep the D3D call stream identical to samp's.
#[allow(dead_code)]
struct Unused {
    plate_font: Font,
    logo: Logo,
    /// Object editor line and SAMPAUX3 22 / 28 (`editor_gizmo`, `select_highlight`).
    editor_line: Option<Line>,
    editor_fonts: (Option<Font>, Option<Font>),
    plate_rt: Option<RenderTarget>,
    extra: [Sprite; 2],
}

/// The SA-MP logo (`logo.png`, samp.saa hash 0xBADDEA6B) and the state block samp records with it: the texture
/// is (re)created from the archive bytes, then a state block is recorded with that texture on stage 0 and fixed
/// blend / raster / stage states. samp builds it twice at start-up; nothing in 0.3.7-R4 is known to draw with it.
/// Mirrors samp.dll B 0x10070C20.
struct Logo {
    tex: *mut c_void,
    state_block: *mut c_void,
}

impl Logo {
    fn create(d3dx: &D3dx) -> Logo {
        let mut l = Logo { tex: std::ptr::null_mut(), state_block: std::ptr::null_mut() };
        l.init(d3dx);
        l.init(d3dx);
        l
    }

    fn release_state_block(&mut self) {
        // SAFETY: we own the reference.
        unsafe { d3d::release(self.state_block) };
        self.state_block = std::ptr::null_mut();
    }

    fn init(&mut self, d3dx: &D3dx) {
        // SAFETY: we own the previous references.
        unsafe {
            d3d::release(self.tex);
            d3d::release(self.state_block);
        }
        self.tex = std::ptr::null_mut();
        self.state_block = std::ptr::null_mut();
        if let Some(bytes) = crate::filehook::archive_read_hash(0xBADD_EA6B) {
            self.tex = d3d::texture_from_memory(d3dx, &bytes).unwrap_or(std::ptr::null_mut());
        }
        self.state_block = d3d::devcall::record_state_block(|| {
            use d3d::devcall::{set_render_state as rs, set_sampler_state as ss, set_texture, set_tss as tss};
            set_texture(0, self.tex);
            for (k, v) in [(27, 1), (19, 5), (20, 6), (15, 1), (24, 8), (25, 7), (8, 3), (22, 3), (52, 0), (136, 1)] {
                rs(k, v);
            }
            for (k, v) in [(152, 0), (151, 0), (167, 0), (28, 0), (168, 0xF)] {
                rs(k, v);
            }
            for (k, v) in [(1, 4), (2, 2), (3, 0), (4, 4), (5, 2), (6, 0), (11, 0), (24, 0)] {
                tss(0, k, v);
            }
            tss(1, 1, 1);
            tss(1, 4, 1);
            ss(0, 6, 1);
            ss(0, 5, 1);
            ss(0, 7, 0);
        });
    }
}

impl Drop for Logo {
    fn drop(&mut self) {
        // SAFETY: we own these references.
        unsafe {
            d3d::release(self.tex);
            d3d::release(self.state_block);
        }
    }
}

impl Renderer {
    /// samp's first-frame UI construction, run from the game-frame entry inside GTA's scene. Everything is
    /// created in samp's order so the D3D call stream matches: font renderer, chat window, chat input, death
    /// window, logo, nametag and 3D-text sprites, object editor, plate render target, two editor sprites, GUI
    /// skin and fonts, `mouse.png` cursor (ui-rendering.md, "UI objects"; order in docs/phase3/ux1-notes.md).
    /// Mirrors samp.dll 0x100C47D0.
    pub fn new(face: &[u8], weight: u32, font_size: i32, direct_mode: bool) -> Result<Self, String> {
        let d3dx = D3dx::load()?;
        let w = video_width();
        d3d::show_cursor(false);
        let main_h = fonts::main_height(w, font_size);
        let small_h = fonts::small_height(w, font_size);
        let gui_h = fonts::gui_height(font_size);
        let aux_h = fonts::aux_height(w, font_size);
        let args = |height: i32, quality: u32| text_font_args(face, weight, height, quality);
        // Font renderer: four fonts, its sprite, the 38/10 Arial plate font, "Y" measures.
        let main = Font::create(&d3dx, args(main_h, 4))?;
        let main_outline = Font::create(&d3dx, args(main_h, 4))?;
        let small = Font::create(&d3dx, args(small_h, 4))?;
        let small_outline = Font::create(&d3dx, args(small_h, 4))?;
        let text = Sprite::create(&d3dx)?;
        let plate_font = Font::create(
            &d3dx,
            FontArgs { height: 38, width: 10, weight: 700, charset: 1, quality: 4, face: b"Arial".to_vec() },
        )?;
        let main_line = main.measure_z(b"Y", 0).1;
        let small_line = small.measure_z(b"Y", 0).1;
        // Chat window: direct and blit sprites, render target, metrics.
        let chat = Sprite::create(&d3dx)?;
        let blit = Sprite::create(&d3dx)?;
        let (rw, rh) = if w <= 1024 { (1024, 512) } else { (2048, 1024) };
        // `directmode`: no render target, the chat draws directly.
        let rt = if direct_mode { None } else { chat_render_target(&d3dx, rw, rh) };
        let _ = main.measure_z(b"Y", dt::SINGLELINE);
        let timestamp_w = main.measure_z(samp_ui::chat::TIMESTAMP_SAMPLE, dt::SINGLELINE).0;
        // Chat input: its GUI dialog brings up the resource manager's sprite, whose OnResetDevice is followed by
        // the manager's `D3DSBT_ALL` state block.
        let gui_sprite = Sprite::create(&d3dx)?;
        gui_sprite.on_reset();
        dxut::set_rm_state_block(d3d::devcall::create_state_block(1));
        // Death window: sprite, names font with its sample measure, two GTAWEAPON3 fonts with "G" measures.
        let death = Sprite::create(&d3dx)?;
        let death_names = Font::create(&d3dx, args(aux_h, 0))?;
        let nick_w = death_names.measure_z(deathwindow::NICK_SAMPLE, 0).0;
        let death_icon = Font::create(&d3dx, weapon_font_args(aux_h + 8))?;
        let death_backdrop = Font::create(&d3dx, weapon_font_args(aux_h + 12))?;
        let (icon_w, icon_h) = weapon_glyph_size(&death_backdrop);
        let _ = weapon_glyph_size(&death_backdrop);
        // The samp.saa logo texture and its state block (built twice, like samp).
        let logo = Logo::create(&d3dx);
        // Nametag painter and 3D-text painter sprites.
        let tags = Sprite::create(&d3dx)?;
        let text3d = Sprite::create(&d3dx)?;
        // Object editor: its line, then its two SAMPAUX3 fonts.
        let editor_line = Line::create(&d3dx).ok();
        let editor_fonts = (aux_font(&d3dx, 22).ok(), aux_font(&d3dx, 28).ok());
        // Number-plate painter render target.
        let plate_rt = RenderTarget::create(&d3dx, 128, 32, 22).ok();
        // Object editing / selection sprites.
        let extra = [Sprite::create(&d3dx)?, Sprite::create(&d3dx)?];
        // GUI resource manager texture and fonts.
        let gui = dxut::load_skin(&d3dx);
        let gui0 = Font::create(&d3dx, args(gui_h, 0))?;
        let gui1 = Font::create(&d3dx, args(gui_h - 2, 0))?;
        let cursor = load_cursor(&d3dx);
        crate::log!(
            "ui: fonts face={:?} weight={weight} main={main_h} small={small_h} gui={gui_h} aux={aux_h} (fontsize {font_size}, W {w}); \"Y\" main {main_line} small {small_line}; death nick {nick_w} icon {icon_w}x{icon_h}",
            String::from_utf8_lossy(face)
        );
        let fonts = Fonts {
            main,
            main_outline,
            small,
            small_outline,
            gui0,
            gui1,
            death_names,
            death_icon,
            death_backdrop,
            main_h: main_line,
            small_h: small_line,
            gui0_height: gui_h,
            gui1_height: gui_h - 2,
            nick_w,
            icon_w,
            icon_h,
            timestamp_w,
        };
        Ok(Self {
            tags_state_block: std::ptr::null_mut(),
            d3dx,
            sprites: Sprites { text, chat, blit, gui: gui_sprite, death, tags, text3d },
            fonts,
            aux: None,
            gui,
            cursor,
            rt,
            rt_valid: false,
            direct_mode,
            rt_scroll: 0,
            built_for: (w, font_size),
            host_size: (w, video_height()),
            unused: Unused { plate_font, logo, editor_line, editor_fonts, plate_rt, extra },
        })
    }

    /// Chat render target: 1024×512 at an adapter width ≤ 1024, else 2048×1024; none with `directmode`
    /// (ui-rendering.md, "Render-to-texture cache").
    fn create_rt(&mut self) {
        if self.direct_mode {
            return;
        }
        let (w, h) = if video_width() <= 1024 { (1024, 512) } else { (2048, 1024) };
        if let Some(rt) = chat_render_target(&self.d3dx, w, h) {
            self.rt = Some(rt);
        }
        self.rt_valid = false;
    }

    /// `/fontsize` or a video-mode change: rebuild all fonts (the GUI fonts are recreated at once too, like
    /// samp).
    pub fn rebuild_fonts(&mut self, face: &[u8], weight: u32, font_size: i32) {
        let w = video_width();
        match build_fonts(&self.d3dx, w, font_size, face, weight) {
            Ok(f) => {
                self.fonts = f;
                self.built_for = (w, font_size);
                self.rt_valid = false;
            }
            Err(e) => crate::log!("ui: font rebuild failed: {e}"),
        }
    }

    /// Whether the fonts were built for another video width or font size.
    pub fn needs_rebuild(&self, font_size: i32) -> bool {
        self.built_for != (video_width(), font_size)
    }

    /// Device lost: every D3DX object's `OnLostDevice`; default-pool resources and state blocks are released.
    pub fn on_lost(&mut self) {
        for s in self.sprites.all() {
            s.on_lost();
        }
        for f in self.fonts.all() {
            f.on_lost();
        }
        if let Some((a, b)) = &self.aux {
            a.on_lost();
            b.on_lost();
        }
        // DEFAULT-pool render target: released and rebuilt after the reset.
        self.rt = None;
        // State blocks do not survive a reset: the GUI one is re-created on reset, the
        // nametag one on the next bars pass.
        // SAFETY: we own these references.
        unsafe {
            d3d::release(dxut::rm_state_block());
            d3d::release(self.tags_state_block);
        }
        dxut::set_rm_state_block(std::ptr::null_mut());
        self.tags_state_block = std::ptr::null_mut();
        // DEFAULT-pool plate target and the logo's state block must go before Reset too.
        let u = &mut self.unused;
        u.plate_rt = None;
        u.logo.release_state_block();
        u.plate_font.on_lost();
        for f in [&u.editor_fonts.0, &u.editor_fonts.1].into_iter().flatten() {
            f.on_lost();
        }
        if let Some(l) = &u.editor_line {
            l.on_lost();
        }
        for sp in &u.extra {
            sp.on_lost();
        }
    }

    /// Device reset: `OnResetDevice` on every D3DX object, then the released resources are re-created.
    /// Mirrors samp.dll 0x100C41F0.
    pub fn on_reset(&mut self) {
        for s in self.sprites.all() {
            s.on_reset();
        }
        for f in self.fonts.all() {
            f.on_reset();
        }
        if let Some((a, b)) = &self.aux {
            a.on_reset();
            b.on_reset();
        }
        dxut::set_rm_state_block(d3d::devcall::create_state_block(1));
        self.create_rt();
        let u = &mut self.unused;
        u.plate_font.on_reset();
        for f in [&u.editor_fonts.0, &u.editor_fonts.1].into_iter().flatten() {
            f.on_reset();
        }
        if let Some(l) = &u.editor_line {
            l.on_reset();
        }
        for sp in &u.extra {
            sp.on_reset();
        }
        u.plate_rt = RenderTarget::create(&self.d3dx, 128, 32, 22).ok();
        u.logo.init(&self.d3dx);
        // The cursor surface is re-created after a reset (ui-rendering.md, "On lost / reset device").
        // SAFETY: we own the old surface.
        unsafe { d3d::release(self.cursor) };
        self.cursor = d3d::create_cursor(&self.d3dx, b"mouse.png").unwrap_or(std::ptr::null_mut());
        // The chat GUI dialog keeps its size: samp's reset pass does not touch it, so after a device loss it stays
        // at the input-box size once the input was opened.
    }

    // ---- the pass ------------------------------------------------------------------------

    /// The Present pass after the HUD decision, in samp's order: nametag bars and names, audio title, `/dl`
    /// labels; then either the scoreboard or the F5 / F10 overlay (and stop), or chat bubbles, 3D labels, chat,
    /// death window, dialog and the chat GUI dialog (ui-rendering.md, "Present: per-frame draw order").
    pub fn frame(&mut self, f: &Frame, cursor_req: &mut dyn FnMut(CursorReq)) {
        if f.name_tag_passes {
            self.tag_bars(f.tags);
            self.tag_names(f.tags, f.nametag_status);
        }
        if let Some(t) = f.audio_title {
            self.audio_title(t);
        }
        // `/dl` vehicle labels: their own batch on the 3D-text painter while the flag is on.
        if let Some(rows) = crate::vehlabels::draws() {
            self.vehicle_labels(&rows);
        }
        if let Some(sb) = &f.scoreboard {
            self.scoreboard(sb, f.cursor);
            return;
        }
        if f.overlay.is_some() {
            // F10's draw is empty.
            if let Some(t) = f.net_stats {
                self.net_stats(t);
            }
            return;
        }
        if f.netgame_object {
            self.bubbles(f.bubbles);
            // 3D text labels: the text3d batch, empty when there are none (like the original).
            self.labels();
        }
        self.chat(f.chat);
        if f.input.is_open() {
            cursor_req(CursorReq::Mode2);
        }
        if let Some(dw) = f.death {
            self.death_window(dw);
        }
        if let Some((d, l)) = f.dialog {
            cursor_req(CursorReq::Mode2);
            self.dialog(d, l, f.cursor);
        }
        self.chat_gui(f.chat, f.input, f.input_first_visible);
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: we own these references.
        unsafe {
            d3d::release(self.gui);
            d3d::release(self.cursor);
        }
    }
}

/// Arguments of a UI text font (`face`, `weight`, ANSI charset) at `height` with `quality`.
fn text_font_args(face: &[u8], weight: u32, height: i32, quality: u32) -> FontArgs {
    FontArgs { height, width: 0, weight, charset: 1, quality, face: face.to_vec() }
}

/// Arguments of a GTAWEAPON3 (symbol charset) death-window font at `height`.
fn weapon_font_args(height: i32) -> FontArgs {
    FontArgs { height, width: 0, weight: 400, charset: 2, quality: 0, face: b"GTAWEAPON3".to_vec() }
}

/// A SAMPAUX3 (symbol charset) font at `height`.
fn aux_font(d3dx: &D3dx, height: i32) -> Result<Font, String> {
    Font::create(d3dx, FontArgs { height, width: 0, weight: 400, charset: 2, quality: 0, face: b"SAMPAUX3".to_vec() })
}

/// Size of the "G" weapon glyph, measured with `DT_CALCRECT` in a 100×100 rect like the death window does.
fn weapon_glyph_size(font: &Font) -> (i32, i32) {
    let mut r = Rect { left: 0, top: 0, right: 100, bottom: 100 };
    font.draw_n(std::ptr::null_mut(), b"G", -1, &mut r, dt::CALCRECT | dt::NOCLIP | dt::VCENTER, 0xFF00_0000);
    (r.right - r.left, r.bottom - r.top)
}

/// The chat render target; on failure the chat falls back to direct drawing and the debug line is queued.
fn chat_render_target(d3dx: &D3dx, w: u32, h: u32) -> Option<RenderTarget> {
    match RenderTarget::create(d3dx, w, h, 21) {
        Ok(rt) => Some(rt),
        Err(e) => {
            crate::log!("ui: chat render target unavailable ({e}); direct drawing");
            super::glue::print_debug_later(chat_rt_failure_line(&e).into());
            None
        }
    }
}

/// The `mouse.png` cursor surface (null when it cannot be made).
fn load_cursor(d3dx: &D3dx) -> *mut c_void {
    match d3d::create_cursor(d3dx, b"mouse.png") {
        Ok(s) => s,
        Err(e) => {
            crate::log!("ui: cursor: {e}");
            std::ptr::null_mut()
        }
    }
}

/// The chat window's debug line when its render target cannot be made: the render-to-surface object and the
/// texture have their own lines; both fall back to direct drawing. Mirrors samp.dll 0x100681D0.
fn chat_rt_failure_line(err: &str) -> &'static str {
    if err.contains("RenderToSurface") {
        "ChatWindow: Can't create a render to surface. Will use direct mode."
    } else {
        "ChatWindow: Can't create a render surface texture. Will use direct mode."
    }
}
