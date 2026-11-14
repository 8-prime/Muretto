//! Muretto: a launcher and content manager for Assetto Corsa.

pub mod assets;
pub mod game;
pub mod job;
pub mod screens;
pub mod shell;
pub mod theme;
pub mod ui;

use gpui_kit::{App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

pub const APP_NAME: &str = "Muretto";

/// Call once, before opening a window.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    theme::load_fonts(cx);
    theme::apply(cx);
}

/// The main window: the size the interface was designed at, centred.
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
            ..Default::default()
        }),
        app_id: Some("dev.muretto.Muretto".to_owned()),
        ..Default::default()
    }
}
