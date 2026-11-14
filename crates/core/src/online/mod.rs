//! Online play: the servers the official lobby lists, what each says about
//! itself, narrowing the list down, and joining one.
//!
//! The lobby answers only requests that present themselves as the stock
//! launcher and carry a Steam id. Servers run either Kunos's `acServer` or
//! AssettoServer, which disagree on whether numbers are strings and flags
//! are numbers, so every field is read leniently.

pub mod offers;
pub mod ping;
pub mod saved;

use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    net::SocketAddr,
    time::Duration,
};

use serde_json::Value;

use crate::{Error, Library, Result, ini::Ini, steam::SteamUser};

const LOBBY: &str = "http://93.57.10.21/lobby.ashx/list";
/// Anything else is refused with "UNKNOWN FAILURE".
const LOBBY_AGENT: &str = "Assetto Corsa Launcher";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionKind {
    Booking,
    Practice,
    Qualifying,
    Race,
}

impl SessionKind {
    pub const ALL: [SessionKind; 4] = [
        SessionKind::Booking,
        SessionKind::Practice,
        SessionKind::Qualifying,
        SessionKind::Race,
    ];

    fn from_code(code: u64) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            SessionKind::Booking => "Booking",
            SessionKind::Practice => "Practice",
            SessionKind::Qualifying => "Qualifying",
            SessionKind::Race => "Race",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub kind: SessionKind,
    /// Laps for a race run over laps, minutes otherwise, as the server
    /// reports it.
    pub length: u64,
}

/// A track as a server names it: `[csp/<build>/../[<flags>/../]]<track>[-<layout>]`.
/// The prefix stops the unpatched game from finding the track, so only
/// players with Custom Shaders Patch can join.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRef {
    /// `csp/2651/../e/../`, or empty.
    pub prefix: String,
    /// The oldest patch build the server accepts.
    pub patch_build: Option<u32>,
    /// What follows the prefix: the track and its layout, joined by `-`.
    pub name: String,
}

impl TrackRef {
    pub fn parse(raw: &str) -> Self {
        if let Some(rest) = raw.strip_prefix("csp/") {
            let parts: Vec<&str> = rest.split("/../").collect();
            if parts.len() > 1 {
                let name = parts[parts.len() - 1];
                return Self {
                    prefix: raw[..raw.len() - name.len()].to_owned(),
                    patch_build: parts[0].parse().ok(),
                    name: name.to_owned(),
                };
            }
        }
        Self {
            prefix: String::new(),
            patch_build: None,
            name: raw.to_owned(),
        }
    }

    /// (track, layout). The game joins them with `-`, which track names may
    /// also contain, so installed tracks decide where the split falls.
    pub fn split(&self, library: &Library) -> (String, Option<String>) {
        let name = self.name.as_str();
        if find_track(library, name).is_some() {
            return (name.to_owned(), None);
        }
        for (at, _) in name.match_indices('-') {
            let (track, layout) = (&name[..at], &name[at + 1..]);
            if let Some(found) = find_track(library, track) {
                let has_layout = found.layouts.iter().any(|l| {
                    l.id.as_deref()
                        .is_some_and(|id| id.eq_ignore_ascii_case(layout))
                });
                if has_layout {
                    return (found.id.clone(), Some(layout.to_owned()));
                }
            }
        }
        match name.split_once('-') {
            Some((track, layout)) if !track.is_empty() && !layout.is_empty() => {
                (track.to_owned(), Some(layout.to_owned()))
            }
            _ => (name.to_owned(), None),
        }
    }
}

fn find_track<'a>(library: &'a Library, id: &str) -> Option<&'a crate::Track> {
    library
        .tracks
        .iter()
        .find(|track| track.id.eq_ignore_ascii_case(id))
}

/// How Muretto came to know of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Found {
    #[default]
    Lobby,
    /// It answered a broadcast on this network.
    Lan,
    /// The player kept it, the lobby does not list it, and it was asked
    /// directly.
    Direct,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub ip: String,
    /// Where the server answers questions about itself.
    pub http_port: u16,
    /// Where the game connects.
    pub tcp_port: u16,
    /// Where it answers pings.
    pub udp_port: u16,
    pub found: Found,
    pub name: String,
    /// AssettoServer and Content Manager's server wrapper append `ℹ<port>`
    /// to the name: the port of an extended description.
    pub details_port: Option<u16>,
    pub clients: u32,
    pub capacity: u32,
    pub track: TrackRef,
    /// Each model once.
    pub cars: Vec<String>,
    pub sessions: Vec<Session>,
    pub current: Option<SessionKind>,
    /// Seconds left in the current session.
    pub time_left: u64,
    /// (name, two-letter code)
    pub country: Option<(String, String)>,
    pub password: bool,
}

impl Server {
    fn from_json(value: &Value) -> Option<Self> {
        let text = |key| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key| value.get(key).and_then(lenient_number);
        let port = |key| number(key).and_then(|n| u16::try_from(n).ok());

        let (name, details_port) = split_name(&text("name")?);
        let kinds: Vec<SessionKind> = value
            .get("sessiontypes")
            .and_then(Value::as_array)
            .map(|types| {
                types
                    .iter()
                    .filter_map(lenient_number)
                    .filter_map(SessionKind::from_code)
                    .collect()
            })
            .unwrap_or_default();
        let lengths: Vec<u64> = value
            .get("durations")
            .and_then(Value::as_array)
            .map(|lengths| lengths.iter().filter_map(lenient_number).collect())
            .unwrap_or_default();
        let sessions: Vec<Session> = kinds
            .iter()
            .enumerate()
            .map(|(index, &kind)| Session {
                kind,
                length: lengths.get(index).copied().unwrap_or(0),
            })
            .collect();
        // acServer reports the current session's kind; some others its index.
        let current = number("session").and_then(|code| {
            let kind = SessionKind::from_code(code);
            kind.filter(|kind| kinds.contains(kind))
                .or_else(|| kinds.get(code as usize).copied())
                .or(kind)
        });
        let country = value
            .get("country")
            .and_then(Value::as_array)
            .and_then(|pair| {
                let name = pair.first()?.as_str()?.trim();
                let code = pair.get(1)?.as_str()?.trim();
                (!code.is_empty() && code != "na")
                    .then(|| (name.to_owned(), code.to_ascii_uppercase()))
            });

        let tcp_port = port("tport").or_else(|| port("port"))?;
        Some(Server {
            ip: text("ip").filter(|ip| !ip.trim().is_empty())?,
            http_port: port("cport")?,
            tcp_port,
            udp_port: port("port").unwrap_or(tcp_port),
            found: Found::Lobby,
            name,
            details_port,
            clients: number("clients").unwrap_or(0) as u32,
            capacity: number("maxclients").unwrap_or(0) as u32,
            track: TrackRef::parse(&text("track")?),
            cars: {
                // Each model once, in the server's order.
                let mut seen = BTreeSet::new();
                value
                    .get("cars")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|car| seen.insert(*car))
                    .map(str::to_owned)
                    .collect()
            },
            sessions,
            current,
            time_left: number("timeleft").unwrap_or(0),
            country,
            password: value.get("pass").is_some_and(lenient_flag),
        })
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.ip, self.http_port)
    }

    pub fn free_slots(&self) -> u32 {
        self.capacity.saturating_sub(self.clients)
    }

    /// Where pings go.
    pub fn ping_address(&self) -> Option<SocketAddr> {
        format!("{}:{}", self.ip, self.udp_port).parse().ok()
    }
}

/// Reads what a server says about itself at `/INFO`, which is what it tells
/// the lobby. Servers leave out their own address, so it is taken from
/// where they were asked.
pub fn parse_info(text: &str, ip: &str, http_port: u16) -> Result<Server> {
    let mut value: Value = serde_json::from_str(text)
        .map_err(|e| Error::Network(format!("The server sent something unexpected: {e}")))?;
    if let Some(object) = value.as_object_mut() {
        let missing = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
        };
        if missing(object.get("ip")) {
            object.insert("ip".to_owned(), Value::from(ip));
        }
        object
            .entry("cport")
            .or_insert_with(|| Value::from(http_port));
    }
    Server::from_json(&value)
        .ok_or_else(|| Error::Network("The server did not describe itself.".to_owned()))
}

/// Splits `"Name ℹ8081"` into the name and the port. Control characters,
/// which some names carry and no font can draw, are dropped.
fn split_name(raw: &str) -> (String, Option<u16>) {
    let raw: String = raw.chars().filter(|c| !c.is_control()).collect();
    let raw = raw.as_str();
    if let Some((name, port)) = raw.rsplit_once('ℹ')
        && let Ok(port) = port.trim().parse()
    {
        return (name.trim().to_owned(), Some(port));
    }
    (raw.trim().to_owned(), None)
}

fn lenient_number(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.) as u64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(u64::from(*b)),
        _ => None,
    }
}

fn lenient_flag(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.),
        Value::String(s) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    }
}

/// Reads the lobby's answer; entries that make no sense are skipped.
pub fn parse_lobby(text: &str) -> Result<Vec<Server>> {
    // Refusals come as plain text: "ERROR,SLOW DOWN,TOO MANY CONCURRENT REQUESTS".
    if let Some(reason) = text.trim().strip_prefix("ERROR,") {
        return Err(Error::Network(if reason.contains("SLOW DOWN") {
            "The lobby is busy and asked Muretto to slow down. Try again in a minute.".to_owned()
        } else {
            format!(
                "The lobby refused: {}",
                reason.replace(',', ", ").to_lowercase()
            )
        }));
    }
    let value: Value = serde_json::from_str(text).map_err(|e| {
        let start: String = text.chars().take(60).collect();
        Error::Network(format!(
            "The lobby sent something unexpected ({e}): {start}"
        ))
    })?;
    let entries = value
        .as_array()
        .ok_or_else(|| Error::Network("The lobby sent something unexpected.".to_owned()))?;
    Ok(entries.iter().filter_map(Server::from_json).collect())
}

/// The lobby's list as last read.
#[derive(Debug, Clone, PartialEq)]
pub struct Lobby {
    pub servers: Vec<Server>,
    /// Set when the lobby could not be read and an earlier copy stands in:
    /// how old the copy is and why the lobby failed.
    pub stale: Option<(Duration, String)>,
}

/// Where servers come from. The official lobby in the app; a stand-in in
/// tests.
pub trait Source: Send + Sync {
    /// The account the game runs under, which servers know players by.
    fn player(&self) -> Result<SteamUser>;
    fn servers(&self, player: &SteamUser) -> Result<Lobby>;
    fn slots(&self, server: &Server, player: &SteamUser) -> Result<Vec<Slot>>;
    /// Asks a server at `ip:http_port` about itself, for kept servers the
    /// lobby does not list.
    fn info(&self, ip: &str, http_port: u16) -> Result<Server>;
    /// Servers on this network.
    fn lan(&self) -> Result<Vec<Server>>;
    /// Pings each server and reports answers as they come.
    fn ping(&self, targets: &[SocketAddr], answered: &mut dyn FnMut(SocketAddr, Duration));
    /// Where the server says its content can be downloaded.
    fn offers(&self, server: &Server) -> Result<Vec<offers::Offer>>;
}

/// The official lobby and the servers themselves.
pub struct Official;

impl Source for Official {
    fn player(&self) -> Result<SteamUser> {
        crate::steam::current_user().ok_or_else(|| {
            Error::Network(
                "No Steam account was found on this machine. Sign in to Steam and try again."
                    .to_owned(),
            )
        })
    }

    /// Reads the lobby and keeps a copy; when the lobby refuses, as it does
    /// when asked too often, the copy stands in.
    fn servers(&self, player: &SteamUser) -> Result<Lobby> {
        let cache = dirs::cache_dir().map(|dir| dir.join("muretto").join("lobby.json"));
        let fresh = get(
            &format!("{LOBBY}?guid={}", player.id),
            LOBBY_AGENT,
            Duration::from_secs(30),
        )
        .and_then(|text| Ok((parse_lobby(&text)?, text)));
        match fresh {
            Ok((servers, text)) => {
                if let Some(cache) = &cache
                    && let Some(dir) = cache.parent()
                {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(cache, text);
                }
                Ok(Lobby {
                    servers,
                    stale: None,
                })
            }
            Err(error) => {
                let copy = cache.as_ref().and_then(|cache| {
                    let age = std::fs::metadata(cache)
                        .ok()?
                        .modified()
                        .ok()?
                        .elapsed()
                        .ok()?;
                    let servers = parse_lobby(&std::fs::read_to_string(cache).ok()?).ok()?;
                    Some(Lobby {
                        servers,
                        stale: Some((age, error.to_string())),
                    })
                });
                copy.ok_or(error)
            }
        }
    }

    fn slots(&self, server: &Server, player: &SteamUser) -> Result<Vec<Slot>> {
        fetch_slots(server, player)
    }

    fn info(&self, ip: &str, http_port: u16) -> Result<Server> {
        let text = get(
            &format!("http://{ip}:{http_port}/INFO"),
            LOBBY_AGENT,
            Duration::from_secs(4),
        )?;
        parse_info(&text, ip, http_port)
    }

    fn lan(&self) -> Result<Vec<Server>> {
        let found = ping::broadcast(ping::LAN_PORTS, Duration::from_millis(1500))
            .map_err(|e| Error::Network(format!("Could not search this network: {e}")))?;
        let asking: Vec<_> = found
            .into_iter()
            .map(|found| {
                std::thread::spawn(move || {
                    let ip = found.address.ip().to_string();
                    let mut server = Official.info(&ip, found.http_port).ok()?;
                    server.udp_port = found.address.port();
                    server.found = Found::Lan;
                    Some(server)
                })
            })
            .collect();
        Ok(asking
            .into_iter()
            .filter_map(|asking| asking.join().ok().flatten())
            .collect())
    }

    fn ping(&self, targets: &[SocketAddr], answered: &mut dyn FnMut(SocketAddr, Duration)) {
        let _ = ping::ping(targets, Duration::from_secs(2), answered);
    }

    fn offers(&self, server: &Server) -> Result<Vec<offers::Offer>> {
        offers::fetch_offers(server)
    }
}

/// A car on a server and who, if anyone, drives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub car: String,
    pub skin: String,
    pub driver: Option<String>,
    pub connected: bool,
    /// Booked for the player asking.
    pub yours: bool,
    /// Driven by the server: AI traffic, which players cannot take.
    pub ai: bool,
}

/// The server's cars, slot by slot.
pub fn fetch_slots(server: &Server, player: &SteamUser) -> Result<Vec<Slot>> {
    let url = format!("http://{}/JSON|{}", server.address(), player.id);
    parse_slots(&get(&url, LOBBY_AGENT, Duration::from_secs(6))?)
}

pub fn parse_slots(text: &str) -> Result<Vec<Slot>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::Network(format!("The server sent something unexpected: {e}")))?;
    let cars = value
        .get("Cars")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Network("The server did not list its cars.".to_owned()))?;
    Ok(cars
        .iter()
        .filter_map(|slot| {
            let text = |key| slot.get(key).and_then(Value::as_str).unwrap_or("").trim();
            let flag = |key| slot.get(key).is_some_and(lenient_flag);
            Some(Slot {
                car: slot.get("Model")?.as_str()?.to_owned(),
                skin: text("Skin").to_owned(),
                driver: Some(text("DriverName"))
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                connected: flag("IsConnected"),
                yours: flag("IsRequestedGUID"),
                // AssettoServer leaves its traffic out of the entry list.
                ai: slot.get("IsEntryList").is_some_and(|v| !lenient_flag(v)),
            })
        })
        .collect())
}

pub(crate) fn get(url: &str, agent_name: &str, timeout: Duration) -> Result<String> {
    let failed = |e: ureq::Error| Error::Network(format!("{url} could not be read: {e}"));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(agent_name)
        .timeout_global(Some(timeout))
        .build()
        .into();
    agent
        .get(url)
        .call()
        .map_err(failed)?
        .body_mut()
        .with_config()
        // The full lobby is several megabytes; the default cap is ten.
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(failed)
}

/// Content as the list shows it: its name if installed, its folder name
/// made readable if not.
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub name: String,
    pub installed: bool,
}

/// A server with what the player has of its content worked out.
#[derive(Debug, Clone)]
pub struct Listed {
    pub server: Server,
    pub track_id: String,
    pub layout: Option<String>,
    pub track_installed: bool,
    pub cars_installed: usize,
    /// Lower-cased text a search matches against.
    haystack: String,
}

impl Listed {
    /// The track and at least one car are installed.
    pub fn joinable(&self) -> bool {
        self.track_installed && self.cars_installed > 0
    }

    pub fn missing_cars(&self) -> usize {
        self.server.cars.len() - self.cars_installed
    }
}

/// The ways the list can be narrowed to chosen values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facet {
    Track,
    Car,
    Country,
    Session,
}

impl Facet {
    pub const ALL: [Facet; 4] = [Facet::Track, Facet::Car, Facet::Country, Facet::Session];

    pub fn label(self) -> &'static str {
        match self {
            Facet::Track => "Track",
            Facet::Car => "Car",
            Facet::Country => "Country",
            Facet::Session => "Session",
        }
    }
}

/// Which servers a tab of the Online section is about.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Scope {
    /// What the lobby lists.
    #[default]
    Lobby,
    /// What answered on this network.
    Lan,
    /// The servers at these addresses, wherever they were found.
    Kept(BTreeSet<String>),
}

/// What the player narrowed the list to. Values chosen within a facet are
/// alternatives; facets, switches and the search all have to hold.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub scope: Scope,
    pub text: String,
    pub free_slots: bool,
    pub not_empty: bool,
    pub no_password: bool,
    pub installed: bool,
    pub tracks: BTreeSet<String>,
    pub cars: BTreeSet<String>,
    pub countries: BTreeSet<String>,
    pub sessions: BTreeSet<SessionKind>,
}

impl Filter {
    pub fn chosen(&self, facet: Facet) -> Vec<String> {
        match facet {
            Facet::Track => self.tracks.iter().cloned().collect(),
            Facet::Car => self.cars.iter().cloned().collect(),
            Facet::Country => self.countries.iter().cloned().collect(),
            Facet::Session => self.sessions.iter().map(|s| session_key(*s)).collect(),
        }
    }

    /// Adds `value` to the facet, or takes it out if it is there.
    pub fn toggle(&mut self, facet: Facet, value: &str) {
        fn flip<T: Ord>(set: &mut BTreeSet<T>, value: T) {
            if !set.remove(&value) {
                set.insert(value);
            }
        }
        match facet {
            Facet::Track => flip(&mut self.tracks, value.to_owned()),
            Facet::Car => flip(&mut self.cars, value.to_owned()),
            Facet::Country => flip(&mut self.countries, value.to_owned()),
            Facet::Session => {
                if let Some(kind) = session_from_key(value) {
                    flip(&mut self.sessions, kind);
                }
            }
        }
    }

    pub fn clear(&mut self, facet: Facet) {
        match facet {
            Facet::Track => self.tracks.clear(),
            Facet::Car => self.cars.clear(),
            Facet::Country => self.countries.clear(),
            Facet::Session => self.sessions.clear(),
        }
    }

    /// Whether nothing narrows the list beyond the tab it is in.
    pub fn is_empty(&self) -> bool {
        Filter {
            scope: Scope::default(),
            ..self.clone()
        } == Self::default()
    }

    /// Whether `listed` passes, leaving out the facet named in `except`.
    fn passes(&self, listed: &Listed, words: &[String], except: Option<Facet>) -> bool {
        let server = &listed.server;
        let facet = |facet| except != Some(facet);
        let in_scope = match &self.scope {
            Scope::Lobby => server.found == Found::Lobby,
            Scope::Lan => server.found == Found::Lan,
            Scope::Kept(addresses) => addresses.contains(&server.address()),
        };
        in_scope
            && (!self.free_slots || server.free_slots() > 0)
            && (!self.not_empty || server.clients > 0)
            && (!self.no_password || !server.password)
            && (!self.installed || listed.joinable())
            && words.iter().all(|word| listed.haystack.contains(word))
            && (!facet(Facet::Track)
                || self.tracks.is_empty()
                || self.tracks.contains(&listed.track_id))
            && (!facet(Facet::Car)
                || self.cars.is_empty()
                || server.cars.iter().any(|car| self.cars.contains(car)))
            && (!facet(Facet::Country)
                || self.countries.is_empty()
                || server
                    .country
                    .as_ref()
                    .is_some_and(|(_, code)| self.countries.contains(code)))
            && (!facet(Facet::Session)
                || self.sessions.is_empty()
                || server.current.is_some_and(|s| self.sessions.contains(&s)))
    }

    fn words(&self) -> Vec<String> {
        self.text
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
}

fn session_key(kind: SessionKind) -> String {
    kind.label().to_lowercase()
}

fn session_from_key(key: &str) -> Option<SessionKind> {
    SessionKind::ALL
        .into_iter()
        .find(|kind| session_key(*kind) == key)
}

/// One choice within a facet, with how many servers it would leave.
#[derive(Debug, Clone, PartialEq)]
pub struct FacetValue {
    pub id: String,
    pub name: String,
    /// Servers matching everything else in the filter that have this value.
    pub count: usize,
    /// For content: whether the player has it.
    pub installed: Option<bool>,
    pub chosen: bool,
}

/// The lobby's servers, ready to be narrowed down.
#[derive(Debug, Default)]
pub struct Listing {
    pub servers: Vec<Listed>,
    tracks: HashMap<String, Known>,
    cars: HashMap<String, Known>,
    countries: HashMap<String, String>,
}

impl Listing {
    pub fn new(servers: Vec<Server>, library: &Library) -> Self {
        let mut tracks: HashMap<String, Known> = HashMap::new();
        let mut cars: HashMap<String, Known> = HashMap::new();
        let mut countries = HashMap::new();
        let installed_car = |id: &str| {
            library
                .cars
                .iter()
                .find(|car| car.owned && car.id.eq_ignore_ascii_case(id))
        };

        let servers = servers
            .into_iter()
            .map(|server| {
                let (track_id, layout) = server.track.split(library);
                let track = tracks.entry(track_id.clone()).or_insert_with(|| {
                    match find_track(library, &track_id).filter(|t| t.owned) {
                        Some(track) => Known {
                            name: track.name.clone(),
                            installed: true,
                        },
                        None => Known {
                            name: readable(&track_id),
                            installed: false,
                        },
                    }
                });
                let mut haystack = format!("{} {} {}", server.name, track.name, server.track.name);
                let track_installed = track.installed;
                let mut cars_installed = 0;
                for id in &server.cars {
                    let car = cars
                        .entry(id.clone())
                        .or_insert_with(|| match installed_car(id) {
                            Some(car) => Known {
                                name: car.name.clone(),
                                installed: true,
                            },
                            None => Known {
                                name: readable(id),
                                installed: false,
                            },
                        });
                    cars_installed += usize::from(car.installed);
                    haystack.push(' ');
                    haystack.push_str(&car.name);
                    haystack.push(' ');
                    haystack.push_str(id);
                }
                if let Some((name, code)) = &server.country {
                    countries
                        .entry(code.clone())
                        .or_insert_with(|| name.clone());
                    haystack.push(' ');
                    haystack.push_str(name);
                }
                Listed {
                    haystack: haystack.to_lowercase(),
                    track_id,
                    layout,
                    track_installed,
                    cars_installed,
                    server,
                }
            })
            .collect();
        Self {
            servers,
            tracks,
            cars,
            countries,
        }
    }

    pub fn track(&self, id: &str) -> Option<&Known> {
        self.tracks.get(id)
    }

    pub fn car(&self, id: &str) -> Option<&Known> {
        self.cars.get(id)
    }

    pub fn car_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.cars.get(id).map_or(id, |car| car.name.as_str())
    }

    pub fn track_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.tracks.get(id).map_or(id, |track| track.name.as_str())
    }

    /// Indices of the servers that pass, in `order`. `pings` holds each
    /// server's ping in milliseconds, by index, once it has answered.
    pub fn matching(&self, filter: &Filter, order: Order, pings: &[Option<u32>]) -> Vec<usize> {
        let words = filter.words();
        let mut found: Vec<usize> = self
            .servers
            .iter()
            .enumerate()
            .filter(|(_, listed)| filter.passes(listed, &words, None))
            .map(|(index, _)| index)
            .collect();
        let ping = |index: usize| pings.get(index).copied().flatten();
        // Names are compared often; lower-case each once.
        let names: HashMap<usize, String> = found
            .iter()
            .map(|&index| (index, self.servers[index].server.name.to_lowercase()))
            .collect();
        found.sort_by(|&a, &b| {
            let (la, lb) = (&self.servers[a], &self.servers[b]);
            let (sa, sb) = (&la.server, &lb.server);
            let busiest = || {
                sb.clients
                    .cmp(&sa.clients)
                    .then_with(|| names[&a].cmp(&names[&b]))
            };
            let by = match order.column {
                Column::Name => names[&a].cmp(&names[&b]),
                Column::Track => self
                    .track_name(&la.track_id)
                    .to_lowercase()
                    .cmp(&self.track_name(&lb.track_id).to_lowercase()),
                Column::Cars => sa.cars.len().cmp(&sb.cars.len()),
                Column::Session => sa.current.cmp(&sb.current),
                Column::Drivers => sa.clients.cmp(&sb.clients),
                Column::Ping => match (ping(a), ping(b)) {
                    (Some(pa), Some(pb)) => pa.cmp(&pb),
                    // Servers yet to answer go last whichever way round.
                    (Some(_), None) => return Ordering::Less,
                    (None, Some(_)) => return Ordering::Greater,
                    (None, None) => return busiest(),
                },
            };
            let by = if order.descending { by.reverse() } else { by };
            by.then_with(busiest)
        });
        found
    }

    /// The choices within `facet`, each counted against the rest of the
    /// filter, most common first. Chosen values are kept even when nothing
    /// has them.
    pub fn facet(&self, facet: Facet, filter: &Filter) -> Vec<FacetValue> {
        let words = filter.words();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for listed in &self.servers {
            if !filter.passes(listed, &words, Some(facet)) {
                continue;
            }
            let server = &listed.server;
            match facet {
                Facet::Track => *counts.entry(listed.track_id.clone()).or_default() += 1,
                Facet::Car => {
                    for car in &server.cars {
                        *counts.entry(car.clone()).or_default() += 1;
                    }
                }
                Facet::Country => {
                    if let Some((_, code)) = &server.country {
                        *counts.entry(code.clone()).or_default() += 1;
                    }
                }
                Facet::Session => {
                    if let Some(kind) = server.current {
                        *counts.entry(session_key(kind)).or_default() += 1;
                    }
                }
            }
        }
        for chosen in filter.chosen(facet) {
            counts.entry(chosen).or_default();
        }
        let chosen: BTreeSet<String> = filter.chosen(facet).into_iter().collect();
        let mut values: Vec<FacetValue> = counts
            .into_iter()
            .map(|(id, count)| {
                let (name, installed) = match facet {
                    Facet::Track => {
                        let known = self.tracks.get(&id);
                        (
                            known.map_or_else(|| readable(&id), |k| k.name.clone()),
                            Some(known.is_some_and(|k| k.installed)),
                        )
                    }
                    Facet::Car => {
                        let known = self.cars.get(&id);
                        (
                            known.map_or_else(|| readable(&id), |k| k.name.clone()),
                            Some(known.is_some_and(|k| k.installed)),
                        )
                    }
                    Facet::Country => (
                        self.countries
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(|| id.clone()),
                        None,
                    ),
                    Facet::Session => (
                        session_from_key(&id).map_or_else(|| id.clone(), |s| s.label().to_owned()),
                        None,
                    ),
                };
                FacetValue {
                    chosen: chosen.contains(&id),
                    id,
                    name,
                    count,
                    installed,
                }
            })
            .collect();
        values.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        values
    }

    /// What a chosen value of `facet` is called.
    pub fn value_name(&self, facet: Facet, id: &str) -> String {
        match facet {
            Facet::Track => self.track_name(id).to_owned(),
            Facet::Car => self.car_name(id).to_owned(),
            Facet::Country => self
                .countries
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.to_owned()),
            Facet::Session => {
                session_from_key(id).map_or_else(|| id.to_owned(), |s| s.label().to_owned())
            }
        }
    }
}

/// A column the server list can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Track,
    Cars,
    Session,
    Drivers,
    Ping,
}

impl Column {
    /// Counts read best largest first; names, sessions and pings smallest.
    fn starts_descending(self) -> bool {
        matches!(self, Column::Cars | Column::Drivers)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Order {
    pub column: Column,
    pub descending: bool,
}

impl Default for Order {
    /// Busiest first.
    fn default() -> Self {
        Order {
            column: Column::Drivers,
            descending: true,
        }
    }
}

impl Order {
    /// The order after clicking `column`'s heading: the other way round if
    /// it is already sorted by it, else by it in its natural direction.
    pub fn by(self, column: Column) -> Self {
        if self.column == column {
            Order {
                column,
                descending: !self.descending,
            }
        } else {
            Order {
                column,
                descending: column.starts_descending(),
            }
        }
    }
}

/// Whether a car is, by its folder name, AI traffic. Servers list traffic
/// models among their cars; their slots say for sure, this guesses before
/// they have answered.
pub fn looks_like_traffic(id: &str) -> bool {
    id.to_ascii_lowercase().contains("traffic")
}

/// `ks_nordschleife` → `Nordschleife`; for content the player does not have
/// and so has no proper name for.
pub fn readable(id: &str) -> String {
    let id = id.strip_prefix("ks_").unwrap_or(id);
    id.split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Everything the game needs to join a server.
#[derive(Debug, Clone)]
pub struct Join<'a> {
    pub listed: &'a Listed,
    pub car: &'a str,
    pub skin: &'a str,
    pub player: &'a SteamUser,
    /// The name others see; the Steam name when empty.
    pub driver_name: &'a str,
    pub password: &'a str,
}

impl Join<'_> {
    /// The `race.ini` that makes the game connect instead of starting a
    /// session of its own. The server decides everything else.
    pub fn race_ini(&self) -> Ini {
        let server = &self.listed.server;
        let name = if self.driver_name.trim().is_empty() {
            self.player.name.as_str()
        } else {
            self.driver_name.trim()
        };
        // The game reads the patch's prefix itself; keep it.
        let track = format!("{}{}", server.track.prefix, self.listed.track_id);
        let layout = self.listed.layout.as_deref().unwrap_or("");
        let mut ini = Ini::new();
        ini.set("HEADER", "VERSION", 2);
        ini.set("RACE", "TRACK", &track)
            .set("RACE", "CONFIG_TRACK", layout)
            .set("RACE", "MODEL", self.car)
            .set("RACE", "MODEL_CONFIG", "")
            .set("RACE", "SKIN", self.skin)
            .set("RACE", "CARS", 1)
            .set("RACE", "AI_LEVEL", 100)
            .set("RACE", "FIXED_SETUP", 0)
            .set("RACE", "PENALTIES", 1);
        ini.set("CAR_0", "MODEL", self.car)
            .set("CAR_0", "MODEL_CONFIG", "")
            .set("CAR_0", "SKIN", self.skin)
            .set("CAR_0", "SETUP", "")
            .set("CAR_0", "DRIVER_NAME", name)
            .set("CAR_0", "NATIONALITY", "")
            .set("CAR_0", "BALLAST", 0)
            .set("CAR_0", "RESTRICTOR", 0);
        ini.set("REMOTE", "ACTIVE", 1)
            .set("REMOTE", "SERVER_IP", &server.ip)
            .set("REMOTE", "SERVER_PORT", server.tcp_port)
            .set("REMOTE", "SERVER_HTTP_PORT", server.http_port)
            .set("REMOTE", "SERVER_NAME", &server.name)
            .set("REMOTE", "REQUESTED_CAR", self.car)
            .set("REMOTE", "NAME", name)
            .set("REMOTE", "TEAM", "")
            .set("REMOTE", "GUID", &self.player.id)
            .set("REMOTE", "PASSWORD", self.password);
        ini.set("REPLAY", "ACTIVE", 0);
        ini.set("GHOST_CAR", "PLAYING", 0)
            .set("GHOST_CAR", "RECORDING", 0);
        ini.set("BENCHMARK", "ACTIVE", 0);
        ini.set("RESTART", "ACTIVE", 0);
        ini
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Install, content::Library};

    /// Excerpts of the real lobby: an acServer entry with strings for
    /// numbers, an AssettoServer one with numbers and a details port, and
    /// one too broken to list.
    const LOBBY_EXCERPT: &str = r#"[
      {"ip":"94.23.107.62","port":9095,"cport":8085,"tport":9095,
       "name":"MKBT5 - Barcelona\u0004 Practice ℹ8085","clients":3,"maxclients":24,
       "track":"fn_barcelona-barcelona_2023","cars":["mkbt_f3","ks_mazda_mx5_cup"],
       "session":1,"sessiontypes":["1"],"durations":["120"],"timeleft":786,
       "country":["Germany","DE"],"pass":false},
      {"ip":"212.115.101.28","port":7790,"cport":8782,"tport":7790,
       "name":"Moov Town Drift","clients":0,"maxclients":32,
       "track":"csp/3465/../Moov Town","cars":["ks_mazda_mx5_cup","ks_toyota_ae86_drift"],
       "session":0,"sessiontypes":[1],"durations":[172800],"timeleft":81821,
       "country":["Saudi Arabia","SA"],"pass":1,"inverted":0},
      {"ip":"148.251.69.57","port":9605,"cport":8085,"tport":9605,
       "name":"Shutoko night","clients":43,"maxclients":43,
       "track":"csp/2651/../e/../park-short","cars":["ks_mazda_mx5_cup"],
       "session":3,"sessiontypes":["1","2","3"],"durations":["145","60","12"],"timeleft":0,
       "country":["Germany","DE"],"pass":false},
      {"name":"no address"}
    ]"#;

    fn library() -> Library {
        // Tests run in parallel; each builds its own folder.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("muretto-online-{}-{n}", std::process::id()))
            .join("steamapps/common/assettocorsa");
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, contents: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        write(
            "content/cars/ks_mazda_mx5_cup/ui/ui_car.json",
            r#"{"name": "Mazda MX-5 Cup", "brand": "Mazda"}"#,
        );
        write("content/cars/ks_mazda_mx5_cup/data/car.ini", "");
        for layout in ["long", "short"] {
            write(
                &format!("content/tracks/park/ui/{layout}/ui_track.json"),
                &format!(r#"{{"name": "Park - {layout}"}}"#),
            );
        }
        write("content/tracks/park/park.kn5", "");
        let library = Library::scan(&Install::at(root.clone(), None));
        std::fs::remove_dir_all(root.parent().unwrap().parent().unwrap().parent().unwrap())
            .unwrap();
        library
    }

    #[test]
    fn lobby_entries_are_read_whatever_server_wrote_them() {
        let servers = parse_lobby(LOBBY_EXCERPT).unwrap();
        assert_eq!(servers.len(), 3, "the broken entry is skipped");

        let kunos = &servers[0];
        assert_eq!(kunos.name, "MKBT5 - Barcelona Practice");
        assert_eq!(kunos.details_port, Some(8085));
        assert_eq!((kunos.http_port, kunos.tcp_port), (8085, 9095));
        assert_eq!(kunos.current, Some(SessionKind::Practice));
        assert_eq!(
            kunos.sessions,
            [Session {
                kind: SessionKind::Practice,
                length: 120
            }]
        );
        assert_eq!(kunos.free_slots(), 21);
        assert!(!kunos.password);

        let assetto_server = &servers[1];
        assert!(assetto_server.password, "a flag sent as a number");
        assert_eq!(
            assetto_server.current,
            Some(SessionKind::Practice),
            "an index"
        );
        assert_eq!(assetto_server.track.patch_build, Some(3465));
        assert_eq!(assetto_server.track.name, "Moov Town");
        assert_eq!(
            assetto_server.country,
            Some(("Saudi Arabia".to_owned(), "SA".to_owned()))
        );

        let full = &servers[2];
        assert_eq!(full.track.prefix, "csp/2651/../e/../");
        assert_eq!(full.current, Some(SessionKind::Race));
        assert_eq!(full.free_slots(), 0);
    }

    #[test]
    fn installed_tracks_decide_where_the_layout_starts() {
        let library = library();
        let split = |raw: &str| TrackRef::parse(raw).split(&library);
        assert_eq!(
            split("park-short"),
            ("park".to_owned(), Some("short".to_owned()))
        );
        assert_eq!(split("park"), ("park".to_owned(), None));
        assert_eq!(
            split("fn_barcelona-barcelona_2023"),
            ("fn_barcelona".to_owned(), Some("barcelona_2023".to_owned()))
        );
        assert_eq!(split("Moov Town"), ("Moov Town".to_owned(), None));
    }

    #[test]
    fn facets_narrow_the_list_and_count_against_the_rest() {
        let library = library();
        let listing = Listing::new(parse_lobby(LOBBY_EXCERPT).unwrap(), &library);
        let names = |filter: &Filter| {
            listing
                .matching(filter, Order::default(), &[])
                .into_iter()
                .map(|i| listing.servers[i].server.name.as_str())
                .collect::<Vec<_>>()
        };
        let mut filter = Filter::default();
        assert_eq!(
            names(&filter),
            [
                "Shutoko night",
                "MKBT5 - Barcelona Practice",
                "Moov Town Drift"
            ],
            "busiest first"
        );

        // Installed content gets its proper name; the rest a readable one.
        let tracks = listing.facet(Facet::Track, &filter);
        let park = tracks.iter().find(|v| v.id == "park").unwrap();
        assert_eq!((park.name.as_str(), park.installed), ("Park", Some(true)));
        let barcelona = tracks.iter().find(|v| v.id == "fn_barcelona").unwrap();
        assert_eq!(
            (barcelona.name.as_str(), barcelona.installed),
            ("Fn Barcelona", Some(false))
        );

        // Choosing a car: servers with any chosen car remain, and the car
        // facet still counts as if nothing were chosen in it.
        filter.toggle(Facet::Car, "ks_toyota_ae86_drift");
        assert_eq!(names(&filter), ["Moov Town Drift"]);
        let cars = listing.facet(Facet::Car, &filter);
        let mx5 = cars.iter().find(|v| v.id == "ks_mazda_mx5_cup").unwrap();
        assert_eq!(
            (mx5.count, mx5.chosen, mx5.name.as_str()),
            (3, false, "Mazda MX-5 Cup")
        );
        filter.toggle(Facet::Car, "mkbt_f3");
        assert_eq!(
            names(&filter),
            ["MKBT5 - Barcelona Practice", "Moov Town Drift"]
        );

        // Other facets count only what the car choice leaves.
        let countries = listing.facet(Facet::Country, &filter);
        assert_eq!(
            countries
                .iter()
                .map(|v| (v.name.as_str(), v.count))
                .collect::<Vec<_>>(),
            [("Germany", 1), ("Saudi Arabia", 1)]
        );

        filter.clear(Facet::Car);
        filter.toggle(Facet::Session, "race");
        assert_eq!(names(&filter), ["Shutoko night"]);
        filter.clear(Facet::Session);

        // Switches and search.
        filter.free_slots = true;
        filter.no_password = true;
        assert_eq!(names(&filter), ["MKBT5 - Barcelona Practice"]);
        filter = Filter {
            installed: true,
            ..Filter::default()
        };
        assert_eq!(names(&filter), ["Shutoko night"]);
        filter = Filter {
            text: "mx-5 saudi".to_owned(),
            ..Filter::default()
        };
        assert_eq!(names(&filter), ["Moov Town Drift"]);
    }

    #[test]
    fn slots_show_who_drives_what() {
        let slots = parse_slots(
            r#"{"Features":["SLOT_INDEX"],"Cars":[
              {"Model":"ks_mazda_mx5_cup","Skin":"red","DriverName":null,"IsRequestedGUID":false,"IsConnected":false},
              {"Model":"ks_mazda_mx5_cup","Skin":"blue","DriverName":"T. Okafor","IsRequestedGUID":false,"IsConnected":true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].driver, None);
        assert!(
            matches!(parse_lobby("ERROR,SLOW DOWN,TOO MANY CONCURRENT REQUESTS"), Err(Error::Network(m)) if m.contains("slow down"))
        );
        assert_eq!(slots[1].driver.as_deref(), Some("T. Okafor"));
        assert!(slots[1].connected && !slots[1].yours);
    }

    #[test]
    fn joining_writes_the_server_into_the_session() {
        let library = library();
        let listing = Listing::new(parse_lobby(LOBBY_EXCERPT).unwrap(), &library);
        let player = SteamUser {
            id: "76561198000000001".to_owned(),
            name: "L. Brandt".to_owned(),
        };
        let join = Join {
            listed: &listing.servers[2],
            car: "ks_mazda_mx5_cup",
            skin: "red",
            player: &player,
            driver_name: "",
            password: "",
        };
        let ini = join.race_ini();
        let get = |section, key| ini.get(section, key).unwrap().to_owned();
        assert_eq!(get("RACE", "TRACK"), "csp/2651/../e/../park");
        assert_eq!(get("RACE", "CONFIG_TRACK"), "short");
        assert_eq!(get("REMOTE", "ACTIVE"), "1");
        assert_eq!(get("REMOTE", "SERVER_IP"), "148.251.69.57");
        assert_eq!(get("REMOTE", "SERVER_PORT"), "9605");
        assert_eq!(get("REMOTE", "SERVER_HTTP_PORT"), "8085");
        assert_eq!(get("REMOTE", "REQUESTED_CAR"), "ks_mazda_mx5_cup");
        assert_eq!(get("REMOTE", "GUID"), player.id);
        assert_eq!(get("REMOTE", "NAME"), "L. Brandt");
        assert_eq!(get("CAR_0", "DRIVER_NAME"), "L. Brandt");
    }

    #[test]
    fn columns_sort_both_ways_and_unanswered_pings_go_last() {
        let library = library();
        let listing = Listing::new(parse_lobby(LOBBY_EXCERPT).unwrap(), &library);
        let names = |order: Order, pings: &[Option<u32>]| {
            listing
                .matching(&Filter::default(), order, pings)
                .into_iter()
                .map(|i| listing.servers[i].server.name.as_str())
                .collect::<Vec<_>>()
        };
        let by_name = Order::default().by(Column::Name);
        assert!(!by_name.descending);
        assert_eq!(
            names(by_name, &[]),
            [
                "MKBT5 - Barcelona Practice",
                "Moov Town Drift",
                "Shutoko night"
            ]
        );
        assert_eq!(
            names(by_name.by(Column::Name), &[])[0],
            "Shutoko night",
            "a second click turns it round"
        );
        let by_ping = Order::default().by(Column::Ping);
        let pings = [Some(80), None, Some(20)];
        assert_eq!(
            names(by_ping, &pings),
            [
                "Shutoko night",
                "MKBT5 - Barcelona Practice",
                "Moov Town Drift"
            ]
        );
        assert_eq!(
            names(by_ping.by(Column::Ping), &pings),
            [
                "MKBT5 - Barcelona Practice",
                "Shutoko night",
                "Moov Town Drift"
            ]
        );
        assert_eq!(
            names(Order::default().by(Column::Cars), &[])[0],
            "MKBT5 - Barcelona Practice"
        );
    }

    #[test]
    fn tabs_show_the_lobby_the_network_or_kept_servers() {
        let library = library();
        let mut servers = parse_lobby(LOBBY_EXCERPT).unwrap();
        servers[1].found = Found::Lan;
        servers[2].found = Found::Direct;
        let listing = Listing::new(servers, &library);
        let names = |scope: Scope| {
            let filter = Filter {
                scope,
                ..Filter::default()
            };
            assert!(filter.is_empty(), "a tab is not a filter");
            listing
                .matching(&filter, Order::default(), &[])
                .into_iter()
                .map(|i| listing.servers[i].server.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Scope::Lobby), ["MKBT5 - Barcelona Practice"]);
        assert_eq!(names(Scope::Lan), ["Moov Town Drift"]);
        assert_eq!(
            names(Scope::Kept(
                [
                    "148.251.69.57:8085".to_owned(),
                    "94.23.107.62:8085".to_owned()
                ]
                .into()
            )),
            ["Shutoko night", "MKBT5 - Barcelona Practice"]
        );
    }

    #[test]
    fn a_server_asked_directly_describes_itself_like_in_the_lobby() {
        let server = parse_info(
            r#"{"ip":"","port":9600,"tport":9600,"cport":8081,"name":"Garage ℹ8082","clients":1,
                "maxclients":8,"track":"park","cars":["ks_mazda_mx5_cup"],"session":1,"sessiontypes":[1]}"#,
            "192.168.1.20",
            8081,
        )
        .unwrap();
        assert_eq!(server.ip, "192.168.1.20");
        assert_eq!(server.address(), "192.168.1.20:8081");
        assert_eq!(server.details_port, Some(8082));
        assert_eq!(
            server.ping_address(),
            Some("192.168.1.20:9600".parse().unwrap())
        );
    }

    #[test]
    fn traffic_slots_are_told_from_the_entry_list() {
        let slots = parse_slots(
            r#"{"Cars":[
              {"Model":"traffic_volvo_v70","Skin":"a","IsEntryList":false,"IsConnected":false},
              {"Model":"hk51_ef9","Skin":"b","IsEntryList":true,"IsConnected":false},
              {"Model":"ks_mazda_mx5_cup","Skin":"c"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            slots.iter().map(|s| s.ai).collect::<Vec<_>>(),
            [true, false, false]
        );
        assert!(looks_like_traffic("nohesi_traffic_bmw_x5"));
        assert!(!looks_like_traffic("ks_mazda_mx5_cup"));
    }

    #[test]
    fn folder_names_are_made_readable() {
        assert_eq!(readable("ks_nordschleife"), "Nordschleife");
        assert_eq!(
            readable("shuto_revival_project_beta"),
            "Shuto Revival Project Beta"
        );
        assert_eq!(readable("Moov Town"), "Moov Town");
    }
}
