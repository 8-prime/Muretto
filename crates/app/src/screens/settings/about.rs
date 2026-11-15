//! Settings › Launcher › About and updates: which Muretto this is, and where
//! newer ones come from.

use gpui_kit::{IntoElement, div, prelude::*, px};

use crate::{
    theme::{GAP, PAGE, font},
    ui::{button, display, muted, stat},
};

/// The version from Cargo.toml, as released.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/8-prime/Muretto/releases";

pub fn about_page() -> impl IntoElement {
    div()
        .id("about-page")
        .size_full()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(GAP)
                .p(PAGE)
                .font_family(font::BODY)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(display("About and updates", 27.))
                        .child(muted("A launcher and content manager for Assetto Corsa.")),
                )
                .child(stat("Version", VERSION))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP)
                        .child(
                            button("about-releases", "Releases on GitHub")
                                .on_click(|_, _, cx| cx.open_url(RELEASES)),
                        )
                        .child(muted("Muretto does not look for updates by itself yet.")),
                ),
        )
}
