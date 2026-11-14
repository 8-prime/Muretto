//! Settings › Custom Shaders Patch › Weather FX: which weather style draws
//! the sky, and the well-known styles that can be added.

use gpui_kit::{
    Context, Div, Entity, EventEmitter, FontWeight, IntoElement, Render, SharedString,
    TestSupportExt as _, Window, div, prelude::*, px, relative, rgb,
};
use muretto_core::packages::{
    recipes::{self, Recipe},
    weather_fx::{self, Settings, Style},
};

use crate::{
    game::Game,
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{button, display, mono, muted, panel, tracked},
};

pub enum WeatherEvent {
    Get(&'static Recipe),
    ShowPatch,
}

pub struct WeatherPage {
    game: Entity<Game>,
    styles: Vec<Style>,
    settings: Settings,
    /// The game's `installed_revision` these were read at.
    read_at: u64,
    /// The recipe whose removal is waiting for a second click.
    confirming: Option<&'static str>,
    error: Option<SharedString>,
}

impl EventEmitter<WeatherEvent> for WeatherPage {}

impl WeatherPage {
    pub fn new(game: Entity<Game>, cx: &mut Context<Self>) -> Self {
        cx.observe(&game, |this, game, cx| {
            // The game also changes on every session edit; only read the
            // disk again when mods may have changed.
            if game.read(cx).installed_revision != this.read_at {
                this.read(cx);
            }
            cx.notify();
        })
        .detach();
        let install = &game.read(cx).install;
        Self {
            styles: weather_fx::styles(install),
            settings: weather_fx::settings(install),
            read_at: game.read(cx).installed_revision,
            game,
            confirming: None,
            error: None,
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let game = self.game.read(cx);
        self.styles = weather_fx::styles(&game.install);
        self.settings = weather_fx::settings(&game.install);
        self.read_at = game.installed_revision;
    }

    /// Reports the outcome of a change to the game folder and makes every
    /// view look at it again.
    fn changed(&mut self, result: muretto_core::Result<()>, cx: &mut Context<Self>) {
        self.confirming = None;
        self.error = result.err().map(|e| e.to_string().into());
        self.game.update(cx, |game, cx| game.refresh_installed(cx));
    }

    fn use_style(&mut self, style: &str, cx: &mut Context<Self>) {
        let install = self.game.read(cx).install.clone();
        // A recipe knows the controller its style goes with.
        let result = match recipe_for(style) {
            Some(recipe) => recipe.use_style(&install, style),
            None => weather_fx::use_style(&install, style, None),
        };
        self.changed(result, cx);
    }

    fn remove(&mut self, recipe: &'static Recipe, cx: &mut Context<Self>) {
        let install = self.game.read(cx).install.clone();
        self.changed(recipe.remove(&install), cx);
    }

    fn styles_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let install = self.game.read(cx).install.clone();
        let rows = self.styles.iter().enumerate().map(|(index, style)| {
            let in_use = self.settings.enabled && style.id == self.settings.style;
            let recipe = recipe_for(&style.id);
            let removable = recipe.filter(|recipe| {
                recipe
                    .installed(&install)
                    .is_some_and(|installed| installed.managed)
            });
            let byline = [style.author.clone(), style.version.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let id = style.id.clone();
            div()
                .flex()
                .items_center()
                .gap(GAP)
                .min_h(px(64.))
                .px(px(16.))
                .py(px(10.))
                .when(index > 0, |row| {
                    row.border_t_1().border_color(rgb(color::BORDER))
                })
                .when(in_use, |row| row.bg(rgb(color::ACCENT_WASH)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(3.))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(style.name.clone()),
                                )
                                .child(mono(byline, 12.).text_color(rgb(color::TEXT_MUTED))),
                        )
                        .when_some(style.description.clone(), |text, description| {
                            text.child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .truncate()
                                    .child(description),
                            )
                        }),
                )
                .map(|row| match removable {
                    Some(recipe) if self.confirming == Some(recipe.id) => row
                        .child(muted(format!("Remove {}?", recipe.name)))
                        .child(button(("style-keep", index), "Keep").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.confirming = None;
                                cx.notify();
                            },
                        )))
                        .child(
                            button(("style-remove-confirm", index), "Remove")
                                .light()
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove(recipe, cx)),
                                ),
                        ),
                    _ => row
                        .when_some(removable, |row, recipe| {
                            row.child(button(("style-remove", index), "Remove").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.confirming = Some(recipe.id);
                                    cx.notify();
                                }),
                            ))
                        })
                        .map(|row| {
                            if in_use {
                                row.child(
                                    tracked("IN USE", px(10.), 0.12)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(color::TEXT)),
                                )
                            } else {
                                row.child(button(("style-use", index), "Use").on_click(
                                    cx.listener(move |this, _, _, cx| this.use_style(&id, cx)),
                                ))
                            }
                        }),
                })
        });
        panel("Weather style")
            .flex_1()
            .children(rows)
            .when(self.styles.is_empty(), |list| {
                list.child(
                    div()
                        .p(px(16.))
                        .child(muted("No weather styles are installed.")),
                )
            })
    }

    fn recipes_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let game = self.game.read(cx);
        let patch = game.patch.clone();
        let install = game.install.clone();
        let cards = recipes::ALL
            .iter()
            .filter(|recipe| recipe.weather.is_some())
            .enumerate()
            .map(|(index, &recipe)| {
                let installed = recipe.installed(&install);
                let state = match &installed {
                    Some(installed) => match &installed.version {
                        Some(version) => format!("Installed · {version}"),
                        None => "Installed".to_owned(),
                    },
                    None => format!("From {}", recipe.page_name),
                };
                let offer = match installed {
                    Some(_) => "Add a newer version…".to_owned(),
                    None => format!("Get {}…", recipe.name),
                };
                let blocked = recipe.patch_shortfall(patch.as_ref());
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .p(px(16.))
                    .when(index > 0, |card| {
                        card.border_t_1().border_color(rgb(color::BORDER))
                    })
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(px(10.))
                            .child(display(recipe.name, 20.))
                            .child(mono(state, 12.).text_color(rgb(color::TEXT_MUTED))),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(relative(1.5))
                            .child(format!("{} By {}.", recipe.summary, recipe.author)),
                    )
                    .map(|card| {
                        if let Some(blocked) = blocked {
                            card.child(muted(blocked)).child(div().flex().child(
                                button(("recipe-needs-patch", index), "Go to the patch").on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(WeatherEvent::ShowPatch)),
                                ),
                            ))
                        } else {
                            card.child(
                                div().flex().child(
                                    button(("recipe-get", index), offer).light().on_click(
                                        cx.listener(move |_, _, _, cx| {
                                            cx.emit(WeatherEvent::Get(recipe))
                                        }),
                                    ),
                                ),
                            )
                        }
                    })
            });
        panel("Well-known styles")
            .w(px(420.))
            .flex_none()
            .children(cards)
    }
}

impl Render for WeatherPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let in_use = self
            .styles
            .iter()
            .find(|style| self.settings.enabled && style.id == self.settings.style)
            .map(|style| style.name.clone());
        let error = self.error.clone();

        div().id("weather-page").size_full().overflow_y_scroll().child(
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
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(display("Weather FX", 27.))
                                .when_some(in_use, |title, name| {
                                    title.child(tag(&format!("Using {name}"), color::GOOD))
                                }),
                        )
                        .child(muted(
                            "How the sky, light and clouds are drawn. Part of Custom Shaders Patch.",
                        )),
                )
                .when_some(error, |page, error| {
                    page.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(GAP)
                            .px(px(16.))
                            .py(px(10.))
                            .bg(rgb(color::SURFACE))
                            .border_1()
                            .border_color(rgb(color::ACCENT_ACTIVE))
                            .rounded(RADIUS)
                            .child(
                                tracked("FAILED", px(10.), 0.12)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(color::ACCENT_TEXT)),
                            )
                            .child(div().flex_1().min_w_0().child(error))
                            .child(button("weather-dismiss", "Dismiss").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.error = None;
                                    cx.notify();
                                },
                            ))),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(GAP)
                        .child(self.styles_panel(cx))
                        .child(self.recipes_panel(cx)),
                ),
        )
        .test_support()
    }
}

fn recipe_for(style: &str) -> Option<&'static Recipe> {
    recipes::ALL
        .iter()
        .copied()
        .find(|recipe| recipe.has_style(style))
}

/// A small outlined word in a colour.
fn tag(text: &str, tone: u32) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(20.))
        .px(px(6.))
        .border_1()
        .border_color(rgb(tone))
        .rounded(RADIUS)
        .child(
            tracked(&text.to_uppercase(), px(10.), 0.12)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone)),
        )
}
