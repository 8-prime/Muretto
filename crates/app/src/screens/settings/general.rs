//! Settings › Launcher › General: how Muretto itself behaves.

use gpui_kit::{Context, IntoElement, div, prelude::*, px, rgb};

use super::{SettingsEvent, SettingsScreen};
use crate::{
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{display, muted, switch},
};

pub const HOSTING_TERMS: &[&str] = &["Server hosting", "Host", "AssettoServer", "Presets"];

pub fn general_page(hosting: bool, cx: &mut Context<SettingsScreen>) -> impl IntoElement {
    div()
        .id("general-page")
        .size_full()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(GAP)
                .p(PAGE)
                .font_family(font::BODY)
                .child(display("General", 27.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .max_w(px(640.))
                        .p(px(16.))
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(rgb(color::BORDER))
                        .bg(rgb(color::SURFACE))
                        .child(
                            switch("hosting", hosting)
                                .caption("Server hosting")
                                .on_toggle({
                                    let screen = cx.entity();
                                    move |on, _, cx| {
                                        screen.update(cx, |screen, cx| screen.set_hosting(on, cx))
                                    }
                                }),
                        )
                        .child(muted(
                            "Adds a Host section for running your own AssettoServer through Muretto's host agent: \
                             add hosts, write presets and switch the server between them. Leave it off if you only join servers.",
                        )),
                ),
        )
}

impl SettingsScreen {
    pub fn set_hosting(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prefs_error = self
            .prefs
            .set_hosting(on)
            .err()
            .map(|error| format!("Could not keep the setting: {error}").into());
        cx.emit(SettingsEvent::Hosting(on));
        cx.notify();
    }

    pub fn hosting(&self) -> bool {
        self.prefs.hosting()
    }
}
