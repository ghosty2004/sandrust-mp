//! The D3D UI pass (Windows only): the fonts, sprites and render targets samp creates on its first frame, and
//! the per-Present draw of nametags, 3D labels, chat, dialogs, scoreboard and death window in samp's order and
//! with its D3D call pattern (so the D3D call-stream diff against the original stays empty).
//! Entry points: [`Renderer::new`], [`Renderer::update_chat_rt`] (game frame) and [`Renderer::frame`] (Present).
//! Spec: docs/re/subsystems/ui-rendering.md; measured call order in docs/phase3/m8a-notes.md.

use std::ffi::c_void;

use samp_ui::chat::{ChatLayout, ChatWindow};
use samp_ui::colour;
use samp_ui::deathwindow::{self, DeathWindow};
use samp_ui::dialog::{self, Dialog, DialogLayout, Style};
use samp_ui::fonts::{self, FontId, Size, TextMeasure};
use samp_ui::input::{self, ChatInput};
use samp_ui::nametags;
use samp_ui::scoreboard::Row;

use super::d3d::{self, D3dx, Font, FontArgs, Line, Rect, RenderTarget, Sprite, devcall, dt};
use crate::objedit::{self, Editor};
use super::dxut::{self, EditView, ScrollBar};

/// `RsGlobal.maximumWidth`: the video mode width (the `W` the font sizes scale with).
const RS_MAX_WIDTH: u32 = 0xC17044;
/// `RsGlobal.maximumHeight`: the video mode height.
const RS_MAX_HEIGHT: u32 = 0xC17048;
/// GTA main window handle.
const GTA_HWND: u32 = 0xC97C1C;
/// Scoreboard text colour (server name, player count).
const SB_TEXT: u32 = 0xFFBE_BEBE;
/// Scoreboard column header colour.
const SB_HEADER: u32 = 0xFF95_B0D0;
/// Scoreboard dialog background (also used by the class-selection panel).
const SB_BACKGROUND: u32 = 0x960A_0A0A;
/// Chat GUI dialog background: drawn like samp does, but fully transparent.
const CHAT_GUI_BACKGROUND: u32 = 0x0000_0000;
/// Dialog background colour.
const DIALOG_BACKGROUND: u32 = 0xDC05_0505;

/// Video mode width in pixels.
pub fn video_width() -> i32 {
    // SAFETY: RsGlobal field read on the main thread.
    unsafe { std::ptr::read_volatile(RS_MAX_WIDTH as usize as *const i32) }
}

/// Video mode height in pixels.
pub fn video_height() -> i32 {
    // SAFETY: as above.
    unsafe { std::ptr::read_volatile(RS_MAX_HEIGHT as usize as *const i32) }
}

/// GTA's main window.
pub fn hwnd() -> *mut c_void {
    // SAFETY: the game's HWND global.
    unsafe { std::ptr::read_volatile(GTA_HWND as usize as *const usize) as *mut c_void }
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

/// The fonts the UI draws with, plus the metrics samp measures once when it creates them.
pub struct Fonts {
    main: Font,
    main_outline: Font,
    small: Font,
    small_outline: Font,
    gui0: Font,
    gui1: Font,
    death_names: Font,
    death_icon: Font,
    death_backdrop: Font,
    /// Height of "Y" in the main / small font (samp keeps these in its font renderer at +0x20 / +0x24).
    pub main_h: i32,
    pub small_h: i32,
    pub gui0_height: i32,
    pub gui1_height: i32,
    /// Death window metrics: nick column width and weapon glyph size (death window object +0x12b, +0x12f/+0x133).
    nick_w: i32,
    icon_w: i32,
    icon_h: i32,
    /// Chat timestamp column sample width (measured once, like samp's chat window).
    timestamp_w: i32,
}

/// Builds the font-renderer, GUI and death-window fonts for a video width and font size, after `/fontsize` or a
/// mode change (ui-rendering.md, "Size and face settings").
fn build_fonts(d3dx: &D3dx, video_w: i32, fs: i32, face: &[u8], weight: u32) -> Result<Fonts, String> {
    let main_h = fonts::main_height(video_w, fs);
    let small_h = fonts::small_height(video_w, fs);
    let gui_h = fonts::gui_height(fs);
    let aux_h = fonts::aux_height(video_w, fs);
    let args =
        |height: i32, quality: u32| FontArgs { height, width: 0, weight, charset: 1, quality, face: face.to_vec() };
    let sym = |height: i32| FontArgs { height, width: 0, weight: 400, charset: 2, quality: 0, face: b"GTAWEAPON3".to_vec() };
    let main = Font::create(d3dx, args(main_h, 4))?;
    let main_outline = Font::create(d3dx, args(main_h, 4))?;
    let small = Font::create(d3dx, args(small_h, 4))?;
    let small_outline = Font::create(d3dx, args(small_h, 4))?;
    let gui0 = Font::create(d3dx, args(gui_h, 0))?;
    let gui1 = Font::create(d3dx, args(gui_h - 2, 0))?;
    let death_names = Font::create(d3dx, args(aux_h, 0))?;
    let death_icon = Font::create(d3dx, sym(aux_h + 8))?;
    let death_backdrop = Font::create(d3dx, sym(aux_h + 12))?;
    let y = b"Y";
    let main_line = main.measure(y, dt::SINGLELINE).1;
    let small_line = small.measure(y, dt::SINGLELINE).1;
    let nick_w = death_names.measure(deathwindow::NICK_SAMPLE, 0).0;
    let timestamp_w = main.measure_z(samp_ui::chat::TIMESTAMP_SAMPLE, dt::SINGLELINE).0;
    let (icon_w, icon_h) = death_backdrop.measure(b"G", dt::CENTER | dt::VCENTER).max(death_icon.measure(b"G", 0));
    crate::log!(
        "ui: fonts face={:?} weight={weight} main={main_h} small={small_h} gui={gui_h} aux={aux_h} (fontsize {fs}, W {video_w}); \"Y\" main {main_line} small {small_line}; death nick {nick_w} icon {icon_w}x{icon_h}",
        String::from_utf8_lossy(face)
    );
    Ok(Fonts {
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
    })
}

impl Fonts {
    fn font(&self, id: FontId) -> &Font {
        match id {
            FontId::Main => &self.main,
            FontId::Small => &self.small,
            FontId::Gui0 => &self.gui0,
            FontId::Gui1 => &self.gui1,
            FontId::DeathNames => &self.death_names,
            FontId::DeathIcon => &self.death_icon,
        }
    }

    fn all(&self) -> [&Font; 9] {
        [
            &self.main,
            &self.main_outline,
            &self.small,
            &self.small_outline,
            &self.gui0,
            &self.gui1,
            &self.death_names,
            &self.death_icon,
            &self.death_backdrop,
        ]
    }
}

impl TextMeasure for Fonts {
    fn measure(&self, font: FontId, text: &[u8]) -> Size {
        let (w, h) = self.font(font).measure(text, dt::SINGLELINE);
        Size { w, h }
    }
    fn timestamp_width(&self) -> i32 {
        self.timestamp_w
    }
    fn line_height(&self, font: FontId) -> i32 {
        match font {
            FontId::Main => self.main_h,
            FontId::Small => self.small_h,
            FontId::Gui0 => self.gui0_height,
            FontId::Gui1 => self.gui1_height,
            f => self.measure(f, b"Y").h,
        }
    }
    /// Multi-line block (`DT_CALCRECT|DT_EXPANDTABS`), the dialog body measure.
    fn measure_block(&self, font: FontId, text: &[u8]) -> Size {
        if text.is_empty() {
            return Size::default();
        }
        let mut r = Rect::default();
        self.font(font).draw_n(std::ptr::null_mut(), text, -1, &mut r, dt::CALCRECT | dt::EXPANDTABS, 0xFF00_0000);
        Size { w: r.right - r.left, h: r.bottom - r.top }
    }
}

/// The sprites samp keeps, one per painter (each a separate `ID3DXSprite`); the offsets are in samp's objects.
struct Sprites {
    /// Font renderer (+0x14).
    text: Sprite,
    /// Chat window direct sprite (+0x63a6), also used inside the render target.
    chat: Sprite,
    /// Chat render-target blit sprite (+0x63aa).
    blit: Sprite,
    /// GUI resource manager sprite (DXUT).
    gui: Sprite,
    /// Death window.
    death: Sprite,
    /// Nametag/bar painter.
    tags: Sprite,
    /// 3D-anchored text painter (bubbles, labels).
    text3d: Sprite,
}

impl Sprites {
    fn all(&self) -> [&Sprite; 7] {
        [&self.text, &self.chat, &self.blit, &self.gui, &self.death, &self.tags, &self.text3d]
    }
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
        let args =
            |height: i32, quality: u32| FontArgs { height, width: 0, weight, charset: 1, quality, face: face.to_vec() };
        let sym =
            |height: i32| FontArgs { height, width: 0, weight: 400, charset: 2, quality: 0, face: b"GTAWEAPON3".to_vec() };
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
        let rt = if direct_mode {
            None
        } else {
            match RenderTarget::create(&d3dx, rw, rh, 21) {
                Ok(rt) => Some(rt),
                Err(e) => {
                    crate::log!("ui: chat render target unavailable ({e}); direct drawing");
                    super::glue::print_debug_later(chat_rt_failure_line(&e).into());
                    None
                }
            }
        };
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
        let death_icon = Font::create(&d3dx, sym(aux_h + 8))?;
        let death_backdrop = Font::create(&d3dx, sym(aux_h + 12))?;
        let glyph = || {
            let mut r = Rect { left: 0, top: 0, right: 100, bottom: 100 };
            death_backdrop.draw_n(std::ptr::null_mut(), b"G", -1, &mut r, dt::CALCRECT | dt::NOCLIP | dt::VCENTER, 0xFF00_0000);
            (r.right - r.left, r.bottom - r.top)
        };
        let (icon_w, icon_h) = glyph();
        let _ = glyph();
        // The samp.saa logo texture and its state block (built twice, like samp).
        let logo = Logo::create(&d3dx);
        // Nametag painter and 3D-text painter sprites.
        let tags = Sprite::create(&d3dx)?;
        let text3d = Sprite::create(&d3dx)?;
        // Object editor: its line, then its two SAMPAUX3 fonts.
        let editor_line = Line::create(&d3dx).ok();
        let aux_font = |h: i32| {
            Font::create(&d3dx, FontArgs { height: h, width: 0, weight: 400, charset: 2, quality: 0, face: b"SAMPAUX3".to_vec() })
        };
        let editor_fonts = (aux_font(22).ok(), aux_font(28).ok());
        // Number-plate painter render target.
        let plate_rt = RenderTarget::create(&d3dx, 128, 32, 22).ok();
        // Object editing / selection sprites.
        let extra = [Sprite::create(&d3dx)?, Sprite::create(&d3dx)?];
        // GUI resource manager texture and fonts.
        let gui = dxut::load_skin(&d3dx);
        let gui0 = Font::create(&d3dx, args(gui_h, 0))?;
        let gui1 = Font::create(&d3dx, args(gui_h - 2, 0))?;
        let cursor = match d3d::create_cursor(&d3dx, b"mouse.png") {
            Ok(s) => s,
            Err(e) => {
                crate::log!("ui: cursor: {e}");
                std::ptr::null_mut()
            }
        };
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
        match RenderTarget::create(&self.d3dx, w, h, 21) {
            Ok(rt) => self.rt = Some(rt),
            Err(e) => {
                crate::log!("ui: chat render target unavailable ({e}); direct drawing");
                super::glue::print_debug_later(chat_rt_failure_line(&e).into());
            }
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

    // ---- font renderer -----------------------------------------------------------------

    /// Outline passes, then the main pass through the colour-code wrapper, all with `sprite`.
    #[allow(clippy::too_many_arguments)]
    fn outlined(
        sprite: &Sprite,
        font: &Font,
        outline_font: &Font,
        text: &[u8],
        rect: Rect,
        flags: u32,
        colour: u32,
        outline: &[(i32, i32)],
    ) {
        let stripped = colour::strip_codes(text);
        // As in samp, the 4-way passes move only left/top; the 8-way chat passes (W > 1280) move the whole rect.
        let whole = outline.len() > 4;
        for &(dx, dy) in outline {
            let (rx, ry) = if whole { (dx, dy) } else { (0, 0) };
            let mut r =
                Rect { left: rect.left + dx, top: rect.top + dy, right: rect.right + rx, bottom: rect.bottom + ry };
            outline_font.draw(sprite.ptr, &stripped, &mut r, flags | dt::NOCLIP, colour::OUTLINE);
        }
        let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
        let mut r = rect;
        d3d::draw_glyph_coloured(font, sprite, &stripped, &mut r, flags | dt::NOCLIP, colour, glyphs);
    }

    /// Font-renderer small-font draw without the outline passes, in its own sprite Begin/End.
    /// Mirrors samp.dll 0x1006B4E0 with the outline off.
    fn small_text_plain(&self, text: &[u8], rect: Rect, flags: u32, colour: u32) {
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        let stripped = colour::strip_codes(text);
        let glyphs: Vec<u32> = colour::glyph_colours(text, colour).iter().map(|g| g.colour).collect();
        let mut r = rect;
        d3d::draw_glyph_coloured(&self.fonts.small, s, &stripped, &mut r, flags | dt::NOCLIP, colour, glyphs);
        s.end();
    }

    /// Audio stream title: small font at (15, H − 20)–(W, H + 10) (ui-rendering.md, "Smaller overlays").
    /// Mirrors samp.dll 0x10066AB0.
    fn audio_title(&self, text: &[u8]) {
        let (w, h) = (video_width(), video_height());
        let rect = Rect { left: 15, top: h - 20, right: w, bottom: h + 10 };
        self.small_text_plain(text, rect, crate::audio::TITLE_FLAGS, crate::audio::TITLE_COLOUR);
    }

    /// Font-renderer small-font draw with the 4-way outline, in its own sprite Begin/End.
    /// Mirrors samp.dll 0x1006B4E0.
    fn small_text(&self, text: &[u8], rect: Rect, flags: u32, colour: u32) {
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, text, rect, flags, colour, &fonts::OUTLINE_4);
        s.end();
    }

    // ---- chat ----------------------------------------------------------------------------

    /// Draws the visible chat lines with the chat sprite (ui-rendering.md, "Chat window", "Layout").
    fn chat_lines(&self, layout: &ChatLayout) {
        let s = &self.sprites.chat;
        for line in &layout.lines {
            for item in &line.items {
                let rect = Rect { left: item.x, top: item.y, right: line.rect.right, bottom: line.rect.bottom };
                Self::outlined(
                    s,
                    &self.fonts.main,
                    &self.fonts.main_outline,
                    &item.text,
                    rect,
                    dt::SINGLELINE,
                    item.colour,
                    layout.outline,
                );
            }
        }
    }

    /// Render-target update, once per game frame from the `CGame::Process` hook: invalid while the GTA menu is
    /// on; redrawn when the chat is dirty or the scroll position changed. Mirrors samp.dll 0x10067ED0.
    pub fn update_chat_rt(&mut self, chat: &mut ChatWindow, menu: bool) {
        if menu {
            self.rt_valid = false;
            return;
        }
        if !chat.mode().chat_visible() {
            return;
        }
        let Some(rt) = &self.rt else { return };
        if self.rt_valid && !chat.is_dirty() && self.rt_scroll == chat.scroll_pos() {
            return;
        }
        let layout = chat.layout(&self.fonts, video_width());
        if rt.begin() < 0 {
            return;
        }
        devcall::clear(3, 0, 0.0, 0);
        let s = &self.sprites.chat;
        s.begin(d3d::sprite::ALPHABLEND);
        devcall::set_render_state(20, 14); // D3DRS_DESTBLEND = D3DBLEND_BLENDFACTOR, as samp sets it
        self.chat_lines(&layout);
        s.end();
        rt.end();
        self.rt_valid = true;
        self.rt_scroll = chat.scroll_pos();
        chat.clear_dirty();
    }

    /// Present step: the chat window, a blit of the render target or a direct draw (ui-rendering.md,
    /// "Draw path").
    fn chat(&self, chat: &ChatWindow) {
        if !chat.mode().chat_visible() {
            return;
        }
        if let (Some(rt), true) = (&self.rt, self.rt_valid) {
            let s = &self.sprites.blit;
            s.begin(d3d::sprite::ALPHABLEND);
            s.draw(rt.tex, None, None, None, 0xFFFF_FFFF);
            s.end();
        } else {
            let layout = chat.layout(&self.fonts, video_width());
            let s = &self.sprites.chat;
            s.begin(d3d::sprite::ALPHABLEND);
            self.chat_lines(&layout);
            s.end();
        }
    }

    // ---- nametags, bubbles -------------------------------------------------------------

    /// World point to screen `(x, y, z)` with samp's copies of the view and projection matrices.
    fn project(&self, p: [f32; 3]) -> [f32; 3] {
        let vp = devcall::viewport();
        let proj = *d3d::PROJECTION.lock().unwrap_or_else(|e| e.into_inner());
        let view = *d3d::VIEW.lock().unwrap_or_else(|e| e.into_inner());
        self.d3dx.project(p, &vp, &proj, &view)
    }

    /// Screen point of a tag: `(x, y, z + 0.0475·d + 0.2)`, truncated; `None` when the
    /// projected z is above 1.
    fn tag_point(&self, t: &TagDraw) -> Option<(i32, i32)> {
        let p = [
            t.anchor[0],
            t.anchor[1],
            t.anchor[2] + t.distance * nametags::RAISE_PER_METRE + nametags::RAISE_BASE,
        ];
        let s = self.project(p);
        (s[2] <= 1.0).then_some((s[0] as i32, s[1] as i32))
    }

    /// Nametag health / armour bars pass. The painter's begin creates its `D3DSBT_ALL` state block on first use
    /// (after the sprite's `OnResetDevice`) and captures it; its end applies it after the sprite's `End`
    /// (ui-rendering.md, "Bars"). Mirrors samp.dll 0x10074270.
    fn tag_bars(&mut self, tags: &[TagDraw]) {
        let s = &self.sprites.tags;
        if self.tags_state_block.is_null() {
            s.on_reset();
            self.tags_state_block = d3d::devcall::create_state_block(1);
        }
        d3d::state_block_capture(self.tags_state_block);
        s.begin(d3d::sprite::DONOTSAVESTATE);
        dxut::ui_states();
        for t in tags {
            let Some((x, y)) = self.tag_point(t) else { continue };
            for group in nametags::bars(x, y, t.health, t.armour) {
                dxut::set_colour_fvf();
                for q in group {
                    dxut::colour_quad(q.x0, q.y0, q.x1, q.y1, q.colour);
                }
            }
        }
        s.end();
        d3d::state_block_apply(self.tags_state_block);
    }

    /// F5 overlay: the whole text and "Y" measured (`DT_CALCRECT`, no sprite), then the title
    /// `Client Network Stats` at (10, 10)–(W − 150, 10 + h) in 0xFF8888EE and every `\n`-terminated line below
    /// it in white, each through the font renderer with its 4-way outline and its own sprite Begin/End.
    /// Mirrors samp.dll 0x10060D70.
    fn net_stats(&self, text: &[u8]) {
        let _ = self.fonts.main.measure_z(&colour::strip_codes(text), 0);
        let h = self.fonts.main.measure_z(b"Y", 0).1;
        let right = video_width() - 150;
        let line = |t: &[u8], top: i32, c: u32| {
            let s = &self.sprites.text;
            s.begin(d3d::sprite::ALPHABLEND);
            let rect = Rect { left: 10, top, right, bottom: top + h };
            Self::outlined(s, &self.fonts.main, &self.fonts.main_outline, t, rect, 0, c, &fonts::OUTLINE_4);
            s.end();
        };
        line(b"Client Network Stats", 10, 0xFF88_88EE);
        let mut top = 10 + h;
        let mut rest = text;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            line(&rest[..i], top, 0xFFFF_FFFF);
            top += h;
            rest = &rest[i + 1..];
        }
    }

    /// Nametag names pass, including the AFK icon (ui-rendering.md, "Name text"). Mirrors samp.dll 0x10075390.
    fn tag_names(&mut self, tags: &[TagDraw], nametag_status: bool) {
        if !tags.is_empty() && self.aux.is_none() {
            let f = |h: i32| {
                Font::create(
                    &self.d3dx,
                    FontArgs { height: h, width: 0, weight: 400, charset: 2, quality: 0, face: b"SAMPAUX3".to_vec() },
                )
            };
            match (f(20), f(22)) {
                (Ok(a), Ok(b)) => self.aux = Some((a, b)),
                (Err(e), _) | (_, Err(e)) => crate::log!("ui: SAMPAUX3 fonts: {e}"),
            }
        }
        let s = &self.sprites.tags;
        s.begin(d3d::sprite::ALPHABLEND);
        for t in tags {
            let Some((x, y)) = self.tag_point(t) else { continue };
            let mut m = Rect::default();
            self.fonts.small.draw_n(std::ptr::null_mut(), &t.name, -1, &mut m, dt::CALCRECT, 0xFF00_0000);
            let (w, h) = (m.right - m.left, m.bottom - m.top);
            let r = nametags::name_rect(x, y, w, h);
            let rect = Rect { left: r[0], top: r[1], right: r[2], bottom: r[3] };
            Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, &t.name, rect, 0, t.colour, &fonts::OUTLINE_4);
            if let Some((a20, a22)) = &self.aux {
                let mut c = Rect::default();
                a22.draw_n(std::ptr::null_mut(), b"C", 1, &mut c, dt::CALCRECT | dt::NOCLIP | dt::CENTER | dt::VCENTER, 0);
                if t.afk && nametag_status {
                    let (cr, er) = nametags::afk_rects(r[0], y, [c.left, c.top, c.right, c.bottom]);
                    let flags = dt::CENTER | dt::VCENTER | dt::NOCLIP;
                    let mut rc = Rect { left: cr[0], top: cr[1], right: cr[2], bottom: cr[3] };
                    a22.draw_n(std::ptr::null_mut(), b"C", 1, &mut rc, flags, nametags::AFK_BACKDROP);
                    let mut re = Rect { left: er[0], top: er[1], right: er[2], bottom: er[3] };
                    a20.draw_n(std::ptr::null_mut(), b"E", 1, &mut re, flags, nametags::AFK_ICON);
                }
            }
        }
        s.end();
    }

    /// The 3D text painter shared by labels and chat bubbles: project with samp's view/projection copies, skip
    /// when z > 1, truncate to a rect [x, y, x+1, y+1], then the font renderer's small draw with format 0x101:
    /// when `outline` is set, four black passes of the stripped text (offsets move only left/top), then the main
    /// pass of the raw text through the colour-code wrapper (per-glyph `{RRGGBB}` colours, the base alpha only
    /// up to the first code). Mirrors samp.dll 0x1006BC90.
    fn text3d(&self, s: &Sprite, pos: [f32; 3], text: &[u8], argb: u32, outline: bool) {
        let p = self.project(pos);
        if p[2] > 1.0 {
            return;
        }
        let (x, y) = (p[0] as i32, p[1] as i32);
        let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        let passes: &[(i32, i32)] = if outline { &fonts::OUTLINE_4 } else { &[] };
        Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, text, rect, dt::CENTER, argb, passes);
    }

    /// Chat bubbles (ui-rendering.md, "Chat bubbles"): the 3D text painter with the outline always on and no
    /// line-of-sight test. Mirrors samp.dll 0x10066ED0.
    fn bubbles(&self, bubbles: &[BubbleDraw]) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for b in bubbles {
            self.text3d(s, b.anchor, &b.text, b.colour, true);
        }
        s.end();
    }

    /// 3D text labels: one batch on the 3D-text sprite through the 3D text painter. The outline is on only when
    /// the label colour's alpha is FF; the text keeps its `{RRGGBB}` codes for the colour wrapper. Placement, draw
    /// distance and line of sight come from `entities.rs` (ui-rendering.md, "Smaller overlays").
    /// Mirrors samp.dll 0x10001350.
    fn labels(&self) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for l in crate::entities::label_draws() {
            self.text3d(s, l.pos, &l.text, l.argb, l.argb & 0xFF00_0000 == 0xFF00_0000);
        }
        s.end();
    }

    /// `/dl` vehicle labels on the 3D-text painter with the outline on: four black outline draws of the stripped
    /// text, then the main draw in [`crate::vehlabels::LABEL_ARGB`] through the colour wrapper.
    fn vehicle_labels(&self, rows: &[([f32; 3], String)]) {
        const OUTLINE: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for (pos, text) in rows {
            let p = self.project(*pos);
            if p[2] > 1.0 {
                continue;
            }
            let (x, y) = (p[0] as i32, p[1] as i32);
            let text = colour::strip_codes(text.as_bytes());
            let font = &self.fonts.small;
            for (dx, dy) in OUTLINE {
                let mut r = Rect { left: x + dx, top: y + dy, right: x + 1, bottom: y + 1 };
                font.draw(s.ptr, &text, &mut r, dt::CENTER | dt::NOCLIP, colour::OUTLINE);
            }
            // The main pass goes through the colour-code font wrapper, which samp calls without a sprite pointer.
            let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
            let argb = crate::vehlabels::LABEL_ARGB;
            let glyphs: Vec<u32> = colour::glyph_colours(&text, argb).iter().map(|g| g.colour).collect();
            d3d::draw_glyph_coloured(font, s, &text, &mut r, dt::CENTER | dt::NOCLIP, argb, glyphs);
        }
        s.end();
    }

    /// The number-plate painter: the 128×32 render target is cleared to 0xFFBEB6A8 and the plate text drawn by the
    /// font renderer (Arial 38/10 bold, four 0xB4F0EEE4 outline passes at (0,−2)/(+2,0)/(0,+2)/(−2,0) around
    /// (0, 3), then 0xEE444470; rect right / bottom 128 / 32, format 0x105, colour codes removed), then copied with
    /// a linear filter into a new 64×32 texture (default pool). Returns the texture, 0 on failure.
    /// Mirrors samp.dll B 0x1006D840.
    pub fn paint_plate(&self, text: &[u8]) -> u32 {
        const FMT: u32 = 0x105;
        const OUTLINE: u32 = 0xB4F0_EEE4;
        const TEXT: u32 = 0xEE44_4470;
        let Some(rt) = &self.unused.plate_rt else { return 0 };
        let mut tex = std::ptr::null_mut();
        // 22 = D3DFMT_X8R8G8B8, the format samp's plate painter uses.
        let hr = (self.d3dx.create_texture)(d3d::real_device(), 64, 32, 1, 0, 22, 0, &mut tex);
        if hr < 0 || tex.is_null() {
            return 0;
        }
        rt.begin();
        devcall::clear(3, 0xFFBE_B6A8, 1.0, 0);
        let stripped = colour::strip_codes(text);
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        let font = &self.unused.plate_font;
        // Every pass goes through the font renderer's colour proxy, as in samp: the glyphs land on the renderer's
        // sprite.
        for (dx, dy, argb) in [(0, -2, OUTLINE), (2, 0, OUTLINE), (0, 2, OUTLINE), (-2, 0, OUTLINE), (0, 0, TEXT)] {
            let mut r = Rect { left: dx, top: 3 + dy, right: 128, bottom: 32 };
            let glyphs: Vec<u32> = colour::glyph_colours(&stripped, argb).iter().map(|g| g.colour).collect();
            d3d::draw_glyph_coloured(font, s, &stripped, &mut r, FMT, argb, glyphs);
        }
        s.end();
        rt.end();
        // SAFETY: `tex` is the live texture just created (slot 14 SetAutoGenFilterType, slot 18 GetSurfaceLevel);
        // `surf` is checked for null and released after the copy.
        unsafe {
            let f: extern "system" fn(*mut c_void, u32) -> d3d::HResult = d3d::vfn(tex, 14);
            f(tex, 2);
            let mut surf = std::ptr::null_mut();
            let f: extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> d3d::HResult = d3d::vfn(tex, 18);
            f(tex, 0, &mut surf);
            if !surf.is_null() {
                (self.d3dx.load_surface_from_surface)(
                    surf,
                    std::ptr::null(),
                    std::ptr::null(),
                    rt.surface,
                    std::ptr::null(),
                    std::ptr::null(),
                    3,
                    0,
                );
                d3d::release(surf);
            }
        }
        tex as u32
    }

    /// One object-editor handle: the background glyph (SAMPAUX3 28), then the handle glyph (SAMPAUX3 22), one
    /// character each at the point, format 0x105; the hit rect is centred on the point.
    /// Mirrors samp.dll B 0x10072570.
    fn editor_handle(&self, ed: &mut Editor, index: i32, x: i32, y: i32) {
        let (Some(small), Some(big)) = (&self.unused.editor_fonts.0, &self.unused.editor_fonts.1) else { return };
        let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        big.draw_n(std::ptr::null_mut(), objedit::GLYPH_BACKGROUND, 1, &mut r, objedit::FMT_HANDLE, ed.background(index));
        let (glyph, argb) = ed.glyph_of(index);
        let mut r = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
        small.draw_n(std::ptr::null_mut(), glyph, 1, &mut r, objedit::FMT_HANDLE, argb);
        ed.place(index, x, y);
    }

    /// Both ends of `axis` (±`s` in the matrix frame) on screen; `None` when either z > 1.
    fn axis_ends(&self, m: &[f32; 16], axis: usize, s: f32) -> Option<([f32; 3], [f32; 3])> {
        let mut v = [0.0f32; 3];
        v[axis] = s;
        let a = self.project(objedit::transform(m, v));
        if a[2] > 1.0 {
            return None;
        }
        v[axis] = -s;
        let b = self.project(objedit::transform(m, v));
        (b[2] <= 1.0).then_some((a, b))
    }

    /// The object-editor gizmo: glyph probe, antialiased line per axis (aborting at the first end point behind the
    /// camera), axis handle at the + end; save / scale / rotate / translate buttons left of the − end of Z.
    /// Returns whether all of it was drawn. Mirrors samp.dll B 0x10072C60.
    pub fn editor_gizmo(&self, ed: &mut Editor, m: &[f32; 16], s: f32) -> bool {
        let Some(line) = &self.unused.editor_line else { return false };
        if let Some(big) = &self.unused.editor_fonts.1 {
            // samp passes an uninitialised rect; the size of the result is what it keeps.
            let mut r = Rect::default();
            big.draw_n(std::ptr::null_mut(), objedit::GLYPH_BACKGROUND, -1, &mut r, objedit::FMT_PROBE, objedit::PROBE_COLOUR);
            ed.glyph = (r.right - r.left, r.bottom - r.top);
        }
        line.set_antialias(true);
        let mut last_b = [0.0f32; 3];
        for axis in 0..3 {
            let Some((a, b)) = self.axis_ends(m, axis, s) else { return false };
            line.segment([a[0], a[1]], [b[0], b[1]], objedit::AXIS_COLOURS[axis]);
            ed.set_direction(axis, [a[0], a[1]], [b[0], b[1]]);
            self.editor_handle(ed, axis as i32, a[0] as i32, a[1] as i32);
            last_b = b;
        }
        let (mut x, y) = (last_b[0] as i32, last_b[1] as i32);
        self.editor_handle(ed, objedit::HANDLE_SAVE, x, y);
        let step = -2 - ed.glyph.0;
        if ed.show_scale {
            x += step;
            self.editor_handle(ed, 5, x, y);
        } else {
            ed.clear_rect(5);
        }
        x += step;
        self.editor_handle(ed, 4, x, y);
        x += step;
        self.editor_handle(ed, 3, x, y);
        true
    }

    /// Selection highlight: the object's axis cross (length 1, one colour) and the id / model / distance label
    /// through the 3D-text painter with the outline on and no line-of-sight test (outlined small text, rect
    /// [x, y, x+1, y+1], format 0x101). Mirrors samp.dll B 0x1006DA20.
    pub fn select_highlight(&self, m: &[f32; 16], label: &[u8]) {
        if let Some(line) = &self.unused.editor_line {
            line.set_antialias(true);
            for axis in 0..3 {
                let Some((a, b)) = self.axis_ends(m, axis, objedit::SELECT_CROSS) else { break };
                line.segment([a[0], a[1]], [b[0], b[1]], objedit::SELECT_LINE_COLOUR);
            }
        }
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        let pos = [m[12], m[13], m[14]];
        {
            let p = self.project(pos);
            if p[2] <= 1.0 {
                let (x, y) = (p[0] as i32, p[1] as i32);
                let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
                Self::outlined(
                    s,
                    &self.fonts.small,
                    &self.fonts.small_outline,
                    label,
                    rect,
                    dt::CENTER,
                    objedit::SELECT_LABEL_COLOUR,
                    &fonts::OUTLINE_4,
                );
            }
        }
        s.end();
    }

    /// `/ctd`: one sprite begin; per camera target the editor's axis cross on the entity matrix (length 1) and its
    /// label at the matrix position (3D-text painter, outline on, no line-of-sight test); one sprite end.
    /// Mirrors samp.dll B 0x10004830.
    pub fn camera_target_labels(&self, items: &[([f32; 16], Vec<u8>)]) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for (m, label) in items {
            if let Some(line) = &self.unused.editor_line {
                line.set_antialias(true);
                for axis in 0..3 {
                    let Some((a, b)) = self.axis_ends(m, axis, objedit::SELECT_CROSS) else { break };
                    line.segment([a[0], a[1]], [b[0], b[1]], objedit::SELECT_LINE_COLOUR);
                }
            }
            let p = self.project([m[12], m[13], m[14]]);
            if p[2] <= 1.0 {
                let (x, y) = (p[0] as i32, p[1] as i32);
                let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
                Self::outlined(
                    s,
                    &self.fonts.small,
                    &self.fonts.small_outline,
                    label,
                    rect,
                    dt::CENTER,
                    crate::camtarget::LABEL_COLOUR,
                    &fonts::OUTLINE_4,
                );
            }
        }
        s.end();
    }

    // ---- scoreboard --------------------------------------------------------------------

    /// Scoreboard: DXUT dialog with the list box, then the texts (ui-rendering.md, "Scoreboard").
    fn scoreboard(&self, sb: &ScoreboardDraw, cursor: (i32, i32)) {
        let (w, h) = (video_width(), video_height());
        let (sw, shh) = samp_ui::scoreboard::size(w);
        let (x, y) = (w / 2 - sw / 2, h / 2 - shh / 2);
        let rect = Rect { left: x, top: y, right: x + sw, bottom: y + shh };
        let g = &self.sprites.gui;
        dxut::dialog_begin(g, rect, SB_BACKGROUND, self.gui);
        let list = Rect { left: x, top: y + 60, right: x + sw, bottom: y + shh };
        let cw = [
            samp_ui::round_i(0.085 * f64::from(sw)),
            samp_ui::round_i(0.4375 * f64::from(sw)),
            samp_ui::round_i(0.265625 * f64::from(sw)),
        ];
        let cells: Vec<(u32, Vec<Vec<u8>>)> = sb.rows.iter().map(|r| (r.colour, r.cells.to_vec())).collect();
        let _ = cursor;
        self.list_box(list, &cells, 4, &cw, sb.selected, sb.scroll);
        dxut::dialog_end(g);
        let text_rect = Rect { left: x + 5, top: y + 5, right: x + sw, bottom: y + 60 };
        let flags = dt::SINGLELINE;
        self.small_text(sb.server_name, text_rect, flags, SB_TEXT);
        let mut m = Rect::default();
        self.fonts.small.draw_n(std::ptr::null_mut(), samp_ui::scoreboard::PLAYERS_SAMPLE, -1, &mut m, dt::CALCRECT, 0xFF00_0000);
        let px = x + sw - (m.right - m.left);
        self.small_text(&sb.players, Rect { left: px, ..text_rect }, flags, SB_TEXT);
        let top = y + 60 - self.fonts.small_h - 3;
        let mut hx = x + 10;
        for (i, hdr) in samp_ui::scoreboard::HEADERS.iter().enumerate() {
            let r = Rect { left: hx, top, right: hx + 46, bottom: y + 60 };
            self.small_text(hdr, r, flags, SB_HEADER);
            hx += cw.get(i).copied().unwrap_or(0);
        }
    }

    // ---- DXUT list box -----------------------------------------------------------------

    /// List box (DXUT `CDXUTListBox::Render` with samp's tablist columns): background,
    /// selection bar + rows from the scroll position, then the scrollbar. Each row is
    /// `(text colour, cells)`; `columns` cells are drawn at the column offsets.
    fn list_box(
        &self,
        list: Rect,
        rows: &[(u32, Vec<Vec<u8>>)],
        columns: usize,
        widths: &[i32],
        selected: Option<usize>,
        top_row: usize,
    ) {
        let g = &self.sprites.gui;
        let li = samp_ui::Rect::new(list.left, list.top, list.right, list.bottom);
        dxut::draw_sprite(g, self.gui, dxut::skin::LIST, list, dxut::colours::LIST_TEX);
        let sel = dialog::list_selection_rect(&li);
        let text = dialog::list_text_rect(&li);
        let row_h = self.fonts.gui1_height;
        let page = if row_h > 0 { (text.height() / row_h).max(1) } else { 1 };
        if !rows.is_empty() {
            let mut top = text.top;
            for (i, (colour, cells)) in rows.iter().enumerate().skip(top_row) {
                if top + row_h > text.bottom {
                    break;
                }
                if Some(i) == selected {
                    let r = Rect { left: sel.left, top, right: sel.right, bottom: top + row_h };
                    dxut::draw_sprite(g, self.gui, dxut::skin::LIST_SELECTION, r, dialog::SELECTION_COLOUR);
                }
                // DXUT draws the selected row with the selection element (white text).
                let c = if Some(i) == selected { dialog::ROW_COLOUR } else { *colour };
                let mut x = text.left;
                for k in 0..columns.max(1) {
                    let cell = cells.get(k).map(Vec::as_slice).unwrap_or(b"");
                    let r = Rect { left: x, top, right: text.right, bottom: top + row_h };
                    dxut::draw_text(g, &self.fonts.gui1, cell, r, dxut::fmt::LIST, c);
                    x += widths.get(k).copied().unwrap_or(0);
                }
                top += row_h;
            }
        }
        let sb = ScrollBar {
            rect: Rect { left: list.right - dialog::LIST_SCROLLBAR_W, top: list.top, right: list.right, bottom: list.bottom },
            start: 0,
            end: rows.len() as i32,
            pos: top_row as i32,
            page,
        };
        dxut::scrollbar(g, self.gui, &sb);
    }

    // ---- dialog ------------------------------------------------------------------------

    /// Dialog: DXUT dialog (caption, buttons, list / edit box focused last), then the body or the style-5
    /// headers through the font renderer (ui-rendering.md, "Dialogs", "Per-frame draw").
    fn dialog(&self, d: &Dialog, l: &DialogLayout, cursor: (i32, i32)) {
        let g = &self.sprites.gui;
        let rect = Rect { left: l.rect.left, top: l.rect.top, right: l.rect.right, bottom: l.rect.bottom };
        dxut::dialog_begin(g, rect, DIALOG_BACKGROUND, self.gui);
        if l.caption_h > 0 {
            dxut::caption(g, self.gui, &self.fonts.gui1, rect, l.caption_h, &d.title);
        }
        let rr = |r: samp_ui::Rect| Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let over = |r: samp_ui::Rect| r.contains(cursor.0, cursor.1);
        dxut::button(g, self.gui, &self.fonts.gui0, &d.button1, rr(l.button1), over(l.button1));
        if let Some(b2) = l.button2 {
            dxut::button(g, self.gui, &self.fonts.gui0, &d.button2, rr(b2), over(b2));
        }
        if let Some(list) = l.list {
            let rows: Vec<(u32, Vec<Vec<u8>>)> = d.rows().iter().map(|r| (dialog::ROW_COLOUR, r.clone())).collect();
            let cols = if d.style == Style::List { 1 } else { d.columns() };
            self.list_box(rr(list), &rows, cols, &l.columns, d.selected(), d.top_row());
        }
        if let Some(e) = l.edit {
            let wide = d.edit.display_wide(to_wide);
            let fv = dialog_first_visible(&self.fonts.gui0, &wide, d.edit.caret(), rr(e));
            let view = EditView {
                text: &wide,
                caret: d.edit.caret(),
                anchor: d.edit.anchor(),
                first_visible: fv,
                focused: true,
                insert_mode: d.edit.insert_mode(),
            };
            let f = &self.fonts.gui0;
            dxut::edit_box(g, self.gui, f, rr(e), &view, &|t| gdi_width(f, t));
        }
        dxut::dialog_end(g);
        if let Some(b) = &l.body {
            let r = Rect { left: b.x, top: b.y, right: rect.right, bottom: rect.bottom };
            self.small_text(&b.text, r, dt::EXPANDTABS, b.colour);
        }
        for hdr in &l.headers {
            let r = Rect { left: hdr.x, top: hdr.y, right: rect.right, bottom: rect.bottom };
            self.small_text(&hdr.text, r, dt::SINGLELINE, hdr.colour);
        }
    }

    // ---- death window --------------------------------------------------------------------

    /// Death window: 5 slots from `ftol(0.3·H)`, newest at the bottom (ui-rendering.md, "Death window").
    /// Mirrors samp.dll 0x1006A2E0.
    fn death_window(&self, dw: &DeathWindow) {
        let s = &self.sprites.death;
        let f = &self.fonts;
        let (w, h) = (video_width(), video_height());
        let mut x = (0.75 * w as f32) as i32;
        let mut y = (0.3 * h as f32) as i32;
        if x + f.nick_w * 2 + f.icon_w > w {
            x = w - (f.icon_w + f.nick_w * 2);
        }
        s.begin(d3d::sprite::ALPHABLEND);
        let rows: Vec<_> = dw.rows().collect();
        let empty = deathwindow::ROWS - rows.len().min(deathwindow::ROWS);
        y += empty as i32 * (f.icon_h + 5);
        for r in rows {
            let (left_text, left_colour, right) = if r.killer.is_empty() {
                (&r.victim, r.victim_colour, None)
            } else {
                (&r.killer, r.killer_colour, Some((&r.victim, r.victim_colour)))
            };
            if left_text.is_empty() {
                y += f.icon_h + 5;
                continue;
            }
            let mut m = Rect::default();
            let mc = if r.killer.is_empty() { 0xFF00_0000 } else { 0xFFFF_FFFF };
            f.death_names.draw_n(std::ptr::null_mut(), left_text, -1, &mut m, dt::CALCRECT, mc);
            let lw = m.right - m.left;
            let rect = Rect { left: x + f.nick_w - lw, top: y, right: x + 60, bottom: y + 30 };
            self.death_name(left_text, rect, left_colour);
            let il = x + f.nick_w + 3;
            self.death_icon(Rect { left: il, top: y, right: il + 35, bottom: y + 30 }, r.reason);
            if let Some((vt, vc)) = right {
                let vl = il + f.icon_w;
                self.death_name(vt, Rect { left: vl, top: y, right: vl + 35, bottom: y + 30 }, vc);
            }
            y += f.icon_h + 5;
        }
        s.end();
    }

    /// Death window name with the 4-way outline in the same font. Mirrors samp.dll 0x10069C40.
    fn death_name(&self, text: &[u8], rect: Rect, colour: u32) {
        let s = &self.sprites.death;
        let f = &self.fonts.death_names;
        for &(dx, dy) in &fonts::OUTLINE_4 {
            let mut r = Rect { left: rect.left + dx, top: rect.top + dy, ..rect };
            f.draw_n(s.ptr, text, -1, &mut r, dt::NOCLIP, colour::OUTLINE);
        }
        let mut r = rect;
        f.draw_n(s.ptr, text, -1, &mut r, dt::NOCLIP, colour);
    }

    /// Death window icon box: backdrop `G`, its measure into the rect, the weapon glyph.
    /// Mirrors samp.dll 0x10069D40.
    fn death_icon(&self, rect: Rect, reason: u8) {
        let s = &self.sprites.death;
        let mut r = Rect { top: rect.top - 5, ..rect };
        self.fonts.death_backdrop.draw_n(s.ptr, b"G", -1, &mut r, dt::NOCLIP, 0xFF00_0000);
        self.fonts.death_backdrop.draw_n(s.ptr, b"G", -1, &mut r, dt::CALCRECT | dt::NOCLIP, 0xFF00_0000);
        let g = [deathwindow::glyph(reason)];
        let c = deathwindow::glyph_colour(reason);
        self.fonts.death_icon.draw_n(s.ptr, &g, -1, &mut r, dt::CENTER | dt::VCENTER | dt::NOCLIP, c);
    }

    // ---- chat GUI dialog ---------------------------------------------------------------

    /// The full-screen chat / input GUI dialog: the chat scrollbar and the input edit box (ui-rendering.md,
    /// "Scrollbar and paging", "Chat input").
    fn chat_gui(&mut self, chat: &ChatWindow, inp: &ChatInput, first_visible: usize) {
        let bottom = chat.bottom(self.fonts.main_h);
        let (cw, _) = client_size();
        let box_r = input::box_rect(cw, bottom, self.fonts.gui0_height);
        if inp.is_open() {
            self.host_size = input::host_dialog_size(box_r.width(), bottom);
        }
        let g = &self.sprites.gui;
        let host = Rect { left: 0, top: 0, right: self.host_size.0, bottom: self.host_size.1 };
        dxut::dialog_begin(g, host, CHAT_GUI_BACKGROUND, self.gui);
        if chat.scrollbar_visible(inp.is_open()) {
            let r = chat.scrollbar_rect(self.fonts.main_h);
            let sb = ScrollBar {
                rect: Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
                start: samp_ui::chat::SCROLL_MIN as i32,
                end: samp_ui::chat::SCROLL_END as i32,
                pos: chat.scroll_pos() as i32,
                page: chat.page_size() as i32,
            };
            dxut::scrollbar(g, self.gui, &sb);
        }
        if inp.is_open() {
            let e = inp.edit();
            let wide = to_wide(e.text());
            let view = EditView {
                text: &wide,
                caret: e.caret(),
                anchor: e.anchor(),
                first_visible,
                focused: true,
                insert_mode: e.insert_mode(),
            };
            let r = Rect { left: box_r.left, top: box_r.top, right: box_r.right, bottom: box_r.bottom };
            let f = &self.fonts.gui0;
            dxut::edit_box(g, self.gui, f, r, &view, &|t| gdi_width(f, t));
        }
        dxut::dialog_end(g);
    }

    /// Class-selection panel (ui-rendering.md, "GUI dialogs created by samp"): 310×40 on the scoreboard
    /// background, centred at `y = clientH − 40 − 50`; buttons `<<` (10,5), `>>` (110,5), `Spawn` (210,5), 90×30.
    /// Returns the three button rects.
    pub fn class_panel(&self, cursor: (i32, i32)) -> [samp_ui::Rect; 3] {
        let (cw, ch) = client_size();
        let (x, y) = ((cw - 310) / 2, ch - 40 - 50);
        let rects = [10, 110, 210].map(|dx| samp_ui::Rect::xywh(x + dx, y + 5, 90, 30));
        let g = &self.sprites.gui;
        dxut::dialog_begin(g, Rect { left: x, top: y, right: x + 310, bottom: y + 40 }, SB_BACKGROUND, self.gui);
        for (r, label) in rects.iter().zip([&b"<<"[..], b">>", b"Spawn"]) {
            let rr = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
            dxut::button(g, self.gui, &self.fonts.gui0, label, rr, r.contains(cursor.0, cursor.1));
        }
        dxut::dialog_end(g);
        rects
    }

    /// The chat input's first visible character after a caret move (DXUT `PlaceCaret`).
    pub fn input_first_visible(&self, chat: &ChatWindow, inp: &ChatInput, first: usize) -> usize {
        let (cw, _) = client_size();
        let r = input::box_rect(cw, chat.bottom(self.fonts.main_h), self.fonts.gui0_height);
        let wide = to_wide(inp.edit().text());
        let width = r.width() - 2 * (5 + 4);
        let f = &self.fonts.gui0;
        samp_ui::edit::place_caret(first, inp.edit().caret(), wide.len(), width, &|cp| gdi_width(f, &wide[..cp]))
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

/// First visible character of a dialog edit box (the caret kept in view).
fn dialog_first_visible(font: &Font, text: &[u16], caret: usize, rect: Rect) -> usize {
    let width = rect.right - rect.left - 2 * (5 + 4);
    samp_ui::edit::place_caret(0, caret, text.len(), width, &|cp| gdi_width(font, &text[..cp]))
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

/// The chat window's debug line when its render target cannot be made: the render-to-surface object and the
/// texture have their own lines; both fall back to direct drawing. Mirrors samp.dll 0x100681D0.
fn chat_rt_failure_line(err: &str) -> &'static str {
    if err.contains("RenderToSurface") {
        "ChatWindow: Can't create a render to surface. Will use direct mode."
    } else {
        "ChatWindow: Can't create a render surface texture. Will use direct mode."
    }
}
