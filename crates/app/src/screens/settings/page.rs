//! A settings page built from a [`Page`]: its sections as panels in two
//! columns, plus whatever extra the page asks for.

use gpui_kit::{
    AnyElement, Context, Div, Entity, IntoElement, Render, SharedString, TestSupportExt as _,
    Window, div, prelude::*, px, rgb,
};
use muretto_core::{
    session::AssistPreset,
    settings::{self, apps},
};

use super::{
    form::{Form, switch_row, toggle_row, write},
    pages::{Extra, PYTHON_APPS, Page},
};
use crate::{
    game::Game,
    theme::{GAP, PAGE, color, font},
    ui::{button, choice_tile, display, mono, muted, panel},
};

/// Every file the System page's "Restore all" puts back. Assists are left
/// alone: they belong to the session.
const ALL_FILES: [&str; 8] = [
    settings::VIDEO,
    settings::CAMERA,
    settings::AUDIO,
    settings::CONTROLS,
    settings::FF_POST,
    settings::GAMEPLAY,
    settings::PYTHON,
    settings::REPLAY,
];

pub struct ConfigPage {
    game: Entity<Game>,
    page: &'static Page,
    /// One per section.
    forms: Vec<Form>,
    /// Python apps found in the game folder.
    apps: Vec<String>,
    /// Which restore is waiting for a second click.
    confirming: Option<&'static [&'static str]>,
}

impl ConfigPage {
    pub fn new(
        page: &'static Page,
        game: Entity<Game>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = page
            .sections
            .iter()
            .map(|section| Form::new(section.defs, &game, window, cx))
            .collect();
        cx.observe_in(&game, window, |this: &mut Self, game, window, cx| {
            for form in &this.forms {
                form.sync(&game, window, cx);
            }
            cx.notify();
        })
        .detach();
        let apps = match page.extra {
            Extra::Apps => {
                let game = game.read(cx);
                apps::apps(&game.install, &game.config)
                    .into_iter()
                    .map(|app| app.id)
                    .collect()
            }
            _ => Vec::new(),
        };
        Self {
            game,
            page,
            forms,
            apps,
            confirming: None,
        }
    }

    /// Words a search can find this page by.
    pub fn terms(&self) -> Vec<SharedString> {
        let mut terms: Vec<SharedString> = self
            .page
            .sections
            .iter()
            .map(|section| section.title.into())
            .collect();
        terms.extend(
            self.forms
                .iter()
                .flat_map(Form::labels)
                .map(SharedString::from),
        );
        match self.page.extra {
            Extra::Apps => {
                terms.push(PYTHON_APPS.label.into());
                terms.extend(self.apps.iter().map(|id| SharedString::from(id.clone())));
            }
            Extra::AssistPresets => {
                terms.extend(AssistPreset::ALL.map(|p| SharedString::from(p.label())));
            }
            Extra::Files => terms.push("Settings folder".into()),
            Extra::None => {}
        }
        terms
    }

    fn restore(&mut self, files: &'static [&'static str], cx: &mut Context<Self>) {
        self.confirming = None;
        self.game.update(cx, |game, cx| {
            let result = game.configure(cx, |config, _| config.restore_defaults(files));
            game.config_error = result.err().map(|e| e.to_string().into());
        });
    }

    fn restore_button(
        &self,
        id: &'static str,
        files: &'static [&'static str],
        title: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = div().flex().items_center().gap(px(8.));
        if self.confirming == Some(files) {
            row.child(muted("Put these back as the game ships them?"))
                .child(button(format!("{id}-keep"), "Keep").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.confirming = None;
                        cx.notify();
                    },
                )))
                .child(
                    button(format!("{id}-confirm"), "Restore")
                        .light()
                        .on_click(cx.listener(move |this, _, _, cx| this.restore(files, cx))),
                )
        } else {
            row.child(
                button(id, title).on_click(cx.listener(move |this, _, _, cx| {
                    this.confirming = Some(files);
                    cx.notify();
                })),
            )
        }
    }

    fn presets(&self, cx: &mut Context<Self>) -> Div {
        let current = self.game.read(cx).session().assists.preset();
        div()
            .flex()
            .gap(px(8.))
            .children(AssistPreset::ALL.into_iter().map(|preset| {
                choice_tile(
                    ("assist-preset", preset as usize),
                    preset.label(),
                    current == Some(preset),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.game.update(cx, |game, cx| {
                        let result = game.set_assists(cx, |a| *a = preset.assists());
                        game.config_error = result.err().map(|e| e.to_string().into());
                    });
                }))
                .test_support()
            }))
            .child(choice_tile(
                "assist-preset-custom",
                "Custom",
                current.is_none(),
            ))
    }

    fn apps_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let game = self.game.read(cx);
        let enabled = game
            .config
            .flag(settings::GAMEPLAY, "PYTHON", "ENABLE_PYTHON")
            .unwrap_or(true);
        let handle = self.game.clone();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(16.))
            .child(toggle_row(&PYTHON_APPS, enabled, move |on, _, cx| {
                write(
                    &PYTHON_APPS,
                    &handle,
                    if on { "1" } else { "0" }.to_owned(),
                    cx,
                )
            }));
        for (index, id) in self.apps.iter().enumerate() {
            let active = game
                .config
                .flag(settings::PYTHON, &id.to_uppercase(), "ACTIVE")
                .unwrap_or(false);
            let game = self.game.clone();
            let id = id.clone();
            body = body.child(
                switch_row(
                    ("app", index),
                    id.clone(),
                    None,
                    active,
                    move |on, _, cx| {
                        let id = id.clone();
                        game.update(cx, |game, cx| {
                            let result =
                                game.configure(cx, |config, _| apps::set_active(config, &id, on));
                            game.config_error = result.err().map(|e| e.to_string().into());
                        });
                    },
                )
                .when(!enabled, |row| row.opacity(0.5)),
            );
        }
        if self.apps.is_empty() {
            body = body.child(muted("No Python apps are installed."));
        }
        panel("Python apps").child(body).into_any_element()
    }

    fn files_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let dir = self.game.read(cx).config.dir().to_path_buf();
        let shown = dir.display().to_string();
        panel("Settings folder")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(16.))
                    .child(muted(
                        "The game reads its settings from here. Muretto writes each change as you make it.",
                    ))
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(8.))
                            .bg(rgb(color::BG))
                            .overflow_hidden()
                            .child(mono(shown, 12.).truncate().text_color(rgb(color::TEXT_MUTED))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(button("open-settings-folder", "Open folder").on_click(
                                move |_, _, cx| {
                                    if dir.is_dir() {
                                        cx.open_with_system(&dir);
                                    }
                                },
                            ))
                            .child(self.restore_button(
                                "restore-all",
                                &ALL_FILES,
                                "Restore all defaults",
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ConfigPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.page;
        let mut columns: [Vec<AnyElement>; 2] = [Vec::new(), Vec::new()];
        match page.extra {
            Extra::Apps => columns[0].push(self.apps_panel(cx)),
            Extra::Files => columns[1].push(self.files_panel(cx)),
            _ => {}
        }
        for (section, form) in page.sections.iter().zip(&self.forms) {
            columns[usize::from(section.right)].push(
                panel(section.title)
                    .child(form.body(&self.game, cx))
                    .into_any_element(),
            );
        }
        let [left, right] = columns;

        div()
            .id(SharedString::from(format!("page-{}", page.name)))
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
                            .items_start()
                            .justify_between()
                            .gap(GAP)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(display(page.name, 27.))
                                    .child(muted(page.blurb)),
                            )
                            .when(!page.files.is_empty(), |header| {
                                header.child(self.restore_button(
                                    "restore-page",
                                    page.files,
                                    "Restore defaults",
                                    cx,
                                ))
                            }),
                    )
                    .when(page.extra == Extra::AssistPresets, |page| {
                        page.child(self.presets(cx))
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(GAP)
                            .child(column(left))
                            .child(column(right)),
                    ),
            )
            .test_support()
    }
}

fn column(panels: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(GAP)
        .children(panels)
}
