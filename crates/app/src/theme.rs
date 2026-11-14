//! Muretto's visual language: one dark palette, three typefaces, square
//! corners. Everything on screen takes its colours and type from here.

use std::borrow::Cow;

use gpui_kit::{
    App, Hsla, Pixels, Rgba,
    component::{Theme, ThemeMode},
    px, rgb, rgba,
};

/// Colours as `0xRRGGBB`.
pub mod color {
    /// Window background; also the inside of inputs and unselected toggles.
    pub const BG: u32 = 0x0D0E10;
    /// Panels, bars and buttons.
    pub const SURFACE: u32 = 0x16171B;
    /// Insets that sit on a panel, and hovered rows.
    pub const RAISED: u32 = 0x1C1F24;
    /// Small filled chips.
    pub const CHIP: u32 = 0x24262B;
    pub const BORDER: u32 = 0x2B2C30;
    pub const BORDER_STRONG: u32 = 0x3C3E44;

    pub const TEXT: u32 = 0xECEDEF;
    pub const TEXT_MUTED: u32 = 0xA2A6AB;
    pub const TEXT_DIM: u32 = 0x7B8085;

    pub const ACCENT: u32 = 0xD9281E;
    pub const ACCENT_HOVER: u32 = 0xE5382D;
    pub const ACCENT_ACTIVE: u32 = 0xBF2018;
    /// Accent used as text on dark surfaces, lightened to stay legible.
    pub const ACCENT_TEXT: u32 = 0xF2584B;
    /// Selected rows: the accent at low strength over the background.
    pub const ACCENT_WASH: u32 = 0x2A1513;

    pub const GOOD: u32 = 0x57D688;
    pub const WARN: u32 = 0xE5A93B;

    /// Light buttons and slider tracks.
    pub const LIGHT: u32 = 0xECEDEF;
    pub const SWITCH_OFF: u32 = 0x3A3D44;
    pub const SWITCH_KNOB_OFF: u32 = 0xC9CCD1;
}

pub mod font {
    /// Headings, navigation and the launch button. Always bold italic caps.
    pub const DISPLAY: &str = "Barlow Condensed";
    pub const BODY: &str = "Inter";
    /// Figures: lap times, specifications, counts.
    pub const MONO: &str = "JetBrains Mono";
}

/// Corner radius for every control.
pub const RADIUS: Pixels = px(2.);
/// Outer page margin.
pub const PAGE: Pixels = px(20.);
/// Space between and inside panels.
pub const GAP: Pixels = px(16.);

pub fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// `hex` at `alpha` (0–1).
pub fn alpha(hex: u32, alpha: f32) -> Rgba {
    rgba((hex << 8) | (alpha.clamp(0., 1.) * 255.) as u32)
}

pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!(
            "../assets/fonts/BarlowCondensed-BoldItalic.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../assets/fonts/BarlowCondensed-SemiBoldItalic.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../assets/fonts/BarlowCondensed-SemiBold.ttf"
        )),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf")),
    ];
    cx.text_system()
        .add_fonts(fonts)
        .expect("bundled fonts are valid");
}

/// Makes the component library's inputs, menus and scrollbars match.
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = font::BODY.into();
        theme.font_size = px(13.);
        theme.mono_font_family = font::MONO.into();
        theme.mono_font_size = px(13.);
        theme.radius = RADIUS;
        theme.radius_lg = RADIUS;
        theme.shadow = false;
        // The frame Muretto draws around its window on Linux.
        theme.window_radius = px(0.);

        let c = &mut theme.colors;
        c.background = hsla(color::BG);
        c.foreground = hsla(color::TEXT);
        c.border = hsla(color::BORDER);
        c.window_border = hsla(color::BORDER_STRONG);
        c.input = hsla(color::BORDER);
        c.ring = hsla(color::BORDER_STRONG);
        c.caret = hsla(color::TEXT);
        c.selection = alpha(color::ACCENT, 0.45).into();
        c.muted = hsla(color::RAISED);
        c.muted_foreground = hsla(color::TEXT_DIM);

        c.primary = hsla(color::ACCENT);
        c.primary_hover = hsla(color::ACCENT_HOVER);
        c.primary_active = hsla(color::ACCENT_ACTIVE);
        c.primary_foreground = hsla(0xFFFFFF);
        c.secondary = hsla(color::SURFACE);
        c.secondary_hover = hsla(color::RAISED);
        c.secondary_active = hsla(color::CHIP);
        c.secondary_foreground = hsla(color::TEXT);
        c.accent = hsla(color::RAISED);
        c.accent_foreground = hsla(color::TEXT);

        c.popover = hsla(color::SURFACE);
        c.popover_foreground = hsla(color::TEXT);
        c.list = hsla(color::SURFACE);
        c.list_hover = hsla(color::RAISED);
        c.list_active = hsla(color::ACCENT_WASH);
        c.list_active_border = hsla(color::ACCENT);

        c.slider_bar = hsla(color::ACCENT);
        c.slider_thumb = hsla(color::ACCENT);
        c.switch = hsla(color::SWITCH_OFF);
        c.switch_thumb = hsla(0xFFFFFF);
        c.scrollbar = hsla(color::BG).opacity(0.);
        c.scrollbar_thumb = hsla(color::BORDER_STRONG);
        c.scrollbar_thumb_hover = hsla(color::TEXT_DIM);
    });
}
