//! Getting a server's car or track from a page Muretto cannot download
//! from itself (Google Drive, Mega, MediaFire). The page opens in the
//! browser; this dialog waits for the archive, or the folder it was
//! unpacked to, and installs whatever cars and tracks are in it.

use std::path::PathBuf;

use gpui_kit::{
    Context, Div, Entity, EventEmitter, ExternalPaths, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, PathPromptOptions, Render, SharedString, TestSupportExt as _,
    Window, div, prelude::*, px, relative, rgb,
};
use muretto_core::{
    Error, Library,
    packages::content::{Added, install_content},
};

use crate::{
    game::Game,
    job::Job,
    theme::{GAP, RADIUS, alpha, color},
    ui::{button, display, label, muted},
};

pub enum ContentEvent {
    Closed,
}

enum Step {
    Waiting,
    Installing(Job),
    Done(Added),
}

pub struct ContentDialog {
    game: Entity<Game>,
    url: String,
    host: String,
    /// What the server says the page has, by name.
    wanted: Vec<String>,
    step: Step,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<ContentEvent> for ContentDialog {}

impl ContentDialog {
    /// Opens the page in the browser and waits for what was downloaded.
    pub fn new(
        game: Entity<Game>,
        url: String,
        host: String,
        wanted: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.open_url(&url);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            game,
            url,
            host,
            wanted,
            step: Step::Waiting,
            error: None,
            focus,
        }
    }

    /// Installs the cars and tracks in the archive or folder the player
    /// handed over.
    pub fn accept(&mut self, source: PathBuf, cx: &mut Context<Self>) {
        if !matches!(self.step, Step::Waiting) {
            return;
        }
        self.error = None;
        let install = self.game.read(cx).install.clone();
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.step = Step::Installing(Job::start(
            format!("Installing {name}"),
            install,
            cx,
            move |install, progress| {
                let added = install_content(install, &source, progress)?;
                Ok((added, Library::scan(install)))
            },
            |this, result, cx| {
                this.step = match result {
                    Ok((added, library)) => {
                        this.game
                            .update(cx, |game, cx| game.set_library(library, cx));
                        Step::Done(added)
                    }
                    Err(error) => {
                        if !matches!(error, Error::Cancelled) {
                            this.error = Some(error.to_string().into());
                        }
                        Step::Waiting
                    }
                };
            },
        ));
        cx.notify();
    }

    fn choose(&mut self, folder: bool, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some("Install".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(source) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.accept(source, cx)).ok();
            }
        })
        .detach();
    }

    fn waiting(&self, cx: &mut Context<Self>) -> Div {
        let wanted = match self.wanted.as_slice() {
            [one] => one.clone(),
            [first, rest @ ..] => format!("{first} and {} more", rest.len()),
            [] => "the content".to_owned(),
        };
        let url = self.url.clone();
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
                        "The server links {wanted} to a page on {}, now open in your browser. Download the archive there and come back.",
                        self.host
                    )))
                    .child(
                        div().flex().child(
                            button("content-open-page", "Open the page again")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                    ),
            )
            .child(
                step("2", "Give it to Muretto").child(
                    div()
                        .id("content-drop-zone")
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
                                .child("Drop the archive or its folder here"),
                        )
                        .child(muted("Zip and 7z archives open as they are. Unpack RAR archives first."))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    button("content-choose-file", "Choose a file…")
                                        .on_click(cx.listener(|this, _, _, cx| this.choose(false, cx))),
                                )
                                .child(
                                    button("content-choose-folder", "Choose a folder…")
                                        .on_click(cx.listener(|this, _, _, cx| this.choose(true, cx))),
                                ),
                        ),
                ),
            )
    }

    fn done(&self, added: &Added, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(added_text(added)),
            )
            .when(!added.kept.is_empty(), |column| {
                column.child(muted(format!(
                    "Left as they were, already installed: {}.",
                    added.kept.join(", ")
                )))
            })
            .child(
                div().flex().justify_end().mt(px(12.)).child(
                    button("content-done", "Done")
                        .light()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ContentEvent::Closed))),
                ),
            )
    }
}

/// "Added 2 cars and 1 track." or why nothing was.
pub fn added_text(added: &Added) -> String {
    let count = |n: usize, one: &str, many: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    match (added.cars.len(), added.tracks.len()) {
        (0, 0) => "Nothing new: everything in it was already installed.".to_owned(),
        (cars, 0) => format!("Added {}.", count(cars, "car", "cars")),
        (0, tracks) => format!("Added {}.", count(tracks, "track", "tracks")),
        (cars, tracks) => format!(
            "Added {} and {}.",
            count(cars, "car", "cars"),
            count(tracks, "track", "tracks")
        ),
    }
}

impl Render for ContentDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = matches!(self.step, Step::Installing(_));
        let body = match &self.step {
            Step::Waiting => self.waiting(cx),
            Step::Installing(job) => job.strip(),
            Step::Done(added) => self.done(&added.clone(), cx),
        };
        div()
            .id("content-dialog")
            .track_focus(&self.focus)
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(100.))
            .bg(alpha(0x000000, 0.6))
            .occlude()
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, _, cx| {
                if let Some(source) = dropped.paths().first() {
                    this.accept(source.clone(), cx);
                }
            }))
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && !busy {
                    cx.emit(ContentEvent::Closed);
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
                            .child(display("Get the server's content", 16.))
                            .when(!busy && !matches!(self.step, Step::Done(_)), |bar| {
                                bar.child(button("content-close", "Close").on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(ContentEvent::Closed)),
                                ))
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
