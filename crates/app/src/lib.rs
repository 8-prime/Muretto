//! Muretto: a launcher and content manager for Assetto Corsa.

pub mod assets;
pub mod game;
pub mod icon;
pub mod job;
pub mod screens;
pub mod shell;
pub mod theme;
pub mod ui;

use gpui_kit::{
    App, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, point, px, size,
};

pub const APP_NAME: &str = "Muretto";
/// How desktops tell Muretto's windows apart, and the name of its icon.
pub const APP_ID: &str = "dev.muretto.Muretto";

/// Call once, before opening a window.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    theme::load_fonts(cx);
    theme::apply(cx);
}

/// The main window: the size the interface was designed at, centred, with
/// Muretto drawing its own title bar (see `shell::Shell::top_bar`).
pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1440.), px(900.)),
            cx,
        ))),
        window_min_size: Some(size(px(1120.), px(640.))),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            // macOS keeps its traffic lights, over the top bar.
            appears_transparent: true,
            traffic_light_position: Some(point(px(18.), px(20.))),
        }),
        window_decorations: Some(WindowDecorations::Client),
        // The frame Muretto draws leaves room around the window for its
        // shadow; that room has to show what is behind the window.
        window_background: WindowBackgroundAppearance::Transparent,
        icon: icon::window_icon(),
        app_id: Some(APP_ID.to_owned()),
        ..Default::default()
    }
}
