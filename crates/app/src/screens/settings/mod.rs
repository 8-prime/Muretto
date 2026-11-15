//! Settings: a list of pages on the left, the chosen page on the right.

mod about;
mod controls;
mod form;
mod page;
mod pages;
mod patch;
mod weather;

use std::sync::Arc;

use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, IntoElement, Render, SharedString,
    Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::*,
    px, rgb, svg,
};
use muretto_core::packages::{csp, recipes::Recipe};

use crate::{
    game::Game,
    theme::{GAP, PAGE, RADIUS, color},
    ui::{button, display, muted, tracked},
};
use controls::ControlsPage;
use page::ConfigPage;
use pages::{APPS_PAGE, ASSISTS_PAGE, AUDIO_PAGE, SYSTEM_PAGE, VIDEO_PAGE};
use patch::PatchPage;
use weather::{WeatherEvent, WeatherPage};

const CONTROLS_PAGE: &str = "Controls";

/// Pages, grouped as in the sidebar. Only some exist yet.
const GROUPS: [(&str, &[&str]); 3] = [
    (
        "Assetto Corsa",
        &[
            VIDEO_PAGE.name,
            AUDIO_PAGE.name,
            CONTROLS_PAGE,
            ASSISTS_PAGE.name,
            APPS_PAGE.name,
            SYSTEM_PAGE.name,
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
            ABOUT_PAGE,
        ],
    ),
];
const PATCH_GROUP: &str = "Custom Shaders Patch";
const PATCH_PAGE: &str = "Versions and updates";
const WEATHER_PAGE: &str = "Weather FX";
const ABOUT_PAGE: &str = "About and updates";

/// Requests the screen cannot fulfil itself because they cover the window.
pub enum SettingsEvent {
    /// The player wants this mod: show them where to get it and take the
    /// archive from them.
    Get(&'static Recipe),
}

pub struct SettingsScreen {
    game: Entity<Game>,
    page: &'static str,
    patch: Entity<PatchPage>,
    weather: Entity<WeatherPage>,
    controls: Entity<ControlsPage>,
    /// The game's settings pages built from data, by name.
    config_pages: Vec<(&'static str, Entity<ConfigPage>)>,
    search: Entity<InputState>,
    query: String,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsScreen {}

impl SettingsScreen {
    pub fn new(
        game: Entity<Game>,
        source: Arc<dyn csp::Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let patch = cx.new(|cx| PatchPage::new(game.clone(), source, cx));
        let weather = cx.new(|cx| WeatherPage::new(game.clone(), cx));
        let controls = cx.new(|cx| ControlsPage::new(game.clone(), window, cx));
        let config_pages = [
            &VIDEO_PAGE,
            &AUDIO_PAGE,
            &ASSISTS_PAGE,
            &APPS_PAGE,
            &SYSTEM_PAGE,
        ]
        .into_iter()
        .map(|page| {
            let view = cx.new(|cx| ConfigPage::new(page, game.clone(), window, cx));
            (page.name, view)
        })
        .collect();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search all settings"));
        cx.observe(&patch, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&weather, |this, _, event, cx| match event {
            WeatherEvent::Get(recipe) => cx.emit(SettingsEvent::Get(recipe)),
            WeatherEvent::ShowPatch => this.show_patch(cx),
        })
        .detach();
        let _subscriptions = vec![
            cx.subscribe(&search, |this, search, event, cx| {
                if let InputEvent::Change = event {
                    this.query = search.read(cx).value().trim().to_owned();
                    // Show the first page that has what was asked for.
                    if this.found(this.page, cx).is_none() {
                        let first = GROUPS
                            .iter()
                            .flat_map(|(_, pages)| pages.iter().copied())
                            .find(|page| this.found(page, cx).is_some());
                        if let Some(page) = first {
                            this.page = page;
                        }
                    }
                    cx.notify();
                }
            }),
            // Errors writing the game's files show above every page.
            cx.observe(&game, |_, _, cx| cx.notify()),
        ];
        Self {
            game,
            page: VIDEO_PAGE.name,
            patch,
            weather,
            controls,
            config_pages,
            search,
            query: String::new(),
            _subscriptions,
        }
    }

    /// Words each page can be found by, besides its name.
    fn terms(&self, page: &str, cx: &Context<Self>) -> Vec<SharedString> {
        if page == CONTROLS_PAGE {
            return self.controls.read(cx).terms();
        }
        if let Some((_, view)) = self.config_pages.iter().find(|(name, _)| *name == page) {
            return view.read(cx).terms();
        }
        let words: &[&str] = match page {
            PATCH_PAGE => &["Custom Shaders Patch", "CSP", "Install", "Update", "Proton"],
            WEATHER_PAGE => &["Weather", "Sky", "Clouds", "Pure", "Sol"],
            ABOUT_PAGE => &["Version", "Release"],
            _ => &[],
        };
        words.iter().map(|w| SharedString::from(*w)).collect()
    }

    /// What a search found on `page`: `None` when the page does not match,
    /// otherwise the settings that did (empty when only its name did).
    fn found(&self, page: &str, cx: &Context<Self>) -> Option<Vec<SharedString>> {
        if self.query.is_empty() {
            return Some(Vec::new());
        }
        let query = self.query.to_lowercase();
        let hits: Vec<SharedString> = self
            .terms(page, cx)
            .into_iter()
            .filter(|term| term.to_lowercase().contains(&query))
            .collect();
        (page.to_lowercase().contains(&query) || !hits.is_empty()).then_some(hits)
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
        let mut shown = 0;
        for (group, pages) in GROUPS {
            let found: Vec<(usize, &'static str, Vec<SharedString>)> = pages
                .iter()
                .enumerate()
                .filter_map(|(index, &page)| Some((index, page, self.found(page, cx)?)))
                .collect();
            if found.is_empty() {
                continue;
            }
            shown += found.len();
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
            for (index, page, hits) in found {
                let chosen = page == self.page;
                let hits = (!hits.is_empty()).then(|| {
                    let mut shown: Vec<&str> = hits.iter().take(3).map(|h| h.as_ref()).collect();
                    if hits.len() > 3 {
                        shown.push("…");
                    }
                    shown.join(" · ")
                });
                sidebar = sidebar.child(
                    div()
                        .id((group, index))
                        .flex()
                        .flex_col()
                        .flex_none()
                        .justify_center()
                        .min_h(px(31.))
                        .px(PAGE)
                        .py(px(5.))
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
                        .when_some(hits, |row, hits| {
                            row.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(color::TEXT_DIM))
                                    .font_weight(FontWeight::NORMAL)
                                    .truncate()
                                    .child(hits),
                            )
                        })
                        .test_support(),
                );
            }
        }
        if shown == 0 {
            sidebar = sidebar.child(div().px(PAGE).pt(px(16.)).child(muted("Nothing matches.")));
        }

        let page: AnyElement = match self.page {
            PATCH_PAGE => self.patch.clone().into_any_element(),
            WEATHER_PAGE => self.weather.clone().into_any_element(),
            CONTROLS_PAGE => self.controls.clone().into_any_element(),
            ABOUT_PAGE => about::about_page().into_any_element(),
            other if let Some((_, view)) = self.config_pages.iter().find(|(n, _)| *n == other) => {
                view.clone().into_any_element()
            }
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
        let error = self.game.read(cx).config_error.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(GAP)
                    .h(px(52.))
                    .px(PAGE)
                    .border_b_1()
                    .border_color(rgb(color::BORDER))
                    .child(
                        div().w(px(308.)).child(Styled::h(
                            Input::new(&self.search)
                                .id("settings-search")
                                .bg(rgb(color::SURFACE))
                                .prefix(
                                    svg()
                                        .path("icons/search.svg")
                                        .size(px(14.))
                                        .text_color(rgb(color::TEXT_DIM)),
                                ),
                            px(32.),
                        )),
                    )
                    .child(muted(
                        "Changes are written to the game's config files and apply on the next launch.",
                    )),
            )
            .when_some(error, |screen, error| {
                screen.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(GAP)
                        .mx(PAGE)
                        .mt(px(12.))
                        .px(px(16.))
                        .py(px(10.))
                        .bg(rgb(color::SURFACE))
                        .border_1()
                        .border_color(rgb(color::ACCENT_ACTIVE))
                        .rounded(RADIUS)
                        .child(
                            tracked("NOT SAVED", px(10.), 0.12)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(color::ACCENT_TEXT)),
                        )
                        .child(div().flex_1().min_w_0().text_size(px(13.)).child(error))
                        .child(button("settings-error-dismiss", "Dismiss").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.game.update(cx, |game, cx| {
                                    game.config_error = None;
                                    cx.notify();
                                });
                            }),
                        )),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    .child(div().flex_1().min_w_0().h_full().child(page)),
            )
    }
}
