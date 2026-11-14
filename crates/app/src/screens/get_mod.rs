//! Getting a mod that Muretto cannot download itself. The mod's page opens
//! in the browser; this dialog waits in Muretto for the player to hand over
//! the archive they downloaded, by dropping it or choosing it.

use std::path::PathBuf;

use gpui_kit::{
    Context, Div, Entity, EventEmitter, ExternalPaths, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, PathPromptOptions, Render, SharedString, TestSupportExt as _,
    Window, div, prelude::*, px, relative, rgb,
};
use muretto_core::{
    Error,
    packages::recipes::{Installed, Recipe},
};

use crate::{
    game::Game,
    job::Job,
    theme::{GAP, RADIUS, alpha, color},
    ui::{button, display, label, muted},
};

pub enum GetEvent {
    Closed,
}

enum Step {
    /// Waiting for the archive.
    Waiting,
    Installing(Job),
    Installed(Installed),
}

pub struct GetDialog {
    game: Entity<Game>,
    recipe: &'static Recipe,
    step: Step,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<GetEvent> for GetDialog {}

impl GetDialog {
    /// Opens the mod's page in the browser and waits for its archive.
    pub fn new(
        game: Entity<Game>,
        recipe: &'static Recipe,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.open_url(recipe.page);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            game,
            recipe,
            step: Step::Waiting,
            error: None,
            focus,
        }
    }

    /// Takes the archive the player dropped or chose, and installs it.
    pub fn accept(&mut self, archive: PathBuf, cx: &mut Context<Self>) {
        if !matches!(self.step, Step::Waiting) {
            return;
        }
        self.error = None;
        let recipe = self.recipe;
        let install = self.game.read(cx).install.clone();
        let name = archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.step = Step::Installing(Job::start(
            format!("Installing {name}"),
            install,
            cx,
            move |install, progress| recipe.install_from_file(install, &archive, progress),
            |this, result, cx| {
                this.step = match result {
                    Ok(installed) => Step::Installed(installed),
                    Err(error) => {
                        if !matches!(error, Error::Cancelled) {
                            this.error = Some(error.to_string().into());
                        }
                        Step::Waiting
                    }
                };
                this.game.update(cx, |game, cx| game.refresh_installed(cx));
            },
        ));
        cx.notify();
    }

    fn choose_file(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Install".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(archive) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.accept(archive, cx)).ok();
            }
        })
        .detach();
    }

    fn use_now(&mut self, cx: &mut Context<Self>) {
        let install = self.game.read(cx).install.clone();
        match self.recipe.use_now(&install) {
            Ok(()) => {
                self.game.update(cx, |game, cx| game.refresh_installed(cx));
                cx.emit(GetEvent::Closed);
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn waiting(&self, cx: &mut Context<Self>) -> Div {
        let recipe = self.recipe;
        let step = |number: &str, title: &str| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(&format!("{number} · {title}")))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(
                step("1", "Download it")
                    .child(div().line_height(relative(1.5)).child(format!(
                        "{} comes from its author, {}, on {}. That page is now open in your browser. Download the zip there and come back.",
                        recipe.name, recipe.author, recipe.page_name
                    )))
                    .child(
                        div().flex().child(
                            button("get-open-page", "Open the page again")
                                .on_click(move |_, _, cx| cx.open_url(recipe.page)),
                        ),
                    ),
            )
            .child(
                step("2", "Give it to Muretto").child(
                    div()
                        .id("get-drop-zone")
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .h(px(132.))
                        .bg(rgb(color::BG))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(color::BORDER_STRONG))
                        .rounded(RADIUS)
                        .drag_over::<ExternalPaths>(|style, _, _, _| {
                            style
                                .bg(rgb(color::ACCENT_WASH))
                                .border_color(rgb(color::ACCENT))
                        })
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Drop the zip here"),
                        )
                        .child(
                            button("get-choose-file", "Choose a file…")
                                .on_click(cx.listener(|this, _, _, cx| this.choose_file(cx))),
                        ),
                ),
            )
    }

    fn installed(&self, installed: &Installed, cx: &mut Context<Self>) -> Div {
        let recipe = self.recipe;
        let install = &self.game.read(cx).install;
        let in_use = recipe.in_use(install);
        let headline = match &installed.version {
            Some(version) => format!("{} {version} is installed.", recipe.name),
            None => format!("{} is installed.", recipe.name),
        };
        let offers_use = recipe.weather.is_some() && !in_use;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(headline),
            )
            .child(muted(if in_use {
                "It is the weather style in use."
            } else if offers_use {
                "It is not in use yet. You can switch to it now, or later under Weather FX."
            } else {
                "It is ready the next time you drive."
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .mt(px(12.))
                    .child(
                        button("get-done", if offers_use { "Not now" } else { "Done" })
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(GetEvent::Closed))),
                    )
                    .when(offers_use, |row| {
                        row.child(
                            button("get-use", format!("Use {} now", recipe.name))
                                .light()
                                .on_click(cx.listener(|this, _, _, cx| this.use_now(cx))),
                        )
                    }),
            )
    }
}

impl Render for GetDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = matches!(self.step, Step::Installing(_));
        let body = match &self.step {
            Step::Waiting => self.waiting(cx),
            Step::Installing(job) => job.strip(),
            Step::Installed(installed) => self.installed(&installed.clone(), cx),
        };

        div()
            .id("get-dialog")
            .track_focus(&self.focus)
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(120.))
            .bg(alpha(0x000000, 0.6))
            .occlude()
            // A drop anywhere on the window counts, not only on the marked zone.
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, _, cx| {
                if let Some(archive) = dropped.paths().first() {
                    this.accept(archive.clone(), cx);
                }
            }))
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && !busy {
                    cx.emit(GetEvent::Closed);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(520.))
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::BORDER_STRONG))
                    .rounded(RADIUS)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(44.))
                            .pl(px(16.))
                            .pr(px(6.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .child(display(&format!("Get {}", self.recipe.name), 16.))
                            .when(!busy && !matches!(self.step, Step::Installed(_)), |bar| {
                                bar.child(
                                    button("get-close", "Close").on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(GetEvent::Closed)),
                                    ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(GAP)
                            .p(px(16.))
                            .text_size(px(13.))
                            .child(body)
                            .when_some(self.error.clone(), |body, error| {
                                body.child(
                                    div()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_1()
                                        .border_color(rgb(color::ACCENT_ACTIVE))
                                        .rounded(RADIUS)
                                        .line_height(relative(1.5))
                                        .text_color(rgb(color::ACCENT_TEXT))
                                        .child(error),
                                )
                            }),
                    ),
            )
            .test_support()
    }
}
