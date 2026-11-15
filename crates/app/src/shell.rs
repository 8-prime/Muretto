//! The window frame: section navigation on top, the active screen in the
//! middle, and the next session with its launch button along the bottom.

use gpui_kit::{
    AnyElement, App, Context, Decorations, Div, Entity, Font, FontFallbacks, FontFeatures,
    FontStyle, FontWeight, Global, IntoElement, MouseButton, Render, SharedString, Stateful,
    Subscription, TestSupportExt as _, Window, WindowControlArea, div, prelude::*, px, rgb, svg,
};
use std::{path::PathBuf, sync::Arc};

use muretto_core::{
    Install, Library,
    group::{self, Groups, WebSocketRelay},
    online::{self, saved::Saved},
    packages::csp,
    session::format_time,
};

use crate::{
    APP_NAME,
    game::{Game, Launch},
    screens::{
        drive::{DriveEvent, DriveScreen},
        friends::GroupSetup,
        get_mod::{GetDialog, GetEvent},
        online::{OnlineScreen, Tab},
        picker::{Picker, PickerEvent, PickerItem},
        settings::{SettingsEvent, SettingsScreen},
    },
    theme::{PAGE, color, font},
    ui::{button, display, label, launch_button, muted, slant, tracked},
};

const DRIVE: usize = 0;
const CONTENT: usize = 1;
const ONLINE: usize = 2;
const SETTINGS: usize = 4;

/// Sections and the tabs within each.
const SECTIONS: [(&str, &[&str]); 5] = [
    (
        "Drive",
        &["Quick drive", "Career", "Championships", "Special events"],
    ),
    (
        "Content",
        &[
            "Cars",
            "Tracks",
            "Showrooms",
            "Weather",
            "Apps",
            "PP filters",
            "Fonts",
        ],
    ),
    // In the order of `online::Tab::ALL`.
    (
        "Online",
        &["All servers", "Favourites", "Recent", "LAN", "Friends"],
    ),
    (
        "Results",
        &[
            "Sessions",
            "Lap times",
            "Replays",
            "Screenshots",
            "Statistics",
        ],
    ),
    ("Settings", &[]),
];

enum Stage {
    Loading,
    Unavailable(SharedString),
    Ready {
        game: Entity<Game>,
        drive: Entity<DriveScreen>,
        online: Entity<OnlineScreen>,
        settings: Entity<SettingsScreen>,
    },
}

/// Where the screens get what they fetch from the internet, and where
/// Muretto keeps what the player chose.
#[derive(Clone)]
pub struct Sources {
    pub patches: Arc<dyn csp::Source>,
    pub servers: Arc<dyn online::Source>,
    /// Favourite and recent servers; kept nowhere when `None`.
    pub saved_servers: Option<PathBuf>,
    /// The groups the player is in; kept nowhere when `None`.
    pub saved_groups: Option<PathBuf>,
    /// The group server; `None` reaches the one the settings name.
    pub group_relay: Option<Arc<dyn group::Relay>>,
}

impl Sources {
    pub fn official() -> Self {
        Self {
            patches: Arc::new(csp::Official),
            servers: Arc::new(online::Official),
            saved_servers: Saved::default_path(),
            saved_groups: Groups::default_path(),
            group_relay: None,
        }
    }
}

/// Faces to draw what the bundled fonts lack: emoji and the scripts server
/// names come in. Whichever of them the system has are used, in order.
const FALLBACK_FONTS: &[&str] = &[
    "Noto Color Emoji",
    "Segoe UI Emoji",
    "Apple Color Emoji",
    "Twemoji",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Noto Sans CJK KR",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "PingFang SC",
    "Hiragino Sans",
    "Noto Sans",
    "Noto Sans Arabic",
    "Noto Sans Thai",
    "Segoe UI",
    "Segoe UI Symbol",
    "DejaVu Sans",
    "Arial Unicode MS",
];

#[derive(Clone, Copy)]
enum Choosing {
    Car,
    Track,
}

pub struct Shell {
    stage: Stage,
    section: usize,
    /// The open tab of each section.
    tabs: [usize; SECTIONS.len()],
    picker: Option<(Entity<Picker>, Subscription)>,
    getting: Option<(Entity<GetDialog>, Subscription)>,
    /// The pointer went down on the title bar; moving it now moves the
    /// window.
    moving: bool,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    /// Opens on a loading screen and reads the game's content in the
    /// background.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async {
                    Install::discover().map(|install| {
                        let library = Library::scan(&install);
                        (install, library)
                    })
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                match found {
                    Ok((install, library)) => {
                        this.show(install, library, Sources::official(), window, cx)
                    }
                    Err(error) => this.stage = Stage::Unavailable(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self::loading()
    }

    /// Opens directly on content that has already been read, fetching from
    /// `sources`.
    pub fn with_content(
        install: Install,
        library: Library,
        sources: Sources,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut shell = Self::loading();
        shell.show(install, library, sources, window, cx);
        shell
    }

    /// The Online screen, once content has been read.
    pub fn online(&self) -> Option<&Entity<OnlineScreen>> {
        match &self.stage {
            Stage::Ready { online, .. } => Some(online),
            _ => None,
        }
    }

    /// The game and session, once content has been read.
    pub fn game(&self) -> Option<&Entity<Game>> {
        match &self.stage {
            Stage::Ready { game, .. } => Some(game),
            _ => None,
        }
    }

    /// Whether a car or track list is open over the window.
    /// The dialog taking a mod's archive from the player, while it is open.
    pub fn getting(&self) -> Option<&Entity<GetDialog>> {
        self.getting.as_ref().map(|(dialog, _)| dialog)
    }

    pub fn is_choosing(&self) -> bool {
        self.picker.is_some()
    }

    /// Whether one of the Online section's server lists is showing.
    fn on_servers(&self) -> bool {
        self.section == ONLINE
    }

    fn loading() -> Self {
        Self {
            stage: Stage::Loading,
            section: DRIVE,
            tabs: [0; SECTIONS.len()],
            picker: None,
            getting: None,
            moving: false,
            _subscriptions: Vec::new(),
        }
    }

    fn show(
        &mut self,
        install: Install,
        library: Library,
        sources: Sources,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(game) = Game::new(install, library) else {
            self.stage = Stage::Unavailable(
                "The game folder has no car and track that can be driven.".into(),
            );
            return;
        };
        let game = cx.new(|_| game);
        let drive = cx.new(|cx| DriveScreen::new(game.clone(), window, cx));
        let settings = cx.new(|cx| SettingsScreen::new(game.clone(), sources.patches, window, cx));
        let saved = Saved::load(sources.saved_servers);
        let kept = Groups::load(sources.saved_groups);
        let relay = sources.group_relay.or_else(|| {
            kept.server_url()
                .map(|url| Arc::new(WebSocketRelay::new(url)) as Arc<dyn group::Relay>)
        });
        let groups = GroupSetup { kept, relay };
        let online = cx
            .new(|cx| OnlineScreen::new(game.clone(), sources.servers, saved, groups, window, cx));
        self._subscriptions = vec![
            cx.observe(&game, |_, _, cx| cx.notify()),
            // The bottom bar shows the chosen server.
            cx.observe(&online, |_, _, cx| cx.notify()),
            cx.subscribe_in(&drive, window, |this, _, event: &DriveEvent, window, cx| {
                let choosing = match event {
                    DriveEvent::ChooseCar => Choosing::Car,
                    DriveEvent::ChooseTrack => Choosing::Track,
                };
                this.choose(choosing, window, cx);
            }),
            cx.subscribe_in(&settings, window, {
                let game = game.clone();
                move |this, _, event: &SettingsEvent, window, cx| {
                    let SettingsEvent::Get(recipe) = *event;
                    let dialog = cx.new(|cx| GetDialog::new(game.clone(), recipe, window, cx));
                    let closed = cx.subscribe(&dialog, |this, _, _: &GetEvent, cx| {
                        this.getting = None;
                        cx.notify();
                    });
                    this.getting = Some((dialog, closed));
                    cx.notify();
                }
            }),
        ];
        self.stage = Stage::Ready {
            game,
            drive,
            online,
            settings,
        };
    }

    fn choose(&mut self, choosing: Choosing, window: &mut Window, cx: &mut Context<Self>) {
        let Stage::Ready { game, .. } = &self.stage else {
            return;
        };
        let state = game.read(cx);
        let library = &state.library;
        let (title, items, current) = match choosing {
            Choosing::Car => (
                "Choose a car",
                library
                    .cars
                    .iter()
                    .filter(|car| car.owned)
                    .map(|car| {
                        let detail = [car.brand.clone(), car.class_label()]
                            .into_iter()
                            .chain(car.year.map(|y| y.to_string()))
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");
                        PickerItem::new(&car.id, &car.name, detail, car.origin.label())
                    })
                    .collect::<Vec<_>>(),
                state.session().car_id.clone(),
            ),
            Choosing::Track => (
                "Choose a track",
                library
                    .tracks
                    .iter()
                    .filter(|track| track.owned)
                    .map(|track| {
                        let layout = &track.layouts[0];
                        let detail = layout
                            .country
                            .clone()
                            .into_iter()
                            .chain(match track.layouts.len() {
                                1 => layout.length_m.map(|m| format!("{:.3} km", m / 1000.)),
                                n => Some(format!("{n} layouts")),
                            })
                            .collect::<Vec<_>>()
                            .join(" · ");
                        PickerItem::new(&track.id, &track.name, detail, track.origin.label())
                    })
                    .collect::<Vec<_>>(),
                state.session().track_id.clone(),
            ),
        };

        let game = game.clone();
        let picker = cx.new(|cx| Picker::new(title, items, &current, window, cx));
        let subscription = cx.subscribe(&picker, move |this, _, event: &PickerEvent, cx| {
            if let PickerEvent::Picked(id) = event {
                game.update(cx, |game, cx| match choosing {
                    Choosing::Car => game.set_car(id, cx),
                    Choosing::Track => game.set_track(id, cx),
                });
            }
            this.picker = None;
            cx.notify();
        });
        self.picker = Some((picker, subscription));
        cx.notify();
    }

    /// The section navigation, which is also the window's title bar: the
    /// logo and the empty stretch move the window, and on Linux and Windows
    /// it ends in the window's controls.
    fn top_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, healthy): (SharedString, bool) = match &self.stage {
            Stage::Loading => ("Reading content…".into(), false),
            Stage::Unavailable(_) => ("Game not found".into(), false),
            Stage::Ready { game, .. } => match &game.read(cx).patch {
                Some(patch) => match &patch.version {
                    Some(version) => (format!("Game ready · CSP {version}").into(), true),
                    None => ("Game ready · CSP installed".into(), true),
                },
                None => ("Game ready · CSP not installed".into(), false),
            },
        };

        let controls = draws_controls(window, cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(56.))
            // Clear of the traffic lights.
            .pl(if cfg!(target_os = "macos") {
                px(84.)
            } else {
                PAGE
            })
            .when(!controls, |bar| bar.pr(PAGE))
            .bg(rgb(color::BG))
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .child(
                self.drag_area("title-logo", cx)
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mr(px(28.))
                    .child(
                        div()
                            .relative()
                            .w(px(27.))
                            .h(px(14.))
                            .child(slant(rgb(color::ACCENT), px(5.))),
                    )
                    .child(display(APP_NAME, 24.)),
            )
            .children(SECTIONS.iter().enumerate().map(|(index, (name, _))| {
                let active = index == self.section;
                div()
                    .id(("section", index))
                    .relative()
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(18.))
                    .cursor_pointer()
                    .border_x_1()
                    .border_color(rgb(if active { color::BORDER } else { color::BG }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = index;
                        if index == ONLINE
                            && let Some(online) = this.online()
                        {
                            online.update(cx, |online, cx| online.visit(cx));
                        }
                        cx.notify();
                    }))
                    .child(display(name, 19.).text_color(rgb(if active {
                        color::TEXT
                    } else {
                        color::TEXT_MUTED
                    })))
                    .when(active, |tab| {
                        tab.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(px(3.))
                                .bg(rgb(color::ACCENT)),
                        )
                    })
                    .test_support()
            }))
            .child(self.drag_area("title-drag", cx).flex_1().h_full())
            .child(
                // The status doubles as the way to the patch's page.
                div()
                    .id("status")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Stage::Ready { settings, .. } = &this.stage {
                            settings.update(cx, |settings, cx| settings.show_patch(cx));
                            this.section = SETTINGS;
                            cx.notify();
                        }
                    }))
                    .child(div().size(px(8.)).rounded_full().bg(rgb(if healthy {
                        color::GOOD
                    } else {
                        color::WARN
                    })))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(color::TEXT_MUTED))
                            .child(status),
                    ),
            )
            .when(controls, |bar| bar.child(window_controls(window, cx)))
    }

    /// Part of the title bar that moves the window when dragged, maximises
    /// it when double-clicked and opens the window menu on a right click.
    fn drag_area(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, _| {
                    if event.click_count >= 2 {
                        this.moving = false;
                        if cfg!(target_os = "macos") {
                            window.titlebar_double_click();
                        } else {
                            window.zoom_window();
                        }
                    } else {
                        this.moving = true;
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.moving = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                // Linux and macOS leave moving to the app; Windows moves
                // the window itself over a drag area.
                if this.moving && !cfg!(target_os = "windows") {
                    this.moving = false;
                    window.start_window_move();
                }
            }))
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                if is_client_decorated(window) {
                    window.show_window_menu(event.position);
                }
            })
    }

    fn tab_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let tabs = SECTIONS[self.section].1;
        if tabs.is_empty() {
            return None;
        }
        let (library, online) = match &self.stage {
            Stage::Ready { game, online, .. } => {
                (Some(game.read(cx).library.clone()), Some(online.read(cx)))
            }
            _ => (None, None),
        };
        let section = self.section;
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(px(45.))
                .px(PAGE)
                .bg(rgb(color::SURFACE))
                .border_b_1()
                .border_color(rgb(color::BORDER))
                .children(tabs.iter().enumerate().map(|(index, name)| {
                    let active = index == self.tabs[section];
                    // Tabs over installed content show how much there is.
                    let count = match section {
                        CONTENT => library.as_ref().and_then(|library| match *name {
                            "Cars" => Some(library.cars.len()),
                            "Tracks" => Some(library.tracks.len()),
                            "Weather" => Some(library.weather.len()),
                            _ => None,
                        }),
                        // So are the servers in each list.
                        ONLINE => online.and_then(|online| online.count(Tab::ALL[index])),
                        _ => None,
                    };
                    let text = match count {
                        Some(count) => format!("{name} · {count}"),
                        None => (*name).to_owned(),
                    };
                    div()
                        .id(("tab", index))
                        .relative()
                        .flex()
                        .items_center()
                        .h_full()
                        .px(px(12.))
                        .mr(px(6.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tabs[section] = index;
                            if section == ONLINE
                                && let Some(online) = this.online()
                            {
                                online.update(cx, |online, cx| online.set_tab(Tab::ALL[index], cx));
                            }
                            cx.notify();
                        }))
                        .child(
                            tracked(&text.to_uppercase(), px(12.5), 0.09)
                                .font_family(font::BODY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(if active {
                                    color::TEXT
                                } else {
                                    color::TEXT_MUTED
                                })),
                        )
                        .when(active, |tab| {
                            tab.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(px(2.))
                                    .bg(rgb(color::ACCENT_TEXT)),
                            )
                        })
                        .test_support()
                })),
        )
    }

    fn content(&self) -> AnyElement {
        match &self.stage {
            Stage::Loading => notice(
                "Reading content",
                "Looking through the game's cars and tracks.",
            ),
            Stage::Unavailable(reason) => notice(
                "Assetto Corsa not found",
                &format!(
                    "{reason}. Install the game through Steam, or point MURETTO_AC_ROOT at its folder and start again."
                ),
            ),
            Stage::Ready {
                drive,
                online,
                settings,
                ..
            } => {
                let (section, tabs) = SECTIONS[self.section];
                let tab = tabs.get(self.tabs[self.section]).copied();
                if self.section == DRIVE && self.tabs[DRIVE] == 0 {
                    drive.clone().into_any_element()
                } else if self.on_servers() {
                    online.clone().into_any_element()
                } else if self.section == SETTINGS {
                    settings.clone().into_any_element()
                } else {
                    notice(
                        tab.unwrap_or(section),
                        "This part of Muretto has not been built yet.",
                    )
                }
            }
        }
    }

    fn session_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(28.))
            .h(px(64.))
            .px(PAGE)
            .bg(rgb(color::SURFACE))
            .border_t_1()
            .border_color(rgb(color::BORDER))
            .child(display("Next session", 17.).text_color(rgb(color::ACCENT_TEXT)));

        let Stage::Ready { game, online, .. } = &self.stage else {
            return bar
                .child(div().flex_1())
                .child(launch_button("drive", "Drive").enabled(false));
        };
        let item = |name: &str, value: String| {
            div()
                .flex()
                .flex_col()
                .flex_shrink(1.)
                .min_w_0()
                .gap(px(3.))
                .child(label(name))
                .child(
                    div()
                        .text_size(px(13.5))
                        .text_color(rgb(color::TEXT))
                        .truncate()
                        .child(value),
                )
        };
        if self.on_servers() {
            let screen = online.clone();
            let Some(summary) = online.read(cx).summary(cx) else {
                return bar
                    .child(muted("Choose a server to join."))
                    .child(div().flex_1())
                    .child(launch_button("join-bar", "Join").enabled(false));
            };
            return bar
                .child(item("Mode", summary.mode))
                .child(item("Car", summary.car))
                .child(item("Track", summary.track))
                .child(item("Server", summary.server).max_w(px(420.)))
                .child(div().flex_1())
                .when_some(
                    match summary.launch {
                        Launch::Started => Some(("Starting the game…".into(), color::TEXT_MUTED)),
                        Launch::Failed(reason) => Some((reason, color::ACCENT_TEXT)),
                        Launch::Idle => None,
                    },
                    |bar, (text, tone): (SharedString, u32)| {
                        bar.child(
                            div()
                                .max_w(px(360.))
                                .text_size(px(12.5))
                                .text_color(rgb(tone))
                                .child(text),
                        )
                    },
                )
                .child(
                    launch_button("join-bar", "Join")
                        .enabled(summary.ready)
                        .on_click(move |_, _, cx| {
                            screen.update(cx, |screen, cx| screen.join(cx));
                        }),
                );
        }
        let state = game.read(cx);
        let session = state.session();
        let weather = state
            .library
            .weather(&session.weather_id)
            .map_or("", |w| w.name.as_str());
        let launch = state.launch.clone();
        let game = game.clone();

        bar.child(item("Mode", session.summary(&state.library)))
            .child(item("Car", state.car().name.clone()))
            .child(item(
                "Track",
                match state.track().layouts.len() {
                    1 => state.track().name.clone(),
                    _ => format!("{} · {}", state.track().name, state.layout().name),
                },
            ))
            .child(item(
                "Conditions",
                format!(
                    "{} · {} · {} °C",
                    format_time(session.time_seconds),
                    weather,
                    session.ambient_c
                ),
            ))
            .child(div().flex_1())
            .when_some(
                match launch {
                    Launch::Idle => None,
                    Launch::Started => Some(("Starting the game…".into(), color::TEXT_MUTED)),
                    Launch::Failed(reason) => Some((reason, color::ACCENT_TEXT)),
                },
                |bar, (text, tone): (SharedString, u32)| {
                    bar.child(
                        div()
                            .max_w(px(360.))
                            .text_size(px(12.5))
                            .text_color(rgb(tone))
                            .child(text),
                    )
                },
            )
            .when(self.section != DRIVE || self.tabs[DRIVE] != 0, |bar| {
                bar.child(button("edit-session", "Edit session").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.section = DRIVE;
                        this.tabs[DRIVE] = 0;
                        cx.notify();
                    },
                )))
            })
            .child(launch_button("drive", "Drive").on_click(move |_, _, cx| {
                game.update(cx, |game, cx| game.drive(cx));
            }))
    }
}

impl Render for Shell {
    // gpui-kit's root view puts this inside the window frame: a border with
    // resize handles, drawn where the system draws none.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(color::BG))
            .text_color(rgb(color::TEXT))
            .font(Font {
                family: font::BODY.into(),
                features: FontFeatures::default(),
                fallbacks: Some(FontFallbacks::from_fonts(
                    FALLBACK_FONTS
                        .iter()
                        .map(|name| (*name).to_owned())
                        .collect(),
                )),
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            })
            .text_size(px(13.))
            .child(self.top_bar(window, cx))
            .children(self.tab_bar(cx))
            .child(div().flex_1().min_h_0().child(self.content()))
            .child(self.session_bar(cx))
            .children(self.picker.as_ref().map(|(picker, _)| picker.clone()))
            .children(self.getting.as_ref().map(|(dialog, _)| dialog.clone()))
    }
}

/// Draws the window controls whatever the platform does, for pictures of
/// the interface taken offscreen, where windows have no decorations.
pub struct ShowWindowControls;

impl Global for ShowWindowControls {}

fn is_client_decorated(window: &Window) -> bool {
    matches!(window.window_decorations(), Decorations::Client { .. })
}

/// Whether Muretto has to draw minimise, maximise and close: on Windows,
/// and on Linux when the compositor left decorating the window to it. macOS
/// keeps its own.
fn draws_controls(window: &Window, cx: &App) -> bool {
    cx.has_global::<ShowWindowControls>()
        || cfg!(target_os = "windows")
        || (cfg!(target_os = "linux") && is_client_decorated(window))
}

/// Minimise, maximise or restore, and close, as wide flat buttons the
/// height of the title bar. Close lights up in the accent.
fn window_controls(window: &Window, cx: &App) -> impl IntoElement {
    let forced = cx.has_global::<ShowWindowControls>();
    let supported = window.window_controls();
    let maximized = window.is_maximized();
    let button = |id: &'static str, icon: &'static str, area: WindowControlArea| {
        let close = area == WindowControlArea::Close;
        div()
            .id(id)
            .group(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(52.))
            .h_full()
            .hover(move |style| style.bg(rgb(if close { color::ACCENT } else { color::RAISED })))
            .active(move |style| {
                style.bg(rgb(if close {
                    color::ACCENT_ACTIVE
                } else {
                    color::CHIP
                }))
            })
            .window_control_area(area)
            // Windows handles presses on these areas itself.
            .when(!cfg!(target_os = "windows"), |button| {
                button
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        match area {
                            WindowControlArea::Min => window.minimize_window(),
                            WindowControlArea::Max => window.zoom_window(),
                            WindowControlArea::Close => window.remove_window(),
                            WindowControlArea::Drag => {}
                        }
                    })
            })
            .child(
                svg()
                    .path(icon)
                    .size(px(if close { 17. } else { 14. }))
                    .text_color(rgb(color::TEXT_MUTED))
                    .group_hover(id, |style| style.text_color(rgb(color::TEXT))),
            )
            .test_support()
    };
    div()
        .flex()
        .h_full()
        .ml(px(16.))
        .border_l_1()
        .border_color(rgb(color::BORDER))
        .when(forced || supported.minimize, |controls| {
            controls.child(button(
                "window-minimize",
                "icons/minus.svg",
                WindowControlArea::Min,
            ))
        })
        .when(forced || supported.maximize, |controls| {
            controls.child(button(
                "window-maximize",
                if maximized {
                    "icons/copy.svg"
                } else {
                    "icons/square.svg"
                },
                WindowControlArea::Max,
            ))
        })
        .child(button(
            "window-close",
            "icons/x.svg",
            WindowControlArea::Close,
        ))
}

/// A centred heading and sentence, for screens with nothing else to show.
fn notice(title: &str, text: &str) -> AnyElement {
    let column: Div = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .size_full()
        .p(PAGE);
    column
        .child(display(title, 27.))
        .child(
            div()
                .max_w(px(460.))
                .text_size(px(13.5))
                .text_color(rgb(color::TEXT_MUTED))
                .child(SharedString::from(text.to_owned())),
        )
        .into_any_element()
}
