//! Settings: a list of pages on the left, the chosen page on the right.

mod patch;
mod weather;

use std::sync::Arc;

use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, IntoElement, Render,
    TestSupportExt as _, Window, div, prelude::*, px, rgb,
};
use muretto_core::packages::{csp, recipes::Recipe};

use crate::{
    game::Game,
    theme::{PAGE, color},
    ui::{display, muted, tracked},
};
use patch::PatchPage;
use weather::{WeatherEvent, WeatherPage};

/// Pages, grouped as in the sidebar. Only some exist yet.
const GROUPS: [(&str, &[&str]); 3] = [
    (
        "Assetto Corsa",
        &[
            "Video",
            "Audio",
            "Controls",
            "Assists and realism",
            "In-game apps",
            "System",
        ],
    ),
    (
        PATCH_GROUP,
        &[
            PATCH_PAGE,
            "General patch settings",
            "Lighting FX",
            WEATHER_PAGE,
            "Rain FX",
            "Graphics adjustments",
        ],
    ),
    (
        "Launcher",
        &[
            "General",
            "Game and content folders",
            "Runtime",
            "Integrations",
            "About and updates",
        ],
    ),
];
const PATCH_GROUP: &str = "Custom Shaders Patch";
const PATCH_PAGE: &str = "Versions and updates";
const WEATHER_PAGE: &str = "Weather FX";

/// Requests the screen cannot fulfil itself because they cover the window.
pub enum SettingsEvent {
    /// The player wants this mod: show them where to get it and take the
    /// archive from them.
    Get(&'static Recipe),
}

pub struct SettingsScreen {
    page: &'static str,
    patch: Entity<PatchPage>,
    weather: Entity<WeatherPage>,
}

impl EventEmitter<SettingsEvent> for SettingsScreen {}

impl SettingsScreen {
    pub fn new(game: Entity<Game>, source: Arc<dyn csp::Source>, cx: &mut Context<Self>) -> Self {
        let patch = cx.new(|cx| PatchPage::new(game.clone(), source, cx));
        let weather = cx.new(|cx| WeatherPage::new(game, cx));
        cx.observe(&patch, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&weather, |this, _, event, cx| match event {
            WeatherEvent::Get(recipe) => cx.emit(SettingsEvent::Get(recipe)),
            WeatherEvent::ShowPatch => this.show_patch(cx),
        })
        .detach();
        Self {
            page: PATCH_PAGE,
            patch,
            weather,
        }
    }

    pub fn show_patch(&mut self, cx: &mut Context<Self>) {
        self.page = PATCH_PAGE;
        cx.notify();
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, patch_state, patch_tone) = self.patch.read(cx).standing(cx);
        let mut sidebar = div()
            .id("settings-pages")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(232.))
            .h_full()
            .py(px(8.))
            .border_r_1()
            .border_color(rgb(color::BORDER))
            .overflow_y_scroll();
        for (group, pages) in GROUPS {
            // The longest heading breaks in two to leave room for the patch's
            // state beside it.
            let lines = match group.split_once(" Patch") {
                Some((first, _)) => vec![first, "Patch"],
                None => vec![group],
            };
            sidebar = sidebar.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px(PAGE)
                    .pt(px(16.))
                    .pb(px(8.))
                    .child(
                        div().flex().flex_col().children(
                            lines
                                .into_iter()
                                .map(|line| display(line, 14.).text_color(rgb(color::TEXT_MUTED))),
                        ),
                    )
                    .when(group == PATCH_GROUP, |heading| {
                        heading.child(
                            tracked(&patch_state.to_uppercase(), px(10.), 0.12)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(patch_tone)),
                        )
                    }),
            );
            for (index, &page) in pages.iter().enumerate() {
                let chosen = page == self.page;
                sidebar = sidebar.child(
                    div()
                        .id((group, index))
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(px(31.))
                        .px(PAGE)
                        .text_size(px(13.5))
                        .cursor_pointer()
                        .map(|row| {
                            if chosen {
                                row.bg(rgb(color::ACCENT_WASH))
                                    .font_weight(FontWeight::SEMIBOLD)
                            } else {
                                row.hover(|style| style.bg(rgb(color::SURFACE)))
                            }
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = page;
                            cx.notify();
                        }))
                        .child(page)
                        .test_support(),
                );
            }
        }

        let page: AnyElement = match self.page {
            PATCH_PAGE => self.patch.clone().into_any_element(),
            WEATHER_PAGE => self.weather.clone().into_any_element(),
            other => div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .size_full()
                .child(display(other, 27.))
                .child(muted("This part of Muretto has not been built yet."))
                .into_any_element(),
        };
        div()
            .flex()
            .size_full()
            .child(sidebar)
            .child(div().flex_1().min_w_0().h_full().child(page))
    }
}
