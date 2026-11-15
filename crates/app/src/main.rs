// Release builds on Windows open no console window beside the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use gpui_kit::prelude::*;
use muretto::{assets::Assets, shell::Shell};

fn main() {
    #[cfg(target_os = "linux")]
    muretto::icon::install_desktop_entry();
    gpui_kit::application().with_assets(Assets).run(|cx| {
        muretto::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = muretto::window_options(cx);
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .expect("the main window opens");
        cx.activate(true);
    });
}
