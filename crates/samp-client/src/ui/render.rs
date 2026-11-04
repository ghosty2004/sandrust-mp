//! The D3D UI pass (Windows only), drawn in the original's order and with its call
//! pattern (UR §2.3; TRACE-3 D3D logs of the original at 1280×720, see
//! docs/phase3/m8a-notes.md):
//!
//! 1. nametag bars (painter sprite `Begin(DONOTSAVESTATE)` + UI states + FVF 0x44 fans),
//! 2. nametag names (painter sprite `Begin(ALPHABLEND)`: measure, 4 outlines, main,
//!    AFK `C` measure and glyphs),
//! 3. chat bubbles, 4. 3D labels (3D-text sprite, one Begin/End each),
//! 5. scoreboard → stop (only the HUD flags change),
//! 6. chat: blit of the 2048×1024 render target (sprite Begin/Draw/End), the target
//!    being redrawn in the game frame only when dirty or scrolled (UR §7.6),
//! 7. chat input (cursor mode 2), 8. death window (own sprite),
//! 9. dialog (cursor mode 2, DXUT dialog, then body/headers via the font renderer),
//! 10. the chat/input GUI dialog (full-screen DXUT dialog: scrollbar, edit box).

use std::ffi::c_void;

use samp_ui::chat::{ChatLayout, ChatWindow};
use samp_ui::colour;
use samp_ui::deathwindow::{self, DeathWindow};
use samp_ui::dialog::{self, Dialog, DialogLayout, Style};
use samp_ui::fonts::{self, FontId, Size, TextMeasure};
use samp_ui::input::{self, ChatInput};
use samp_ui::nametags;
use samp_ui::scoreboard::Row;

use super::d3d::{self, D3dx, Font, FontArgs, Rect, RenderTarget, Sprite, devcall, dt};
use super::dxut::{self, EditView, ScrollBar};

/// `RsGlobal.maximumWidth` / `maximumHeight` (UR §3.2 `W`).
const RS_MAX_WIDTH: u32 = 0xC17044;
const RS_MAX_HEIGHT: u32 = 0xC17048;
/// GTA main window handle.
const GTA_HWND: u32 = 0xC97C1C;
/// Scoreboard texts / headers colours (UR §15.1).
const SB_TEXT: u32 = 0xFFBE_BEBE;
const SB_HEADER: u32 = 0xFF95_B0D0;
const SB_BACKGROUND: u32 = 0x960A_0A0A;
/// Chat GUI dialog background (TRACE-3: drawn, fully transparent).
const CHAT_GUI_BACKGROUND: u32 = 0x0000_0000;
/// Dialog background (UR §13.1).
const DIALOG_BACKGROUND: u32 = 0xDC05_0505;

pub fn video_width() -> i32 {
    // SAFETY: RsGlobal field read on the main thread.
    unsafe { std::ptr::read_volatile(RS_MAX_WIDTH as usize as *const i32) }
}

pub fn video_height() -> i32 {
    // SAFETY: as above.
    unsafe { std::ptr::read_volatile(RS_MAX_HEIGHT as usize as *const i32) }
}

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

/// Text width with the font's GDI DC (what DXUT's Uniscribe buffer measures; not a
/// D3DX call, so it does not show in the call stream).
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
    /// Height of "Y" in the main / small font (font renderer +0x20/+0x24).
    pub main_h: i32,
    pub small_h: i32,
    pub gui0_height: i32,
    pub gui1_height: i32,
    /// Death window metrics (+0x12b nick width, +0x12f/+0x133 glyph size).
    nick_w: i32,
    icon_w: i32,
    icon_h: i32,
}

/// Builds the font-renderer, GUI and death-window fonts (UR §3.2 table, TRACE-3).
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

/// The sprites samp keeps per painter (each a separate `ID3DXSprite`).
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
    fn create(d3dx: &D3dx) -> Result<Self, String> {
        Ok(Self {
            text: Sprite::create(d3dx)?,
            chat: Sprite::create(d3dx)?,
            blit: Sprite::create(d3dx)?,
            gui: Sprite::create(d3dx)?,
            death: Sprite::create(d3dx)?,
            tags: Sprite::create(d3dx)?,
            text3d: Sprite::create(d3dx)?,
        })
    }
    fn all(&self) -> [&Sprite; 7] {
        [&self.text, &self.chat, &self.blit, &self.gui, &self.death, &self.tags, &self.text3d]
    }
}

/// One nametag to consider this frame.
#[derive(Clone, Debug)]
pub struct TagDraw {
    /// Head bone position (bone 8).
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
}

/// The renderer: D3DX objects created on the first Present.
pub struct Renderer {
    pub d3dx: D3dx,
    sprites: Sprites,
    pub fonts: Fonts,
    /// SAMPAUX3 20 / 22, created lazily on the first nametag frame (TRACE-3).
    aux: Option<(Font, Font)>,
    gui: *mut c_void,
    cursor: *mut c_void,
    rt: Option<RenderTarget>,
    rt_valid: bool,
    rt_scroll: u32,
    built_for: (i32, i32),
    /// Size of the chat GUI dialog: the video mode, then `(w + 100) × (bottom + 50)`
    /// once the input was opened (UR §8).
    host_size: (i32, i32),
}

impl Renderer {
    pub fn new(face: &[u8], weight: u32, font_size: i32) -> Result<Self, String> {
        let d3dx = D3dx::load()?;
        let w = video_width();
        let fonts = build_fonts(&d3dx, w, font_size, face, weight)?;
        let sprites = Sprites::create(&d3dx)?;
        let gui = dxut::load_skin(&d3dx);
        let cursor = match d3d::create_cursor(&d3dx, b"mouse.png") {
            Ok(s) => s,
            Err(e) => {
                crate::log!("ui: cursor: {e}");
                std::ptr::null_mut()
            }
        };
        let mut r = Self {
            d3dx,
            sprites,
            fonts,
            aux: None,
            gui,
            cursor,
            rt: None,
            rt_valid: false,
            rt_scroll: 0,
            built_for: (w, font_size),
            host_size: (w, video_height()),
        };
        r.create_rt();
        Ok(r)
    }

    /// Chat render target: 1024×512 at an adapter width ≤ 1024, else 2048×1024
    /// (UR §7.6); `directmode` is not supported (TODO-unknown: config read).
    fn create_rt(&mut self) {
        let (w, h) = if video_width() <= 1024 { (1024, 512) } else { (2048, 1024) };
        match RenderTarget::create(&self.d3dx, w, h) {
            Ok(rt) => self.rt = Some(rt),
            Err(e) => crate::log!("ui: chat render target unavailable ({e}); direct drawing"),
        }
        self.rt_valid = false;
    }

    /// `/fontsize` or a video-mode change: rebuild all fonts (TRACE-3: the GUI fonts
    /// are recreated at once too).
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

    pub fn needs_rebuild(&self, font_size: i32) -> bool {
        self.built_for != (video_width(), font_size)
    }

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
    }

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
        self.create_rt();
        // UR §2.2: the cursor surface is re-created after a reset.
        // SAFETY: we own the old surface.
        unsafe { d3d::release(self.cursor) };
        self.cursor = d3d::create_cursor(&self.d3dx, b"mouse.png").unwrap_or(std::ptr::null_mut());
        self.host_size = (video_width(), video_height());
    }

    // ---- font renderer -----------------------------------------------------------------

    /// Outline passes + the main pass through the colour-code wrapper, all with `sprite`.
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
        // TRACE-3: the 4-way passes move only left/top; the 8-way chat passes (W > 1280)
        // move the whole rect.
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

    /// Font renderer small-font draw with outline (A 0x1006b4e0): its own Begin/End.
    fn small_text(&self, text: &[u8], rect: Rect, flags: u32, colour: u32) {
        let s = &self.sprites.text;
        s.begin(d3d::sprite::ALPHABLEND);
        Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, text, rect, flags, colour, &fonts::OUTLINE_4);
        s.end();
    }

    // ---- chat ----------------------------------------------------------------------------

    /// Draws the visible chat lines with `sprite` (UR §7.2).
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

    /// Render-target update, once per game frame from the `CGame::Process` hook
    /// (A 0x10067ed0): invalid while the GTA menu is on; redrawn when the chat is dirty
    /// or the scroll position changed.
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
        devcall::set_render_state(20, 14); // DESTBLEND (UR §7.6)
        self.chat_lines(&layout);
        s.end();
        rt.end();
        self.rt_valid = true;
        self.rt_scroll = chat.scroll_pos();
        chat.clear_dirty();
    }

    /// Present step: the chat window (UR §7.5).
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

    /// Bars pass (A 0x10074270 / 0x1006d0a0).
    fn tag_bars(&self, tags: &[TagDraw]) {
        let s = &self.sprites.tags;
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
    }

    /// Names pass (A 0x10075390 / 0x1006cda0) incl. the AFK icon (UR §16.3).
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

    /// Chat bubbles (UR §11): small font, outline, `DT_CENTER|DT_NOCLIP` on [x, y, x+1, y+2].
    fn bubbles(&self, bubbles: &[BubbleDraw]) {
        let s = &self.sprites.text3d;
        s.begin(d3d::sprite::ALPHABLEND);
        for b in bubbles {
            let p = self.project(b.anchor);
            if p[2] > 1.0 {
                continue;
            }
            let (x, y) = (p[0] as i32, p[1] as i32);
            let rect = Rect { left: x, top: y, right: x + 1, bottom: y + 2 };
            Self::outlined(s, &self.fonts.small, &self.fonts.small_outline, &b.text, rect, dt::CENTER, b.colour, &fonts::OUTLINE_4);
        }
        s.end();
    }

    // ---- scoreboard --------------------------------------------------------------------

    /// Scoreboard (UR §15): DXUT dialog with the list box, then the texts.
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

    /// Dialog (UR §13.3): DXUT dialog (caption, buttons, list/edit box focused last),
    /// then the body or the style-5 headers through the font renderer.
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

    /// Death window (A 0x1006a2e0): 5 slots from `ftol(0.3·H)`, newest at the bottom.
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

    /// Name with the 4-way outline, same font (A 0x10069c40).
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

    /// Icon box (A 0x10069d40): backdrop `G`, its measure into the rect, the glyph.
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

    /// The chat/input GUI dialog (0x1026ea9c): scrollbar and edit box (UR §7.4, §8).
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

    /// Class-selection panel (GUI dialog 0x1026eaa4, UR §4.2): 310×40, background
    /// 0x960A0A0A, centred at `y = clientH − 40 − 50`; buttons `<<` (10,5), `>>` (110,5),
    /// `Spawn` (210,5), 90×30. Returns the three button rects.
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

    /// The Present pass after the HUD decision (UR §2.3 steps 4–12). Returns early for
    /// the scoreboard (steps 6–12 skipped).
    pub fn frame(&mut self, f: &Frame, cursor_req: &mut dyn FnMut(CursorReq)) {
        self.tag_bars(f.tags);
        self.tag_names(f.tags, f.nametag_status);
        if let Some(sb) = &f.scoreboard {
            self.scoreboard(sb, f.cursor);
            return;
        }
        self.bubbles(f.bubbles);
        // 3D text labels (none before M6): an empty batch like the original.
        self.sprites.text3d.begin(d3d::sprite::ALPHABLEND);
        self.sprites.text3d.end();
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
