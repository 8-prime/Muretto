//! Online: the lobby's servers, the player's favourites and recent ones,
//! and those on the local network, narrowed down by searching, by switches,
//! and by ticking tracks, cars, countries and sessions from lists of what
//! servers actually run. The chosen server's details, the content it needs
//! and the way in sit on the right.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, mpsc},
    time::Duration,
};

use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FontWeight, IntoElement, ObjectFit, Render,
    SharedString, Stateful, Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputEvent, InputState},
    div, img, pattern_slash,
    prelude::*,
    px, relative, rgb, svg, uniform_list,
};
use muretto_core::{
    Car, Error, Library, Skin,
    country::Flags,
    launch,
    online::{
        self, Column, Facet, Filter, Found, Join, Listed, Listing, Order, Scope, Server,
        SessionKind, Slot, looks_like_traffic,
        offers::{Kind, Offer},
        saved::Saved,
    },
    packages::content::{Added, install_offer},
    steam::SteamUser,
};

use crate::{
    game::{Game, Launch},
    job::Job,
    screens::{
        facet_picker::{FacetEvent, FacetPicker},
        friends::{Friends, GroupSetup},
        get_content::{ContentDialog, ContentEvent, added_text},
    },
    theme::{PAGE, RADIUS, color, font, hsla},
    ui::{badge, button, label, launch_button, mono, muted, stat_strip},
};

/// The tabs of the Online section, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    Favourites,
    Recent,
    Lan,
    Friends,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::All,
        Tab::Favourites,
        Tab::Recent,
        Tab::Lan,
        Tab::Friends,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All servers",
            Tab::Favourites => "Favourites",
            Tab::Recent => "Recent",
            Tab::Lan => "LAN",
            Tab::Friends => "Friends",
        }
    }
}

enum Lan {
    Idle,
    Searching,
    Searched,
}

enum Offers {
    Loading,
    Loaded(Vec<Offer>),
    Failed(SharedString),
}

/// A ping at or above this many milliseconds is shown as a warning.
const SLOW_PING: u32 = 90;

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
    pub(super) source: Arc<dyn online::Source>,
    load: Load,
    player: Option<SteamUser>,
    saved: Saved,
    tab: Tab,
    /// The lobby's servers, and kept ones asked directly.
    servers: Arc<Vec<Server>>,
    /// Servers that answered on this network.
    lan_servers: Vec<Server>,
    lan: Lan,
    /// Names of kept servers that neither the lobby lists nor answered.
    unanswered: Vec<(String, String)>,
    listing: Arc<Listing>,
    /// The content the listing was worked out against.
    listed_library: Arc<Library>,
    /// Why the list on show is an older copy, if it is.
    stale: Option<SharedString>,
    filter: Filter,
    order: Order,
    /// Milliseconds, by where the server answers pings.
    pings: HashMap<SocketAddr, u32>,
    /// The same, by index into the listing.
    ping_by_index: Vec<Option<u32>>,
    /// Indices into the listing that pass the filter, in order.
    matching: Vec<usize>,
    /// Index into the listing of the server shown on the right.
    selected: Option<usize>,
    /// The selected server's slots, as it last reported them.
    slots: Option<Slots>,
    car: Option<String>,
    /// The car on the selected server whose picture and figures are open.
    car_info: Option<String>,
    /// Where the selected server says its content can be downloaded.
    offers: Option<Offers>,
    /// Content being downloaded for the selected server.
    fetching: Option<Job>,
    /// What the last download added, or why it failed.
    fetched: Option<(SharedString, bool)>,
    show_traffic: bool,
    launch: Launch,
    /// The game's flag pictures, for the servers' countries.
    flags: Flags,
    search: Entity<InputState>,
    password: Entity<InputState>,
    picker: Option<(Entity<FacetPicker>, Subscription)>,
    dialog: Option<(Entity<ContentDialog>, Subscription)>,
    /// The player's groups, on the Friends tab.
    pub(super) friends: Friends,
    _subscriptions: Vec<Subscription>,
}

const DETAIL_WIDTH: f32 = 440.;
const ROW: f32 = 54.;
const STAR_W: f32 = 16.;
const TRACK_W: f32 = 170.;
const CARS_W: f32 = 150.;
const SESSION_W: f32 = 104.;
const DRIVERS_W: f32 = 64.;
const PING_W: f32 = 56.;

impl OnlineScreen {
    pub fn new(
        game: Entity<Game>,
        source: Arc<dyn online::Source>,
        saved: Saved,
        groups: GroupSetup,
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
        let _subscriptions = vec![
            cx.subscribe(&search, |this, search, event, cx| {
                if let InputEvent::Change = event {
                    this.filter.text = search.read(cx).value().to_string();
                    this.refilter();
                    cx.notify();
                }
            }),
            // Content added elsewhere changes what is installed and missing.
            cx.observe(&game, |this, game, cx| {
                if !Arc::ptr_eq(&this.listed_library, &game.read(cx).library) {
                    this.relist(cx);
                }
            }),
        ];
        let listed_library = game.read(cx).library.clone();
        let flags = Flags::scan(&game.read(cx).install);
        let friends = Friends::new(groups, window, cx);
        let mut screen = Self {
            game,
            source,
            load: Load::Idle,
            player: None,
            saved,
            tab: Tab::All,
            servers: Arc::default(),
            lan_servers: Vec::new(),
            lan: Lan::Idle,
            unanswered: Vec::new(),
            listing: Arc::default(),
            listed_library,
            stale: None,
            filter: Filter::default(),
            order: Order::default(),
            pings: HashMap::new(),
            ping_by_index: Vec::new(),
            matching: Vec::new(),
            selected: None,
            slots: None,
            car: None,
            car_info: None,
            offers: None,
            fetching: None,
            fetched: None,
            show_traffic: false,
            launch: Launch::Idle,
            flags,
            search,
            password,
            picker: None,
            dialog: None,
            friends,
            _subscriptions,
        };
        screen.start_groups(cx);
        screen
    }

    pub fn friends(&self) -> &Friends {
        &self.friends
    }

    /// Where in the listing the server at `address` (`ip:http-port`) is.
    pub(super) fn index_of(&self, address: &str) -> Option<usize> {
        self.listing
            .servers
            .iter()
            .position(|listed| listed.server.address() == address)
    }

    /// Lists a server that was asked directly.
    pub(super) fn add_server(&mut self, server: Server, cx: &mut Context<Self>) {
        let servers = Arc::make_mut(&mut self.servers);
        servers.retain(|known| known.address() != server.address());
        servers.push(server);
        self.relist(cx);
    }

    /// Works out the tab's servers again, as when what it holds changed.
    pub(super) fn rescope(&mut self) {
        self.filter.scope = self.scope();
        self.refilter();
    }

    /// Called whenever the screen is shown; reads the lobby the first time,
    /// and searches the network the first time LAN is open.
    pub fn visit(&mut self, cx: &mut Context<Self>) {
        if matches!(self.load, Load::Idle) {
            self.read_lobby(cx);
        }
        if self.tab == Tab::Lan && matches!(self.lan, Lan::Idle) {
            self.search_lan(cx);
        }
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.filter.scope = self.scope();
        self.refilter();
        self.visit(cx);
        cx.notify();
    }

    fn scope(&self) -> Scope {
        let addresses = |kept: &[online::saved::Kept]| {
            Scope::Kept(kept.iter().map(|k| k.address.clone()).collect())
        };
        match self.tab {
            Tab::All => Scope::Lobby,
            Tab::Favourites => addresses(&self.saved.favourites),
            Tab::Recent => addresses(&self.saved.recent),
            Tab::Lan => Scope::Lan,
            Tab::Friends => Scope::Kept(self.friends.viewed().into_iter().collect()),
        }
    }

    /// How many servers a tab holds, before any filter.
    pub fn count(&self, tab: Tab) -> Option<usize> {
        match tab {
            Tab::All => (!self.servers.is_empty()).then(|| {
                self.servers
                    .iter()
                    .filter(|server| server.found == Found::Lobby)
                    .count()
            }),
            Tab::Favourites => Some(self.saved.favourites.len()),
            Tab::Recent => Some(self.saved.recent.len()),
            Tab::Lan => matches!(self.lan, Lan::Searched).then_some(self.lan_servers.len()),
            Tab::Friends => self.friends.has_groups().then(|| self.friends.online()),
        }
    }

    pub fn saved(&self) -> &Saved {
        &self.saved
    }

    pub fn order(&self) -> Order {
        self.order
    }

    /// The selected server's ping in milliseconds, once it has answered.
    pub fn ping(&self, index: usize) -> Option<u32> {
        self.ping_by_index.get(index).copied().flatten()
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

    /// Reads the lobby again, or searches the network again on LAN.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.tab == Tab::Lan {
            self.search_lan(cx);
        } else {
            self.read_lobby(cx);
        }
    }

    pub fn is_searching_lan(&self) -> bool {
        matches!(self.lan, Lan::Searching)
    }

    fn read_lobby(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        let source = self.source.clone();
        let library = self.game.read(cx).library.clone();
        let lan = self.lan_servers.clone();
        let kept: Vec<(String, String)> = self
            .saved
            .addresses()
            .map(|kept| (kept.address.clone(), kept.name.clone()))
            .collect();
        let reading = cx.background_executor().spawn(async move {
            let player = source.player()?;
            let lobby = source.servers(&player)?;
            let mut servers = lobby.servers;
            // Kept servers the lobby does not list may still be up: ask them.
            let listed: HashSet<String> = servers.iter().map(Server::address).collect();
            let mut asked = HashSet::new();
            let asking: Vec<_> = kept
                .into_iter()
                .filter(|(address, _)| !listed.contains(address) && asked.insert(address.clone()))
                .map(|(address, name)| {
                    let source = source.clone();
                    std::thread::spawn(move || {
                        let (ip, port) = address.rsplit_once(':')?;
                        let answer = source.info(ip, port.parse().ok()?);
                        Some(match answer {
                            Ok(mut server) => {
                                server.found = Found::Direct;
                                Ok(server)
                            }
                            Err(_) => Err((address, name)),
                        })
                    })
                })
                .collect();
            let mut unanswered = Vec::new();
            for asking in asking {
                match asking.join().ok().flatten() {
                    Some(Ok(server)) => servers.push(server),
                    Some(Err(kept)) => unanswered.push(kept),
                    None => {}
                }
            }
            let listing = Listing::new(servers.iter().cloned().chain(lan).collect(), &library);
            Ok::<_, Error>((player, lobby.stale, servers, unanswered, listing, library))
        });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((player, stale, servers, unanswered, listing, library)) => {
                        this.stale = stale.map(|(age, why)| {
                            format!("Showing the list from {} ago. {why}", age_text(age)).into()
                        });
                        this.player = Some(player);
                        this.unanswered = unanswered;
                        let targets: Vec<SocketAddr> =
                            servers.iter().filter_map(Server::ping_address).collect();
                        this.servers = Arc::new(servers);
                        this.show_listing(listing, library);
                        this.load = Load::Ready;
                        this.listed_jump(cx);
                        this.start_pinging(targets, cx);
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

    /// Asks the local network which servers run on it.
    fn search_lan(&mut self, cx: &mut Context<Self>) {
        if self.is_searching_lan() {
            return;
        }
        self.lan = Lan::Searching;
        let source = self.source.clone();
        let searching = cx.background_executor().spawn(async move { source.lan() });
        cx.spawn(async move |this, cx| {
            let result = searching.await;
            this.update(cx, |this, cx| {
                this.lan = Lan::Searched;
                this.lan_servers = result.unwrap_or_default();
                let targets = this
                    .lan_servers
                    .iter()
                    .filter_map(Server::ping_address)
                    .collect();
                this.relist(cx);
                this.start_pinging(targets, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Works the servers out again against the installed content.
    fn relist(&mut self, cx: &mut Context<Self>) {
        let library = self.game.read(cx).library.clone();
        let servers: Vec<Server> = self
            .servers
            .iter()
            .cloned()
            .chain(self.lan_servers.iter().cloned())
            .collect();
        let listing = cx
            .background_executor()
            .spawn(async move { (Listing::new(servers, &library), library) });
        cx.spawn(async move |this, cx| {
            let (listing, library) = listing.await;
            this.update(cx, |this, cx| {
                this.show_listing(listing, library);
                this.listed_jump(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Puts a new listing on show, keeping the same server selected if it
    /// is still listed.
    fn show_listing(&mut self, listing: Listing, library: Arc<Library>) {
        let selected = self.selected().map(|listed| listed.server.address());
        self.listing = Arc::new(listing);
        self.listed_library = library;
        self.selected = selected.and_then(|address| {
            self.listing
                .servers
                .iter()
                .position(|listed| listed.server.address() == address)
        });
        if self.selected.is_none() {
            self.slots = None;
            self.offers = None;
            self.car = None;
        } else if self.car.as_ref().is_none_or(|car| !self.installed(car)) {
            // A car that has just been installed can now be taken.
            self.car = self.default_car();
        }
        self.index_pings();
        self.refilter();
        // The server this player had open may have gone from the list.
        self.tell_groups();
    }

    /// Pings the servers in the background and shows each answer as it
    /// comes.
    fn start_pinging(&mut self, targets: Vec<SocketAddr>, cx: &mut Context<Self>) {
        if targets.is_empty() {
            return;
        }
        let (send, answers) = mpsc::channel();
        let source = self.source.clone();
        std::thread::spawn(move || {
            source.ping(&targets, &mut |from, took| {
                let _ = send.send((from, took));
            });
        });
        // Answers are collected off the interface thread, a quarter of a
        // second's worth at a time, and shown together.
        let answers = Arc::new(std::sync::Mutex::new(answers));
        cx.spawn(async move |this, cx| {
            loop {
                let answers = answers.clone();
                let (batch, finished) = cx
                    .background_executor()
                    .spawn(async move {
                        let answers = answers.lock().unwrap();
                        let mut batch = Vec::new();
                        let started = std::time::Instant::now();
                        let quarter = Duration::from_millis(250);
                        loop {
                            let left = quarter.saturating_sub(started.elapsed());
                            match answers.recv_timeout(left) {
                                Ok(answer) => batch.push(answer),
                                Err(mpsc::RecvTimeoutError::Timeout) => break (batch, false),
                                Err(mpsc::RecvTimeoutError::Disconnected) => break (batch, true),
                            }
                        }
                    })
                    .await;
                if !batch.is_empty() && this.update(cx, |this, cx| this.pinged(batch, cx)).is_err()
                {
                    break;
                }
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn pinged(&mut self, answers: Vec<(SocketAddr, Duration)>, cx: &mut Context<Self>) {
        for (from, took) in answers {
            self.pings.insert(from, took.as_millis() as u32);
        }
        self.index_pings();
        if self.order.column == Column::Ping {
            self.refilter();
        }
        cx.notify();
    }

    fn index_pings(&mut self) {
        self.ping_by_index = self
            .listing
            .servers
            .iter()
            .map(|listed| {
                listed
                    .server
                    .ping_address()
                    .and_then(|address| self.pings.get(&address).copied())
            })
            .collect();
    }

    fn refilter(&mut self) {
        self.matching = self
            .listing
            .matching(&self.filter, self.order, &self.ping_by_index);
    }

    /// Sorts by `column`, or turns the order round if it already is.
    pub fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.order = self.order.by(column);
        self.refilter();
        cx.notify();
    }

    /// Stars the server at `index` in the listing, or takes the star away.
    pub fn toggle_favourite(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(listed) = self.listing.servers.get(index) else {
            return;
        };
        let _ = self.saved.toggle_favourite(&listed.server);
        self.filter.scope = self.scope();
        self.refilter();
        cx.notify();
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
        self.tell_groups();
        self.slots = Some(Slots::Loading);
        self.offers = None;
        self.fetched = None;
        self.show_traffic = false;
        self.car_info = None;
        self.launch = Launch::Idle;
        self.car = self.default_car();
        cx.notify();

        let (Some(player), Some(listed)) = (self.player.clone(), self.selected().cloned()) else {
            return;
        };
        if missing_text(&listed).is_some() {
            self.ask_for_offers(index, &listed.server, cx);
        }
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

    /// Asks the server where its content can be downloaded.
    fn ask_for_offers(&mut self, index: usize, server: &Server, cx: &mut Context<Self>) {
        if server.details_port.is_none() {
            self.offers = Some(Offers::Loaded(Vec::new()));
            return;
        }
        self.offers = Some(Offers::Loading);
        let (source, server) = (self.source.clone(), server.clone());
        let asking = cx
            .background_executor()
            .spawn(async move { source.offers(&server) });
        cx.spawn(async move |this, cx| {
            let result = asking.await;
            this.update(cx, |this, cx| {
                if this.selected != Some(index) {
                    return;
                }
                this.offers = Some(match result {
                    Ok(offers) => Offers::Loaded(offers),
                    Err(error) => Offers::Failed(error.to_string().into()),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Downloads and installs what the links point at, one after another.
    pub fn fetch(&mut self, urls: Vec<String>, cx: &mut Context<Self>) {
        if self.fetching.is_some() || urls.is_empty() {
            return;
        }
        self.fetched = None;
        let install = self.game.read(cx).install.clone();
        let title = match urls.len() {
            1 => "Getting the content".to_owned(),
            n => format!("Getting {n} downloads"),
        };
        self.fetching = Some(Job::start(
            title,
            install,
            cx,
            move |install, progress| {
                let mut added = Added::default();
                let mut failed = Vec::new();
                for url in &urls {
                    match install_offer(install, url, progress) {
                        Ok(more) => {
                            added.cars.extend(more.cars);
                            added.tracks.extend(more.tracks);
                            added.kept.extend(more.kept);
                        }
                        Err(Error::Cancelled) => return Err(Error::Cancelled),
                        Err(error) => failed.push(error.to_string()),
                    }
                }
                Ok((added, failed, Library::scan(install)))
            },
            |this, result, cx| {
                this.fetching = None;
                this.fetched = match result {
                    Ok((added, failed, library)) => {
                        this.game
                            .update(cx, |game, cx| game.set_library(library, cx));
                        let mut text = added_text(&added);
                        if !failed.is_empty() {
                            text = format!("{text} Not everything came: {}", failed.join(" "));
                        }
                        Some((text.into(), failed.is_empty()))
                    }
                    Err(Error::Cancelled) => None,
                    Err(error) => Some((error.to_string().into(), false)),
                };
            },
        ));
        cx.notify();
    }

    /// Opens a download page in the browser, and a dialog to take what was
    /// downloaded there.
    pub fn get_from_page(
        &mut self,
        offer: &Offer,
        wanted: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let game = self.game.clone();
        let (url, host) = (offer.url.clone(), offer.host().to_owned());
        let dialog = cx.new(|cx| ContentDialog::new(game, url, host, wanted, window, cx));
        let closed = cx.subscribe(&dialog, |this, _, _: &ContentEvent, cx| {
            this.dialog = None;
            cx.notify();
        });
        self.dialog = Some((dialog, closed));
        cx.notify();
    }

    pub fn dialog(&self) -> Option<&Entity<ContentDialog>> {
        self.dialog.as_ref().map(|(dialog, _)| dialog)
    }

    pub fn is_fetching(&self) -> bool {
        self.fetching.is_some()
    }

    fn installed(&self, car: &str) -> bool {
        self.listing.car(car).is_some_and(|known| known.installed)
    }

    /// Whether `car` on the selected server is AI traffic. The server's
    /// slots say; until they have, its name is the guess.
    pub fn is_traffic(&self, car: &str) -> bool {
        if let Some(Slots::Loaded(slots)) = &self.slots {
            let mut of_car = slots.iter().filter(|slot| slot.car == car).peekable();
            if of_car.peek().is_some() {
                return of_car.all(|slot| slot.ai);
            }
        }
        looks_like_traffic(car)
    }

    /// The car whose picture and figures are open in the car list.
    pub fn car_info(&self) -> Option<&str> {
        self.car_info.as_deref()
    }

    /// Opens a car's picture and figures in the list, or closes them.
    pub fn toggle_car_info(&mut self, car: &str, cx: &mut Context<Self>) {
        self.car_info = match self.car_info.take() {
            Some(open) if open == car => None,
            _ => Some(car.to_owned()),
        };
        cx.notify();
    }

    /// The skin the server hands out with a free slot of `car`, if this
    /// copy of the car has it, else the car's first.
    fn offered_skin<'a>(&self, car: &'a Car) -> Option<&'a Skin> {
        let offered = match &self.slots {
            Some(Slots::Loaded(slots)) => slots
                .iter()
                .find(|slot| slot.car == car.id && !slot.connected)
                .and_then(|slot| car.skin(&slot.skin)),
            _ => None,
        };
        offered.or(car.skins.first())
    }

    pub fn choose_car(&mut self, car: &str, cx: &mut Context<Self>) {
        if self.installed(car) && !self.is_traffic(car) {
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
                    .filter(|slot| slot.car == car && !slot.ai && (!slot.connected || slot.yours))
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
            .filter(|car| self.installed(car) && !self.is_traffic(car))
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
        // The server hands out a skin with the slot; ask for that one.
        let skin = game
            .library
            .car(car)
            .and_then(|car| self.offered_skin(car))
            .map(|skin| skin.id.clone())
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
        let server = listed.server.clone();
        let started = std::time::SystemTime::now();
        self.launch = match launch::join(&game.install, &join) {
            Ok(child) => {
                let install = game.install.clone();
                crate::game::watch(child, started, install, cx, |this, ended| {
                    this.launch = ended;
                });
                Launch::Started
            }
            Err(error) => Launch::Failed(error.to_string().into()),
        };
        if self.launch == Launch::Started {
            let _ = self.saved.joined(&server);
            self.filter.scope = self.scope();
            self.refilter();
        }
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
                    match self.tab {
                        Tab::Lan if self.is_searching_lan() => "Searching…",
                        Tab::Lan => "Search again",
                        _ if self.is_loading() => "Reading…",
                        _ => "Refresh",
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
        let order = self.order;
        let heading = |column: Column, text: &str, width: Option<f32>, right: bool| {
            let sorted = order.column == column;
            let arrow = if order.descending {
                "icons/chevron-down.svg"
            } else {
                "icons/chevron-up.svg"
            };
            div()
                .id(("sort", column as usize))
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .map(|cell| match width {
                    Some(width) => cell.w(px(width)).flex_none(),
                    None => cell.flex_1().min_w_0(),
                })
                .when(right, |cell| cell.justify_end())
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx)))
                .child(label(text).when(sorted, |text| text.text_color(rgb(color::TEXT))))
                .when(sorted, |cell| {
                    cell.child(svg().path(arrow).size(px(11.)).text_color(rgb(color::TEXT)))
                })
                .test_support()
        };
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(34.))
            .px(PAGE)
            .border_b_1()
            .border_color(rgb(color::BORDER))
            .child(div().w(px(STAR_W)).flex_none())
            .child(heading(Column::Name, "Server", None, false))
            .child(heading(Column::Track, "Track", Some(TRACK_W), false))
            .child(heading(Column::Cars, "Cars", Some(CARS_W), false))
            .child(heading(Column::Session, "Session", Some(SESSION_W), false))
            .child(heading(Column::Drivers, "Drivers", Some(DRIVERS_W), true))
            .child(heading(Column::Ping, "Ping", Some(PING_W), true));

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
                            .child(muted(self.empty_text())),
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
        // Traffic is not something to drive; say how many cars there are
        // to choose from.
        let drivable: Vec<&String> = server
            .cars
            .iter()
            .filter(|car| !looks_like_traffic(car))
            .collect();
        let cars = match drivable.as_slice() {
            [one] => self.listing.car_name(one).to_owned(),
            [] => format!("{} cars", server.cars.len()),
            all => format!("{} cars", all.len()),
        };
        let mut about: Vec<String> = Vec::new();
        if let Some((country, _)) = &server.country {
            about.push(country.clone());
        }
        let flag = self.flag(server);
        if server.track.patch_build.is_some() {
            about.push("CSP required".to_owned());
        }
        let missing = missing_text(listed);
        let full = server.free_slots() == 0;
        let favourite = self.saved.is_favourite(&server.address());
        let ping = self.ping(index);

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
                    // A border takes one colour on every side, so the accent
                    // is its own bar; the divider below stays the same as
                    // every other row's.
                    row.relative().bg(rgb(color::ACCENT_WASH)).child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(px(2.))
                            .bg(rgb(color::ACCENT)),
                    )
                } else {
                    row.hover(|style| style.bg(rgb(color::RAISED)))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.stop_following();
                this.select(index, cx)
            }))
            .child(
                div()
                    .id(("star", row))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(STAR_W))
                    .h_full()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favourite(index, cx);
                    }))
                    .child(
                        svg()
                            .path(if favourite {
                                "icons/star-fill.svg"
                            } else {
                                "icons/star.svg"
                            })
                            .size(px(12.))
                            .text_color(rgb(if favourite {
                                color::TEXT
                            } else {
                                color::BORDER_STRONG
                            }))
                            .hover(|style| style.text_color(rgb(color::TEXT_MUTED))),
                    )
                    .test_support(),
            )
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
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(rgb(color::TEXT_MUTED))
                            .truncate()
                            .children(flag.map(|flag| flag_image(flag, 16.)))
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
                    .w(px(TRACK_W))
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
                    .w(px(CARS_W))
                    .flex_none()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(color::TEXT_MUTED))
                    .child(cars),
            )
            .child(
                div()
                    .w(px(SESSION_W))
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
                div()
                    .w(px(DRIVERS_W))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(
                        mono(format!("{}/{}", server.clients, server.capacity), 13.)
                            .text_color(rgb(if full { color::TEXT_DIM } else { color::TEXT })),
                    ),
            )
            .child(
                div()
                    .w(px(PING_W))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(ping_text(ping, 12.5)),
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
        let index = self.selected.unwrap_or_default();
        let mut about: Vec<String> = Vec::new();
        if let Some((country, _)) = &server.country {
            about.push(country.clone());
        }
        if let Some(ping) = self.ping(index) {
            about.push(format!("{ping} ms"));
        }
        match server.found {
            Found::Lan => about.push("On this network".to_owned()),
            Found::Direct => about.push("Not in the lobby".to_owned()),
            Found::Lobby => {}
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
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(font::DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .italic()
                                    .text_size(px(22.))
                                    .line_height(relative(1.1))
                                    .child(server.name.to_uppercase()),
                            )
                            .child(
                                star_button(self.saved.is_favourite(&server.address()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_favourite(index, cx)
                                    }))
                                    .test_support(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .children(self.flag(server).map(|flag| flag_image(flag, 18.)))
                            .child(muted(about.join(" · "))),
                    ),
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
            // What blocks joining comes before the choice of car.
            .children(self.content_box(listed, cx))
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
        // AI traffic cannot be driven; it goes in its own group at the end.
        let (traffic, mut cars): (Vec<&String>, Vec<&String>) =
            server.cars.iter().partition(|car| self.is_traffic(car));
        // Cars the player can take first, then full ones, then the rest.
        cars.sort_by_key(|car| match (self.installed(car), self.free(car)) {
            (true, Some(0)) => 1,
            (true, _) => 0,
            (false, _) => 2,
        });
        let (free_total, slots_total) = match &self.slots {
            Some(Slots::Loaded(slots)) => {
                let players = slots.iter().filter(|s| !s.ai);
                (
                    Some(players.clone().filter(|s| !s.connected).count()),
                    players.count(),
                )
            }
            _ => (None, server.capacity as usize),
        };
        let heading = match free_total {
            Some(free) => format!("Your car · {free} of {slots_total} slots free"),
            None => format!("Your car · {} to choose from", cars.len()),
        };
        let library = self.game.read(cx).library.clone();
        let rows = cars.into_iter().enumerate().map(|(index, car)| {
            let installed = self.installed(car);
            let known = library.car(car);
            let info_open = known.is_some() && self.car_info.as_deref() == Some(car.as_str());
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
            let info_id = car.clone();
            let header = div()
                .id(("car", index))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(40.))
                .px(px(12.))
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
                .when(known.is_some(), |row| {
                    row.child(
                        div()
                            .id(("car-info", index))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(24.))
                            .mr(px(-6.))
                            .rounded(RADIUS)
                            .cursor_pointer()
                            .when(info_open, |button| button.bg(rgb(color::CHIP)))
                            .hover(|style| style.bg(rgb(color::CHIP)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_car_info(&info_id, cx);
                            }))
                            .child(svg().path("icons/info.svg").size(px(14.)).text_color(rgb(
                                if info_open {
                                    color::TEXT
                                } else {
                                    color::TEXT_MUTED
                                },
                            )))
                            .test_support(),
                    )
                })
                .test_support();
            div()
                .flex()
                .flex_col()
                .border_1()
                .rounded(RADIUS)
                .overflow_hidden()
                .border_color(rgb(if chosen { color::ACCENT } else { color::BORDER }))
                .child(header)
                .when_some(known.filter(|_| info_open), |column, known| {
                    column.child(self.car_details(known))
                })
        });

        let traffic_missing = traffic.iter().filter(|car| !self.installed(car)).count();
        let traffic_group = (!traffic.is_empty()).then(|| {
            let summary = match (traffic.len(), traffic_missing) {
                (1, 0) => "1 car, installed".to_owned(),
                (1, _) => "1 car, not installed".to_owned(),
                (n, 0) => format!("{n} cars, all installed"),
                (n, missing) => format!("{n} cars, {missing} not installed"),
            };
            let open = self.show_traffic;
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .id("traffic")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(32.))
                        .px(px(12.))
                        .rounded(RADIUS)
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(color::RAISED)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_traffic = !this.show_traffic;
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path(if open {
                                    "icons/chevron-up.svg"
                                } else {
                                    "icons/chevron-down.svg"
                                })
                                .size(px(12.))
                                .text_color(rgb(color::TEXT_MUTED)),
                        )
                        .child(div().text_size(px(13.)).child("AI traffic"))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.5))
                                .text_color(rgb(color::TEXT_MUTED))
                                .child(summary),
                        )
                        .test_support(),
                )
                .when(open, |group| {
                    group.children(traffic.iter().map(|car| {
                        let installed = self.installed(car);
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(12.))
                            .pl(px(32.))
                            .pr(px(12.))
                            .text_size(px(12.5))
                            .text_color(rgb(color::TEXT_MUTED))
                            .child(
                                div()
                                    .truncate()
                                    .child(self.listing.car_name(car).to_owned()),
                            )
                            .child(
                                mono(
                                    if installed {
                                        "installed"
                                    } else {
                                        "not installed"
                                    },
                                    11.5,
                                )
                                .text_color(rgb(color::TEXT_DIM)),
                            )
                    }))
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(20.))
            .pt(px(16.))
            .child(label(&heading))
            .children(rows)
            .children(traffic_group)
    }

    /// A car's picture in the livery the server hands out, its figures and
    /// what its makers say about it, opened under its row in the car list.
    fn car_details(&self, car: &Car) -> Div {
        let skin = self.offered_skin(car);
        let specs = &car.specs;
        let figure = |value: Option<f64>, unit: &str| -> SharedString {
            value.map_or("—".into(), |v| format!("{v:.0} {unit}").into())
        };
        let meta: Vec<String> = [
            Some(car.brand.clone()),
            car.year.map(|y| y.to_string()),
            car.country.clone(),
            Some(car.class_label()),
            car.drivetrain().map(str::to_owned),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
        let description = car.plain_description();
        // The first paragraph says what the car is; the rest is history.
        let description = description.lines().next().unwrap_or_default().to_owned();

        div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(color::BORDER))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(200.))
                    .overflow_hidden()
                    .bg(pattern_slash(hsla(color::RAISED), 10., 10.))
                    .when_some(skin.and_then(|s| s.preview.clone()), |strip, preview| {
                        strip.child(
                            img(preview)
                                .absolute()
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(muted(meta.join(" · ")))
                            .when_some(skin, |column, skin| {
                                column.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(color::TEXT_DIM))
                                        .child(format!("Livery · {}", skin.label())),
                                )
                            }),
                    )
                    .child(stat_strip([
                        ("Power", figure(specs.power_bhp, "bhp")),
                        ("Torque", figure(specs.torque_nm, "Nm")),
                        ("Weight", figure(specs.weight_kg, "kg")),
                        ("Top speed", figure(specs.top_speed_kmh, "km/h")),
                    ]))
                    .when(!description.is_empty(), |column| {
                        column.child(
                            div()
                                .text_size(px(12.5))
                                .line_height(relative(1.45))
                                .text_color(rgb(color::TEXT_MUTED))
                                .line_clamp(4)
                                .child(description),
                        )
                    }),
            )
    }

    /// What the server needs that the player lacks, and where to get it.
    fn content_box(&self, listed: &Listed, cx: &mut Context<Self>) -> Option<Div> {
        let missing = missing_text(listed)?;
        let server = &listed.server;
        // (kind, folder name, shown name), the track first, traffic last.
        let mut wanted: Vec<(Kind, String, String)> = Vec::new();
        if !listed.track_installed {
            wanted.push((
                Kind::Track,
                listed.track_id.clone(),
                self.listing.track_name(&listed.track_id).to_owned(),
            ));
        }
        let mut cars: Vec<&String> = server
            .cars
            .iter()
            .filter(|car| !self.installed(car))
            .collect();
        cars.sort_by_key(|car| self.is_traffic(car));
        wanted.extend(cars.into_iter().map(|car| {
            (
                Kind::Car,
                car.clone(),
                self.listing.car_name(car).to_owned(),
            )
        }));

        let offers: &[Offer] = match &self.offers {
            Some(Offers::Loaded(offers)) => offers,
            _ => &[],
        };
        let offer_for = |kind: Kind, id: &str| {
            offers.iter().find(|offer| {
                offer.kind == kind && (kind == Kind::Track || offer.id.eq_ignore_ascii_case(id))
            })
        };
        // Downloads several items share are fetched once.
        let mut direct: Vec<String> = Vec::new();
        for (kind, id, _) in &wanted {
            if let Some(offer) = offer_for(*kind, id).filter(|offer| offer.is_direct())
                && !direct.contains(&offer.url)
            {
                direct.push(offer.url.clone());
            }
        }

        let status = match &self.offers {
            _ if server.details_port.is_none() => {
                Some("The server does not say where to get its content.".to_owned())
            }
            None | Some(Offers::Loading) => {
                Some("Asking the server for download links…".to_owned())
            }
            Some(Offers::Failed(error)) => Some(format!(
                "The server did not say where to get its content: {error}"
            )),
            Some(Offers::Loaded(offers)) if offers.is_empty() => {
                Some("The server offers no download links.".to_owned())
            }
            Some(Offers::Loaded(_)) => None,
        };

        const SHOWN: usize = 12;
        let more = wanted.len().saturating_sub(SHOWN);
        let names_by_url = |url: &str| -> Vec<String> {
            wanted
                .iter()
                .filter(|(kind, id, _)| offer_for(*kind, id).is_some_and(|o| o.url == url))
                .map(|(_, _, name)| name.clone())
                .collect()
        };
        let rows = wanted
            .iter()
            .take(SHOWN)
            .enumerate()
            .map(|(row, (kind, id, name))| {
                let offer = offer_for(*kind, id).cloned();
                let action: AnyElement =
                    match offer {
                        _ if self.fetching.is_some() => div().into_any_element(),
                        Some(offer) if offer.is_direct() => {
                            let url = offer.url.clone();
                            button(("fetch", row), "Download")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.fetch(vec![url.clone()], cx)
                                }))
                                .into_any_element()
                        }
                        Some(offer) => {
                            let wanted = names_by_url(&offer.url);
                            button(("page", row), format!("Open {}", offer.host()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.get_from_page(&offer, wanted.clone(), window, cx)
                                }))
                                .into_any_element()
                        }
                        None => mono("no link", 11.5)
                            .text_color(rgb(color::TEXT_DIM))
                            .into_any_element(),
                    };
                let version = offer_for(*kind, id).and_then(|offer| offer.version.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .min_h(px(32.))
                    .child(
                        div()
                            .w(px(40.))
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(color::TEXT_DIM))
                            .child(match kind {
                                Kind::Track => "Track",
                                Kind::Car if self.is_traffic(id) => "AI",
                                Kind::Car => "Car",
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(name.clone()),
                    )
                    .when_some(version, |row, version| {
                        row.child(mono(version, 11.5).text_color(rgb(color::TEXT_MUTED)))
                    })
                    .child(action)
            });

        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(20.))
                .pt(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(label(&format!("Missing · {missing}")))
                        .when(direct.len() > 1 && self.fetching.is_none(), |row| {
                            let count = direct.len();
                            row.child(
                                button("fetch-all", format!("Download all {count}")).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.fetch(direct.clone(), cx)
                                    }),
                                ),
                            )
                        }),
                )
                .when_some(self.fetching.as_ref(), |column, job| {
                    column.child(div().py(px(6.)).child(job.strip()))
                })
                .when_some(self.fetched.clone(), |column, (text, good)| {
                    column.child(
                        div()
                            .text_size(px(12.5))
                            .line_height(relative(1.4))
                            .text_color(rgb(if good {
                                color::GOOD
                            } else {
                                color::ACCENT_TEXT
                            }))
                            .child(text),
                    )
                })
                .when_some(status, |column, status| column.child(muted(status)))
                .children(rows)
                .when(more > 0, |column| {
                    column.child(muted(format!("And {more} more.")))
                }),
        )
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
        if self.tab == Tab::Friends {
            return root
                .child(self.friends_panel(cx))
                .child(self.detail(cx))
                .children(self.picker.as_ref().map(|(picker, _)| picker.clone()))
                .children(self.dialog.as_ref().map(|(dialog, _)| dialog.clone()));
        }
        match &self.load {
            _ if self.tab == Tab::Lan => {}
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
                .when_some(
                    self.stale.clone().filter(|_| self.tab != Tab::Lan),
                    |column, stale| column.child(note(stale)),
                )
                .when_some(self.unanswered_text(), |column, text| {
                    column.child(note(text.into()))
                })
                .child(self.facet_bar(cx))
                .child(self.list(cx)),
        )
        .child(self.detail(cx))
        .children(self.picker.as_ref().map(|(picker, _)| picker.clone()))
        .children(self.dialog.as_ref().map(|(dialog, _)| dialog.clone()))
    }
}

impl OnlineScreen {
    /// What the list says when nothing is in it.
    /// The game's flag for the server's country, if it has one.
    fn flag(&self, server: &Server) -> Option<std::path::PathBuf> {
        let (_, code) = server.country.as_ref()?;
        self.flags.for_code(code)
    }

    fn empty_text(&self) -> &'static str {
        if !self.filter.is_empty() {
            return "No server matches. Loosen the filter to see more.";
        }
        match self.tab {
            Tab::All => "The lobby lists no servers.",
            Tab::Favourites => "No favourites yet. Star a server to keep it here.",
            Tab::Recent => "Servers you join show up here.",
            Tab::Lan if self.is_searching_lan() => "Looking for servers on this network…",
            Tab::Lan => {
                "No server answered on this network. Servers set up for a LAN answer on ports 9456–9458, 9556, 9600–9612 or 9700."
            }
            Tab::Friends => "None of your friends has a server open.",
        }
    }

    /// Kept servers of this tab that could not be reached.
    fn unanswered_text(&self) -> Option<String> {
        let kept: BTreeSet<&str> = match self.tab {
            Tab::Favourites => self.saved.favourites.iter(),
            Tab::Recent => self.saved.recent.iter(),
            _ => return None,
        }
        .map(|kept| kept.address.as_str())
        .collect();
        let names: Vec<&str> = self
            .unanswered
            .iter()
            .filter(|(address, _)| kept.contains(address.as_str()))
            .map(|(_, name)| name.as_str())
            .collect();
        (!names.is_empty()).then(|| format!("Not answering: {}.", names.join(", ")))
    }
}

/// One of the game's flag pictures: the flag sits inside a square canvas.
fn flag_image(path: std::path::PathBuf, size: f32) -> impl IntoElement {
    img(path)
        .flex_none()
        .size(px(size))
        .object_fit(ObjectFit::Contain)
}

fn note(text: SharedString) -> Div {
    div()
        .mx(PAGE)
        .mt(px(10.))
        .px(px(12.))
        .py(px(8.))
        .rounded(RADIUS)
        .bg(rgb(color::RAISED))
        .text_size(px(12.5))
        .text_color(rgb(color::WARN))
        .child(text)
}

/// "18 ms", in the warning colour when slow, or a dash before an answer.
fn ping_text(ping: Option<u32>, size: f32) -> Div {
    match ping {
        Some(ms) => mono(format!("{ms} ms"), size).text_color(rgb(if ms >= SLOW_PING {
            color::WARN
        } else {
            color::TEXT_MUTED
        })),
        None => mono("—", size).text_color(rgb(color::TEXT_DIM)),
    }
}

fn star_button(on: bool) -> Stateful<Div> {
    div()
        .id("favourite")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(RADIUS)
        .border_1()
        .border_color(rgb(color::BORDER_STRONG))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(color::RAISED)))
        .child(
            svg()
                .path(if on {
                    "icons/star-fill.svg"
                } else {
                    "icons/star.svg"
                })
                .size(px(14.))
                .text_color(rgb(color::TEXT)),
        )
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
