//! Renders the main window offscreen and saves it as a PNG, for checking the
//! interface against its design without opening a window:
//!
//! `cargo run -p muretto --features shot --bin shot -- out.png [element-id …]`
//!
//! Element ids after the path are clicked in order before the picture is
//! taken, e.g. `change-car` to capture the car list or `section:4` for the
//! fifth section. `drop=<file>` hands a file to the open get-a-mod dialog;
//! `type=<text>` types into whatever has focus; `wait=<ms>` waits for
//! background work.

use std::{sync::Arc, time::Duration};

use gpui_kit::{
    Bounds, ElementId, HeadlessAppContext, SharedString, WindowBounds, WindowOptions, prelude::*,
    px, size, test::TestWindowExt as _,
};
use muretto::{
    assets::Assets,
    shell::{Shell, ShowWindowControls, Sources},
};
use muretto_core::{Install, Library};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "muretto.png".to_owned());
    let install = Install::discover().expect("Assetto Corsa is installed");
    let library = Library::scan(&install);

    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        muretto::init(cx);
        cx.set_global(ShowWindowControls);
    });
    let (window, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1440.), px(900.)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Shell::with_content(install, library, Sources::official(), window, cx)
                    })
                },
            )
        })
        .expect("the window opens");

    // Pictures decode in the background; keep drawing until they have landed.
    cx.allow_parking();
    let settle = |cx: &mut HeadlessAppContext| {
        for _ in 0..12 {
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .expect("the window draws");
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(40));
        }
    };
    settle(&mut cx);
    for id in std::env::args().skip(2) {
        // "drop=/path/to.zip" hands a file to the open get-a-mod dialog, as
        // dropping it on the window would.
        if let Some(path) = id.strip_prefix("drop=") {
            cx.update(|cx| {
                let dialog = shell.read(cx).getting().expect("a dialog is open").clone();
                dialog.update(cx, |dialog, cx| dialog.accept(path.into(), cx));
            });
            settle(&mut cx);
            continue;
        }
        // "wait=3000" lets background work, such as pings, land.
        if let Some(ms) = id.strip_prefix("wait=").and_then(|ms| ms.parse().ok()) {
            let until = std::time::Instant::now() + Duration::from_millis(ms);
            while std::time::Instant::now() < until {
                settle(&mut cx);
            }
            continue;
        }
        // "type=text" types into whatever has focus.
        if let Some(text) = id.strip_prefix("type=") {
            cx.update_window(window, |_, window, cx| window.input(text, cx))
                .expect("the text is typed");
            settle(&mut cx);
            continue;
        }
        // "section:4" names the fifth of a numbered family of elements.
        let numbered = id
            .split_once(':')
            .and_then(|(name, n)| Some((name.to_owned(), n.parse().ok()?)));
        let id = match numbered {
            Some((name, index)) => ElementId::NamedInteger(SharedString::from(name), index),
            None => ElementId::Name(SharedString::from(id)),
        };
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .expect("the element is clicked");
        settle(&mut cx);
    }

    cx.capture_screenshot(window)
        .expect("offscreen rendering is available")
        .save(&out)
        .expect("the picture is written");
    println!("{out}");
}
