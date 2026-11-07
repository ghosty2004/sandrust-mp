//! The fonts and sprites the UI draws with, and the metrics samp measures when it creates the fonts.

use samp_ui::deathwindow;
use samp_ui::fonts::{self, FontId, Size, TextMeasure};

use super::{text_font_args, weapon_font_args};
use crate::ui::d3d::{D3dx, Font, Rect, Sprite, dt};

/// The fonts the UI draws with, plus the metrics samp measures once when it creates them.
pub struct Fonts {
    pub(super) main: Font,
    pub(super) main_outline: Font,
    pub(super) small: Font,
    pub(super) small_outline: Font,
    pub(super) gui0: Font,
    pub(super) gui1: Font,
    pub(super) death_names: Font,
    pub(super) death_icon: Font,
    pub(super) death_backdrop: Font,
    /// Height of "Y" in the main / small font (samp keeps these in its font renderer at +0x20 / +0x24).
    pub main_h: i32,
    pub small_h: i32,
    pub gui0_height: i32,
    pub gui1_height: i32,
    /// Death window metrics: nick column width and weapon glyph size (death window object +0x12b, +0x12f/+0x133).
    pub(super) nick_w: i32,
    pub(super) icon_w: i32,
    pub(super) icon_h: i32,
    /// Chat timestamp column sample width (measured once, like samp's chat window).
    pub(super) timestamp_w: i32,
}

/// Builds the font-renderer, GUI and death-window fonts for a video width and font size, after `/fontsize` or a
/// mode change (ui-rendering.md, "Size and face settings").
pub(super) fn build_fonts(d3dx: &D3dx, video_w: i32, fs: i32, face: &[u8], weight: u32) -> Result<Fonts, String> {
    let main_h = fonts::main_height(video_w, fs);
    let small_h = fonts::small_height(video_w, fs);
    let gui_h = fonts::gui_height(fs);
    let aux_h = fonts::aux_height(video_w, fs);
    let args = |height: i32, quality: u32| text_font_args(face, weight, height, quality);
    let main = Font::create(d3dx, args(main_h, 4))?;
    let main_outline = Font::create(d3dx, args(main_h, 4))?;
    let small = Font::create(d3dx, args(small_h, 4))?;
    let small_outline = Font::create(d3dx, args(small_h, 4))?;
    let gui0 = Font::create(d3dx, args(gui_h, 0))?;
    let gui1 = Font::create(d3dx, args(gui_h - 2, 0))?;
    let death_names = Font::create(d3dx, args(aux_h, 0))?;
    let death_icon = Font::create(d3dx, weapon_font_args(aux_h + 8))?;
    let death_backdrop = Font::create(d3dx, weapon_font_args(aux_h + 12))?;
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

    pub(super) fn all(&self) -> [&Font; 9] {
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
pub(super) struct Sprites {
    /// Font renderer (+0x14).
    pub(super) text: Sprite,
    /// Chat window direct sprite (+0x63a6), also used inside the render target.
    pub(super) chat: Sprite,
    /// Chat render-target blit sprite (+0x63aa).
    pub(super) blit: Sprite,
    /// GUI resource manager sprite (DXUT).
    pub(super) gui: Sprite,
    /// Death window.
    pub(super) death: Sprite,
    /// Nametag/bar painter.
    pub(super) tags: Sprite,
    /// 3D-anchored text painter (bubbles, labels).
    pub(super) text3d: Sprite,
}

impl Sprites {
    pub(super) fn all(&self) -> [&Sprite; 7] {
        [&self.text, &self.chat, &self.blit, &self.gui, &self.death, &self.tags, &self.text3d]
    }
}
