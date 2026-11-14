//! Online › All servers: the lobby's servers, narrowed down by searching,
//! by switches, and by ticking tracks, cars, countries and sessions from
//! lists of what servers actually run. The chosen server's details and the
//! way in sit on the right.

use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FontWeight, IntoElement, Render, SharedString, Stateful,
    Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::*,
    px, relative, rgb, svg, uniform_list,
};
use muretto_core::{
    launch,
    online::{self, Facet, Filter, Join, Listed, Listing, Server, SessionKind, Slot},
    steam::SteamUser,
};

use crate::{
    game::{Game, Launch},
    screens::facet_picker::{FacetEvent, FacetPicker},
    theme::{PAGE, RADIUS, color, font},
    ui::{badge, button, label, launch_button, mono, muted},
};

enum Load {
    /// Not asked for yet: the lobby is read when the screen is first shown.
    Idle,
    Loading,
    Failed(SharedString),
    Ready,
}

enum Slots {
    Loading,
    Loaded(Vec<Slot>),
    Failed(SharedString),
}

/// What the bottom bar shows about the chosen server.
pub struct Summary {
    pub mode: String,
    pub car: String,
    pub track: String,
    pub server: String,
    pub ready: bool,
    pub launch: Launch,
}

pub struct OnlineScreen {
    game: Entity<Game>,
    source: Arc<dyn online::Source>,
    load: Load,
    player: Option<SteamUser>,
    listing: Arc<Listing>,
    /// Why the list on show is an older copy, if it is.
    stale: Option<SharedString>,
    filter: Filter,
    /// Indices into the listing that pass the filter, in order.
    matching: Vec<usize>,
    /// Index into the listing of the server shown on the right.
    selected: Option<usize>,
    /// The selected server's slots, as it last reported them.
    slots: Option<Slots>,
    car: Option<String>,
    launch: Launch,
    search: Entity<InputState>,
    password: Entity<InputState>,
    picker: Option<(Entity<FacetPicker>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

const DETAIL_WIDTH: f32 = 440.;
const ROW: f32 = 54.;

impl OnlineScreen {
    pub fn new(
        game: Entity<Game>,
        source: Arc<dyn online::Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search name, track, car or country")
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Server password")
                .masked(true)
        });
        let _subscriptions = vec![cx.subscribe(&search, |this, search, event, cx| {
            if let InputEvent::Change = event {
                this.filter.text = search.read(cx).value().to_string();
                this.refilter();
                cx.notify();
            }
        })];
        Self {
            game,
            source,
            load: Load::Idle,
            player: None,
            listing: Arc::default(),
            stale: None,
            filter: Filter::default(),
            matching: Vec::new(),
            selected: None,
            slots: None,
            car: None,
            launch: Launch::Idle,
            search,
            password,
            picker: None,
            _subscriptions,
        }
    }

    /// Called whenever the screen is shown; reads the lobby the first time.
    pub fn visit(&mut self, cx: &mut Context<Self>) {
        if matches!(self.load, Load::Idle) {
            self.refresh(cx);
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.load, Load::Loading)
    }

    pub fn listing(&self) -> &Listing {
        &self.listing
    }

    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Servers that pass the filter, as indices into the listing.
    pub fn matching(&self) -> &[usize] {
        &self.matching
    }

    pub fn selected(&self) -> Option<&Listed> {
        self.selected.map(|index| &self.listing.servers[index])
    }

    pub fn chosen_car(&self) -> Option<&str> {
        self.car.as_deref()
    }

    pub fn picker(&self) -> Option<&Entity<FacetPicker>> {
        self.picker.as_ref().map(|(picker, _)| picker)
    }

    /// Reads the lobby again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        let source = self.source.clone();
        let library = self.game.read(cx).library.clone();
        let reading = cx.background_executor().spawn(async move {
            let player = source.player()?;
            let lobby = source.servers(&player)?;
            Ok::<_, muretto_core::Error>((
                player,
                lobby.stale,
                Listing::new(lobby.servers, &library),
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((player, stale, listing)) => {
                        this.stale = stale.map(|(age, why)| {
                            format!("Showing the list from {} ago. {why}", age_text(age)).into()
                        });
                        // Keep the same server selected if it is still listed.
                        let selected = this.selected().map(|listed| listed.server.address());
                        this.player = Some(player);
                        this.listing = Arc::new(listing);
                        this.selected = selected.and_then(|address| {
                            this.listing
                                .servers
                                .iter()
                                .position(|listed| listed.server.address() == address)
                        });
                        if this.selected.is_none() {
                            this.slots = None;
                            this.car = None;
                        }
                        this.load = Load::Ready;
                        this.refilter();
                    }
                    Err(error) => {
                        // Keep showing the list there is, and say why it is old.
                        if !this.listing.servers.is_empty() {
                            this.stale = Some(error.to_string().into());
                        }
                        this.load = Load::Failed(error.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn refilter(&mut self) {
        self.matching = self.listing.matching(&self.filter);
    }

    fn edit_filter(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Filter)) {
        change(&mut self.filter);
        self.refilter();
        cx.notify();
    }

    /// Narrows the list to `value` alone within `facet`, as the detail
    /// panel's "only this" buttons do.
    pub fn only(&mut self, facet: Facet, value: &str, cx: &mut Context<Self>) {
        self.edit_filter(cx, |filter| {
            filter.clear(facet);
            filter.toggle(facet, value);
        });
    }

    pub fn open_facet(&mut self, facet: Facet, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.listing.facet(facet, &self.filter);
        let picker = cx.new(|cx| FacetPicker::new(facet, values, window, cx));
        let subscription = cx.subscribe(&picker, |this, _, event: &FacetEvent, cx| match event {
            FacetEvent::Toggled(facet, value) => {
                let (facet, value) = (*facet, value.clone());
                this.edit_filter(cx, |filter| filter.toggle(facet, &value));
            }
            FacetEvent::Cleared(facet) => {
                let facet = *facet;
                this.edit_filter(cx, |filter| filter.clear(facet));
            }
            FacetEvent::Dismissed => {
                this.picker = None;
                cx.notify();
            }
        });
        self.picker = Some((picker, subscription));
        cx.notify();
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        self.slots = Some(Slots::Loading);
        self.launch = Launch::Idle;
        self.car = self.default_car();
        cx.notify();

        let (Some(player), Some(listed)) = (self.player.clone(), self.selected().cloned()) else {
            return;
        };
        let source = self.source.clone();
        let asking = cx
            .background_executor()
            .spawn(async move { source.slots(&listed.server, &player) });
        cx.spawn(async move |this, cx| {
            let result = asking.await;
            this.update(cx, |this, cx| {
                // The player may have moved on to another server meanwhile.
                if this.selected != Some(index) {
                    return;
                }
                this.slots = Some(match result {
                    Ok(slots) => Slots::Loaded(slots),
                    Err(error) => Slots::Failed(error.to_string().into()),
                });
                // With the slots known, prefer a car that has room.
                if this
                    .car
                    .as_ref()
                    .is_none_or(|car| this.free(car) == Some(0))
                {
                    this.car = this.default_car().or(this.car.take());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn choose_car(&mut self, car: &str, cx: &mut Context<Self>) {
        if self.listing.car(car).is_some_and(|known| known.installed) {
            self.car = Some(car.to_owned());
            self.launch = Launch::Idle;
            cx.notify();
        }
    }

    /// Free slots for `car` on the selected server, once it has said.
    fn free(&self, car: &str) -> Option<usize> {
        match &self.slots {
            Some(Slots::Loaded(slots)) => Some(
                slots
                    .iter()
                    .filter(|slot| slot.car == car && (!slot.connected || slot.yours))
                    .count(),
            ),
            _ => None,
        }
    }

    /// The car to offer first: one booked for the player, else an installed
    /// one with room, else any installed one.
    fn default_car(&self) -> Option<String> {
        let listed = self.selected()?;
        if let Some(Slots::Loaded(slots)) = &self.slots
            && let Some(yours) = slots.iter().find(|slot| slot.yours)
        {
            return Some(yours.car.clone());
        }
        let installed: Vec<&String> = listed
            .server
            .cars
            .iter()
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|car| self.listing.car(car).is_some_and(|known| known.installed))
            .collect();
        installed
            .iter()
            .find(|car| self.free(car).is_none_or(|free| free > 0))
            .or(installed.first())
            .map(|car| (*car).clone())
    }

    /// Why joining cannot happen yet, if it cannot.
    fn blocker(&self, cx: &App) -> Option<&'static str> {
        let listed = self.selected()?;
        if !listed.track_installed {
            return Some("The track is not installed.");
        }
        let car = self.car.as_deref()?;
        if !self.listing.car(car).is_some_and(|known| known.installed) {
            return Some("Choose a car you have installed.");
        }
        if self.free(car) == Some(0) {
            return Some("That car has no free slot.");
        }
        if listed.server.password && self.password.read(cx).value().is_empty() {
            return Some("The server needs a password.");
        }
        None
    }

    pub fn can_join(&self, cx: &App) -> bool {
        self.selected().is_some() && self.car.is_some() && self.blocker(cx).is_none()
    }

    /// Writes the server into the session and starts the game.
    pub fn join(&mut self, cx: &mut Context<Self>) {
        if !self.can_join(cx) {
            return;
        }
        let (Some(listed), Some(car), Some(player)) =
            (self.selected(), self.car.as_deref(), self.player.as_ref())
        else {
            return;
        };
        let game = self.game.read(cx);
        // The server hands out a skin with the slot; ask for the one it
        // offers if this copy of the car has it.
        let installed = game.library.car(car);
        let offered = match &self.slots {
            Some(Slots::Loaded(slots)) => slots
                .iter()
                .find(|slot| slot.car == car && !slot.connected)
                .map(|slot| slot.skin.clone()),
            _ => None,
        };
        let skin = offered
            .filter(|skin| installed.is_some_and(|c| c.skin(skin).is_some()))
            .or_else(|| {
                installed
                    .and_then(|c| c.skins.first())
                    .map(|s| s.id.clone())
            })
            .unwrap_or_default();
        let password = self.password.read(cx).value().to_string();
        let join = Join {
            listed,
            car,
            skin: &skin,
            player,
            driver_name: &game.session().driver_name,
            password: &password,
        };
        self.launch = match launch::join(&game.install, &join) {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Launch::Started
            }
            Err(error) => Launch::Failed(error.to_string().into()),
        };
        cx.notify();
    }

    pub fn summary(&self, cx: &App) -> Option<Summary> {
        let listed = self.selected()?;
        let server = &listed.server;
        let mode = match server.current {
            Some(kind) => format!("Online · {}", kind.label()),
            None => "Online".to_owned(),
        };
        Some(Summary {
            mode,
            car: self
                .car
                .as_deref()
                .map_or("No car chosen".to_owned(), |car| {
                    self.listing.car_name(car).to_owned()
                }),
            track: self.listing.track_name(&listed.track_id).to_owned(),
            server: format!("{} · {}/{}", server.name, server.clients, server.capacity),
            ready: self.can_join(cx),
            launch: self.launch.clone(),
        })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = |id: &'static str, text: &'static str, on: bool, set: fn(&mut Filter)| {
            chip(id, text, on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit_filter(cx, set);
                }))
                .test_support()
        };
        let filter = &self.filter;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(PAGE)
            .pt(px(14.))
            .child(
                div().w(px(340.)).flex_none().child(
                    Input::new(&self.search).prefix(
                        svg()
                            .path("icons/search.svg")
                            .size(px(14.))
                            .text_color(rgb(color::TEXT_DIM)),
                    ),
                ),
            )
            .child(toggle("only-free", "Free slots", filter.free_slots, |f| {
                f.free_slots = !f.free_slots
            }))
            .child(toggle("only-busy", "Not empty", filter.not_empty, |f| {
                f.not_empty = !f.not_empty
            }))
            .child(toggle(
                "only-open",
                "No password",
                filter.no_password,
                |f| f.no_password = !f.no_password,
            ))
            .child(toggle(
                "only-installed",
                "Content installed",
                filter.installed,
                |f| f.installed = !f.installed,
            ))
            .child(div().flex_1())
            .child(
                mono(
                    format!("{} of {}", self.matching.len(), self.listing.servers.len()),
                    12.,
                )
                .text_color(rgb(color::TEXT_MUTED)),
            )
            .child(
                button(
                    "online-refresh",
                    if self.is_loading() {
                        "Reading…"
                    } else {
                        "Refresh"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.is_loading() {
                        this.refresh(cx);
                    }
                })),
            )
    }

    /// One button per facet, then every chosen value as a removable chip.
    fn facet_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(PAGE)
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(color::BORDER));
        for (index, facet) in Facet::ALL.into_iter().enumerate() {
            let chosen = self.filter.chosen(facet).len();
            let text = match chosen {
                0 => facet.label().to_owned(),
                n => format!("{} · {n}", facet.label()),
            };
            bar = bar.child(
                div()
                    .id(("facet", index))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(30.))
                    .pl(px(10.))
                    .pr(px(8.))
                    .rounded(RADIUS)
                    .border_1()
                    .border_color(rgb(if chosen > 0 {
                        color::ACCENT
                    } else {
                        color::BORDER_STRONG
                    }))
                    .bg(rgb(color::SURFACE))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(color::RAISED)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_facet(facet, window, cx)),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(text),
                    )
                    .child(
                        svg()
                            .path("icons/chevron-down.svg")
                            .size(px(12.))
                            .text_color(rgb(color::TEXT_MUTED)),
                    )
                    .test_support(),
            );
        }

        let chosen: Vec<(Facet, String)> = Facet::ALL
            .into_iter()
            .flat_map(|facet| {
                self.filter
                    .chosen(facet)
                    .into_iter()
                    .map(move |value| (facet, value))
            })
            .collect();
        if chosen.is_empty() {
            return bar.child(
                div()
                    .ml(px(4.))
                    .text_size(px(12.5))
                    .text_color(rgb(color::TEXT_DIM))
                    .child("Pick tracks, cars, countries or sessions to narrow the list."),
            );
        }
        bar = bar.child(div().w(px(1.)).h(px(20.)).mx(px(4.)).bg(rgb(color::BORDER)));
        for (index, (facet, value)) in chosen.into_iter().enumerate() {
            let name = self.listing.value_name(facet, &value);
            bar = bar.child(
                div()
                    .id(("chosen", index))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .rounded(RADIUS)
                    .bg(rgb(color::ACCENT_WASH))
                    .border_1()
                    .border_color(rgb(color::ACCENT))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_filter(cx, |filter| filter.toggle(facet, &value));
                    }))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(color::TEXT_MUTED))
                            .child(facet.label()),
                    )
                    .child(div().text_size(px(12.5)).child(name))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(px(11.))
                            .text_color(rgb(color::TEXT_MUTED)),
                    )
                    .test_support(),
            );
        }
        bar.child(
            div()
                .id("clear-filters")
                .text_size(px(12.5))
                .text_color(rgb(color::ACCENT_TEXT))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.edit_filter(cx, |filter| {
                        for facet in Facet::ALL {
                            filter.clear(facet);
                        }
                    });
                }))
                .child("Clear all"),
        )
    }

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(34.))
            .px(PAGE)
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .child(label("Server").flex_1().min_w_0())
            .child(label("Track").w(px(190.)).flex_none())
            .child(label("Cars").w(px(170.)).flex_none())
            .child(label("Session").w(px(110.)).flex_none())
            .child(
                div()
                    .w(px(64.))
                    .flex()
                    .justify_end()
                    .child(label("Drivers")),
            );

        let count = self.matching.len();
        let rows = uniform_list(
            "servers",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|row| {
                        let index = this.matching[row];
                        this.row(row, index, cx)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .map(|list| {
                if count == 0 {
                    list.child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .child(muted(if self.filter.is_empty() {
                                "The lobby lists no servers."
                            } else {
                                "No server matches. Loosen the filter to see more."
                            })),
                    )
                } else {
                    list.child(rows)
                }
            })
    }

    fn row(&self, row: usize, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let listed = &self.listing.servers[index];
        let server = &listed.server;
        let selected = self.selected == Some(index);
        let track = self.listing.track_name(&listed.track_id).to_owned();
        let cars = match server.cars.as_slice() {
            [one] => self.listing.car_name(one).to_owned(),
            all => format!("{} cars", all.len()),
        };
        let mut about: Vec<String> = Vec::new();
        if let Some((country, _)) = &server.country {
            about.push(country.clone());
        }
        if server.track.patch_build.is_some() {
            about.push("CSP required".to_owned());
        }
        let missing = missing_text(listed);
        let full = server.free_slots() == 0;

        div()
            .id(("server", row))
            .flex()
            .w_full()
            .items_center()
            .gap(px(12.))
            .h(px(ROW))
            .px(PAGE)
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .cursor_pointer()
            .map(|row| {
                if selected {
                    row.bg(rgb(color::ACCENT_WASH))
                        .border_l_2()
                        .border_color(rgb(color::ACCENT))
                } else {
                    row.hover(|style| style.bg(rgb(color::RAISED)))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
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
                            .items_center()
                            .gap(px(6.))
                            .when(server.password, |name| {
                                name.child(
                                    svg()
                                        .path("icons/lock.svg")
                                        .flex_none()
                                        .size(px(12.))
                                        .text_color(rgb(color::TEXT_MUTED)),
                                )
                            })
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(server.name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(rgb(color::TEXT_MUTED))
                            .truncate()
                            .child(about.join(" · "))
                            .when_some(missing, |line, missing| {
                                line.child(
                                    div()
                                        .text_color(rgb(color::WARN))
                                        .child(format!("Missing: {missing}")),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(190.))
                    .flex_none()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(if listed.track_installed {
                        color::TEXT
                    } else {
                        color::TEXT_MUTED
                    }))
                    .child(track),
            )
            .child(
                div()
                    .w(px(170.))
                    .flex_none()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(color::TEXT_MUTED))
                    .child(cars),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .child(server.current.map_or("—", SessionKind::label)),
                    )
                    .child(mono(time_left(server), 11.).text_color(rgb(color::TEXT_DIM))),
            )
            .child(
                div().w(px(64.)).flex().justify_end().child(
                    mono(format!("{}/{}", server.clients, server.capacity), 13.)
                        .text_color(rgb(if full { color::TEXT_DIM } else { color::TEXT })),
                ),
            )
            .test_support()
            .into_any_element()
    }

    fn detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = div()
            .id("server-detail")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(DETAIL_WIDTH))
            .h_full()
            .border_l_1()
            .border_color(rgb(color::BORDER))
            .bg(rgb(color::SURFACE))
            .overflow_y_scroll();
        let Some(listed) = self.selected() else {
            return panel.child(div().p(px(24.)).child(muted(
                "Choose a server to see its sessions, cars and drivers.",
            )));
        };
        let server = &listed.server;
        let mut about: Vec<String> = Vec::new();
        if let Some((country, _)) = &server.country {
            about.push(country.clone());
        }
        about.push(if server.password {
            "Password".to_owned()
        } else {
            "No password".to_owned()
        });
        if let Some(build) = server.track.patch_build {
            about.push(format!("CSP build {build}+"));
        }
        about.push(server.address());

        let section = || div().flex().flex_col().gap(px(10.)).px(px(20.)).py(px(16.));

        panel
            .child(
                section()
                    .border_b_1()
                    .border_color(rgb(color::BORDER))
                    .child(
                        div()
                            .font_family(font::DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .italic()
                            .text_size(px(22.))
                            .line_height(relative(1.1))
                            .child(server.name.to_uppercase()),
                    )
                    .child(muted(about.join(" · "))),
            )
            .when(!server.sessions.is_empty(), |panel| {
                panel.child(
                    section()
                        .child(label("Sessions"))
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .children(server.sessions.iter().map(|session| {
                                    let current = server.current == Some(session.kind);
                                    session_tile(server, session, current)
                                })),
                        ),
                )
            })
            .child(self.track_box(listed, cx))
            .child(self.cars_box(listed, cx))
            .child(self.drivers_box())
            .child(self.join_box(listed, cx))
    }

    fn track_box(&self, listed: &Listed, cx: &mut Context<Self>) -> impl IntoElement {
        let known = self.listing.track(&listed.track_id);
        let name = self.listing.track_name(&listed.track_id).to_owned();
        let layout = listed.layout.clone();
        let track_id = listed.track_id.clone();
        let only = self.filter.tracks.len() == 1 && self.filter.tracks.contains(&track_id);
        div().px(px(20.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .p(px(12.))
                .border_1()
                .border_color(rgb(color::BORDER))
                .rounded(RADIUS)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .child(label("Track"))
                        .child(div().text_size(px(14.)).truncate().child(name))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .items_center()
                                .when_some(layout, |row, layout| {
                                    row.child(mono(layout, 11.5).text_color(rgb(color::TEXT_MUTED)))
                                })
                                .child(if known.is_some_and(|k| k.installed) {
                                    badge("Installed")
                                } else {
                                    badge("Not installed")
                                }),
                        ),
                )
                .when(!only, |row| {
                    row.child(button("only-track", "Only this track").on_click(
                        cx.listener(move |this, _, _, cx| this.only(Facet::Track, &track_id, cx)),
                    ))
                }),
        )
    }

    fn cars_box(&self, listed: &Listed, cx: &mut Context<Self>) -> impl IntoElement {
        let server = &listed.server;
        // Cars the player can take first, then full ones, then the rest.
        let mut cars: Vec<&String> = server.cars.iter().collect();
        cars.sort_by_key(|car| {
            let installed = self.listing.car(car).is_some_and(|k| k.installed);
            match (installed, self.free(car)) {
                (true, Some(0)) => 1,
                (true, _) => 0,
                (false, _) => 2,
            }
        });
        let free_total = match &self.slots {
            Some(Slots::Loaded(slots)) => Some(slots.iter().filter(|s| !s.connected).count()),
            _ => None,
        };
        let heading = match free_total {
            Some(free) => format!("Your car · {free} of {} slots free", server.capacity),
            None => format!("Your car · {} to choose from", cars.len()),
        };
        let rows = cars.into_iter().enumerate().map(|(index, car)| {
            let known = self.listing.car(car);
            let installed = known.is_some_and(|k| k.installed);
            let chosen = self.car.as_deref() == Some(car.as_str());
            let free = self.free(car);
            let state = match (installed, free) {
                (false, _) => "not installed".to_owned(),
                (true, Some(0)) => "full".to_owned(),
                (true, Some(n)) => format!("{n} free"),
                (true, None) => String::new(),
            };
            let only = self.filter.cars.len() == 1 && self.filter.cars.contains(car);
            let id = car.clone();
            let filter_id = car.clone();
            div()
                .id(("car", index))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(40.))
                .px(px(12.))
                .border_1()
                .rounded(RADIUS)
                .border_color(rgb(if chosen { color::ACCENT } else { color::BORDER }))
                .when(chosen, |row| row.bg(rgb(color::ACCENT_WASH)))
                .when(installed, |row| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(rgb(color::RAISED)))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_car(&id, cx)))
                })
                .child(radio(chosen, installed))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(rgb(if installed {
                            color::TEXT
                        } else {
                            color::TEXT_DIM
                        }))
                        .child(self.listing.car_name(car).to_owned()),
                )
                .child(mono(state, 11.5).text_color(rgb(color::TEXT_MUTED)))
                .when(!only, |row| {
                    row.child(
                        div()
                            .id(("only-car", index))
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(RADIUS)
                            .text_size(px(11.5))
                            .text_color(rgb(color::TEXT_MUTED))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(color::CHIP)).text_color(rgb(color::TEXT)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.only(Facet::Car, &filter_id, cx);
                            }))
                            .child("Only this")
                            .test_support(),
                    )
                })
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(20.))
            .pt(px(16.))
            .child(label(&heading))
            .children(rows)
    }

    fn drivers_box(&self) -> impl IntoElement {
        let section = div().flex().flex_col().gap(px(8.)).px(px(20.)).pt(px(16.));
        match &self.slots {
            None | Some(Slots::Loading) => section
                .child(label("Drivers"))
                .child(muted("Asking the server…")),
            Some(Slots::Failed(error)) => section
                .child(label("Drivers"))
                .child(muted(format!("The server did not answer: {error}"))),
            Some(Slots::Loaded(slots)) => {
                let driving: Vec<&Slot> = slots.iter().filter(|s| s.connected).collect();
                section
                    .child(label(&format!("Drivers · {}", driving.len())))
                    .when(driving.is_empty(), |section| {
                        section.child(muted("Nobody is driving."))
                    })
                    .children(driving.into_iter().take(40).map(|slot| {
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(12.))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .truncate()
                                    .child(slot.driver.clone().unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .child(self.listing.car_name(&slot.car).to_owned()),
                            )
                    }))
            }
        }
    }

    fn join_box(&self, listed: &Listed, cx: &mut Context<Self>) -> impl IntoElement {
        let (tone, word, text) = match missing_text(listed) {
            None => (
                color::GOOD,
                "Ready",
                "The track and your car are installed.".to_owned(),
            ),
            Some(missing) if listed.joinable() => (
                color::GOOD,
                "Ready",
                format!("The track and some cars are installed. Not installed: {missing}."),
            ),
            Some(missing) => (
                color::WARN,
                "Missing",
                format!("{missing}. Install the content to join."),
            ),
        };
        let blocker = self.blocker(cx);
        let can_join = self.can_join(cx);
        let status = match &self.launch {
            Launch::Idle => blocker.map(|text| (SharedString::from(text), color::TEXT_MUTED)),
            Launch::Started => Some(("Starting the game…".into(), color::TEXT_MUTED)),
            Launch::Failed(reason) => Some((reason.clone(), color::ACCENT_TEXT)),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .p(px(12.))
                    .rounded(RADIUS)
                    .bg(rgb(color::RAISED))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(tone))
                            .child(word.to_uppercase()),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(relative(1.4))
                            .child(text),
                    ),
            )
            .when(listed.server.password, |column| {
                column.child(Input::new(&self.password))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .when_some(status, |line, (text, tone)| {
                                line.text_color(rgb(tone)).child(text)
                            }),
                    )
                    .child(
                        launch_button("join", "Join")
                            .enabled(can_join)
                            .on_click(cx.listener(|this, _, _, cx| this.join(cx))),
                    ),
            )
    }
}

impl Render for OnlineScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("online").relative().flex().size_full();
        match &self.load {
            Load::Idle | Load::Loading if self.listing.servers.is_empty() => {
                return root.child(centered(
                    "Reading the lobby",
                    "Asking the official lobby which servers are up.".into(),
                    None,
                ));
            }
            Load::Failed(error) if self.listing.servers.is_empty() => {
                let error = error.clone();
                return root.child(centered(
                    "The lobby did not answer",
                    error,
                    Some(
                        button("online-retry", "Try again")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
                ));
            }
            _ => {}
        }
        root.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(self.toolbar(cx))
                .when_some(self.stale.clone(), |column, stale| {
                    column.child(
                        div()
                            .mx(PAGE)
                            .mt(px(10.))
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(RADIUS)
                            .bg(rgb(color::RAISED))
                            .text_size(px(12.5))
                            .text_color(rgb(color::WARN))
                            .child(stale),
                    )
                })
                .child(self.facet_bar(cx))
                .child(self.list(cx)),
        )
        .child(self.detail(cx))
        .children(self.picker.as_ref().map(|(picker, _)| picker.clone()))
    }
}

/// "the track, 6 cars", or nothing when everything is installed.
fn missing_text(listed: &Listed) -> Option<String> {
    let cars = listed.missing_cars();
    let mut parts = Vec::new();
    if !listed.track_installed {
        parts.push("the track".to_owned());
    }
    match cars {
        0 => {}
        1 => parts.push("1 car".to_owned()),
        n => parts.push(format!("{n} cars")),
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// "3 minutes", "2 hours".
fn age_text(age: std::time::Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "moments".to_owned(),
        1 => "a minute".to_owned(),
        2..=119 => format!("{minutes} minutes"),
        _ => format!("{} hours", minutes / 60),
    }
}

/// Time left in the current session, as the list shows it.
fn time_left(server: &Server) -> String {
    let seconds = server.time_left;
    if seconds == 0 {
        return String::new();
    }
    if seconds >= 3600 {
        format!("{}:{:02} h left", seconds / 3600, seconds / 60 % 60)
    } else {
        format!("{:02}:{:02} left", seconds / 60, seconds % 60)
    }
}

fn session_tile(server: &Server, session: &online::Session, current: bool) -> Div {
    // Races run over laps unless the server says otherwise; other sessions
    // last minutes.
    let length = match (session.kind, session.length) {
        (_, 0) => String::new(),
        (SessionKind::Race, laps) if laps < 200 => format!("{laps} laps"),
        (_, minutes) => format!("{minutes} min"),
    };
    let detail = if current { time_left(server) } else { length };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .p(px(8.))
        .rounded(RADIUS)
        .border_1()
        .border_color(rgb(if current {
            color::ACCENT
        } else {
            color::BORDER
        }))
        .when(current, |tile| tile.bg(rgb(color::ACCENT_WASH)))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(session.kind.label()),
        )
        .child(mono(detail, 11.).text_color(rgb(color::TEXT_MUTED)))
}

/// A switch-like chip in the toolbar: filled when on.
fn chip(id: &'static str, text: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(30.))
        .px(px(10.))
        .rounded(RADIUS)
        .border_1()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|chip| {
            if on {
                chip.bg(rgb(color::LIGHT))
                    .border_color(rgb(color::LIGHT))
                    .text_color(rgb(color::BG))
            } else {
                chip.bg(rgb(color::SURFACE))
                    .border_color(rgb(color::BORDER_STRONG))
                    .text_color(rgb(color::TEXT))
                    .hover(|style| style.bg(rgb(color::RAISED)))
            }
        })
        .child(text)
}

fn radio(on: bool, enabled: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.))
        .rounded_full()
        .border_1()
        .border_color(rgb(if on {
            color::ACCENT
        } else if enabled {
            color::BORDER_STRONG
        } else {
            color::BORDER
        }))
        .when(on, |dot| {
            dot.child(div().size(px(6.)).rounded_full().bg(rgb(color::ACCENT)))
        })
}

fn centered(
    title: &str,
    text: SharedString,
    action: Option<crate::ui::Button>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .size_full()
        .p(PAGE)
        .child(crate::ui::display(title, 27.))
        .child(
            div()
                .max_w(px(460.))
                .text_size(px(13.5))
                .text_color(rgb(color::TEXT_MUTED))
                .child(text),
        )
        .children(action)
}
