use gpui_kit::prelude::*;
use muretto::{assets::Assets, shell::Shell};

fn main() {
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
