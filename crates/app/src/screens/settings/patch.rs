//! Settings › Custom Shaders Patch › Versions and updates.

use std::sync::Arc;

use gpui_kit::{
    Context, Div, Entity, FontWeight, IntoElement, PathPromptOptions, Render, SharedString, Window,
    div, prelude::*, px, relative, rgb,
};
use muretto_core::{
    Error, Install,
    install::Runtime,
    packages::{
        Progress,
        csp::{self, Catalogue, Installed, Release, Standing, version_key},
    },
};

use crate::{
    game::Game,
    job::Job,
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{button, display, mono, muted, panel, stat_strip, tracked},
};

enum Offer {
    Loading,
    /// The author's current list.
    Live(Catalogue),
    /// The download page could not be read; these are the builds Muretto
    /// knew about when it was made.
    BuiltIn(Catalogue, SharedString),
}

/// Settings › Custom Shaders Patch › Versions and updates.
pub struct PatchPage {
    game: Entity<Game>,
    source: Arc<dyn csp::Source>,
    offer: Offer,
    job: Option<Job>,
    error: Option<SharedString>,
    confirming_removal: bool,
    all_versions: bool,
}

/// Builds listed before the rest fold away.
const VERSIONS_SHOWN: usize = 6;

impl PatchPage {
    pub fn new(game: Entity<Game>, source: Arc<dyn csp::Source>, cx: &mut Context<Self>) -> Self {
        cx.observe(&game, |_, _, cx| cx.notify()).detach();
        let listing = cx.background_executor().spawn({
            let source = source.clone();
            async move { source.catalogue() }
        });
        cx.spawn(async move |this, cx| {
            let offer = match listing.await {
                Ok(catalogue) => Offer::Live(catalogue),
                Err(error) => Offer::BuiltIn(Catalogue::built_in(), error.to_string().into()),
            };
            this.update(cx, |this, cx| {
                this.offer = offer;
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            game,
            source,
            offer: Offer::Loading,
            job: None,
            error: None,
            confirming_removal: false,
            all_versions: false,
        }
    }

    fn catalogue(&self) -> Option<&Catalogue> {
        match &self.offer {
            Offer::Loading => None,
            Offer::Live(catalogue) | Offer::BuiltIn(catalogue, _) => Some(catalogue),
        }
    }

    /// Where the installed patch stands: in full, in a word for the
    /// sidebar, and the colour to say it in.
    pub fn standing(&self, cx: &gpui_kit::App) -> (&'static str, &'static str, u32) {
        let Some(patch) = &self.game.read(cx).patch else {
            return ("Not installed", "Missing", color::WARN);
        };
        let recommended = self.catalogue().and_then(Catalogue::recommended);
        match (&patch.version, recommended) {
            (Some(version), Some(recommended))
                if version_key(version) < version_key(&recommended.version) =>
            {
                ("Update available", "Update", color::ACCENT_TEXT)
            }
            (Some(version), Some(recommended))
                if version_key(version) > version_key(&recommended.version) =>
            {
                ("Ahead of recommended", "Newer", color::GOOD)
            }
            (Some(_), Some(_)) => ("Up to date", "Current", color::GOOD),
            _ => ("Installed", "Installed", color::GOOD),
        }
    }

    /// Runs an installation in the background and refreshes the game's
    /// state when it ends.
    fn run(
        &mut self,
        title: impl Into<SharedString>,
        work: impl FnOnce(&Install, &Progress) -> muretto_core::Result<Installed> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        self.confirming_removal = false;
        let install = self.game.read(cx).install.clone();
        self.job = Some(Job::start(title, install, cx, work, |this, result, cx| {
            this.job = None;
            match result {
                Ok(_) | Err(Error::Cancelled) => {}
                Err(error) => this.error = Some(error.to_string().into()),
            }
            this.game.update(cx, |game, cx| game.refresh_installed(cx));
        }));
        cx.notify();
    }

    fn install(&mut self, release: Release, cx: &mut Context<Self>) {
        let source = self.source.clone();
        self.run(
            format!("Installing {}", release.version),
            move |install, progress| csp::install(install, source.as_ref(), &release, progress),
            cx,
        );
    }

    fn install_from_file(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Install".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(archive) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| {
                let name = archive
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                this.run(
                    format!("Installing {name}"),
                    move |install, progress| csp::install_from_file(install, &archive, progress),
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        self.confirming_removal = false;
        let install = self.game.read(cx).install.clone();
        self.error = csp::remove(&install).err().map(|e| e.to_string().into());
        self.game.update(cx, |game, cx| game.refresh_installed(cx));
        cx.notify();
    }

    fn actions(&self, patch: Option<&Installed>, cx: &mut Context<Self>) -> Div {
        let row = div().flex().flex_none().items_center().gap(px(8.));
        if self.job.is_some() {
            return row;
        }
        if self.confirming_removal {
            let text = match patch {
                Some(Installed { managed: false, .. }) => {
                    "This copy was not installed by Muretto. Only its loader will be removed; the extension folder stays."
                }
                _ => "Remove the patch and everything it installed?",
            };
            return row
                .child(
                    div()
                        .max_w(px(380.))
                        .text_size(px(13.))
                        .text_color(rgb(color::TEXT_MUTED))
                        .child(text),
                )
                .child(
                    button("patch-keep", "Keep").on_click(cx.listener(|this, _, _, cx| {
                        this.confirming_removal = false;
                        cx.notify();
                    })),
                )
                .child(
                    button("patch-remove-confirm", "Remove")
                        .light()
                        .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
                );
        }

        let recommended = self.catalogue().and_then(Catalogue::recommended).cloned();
        let installed = patch.and_then(|p| p.version.as_deref()).map(version_key);
        let primary = recommended.filter(|release| match &installed {
            Some(installed) => *installed < version_key(&release.version),
            None => true,
        });
        row.child(
            button("patch-from-file", "Install from file…")
                .on_click(cx.listener(|this, _, _, cx| this.install_from_file(cx))),
        )
        .when(patch.is_some(), |row| {
            row.child(
                button("patch-remove", "Remove").on_click(cx.listener(|this, _, _, cx| {
                    this.confirming_removal = true;
                    cx.notify();
                })),
            )
        })
        .when_some(primary, |row, release| {
            let verb = if patch.is_some() {
                "Update to"
            } else {
                "Install"
            };
            row.child(
                button("patch-install", format!("{verb} {}", release.version))
                    .light()
                    .on_click(cx.listener(move |this, _, _, cx| this.install(release.clone(), cx))),
            )
        })
    }

    fn banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let strip = || {
            div()
                .flex()
                .items_center()
                .gap(GAP)
                .min_h(px(52.))
                .px(px(16.))
                .py(px(10.))
                .bg(rgb(color::SURFACE))
                .border_1()
                .border_color(rgb(color::BORDER))
                .rounded(RADIUS)
        };
        if let Some(job) = &self.job {
            return Some(strip().child(job.strip().flex_1()));
        }
        if let Some(error) = &self.error {
            return Some(
                strip()
                    .border_color(rgb(color::ACCENT_ACTIVE))
                    .child(
                        tracked("FAILED", px(10.), 0.12)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(color::ACCENT_TEXT)),
                    )
                    .child(div().flex_1().min_w_0().child(error.clone()))
                    .child(button("patch-dismiss", "Dismiss").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        },
                    ))),
            );
        }
        if let Offer::BuiltIn(_, reason) = &self.offer {
            return Some(
                strip()
                    .child(
                        tracked("OFFLINE", px(10.), 0.12)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(color::WARN)),
                    )
                    .child(div().flex_1().min_w_0().text_color(rgb(color::TEXT_MUTED)).child(format!(
                        "The download page could not be read, so this list may be out of date. {reason}"
                    ))),
            );
        }
        None
    }

    fn versions(&self, patch: Option<&Installed>, cx: &mut Context<Self>) -> impl IntoElement {
        let installed = patch.and_then(|p| p.version.clone());
        let releases = self
            .catalogue()
            .map(|c| c.releases.as_slice())
            .unwrap_or_default();
        let shown = if self.all_versions {
            releases.len()
        } else {
            VERSIONS_SHOWN
        };
        let hidden = releases.len().saturating_sub(shown);
        let idle = self.job.is_none();

        let rows = releases
            .iter()
            .take(shown)
            .enumerate()
            .map(|(index, release)| {
                let current = installed.as_deref() == Some(release.version.as_str());
                let tone = match release.standing {
                    Standing::Recommended => color::GOOD,
                    Standing::Buggy => color::WARN,
                    _ => color::TEXT_MUTED,
                };
                let release_to_install = release.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .h(px(46.))
                    .px(px(16.))
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgb(color::BORDER))
                    })
                    .when(current, |row| row.bg(rgb(color::ACCENT_WASH)))
                    .child(mono(release.version.clone(), 14.).w(px(72.)))
                    .child(
                        mono(
                            release.size_bytes.map_or(String::new(), |bytes| {
                                format!("{:.1} MB", bytes as f64 / 1e6)
                            }),
                            12.5,
                        )
                        .w(px(88.))
                        .text_color(rgb(color::TEXT_MUTED)),
                    )
                    .child(
                        tracked(&release.standing.label().to_uppercase(), px(10.), 0.12)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(tone)),
                    )
                    .child(div().flex_1())
                    .map(|row| {
                        if current {
                            row.child(
                                tracked("INSTALLED", px(10.), 0.12)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(color::TEXT)),
                            )
                        } else if idle {
                            row.child(button(("patch-version", index), "Install").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.install(release_to_install.clone(), cx)
                                }),
                            ))
                        } else {
                            row
                        }
                    })
            });

        panel("Versions")
            .flex_1()
            .children(rows)
            .when(matches!(self.offer, Offer::Loading), |list| {
                list.child(div().p(px(16.)).child(muted("Reading the download page…")))
            })
            .when(hidden > 0, |list| {
                list.child(
                    div()
                        .id("patch-all-versions")
                        .flex()
                        .items_center()
                        .h(px(40.))
                        .px(px(16.))
                        .border_t_1()
                        .border_color(rgb(color::BORDER))
                        .text_color(rgb(color::TEXT_MUTED))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(color::RAISED)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.all_versions = true;
                            cx.notify();
                        }))
                        .child(format!("Show {hidden} older versions")),
                )
            })
    }

    fn notes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let proton = matches!(self.game.read(cx).install.runtime, Runtime::Proton { .. });
        let note = |title: &str, text: &str| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    tracked(&title.to_uppercase(), px(11.), 0.1)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(color::TEXT_MUTED)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(relative(1.5))
                        .text_color(rgb(color::TEXT))
                        .child(SharedString::from(text.to_owned())),
                )
        };
        panel("Good to know").w(px(420.)).flex_none().child(
            div()
                .flex()
                .flex_col()
                .gap(px(18.))
                .p(px(16.))
                .when(proton, |notes| {
                    notes.child(note(
                        "Running under Proton",
                        "The patch loads through a DLL override that Muretto adds whenever it starts the game. Started from Steam directly, the game runs without the patch.",
                    ))
                })
                .child(note(
                    "Preview builds",
                    "Builds newer than the public ones are given to the author's supporters as a zip. Download yours, then use Install from file.",
                ))
                .child(note(
                    "Your settings",
                    "Patch settings live in your Documents folder and are kept when you update or remove the patch.",
                )),
        )
    }
}

impl Render for PatchPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let patch = self.game.read(cx).patch.clone();
        let (standing, _, tone) = self.standing(cx);
        let or_dash =
            |text: Option<String>| -> SharedString { text.map_or("—".into(), Into::into) };
        let catalogue = self.catalogue();
        let recommended = catalogue
            .and_then(Catalogue::recommended)
            .map(|r| r.version.clone());
        let newest = catalogue
            .and_then(Catalogue::latest)
            .map(|r| r.version.clone());
        let installed = patch.as_ref().map(|patch| {
            patch
                .version
                .clone()
                .unwrap_or_else(|| "Unknown".to_owned())
        });

        div().id("patch-page").size_full().overflow_y_scroll().child(
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
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(12.))
                                        .child(display("Custom Shaders Patch", 27.))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .h(px(20.))
                                                .px(px(6.))
                                                .border_1()
                                                .border_color(rgb(tone))
                                                .rounded(RADIUS)
                                                .child(
                                                    tracked(&standing.to_uppercase(), px(10.), 0.12)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(rgb(tone)),
                                                ),
                                        ),
                                )
                                .child(muted(
                                    "Lighting, weather, rain and physics extensions that most mods rely on. By Ilja Jusupov.",
                                )),
                        )
                        .child(self.actions(patch.as_ref(), cx)),
                )
                .children(self.banner(cx))
                .child(stat_strip([
                    ("Installed", or_dash(installed)),
                    ("Build", or_dash(patch.as_ref().and_then(|p| p.build.clone()))),
                    ("Recommended", or_dash(recommended)),
                    ("Newest", or_dash(newest)),
                ]))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(GAP)
                        .child(self.versions(patch.as_ref(), cx))
                        .child(self.notes(cx)),
                ),
        )
    }
}
