//! Settings: a list of pages on the left, the chosen page on the right.

use std::{sync::Arc, time::Duration};

use gpui_kit::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, PathPromptOptions, Render,
    SharedString, TestSupportExt as _, Window, div, prelude::*, px, relative, rgb,
};
use muretto_core::{
    Error, Install,
    install::Runtime,
    packages::{
        Progress, Stage,
        csp::{self, Catalogue, Installed, Release, Standing, version_key},
    },
};

use crate::{
    game::Game,
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{button, display, mono, muted, panel, stat_strip, tracked},
};

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
        "Custom Shaders Patch",
        &[
            PATCH_PAGE,
            "General patch settings",
            "Lighting FX",
            "Weather FX",
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
const PATCH_PAGE: &str = "Versions and updates";

pub struct SettingsScreen {
    page: &'static str,
    patch: Entity<PatchPage>,
}

impl SettingsScreen {
    pub fn new(game: Entity<Game>, source: Arc<dyn csp::Source>, cx: &mut Context<Self>) -> Self {
        let patch = cx.new(|cx| PatchPage::new(game, source, cx));
        cx.observe(&patch, |_, _, cx| cx.notify()).detach();
        Self {
            page: PATCH_PAGE,
            patch,
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
            sidebar = sidebar.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px(PAGE)
                    .pt(px(16.))
                    .pb(px(8.))
                    // The longest heading breaks in two to leave room for the
                    // patch's state beside it.
                    .child(
                        div().flex().flex_col().children(
                            group
                                .split_once(" Patch")
                                .map_or(vec![group], |(first, _)| vec![first, "Patch"])
                                .into_iter()
                                .map(|line| display(line, 14.).text_color(rgb(color::TEXT_MUTED))),
                        ),
                    )
                    .when(group == "Custom Shaders Patch", |heading| {
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

        let page: AnyElement = if self.page == PATCH_PAGE {
            self.patch.clone().into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .size_full()
                .child(display(self.page, 27.))
                .child(muted("This part of Muretto has not been built yet."))
                .into_any_element()
        };
        div()
            .flex()
            .size_full()
            .child(sidebar)
            .child(div().flex_1().min_w_0().h_full().child(page))
    }
}

enum Offer {
    Loading,
    /// The author's current list.
    Live(Catalogue),
    /// The download page could not be read; these are the builds Muretto
    /// knew about when it was made.
    BuiltIn(Catalogue, SharedString),
}

struct Job {
    title: SharedString,
    progress: Arc<Progress>,
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
    fn new(game: Entity<Game>, source: Arc<dyn csp::Source>, cx: &mut Context<Self>) -> Self {
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
    fn standing(&self, cx: &gpui_kit::App) -> (&'static str, &'static str, u32) {
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

    /// Runs an installation in the background, redrawing while it reports
    /// progress and refreshing the game's state when it ends.
    fn run(
        &mut self,
        title: impl Into<SharedString>,
        work: impl FnOnce(&Install, &Progress) -> muretto_core::Result<Installed> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.job.is_some() {
            return;
        }
        let progress = Progress::new();
        let install = self.game.read(cx).install.clone();
        self.error = None;
        self.confirming_removal = false;
        self.job = Some(Job {
            title: title.into(),
            progress: progress.clone(),
        });
        cx.notify();

        let working = cx
            .background_executor()
            .spawn(async move { work(&install, &progress) });
        cx.spawn(async move |this, cx| {
            let result = working.await;
            this.update(cx, |this, cx| {
                this.job = None;
                match result {
                    Ok(_) | Err(Error::Cancelled) => {}
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                this.game.update(cx, |game, cx| game.refresh_patch(cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let running = this.update(cx, |this, cx| {
                    cx.notify();
                    this.job.is_some()
                });
                if !running.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
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
        self.game.update(cx, |game, cx| game.refresh_patch(cx));
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
            let progress = &job.progress;
            let megabytes = |bytes: u64| format!("{:.1}", bytes as f64 / 1e6);
            let detail = match (progress.stage(), progress.total()) {
                (Stage::Starting, _) => "Contacting the download site".to_owned(),
                (Stage::Downloading, Some(total)) => {
                    format!(
                        "Downloading · {} of {} MB",
                        megabytes(progress.done()),
                        megabytes(total)
                    )
                }
                (Stage::Downloading, None) => {
                    format!("Downloading · {} MB", megabytes(progress.done()))
                }
                (Stage::Unpacking, _) => "Unpacking".to_owned(),
                (Stage::Installing, _) => "Moving files into the game folder".to_owned(),
            };
            let cancellable = progress.stage() != Stage::Installing;
            let progress = progress.clone();
            return Some(
                strip()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(job.title.clone()),
                                    )
                                    .child(mono(detail, 12.5).text_color(rgb(color::TEXT_MUTED))),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(rgb(color::BORDER))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(progress.fraction().unwrap_or(0.)))
                                            .rounded_full()
                                            .bg(rgb(color::ACCENT)),
                                    ),
                            ),
                    )
                    .when(cancellable, |strip| {
                        strip.child(
                            button("patch-cancel", "Cancel")
                                .on_click(move |_, _, _| progress.cancel()),
                        )
                    }),
            );
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
